//! This is the poller modulo
//! This modulo is for implementation of a i/o handler using io-uring

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::task::Waker;

use io_uring::squeue::Entry;
use io_uring::{IoUring, opcode, types};
use nix::sys::eventfd::{EfdFlags, EventFd};
use slab::Slab;

const WAKE_TOKEN: u64 = u64::MAX;

pub struct Poller {
    ring: IoUring,
    wakers: Slab<Waker>,
    results: HashMap<u64, i32>, // store the result of ops, mostly are length
    addrs: HashMap<u64, (Box<libc::sockaddr_storage>, Box<libc::socklen_t>)>,
    wakeup_fd: EventFd,
}

impl Poller {
    pub fn new() -> io::Result<Self> {
        // Typically, 256 or 512 is a reasonable default for entries.
        // TODO: make this configurable.
        let entries = 512;
        let mut ring = IoUring::new(entries)?;
        let wakeup_fd =
            EventFd::from_value_and_flags(0, EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK)
                .map_err(|e| io::Error::from_raw_os_error(e as i32))?;

        // Register the multishot poll operation at creation time
        let poll_e = opcode::PollAdd::new(types::Fd(wakeup_fd.as_raw_fd()), libc::POLLIN as _)
            .multi(true)
            .build()
            .user_data(WAKE_TOKEN);
        unsafe {
            ring.submission()
                .push(&poll_e)
                .expect("submission queue is full");
        }
        Ok(Self {
            ring,
            wakers: Slab::with_capacity(entries as usize),
            results: HashMap::new(),
            addrs: HashMap::new(),
            wakeup_fd,
        })
    }

    pub fn wakeup(&self) -> io::Result<()> {
        let val: u64 = 1; // u64 is necessary to ensure read 8 bytes
        let ret = unsafe {
            libc::write(
                self.wakeup_fd.as_raw_fd(),
                &val as *const _ as *const _,
                std::mem::size_of::<u64>(),
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn poll(&mut self) -> io::Result<Vec<Waker>> {
        self.ring.submit_and_wait(1)?;

        let mut cqueue = self.ring.completion();
        cqueue.sync();

        let mut ready_wakers = Vec::new();

        for cqe in cqueue {
            let user_data = cqe.user_data();
            let result = cqe.result();

            if user_data == WAKE_TOKEN {
                let mut buf: u64 = 0;
                unsafe {
                    libc::read(self.wakeup_fd.as_raw_fd(), &mut buf as *mut _ as *mut _, 8);
                }
                continue;
            }

            if let Some(waker) = self.wakers.try_remove(user_data as _) {
                //TODO: Maybe add err handling logic for cases that result < 0
                ready_wakers.push(waker);
                self.results.insert(user_data, result as _);
            }
        }

        Ok(ready_wakers)
    }
    pub fn submit_read_entry<F>(&mut self, fd: F, buf: &mut [u8], waker: Waker) -> u64
    where
        F: IntoRawFd,
    {
        let user_data = self.wakers.insert(waker) as _;
        let read_e = opcode::Read::new(
            types::Fd(fd.into_raw_fd()),
            buf.as_mut_ptr(),
            buf.len() as _,
        )
        .build()
        .user_data(user_data);

        // self.wakers.insert(user_data, waker);

        unsafe {
            self.push_entry(read_e);
        }

        user_data
    }

    pub fn submit_write_entry<F>(&mut self, fd: F, buf: &[u8], waker: Waker) -> u64
    where
        F: IntoRawFd,
    {
        let user_data = self.wakers.insert(waker) as _;

        let write_e = opcode::Write::new(types::Fd(fd.into_raw_fd()), buf.as_ptr(), buf.len() as _)
            .build()
            .user_data(user_data);

        // self.wakers.insert(user_data, waker);

        unsafe {
            self.push_entry(write_e);
        }

        user_data
    }

    pub fn submit_send_to_entry<F>(
        &mut self,
        fd: F,
        buf: &[u8],
        addr: SocketAddr,
        waker: Waker,
    ) -> u64
    where
        F: IntoRawFd,
    {
        let user_data = self.wakers.insert(waker) as _;
        let (addr_storage, addr_len) = socket_addr_to_storage(addr);
        let iov = [libc::iovec {
            iov_base: buf.as_ptr() as *mut _,
            iov_len: buf.len(),
        }];
        let msg = libc::msghdr {
            msg_name: &addr_storage as *const _ as *mut _,
            msg_namelen: addr_len,
            msg_iov: iov.as_ptr() as *mut _,
            msg_iovlen: iov.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };

        let send_e = opcode::SendMsg::new(types::Fd(fd.into_raw_fd()), &msg)
            .build()
            .user_data(user_data);

        // self.wakers.insert(user_data, waker);

        unsafe {
            self.push_entry(send_e);
        }

        user_data
    }

    pub fn submit_recv_from_entry<F>(&mut self, fd: F, buf: &mut [u8], waker: Waker) -> u64
    where
        F: IntoRawFd,
    {
        let user_data = self.wakers.insert(waker) as _;
        let mut storage = Box::new(unsafe { std::mem::zeroed() });
        let addrlen = Box::new(std::mem::size_of::<libc::sockaddr_storage>() as _);
        let iov = [libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut _,
            iov_len: buf.len(),
        }];
        let mut msg = libc::msghdr {
            msg_name: &mut *storage as *mut _ as *mut _,
            msg_namelen: *addrlen,
            msg_iov: iov.as_ptr() as *mut _,
            msg_iovlen: iov.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };

        let recv_e = opcode::RecvMsg::new(types::Fd(fd.into_raw_fd()), &mut msg)
            .build()
            .user_data(user_data);

        // self.wakers.insert(user_data, waker);
        self.addrs.insert(user_data, (storage, addrlen));

        unsafe {
            self.push_entry(recv_e);
        }

        user_data
    }

    pub fn submit_accept_entry(
        &mut self,
        fd: RawFd,
        storage: &mut libc::sockaddr_storage,
        addrlen: &mut libc::socklen_t,
        waker: Waker,
    ) -> u64 {
        let user_data = self.wakers.insert(waker) as _;
        *addrlen = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let accept_e = opcode::Accept::new(types::Fd(fd), storage as *mut _ as *mut _, addrlen)
            .build()
            .user_data(user_data);

        // self.wakers.insert(user_data, waker);

        unsafe {
            self.push_entry(accept_e);
        }

        user_data
    }

    pub fn submit_connect_entry(&mut self, fd: RawFd, addr: &SocketAddr, waker: Waker) -> u64 {
        let user_data = self.wakers.insert(waker) as _;
        let (addr_storage, addr_len) = socket_addr_to_storage(*addr);

        let connect_e =
            opcode::Connect::new(types::Fd(fd), &addr_storage as *const _ as *mut _, addr_len)
                .build()
                .user_data(user_data);

        unsafe {
            self.push_entry(connect_e);
        }

        user_data
    }

    pub fn unique_token(&self) -> u64 {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CURRENT_TOKEN: AtomicUsize = AtomicUsize::new(0);
        CURRENT_TOKEN
            .fetch_add(1, Ordering::Relaxed)
            .try_into()
            .unwrap()
    }

    pub fn get_result(&mut self, user_data: u64) -> Option<i32> {
        self.results.remove(&user_data)
    }

    pub fn get_addr_result(&mut self, user_data: u64) -> Option<(i32, SocketAddr)> {
        let result = self.results.remove(&user_data)?;
        let (storage, len) = self.addrs.remove(&user_data)?;
        let addr = socket_addr_from_storage(&storage, *len as _).unwrap();
        Some((result, addr))
    }

    unsafe fn push_entry(&mut self, entry: Entry) {
        unsafe {
            if self.ring.submission().push(&entry).is_err() {
                self.ring
                    .submit()
                    .expect("Failed to submit when queue is full");
                self.ring
                    .submission()
                    .push(&entry)
                    .expect("Failed to push event after submitting")
            }
        }
    }
}

fn socket_addr_from_storage(
    storage: &libc::sockaddr_storage,
    _len: libc::socklen_t,
) -> io::Result<SocketAddr> {
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            let addr: &libc::sockaddr_in = unsafe { std::mem::transmute(storage) };
            let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
            let port = u16::from_be(addr.sin_port);
            Ok(SocketAddr::new(ip.into(), port))
        }
        libc::AF_INET6 => {
            let addr: &libc::sockaddr_in6 = unsafe { std::mem::transmute(storage) };
            let ip = Ipv6Addr::from(addr.sin6_addr.s6_addr);
            let port = u16::from_be(addr.sin6_port);
            Ok(SocketAddr::new(ip.into(), port))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid address family",
        )),
    }
}

fn socket_addr_to_storage(addr: SocketAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = match addr {
        SocketAddr::V4(addr) => {
            let mut sockaddr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            sockaddr.sin_family = libc::AF_INET as _;
            sockaddr.sin_port = addr.port().to_be();
            sockaddr.sin_addr.s_addr = u32::from(*addr.ip()).to_be();
            unsafe {
                // std::ptr::copy_nonoverlapping(
                //     &sockaddr_in as *const _ as *const _,
                //     &mut storage as *mut _ as *mut _,
                //     std::mem::size_of::<libc::sockaddr_in>(),
                // );
                std::ptr::write(&mut storage as *mut _ as *mut _, sockaddr);
            }
            std::mem::size_of::<libc::sockaddr_in>() as _
        }
        SocketAddr::V6(addr) => {
            let mut sockaddr: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
            sockaddr.sin6_family = libc::AF_INET6 as _;
            sockaddr.sin6_port = addr.port().to_be();
            sockaddr.sin6_addr.s6_addr = addr.ip().octets();
            unsafe {
                // std::ptr::copy_nonoverlapping(
                //     &sockaddr_in6 as *const _ as *const _,
                //     &mut storage as *mut _ as *mut _,
                //     std::mem::size_of::<libc::sockaddr_in6>(),
                // );
                std::ptr::write(&mut storage as *mut _ as *mut _, sockaddr);
            }
            std::mem::size_of::<libc::sockaddr_in6>() as _
        }
    };
    (storage, len)
}

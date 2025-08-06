//! This is the poller modulo
//! This modulo is for implementation of a i/o handler using io-uring

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::task::Waker;

use io_uring::{IoUring, opcode, types};
use nix::sys::eventfd::{EfdFlags, EventFd};

const WAKE_TOKEN: u64 = u64::MAX;

pub struct Poller {
    ring: IoUring,
    wakers: HashMap<u64, Waker>,
    results: HashMap<u64, usize>, // store the result of ops, mostly are length
    wakeup_fd: EventFd,
}

impl Poller {
    pub fn new() -> io::Result<Self> {
        // Typically, 256 or 512 is a reasonable default for entries.
        // TODO: make this configurable.
        let entries = 256;
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
            wakers: HashMap::new(),
            results: HashMap::new(),
            wakeup_fd,
        })
    }

    pub fn wakeup(&self) -> io::Result<()> {
        let val = 1;
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
                let mut buf = 0;
                unsafe {
                    libc::read(self.wakeup_fd.as_raw_fd(), &mut buf as *mut _ as *mut _, 8);
                }
                continue;
            }
            if let Some(waker) = self.wakers.remove(&user_data) {
                ready_wakers.push(waker);
                self.results.insert(user_data, result as _);
            }
        }

        Ok(ready_wakers)
    }
    pub fn sumbit_read_entry<F>(&mut self, fd: F, buf: &mut [u8], user_data: u64, waker: Waker)
    where
        F: IntoRawFd,
    {
        let read_e = opcode::Read::new(
            types::Fd(fd.into_raw_fd()),
            buf.as_mut_ptr(),
            buf.len() as _,
        )
        .build()
        .user_data(user_data);

        self.wakers.insert(user_data, waker);

        unsafe {
            self.ring
                .submission()
                .push(&read_e)
                .expect("submission queue is full")
        }
    }

    pub fn sumbit_write_entry<F>(&mut self, fd: F, buf: &[u8], user_data: u64, waker: Waker)
    where
        F: IntoRawFd,
    {
        let write_e = opcode::Write::new(types::Fd(fd.into_raw_fd()), buf.as_ptr(), buf.len() as _)
            .build()
            .user_data(user_data);

        self.wakers.insert(user_data, waker);

        unsafe {
            self.ring
                .submission()
                .push(&write_e)
                .expect("submission queue is full")
        }
    }

    pub fn submit_accept_entry(
        &mut self,
        fd: RawFd,
        storage: &mut libc::sockaddr_storage,
        addrlen: &mut libc::socklen_t,
        user_data: u64,
        waker: Waker,
    ) {
        *addrlen = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let accept_e = opcode::Accept::new(types::Fd(fd), storage as *mut _ as *mut _, addrlen)
            .build()
            .user_data(user_data);

        self.wakers.insert(user_data, waker);

        unsafe {
            self.ring
                .submission()
                .push(&accept_e)
                .expect("submission queue is full");
        }
    }

    pub fn unique_token(&self) -> u64 {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CURRENT_TOKEN: AtomicUsize = AtomicUsize::new(0);
        CURRENT_TOKEN
            .fetch_add(1, Ordering::Relaxed)
            .try_into()
            .unwrap()
    }

    pub fn get_result(&mut self, user_data: u64) -> Option<usize> {
        self.results.remove(&user_data)
    }
}

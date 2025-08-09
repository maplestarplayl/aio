use std::future::Future;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use libc::{sockaddr_storage, socklen_t};

use crate::BufResult;
use crate::proactor::{FutureState, Proactor};
use crate::traits::{IoBuf, IoBufMut};

pub struct AsyncTcpListener {
    listener: TcpListener,
    // proactor: Arc<Proactor>,
}

impl AsyncTcpListener {
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub async fn accept(&self) -> io::Result<AsyncTcpStream> {
        // let proactor = self.proactor.clone();

        let accept_fut = AcceptFuture::new(&self.listener);

        let new_fd = accept_fut.await?;

        let stream = unsafe { TcpStream::from_raw_fd(new_fd) };
        stream.set_nonblocking(true)?;

        Ok(AsyncTcpStream {
            stream,
            // proactor: self.proactor.clone(),
        })
    }
}

pub struct AsyncTcpStream {
    pub(crate) stream: TcpStream,
    // proactor: Arc<Proactor>,
}

impl AsyncTcpStream {
    pub async fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid address"))?;

        let domain = if addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };

        let socket =
            socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
        socket.set_nonblocking(true)?;

        let connect_fut = ConnectFuture::new(socket.as_raw_fd(), addr);
        connect_fut.await?;

        Ok(Self {
            stream: socket.into(),
        })
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }
}

pub struct ConnectFuture {
    fd: RawFd,
    addr: SocketAddr,
    state: FutureState,
}

impl Drop for ConnectFuture {
    fn drop(&mut self) {
        if let FutureState::Pending(token) = self.state {
            crate::proactor::Proactor::with(|p| {
                p.get_poller().drop_request(token);
            });
        }
    }
}

impl ConnectFuture {
    pub fn new(fd: RawFd, addr: SocketAddr) -> Self {
        Self {
            fd,
            addr,
            state: FutureState::Unsubmitted,
        }
    }
}

impl Future for ConnectFuture {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| match this.state {
            FutureState::Unsubmitted => {
                let mut poller = local_proactor.get_poller();
                let user_data =
                    poller.submit_connect_entry(this.fd, &this.addr, cx.waker().clone());

                this.state = FutureState::Pending(user_data);
                Poll::Pending
            }
            FutureState::Pending(user_data) => {
                let mut poller = local_proactor.get_poller();

                if let Some(_res) = poller.get_result(user_data) {
                    this.state = FutureState::Done;
                    // if res < 0 {
                    //     return Poll::Ready(Err(io::Error::from_raw_os_error(res as _)));
                    // }
                    Poll::Ready(Ok(()))
                } else {
                    Poll::Pending
                }
            }
            FutureState::Done => {
                panic!("Polling after future completed")
            }
        })
    }
}

pub struct AcceptFuture<'a> {
    listener: &'a TcpListener,
    // poller: Arc<Mutex<Poller>>,
    state: FutureState,
    storage: sockaddr_storage,
}

impl<'a> Drop for AcceptFuture<'a> {
    fn drop(&mut self) {
        if let FutureState::Pending(token) = self.state {
            crate::proactor::Proactor::with(|p| {
                p.get_poller().drop_request(token);
            });
        }
    }
}

impl<'a> AcceptFuture<'a> {
    pub fn new(listener: &'a TcpListener) -> Self {
        Self {
            listener,
            // poller,
            state: FutureState::Unsubmitted,
            storage: unsafe { std::mem::zeroed() },
        }
    }
}

impl<'a> Future for AcceptFuture<'a> {
    type Output = io::Result<i32>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| match this.state {
            FutureState::Unsubmitted => {
                let mut poller = local_proactor.get_poller();
                let mut addr_len = std::mem::size_of::<sockaddr_storage>() as socklen_t;

                let user_data = poller.submit_accept_entry(
                    this.listener.as_raw_fd(),
                    &mut this.storage,
                    &mut addr_len,
                    cx.waker().clone(),
                );

                this.state = FutureState::Pending(user_data);
                Poll::Pending
            }
            FutureState::Pending(user_data) => {
                let mut poller = local_proactor.get_poller();

                if let Some(res) = poller.get_result(user_data) {
                    this.state = FutureState::Done;
                    if res < 0 {
                        return Poll::Ready(Err(io::Error::from_raw_os_error(-res)));
                    }
                    Poll::Ready(Ok(res))
                } else {
                    Poll::Pending
                }
            }
            FutureState::Done => {
                panic!("Polling after future completed")
            }
        })
    }
}

pub struct TcpStreamReadFuture<'a, B: IoBufMut> {
    stream: &'a AsyncTcpStream,
    buf: Option<B>,
    state: FutureState,
}

impl<'a, B: IoBufMut> Drop for TcpStreamReadFuture<'a, B> {
    fn drop(&mut self) {
        if let FutureState::Pending(token) = self.state {
            crate::proactor::Proactor::with(|p| {
                p.get_poller().drop_request(token);
            });
        }
    }
}

impl<'a, B: IoBufMut> TcpStreamReadFuture<'a, B> {
    pub(crate) fn new(stream: &'a AsyncTcpStream, buf: B) -> Self {
        Self {
            stream,
            buf: Some(buf),
            state: FutureState::Unsubmitted,
        }
    }
}

impl<'a, B: IoBufMut> Future for TcpStreamReadFuture<'a, B> {
    type Output = BufResult<usize, B>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We never move the fields out of the pinned struct.
        let this = unsafe { self.get_unchecked_mut() };

        let mut buf = this.buf.take().expect("Buffer must be set before polling");
        Proactor::with(|local_proactor| {
            let mut poller = local_proactor.get_poller();
            match this.state {
                FutureState::Unsubmitted => {
                    // NOTE: Assumes `submit_read_entry` exists on the poller.
                    let user_data = poller.submit_read_entry(
                        this.stream.stream.as_raw_fd(),
                        buf.as_mut_slice(),
                        // buf.bytes_total(),
                        cx.waker().clone(),
                    );
                    this.state = FutureState::Pending(user_data);
                    this.buf = Some(buf);
                    Poll::Pending
                }
                FutureState::Pending(user_data) => {
                    if let Some(res) = poller.get_result(user_data) {
                        this.state = FutureState::Done;
                        if res < 0 {
                            Poll::Ready(Err((io::Error::from_raw_os_error(res as _), buf)))
                        } else {
                            let n = res as usize;
                            unsafe { buf.set_init(n) };
                            Poll::Ready(Ok((n, buf)))
                        }
                    } else {
                        this.buf = Some(buf);
                        Poll::Pending
                    }
                }
                FutureState::Done => {
                    panic!("Polling after future completed")
                }
            }
        })
    }
}

pub struct TcpStreamWriteFuture<'a, B: IoBuf> {
    stream: &'a AsyncTcpStream,
    buf: Option<B>,
    state: FutureState,
}

impl<'a, B: IoBuf> Drop for TcpStreamWriteFuture<'a, B> {
    fn drop(&mut self) {
        if let FutureState::Pending(token) = self.state {
            crate::proactor::Proactor::with(|p| {
                p.get_poller().drop_request(token);
            });
        }
    }
}

impl<'a, B: IoBuf> TcpStreamWriteFuture<'a, B> {
    pub(crate) fn new(stream: &'a AsyncTcpStream, buf: B) -> Self {
        Self {
            stream,
            buf: Some(buf),
            state: FutureState::Unsubmitted,
        }
    }
}

impl<'a, B: IoBuf> Future for TcpStreamWriteFuture<'a, B> {
    type Output = BufResult<usize, B>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We never move the fields out of the pinned struct.
        let this = unsafe { self.get_unchecked_mut() };

        let buf = this.buf.take().expect("Buffer must be set before polling");
        Proactor::with(|local_proactor| {
            let mut poller = local_proactor.get_poller();
            match this.state {
                FutureState::Unsubmitted => {
                    // NOTE: Assumes `submit_write_entry` exists on the poller.
                    let user_data = poller.submit_write_entry(
                        this.stream.stream.as_raw_fd(),
                        // SAFETY: The IoBuf trait ensures that the pointer is valid and the
                        // number of initialized bytes is correct.
                        unsafe { std::slice::from_raw_parts(buf.stable_ptr(), buf.bytes_init()) },
                        cx.waker().clone(),
                    );
                    this.state = FutureState::Pending(user_data);
                    this.buf = Some(buf);
                    Poll::Pending
                }
                FutureState::Pending(user_data) => {
                    if let Some(res) = poller.get_result(user_data) {
                        this.state = FutureState::Done;
                        if res < 0 {
                            Poll::Ready(Err((io::Error::from_raw_os_error(res as _), buf)))
                        } else {
                            let n = res as usize;
                            Poll::Ready(Ok((n, buf)))
                        }
                    } else {
                        this.buf = Some(buf);
                        Poll::Pending
                    }
                }
                FutureState::Done => {
                    panic!("Polling after future completed")
                }
            }
        })
    }
}

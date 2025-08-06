use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, FromRawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use libc::{sockaddr_storage, socklen_t};

use crate::proactor::{Proactor, ReadState};

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
    stream: TcpStream,
    // proactor: Arc<Proactor>,
}

impl AsyncTcpStream {
    pub async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // let proactor = self.proactor.clone();
        Proactor::read(self.stream.as_raw_fd(), buf).await
    }
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Proactor::write(self.stream.as_raw_fd(), buf).await
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let mut written = 0;
        while written < buf.len() {
            let n = self.write(&buf[written..]).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ));
            }
            written += n;
        }
        Ok(())
    }
}
pub struct AcceptFuture<'a> {
    listener: &'a TcpListener,
    // poller: Arc<Mutex<Poller>>,
    state: ReadState,
    storage: sockaddr_storage,
}

impl<'a> AcceptFuture<'a> {
    pub fn new(listener: &'a TcpListener) -> Self {
        Self {
            listener,
            // poller,
            state: ReadState::Unsubmitted,
            storage: unsafe { std::mem::zeroed() },
        }
    }
}

impl<'a> Future for AcceptFuture<'a> {
    type Output = io::Result<i32>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| match this.state {
            ReadState::Unsubmitted => {
                let mut poller = local_proactor.get_poller();
                let user_data = poller.unique_token();
                let mut addr_len = std::mem::size_of::<sockaddr_storage>() as socklen_t;

                poller.submit_accept_entry(
                    this.listener.as_raw_fd(),
                    &mut this.storage,
                    &mut addr_len,
                    user_data,
                    cx.waker().clone(),
                );

                this.state = ReadState::Pending(user_data);
                Poll::Pending
            }
            ReadState::Pending(user_data) => {
                let mut poller = local_proactor.get_poller();

                if let Some(res) = poller.get_result(user_data) {
                    this.state = ReadState::Done;
                    Poll::Ready(Ok(res as i32))
                } else {
                    Poll::Pending
                }
            }
            ReadState::Done => {
                panic!("Polling after future completed")
            }
        })
    }
}

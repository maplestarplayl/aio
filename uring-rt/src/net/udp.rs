use std::io;
use std::net::{self, SocketAddr};
use std::os::fd::AsRawFd;

use crate::proactor::Proactor;

pub struct UdpSocket {
    inner: net::UdpSocket,
}

impl UdpSocket {
    pub fn bind(addr: SocketAddr) -> io::Result<UdpSocket> {
        let socket = net::UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(UdpSocket { inner: socket })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        Proactor::recv_from(self, buf).await
    }

    pub async fn send_to(&self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
        Proactor::send_to(self, buf, target).await
    }
}

impl AsRawFd for UdpSocket {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.inner.as_raw_fd()
    }
}

impl AsRawFd for &UdpSocket {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.inner.as_raw_fd()
    }
}

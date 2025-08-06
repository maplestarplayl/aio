// use aio_macros::test;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;

use uring_rt::net::AsyncTcpListener;

#[aio::test]
async fn test_tcp_echo() {
    let listener = AsyncTcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = thread::spawn(move || {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(b"hello").unwrap();
        let mut buf = [0; 5];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
    });

    let mut stream = listener.accept().await.unwrap();
    let mut buf = [0; 5];
    let n = stream.read(&mut buf).await.unwrap();
    stream.write_all(&buf[..n]).await.unwrap();

    handle.join().unwrap();
}

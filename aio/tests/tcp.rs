// use aio_macros::test;

use std::net::SocketAddr;

use aio::{AsyncReadRent, AsyncReadRentExt, AsyncWriteRentExt};
use uring_rt::net::{AsyncTcpListener, AsyncTcpStream};

#[aio::test]
async fn test_tcp_echo() {
    let listener = AsyncTcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = async {
        let mut stream = listener.accept().await.unwrap();
        let buf = vec![0; 5];
        let (n, buf) = stream.read(buf).await.unwrap();
        stream.write_all(buf[..n].to_vec()).await.unwrap();
    };

    let client = async {
        let mut stream = AsyncTcpStream::connect(addr).await.unwrap();
        stream.write_all(b"hello").await.unwrap();

        let buf = vec![0; 5];
        let (_, buf) = stream.read_exact(buf).await.unwrap();

        assert_eq!(buf, b"hello");
    };

    futures::join!(server, client);
}

#[aio::test]
async fn aio_main() {
    let listener = AsyncTcpListener::bind("127.0.0.1:0").unwrap();
    let local_addr = listener.local_addr().unwrap();

    aio::spawn(echo_server_aio(listener));

    let mut handles = Vec::new();
    for id in 0..10 {
        let h = aio::spawn(async move {
            run_client_aio(local_addr).await;
        });
        handles.push(h);
    }

    for h in handles {
        h.await
    }
}

async fn echo_server_aio(listener: AsyncTcpListener) {
    println!("Echo server started, waiting for connections...");
    for _ in 0..10 {
        let mut socket = listener.accept().await.unwrap();

        // 每个连接交给单独任务处理
        aio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            loop {
                let (n, new_buf) = match socket.read(buf).await {
                    Ok((0, _)) => break, // 对方关闭连接
                    Ok((n, buf)) => (n, buf),
                    Err(_) => break,
                };
                buf = new_buf;
                // 原样回写
                if socket.write_all(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
        });
    }
}

async fn run_client_aio(server_addr: SocketAddr) {
    let mut stream = AsyncTcpStream::connect(server_addr).await.unwrap();
    let msg = b"hello from client";
    stream.write_all(msg).await.unwrap();

    let buf = vec![0u8; msg.len()];
    let (_, buf) = stream.read_exact(buf).await.unwrap();
    assert_eq!(buf, msg);
}

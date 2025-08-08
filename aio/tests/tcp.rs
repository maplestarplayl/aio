// use aio_macros::test;

use std::net::SocketAddr;

use uring_rt::net::{AsyncTcpListener, AsyncTcpStream};

#[aio::test]
async fn test_tcp_echo() {
    let listener = AsyncTcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = async {
        let mut stream = listener.accept().await.unwrap();
        let mut buf = [0; 5];
        let n = stream.read(&mut buf).await.unwrap();
        stream.write_all(&buf[..n]).await.unwrap();
    };

    let client = async {
        let mut stream = AsyncTcpStream::connect(addr).await.unwrap();
        stream.write_all(b"hello").await.unwrap();
        let mut buf = [0; 5];

        let mut bytes_read = 0;
        while bytes_read < buf.len() {
            let n = stream.read(&mut buf[bytes_read..]).await.unwrap();
            if n == 0 {
                panic!("failed to read exact number of bytes");
            }
            bytes_read += n;
        }
        assert_eq!(&buf, b"hello");
    };

    futures::join!(server, client);
}

// #[test]
// fn test_aio() {
//     aio::spawn(async {
//         aio_main().await;
//     });
//     let _ = aio::run();
// }
#[aio::test]
async fn aio_main() {
    let listener = AsyncTcpListener::bind("127.0.0.1:0").unwrap();
    let local_addr = listener.local_addr().unwrap();

    aio::spawn(echo_server_aio(listener));

    let mut handles = Vec::new();
    for id in 0..10 {
        let h = aio::spawn(async move {
            run_client_aio(local_addr, id).await;
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
            let mut buf = [0u8; 1024];
            loop {
                let n = match socket.read(&mut buf).await {
                    Ok(0) => break, // 对方关闭连接
                    Ok(n) => n,
                    Err(_) => break,
                };
                // 原样回写
                if socket.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        });
    }
}

async fn run_client_aio(server_addr: SocketAddr, id: usize) {
    let mut stream = AsyncTcpStream::connect(server_addr).await.unwrap();
    let msg = format!("hello from client {}", id);
    stream.write_all(msg.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; msg.len()];
    stream.read(&mut buf).await.unwrap();
}

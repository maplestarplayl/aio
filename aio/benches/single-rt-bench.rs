use std::net::SocketAddr;

use aio::{AsyncReadRent, AsyncReadRentExt, AsyncTcpStream, AsyncWriteRentExt};
use criterion::{Criterion, criterion_group, criterion_main};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener as TokioTcpListener;
use tokio::runtime::Builder;
use uring_rt::net::AsyncTcpListener;

const NUM_CLIENTS: usize = 50;
const NUM_MESSAGES_PER_CLIENT: usize = 100;

fn bench_tcp_echo(c: &mut Criterion) {
    c.bench_function("tcp_echo_with_poller_multi_client", |b| {
        b.iter(|| {
            aio::spawn(aio_main());
            let _ = aio::run();
        });
    });
    c.bench_function("tcp_echo_with_tokio_multi_client", |b| {
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        b.iter(|| {
            rt.block_on(tokio_main());
        });
    });
}

criterion_group!(benches, bench_tcp_echo);
criterion_main!(benches);

async fn tokio_main() {
    let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();
    // 启动服务端
    let server_handle = tokio::spawn(echo_server(listener));

    // 启动多个客户端任务
    let mut handles = Vec::new();
    for _ in 0..NUM_CLIENTS {
        handles.push(tokio::spawn(async move {
            run_client(local_addr).await;
        }));
    }

    // 等待所有客户端完成
    for h in handles {
        h.await.unwrap();
    }
    // 等待服务端完成
    server_handle.await.unwrap();
}

async fn echo_server(listener: TokioTcpListener) {
    let mut handles = Vec::new();
    for _ in 0..NUM_CLIENTS {
        let (mut socket, _addr) = listener.accept().await.unwrap();

        // 每个连接交给单独任务处理
        let handle = tokio::spawn(async move {
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
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }
}

async fn run_client(server_addr: SocketAddr) {
    let mut stream = tokio::net::TcpStream::connect(server_addr).await.unwrap();
    let msg = format!("hello from client");

    for _ in 0..NUM_MESSAGES_PER_CLIENT {
        stream.write_all(msg.as_bytes()).await.unwrap();

        let mut buf = vec![0u8; msg.len()];
        stream.read_exact(&mut buf).await.unwrap();
    }
}

async fn aio_main() {
    let listener = AsyncTcpListener::bind("127.0.0.1:0").unwrap();
    let local_addr = listener.local_addr().unwrap();

    let handler = aio::spawn(echo_server_aio(listener));

    let mut handles = Vec::new();
    for _ in 0..NUM_CLIENTS {
        let h = aio::spawn(async move {
            run_client_aio(local_addr).await;
        });
        handles.push(h);
    }

    for h in handles {
        h.await
    }
    handler.await;
}

async fn echo_server_aio(listener: AsyncTcpListener) {
    // let mut handles = Vec::new();
    for _ in 0..NUM_CLIENTS {
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

    // for handle in handles {
    //     handle.await;
    // }
}

async fn run_client_aio(server_addr: SocketAddr) {
    let mut stream = AsyncTcpStream::connect(server_addr).await.unwrap();
    let msg = b"Hello from client";

    for _ in 0..NUM_MESSAGES_PER_CLIENT {
        stream.write_all(msg).await.unwrap();

        let buf = vec![0u8; msg.len()];
        stream.read_exact(buf).await.unwrap();
    }
}



use aio::{AsyncTcpListener, AsyncTcpStream, spawn};

#[aio::main]
pub async fn main()  {
    work().await
}

async fn work() {
    let listener = AsyncTcpListener::bind("127.0.0.1:8080").unwrap();

    loop {
        match listener.accept().await {
            Ok(stream) => {
                println!("Accepted a new connection");
                spawn(stream_work(stream));
            }
            Err(e) => {
                eprintln!("Failed to accept connection: {e}");
                break;
            }
        }
    }

}

async fn stream_work(mut stream: AsyncTcpStream) {
    let mut buf = vec![0; 1024];
    loop {
        match stream.read(&mut buf).await {
            Ok(bytes_read) => {
                // Check for disconnection (EOF)
                if bytes_read == 0 {
                    println!("Client closed the connection.");
                    return;
                }
                println!("{:?}", String::from_utf8_lossy(&buf[..bytes_read]))
            }
            Err(_) => return,
        }
        
    }
}

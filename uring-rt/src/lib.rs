pub mod net;
use std::future::Future;
use std::io as stdio;

pub use net::{AsyncTcpListener, AsyncTcpStream, UdpSocket};

mod poller;
mod proactor;
mod traits;

pub use aio_macros::{main, test};
pub use proactor::Proactor;
pub use traits::{
    AsyncReadRent, AsyncReadRentExt, AsyncWriteRent, AsyncWriteRentExt, BufResult, IoBuf, IoBufMut,
};

use crate::proactor::JoinHandle;
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future<Output = ()> + 'static,
{
    Proactor::with(|p| p.spawn(future))
}

pub fn run() -> stdio::Result<()> {
    Proactor::with(|p| p.run())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::net::AsyncTcpListener;
    use crate::proactor::Proactor;
    use crate::traits::AsyncReadRent;

    #[test]
    fn proactor_reads_file_correctly() -> io::Result<()> {
        Proactor::with(|proactor| {
            let (tx, rx) = mpsc::channel();
            proactor.spawn(async move {
                // 1. Setup
                let file_path = "test_proactor_read.txt";
                let content_to_write = b"Hello from uring-runtime!";
                fs::write(file_path, content_to_write).unwrap();

                // 2. Async read
                let file_to_read = File::open(file_path).unwrap();
                let mut read_buffer = vec![0u8; content_to_write.len()];
                let bytes_read = Proactor::read(file_to_read, &mut read_buffer)
                    .await
                    .unwrap();

                // 3. Assert
                assert_eq!(bytes_read as usize, content_to_write.len());
                assert_eq!(read_buffer.as_slice(), content_to_write);

                // 4. Teardown & Signal completion
                fs::remove_file(file_path).unwrap();
                tx.send(()).unwrap();
            });

            proactor.run().expect("Proactor failed to run");
            rx.recv().expect("Test task panicked");
        });
        Ok(())
    }

    /// Tests that the proactor can successfully write to a file.
    #[test]
    fn proactor_writes_file_correctly() -> io::Result<()> {
        Proactor::with(|proactor| {
            let (tx, rx) = mpsc::channel();
            proactor.spawn(async move {
                // 1. Setup
                let file_path = "test_proactor_write.txt";
                let content_to_write = b"Writing is fun with io_uring!";
                let file = File::create(file_path).unwrap();

                // 2. Async write
                let bytes_written = Proactor::write(file, content_to_write).await.unwrap();

                // 3. Assert
                assert_eq!(bytes_written as usize, content_to_write.len());
                let file_content = fs::read(file_path).unwrap();
                assert_eq!(file_content.as_slice(), content_to_write);

                // 4. Teardown & Signal completion
                fs::remove_file(file_path).unwrap();
                tx.send(()).unwrap();
            });

            proactor.run().expect("Proactor failed to run");
            rx.recv().expect("Test task panicked");
        });
        Ok(())
    }

    /// Tests that the async TCP listener can accept a connection and read from the stream.
    #[test]
    fn tcp_stream_read_works() -> io::Result<()> {
        Proactor::with(|proactor| {
            // 1. Setup: Start a TCP listener on an available port.
            let listener = AsyncTcpListener::bind("127.0.0.1:0")?;
            let server_addr = listener.local_addr()?;

            // 2. Client: Spawn a thread to connect and send data.
            let client_handle = thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                let mut client_stream =
                    std::net::TcpStream::connect(server_addr).expect("Client failed to connect");
                client_stream
                    .write_all(b"hello world")
                    .expect("Client failed to write");
            });

            // 3. Server Logic: Spawn a task to accept and read from the client.
            proactor.spawn(async move {
                let mut stream = listener
                    .accept()
                    .await
                    .expect("Failed to accept connection");
                let buf = vec![0; 11];
                let (bytes_read, buf) = stream.read(buf).await.expect("Failed to read from stream");

                // 4. Assert: Verify the received data.
                assert_eq!(bytes_read, 11);
                assert_eq!(buf, b"hello world");
            });

            // 5. Run server and cleanup.
            proactor.run()?;
            client_handle.join().expect("Client thread panicked");
            Ok(())
        })
    }
}

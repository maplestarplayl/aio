# AIO: An Experimental `io_uring`-based Async Runtime

AIO is an experimental async runtime for Rust, built from the ground up using Linux's high-performance `io_uring` interface. It aims to provide a modern, efficient, and easy-to-use platform for building fast network applications.

## Features

*   **`io_uring` Backend**: Leverages the power of `io_uring` for true asynchronous I/O, minimizing syscalls and memory copies.
*   **Proactor Design**: Implements the proactor pattern, where the runtime handles I/O operations and wakes tasks only upon completion.
*   **Async Networking APIs**: Provides high-level async abstractions for `TcpListener`, `TcpStream`, and `UdpSocket` and `tokio`-like apis
*   **Ergonomic Macros**: Includes `#[aio::main]` and `#[aio::test]` to simplify writing applications and tests.
*   **Safe Buffer Management**: Uses the `AsyncReadRent` trait to allow for safe and efficient buffer handling across `await` points without lifetime complexities.

*   **Thread-per-core design**: Take inspirations from `monoio`, implement a thread-per-core archtitecture, eliminate the need to add `Send` `Sync` to tasks to enable cross-thread transfering.


## Getting Started

To use `aio`, add it to your `Cargo.toml`:

```toml
[dependencies]
aio = { path = "aio" } # Or from crates.io if published
```

### Example: A Simple TCP Echo Server

Here is a simple TCP server that accepts connections and prints whatever data it receives.

```rust
// src/main.rs
use aio::{AsyncTcpListener, AsyncTcpStream, spawn, AsyncReadRent};

#[aio::main]
pub async fn main() {
    let listener = AsyncTcpListener::bind("127.0.0.1:8080").unwrap();
    println!("Listening on 127.0.0.1:8080");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                println!("Accepted a new connection");
                spawn(handle_connection(stream));
            }
            Err(e) => {
                eprintln!("Failed to accept connection: {e}");
                break;
            }
        }
    }
}

async fn handle_connection(stream: AsyncTcpStream) {
    let mut buf = vec![0; 1024];
    loop {
        let (n, new_buf) = match socket.read(buf).await {
            Ok((0, _)) => break, 
            Ok((n, buf)) => (n, buf),
            Err(_) => break,
        };
        buf = new_buf;
                
        if socket.write_all(buf[..n].to_vec()).await.is_err() {
            break;
        }
    }
}
```

The `#[aio::main]` macro sets up and runs the `io_uring` proactor, and `aio::spawn` is used to launch a new asynchronous task.

## Running Tests

The project includes a suite of tests to verify functionality. You can run them using:

```bash
cargo test
```

This will run unit tests and integration tests, including TCP and UDP echo tests.

## Running Benchmarks

A benchmark suite is included to compare the performance of this runtime against Tokio. To run the benchmarks:

```bash
cargo bench
```

## Project Structure

The workspace is divided into three main crates:

*   [`uring-rt`](./uring-rt/): The core of the runtime. It contains the `Proactor`, the `io_uring` `Poller`, and the low-level async I/O future implementations.
*   [`aio-macros`](./aio-macros/): Provides the procedural macros (`#[aio::main]`, `#[aio::test]`) for improved ergonomics.
*   [`aio`](./aio/): The public-facing crate that brings together the runtime and macros into a single, easy-to-use library.


## Roadmap


This project is still in its early, experimental stages. The future roadmap includes:

### Functionality support
- [ ] **Async File I/O**: Implement `aio::fs::File` for asynchronous file operations.
- [ ] **Timer Support**: Add `aio::time::sleep` and other time-related utilities.
- [ ] **More `io_uring` Operations**: Integrate support for more advanced `io_uring` opcodes like `sendmsg`/`recvmsg`, `splice`, and `tee`.
- [ ] **Synchronization Primitives**: Introduce task synchronization primitives like `Mutex`, `RwLock`, and `Semaphore` tailored for the thread-per-core model.

### Library Usage
- [ ] **More Ergonomic Apis**: Add or refactor to provide more ergonomic apis to user.
- [ ] **Enhanced Documentation**: Write more in-depth documentation and provide a wider range of examples.
- [ ] **Publish to Crates.io**: Prepare the library for an initial public release.
- [ ] **Add more Useful Tests**: Add tests to enable the safety to use.
### Performance Improve
- [ ] **Performance Tuning**: Continuously benchmark and optimize the runtime, scheduler, and I/O submission logic.


## Problems to be solved

- Implement all possible data types for `IoBuf` and `IoBufMut` trait and related reference types, maybe need to refactor
- Split code of `proactor` into `worker` and `runtime` to provide easy-to-use multiple runtime instances apis
- Use flamegraph or perf to gain insights about the most expensive part and improve


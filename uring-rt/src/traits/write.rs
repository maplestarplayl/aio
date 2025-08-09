use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::net::TcpStreamWriteFuture;
use crate::traits::IoBuf;
use crate::{AsyncTcpStream, BufResult};

/// A trait for objects that can be written to asynchronously.
pub trait AsyncWriteRent {
    /// The future returned by the `write` method.
    type WriteFut<'a, B>: Future<Output = BufResult<usize, B>>
    where
        Self: 'a,
        B: 'a + IoBuf;

    /// Write data from the buffer into this writer.
    fn write<B: IoBuf>(&self, buf: B) -> Self::WriteFut<'_, B>;
}

impl AsyncWriteRent for AsyncTcpStream {
    type WriteFut<'a, B>
        = TcpStreamWriteFuture<'a, B>
    where
        Self: 'a,
        B: 'a + IoBuf;

    fn write<B: IoBuf>(&self, buf: B) -> Self::WriteFut<'_, B> {
        TcpStreamWriteFuture::new(self, buf)
    }
}

/// A helper struct to represent a slice of an owned buffer.
pub struct Slice<T> {
    inner: T,
    start: usize,
    end: usize,
}

impl<T> Slice<T> {
    fn new(inner: T, start: usize, end: usize) -> Self {
        assert!(start <= end);
        Self { inner, start, end }
    }

    fn into_inner(self) -> T {
        self.inner
    }
}

// SAFETY: The pointer of the inner buffer is valid, and we are just offsetting it.
// The lifetime of the slice is tied to the lifetime of the inner buffer.
unsafe impl<T: IoBuf> IoBuf for Slice<T> {
    fn stable_ptr(&self) -> *const u8 {
        unsafe { self.inner.stable_ptr().add(self.start) }
    }

    fn bytes_init(&self) -> usize {
        self.end - self.start
    }
}

/// An extension trait for `AsyncWriteRent` that provides the `write_all` method.
pub trait AsyncWriteRentExt<B: 'static> {
    /// The future returned by `write_all`.
    type Future<'a>: Future<Output = BufResult<usize, B>>
    where
        Self: 'a,
        B: 'a;

    /// Write the entire buffer to the writer.
    fn write_all(&self, buf: B) -> <Self as AsyncWriteRentExt<B>>::Future<'_>;
}

impl<A: AsyncWriteRent + ?Sized, B: 'static + IoBuf> AsyncWriteRentExt<B> for A {
    type Future<'a>
        = Pin<Box<dyn Future<Output = BufResult<usize, B>> + 'a>>
    where
        Self: 'a,
        B: 'a;

    fn write_all(&self, mut buf: B) -> Self::Future<'_> {
        Box::pin(async move {
            let len = buf.bytes_init();
            let mut written = 0;
            while written < len {
                let slice = Slice::new(buf, written, len);
                let result = self.write(slice).await;

                match result {
                    Ok((0, slice)) => {
                        let err = io::Error::new(
                            io::ErrorKind::WriteZero,
                            "failed to write whole buffer",
                        );
                        return Err((err, slice.into_inner()));
                    }
                    Ok((n, slice)) => {
                        written += n;
                        buf = slice.into_inner();
                    }
                    Err((err, slice)) => {
                        return Err((err, slice.into_inner()));
                    }
                }
            }
            Ok((written, buf))
        })
    }
}

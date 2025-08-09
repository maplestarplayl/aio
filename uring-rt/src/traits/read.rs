use std::io;
use std::pin::Pin;

use crate::net::TcpStreamReadFuture;
use crate::{AsyncTcpStream, IoBufMut};

pub type BufResult<T, B> = Result<(T, B), (io::Error, B)>;

pub trait AsyncReadRent {
    type ReadFut<'a, B>: Future<Output = BufResult<usize, B>>
    where
        Self: 'a,
        B: 'a + IoBufMut;

    fn read<B: IoBufMut>(&mut self, buf: B) -> Self::ReadFut<'_, B>;
}

impl AsyncReadRent for AsyncTcpStream {
    type ReadFut<'a, B>
        = TcpStreamReadFuture<'a, B>
    where
        Self: 'a,
        B: 'a + IoBufMut;

    fn read<B: IoBufMut>(&mut self, buf: B) -> Self::ReadFut<'_, B> {
        TcpStreamReadFuture::new(self, buf)
    }
}

pub struct Slice<T> {
    inner: T,
    start: usize,
    end: usize,
}

impl<T> Slice<T> {
    /// Creates a new `Slice`.
    fn new(inner: T, start: usize, end: usize) -> Self {
        assert!(start <= end);
        Self { inner, start, end }
    }

    /// Consumes the `Slice`, returning the underlying buffer.
    fn into_inner(self) -> T {
        self.inner
    }
}

unsafe impl<T: IoBufMut> IoBufMut for Slice<T> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        // SAFETY: The pointer of the inner buffer is valid, and we are just offsetting it.
        unsafe { self.inner.stable_mut_ptr().add(self.start) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: See `Vec<u8>` impl. This creates a slice of potentially uninitialized memory.
        unsafe { std::slice::from_raw_parts_mut(self.stable_mut_ptr(), self.end - self.start) }
    }

    fn bytes_total(&self) -> usize {
        self.end - self.start
    }

    unsafe fn set_init(&mut self, len: usize) {
        // SAFETY: The caller guarantees that `len` bytes have been initialized in this slice.
        // We update the inner buffer's initialized length. This assumes that the bytes
        // from 0 to `self.start` were already initialized, which `read_exact` ensures.
        debug_assert!(len <= self.bytes_total());
        unsafe {
            self.inner.set_init(self.start + len);
        }
    }
}

pub trait AsyncReadRentExt<B: 'static> {
    /// The future of Result<size, buffer>
    type Future<'a>: Future<Output = BufResult<usize, B>>
    where
        Self: 'a,
        B: 'a;

    /// Read until buf capacity is fulfilled
    fn read_exact(&mut self, buf: B) -> <Self as AsyncReadRentExt<B>>::Future<'_>;
}

impl<A: AsyncReadRent, B: 'static + IoBufMut> AsyncReadRentExt<B> for A {
    type Future<'a>
        = Pin<Box<dyn Future<Output = BufResult<usize, B>> + 'a>>
    where
        Self: 'a,
        B: 'a;
    fn read_exact(&mut self, mut buf: B) -> Self::Future<'_> {
        Box::pin(async move {
            let len = buf.bytes_total();
            let mut read = 0;
            while read < len {
                // Create a slice of the buffer for the next read operation.
                let slice = Slice::new(buf, read, len);

                // Await the read operation on the slice.
                let result = self.read(slice).await;

                match result {
                    Ok((0, slice)) => {
                        // Reached EOF before filling the buffer.
                        let err = io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "failed to fill whole buffer",
                        );
                        return Err((err, slice.into_inner()));
                    }
                    Ok((n, slice)) => {
                        read += n;
                        buf = slice.into_inner();
                    }
                    Err((err, slice)) => {
                        return Err((err, slice.into_inner()));
                    }
                }
            }
            Ok((read, buf))
        })
    }
}

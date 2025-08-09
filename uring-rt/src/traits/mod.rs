mod read;
mod write;

pub use read::{AsyncReadRent, AsyncReadRentExt};
pub use write::{AsyncWriteRent, AsyncWriteRentExt};

pub type BufResult<T, B> = Result<(T, B), (std::io::Error, B)>;

/// A trait for immutable buffers.
///
/// # Safety
///
/// The pointer returned by `stable_ptr` must be valid for reads of `bytes_init`
/// bytes and must not be moved or invalidated until the operation is complete.
pub unsafe trait IoBuf: 'static {
    /// Returns a pointer to the buffer.
    fn stable_ptr(&self) -> *const u8;
    /// Returns the number of initialized bytes in the buffer.
    fn bytes_init(&self) -> usize;
}

// SAFETY: The user of this implementation must ensure that the `Vec<u8>` is not
// moved or reallocated while an async operation is pending.
// Since the `write` operation takes ownership of the buffer, this is generally safe.
unsafe impl IoBuf for Vec<u8> {
    // TODO: how to record the need-to-write length
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn bytes_init(&self) -> usize {
        self.len()
    }
}

unsafe impl<const N: usize> IoBuf for &'static [u8; N] {
    #[inline]
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    #[inline]
    fn bytes_init(&self) -> usize {
        self.len()
    }
}

unsafe impl IoBuf for &'static [u8] {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn bytes_init(&self) -> usize {
        self.len()
    }
}

/// A trait for mutable buffers.
///
/// # Safety
///
/// The pointer returned by `stable_mut_ptr` must be valid for writes of `bytes_total`
/// bytes and must not be moved or invalidated until the operation is complete.
pub unsafe trait IoBufMut: 'static {
    /// Returns a mutable pointer to the buffer.
    fn stable_mut_ptr(&mut self) -> *mut u8;
    fn as_mut_slice(&mut self) -> &mut [u8];
    /// Returns the total capacity of the buffer.
    fn bytes_total(&self) -> usize;
    /// Sets the initialized length of the buffer.
    /// # Safety
    /// The caller must ensure that `len` bytes of the buffer have been initialized.
    unsafe fn set_init(&mut self, len: usize);
}

// SAFETY: The user of this implementation must ensure that the `Vec<u8>` is not used
// in a way that would cause it to reallocate while an async operation is pending.
// Since the `read` operation takes ownership of the buffer, this is generally safe.
unsafe impl IoBufMut for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: This is unsafe because it creates a slice of potentially
        // uninitialized memory. The `AsyncReadRent::read` operation is expected
        // to write into this slice, and `set_init` will be called to update the
        // vector's length to reflect the number of bytes written.
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), self.capacity()) }
    }

    fn bytes_total(&self) -> usize {
        self.capacity()
    }

    unsafe fn set_init(&mut self, len: usize) {
        // SAFETY: The caller of this method must guarantee that `len` bytes
        // of the buffer have been initialized.
        debug_assert!(len <= self.capacity());
        unsafe {
            self.set_len(len);
        }
    }
}

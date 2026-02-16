//! Kernel pipe objects for inter-process and shell pipeline communication.
//!
//! Provides a unidirectional byte stream between a writer and a reader.
//! Used by the shell's `|` operator and the `pipe` syscall.

#![allow(dead_code)]

use alloc::{collections::VecDeque, sync::Arc, vec::Vec};

use spin::Mutex;

use crate::error::KernelError;

/// Default pipe capacity (64 KB).
const PIPE_CAPACITY: usize = 64 * 1024;

/// Internal shared state of a pipe.
struct PipeInner {
    /// Data buffer.
    buffer: VecDeque<u8>,
    /// Maximum capacity in bytes.
    capacity: usize,
    /// True when the write end has been closed.
    write_closed: bool,
    /// True when the read end has been closed.
    read_closed: bool,
}

impl PipeInner {
    fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            write_closed: false,
            read_closed: false,
        }
    }
}

/// A handle to the shared pipe state.
type PipeState = Arc<Mutex<PipeInner>>;

/// The read end of a kernel pipe.
pub struct PipeReader {
    inner: PipeState,
}

/// The write end of a kernel pipe.
pub struct PipeWriter {
    inner: PipeState,
}

/// Create a new pipe pair `(reader, writer)`.
pub fn create_pipe() -> Result<(PipeReader, PipeWriter), KernelError> {
    create_pipe_with_capacity(PIPE_CAPACITY)
}

/// Create a pipe pair with a custom capacity.
pub fn create_pipe_with_capacity(capacity: usize) -> Result<(PipeReader, PipeWriter), KernelError> {
    let inner = Arc::new(Mutex::new(PipeInner::new(capacity)));
    Ok((
        PipeReader {
            inner: inner.clone(),
        },
        PipeWriter { inner },
    ))
}

impl PipeReader {
    /// Read up to `buf.len()` bytes from the pipe.
    ///
    /// Returns the number of bytes read. Returns 0 when the write end is
    /// closed and the buffer is empty (EOF). Spins briefly if the buffer is
    /// empty but the write end is still open.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        loop {
            {
                let mut pipe = self.inner.lock();
                if !pipe.buffer.is_empty() {
                    let to_read = buf.len().min(pipe.buffer.len());
                    for byte in buf.iter_mut().take(to_read) {
                        *byte = pipe.buffer.pop_front().unwrap_or(0);
                    }
                    return Ok(to_read);
                }
                if pipe.write_closed {
                    return Ok(0); // EOF
                }
                if pipe.read_closed {
                    return Ok(0);
                }
            }
            // Buffer empty, write end still open — spin wait
            core::hint::spin_loop();
        }
    }

    /// Non-blocking read: return immediately if no data available.
    pub fn try_read(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        let mut pipe = self.inner.lock();
        if pipe.buffer.is_empty() {
            if pipe.write_closed {
                return Ok(0); // EOF
            }
            return Err(KernelError::WouldBlock);
        }
        let to_read = buf.len().min(pipe.buffer.len());
        for byte in buf.iter_mut().take(to_read) {
            *byte = pipe.buffer.pop_front().unwrap_or(0);
        }
        Ok(to_read)
    }

    /// Close the read end.
    pub fn close(&self) {
        self.inner.lock().read_closed = true;
    }

    /// Check if there is data available to read.
    pub fn has_data(&self) -> bool {
        !self.inner.lock().buffer.is_empty()
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.close();
    }
}

impl PipeWriter {
    /// Write data to the pipe.
    ///
    /// Returns the number of bytes written. Returns an error if the read
    /// end has been closed (broken pipe).
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        let mut pipe = self.inner.lock();
        if pipe.read_closed {
            return Err(KernelError::BrokenPipe);
        }
        if pipe.write_closed {
            return Err(KernelError::BrokenPipe);
        }
        let available = pipe.capacity.saturating_sub(pipe.buffer.len());
        let to_write = data.len().min(available);
        for &byte in &data[..to_write] {
            pipe.buffer.push_back(byte);
        }
        Ok(to_write)
    }

    /// Write all data, blocking until complete.
    pub fn write_all(&self, data: &[u8]) -> Result<(), KernelError> {
        let mut offset = 0;
        while offset < data.len() {
            let written = self.write(&data[offset..])?;
            if written == 0 {
                core::hint::spin_loop();
            }
            offset += written;
        }
        Ok(())
    }

    /// Close the write end.
    pub fn close(&self) {
        self.inner.lock().write_closed = true;
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.close();
    }
}

/// Capture all output written to a pipe writer and return it as bytes.
///
/// This is a helper for the shell to capture command output for piping
/// and command substitution. The writer should already be closed.
pub fn drain_pipe(reader: &PipeReader) -> Vec<u8> {
    let mut result = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.try_read(&mut buf) {
            Ok(0) => break,
            Ok(n) => result.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipe_basic_read_write() {
        let (reader, writer) = create_pipe().unwrap();
        writer.write(b"hello").unwrap();
        writer.close();
        let mut buf = [0u8; 16];
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn test_pipe_eof_after_close() {
        let (reader, writer) = create_pipe().unwrap();
        writer.close();
        let mut buf = [0u8; 16];
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_pipe_broken_pipe() {
        let (reader, writer) = create_pipe().unwrap();
        reader.close();
        let result = writer.write(b"data");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipe_large_write() {
        let (reader, writer) = create_pipe_with_capacity(16).unwrap();
        // Write more than capacity
        let n = writer.write(b"this is a long string").unwrap();
        assert_eq!(n, 16); // Only capacity bytes written
        writer.close();
        let mut buf = [0u8; 32];
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(n, 16);
    }

    #[test]
    fn test_drain_pipe() {
        let (reader, writer) = create_pipe().unwrap();
        writer.write(b"hello ").unwrap();
        writer.write(b"world").unwrap();
        writer.close();
        let data = drain_pipe(&reader);
        assert_eq!(&data, b"hello world");
    }
}

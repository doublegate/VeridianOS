//! Buffered output writer.
//!
//! Provides a simple buffered output abstraction over raw `write` syscalls
//! on file descriptors 1 (stdout) and 2 (stderr).

use core::fmt;

use crate::syscall;

/// Buffered writer for a file descriptor.
pub struct Writer {
    fd: i32,
}

impl Writer {
    /// Create a writer for stdout (fd 1).
    pub fn stdout() -> Self {
        Self { fd: 1 }
    }

    /// Create a writer for stderr (fd 2).
    pub fn stderr() -> Self {
        Self { fd: 2 }
    }

    /// Create a writer for an arbitrary file descriptor.
    #[allow(dead_code)]
    pub fn new(fd: i32) -> Self {
        Self { fd }
    }

    /// Write raw bytes to the file descriptor.
    pub fn write_bytes(&self, buf: &[u8]) {
        let mut written = 0;
        while written < buf.len() {
            let n = syscall::sys_write(self.fd, &buf[written..]);
            if n <= 0 {
                break;
            }
            written += n as usize;
        }
    }

    /// Write a string slice.
    pub fn write_str(&self, s: &str) {
        self.write_bytes(s.as_bytes());
    }

    /// Write a string slice followed by a newline.
    #[allow(dead_code)] // Public API for output
    pub fn write_line(&self, s: &str) {
        self.write_str(s);
        self.write_bytes(b"\n");
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_bytes(s.as_bytes());
        Ok(())
    }
}

/// Print a formatted string to stdout.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut w = $crate::output::Writer::stdout();
        let _ = core::write!(w, $($arg)*);
    }};
}

/// Print a formatted string to stdout followed by a newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut w = $crate::output::Writer::stdout();
        let _ = core::write!(w, $($arg)*);
        w.write_str("\n");
    }};
}

/// Print a formatted string to stderr.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut w = $crate::output::Writer::stderr();
        let _ = core::write!(w, $($arg)*);
    }};
}

/// Print a formatted string to stderr followed by a newline.
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut w = $crate::output::Writer::stderr();
        let _ = core::write!(w, $($arg)*);
        w.write_str("\n");
    }};
}

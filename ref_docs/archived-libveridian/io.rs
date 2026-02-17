//! I/O functionality for user-space programs

use crate::sys;
use core::fmt;

const STDOUT: usize = 1;
const STDERR: usize = 2;

pub struct Writer {
    fd: usize,
}

impl Writer {
    pub const fn stdout() -> Self {
        Writer { fd: STDOUT }
    }
    
    pub const fn stderr() -> Self {
        Writer { fd: STDERR }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        sys::write(self.fd, s.as_bytes()).map_err(|_| fmt::Error)?;
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        use core::fmt::Write;
        let mut writer = $crate::io::Writer::stdout();
        write!(writer, $($arg)*).unwrap();
    });
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ({
        $crate::print!("{}\n", format_args!($($arg)*));
    });
}
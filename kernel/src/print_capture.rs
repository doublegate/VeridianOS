//! Output capture buffer for redirecting `println!` output.
//!
//! When capture mode is active, `println!` output is appended to a global
//! String buffer in addition to serial/framebuffer.  The GUI terminal uses
//! this to display command output.

use alloc::string::String;
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use spin::Mutex;

/// Whether capture mode is active.
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// The capture buffer (protected by a spinlock).
static CAPTURE_BUF: Mutex<Option<String>> = Mutex::new(None);

/// Start capturing `println!` output.
pub fn start_capture() {
    let mut buf = CAPTURE_BUF.lock();
    *buf = Some(String::new());
    CAPTURING.store(true, Ordering::Release);
}

/// Stop capturing and return the captured output.
pub fn stop_capture() -> String {
    CAPTURING.store(false, Ordering::Release);
    let mut buf = CAPTURE_BUF.lock();
    buf.take().unwrap_or_default()
}

/// Called by the `print!` macro.  Appends to the capture buffer if active.
pub fn _capture_print(args: fmt::Arguments) {
    if CAPTURING.load(Ordering::Acquire) {
        use core::fmt::Write;
        let mut buf = CAPTURE_BUF.lock();
        if let Some(ref mut s) = *buf {
            let _ = s.write_fmt(args);
        }
    }
}

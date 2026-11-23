//! The kernel log ring (Linux's printk buffer) and the console log level
//! (ADR 0027, item 5 decisions): what syslog(2) reads.
//!
//! Every line the kernel prints is kept, in syslog's text form
//! (`<4>[    1.234567] text`: Linux's default message level, then the
//! time since boot), in a 128 KiB ring that needs no heap, so it works from
//! the first print of the boot. The print path never waits for the ring:
//! a print that finds it busy (an interrupt printing over a print) drops
//! its text from the ring and still reaches the console.
//!
//! The console shows a message only below `console_loglevel` (default 7),
//! as Linux, so syslog's CONSOLE_OFF and CONSOLE_LEVEL quiet it.

use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicU8, Ordering},
};

use spin::Mutex;

/// The ring's size (CONFIG_LOG_BUF_SHIFT 17).
pub const LOG_BUF_LEN: usize = 1 << 17;

/// The level of a message printed without one (default_message_loglevel,
/// KERN_WARNING).
pub const DEFAULT_MESSAGE_LEVEL: u8 = 4;
/// The lowest console level syslog may set (minimum_console_loglevel).
pub const MINIMUM_CONSOLE_LEVEL: u8 = 1;
/// The console level at boot (default_console_loglevel).
pub const DEFAULT_CONSOLE_LEVEL: u8 = 7;

/// Messages below this level reach the console (console_loglevel).
static CONSOLE_LEVEL: AtomicU8 = AtomicU8::new(DEFAULT_CONSOLE_LEVEL);
/// The level CONSOLE_OFF saved, 0 when none (saved_console_loglevel).
static SAVED_CONSOLE_LEVEL: AtomicU8 = AtomicU8::new(0);

/// Whether a message of the default level reaches the console now.
pub fn console_enabled() -> bool {
    DEFAULT_MESSAGE_LEVEL < CONSOLE_LEVEL.load(Ordering::Relaxed)
}

/// `/proc/sys/kernel/printk`'s four values.
pub fn levels() -> [u8; 4] {
    [
        CONSOLE_LEVEL.load(Ordering::Relaxed),
        DEFAULT_MESSAGE_LEVEL,
        MINIMUM_CONSOLE_LEVEL,
        DEFAULT_CONSOLE_LEVEL,
    ]
}

/// Set the console level (`/proc/sys/kernel/printk`'s first value, or
/// syslog's CONSOLE_LEVEL): below the minimum the minimum.
pub fn set_console_level(level: u8) {
    CONSOLE_LEVEL.store(level.max(MINIMUM_CONSOLE_LEVEL), Ordering::Relaxed);
}

/// The console takes everything from now on (Linux's console_verbose,
/// CONSOLE_LOGLEVEL_MOTORMOUTH): a panic, a fatal fault or a hang report
/// is shown whatever level a program set.
pub fn console_verbose() {
    CONSOLE_LEVEL.store(15, Ordering::Relaxed);
}

/// Held across each change of the console level that reads it first
/// (CONSOLE_OFF, CONSOLE_ON, CONSOLE_LEVEL), so two at once cannot save the
/// level the other just lowered (PR #49 review; Linux's do_syslog does
/// these without a lock). Readers take only the atomic.
static CONSOLE_STATE: spin::Mutex<()> = spin::Mutex::new(());

/// syslog CONSOLE_OFF: the console takes only the minimum level.
pub fn console_off() {
    let _state = CONSOLE_STATE.lock();
    if SAVED_CONSOLE_LEVEL.load(Ordering::Relaxed) == 0 {
        SAVED_CONSOLE_LEVEL.store(CONSOLE_LEVEL.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    CONSOLE_LEVEL.store(MINIMUM_CONSOLE_LEVEL, Ordering::Relaxed);
}

/// syslog CONSOLE_ON: the level CONSOLE_OFF saved, if it saved one.
pub fn console_on() {
    let _state = CONSOLE_STATE.lock();
    let saved = SAVED_CONSOLE_LEVEL.swap(0, Ordering::Relaxed);
    if saved != 0 {
        CONSOLE_LEVEL.store(saved, Ordering::Relaxed);
    }
}

/// syslog CONSOLE_LEVEL (1 to 8; the caller checked): set, and nothing
/// saved any more.
pub fn console_level(level: u8) {
    let _state = CONSOLE_STATE.lock();
    set_console_level(level);
    SAVED_CONSOLE_LEVEL.store(0, Ordering::Relaxed);
}

/// The ring: bytes by absolute position (`head` is how many were ever
/// written), only the last LOG_BUF_LEN still there.
pub struct Ring {
    buf: [u8; LOG_BUF_LEN],
    head: u64,
    /// Where syslog READ goes on from (syslog_seq).
    syslog_pos: u64,
    /// Where READ_ALL starts, moved by CLEAR (clear_seq).
    clear_pos: u64,
    /// The next byte begins a line (its prefix not written yet).
    line_start: bool,
}

static RING: Mutex<Ring> = Mutex::new(Ring::new());

impl Ring {
    const fn new() -> Self {
        Self {
            buf: [0; LOG_BUF_LEN],
            head: 0,
            syslog_pos: 0,
            clear_pos: 0,
            line_start: true,
        }
    }

    fn push(&mut self, b: u8) {
        self.buf[(self.head % LOG_BUF_LEN as u64) as usize] = b;
        self.head += 1;
    }

    fn push_all(&mut self, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            self.push(bytes[i]);
            i += 1;
        }
    }

    fn byte(&self, pos: u64) -> u8 {
        self.buf[(pos % LOG_BUF_LEN as u64) as usize]
    }

    /// The oldest position still in the ring, at a line's start.
    fn oldest(&self) -> u64 {
        let first = self.head.saturating_sub(LOG_BUF_LEN as u64);
        if first == 0 {
            return 0;
        }
        // The byte before `first` is overwritten, so whether `first` begins
        // a line is unknown: the first line start provable is after the
        // first newline from it (Linux likewise drops a partial record).
        let mut p = first;
        while p < self.head {
            if self.byte(p) == b'\n' {
                return p + 1;
            }
            p += 1;
        }
        self.head
    }

    /// The start of the first line at or after `pos` (`head` if none).
    fn next_line(&self, pos: u64) -> u64 {
        if pos == 0 || pos >= self.head || self.byte(pos - 1) == b'\n' {
            return pos;
        }
        let mut p = pos;
        while p < self.head {
            if self.byte(p) == b'\n' {
                return p + 1;
            }
            p += 1;
        }
        self.head
    }

    fn copy(&self, from: u64, to: u64, out: &mut [u8]) -> usize {
        let n = ((to - from) as usize).min(out.len());
        let mut i = 0;
        while i < n {
            out[i] = self.byte(from + i as u64);
            i += 1;
        }
        n
    }

    /// The prefix of a line begun now.
    fn prefix(&mut self) {
        let ns = crate::timer::monotonic_ns();
        let mut tmp = Prefix {
            buf: [0; 32],
            len: 0,
        };
        let _ = write!(
            tmp,
            "<{}>[{:5}.{:06}] ",
            DEFAULT_MESSAGE_LEVEL,
            ns / 1_000_000_000,
            ns % 1_000_000_000 / 1000
        );
        let (b, n) = (tmp.buf, tmp.len);
        self.push_all(&b[..n]);
    }
}

/// A small stack buffer for the prefix.
struct Prefix {
    buf: [u8; 32],
    len: usize,
}

impl Write for Prefix {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let b = s.as_bytes();
        let n = b.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
        self.len += n;
        Ok(())
    }
}

impl Write for Ring {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if self.line_start {
                self.prefix();
                self.line_start = false;
            }
            self.push(b[i]);
            if b[i] == b'\n' {
                self.line_start = true;
            }
            i += 1;
        }
        Ok(())
    }
}

/// Keep what the kernel prints (the `print!` macros call this). Never
/// waits: a busy ring loses the text, not the console.
pub fn record(args: fmt::Arguments) {
    if let Some(mut r) = RING.try_lock() {
        let _ = r.write_fmt(args);
    }
}

/// [`record`] of a plain string, with no formatting (AArch64's literal
/// prints, which keep away from `core::fmt`).
pub fn record_str(s: &str) {
    if let Some(mut r) = RING.try_lock() {
        let _ = r.write_str(s);
    }
}

/// syslog SIZE_BUFFER.
pub fn size_buffer() -> usize {
    LOG_BUF_LEN
}

/// syslog SIZE_UNREAD: the bytes READ has not taken yet.
pub fn size_unread() -> usize {
    let r = RING.lock();
    let from = r.syslog_pos.max(r.oldest());
    (r.head - from) as usize
}

/// syslog READ: take up to `out.len()` unread bytes; 0 when there are none
/// (the caller waits for some).
pub fn read(out: &mut [u8]) -> usize {
    let mut r = RING.lock();
    let from = r.syslog_pos.max(r.oldest());
    let n = r.copy(from, r.head, out);
    r.syslog_pos = from + n as u64;
    n
}

/// syslog READ_ALL and READ_CLEAR: the newest whole lines since the last
/// CLEAR that fit in `out`, and where they end, for READ_CLEAR to commit
/// with [`clear_to`] once they reached the caller (PR #49 review).
pub fn read_all(out: &mut [u8]) -> (usize, u64) {
    let r = RING.lock();
    let mut from = r.clear_pos.max(r.oldest());
    while r.head - from > out.len() as u64 {
        let next = r.next_line(from + 1);
        if next >= r.head {
            from = r.head;
            break;
        }
        from = next;
    }
    let n = r.copy(from, r.head, out);
    (n, r.head)
}

/// READ_CLEAR's commit: what a read up to `end` returned is cleared, and
/// nothing written since, as Linux's syslog_print_all clears to the
/// sequence it read.
pub fn clear_to(end: u64) {
    let mut r = RING.lock();
    if end > r.clear_pos {
        r.clear_pos = end;
    }
}

/// syslog CLEAR.
pub fn clear() {
    let mut r = RING.lock();
    r.clear_pos = r.head;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lowered console level silences ordinary prints; a panic's
    /// console_verbose shows everything again.
    #[test]
    fn console_level_and_verbose() {
        console_level(1);
        assert!(!console_enabled());
        console_verbose();
        assert!(console_enabled());
        console_level(DEFAULT_CONSOLE_LEVEL);
        assert!(console_enabled());
    }

    #[test]
    fn lines_are_kept_with_their_prefix_and_read_once() {
        let mut r = Ring::new();
        let _ = write!(r, "one\ntwo\n");
        assert_eq!(r.head as usize, 2 * "<4>[    0.000000] ".len() + 8);
        let mut out = [0u8; 256];
        let n = r.copy(0, r.head, &mut out);
        let text = core::str::from_utf8(&out[..n]).unwrap();
        assert!(
            text.starts_with("<4>[") && text.contains("] one\n<4>[") && text.ends_with("] two\n")
        );
        // Wrapping keeps whole lines from the oldest one left.
        let mut w = Ring::new();
        let line = "x".repeat(1000);
        for _ in 0..200 {
            let _ = writeln!(w, "{line}");
        }
        let o = w.oldest();
        assert!(o > 0 && w.byte(o) == b'<');
    }
}

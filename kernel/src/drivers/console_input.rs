//! Serial console input, in one place (N-118, console part).
//!
//! Four readers used to poll the UART themselves: the `/dev/console` node,
//! the stdin fallback of `read(2)`, the loader's stdin node and the input
//! multiplexer the kernel shell uses. Each busy-waited on the line status
//! register, and the kernel is not preemptible, so a process blocked reading
//! the console kept every other task off the CPU; two of them could also
//! split a line's bytes between them.
//!
//! Now received bytes go into one ring buffer. On x86_64 the timer tick
//! drains the UART into it (serial RX is not interrupt driven yet) and wakes
//! [`CONSOLE_INPUT`] and the poll/epoll waiters; every reader also drains it
//! before looking, so input works before the dispatcher runs and on
//! architectures without a tick hook. A blocking reader sleeps on the queue
//! instead of spinning.

use spin::Mutex;

/// Bytes kept while nobody reads. More are dropped (the UART would have
/// overrun anyway).
const CAPACITY: usize = 4096;

struct Ring {
    buf: [u8; CAPACITY],
    head: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            buf: [0; CAPACITY],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.len == CAPACITY {
            return false;
        }
        self.buf[(self.head + self.len) % CAPACITY] = byte;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.buf[self.head];
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(byte)
    }
}

static RING: Mutex<Ring> = Mutex::new(Ring::new());

/// Readers waiting for console input; woken whenever bytes arrive.
#[cfg(feature = "alloc")]
pub static CONSOLE_INPUT: crate::sched::dispatch::WaitQueue =
    crate::sched::dispatch::WaitQueue::new();

/// Run `f` on the ring with interrupts off: the timer tick takes the lock
/// too, so holding it with interrupts on could deadlock this CPU.
fn with_ring<R>(f: impl FnOnce(&mut Ring) -> R) -> R {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        x86_64::instructions::interrupts::without_interrupts(|| f(&mut RING.lock()))
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        f(&mut RING.lock())
    }
}

/// Move whatever the UART holds into the ring. Returns whether any byte
/// arrived.
pub fn poll_hardware() -> bool {
    let mut any = false;
    // At most a FIFO's worth per call, so a stuck "data ready" bit cannot
    // keep the caller here.
    for _ in 0..64 {
        // The status check, the data read and the push happen with
        // interrupts off: if the timer tick took the byte between a
        // reader's status check and its data read, the reader would read an
        // empty data register and a typed byte would be lost.
        let got = with_ring(|r| match super::input::read_uart_raw() {
            Some(byte) => {
                r.push(byte);
                true
            }
            None => false,
        });
        if !got {
            break;
        }
        any = true;
    }
    any
}

/// Input arrived but not every waiter could be woken yet (a lock was held
/// by the interrupted code); retried on the next tick.
static WAKE_PENDING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// From the timer tick (interrupt context): drain the UART and wake
/// readers if input arrived. Wakes only try locks, so an interrupted lock
/// holder is never waited for; what is missed is retried next tick.
#[cfg(feature = "alloc")]
pub fn tick() {
    use core::sync::atomic::Ordering;
    if poll_hardware() {
        WAKE_PENDING.store(true, Ordering::Relaxed);
    }
    if WAKE_PENDING.load(Ordering::Relaxed) {
        let io = crate::sched::dispatch::io_event_from_irq();
        let console = CONSOLE_INPUT.wake_all_from_irq();
        if io && console {
            WAKE_PENDING.store(false, Ordering::Relaxed);
        }
    }
}

/// Whether a byte is waiting.
pub fn has_input() -> bool {
    poll_hardware();
    with_ring(|r| r.len > 0)
}

/// The next byte, if one is waiting (non-blocking).
pub fn try_read() -> Option<u8> {
    poll_hardware();
    with_ring(|r| r.pop())
}

/// The next byte, sleeping until one arrives, a signal must be acted on
/// (`Interrupted`) or `deadline` (`monotonic_ns`) passes (`TimedOut`).
#[cfg(feature = "alloc")]
pub fn read_wait(deadline: Option<u64>) -> Result<u8, crate::sched::dispatch::WaitError> {
    loop {
        if let Some(byte) = try_read() {
            return Ok(byte);
        }
        crate::sched::dispatch::wait_event(&CONSOLE_INPUT, deadline, has_input)?;
    }
}

// ── The console's line discipline ──────────────────────────────────────
//
// Shared by every reader of the console (one terminal): a line typed in
// canonical mode is assembled here across reads, so a reader that has to
// wait keeps what was typed so far. Reads never sleep here; they return
// `WouldBlock` and the caller waits (the read system call through
// `wait_ready`, with EINTR and O_NONBLOCK handled there).

const POLLIN: u16 = 0x0001;
const POLLOUT: u16 = 0x0004;
const EOF_CHAR: u8 = 4; // ^D

struct Line {
    buf: [u8; CAPACITY],
    len: usize,
    /// A complete line (or ^D) is waiting to be read out.
    complete: bool,
}

static LINE: Mutex<Line> = Mutex::new(Line {
    buf: [0; CAPACITY],
    len: 0,
    complete: false,
});

/// Raw mode with VMIN = 0, VTIME > 0: when the current timed read gives up
/// (`monotonic_ns`; 0 when none is running).
static VTIME_DEADLINE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn echo_bytes(bytes: &[u8]) {
    for &b in bytes {
        crate::print!("{}", b as char);
    }
}

fn now_ns() -> u64 {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::clock_ns()
    }
    #[cfg(not(feature = "alloc"))]
    {
        0
    }
}

/// Read from the console as a terminal does: in canonical mode a whole
/// line (erase, CR to NL, ^D ends the read), otherwise what has arrived,
/// following VMIN/VTIME. With nothing to return yet: `WouldBlock`.
pub fn read_tty(out: &mut [u8]) -> Result<usize, crate::error::KernelError> {
    use crate::drivers::terminal;
    if out.is_empty() {
        return Ok(0);
    }
    let echo = terminal::is_echo_enabled();
    if terminal::is_canonical_mode() {
        let verase = terminal::get_verase();
        let mut line = LINE.lock();
        while !line.complete {
            let Some(byte) = try_read() else {
                return Err(crate::error::KernelError::WouldBlock);
            };
            if byte == verase || byte == 8 {
                if line.len > 0 {
                    line.len -= 1;
                    if echo {
                        echo_bytes(&[8, b' ', 8]);
                    }
                }
                continue;
            }
            if byte == EOF_CHAR {
                line.complete = true;
                break;
            }
            let byte = if byte == b'\r' { b'\n' } else { byte };
            if line.len < CAPACITY {
                let at = line.len;
                line.buf[at] = byte;
                line.len += 1;
            }
            if echo {
                echo_bytes(if byte == b'\n' {
                    b"\n\r"
                } else {
                    core::slice::from_ref(&byte)
                });
            }
            if byte == b'\n' {
                line.complete = true;
            }
        }
        // Hand out the line, keeping any rest for the next read.
        let n = out.len().min(line.len);
        out[..n].copy_from_slice(&line.buf[..n]);
        let len = line.len;
        line.buf.copy_within(n..len, 0);
        line.len -= n;
        if line.len == 0 {
            line.complete = false;
        }
        return Ok(n);
    }
    // Raw mode. VMIN > 0 with VTIME = 0 waits until that many bytes (at
    // most the buffer) have arrived.
    let (vmin, vtime) = (terminal::get_vmin(), terminal::get_vtime());
    let want = usize::from(vmin).min(out.len());
    if vtime == 0 && want > 1 {
        poll_hardware();
        if with_ring(|r| r.len) < want {
            return Err(crate::error::KernelError::WouldBlock);
        }
    }
    let mut n = 0;
    while n < out.len() {
        let Some(byte) = try_read() else {
            break;
        };
        out[n] = byte;
        n += 1;
    }
    if echo {
        echo_bytes(&out[..n]);
    }
    if n > 0 {
        VTIME_DEADLINE.store(0, core::sync::atomic::Ordering::Relaxed);
        return Ok(n);
    }
    match (vmin, vtime) {
        // Polling read.
        (0, 0) => Ok(0),
        // Timed read: 0 bytes once VTIME tenths of a second pass.
        (0, t) => {
            let now = now_ns();
            let deadline = VTIME_DEADLINE.load(core::sync::atomic::Ordering::Relaxed);
            if deadline == 0 {
                VTIME_DEADLINE.store(
                    now + u64::from(t) * 100_000_000,
                    core::sync::atomic::Ordering::Relaxed,
                );
                Err(crate::error::KernelError::WouldBlock)
            } else if now >= deadline {
                VTIME_DEADLINE.store(0, core::sync::atomic::Ordering::Relaxed);
                Ok(0)
            } else {
                Err(crate::error::KernelError::WouldBlock)
            }
        }
        _ => Err(crate::error::KernelError::WouldBlock),
    }
}

/// [`read_tty`] for the console file nodes. Outside a dispatched task
/// (boot-context programs) nothing could wait for the `WouldBlock`, so the
/// read polls here as it always did.
pub fn read_node(out: &mut [u8]) -> Result<usize, crate::error::KernelError> {
    loop {
        match read_tty(out) {
            Err(crate::error::KernelError::WouldBlock) if !dispatched() => {
                core::hint::spin_loop();
            }
            other => return other,
        }
    }
}

fn dispatched() -> bool {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::current_owner().is_some()
    }
    #[cfg(not(feature = "alloc"))]
    {
        false
    }
}

/// poll bits of the console: always writable, readable when a byte or a
/// finished line is waiting.
pub fn tty_readiness() -> u16 {
    let mut ready = POLLOUT;
    if has_input() || LINE.lock().complete {
        ready |= POLLIN;
    }
    ready
}

/// When a timed raw-mode read gives up, for the waiter's deadline.
pub fn tty_ready_at() -> Option<u64> {
    match VTIME_DEADLINE.load(core::sync::atomic::Ordering::Relaxed) {
        0 => None,
        t => Some(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_order_and_drops_when_full() {
        let mut r = Ring::new();
        assert_eq!(r.pop(), None);
        for i in 0..CAPACITY {
            assert!(r.push(i as u8));
        }
        assert!(!r.push(0xEE), "full: the new byte is dropped");
        for i in 0..CAPACITY {
            assert_eq!(r.pop(), Some(i as u8));
        }
        assert_eq!(r.pop(), None);
        // Wraps around.
        assert!(r.push(1));
        assert!(r.push(2));
        assert_eq!((r.pop(), r.pop()), (Some(1), Some(2)));
    }

    /// The ring and the line buffer are global: tests that use them run
    /// one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn inject(bytes: &[u8]) {
        for &b in bytes {
            with_ring(|r| r.push(b));
        }
    }

    fn reset() {
        with_ring(|r| *r = Ring::new());
        let mut line = LINE.lock();
        line.len = 0;
        line.complete = false;
    }

    #[test]
    fn canonical_read_waits_for_the_line_and_keeps_partial_input() {
        let _g = SERIAL.lock();
        reset();
        let mut out = [0u8; 64];
        inject(b"ab");
        // No newline yet: nothing to return, and what was typed is kept.
        assert_eq!(
            read_tty(&mut out),
            Err(crate::error::KernelError::WouldBlock)
        );
        assert_eq!(
            tty_readiness() & POLLIN,
            0,
            "a partial line is not readable"
        );
        // DEL erases, CR ends the line as NL.
        inject(b"\x7fc\r");
        assert_ne!(tty_readiness() & POLLIN, 0);
        assert_eq!(read_tty(&mut out), Ok(3));
        assert_eq!(&out[..3], b"ac\n");
        assert_eq!(
            read_tty(&mut out),
            Err(crate::error::KernelError::WouldBlock)
        );
        reset();
    }

    #[test]
    fn canonical_read_hands_out_a_long_line_across_reads() {
        let _g = SERIAL.lock();
        reset();
        inject(b"hello\nnext");
        let mut small = [0u8; 2];
        assert_eq!(read_tty(&mut small), Ok(2));
        assert_eq!(&small, b"he");
        assert_eq!(read_tty(&mut small), Ok(2));
        assert_eq!(&small, b"ll");
        let mut out = [0u8; 8];
        assert_eq!(read_tty(&mut out), Ok(2));
        assert_eq!(&out[..2], b"o\n");
        // The next line's bytes wait in the ring until its newline.
        assert_eq!(
            read_tty(&mut out),
            Err(crate::error::KernelError::WouldBlock)
        );
        reset();
    }

    #[test]
    fn eof_char_ends_the_read_with_what_was_typed() {
        let _g = SERIAL.lock();
        reset();
        let mut out = [0u8; 8];
        inject(b"xy\x04");
        assert_eq!(read_tty(&mut out), Ok(2));
        assert_eq!(&out[..2], b"xy");
        // ^D on an empty line reads as end of file.
        inject(b"\x04");
        assert_eq!(read_tty(&mut out), Ok(0));
        reset();
    }
}

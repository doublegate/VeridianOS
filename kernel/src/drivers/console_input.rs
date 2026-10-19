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
    // Whoever drained the UART -- the tick, or a reader or poll in task
    // context -- the waiters are woken at the next tick: a reader asleep
    // while another task's poll moved its newline into the ring was not
    // (PR #24 review).
    if any {
        WAKE_PENDING.store(true, core::sync::atomic::Ordering::Relaxed);
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

/// A raw-mode read's VTIME timer: when it gives up (`monotonic_ns`), and
/// how many bytes had arrived when it was (re)started -- an inter-byte
/// timer restarts with each new byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Timer {
    deadline: u64,
    seen: usize,
}

/// The VTIME timer of each reader in a timed raw-mode read, by thread:
/// one per read, ended by [`end_read`] however the read ends. A single
/// global timer was shared by concurrent readers and outlived a read that
/// ended in EINTR or EAGAIN, so the next timed read returned 0 at once
/// (PR #24 review).
static TIMERS: Mutex<alloc::collections::BTreeMap<u64, Timer>> =
    Mutex::new(alloc::collections::BTreeMap::new());

/// The reading thread (0 outside a dispatched task).
fn reader() -> u64 {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::current_owner().map_or(0, |(_, tid)| tid)
    }
    #[cfg(not(feature = "alloc"))]
    {
        0
    }
}

/// The calling thread's read of the console has ended: its VTIME timer, if
/// any, goes. Called when every read system call returns.
pub fn end_read() {
    let me = reader();
    let mut timers = TIMERS.lock();
    if !timers.is_empty() {
        timers.remove(&me);
    }
}

/// What a raw-mode read does now, by POSIX's MIN and TIME (Linux's
/// n_tty_read), with `avail` bytes waiting, room for `out_len`, and the
/// reader's VTIME timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Raw {
    /// Return what is waiting (possibly nothing, for MIN = TIME = 0).
    Take,
    /// The timer ran out with nothing to return: 0 bytes.
    Zero,
    /// Wait, keeping (or starting) this timer.
    Wait(Option<Timer>),
}

fn raw_step(
    vmin: u8,
    vtime: u8,
    avail: usize,
    out_len: usize,
    now: u64,
    timer: Option<Timer>,
) -> Raw {
    let want = usize::from(vmin).min(out_len);
    let period = u64::from(vtime) * 100_000_000;
    match (vmin, vtime) {
        // Polling read.
        (0, 0) => Raw::Take,
        // MIN bytes, however long that takes.
        (_, 0) if avail >= want => Raw::Take,
        (_, 0) => Raw::Wait(None),
        // Timed read: what arrives within TIME, else 0.
        (0, _) if avail > 0 => Raw::Take,
        (0, _) => match timer {
            None => Raw::Wait(Some(Timer {
                deadline: now.saturating_add(period),
                seen: 0,
            })),
            Some(t) if now >= t.deadline => Raw::Zero,
            Some(t) => Raw::Wait(Some(t)),
        },
        // MIN bytes, or fewer once TIME passes between two of them; the
        // first byte is waited for without limit. It used to return on the
        // first byte (PR #24 review).
        _ if avail >= want => Raw::Take,
        _ if avail == 0 => Raw::Wait(None),
        _ => match timer {
            Some(t) if t.seen == avail && now >= t.deadline => Raw::Take,
            Some(t) if t.seen == avail => Raw::Wait(Some(t)),
            _ => Raw::Wait(Some(Timer {
                deadline: now.saturating_add(period),
                seen: avail,
            })),
        },
    }
}

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
    // Raw mode. Bytes a canonical read already gathered come first (they
    // were stranded once ICANON went off; PR #24 review), then the ring;
    // how many there are and taking them is one step under both locks
    // (LINE, then the ring), so a reader on another CPU cannot take the
    // bytes this one counted.
    let (vmin, vtime) = (terminal::get_vmin(), terminal::get_vtime());
    poll_hardware();
    let me = reader();
    let now = now_ns();
    let timer = TIMERS.lock().get(&me).copied();
    let mut line = LINE.lock();
    let step = with_ring(|r| {
        let avail = line.len + r.len;
        let step = raw_step(vmin, vtime, avail, out.len(), now, timer);
        if step != Raw::Take {
            return (step, 0, 0);
        }
        let from_line = line.len.min(out.len());
        out[..from_line].copy_from_slice(&line.buf[..from_line]);
        let len = line.len;
        line.buf.copy_within(from_line..len, 0);
        line.len -= from_line;
        if line.len == 0 {
            line.complete = false;
        }
        let mut n = from_line;
        while n < out.len() {
            let Some(byte) = r.pop() else {
                break;
            };
            out[n] = byte;
            n += 1;
        }
        (step, from_line, n)
    });
    drop(line);
    match step {
        (Raw::Take, from_line, n) => {
            TIMERS.lock().remove(&me);
            // The gathered line was echoed as it was typed.
            if echo {
                echo_bytes(&out[from_line..n]);
            }
            Ok(n)
        }
        (Raw::Zero, ..) => {
            TIMERS.lock().remove(&me);
            Ok(0)
        }
        (Raw::Wait(t), ..) => {
            let mut timers = TIMERS.lock();
            match t {
                Some(t) => timers.insert(me, t),
                None => timers.remove(&me),
            };
            Err(crate::error::KernelError::WouldBlock)
        }
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

/// poll bits of the console: always writable; readable as Linux's
/// input_available_p says -- in canonical mode a whole line (or ^D), in
/// raw mode MIN bytes when TIME is 0, else one. "Any byte in the ring" made
/// a partial canonical line readable, and a reader waiting on that spun in
/// the kernel (PR #24 review).
pub fn tty_readiness() -> u16 {
    use crate::drivers::terminal;
    poll_hardware();
    let line = LINE.lock();
    let readable = if terminal::is_canonical_mode() {
        line.complete || with_ring(|r| ring_has_line_end(r))
    } else {
        let (vmin, vtime) = (terminal::get_vmin(), terminal::get_vtime());
        raw_readable(vmin, vtime, line.len + with_ring(|r| r.len))
    };
    POLLOUT | if readable { POLLIN } else { 0 }
}

/// Whether a read of up to `out_len` bytes would return now (not
/// `WouldBlock`): what a reader sleeping on [`CONSOLE_INPUT`] waits for.
/// Stricter than poll readiness for a raw read waiting out its
/// inter-byte timer.
pub fn read_would_return(out_len: usize) -> bool {
    use crate::drivers::terminal;
    poll_hardware();
    let line = LINE.lock();
    if terminal::is_canonical_mode() {
        return line.complete || with_ring(|r| ring_has_line_end(r));
    }
    let (vmin, vtime) = (terminal::get_vmin(), terminal::get_vtime());
    let avail = line.len + with_ring(|r| r.len);
    let timer = TIMERS.lock().get(&reader()).copied();
    !matches!(
        raw_step(vmin, vtime, avail, out_len, now_ns(), timer),
        Raw::Wait(_)
    )
}

/// Raw-mode poll readiness with `avail` bytes waiting (Linux's `amt`).
fn raw_readable(vmin: u8, vtime: u8, avail: usize) -> bool {
    let amt = if vtime == 0 && vmin > 0 {
        usize::from(vmin)
    } else {
        1
    };
    avail >= amt
}

/// Whether the ring holds what ends a canonical line.
fn ring_has_line_end(r: &Ring) -> bool {
    (0..r.len).any(|i| matches!(r.buf[(r.head + i) % CAPACITY], b'\n' | b'\r' | EOF_CHAR))
}

/// When the calling reader's VTIME timer gives up, for its wait's deadline.
pub fn tty_ready_at() -> Option<u64> {
    TIMERS.lock().get(&reader()).map(|t| t.deadline)
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

    /// POSIX MIN/TIME (PR #24 review): MIN > 0 with TIME > 0 waits for the
    /// first byte without limit, then returns at MIN bytes or once TIME
    /// passes with no new byte; MIN = 0 with TIME > 0 returns what comes
    /// within TIME, else 0.
    #[test]
    fn raw_reads_follow_min_and_time() {
        const T: u64 = 100_000_000; // VTIME 1
                                    // MIN 2, TIME 1.
        assert_eq!(raw_step(2, 1, 0, 8, 0, None), Raw::Wait(None));
        let started = raw_step(2, 1, 1, 8, 50, None);
        assert_eq!(
            started,
            Raw::Wait(Some(Timer {
                deadline: 50 + T,
                seen: 1
            }))
        );
        let Raw::Wait(t) = started else {
            unreachable!()
        };
        assert_eq!(raw_step(2, 1, 1, 8, 60, t), Raw::Wait(t), "within TIME");
        assert_eq!(raw_step(2, 1, 1, 8, 50 + T, t), Raw::Take, "TIME passed");
        // A new byte restarts the inter-byte timer.
        assert_eq!(raw_step(2, 1, 2, 8, 70, t), Raw::Take, "MIN reached");
        assert_eq!(
            raw_step(5, 1, 3, 8, 70, t),
            Raw::Wait(Some(Timer {
                deadline: 70 + T,
                seen: 3
            }))
        );
        // MIN 0, TIME 1.
        let r = raw_step(0, 1, 0, 8, 10, None);
        let Raw::Wait(t) = r else { panic!("{r:?}") };
        assert_eq!(raw_step(0, 1, 0, 8, 10 + T, t), Raw::Zero);
        assert_eq!(raw_step(0, 1, 1, 8, 20, t), Raw::Take);
        // MIN 2, TIME 0, and the polling read.
        assert_eq!(raw_step(2, 0, 1, 8, 0, None), Raw::Wait(None));
        assert_eq!(
            raw_step(2, 0, 1, 1, 0, None),
            Raw::Take,
            "MIN capped by the buffer"
        );
        assert_eq!(raw_step(0, 0, 0, 8, 0, None), Raw::Take);
        // Poll readiness, Linux's amt.
        assert!(!raw_readable(2, 0, 1) && raw_readable(2, 0, 2));
        assert!(raw_readable(2, 1, 1) && !raw_readable(0, 1, 0));
    }

    /// Typed bytes with no line end are not readable in canonical mode,
    /// before a read gathered them as after (PR #24 review).
    #[test]
    fn a_partial_canonical_line_is_not_readable() {
        let _g = SERIAL.lock();
        reset();
        inject(b"ab");
        assert_eq!(tty_readiness() & POLLIN, 0);
        assert!(!read_would_return(8));
        inject(b"\n");
        assert_ne!(tty_readiness() & POLLIN, 0);
        assert!(read_would_return(8));
        reset();
    }

    /// What a canonical read gathered is read first once ICANON goes off
    /// (it was stranded until canonical mode came back; PR #24 review).
    #[test]
    fn raw_mode_reads_the_gathered_line_first() {
        use crate::drivers::terminal;
        let _g = SERIAL.lock();
        reset();
        terminal::init();
        let canonical = terminal::get_termios_snapshot();
        let mut out = [0u8; 8];
        inject(b"ab");
        assert_eq!(
            read_tty(&mut out),
            Err(crate::error::KernelError::WouldBlock)
        );
        let mut raw = canonical;
        raw.c_lflag &= !(terminal::ICANON | terminal::ECHO);
        terminal::set_termios(&raw);
        inject(b"c");
        let got = read_tty(&mut out);
        terminal::set_termios(&canonical);
        assert_eq!(got, Ok(3));
        assert_eq!(&out[..3], b"abc");
        reset();
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

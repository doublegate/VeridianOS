//! Lightweight static tracepoints for kernel performance analysis.
//!
//! Provides a per-CPU ring buffer of trace events that can be enabled/disabled
//! at runtime. When disabled, the overhead is a single atomic load (branch on
//! `TRACING_ENABLED`). When enabled, events are written to a fixed-size ring
//! buffer per CPU, requiring no heap allocation.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Global tracing enable flag -- zero overhead when false (single atomic load).
pub static TRACING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Trace event types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceEventType {
    /// System call entry
    SyscallEntry = 0,
    /// System call exit
    SyscallExit = 1,
    /// Context switch: task switched out
    SchedSwitchOut = 2,
    /// Context switch: task switched in
    SchedSwitchIn = 3,
    /// IPC fast path send
    IpcFastSend = 4,
    /// IPC fast path receive
    IpcFastReceive = 5,
    /// Frame allocator: allocate
    FrameAlloc = 6,
    /// Frame allocator: free
    FrameFree = 7,
    /// Page fault
    PageFault = 8,
    /// IPC slow path fallback
    IpcSlowPath = 9,
}

/// A single trace event (32 bytes, cache-line friendly).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TraceEvent {
    /// Timestamp (architecture-specific cycle counter)
    pub timestamp: u64,
    /// Event type
    pub event_type: u8,
    /// CPU that generated this event
    pub cpu: u8,
    /// Padding for alignment
    _pad: [u8; 6],
    /// Event-specific data (e.g., PID, syscall number, frame number)
    pub data: [u64; 2],
}

impl TraceEvent {
    const fn empty() -> Self {
        Self {
            timestamp: 0,
            event_type: 0,
            cpu: 0,
            _pad: [0; 6],
            data: [0; 2],
        }
    }
}

/// Number of events per CPU ring buffer (4096 events = 128KB per CPU)
const RING_SIZE: usize = 4096;

/// Per-CPU trace ring buffer
struct TraceRing {
    events: [TraceEvent; RING_SIZE],
    write_idx: AtomicUsize,
}

impl TraceRing {
    const fn new() -> Self {
        Self {
            events: [TraceEvent::empty(); RING_SIZE],
            write_idx: AtomicUsize::new(0),
        }
    }

    /// Record an event into the ring buffer (overwrites oldest on wrap).
    #[inline]
    fn record(&mut self, event: TraceEvent) {
        let idx = self.write_idx.load(Ordering::Relaxed) % RING_SIZE;
        self.events[idx] = event;
        self.write_idx.fetch_add(1, Ordering::Relaxed);
    }

    /// Get the total number of events recorded (may wrap)
    fn total_events(&self) -> usize {
        self.write_idx.load(Ordering::Relaxed)
    }

    /// Read events from the ring buffer (most recent first, up to `count`)
    fn read_recent(&self, count: usize) -> impl Iterator<Item = &TraceEvent> {
        let total = self.total_events();
        let available = total.min(RING_SIZE);
        let to_read = count.min(available);

        let start_raw = if total >= RING_SIZE {
            total - to_read
        } else {
            total.saturating_sub(to_read)
        };

        (start_raw..start_raw + to_read).map(move |i| &self.events[i % RING_SIZE])
    }
}

/// Maximum CPUs for trace ring allocation
const MAX_TRACE_CPUS: usize = 16;

/// Per-CPU trace rings.
///
/// SAFETY: Each CPU writes only to its own ring via `current_cpu_id()`.
/// Reading is done with tracing disabled or from the shell (single-threaded).
static mut TRACE_RINGS: [TraceRing; MAX_TRACE_CPUS] = [const { TraceRing::new() }; MAX_TRACE_CPUS];

/// Record a trace event (inline, minimal overhead).
///
/// When `TRACING_ENABLED` is false, this compiles down to a single atomic
/// load and branch (typically predicted not-taken).
#[inline(always)]
pub fn trace_event(event_type: TraceEventType, data0: u64, data1: u64) {
    if !TRACING_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let cpu = crate::sched::smp::current_cpu_id() as usize;
    let event = TraceEvent {
        timestamp: crate::bench::read_timestamp(),
        event_type: event_type as u8,
        cpu: cpu as u8,
        _pad: [0; 6],
        data: [data0, data1],
    };

    // SAFETY: Each CPU writes only to its own ring. The cpu index is
    // bounded by MAX_TRACE_CPUS via the min() call.
    unsafe {
        TRACE_RINGS[cpu.min(MAX_TRACE_CPUS - 1)].record(event);
    }
}

/// Enable tracing
pub fn enable() {
    TRACING_ENABLED.store(true, Ordering::Release);
}

/// Disable tracing
pub fn disable() {
    TRACING_ENABLED.store(false, Ordering::Release);
}

/// Check if tracing is enabled
pub fn is_enabled() -> bool {
    TRACING_ENABLED.load(Ordering::Relaxed)
}

/// Dump trace events from all CPUs to serial output.
///
/// Prints the most recent `count` events per CPU.
pub fn dump_trace(count: usize) {
    let was_enabled = is_enabled();
    disable(); // Pause tracing during dump

    crate::println!("=== Trace Dump (most recent {} per CPU) ===", count);
    crate::println!(
        "{:>12} {:>4} {:>18} {:>16} {:>16}",
        "TIMESTAMP",
        "CPU",
        "EVENT",
        "DATA0",
        "DATA1"
    );

    #[allow(clippy::needless_range_loop)]
    for cpu in 0..MAX_TRACE_CPUS {
        // SAFETY: We disabled tracing, so no concurrent writes.
        // We only read from the ring buffer.
        let ring = unsafe { &TRACE_RINGS[cpu] };
        let total = ring.total_events();
        if total == 0 {
            continue;
        }

        for event in ring.read_recent(count) {
            if event.timestamp == 0 {
                continue;
            }
            let name = event_type_name(event.event_type);
            crate::println!(
                "{:>12} {:>4} {:>18} {:#016x} {:#016x}",
                event.timestamp,
                event.cpu,
                name,
                event.data[0],
                event.data[1]
            );
        }
    }

    let total: usize = (0..MAX_TRACE_CPUS)
        .map(|cpu| unsafe { TRACE_RINGS[cpu].total_events() })
        .sum();
    crate::println!("=== Total events recorded: {} ===", total);

    if was_enabled {
        enable(); // Re-enable if it was on
    }
}

/// Get total events across all CPUs
pub fn total_events() -> usize {
    (0..MAX_TRACE_CPUS)
        .map(|cpu| unsafe { TRACE_RINGS[cpu].total_events() })
        .sum()
}

fn event_type_name(t: u8) -> &'static str {
    match t {
        0 => "syscall_entry",
        1 => "syscall_exit",
        2 => "sched_switch_out",
        3 => "sched_switch_in",
        4 => "ipc_fast_send",
        5 => "ipc_fast_recv",
        6 => "frame_alloc",
        7 => "frame_free",
        8 => "page_fault",
        9 => "ipc_slow_path",
        _ => "unknown",
    }
}

/// Convenience macro for recording trace events with zero overhead when
/// disabled.
#[macro_export]
macro_rules! trace {
    ($event_type:expr, $data0:expr, $data1:expr) => {
        $crate::perf::trace::trace_event($event_type, $data0, $data1)
    };
    ($event_type:expr, $data0:expr) => {
        $crate::perf::trace::trace_event($event_type, $data0, 0)
    };
    ($event_type:expr) => {
        $crate::perf::trace::trace_event($event_type, 0, 0)
    };
}

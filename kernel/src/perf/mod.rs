//! Performance optimization and monitoring
//!
//! Provides tools for profiling, optimization, and performance analysis.
//! Includes per-CPU run-queue instrumentation and IPC workload stats.

pub mod bench;
pub mod pmu;
pub mod trace;

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::error::KernelError;

/// Performance counters (snapshot view)
#[derive(Debug, Default, Clone, Copy)]
pub struct PerfCounters {
    pub syscalls: u64,
    pub context_switches: u64,
    pub page_faults: u64,
    pub interrupts: u64,
    pub ipc_messages: u64,
}

/// Atomic performance counters for safe concurrent access
static SYSCALL_COUNT: AtomicU64 = AtomicU64::new(0);
static CONTEXT_SWITCH_COUNT: AtomicU64 = AtomicU64::new(0);
static PAGE_FAULT_COUNT: AtomicU64 = AtomicU64::new(0);
static INTERRUPT_COUNT: AtomicU64 = AtomicU64::new(0);
static IPC_MESSAGE_COUNT: AtomicU64 = AtomicU64::new(0);

/// Increment syscall counter
#[inline(always)]
pub fn count_syscall() {
    SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Increment context switch counter
#[inline(always)]
pub fn count_context_switch() {
    CONTEXT_SWITCH_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Increment page fault counter
#[inline(always)]
pub fn count_page_fault() {
    PAGE_FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Increment interrupt counter
#[inline(always)]
pub fn count_interrupt() {
    INTERRUPT_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Get performance statistics as a point-in-time snapshot
pub fn get_stats() -> PerfCounters {
    PerfCounters {
        syscalls: SYSCALL_COUNT.load(Ordering::Relaxed),
        context_switches: CONTEXT_SWITCH_COUNT.load(Ordering::Relaxed),
        page_faults: PAGE_FAULT_COUNT.load(Ordering::Relaxed),
        interrupts: INTERRUPT_COUNT.load(Ordering::Relaxed),
        ipc_messages: IPC_MESSAGE_COUNT.load(Ordering::Relaxed),
    }
}

/// Reset performance counters
pub fn reset_stats() {
    SYSCALL_COUNT.store(0, Ordering::Relaxed);
    CONTEXT_SWITCH_COUNT.store(0, Ordering::Relaxed);
    PAGE_FAULT_COUNT.store(0, Ordering::Relaxed);
    INTERRUPT_COUNT.store(0, Ordering::Relaxed);
    IPC_MESSAGE_COUNT.store(0, Ordering::Relaxed);
}

/// Performance profiler
pub struct Profiler {
    start_time: u64,
    /// Read in end() via println! which is a no-op on some architectures.
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    name: &'static str,
}

impl Profiler {
    /// Start profiling a section
    pub fn start(name: &'static str) -> Self {
        Self {
            start_time: crate::test_framework::read_timestamp(),
            name,
        }
    }

    /// End profiling and print results
    pub fn end(self) {
        let _elapsed = crate::test_framework::read_timestamp() - self.start_time;
        println!("[PERF] {} took {} cycles", self.name, _elapsed);
    }
}

// ---------------------------------------------------------------------------
// Per-CPU Run-Queue Instrumentation
// ---------------------------------------------------------------------------

/// Maximum CPUs for run-queue stats tracking.
const MAX_RQ_CPUS: usize = 16;

/// Per-CPU run-queue statistics.
pub struct RunQueueStats {
    /// Total enqueue operations on this CPU.
    pub enqueue_count: AtomicU64,
    /// Total dequeue operations on this CPU.
    pub dequeue_count: AtomicU64,
    /// High-water mark for queue length.
    pub max_length: AtomicU32,
    /// Cumulative wait ticks across all dequeued tasks.
    pub total_wait_ticks: AtomicU64,
}

impl RunQueueStats {
    /// Create new zeroed stats.
    pub const fn new() -> Self {
        Self {
            enqueue_count: AtomicU64::new(0),
            dequeue_count: AtomicU64::new(0),
            max_length: AtomicU32::new(0),
            total_wait_ticks: AtomicU64::new(0),
        }
    }
}

impl Default for RunQueueStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Global per-CPU run-queue stats array.
#[allow(clippy::declare_interior_mutable_const)]
static RQ_STATS: [RunQueueStats; MAX_RQ_CPUS] = {
    const INIT: RunQueueStats = RunQueueStats::new();
    [INIT; MAX_RQ_CPUS]
};

/// Record an enqueue operation for a CPU's run queue.
#[inline(always)]
pub fn record_enqueue(cpu_id: usize, queue_len: u32) {
    if cpu_id < MAX_RQ_CPUS {
        RQ_STATS[cpu_id]
            .enqueue_count
            .fetch_add(1, Ordering::Relaxed);
        // Update high-water mark (best-effort CAS)
        let current_max = RQ_STATS[cpu_id].max_length.load(Ordering::Relaxed);
        if queue_len > current_max {
            let _ = RQ_STATS[cpu_id].max_length.compare_exchange(
                current_max,
                queue_len,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
}

/// Record a dequeue operation with wait time.
#[inline(always)]
pub fn record_dequeue(cpu_id: usize, wait_ticks: u64) {
    if cpu_id < MAX_RQ_CPUS {
        RQ_STATS[cpu_id]
            .dequeue_count
            .fetch_add(1, Ordering::Relaxed);
        RQ_STATS[cpu_id]
            .total_wait_ticks
            .fetch_add(wait_ticks, Ordering::Relaxed);
    }
}

/// Aggregated scheduler profile from all CPUs.
#[derive(Debug, Default, Clone, Copy)]
pub struct SchedulerProfile {
    /// Average wait ticks per dequeue across all CPUs.
    pub avg_wait_ticks: u64,
    /// Maximum queue length seen on any CPU.
    pub max_queue_length: u32,
    /// Total enqueues across all CPUs.
    pub total_enqueues: u64,
    /// Total dequeues across all CPUs.
    pub total_dequeues: u64,
}

/// Collect aggregated scheduler stats from all CPUs.
pub fn get_scheduler_stats() -> SchedulerProfile {
    let mut total_enq = 0u64;
    let mut total_deq = 0u64;
    let mut total_wait = 0u64;
    let mut max_len = 0u32;

    for stats in &RQ_STATS {
        total_enq += stats.enqueue_count.load(Ordering::Relaxed);
        total_deq += stats.dequeue_count.load(Ordering::Relaxed);
        total_wait += stats.total_wait_ticks.load(Ordering::Relaxed);
        let ml = stats.max_length.load(Ordering::Relaxed);
        if ml > max_len {
            max_len = ml;
        }
    }

    let avg_wait = if total_deq > 0 {
        total_wait / total_deq
    } else {
        0
    };

    SchedulerProfile {
        avg_wait_ticks: avg_wait,
        max_queue_length: max_len,
        total_enqueues: total_enq,
        total_dequeues: total_deq,
    }
}

// ---------------------------------------------------------------------------
// IPC Workload Profiling
// ---------------------------------------------------------------------------

/// IPC messages sent via fast path.
static IPC_MESSAGES_SENT: AtomicU64 = AtomicU64::new(0);
/// IPC batches flushed.
static IPC_BATCHES_FLUSHED: AtomicU64 = AtomicU64::new(0);

/// Record an IPC message sent via the fast path.
#[inline(always)]
pub fn record_ipc_message_sent() {
    IPC_MESSAGES_SENT.fetch_add(1, Ordering::Relaxed);
}

/// Record an IPC batch flush.
#[inline(always)]
pub fn record_ipc_batch_flushed() {
    IPC_BATCHES_FLUSHED.fetch_add(1, Ordering::Relaxed);
}

/// Get IPC workload stats: (messages_sent, batches_flushed).
pub fn get_ipc_workload_stats() -> (u64, u64) {
    (
        IPC_MESSAGES_SENT.load(Ordering::Relaxed),
        IPC_BATCHES_FLUSHED.load(Ordering::Relaxed),
    )
}

// ---------------------------------------------------------------------------
// Optimization Reporting
// ---------------------------------------------------------------------------

/// Optimize memory allocator.
///
/// Collects allocation statistics and logs fragmentation metrics.
pub fn optimize_memory() {
    println!("[PERF] Optimizing memory allocator...");
    let stats = crate::mm::get_memory_stats();
    let used = stats.total_frames.saturating_sub(stats.free_frames);
    let utilization = if stats.total_frames > 0 {
        (used * 100) / stats.total_frames
    } else {
        0
    };
    println!(
        "[PERF]   Memory: {} total, {} free, {} cached, {}% used",
        stats.total_frames, stats.free_frames, stats.cached_frames, utilization
    );
}

/// Optimize scheduler.
///
/// Reports per-CPU run-queue instrumentation data: average wait time,
/// max queue depth, and total enqueue/dequeue counts.
pub fn optimize_scheduler() {
    println!("[PERF] Optimizing scheduler...");
    let counters = get_stats();
    let sched_profile = get_scheduler_stats();
    println!(
        "[PERF]   Scheduler: {} context switches, {} syscalls",
        counters.context_switches, counters.syscalls
    );
    println!(
        "[PERF]   Run-queue: avg_wait={} ticks, max_depth={}, enq={}, deq={}",
        sched_profile.avg_wait_ticks,
        sched_profile.max_queue_length,
        sched_profile.total_enqueues,
        sched_profile.total_dequeues
    );
}

/// Optimize IPC.
///
/// Reports IPC message throughput and batch flush statistics.
pub fn optimize_ipc() {
    println!("[PERF] Optimizing IPC...");
    let counters = get_stats();
    let (msgs_sent, batches) = get_ipc_workload_stats();
    println!("[PERF]   IPC: {} messages delivered", counters.ipc_messages);
    println!(
        "[PERF]   IPC workload: {} fast-path sends, {} batch flushes",
        msgs_sent, batches
    );
}

/// Initialize performance subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[PERF] Initializing performance subsystem...");

    reset_stats();

    // Apply optimizations
    optimize_memory();
    optimize_scheduler();
    optimize_ipc();

    println!("[PERF] Performance subsystem initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counters() {
        reset_stats();
        count_syscall();
        count_context_switch();
        let stats = get_stats();
        assert_eq!(stats.syscalls, 1);
        assert_eq!(stats.context_switches, 1);
    }

    #[test]
    fn test_profiler() {
        let p = Profiler::start("test");
        // Do some work
        for _ in 0..1000 {
            core::hint::black_box(42);
        }
        p.end();
    }

    #[test]
    fn test_run_queue_stats() {
        record_enqueue(0, 5);
        record_enqueue(0, 10);
        record_dequeue(0, 100);
        let profile = get_scheduler_stats();
        assert!(profile.total_enqueues >= 2);
        assert!(profile.total_dequeues >= 1);
    }

    #[test]
    fn test_ipc_workload_stats() {
        record_ipc_message_sent();
        record_ipc_batch_flushed();
        let (msgs, batches) = get_ipc_workload_stats();
        assert!(msgs >= 1);
        assert!(batches >= 1);
    }
}

//! Performance optimization and monitoring
//!
//! Provides tools for profiling, optimization, and performance analysis.

pub mod bench;
pub mod pmu;
pub mod trace;

use core::sync::atomic::{AtomicU64, Ordering};

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

/// Optimize memory allocator.
///
/// Collects allocation statistics and logs fragmentation metrics.
/// Full compaction/defragmentation requires a stop-the-world phase
/// and is deferred to Phase 6.
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
/// Reports scheduling performance counters (context switches, syscalls).
/// Affinity tuning and cross-node load rebalancing require per-CPU
/// run-queue instrumentation (TODO(phase6)).
pub fn optimize_scheduler() {
    println!("[PERF] Optimizing scheduler...");
    let counters = get_stats();
    println!(
        "[PERF]   Scheduler: {} context switches, {} syscalls",
        counters.context_switches, counters.syscalls
    );
}

/// Optimize IPC.
///
/// Reports IPC message throughput counters.  Fast-path tuning and
/// message batching require workload profiling (TODO(phase6)).
pub fn optimize_ipc() {
    println!("[PERF] Optimizing IPC...");
    let counters = get_stats();
    println!("[PERF]   IPC: {} messages delivered", counters.ipc_messages);
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
}

//! Performance optimization and monitoring
//!
//! Provides tools for profiling, optimization, and performance analysis.

use crate::error::KernelError;

/// Performance counters
#[derive(Debug, Default, Clone, Copy)]
pub struct PerfCounters {
    pub syscalls: u64,
    pub context_switches: u64,
    pub page_faults: u64,
    pub interrupts: u64,
    pub ipc_messages: u64,
}

static mut COUNTERS: PerfCounters = PerfCounters {
    syscalls: 0,
    context_switches: 0,
    page_faults: 0,
    interrupts: 0,
    ipc_messages: 0,
};

/// Increment syscall counter
#[inline(always)]
pub fn count_syscall() {
    // SAFETY: COUNTERS is a static mut used as a simple global accumulator. All
    // accesses are non-atomic wrapping increments on a single field. In the
    // current single-core kernel, no concurrent mutation occurs. Minor races on
    // SMP would only cause a missed count, not memory unsafety.
    unsafe {
        COUNTERS.syscalls = COUNTERS.syscalls.wrapping_add(1);
    }
}

/// Increment context switch counter
#[inline(always)]
pub fn count_context_switch() {
    // SAFETY: COUNTERS is a static mut global accumulator. The wrapping_add on a
    // single u64 field is safe in the current single-core kernel. On SMP, a missed
    // count is the worst outcome (no memory corruption).
    unsafe {
        COUNTERS.context_switches = COUNTERS.context_switches.wrapping_add(1);
    }
}

/// Increment page fault counter
#[inline(always)]
pub fn count_page_fault() {
    // SAFETY: COUNTERS is a static mut global accumulator. The wrapping_add on a
    // single u64 field is safe in the current single-core kernel. On SMP, a missed
    // count is the worst outcome (no memory corruption).
    unsafe {
        COUNTERS.page_faults = COUNTERS.page_faults.wrapping_add(1);
    }
}

/// Get performance statistics
pub fn get_stats() -> PerfCounters {
    // SAFETY: COUNTERS is a static mut read as a snapshot. The Copy on PerfCounters
    // produces a consistent point-in-time value. Slight staleness on SMP is
    // acceptable for diagnostic counters.
    unsafe { COUNTERS }
}

/// Reset performance counters
pub fn reset_stats() {
    // SAFETY: COUNTERS is a static mut reset to the default zero state. This is
    // called from diagnostic paths only. In the current single-core kernel, no
    // concurrent access occurs during reset.
    unsafe {
        COUNTERS = PerfCounters::default();
    }
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

/// Optimize memory allocator
pub fn optimize_memory() {
    println!("[PERF] Optimizing memory allocator...");
    // TODO(phase5): Implement memory allocator optimization (compaction,
    // defrag)
}

/// Optimize scheduler
pub fn optimize_scheduler() {
    println!("[PERF] Optimizing scheduler...");
    // TODO(phase5): Implement scheduler optimization (affinity tuning, load
    // rebalance)
}

/// Optimize IPC
pub fn optimize_ipc() {
    println!("[PERF] Optimizing IPC...");
    // TODO(phase5): Implement IPC optimization (fast-path tuning, batching)
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

    #[test_case]
    fn test_counters() {
        reset_stats();
        count_syscall();
        count_context_switch();
        let stats = get_stats();
        assert_eq!(stats.syscalls, 1);
        assert_eq!(stats.context_switches, 1);
    }

    #[test_case]
    fn test_profiler() {
        let p = Profiler::start("test");
        // Do some work
        for _ in 0..1000 {
            core::hint::black_box(42);
        }
        p.end();
    }
}

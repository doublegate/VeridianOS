//! IPC performance measurement
//!
//! This module provides functions for measuring IPC performance
//! to ensure we meet our latency targets.

#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, Ordering};

/// Global IPC performance statistics
pub struct IpcPerfStats {
    /// Total IPC operations
    pub total_ops: AtomicU64,
    /// Total cycles spent in IPC
    pub total_cycles: AtomicU64,
    /// Minimum latency observed
    pub min_latency: AtomicU64,
    /// Maximum latency observed
    pub max_latency: AtomicU64,
    /// Fast path success count
    pub fast_path_count: AtomicU64,
    /// Slow path count
    pub slow_path_count: AtomicU64,
}

impl IpcPerfStats {
    pub const fn new() -> Self {
        Self {
            total_ops: AtomicU64::new(0),
            total_cycles: AtomicU64::new(0),
            min_latency: AtomicU64::new(u64::MAX),
            max_latency: AtomicU64::new(0),
            fast_path_count: AtomicU64::new(0),
            slow_path_count: AtomicU64::new(0),
        }
    }

    /// Record an IPC operation
    pub fn record_operation(&self, cycles: u64, is_fast_path: bool) {
        self.total_ops.fetch_add(1, Ordering::Relaxed);
        self.total_cycles.fetch_add(cycles, Ordering::Relaxed);

        // Update min latency
        let mut current_min = self.min_latency.load(Ordering::Relaxed);
        while cycles < current_min {
            match self.min_latency.compare_exchange_weak(
                current_min,
                cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(val) => current_min = val,
            }
        }

        // Update max latency
        let mut current_max = self.max_latency.load(Ordering::Relaxed);
        while cycles > current_max {
            match self.max_latency.compare_exchange_weak(
                current_max,
                cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(val) => current_max = val,
            }
        }

        // Update path counters
        if is_fast_path {
            self.fast_path_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.slow_path_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get average latency in cycles
    pub fn average_latency(&self) -> u64 {
        let ops = self.total_ops.load(Ordering::Relaxed);
        let cycles = self.total_cycles.load(Ordering::Relaxed);
        if ops > 0 {
            cycles / ops
        } else {
            0
        }
    }

    /// Get a performance report
    pub fn get_report(&self) -> IpcPerfReport {
        let total_ops = self.total_ops.load(Ordering::Relaxed);
        let fast_path = self.fast_path_count.load(Ordering::Relaxed);
        let _slow_path = self.slow_path_count.load(Ordering::Relaxed);

        IpcPerfReport {
            total_operations: total_ops,
            average_latency_cycles: self.average_latency(),
            min_latency_cycles: self.min_latency.load(Ordering::Relaxed),
            max_latency_cycles: self.max_latency.load(Ordering::Relaxed),
            fast_path_percentage: if total_ops > 0 {
                (fast_path * 100) / total_ops
            } else {
                0
            },
            average_latency_ns: cycles_to_ns(self.average_latency()),
            min_latency_ns: cycles_to_ns(self.min_latency.load(Ordering::Relaxed)),
            max_latency_ns: cycles_to_ns(self.max_latency.load(Ordering::Relaxed)),
        }
    }
}

impl Default for IpcPerfStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Global IPC performance statistics instance
pub static IPC_PERF_STATS: IpcPerfStats = IpcPerfStats::new();

/// IPC performance report
#[derive(Debug, Clone, Copy)]
pub struct IpcPerfReport {
    pub total_operations: u64,
    pub average_latency_cycles: u64,
    pub min_latency_cycles: u64,
    pub max_latency_cycles: u64,
    pub fast_path_percentage: u64,
    pub average_latency_ns: u64,
    pub min_latency_ns: u64,
    pub max_latency_ns: u64,
}

impl IpcPerfReport {
    /// Check if we meet Phase 1 targets
    pub fn meets_phase1_targets(&self) -> bool {
        // Phase 1 targets: < 5μs (5000ns) for all messages
        self.average_latency_ns < 5000 && self.max_latency_ns < 10000
    }

    /// Check if we meet Phase 5 targets
    pub fn meets_phase5_targets(&self) -> bool {
        // Phase 5 targets: < 1μs (1000ns) average
        self.average_latency_ns < 1000
    }

    /// Print the performance report
    pub fn print(&self) {
        println!("\n=== IPC Performance Report ===");
        println!("Total operations: {}", self.total_operations);
        println!("Fast path usage: {}%", self.fast_path_percentage);
        println!("\nLatency (cycles):");
        println!("  Average: {}", self.average_latency_cycles);
        println!("  Min: {}", self.min_latency_cycles);
        println!("  Max: {}", self.max_latency_cycles);
        println!("\nLatency (nanoseconds):");
        println!("  Average: {} ns", self.average_latency_ns);
        println!("  Min: {} ns", self.min_latency_ns);
        println!("  Max: {} ns", self.max_latency_ns);

        let _phase1_status = if self.meets_phase1_targets() {
            "\n[PASS] Meets Phase 1 targets (<5us)"
        } else {
            "\n[FAIL] Does not meet Phase 1 targets"
        };
        println!("{}", _phase1_status);

        let _phase5_status = if self.meets_phase5_targets() {
            "[PASS] Meets Phase 5 targets (<1us average)"
        } else {
            "[FAIL] Does not meet Phase 5 targets"
        };
        println!("{}", _phase5_status);
    }
}

/// Convert CPU cycles to nanoseconds (assumes 2GHz CPU)
pub fn cycles_to_ns(cycles: u64) -> u64 {
    // For a 2GHz CPU: 1 cycle = 0.5ns
    // Adjust this based on actual CPU frequency
    cycles / 2
}

/// Measure a single IPC operation
#[inline(always)]
pub fn measure_ipc_operation<F, R>(f: F) -> (R, u64)
where
    F: FnOnce() -> R,
{
    let start = read_timestamp();
    let result = f();
    let elapsed = read_timestamp() - start;
    (result, elapsed)
}

/// Read CPU timestamp counter
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn read_timestamp() -> u64 {
    // SAFETY: _rdtsc reads the x86_64 Time Stamp Counter. It is always available
    // in kernel mode and requires no special setup or preconditions.
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn read_timestamp() -> u64 {
    // Read the system counter
    let val: u64;
    // SAFETY: The mrs instruction reads the AArch64 Virtual Counter register
    // (cntvct_el0), which is always accessible in kernel mode and produces no
    // side effects beyond reading a monotonically increasing counter.
    unsafe {
        core::arch::asm!("mrs {}, cntvct_el0", out(reg) val);
    }
    val
}

#[cfg(target_arch = "riscv64")]
#[inline(always)]
pub fn read_timestamp() -> u64 {
    // Read the cycle counter
    let val: u64;
    // SAFETY: The rdcycle instruction reads the RISC-V cycle counter CSR, which
    // is always accessible in kernel mode and produces no side effects beyond
    // reading a monotonically increasing counter.
    unsafe {
        core::arch::asm!("rdcycle {}", out(reg) val);
    }
    val
}

#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64"
)))]
#[inline(always)]
pub fn read_timestamp() -> u64 {
    0
}

/// Benchmark utilities for measuring IPC performance
pub mod bench {
    use super::*;

    /// Run a performance test
    pub fn run_perf_test<F>(name: &str, iterations: usize, mut f: F)
    where
        F: FnMut(),
    {
        kprintln!("\nRunning performance test:");
        kprint_rt!(name);
        kprintln!();

        // Warmup
        for _ in 0..10 {
            f();
        }

        // Actual measurement
        let start = read_timestamp();
        for _ in 0..iterations {
            f();
        }
        let total_cycles = read_timestamp() - start;
        let avg_cycles = total_cycles / iterations as u64;

        let avg_ns = cycles_to_ns(avg_cycles);

        kprintln!("  Iterations:");
        kprint_u64!(iterations);
        kprintln!();
        kprintln!("  Average cycles:");
        kprint_u64!(avg_cycles);
        kprintln!();
        kprintln!("  Average ns:");
        kprint_u64!(avg_ns);
        kprintln!();

        if avg_ns < 1000 {
            kprintln!("  Sub-microsecond performance!");
        }
    }

    /// Measure IPC throughput
    pub fn measure_throughput<F>(name: &str, duration_ms: u64, mut f: F) -> u64
    where
        F: FnMut(),
    {
        kprintln!("\nMeasuring throughput:");
        kprint_rt!(name);
        kprintln!();

        let duration_cycles = duration_ms * 2_000_000; // Assuming 2GHz
        let start = read_timestamp();
        let mut operations = 0u64;

        while read_timestamp() - start < duration_cycles {
            f();
            operations += 1;
        }

        let actual_duration_ms = cycles_to_ns(read_timestamp() - start) / 1_000_000;

        kprintln!("  Operations:");
        kprint_u64!(operations);
        kprintln!();
        kprintln!("  Duration ms:");
        kprint_u64!(actual_duration_ms);
        kprintln!();

        (operations * 1000) / actual_duration_ms
    }
}

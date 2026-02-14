//! Benchmarking framework for VeridianOS kernel
//!
//! Provides performance measurement capabilities for kernel subsystems

// Benchmarking APIs are intentionally kept available even when not actively
// called -- they are invoked on-demand via the `benchmarks` feature or from
// integration benchmark binaries.
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

/// Trait for benchmarkable operations
pub trait Benchmark {
    /// Run the benchmark and return the result
    fn run(&mut self) -> BenchmarkResult;

    /// Get the name of this benchmark
    fn name(&self) -> &str;
}

/// Result of a benchmark run
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    #[cfg(feature = "alloc")]
    pub name: alloc::string::String,
    #[cfg(not(feature = "alloc"))]
    pub name: &'static str,
    pub iterations: u64,
    pub total_time_ns: u64,
    pub avg_time_ns: u64,
    pub min_time_ns: u64,
    pub max_time_ns: u64,
}

impl BenchmarkResult {
    /// Create a new benchmark result
    #[cfg(feature = "alloc")]
    pub fn new(name: alloc::string::String, times: &[u64]) -> Self {
        let iterations = times.len() as u64;
        let total_time_ns: u64 = times.iter().sum();
        let avg_time_ns = total_time_ns / iterations;
        let min_time_ns = *times.iter().min().unwrap_or(&0);
        let max_time_ns = *times.iter().max().unwrap_or(&0);

        Self {
            name,
            iterations,
            total_time_ns,
            avg_time_ns,
            min_time_ns,
            max_time_ns,
        }
    }

    /// Check if this benchmark meets a target latency
    pub fn meets_target(&self, target_ns: u64) -> bool {
        self.avg_time_ns <= target_ns
    }
}

/// Architecture-specific timestamp counter.
///
/// Re-exports the centralized [`crate::arch::entropy::read_timestamp`] which
/// provides implementations for x86_64 (RDTSC), AArch64 (CNTVCT_EL0), and
/// RISC-V (rdcycle).
#[inline]
pub fn read_timestamp() -> u64 {
    crate::arch::entropy::read_timestamp()
}

/// Convert cycles to nanoseconds (approximate)
/// This assumes a 2GHz processor for now
pub fn cycles_to_ns(cycles: u64) -> u64 {
    // 2GHz = 2 cycles per nanosecond
    cycles / 2
}

/// Run a benchmark function multiple times
#[cfg(feature = "alloc")]
pub fn bench_function<F>(name: &str, iterations: u64, mut f: F) -> BenchmarkResult
where
    F: FnMut(),
{
    extern crate alloc;
    use alloc::{string::ToString, vec::Vec};

    let mut times = Vec::with_capacity(iterations as usize);

    // Warmup
    for _ in 0..10 {
        f();
    }

    // Actual benchmark
    for _ in 0..iterations {
        let start = read_timestamp();
        f();
        let end = read_timestamp();
        let elapsed_cycles = end.saturating_sub(start);
        times.push(cycles_to_ns(elapsed_cycles));
    }

    BenchmarkResult::new(name.to_string(), &times)
}

/// Benchmark harness for running multiple benchmarks
#[cfg(feature = "alloc")]
pub struct BenchmarkHarness {
    benchmarks: alloc::vec::Vec<alloc::boxed::Box<dyn Benchmark>>,
}

#[cfg(feature = "alloc")]
impl Default for BenchmarkHarness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl BenchmarkHarness {
    pub fn new() -> Self {
        Self {
            benchmarks: alloc::vec::Vec::new(),
        }
    }

    pub fn add_benchmark(&mut self, bench: alloc::boxed::Box<dyn Benchmark>) {
        self.benchmarks.push(bench);
    }

    pub fn run_all(&mut self) -> alloc::vec::Vec<BenchmarkResult> {
        use crate::serial_println;

        serial_println!("Running {} benchmarks...", self.benchmarks.len());

        let mut results = alloc::vec::Vec::new();

        for bench in &mut self.benchmarks {
            serial_println!("Running benchmark: {}", bench.name());
            let result = bench.run();

            serial_println!(
                "  Avg: {} ns, Min: {} ns, Max: {} ns",
                result.avg_time_ns,
                result.min_time_ns,
                result.max_time_ns
            );

            results.push(result);
        }

        results
    }
}

#[macro_export]
macro_rules! benchmark {
    ($name:expr, $iterations:expr, $code:block) => {{
        $crate::bench::bench_function($name, $iterations, || $code)
    }};
}

/// Simple bencher for benchmark tests
pub struct Bencher {
    iterations: u64,
}

impl Bencher {
    pub fn new() -> Self {
        Self { iterations: 100 }
    }

    pub fn iter<F>(&mut self, mut f: F)
    where
        F: FnMut(),
    {
        // Simple iteration - in real benchmarking framework
        // this would do more sophisticated timing
        for _ in 0..self.iterations {
            f();
        }
    }
}

impl Default for Bencher {
    fn default() -> Self {
        Self::new()
    }
}

/// Black box to prevent compiler optimizations
#[inline]
pub fn black_box<T>(x: T) -> T {
    // This is a simple implementation that prevents the compiler
    // from optimizing away the value
    // SAFETY: read_volatile reads the value from a valid stack reference.
    // mem::forget prevents the original from being dropped (avoiding
    // double-free). The volatile read prevents the compiler from
    // optimizing away the benchmark computation.
    unsafe {
        let ret = core::ptr::read_volatile(&x);
        core::mem::forget(x);
        ret
    }
}

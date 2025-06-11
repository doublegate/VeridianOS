//! No-std test framework for VeridianOS kernel
//!
//! This module provides testing infrastructure that works in a no_std
//! environment by using serial output and QEMU exit codes to report test
//! results.

use core::panic::PanicInfo;
use core::time::Duration;

use crate::{serial_print, serial_println};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

/// Trait that all testable functions must implement
pub trait Testable {
    fn run(&self);
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

/// Custom test runner for kernel tests
#[allow(dead_code)]
pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

/// Panic handler for test mode
pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
}

/// Exit QEMU with a specific exit code
pub fn exit_qemu(_exit_code: QemuExitCode) -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use x86_64::instructions::port::Port;
        let mut port = Port::new(0xf4);
        port.write(_exit_code as u32);
        core::hint::unreachable_unchecked();
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Use PSCI SYSTEM_OFF for AArch64
        const PSCI_SYSTEM_OFF: u32 = 0x84000008;
        unsafe {
            core::arch::asm!(
                "mov w0, {psci_off:w}",
                "hvc #0",
                psci_off = in(reg) PSCI_SYSTEM_OFF,
                options(noreturn)
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // Use SBI shutdown call
        const SBI_SHUTDOWN: usize = 8;
        unsafe {
            core::arch::asm!(
                "li a7, {sbi_shutdown}",
                "ecall",
                sbi_shutdown = const SBI_SHUTDOWN,
                options(noreturn)
            );
        }
    }

    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    loop {
        core::hint::spin_loop();
    }
}

/// Helper macro for creating test modules
#[macro_export]
macro_rules! test_module {
    ($name:ident, $($test:path),* $(,)?) => {
        #[cfg(test)]
        mod $name {
            use super::*;

            #[test_case]
            $(
                fn $test() {
                    $test();
                }
            )*
        }
    };
}

/// Assertion macros for kernel tests
#[macro_export]
macro_rules! kernel_assert {
    ($cond:expr) => {
        if !$cond {
            serial_println!("Assertion failed: {}", stringify!($cond));
            panic!("Assertion failed");
        }
    };
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            serial_println!($($arg)*);
            panic!("Assertion failed");
        }
    };
}

#[macro_export]
macro_rules! kernel_assert_eq {
    ($left:expr, $right:expr) => {
        if $left != $right {
            serial_println!(
                "Assertion failed: {} != {}\n  left: {:?}\n right: {:?}",
                stringify!($left),
                stringify!($right),
                $left,
                $right
            );
            panic!("Assertion failed: not equal");
        }
    };
}

#[macro_export]
macro_rules! kernel_assert_ne {
    ($left:expr, $right:expr) => {
        if $left == $right {
            serial_println!(
                "Assertion failed: {} == {}\n  left: {:?}\n right: {:?}",
                stringify!($left),
                stringify!($right),
                $left,
                $right
            );
            panic!("Assertion failed: equal");
        }
    };
}

// ===== Benchmark Infrastructure =====

/// Trait for benchmarkable functions
pub trait Benchmark {
    fn run(&self, iterations: u64) -> Duration;
    fn warmup(&self, iterations: u64);
    fn name(&self) -> &'static str;
}

/// A benchmark result
#[derive(Debug, Clone, Copy)]
pub struct BenchmarkResult {
    pub name: &'static str,
    pub iterations: u64,
    pub total_time: Duration,
    pub avg_time_ns: u64,
    pub min_time_ns: u64,
    pub max_time_ns: u64,
}

/// Get current timestamp in nanoseconds (architecture-specific)
#[inline(always)]
pub fn read_timestamp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::x86_64::_rdtsc;
        _rdtsc()
    }
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let timestamp: u64;
        core::arch::asm!("mrs {}, cntvct_el0", out(reg) timestamp);
        timestamp
    }
    
    #[cfg(target_arch = "riscv64")]
    unsafe {
        let timestamp: u64;
        core::arch::asm!("rdcycle {}", out(reg) timestamp);
        timestamp
    }
}

/// Convert CPU cycles to nanoseconds (approximate)
#[inline(always)]
pub fn cycles_to_ns(cycles: u64) -> u64 {
    // Assume 2GHz CPU for now (should be configurable)
    const CPU_FREQ_GHZ: u64 = 2;
    cycles / CPU_FREQ_GHZ
}

/// Benchmark runner
pub struct BenchmarkRunner {
    iterations: u64,
    warmup_iterations: u64,
}

impl BenchmarkRunner {
    pub const fn new() -> Self {
        Self {
            iterations: 1000,
            warmup_iterations: 100,
        }
    }

    pub fn run_benchmark<F>(&self, name: &'static str, mut f: F) -> BenchmarkResult
    where
        F: FnMut(),
    {
        serial_print!("{}...\t", name);
        
        // Warmup
        for _ in 0..self.warmup_iterations {
            f();
        }
        
        // Actual benchmark
        let mut min_cycles = u64::MAX;
        let mut max_cycles = 0u64;
        let mut total_cycles = 0u64;
        
        for _ in 0..self.iterations {
            let start = read_timestamp();
            f();
            let end = read_timestamp();
            let elapsed = end.saturating_sub(start);
            
            total_cycles += elapsed;
            min_cycles = min_cycles.min(elapsed);
            max_cycles = max_cycles.max(elapsed);
        }
        
        let avg_cycles = total_cycles / self.iterations;
        let result = BenchmarkResult {
            name,
            iterations: self.iterations,
            total_time: Duration::from_nanos(cycles_to_ns(total_cycles)),
            avg_time_ns: cycles_to_ns(avg_cycles),
            min_time_ns: cycles_to_ns(min_cycles),
            max_time_ns: cycles_to_ns(max_cycles),
        };
        
        serial_println!("[ok] avg: {} ns", result.avg_time_ns);
        result
    }
}

/// Macro for creating benchmarks
#[macro_export]
macro_rules! kernel_bench {
    ($name:ident, $body:expr) => {
        #[test_case]
        fn $name() {
            use $crate::test_framework::{BenchmarkRunner, cycles_to_ns, read_timestamp};
            let runner = BenchmarkRunner::new();
            let result = runner.run_benchmark(stringify!($name), || {
                $body
            });
            serial_println!("  Min: {} ns, Max: {} ns", result.min_time_ns, result.max_time_ns);
        }
    };
}

// ===== Test Registry =====

#[cfg(feature = "alloc")]
pub struct TestRegistry {
    tests: Vec<(&'static str, fn())>,
    benchmarks: Vec<(&'static str, fn())>,
}

#[cfg(feature = "alloc")]
impl TestRegistry {
    pub const fn new() -> Self {
        Self {
            tests: Vec::new(),
            benchmarks: Vec::new(),
        }
    }
    
    pub fn register_test(&mut self, name: &'static str, test: fn()) {
        self.tests.push((name, test));
    }
    
    pub fn register_benchmark(&mut self, name: &'static str, bench: fn()) {
        self.benchmarks.push((name, bench));
    }
    
    pub fn run_all(&self) -> (usize, usize) {
        let mut passed = 0;
        let failed = 0;
        
        serial_println!("Running {} tests", self.tests.len());
        for (name, test) in &self.tests {
            serial_print!("{}...\t", name);
            test();
            serial_println!("[ok]");
            passed += 1;
        }
        
        if !self.benchmarks.is_empty() {
            serial_println!("\nRunning {} benchmarks", self.benchmarks.len());
            for (_name, bench) in &self.benchmarks {
                bench();
            }
        }
        
        (passed, failed)
    }
}

#[cfg(feature = "alloc")]
pub static mut TEST_REGISTRY: Option<TestRegistry> = None;

#[cfg(feature = "alloc")]
pub fn init_test_registry() {
    unsafe {
        TEST_REGISTRY = Some(TestRegistry::new());
    }
}

#[cfg(feature = "alloc")]
#[macro_export]
macro_rules! register_test {
    ($name:ident) => {
        #[allow(non_snake_case)]
        #[used]
        #[link_section = ".test_registry"]
        static $name: fn() = || {
            unsafe {
                if let Some(registry) = &mut $crate::test_framework::TEST_REGISTRY {
                    registry.register_test(stringify!($name), $name);
                }
            }
        };
    };
}

// ===== Test Timeout Support =====

/// Run a test with a timeout (uses architecture-specific timer)
pub fn run_with_timeout<F>(f: F, timeout_cycles: u64) -> Result<(), &'static str>
where
    F: FnOnce(),
{
    let start = read_timestamp();
    f();
    let end = read_timestamp();
    
    if end.saturating_sub(start) > timeout_cycles {
        Err("Test timeout exceeded")
    } else {
        Ok(())
    }
}

#[macro_export]
macro_rules! test_timeout {
    ($timeout_ms:expr, $body:expr) => {{
        use $crate::test_framework::run_with_timeout;
        // Convert ms to cycles (approximate)
        let timeout_cycles = $timeout_ms * 2_000_000; // Assuming 2GHz
        run_with_timeout(|| $body, timeout_cycles)
    }};
}

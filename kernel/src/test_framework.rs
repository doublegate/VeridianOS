//! No-std test framework for VeridianOS kernel
//!
//! This module provides testing infrastructure that works in a no_std
//! environment by using serial output and QEMU exit codes to report test
//! results.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::{panic::PanicInfo, time::Duration};

use crate::{error::KernelError, serial_print, serial_println};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

/// Trait that all testable functions must implement
pub trait Testable {
    fn run(&self) -> Result<(), KernelError>;
}

impl<T> Testable for T
where
    T: Fn() -> Result<(), KernelError>,
{
    fn run(&self) -> Result<(), KernelError> {
        serial_print!("{}...\t", core::any::type_name::<T>());
        match self() {
            Ok(()) => {
                serial_println!("[ok]");
                Ok(())
            }
            Err(e) => {
                serial_println!("[failed]: {}", e);
                Err(e)
            }
        }
    }
}

/// Custom test runner for kernel tests
#[cfg(test)]
pub fn test_runner(tests: &[&dyn Testable]) -> ! {
    serial_println!("Running {} tests", tests.len());
    let mut passed = 0;
    let mut failed = 0;

    for test in tests {
        match test.run() {
            Ok(()) => passed += 1,
            Err(e) => {
                failed += 1;
                serial_println!("[ERROR] Test failed: {}", e);
            }
        }
    }

    serial_println!("\nTest Results: {} passed, {} failed", passed, failed);

    if failed == 0 {
        exit_qemu(QemuExitCode::Success);
    } else {
        exit_qemu(QemuExitCode::Failed);
    }
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
    // SAFETY: Writing to I/O port 0xf4 is the QEMU debug exit device.
    // This triggers QEMU to exit with the given code. The function is
    // marked as noreturn (-> !), so unreachable_unchecked is valid
    // since QEMU terminates before the instruction after the port write.
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
        // SAFETY: PSCI SYSTEM_OFF (0x84000008) is a standard ARM PSCI
        // call that powers off the system. The HVC instruction traps to
        // the hypervisor (QEMU). This is noreturn since the VM terminates.
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
        // SAFETY: SBI shutdown (EID 8) is a standard RISC-V SBI call
        // that powers off the system. The ecall traps to the SBI
        // firmware (OpenSBI in QEMU). This is noreturn since the VM
        // terminates.
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

/// Macro to define kernel tests
#[macro_export]
macro_rules! kernel_test {
    ($name:ident, $test:expr) => {
        #[test_case]
        const $name: &dyn $crate::test_framework::Testable =
            &|| -> Result<(), $crate::error::KernelError> { $test };
    };
}

/// Helper macro for creating test modules
#[macro_export]
macro_rules! test_module {
    ($name:ident, $($test_name:ident => $test_fn:expr),* $(,)?) => {
        #[cfg(test)]
        mod $name {
            use super::*;

            $(
                kernel_test!($test_name, $test_fn);
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
///
/// Intentionally kept available for on-demand benchmark binaries.
#[allow(dead_code)]
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

/// Get current timestamp in nanoseconds (architecture-specific).
///
/// Delegates to the centralized [`crate::arch::entropy::read_timestamp`] which
/// provides implementations for x86_64 (RDTSC), AArch64 (CNTVCT_EL0), and
/// RISC-V (rdcycle).
#[inline(always)]
pub fn read_timestamp() -> u64 {
    crate::arch::entropy::read_timestamp()
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

impl Default for BenchmarkRunner {
    fn default() -> Self {
        Self::new()
    }
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
            use $crate::test_framework::{cycles_to_ns, read_timestamp, BenchmarkRunner};
            let runner = BenchmarkRunner::new();
            let result = runner.run_benchmark(stringify!($name), || $body);
            serial_println!(
                "  Min: {} ns, Max: {} ns",
                result.min_time_ns,
                result.max_time_ns
            );
        }
    };
}

// ===== Test Registry =====

/// Test registry for collecting and running kernel tests.
///
/// Used by the `testing` feature when test binaries register
/// their tests via the `register_test!` macro.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub struct TestRegistry {
    tests: Vec<(&'static str, fn())>,
    benchmarks: Vec<(&'static str, fn())>,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
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
#[allow(dead_code)]
static TEST_REGISTRY: spin::Mutex<Option<TestRegistry>> = spin::Mutex::new(None);

/// Initialize the test registry. Called once before tests run.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub fn init_test_registry() {
    *TEST_REGISTRY.lock() = Some(TestRegistry::new());
}

/// Execute a closure with the test registry (mutable access)
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub fn with_test_registry<R, F: FnOnce(&mut TestRegistry) -> R>(f: F) -> Option<R> {
    TEST_REGISTRY.lock().as_mut().map(f)
}

#[cfg(feature = "alloc")]
#[macro_export]
macro_rules! register_test {
    ($name:ident) => {
        #[allow(non_snake_case)]
        #[used]
        #[link_section = ".test_registry"]
        static $name: fn() = || {
            $crate::test_framework::with_test_registry(|registry| {
                registry.register_test(stringify!($name), $name);
            });
        };
    };
}

// ===== Test Timeout Support =====

/// Run a test with a timeout (uses architecture-specific timer)
///
/// Available for test binaries that need timeout enforcement.
#[allow(dead_code)]
pub fn run_with_timeout<F>(f: F, timeout_cycles: u64) -> Result<(), KernelError>
where
    F: FnOnce(),
{
    let start = read_timestamp();
    f();
    let end = read_timestamp();

    if end.saturating_sub(start) > timeout_cycles {
        Err(KernelError::Timeout {
            operation: "test execution",
            duration_ms: timeout_cycles / 2_000_000, // Approximate conversion from cycles to ms
        })
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

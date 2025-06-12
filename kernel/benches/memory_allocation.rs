//! Memory allocation benchmark for VeridianOS
//!
//! Measures the speed of memory allocation operations

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

extern crate alloc;

use core::panic::PanicInfo;

use veridian_kernel::{
    bench::{cycles_to_ns, read_timestamp},
    serial_println,
};

const MEMORY_ALLOC_TARGET_NS: u64 = 1000; // 1μs target
const ITERATIONS: u64 = 1000;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("Memory Allocation Benchmark");
    serial_println!("===========================");
    serial_println!(
        "Target: < {} ns ({}μs)",
        MEMORY_ALLOC_TARGET_NS,
        MEMORY_ALLOC_TARGET_NS / 1000
    );
    serial_println!();

    // Initialize a simple allocator for benchmarking
    init_test_allocator();

    // Run different allocation scenarios
    let small_alloc_result = benchmark_small_allocation();
    let medium_alloc_result = benchmark_medium_allocation();
    let large_alloc_result = benchmark_large_allocation();
    let free_result = benchmark_deallocation();

    // Print results
    serial_println!("\nResults:");
    serial_println!("--------");
    print_result("Small Alloc (64B)", &small_alloc_result);
    print_result("Medium Alloc (4KB)", &medium_alloc_result);
    print_result("Large Alloc (64KB)", &large_alloc_result);
    print_result("Deallocation", &free_result);

    // Check if we meet targets
    serial_println!("\nTarget Analysis:");
    serial_println!("----------------");
    check_target("Small Alloc", &small_alloc_result, MEMORY_ALLOC_TARGET_NS);
    check_target("Medium Alloc", &medium_alloc_result, MEMORY_ALLOC_TARGET_NS);
    check_target(
        "Large Alloc",
        &large_alloc_result,
        MEMORY_ALLOC_TARGET_NS * 2,
    ); // Allow 2x for large
    check_target("Deallocation", &free_result, MEMORY_ALLOC_TARGET_NS);

    // Exit with success
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Success)
}

fn init_test_allocator() {
    // In Phase 0, we're using a simple bump allocator
    // This establishes baseline for the hybrid allocator in Phase 1
    // Note: The global allocator is defined in lib.rs and automatically
    // initialized
}

fn benchmark_small_allocation() -> BenchmarkResult {
    // Benchmark small allocations (64 bytes)
    use alloc::vec::Vec;

    let start = read_timestamp();
    for _ in 0..ITERATIONS {
        let v: Vec<u8> = Vec::with_capacity(64);
        // Force the allocation to not be optimized away
        // Use a volatile read to prevent optimization
        unsafe {
            core::ptr::read_volatile(&v as *const _);
        }
    }
    let end = read_timestamp();

    let total_cycles = end - start;
    let avg_cycles = total_cycles / ITERATIONS;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Small Allocation"),
        iterations: ITERATIONS,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: avg_ns,
        max_time_ns: avg_ns,
    }
}

fn benchmark_medium_allocation() -> BenchmarkResult {
    // Benchmark medium allocations (4KB - typical page size)
    use alloc::vec::Vec;

    let start = read_timestamp();
    for _ in 0..ITERATIONS {
        let v: Vec<u8> = Vec::with_capacity(4096);
        unsafe {
            core::ptr::read_volatile(&v as *const _);
        }
    }
    let end = read_timestamp();

    let total_cycles = end - start;
    let avg_cycles = total_cycles / ITERATIONS;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Medium Allocation"),
        iterations: ITERATIONS,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: avg_ns,
        max_time_ns: avg_ns,
    }
}

fn benchmark_large_allocation() -> BenchmarkResult {
    // Benchmark large allocations (64KB)
    use alloc::vec::Vec;

    let iterations = ITERATIONS / 10;
    let start = read_timestamp();
    for _ in 0..iterations {
        // Fewer iterations for large allocs
        let v: Vec<u8> = Vec::with_capacity(65536);
        unsafe {
            core::ptr::read_volatile(&v as *const _);
        }
    }
    let end = read_timestamp();

    let total_cycles = end - start;
    let avg_cycles = total_cycles / iterations;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Large Allocation"),
        iterations,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: avg_ns,
        max_time_ns: avg_ns,
    }
}

fn benchmark_deallocation() -> BenchmarkResult {
    use alloc::vec::Vec;

    // Pre-allocate vectors for deallocation benchmark
    let mut vectors: Vec<Vec<u8>> = Vec::with_capacity(ITERATIONS as usize);
    for _ in 0..ITERATIONS {
        vectors.push(Vec::with_capacity(64));
    }

    // Benchmark deallocation
    let mut total_cycles = 0u64;
    let mut min_cycles = u64::MAX;
    let mut max_cycles = 0u64;

    for v in vectors {
        let start = read_timestamp();
        drop(v);
        let end = read_timestamp();
        let cycles = end.saturating_sub(start);
        total_cycles += cycles;
        min_cycles = min_cycles.min(cycles);
        max_cycles = max_cycles.max(cycles);
    }

    let avg_cycles = total_cycles / ITERATIONS;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Deallocation"),
        iterations: ITERATIONS,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: cycles_to_ns(min_cycles),
        max_time_ns: cycles_to_ns(max_cycles),
    }
}

fn print_result(name: &str, result: &BenchmarkResult) {
    serial_println!(
        "{:<20} Avg: {:>6} ns, Min: {:>6} ns, Max: {:>6} ns",
        name,
        result.avg_time_ns,
        result.min_time_ns,
        result.max_time_ns
    );
}

fn check_target(name: &str, result: &BenchmarkResult, target_ns: u64) {
    if result.avg_time_ns < target_ns {
        serial_println!(
            "{:<20} ✓ PASS ({}ns < {}ns)",
            name,
            result.avg_time_ns,
            target_ns
        );
    } else {
        serial_println!(
            "{:<20} ✗ FAIL ({}ns > {}ns)",
            name,
            result.avg_time_ns,
            target_ns
        );
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("Benchmark panic: {}", info);
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Failed)
}

//! Memory allocation benchmark for VeridianOS
//!
//! Measures the speed of memory allocation operations

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

extern crate alloc;

use core::panic::PanicInfo;
use veridian_kernel::{benchmark, serial_println};
use veridian_kernel::bench::{BenchmarkResult, cycles_to_ns, read_timestamp};
use alloc::vec::Vec;

const MEMORY_ALLOC_TARGET_NS: u64 = 1000; // 1μs target
const ITERATIONS: u64 = 1000;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("Memory Allocation Benchmark");
    serial_println!("===========================");
    serial_println!("Target: < {} ns ({}μs)", MEMORY_ALLOC_TARGET_NS, MEMORY_ALLOC_TARGET_NS / 1000);
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
    check_target("Large Alloc", &large_alloc_result, MEMORY_ALLOC_TARGET_NS * 2); // Allow 2x for large
    check_target("Deallocation", &free_result, MEMORY_ALLOC_TARGET_NS);
    
    // Exit with success
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Success);
}

fn init_test_allocator() {
    // In Phase 0, we're using a simple bump allocator
    // This establishes baseline for the hybrid allocator in Phase 1
    use linked_list_allocator::LockedHeap;
    
    #[global_allocator]
    static ALLOCATOR: LockedHeap = LockedHeap::empty();
    
    // Initialize with 1MB heap
    const HEAP_SIZE: usize = 1024 * 1024;
    static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
    
    unsafe {
        ALLOCATOR.lock().init(HEAP.as_mut_ptr() as usize, HEAP_SIZE);
    }
}

fn benchmark_small_allocation() -> BenchmarkResult {
    // Benchmark small allocations (64 bytes)
    benchmark!("Small Allocation", ITERATIONS, {
        let v: Vec<u8> = Vec::with_capacity(64);
        // Force the allocation to not be optimized away
        core::hint::black_box(v);
    })
}

fn benchmark_medium_allocation() -> BenchmarkResult {
    // Benchmark medium allocations (4KB - typical page size)
    benchmark!("Medium Allocation", ITERATIONS, {
        let v: Vec<u8> = Vec::with_capacity(4096);
        core::hint::black_box(v);
    })
}

fn benchmark_large_allocation() -> BenchmarkResult {
    // Benchmark large allocations (64KB)
    benchmark!("Large Allocation", ITERATIONS / 10, { // Fewer iterations for large allocs
        let v: Vec<u8> = Vec::with_capacity(65536);
        core::hint::black_box(v);
    })
}

fn benchmark_deallocation() -> BenchmarkResult {
    // Pre-allocate vectors for deallocation benchmark
    let mut vectors: Vec<Vec<u8>> = Vec::with_capacity(ITERATIONS as usize);
    for _ in 0..ITERATIONS {
        vectors.push(Vec::with_capacity(64));
    }
    
    // Benchmark deallocation
    let mut times = Vec::with_capacity(ITERATIONS as usize);
    
    for v in vectors {
        let start = read_timestamp();
        drop(v);
        let end = read_timestamp();
        times.push(cycles_to_ns(end.saturating_sub(start)));
    }
    
    BenchmarkResult::new("Deallocation".to_string(), &times)
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
    if result.meets_target(target_ns) {
        serial_println!("{:<20} ✓ PASS ({}ns < {}ns)", name, result.avg_time_ns, target_ns);
    } else {
        serial_println!("{:<20} ✗ FAIL ({}ns > {}ns)", name, result.avg_time_ns, target_ns);
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("Benchmark panic: {}", info);
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Failed);
}
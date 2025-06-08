//! IPC latency benchmark for VeridianOS
//!
//! Measures the baseline latency for inter-process communication

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

extern crate alloc;

use core::panic::PanicInfo;
use veridian_kernel::{benchmark, serial_println};
use veridian_kernel::bench::{BenchmarkResult, cycles_to_ns, read_timestamp};

const IPC_TARGET_NS: u64 = 5000; // 5μs target
const ITERATIONS: u64 = 1000;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("IPC Latency Benchmark");
    serial_println!("=====================");
    serial_println!("Target: < {} ns ({}μs)", IPC_TARGET_NS, IPC_TARGET_NS / 1000);
    serial_println!();
    
    // Run different IPC scenarios
    let small_msg_result = benchmark_small_message_ipc();
    let large_msg_result = benchmark_large_message_ipc();
    let capability_result = benchmark_capability_passing();
    
    // Print results
    serial_println!("\nResults:");
    serial_println!("--------");
    print_result("Small Message IPC", &small_msg_result);
    print_result("Large Message IPC", &large_msg_result);
    print_result("Capability Passing", &capability_result);
    
    // Check if we meet targets
    serial_println!("\nTarget Analysis:");
    serial_println!("----------------");
    check_target("Small Message", &small_msg_result, IPC_TARGET_NS);
    check_target("Large Message", &large_msg_result, IPC_TARGET_NS);
    check_target("Capability", &capability_result, IPC_TARGET_NS);
    
    // Exit with success
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Success);
}

fn benchmark_small_message_ipc() -> BenchmarkResult {
    // Simulate small message IPC (≤64 bytes)
    benchmark!("Small Message IPC", ITERATIONS, {
        // In Phase 0, we just measure the overhead of the measurement itself
        // This establishes a baseline for Phase 1
        unsafe {
            // Simulate register-based message passing
            core::arch::asm!(
                "mov rax, 0x1234",
                "mov rbx, 0x5678",
                "mov rcx, 0x9ABC",
                "mov rdx, 0xDEF0",
                out("rax") _,
                out("rbx") _,
                out("rcx") _,
                out("rdx") _,
            );
        }
    })
}

fn benchmark_large_message_ipc() -> BenchmarkResult {
    // Simulate large message IPC (>64 bytes)
    benchmark!("Large Message IPC", ITERATIONS, {
        // Simulate memory-based message passing
        let mut buffer = [0u8; 256];
        unsafe {
            // Simulate copying data
            let src = 0x1000 as *const u8;
            let dst = buffer.as_mut_ptr();
            
            // Small memcpy simulation
            for i in 0..8 {
                *dst.add(i) = *src.add(i);
            }
        }
    })
}

fn benchmark_capability_passing() -> BenchmarkResult {
    // Simulate capability token passing
    benchmark!("Capability Passing", ITERATIONS, {
        // Simulate capability lookup and validation
        let cap_id = 0x1234u64;
        let _validated = validate_capability(cap_id);
    })
}

#[inline(never)]
fn validate_capability(cap_id: u64) -> bool {
    // Simulate O(1) capability lookup
    // In real implementation, this would check a capability table
    cap_id != 0
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
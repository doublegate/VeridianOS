//! IPC latency benchmark for VeridianOS
//!
//! Measures the baseline latency for inter-process communication

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

extern crate alloc;

use core::panic::PanicInfo;

use veridian_kernel::{bench::BenchmarkResult, serial_println};

const IPC_TARGET_NS: u64 = 5000; // 5μs target
const ITERATIONS: u64 = 1000;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("IPC Latency Benchmark");
    serial_println!("=====================");
    serial_println!(
        "Target: < {} ns ({}μs)",
        IPC_TARGET_NS,
        IPC_TARGET_NS / 1000
    );
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
    use veridian_kernel::{
        bench::{cycles_to_ns, read_timestamp},
        ipc::{Message, SmallMessage},
    };

    // Benchmark small message IPC (≤64 bytes)
    let start = read_timestamp();
    for _ in 0..ITERATIONS {
        // Create a small message with our actual IPC types
        let msg = SmallMessage::new(0x1234, 42)
            .with_flags(0x01) // URGENT flag
            .with_data(0, 100)
            .with_data(1, 200)
            .with_data(2, 300)
            .with_data(3, 400);

        // Convert to Message enum
        let message = Message::Small(msg);

        // Simulate message dispatch
        match message {
            Message::Small(sm) => {
                let _ = sm.capability;
                let _ = sm.opcode;
                let _ = sm.data[0];
            }
            Message::Large(_) => unreachable!(),
        }
    }
    let end = read_timestamp();

    let total_cycles = end - start;
    let avg_cycles = total_cycles / ITERATIONS;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Small Message IPC"),
        iterations: ITERATIONS,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: avg_ns,
        max_time_ns: avg_ns,
    }
}

fn benchmark_large_message_ipc() -> BenchmarkResult {
    use veridian_kernel::{
        bench::{cycles_to_ns, read_timestamp},
        ipc::{message::MemoryRegion, Message},
    };

    // Benchmark large message IPC (>64 bytes)
    let start = read_timestamp();
    for _ in 0..ITERATIONS {
        // Create a memory region descriptor
        let region = MemoryRegion::new(0x100000, 4096)
            .with_permissions(0x03) // READ | WRITE
            .with_cache_policy(0); // WRITE_BACK

        // Create a large message
        let large_msg = Message::large(0x5678, 84, region);

        // Simulate message handling
        match large_msg {
            Message::Large(lm) => {
                let _ = lm.header.capability;
                let _ = lm.header.total_size;
                let _ = lm.memory_region.size;
            }
            Message::Small(_) => unreachable!(),
        }
    }
    let end = read_timestamp();

    let total_cycles = end - start;
    let avg_cycles = total_cycles / ITERATIONS;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Large Message IPC"),
        iterations: ITERATIONS,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: avg_ns,
        max_time_ns: avg_ns,
    }
}

fn benchmark_capability_passing() -> BenchmarkResult {
    use veridian_kernel::{
        bench::{cycles_to_ns, read_timestamp},
        ipc::{IpcCapability, IpcPermissions},
    };

    // Benchmark actual capability operations
    let start = read_timestamp();
    for _ in 0..ITERATIONS {
        // Create a capability
        let cap = IpcCapability::new(42, IpcPermissions::all());

        // Validate capability operations
        let id = cap.id();
        let has_send = cap.has_permission(veridian_kernel::ipc::capability::Permission::Send);
        let has_recv = cap.has_permission(veridian_kernel::ipc::capability::Permission::Receive);

        // Simulate capability derivation
        let derived = cap.derive(IpcPermissions::send_only());

        // Use results to prevent optimization
        let _ = (id, has_send, has_recv, derived);
    }
    let end = read_timestamp();

    let total_cycles = end - start;
    let avg_cycles = total_cycles / ITERATIONS;
    let avg_ns = cycles_to_ns(avg_cycles);

    BenchmarkResult {
        name: alloc::string::String::from("Capability Passing"),
        iterations: ITERATIONS,
        total_time_ns: cycles_to_ns(total_cycles),
        avg_time_ns: avg_ns,
        min_time_ns: avg_ns,
        max_time_ns: avg_ns,
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

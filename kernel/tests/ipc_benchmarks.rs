//! IPC Performance Benchmarks
//!
//! Validates performance claims using custom no_std benchmark framework

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use veridian_kernel::{
    ipc::{self},
    serial_println,
};

#[path = "common/mod.rs"]
mod common;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    common::init_test_env("IPC Benchmarks");
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

// ===== Message Creation Benchmarks =====

#[test_case]
fn bench_small_message_creation() {
    serial_println!("bench_small_message_creation...");
    let start = read_timestamp();
    for _ in 0..1000 {
        let msg = Message::small(0, 1);
        black_box(msg);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!(
        "  Average: {} cycles ({} ns)",
        avg_cycles,
        cycles_to_ns(avg_cycles)
    );
    serial_println!("[ok]");
}

#[test_case]
fn bench_large_message_creation() {
    serial_println!("bench_large_message_creation...");
    let data = [0u8; 1024];
    let region = veridian_kernel::ipc::message::MemoryRegion::new(0, data.len() as u64);
    let start = read_timestamp();
    for _ in 0..1000 {
        let msg = Message::large(0, 1, region);
        black_box(msg);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!(
        "  Average: {} cycles ({} ns)",
        avg_cycles,
        cycles_to_ns(avg_cycles)
    );
    serial_println!("[ok]");
}

// ===== Registry Operation Benchmarks =====

#[test_case]
fn bench_endpoint_creation() {
    ipc::init();
    let runner = BenchmarkRunner::new();
    let mut counter = 0u64;

    let result = runner.run_benchmark("endpoint_creation", || {
        let (id, cap) = create_endpoint(ProcessId(counter)).expect("Failed to create endpoint");
        counter += 1;
        black_box((id, cap));
    });

    // Endpoint creation should be fast (<1μs)
    serial_println!("  Average: {} ns", result.avg_time_ns);
    assert!(
        result.avg_time_ns < 1000,
        "Endpoint creation too slow: {} ns",
        result.avg_time_ns
    );
    serial_println!("[ok]");
}

#[test_case]
fn bench_channel_creation() {
    ipc::init();
    let runner = BenchmarkRunner::new();
    let mut counter = 0u64;

    let result = runner.run_benchmark("channel_creation", || {
        let (send_id, recv_id, send_cap, recv_cap) =
            create_channel(ProcessId(counter), 100).expect("Failed to create channel");
        counter += 1;
        black_box((send_id, recv_id, send_cap, recv_cap));
    });

    // Channel creation should be reasonably fast (<5μs)
    serial_println!("  Average: {} ns", result.avg_time_ns);
    assert!(
        result.avg_time_ns < 5000,
        "Channel creation too slow: {} ns",
        result.avg_time_ns
    );
    serial_println!("[ok]");
}

// ===== Async Channel Benchmarks =====

#[test_case]
fn bench_async_channel_send_receive() {
    ipc::init();
    let channel = AsyncChannel::new(1, ProcessId(1), 1000);
    let msg = Message::small(0, 1);

    let runner = BenchmarkRunner::new();
    let result = runner.run_benchmark("async_send_receive", || {
        channel.send_async(msg.clone()).expect("Send failed");
        let received = channel.receive_async().expect("Receive failed");
        black_box(received);
    });

    // Single message round-trip should be very fast (<1μs)
    serial_println!("  Average: {} ns", result.avg_time_ns);
    assert!(
        result.avg_time_ns < 1000,
        "Async send/receive too slow: {} ns",
        result.avg_time_ns
    );
    serial_println!("[ok]");
}

#[test_case]
fn bench_async_channel_throughput() {
    ipc::init();
    let channel = AsyncChannel::new(1, ProcessId(1), 10000);
    let messages: Vec<_> = (0..1000).map(|i| Message::small(0, i as u32)).collect();

    let start = read_timestamp();

    // Send all messages
    for msg in &messages {
        channel.send_async(msg.clone()).expect("Send failed");
    }

    // Receive all messages
    let mut received = 0;
    while let Ok(_) = channel.receive_async() {
        received += 1;
        if received >= 1000 {
            break;
        }
    }

    let elapsed = read_timestamp() - start;
    let throughput = (1000 * 1_000_000_000) / cycles_to_ns(elapsed);

    serial_println!("Async throughput: {} msgs/sec", throughput);
    assert!(throughput > 100_000); // Should handle >100k msgs/sec
    serial_println!("[ok]");
}

// ===== Shared Memory Benchmarks =====

#[test_case]
fn bench_shared_region_creation() {
    serial_println!("bench_shared_region_creation...");
    ipc::init();
    let start = read_timestamp();
    for i in 1..101 {
        let region = SharedRegion::new(i, 4096, Permissions::READ_WRITE);
        black_box(region);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 100;
    serial_println!(
        "  Average: {} cycles ({} ns)",
        avg_cycles,
        cycles_to_ns(avg_cycles)
    );
    serial_println!("[ok]");
}

// ===== Capability Benchmarks =====

#[test_case]
fn bench_capability_validation() {
    ipc::init();
    let (_, cap) = create_endpoint(ProcessId(1)).expect("Failed to create endpoint");

    let runner = BenchmarkRunner::new();
    let result = runner.run_benchmark("capability_validation", || {
        let valid = validate_capability(ProcessId(1), &cap).is_ok();
        black_box(valid);
    });

    // Capability validation should be O(1) and very fast
    serial_println!("  Average: {} ns", result.avg_time_ns);
    assert!(
        result.avg_time_ns < 100,
        "Capability validation too slow: {} ns",
        result.avg_time_ns
    );
    serial_println!("[ok]");
}

// ===== Fast Path vs Slow Path =====

#[test_case]
fn bench_fast_path_vs_slow_path() {
    ipc::init();

    // Benchmark small message (fast path)
    let runner = BenchmarkRunner::new();
    let small_msg = Message::small(0, 42);

    let fast_result = runner.run_benchmark("fast_path_message", || {
        let msg = small_msg.clone();
        black_box(msg);
    });

    // Benchmark large message (slow path)
    let data = [0u8; 4096];
    let region = veridian_kernel::ipc::message::MemoryRegion::new(0, data.len() as u64);
    let large_msg = Message::large(0, 42, region);

    let slow_result = runner.run_benchmark("slow_path_message", || {
        let msg = large_msg.clone();
        black_box(msg);
    });

    serial_println!(
        "Fast path: {} ns, Slow path: {} ns",
        fast_result.avg_time_ns,
        slow_result.avg_time_ns
    );

    // Fast path should be significantly faster
    assert!(fast_result.avg_time_ns < slow_result.avg_time_ns / 2);
    serial_println!("[ok]");
}

// ===== Performance Statistics =====

#[test_case]
fn test_ipc_performance_stats() {
    ipc::init();

    // Perform some operations
    for i in 0..100 {
        let _ = create_endpoint(ProcessId(i));
    }

    // Check performance stats
    let report = IPC_PERF_STATS.get_report();
    serial_println!("IPC Performance Summary:");
    serial_println!("  Total operations: {}", report.total_operations);
    serial_println!("  Average latency: {} ns", report.average_latency_ns);
    serial_println!("  Min latency: {} ns", report.min_latency_ns);
    serial_println!("  Max latency: {} ns", report.max_latency_ns);

    // Verify performance meets targets
    assert!(report.average_latency_ns < 1000); // <1μs average
    serial_println!("[ok]");
}

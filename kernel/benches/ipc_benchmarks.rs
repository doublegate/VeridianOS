//! IPC Performance Benchmarks
//!
//! Validates performance claims and provides regression testing

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

#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

#[test_case]
fn bench_small_message_creation() {
    serial_println!("bench_small_message_creation...");
    let start = read_timestamp();
    for _ in 0..1000 {
        let msg = Message::small(0, 1);
        core::hint::black_box(msg);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
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
        core::hint::black_box(msg);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_endpoint_creation() {
    serial_println!("bench_endpoint_creation...");
    ipc::init();
    
    let start = read_timestamp();
    for i in 0..100 {
        let (id, cap) = create_endpoint(ProcessId(i)).expect("Failed to create endpoint");
        core::hint::black_box((id, cap));
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 100;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_channel_creation() {
    serial_println!("bench_channel_creation...");
    ipc::init();
    
    let start = read_timestamp();
    for i in 0..100 {
        let (send_id, recv_id, send_cap, recv_cap) =
            create_channel(ProcessId(i), 100).expect("Failed to create channel");
        core::hint::black_box((send_id, recv_id, send_cap, recv_cap));
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 100;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_async_channel_send_receive() {
    serial_println!("bench_async_channel_send_receive...");
    ipc::init();
    let channel = AsyncChannel::new(1, ProcessId(1), 1000); // id=1, owner=1, capacity=1000
    let msg = Message::small(0, 1);
    
    let start = read_timestamp();
    for _ in 0..1000 {
        channel.send_async(msg.clone()).expect("Send failed");
        let received = channel.receive_async().expect("Receive failed");
        core::hint::black_box(received);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_async_channel_throughput() {
    serial_println!("bench_async_channel_throughput...");
    ipc::init();
    let channel = AsyncChannel::new(1, ProcessId(1), 10000); // id=1, owner=1, capacity=10000
    let messages: Vec<_> = (0..1000).map(|i| Message::small(0, i as u32)).collect();
    
    let start = read_timestamp();
    // Send all messages
    for msg in &messages {
        channel.send_async(msg.clone()).expect("Send failed");
    }
    
    // Receive all messages
    let mut count = 0;
    while let Ok(_) = channel.receive_async() {
        count += 1;
        if count >= messages.len() {
            break;
        }
    }
    let elapsed = read_timestamp() - start;
    let throughput = (1000 * 1_000_000_000) / cycles_to_ns(elapsed);
    serial_println!("  Throughput: {} msgs/sec", throughput);
    serial_println!("[ok]");
}

#[test_case]
fn bench_shared_region_creation() {
    serial_println!("bench_shared_region_creation...");
    ipc::init();
    
    let start = read_timestamp();
    for i in 1..101 {
        let region = SharedRegion::new(i, 4096, Permissions::READ_WRITE);
        core::hint::black_box(region);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 100;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_capability_creation() {
    serial_println!("bench_capability_creation...");
    ipc::init();
    let region = SharedRegion::new(1, 4096, Permissions::READ_WRITE);
    
    let start = read_timestamp();
    for i in 2..102 {
        let cap = region.create_capability(ProcessId(i), TransferMode::Share);
        core::hint::black_box(cap);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 100;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_fast_path_message_creation() {
    serial_println!("bench_fast_path_message_creation...");
    // Benchmark small message creation which would use fast path
    let _data = [1u8, 2, 3, 4, 5, 6, 7, 8];
    
    let start = read_timestamp();
    for _ in 0..1000 {
        let msg = Message::small(0, 1);
        core::hint::black_box(msg);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_message_clone() {
    serial_println!("bench_message_clone...");
    let small_msg = Message::small(0, 1);
    
    let start = read_timestamp();
    for _ in 0..1000 {
        let cloned = small_msg.clone();
        core::hint::black_box(cloned);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_registry_lookup() {
    serial_println!("bench_registry_lookup...");
    ipc::init();
    
    // Create some endpoints to look up
    let mut endpoints = Vec::new();
    let mut capabilities = Vec::new();
    for i in 0..100 {
        let (id, cap) = create_endpoint(ProcessId(i)).expect("Failed to create endpoint");
        endpoints.push(id);
        capabilities.push(cap);
    }
    
    let start = read_timestamp();
    for i in 0..1000 {
        let cap_idx = i % capabilities.len();
        let cap = &capabilities[cap_idx];
        // Validate capability as a proxy for registry lookup
        let result = validate_capability(cap.owner(), cap);
        core::hint::black_box(result);
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_performance_measurement_overhead() {
    serial_println!("bench_performance_measurement_overhead...");
    
    let start = read_timestamp();
    for _ in 0..1000 {
        let (result, cycles) = measure_ipc_operation(|| {
            // Minimal operation
            42
        });
        core::hint::black_box((result, cycles));
    }
    let elapsed = read_timestamp() - start;
    let avg_cycles = elapsed / 1000;
    serial_println!("  Average: {} cycles ({} ns)", avg_cycles, cycles_to_ns(avg_cycles));
    serial_println!("[ok]");
}

#[test_case]
fn bench_zero_copy_vs_regular_copy() {
    serial_println!("bench_zero_copy_vs_regular_copy...");
    ipc::init();
    
    // Test data
    let data = [0u8; 4096];
    
    // First, benchmark regular message copy
    serial_println!("  Benchmarking regular message copy...");
    let mut copy_results = Vec::new();
    for _ in 0..10 {
        let start = read_timestamp();
        let region = veridian_kernel::ipc::message::MemoryRegion::new(0, data.len() as u64);
        let msg = Message::large(0, 1, region);
        let _cloned = msg.clone();
        let elapsed = read_timestamp() - start;
        copy_results.push(elapsed);
    }
    
    // Then benchmark zero-copy
    serial_println!("  Benchmarking zero-copy transfer...");
    let region = SharedRegion::new(1, 4096, Permissions::READ_WRITE);
    let mut zero_copy_results = Vec::new();
    for i in 0..10 {
        let start = read_timestamp();
        let _cap = region.create_capability(ProcessId(i + 2), TransferMode::Share);
        let elapsed = read_timestamp() - start;
        zero_copy_results.push(elapsed);
    }
    
    // Compare results
    let avg_copy: u64 = copy_results.iter().sum::<u64>() / copy_results.len() as u64;
    let avg_zero_copy: u64 = zero_copy_results.iter().sum::<u64>() / zero_copy_results.len() as u64;
    
    serial_println!("  Average copy time: {} cycles", avg_copy);
    serial_println!("  Average zero-copy time: {} cycles", avg_zero_copy);
    serial_println!("  Zero-copy speedup: {}x", avg_copy / avg_zero_copy.max(1));
    serial_println!("[ok]");
}

#[test_case]
fn bench_ipc_latency_targets() {
    serial_println!("bench_ipc_latency_targets...");
    ipc::init();
    
    // Warm up
    let (_send_id, _recv_id, _, _) = create_channel(ProcessId(1), 100).unwrap();
    
    // Measure various IPC operations
    serial_println!("  Measuring IPC operation latencies...");
    
    // Small message latency
    let small_msg = Message::small(0, 1);
    let (_, small_cycles) = measure_ipc_operation(|| {
        let _ = small_msg.clone();
    });
    let small_ns = cycles_to_ns(small_cycles);
    serial_println!("  Small message: {} cycles ({} ns)", small_cycles, small_ns);
    
    // Large message latency
    let large_data = [0u8; 1024];
    let region = veridian_kernel::ipc::message::MemoryRegion::new(0, large_data.len() as u64);
    let large_msg = Message::large(0, 1, region);
    let (_, large_cycles) = measure_ipc_operation(|| {
        let _ = large_msg.clone();
    });
    let large_ns = cycles_to_ns(large_cycles);
    serial_println!("  Large message: {} cycles ({} ns)", large_cycles, large_ns);
    
    // Capability creation
    let (endpoint_id, _) = create_endpoint(ProcessId(99)).unwrap();
    let (_, cap_cycles) = measure_ipc_operation(|| {
        let _ = IpcCapability::new(endpoint_id, IpcPermissions::all());
    });
    let cap_ns = cycles_to_ns(cap_cycles);
    serial_println!("  Capability creation: {} cycles ({} ns)", cap_cycles, cap_ns);
    
    // Check against targets
    let report = IPC_PERF_STATS.get_report();
    if report.meets_phase1_targets() {
        serial_println!("  ✓ Meets Phase 1 targets (<5μs)");
    }
    if report.meets_phase5_targets() {
        serial_println!("  ✓ Meets Phase 5 targets (<1μs)");
    }
    
    serial_println!("[ok]");
}
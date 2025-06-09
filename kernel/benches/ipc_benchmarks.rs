//! IPC Performance Benchmarks
//!
//! Validates performance claims and provides regression testing

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::benchmark_runner)]
#![reexport_test_harness_main = "benchmark_main"]

extern crate alloc;

use alloc::vec::Vec;

use veridian_kernel::{
    bench::Bencher,
    ipc::{
        self, create_channel, create_endpoint, cycles_to_ns, measure_ipc_operation, read_timestamp,
        validate_capability, AsyncChannel, IpcCapability, IpcPermissions, Message, Permissions,
        SharedRegion, TransferMode, IPC_PERF_STATS,
    },
    serial_print, serial_println,
};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    benchmark_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

#[bench]
fn bench_small_message_creation(b: &mut Bencher) {
    b.iter(|| {
        let msg = Message::small(b"test");
        core::hint::black_box(msg);
    });
}

#[bench]
fn bench_large_message_creation(b: &mut Bencher) {
    let data = [0u8; 1024];
    b.iter(|| {
        let msg = Message::large(&data);
        core::hint::black_box(msg);
    });
}

#[bench]
fn bench_endpoint_creation(b: &mut Bencher) {
    ipc::init();
    let mut counter = 0u64;

    b.iter(|| {
        let (id, cap) = create_endpoint(counter).expect("Failed to create endpoint");
        counter += 1;
        core::hint::black_box((id, cap));
    });
}

#[bench]
fn bench_channel_creation(b: &mut Bencher) {
    ipc::init();
    let mut counter = 0u64;

    b.iter(|| {
        let (send_id, recv_id, send_cap, recv_cap) =
            create_channel(counter, 100).expect("Failed to create channel");
        counter += 1;
        core::hint::black_box((send_id, recv_id, send_cap, recv_cap));
    });
}

#[bench]
fn bench_async_channel_send_receive(b: &mut Bencher) {
    ipc::init();
    let channel = AsyncChannel::new(1, 1000);
    let msg = Message::small(b"bench");

    b.iter(|| {
        channel.send(msg.clone()).expect("Send failed");
        let received = channel.receive().expect("Receive failed");
        core::hint::black_box(received);
    });
}

#[bench]
fn bench_async_channel_throughput(b: &mut Bencher) {
    ipc::init();
    let channel = AsyncChannel::new(1, 10000);
    let messages: Vec<_> = (0..1000)
        .map(|i| Message::small(&i.to_ne_bytes()))
        .collect();

    b.iter(|| {
        // Send all messages
        for msg in &messages {
            channel.send(msg.clone()).expect("Send failed");
        }

        // Receive all messages
        let mut count = 0;
        while let Ok(Some(_)) = channel.receive() {
            count += 1;
            if count >= messages.len() {
                break;
            }
        }
    });
}

#[bench]
fn bench_shared_region_creation(b: &mut Bencher) {
    ipc::init();
    let mut counter = 1u64;

    b.iter(|| {
        let region = SharedRegion::new(counter, 4096, Permissions::READ_WRITE);
        counter += 1;
        core::hint::black_box(region);
    });
}

#[bench]
fn bench_capability_creation(b: &mut Bencher) {
    ipc::init();
    let region = SharedRegion::new(1, 4096, Permissions::READ_WRITE);
    let mut counter = 2u64;

    b.iter(|| {
        let cap = region.create_capability(counter, TransferMode::Share);
        counter += 1;
        core::hint::black_box(cap);
    });
}

#[bench]
fn bench_fast_path_message_creation(b: &mut Bencher) {
    // Benchmark small message creation which would use fast path
    let data = [1u8, 2, 3, 4, 5, 6, 7, 8];

    b.iter(|| {
        let msg = Message::small(&data);
        core::hint::black_box(msg);
    });
}

#[bench]
fn bench_message_clone(b: &mut Bencher) {
    let small_msg = Message::small(b"test");

    b.iter(|| {
        let cloned = small_msg.clone();
        core::hint::black_box(cloned);
    });
}

#[bench]
fn bench_registry_lookup(b: &mut Bencher) {
    ipc::init();

    // Create some endpoints to look up
    let mut endpoints = Vec::new();
    let mut capabilities = Vec::new();
    for i in 0..100 {
        let (id, cap) = create_endpoint(i).expect("Failed to create endpoint");
        endpoints.push(id);
        capabilities.push(cap);
    }

    let mut idx = 0;
    b.iter(|| {
        let cap_idx = idx % capabilities.len();
        let cap = &capabilities[cap_idx];
        // Validate capability as a proxy for registry lookup
        let result = validate_capability(cap.owner(), cap);
        idx += 1;
        core::hint::black_box(result);
    });
}

#[bench]
fn bench_performance_measurement_overhead(b: &mut Bencher) {
    b.iter(|| {
        let (result, cycles) = measure_ipc_operation(|| {
            // Minimal operation
            42
        });
        core::hint::black_box((result, cycles));
    });
}

#[bench]
fn bench_zero_copy_vs_regular_copy(b: &mut Bencher) {
    ipc::init();

    // Test data
    let data = [0u8; 4096];

    // First, benchmark regular message copy
    serial_println!("\nBenchmarking regular message copy...");
    let mut copy_results = Vec::new();
    for _ in 0..10 {
        let start = read_timestamp();
        let msg = Message::large(&data);
        let _cloned = msg.clone();
        let elapsed = read_timestamp() - start;
        copy_results.push(elapsed);
    }

    // Then benchmark zero-copy
    serial_println!("Benchmarking zero-copy transfer...");
    let region = SharedRegion::new(1, 4096, Permissions::READ_WRITE);
    let mut zero_copy_results = Vec::new();
    for i in 0..10 {
        let start = read_timestamp();
        let _cap = region.create_capability(i + 2, TransferMode::Share);
        let elapsed = read_timestamp() - start;
        zero_copy_results.push(elapsed);
    }

    // Compare results
    let avg_copy: u64 = copy_results.iter().sum::<u64>() / copy_results.len() as u64;
    let avg_zero_copy: u64 = zero_copy_results.iter().sum::<u64>() / zero_copy_results.len() as u64;

    serial_println!("Average copy time: {} cycles", avg_copy);
    serial_println!("Average zero-copy time: {} cycles", avg_zero_copy);
    serial_println!("Zero-copy speedup: {}x", avg_copy / avg_zero_copy.max(1));

    b.iter(|| {
        let _cap = region.create_capability(100, TransferMode::Share);
    });
}

#[bench]
fn bench_ipc_latency_targets(b: &mut Bencher) {
    ipc::init();

    // Warm up
    let (_send_id, _recv_id, _, _) = create_channel(1, 100).unwrap();

    // Measure various IPC operations
    serial_println!("\nMeasuring IPC operation latencies...");

    // Small message latency
    let small_msg = Message::small(b"test");
    let (_, small_cycles) = measure_ipc_operation(|| {
        let _ = small_msg.clone();
    });
    let small_ns = cycles_to_ns(small_cycles);
    serial_println!("Small message: {} cycles ({} ns)", small_cycles, small_ns);

    // Large message latency
    let large_data = [0u8; 1024];
    let large_msg = Message::large(&large_data);
    let (_, large_cycles) = measure_ipc_operation(|| {
        let _ = large_msg.clone();
    });
    let large_ns = cycles_to_ns(large_cycles);
    serial_println!("Large message: {} cycles ({} ns)", large_cycles, large_ns);

    // Capability creation
    let (endpoint_id, _) = create_endpoint(99).unwrap();
    let (_, cap_cycles) = measure_ipc_operation(|| {
        let _ = IpcCapability::new(endpoint_id, IpcPermissions::all());
    });
    let cap_ns = cycles_to_ns(cap_cycles);
    serial_println!("Capability creation: {} cycles ({} ns)", cap_cycles, cap_ns);

    // Check against targets
    let report = IPC_PERF_STATS.get_report();
    if report.meets_phase1_targets() {
        serial_println!("✓ Meets Phase 1 targets (<5μs)");
    }
    if report.meets_phase5_targets() {
        serial_println!("✓ Meets Phase 5 targets (<1μs)");
    }

    b.iter(|| {
        let _ = small_msg.clone();
    });
}

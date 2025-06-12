//! Performance validation benchmarks for Phase 1 components
//!
//! These benchmarks verify that performance targets are met

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexec_test_harness_main = "test_main"]

extern crate alloc;
use alloc::vec::Vec;

use veridian_kernel::{
    arch::timer::read_tsc,
    cap::{CapabilitySpace, CapabilityToken, Rights},
    ipc::{IpcError, IpcPermissions, Message, MessageHeader},
    mm::FrameAllocator,
    process::ProcessId,
    test_utils::*,
};

const ITERATIONS: usize = 1000;

#[test_case]
fn bench_ipc_small_message_latency() {
    use veridian_kernel::ipc::sync::SyncChannel;

    // Create channel
    let channel = SyncChannel::new(1000, ProcessId(1), IpcPermissions::all());
    let cap = CapabilityToken::new(1, 0);

    // Prepare message
    let mut data = [0u8; 32];
    for i in 0..32 {
        data[i] = i as u8;
    }

    let header = MessageHeader {
        msg_type: 1,
        flags: 0,
        sender: ProcessId(1),
        receiver: ProcessId(2),
        capability: Some(cap),
        timestamp: 0,
    };

    // Measure send/receive latency
    let mut total_cycles = 0u64;

    for _ in 0..ITERATIONS {
        let message = Message::new(header.clone(), &data);

        let start = read_tsc();
        channel.send(message, cap).expect("Send failed");
        let received = channel.receive(cap).expect("Receive failed");
        let end = read_tsc();

        total_cycles += end - start;

        // Verify message
        assert_eq!(received.header.msg_type, 1);
        assert_eq!(received.data()[0], 0);
    }

    let avg_cycles = total_cycles / ITERATIONS as u64;
    let avg_us = avg_cycles / 3000; // Assume 3GHz CPU

    println!(
        "[BENCH] IPC small message latency: {} cycles ({} µs)",
        avg_cycles, avg_us
    );

    // Target: < 1µs
    assert!(avg_us < 1, "IPC latency exceeds target of 1µs");
}

#[test_case]
fn bench_capability_lookup() {
    let cap_space = CapabilitySpace::new();

    // Insert capabilities
    for i in 0..100 {
        let cap = CapabilityToken::new(i, 0);
        cap_space
            .insert(
                cap,
                veridian_kernel::cap::object::ObjectRef::Process {
                    pid: ProcessId(i as u64),
                },
                Rights::all(),
            )
            .expect("Insert failed");
    }

    // Measure lookup time
    let mut total_cycles = 0u64;
    let test_cap = CapabilityToken::new(50, 0);

    for _ in 0..ITERATIONS {
        let start = read_tsc();
        let rights = cap_space.lookup(test_cap);
        let end = read_tsc();

        total_cycles += end - start;
        assert!(rights.is_some());
    }

    let avg_cycles = total_cycles / ITERATIONS as u64;
    let avg_ns = avg_cycles * 1000 / 3000; // Assume 3GHz CPU

    println!(
        "[BENCH] Capability lookup: {} cycles ({} ns)",
        avg_cycles, avg_ns
    );

    // Target: O(1) - should be < 100ns
    assert!(avg_ns < 100, "Capability lookup exceeds target");
}

#[test_case]
fn bench_memory_allocation() {
    let mut allocator = FrameAllocator::new();
    allocator.init(0x100000, 0x10000000); // 256MB region

    let mut total_cycles = 0u64;
    let mut frames = Vec::new();

    // Measure allocation time
    for _ in 0..ITERATIONS {
        let start = read_tsc();
        let frame = allocator.allocate_frame().expect("Allocation failed");
        let end = read_tsc();

        total_cycles += end - start;
        frames.push(frame);
    }

    let avg_cycles = total_cycles / ITERATIONS as u64;
    let avg_us = avg_cycles / 3000; // Assume 3GHz CPU

    println!(
        "[BENCH] Memory allocation: {} cycles ({} µs)",
        avg_cycles, avg_us
    );

    // Clean up
    for frame in frames {
        allocator.deallocate_frame(frame);
    }

    // Target: < 1µs
    assert!(avg_us < 1, "Memory allocation exceeds target of 1µs");
}

#[test_case]
fn bench_context_switch() {
    use core::ptr::NonNull;

    use veridian_kernel::sched::{ProcessId as SchedPid, Scheduler, Task, ThreadId as SchedTid};

    // Create scheduler and tasks
    let mut scheduler = Scheduler::new();

    // Create dummy tasks
    let task1 = Task::new(
        SchedPid(1),
        SchedTid(1),
        alloc::string::String::from("task1"),
        0x100000,
        0x200000,
        0,
    );

    let task2 = Task::new(
        SchedPid(2),
        SchedTid(2),
        alloc::string::String::from("task2"),
        0x110000,
        0x210000,
        0,
    );

    let task1_ptr = NonNull::new(&task1 as *const _ as *mut _).unwrap();
    let task2_ptr = NonNull::new(&task2 as *const _ as *mut _).unwrap();

    // Initialize scheduler
    scheduler.init(task1_ptr);
    scheduler.enqueue(task2_ptr);

    // Measure context switch time
    let mut total_cycles = 0u64;

    for _ in 0..100 {
        // Less iterations for context switch
        let start = read_tsc();
        scheduler.schedule();
        let end = read_tsc();

        total_cycles += end - start;
    }

    let avg_cycles = total_cycles / 100;
    let avg_us = avg_cycles / 3000; // Assume 3GHz CPU

    println!(
        "[BENCH] Context switch: {} cycles ({} µs)",
        avg_cycles, avg_us
    );

    // Target: < 10µs
    assert!(avg_us < 10, "Context switch exceeds target of 10µs");
}

#[test_case]
fn bench_ipc_throughput() {
    use veridian_kernel::ipc::async_ipc::AsyncChannel;

    // Create async channel for throughput test
    let channel = AsyncChannel::new(1000, ProcessId(1), 1024); // 1KB buffer
    let cap = CapabilityToken::new(2, 0);

    // Prepare messages
    let data = [0x42u8; 64];
    let header = MessageHeader {
        msg_type: 2,
        flags: 0,
        sender: ProcessId(1),
        receiver: ProcessId(2),
        capability: Some(cap),
        timestamp: 0,
    };

    // Measure throughput
    let start = read_tsc();
    let mut sent = 0;

    // Send as many as possible
    for _ in 0..ITERATIONS {
        let message = Message::new(header.clone(), &data);
        match channel.send_async(message, cap) {
            Ok(()) => sent += 1,
            Err(IpcError::BufferFull) => break,
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    let end = read_tsc();
    let total_cycles = end - start;
    let total_us = total_cycles / 3000; // Assume 3GHz CPU
    let throughput_mbps = (sent * 64 * 8) / total_us; // Mbps

    println!(
        "[BENCH] IPC throughput: {} messages in {} µs ({} Mbps)",
        sent, total_us, throughput_mbps
    );

    // Target: > 100 Mbps
    assert!(throughput_mbps > 100, "IPC throughput below target");
}

#[test_case]
fn bench_capability_revocation() {
    use veridian_kernel::cap::revocation::revoke_capability;

    let cap_space = CapabilitySpace::new();

    // Create many capabilities
    let mut caps = Vec::new();
    for i in 0..100 {
        let cap = CapabilityToken::new(i, 0);
        cap_space
            .insert(
                cap,
                veridian_kernel::cap::object::ObjectRef::Memory {
                    base: i as usize * 4096,
                    size: 4096,
                },
                Rights::all(),
            )
            .expect("Insert failed");
        caps.push(cap);
    }

    // Measure revocation time
    let mut total_cycles = 0u64;

    for cap in caps.iter().take(10) {
        let start = read_tsc();
        revoke_capability(*cap).expect("Revocation failed");
        let end = read_tsc();

        total_cycles += end - start;
    }

    let avg_cycles = total_cycles / 10;
    let avg_us = avg_cycles / 3000; // Assume 3GHz CPU

    println!(
        "[BENCH] Capability revocation: {} cycles ({} µs)",
        avg_cycles, avg_us
    );

    // Verify revocation worked
    for cap in caps.iter().take(10) {
        assert!(cap_space.lookup(*cap).is_none());
    }
}

#[test_case]
fn bench_scheduler_decision() {
    use veridian_kernel::sched::{SchedAlgorithm, Scheduler};

    let mut scheduler = Scheduler::new();
    scheduler.algorithm = SchedAlgorithm::Priority;

    // Add many tasks
    for i in 0..50 {
        let task = veridian_kernel::sched::Task::new(
            veridian_kernel::sched::ProcessId(i),
            veridian_kernel::sched::ThreadId(i),
            alloc::string::String::from("bench-task"),
            0x100000 + i as usize * 0x1000,
            0x200000 + i as usize * 0x1000,
            0,
        );

        let task_ptr = core::ptr::NonNull::new(&task as *const _ as *mut _).unwrap();
        scheduler.enqueue(task_ptr);
    }

    // Measure scheduling decision time
    let mut total_cycles = 0u64;

    for _ in 0..ITERATIONS {
        let start = read_tsc();
        let _next = scheduler.pick_next();
        let end = read_tsc();

        total_cycles += end - start;
    }

    let avg_cycles = total_cycles / ITERATIONS as u64;
    let avg_ns = avg_cycles * 1000 / 3000; // Assume 3GHz CPU

    println!(
        "[BENCH] Scheduler decision: {} cycles ({} ns)",
        avg_cycles, avg_ns
    );

    // Should be fast even with many tasks
    assert!(avg_ns < 500, "Scheduler decision too slow");
}

// Test harness entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    veridian_kernel::init();
    test_main();
    veridian_kernel::arch::halt();
}

// Panic handler
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

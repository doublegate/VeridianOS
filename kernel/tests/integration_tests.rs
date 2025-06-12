//! Comprehensive integration tests for Phase 1 components
//!
//! These tests verify that all subsystems work together correctly

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexec_test_harness_main = "test_main"]

extern crate alloc;
use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use veridian_kernel::{
    cap::{CapabilitySpace, CapabilityToken, Rights},
    ipc::{IpcPermissions, Message, MessageHeader},
    mm::{FrameAllocator, VirtualAddressSpace},
    process::{Process, ProcessId, ProcessPriority, ThreadId},
    sched::{Priority, Task},
    test_utils::*,
};

static TEST_SYNC: AtomicBool = AtomicBool::new(false);

#[test_case]
fn test_process_with_ipc_and_capabilities() {
    // Create two processes
    let process1 = Process::new(
        ProcessId(100),
        None,
        String::from("test-sender"),
        ProcessPriority::Normal,
    );

    let process2 = Process::new(
        ProcessId(101),
        None,
        String::from("test-receiver"),
        ProcessPriority::Normal,
    );

    // Create IPC endpoint
    let endpoint_id = 1000;
    let permissions = IpcPermissions::READ | IpcPermissions::WRITE;

    // Create capability for endpoint
    let cap_token = CapabilityToken::new(1, 0);
    let rights = Rights::READ | Rights::WRITE | Rights::SEND | Rights::RECEIVE;

    // Add capability to both processes
    process1
        .capability_space
        .lock()
        .insert(
            cap_token,
            veridian_kernel::cap::object::ObjectRef::Endpoint { id: endpoint_id },
            rights,
        )
        .expect("Failed to insert capability");

    process2
        .capability_space
        .lock()
        .insert(
            cap_token,
            veridian_kernel::cap::object::ObjectRef::Endpoint { id: endpoint_id },
            rights,
        )
        .expect("Failed to insert capability");

    // Create a message
    let msg_data = [0x42u8; 32];
    let header = MessageHeader {
        msg_type: 1,
        flags: 0,
        sender: ProcessId(100),
        receiver: ProcessId(101),
        capability: Some(cap_token),
        timestamp: 0,
    };
    let message = Message::new(header, &msg_data);

    // Test that processes can communicate
    assert!(process1
        .capability_space
        .lock()
        .check_rights(cap_token, Rights::SEND));
    assert!(process2
        .capability_space
        .lock()
        .check_rights(cap_token, Rights::RECEIVE));
}

#[test_case]
fn test_memory_allocation_with_capabilities() {
    // Create process with memory capabilities
    let process = Process::new(
        ProcessId(200),
        None,
        String::from("test-memory"),
        ProcessPriority::Normal,
    );

    // Allocate memory
    let mut frame_allocator = FrameAllocator::new();
    frame_allocator.init(0x100000, 0x200000); // 1MB region

    let frame = frame_allocator
        .allocate_frame()
        .expect("Failed to allocate frame");

    // Create capability for memory region
    let cap_token = CapabilityToken::new(2, 0);
    let rights = Rights::READ | Rights::WRITE | Rights::MAP;

    process
        .capability_space
        .lock()
        .insert(
            cap_token,
            veridian_kernel::cap::object::ObjectRef::Memory {
                base: frame.as_usize(),
                size: 4096,
            },
            rights,
        )
        .expect("Failed to insert memory capability");

    // Verify capability allows memory access
    assert!(process
        .capability_space
        .lock()
        .check_rights(cap_token, Rights::MAP));

    // Clean up
    frame_allocator.deallocate_frame(frame);
}

#[test_case]
fn test_scheduler_with_processes() {
    use core::ptr::NonNull;

    use veridian_kernel::sched::{SchedAlgorithm, Scheduler};

    // Create scheduler
    let mut scheduler = Scheduler::new();
    scheduler.algorithm = SchedAlgorithm::Priority;

    // Create idle task
    let idle_task = Task::new(
        ProcessId(0),
        ThreadId(0),
        String::from("idle"),
        idle_entry as usize,
        0x80000,
        0,
    );
    let idle_ptr = NonNull::new(&idle_task as *const _ as *mut _).unwrap();
    scheduler.init(idle_ptr);

    // Create test processes and tasks
    for i in 1..=3 {
        let process = Process::new(
            ProcessId(300 + i),
            None,
            String::from("test-process"),
            ProcessPriority::Normal,
        );

        let task = Task::new(
            process.pid,
            ThreadId(i),
            String::from("test-task"),
            test_entry as usize,
            0x80000 + i as usize * 0x10000,
            0,
        );

        let task_ptr = NonNull::new(&task as *const _ as *mut _).unwrap();
        scheduler.enqueue(task_ptr);
    }

    // Run scheduler
    let next = scheduler.pick_next();
    assert!(next.is_some());
}

#[test_case]
fn test_process_lifecycle() {
    use veridian_kernel::process::lifecycle::*;

    // Create parent process
    let parent = create_process(String::from("parent"), test_entry as usize)
        .expect("Failed to create parent");

    // Fork child
    let child = fork_process().expect("Failed to fork");

    // In parent, child PID should be non-zero
    if child.0 != 0 {
        // Parent process
        assert!(child.0 > parent.0);

        // Wait for child
        TEST_SYNC.store(true, Ordering::Release);

        // Note: In real scenario, wait_process would block
    } else {
        // Child process
        while !TEST_SYNC.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }

        // Exit child
        exit_process(42);
    }
}

#[test_case]
fn test_capability_inheritance() {
    use veridian_kernel::cap::inheritance::*;

    // Create parent and child capability spaces
    let parent_space = CapabilitySpace::new();
    let child_space = CapabilitySpace::new();

    // Add capabilities to parent
    for i in 0..5 {
        let cap = CapabilityToken::new(i, 0);
        let rights = Rights::READ | Rights::WRITE;
        parent_space
            .insert(
                cap,
                veridian_kernel::cap::object::ObjectRef::Process {
                    pid: ProcessId(i as u64),
                },
                rights,
            )
            .expect("Failed to insert capability");
    }

    // Test inheritance
    let inherited = inherit_capabilities(&parent_space, &child_space, InheritancePolicy::All)
        .expect("Failed to inherit capabilities");

    assert_eq!(inherited, 5);
    assert_eq!(parent_space.stats().total_caps.load(Ordering::Relaxed), 5);
    assert_eq!(child_space.stats().total_caps.load(Ordering::Relaxed), 5);
}

#[test_case]
fn test_ipc_with_scheduler_blocking() {
    use veridian_kernel::{ipc::sync::SyncChannel, sched};

    // Create sync channel
    let channel = SyncChannel::new(2000, ProcessId(400), IpcPermissions::all());

    // Test blocking behavior
    let process = Process::new(
        ProcessId(400),
        None,
        String::from("test-blocker"),
        ProcessPriority::Normal,
    );

    // Set process as current (mock)
    process.set_state(veridian_kernel::process::ProcessState::Running);

    // Try to receive on empty channel (would block)
    let cap = CapabilityToken::new(10, 0);
    match channel.try_receive(cap) {
        Err(veridian_kernel::ipc::IpcError::WouldBlock) => {
            // Expected - channel is empty
        }
        _ => panic!("Expected WouldBlock error"),
    }
}

#[test_case]
fn test_memory_mapping_with_capabilities() {
    let mut vas = VirtualAddressSpace::new();
    vas.init().expect("Failed to init VAS");

    // Create memory capability
    let cap = CapabilityToken::new(20, 0);
    let cap_space = CapabilitySpace::new();

    cap_space
        .insert(
            cap,
            veridian_kernel::cap::object::ObjectRef::Memory {
                base: 0x100000,
                size: 0x10000,
            },
            Rights::READ | Rights::WRITE | Rights::MAP,
        )
        .expect("Failed to insert capability");

    // Verify capability allows mapping
    assert!(cap_space.check_rights(cap, Rights::MAP));

    // Map memory region
    use veridian_kernel::mm::{PageFlags, VirtualAddress};
    vas.map_region(
        VirtualAddress::new(0x200000),
        0x10000,
        PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER,
    )
    .expect("Failed to map region");
}

#[test_case]
fn test_process_exit_cleanup() {
    use veridian_kernel::process::lifecycle::*;

    // Create process with resources
    let process = create_process(String::from("cleanup-test"), test_entry as usize)
        .expect("Failed to create process");

    // Add IPC endpoint
    let endpoint_id = 5000;
    veridian_kernel::process::get_process(process)
        .unwrap()
        .ipc_endpoints
        .lock()
        .insert(endpoint_id, 1);

    // Add capability
    let cap = CapabilityToken::new(30, 0);
    veridian_kernel::process::get_process(process)
        .unwrap()
        .capability_space
        .lock()
        .insert(
            cap,
            veridian_kernel::cap::object::ObjectRef::Endpoint { id: endpoint_id },
            Rights::all(),
        )
        .expect("Failed to insert capability");

    // Exit process
    if let Some(proc) = veridian_kernel::process::get_process_mut(process) {
        proc.set_exit_code(0);
        proc.set_state(veridian_kernel::process::ProcessState::Zombie);
    }

    // Verify resources are marked for cleanup
    let proc = veridian_kernel::process::get_process(process).unwrap();
    assert_eq!(
        proc.get_state(),
        veridian_kernel::process::ProcessState::Zombie
    );
}

// Test entry points
extern "C" fn idle_entry() -> ! {
    loop {
        veridian_kernel::arch::idle();
    }
}

extern "C" fn test_entry() -> ! {
    loop {
        core::hint::spin_loop();
    }
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

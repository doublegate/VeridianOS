//! Comprehensive integration tests for Phase 1 components
//!
//! These tests verify that all subsystems work together correctly

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
use alloc::string::String;

use veridian_kernel::{
    cap::{CapabilitySpace, CapabilityToken, Rights},
    ipc::{self, Message, ProcessId},
    mm::{FRAME_ALLOCATOR, PhysAddr},
    process::{self, ProcessPriority, ThreadId},
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
fn test_process_with_ipc_and_capabilities() {
    serial_println!("test_process_with_ipc_and_capabilities...");
    
    // Initialize subsystems
    process::init();
    ipc::init();
    
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
    let (endpoint_id, endpoint_cap) = create_endpoint(ProcessId(100)).expect("Failed to create endpoint");
    
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
    let msg = Message::small(endpoint_cap.id(), 42);
    
    // Test that processes can communicate
    assert!(process1
        .capability_space
        .lock()
        .check_rights(cap_token, Rights::SEND));
    assert!(process2
        .capability_space
        .lock()
        .check_rights(cap_token, Rights::RECEIVE));
    
    serial_println!("[ok]");
}

#[test_case]
fn test_memory_allocation_with_capabilities() {
    serial_println!("test_memory_allocation_with_capabilities...");
    
    process::init();
    
    // Create process with memory capabilities
    let process = Process::new(
        ProcessId(200),
        None,
        String::from("test-memory"),
        ProcessPriority::Normal,
    );
    
    // Allocate memory
    if let Some(frame) = FRAME_ALLOCATOR.allocate() {
        // Create capability for memory region
        let cap_token = CapabilityToken::new(2, 0);
        let rights = Rights::READ | Rights::WRITE | Rights::MAP;
        
        process
            .capability_space
            .lock()
            .insert(
                cap_token,
                veridian_kernel::cap::object::ObjectRef::Memory {
                    start: frame,
                    size: 4096,
                    flags: 0,
                },
                rights,
            )
            .expect("Failed to insert memory capability");
        
        // Verify capability
        assert!(process
            .capability_space
            .lock()
            .check_rights(cap_token, Rights::MAP));
        
        // Clean up
        FRAME_ALLOCATOR.deallocate(frame);
    }
    
    serial_println!("[ok]");
}

#[test_case]
fn test_scheduler_with_multiple_processes() {
    serial_println!("test_scheduler_with_multiple_processes...");
    
    use veridian_kernel::sched;
    
    // Initialize scheduler
    process::init();
    sched::init();
    
    // Create multiple processes with threads
    for i in 300..303 {
        let pid = ProcessId(i);
        let process = Process::new(
            pid,
            None,
            String::from("sched-test"),
            ProcessPriority::Normal,
        );
        
        // Create main thread
        let thread = Thread::new(ThreadId(i), pid);
        process.add_thread(thread).expect("Failed to add thread");
        
        // Insert into process table
        process::table::insert_process(process);
        
        // Create scheduler task
        let task = sched::create_task("sched-test", pid, ThreadId(i), 0, 0);
        assert!(!task.is_null());
    }
    
    // Run scheduler for a few ticks
    for _ in 0..10 {
        sched::schedule();
    }
    
    // Check scheduler metrics
    let metrics = sched::metrics::SCHEDULER_METRICS.get_summary();
    assert!(metrics.context_switches > 0);
    
    serial_println!("[ok]");
}

#[test_case]
fn test_capability_revocation() {
    serial_println!("test_capability_revocation...");
    
    process::init();
    
    let process = Process::new(
        ProcessId(400),
        None,
        String::from("cap-test"),
        ProcessPriority::Normal,
    );
    
    // Create and insert capability
    let cap_token = CapabilityToken::new(3, 0);
    let rights = Rights::all();
    
    process
        .capability_space
        .lock()
        .insert(
            cap_token,
            veridian_kernel::cap::object::ObjectRef::Process {
                pid: ProcessId(400),
            },
            rights,
        )
        .expect("Failed to insert capability");
    
    // Verify it exists
    assert!(process
        .capability_space
        .lock()
        .lookup(cap_token)
        .is_some());
    
    // Revoke capability
    process.capability_space.lock().revoke(cap_token);
    
    // Verify it's gone
    assert!(process
        .capability_space
        .lock()
        .lookup(cap_token)
        .is_none());
    
    serial_println!("[ok]");
}

#[test_case]
fn test_thread_synchronization() {
    serial_println!("test_thread_synchronization...");
    
    use veridian_kernel::process::sync::{KernelMutex, KernelSemaphore};
    
    process::init();
    
    // Create shared mutex
    let mutex = KernelMutex::new();
    
    // Lock and unlock
    assert!(mutex.try_lock());
    mutex.unlock();
    
    // Create semaphore
    let sem = KernelSemaphore::new(2);
    
    // Acquire permits
    assert!(sem.try_wait());
    assert!(sem.try_wait());
    assert!(!sem.try_wait()); // Should fail
    
    // Release permits
    sem.signal();
    assert!(sem.try_wait()); // Should succeed now
    
    serial_println!("[ok]");
}

#[test_case]
fn test_ipc_with_shared_memory() {
    serial_println!("test_ipc_with_shared_memory...");
    
    use veridian_kernel::ipc::{SharedRegion, Permissions, TransferMode};
    
    ipc::init();
    
    // Create shared memory region
    let region = SharedRegion::new(1, 8192, Permissions::READ_WRITE);
    
    // Create capability for sharing
    let cap = region.create_capability(ProcessId(500), TransferMode::Share);
    assert!(cap.is_some());
    
    // Verify region properties
    assert_eq!(region.size(), 8192);
    assert_eq!(region.id(), 1);
    assert!(region.permissions().contains(Permissions::READ));
    assert!(region.permissions().contains(Permissions::WRITE));
    
    serial_println!("[ok]");
}

#[test_case]
fn test_full_system_integration() {
    serial_println!("test_full_system_integration...");
    
    // Initialize all subsystems
    process::init();
    ipc::init();
    veridian_kernel::sched::init();
    
    // Create a parent process
    let parent_pid = ProcessId(600);
    let parent = Process::new(
        parent_pid,
        None,
        String::from("parent"),
        ProcessPriority::Normal,
    );
    
    // Create child process
    let child_pid = ProcessId(601);
    let child = Process::new(
        child_pid,
        Some(parent_pid),
        String::from("child"),
        ProcessPriority::Normal,
    );
    
    // Create IPC channel between them
    let (send_id, recv_id, send_cap, recv_cap) = 
        create_channel(parent_pid, 100).expect("Failed to create channel");
    
    // Add capabilities
    parent.capability_space.lock().insert(
        CapabilityToken::new(4, 0),
        veridian_kernel::cap::object::ObjectRef::Endpoint { id: send_id },
        Rights::SEND,
    ).expect("Failed to insert send capability");
    
    child.capability_space.lock().insert(
        CapabilityToken::new(5, 0),
        veridian_kernel::cap::object::ObjectRef::Endpoint { id: recv_id },
        Rights::RECEIVE,
    ).expect("Failed to insert receive capability");
    
    // Insert into process table
    process::table::insert_process(parent);
    process::table::insert_process(child);
    
    // Verify processes exist
    assert!(process::table::get_process(parent_pid).is_some());
    assert!(process::table::get_process(child_pid).is_some());
    
    // Clean up
    process::table::remove_process(parent_pid);
    process::table::remove_process(child_pid);
    
    serial_println!("[ok]");
}
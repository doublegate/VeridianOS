//! Common test utilities and helpers for VeridianOS kernel tests

use crate::{serial_print, serial_println, test_framework::*};

/// Initialize test environment for a specific subsystem
pub fn init_test_env(subsystem: &str) {
    serial_println!("\n=== {} Test Suite ===", subsystem);

    // Initialize subsystems needed for testing
    #[cfg(feature = "alloc")]
    if crate::test_framework::TEST_REGISTRY.is_none() {
        crate::test_framework::init_test_registry();
    }
}

/// Helper to create test processes
#[cfg(feature = "alloc")]
pub fn create_test_process(name: &str) -> crate::process::ProcessId {
    use crate::process::{Process, ProcessId};

    // Create a minimal test process
    let pid = ProcessId(crate::process::table::next_pid());
    let process = Process::new(pid, name);

    // Add to process table
    crate::process::table::insert_process(process);

    pid
}

/// Helper to clean up test processes
#[cfg(feature = "alloc")]
pub fn cleanup_test_process(pid: crate::process::ProcessId) {
    crate::process::table::remove_process(pid);
}

/// Test helper for IPC operations
pub mod ipc_helpers {
    use crate::ipc::{self, IpcCapability, Message, ProcessId};

    /// Create a test IPC channel
    pub fn create_test_channel(
        owner: ProcessId,
        capacity: usize,
    ) -> Result<(u64, u64, IpcCapability, IpcCapability), ipc::IpcError> {
        ipc::registry::create_channel(owner, capacity)
    }

    /// Create a test endpoint
    pub fn create_test_endpoint(owner: ProcessId) -> Result<(u64, IpcCapability), ipc::IpcError> {
        ipc::registry::create_endpoint(owner)
    }

    /// Send a test message
    pub fn send_test_message(_msg: Message) -> Result<(), ipc::IpcError> {
        // Simplified test message send
        Ok(())
    }
}

/// Test helper for scheduler operations
pub mod scheduler_helpers {
    use crate::{
        process::{ProcessId, ThreadId},
        sched::{self, Task},
    };

    /// Create a test task
    pub fn create_test_task(name: &str, pid: ProcessId, tid: ThreadId) -> *mut Task {
        sched::create_task(name, pid, tid, 0, 0)
    }

    /// Clean up test task
    pub fn cleanup_test_task(task: *mut Task) {
        unsafe {
            if !task.is_null() {
                sched::exit_task(task);
            }
        }
    }
}

/// Test helper for memory operations
pub mod memory_helpers {
    use crate::mm::{PhysicalAddress, VirtualAddress};

    /// Allocate test memory frame
    pub fn alloc_test_frame() -> Option<PhysicalAddress> {
        // Use frame allocator when available
        Some(PhysicalAddress::new(0x100000)) // Placeholder
    }

    /// Free test memory frame
    pub fn free_test_frame(addr: PhysicalAddress) {
        // Free frame when allocator available
    }
}

/// Assertion helpers for kernel tests
#[macro_export]
macro_rules! assert_ok {
    ($result:expr) => {
        match $result {
            Ok(val) => val,
            Err(e) => {
                serial_println!("Assertion failed: {:?} is not Ok", e);
                panic!("Expected Ok, got Err");
            }
        }
    };
}

#[macro_export]
macro_rules! assert_err {
    ($result:expr) => {
        match $result {
            Ok(_) => {
                serial_println!("Assertion failed: result is Ok");
                panic!("Expected Err, got Ok");
            }
            Err(e) => e,
        }
    };
}

/// Performance assertion for benchmarks
#[macro_export]
macro_rules! assert_performance {
    ($time_ns:expr, < $limit_ns:expr) => {
        if $time_ns >= $limit_ns {
            serial_println!(
                "Performance assertion failed: {} ns >= {} ns",
                $time_ns,
                $limit_ns
            );
            panic!("Performance requirement not met");
        }
    };
}

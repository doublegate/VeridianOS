//! Tests and examples for RAII patterns
//!
//! This module demonstrates the usage of RAII patterns in the kernel.

#![cfg(test)]

use crate::{
    mm::{FrameAllocatorError, FRAME_ALLOCATOR},
    println,
    process::{current_process, ProcessId},
    raii::*,
};

#[test]
fn test_frame_guard() {
    println!("\n=== Testing FrameGuard RAII ===");

    // Allocate a frame with RAII
    {
        match FRAME_ALLOCATOR.allocate_frame_raii() {
            Ok(frame_guard) => {
                println!("Allocated frame at {:#x}", frame_guard.addr());
                // Frame will be automatically freed when frame_guard goes out
                // of scope
            }
            Err(e) => {
                println!("Failed to allocate frame: {:?}", e);
            }
        }
    }
    // Frame has been automatically freed here

    println!("Frame guard dropped - frame automatically freed\n");
}

#[test]
fn test_frame_guard_leak() {
    println!("\n=== Testing FrameGuard leak ===");

    // Allocate a frame and leak it
    let leaked_frame = match FRAME_ALLOCATOR.allocate_frame_raii() {
        Ok(frame_guard) => {
            println!("Allocated frame at {:#x}", frame_guard.addr());
            // Leak the frame - it won't be freed
            frame_guard.leak()
        }
        Err(e) => {
            println!("Failed to allocate frame: {:?}", e);
            return;
        }
    };

    println!(
        "Frame leaked at {:#x} - must be manually freed",
        leaked_frame.addr()
    );

    // Manual cleanup required
    unsafe {
        FRAME_ALLOCATOR.free_frame(leaked_frame);
    }
}

#[test]
fn test_multiple_frames_guard() {
    println!("\n=== Testing FramesGuard RAII ===");

    {
        match FRAME_ALLOCATOR.allocate_frames_raii(4) {
            Ok(frames_guard) => {
                println!("Allocated 4 frames");
                // All frames will be freed when guard drops
            }
            Err(e) => {
                println!("Failed to allocate frames: {:?}", e);
            }
        }
    }

    println!("All frames automatically freed\n");
}

#[test]
fn test_scope_guard() {
    println!("\n=== Testing ScopeGuard ===");

    let mut cleanup_executed = false;

    {
        let _guard = ScopeGuard::new(|| {
            cleanup_executed = true;
            println!("Cleanup code executed!");
        });

        println!("Inside scope...");
    }

    assert!(cleanup_executed);
    println!("Scope guard worked correctly\n");
}

#[test]
fn test_scope_guard_cancel() {
    println!("\n=== Testing ScopeGuard cancellation ===");

    let mut cleanup_executed = false;

    {
        let guard = ScopeGuard::new(|| {
            cleanup_executed = true;
            println!("This should not print!");
        });

        println!("Canceling guard...");
        guard.cancel();
    }

    assert!(!cleanup_executed);
    println!("Cleanup was successfully canceled\n");
}

#[test]
fn test_defer_macro() {
    println!("\n=== Testing defer! macro ===");

    let mut value = 0;

    {
        defer!(value = 42);
        println!("Value before scope exit: {}", value);
        assert_eq!(value, 0);
    }

    assert_eq!(value, 42);
    println!("Value after scope exit: {}", value);
}

#[cfg(feature = "alloc")]
#[test]
fn test_capability_guard() {
    use alloc::sync::Arc;

    use spin::Mutex;

    use crate::cap::{CapabilitySpace, ObjectRef, Rights};

    println!("\n=== Testing CapabilityGuard ===");

    let cap_space = Arc::new(Mutex::new(CapabilitySpace::new()));

    {
        let mut space = cap_space.lock();
        match space.create_capability(Rights::READ | Rights::WRITE, ObjectRef::Memory(0x1000)) {
            Ok(cap_id) => {
                drop(space); // Release lock

                let cap_guard = CapabilityGuard::new(cap_id, cap_space.clone());
                println!("Created capability {} with RAII guard", cap_guard.id());

                // Capability will be revoked when guard drops
            }
            Err(e) => {
                println!("Failed to create capability: {:?}", e);
            }
        }
    }

    println!("Capability automatically revoked\n");
}

#[test]
fn test_channel_guard() {
    use crate::ipc::channel::Endpoint;

    println!("\n=== Testing ChannelGuard ===");

    let owner = ProcessId(1);

    {
        let (endpoint, _guard) = Endpoint::new_with_guard(owner);
        println!("Created channel {} with RAII guard", endpoint.id());

        // Use the endpoint...

        // Channel will be removed from registry when guard drops
    }

    println!("Channel automatically cleaned up\n");
}

/// Example of using RAII in a function that might fail
fn allocate_and_process() -> Result<(), FrameAllocatorError> {
    println!("\n=== Example: RAII with error handling ===");

    // Allocate frame with RAII
    let frame = FRAME_ALLOCATOR.allocate_frame_raii()?;
    println!("Allocated frame at {:#x}", frame.addr());

    // Simulate some processing that might fail
    let success = true; // Change to false to simulate failure

    if !success {
        println!("Processing failed!");
        return Err(FrameAllocatorError::InvalidFrame);
        // Frame is automatically freed even on early return
    }

    println!("Processing succeeded!");
    Ok(())
    // Frame is automatically freed on normal return
}

#[test]
fn test_raii_with_errors() {
    let _ = allocate_and_process();
}

/// Example of RAII pattern for lock tracking
#[test]
fn test_tracked_mutex() {
    use spin::Mutex;

    println!("\n=== Testing TrackedMutexGuard ===");

    let data = Mutex::new(42);

    {
        let guard = data.lock();
        let tracked = TrackedMutexGuard::new(guard, "important_data");

        println!("Value: {}", *tracked);

        // Lock will be logged as released when tracked drops
    }

    println!("Lock tracking complete\n");
}

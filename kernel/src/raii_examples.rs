//! Examples of RAII usage in kernel code
//!
//! This module demonstrates practical applications of RAII patterns
//! for common kernel operations.

#![cfg(feature = "alloc")]

use crate::{
    error::KernelError,
    mm::{FRAME_ALLOCATOR, MappingType, VirtualAddress},
    process::{ProcessId, current_process},
    cap::{Rights, ObjectRef},
    raii::*,
    println,
};

use alloc::sync::Arc;
use spin::Mutex;

/// Example: Safe DMA buffer allocation
/// 
/// This function allocates a DMA buffer that is automatically
/// freed when it goes out of scope.
pub fn allocate_dma_buffer(size_in_pages: usize) -> Result<FramesGuard, KernelError> {
    println!("[RAII Example] Allocating DMA buffer of {} pages", size_in_pages);
    
    // Allocate contiguous frames for DMA
    let frames = FRAME_ALLOCATOR.allocate_frames_raii(size_in_pages)?;
    
    println!("[RAII Example] DMA buffer allocated at frame {:#x}", frames.frames[0].addr());
    
    // The FramesGuard will automatically free the frames when dropped
    Ok(frames)
}

/// Example: Temporary memory mapping
/// 
/// Maps a region temporarily for an operation, then automatically unmaps it.
pub fn temporary_mapping_example(process_id: ProcessId) -> Result<(), KernelError> {
    println!("[RAII Example] Creating temporary mapping");
    
    if let Some(process) = crate::process::find_process(process_id) {
        let vas = &process.memory_space;
        
        // Create a temporary mapping with RAII
        let _mapping = vas.lock().map_region_raii(
            VirtualAddress::new(0x8000_0000),
            4096,
            MappingType::Data,
            process_id
        )?;
        
        println!("[RAII Example] Mapping created at {:#x}", 0x8000_0000);
        
        // Do some work with the mapping...
        // ... operations here ...
        
        // Mapping is automatically unmapped when _mapping goes out of scope
    }
    
    println!("[RAII Example] Mapping automatically cleaned up");
    Ok(())
}

/// Example: Scoped capability creation
/// 
/// Creates a capability that is automatically revoked when no longer needed.
pub fn scoped_capability_example(
    cap_space: Arc<Mutex<crate::cap::CapabilitySpace>>
) -> Result<(), KernelError> {
    println!("[RAII Example] Creating scoped capability");
    
    // Create a temporary capability
    let cap_id = {
        let mut space = cap_space.lock();
        space.create_capability(Rights::READ | Rights::WRITE, ObjectRef::Memory(0x2000))?
    };
    
    // Create RAII guard for automatic revocation
    let _cap_guard = CapabilityGuard::new(cap_id, cap_space.clone());
    
    println!("[RAII Example] Capability {} created with RAII guard", cap_id);
    
    // Use the capability...
    // ... operations here ...
    
    // Capability is automatically revoked when _cap_guard drops
    println!("[RAII Example] Capability will be revoked on scope exit");
    
    Ok(())
}

/// Example: Complex resource management
/// 
/// Demonstrates managing multiple resources with RAII.
pub fn complex_resource_example() -> Result<(), KernelError> {
    println!("[RAII Example] Complex resource management");
    
    // Allocate multiple resources
    let frame1 = FRAME_ALLOCATOR.allocate_frame_raii()?;
    let frame2 = FRAME_ALLOCATOR.allocate_frame_raii()?;
    
    // Use defer! for custom cleanup
    let mut cleanup_flag = false;
    defer!(cleanup_flag = true);
    
    println!("[RAII Example] Allocated frames at {:#x} and {:#x}", 
        frame1.addr(), frame2.addr());
    
    // Simulate an operation that might fail
    let success = true; // Change to false to test cleanup
    
    if !success {
        return Err(KernelError::InvalidState {
            expected: "success",
            actual: "operation failed",
        });
        // Even on early return, all RAII guards clean up properly
    }
    
    // Leak one frame intentionally
    let leaked = frame1.leak();
    println!("[RAII Example] Frame at {:#x} leaked intentionally", leaked.addr());
    
    // frame2 will be freed, cleanup_flag will be set
    // leaked frame must be manually freed later
    
    Ok(())
}

/// Example: Transaction-like operation with rollback
/// 
/// Uses RAII to ensure operations are rolled back on failure.
pub fn transaction_example() -> Result<(), KernelError> {
    println!("[RAII Example] Starting transaction");
    
    // Track allocated resources
    let mut allocated_frames = Vec::new();
    
    // Create rollback guard
    let rollback = ScopeGuard::new(|| {
        println!("[RAII Example] Rolling back transaction!");
        // Cleanup would happen here
    });
    
    // Allocate resources
    for i in 0..3 {
        match FRAME_ALLOCATOR.allocate_frame_raii() {
            Ok(frame) => {
                println!("[RAII Example] Transaction: allocated frame {}", i);
                allocated_frames.push(frame);
            }
            Err(e) => {
                println!("[RAII Example] Transaction failed at step {}: {:?}", i, e);
                return Err(KernelError::OutOfMemory {
                    requested: 1,
                    available: 0,
                });
                // rollback guard executes here
            }
        }
    }
    
    // If we get here, transaction succeeded
    rollback.cancel();
    println!("[RAII Example] Transaction completed successfully");
    
    // allocated_frames will be cleaned up normally
    Ok(())
}

/// Example: Lock tracking for debugging
/// 
/// Uses TrackedMutexGuard to log lock acquisition/release.
pub fn lock_tracking_example() {
    use spin::Mutex;
    
    println!("[RAII Example] Demonstrating lock tracking");
    
    let data = Mutex::new(42);
    
    {
        let guard = data.lock();
        let tracked = TrackedMutexGuard::new(guard, "critical_data");
        
        println!("[RAII Example] Working with locked data: {}", *tracked);
        
        // Lock release will be logged when tracked drops
    }
    
    println!("[RAII Example] Lock tracking complete");
}

/// Example: Process resource cleanup simulation
/// 
/// Shows how ProcessResources RAII guard ensures cleanup.
#[cfg(feature = "alloc")]
pub fn process_cleanup_example() -> Result<(), KernelError> {
    use crate::process::ThreadId;
    use alloc::vec;
    
    println!("[RAII Example] Simulating process resource cleanup");
    
    // Create mock process resources
    let pid = ProcessId(999);
    let threads = vec![ThreadId(1), ThreadId(2), ThreadId(3)];
    let cap_space = Arc::new(Mutex::new(crate::cap::CapabilitySpace::new()));
    let mem_space = Arc::new(Mutex::new(crate::mm::VirtualAddressSpace::new()));
    
    // Create RAII guard for process resources
    let _resources = ProcessResources::new(
        pid,
        threads,
        cap_space,
        mem_space
    );
    
    println!("[RAII Example] Process resources created");
    
    // When _resources drops, it will:
    // 1. Terminate all threads
    // 2. Revoke all capabilities
    // 3. Destroy memory space
    
    Ok(())
}

/// Run all RAII examples
pub fn run_all_examples() {
    println!("\n=== Running RAII Examples ===\n");
    
    // DMA buffer example
    if let Ok(dma_buffer) = allocate_dma_buffer(4) {
        println!("[RAII Example] DMA buffer will be freed automatically");
    }
    
    // Temporary mapping example
    if let Some(current) = current_process() {
        let _ = temporary_mapping_example(current.pid);
    }
    
    // Complex resource example
    let _ = complex_resource_example();
    
    // Transaction example
    let _ = transaction_example();
    
    // Lock tracking example
    lock_tracking_example();
    
    println!("\n=== RAII Examples Complete ===\n");
}
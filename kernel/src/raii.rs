//! RAII (Resource Acquisition Is Initialization) patterns for kernel resources
//!
//! This module provides RAII wrappers for various kernel resources to ensure
//! proper cleanup when resources go out of scope.

use core::{
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::sync::Arc;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// Stub Vec for no-alloc builds
#[cfg(not(feature = "alloc"))]
struct Vec<T> {
    _phantom: core::marker::PhantomData<T>,
}

#[cfg(not(feature = "alloc"))]
impl<T> Vec<T> {
    fn len(&self) -> usize {
        0
    }
    fn clone(&self) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

use spin::{Mutex, MutexGuard};

use crate::{
    cap::{CapabilityId, CapabilitySpace},
    mm::{frame_allocator::FrameAllocator, PhysicalFrame},
    println,
    process::ProcessId,
};

/// RAII wrapper for physical frames
///
/// Automatically returns frames to the allocator when dropped
pub struct FrameGuard {
    frame: PhysicalFrame,
    allocator: &'static FrameAllocator,
}

impl FrameGuard {
    /// Create a new frame guard
    pub fn new(frame: PhysicalFrame, allocator: &'static FrameAllocator) -> Self {
        Self { frame, allocator }
    }

    /// Get the physical frame address
    pub fn addr(&self) -> usize {
        self.frame.addr()
    }

    /// Release ownership of the frame without deallocating
    pub fn leak(self) -> PhysicalFrame {
        let frame = self.frame;
        core::mem::forget(self);
        frame
    }
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        // Return the frame to the allocator
        unsafe {
            self.allocator.free_frame(self.frame);
        }
        println!("[RAII] Released frame at {:#x}", self.frame.addr());
    }
}

impl Deref for FrameGuard {
    type Target = PhysicalFrame;

    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

/// RAII wrapper for multiple frames
pub struct FramesGuard {
    frames: Vec<PhysicalFrame>,
    #[allow(dead_code)]
    count: usize,
    allocator: &'static FrameAllocator,
}

impl FramesGuard {
    /// Create a new frames guard
    pub fn new(frames: Vec<PhysicalFrame>, allocator: &'static FrameAllocator) -> Self {
        let count = frames.len();
        Self {
            frames,
            count,
            allocator,
        }
    }

    /// Release ownership of the frames without deallocating
    pub fn leak(self) -> Vec<PhysicalFrame> {
        let frames = self.frames.clone();
        core::mem::forget(self);
        frames
    }
}

impl Drop for FramesGuard {
    fn drop(&mut self) {
        // Return all frames to the allocator
        for frame in &self.frames {
            unsafe {
                self.allocator.free_frame(*frame);
            }
        }
        println!("[RAII] Released {} frames", self.count);
    }
}

/// RAII wrapper for mapped memory regions
pub struct MappedRegion {
    virt_addr: usize,
    size: usize,
    process_id: ProcessId,
}

impl MappedRegion {
    /// Create a new mapped region guard
    pub fn new(virt_addr: usize, size: usize, process_id: ProcessId) -> Self {
        Self {
            virt_addr,
            size,
            process_id,
        }
    }

    /// Get the virtual address
    pub fn addr(&self) -> usize {
        self.virt_addr
    }

    /// Get the size
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for MappedRegion {
    fn drop(&mut self) {
        // Unmap the region from the process's address space
        if let Some(process) = crate::process::find_process(self.process_id) {
            let memory_space = process.memory_space.lock();
            if let Err(_e) = memory_space.unmap(self.virt_addr, self.size) {
                println!(
                    "[RAII] Warning: Failed to unmap region at {:#x}: {:?}",
                    self.virt_addr, _e
                );
            } else {
                println!(
                    "[RAII] Unmapped region at {:#x} (size: {:#x})",
                    self.virt_addr, self.size
                );
            }
        }
    }
}

/// RAII wrapper for capability space operations
pub struct CapabilityGuard {
    cap_id: CapabilityId,
    space: Arc<Mutex<CapabilitySpace>>,
}

impl CapabilityGuard {
    /// Create a new capability guard
    pub fn new(cap_id: CapabilityId, space: Arc<Mutex<CapabilitySpace>>) -> Self {
        Self { cap_id, space }
    }

    /// Get the capability ID
    pub fn id(&self) -> CapabilityId {
        self.cap_id
    }

    /// Release ownership without revoking
    pub fn leak(self) -> CapabilityId {
        let id = self.cap_id;
        core::mem::forget(self);
        id
    }
}

impl Drop for CapabilityGuard {
    fn drop(&mut self) {
        // Revoke the capability
        let mut space = self.space.lock();
        if let Err(_e) = space.revoke(self.cap_id) {
            println!(
                "[RAII] Warning: Failed to revoke capability {}: {:?}",
                self.cap_id, _e
            );
        } else {
            println!("[RAII] Revoked capability {}", self.cap_id);
        }
    }
}

/// RAII wrapper for process resources
///
/// Ensures all process resources are cleaned up when the process exits
#[cfg(feature = "alloc")]
pub struct ProcessResources {
    pid: ProcessId,
    // We use ManuallyDrop to control the order of cleanup
    threads: ManuallyDrop<Vec<crate::process::ThreadId>>,
    capabilities: ManuallyDrop<Arc<Mutex<CapabilitySpace>>>,
    memory_space: ManuallyDrop<Arc<Mutex<crate::mm::VirtualAddressSpace>>>,
}

#[cfg(feature = "alloc")]
impl ProcessResources {
    /// Create a new process resources guard
    pub fn new(
        pid: ProcessId,
        threads: Vec<crate::process::ThreadId>,
        capabilities: Arc<Mutex<CapabilitySpace>>,
        memory_space: Arc<Mutex<crate::mm::VirtualAddressSpace>>,
    ) -> Self {
        Self {
            pid,
            threads: ManuallyDrop::new(threads),
            capabilities: ManuallyDrop::new(capabilities),
            memory_space: ManuallyDrop::new(memory_space),
        }
    }
}

#[cfg(feature = "alloc")]
impl Drop for ProcessResources {
    fn drop(&mut self) {
        println!("[RAII] Cleaning up resources for process {}", self.pid);

        // 1. First terminate all threads
        for &thread_id in self.threads.iter() {
            if let Err(_e) = crate::process::terminate_thread(self.pid, thread_id) {
                println!(
                    "[RAII] Warning: Failed to terminate thread {:?}: {:?}",
                    thread_id, _e
                );
            }
        }

        // 2. Then revoke all capabilities
        unsafe {
            let capabilities = ManuallyDrop::take(&mut self.capabilities);
            let mut cap_space = capabilities.lock();
            cap_space.revoke_all();
        }

        // 3. Finally clean up memory space
        unsafe {
            let memory_space = ManuallyDrop::take(&mut self.memory_space);
            let mut mem_space = memory_space.lock();
            mem_space.destroy();
        }

        println!("[RAII] Process {} resources cleaned up", self.pid);
    }
}

/// RAII lock guard that logs acquisition and release
pub struct TrackedMutexGuard<'a, T> {
    guard: MutexGuard<'a, T>,
    #[allow(dead_code)]
    name: &'static str,
}

impl<'a, T> TrackedMutexGuard<'a, T> {
    /// Create a new tracked mutex guard
    pub fn new(guard: MutexGuard<'a, T>, name: &'static str) -> Self {
        println!("[RAII] Acquired lock: {}", name);
        Self { guard, name }
    }
}

impl<T> Drop for TrackedMutexGuard<'_, T> {
    fn drop(&mut self) {
        println!("[RAII] Released lock: {}", self.name);
    }
}

impl<T> Deref for TrackedMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T> DerefMut for TrackedMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

/// RAII wrapper for IPC channel cleanup
pub struct ChannelGuard {
    channel_id: u64,
}

impl ChannelGuard {
    /// Create a new channel guard
    pub fn new(channel_id: u64) -> Self {
        Self { channel_id }
    }

    /// Get the channel ID
    pub fn id(&self) -> u64 {
        self.channel_id
    }

    /// Release ownership without cleanup
    pub fn leak(self) -> u64 {
        let id = self.channel_id;
        core::mem::forget(self);
        id
    }
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        // Remove from global registry
        if let Err(_e) = crate::ipc::registry::remove_channel(self.channel_id) {
            println!(
                "[RAII] Warning: Failed to remove channel {}: {:?}",
                self.channel_id, _e
            );
        } else {
            println!("[RAII] Removed channel {} from registry", self.channel_id);
        }
    }
}

/// Macro to create RAII scope guards
#[macro_export]
macro_rules! defer {
    ($e:expr) => {
        let _guard = $crate::raii::ScopeGuard::new(|| $e);
    };
}

/// Generic scope guard that runs cleanup code on drop
pub struct ScopeGuard<F: FnOnce()> {
    cleanup: Option<F>,
}

impl<F: FnOnce()> ScopeGuard<F> {
    /// Create a new scope guard
    pub fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }

    /// Cancel the cleanup
    pub fn cancel(mut self) {
        self.cleanup = None;
    }
}

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_guard() {
        let mut cleaned = false;
        {
            let _guard = ScopeGuard::new(|| {
                cleaned = true;
            });
        }
        assert!(cleaned);
    }

    #[test]
    fn test_scope_guard_cancel() {
        let mut cleaned = false;
        {
            let guard = ScopeGuard::new(|| {
                cleaned = true;
            });
            guard.cancel();
        }
        assert!(!cleaned);
    }
}

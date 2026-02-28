//! DMA Buffer Pool for Zero-Copy Networking
//!
//! Provides pre-allocated DMA-capable buffers for network packet transmission
//! and reception, enabling zero-copy operation with minimal allocation
//! overhead.
//!
//! Buffers are allocated from physical frames below 4GB for 32-bit DMA
//! compatibility. Each buffer gets one 4KB frame; the usable portion is
//! `DMA_BUFFER_SIZE` (2048 bytes) to accommodate network MTU + headers.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::{
    error::KernelError,
    mm::{
        frame_allocator::MemoryZone, phys_to_virt_addr, FrameNumber, PhysicalAddress,
        FRAME_ALLOCATOR, FRAME_SIZE,
    },
};

/// Standard network buffer size (1500 MTU + headers + alignment)
pub const DMA_BUFFER_SIZE: usize = 2048;

/// Maximum number of buffers in the pool
pub const MAX_BUFFERS: usize = 512;

/// Maximum physical address for 32-bit DMA compatibility (4GB)
const DMA_PHYS_LIMIT: u64 = 0x1_0000_0000;

/// DMA Buffer
pub struct DmaBuffer {
    /// Virtual address of the buffer
    virt_addr: usize,

    /// Physical address for DMA
    phys_addr: PhysicalAddress,

    /// Buffer size in bytes
    size: usize,

    /// Reference count for buffer ownership
    refcount: AtomicU64,

    /// Buffer index in pool
    index: u16,

    /// Frame number backing this buffer (for deallocation)
    #[allow(dead_code)] // Needed for future pool teardown / frame reclamation
    frame: FrameNumber,
}

impl DmaBuffer {
    /// Create a new DMA buffer with explicit addresses
    #[allow(dead_code)] // Used in tests for direct construction
    fn new(virt_addr: usize, phys_addr: PhysicalAddress, size: usize, index: u16) -> Self {
        Self {
            virt_addr,
            phys_addr,
            size,
            refcount: AtomicU64::new(0),
            index,
            frame: FrameNumber::new(phys_addr.as_u64() / FRAME_SIZE as u64),
        }
    }

    /// Create a DMA buffer from an allocated physical frame.
    ///
    /// Converts the frame number to physical and virtual addresses using the
    /// kernel's direct physical memory mapping.
    pub fn from_frame(frame: FrameNumber, index: u16) -> Self {
        let phys_addr = PhysicalAddress::new(frame.as_u64() * FRAME_SIZE as u64);
        let virt_addr = phys_to_virt_addr(phys_addr.as_u64()) as usize;

        Self {
            virt_addr,
            phys_addr,
            size: DMA_BUFFER_SIZE,
            refcount: AtomicU64::new(0),
            index,
            frame,
        }
    }

    /// Get virtual address
    pub fn virt_addr(&self) -> usize {
        self.virt_addr
    }

    /// Get physical address for DMA
    pub fn phys_addr(&self) -> PhysicalAddress {
        self.phys_addr
    }

    /// Get buffer size
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get buffer index in pool
    pub fn index(&self) -> u16 {
        self.index
    }

    /// Get buffer as slice
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: virt_addr points to a DMA buffer of exactly `size` bytes allocated
        // during pool creation from the frame allocator. The buffer remains valid for
        // the lifetime of the pool. We hold &self so no mutable alias exists.
        unsafe { core::slice::from_raw_parts(self.virt_addr as *const u8, self.size) }
    }

    /// Get buffer as mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: virt_addr points to a DMA buffer of exactly `size` bytes allocated
        // during pool creation from the frame allocator. We hold &mut self so no other
        // reference to this buffer exists, making the mutable slice safe.
        unsafe { core::slice::from_raw_parts_mut(self.virt_addr as *mut u8, self.size) }
    }

    /// Increment reference count
    pub fn acquire(&self) -> u64 {
        self.refcount.fetch_add(1, Ordering::Relaxed)
    }

    /// Decrement reference count
    pub fn release(&self) -> u64 {
        self.refcount.fetch_sub(1, Ordering::Release)
    }

    /// Check if buffer is free (refcount == 0)
    pub fn is_free(&self) -> bool {
        self.refcount.load(Ordering::Acquire) == 0
    }
}

/// DMA Buffer Pool
pub struct DmaBufferPool {
    /// Pool of DMA buffers
    buffers: Vec<DmaBuffer>,

    /// Free list (indices of available buffers)
    free_list: Vec<u16>,

    /// Total buffers allocated
    total_buffers: usize,

    /// Statistics
    allocations: AtomicU64,
    deallocations: AtomicU64,
    allocation_failures: AtomicU64,
}

impl DmaBufferPool {
    /// Create a new DMA buffer pool with physically contiguous frames.
    ///
    /// Allocates `num_buffers` frames from the frame allocator for DMA use.
    /// Frames are filtered to be below 4GB for 32-bit DMA engine compatibility.
    /// Each frame provides one `DMA_BUFFER_SIZE` buffer. If allocation fails
    /// for some frames, the pool is created with however many were successful.
    pub fn new(num_buffers: usize) -> Result<Self, KernelError> {
        if num_buffers > MAX_BUFFERS {
            return Err(KernelError::InvalidArgument {
                name: "num_buffers",
                value: "exceeds_max",
            });
        }

        let mut buffers = Vec::with_capacity(num_buffers);
        let mut free_list = Vec::with_capacity(num_buffers);
        let mut allocated = 0usize;

        let allocator = FRAME_ALLOCATOR.lock();

        for i in 0..num_buffers {
            // Allocate from the Normal zone (16MB-MAX on 64-bit) and then filter
            // for <4GB. The DMA zone only covers 0-16MB which is often reserved.
            let frame = match allocator.allocate_frames_in_zone(1, None, Some(MemoryZone::Normal)) {
                Ok(f) => f,
                Err(_) => {
                    // Try without zone constraint as fallback
                    match allocator.allocate_frames(1, None) {
                        Ok(f) => f,
                        Err(_) => break, // No more frames available
                    }
                }
            };

            let phys_addr = frame.as_u64() * FRAME_SIZE as u64;

            // Filter: DMA buffers must be below 4GB for 32-bit DMA engines
            if phys_addr >= DMA_PHYS_LIMIT {
                // Frame is above 4GB -- free it and continue trying.
                // On systems with limited low memory this may exhaust quickly.
                let _ = allocator.free_frames(frame, 1);
                continue;
            }

            // Zero-initialize the buffer memory for safety
            let virt = phys_to_virt_addr(phys_addr) as *mut u8;
            // SAFETY: virt points to a freshly allocated frame of FRAME_SIZE bytes.
            // The frame allocator guarantees this memory is not in use. We zero it
            // before handing it out to prevent information leaks.
            unsafe {
                core::ptr::write_bytes(virt, 0, FRAME_SIZE);
            }

            let buffer = DmaBuffer::from_frame(frame, i as u16);
            free_list.push(i as u16);
            buffers.push(buffer);
            allocated += 1;
        }

        drop(allocator);

        if allocated == 0 && num_buffers > 0 {
            return Err(KernelError::OutOfMemory {
                requested: num_buffers * FRAME_SIZE,
                available: 0,
            });
        }

        println!(
            "[DMA-POOL] Allocated {}/{} DMA buffers ({}KB, all below 4GB)",
            allocated,
            num_buffers,
            allocated * DMA_BUFFER_SIZE / 1024,
        );

        Ok(Self {
            buffers,
            free_list,
            total_buffers: allocated,
            allocations: AtomicU64::new(0),
            deallocations: AtomicU64::new(0),
            allocation_failures: AtomicU64::new(0),
        })
    }

    /// Allocate a buffer from the pool
    pub fn alloc(&mut self) -> Result<&mut DmaBuffer, KernelError> {
        if let Some(index) = self.free_list.pop() {
            let buffer = &mut self.buffers[index as usize];
            buffer.acquire();
            self.allocations.fetch_add(1, Ordering::Relaxed);
            Ok(buffer)
        } else {
            self.allocation_failures.fetch_add(1, Ordering::Relaxed);
            Err(KernelError::ResourceExhausted {
                resource: "dma_buffers",
            })
        }
    }

    /// Free a buffer back to the pool
    pub fn free(&mut self, buffer_index: u16) -> Result<(), KernelError> {
        if buffer_index as usize >= self.buffers.len() {
            return Err(KernelError::InvalidArgument {
                name: "buffer_index",
                value: "out_of_range",
            });
        }

        let buffer = &self.buffers[buffer_index as usize];
        let prev_count = buffer.release();

        // Only return to free list if refcount reaches 0
        if prev_count == 1 {
            self.free_list.push(buffer_index);
            self.deallocations.fetch_add(1, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Get number of free buffers
    pub fn free_count(&self) -> usize {
        self.free_list.len()
    }

    /// Get total number of buffers
    pub fn total_count(&self) -> usize {
        self.total_buffers
    }

    /// Get allocation statistics
    pub fn stats(&self) -> DmaPoolStats {
        DmaPoolStats {
            total_buffers: self.total_buffers,
            free_buffers: self.free_list.len(),
            allocations: self.allocations.load(Ordering::Relaxed),
            deallocations: self.deallocations.load(Ordering::Relaxed),
            allocation_failures: self.allocation_failures.load(Ordering::Relaxed),
        }
    }
}

/// DMA Pool Statistics
#[derive(Debug, Clone, Copy)]
pub struct DmaPoolStats {
    pub total_buffers: usize,
    pub free_buffers: usize,
    pub allocations: u64,
    pub deallocations: u64,
    pub allocation_failures: u64,
}

/// Global DMA buffer pool for network operations
static NETWORK_DMA_POOL: Mutex<Option<DmaBufferPool>> = Mutex::new(None);

/// Initialize the global network DMA pool
pub fn init_network_pool(num_buffers: usize) -> Result<(), KernelError> {
    let mut pool_lock = NETWORK_DMA_POOL.lock();
    if pool_lock.is_some() {
        return Ok(());
    }

    let pool = DmaBufferPool::new(num_buffers)?;
    let stats = pool.stats();
    *pool_lock = Some(pool);

    println!(
        "[DMA-POOL] Network pool: {} buffers, {} free",
        stats.total_buffers, stats.free_buffers,
    );
    Ok(())
}

/// Execute a closure with the global network DMA pool (mutable access)
pub fn with_network_pool<R, F: FnOnce(&mut DmaBufferPool) -> R>(f: F) -> Result<R, KernelError> {
    let mut pool_lock = NETWORK_DMA_POOL.lock();
    pool_lock.as_mut().map(f).ok_or(KernelError::InvalidState {
        expected: "initialized",
        actual: "uninitialized",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dma_pool_constants() {
        assert_eq!(DMA_BUFFER_SIZE, 2048);
        assert!(MAX_BUFFERS >= 512);
        assert!(DMA_PHYS_LIMIT == 0x1_0000_0000);
    }

    #[test]
    fn test_dma_pool_exceeds_max() {
        let pool = DmaBufferPool::new(MAX_BUFFERS + 1);
        assert!(pool.is_err());
    }

    #[test]
    fn test_buffer_reference_counting() {
        let buffer = DmaBuffer::new(0x1000, PhysicalAddress(0x2000), 2048, 0);
        assert!(buffer.is_free());
        assert_eq!(buffer.index(), 0);
        assert_eq!(buffer.size(), 2048);
        assert_eq!(buffer.phys_addr().as_u64(), 0x2000);
        assert_eq!(buffer.virt_addr(), 0x1000);

        buffer.acquire();
        assert!(!buffer.is_free());

        buffer.release();
        assert!(buffer.is_free());
    }

    #[test]
    fn test_buffer_from_frame() {
        let frame = FrameNumber::new(0x100); // Frame 256 = physical 0x100000
        let buffer = DmaBuffer::from_frame(frame, 5);

        assert_eq!(buffer.index(), 5);
        assert_eq!(buffer.size(), DMA_BUFFER_SIZE);
        assert_eq!(buffer.phys_addr().as_u64(), 0x100 * FRAME_SIZE as u64);
        assert!(buffer.is_free());
    }
}

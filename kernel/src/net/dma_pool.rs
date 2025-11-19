//! DMA Buffer Pool for Zero-Copy Networking
//!
//! Provides pre-allocated DMA-capable buffers for network packet transmission
//! and reception, enabling zero-copy operation with minimal allocation overhead.

use crate::error::KernelError;
use crate::mm::PhysicalAddress;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// Standard network buffer size (1500 MTU + headers + alignment)
pub const DMA_BUFFER_SIZE: usize = 2048;

/// Maximum number of buffers in the pool
pub const MAX_BUFFERS: usize = 512;

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
}

impl DmaBuffer {
    /// Create a new DMA buffer
    fn new(virt_addr: usize, phys_addr: PhysicalAddress, size: usize, index: u16) -> Self {
        Self {
            virt_addr,
            phys_addr,
            size,
            refcount: AtomicU64::new(0),
            index,
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

    /// Get buffer as slice
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.virt_addr as *const u8, self.size) }
    }

    /// Get buffer as mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
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
    /// Create a new DMA buffer pool
    ///
    /// NOTE: This is a stub implementation. Proper DMA buffer allocation requires:
    /// 1. Physically contiguous memory allocation
    /// 2. Cache-coherent memory mapping
    /// 3. IOMMU configuration (if available)
    /// 4. Platform-specific DMA constraints
    pub fn new(num_buffers: usize) -> Result<Self, KernelError> {
        if num_buffers > MAX_BUFFERS {
            return Err(KernelError::InvalidArgument {
                name: "num_buffers",
                value: "exceeds_max",
            });
        }

        let buffers = Vec::with_capacity(num_buffers);
        let free_list = Vec::with_capacity(num_buffers);

        // TODO: Proper DMA buffer allocation
        // For now, this is a placeholder that documents the requirements

        println!("[DMA-POOL] Created buffer pool with {} buffers (stub)", num_buffers);
        println!("[DMA-POOL] NOTE: Proper implementation requires:");
        println!("[DMA-POOL]   - Physically contiguous memory allocation");
        println!("[DMA-POOL]   - Cache-coherent DMA mapping");
        println!("[DMA-POOL]   - IOMMU support (if available)");

        Ok(Self {
            buffers,
            free_list,
            total_buffers: num_buffers,
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
static mut NETWORK_DMA_POOL: Option<DmaBufferPool> = None;

/// Initialize the global network DMA pool
pub fn init_network_pool(num_buffers: usize) -> Result<(), KernelError> {
    unsafe {
        if NETWORK_DMA_POOL.is_some() {
            return Err(KernelError::InvalidState {
                expected: "uninitialized",
                actual: "initialized",
            });
        }

        let pool = DmaBufferPool::new(num_buffers)?;
        NETWORK_DMA_POOL = Some(pool);

        println!("[DMA-POOL] Global network DMA pool initialized");
        Ok(())
    }
}

/// Get the global network DMA pool
pub fn get_network_pool() -> Result<&'static mut DmaBufferPool, KernelError> {
    unsafe {
        NETWORK_DMA_POOL.as_mut().ok_or(KernelError::InvalidState {
            expected: "initialized",
            actual: "uninitialized",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_dma_pool_creation() {
        let pool = DmaBufferPool::new(16);
        assert!(pool.is_ok());
    }

    #[test_case]
    fn test_buffer_reference_counting() {
        let buffer = DmaBuffer::new(0x1000, PhysicalAddress(0x2000), 2048, 0);
        assert!(buffer.is_free());

        buffer.acquire();
        assert!(!buffer.is_free());

        buffer.release();
        assert!(buffer.is_free());
    }
}

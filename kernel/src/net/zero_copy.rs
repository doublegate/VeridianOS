//! Zero-Copy Networking
//!
//! Eliminates memory copies in the network stack for maximum throughput.
//!
//! ## Zero-Copy Techniques
//!
//! 1. **DMA Buffers**: Direct memory access from network card
//! 2. **Scatter-Gather I/O**: Compose packets from multiple buffers
//! 3. **Page Remapping**: Transfer ownership instead of copying data
//! 4. **sendfile()**: Kernel-to-kernel transfer bypassing user space
//! 5. **TCP_CORK**: Batch small writes into single packet
//! 6. **Memory Mapping**: mmap() network buffers to user space

use alloc::{collections::VecDeque, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::error::KernelError;

/// DMA buffer pool for zero-copy operations
pub struct DmaBufferPool {
    /// Pre-allocated DMA-capable buffers
    free_buffers: Mutex<VecDeque<DmaBuffer>>,
    /// Total buffers allocated
    total_buffers: AtomicU64,
    /// Buffers currently in use
    in_use: AtomicU64,
    /// Buffer size (typically 2KB for networking)
    buffer_size: usize,
}

/// DMA-capable buffer
pub struct DmaBuffer {
    /// Physical address (for DMA)
    pub physical_addr: u64,
    /// Virtual address (for CPU access)
    pub virtual_addr: u64,
    /// Buffer size
    pub size: usize,
}

impl DmaBuffer {
    /// Create new DMA buffer
    pub fn new(size: usize) -> Result<Self, KernelError> {
        // TODO(phase6): Allocate DMA-capable memory (below 4GB for 32-bit DMA)

        Ok(Self {
            physical_addr: 0,
            virtual_addr: 0,
            size,
        })
    }

    /// Get mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: virtual_addr points to a DMA-capable buffer of exactly `size` bytes
        // allocated for zero-copy networking. We hold &mut self so no other reference
        // to this buffer memory exists.
        unsafe { core::slice::from_raw_parts_mut(self.virtual_addr as *mut u8, self.size) }
    }

    /// Get immutable slice
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: virtual_addr points to a DMA-capable buffer of exactly `size` bytes.
        // We hold &self so no mutable alias exists.
        unsafe { core::slice::from_raw_parts(self.virtual_addr as *const u8, self.size) }
    }
}

impl DmaBufferPool {
    /// Create new DMA buffer pool
    pub fn new(buffer_size: usize, initial_count: usize) -> Self {
        let pool = Self {
            free_buffers: Mutex::new(VecDeque::new()),
            total_buffers: AtomicU64::new(0),
            in_use: AtomicU64::new(0),
            buffer_size,
        };

        // Pre-allocate initial buffers
        for _ in 0..initial_count {
            if let Ok(buf) = DmaBuffer::new(buffer_size) {
                pool.free_buffers.lock().push_back(buf);
                pool.total_buffers.fetch_add(1, Ordering::Relaxed);
            }
        }

        pool
    }

    /// Allocate a buffer from the pool
    pub fn alloc(&self) -> Option<DmaBuffer> {
        let mut free = self.free_buffers.lock();

        if let Some(buf) = free.pop_front() {
            self.in_use.fetch_add(1, Ordering::Relaxed);
            Some(buf)
        } else {
            // Pool exhausted - try to allocate new buffer
            drop(free);

            if let Ok(buf) = DmaBuffer::new(self.buffer_size) {
                self.total_buffers.fetch_add(1, Ordering::Relaxed);
                self.in_use.fetch_add(1, Ordering::Relaxed);
                Some(buf)
            } else {
                None
            }
        }
    }

    /// Return buffer to pool
    pub fn free(&self, buf: DmaBuffer) {
        self.free_buffers.lock().push_back(buf);
        self.in_use.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get pool statistics
    pub fn stats(&self) -> DmaPoolStats {
        DmaPoolStats {
            total_buffers: self.total_buffers.load(Ordering::Relaxed),
            in_use: self.in_use.load(Ordering::Relaxed),
            buffer_size: self.buffer_size,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DmaPoolStats {
    pub total_buffers: u64,
    pub in_use: u64,
    pub buffer_size: usize,
}

/// Scatter-gather list for zero-copy I/O
pub struct ScatterGatherList {
    /// List of buffer segments
    segments: Vec<ScatterGatherSegment>,
}

#[derive(Debug, Clone)]
pub struct ScatterGatherSegment {
    /// Physical address
    pub physical_addr: u64,
    /// Length in bytes
    pub length: usize,
}

impl ScatterGatherList {
    /// Create new scatter-gather list
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Add segment
    pub fn add_segment(&mut self, addr: u64, length: usize) {
        self.segments.push(ScatterGatherSegment {
            physical_addr: addr,
            length,
        });
    }

    /// Get total length
    pub fn total_length(&self) -> usize {
        self.segments.iter().map(|s| s.length).sum()
    }

    /// Get segments
    pub fn segments(&self) -> &[ScatterGatherSegment] {
        &self.segments
    }

    /// Copy to contiguous buffer (fallback)
    pub fn copy_to_buffer(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        let mut offset = 0;

        for segment in &self.segments {
            if offset + segment.length > buf.len() {
                return Err(KernelError::OutOfMemory {
                    requested: offset + segment.length,
                    available: buf.len(),
                });
            }

            // TODO(phase6): Copy data from physical address to contiguous buffer

            offset += segment.length;
        }

        Ok(offset)
    }
}

impl Default for ScatterGatherList {
    fn default() -> Self {
        Self::new()
    }
}

/// Zero-copy send operation
pub struct ZeroCopySend {
    /// Scatter-gather list
    sg_list: ScatterGatherList,
    /// Completion callback
    completion: Option<fn()>,
}

impl ZeroCopySend {
    /// Create new zero-copy send
    pub fn new() -> Self {
        Self {
            sg_list: ScatterGatherList::new(),
            completion: None,
        }
    }

    /// Add data from user buffer (zero-copy via page remapping)
    pub fn add_user_buffer(&mut self, user_addr: u64, length: usize) -> Result<(), KernelError> {
        // TODO(phase6): Pin user pages and translate to physical addresses

        self.sg_list.add_segment(user_addr, length);
        Ok(())
    }

    /// Set completion callback
    pub fn on_complete(&mut self, callback: fn()) {
        self.completion = Some(callback);
    }

    /// Execute send (hardware-assisted)
    pub fn execute(&self) -> Result<(), KernelError> {
        // TODO(phase6): Program network card DMA engine with scatter-gather list

        Ok(())
    }
}

impl Default for ZeroCopySend {
    fn default() -> Self {
        Self::new()
    }
}

/// Kernel-to-kernel transfer (sendfile equivalent)
pub struct SendFile {
    /// Source file descriptor
    source_fd: u32,
    /// Destination socket
    dest_socket: u32,
    /// Offset in source
    offset: u64,
    /// Bytes to transfer
    count: usize,
}

impl SendFile {
    /// Create new sendfile operation
    pub fn new(source_fd: u32, dest_socket: u32, offset: u64, count: usize) -> Self {
        Self {
            source_fd,
            dest_socket,
            offset,
            count,
        }
    }

    /// Execute transfer without copying to user space
    pub fn execute(&self) -> Result<usize, KernelError> {
        // TODO(phase5): Implement kernel-to-kernel sendfile transfer via page remapping

        let _ = (self.source_fd, self.dest_socket, self.offset);
        Ok(self.count)
    }
}

/// TCP Cork (batch small writes)
pub struct TcpCork {
    /// Pending data
    pending: Vec<u8>,
    /// Maximum pending size before flush
    max_pending: usize,
}

impl TcpCork {
    /// Create new TCP cork
    pub fn new(max_pending: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_pending,
        }
    }

    /// Add data (may not send immediately)
    pub fn write(&mut self, data: &[u8]) -> Result<(), KernelError> {
        self.pending.extend_from_slice(data);

        if self.pending.len() >= self.max_pending {
            self.flush()?;
        }

        Ok(())
    }

    /// Force send pending data
    pub fn flush(&mut self) -> Result<(), KernelError> {
        if !self.pending.is_empty() {
            // TODO(phase6): Send pending data via TCP socket
            self.pending.clear();
        }
        Ok(())
    }
}

/// Statistics for zero-copy operations
pub struct ZeroCopyStats {
    /// Total bytes transferred without copying
    pub zero_copy_bytes: AtomicU64,
    /// Total bytes that required copying (fallback)
    pub copied_bytes: AtomicU64,
    /// Number of zero-copy operations
    pub zero_copy_ops: AtomicU64,
    /// Number of copy operations
    pub copy_ops: AtomicU64,
}

impl ZeroCopyStats {
    pub const fn new() -> Self {
        Self {
            zero_copy_bytes: AtomicU64::new(0),
            copied_bytes: AtomicU64::new(0),
            zero_copy_ops: AtomicU64::new(0),
            copy_ops: AtomicU64::new(0),
        }
    }

    pub fn record_zero_copy(&self, bytes: u64) {
        self.zero_copy_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.zero_copy_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_copy(&self, bytes: u64) {
        self.copied_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.copy_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_efficiency(&self) -> f64 {
        let zc = self.zero_copy_bytes.load(Ordering::Relaxed) as f64;
        let cp = self.copied_bytes.load(Ordering::Relaxed) as f64;

        if zc + cp == 0.0 {
            return 0.0;
        }

        (zc / (zc + cp)) * 100.0
    }
}

impl Default for ZeroCopyStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Global zero-copy statistics
pub static ZERO_COPY_STATS: ZeroCopyStats = ZeroCopyStats::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dma_buffer_pool() {
        let pool = DmaBufferPool::new(2048, 10);
        let stats = pool.stats();

        assert_eq!(stats.total_buffers, 10);
        assert_eq!(stats.in_use, 0);

        let buf = pool.alloc();
        assert!(buf.is_some());

        let stats = pool.stats();
        assert_eq!(stats.in_use, 1);
    }

    #[test]
    fn test_scatter_gather() {
        let mut sg = ScatterGatherList::new();
        sg.add_segment(0x1000, 512);
        sg.add_segment(0x2000, 1024);

        assert_eq!(sg.total_length(), 1536);
        assert_eq!(sg.segments().len(), 2);
    }

    #[test]
    fn test_zero_copy_stats() {
        let stats = ZeroCopyStats::new();
        stats.record_zero_copy(1000);
        stats.record_copy(100);

        let efficiency = stats.get_efficiency();
        assert!(efficiency > 90.0); // 1000/(1000+100) = 90.9%
    }
}

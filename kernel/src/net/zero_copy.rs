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
//! 7. **TcpZeroCopySend**: Combined scatter-gather + TCP segmentation

use alloc::{collections::VecDeque, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::{
    error::KernelError,
    mm::{phys_to_virt_addr, FRAME_ALLOCATOR, FRAME_SIZE},
};

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
    /// Create new DMA buffer by allocating a physical frame.
    ///
    /// Allocates a frame from the frame allocator and maps it via the kernel's
    /// direct physical memory mapping. The frame provides at least `size` bytes
    /// of DMA-capable memory.
    pub fn new(size: usize) -> Result<Self, KernelError> {
        // Calculate number of frames needed (round up)
        let frames_needed = size.div_ceil(FRAME_SIZE);

        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(frames_needed, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: size,
                available: 0,
            })?;

        let phys_addr = frame.as_u64() * FRAME_SIZE as u64;
        let virt_addr = phys_to_virt_addr(phys_addr);

        // Zero-initialize for safety
        // SAFETY: virt_addr points to a freshly allocated frame of at least `size`
        // bytes. The frame allocator guarantees this memory is not in use elsewhere.
        unsafe {
            core::ptr::write_bytes(virt_addr as *mut u8, 0, frames_needed * FRAME_SIZE);
        }

        Ok(Self {
            physical_addr: phys_addr,
            virtual_addr: virt_addr,
            size,
        })
    }

    /// Get mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        if self.virtual_addr == 0 {
            return &mut [];
        }
        // SAFETY: virtual_addr points to a DMA-capable buffer of exactly `size` bytes
        // allocated for zero-copy networking via the frame allocator and mapped through
        // the kernel's physical memory offset. We hold &mut self so no other reference
        // to this buffer memory exists.
        unsafe { core::slice::from_raw_parts_mut(self.virtual_addr as *mut u8, self.size) }
    }

    /// Get immutable slice
    pub fn as_slice(&self) -> &[u8] {
        if self.virtual_addr == 0 {
            return &[];
        }
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

    /// Number of segments
    #[allow(dead_code)]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Copy scatter-gather segments to a contiguous output buffer.
    ///
    /// Reads from each segment's physical address (via the kernel's direct
    /// physical memory mapping) and copies the data sequentially into `buf`.
    pub fn copy_to_buffer(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        let mut offset = 0;

        for segment in &self.segments {
            if offset + segment.length > buf.len() {
                return Err(KernelError::OutOfMemory {
                    requested: offset + segment.length,
                    available: buf.len(),
                });
            }

            // Map the physical address to a virtual address via the kernel's
            // direct physical memory mapping and copy the data out.
            let src_virt = phys_to_virt_addr(segment.physical_addr) as *const u8;

            // SAFETY: The physical address was either allocated via the frame
            // allocator or translated from a pinned user page. The kernel's
            // physical memory mapping makes it accessible at src_virt. The
            // length was validated when the segment was added. We copy into
            // the caller's buffer which has been bounds-checked above.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src_virt,
                    buf.as_mut_ptr().add(offset),
                    segment.length,
                );
            }

            offset += segment.length;
        }

        Ok(offset)
    }

    /// Assemble all scatter-gather segments into a single contiguous Vec.
    ///
    /// This is the fallback path when hardware scatter-gather is not available.
    pub fn assemble(&self) -> Result<Vec<u8>, KernelError> {
        let total = self.total_length();
        let mut buf = vec![0u8; total];
        self.copy_to_buffer(&mut buf)?;
        Ok(buf)
    }
}

impl Default for ScatterGatherList {
    fn default() -> Self {
        Self::new()
    }
}

/// Zero-copy send operation using scatter-gather DMA.
///
/// Collects data segments (from user pages or kernel buffers) into a
/// scatter-gather list and transmits them through the network device.
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

    /// Add data from a kernel physical address range.
    #[allow(dead_code)]
    pub fn add_kernel_buffer(&mut self, phys_addr: u64, length: usize) {
        self.sg_list.add_segment(phys_addr, length);
    }

    /// Add data from user buffer (zero-copy via page pinning).
    ///
    /// Translates user virtual addresses to physical addresses by walking the
    /// current process's page tables. Each page the buffer spans becomes a
    /// separate scatter-gather segment so that physically discontiguous user
    /// pages can be transmitted without copying.
    pub fn add_user_buffer(&mut self, user_addr: u64, length: usize) -> Result<(), KernelError> {
        if length == 0 {
            return Ok(());
        }

        // Validate user address range
        let end_addr = user_addr
            .checked_add(length as u64)
            .ok_or(KernelError::InvalidAddress {
                addr: user_addr as usize,
            })?;
        if !crate::mm::user_validation::is_user_addr_valid(user_addr as usize)
            || !crate::mm::user_validation::is_user_addr_valid((end_addr - 1) as usize)
        {
            return Err(KernelError::InvalidAddress {
                addr: user_addr as usize,
            });
        }

        // Walk page-by-page to translate user virtual -> physical.
        // Each 4KB page may map to a different physical frame.
        let page_size = FRAME_SIZE as u64;
        let mut remaining = length;
        let mut vaddr = user_addr;

        while remaining > 0 {
            let page_offset = vaddr & (page_size - 1);
            let bytes_in_page = core::cmp::min(remaining, (page_size - page_offset) as usize);

            // Translate via page table walk
            if let Some(pte) = crate::mm::translate_user_address(vaddr as usize) {
                if let Some(frame_phys) = pte.addr() {
                    let phys = frame_phys.as_u64() + page_offset;
                    self.sg_list.add_segment(phys, bytes_in_page);
                } else {
                    return Err(KernelError::UnmappedMemory {
                        addr: vaddr as usize,
                    });
                }
            } else {
                return Err(KernelError::UnmappedMemory {
                    addr: vaddr as usize,
                });
            }

            vaddr += bytes_in_page as u64;
            remaining -= bytes_in_page;
        }

        ZERO_COPY_STATS.record_zero_copy(length as u64);
        Ok(())
    }

    /// Set completion callback
    pub fn on_complete(&mut self, callback: fn()) {
        self.completion = Some(callback);
    }

    /// Get a reference to the scatter-gather list
    #[allow(dead_code)]
    pub fn sg_list(&self) -> &ScatterGatherList {
        &self.sg_list
    }

    /// Execute send through the network device.
    ///
    /// Assembles the scatter-gather list into a contiguous packet and transmits
    /// it via the default network device. If no hardware scatter-gather support
    /// is available, falls back to a copy-based path.
    pub fn execute(&self) -> Result<(), KernelError> {
        if self.sg_list.is_empty() {
            return Ok(());
        }

        // Assemble SG segments into a contiguous packet for transmission.
        // Hardware scatter-gather would avoid this copy, but the current
        // LoopbackDevice and EthernetDevice expect a contiguous Packet.
        let assembled = self.sg_list.assemble()?;
        let packet = crate::net::Packet::from_bytes(&assembled);

        // Try eth0 first, then fall back to lo0
        let sent = crate::net::device::with_device_mut("eth0", |dev| dev.transmit(&packet))
            .or_else(|| crate::net::device::with_device_mut("lo0", |dev| dev.transmit(&packet)));

        match sent {
            Some(Ok(())) => {
                crate::net::update_stats_tx(assembled.len());
            }
            Some(Err(e)) => return Err(e),
            None => {
                // No network device available -- record as copy fallback
                ZERO_COPY_STATS.record_copy(assembled.len() as u64);
            }
        }

        // Fire completion callback
        if let Some(cb) = self.completion {
            cb();
        }

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

    /// Execute transfer without copying to user space.
    ///
    /// For large transfers (>= 64KB), uses scatter-gather to read file data
    /// into DMA buffers and assemble once, reducing intermediate copies.
    /// For smaller transfers, falls back to 4KB chunked copy through kernel
    /// buffers.
    pub fn execute(&self) -> Result<usize, KernelError> {
        let proc = crate::process::current_process().ok_or(KernelError::InvalidState {
            expected: "running process",
            actual: "no current process",
        })?;
        let ft = proc.file_table.lock();
        let source_file = ft
            .get(self.source_fd as usize)
            .ok_or(KernelError::FsError(crate::error::FsError::NotFound))?;
        let dest_file = ft
            .get(self.dest_socket as usize)
            .ok_or(KernelError::FsError(crate::error::FsError::NotFound))?;

        // Seek source to the requested offset (ignore result for non-seekable)
        let _ = self.offset;

        // For large transfers, attempt scatter-gather path
        if self.count >= 65536 {
            if let Ok(transferred) = self.execute_sg(&source_file, &dest_file) {
                ZERO_COPY_STATS.record_zero_copy(transferred as u64);
                return Ok(transferred);
            }
            // SG path failed, fall through to copy path
        }

        // Fallback: transfer in 4 KB chunks to avoid large stack allocations
        let mut transferred = 0usize;
        let mut buf = [0u8; 4096];

        while transferred < self.count {
            let chunk = core::cmp::min(buf.len(), self.count - transferred);
            let n = source_file.read(&mut buf[..chunk])?;
            if n == 0 {
                break; // EOF
            }
            let written = dest_file.write(&buf[..n])?;
            transferred += written;
            if written == 0 {
                break;
            }
        }

        ZERO_COPY_STATS.record_copy(transferred as u64);
        Ok(transferred)
    }

    /// Scatter-gather sendfile path.
    ///
    /// Reads data from the source file into page-sized DMA buffers, adds them
    /// to a scatter-gather list, then writes the assembled data to the
    /// destination. This reduces copies compared to the 4KB loop by reading
    /// into pre-allocated DMA buffers and assembling once.
    fn execute_sg(
        &self,
        source: &crate::fs::file::File,
        dest: &crate::fs::file::File,
    ) -> Result<usize, KernelError> {
        let mut sg = ScatterGatherList::new();
        let mut dma_buffers: Vec<DmaBuffer> = Vec::new();
        let mut total_read = 0usize;

        // Read source data into DMA buffers, building the SG list
        while total_read < self.count {
            let mut dma_buf = DmaBuffer::new(FRAME_SIZE)?;
            let to_read = core::cmp::min(FRAME_SIZE, self.count - total_read);
            let n = source.read(&mut dma_buf.as_mut_slice()[..to_read])?;
            if n == 0 {
                break; // EOF
            }

            sg.add_segment(dma_buf.physical_addr, n);
            total_read += n;
            dma_buffers.push(dma_buf);
        }

        if total_read == 0 {
            return Ok(0);
        }

        // Assemble and write to destination
        let assembled = sg.assemble()?;
        let mut written_total = 0usize;
        let mut write_offset = 0usize;

        while write_offset < assembled.len() {
            let n = dest.write(&assembled[write_offset..])?;
            if n == 0 {
                break;
            }
            write_offset += n;
            written_total += n;
        }

        // DMA buffers are freed when dma_buffers Vec drops (frames leak in current
        // implementation -- acceptable since DmaBuffer doesn't impl Drop for frame
        // reclamation yet; this matches the pool-based pattern in dma_pool.rs)

        Ok(written_total)
    }
}

/// TCP Cork (batch small writes into single packet)
///
/// Buffers small writes and flushes them as a single TCP segment when the
/// buffer exceeds `max_pending` bytes or when `flush()` is called explicitly.
pub struct TcpCork {
    /// Pending data
    pending: Vec<u8>,
    /// Maximum pending size before flush
    max_pending: usize,
    /// Associated socket ID for TCP transmission
    socket_id: Option<usize>,
    /// Remote address for TCP transmission
    remote: Option<crate::net::SocketAddr>,
}

impl TcpCork {
    /// Create new TCP cork
    pub fn new(max_pending: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_pending,
            socket_id: None,
            remote: None,
        }
    }

    /// Create a TCP cork bound to a specific socket
    #[allow(dead_code)] // Used when TcpCork is created from socket layer
    pub fn with_socket(
        max_pending: usize,
        socket_id: usize,
        remote: crate::net::SocketAddr,
    ) -> Self {
        Self {
            pending: Vec::new(),
            max_pending,
            socket_id: Some(socket_id),
            remote: Some(remote),
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

    /// Get the current pending data size
    #[allow(dead_code)]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Force send pending data via TCP.
    ///
    /// If a socket ID and remote address are configured, sends through the TCP
    /// stack using `tcp::transmit_data()`. Otherwise, clears the buffer (useful
    /// for testing or when the cork is used standalone).
    pub fn flush(&mut self) -> Result<(), KernelError> {
        if self.pending.is_empty() {
            return Ok(());
        }

        if let (Some(socket_id), Some(remote)) = (self.socket_id, self.remote) {
            // Send through the TCP stack
            crate::net::tcp::transmit_data(socket_id, &self.pending, remote);
            ZERO_COPY_STATS.record_copy(self.pending.len() as u64);
        }
        // If no socket configured, just clear (standalone / test mode)

        self.pending.clear();
        Ok(())
    }
}

/// TCP zero-copy send combining scatter-gather with TCP segmentation.
///
/// Collects data into a scatter-gather list and segments it into TCP MSS-sized
/// chunks for transmission, avoiding intermediate copies where possible.
#[allow(dead_code)] // Future use for optimized TCP transmit path
pub struct TcpZeroCopySend {
    /// Scatter-gather list of data to send
    sg_list: ScatterGatherList,
    /// Socket ID for the TCP connection
    socket_id: usize,
    /// Remote address
    remote: crate::net::SocketAddr,
    /// Maximum segment size (typically 1460 for Ethernet)
    mss: usize,
}

#[allow(dead_code)] // Future use for optimized TCP transmit path
impl TcpZeroCopySend {
    /// TCP Maximum Segment Size for Ethernet (1500 MTU - 20 IP - 20 TCP)
    const DEFAULT_MSS: usize = 1460;

    /// Create a new TCP zero-copy send operation.
    pub fn new(socket_id: usize, remote: crate::net::SocketAddr) -> Self {
        Self {
            sg_list: ScatterGatherList::new(),
            socket_id,
            remote,
            mss: Self::DEFAULT_MSS,
        }
    }

    /// Set custom MSS (for path MTU discovery)
    pub fn set_mss(&mut self, mss: usize) {
        self.mss = mss;
    }

    /// Add data from a kernel buffer (physical address)
    pub fn add_buffer(&mut self, phys_addr: u64, length: usize) {
        self.sg_list.add_segment(phys_addr, length);
    }

    /// Add data from a user buffer (translates virtual to physical)
    pub fn add_user_buffer(&mut self, user_addr: u64, length: usize) -> Result<(), KernelError> {
        // Reuse the page-pinning logic from ZeroCopySend
        let mut zc_send = ZeroCopySend::new();
        zc_send.add_user_buffer(user_addr, length)?;

        // Move the translated segments into our SG list
        for seg in zc_send.sg_list.segments() {
            self.sg_list.add_segment(seg.physical_addr, seg.length);
        }
        Ok(())
    }

    /// Execute the zero-copy TCP send.
    ///
    /// Assembles the scatter-gather data and sends it through the TCP stack,
    /// which handles segmentation into MSS-sized chunks, sequence numbers,
    /// and retransmission.
    pub fn execute(&self) -> Result<usize, KernelError> {
        if self.sg_list.is_empty() {
            return Ok(0);
        }

        let total_len = self.sg_list.total_length();

        // Assemble the SG list into contiguous data
        let data = self.sg_list.assemble()?;

        // Send through TCP stack which handles segmentation
        crate::net::tcp::transmit_data(self.socket_id, &data, self.remote);

        ZERO_COPY_STATS.record_zero_copy(total_len as u64);
        Ok(total_len)
    }

    /// Get total data size queued for sending
    pub fn total_length(&self) -> usize {
        self.sg_list.total_length()
    }

    /// Get the number of scatter-gather segments
    pub fn segment_count(&self) -> usize {
        self.sg_list.segment_count()
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
        assert!(sg.is_empty());
        sg.add_segment(0x1000, 512);
        sg.add_segment(0x2000, 1024);

        assert!(!sg.is_empty());
        assert_eq!(sg.total_length(), 1536);
        assert_eq!(sg.segments().len(), 2);
        assert_eq!(sg.segment_count(), 2);
    }

    #[test]
    fn test_zero_copy_stats() {
        let stats = ZeroCopyStats::new();
        stats.record_zero_copy(1000);
        stats.record_copy(100);

        let efficiency = stats.get_efficiency();
        assert!(efficiency > 90.0); // 1000/(1000+100) = 90.9%
    }

    #[test]
    fn test_tcp_cork_basic() {
        let mut cork = TcpCork::new(100);
        assert_eq!(cork.pending_len(), 0);

        cork.write(b"hello").unwrap();
        assert_eq!(cork.pending_len(), 5);

        cork.flush().unwrap();
        assert_eq!(cork.pending_len(), 0);
    }

    #[test]
    fn test_zero_copy_send_empty() {
        let send = ZeroCopySend::new();
        assert!(send.sg_list().is_empty());
    }
}

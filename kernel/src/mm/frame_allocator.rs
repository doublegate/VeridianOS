//! Physical frame allocator for VeridianOS
//!
//! Implements a hybrid allocator combining bitmap (for small allocations)
//! and buddy system (for large allocations) with NUMA awareness.

// Frame allocator -- bitmap+buddy hybrid, exercised during boot and page fault
#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use spin::Mutex;

// Import println! macro - may be no-op on some architectures
#[allow(unused_imports)]
use crate::println;
use crate::raii::{FrameGuard, FramesGuard};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// For non-alloc builds, provide Vec stub
#[cfg(not(feature = "alloc"))]
struct Vec<T> {
    _phantom: core::marker::PhantomData<T>,
}

#[cfg(not(feature = "alloc"))]
impl<T> Vec<T> {
    fn with_capacity(_: usize) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
    fn push(&mut self, _: T) {}
}

/// Size of a physical frame (4KB)
pub const FRAME_SIZE: usize = 4096;

/// Threshold for switching between bitmap and buddy allocator (512 frames =
/// 2MB)
const BITMAP_BUDDY_THRESHOLD: usize = 512;

/// Maximum number of NUMA nodes supported
const MAX_NUMA_NODES: usize = 8;

/// Memory zone for frame allocation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryZone {
    /// DMA zone (0-16MB on x86)
    Dma,
    /// Normal zone (16MB-4GB on 32-bit, all memory on 64-bit)
    Normal,
    /// High memory zone (>4GB on 32-bit, unused on 64-bit)
    High,
}

impl MemoryZone {
    /// Get the frame range for this zone on the current architecture
    pub fn frame_range(&self) -> (FrameNumber, FrameNumber) {
        match self {
            MemoryZone::Dma => (FrameNumber::new(0), FrameNumber::new(4096)), // 0-16MB
            MemoryZone::Normal => {
                #[cfg(target_pointer_width = "32")]
                {
                    (FrameNumber::new(4096), FrameNumber::new(1048576)) // 16MB-4GB
                }
                #[cfg(target_pointer_width = "64")]
                {
                    (FrameNumber::new(4096), FrameNumber::new(u64::MAX >> 12)) // 16MB-MAX
                }
            }
            MemoryZone::High => {
                #[cfg(target_pointer_width = "32")]
                {
                    (FrameNumber::new(1048576), FrameNumber::new(u64::MAX >> 12))
                    // 4GB-MAX
                }
                #[cfg(target_pointer_width = "64")]
                {
                    // High zone not used on 64-bit
                    (FrameNumber::new(0), FrameNumber::new(0))
                }
            }
        }
    }

    /// Check if a frame belongs to this zone
    pub fn contains(&self, frame: FrameNumber) -> bool {
        let (start, end) = self.frame_range();
        frame >= start && frame < end
    }

    /// Get the appropriate zone for a frame number
    pub fn for_frame(frame: FrameNumber) -> Self {
        if MemoryZone::Dma.contains(frame) {
            MemoryZone::Dma
        } else if MemoryZone::High.contains(frame) && cfg!(target_pointer_width = "32") {
            MemoryZone::High
        } else {
            MemoryZone::Normal
        }
    }
}

/// Physical frame number
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameNumber(u64);

impl FrameNumber {
    pub const fn new(num: u64) -> Self {
        Self(num)
    }

    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    pub const fn as_addr(&self) -> PhysicalAddress {
        PhysicalAddress::new(self.0 * FRAME_SIZE as u64)
    }
}

/// Physical memory address
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalAddress(pub u64);

impl PhysicalAddress {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    pub const fn as_usize(&self) -> usize {
        self.0 as usize
    }

    pub const fn as_frame(&self) -> FrameNumber {
        FrameNumber::new(self.0 / FRAME_SIZE as u64)
    }

    pub const fn offset(&self, offset: u64) -> Self {
        Self::new(self.0 + offset)
    }
}

/// Physical frame representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalFrame {
    number: FrameNumber,
}

impl PhysicalFrame {
    pub fn new(number: FrameNumber) -> Self {
        Self { number }
    }

    pub fn number(&self) -> FrameNumber {
        self.number
    }

    pub fn addr(&self) -> usize {
        (self.number.0 * FRAME_SIZE as u64) as usize
    }
}

/// Frame allocation result
pub type Result<T> = core::result::Result<T, FrameAllocatorError>;

/// Frame allocator errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAllocatorError {
    /// No frames available
    OutOfMemory,
    /// Invalid frame number
    InvalidFrame,
    /// Invalid allocation size
    InvalidSize,
    /// NUMA node not available
    InvalidNumaNode,
    /// Region overlaps with reserved memory
    ReservedMemoryConflict,
}

/// Reserved memory region
#[derive(Debug, Clone, Copy)]
pub struct ReservedRegion {
    /// Start frame number
    pub start: FrameNumber,
    /// End frame number (exclusive)
    pub end: FrameNumber,
    /// Description of what this region is reserved for
    pub description: &'static str,
}

/// Statistics for frame allocator
#[derive(Debug)]
pub struct FrameAllocatorStats {
    pub total_frames: u64,
    pub free_frames: u64,
    pub bitmap_allocations: u64,
    pub buddy_allocations: u64,
    pub allocation_time_ns: u64,
}

/// Bitmap allocator for small allocations (<512 frames)
struct BitmapAllocator {
    /// Bitmap tracking free frames (1 = free, 0 = allocated)
    /// Reduced from 16384 to 2048 for bootloader 0.11 compatibility (128K
    /// frames = 512MB)
    bitmap: Mutex<[u64; 2048]>,
    /// Starting frame number
    start_frame: FrameNumber,
    /// Total frames managed
    total_frames: usize,
    /// Free frame count
    free_frames: AtomicUsize,
}

impl BitmapAllocator {
    const fn new(start_frame: FrameNumber, frame_count: usize) -> Self {
        Self {
            bitmap: Mutex::new([u64::MAX; 2048]),
            start_frame,
            total_frames: frame_count,
            free_frames: AtomicUsize::new(frame_count),
        }
    }

    /// Allocate contiguous frames
    fn allocate(&self, count: usize) -> Result<FrameNumber> {
        if count == 0 || count >= BITMAP_BUDDY_THRESHOLD {
            return Err(FrameAllocatorError::InvalidSize);
        }

        let mut bitmap = self.bitmap.lock();

        // Find contiguous free frames
        let mut consecutive = 0;
        let mut start_bit = 0;

        for (word_idx, word) in bitmap.iter_mut().enumerate() {
            if *word == 0 {
                consecutive = 0;
                continue;
            }

            for bit in 0..64 {
                if *word & (1 << bit) != 0 {
                    if consecutive == 0 {
                        // Mark the start of a new consecutive sequence
                        start_bit = word_idx * 64 + bit;
                    }
                    consecutive += 1;
                    if consecutive == count {
                        // Found enough frames, allocate them
                        let first_frame = start_bit;

                        // Mark frames as allocated
                        for i in 0..count {
                            let frame_bit = first_frame + i;
                            let word_idx = frame_bit / 64;
                            let bit_idx = frame_bit % 64;
                            bitmap[word_idx] &= !(1 << bit_idx);
                        }

                        self.free_frames.fetch_sub(count, Ordering::Release);

                        return Ok(FrameNumber::new(
                            self.start_frame.as_u64() + first_frame as u64,
                        ));
                    }
                } else {
                    consecutive = 0;
                }
            }
        }

        Err(FrameAllocatorError::OutOfMemory)
    }

    /// Mark a specific frame as allocated (reserved) so it won't be handed out.
    /// Used to protect boot page table frames from being overwritten.
    fn mark_used(&self, frame: FrameNumber) -> Result<()> {
        let frame_num = frame.as_u64();
        let start = self.start_frame.as_u64();
        if frame_num < start || frame_num >= start + self.total_frames as u64 {
            // Frame is outside our range -- nothing to do
            return Ok(());
        }
        let offset = (frame_num - start) as usize;
        let word_idx = offset / 64;
        let bit_idx = offset % 64;

        let mut bitmap = self.bitmap.lock();
        if bitmap[word_idx] & (1 << bit_idx) != 0 {
            // Frame is currently free -- mark as allocated
            bitmap[word_idx] &= !(1 << bit_idx);
            self.free_frames.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Free previously allocated frames
    fn free(&self, frame: FrameNumber, count: usize) -> Result<()> {
        let offset = (frame.as_u64() - self.start_frame.as_u64()) as usize;

        if offset + count > self.total_frames {
            return Err(FrameAllocatorError::InvalidFrame);
        }

        let mut bitmap = self.bitmap.lock();

        // Mark frames as free
        for i in 0..count {
            let frame_bit = offset + i;
            let word_idx = frame_bit / 64;
            let bit_idx = frame_bit % 64;

            // Check if already free (double free detection)
            if bitmap[word_idx] & (1 << bit_idx) != 0 {
                return Err(FrameAllocatorError::InvalidFrame);
            }

            bitmap[word_idx] |= 1 << bit_idx;
        }

        self.free_frames.fetch_add(count, Ordering::Release);
        Ok(())
    }

    fn free_count(&self) -> usize {
        self.free_frames.load(Ordering::Acquire)
    }
}

/// Buddy allocator for large allocations (≥512 frames)
struct BuddyAllocator {
    /// Free lists for each order (order 0 = 1 frame, order 20 = 1M frames)
    free_lists: [Mutex<Option<BuddyBlock>>; 21],
    /// Starting frame
    start_frame: FrameNumber,
    /// Total frames (must be power of 2)
    total_frames: usize,
    /// Free frame count
    free_frames: AtomicUsize,
}

#[derive(Debug)]
struct BuddyBlock {
    frame: FrameNumber,
    #[cfg(feature = "alloc")]
    next: Option<Box<BuddyBlock>>,
    #[cfg(not(feature = "alloc"))]
    next: Option<*mut BuddyBlock>,
}

impl BuddyAllocator {
    fn new(start_frame: FrameNumber, frame_count: usize) -> Self {
        // Round down to nearest power of 2 (keep as-is if already power of 2)
        let total_frames = if frame_count.is_power_of_two() {
            frame_count
        } else {
            frame_count.next_power_of_two() / 2
        };

        let mut allocator = Self {
            free_lists: Default::default(),
            start_frame,
            total_frames,
            free_frames: AtomicUsize::new(total_frames),
        };

        // Initialize with one large block
        let max_order = total_frames.trailing_zeros() as usize;

        // Only initialize buddy allocator when alloc is available
        #[cfg(feature = "alloc")]
        {
            allocator.free_lists[max_order] = Mutex::new(Some(BuddyBlock {
                frame: start_frame,
                next: None,
            }));
        }

        allocator
    }

    /// Get the order (power of 2) for a given frame count
    fn get_order(count: usize) -> usize {
        count.next_power_of_two().trailing_zeros() as usize
    }

    /// Allocate frames of the given order
    fn allocate(&self, count: usize) -> Result<FrameNumber> {
        if count == 0 {
            return Err(FrameAllocatorError::InvalidSize);
        }

        #[cfg(not(feature = "alloc"))]
        {
            // Buddy allocator requires alloc feature
            return Err(FrameAllocatorError::OutOfMemory);
        }

        #[cfg(feature = "alloc")]
        {
            let order = Self::get_order(count);
            if order >= self.free_lists.len() {
                return Err(FrameAllocatorError::InvalidSize);
            }

            // Try to find a block of the right size
            for current_order in order..self.free_lists.len() {
                let mut list = self.free_lists[current_order].lock();

                if let Some(mut block) = list.take() {
                    // Remove block from free list
                    *list = block.next.take().map(|b| *b);

                    // Split block if necessary
                    let mut split_order = current_order;
                    while split_order > order {
                        split_order -= 1;
                        let buddy_frame =
                            FrameNumber::new(block.frame.as_u64() + (1 << split_order));

                        // Add buddy to free list
                        let mut buddy_list = self.free_lists[split_order].lock();
                        let buddy_block = BuddyBlock {
                            frame: buddy_frame,
                            next: buddy_list.take().map(Box::new),
                        };
                        *buddy_list = Some(buddy_block);
                    }

                    self.free_frames.fetch_sub(1 << order, Ordering::Release);
                    return Ok(block.frame);
                }
            }

            Err(FrameAllocatorError::OutOfMemory)
        }
    }

    /// Free frames back to the allocator
    fn free(&self, frame: FrameNumber, count: usize) -> Result<()> {
        #[cfg(not(feature = "alloc"))]
        {
            // Buddy allocator requires alloc feature
            return Err(FrameAllocatorError::InvalidFrame);
        }

        #[cfg(feature = "alloc")]
        {
            let order = Self::get_order(count);
            if order >= self.free_lists.len() {
                return Err(FrameAllocatorError::InvalidSize);
            }

            // Try to merge with buddy
            let mut current_frame = frame;
            let mut current_order = order;

            while current_order < self.free_lists.len() - 1 {
                let buddy_frame = FrameNumber::new(current_frame.as_u64() ^ (1 << current_order));

                // Check if buddy is free
                let mut list = self.free_lists[current_order].lock();
                let mut found_buddy = false;

                // Look for buddy in free list
                if let Some(ref mut head) = *list {
                    if head.frame == buddy_frame {
                        // Buddy is at head, remove it
                        *list = head.next.take().map(|b| *b);
                        found_buddy = true;
                    } else {
                        // Search for buddy in list - need to handle borrowing carefully
                        let mut prev: *mut BuddyBlock = head;
                        // SAFETY: We traverse the linked list of BuddyBlocks using raw
                        // pointers to work around Rust's borrow checker limitations with
                        // linked list mutation. `prev` always points to a valid BuddyBlock
                        // because: (1) it starts as `head`, which is a valid &mut reference,
                        // and (2) each iteration advances it to the next block obtained from
                        // a `Box<BuddyBlock>`, which is heap-allocated and valid. The list
                        // is protected by the Mutex on `self.free_lists[current_order]`,
                        // ensuring exclusive access. We only modify `prev.next` (removing
                        // one node) and then break, so no dangling pointers are created.
                        unsafe {
                            while let Some(ref mut next_box) = (*prev).next {
                                if next_box.frame == buddy_frame {
                                    // Remove buddy from list
                                    (*prev).next = next_box.next.take();
                                    found_buddy = true;
                                    break;
                                }
                                prev = &mut **next_box as *mut BuddyBlock;
                            }
                        }
                    }
                }

                if found_buddy {
                    // Merge with buddy
                    current_frame =
                        FrameNumber::new(current_frame.as_u64().min(buddy_frame.as_u64()));
                    current_order += 1;
                } else {
                    // No buddy found, stop merging
                    break;
                }
            }

            // Add block to free list
            let mut list = self.free_lists[current_order].lock();
            let block = BuddyBlock {
                frame: current_frame,
                next: list.take().map(Box::new),
            };
            *list = Some(block);

            self.free_frames.fetch_add(1 << order, Ordering::Release);
            Ok(())
        }
    }

    fn free_count(&self) -> usize {
        self.free_frames.load(Ordering::Acquire)
    }
}

/// NUMA-aware hybrid frame allocator
pub struct FrameAllocator {
    /// Bitmap allocators for each NUMA node
    bitmap_allocators: [Option<BitmapAllocator>; MAX_NUMA_NODES],
    /// Buddy allocators for each NUMA node
    buddy_allocators: [Option<BuddyAllocator>; MAX_NUMA_NODES],
    /// Statistics
    stats: Mutex<FrameAllocatorStats>,
    /// Allocation counter
    allocation_count: AtomicU64,
    /// Reserved memory regions
    #[cfg(feature = "alloc")]
    reserved_regions: Mutex<Vec<ReservedRegion>>,
}

impl FrameAllocator {
    /// Create a new frame allocator
    pub const fn new() -> Self {
        const NONE_BITMAP: Option<BitmapAllocator> = None;
        const NONE_BUDDY: Option<BuddyAllocator> = None;

        Self {
            bitmap_allocators: [NONE_BITMAP; MAX_NUMA_NODES],
            buddy_allocators: [NONE_BUDDY; MAX_NUMA_NODES],
            stats: Mutex::new(FrameAllocatorStats {
                total_frames: 0,
                free_frames: 0,
                bitmap_allocations: 0,
                buddy_allocations: 0,
                allocation_time_ns: 0,
            }),
            allocation_count: AtomicU64::new(0),
            #[cfg(feature = "alloc")]
            reserved_regions: Mutex::new(Vec::new()),
        }
    }

    /// Add a reserved memory region
    #[cfg(feature = "alloc")]
    pub fn add_reserved_region(&self, region: ReservedRegion) -> Result<()> {
        let mut reserved = self.reserved_regions.lock();

        // Check for overlaps with existing reserved regions
        for existing in reserved.iter() {
            if region.start < existing.end && region.end > existing.start {
                return Err(FrameAllocatorError::ReservedMemoryConflict);
            }
        }

        reserved.push(region);
        Ok(())
    }

    /// Check if a frame range is reserved
    #[cfg(feature = "alloc")]
    pub fn is_reserved(&self, start: FrameNumber, count: usize) -> bool {
        let end = FrameNumber::new(start.as_u64() + count as u64);
        let reserved = self.reserved_regions.lock();

        for region in reserved.iter() {
            if start < region.end && end > region.start {
                return true;
            }
        }

        false
    }

    /// Mark standard reserved regions (e.g., BIOS, kernel, boot data)
    #[cfg(feature = "alloc")]
    pub fn mark_standard_reserved_regions(&self) {
        // Reserve first 1MB for BIOS and legacy devices
        let _ = self.add_reserved_region(ReservedRegion {
            start: FrameNumber::new(0),
            end: FrameNumber::new(256), // 1MB / 4KB
            description: "BIOS and legacy devices",
        });

        // Note: Kernel and boot data regions should be marked by the bootloader
    }

    /// Initialize a NUMA node with memory range
    pub fn init_numa_node(
        &mut self,
        node: usize,
        start_frame: FrameNumber,
        frame_count: usize,
    ) -> Result<()> {
        #[cfg(not(target_arch = "aarch64"))]
        println!(
            "[FA] init_numa_node: node={}, start_frame={}, frame_count={}",
            node,
            start_frame.as_u64(),
            frame_count
        );

        if node >= MAX_NUMA_NODES {
            return Err(FrameAllocatorError::InvalidNumaNode);
        }

        // Split frames between bitmap and buddy allocators
        // Max 128K frames (512MB) for bitmap with 2048-entry bitmap array
        let bitmap_frames = frame_count.min(2048 * 64);
        let buddy_frames = frame_count.saturating_sub(bitmap_frames);

        #[cfg(not(target_arch = "aarch64"))]
        println!(
            "[FA] bitmap_frames={}, buddy_frames={}",
            bitmap_frames, buddy_frames
        );

        if bitmap_frames > 0 {
            #[cfg(not(target_arch = "aarch64"))]
            println!("[FA] Creating BitmapAllocator...");
            self.bitmap_allocators[node] = Some(BitmapAllocator::new(start_frame, bitmap_frames));
            #[cfg(not(target_arch = "aarch64"))]
            println!("[FA] BitmapAllocator created");
        }

        if buddy_frames > 0 {
            #[cfg(not(target_arch = "aarch64"))]
            println!("[FA] Creating BuddyAllocator...");
            let buddy_start = FrameNumber::new(start_frame.as_u64() + bitmap_frames as u64);
            self.buddy_allocators[node] = Some(BuddyAllocator::new(buddy_start, buddy_frames));
            #[cfg(not(target_arch = "aarch64"))]
            println!("[FA] BuddyAllocator created");
        }

        #[cfg(not(target_arch = "aarch64"))]
        println!("[FA] Skipping stats update during init to avoid deadlock");

        Ok(())
    }

    /// Allocate frames from a specific NUMA node
    pub fn allocate_frames(&self, count: usize, numa_node: Option<usize>) -> Result<FrameNumber> {
        self.allocate_frames_in_zone(count, numa_node, None)
    }

    /// Allocate frames from a specific NUMA node and memory zone
    pub fn allocate_frames_in_zone(
        &self,
        count: usize,
        numa_node: Option<usize>,
        zone: Option<MemoryZone>,
    ) -> Result<FrameNumber> {
        let start_time = crate::bench::read_timestamp();

        let result = if count < BITMAP_BUDDY_THRESHOLD {
            // Try bitmap allocator first for small allocations
            match self.allocate_bitmap_with_zone(count, numa_node, zone) {
                Ok(frame) => Ok(frame),
                Err(_) => {
                    // Bitmap exhausted: fall back to buddy allocator
                    self.allocate_buddy_with_zone(count, numa_node, zone)
                }
            }
        } else {
            // Use buddy allocator for large allocations
            self.allocate_buddy_with_zone(count, numa_node, zone)
        };

        let elapsed = crate::bench::read_timestamp() - start_time;
        {
            let mut stats = self.stats.lock();
            stats.allocation_time_ns += crate::bench::cycles_to_ns(elapsed);
        }
        self.allocation_count.fetch_add(1, Ordering::Relaxed);

        result
    }

    /// Allocate using bitmap allocator with zone constraint
    fn allocate_bitmap_with_zone(
        &self,
        count: usize,
        numa_node: Option<usize>,
        zone: Option<MemoryZone>,
    ) -> Result<FrameNumber> {
        // Try with zone constraint first
        if let Ok(frame) = self.allocate_bitmap_internal(count, numa_node, zone) {
            return Ok(frame);
        }

        // If zone was specified but allocation failed, try zone fallback
        if zone.is_some() {
            // For DMA zone, don't fallback
            if zone == Some(MemoryZone::Dma) {
                return Err(FrameAllocatorError::OutOfMemory);
            }
            // For other zones, try without zone constraint
            self.allocate_bitmap_internal(count, numa_node, None)
        } else {
            Err(FrameAllocatorError::OutOfMemory)
        }
    }

    /// Allocate using bitmap allocator
    fn allocate_bitmap(&self, count: usize, numa_node: Option<usize>) -> Result<FrameNumber> {
        self.allocate_bitmap_internal(count, numa_node, None)
    }

    /// Internal bitmap allocation with optional zone checking
    fn allocate_bitmap_internal(
        &self,
        count: usize,
        numa_node: Option<usize>,
        zone: Option<MemoryZone>,
    ) -> Result<FrameNumber> {
        if let Some(node) = numa_node {
            // Try specified node first
            if node < MAX_NUMA_NODES {
                if let Some(ref allocator) = self.bitmap_allocators[node] {
                    if let Ok(frame) = allocator.allocate(count) {
                        // Check zone constraint
                        if let Some(z) = zone {
                            if !z.contains(frame) {
                                let _ = allocator.free(frame, count);
                                return Err(FrameAllocatorError::OutOfMemory);
                            }
                        }

                        // Check if allocated frames are reserved
                        #[cfg(feature = "alloc")]
                        if self.is_reserved(frame, count) {
                            // Try to free and continue searching
                            let _ = allocator.free(frame, count);
                        } else {
                            return Ok(frame);
                        }
                        #[cfg(not(feature = "alloc"))]
                        return Ok(frame);
                    }
                }
            }
        }

        // Try all nodes
        for allocator in self.bitmap_allocators.iter().flatten() {
            if let Ok(frame) = allocator.allocate(count) {
                // Check if allocated frames are reserved
                #[cfg(feature = "alloc")]
                if self.is_reserved(frame, count) {
                    // Try to free and continue searching
                    let _ = allocator.free(frame, count);
                    continue;
                }
                return Ok(frame);
            }
        }

        Err(FrameAllocatorError::OutOfMemory)
    }

    /// Allocate using buddy allocator with zone constraint
    fn allocate_buddy_with_zone(
        &self,
        count: usize,
        numa_node: Option<usize>,
        zone: Option<MemoryZone>,
    ) -> Result<FrameNumber> {
        // Try with zone constraint first
        if let Ok(frame) = self.allocate_buddy_internal(count, numa_node, zone) {
            return Ok(frame);
        }

        // If zone was specified but allocation failed, try zone fallback
        if zone.is_some() {
            // For DMA zone, don't fallback
            if zone == Some(MemoryZone::Dma) {
                return Err(FrameAllocatorError::OutOfMemory);
            }
            // For other zones, try without zone constraint
            self.allocate_buddy_internal(count, numa_node, None)
        } else {
            Err(FrameAllocatorError::OutOfMemory)
        }
    }

    /// Allocate using buddy allocator
    fn allocate_buddy(&self, count: usize, numa_node: Option<usize>) -> Result<FrameNumber> {
        self.allocate_buddy_internal(count, numa_node, None)
    }

    /// Internal buddy allocation with optional zone checking
    fn allocate_buddy_internal(
        &self,
        count: usize,
        numa_node: Option<usize>,
        zone: Option<MemoryZone>,
    ) -> Result<FrameNumber> {
        if let Some(node) = numa_node {
            // Try specified node first
            if node < MAX_NUMA_NODES {
                if let Some(ref allocator) = self.buddy_allocators[node] {
                    if let Ok(frame) = allocator.allocate(count) {
                        // Check zone constraint
                        if let Some(z) = zone {
                            if !z.contains(frame) {
                                let _ = allocator.free(frame, count);
                                return Err(FrameAllocatorError::OutOfMemory);
                            }
                        }

                        // Check if allocated frames are reserved
                        #[cfg(feature = "alloc")]
                        if self.is_reserved(frame, count) {
                            // Try to free and continue searching
                            let _ = allocator.free(frame, count);
                        } else {
                            return Ok(frame);
                        }
                        #[cfg(not(feature = "alloc"))]
                        return Ok(frame);
                    }
                }
            }
        }

        // Try all nodes
        for allocator in self.buddy_allocators.iter().flatten() {
            if let Ok(frame) = allocator.allocate(count) {
                // Check if allocated frames are reserved
                #[cfg(feature = "alloc")]
                if self.is_reserved(frame, count) {
                    // Try to free and continue searching
                    let _ = allocator.free(frame, count);
                    continue;
                }
                return Ok(frame);
            }
        }

        Err(FrameAllocatorError::OutOfMemory)
    }

    /// Mark a specific physical frame as used (reserved) so it won't be
    /// allocated. Used to protect boot page table frames from being
    /// overwritten by the frame allocator.
    pub fn mark_frame_used(&self, frame: FrameNumber) -> Result<()> {
        for allocator in self.bitmap_allocators.iter().flatten() {
            allocator.mark_used(frame)?;
        }
        Ok(())
    }

    /// Free frames back to the allocator
    pub fn free_frames(&self, frame: FrameNumber, count: usize) -> Result<()> {
        // Try bitmap allocators first (they manage the lower portion of RAM)
        for allocator in self.bitmap_allocators.iter().flatten() {
            if allocator.free(frame, count).is_ok() {
                return Ok(());
            }
        }

        // Then try buddy allocators (they manage the upper portion)
        for allocator in self.buddy_allocators.iter().flatten() {
            if allocator.free(frame, count).is_ok() {
                return Ok(());
            }
        }

        Err(FrameAllocatorError::InvalidFrame)
    }

    /// Get allocator statistics
    pub fn get_stats(&self) -> FrameAllocatorStats {
        let mut free_frames = 0;

        for allocator in self.bitmap_allocators.iter().flatten() {
            free_frames += allocator.free_count() as u64;
        }

        for allocator in self.buddy_allocators.iter().flatten() {
            free_frames += allocator.free_count() as u64;
        }

        let stats = self.stats.lock();
        FrameAllocatorStats {
            total_frames: stats.total_frames,
            free_frames,
            bitmap_allocations: stats.bitmap_allocations,
            buddy_allocations: stats.buddy_allocations,
            allocation_time_ns: stats.allocation_time_ns,
        }
    }

    /// Allocate a single frame with RAII guard
    pub fn allocate_frame_raii(&'static self) -> Result<FrameGuard> {
        let frame_num = self.allocate_frames(1, None)?;
        let frame = PhysicalFrame::new(frame_num);
        Ok(FrameGuard::new(frame, self))
    }

    /// Allocate multiple frames with RAII guard
    pub fn allocate_frames_raii(&'static self, count: usize) -> Result<FramesGuard> {
        let start_frame = self.allocate_frames(count, None)?;
        let mut frames = Vec::with_capacity(count);
        for i in 0..count {
            frames.push(PhysicalFrame::new(FrameNumber(start_frame.0 + i as u64)));
        }
        Ok(FramesGuard::new(frames, self))
    }

    /// Allocate frame from specific NUMA node with RAII guard
    pub fn allocate_frame_raii_numa(&'static self, numa_node: usize) -> Result<FrameGuard> {
        let frame_num = self.allocate_frames(1, Some(numa_node))?;
        let frame = PhysicalFrame::new(frame_num);
        Ok(FrameGuard::new(frame, self))
    }

    /// Free a frame (used by RAII guards)
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - The frame was previously allocated by this allocator
    /// - The frame is not currently in use
    /// - The frame will not be used after this call
    pub unsafe fn free_frame(&self, frame: PhysicalFrame) {
        if let Err(_e) = self.free_frames(frame.number(), 1) {
            #[cfg(not(target_arch = "aarch64"))]
            println!(
                "[FrameAllocator] Warning: Failed to free frame {}: {:?}",
                frame.number().0,
                _e
            );
        }
    }

    /// Deallocate a single frame (wrapper for free_frames)
    pub fn deallocate_frame(&self, frame: PhysicalAddress) {
        let frame_num = FrameNumber::new(frame.as_u64() / FRAME_SIZE as u64);
        if let Err(_e) = self.free_frames(frame_num, 1) {
            #[cfg(not(target_arch = "aarch64"))]
            println!(
                "[FrameAllocator] Warning: Failed to deallocate frame at {:#x}: {:?}",
                frame.as_u64(),
                _e
            );
        }
    }
}

impl Default for FrameAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Global frame allocator instance
pub static FRAME_ALLOCATOR: Mutex<FrameAllocator> = Mutex::new(FrameAllocator::new());

// ============================================================================
// Per-CPU Page Cache
// ============================================================================

/// Per-CPU page frame cache to reduce global FRAME_ALLOCATOR contention.
///
/// Single-frame allocations (page faults, mmap, fork) dominate. By caching
/// frames per-CPU, we avoid acquiring the global lock on every allocation.
///
/// When the cache is empty, it batch-refills from the global allocator.
/// When full, it batch-drains back to the global allocator.
pub struct PerCpuPageCache {
    /// Cached frame numbers
    frames: [u64; Self::CAPACITY],
    /// Number of valid entries in `frames`
    count: usize,
}

impl Default for PerCpuPageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PerCpuPageCache {
    /// Maximum frames cached per CPU
    const CAPACITY: usize = 64;
    /// Refill from global when cache drops below this
    const LOW_WATERMARK: usize = 16;
    /// Drain to global when cache exceeds this
    const HIGH_WATERMARK: usize = 48;
    /// Number of frames to transfer in a batch
    const BATCH_SIZE: usize = 32;

    pub const fn new() -> Self {
        Self {
            frames: [0; Self::CAPACITY],
            count: 0,
        }
    }

    /// Try to allocate a single frame from the per-CPU cache.
    /// Returns None if cache is empty (caller should refill from global).
    #[inline]
    pub fn alloc_one(&mut self) -> Option<FrameNumber> {
        if self.count == 0 {
            return None;
        }
        self.count -= 1;
        Some(FrameNumber::new(self.frames[self.count]))
    }

    /// Return a single frame to the per-CPU cache.
    /// Returns false if cache is full (caller should drain to global).
    #[inline]
    pub fn free_one(&mut self, frame: FrameNumber) -> bool {
        if self.count >= Self::CAPACITY {
            return false;
        }
        self.frames[self.count] = frame.as_u64();
        self.count += 1;
        true
    }

    /// Is the cache below the low watermark?
    #[inline]
    pub fn needs_refill(&self) -> bool {
        self.count < Self::LOW_WATERMARK
    }

    /// Is the cache above the high watermark?
    #[inline]
    pub fn needs_drain(&self) -> bool {
        self.count > Self::HIGH_WATERMARK
    }

    /// Batch-refill from the global frame allocator.
    /// Acquires the global lock once, filling up to BATCH_SIZE frames.
    pub fn batch_refill(&mut self) {
        let global = FRAME_ALLOCATOR.lock();
        let to_refill = Self::BATCH_SIZE.min(Self::CAPACITY - self.count);
        for _ in 0..to_refill {
            match global.allocate_frames(1, None) {
                Ok(frame) => {
                    self.frames[self.count] = frame.as_u64();
                    self.count += 1;
                }
                Err(_) => break,
            }
        }
    }

    /// Batch-drain excess frames back to the global allocator.
    /// Acquires the global lock once, returning BATCH_SIZE frames.
    pub fn batch_drain(&mut self) {
        let global = FRAME_ALLOCATOR.lock();
        let to_drain = Self::BATCH_SIZE.min(self.count);
        for _ in 0..to_drain {
            if self.count == 0 {
                break;
            }
            self.count -= 1;
            let frame = FrameNumber::new(self.frames[self.count]);
            let _ = global.free_frames(frame, 1);
        }
    }

    /// Number of cached frames
    pub fn cached_count(&self) -> usize {
        self.count
    }
}

/// Per-CPU page caches (one per CPU, protected by per-CPU access pattern)
///
/// SAFETY: Each CPU accesses only its own index via `current_cpu_id()`.
/// During bootstrap, only CPU 0 runs. After SMP bringup, each CPU
/// initializes its own cache. No cross-CPU access occurs.
static PER_CPU_PAGE_CACHES: Mutex<[PerCpuPageCache; 16]> =
    Mutex::new([const { PerCpuPageCache::new() }; 16]);

/// Allocate a single physical frame using the per-CPU cache.
///
/// Fast path: no global lock contention for single-frame allocs.
/// Falls back to global allocator if cache is empty and refill fails.
pub fn per_cpu_alloc_frame() -> Result<FrameNumber> {
    let cpu_id = crate::sched::smp::current_cpu_id() as usize;

    let mut caches = PER_CPU_PAGE_CACHES.lock();
    let cache = &mut caches[cpu_id.min(15)];

    // Try cache first
    if let Some(frame) = cache.alloc_one() {
        return Ok(frame);
    }

    // Cache empty -- batch refill from global
    cache.batch_refill();

    // Try again after refill
    if let Some(frame) = cache.alloc_one() {
        return Ok(frame);
    }

    // Still empty -- fall back to direct global allocation
    FRAME_ALLOCATOR.lock().allocate_frames(1, None)
}

/// Free a single physical frame using the per-CPU cache.
///
/// Fast path: no global lock contention for single-frame frees.
/// Drains excess frames back to global if cache is full.
pub fn per_cpu_free_frame(frame: FrameNumber) -> Result<()> {
    let cpu_id = crate::sched::smp::current_cpu_id() as usize;

    let mut caches = PER_CPU_PAGE_CACHES.lock();
    let cache = &mut caches[cpu_id.min(15)];

    // Try cache first
    if cache.free_one(frame) {
        // Drain excess if above high watermark
        if cache.needs_drain() {
            cache.batch_drain();
        }
        return Ok(());
    }

    // Cache full -- drain first, then retry
    cache.batch_drain();
    if cache.free_one(frame) {
        return Ok(());
    }

    // Still full (shouldn't happen after drain) -- go direct
    FRAME_ALLOCATOR.lock().free_frames(frame, 1)
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_bitmap_allocator() {
        let allocator = BitmapAllocator::new(FrameNumber::new(0), 1000);

        // Test single frame allocation
        let frame = allocator
            .allocate(1)
            .expect("single frame allocation from fresh allocator should succeed");
        assert_eq!(frame.as_u64(), 0);

        // Test contiguous allocation
        let frame = allocator
            .allocate(10)
            .expect("10-frame contiguous allocation should succeed with 999 free frames");
        assert_eq!(frame.as_u64(), 1);

        // Test free
        allocator
            .free(frame, 10)
            .expect("freeing previously allocated frames should succeed");

        // Should be able to allocate again
        let frame2 = allocator
            .allocate(10)
            .expect("re-allocation after free should succeed");
        assert_eq!(frame2.as_u64(), frame.as_u64());
    }

    #[test]
    fn test_buddy_allocator() {
        let allocator = BuddyAllocator::new(FrameNumber::new(0), 1024);

        // Test power-of-2 allocation
        let frame = allocator
            .allocate(512)
            .expect("512-frame allocation from 1024-frame buddy allocator should succeed");
        assert_eq!(frame.as_u64(), 0);

        // Test buddy splitting
        let frame2 = allocator
            .allocate(512)
            .expect("second 512-frame allocation should succeed after buddy split");
        assert_eq!(frame2.as_u64(), 512);

        // Test buddy merging
        allocator
            .free(frame, 512)
            .expect("freeing first buddy block should succeed");
        allocator
            .free(frame2, 512)
            .expect("freeing second buddy block should succeed and trigger merge");

        // Should be able to allocate full size again
        let frame3 = allocator
            .allocate(1024)
            .expect("full-size allocation should succeed after buddy merge");
        assert_eq!(frame3.as_u64(), 0);
    }
}

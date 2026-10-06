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

/// Report a failed free from a path that cannot return the error (a `Drop`
/// or a rollback). A free fails only when an invariant is broken (a frame
/// freed twice, or outside every allocator), so it is logged rather than
/// discarded with `let _` (agy review of the v0.26.0 stack, PR #12).
pub fn note_free_failure(result: Result<()>, frame: FrameNumber, context: &str) {
    if let Err(e) = result {
        crate::println!(
            "[MM] {}: freeing frame {:#x} failed: {:?}",
            context,
            frame.as_u64(),
            e
        );
    }
}

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
///
/// MEM-PERF-02: searches a word at a time (whole free words count 64 frames
/// at once, partial words are walked with trailing_zeros/trailing_ones)
/// starting from a roving hint, and sets/clears ranges with word masks.
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
    /// Word where the next search starts (just past the last allocation).
    hint: AtomicUsize,
}

/// Call `f(word, mask)` for each bitmap word covering bits
/// `[start, start + count)`, with the bits of that range set in `mask`.
fn for_each_word_mask(start: usize, count: usize, mut f: impl FnMut(usize, u64)) {
    let end = start + count;
    let mut bit = start;
    while bit < end {
        let word = bit / 64;
        let lo = bit % 64;
        let hi = (end - word * 64).min(64);
        let mask = if hi - lo == 64 {
            u64::MAX
        } else {
            ((1u64 << (hi - lo)) - 1) << lo
        };
        f(word, mask);
        bit = word * 64 + hi;
    }
}

/// First run of `count` set bits within words `[lo, hi)` of `bitmap`;
/// returns the bit index of its start.
fn find_run(bitmap: &[u64], lo: usize, hi: usize, count: usize) -> Option<usize> {
    let mut run = 0usize;
    let mut run_start = 0usize;
    for (i, &word) in bitmap.iter().enumerate().take(hi).skip(lo) {
        if word == 0 {
            run = 0;
            continue;
        }
        if word == u64::MAX {
            if run == 0 {
                run_start = i * 64;
            }
            run += 64;
            if run >= count {
                return Some(run_start);
            }
            continue;
        }
        let mut pos = 0u32;
        while pos < 64 {
            let w = word >> pos;
            if w == 0 {
                run = 0;
                break;
            }
            let zeros = w.trailing_zeros();
            if zeros > 0 {
                run = 0;
                pos += zeros;
                continue;
            }
            let ones = w.trailing_ones();
            if run == 0 {
                run_start = i * 64 + pos as usize;
            }
            run += ones as usize;
            if run >= count {
                return Some(run_start);
            }
            pos += ones;
        }
        // A run reaching bit 63 carries on into the next word; any other
        // run was reset by the zero bit after it.
    }
    None
}

impl BitmapAllocator {
    /// Frames one bitmap can track.
    const CAPACITY: usize = 2048 * 64;

    /// Track `frame_count` frames starting at `start_frame`. Only those
    /// frames start out free: bits past the end of the node stay 0 so they
    /// are never handed out (N-01 -- they would lie beyond physical RAM).
    const fn new(start_frame: FrameNumber, frame_count: usize) -> Self {
        let frame_count = if frame_count > Self::CAPACITY {
            Self::CAPACITY
        } else {
            frame_count
        };
        let mut bitmap = [0u64; 2048];
        let full_words = frame_count / 64;
        let mut i = 0;
        while i < full_words {
            bitmap[i] = u64::MAX;
            i += 1;
        }
        let tail_bits = frame_count % 64;
        if tail_bits != 0 {
            bitmap[full_words] = (1u64 << tail_bits) - 1;
        }
        Self {
            bitmap: Mutex::new(bitmap),
            start_frame,
            total_frames: frame_count,
            free_frames: AtomicUsize::new(frame_count),
            hint: AtomicUsize::new(0),
        }
    }

    /// Number of bitmap words in use.
    fn words(&self) -> usize {
        self.total_frames.div_ceil(64)
    }

    /// Allocate contiguous frames
    fn allocate(&self, count: usize) -> Result<FrameNumber> {
        if count == 0 || count >= BITMAP_BUDDY_THRESHOLD {
            return Err(FrameAllocatorError::InvalidSize);
        }
        if self.free_frames.load(Ordering::Relaxed) < count {
            return Err(FrameAllocatorError::OutOfMemory);
        }

        let mut bitmap = self.bitmap.lock();
        let words = self.words();
        let hint = self.hint.load(Ordering::Relaxed).min(words);

        // From the hint to the end, then from the start up to the hint (a
        // run may straddle the hint, so the second pass overlaps it).
        let found = find_run(&bitmap[..], hint, words, count).or_else(|| {
            find_run(
                &bitmap[..],
                0,
                (hint + count.div_ceil(64) + 1).min(words),
                count,
            )
        });
        let Some(first) = found else {
            return Err(FrameAllocatorError::OutOfMemory);
        };

        for_each_word_mask(first, count, |w, mask| bitmap[w] &= !mask);
        self.free_frames.fetch_sub(count, Ordering::Release);
        self.hint.store((first + count) / 64, Ordering::Relaxed);
        Ok(FrameNumber::new(self.start_frame.as_u64() + first as u64))
    }

    /// Allocate up to `out.len()` single frames (not necessarily
    /// contiguous) under one lock acquisition; returns how many.
    fn allocate_singles(&self, out: &mut [u64]) -> usize {
        if out.is_empty() {
            return 0;
        }
        let mut bitmap = self.bitmap.lock();
        let words = self.words();
        let hint = self.hint.load(Ordering::Relaxed).min(words);
        let mut n = 0;
        for w in (hint..words).chain(0..hint) {
            while bitmap[w] != 0 && n < out.len() {
                let bit = bitmap[w].trailing_zeros() as usize;
                bitmap[w] &= !(1u64 << bit);
                out[n] = self.start_frame.as_u64() + (w * 64 + bit) as u64;
                n += 1;
            }
            if n == out.len() {
                self.hint.store(w, Ordering::Relaxed);
                break;
            }
        }
        self.free_frames.fetch_sub(n, Ordering::Release);
        n
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
        // The range must lie entirely inside this node (a frame below the
        // start used to underflow the offset).
        let offset = frame
            .as_u64()
            .checked_sub(self.start_frame.as_u64())
            .ok_or(FrameAllocatorError::InvalidFrame)? as usize;
        let end = offset
            .checked_add(count)
            .ok_or(FrameAllocatorError::InvalidFrame)?;
        if end > self.total_frames || count == 0 {
            return Err(FrameAllocatorError::InvalidFrame);
        }

        let mut bitmap = self.bitmap.lock();

        // Double-free detection: verify every frame is allocated before
        // changing any bit, so a rejected free leaves the bitmap untouched.
        let mut already_free = false;
        for_each_word_mask(offset, count, |w, mask| {
            already_free |= bitmap[w] & mask != 0
        });
        if already_free {
            return Err(FrameAllocatorError::InvalidFrame);
        }
        for_each_word_mask(offset, count, |w, mask| bitmap[w] |= mask);

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
    /// Total time spent allocating (ns). Atomic: allocation must not take
    /// a lock just to update statistics (MEM-PERF-02).
    allocation_time_ns: AtomicU64,
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
            allocation_time_ns: AtomicU64::new(0),
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

        // Take the region's frames out of the bitmaps now, so bitmap
        // allocations never have to consult the reserved list (MEM-PERF-02).
        // The buddy path still checks it.
        for frame in region.start.as_u64()..region.end.as_u64() {
            for allocator in self.bitmap_allocators.iter().flatten() {
                let _ = allocator.mark_used(FrameNumber::new(frame));
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
        self.allocation_time_ns
            .fetch_add(crate::bench::cycles_to_ns(elapsed), Ordering::Relaxed);
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
                        // Reserved frames were removed from the bitmap when
                        // the region was added.
                        return Ok(frame);
                    }
                }
            }
        }

        // Try all nodes
        for allocator in self.bitmap_allocators.iter().flatten() {
            if let Ok(frame) = allocator.allocate(count) {
                return Ok(frame);
            }
        }

        Err(FrameAllocatorError::OutOfMemory)
    }

    /// Allocate up to `out.len()` single frames (not necessarily contiguous)
    /// with one pass over the bitmaps; returns how many were allocated.
    /// Used to refill per-CPU caches.
    pub fn allocate_single_frames(&self, out: &mut [u64]) -> usize {
        let mut n = 0;
        for allocator in self.bitmap_allocators.iter().flatten() {
            n += allocator.allocate_singles(&mut out[n..]);
            if n == out.len() {
                return n;
            }
        }
        // Bitmaps exhausted: fall back to the general path.
        while n < out.len() {
            match self.allocate_frames(1, None) {
                Ok(f) => {
                    out[n] = f.as_u64();
                    n += 1;
                }
                Err(_) => break,
            }
        }
        n
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
        let mut free_frames = 0u64;
        let mut total_frames = 0u64;

        for allocator in self.bitmap_allocators.iter().flatten() {
            free_frames += allocator.free_count() as u64;
            total_frames += allocator.total_frames as u64;
        }

        for allocator in self.buddy_allocators.iter().flatten() {
            free_frames += allocator.free_count() as u64;
            total_frames += allocator.total_frames as u64;
        }

        FrameAllocatorStats {
            total_frames,
            free_frames,
            bitmap_allocations: 0,
            buddy_allocations: 0,
            allocation_time_ns: self.allocation_time_ns.load(Ordering::Relaxed),
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
pub(crate) static FRAME_ALLOCATOR: Mutex<FrameAllocator> = Mutex::new(FrameAllocator::new());

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
/// Cache-line aligned to prevent false sharing when per-CPU caches are
/// stored in adjacent array slots accessed by different cores.
#[repr(align(64))]
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

    /// Number of cached frames
    pub fn cached_count(&self) -> usize {
        self.count
    }
}

/// Per-CPU page caches: one lock per CPU, each on its own cache line, so
/// CPUs never contend on (or false-share) another CPU's cache (MEM-PERF-01).
/// The previous version put all CPUs' caches behind one global mutex.
static PER_CPU_PAGE_CACHES: [crate::mm::cache_aligned::CacheAligned<Mutex<PerCpuPageCache>>;
    crate::sched::smp::MAX_CPUS] =
    [const { crate::mm::cache_aligned::CacheAligned::new(Mutex::new(PerCpuPageCache::new())) };
        crate::sched::smp::MAX_CPUS];

/// This CPU's page cache.
fn this_cpu_cache() -> &'static Mutex<PerCpuPageCache> {
    let cpu = crate::sched::smp::current_cpu_id() as usize;
    &PER_CPU_PAGE_CACHES[cpu.min(crate::sched::smp::MAX_CPUS - 1)]
}

/// Allocate a single physical frame using the per-CPU cache.
///
/// Fast path: only this CPU's cache lock. On a miss, a batch of frames is
/// taken from the global allocator in one bitmap pass into a stack buffer
/// -- without holding the cache lock -- and the rest of the batch refills
/// the cache.
pub fn per_cpu_alloc_frame() -> Result<FrameNumber> {
    let cache = this_cpu_cache();
    if let Some(frame) = cache.lock().alloc_one() {
        return Ok(frame);
    }

    let mut batch = [0u64; PerCpuPageCache::BATCH_SIZE];
    let n = FRAME_ALLOCATOR.lock().allocate_single_frames(&mut batch);
    if n == 0 {
        return Err(FrameAllocatorError::OutOfMemory);
    }
    let mut leftover = 0;
    {
        let mut c = cache.lock();
        for i in 1..n {
            let f = batch[i];
            if !c.free_one(FrameNumber::new(f)) {
                batch[1 + leftover] = f;
                leftover += 1;
            }
        }
    }
    if leftover > 0 {
        // The cache filled up meanwhile: return what did not fit.
        let global = FRAME_ALLOCATOR.lock();
        for &f in &batch[1..1 + leftover] {
            let _ = global.free_frames(FrameNumber::new(f), 1);
        }
    }
    let frame = FrameNumber::new(batch[0]);
    crate::trace!(
        crate::perf::trace::TraceEventType::FrameAlloc,
        frame.as_u64(),
        0u64
    );
    Ok(frame)
}

/// Free a single physical frame using the per-CPU cache.
///
/// Fast path: only this CPU's cache lock. Above the high watermark a batch
/// is moved to a stack buffer under the cache lock and returned to the
/// global allocator after it is released.
pub fn per_cpu_free_frame(frame: FrameNumber) -> Result<()> {
    crate::trace!(
        crate::perf::trace::TraceEventType::FrameFree,
        frame.as_u64(),
        0u64
    );
    let cache = this_cpu_cache();
    let mut batch = [0u64; PerCpuPageCache::BATCH_SIZE];
    let mut n = 0;
    let direct = {
        let mut c = cache.lock();
        let cached = c.free_one(frame);
        if !cached || c.needs_drain() {
            while n < batch.len() {
                match c.alloc_one() {
                    Some(f) => {
                        batch[n] = f.as_u64();
                        n += 1;
                    }
                    None => break,
                }
            }
        }
        !cached
    };
    if n > 0 || direct {
        let global = FRAME_ALLOCATOR.lock();
        for &f in &batch[..n] {
            let _ = global.free_frames(FrameNumber::new(f), 1);
        }
        if direct {
            return global.free_frames(frame, 1);
        }
    }
    Ok(())
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    /// Naive reference: first run of `count` set bits in bits [0, nbits).
    fn naive_run(bits: &[bool], count: usize) -> Option<usize> {
        let mut run = 0;
        for (i, &b) in bits.iter().enumerate() {
            run = if b { run + 1 } else { 0 };
            if run == count {
                return Some(i + 1 - count);
            }
        }
        None
    }

    #[test]
    fn find_run_matches_naive_scan() {
        // xorshift: deterministic patterns, including runs across words.
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..300 {
            let mut words = [0u64; 6];
            for w in words.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Bias towards long runs of ones and zeros.
                *w = match x % 4 {
                    0 => u64::MAX,
                    1 => 0,
                    _ => x,
                };
            }
            let bits: alloc::vec::Vec<bool> = (0..384)
                .map(|i| words[i / 64] >> (i % 64) & 1 == 1)
                .collect();
            for count in [1, 2, 3, 7, 31, 63, 64, 65, 100, 129, 200] {
                assert_eq!(
                    find_run(&words, 0, 6, count),
                    naive_run(&bits, count),
                    "words={:x?} count={}",
                    words,
                    count
                );
            }
        }
    }

    #[test]
    fn bitmap_alloc_free_masks_and_hint_wrap() {
        let a = BitmapAllocator::new(FrameNumber::new(1000), 300);
        // Contiguous run crossing a word boundary.
        let f = a.allocate(100).unwrap();
        assert_eq!(f.as_u64(), 1000);
        let g = a.allocate(70).unwrap();
        assert_eq!(g.as_u64(), 1100);
        assert_eq!(a.free_count(), 130);
        // Free the first run; the hint is past it, so the search wraps.
        a.free(f, 100).unwrap();
        assert!(a.free(f, 1).is_err(), "double free rejected");
        let h = a.allocate(130).unwrap();
        assert_eq!(h.as_u64(), 1170, "first fit from the hint");
        let k = a.allocate(90).unwrap();
        assert_eq!(k.as_u64(), 1000, "wrapped to the start");
        assert_eq!(a.free_count(), 10);
        assert!(a.allocate(11).is_err());
        // Singles: the remaining 10 frames, none twice.
        let mut out = [0u64; 16];
        assert_eq!(a.allocate_singles(&mut out), 10);
        let mut v = out[..10].to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), 10);
        assert!(v.iter().all(|&f| (1090..1100).contains(&f)));
        assert_eq!(a.free_count(), 0);
        assert_eq!(a.allocate_singles(&mut out), 0);
    }

    /// N-01: a node smaller than the bitmap must never hand out frames past
    /// its own end (they would be beyond physical RAM).
    #[test]
    fn test_bitmap_allocator_respects_node_size() {
        let start = 0x81400;
        for count in [1usize, 63, 64, 65, 100, 27_648] {
            let allocator = BitmapAllocator::new(FrameNumber::new(start), count);
            for i in 0..count {
                let frame = allocator.allocate(1).expect("frame within the node");
                assert_eq!(frame.as_u64(), start + i as u64);
            }
            assert!(
                allocator.allocate(1).is_err(),
                "node of {} frames overran",
                count
            );
            assert_eq!(allocator.free_count(), 0);
        }
    }

    /// N-01: frees outside the node are rejected, including below its start
    /// (which used to underflow the offset).
    #[test]
    fn test_bitmap_free_rejects_out_of_range() {
        let allocator = BitmapAllocator::new(FrameNumber::new(1000), 100);
        let frame = allocator.allocate(1).unwrap();
        assert!(allocator.free(FrameNumber::new(999), 1).is_err());
        assert!(allocator.free(FrameNumber::new(1100), 1).is_err());
        assert!(allocator.free(frame, 101).is_err());
        assert!(allocator.free(frame, 1).is_ok());
    }

    /// A double free detected part-way through must change nothing.
    #[test]
    fn test_bitmap_double_free_is_atomic() {
        let allocator = BitmapAllocator::new(FrameNumber::new(0), 100);
        let frame = allocator.allocate(2).unwrap();
        allocator
            .free(FrameNumber::new(frame.as_u64() + 1), 1)
            .unwrap();
        let before = allocator.free_count();
        assert!(allocator.free(frame, 2).is_err());
        assert_eq!(allocator.free_count(), before);
        // The first frame is still allocated, so the next single-frame
        // allocation returns the second (freed) frame, not the first.
        assert_eq!(allocator.allocate(1).unwrap().as_u64(), frame.as_u64() + 1);
    }

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

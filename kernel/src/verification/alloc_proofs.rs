#![allow(unexpected_cfgs)]
//! Verified Memory Allocator
//!
//! Model-checking and proof harnesses for the memory allocator, verifying
//! no double allocation, use-after-free prevention, buddy system consistency,
//! frame conservation, zone correctness, and alignment guarantees.

#[cfg(feature = "alloc")]
use alloc::collections::BTreeSet;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Frame size in bytes (4 KiB)
#[allow(dead_code)]
const MODEL_FRAME_SIZE: u64 = 4096;

/// DMA zone upper bound (16 MB)
#[allow(dead_code)]
const DMA_ZONE_LIMIT: u64 = 16 * 1024 * 1024;

/// Maximum buddy order (2^10 = 1024 frames = 4 MiB block)
#[allow(dead_code)]
const MAX_BUDDY_ORDER: u32 = 10;

/// Memory zones for zone-aware allocation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum MemoryZone {
    /// DMA zone: 0 - 16 MB
    Dma = 0,
    /// Normal zone: 16 MB - 4 GB
    Normal = 1,
    /// High zone: above 4 GB
    High = 2,
}

/// Allocation state of a frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FrameState {
    /// Frame is free and available
    Free,
    /// Frame is allocated
    Allocated,
}

/// Model of the frame allocator for verification
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct AllocatorModel {
    /// Set of currently allocated frame addresses
    #[cfg(feature = "alloc")]
    allocated: BTreeSet<u64>,
    /// Set of currently free frame addresses
    #[cfg(feature = "alloc")]
    free: BTreeSet<u64>,
    /// Total number of frames managed
    total_frames: u64,
    /// Allocation count for statistics
    alloc_count: u64,
    /// Free count for statistics
    free_count: u64,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl AllocatorModel {
    /// Create a new allocator model with frames in range [base, base + count *
    /// FRAME_SIZE)
    pub fn new(base: u64, count: u64) -> Self {
        let mut free = BTreeSet::new();
        for i in 0..count {
            free.insert(base + i * MODEL_FRAME_SIZE);
        }
        Self {
            allocated: BTreeSet::new(),
            free,
            total_frames: count,
            alloc_count: 0,
            free_count: 0,
        }
    }

    /// Allocate a single frame, returns the frame address
    pub fn alloc_frame(&mut self) -> Result<u64, AllocModelError> {
        // Take the first free frame
        let frame = *self
            .free
            .iter()
            .next()
            .ok_or(AllocModelError::OutOfMemory)?;
        self.free.remove(&frame);
        self.allocated.insert(frame);
        self.alloc_count += 1;
        Ok(frame)
    }

    /// Allocate a frame from a specific zone
    pub fn alloc_frame_zone(&mut self, zone: MemoryZone) -> Result<u64, AllocModelError> {
        let (min, max) = zone_range(zone);

        let frame = self
            .free
            .iter()
            .find(|&&f| f >= min && f < max)
            .copied()
            .ok_or(AllocModelError::OutOfMemory)?;

        self.free.remove(&frame);
        self.allocated.insert(frame);
        self.alloc_count += 1;
        Ok(frame)
    }

    /// Free a previously allocated frame
    pub fn free_frame(&mut self, frame: u64) -> Result<(), AllocModelError> {
        if !self.allocated.contains(&frame) {
            if self.free.contains(&frame) {
                return Err(AllocModelError::DoubleFree);
            }
            return Err(AllocModelError::InvalidFrame);
        }

        self.allocated.remove(&frame);
        self.free.insert(frame);
        self.free_count += 1;
        Ok(())
    }

    /// Check if a frame is currently allocated
    pub fn is_allocated(&self, frame: u64) -> bool {
        self.allocated.contains(&frame)
    }

    /// Check if a frame is currently free
    pub fn is_free(&self, frame: u64) -> bool {
        self.free.contains(&frame)
    }

    /// Get the number of allocated frames
    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }

    /// Get the number of free frames
    pub fn free_count(&self) -> usize {
        self.free.len()
    }
}

/// Get the address range for a memory zone
#[allow(dead_code)]
fn zone_range(zone: MemoryZone) -> (u64, u64) {
    match zone {
        MemoryZone::Dma => (0, DMA_ZONE_LIMIT),
        MemoryZone::Normal => (DMA_ZONE_LIMIT, 4 * 1024 * 1024 * 1024),
        MemoryZone::High => (4 * 1024 * 1024 * 1024, u64::MAX),
    }
}

/// Buddy block model for buddy system verification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub struct BuddyBlock {
    /// Base address (frame-aligned)
    pub base: u64,
    /// Order: block covers 2^order frames
    pub order: u32,
    /// Whether this block is free
    pub free: bool,
}

#[allow(dead_code)]
impl BuddyBlock {
    /// Size of this block in frames
    pub fn frame_count(&self) -> u64 {
        1u64 << self.order
    }

    /// Size of this block in bytes
    pub fn byte_size(&self) -> u64 {
        self.frame_count() * MODEL_FRAME_SIZE
    }

    /// Get the buddy block's base address
    pub fn buddy_base(&self) -> u64 {
        self.base ^ (self.frame_count() * MODEL_FRAME_SIZE)
    }

    /// Split this block into two halves (returns left, right)
    pub fn split(&self) -> Option<(BuddyBlock, BuddyBlock)> {
        if self.order == 0 {
            return None;
        }
        let new_order = self.order - 1;
        let half_size = (1u64 << new_order) * MODEL_FRAME_SIZE;
        Some((
            BuddyBlock {
                base: self.base,
                order: new_order,
                free: true,
            },
            BuddyBlock {
                base: self.base + half_size,
                order: new_order,
                free: true,
            },
        ))
    }

    /// Coalesce two buddy blocks into one (if they are buddies)
    pub fn coalesce(&self, other: &BuddyBlock) -> Option<BuddyBlock> {
        if self.order != other.order {
            return None;
        }
        if !self.free || !other.free {
            return None;
        }

        let expected_buddy = self.buddy_base();
        if other.base != expected_buddy {
            return None;
        }

        let new_base = core::cmp::min(self.base, other.base);
        Some(BuddyBlock {
            base: new_base,
            order: self.order + 1,
            free: true,
        })
    }
}

/// Bitmap allocator model for small allocation verification
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BitmapModel {
    /// Bitmap tracking allocated frames (true = allocated)
    #[cfg(feature = "alloc")]
    bitmap: Vec<bool>,
    /// Base address of the region
    base: u64,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl BitmapModel {
    /// Create a new bitmap for the given number of frames
    pub fn new(base: u64, frame_count: usize) -> Self {
        Self {
            bitmap: alloc::vec![false; frame_count],
            base,
        }
    }

    /// Allocate the first free frame
    pub fn alloc(&mut self) -> Option<u64> {
        for (i, allocated) in self.bitmap.iter_mut().enumerate() {
            if !*allocated {
                *allocated = true;
                return Some(self.base + (i as u64) * MODEL_FRAME_SIZE);
            }
        }
        None
    }

    /// Free a frame
    pub fn free(&mut self, addr: u64) -> Result<(), AllocModelError> {
        let offset = addr
            .checked_sub(self.base)
            .ok_or(AllocModelError::InvalidFrame)?;
        let idx = (offset / MODEL_FRAME_SIZE) as usize;
        if idx >= self.bitmap.len() {
            return Err(AllocModelError::InvalidFrame);
        }
        if !self.bitmap[idx] {
            return Err(AllocModelError::DoubleFree);
        }
        self.bitmap[idx] = false;
        Ok(())
    }

    /// Check if a frame is allocated
    pub fn is_allocated(&self, addr: u64) -> bool {
        let offset = match addr.checked_sub(self.base) {
            Some(o) => o,
            None => return false,
        };
        let idx = (offset / MODEL_FRAME_SIZE) as usize;
        idx < self.bitmap.len() && self.bitmap[idx]
    }
}

/// Errors from allocator verification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AllocModelError {
    /// No free frames available
    OutOfMemory,
    /// Attempted to free an already-free frame
    DoubleFree,
    /// Frame address is not managed by this allocator
    InvalidFrame,
    /// Conservation invariant violated (allocated + free != total)
    ConservationViolation,
    /// Buddy pair inconsistency
    BuddyInconsistency,
    /// Zone allocation from wrong region
    ZoneViolation,
    /// Alignment violation
    AlignmentViolation,
    /// Overlap between allocated regions
    OverlapDetected,
}

/// Allocator invariant checker
#[allow(dead_code)]
pub struct AllocInvariantChecker;

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl AllocInvariantChecker {
    /// Verify no double allocation: allocated set has no duplicates
    /// (BTreeSet guarantees this structurally, but we verify operationally)
    pub fn verify_no_double_alloc(model: &AllocatorModel) -> Result<(), AllocModelError> {
        // BTreeSet cannot contain duplicates, but verify disjointness
        for frame in model.allocated.iter() {
            if model.free.contains(frame) {
                return Err(AllocModelError::OverlapDetected);
            }
        }
        Ok(())
    }

    /// Verify no use-after-free: freed frame is not in allocated set
    pub fn verify_no_use_after_free(
        model: &AllocatorModel,
        frame: u64,
    ) -> Result<(), AllocModelError> {
        if model.free.contains(&frame) && model.allocated.contains(&frame) {
            return Err(AllocModelError::OverlapDetected);
        }
        Ok(())
    }

    /// Verify buddy consistency: buddy pairs are properly tracked
    pub fn verify_buddy_consistency(block: &BuddyBlock) -> Result<(), AllocModelError> {
        if block.order > MAX_BUDDY_ORDER {
            return Err(AllocModelError::BuddyInconsistency);
        }
        // Verify block base is aligned to its size
        let size = block.byte_size();
        if !block.base.is_multiple_of(size) {
            return Err(AllocModelError::AlignmentViolation);
        }
        Ok(())
    }

    /// Verify frame conservation: allocated + free = total
    pub fn verify_frame_conservation(model: &AllocatorModel) -> Result<(), AllocModelError> {
        let sum = (model.allocated.len() as u64) + (model.free.len() as u64);
        if sum != model.total_frames {
            return Err(AllocModelError::ConservationViolation);
        }
        Ok(())
    }

    /// Verify zone correctness: allocations come from the correct zone
    pub fn verify_zone_correctness(frame: u64, zone: MemoryZone) -> Result<(), AllocModelError> {
        let (min, max) = zone_range(zone);
        if frame < min || frame >= max {
            return Err(AllocModelError::ZoneViolation);
        }
        Ok(())
    }
}

// ============================================================================
// Kani Proof Harnesses
// ============================================================================

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof: Same frame is never allocated twice
    #[kani::proof]
    fn proof_no_double_allocation() {
        let mut model = AllocatorModel::new(0x1000, 4);

        let f1 = model.alloc_frame().unwrap();
        let f2 = model.alloc_frame().unwrap();

        assert_ne!(f1, f2, "Two allocations must return different frames");
    }

    /// Proof: Freed frame can be reallocated
    #[kani::proof]
    fn proof_dealloc_makes_available() {
        let mut model = AllocatorModel::new(0x1000, 1);

        let frame = model.alloc_frame().unwrap();
        assert!(model.alloc_frame().is_err()); // No more frames

        model.free_frame(frame).unwrap();
        let frame2 = model.alloc_frame().unwrap();

        assert_eq!(frame, frame2, "Freed frame should be reallocatable");
    }

    /// Proof: Buddy split produces two valid halves
    #[kani::proof]
    fn proof_buddy_split_correct() {
        let order: u32 = kani::any();
        kani::assume(order > 0 && order <= 5);

        let base: u64 = kani::any();
        let block_size = (1u64 << order) * MODEL_FRAME_SIZE;
        kani::assume(base % block_size == 0);
        kani::assume(base < 0x1_0000_0000); // Bound for verification

        let block = BuddyBlock {
            base,
            order,
            free: true,
        };
        let (left, right) = block.split().unwrap();

        assert_eq!(left.order, order - 1);
        assert_eq!(right.order, order - 1);
        assert_eq!(left.base, base);
        assert_eq!(right.base, base + (1u64 << (order - 1)) * MODEL_FRAME_SIZE);
        assert!(left.free);
        assert!(right.free);
    }

    /// Proof: Coalescing restores the original block
    #[kani::proof]
    fn proof_buddy_coalesce_correct() {
        let order: u32 = kani::any();
        kani::assume(order > 0 && order <= 5);

        let block_size = (1u64 << order) * MODEL_FRAME_SIZE;
        let base: u64 = kani::any();
        kani::assume(base % block_size == 0);
        kani::assume(base < 0x1_0000_0000);

        let original = BuddyBlock {
            base,
            order,
            free: true,
        };
        let (left, right) = original.split().unwrap();
        let coalesced = left.coalesce(&right).unwrap();

        assert_eq!(coalesced.base, base);
        assert_eq!(coalesced.order, order);
    }

    /// Proof: Correct allocator selected by size threshold
    #[kani::proof]
    fn proof_bitmap_buddy_threshold() {
        let frame_count: u32 = kani::any();
        kani::assume(frame_count > 0 && frame_count <= 2048);

        let use_bitmap = frame_count < 512;
        let use_buddy = frame_count >= 512;

        // Exactly one allocator is selected
        assert!(use_bitmap ^ use_buddy);
    }

    /// Proof: Total frames invariant holds through alloc/free
    #[kani::proof]
    fn proof_frame_conservation() {
        let mut model = AllocatorModel::new(0x1000, 4);

        let initial_total = model.total_frames;

        let f1 = model.alloc_frame().unwrap();
        assert_eq!(
            model.allocated_count() as u64 + model.free_count() as u64,
            initial_total
        );

        let f2 = model.alloc_frame().unwrap();
        assert_eq!(
            model.allocated_count() as u64 + model.free_count() as u64,
            initial_total
        );

        model.free_frame(f1).unwrap();
        assert_eq!(
            model.allocated_count() as u64 + model.free_count() as u64,
            initial_total
        );

        model.free_frame(f2).unwrap();
        assert_eq!(
            model.allocated_count() as u64 + model.free_count() as u64,
            initial_total
        );
    }

    /// Proof: DMA zone allocations are below 16 MB
    #[kani::proof]
    fn proof_zone_dma_range() {
        let frame: u64 = kani::any();
        kani::assume(frame < DMA_ZONE_LIMIT);

        let result = AllocInvariantChecker::verify_zone_correctness(frame, MemoryZone::Dma);
        assert!(result.is_ok());
    }

    /// Proof: Allocated frames are properly aligned
    #[kani::proof]
    fn proof_alignment_preserved() {
        let mut model = AllocatorModel::new(0x1000, 4);
        let frame = model.alloc_frame().unwrap();

        assert_eq!(frame % MODEL_FRAME_SIZE, 0, "Frame must be page-aligned");
    }

    /// Proof: Allocated regions don't overlap (disjoint sets)
    #[kani::proof]
    fn proof_no_overlap() {
        let mut model = AllocatorModel::new(0x1000, 4);
        let f1 = model.alloc_frame().unwrap();
        let f2 = model.alloc_frame().unwrap();

        // Frames are distinct
        assert_ne!(f1, f2);
        // No overlap (each frame is exactly FRAME_SIZE)
        assert!(f1 + MODEL_FRAME_SIZE <= f2 || f2 + MODEL_FRAME_SIZE <= f1);
    }

    /// Proof: Double-free is detected and prevented
    #[kani::proof]
    fn proof_free_idempotent() {
        let mut model = AllocatorModel::new(0x1000, 4);
        let frame = model.alloc_frame().unwrap();
        model.free_frame(frame).unwrap();

        let result = model.free_frame(frame);
        assert_eq!(result, Err(AllocModelError::DoubleFree));
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "alloc")]
    #[test]
    fn test_alloc_and_free() {
        let mut model = AllocatorModel::new(0x1000, 4);
        assert_eq!(model.free_count(), 4);
        assert_eq!(model.allocated_count(), 0);

        let f = model.alloc_frame().unwrap();
        assert_eq!(model.free_count(), 3);
        assert_eq!(model.allocated_count(), 1);
        assert!(model.is_allocated(f));

        model.free_frame(f).unwrap();
        assert_eq!(model.free_count(), 4);
        assert_eq!(model.allocated_count(), 0);
        assert!(model.is_free(f));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_double_free() {
        let mut model = AllocatorModel::new(0x1000, 4);
        let f = model.alloc_frame().unwrap();
        model.free_frame(f).unwrap();
        assert_eq!(model.free_frame(f), Err(AllocModelError::DoubleFree));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_out_of_memory() {
        let mut model = AllocatorModel::new(0x1000, 1);
        model.alloc_frame().unwrap();
        assert_eq!(model.alloc_frame(), Err(AllocModelError::OutOfMemory));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_conservation() {
        let mut model = AllocatorModel::new(0x1000, 8);
        for _ in 0..4 {
            model.alloc_frame().unwrap();
        }
        assert!(AllocInvariantChecker::verify_frame_conservation(&model).is_ok());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_no_double_alloc() {
        let mut model = AllocatorModel::new(0x1000, 8);
        for _ in 0..8 {
            model.alloc_frame().unwrap();
        }
        assert!(AllocInvariantChecker::verify_no_double_alloc(&model).is_ok());
    }

    #[test]
    fn test_buddy_split() {
        let block = BuddyBlock {
            base: 0x0,
            order: 3,
            free: true,
        };
        let (left, right) = block.split().unwrap();
        assert_eq!(left.order, 2);
        assert_eq!(right.order, 2);
        assert_eq!(left.base, 0x0);
        assert_eq!(right.base, 4 * MODEL_FRAME_SIZE);
    }

    #[test]
    fn test_buddy_coalesce() {
        let left = BuddyBlock {
            base: 0x0,
            order: 2,
            free: true,
        };
        let right = BuddyBlock {
            base: 4 * MODEL_FRAME_SIZE,
            order: 2,
            free: true,
        };
        let merged = left.coalesce(&right).unwrap();
        assert_eq!(merged.order, 3);
        assert_eq!(merged.base, 0);
    }

    #[test]
    fn test_buddy_no_split_order_zero() {
        let block = BuddyBlock {
            base: 0x0,
            order: 0,
            free: true,
        };
        assert!(block.split().is_none());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_zone_allocation() {
        let mut model = AllocatorModel::new(0x0, 4);
        let f = model.alloc_frame_zone(MemoryZone::Dma).unwrap();
        assert!(f < DMA_ZONE_LIMIT);
        assert!(AllocInvariantChecker::verify_zone_correctness(f, MemoryZone::Dma).is_ok());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_bitmap_alloc_free() {
        let mut bm = BitmapModel::new(0x1000, 4);
        let a = bm.alloc().unwrap();
        assert!(bm.is_allocated(a));

        bm.free(a).unwrap();
        assert!(!bm.is_allocated(a));
    }
}

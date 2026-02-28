//! Demand Paging and Copy-on-Write (COW) Manager
//!
//! Provides infrastructure for lazy page allocation and COW fork support.
//!
//! ## Demand Paging
//! Pages can be registered as "lazy" mappings via [`register_lazy`]. When
//! a page fault hits a lazy-mapped address, the manager allocates a physical
//! frame and returns it (along with the appropriate flags) so the caller can
//! install the mapping in the page table.
//!
//! ## Copy-on-Write
//! [`CowTable`] tracks shared physical frames with reference counts. When a
//! COW page is written, the fault handler calls
//! [`DemandPagingManager::handle_cow_fault`] to allocate a private copy and
//! decrement the shared reference count.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use spin::Mutex;

use crate::{
    error::KernelError,
    mm::{FrameNumber, PageFlags, FRAME_ALLOCATOR, PAGE_SIZE},
};

// ===========================================================================
// Lazy Mapping Types
// ===========================================================================

/// How a lazy page is backed when finally faulted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackingType {
    /// Anonymous memory (zero-filled on first access).
    Anonymous,
    /// File-backed memory (load from inode + offset).
    FileBacked {
        /// Inode number of the backing file.
        inode: u64,
        /// Byte offset into the file for this mapping.
        offset: u64,
    },
}

/// A region of virtual address space registered for demand paging.
///
/// No physical frames are allocated when a lazy mapping is created.
/// The first access triggers a page fault, which the demand paging
/// manager resolves by allocating a frame and returning it.
#[cfg(feature = "alloc")]
pub struct LazyMapping {
    /// Start virtual address (page-aligned).
    pub start_vaddr: usize,
    /// Size in bytes (multiple of PAGE_SIZE).
    pub size: usize,
    /// Page flags to apply when the page is finally mapped.
    pub flags: PageFlags,
    /// Backing type for this mapping.
    pub backing: BackingType,
    /// Per-page tracking: true if the page has been faulted in.
    faulted_in: Vec<bool>,
}

#[cfg(feature = "alloc")]
impl LazyMapping {
    /// Create a new lazy mapping.
    pub fn new(start_vaddr: usize, size: usize, flags: PageFlags, backing: BackingType) -> Self {
        let page_count = size.div_ceil(PAGE_SIZE);
        Self {
            start_vaddr,
            size,
            flags,
            backing,
            faulted_in: vec![false; page_count],
        }
    }

    /// Check whether a virtual address falls within this mapping.
    pub fn contains(&self, vaddr: usize) -> bool {
        vaddr >= self.start_vaddr && vaddr < self.start_vaddr + self.size
    }

    /// Page index for a given virtual address within this mapping.
    fn page_index(&self, vaddr: usize) -> usize {
        (vaddr - self.start_vaddr) / PAGE_SIZE
    }
}

// ===========================================================================
// Copy-on-Write Table
// ===========================================================================

/// A single COW-shared physical frame.
pub struct CowEntry {
    /// The shared physical frame.
    pub frame: FrameNumber,
    /// Number of address spaces sharing this frame.
    pub ref_count: AtomicU32,
}

/// Table of COW-shared frames keyed by virtual page address.
#[cfg(feature = "alloc")]
pub struct CowTable {
    /// Map from virtual page address to COW entry.
    pub entries: BTreeMap<usize, CowEntry>,
}

#[cfg(feature = "alloc")]
impl Default for CowTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl CowTable {
    /// Create an empty COW table.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Register a frame as COW-shared with initial ref_count = 2.
    pub fn mark_cow(&mut self, vaddr: usize, frame: FrameNumber) {
        self.entries.insert(
            vaddr,
            CowEntry {
                frame,
                ref_count: AtomicU32::new(2),
            },
        );
    }

    /// Decrement ref count for a COW page; returns true if this was the
    /// last reference (frame can be freed).
    pub fn release(&self, vaddr: usize) -> bool {
        if let Some(entry) = self.entries.get(&vaddr) {
            let prev = entry.ref_count.fetch_sub(1, Ordering::AcqRel);
            prev == 1
        } else {
            false
        }
    }

    /// Check whether an address is COW-tracked.
    pub fn is_cow(&self, vaddr: usize) -> bool {
        self.entries.contains_key(&vaddr)
    }
}

// ===========================================================================
// Demand Paging Manager
// ===========================================================================

/// Manages lazy mappings and COW state.
///
/// The manager does NOT directly modify page tables. Instead, its methods
/// return allocation results (frame number + flags) that the caller uses
/// to install the actual mapping via the VAS / page table infrastructure.
#[cfg(feature = "alloc")]
pub struct DemandPagingManager {
    /// Registered lazy mappings keyed by start address.
    lazy_mappings: BTreeMap<usize, LazyMapping>,
    /// COW-shared frame tracking.
    pub cow_table: CowTable,
}

#[cfg(feature = "alloc")]
impl Default for DemandPagingManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl DemandPagingManager {
    /// Create a new demand paging manager.
    pub fn new() -> Self {
        Self {
            lazy_mappings: BTreeMap::new(),
            cow_table: CowTable::new(),
        }
    }

    /// Register a lazy mapping. No physical memory is allocated.
    pub fn register_lazy(
        &mut self,
        start_vaddr: usize,
        size: usize,
        flags: PageFlags,
        backing: BackingType,
    ) {
        let mapping = LazyMapping::new(start_vaddr, size, flags, backing);
        self.lazy_mappings.insert(start_vaddr, mapping);
    }

    /// Try to resolve a demand-page fault at `vaddr`.
    ///
    /// If the address falls within a registered lazy mapping that has not
    /// yet been faulted in, allocates a physical frame and returns
    /// `Ok((frame, flags))`. The caller is responsible for installing the
    /// mapping in the page table.
    pub fn try_demand_page(
        &mut self,
        vaddr: usize,
    ) -> Result<(FrameNumber, PageFlags), KernelError> {
        // Find which lazy mapping contains this address.
        let mapping = self.lazy_mappings.values_mut().find(|m| m.contains(vaddr));

        let mapping = match mapping {
            Some(m) => m,
            None => {
                return Err(KernelError::UnmappedMemory { addr: vaddr });
            }
        };

        let idx = mapping.page_index(vaddr);
        if idx >= mapping.faulted_in.len() {
            return Err(KernelError::InvalidAddress { addr: vaddr });
        }
        if mapping.faulted_in[idx] {
            // Already faulted in -- not a lazy fault.
            return Err(KernelError::InvalidAddress { addr: vaddr });
        }

        // Allocate a physical frame.
        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: PAGE_SIZE,
                available: 0,
            })?;

        // Zero the frame for anonymous mappings.
        if mapping.backing == BackingType::Anonymous {
            let virt = crate::mm::phys_to_virt_addr(frame.as_u64() * PAGE_SIZE as u64) as *mut u8;
            // SAFETY: frame is freshly allocated within the physical memory window.
            unsafe {
                core::ptr::write_bytes(virt, 0, PAGE_SIZE);
            }
        }

        mapping.faulted_in[idx] = true;
        let flags = mapping.flags;

        Ok((frame, flags))
    }

    /// Handle a COW fault at `vaddr`.
    ///
    /// Allocates a new frame, copies the contents from the old shared frame,
    /// decrements the COW ref count, and returns the new frame.
    pub fn handle_cow_fault(&self, vaddr: usize) -> Result<FrameNumber, KernelError> {
        let page_addr = vaddr & !(PAGE_SIZE - 1);

        let entry = self
            .cow_table
            .entries
            .get(&page_addr)
            .ok_or(KernelError::InvalidAddress { addr: vaddr })?;

        let old_frame = entry.frame;

        // Allocate a private copy.
        let new_frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: PAGE_SIZE,
                available: 0,
            })?;

        // Copy old frame contents to new frame.
        let old_virt = crate::mm::phys_to_virt_addr(old_frame.as_u64() * PAGE_SIZE as u64);
        let new_virt = crate::mm::phys_to_virt_addr(new_frame.as_u64() * PAGE_SIZE as u64);
        // SAFETY: Both virtual addresses are within the kernel physical memory
        // window, pointing to valid 4KB frames.
        unsafe {
            core::ptr::copy_nonoverlapping(old_virt as *const u8, new_virt as *mut u8, PAGE_SIZE);
        }

        // Decrement ref count on the old shared frame.
        let _last_ref = self.cow_table.release(page_addr);

        Ok(new_frame)
    }

    /// Remove a lazy mapping.
    pub fn unregister_lazy(&mut self, start_vaddr: usize) {
        self.lazy_mappings.remove(&start_vaddr);
    }

    /// Mark a range of pages as COW-shared.
    pub fn mark_cow_range(&mut self, base: usize, pages: &[(usize, FrameNumber)]) {
        for &(vaddr, frame) in pages {
            let _ = base; // base provided for future use (relative offsets)
            self.cow_table.mark_cow(vaddr, frame);
        }
    }

    /// Add a single COW entry (used by cow_fork).
    pub fn add_cow_entry(&mut self, vaddr: usize, frame: FrameNumber) {
        self.cow_table.mark_cow(vaddr, frame);
    }
}

// ===========================================================================
// Global Instance
// ===========================================================================

#[cfg(feature = "alloc")]
static DEMAND_PAGING: Mutex<Option<DemandPagingManager>> = Mutex::new(None);

/// Initialize the global demand paging manager.
#[cfg(feature = "alloc")]
pub fn init() {
    *DEMAND_PAGING.lock() = Some(DemandPagingManager::new());
    crate::println!("[DEMAND_PAGING] Manager initialized");
}

/// Register a lazy mapping via the global manager.
#[cfg(feature = "alloc")]
pub fn register_lazy(start_vaddr: usize, size: usize, flags: PageFlags, backing: BackingType) {
    if let Some(ref mut mgr) = *DEMAND_PAGING.lock() {
        mgr.register_lazy(start_vaddr, size, flags, backing);
    }
}

/// Try to resolve a page fault via demand paging.
///
/// Returns `Ok((frame, flags))` if the fault was resolved.
#[cfg(feature = "alloc")]
pub fn handle_page_fault(vaddr: usize) -> Result<(FrameNumber, PageFlags), KernelError> {
    let mut guard = DEMAND_PAGING.lock();
    let mgr = guard.as_mut().ok_or(KernelError::NotInitialized {
        subsystem: "demand_paging",
    })?;
    mgr.try_demand_page(vaddr)
}

/// Access the global demand paging manager (mutable).
#[cfg(feature = "alloc")]
pub fn with_manager_mut<R, F: FnOnce(&mut DemandPagingManager) -> R>(f: F) -> R {
    let mut guard = DEMAND_PAGING.lock();
    let mgr = guard.get_or_insert_with(DemandPagingManager::new);
    f(mgr)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backing_type() {
        let anon = BackingType::Anonymous;
        let file = BackingType::FileBacked {
            inode: 42,
            offset: 0,
        };
        assert_eq!(anon, BackingType::Anonymous);
        assert_ne!(anon, file);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_cow_table() {
        let mut table = CowTable::new();
        let frame = FrameNumber::new(100);
        table.mark_cow(0x1000, frame);

        assert!(table.is_cow(0x1000));
        assert!(!table.is_cow(0x2000));

        // First release: ref goes from 2 -> 1, not last
        assert!(!table.release(0x1000));
        // Second release: ref goes from 1 -> 0, last ref
        assert!(table.release(0x1000));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_lazy_mapping_contains() {
        let mapping = LazyMapping::new(
            0x10000,
            PAGE_SIZE * 4,
            PageFlags::PRESENT | PageFlags::WRITABLE,
            BackingType::Anonymous,
        );

        assert!(mapping.contains(0x10000));
        assert!(mapping.contains(0x10000 + PAGE_SIZE * 3));
        assert!(!mapping.contains(0x10000 + PAGE_SIZE * 4));
        assert!(!mapping.contains(0x0));
    }
}

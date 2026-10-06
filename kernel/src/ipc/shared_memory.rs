//! Zero-copy shared memory IPC implementation
//!
//! Provides high-performance shared memory regions for large data transfers
//! between processes without copying.

// Shared memory IPC -- used for zero-copy large transfers
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use spin::Mutex;

use super::{error::Result, IpcError};
use crate::{
    mm::{PageFlags, PageSize, PhysicalAddress, VirtualAddress},
    process::ProcessId,
};

/// Shared memory region ID generator
static REGION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Memory region permissions
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Read-only access
    Read = 0b001,
    /// Write access (implies read)
    Write = 0b011,
    /// Execute access
    Execute = 0b100,
    /// Read and execute
    ReadExecute = 0b101,
    /// Read, write, and execute
    ReadWriteExecute = 0b111,
}

/// Alias for Permission to match test expectations
pub type Permissions = Permission;

/// Transfer mode for shared memory operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    /// Move ownership to receiver
    Move,
    /// Share region with receiver
    Share,
    /// Copy-on-write sharing
    CopyOnWrite,
}

impl Permission {
    /// Constant for read-write permissions
    pub const READ_WRITE: Self = Self::Write;

    /// Check if permission allows reading
    pub fn can_read(self) -> bool {
        (self as u32) & 0b001 != 0
    }

    /// Check if permission allows writing
    pub fn can_write(self) -> bool {
        (self as u32) & 0b010 != 0
    }

    /// Check if permission allows execution
    pub fn can_execute(self) -> bool {
        (self as u32) & 0b100 != 0
    }
}

/// Cache policy for shared memory regions
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Write-back caching (default)
    WriteBack = 0,
    /// Write-through caching
    WriteThrough = 1,
    /// Uncached (for device memory)
    Uncached = 2,
    /// Write-combining (for framebuffers)
    WriteCombining = 3,
}

/// Shared memory region descriptor
///
/// The region owns its physical frames and frees them when dropped. Every
/// process that maps it gets a `MappingType::SharedRegion` mapping of those
/// same frames, which its address space never frees (IPC-INC-02). Regions
/// that processes share live in the registry below, which keeps them alive.
#[derive(Debug)]
pub struct SharedRegion {
    /// Unique region ID
    id: u64,
    /// Physical memory backing this region
    physical_base: PhysicalAddress,
    /// Size of the region in bytes
    size: usize,
    /// Owner process
    owner: ProcessId,
    /// Processes with access to this region
    mappings: Mutex<BTreeMap<ProcessId, RegionMapping>>,
    /// Number of active mappings
    ref_count: AtomicU32,
    /// Cache policy
    cache_policy: CachePolicy,
    /// NUMA node preference
    numa_node: Option<u32>,
}

/// Per-process mapping of a shared region
#[derive(Debug, Clone)]
struct RegionMapping {
    /// Virtual address in the process
    virtual_base: VirtualAddress,
    /// Permissions for this mapping
    permissions: Permission,
}

impl SharedRegion {
    /// Create a new shared memory region (convenience wrapper).
    ///
    /// Returns an error if physical memory cannot be allocated for the region.
    pub fn new(owner: ProcessId, size: usize, _permissions: Permission) -> Result<Self> {
        Self::new_with_policy(owner, size, CachePolicy::WriteBack, None)
    }

    /// Create a new shared memory region backed by real physical frames.
    ///
    /// Allocates contiguous physical frames from the global frame allocator
    /// and zeroes them (they are about to be mapped into user space).
    /// Returns `IpcError::OutOfMemory` if the allocation fails.
    pub fn new_with_policy(
        owner: ProcessId,
        size: usize,
        cache_policy: CachePolicy,
        numa_node: Option<u32>,
    ) -> Result<Self> {
        if size == 0 {
            return Err(IpcError::InvalidMemoryRegion);
        }
        // Round size up to page boundary
        let page_size = PageSize::Small as usize;
        let size = size.div_ceil(page_size) * page_size;
        let num_frames = size / page_size;

        // Allocate physical frames from the global frame allocator
        let frame = crate::mm::FRAME_ALLOCATOR
            .lock()
            .allocate_frames(num_frames, numa_node.map(|n| n as usize))
            .map_err(|_| IpcError::OutOfMemory)?;

        let physical_base = PhysicalAddress::new(frame.as_u64() * page_size as u64);

        // SAFETY: the frames were just allocated for this region and nothing
        // else references them; the kernel's physical map covers them.
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(physical_base.as_u64()) as *mut u8;
            core::ptr::write_bytes(virt, 0, size);
        }

        Ok(Self {
            id: REGION_COUNTER.fetch_add(1, Ordering::Relaxed),
            physical_base,
            size,
            owner,
            mappings: Mutex::new(BTreeMap::new()),
            ref_count: AtomicU32::new(0),
            cache_policy,
            numa_node,
        })
    }

    /// Get region ID
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get region size
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the physical base address of the backing memory
    pub fn physical_base(&self) -> PhysicalAddress {
        self.physical_base
    }

    /// The owner process
    pub fn owner(&self) -> ProcessId {
        self.owner
    }

    fn frames(&self) -> Vec<crate::mm::FrameNumber> {
        let first = self.physical_base.as_u64() / PageSize::Small as u64;
        (0..(self.size / PageSize::Small as usize) as u64)
            .map(|i| crate::mm::FrameNumber::new(first + i))
            .collect()
    }

    /// Copy `data` into the region at `offset`.
    pub fn write_at(&self, offset: usize, data: &[u8]) -> Result<()> {
        if offset
            .checked_add(data.len())
            .is_none_or(|end| end > self.size)
        {
            return Err(IpcError::InvalidMemoryRegion);
        }
        // SAFETY: the region owns `size` contiguous bytes at physical_base,
        // reachable through the kernel's physical map, for its lifetime, and
        // `offset + data.len() <= size`. Processes mapping the region may write it
        // concurrently; it holds plain bytes, so a racing write only mixes
        // contents, as with any shared memory.
        unsafe {
            let dst = crate::mm::phys_to_virt_addr(self.physical_base.as_u64()) as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr(), dst.add(offset), data.len());
        }
        Ok(())
    }

    /// Map region into a process address space and return where.
    ///
    /// Installs page-table entries for the region's own frames (it used to
    /// only record the address, and the "zero-copy" paths that relied on it
    /// mapped fresh zeroed frames). `at` must be page-aligned user space
    /// that is not already mapped; `None` lets the address space choose.
    /// Authorization is the caller's job (capability checks at the syscall
    /// or transfer boundary).
    pub fn map(
        &self,
        process: ProcessId,
        at: Option<VirtualAddress>,
        permissions: Permission,
    ) -> Result<VirtualAddress> {
        let mut mappings = self.mappings.lock();
        if mappings.contains_key(&process) {
            return Err(IpcError::InvalidMemoryRegion);
        }

        let proc = crate::process::find_process(process).ok_or(IpcError::ProcessNotFound)?;
        let mut flags = PageFlags::PRESENT | PageFlags::USER;
        if permissions.can_write() {
            flags |= PageFlags::WRITABLE;
        }
        if !permissions.can_execute() {
            flags |= PageFlags::NO_EXECUTE;
        }
        let virtual_base = proc
            .memory_space
            .lock()
            .map_borrowed_frames(at, &self.frames(), flags)
            .map_err(|_| IpcError::InvalidMemoryRegion)?;

        mappings.insert(
            process,
            RegionMapping {
                virtual_base,
                permissions,
            },
        );
        self.ref_count.fetch_add(1, Ordering::Relaxed);
        Ok(virtual_base)
    }

    /// Unmap region from a process. The page-table entries are removed and
    /// flushed page by page; the frames stay with the region.
    pub fn unmap(&self, process: ProcessId) -> Result<()> {
        let mapping = self
            .mappings
            .lock()
            .remove(&process)
            .ok_or(IpcError::InvalidMemoryRegion)?;
        self.ref_count.fetch_sub(1, Ordering::Relaxed);

        // A process that already exited took its page tables with it.
        if let Some(proc) = crate::process::find_process(process) {
            proc.memory_space
                .lock()
                .unmap_region(mapping.virtual_base)
                .map_err(|_| IpcError::InvalidMemoryRegion)?;
        }
        Ok(())
    }

    /// Transfer ownership of region to another process.
    ///
    /// Validates that the target process exists before transferring.
    pub fn transfer_ownership(&mut self, new_owner: ProcessId) -> Result<()> {
        // Validate new owner exists
        if crate::process::find_process(new_owner).is_none() {
            return Err(IpcError::ProcessNotFound);
        }
        self.owner = new_owner;
        Ok(())
    }

    /// Get virtual address for a specific process
    pub fn get_mapping(&self, process: ProcessId) -> Option<VirtualAddress> {
        self.mappings.lock().get(&process).map(|m| m.virtual_base)
    }

    /// Number of processes mapping the region.
    pub fn mapping_count(&self) -> u32 {
        self.ref_count.load(Ordering::Relaxed)
    }

    /// Get the NUMA node for this region
    pub fn numa_node(&self) -> usize {
        self.numa_node.unwrap_or(0) as usize
    }

    /// Create a new shared memory region with specific NUMA node.
    ///
    /// Returns an error if physical memory cannot be allocated for the region.
    pub fn new_numa(
        owner: ProcessId,
        size: usize,
        _permissions: Permission,
        numa_node: usize,
    ) -> Result<Self> {
        Self::new_with_policy(owner, size, CachePolicy::WriteBack, Some(numa_node as u32))
    }
}

impl Drop for SharedRegion {
    fn drop(&mut self) {
        // Mappings borrow the frames; never free memory a process can still
        // reach. Leaking is the safe failure.
        if self.ref_count.load(Ordering::Relaxed) != 0 {
            crate::kprintln!(
                "[IPC] Shared region {} dropped while mapped; leaking its frames",
                self.id
            );
            return;
        }
        let page_size = PageSize::Small as usize;
        let first = crate::mm::FrameNumber::new(self.physical_base.as_u64() / page_size as u64);
        let _ = crate::mm::FRAME_ALLOCATOR
            .lock()
            .free_frames(first, self.size / page_size);
    }
}

// ── Region registry ─────────────────────────────────────────────────────────
//
// Regions shared through capabilities live here, keyed by physical base (the
// identity a memory capability carries), so a mapping request can find the
// region's frames. A region is removed only when no process maps it.

static REGISTRY: Mutex<BTreeMap<u64, Arc<SharedRegion>>> = Mutex::new(BTreeMap::new());

/// Register a region and return the shared handle.
pub fn register_region(region: SharedRegion) -> Arc<SharedRegion> {
    let region = Arc::new(region);
    REGISTRY
        .lock()
        .insert(region.physical_base().as_u64(), region.clone());
    region
}

/// The registered region whose physical base is `base`.
pub fn lookup_region(base: u64) -> Option<Arc<SharedRegion>> {
    REGISTRY.lock().get(&base).cloned()
}

/// Remove a registered region; refused while any process maps it. Its
/// frames are freed when the last handle is dropped.
pub fn unregister_region(base: u64) -> Result<()> {
    let mut registry = REGISTRY.lock();
    match registry.get(&base) {
        None => Err(IpcError::InvalidMemoryRegion),
        Some(region) if region.mapping_count() > 0 => Err(IpcError::ResourceBusy),
        Some(_) => {
            registry.remove(&base);
            Ok(())
        }
    }
}

// MemoryRegion is defined in ipc::message -- re-use it here.
pub use super::message::MemoryRegion;

impl MemoryRegion {
    /// Create from a SharedRegion
    pub fn from_shared(region: &SharedRegion, vaddr: VirtualAddress) -> Self {
        Self {
            base_addr: vaddr.as_u64(),
            size: region.size as u64,
            permissions: Permission::Read as u32, // Default to read-only
            cache_policy: region.cache_policy as u32,
        }
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_permission_flags() {
        assert!(Permission::Read.can_read());
        assert!(!Permission::Read.can_write());
        assert!(!Permission::Read.can_execute());

        assert!(Permission::Write.can_read());
        assert!(Permission::Write.can_write());
        assert!(!Permission::Write.can_execute());

        assert!(Permission::ReadWriteExecute.can_read());
        assert!(Permission::ReadWriteExecute.can_write());
        assert!(Permission::ReadWriteExecute.can_execute());
    }

    // These tests require the global FRAME_ALLOCATOR to be initialized with
    // physical memory, which is only available on bare-metal targets.
    #[cfg(target_os = "none")]
    #[test]
    fn test_shared_region_creation() {
        let region =
            SharedRegion::new_with_policy(ProcessId(1), 4096, CachePolicy::WriteBack, None)
                .unwrap();
        assert_eq!(region.size(), 4096);
        assert_eq!(region.owner, ProcessId(1));
    }
}

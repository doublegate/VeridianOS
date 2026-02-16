//! Zero-copy shared memory IPC implementation
//!
//! Provides high-performance shared memory regions for large data transfers
//! between processes without copying.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use spin::Mutex;

use super::{error::Result, IpcError};
use crate::{
    mm::{PageSize, PhysicalAddress, VirtualAddress},
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
    /// Reference count
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
    /// Whether this mapping is active
    active: bool,
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
    /// to back the shared region. Returns `IpcError::OutOfMemory` if the
    /// allocation fails.
    pub fn new_with_policy(
        owner: ProcessId,
        size: usize,
        cache_policy: CachePolicy,
        numa_node: Option<u32>,
    ) -> Result<Self> {
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

        Ok(Self {
            id: REGION_COUNTER.fetch_add(1, Ordering::Relaxed),
            physical_base,
            size,
            owner,
            mappings: Mutex::new(BTreeMap::new()),
            ref_count: AtomicU32::new(1),
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

    /// Map region into a process address space
    pub fn map(
        &self,
        process: ProcessId,
        virtual_base: VirtualAddress,
        permissions: Permission,
    ) -> Result<()> {
        // Verify the calling process has capability to map this region
        if let Some(current_process) = crate::process::current_process() {
            // Only owner or processes with proper capability can map
            if current_process.pid != self.owner && current_process.pid != process {
                // Would need to check for a memory capability here
                // For now, only allow owner to map
                return Err(IpcError::PermissionDenied);
            }
        }

        // Check if process already has a mapping
        let mut mappings = self.mappings.lock();
        if mappings.contains_key(&process) {
            return Err(IpcError::InvalidMemoryRegion);
        }

        // Flush TLB for all pages in the mapped range so the CPU picks up
        // the new mapping immediately.
        let num_pages = self.size / (PageSize::Small as usize);
        for i in 0..num_pages {
            let page_addr = virtual_base.as_u64() + (i as u64) * (PageSize::Small as u64);
            crate::arch::tlb_flush_address(page_addr);
        }

        mappings.insert(
            process,
            RegionMapping {
                virtual_base,
                permissions,
                active: true,
            },
        );

        self.ref_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Unmap region from a process and flush the TLB for the affected range.
    pub fn unmap(&self, process: ProcessId) -> Result<()> {
        let mut mappings = self.mappings.lock();

        if let Some(mapping) = mappings.get_mut(&process) {
            if !mapping.active {
                return Err(IpcError::InvalidMemoryRegion);
            }

            // Flush TLB for every page in the unmapped range so stale
            // translations are invalidated.
            let num_pages = self.size / (PageSize::Small as usize);
            for i in 0..num_pages {
                let page_addr =
                    mapping.virtual_base.as_u64() + (i as u64) * (PageSize::Small as u64);
                crate::arch::tlb_flush_address(page_addr);
            }

            mapping.active = false;
            self.ref_count.fetch_sub(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(IpcError::InvalidMemoryRegion)
        }
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
        self.mappings
            .lock()
            .get(&process)
            .filter(|m| m.active)
            .map(|m| m.virtual_base)
    }

    /// Create a capability for this shared region
    pub fn create_capability(&self, target_process: ProcessId, mode: TransferMode) -> u64 {
        use crate::cap::{
            token::{CapabilityFlags, CapabilityToken},
            types::{Capability, CapabilityId, CapabilityPermissions, CapabilityType},
        };

        // Determine permissions based on transfer mode
        let perms = match mode {
            TransferMode::Move => {
                CapabilityPermissions::READ
                    | CapabilityPermissions::WRITE
                    | CapabilityPermissions::GRANT
            }
            TransferMode::Share => CapabilityPermissions::READ | CapabilityPermissions::WRITE,
            TransferMode::CopyOnWrite => CapabilityPermissions::READ,
        };

        // Create capability ID based on region ID and target process
        let cap_id = CapabilityId(self.id ^ target_process.0);

        // Create capability for shared memory region
        let _cap = Capability::new(
            cap_id,
            CapabilityType::Memory,
            perms,
            self.physical_base.as_u64(),
        );

        // Create token with appropriate flags
        let flags = match mode {
            TransferMode::Move => CapabilityFlags::Read as u8 | CapabilityFlags::Write as u8,
            TransferMode::Share => CapabilityFlags::Read as u8 | CapabilityFlags::Write as u8,
            TransferMode::CopyOnWrite => CapabilityFlags::Read as u8,
        };

        let token = CapabilityToken::new(cap_id.0, 0, CapabilityType::Memory as u8, flags);

        token.to_u64()
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

/// Memory region descriptor for IPC messages
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    /// Virtual address in sender's address space
    pub base_addr: u64,
    /// Size of the region
    pub size: u64,
    /// Permissions (read/write/execute)
    pub permissions: u32,
    /// Cache policy
    pub cache_policy: u32,
}

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

/// Shared memory manager
pub struct SharedMemoryManager {
    /// All shared regions in the system
    regions: Mutex<BTreeMap<u64, SharedRegion>>,
    /// NUMA node memory tracking
    numa_stats: Vec<AtomicU64>,
}

impl SharedMemoryManager {
    /// Create a new shared memory manager
    pub fn new(numa_nodes: usize) -> Self {
        let mut numa_stats = Vec::with_capacity(numa_nodes);
        for _ in 0..numa_nodes {
            numa_stats.push(AtomicU64::new(0));
        }

        Self {
            regions: Mutex::new(BTreeMap::new()),
            numa_stats,
        }
    }

    /// Create a new shared memory region
    pub fn create_region(
        &self,
        owner: ProcessId,
        size: usize,
        cache_policy: CachePolicy,
        numa_node: Option<u32>,
    ) -> Result<u64> {
        let region = SharedRegion::new_with_policy(owner, size, cache_policy, numa_node)?;
        let id = region.id();

        // Track NUMA allocation
        if let Some(node) = numa_node {
            if (node as usize) < self.numa_stats.len() {
                self.numa_stats[node as usize].fetch_add(size as u64, Ordering::Relaxed);
            }
        }

        self.regions.lock().insert(id, region);
        Ok(id)
    }

    /// Get a shared region by ID
    pub fn get_region(&self, id: u64) -> Option<u64> {
        self.regions.lock().get(&id).map(|r| r.id)
    }

    /// Remove a shared region
    pub fn remove_region(&self, id: u64) -> Result<()> {
        let mut regions = self.regions.lock();
        if let Some(region) = regions.remove(&id) {
            // Check reference count
            if region.ref_count.load(Ordering::Relaxed) > 0 {
                // Still in use, put it back
                regions.insert(id, region);
                return Err(IpcError::ResourceBusy);
            }

            // Update NUMA stats
            if let Some(node) = region.numa_node {
                if (node as usize) < self.numa_stats.len() {
                    self.numa_stats[node as usize].fetch_sub(region.size as u64, Ordering::Relaxed);
                }
            }

            // Free physical frames backing this region
            let page_size = PageSize::Small as usize;
            let num_frames = region.size / page_size;
            let frame_number =
                crate::mm::FrameNumber::new(region.physical_base.as_u64() / page_size as u64);
            if let Err(_e) = crate::mm::FRAME_ALLOCATOR
                .lock()
                .free_frames(frame_number, num_frames)
            {
                crate::kprintln!(
                    "[IPC] Warning: Failed to free physical frames for shared memory region"
                );
            }

            Ok(())
        } else {
            Err(IpcError::InvalidMemoryRegion)
        }
    }

    /// Get NUMA memory usage statistics
    pub fn numa_usage(&self, node: u32) -> Option<u64> {
        self.numa_stats
            .get(node as usize)
            .map(|stat| stat.load(Ordering::Relaxed))
    }
}

/// Zero-copy message transfer using shared memory
pub fn zero_copy_transfer(
    _region_id: u64,
    _from_process: ProcessId,
    _to_process: ProcessId,
    _manager: &SharedMemoryManager,
) -> Result<()> {
    // TODO(future): Implement zero-copy transfer (capability validation, page
    // remap, TLB flush)

    Ok(())
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

    #[test]
    fn test_shared_region_creation() {
        let region = SharedRegion::new_with_policy(1, 4096, CachePolicy::WriteBack, None).unwrap();
        assert_eq!(region.size(), 4096);
        assert_eq!(region.owner, 1);
    }

    #[test]
    fn test_memory_manager() {
        let manager = SharedMemoryManager::new(4);
        let id = manager
            .create_region(1, 8192, CachePolicy::WriteBack, Some(0))
            .unwrap();

        assert!(manager.get_region(id).is_some());
        assert_eq!(manager.numa_usage(0), Some(8192));
    }
}

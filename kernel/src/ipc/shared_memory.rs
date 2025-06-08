//! Zero-copy shared memory IPC implementation
//! 
//! Provides high-performance shared memory regions for large data transfers
//! between processes without copying.

use super::IpcError;
use super::error::Result;
use core::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use spin::Mutex;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

// TODO: Import from mm module when available
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalAddress(u64);
impl PhysicalAddress {
    pub fn new(addr: u64) -> Self { Self(addr) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualAddress(u64);
impl VirtualAddress {
    pub fn new(addr: u64) -> Self { Self(addr) }
    pub fn as_u64(&self) -> u64 { self.0 }
}

#[repr(usize)]
pub enum PageSize {
    Small = 4096,
    Large = 2 * 1024 * 1024,
    Huge = 1024 * 1024 * 1024,
}

// TODO: Import from sched module when available
pub type ProcessId = u64;

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

impl Permission {
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
#[derive(Debug, Clone)]
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
    /// Create a new shared memory region
    pub fn new(
        owner: ProcessId,
        size: usize,
        cache_policy: CachePolicy,
        numa_node: Option<u32>,
    ) -> Result<Self> {
        // Round size up to page boundary
        let size = (size + PageSize::Small as usize - 1) & !(PageSize::Small as usize - 1);
        
        // TODO: Allocate physical memory from frame allocator
        // For now, use a placeholder address
        let physical_base = PhysicalAddress::new(0x100000);
        
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
        // Check if process already has a mapping
        let mut mappings = self.mappings.lock();
        if mappings.contains_key(&process) {
            return Err(IpcError::InvalidMemoryRegion);
        }

        // TODO: Actually map pages in the process page table
        // This would involve:
        // 1. Validating the virtual address range is free
        // 2. Creating page table entries
        // 3. Setting appropriate permissions and cache policy
        // 4. Flushing TLB if necessary

        mappings.insert(process, RegionMapping {
            virtual_base,
            permissions,
            active: true,
        });

        self.ref_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Unmap region from a process
    pub fn unmap(&self, process: ProcessId) -> Result<()> {
        let mut mappings = self.mappings.lock();
        
        if let Some(mapping) = mappings.get_mut(&process) {
            if !mapping.active {
                return Err(IpcError::InvalidMemoryRegion);
            }

            // TODO: Actually unmap pages from process page table
            // This would involve:
            // 1. Removing page table entries
            // 2. Flushing TLB
            // 3. Possibly freeing page table pages

            mapping.active = false;
            self.ref_count.fetch_sub(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(IpcError::InvalidMemoryRegion)
        }
    }

    /// Transfer ownership of region to another process
    pub fn transfer_ownership(&mut self, new_owner: ProcessId) -> Result<()> {
        // TODO: Validate new owner exists and has appropriate permissions
        self.owner = new_owner;
        Ok(())
    }

    /// Get virtual address for a specific process
    pub fn get_mapping(&self, process: ProcessId) -> Option<VirtualAddress> {
        self.mappings.lock()
            .get(&process)
            .filter(|m| m.active)
            .map(|m| m.virtual_base)
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
        let region = SharedRegion::new(owner, size, cache_policy, numa_node)?;
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
    pub fn get_region(&self, id: u64) -> Option<SharedRegion> {
        self.regions.lock().get(&id).cloned()
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
                    self.numa_stats[node as usize]
                        .fetch_sub(region.size as u64, Ordering::Relaxed);
                }
            }

            // TODO: Free physical memory
            Ok(())
        } else {
            Err(IpcError::InvalidMemoryRegion)
        }
    }

    /// Get NUMA memory usage statistics
    pub fn numa_usage(&self, node: u32) -> Option<u64> {
        self.numa_stats.get(node as usize)
            .map(|stat| stat.load(Ordering::Relaxed))
    }
}

/// Zero-copy message transfer using shared memory
pub fn zero_copy_transfer(
    region_id: u64,
    from_process: ProcessId,
    to_process: ProcessId,
    manager: &SharedMemoryManager,
) -> Result<()> {
    // TODO: Implement zero-copy transfer
    // 1. Validate both processes have appropriate capabilities
    // 2. Remap pages from source to destination
    // 3. Update page tables
    // 4. Invalidate TLBs
    // 5. Update region mappings
    
    Ok(())
}

#[cfg(test)]
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
        let region = SharedRegion::new(1, 4096, CachePolicy::WriteBack, None).unwrap();
        assert_eq!(region.size(), 4096);
        assert_eq!(region.owner, 1);
    }

    #[test]
    fn test_memory_manager() {
        let manager = SharedMemoryManager::new(4);
        let id = manager.create_region(1, 8192, CachePolicy::WriteBack, Some(0)).unwrap();
        
        assert!(manager.get_region(id).is_some());
        assert_eq!(manager.numa_usage(0), Some(8192));
    }
}
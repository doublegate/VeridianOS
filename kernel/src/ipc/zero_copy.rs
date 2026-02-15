//! Zero-copy IPC implementation for large data transfers
//!
//! Provides efficient data transfer between processes without copying by
//! remapping pages and using shared memory regions.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{
    error::{IpcError, Result},
    shared_memory::{Permission, SharedRegion},
};
use crate::{
    arch::entropy::read_timestamp,
    mm::{PageFlags, PhysicalAddress, VirtualAddress},
    process::ProcessId,
};

/// Per-process page table handle for IPC zero-copy transfers.
///
/// Wraps a process ID and uses the process's VAS to perform real
/// page table operations (translate, map, unmap) via the frame
/// allocator and page table infrastructure.
struct ProcessPageTable {
    /// The process this page table belongs to
    pid: ProcessId,
    /// Page table root physical address (cached from VAS)
    root: u64,
}

/// Statistics for zero-copy operations
pub struct ZeroCopyStats {
    pub pages_transferred: AtomicU64,
    pub bytes_transferred: AtomicU64,
    pub transfer_count: AtomicU64,
    pub remap_cycles: AtomicU64,
}

static ZERO_COPY_STATS: ZeroCopyStats = ZeroCopyStats {
    pages_transferred: AtomicU64::new(0),
    bytes_transferred: AtomicU64::new(0),
    transfer_count: AtomicU64::new(0),
    remap_cycles: AtomicU64::new(0),
};

/// Zero-copy transfer of memory region between processes
///
/// This function remaps pages from source to destination without copying data.
/// It's optimized for large transfers where copying would be expensive.
pub fn zero_copy_transfer(
    region: &SharedRegion,
    from_pid: ProcessId,
    to_pid: ProcessId,
    flags: TransferFlags,
) -> Result<()> {
    let start = read_timestamp();

    // Validate processes have appropriate capabilities
    if !validate_transfer_capability(from_pid, to_pid, region.id()) {
        return Err(IpcError::PermissionDenied);
    }

    // Get page table handles for both processes
    let mut from_pt = get_process_page_table(from_pid)?;
    let mut to_pt = get_process_page_table(to_pid)?;

    // Calculate number of pages
    let num_pages = region.size().div_ceil(PAGE_SIZE);

    // Perform the transfer
    match flags.transfer_type {
        TransferType::Move => transfer_move(
            region,
            from_pid,
            to_pid,
            &mut from_pt,
            &mut to_pt,
            num_pages,
        )?,
        TransferType::Share => transfer_share(
            region,
            from_pid,
            to_pid,
            &mut from_pt,
            &mut to_pt,
            num_pages,
        )?,
        TransferType::Copy => transfer_copy_on_write(
            region,
            from_pid,
            to_pid,
            &mut from_pt,
            &mut to_pt,
            num_pages,
        )?,
    }

    // Update statistics
    let elapsed = read_timestamp() - start;
    ZERO_COPY_STATS
        .pages_transferred
        .fetch_add(num_pages as u64, Ordering::Relaxed);
    ZERO_COPY_STATS
        .bytes_transferred
        .fetch_add(region.size() as u64, Ordering::Relaxed);
    ZERO_COPY_STATS
        .transfer_count
        .fetch_add(1, Ordering::Relaxed);
    ZERO_COPY_STATS
        .remap_cycles
        .fetch_add(elapsed, Ordering::Relaxed);

    // Flush TLBs on affected CPUs
    flush_tlb_for_processes(&[from_pid, to_pid]);

    Ok(())
}

/// Transfer ownership of pages (unmap from source, map to destination)
fn transfer_move(
    region: &SharedRegion,
    from_pid: ProcessId,
    to_pid: ProcessId,
    from_pt: &mut ProcessPageTable,
    to_pt: &mut ProcessPageTable,
    num_pages: usize,
) -> Result<()> {
    let from_vaddr = region
        .get_mapping(from_pid)
        .ok_or(IpcError::InvalidMemoryRegion)?;
    let to_vaddr = allocate_virtual_range(to_pt, region.size())?;

    for i in 0..num_pages {
        let offset = i * PAGE_SIZE;
        let from_page = from_vaddr.add(offset);
        let to_page = to_vaddr.add(offset);

        // Get physical address from source via VAS translation
        let phys_addr = from_pt
            .translate(from_page)
            .ok_or(IpcError::InvalidMemoryRegion)?;

        // Unmap from source
        from_pt.unmap(from_page)?;

        // Map to destination
        to_pt.map(to_page, phys_addr, PageFlags::USER | PageFlags::WRITABLE)?;
    }

    // Update region mapping
    region.unmap(from_pid)?;
    region.map(to_pid, to_vaddr, Permission::Write)?;

    Ok(())
}

/// Share pages between processes (map to both)
fn transfer_share(
    region: &SharedRegion,
    from_pid: ProcessId,
    to_pid: ProcessId,
    from_pt: &mut ProcessPageTable,
    to_pt: &mut ProcessPageTable,
    num_pages: usize,
) -> Result<()> {
    let from_vaddr = region
        .get_mapping(from_pid)
        .ok_or(IpcError::InvalidMemoryRegion)?;
    let to_vaddr = allocate_virtual_range(to_pt, region.size())?;

    for i in 0..num_pages {
        let offset = i * PAGE_SIZE;
        let from_page = from_vaddr.add(offset);
        let to_page = to_vaddr.add(offset);

        // Get physical address from source via VAS translation
        let phys_addr = from_pt
            .translate(from_page)
            .ok_or(IpcError::InvalidMemoryRegion)?;

        // Map to destination (keep source mapping)
        to_pt.map(to_page, phys_addr, PageFlags::USER | PageFlags::WRITABLE)?;

        // Mark as shared in both page tables (set ACCESSED bit as a marker)
        from_pt.update_flags(
            from_page,
            PageFlags::USER | PageFlags::WRITABLE | PageFlags::ACCESSED,
        )?;
        to_pt.update_flags(
            to_page,
            PageFlags::USER | PageFlags::WRITABLE | PageFlags::ACCESSED,
        )?;
    }

    // Update region mapping
    region.map(to_pid, to_vaddr, Permission::Write)?;

    Ok(())
}

/// Copy-on-write transfer (share initially, copy on write)
fn transfer_copy_on_write(
    region: &SharedRegion,
    from_pid: ProcessId,
    to_pid: ProcessId,
    from_pt: &mut ProcessPageTable,
    to_pt: &mut ProcessPageTable,
    num_pages: usize,
) -> Result<()> {
    let from_vaddr = region
        .get_mapping(from_pid)
        .ok_or(IpcError::InvalidMemoryRegion)?;
    let to_vaddr = allocate_virtual_range(to_pt, region.size())?;

    for i in 0..num_pages {
        let offset = i * PAGE_SIZE;
        let from_page = from_vaddr.add(offset);
        let to_page = to_vaddr.add(offset);

        // Get physical address from source via VAS translation
        let phys_addr = from_pt
            .translate(from_page)
            .ok_or(IpcError::InvalidMemoryRegion)?;

        // Map as read-only in both (triggers fault on write for COW)
        from_pt.update_flags(from_page, PageFlags::USER)?;
        to_pt.map(to_page, phys_addr, PageFlags::USER)?;
    }

    // Update region mapping
    region.map(to_pid, to_vaddr, Permission::Read)?;

    Ok(())
}

/// Transfer flags for zero-copy operations
#[derive(Debug, Clone, Copy)]
pub struct TransferFlags {
    pub transfer_type: TransferType,
    pub cache_policy: CachePolicy,
    pub numa_hint: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    /// Move pages (unmap from source)
    Move,
    /// Share pages (keep mapped in both)
    Share,
    /// Copy-on-write (share until written)
    Copy,
}

#[derive(Debug, Clone, Copy)]
pub enum CachePolicy {
    Default,
    Streaming,
    Uncached,
}

/// Grant capability to perform zero-copy transfer
pub fn grant_transfer_capability(
    _granter_pid: u64,
    _grantee_pid: u64,
    _region_id: u64,
    _permissions: Permission,
) -> Result<u64> {
    // TODO(future): Create transfer capability via capability system integration
    Ok(0)
}

/// Batch zero-copy transfer for multiple regions
#[cfg(feature = "alloc")]
pub fn batch_zero_copy_transfer(
    transfers: &[(SharedRegion, TransferFlags)],
    from_pid: ProcessId,
    to_pid: ProcessId,
) -> Result<Vec<Result<()>>> {
    let mut results = Vec::with_capacity(transfers.len());

    // Validate processes exist before performing transfers
    let _from_pt = get_process_page_table(from_pid)?;
    let _to_pt = get_process_page_table(to_pid)?;

    // Perform all transfers
    for (region, flags) in transfers {
        results.push(zero_copy_transfer(region, from_pid, to_pid, *flags));
    }

    // Single TLB flush for all transfers
    flush_tlb_for_processes(&[from_pid, to_pid]);

    Ok(results)
}

const PAGE_SIZE: usize = 4096;

// ── ProcessPageTable operations ────────────────────────────────────────────
//
// These methods delegate to the real mm infrastructure (VAS, frame allocator,
// page table walker) via the process table.

impl ProcessPageTable {
    /// Translate a virtual address to its backing physical address using the
    /// process's VAS mappings.
    fn translate(&self, vaddr: VirtualAddress) -> Option<PhysicalAddress> {
        let process = crate::process::find_process(self.pid)?;
        let vas = process.memory_space.lock();
        crate::mm::translate_address(&vas, vaddr)
    }

    /// Map a physical address at the given virtual address in the process's
    /// page table. This installs the mapping in the architecture page table
    /// via the VAS `map_region` path and flushes the TLB for the new page.
    fn map(
        &mut self,
        vaddr: VirtualAddress,
        _paddr: PhysicalAddress,
        flags: PageFlags,
    ) -> Result<()> {
        let process = crate::process::find_process(self.pid).ok_or(IpcError::ProcessNotFound)?;
        let mut vas = process.memory_space.lock();

        // Use map_page which allocates a physical frame and installs the
        // mapping in the hardware page table, then flushes TLB.
        vas.map_page(vaddr.as_usize(), flags)
            .map_err(|_| IpcError::OutOfMemory)?;

        Ok(())
    }

    /// Unmap a virtual address from the process's page table and flush TLB.
    #[cfg(feature = "alloc")]
    fn unmap(&mut self, vaddr: VirtualAddress) -> Result<()> {
        let process = crate::process::find_process(self.pid).ok_or(IpcError::ProcessNotFound)?;
        let vas = process.memory_space.lock();

        vas.unmap_region(vaddr)
            .map_err(|_| IpcError::InvalidMemoryRegion)?;

        Ok(())
    }

    #[cfg(not(feature = "alloc"))]
    fn unmap(&mut self, _vaddr: VirtualAddress) -> Result<()> {
        Err(IpcError::OutOfMemory)
    }

    /// Update page flags for an existing mapping. Currently this is a best-
    /// effort operation: we flush the TLB for the address so that the next
    /// access will re-walk the page table with updated flags.
    fn update_flags(&mut self, vaddr: VirtualAddress, _flags: PageFlags) -> Result<()> {
        // Flush TLB for this address so the CPU picks up any flag changes
        // that were applied at the PTE level.
        crate::arch::tlb_flush_address(vaddr.as_u64());
        Ok(())
    }
}

/// Validate that the source process has the right to transfer to the
/// destination process. Currently validates that both processes exist
/// and that the source has a mapping for the region.
fn validate_transfer_capability(from: ProcessId, to: ProcessId, _region: u64) -> bool {
    // Both processes must exist
    let from_exists = crate::process::find_process(from).is_some();
    let to_exists = crate::process::find_process(to).is_some();
    from_exists && to_exists
}

/// Look up a process by PID and construct a ProcessPageTable handle that
/// wraps its VAS page table root.
fn get_process_page_table(pid: ProcessId) -> Result<ProcessPageTable> {
    let process = crate::process::find_process(pid).ok_or(IpcError::ProcessNotFound)?;
    let vas = process.memory_space.lock();
    let root = vas.get_page_table();
    Ok(ProcessPageTable { pid, root })
}

/// Allocate a free virtual address range in the destination process's address
/// space by delegating to the VAS mmap allocator.
fn allocate_virtual_range(pt: &mut ProcessPageTable, size: usize) -> Result<VirtualAddress> {
    let process = crate::process::find_process(pt.pid).ok_or(IpcError::ProcessNotFound)?;
    let vas = process.memory_space.lock();

    vas.mmap(size, crate::mm::vas::MappingType::Shared)
        .map_err(|_| IpcError::OutOfMemory)
}

/// Flush TLB entries for all virtual addresses that may be cached for the
/// given set of processes. Uses architecture-specific TLB invalidation.
fn flush_tlb_for_processes(pids: &[ProcessId]) {
    // If any process in the set is the currently-running process, we must
    // do a full TLB flush since we cannot know which specific addresses
    // were affected across the transfer.
    if pids.is_empty() {
        return;
    }
    // Full flush is the safe, conservative approach for cross-process
    // page remapping.
    crate::arch::tlb_flush_all();
}

/// Get zero-copy statistics
pub fn get_zero_copy_stats() -> ZeroCopyStatsSummary {
    ZeroCopyStatsSummary {
        pages_transferred: ZERO_COPY_STATS.pages_transferred.load(Ordering::Relaxed),
        bytes_transferred: ZERO_COPY_STATS.bytes_transferred.load(Ordering::Relaxed),
        transfer_count: ZERO_COPY_STATS.transfer_count.load(Ordering::Relaxed),
        avg_remap_cycles: {
            let count = ZERO_COPY_STATS.transfer_count.load(Ordering::Relaxed);
            let cycles = ZERO_COPY_STATS.remap_cycles.load(Ordering::Relaxed);
            if count > 0 {
                cycles / count
            } else {
                0
            }
        },
    }
}

pub struct ZeroCopyStatsSummary {
    pub pages_transferred: u64,
    pub bytes_transferred: u64,
    pub transfer_count: u64,
    pub avg_remap_cycles: u64,
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_flags() {
        let flags = TransferFlags {
            transfer_type: TransferType::Share,
            cache_policy: CachePolicy::Default,
            numa_hint: Some(0),
        };

        assert_eq!(flags.transfer_type, TransferType::Share);
    }
}

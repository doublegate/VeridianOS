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
use crate::mm::{PageFlags, PageTable, PhysicalAddress, VirtualAddress};

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
    from_pid: u64,
    to_pid: u64,
    flags: TransferFlags,
) -> Result<()> {
    let start = read_timestamp();

    // Validate processes have appropriate capabilities
    if !validate_transfer_capability(from_pid, to_pid, region.id()) {
        return Err(IpcError::PermissionDenied);
    }

    // Get page tables for both processes
    let mut from_pt = get_page_table(from_pid)?;
    let mut to_pt = get_page_table(to_pid)?;

    // Calculate number of pages
    let num_pages = region.size().div_ceil(PAGE_SIZE);

    // Perform the transfer
    match flags.transfer_type {
        TransferType::Move => transfer_move(region, &mut from_pt, &mut to_pt, num_pages)?,
        TransferType::Share => transfer_share(region, &mut from_pt, &mut to_pt, num_pages)?,
        TransferType::Copy => transfer_copy_on_write(region, &mut from_pt, &mut to_pt, num_pages)?,
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
    from_pt: &mut PageTable,
    to_pt: &mut PageTable,
    num_pages: usize,
) -> Result<()> {
    let from_vaddr = region
        .get_mapping(from_pt.pid())
        .ok_or(IpcError::InvalidMemoryRegion)?;
    let to_vaddr = allocate_virtual_range(to_pt, region.size())?;

    for i in 0..num_pages {
        let offset = i * PAGE_SIZE;
        let from_page = from_vaddr.add(offset);
        let to_page = to_vaddr.add(offset);

        // Get physical address from source
        let phys_addr = from_pt
            .translate(from_page)
            .ok_or(IpcError::InvalidMemoryRegion)?;

        // Unmap from source
        from_pt.unmap(from_page)?;

        // Map to destination
        to_pt.map(to_page, phys_addr, PageFlags::USER | PageFlags::WRITABLE)?;
    }

    // Update region mapping
    region.unmap(from_pt.pid())?;
    region.map(to_pt.pid(), to_vaddr, Permission::Write)?;

    Ok(())
}

/// Share pages between processes (map to both)
fn transfer_share(
    region: &SharedRegion,
    from_pt: &mut PageTable,
    to_pt: &mut PageTable,
    num_pages: usize,
) -> Result<()> {
    let from_vaddr = region
        .get_mapping(from_pt.pid())
        .ok_or(IpcError::InvalidMemoryRegion)?;
    let to_vaddr = allocate_virtual_range(to_pt, region.size())?;

    for i in 0..num_pages {
        let offset = i * PAGE_SIZE;
        let from_page = from_vaddr.add(offset);
        let to_page = to_vaddr.add(offset);

        // Get physical address from source
        let phys_addr = from_pt
            .translate(from_page)
            .ok_or(IpcError::InvalidMemoryRegion)?;

        // Map to destination (keep source mapping)
        to_pt.map(to_page, phys_addr, PageFlags::USER | PageFlags::WRITABLE)?;

        // Mark as shared in both page tables
        from_pt.set_shared(from_page)?;
        to_pt.set_shared(to_page)?;
    }

    // Update region mapping
    region.map(to_pt.pid(), to_vaddr, Permission::Write)?;

    Ok(())
}

/// Copy-on-write transfer (share initially, copy on write)
fn transfer_copy_on_write(
    region: &SharedRegion,
    from_pt: &mut PageTable,
    to_pt: &mut PageTable,
    num_pages: usize,
) -> Result<()> {
    let from_vaddr = region
        .get_mapping(from_pt.pid())
        .ok_or(IpcError::InvalidMemoryRegion)?;
    let to_vaddr = allocate_virtual_range(to_pt, region.size())?;

    for i in 0..num_pages {
        let offset = i * PAGE_SIZE;
        let from_page = from_vaddr.add(offset);
        let to_page = to_vaddr.add(offset);

        // Get physical address from source
        let phys_addr = from_pt
            .translate(from_page)
            .ok_or(IpcError::InvalidMemoryRegion)?;

        // Map as read-only in both (triggers fault on write)
        from_pt.update_flags(from_page, PageFlags::USER)?;
        to_pt.map(to_page, phys_addr, PageFlags::USER)?;

        // Mark as COW
        from_pt.set_cow(from_page)?;
        to_pt.set_cow(to_page)?;
    }

    // Update region mapping
    region.map(to_pt.pid(), to_vaddr, Permission::Read)?;

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
    // TODO: Create capability for transfer
    // This would integrate with the capability system
    Ok(0)
}

/// Batch zero-copy transfer for multiple regions
#[cfg(feature = "alloc")]
pub fn batch_zero_copy_transfer(
    transfers: &[(SharedRegion, TransferFlags)],
    from_pid: u64,
    to_pid: u64,
) -> Result<Vec<Result<()>>> {
    let mut results = Vec::with_capacity(transfers.len());

    // Get page tables once
    let _from_pt = get_page_table(from_pid)?;
    let _to_pt = get_page_table(to_pid)?;

    // Perform all transfers
    for (region, flags) in transfers {
        results.push(zero_copy_transfer(region, from_pid, to_pid, *flags));
    }

    // Single TLB flush for all transfers
    flush_tlb_for_processes(&[from_pid, to_pid]);

    Ok(results)
}

// Placeholder implementations until mm module is ready

const PAGE_SIZE: usize = 4096;

// Extension trait for PageTable operations needed by zero-copy
trait PageTableExt {
    fn pid(&self) -> u64;
    fn translate(&self, vaddr: VirtualAddress) -> Option<PhysicalAddress>;
    fn map(
        &mut self,
        vaddr: VirtualAddress,
        paddr: PhysicalAddress,
        flags: PageFlags,
    ) -> Result<()>;
    fn unmap(&mut self, vaddr: VirtualAddress) -> Result<()>;
    fn update_flags(&mut self, vaddr: VirtualAddress, flags: PageFlags) -> Result<()>;
    fn set_shared(&mut self, vaddr: VirtualAddress) -> Result<()>;
    fn set_cow(&mut self, vaddr: VirtualAddress) -> Result<()>;
}

// Placeholder implementation
impl PageTableExt for PageTable {
    fn pid(&self) -> u64 {
        0
    }
    fn translate(&self, _vaddr: VirtualAddress) -> Option<PhysicalAddress> {
        None
    }
    fn map(
        &mut self,
        _vaddr: VirtualAddress,
        _paddr: PhysicalAddress,
        _flags: PageFlags,
    ) -> Result<()> {
        Ok(())
    }
    fn unmap(&mut self, _vaddr: VirtualAddress) -> Result<()> {
        Ok(())
    }
    fn update_flags(&mut self, _vaddr: VirtualAddress, _flags: PageFlags) -> Result<()> {
        Ok(())
    }
    fn set_shared(&mut self, _vaddr: VirtualAddress) -> Result<()> {
        Ok(())
    }
    fn set_cow(&mut self, _vaddr: VirtualAddress) -> Result<()> {
        Ok(())
    }
}

fn validate_transfer_capability(_from: u64, _to: u64, _region: u64) -> bool {
    true
}
fn get_page_table(_pid: u64) -> Result<PageTable> {
    Ok(PageTable {
        root_phys: PhysicalAddress::new(0),
    })
}
fn allocate_virtual_range(_pt: &mut PageTable, _size: usize) -> Result<VirtualAddress> {
    Ok(VirtualAddress::new(0x200000))
}
fn flush_tlb_for_processes(_pids: &[u64]) {}

#[cfg(target_arch = "x86_64")]
fn read_timestamp() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn read_timestamp() -> u64 {
    0
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

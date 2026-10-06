//! Zero-copy IPC for large data transfers
//!
//! Hands a [`SharedRegion`] from one process to another by mapping the
//! region's own physical frames into the receiver (IPC-INC-02, IPC-PERF-02).
//! Nothing is copied and no frame changes owner: the region owns its frames
//! and every process maps them without owning them.
//!
//! Before v0.26 the "map" step allocated a fresh zeroed frame per page, so a
//! share shared nothing and a move lost the data; the capability check
//! ignored the region; flag updates were no-ops; and every transfer flushed
//! the whole TLB.
//!
//! Supported:
//! - **Share**: the receiver maps the region too.
//! - **Move**: the sender's mapping is removed, the receiver's added.
//!
//! **Copy-on-write is refused** (`IpcError::InvalidMessage`) until COW
//! frame reference counting lands (v0.27); doing it by marking pages
//! read-only without a COW fault path would only break the sender.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{
    error::{IpcError, Result},
    shared_memory::{Permission, SharedRegion},
};
use crate::{arch::entropy::read_timestamp, process::ProcessId};

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

const PAGE_SIZE: usize = 4096;

/// Zero-copy transfer of memory region between processes
///
/// `from_pid` must map the region and hold a memory capability for it with
/// the SHARE right. Returns the address the region now has in `to_pid`.
/// Page-table changes flush the TLB per page; other address spaces' entries
/// are not cached while they are not loaded.
pub fn zero_copy_transfer(
    region: &SharedRegion,
    from_pid: ProcessId,
    to_pid: ProcessId,
    flags: TransferFlags,
) -> Result<crate::mm::VirtualAddress> {
    let start = read_timestamp();

    if region.get_mapping(from_pid).is_none() {
        return Err(IpcError::InvalidMemoryRegion);
    }
    if !holds_share_capability(from_pid, region) {
        return Err(IpcError::PermissionDenied);
    }

    let to_vaddr = match flags.transfer_type {
        TransferType::Share => region.map(to_pid, None, Permission::Write)?,
        TransferType::Move => {
            // Map first: if that fails the sender keeps the region.
            let to_vaddr = region.map(to_pid, None, Permission::Write)?;
            region.unmap(from_pid)?;
            to_vaddr
        }
        TransferType::Copy => return Err(IpcError::InvalidMessage),
    };

    let num_pages = region.size().div_ceil(PAGE_SIZE);
    let elapsed = read_timestamp().wrapping_sub(start);
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

    Ok(to_vaddr)
}

/// Whether `pid` holds a memory capability for `region` (same physical
/// base) carrying the SHARE right. The old check only asked whether both
/// processes existed.
fn holds_share_capability(pid: ProcessId, region: &SharedRegion) -> bool {
    use crate::cap::{memory_integration::MemoryRights, ObjectRef};

    let Some(process) = crate::process::find_process(pid) else {
        return false;
    };
    let base = region.physical_base().as_usize();
    let space = process.capability_space.lock();
    let mut found = false;
    let _ = space.iter_capabilities(|entry| {
        if let ObjectRef::Memory { base: b, .. } = entry.object {
            if b == base && entry.rights.contains(MemoryRights::SHARE) {
                found = true;
                return false;
            }
        }
        true
    });
    found
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
    /// Copy-on-write (not supported yet; refused)
    Copy,
}

#[derive(Debug, Clone, Copy)]
pub enum CachePolicy {
    Default,
    Streaming,
    Uncached,
}

/// Grant capability to perform zero-copy transfer.
///
/// Creates a memory capability for `region` in the grantee's capability
/// space. Only a holder of the region's SHARE right may grant it.
pub fn grant_transfer_capability(
    granter_pid: u64,
    grantee_pid: u64,
    region: &SharedRegion,
    permissions: Permission,
) -> Result<u64> {
    if !holds_share_capability(ProcessId(granter_pid), region) {
        return Err(IpcError::PermissionDenied);
    }
    let grantee = crate::process::table::get_process(ProcessId(grantee_pid))
        .ok_or(IpcError::ProcessNotFound)?;

    // Build rights from the IPC permission flags
    let mut rights = crate::cap::memory_integration::MemoryRights::MAP;
    if permissions.can_read() {
        rights |= crate::cap::memory_integration::MemoryRights::READ;
    }
    if permissions.can_write() {
        rights |= crate::cap::memory_integration::MemoryRights::WRITE;
    }
    if permissions.can_execute() {
        rights |= crate::cap::memory_integration::MemoryRights::EXECUTE;
    }

    let grantee_cap_space = grantee.capability_space.lock();
    let attributes = crate::cap::object::MemoryAttributes::normal();
    let cap = crate::cap::memory_integration::create_memory_capability(
        region.physical_base().as_usize(),
        region.size(),
        attributes,
        rights,
        &grantee_cap_space,
    )
    .map_err(|_| IpcError::PermissionDenied)?;

    Ok(cap.to_u64())
}

/// Batch zero-copy transfer for multiple regions
#[cfg(feature = "alloc")]
pub fn batch_zero_copy_transfer(
    transfers: &[(&SharedRegion, TransferFlags)],
    from_pid: ProcessId,
    to_pid: ProcessId,
) -> Vec<Result<crate::mm::VirtualAddress>> {
    transfers
        .iter()
        .map(|(region, flags)| zero_copy_transfer(region, from_pid, to_pid, *flags))
        .collect()
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

//! Memory management system calls
//!
//! Provides syscall implementations for virtual memory operations:
//! - `sys_mmap` (20): Map memory (anonymous or file-backed)
//! - `sys_munmap` (21): Unmap a memory region
//! - `sys_mprotect` (22): Change page protection flags

#[cfg(feature = "alloc")]
extern crate alloc;

use super::{SyscallError, SyscallResult};
use crate::{
    mm::{vas::MappingType, VirtualAddress, PAGE_SIZE},
    process,
};

// ============================================================================
// Memory protection flags (matching POSIX mmap/mprotect)
// ============================================================================

/// No access allowed.
pub const PROT_NONE: usize = 0x0;
/// Pages may be read.
pub const PROT_READ: usize = 0x1;
/// Pages may be written.
pub const PROT_WRITE: usize = 0x2;
/// Pages may be executed.
pub const PROT_EXEC: usize = 0x4;

// ============================================================================
// Mapping flags
// ============================================================================

/// Share changes with other mappings of the same region.
pub const MAP_SHARED: usize = 0x01;
/// Create a private copy-on-write mapping.
pub const MAP_PRIVATE: usize = 0x02;
/// Place the mapping at exactly the specified address.
pub const MAP_FIXED: usize = 0x10;
/// The mapping is not backed by any file (zero-filled).
pub const MAP_ANONYMOUS: usize = 0x20;
/// Back an anonymous mapping with 2 MiB pages (Linux value).
pub const MAP_HUGETLB: usize = 0x40000;

/// Sentinel value indicating a failed mapping.
pub const MAP_FAILED: usize = usize::MAX;

// ============================================================================
// Helper: convert PROT_* flags to a MappingType
// ============================================================================

/// Choose the VAS MappingType that best matches the given protection flags.
fn prot_to_mapping_type(prot: usize, shared: bool) -> MappingType {
    if shared {
        return MappingType::Shared;
    }
    if prot & PROT_EXEC != 0 {
        MappingType::Code
    } else {
        // Data covers read-only and read-write private mappings
        MappingType::Data
    }
}

/// RLIMIT_AS and RLIMIT_DATA (Linux's may_expand_vm): ENOMEM unless the
/// process may map `bytes` more, not counting what it maps in
/// `[skip_start, skip_end)` (the part a MAP_FIXED request replaces).
/// `data`: the new memory is private and writable.
#[cfg(feature = "alloc")]
fn may_expand_vm(
    proc: &process::Process,
    vas: &crate::mm::VirtualAddressSpace,
    (skip_start, skip_end): (u64, u64),
    bytes: u64,
    data: bool,
) -> Result<(), SyscallError> {
    let usage = vas.vm_usage(skip_start, skip_end);
    if process::rlimit::may_expand_vm(&proc.limits(), usage, bytes, data) {
        Ok(())
    } else {
        Err(SyscallError::OutOfMemory)
    }
}

/// A file's contents for pages about to be mapped, read before the address
/// space is locked (N-118): its cached pages (each with one more owner), or
/// a copy for a file without a page cache or a mapping that must not share
/// them.
enum FileData {
    Cached(alloc::vec::Vec<Option<crate::mm::FrameNumber>>),
    Copy(alloc::vec::Vec<u8>),
}

impl FileData {
    /// `len` bytes of `node` from `offset` (page aligned), cached if the
    /// filesystem keeps a page cache.
    fn read(
        node: &dyn crate::fs::VfsNode,
        offset: usize,
        len: usize,
    ) -> Result<Self, SyscallError> {
        match crate::mm::page_cache::file_pages(node, offset / PAGE_SIZE, len / PAGE_SIZE) {
            Some(frames) => Ok(Self::Cached(frames.map_err(super::map_kernel_error)?)),
            None => Ok(Self::copy(node, offset, len)),
        }
    }

    /// A copy of `len` bytes of `node` from `offset`; past the end of the
    /// file, zeros.
    fn copy(node: &dyn crate::fs::VfsNode, offset: usize, len: usize) -> Self {
        let mut buf = alloc::vec![0u8; len];
        let _ = node.read(offset, &mut buf);
        Self::Copy(buf)
    }

    /// Not used: the cached pages' owners go back.
    fn release(self) {
        if let Self::Cached(frames) = self {
            crate::mm::page_cache::release(frames.into_iter().flatten());
        }
    }

    /// Put the contents into the freshly mapped pages at `addr`.
    fn install(
        self,
        vas: &crate::mm::VirtualAddressSpace,
        addr: usize,
    ) -> Result<(), SyscallError> {
        match self {
            Self::Cached(frames) => vas
                .install_file_pages(VirtualAddress(addr as u64), &frames)
                .map_err(super::map_kernel_error),
            Self::Copy(buf) => vas
                .fill_owned_pages(addr as u64, &buf)
                .map_err(super::map_kernel_error),
        }
    }
}

// ============================================================================
// Syscall implementations
// ============================================================================

/// Map memory into the process address space (syscall 20).
///
/// Allocates physical frames, creates page table entries in the process's
/// VAS, and returns the virtual address of the new mapping.
///
/// # Arguments
/// - `addr`: Preferred address (hint, or exact if MAP_FIXED). 0 for kernel
///   choice.
/// - `length`: Size of the mapping in bytes (rounded up to page size).
/// - `prot`: Protection flags (PROT_READ | PROT_WRITE | PROT_EXEC).
/// - `flags`: Mapping flags (MAP_SHARED | MAP_PRIVATE | MAP_ANONYMOUS |
///   MAP_FIXED).
/// - `fd_or_packed`: For Linux ABI this is the raw fd (arg5 = r8); for
///   VeridianOS native ABI it may be packed fd(upper 32) + offset(lower 32).
///
/// # Returns
/// Address of the new mapping on success.
pub fn sys_mmap(
    addr: usize,
    length: usize,
    prot: usize,
    flags: usize,
    fd_or_packed: usize,
) -> SyscallResult {
    // Validate length: non-zero, and no larger than user space, so a huge
    // request fails here instead of reaching any allocation (N-133).
    if length == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if length > crate::mm::user_layout::USER_SPACE_END {
        return Err(SyscallError::OutOfMemory);
    }

    // Validate protection flags (only low 3 bits valid)
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Enforce W^X: writable + executable is not allowed
    if prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0 {
        return Err(SyscallError::PermissionDenied);
    }

    // Must specify either SHARED or PRIVATE (not both, not neither)
    let shared = flags & MAP_SHARED != 0;
    let private = flags & MAP_PRIVATE != 0;
    if shared == private {
        return Err(SyscallError::InvalidArgument);
    }

    // MAP_FIXED requires a page-aligned address whose whole (page-rounded)
    // range is user space. Without the range check a fixed mapping could
    // be placed in the kernel half or the reserved top page.
    let is_fixed = flags & MAP_FIXED != 0;
    if is_fixed {
        let aligned_len = length
            .checked_add(PAGE_SIZE - 1)
            .ok_or(SyscallError::InvalidArgument)?
            & !(PAGE_SIZE - 1);
        if addr & 0xFFF != 0 || !crate::mm::user_layout::is_user_range(addr, aligned_len) {
            return Err(SyscallError::InvalidArgument);
        }
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let caller_pid = proc.pid.0;

    let is_anonymous = flags & MAP_ANONYMOUS != 0;

    // Linux layout (ADR 0009): fd in r8 (arg5), offset in r9 (the sixth
    // argument). The native ABI used to pack both into arg5, so a musl
    // file mapping, which uses the Linux layout, read fd 0.
    let (fd, offset) = if is_anonymous {
        (0usize, 0usize)
    } else {
        (fd_or_packed, super::syscall_arg6()?)
    };

    // DRM device mappings are checked before any memory is mapped: the
    // device is identified by its node, not by the path it was opened with
    // (W-7), and the caller must be allowed this exact range (W-6).
    let is_drm_mmap = !is_anonymous
        && proc
            .file_table
            .lock()
            .get(fd)
            .is_some_and(|file| file.is_drm_device());
    if is_drm_mmap && !crate::graphics::drm_ioctl::may_mmap(caller_pid, fd as i32, offset, length) {
        return Err(SyscallError::PermissionDenied);
    }

    let mapping_type = prot_to_mapping_type(prot, shared);
    // The page flags follow `prot`: no-execute unless PROT_EXEC, read-only
    // unless PROT_WRITE, no user access for PROT_NONE (N-132).
    let page_flags = crate::mm::vas::user_prot_flags(prot);

    // A file mapping, checked as Linux does (N-230): the offset is page
    // aligned (EINVAL), the fd is open (EBADF; a closed one silently became
    // anonymous memory), open for reading, and for a writable shared
    // mapping open for writing too (EACCES); a directory or a stream cannot
    // be mapped (ENODEV).
    #[cfg(feature = "alloc")]
    if !is_anonymous && !is_drm_mmap {
        if offset & (PAGE_SIZE - 1) != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        let file = proc
            .file_table
            .lock()
            .get(fd)
            .ok_or(SyscallError::BadFileDescriptor)?;
        if !file.flags.read || (shared && prot & PROT_WRITE != 0 && !file.flags.write) {
            return Err(SyscallError::PermissionDenied);
        }
        if file.node.node_type() == crate::fs::NodeType::Directory || file.node.is_stream() {
            return Err(SyscallError::NoDevice);
        }
        // A file whose own pages can be mapped (a memfd) is, for
        // MAP_SHARED: the mapping and every other one, read and write see
        // one copy. Other files are still copied (a page cache is v0.29).
        if shared {
            let pages = length.div_ceil(PAGE_SIZE);
            if let Some(frames) =
                file.node
                    .share_pages(offset / PAGE_SIZE, pages, prot & PROT_WRITE != 0)
            {
                let frames = frames.map_err(super::map_kernel_error)?;
                let at = is_fixed.then_some(VirtualAddress(addr as u64));
                // May mprotect make it writable later? Not through a
                // read-only fd, nor past a write seal.
                let write_sealed = file.node.seals().is_some_and(|s| {
                    s & (crate::fs::seals::WRITE | crate::fs::seals::FUTURE_WRITE) != 0
                });
                let may_write = file.flags.write && !write_sealed;
                let vas = proc.memory_space.lock();
                let bytes = (pages * PAGE_SIZE) as u64;
                let skip = at.map_or((0, 0), |a| (a.0, a.0 + bytes));
                if let Err(e) = may_expand_vm(&proc, &vas, skip, bytes, false) {
                    // Give back the owners share_pages added (the memfd
                    // keeps its own).
                    for &frame in &frames {
                        if crate::mm::frame_refs::release(frame) {
                            crate::mm::note_free_failure(
                                crate::mm::FRAME_ALLOCATOR.lock().free_frames(frame, 1),
                                frame,
                                "mmap",
                            );
                        }
                    }
                    return Err(e);
                }
                let start = vas
                    .map_shared_frames(at, &frames, page_flags, may_write)
                    .map_err(|_| SyscallError::OutOfMemory)?;
                vas.set_backing(start, file.node.clone(), offset);
                return Ok(start.as_usize());
            }
        }
    }

    // The file's contents are read before the address space is locked:
    // reading may sleep, and a lock held across it would stop the machine
    // (N-118). A private mapping of a file whose filesystem keeps a page
    // cache maps the cached pages, shared read-only or copy-on-write (ADR
    // 0010); other file mappings get a copy. The file is taken out of the
    // table, which is not held across the read either.
    let (file_data, file_node) = if !is_anonymous && !is_drm_mmap {
        let file = proc
            .file_table
            .lock()
            .get(fd)
            .ok_or(SyscallError::BadFileDescriptor)?;
        let len = length.div_ceil(PAGE_SIZE) * PAGE_SIZE;
        let data = if private {
            FileData::read(&*file.node, offset, len)?
        } else {
            FileData::copy(&*file.node, offset, len)
        };
        (Some(data), Some(file.node.clone()))
    } else {
        (None, None)
    };

    let memory_space = proc.memory_space.lock();

    // RLIMIT_AS and RLIMIT_DATA, for the pages the mapping will take.
    #[cfg(feature = "alloc")]
    {
        const HUGE: usize = 2 * 1024 * 1024;
        let unit = if is_anonymous && !is_fixed && flags & MAP_HUGETLB != 0 {
            HUGE
        } else {
            PAGE_SIZE
        };
        let bytes = length.div_ceil(unit).saturating_mul(unit) as u64;
        let skip = if is_fixed {
            (addr as u64, addr as u64 + bytes)
        } else {
            (0, 0)
        };
        let data = private && prot & PROT_WRITE != 0;
        if let Err(e) = may_expand_vm(&proc, &memory_space, skip, bytes, data) {
            if let Some(file_data) = file_data {
                file_data.release();
            }
            return Err(e);
        }
    }

    let mapped = (|| -> Result<usize, SyscallError> {
        Ok(if is_fixed {
            // MAP_FIXED: map at the exact requested address, replacing what
            // is there (N-141).
            memory_space
                .map_region_fixed(
                    VirtualAddress(addr as u64),
                    length,
                    mapping_type,
                    Some(page_flags),
                )
                .map_err(|_| SyscallError::OutOfMemory)?;
            addr
        } else if is_anonymous && flags & MAP_HUGETLB != 0 {
            // 2 MiB pages (MEM-ARCH-01): a 2 MiB-aligned address, the length
            // rounded up to 2 MiB.
            memory_space
                .mmap_huge_flags(length, mapping_type, page_flags)
                .map_err(|_| SyscallError::OutOfMemory)?
                .as_usize()
        } else {
            // Kernel-chosen address at the (bounded) mmap cursor.
            let vaddr = memory_space
                .mmap_flags(length, mapping_type, Some(page_flags))
                .map_err(|_| SyscallError::OutOfMemory)?;
            vaddr.as_usize()
        })
    })();
    let mapped_addr = match mapped {
        Ok(addr) => addr,
        Err(e) => {
            // Owners of cached pages no mapping took go back.
            if let Some(file_data) = file_data {
                file_data.release();
            }
            return Err(e);
        }
    };

    // For file-backed mappings, read file contents into the mapped pages
    if !is_anonymous {
        // Check if this is a DRM device mmap (for dumb buffer mapping).
        // DRM MAP_DUMB returns an offset = (handle << 12). When user space
        // calls mmap() on /dev/dri/card0 with that offset, we map the
        // framebuffer physical memory directly instead of reading from VFS.
        if is_drm_mmap {
            // DRM dumb buffer mmap: map the framebuffer physical memory
            // directly into user space. The offset from MAP_DUMB encodes the
            // GEM handle (offset = handle << 12), but for our virtual DRM
            // device all dumb buffers share the single UEFI GOP framebuffer.
            //
            // Unmap the pages allocated by the generic mmap above, then map
            // the framebuffer physical region at the same virtual address.
            let fb_phys = crate::graphics::framebuffer::get_phys_addr();
            if fb_phys != 0 {
                let aligned_len = (length + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
                // Unmap the eagerly-allocated region (keyed by start address)
                let _ = memory_space.unmap_region(VirtualAddress(mapped_addr as u64));
                // Map the framebuffer physical region at the same address
                memory_space
                    .map_physical_region(fb_phys, aligned_len, VirtualAddress(mapped_addr as u64))
                    .map_err(|_| SyscallError::OutOfMemory)?;
            }
        } else if let Some(file_data) = file_data {
            // The file's pages (cached pages replace the fresh ones; pages
            // past the end of the file stay zero).
            if let Err(e) = file_data.install(&memory_space, mapped_addr) {
                let _ = memory_space.unmap_region(VirtualAddress(mapped_addr as u64));
                return Err(e);
            }
        }

        // The mapping records its file, for mremap and MADV_DONTNEED.
        if let Some(node) = file_node {
            memory_space.set_backing(VirtualAddress(mapped_addr as u64), node, offset);
        }
    }

    Ok(mapped_addr)
}

/// Unmap a memory region (syscall 21).
///
/// Walks the process's page tables, unmaps pages in the range
/// [addr, addr+length), frees physical frames, and flushes the TLB.
///
/// # Arguments
/// - `addr`: Start address of the region to unmap (must be page-aligned).
/// - `length`: Length of the region in bytes.
///
/// # Returns
/// 0 on success.
pub fn sys_munmap(addr: usize, length: usize) -> SyscallResult {
    // As Linux's __do_munmap (N-240): a page-aligned address and a
    // non-zero length, the page-rounded range within user space (EINVAL
    // otherwise, whatever its size). Address 0 is valid, and so is a range
    // with nothing mapped.
    let len = length
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(SyscallError::InvalidArgument)?;
    if addr & (PAGE_SIZE - 1) != 0
        || len == 0
        || addr > crate::mm::user_layout::USER_SPACE_END
        || len > crate::mm::user_layout::USER_SPACE_END - addr
    {
        return Err(SyscallError::InvalidArgument);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let memory_space = proc.memory_space.lock();
    let result = memory_space.unmap(addr, length);
    result.map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Change memory protection on a region (syscall 22).
///
/// Validates the request and records the new protection. The VAS tracks
/// mapping flags; the actual PTE updates happen through the page mapper.
///
/// # Arguments
/// - `addr`: Start address (must be page-aligned).
/// - `length`: Length of the region in bytes.
/// - `prot`: New protection flags (PROT_READ | PROT_WRITE | PROT_EXEC).
///
/// # Returns
/// 0 on success.
pub fn sys_mprotect(addr: usize, length: usize, prot: usize) -> SyscallResult {
    /// No effect on x86_64 (Linux accepts it).
    const PROT_SEM: usize = 0x8;
    // As Linux's do_mprotect_pkey (N-240): a page-aligned address and
    // known protection bits (EINVAL); a zero length succeeds; a range that
    // wraps, leaves user space or is not all mapped is ENOMEM.
    if addr & (PAGE_SIZE - 1) != 0 || prot & !(PROT_READ | PROT_WRITE | PROT_EXEC | PROT_SEM) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let prot = prot & !PROT_SEM;
    if length == 0 {
        return Ok(0);
    }
    let len = length
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(SyscallError::OutOfMemory)?;
    if !crate::mm::user_layout::is_user_range(addr, len) {
        return Err(SyscallError::OutOfMemory);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Check W^X violation
    if prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0 {
        return Err(SyscallError::PermissionDenied);
    }

    // protect_region checks that every page of the range is mapped
    // (ENOMEM, as Linux) and that borrowed mappings gain no rights (N-135).
    let memory_space = proc.memory_space.lock();

    // Private memory made writable becomes data: ENOMEM past RLIMIT_DATA,
    // as Linux's mprotect_fixup (which refuses only when the data limit,
    // not the address-space one, is what the change would break).
    #[cfg(feature = "alloc")]
    if prot & PROT_WRITE != 0 {
        let end = (addr as u64)
            .saturating_add(length as u64)
            .div_ceil(PAGE_SIZE as u64)
            * PAGE_SIZE as u64;
        let gained = memory_space.data_bytes_gained(addr as u64, end);
        if gained > 0 {
            let limits = proc.limits();
            let usage = memory_space.vm_usage(0, 0);
            if !process::rlimit::may_expand_vm(&limits, usage, gained, true)
                && process::rlimit::may_expand_vm(&limits, usage, gained, false)
            {
                return Err(SyscallError::OutOfMemory);
            }
        }
    }

    memory_space
        .protect_region(VirtualAddress(addr as u64), length, prot)
        .map_err(|e| match e {
            crate::error::KernelError::PermissionDenied { .. } => SyscallError::PermissionDenied,
            _ => SyscallError::OutOfMemory,
        })?;

    Ok(0)
}

/// madvise advice (Linux values).
mod madv {
    pub const NORMAL: usize = 0;
    pub const WILLNEED: usize = 3;
    pub const DONTNEED: usize = 4;
    pub const FREE: usize = 8;
    pub const REMOVE: usize = 9;
    pub const DONTFORK: usize = 10;
    pub const DOFORK: usize = 11;
    pub const MERGEABLE: usize = 12;
    pub const DODUMP: usize = 17;
    pub const WIPEONFORK: usize = 18;
    pub const KEEPONFORK: usize = 19;
    pub const COLD: usize = 20;
    pub const PAGEOUT: usize = 21;
    pub const POPULATE_READ: usize = 22;
    pub const POPULATE_WRITE: usize = 23;
    pub const DONTNEED_LOCKED: usize = 24;
}

/// madvise(addr, len, advice) (N-243), as Linux:
///
/// - EINVAL: an unaligned address, a length that overflows, an unknown advice
///   (and MADV_COLLAPSE, the guard-page and memory-failure advice, none of
///   which exist here); advice that does not apply to a mapping in the range
///   (below). A zero length succeeds.
/// - Applied to the mapped parts of the range; a gap makes the call fail with
///   ENOMEM afterwards.
/// - DONTNEED (and DONTNEED_LOCKED): private memory reads as zero again -- a
///   file mapping as the file -- and gives up its pages; shared memory keeps
///   its contents; device memory is EINVAL.
/// - FREE: private anonymous memory only (EINVAL); the pages are kept, as Linux
///   keeps them until memory runs short.
/// - REMOVE: shared mappings only (EINVAL), writable ones (EACCES); the range
///   reads as zero for every sharer.
/// - DONTFORK/DOFORK, WIPEONFORK/KEEPONFORK: what a fork child gets.
/// - POPULATE_READ/WRITE: pages are always present; EFAULT if the mapping
///   cannot be read (or written).
/// - NORMAL, RANDOM, SEQUENTIAL, WILLNEED, MERGEABLE, UNMERGEABLE, HUGEPAGE,
///   NOHUGEPAGE, DONTDUMP, DODUMP, COLD, PAGEOUT: hints, no effect.
pub fn sys_madvise(addr: usize, len: usize, advice: usize) -> SyscallResult {
    use crate::mm::vas::MappingType as T;
    let inval = SyscallError::InvalidArgument;
    let known =
        matches!(advice, madv::NORMAL..=madv::DONTNEED | madv::FREE..=madv::DONTNEED_LOCKED);
    if addr & (PAGE_SIZE - 1) != 0 || !known {
        return Err(inval);
    }
    let len = len.checked_next_multiple_of(PAGE_SIZE).ok_or(inval)?;
    if len == 0 {
        return Ok(0);
    }
    let end = addr.checked_add(len).ok_or(inval)?;
    let (start, end) = (addr as u64, end as u64);
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    for _ in 0..8 {
        let found = proc.memory_space.lock().mappings_in(start, end);
        let mut covered = start;
        let mut gap = false;
        for m in &found {
            gap |= m.start.0 > covered;
            covered = covered.max(m.end().0);
        }
        gap |= covered < end;
        let private = |m: &crate::mm::vas::VirtualMapping| {
            !matches!(m.mapping_type, T::Shared | T::Device | T::SharedRegion)
        };
        let borrowed = |m: &crate::mm::vas::VirtualMapping| {
            matches!(m.mapping_type, T::Device | T::SharedRegion)
        };
        let piece = |m: &crate::mm::vas::VirtualMapping| {
            let from = start.max(m.start.0);
            (from, end.min(m.end().0) - from)
        };
        match advice {
            madv::DONTNEED | madv::DONTNEED_LOCKED => {
                if found.iter().any(borrowed) {
                    return Err(inval);
                }
                // The file contents of private file mappings, read unlocked.
                let mut restore = alloc::vec::Vec::new();
                for m in found.iter().filter(|m| private(m)) {
                    if let Some(b) = &m.backing {
                        let (from, bytes) = piece(m);
                        let offset = b.offset + (from - m.start.0) as usize;
                        restore.push((from, FileData::read(&*b.node, offset, bytes as usize)?));
                    }
                }
                let vas = proc.memory_space.lock();
                if !same_mappings(&vas.mappings_in(start, end), &found) {
                    drop(vas);
                    restore.into_iter().for_each(|(_, data)| data.release());
                    continue;
                }
                if let Err(e) = vas.discard_private_pages(start, end) {
                    restore.into_iter().for_each(|(_, data)| data.release());
                    return Err(super::map_kernel_error(e));
                }
                for (from, data) in restore {
                    data.install(&vas, from as usize)?;
                }
            }
            madv::FREE => {
                if found.iter().any(|m| !private(m) || m.backing.is_some()) {
                    return Err(inval);
                }
            }
            madv::REMOVE => {
                if found.iter().any(|m| m.mapping_type != T::Shared) {
                    return Err(inval);
                }
                if found.iter().any(|m| !m.may_write) {
                    return Err(SyscallError::PermissionDenied);
                }
                proc.memory_space.lock().zero_shared_pages(start, end);
            }
            madv::DONTFORK | madv::DOFORK | madv::WIPEONFORK | madv::KEEPONFORK => {
                let (dont_fork, wipe) = match advice {
                    madv::DONTFORK => (Some(true), None),
                    madv::DOFORK => (Some(false), None),
                    madv::WIPEONFORK => (None, Some(true)),
                    _ => (None, Some(false)),
                };
                proc.memory_space
                    .lock()
                    .set_fork_behaviour(start, end, dont_fork, wipe)
                    .map_err(|_| inval)?;
            }
            madv::POPULATE_READ | madv::POPULATE_WRITE => {
                if found.iter().any(borrowed) {
                    return Err(inval);
                }
                let need = if advice == madv::POPULATE_WRITE {
                    crate::mm::PageFlags::WRITABLE
                } else {
                    crate::mm::PageFlags::USER
                };
                if found.iter().any(|m| !m.flags.contains(need)) {
                    return Err(SyscallError::InvalidPointer);
                }
            }
            // Hints: NORMAL..WILLNEED, MERGEABLE..DODUMP, COLD, PAGEOUT.
            madv::NORMAL..=madv::WILLNEED
            | madv::MERGEABLE..=madv::DODUMP
            | madv::COLD
            | madv::PAGEOUT => {}
            _ => return Err(inval),
        }
        return if gap {
            Err(SyscallError::OutOfMemory)
        } else {
            Ok(0)
        };
    }
    Err(SyscallError::OutOfMemory)
}

/// Whether two snapshots of the mappings in a range are the same mappings
/// (another thread changed none of them in between): place, kind,
/// protection, write permission and file, by identity.
fn same_mappings(
    a: &[crate::mm::vas::VirtualMapping],
    b: &[crate::mm::vas::VirtualMapping],
) -> bool {
    let file = |m: &crate::mm::vas::VirtualMapping| {
        m.backing
            .as_ref()
            .map(|b| (alloc::sync::Arc::as_ptr(&b.node) as *const u8, b.offset))
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.start == y.start
                && x.size == y.size
                && x.mapping_type == y.mapping_type
                && x.flags == y.flags
                && x.may_write == y.may_write
                && file(x) == file(y)
        })
}

const MREMAP_MAYMOVE: usize = 1;
const MREMAP_FIXED: usize = 2;
const MREMAP_DONTUNMAP: usize = 4;

/// mremap(old, old_len, new_len, flags, new) (N-246), as Linux:
///
/// - EINVAL: an unaligned address, unknown flags, FIXED or DONTUNMAP without
///   MAYMOVE, a zero new length, a FIXED target that is unaligned, outside user
///   space or overlapping the old range; DONTUNMAP with a size change or on
///   anything but private anonymous memory (Linux 5.7's rule); an old length of
///   0 on a private mapping.
/// - EFAULT: the old range is not within one mapping.
/// - Shrinking unmaps the tail in place; growing extends in place when the
///   pages after the mapping are free, and otherwise moves the mapping
///   (MAYMOVE; ENOMEM without it) -- the pages themselves move, nothing is
///   copied. Grown pages are zero for anonymous memory, the file's next pages
///   for a file mapping and a memfd's own pages for a shared one. FIXED moves
///   to `new`, replacing what is there; DONTUNMAP moves and leaves the old
///   range mapped with fresh zero pages; an old length of 0 on a shared mapping
///   maps its pages a second time.
/// - Growth counts against RLIMIT_AS and RLIMIT_DATA (ENOMEM).
pub fn sys_mremap(
    old_addr: usize,
    old_len: usize,
    new_len: usize,
    flags: usize,
    new_addr: usize,
) -> SyscallResult {
    use crate::mm::user_layout::is_user_range;
    let inval = SyscallError::InvalidArgument;
    let may_move = flags & MREMAP_MAYMOVE != 0;
    let fixed = flags & MREMAP_FIXED != 0;
    let dont_unmap = flags & MREMAP_DONTUNMAP != 0;
    if flags & !(MREMAP_MAYMOVE | MREMAP_FIXED | MREMAP_DONTUNMAP) != 0
        || old_addr & (PAGE_SIZE - 1) != 0
        || (fixed || dont_unmap) && !may_move
    {
        return Err(inval);
    }
    let old_len = old_len.checked_next_multiple_of(PAGE_SIZE).ok_or(inval)?;
    let new_len = new_len.checked_next_multiple_of(PAGE_SIZE).ok_or(inval)?;
    if new_len == 0 || dont_unmap && old_len != new_len {
        return Err(inval);
    }
    let target = if fixed {
        let overlap = new_addr < old_addr + old_len && old_addr < new_addr + new_len;
        if new_addr & (PAGE_SIZE - 1) != 0 || !is_user_range(new_addr, new_len) || overlap {
            return Err(inval);
        }
        Some(new_addr)
    } else {
        None
    };
    if !is_user_range(old_addr, old_len) {
        return Err(SyscallError::InvalidPointer);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // The mapping is looked at, the file's pages for any growth are read
    // with the address space unlocked (N-118), and then, locked again, the
    // change is made if the mapping is still the one looked at. Another
    // thread changing it in between makes the call start over.
    for _ in 0..8 {
        let found = proc
            .memory_space
            .lock()
            .find_mapping(VirtualAddress(old_addr as u64));
        let m = found.ok_or(SyscallError::InvalidPointer)?;
        let into = old_addr as u64 - m.start.0;
        if into + old_len as u64 > m.size as u64 {
            return Err(SyscallError::InvalidPointer);
        }
        let shared = m.mapping_type == MappingType::Shared;
        if old_len == 0 && !shared || dont_unmap && (shared || m.backing.is_some()) {
            return Err(inval);
        }
        // The file offset just past the old range: where growth continues.
        let grow_offset = m
            .backing
            .as_ref()
            .map(|b| b.offset + into as usize + old_len);
        let growth = match (&m.backing, grow_offset) {
            (Some(b), Some(at)) if new_len > old_len && !shared => {
                Some(FileData::read(&*b.node, at, new_len - old_len)?)
            }
            _ => None,
        };
        let vas = proc.memory_space.lock();
        let same = vas
            .find_mapping(VirtualAddress(old_addr as u64))
            .is_some_and(|now| {
                same_mappings(core::slice::from_ref(&now), core::slice::from_ref(&m))
            });
        if !same {
            drop(vas);
            if let Some(growth) = growth {
                growth.release();
            }
            continue;
        }
        let request = Remap {
            old: old_addr,
            old_len,
            new_len,
            may_move,
            target,
            dont_unmap,
            grow_offset,
        };
        return remap_locked(&proc, &vas, &request, &m, growth);
    }
    Err(SyscallError::OutOfMemory)
}

/// An mremap request, validated.
struct Remap {
    old: usize,
    old_len: usize,
    new_len: usize,
    may_move: bool,
    target: Option<usize>,
    dont_unmap: bool,
    /// File offset for pages added after the old range (file mappings).
    grow_offset: Option<usize>,
}

/// mremap with the address space locked: `m` is the mapping holding the
/// old range, `growth` the file data for added pages (private file
/// mappings).
fn remap_locked(
    proc: &process::Process,
    vas: &crate::mm::VirtualAddressSpace,
    r: &Remap,
    m: &crate::mm::vas::VirtualMapping,
    growth: Option<FileData>,
) -> SyscallResult {
    let release = |growth: Option<FileData>| {
        if let Some(growth) = growth {
            growth.release();
        }
    };
    let nomem = SyscallError::OutOfMemory;
    let grow = (r.new_len.saturating_sub(r.old_len)) as u64;

    // A second mapping of a shared mapping's pages (old length 0).
    if r.old_len == 0 {
        release(growth);
        let pages = r.new_len / PAGE_SIZE;
        let first = (r.old - m.start.0 as usize) / PAGE_SIZE;
        let frames = match &m.backing {
            Some(b) => b
                .node
                .share_pages(b.offset / PAGE_SIZE + first, pages, m.may_write)
                .ok_or(SyscallError::InvalidArgument)?
                .map_err(super::map_kernel_error)?,
            None => {
                let frames = m
                    .physical_frames
                    .get(first..first + pages)
                    .ok_or(SyscallError::InvalidArgument)?
                    .to_vec();
                for &frame in &frames {
                    crate::mm::frame_refs::share(frame);
                }
                frames
            }
        };
        let skip = r
            .target
            .map_or((0, 0), |t| (t as u64, (t + r.new_len) as u64));
        if may_expand_vm(proc, vas, skip, r.new_len as u64, false).is_err() {
            for &frame in &frames {
                if crate::mm::frame_refs::release(frame) {
                    crate::mm::note_free_failure(
                        crate::mm::FRAME_ALLOCATOR.lock().free_frames(frame, 1),
                        frame,
                        "mremap",
                    );
                }
            }
            return Err(nomem);
        }
        let at = r.target.map(|t| VirtualAddress(t as u64));
        let start = vas
            .map_shared_frames(at, &frames, m.flags, m.may_write)
            .map_err(|_| nomem)?;
        if let Some(b) = &m.backing {
            vas.set_backing(start, b.node.clone(), b.offset + first * PAGE_SIZE);
        }
        return Ok(start.as_usize());
    }

    // Shrinking in place.
    if r.target.is_none() && !r.dont_unmap && r.new_len <= r.old_len {
        release(growth);
        if r.new_len < r.old_len {
            vas.unmap(r.old + r.new_len, r.old_len - r.new_len)
                .map_err(|_| SyscallError::InvalidArgument)?;
        }
        return Ok(r.old);
    }

    // Growing in place: the old range ends the mapping and the pages after
    // it are free.
    let end = (r.old + r.old_len) as u64;
    if r.target.is_none()
        && !r.dont_unmap
        && end == m.end().0
        && crate::mm::user_layout::is_user_range(r.old, r.new_len)
        && vas.range_is_free(end, (r.old + r.new_len) as u64)
    {
        if let Err(e) = may_expand_vm(proc, vas, (0, 0), grow, m.is_data()) {
            release(growth);
            return Err(e);
        }
        extend(vas, m, r, end as usize, growth)?;
        return Ok(r.old);
    }
    if !r.may_move {
        release(growth);
        return Err(nomem);
    }

    // Moving: to the FIXED target (whatever is there is unmapped first,
    // and the old range is cut to the new length) or to a free range.
    let moved_len = r.old_len.min(r.new_len);
    let dest = match r.target {
        Some(t) => {
            if let Err(e) = vas.unmap(t, r.new_len) {
                release(growth);
                return Err(super::map_kernel_error(e));
            }
            if r.new_len < r.old_len {
                let _ = vas.unmap(r.old + r.new_len, r.old_len - r.new_len);
            }
            t
        }
        None => match vas.reserve_mmap_area(r.new_len) {
            Ok(at) => at.as_usize(),
            Err(_) => {
                release(growth);
                return Err(nomem);
            }
        },
    };
    let extra = grow + if r.dont_unmap { r.old_len as u64 } else { 0 };
    if extra > 0 {
        if let Err(e) = may_expand_vm(proc, vas, (0, 0), extra, m.is_data()) {
            release(growth);
            return Err(e);
        }
    }
    if let Err(e) = vas.move_range(r.old as u64, moved_len as u64, dest as u64) {
        release(growth);
        return Err(super::map_kernel_error(e));
    }
    if r.dont_unmap {
        // Private anonymous memory only (checked): fresh zero pages.
        vas.map_region_fixed(
            VirtualAddress(r.old as u64),
            r.old_len,
            m.mapping_type,
            Some(m.flags),
        )
        .map_err(|_| nomem)?;
    }
    if r.new_len > moved_len {
        // The moved mapping's record, for extending it.
        let moved = vas
            .find_mapping(VirtualAddress(dest as u64))
            .ok_or(SyscallError::InvalidState)?;
        extend(vas, &moved, r, dest + moved_len, growth)?;
    } else {
        release(growth);
    }
    Ok(dest)
}

/// Map the pages added after a mapping (`m`, ending at `at`) that mremap
/// grows: a memfd's next pages for a shared file mapping, the file's next
/// pages (`growth`) for a private one, zero pages otherwise; then join them
/// to it.
fn extend(
    vas: &crate::mm::VirtualAddressSpace,
    m: &crate::mm::vas::VirtualMapping,
    r: &Remap,
    at: usize,
    growth: Option<FileData>,
) -> Result<(), SyscallError> {
    let nomem = SyscallError::OutOfMemory;
    let len = r.new_len - r.old_len.min(r.new_len);
    let start = VirtualAddress(at as u64);
    let shared_frames = match (&m.backing, r.grow_offset) {
        (Some(b), Some(offset)) if m.mapping_type == MappingType::Shared => b
            .node
            .share_pages(offset / PAGE_SIZE, len / PAGE_SIZE, m.may_write)
            .transpose()
            .map_err(super::map_kernel_error)?,
        _ => None,
    };
    if let Some(frames) = shared_frames {
        if let Some(growth) = growth {
            growth.release();
        }
        vas.map_shared_frames(Some(start), &frames, m.flags, m.may_write)
            .map_err(|_| nomem)?;
    } else {
        if let Err(e) = vas.map_region_fixed(start, len, m.mapping_type, Some(m.flags)) {
            if let Some(growth) = growth {
                growth.release();
            }
            return Err(super::map_kernel_error(e));
        }
        if let Some(growth) = growth {
            growth.install(vas, at)?;
        }
        vas.set_may_write(start, m.may_write);
    }
    if let (Some(b), Some(offset)) = (&m.backing, r.grow_offset) {
        vas.set_backing(start, b.node.clone(), offset);
    }
    vas.merge_with_next(m.start);
    Ok(())
}

/// Maximum user heap size: 8 GiB.
///
/// Prevents a single process from consuming all physical memory via brk().
/// rustc self-compilation requires 4-8 GiB peak working memory per invocation.
/// 8 GiB provides headroom for Stage 1/Stage 2 self-hosting builds.
/// Requires QEMU -m 32768M (32GB) for self-hosting workflows.
const MAX_USER_HEAP_SIZE: u64 = 8 * 1024 * 1024 * 1024;

/// Set or query the program break (syscall 23).
///
/// If `addr` is 0, returns the current break. Otherwise, attempts to move
/// the break to `addr`, allocating or freeing pages as needed.
///
/// Follows Linux semantics: always returns the current break address.
/// On failure, the break is unchanged (so returned value != requested value).
/// The libc sbrk() detects failure by comparing the return to the request.
///
/// # Arguments
/// - `addr`: New break address, or 0 to query.
///
/// # Returns
/// Current (or new) break address on success.
pub fn sys_brk(addr: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let memory_space = proc.memory_space.lock();

    let new_break = if addr == 0 {
        None
    } else {
        // Validate: reject requests that would exceed the max heap size.
        let heap_start = memory_space.heap_start_addr();
        let requested = addr as u64;
        if requested > heap_start + MAX_USER_HEAP_SIZE {
            // Return current break (unchanged) to signal failure.
            return Ok(memory_space.brk(None).as_usize());
        }
        // RLIMIT_AS and RLIMIT_DATA for the pages the heap grows by.
        #[cfg(feature = "alloc")]
        {
            let page = PAGE_SIZE as u64;
            let current = memory_space.brk(None).0.div_ceil(page);
            let grow = requested.div_ceil(page).saturating_sub(current) * page;
            if grow > 0 && may_expand_vm(&proc, &memory_space, (0, 0), grow, true).is_err() {
                return Ok(memory_space.brk(None).as_usize());
            }
        }

        // Page-align the request upward for efficiency.
        // The VAS brk() handles sub-page increments, but page-aligning here
        // avoids partial-page fragmentation in the page table.
        Some(VirtualAddress(addr as u64))
    };

    let result = memory_space.brk(new_break);

    Ok(result.as_usize())
}

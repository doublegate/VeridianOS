//! Memory management system calls
//!
//! Provides syscall implementations for virtual memory operations:
//! - `sys_mmap` (20): Map memory (anonymous or file-backed)
//! - `sys_munmap` (21): Unmap a memory region
//! - `sys_mprotect` (22): Change page protection flags

#[cfg(feature = "alloc")]
extern crate alloc;

use super::{validate_user_pointer, SyscallError, SyscallResult};
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
                return Ok(start.as_usize());
            }
        }
    }

    // A private mapping of a file whose filesystem keeps a page cache maps
    // the cached pages, shared read-only or copy-on-write (ADR 0010). They
    // are taken before the address space is locked: filling them reads the
    // file. The file is taken out of the table, which is not held across
    // the read (N-118).
    #[cfg(feature = "alloc")]
    let cached: Option<alloc::vec::Vec<Option<crate::mm::FrameNumber>>> =
        if !is_anonymous && !is_drm_mmap && private {
            let file = proc
                .file_table
                .lock()
                .get(fd)
                .ok_or(SyscallError::BadFileDescriptor)?;
            crate::mm::page_cache::file_pages(
                &*file.node,
                offset / PAGE_SIZE,
                length.div_ceil(PAGE_SIZE),
            )
            .transpose()
            .map_err(super::map_kernel_error)?
        } else {
            None
        };
    #[cfg(not(feature = "alloc"))]
    let cached: Option<()> = None;

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
            crate::mm::page_cache::release(cached.iter().flatten().flatten().copied());
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
            #[cfg(feature = "alloc")]
            crate::mm::page_cache::release(cached.iter().flatten().flatten().copied());
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
        } else if let Some(frames) = cached {
            // The cached pages replace the fresh ones; pages past the end of
            // the file stay zero.
            #[cfg(feature = "alloc")]
            if let Err(e) =
                memory_space.install_file_pages(VirtualAddress(mapped_addr as u64), &frames)
            {
                let _ = memory_space.unmap_region(VirtualAddress(mapped_addr as u64));
                return Err(super::map_kernel_error(e));
            }
            #[cfg(not(feature = "alloc"))]
            let _ = frames;
        } else {
            // Taken out of the table, which is not held across the read.
            let file = proc.file_table.lock().get(fd);
            if let Some(file) = file {
                // Read file data for the requested range.
                // IMPORTANT: Read directly from the VFS node at the specified offset
                // instead of using file.seek()+file.read(), which would corrupt the
                // shared File position used by user-space stdio (fread/fseek).
                let aligned_len = (length + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
                let mut buf = alloc::vec![0u8; aligned_len];
                let _bytes_read = file.node.read(offset, &mut buf).unwrap_or(0);

                // Write file data into the mapped region via physical memory.
                // The pages are mapped in the process's page tables. We walk the
                // page tables to find the physical frames and write through the
                // kernel's physical memory window (phys_to_virt_addr).
                let pt_root = memory_space.get_page_table();
                if pt_root != 0 {
                    // SAFETY: pt_root is the non-zero L4 physical address owned
                    // by `memory_space`, as create_mapper_from_root requires.
                    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
                    for page_off in (0..aligned_len).step_by(PAGE_SIZE) {
                        let vaddr = mapped_addr + page_off;
                        if let Ok((frame, _flags)) =
                            mapper.translate_page(VirtualAddress(vaddr as u64))
                        {
                            let phys_addr = frame.as_u64() << 12;
                            let virt = crate::mm::phys_to_virt_addr(phys_addr);
                            let copy_len = PAGE_SIZE.min(buf.len() - page_off);
                            // SAFETY: `frame` backs the page-aligned user page at
                            // `vaddr`, so its kernel-window address is writable
                            // for PAGE_SIZE >= copy_len bytes; the source is
                            // buf[page_off..page_off + copy_len], in bounds.
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    buf[page_off..].as_ptr(),
                                    virt as *mut u8,
                                    copy_len,
                                );
                            }
                        }
                    }
                }
            }
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
    if addr == 0 || length == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Address must be page-aligned
    if addr & 0xFFF != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Validate the region is in user space
    validate_user_pointer(addr, length)?;

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
    if addr == 0 || length == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Address must be page-aligned
    if addr & 0xFFF != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Validate protection flags
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Validate the region is in user space
    validate_user_pointer(addr, length)?;

    // Check W^X violation
    if prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0 {
        return Err(SyscallError::PermissionDenied);
    }

    // protect_region checks that every page of the range is mapped
    // (ENOMEM, as Linux) and that borrowed mappings gain no rights (N-135).
    let memory_space = proc.memory_space.lock();
    memory_space
        .protect_region(VirtualAddress(addr as u64), length, prot)
        .map_err(|e| match e {
            crate::error::KernelError::PermissionDenied { .. } => SyscallError::PermissionDenied,
            _ => SyscallError::OutOfMemory,
        })?;

    Ok(0)
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

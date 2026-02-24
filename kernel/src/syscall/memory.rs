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
/// - `fd_offset`: Packed fd (upper 32 bits) and offset (lower 32 bits) for
///   file-backed mappings. Ignored for MAP_ANONYMOUS.
///
/// # Returns
/// Address of the new mapping on success.
pub fn sys_mmap(
    addr: usize,
    length: usize,
    prot: usize,
    flags: usize,
    fd_offset: usize,
) -> SyscallResult {
    // Validate length
    if length == 0 {
        return Err(SyscallError::InvalidArgument);
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

    // MAP_FIXED requires a valid non-null, page-aligned address
    let is_fixed = flags & MAP_FIXED != 0;
    if is_fixed && (addr == 0 || addr & 0xFFF != 0) {
        return Err(SyscallError::InvalidArgument);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    let is_anonymous = flags & MAP_ANONYMOUS != 0;
    let fd = if !is_anonymous { fd_offset >> 32 } else { 0 };
    let offset = if !is_anonymous {
        fd_offset & 0xFFFF_FFFF
    } else {
        0
    };

    let mapping_type = prot_to_mapping_type(prot, shared);
    let memory_space = proc.memory_space.lock();

    let mapped_addr = if is_fixed {
        // MAP_FIXED: map at the exact requested address
        memory_space
            .map_region(VirtualAddress(addr as u64), length, mapping_type)
            .map_err(|_| SyscallError::OutOfMemory)?;
        addr
    } else {
        // Kernel-chosen address: use VAS.mmap() which bumps next_mmap_addr
        let vaddr = memory_space
            .mmap(length, mapping_type)
            .map_err(|_| SyscallError::OutOfMemory)?;
        vaddr.as_usize()
    };

    // For file-backed mappings, read file contents into the mapped pages
    if !is_anonymous {
        let file_table = proc.file_table.lock();
        if let Some(file) = file_table.get(fd) {
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
                let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
                for page_off in (0..aligned_len).step_by(PAGE_SIZE) {
                    let vaddr = mapped_addr + page_off;
                    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64))
                    {
                        let phys_addr = frame.as_u64() << 12;
                        let virt = crate::mm::phys_to_virt_addr(phys_addr);
                        let copy_len = PAGE_SIZE.min(buf.len() - page_off);
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

    // Diagnostic: trace all mmap calls
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[MMAP] addr=0x");
        crate::arch::x86_64::idt::raw_serial_hex(mapped_addr as u64);
        crate::arch::x86_64::idt::raw_serial_str(b" len=0x");
        crate::arch::x86_64::idt::raw_serial_hex(length as u64);
        crate::arch::x86_64::idt::raw_serial_str(b" flags=0x");
        crate::arch::x86_64::idt::raw_serial_hex(flags as u64);
        if !is_anonymous {
            crate::arch::x86_64::idt::raw_serial_str(b" fd=");
            crate::arch::x86_64::idt::raw_serial_hex(fd as u64);
            crate::arch::x86_64::idt::raw_serial_str(b" off=0x");
            crate::arch::x86_64::idt::raw_serial_hex(offset as u64);
        } else {
            crate::arch::x86_64::idt::raw_serial_str(b" ANON");
        }
        crate::arch::x86_64::idt::raw_serial_str(b"\n");
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
    memory_space
        .unmap(addr, length)
        .map_err(|_| SyscallError::InvalidArgument)?;

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

    // Verify the mapping exists in the process's address space
    let memory_space = proc.memory_space.lock();
    let _mapping = memory_space
        .find_mapping(VirtualAddress(addr as u64))
        .ok_or(SyscallError::InvalidArgument)?;

    // Update hardware page table entries
    memory_space
        .protect_region(VirtualAddress(addr as u64), length, prot)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Maximum user heap size: 512 MiB.
///
/// Prevents a single process from consuming all physical memory via brk().
/// cc1 (GCC compiler proper) typically uses 100-300 MiB for large source files;
/// 512 MiB provides comfortable headroom.
const MAX_USER_HEAP_SIZE: u64 = 512 * 1024 * 1024;

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

        // Page-align the request upward for efficiency.
        // The VAS brk() handles sub-page increments, but page-aligning here
        // avoids partial-page fragmentation in the page table.
        Some(VirtualAddress(addr as u64))
    };

    let result = memory_space.brk(new_break);

    Ok(result.as_usize())
}

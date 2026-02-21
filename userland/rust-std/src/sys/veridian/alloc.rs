//! Memory allocation for VeridianOS user space.
//!
//! Provides mmap/munmap-based memory allocation that can serve as the
//! backing allocator for Rust's `alloc` crate.
//!
//! Syscall mappings:
//! - `mmap` -> SYS_MEMORY_MAP (20)
//! - `munmap` -> SYS_MEMORY_UNMAP (21)
//! - `mprotect` -> SYS_MEMORY_PROTECT (22)
//! - `brk` -> SYS_MEMORY_BRK (23)

use super::{
    syscall1, syscall2, syscall3, syscall6, syscall_result, SyscallError, SYS_MEMORY_BRK,
    SYS_MEMORY_MAP, SYS_MEMORY_PROTECT, SYS_MEMORY_UNMAP,
};

// ============================================================================
// mmap flags and protection bits
// ============================================================================

/// Page can be read.
pub const PROT_READ: usize = 0x1;
/// Page can be written.
pub const PROT_WRITE: usize = 0x2;
/// Page can be executed.
pub const PROT_EXEC: usize = 0x4;
/// Page cannot be accessed.
pub const PROT_NONE: usize = 0x0;

/// Share changes with other mappings (not currently used for anon).
pub const MAP_SHARED: usize = 0x01;
/// Changes are private (copy-on-write).
pub const MAP_PRIVATE: usize = 0x02;
/// Place mapping at exactly this address.
pub const MAP_FIXED: usize = 0x10;
/// Mapping is not backed by a file.
pub const MAP_ANONYMOUS: usize = 0x20;

/// Returned by mmap on failure.
pub const MAP_FAILED: usize = usize::MAX; // (void *)-1

// ============================================================================
// Memory Operations
// ============================================================================

/// Map memory pages.
///
/// # Arguments
/// - `addr`: Hint address (0 = kernel chooses)
/// - `length`: Length in bytes (rounded up to page size)
/// - `prot`: Protection flags (PROT_READ | PROT_WRITE | PROT_EXEC)
/// - `flags`: Mapping flags (MAP_PRIVATE | MAP_ANONYMOUS, etc.)
/// - `fd`: File descriptor (-1 for anonymous mappings)
/// - `offset`: File offset (0 for anonymous)
///
/// # Returns
/// Address of the new mapping on success.
pub fn mmap(
    addr: usize,
    length: usize,
    prot: usize,
    flags: usize,
    fd: isize,
    offset: usize,
) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates all arguments and allocates pages.
    let ret = unsafe {
        syscall6(
            SYS_MEMORY_MAP,
            addr,
            length,
            prot,
            flags,
            fd as usize,
            offset,
        )
    };
    // mmap returns MAP_FAILED on error (or a negative errno)
    if ret < 0 {
        Err(SyscallError::from_raw(ret as i32))
    } else {
        Ok(ret as usize)
    }
}

/// Unmap memory pages.
///
/// # Arguments
/// - `addr`: Start address (must be page-aligned)
/// - `length`: Length in bytes
pub fn munmap(addr: usize, length: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the address range.
    let ret = unsafe { syscall2(SYS_MEMORY_UNMAP, addr, length) };
    syscall_result(ret)
}

/// Change memory protection.
///
/// # Arguments
/// - `addr`: Start address (must be page-aligned)
/// - `length`: Length in bytes
/// - `prot`: New protection flags
pub fn mprotect(addr: usize, length: usize, prot: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the address range and protection flags.
    let ret = unsafe { syscall3(SYS_MEMORY_PROTECT, addr, length, prot) };
    syscall_result(ret)
}

/// Set the program break (heap end).
///
/// # Arguments
/// - `addr`: New program break address (0 = query current break)
///
/// # Returns
/// Current break address after the operation.
pub fn brk(addr: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the new break address.
    let ret = unsafe { syscall1(SYS_MEMORY_BRK, addr) };
    syscall_result(ret)
}

/// Allocate anonymous memory (convenience wrapper for mmap).
///
/// # Arguments
/// - `size`: Number of bytes to allocate (rounded up to page size)
///
/// # Returns
/// Address of the allocated memory on success.
pub fn alloc_pages(size: usize) -> Result<usize, SyscallError> {
    mmap(
        0,
        size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
}

/// Free previously allocated anonymous memory (convenience wrapper for munmap).
pub fn free_pages(addr: usize, size: usize) -> Result<usize, SyscallError> {
    munmap(addr, size)
}

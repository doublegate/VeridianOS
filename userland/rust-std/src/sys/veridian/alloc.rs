//! Memory allocation for VeridianOS user space.
//!
//! Provides mmap/munmap-based memory allocation that serves as the backing
//! allocator for Rust's `alloc` crate, plus low-level memory mapping
//! primitives.
//!
//! # Allocator Design
//!
//! The `VeridianAllocator` implements `GlobalAlloc` using anonymous mmap:
//!
//! - Small allocations (up to half a page) are served from a simple bump
//!   allocator within mmap'd pages, with a free list for recycling.
//! - Large allocations (over half a page) get their own mmap region with a
//!   metadata header preceding the returned pointer.
//! - All allocations are page-aligned at the mmap level; the user pointer is
//!   offset by the allocation header size.
//!
//! # Stack Overflow Protection
//!
//! `install_guard_page()` places a `PROT_NONE` guard page at the bottom
//! of a thread stack so that overflows trigger a page fault (SIGSEGV)
//! instead of silently corrupting adjacent memory.
//!
//! # Syscall mappings
//!
//! - `mmap` -> SYS_MEMORY_MAP (20)
//! - `munmap` -> SYS_MEMORY_UNMAP (21)
//! - `mprotect` -> SYS_MEMORY_PROTECT (22)
//! - `brk` -> SYS_MEMORY_BRK (23)

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

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

/// System page size (4 KiB on all supported VeridianOS architectures).
pub const PAGE_SIZE: usize = 4096;

// ============================================================================
// Memory Operations (low-level syscall wrappers)
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

// ============================================================================
// Alignment and size helpers
// ============================================================================

/// Round `size` up to the next multiple of `align`.
#[inline]
const fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

/// Round `size` up to the next page boundary.
#[inline]
const fn page_align(size: usize) -> usize {
    align_up(size, PAGE_SIZE)
}

// ============================================================================
// Allocation metadata header
// ============================================================================

/// Header stored immediately before every large allocation.
///
/// For mmap-backed allocations this records the total mapped size so that
/// `dealloc` knows how many bytes to unmap.
///
/// The header is always aligned to `HEADER_ALIGN` bytes so that the user
/// pointer that follows it satisfies common alignment requirements.
#[repr(C)]
struct AllocHeader {
    /// Total bytes mapped (header + padding + user data).
    mapped_size: usize,
    /// The original `Layout::size()` requested by the caller.
    user_size: usize,
}

/// Alignment of the header (and minimum user-pointer alignment for large
/// allocations).  16 bytes satisfies most SIMD types.
const HEADER_ALIGN: usize = 16;

/// Size of the header, rounded up to `HEADER_ALIGN`.
const HEADER_SIZE: usize = align_up(core::mem::size_of::<AllocHeader>(), HEADER_ALIGN);

// ============================================================================
// Small-allocation slab pool
// ============================================================================

/// Threshold below which we use the slab pool instead of a dedicated mmap.
/// This is set to half a page so that multiple small allocations can share
/// a single page.
const SMALL_ALLOC_MAX: usize = PAGE_SIZE / 2;

/// Size of each slab chunk obtained from mmap.
const SLAB_CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

/// Intrusive free list node stored inside freed slab memory.
struct FreeNode {
    next: *mut FreeNode,
    size: usize,
}

/// Per-size-class slab state.
///
/// This is a very simple bump allocator with a free list.  For a `no_std`
/// environment where performance is secondary to correctness and simplicity,
/// this is sufficient.
struct SlabState {
    /// Current bump pointer within the active slab chunk.
    bump: AtomicUsize,
    /// End of the current slab chunk.
    bump_end: AtomicUsize,
    /// Head of the free list (coarse -- not size-segregated).
    free_head: AtomicUsize,
}

/// Global slab state.  Accessed under a simple spin lock (AtomicUsize).
static SLAB: SlabState = SlabState {
    bump: AtomicUsize::new(0),
    bump_end: AtomicUsize::new(0),
    free_head: AtomicUsize::new(0),
};

/// Simple spin lock protecting slab mutations.
static SLAB_LOCK: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn slab_lock() {
    while SLAB_LOCK
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

#[inline]
fn slab_unlock() {
    SLAB_LOCK.store(0, Ordering::Release);
}

/// Attempt to allocate from the free list.
///
/// Returns a pointer to a block of at least `size` bytes with the given
/// alignment, or null if no suitable free block exists.
///
/// # Safety
/// Must be called with the slab lock held.
unsafe fn slab_alloc_free_list(size: usize, align: usize) -> *mut u8 {
    let mut prev_ptr: *mut *mut FreeNode =
        &SLAB.free_head as *const AtomicUsize as *mut *mut FreeNode;
    let mut current = SLAB.free_head.load(Ordering::Relaxed) as *mut FreeNode;

    while !current.is_null() {
        let node = unsafe { &*current };
        let addr = current as usize;
        let aligned = align_up(addr, align);
        let waste = aligned - addr;
        if node.size >= size + waste {
            // Remove from free list.
            unsafe {
                *prev_ptr = node.next;
            }
            return aligned as *mut u8;
        }
        prev_ptr = unsafe { &mut (*current).next };
        current = node.next;
    }
    ptr::null_mut()
}

/// Allocate `size` bytes with `align` from the slab pool.
///
/// # Safety
/// `size` must be <= SMALL_ALLOC_MAX and `align` must be a power of two.
unsafe fn slab_alloc(size: usize, align: usize) -> *mut u8 {
    let alloc_size = align_up(size, align.max(core::mem::size_of::<FreeNode>()));

    slab_lock();

    // 1) Try the free list first.
    let ptr = unsafe { slab_alloc_free_list(alloc_size, align) };
    if !ptr.is_null() {
        slab_unlock();
        return ptr;
    }

    // 2) Bump allocate.
    let mut bump = SLAB.bump.load(Ordering::Relaxed);
    let bump_end = SLAB.bump_end.load(Ordering::Relaxed);

    // Align the bump pointer.
    let aligned = align_up(bump, align);
    let new_bump = aligned + alloc_size;

    if bump_end == 0 || new_bump > bump_end {
        // Need a new slab chunk.
        let chunk = match mmap(
            0,
            SLAB_CHUNK_SIZE,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        ) {
            Ok(addr) => addr,
            Err(_) => {
                slab_unlock();
                return ptr::null_mut();
            }
        };
        bump = chunk;
        SLAB.bump_end
            .store(chunk + SLAB_CHUNK_SIZE, Ordering::Relaxed);

        let aligned2 = align_up(bump, align);
        let new_bump2 = aligned2 + alloc_size;
        SLAB.bump.store(new_bump2, Ordering::Relaxed);
        slab_unlock();
        return aligned2 as *mut u8;
    }

    SLAB.bump.store(new_bump, Ordering::Relaxed);
    slab_unlock();
    aligned as *mut u8
}

/// Return a slab allocation to the free list.
///
/// # Safety
/// `ptr` must have been obtained from `slab_alloc` with matching `size`.
unsafe fn slab_dealloc(ptr: *mut u8, size: usize, align: usize) {
    let alloc_size = align_up(size, align.max(core::mem::size_of::<FreeNode>()));
    // Only add to free list if the block is large enough to hold a FreeNode.
    if alloc_size < core::mem::size_of::<FreeNode>() {
        return;
    }

    slab_lock();
    let node = ptr as *mut FreeNode;
    unsafe {
        (*node).size = alloc_size;
        (*node).next = SLAB.free_head.load(Ordering::Relaxed) as *mut FreeNode;
    }
    SLAB.free_head.store(node as usize, Ordering::Relaxed);
    slab_unlock();
}

// ============================================================================
// VeridianAllocator -- GlobalAlloc implementation
// ============================================================================

/// The VeridianOS global memory allocator.
///
/// Uses anonymous `mmap` for all allocations and `munmap` for deallocation.
/// Small allocations are pooled via a slab bump allocator; large allocations
/// each get their own mapping with a metadata header.
///
/// # Example
///
/// ```ignore
/// // In the crate root or any binary:
/// #[global_allocator]
/// static ALLOC: veridian_std::platform::alloc::VeridianAllocator =
///     veridian_std::platform::alloc::VeridianAllocator;
/// ```
pub struct VeridianAllocator;

unsafe impl GlobalAlloc for VeridianAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        if size == 0 {
            // Return a non-null, well-aligned dangling pointer for ZSTs.
            return align as *mut u8;
        }

        // Small allocations: use slab pool.
        if size <= SMALL_ALLOC_MAX && align <= PAGE_SIZE {
            let ptr = unsafe { slab_alloc(size, align) };
            if !ptr.is_null() {
                return ptr;
            }
            // Fall through to large-alloc path on slab failure.
        }

        // Large allocations: mmap a dedicated region with a metadata header.
        //
        // Layout:
        //   [page boundary] -> [AllocHeader] -> [padding to align] -> [user data]
        //
        // We ensure the user pointer satisfies the requested alignment.
        let header_and_align = align_up(HEADER_SIZE, align);
        let total = match header_and_align.checked_add(size) {
            Some(t) => page_align(t),
            None => return ptr::null_mut(),
        };

        let base = match mmap(
            0,
            total,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        ) {
            Ok(addr) => addr,
            Err(_) => return ptr::null_mut(),
        };

        // Write the header.
        let header_ptr = base as *mut AllocHeader;
        unsafe {
            (*header_ptr).mapped_size = total;
            (*header_ptr).user_size = size;
        }

        // The user pointer starts after the header, aligned.
        let user_ptr = align_up(base + HEADER_SIZE, align);
        user_ptr as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size();
        let align = layout.align();

        if size == 0 {
            return;
        }

        // Small allocations: return to slab free list.
        if size <= SMALL_ALLOC_MAX && align <= PAGE_SIZE {
            unsafe {
                slab_dealloc(ptr, size, align);
            }
            return;
        }

        // Large allocations: recover the header and munmap.
        let user_addr = ptr as usize;
        // The header is located at the page-aligned base of the mapping.
        // Since we aligned user_ptr = align_up(base + HEADER_SIZE, align),
        // and base is page-aligned, we can recover it:
        let base = (user_addr - HEADER_SIZE) & !(PAGE_SIZE - 1);
        // But that's only correct if align <= PAGE_SIZE.  For larger
        // alignments the header is always at `base` of the mmap, which is
        // the page containing `user_addr - HEADER_SIZE`.
        let header_ptr = base as *const AllocHeader;
        let mapped_size = unsafe { (*header_ptr).mapped_size };

        let _ = munmap(base, mapped_size);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old_size = layout.size();

        // Allocate new block.
        let new_layout = match Layout::from_size_align(new_size, layout.align()) {
            Ok(l) => l,
            Err(_) => return ptr::null_mut(),
        };
        let new_ptr = unsafe { self.alloc(new_layout) };
        if new_ptr.is_null() {
            return ptr::null_mut();
        }

        // Copy old data.
        let copy_size = if old_size < new_size {
            old_size
        } else {
            new_size
        };
        unsafe {
            ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
        }

        // Free old block.
        unsafe {
            self.dealloc(ptr, layout);
        }

        new_ptr
    }
}

// ============================================================================
// Allocation statistics
// ============================================================================

/// Total bytes currently allocated via mmap for large allocations.
static LARGE_ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Total number of large allocations currently live.
static LARGE_ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Total bytes allocated via the slab pool.
static SLAB_ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Allocation statistics snapshot.
#[derive(Debug, Clone, Copy)]
pub struct AllocStats {
    /// Bytes currently mapped for large allocations.
    pub large_alloc_bytes: usize,
    /// Number of live large allocations.
    pub large_alloc_count: usize,
    /// Total bytes allocated through the slab pool.
    pub slab_alloc_bytes: usize,
}

/// Get a snapshot of allocation statistics.
///
/// Note: these are approximate in a multithreaded context.
pub fn alloc_stats() -> AllocStats {
    AllocStats {
        large_alloc_bytes: LARGE_ALLOC_BYTES.load(Ordering::Relaxed),
        large_alloc_count: LARGE_ALLOC_COUNT.load(Ordering::Relaxed),
        slab_alloc_bytes: SLAB_ALLOC_BYTES.load(Ordering::Relaxed),
    }
}

// ============================================================================
// Stack overflow protection
// ============================================================================

/// Install a guard page at the given address.
///
/// Marks one page as `PROT_NONE` so that any access (read/write/execute)
/// triggers a page fault.  This is typically used at the bottom of a
/// thread stack to detect stack overflow.
///
/// # Arguments
/// - `addr`: The page-aligned address of the guard page.
///
/// # Returns
/// `Ok(())` on success, or an error if the `mprotect` syscall fails.
pub fn install_guard_page(addr: usize) -> Result<(), SyscallError> {
    mprotect(addr, PAGE_SIZE, PROT_NONE)?;
    Ok(())
}

/// Allocate a thread stack with a guard page at the bottom.
///
/// Returns `(stack_bottom, stack_top, guard_page_addr)` where:
/// - `guard_page_addr` is the address of the guard page (bottom of mapping)
/// - `stack_bottom` is the first usable byte above the guard page
/// - `stack_top` is one past the last usable byte (initial stack pointer on
///   architectures where the stack grows downward)
///
/// The total mapped region is `stack_size + PAGE_SIZE`.
///
/// # Arguments
/// - `stack_size`: Desired usable stack size in bytes (will be page-aligned).
pub fn alloc_stack_with_guard(stack_size: usize) -> Result<(usize, usize, usize), SyscallError> {
    let stack_size = page_align(stack_size);
    let total = stack_size + PAGE_SIZE; // guard page + usable stack

    let base = mmap(
        0,
        total,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )?;

    // The guard page is at the very bottom of the mapping.
    install_guard_page(base)?;

    let stack_bottom = base + PAGE_SIZE;
    let stack_top = base + total;

    Ok((stack_bottom, stack_top, base))
}

/// Free a stack (including its guard page) that was allocated with
/// `alloc_stack_with_guard`.
///
/// # Arguments
/// - `guard_addr`: The guard page address returned by `alloc_stack_with_guard`.
/// - `stack_size`: The same stack size passed to `alloc_stack_with_guard`.
pub fn free_stack_with_guard(guard_addr: usize, stack_size: usize) -> Result<(), SyscallError> {
    let total = page_align(stack_size) + PAGE_SIZE;
    munmap(guard_addr, total)?;
    Ok(())
}

// ============================================================================
// Memory-mapped file support
// ============================================================================

/// Map a file into memory.
///
/// # Arguments
/// - `fd`: File descriptor to map.
/// - `offset`: Offset within the file (must be page-aligned).
/// - `length`: Number of bytes to map.
/// - `prot`: Protection flags (PROT_READ, PROT_WRITE, PROT_EXEC).
/// - `flags`: Mapping flags (MAP_SHARED or MAP_PRIVATE).
///
/// # Returns
/// Address of the mapped region on success.
pub fn mmap_file(
    fd: usize,
    offset: usize,
    length: usize,
    prot: usize,
    flags: usize,
) -> Result<usize, SyscallError> {
    mmap(0, length, prot, flags, fd as isize, offset)
}

/// Map a file read-only with MAP_PRIVATE.
pub fn mmap_file_ro(fd: usize, offset: usize, length: usize) -> Result<usize, SyscallError> {
    mmap_file(fd, offset, length, PROT_READ, MAP_PRIVATE)
}

/// Allocate anonymous read-write-execute memory.
///
/// Useful for JIT compilation or dynamic code generation.
///
/// # Safety
/// Executable memory is inherently dangerous.  The caller must ensure that
/// only trusted code is written to the returned region.
pub unsafe fn alloc_executable(size: usize) -> Result<usize, SyscallError> {
    mmap(
        0,
        page_align(size),
        PROT_READ | PROT_WRITE | PROT_EXEC,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
}

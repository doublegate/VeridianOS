//! Kernel heap
//!
//! The backing memory and initialisation of the global allocator: a
//! `linked_list_allocator::LockedHeap` on x86_64 and a bump allocator on
//! AArch64 and RISC-V (MEM-SEC-03 tracks giving those a real heap). The
//! slab allocator that used to live here was never instantiated -- every
//! request went to its fallback -- and was removed (MEM-INC-02).

// Static heap storage - kept in BSS for layout stability across all
// architectures. x86_64 uses this directly; AArch64/RISC-V use fixed physical
// addresses instead to avoid BSS/heap overlap issues.
//
// SAFETY JUSTIFICATION: This static mut is intentionally kept because:
// 1. It provides the raw backing memory for the kernel heap allocator
// 2. It must exist before the heap is initialized (pre-heap bootstrap)
// 3. Only accessed via addr_of_mut!() in init(), never through &mut references
// 4. After init, all access goes through the allocator's own synchronization
// 5. Cannot use OnceLock/GlobalState as those require heap allocation
//    themselves
// x86_64 uses a 512MB heap to support loading the self-hosting toolchain
// rootfs (~57MB TAR with BusyBox source + GCC toolchain) and Phase C native
// compilation. The rootfs extracts to ~54MB of VFS content (cc1 alone is
// 35MB). During exec, fs::read_file() creates a second 35MB copy of cc1
// for ELF loading. Combined with VFS metadata (~10MB), the BlockFS block
// cache (below), process structures, and heap fragmentation from Phase B
// tests, 384MB was insufficient when both BlockFS and native compilation
// are active.
// 1GB provides headroom for BlockFS cache, VFS, native compilation, and
// the Rust toolchain rootfs (rustc+cargo+std ~400MB). Requires QEMU -m 4096M
// minimum (typically -m 32768M for Phase 6.5 self-hosting).
// AArch64/RISC-V keep 8MB since they have less RAM (128MB default). Their
// bump allocator never frees, so the BlockFS block cache recycles its
// buffers. Its configured capacity is 1 MiB there (16 MiB on x86_64). That
// is a target, not a hard limit: dirty blocks stay pinned until sync and can
// grow the slot pool past it, and the slots are recycled rather than freed,
// so the pool stays that large after a sync (fs/blockfs/cache.rs, ADR 0003).
#[cfg(target_arch = "x86_64")]
#[allow(static_mut_refs)]
static mut HEAP_MEMORY: [u8; 1024 * 1024 * 1024] = [0; 1024 * 1024 * 1024];

#[cfg(not(target_arch = "x86_64"))]
#[allow(static_mut_refs)]
static mut HEAP_MEMORY: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];

/// Kernel heap size
#[cfg(target_arch = "x86_64")]
pub const HEAP_SIZE: usize = 1024 * 1024 * 1024;

#[cfg(not(target_arch = "x86_64"))]
pub const HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Kernel heap start address (re-exported from architecture module)
pub const HEAP_START: usize = crate::arch::HEAP_START;

/// Return the virtual address of the last byte of the HEAP_MEMORY array.
///
/// Used by the memory management init code when `__kernel_end` translation
/// fails. The heap is the largest BSS object; its end address is a tight
/// lower bound on the kernel's physical extent.
///
/// # Safety
///
/// Accesses the static `HEAP_MEMORY` address. This is safe because we only
/// compute the pointer value -- we do not read from or write to it.
pub fn heap_end_vaddr() -> u64 {
    // SAFETY: We only compute the address of the end of HEAP_MEMORY.
    // `addr_of!` avoids creating a reference to the `static mut`, which
    // would be UB under Rust 2024 rules. The pointer arithmetic is valid
    // as long as HEAP_MEMORY has been placed by the linker.
    unsafe { (core::ptr::addr_of!(HEAP_MEMORY) as *const u8).add(HEAP_SIZE) as u64 }
}

/// Get current heap statistics (x86_64 only).
///
/// Returns (total, used, free) in bytes.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn get_heap_stats() -> (usize, usize, usize) {
    let allocator = crate::get_allocator().lock();
    let total = HEAP_SIZE;
    let free = allocator.free();
    let used = total.saturating_sub(free);
    (total, used, free)
}

/// Initialize the kernel heap
pub fn init() -> Result<(), crate::error::KernelError> {
    kprintln!("[HEAP] Initializing kernel heap");

    // SAFETY: We access `HEAP_MEMORY`, a static mut byte array in the kernel's BSS
    // section. This function is called exactly once during kernel initialization
    // (single-threaded boot context), so there are no concurrent accesses. The
    // resulting `heap_start` pointer is valid for `heap_size` bytes and the memory
    // does not overlap with any other allocation because it is a dedicated static
    // array in the kernel binary.
    #[allow(unused_unsafe, unused_variables)]
    unsafe {
        let heap_start = core::ptr::addr_of_mut!(HEAP_MEMORY) as *mut u8;
        let heap_size = HEAP_SIZE;

        // RISC-V: Use UnsafeBumpAllocator (same as AArch64).
        // LockedHeap's linked-list free list gets corrupted on RISC-V bare
        // metal ("Hole list out of order?"), so we use the simpler bump
        // allocator with a 4MB heap that provides ample space for boot.
        #[cfg(target_arch = "riscv64")]
        {
            println!("[HEAP] Initializing RISC-V UnsafeBumpAllocator");
            println!(
                "[HEAP] Heap start: {:p}, size: {} bytes",
                heap_start, heap_size
            );

            // SAFETY: `ALLOCATOR` is the global bump allocator. `heap_start` points to
            // valid memory of at least `heap_size` bytes (the static HEAP_MEMORY array).
            // This is called once during single-threaded boot, so no concurrent access.
            unsafe {
                crate::ALLOCATOR.init(heap_start, heap_size);
            }

            println!("[HEAP] RISC-V heap initialization complete");
        }

        // AArch64: Use lock-free UnsafeBumpAllocator (LockedHeap deadlocks on AArch64)
        // Initialize fields directly to avoid function call issues on AArch64
        #[cfg(target_arch = "aarch64")]
        {
            use core::sync::atomic::Ordering;

            use crate::arch::aarch64::direct_uart::uart_write_str;

            uart_write_str("[HEAP] Initializing AArch64 UnsafeBumpAllocator\n");

            let start_addr = heap_start as usize;

            // Initialize ALLOCATOR atomics directly (bypasses function call)
            crate::ALLOCATOR.start.store(start_addr, Ordering::SeqCst);
            crate::ALLOCATOR.size.store(heap_size, Ordering::SeqCst);
            crate::ALLOCATOR.next.store(start_addr, Ordering::SeqCst);
            crate::ALLOCATOR.allocations.store(0, Ordering::SeqCst);
            core::sync::atomic::fence(Ordering::SeqCst);

            // AArch64 memory barriers
            // SAFETY: DSB SY (Data Synchronization Barrier) and ISB (Instruction
            // Synchronization Barrier) are architectural barrier instructions that
            // are always safe to execute at any exception level. They ensure all
            // preceding memory operations complete before subsequent ones begin,
            // which is required after writing to the allocator's atomic fields so
            // that the allocator state is visible before any allocation attempts.
            unsafe {
                core::arch::asm!("dsb sy", "isb", options(nomem, nostack));
            }

            uart_write_str("[HEAP] ALLOCATOR initialized\n");

            // Verify allocator state
            let next_val = crate::ALLOCATOR.next.load(Ordering::SeqCst);
            if next_val != 0 {
                uart_write_str("[HEAP] Allocator state verified OK\n");
            } else {
                uart_write_str("[HEAP] WARNING: Allocator next=0\n");
            }

            uart_write_str("[HEAP] AArch64 heap initialization complete\n");
        }

        // x86_64: Use LockedHeap
        // Note: The `init` call on LockedHeap is unsafe because it trusts the
        // caller to provide valid memory. The outer unsafe block already
        // establishes that `heap_start`/`heap_size` describe valid, exclusive
        // memory from the static HEAP_MEMORY array.
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            let mut allocator = crate::get_allocator().lock();
            allocator.init(heap_start, heap_size);
            drop(allocator);
        }

        println!(
            "[HEAP] Heap initialized: {} KB at {:p}",
            heap_size / 1024,
            core::ptr::addr_of!(HEAP_MEMORY)
        );
    }

    Ok(())
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use alloc::{boxed::Box, vec::Vec};

    #[test]
    fn test_heap_allocation() {
        // Test various allocations
        let x = Box::new(42);
        assert_eq!(*x, 42);

        let mut v = Vec::new();
        for i in 0..100 {
            v.push(i);
        }
        assert_eq!(v.len(), 100);
    }
}

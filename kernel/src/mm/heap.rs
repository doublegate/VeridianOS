//! Kernel heap
//!
//! The backing memory and initialisation of the global allocator: a
//! `linked_list_allocator::LockedHeap` on x86_64 and RISC-V, and a bump
//! allocator on AArch64 until its MMU is on (N-28; MEM-SEC-03 tracks giving
//! it a real heap). The
//! slab allocator that used to live here was never instantiated -- every
//! request went to its fallback -- and was removed (MEM-INC-02).

use core::sync::atomic::{AtomicUsize, Ordering};

// x86_64 (N-278): the heap is a run of physical frames taken from the frame
// allocator at boot and used through the physical-memory map, sized from the
// RAM the bootloader reports (`heap_size_for`). It used to be a 1 GiB static
// array in the BSS, which made every guest need more than 1 GiB of RAM just
// to load the kernel. The sizing keeps the old capacity where the memory is
// there: a third of usable RAM rounded down to a power of two, 512 MiB at
// `-m 2048M`, the old 1 GiB from 4 GiB --
// what loading the self-hosting toolchain needs (cc1 is 35 MB, and exec
// keeps a second copy while loading it; the BlockFS cache, VFS metadata and
// the Rust toolchain rootfs come on top).
//
// AArch64/RISC-V keep a static 8 MiB heap until their MMU work (sprint E):
// the BlockFS cache there is 1 MiB (fs/blockfs/cache.rs, ADR 0003/0011).
//
// SAFETY JUSTIFICATION for the static: it is the raw backing memory of the
// heap, needed before any allocation exists; it is only accessed through
// addr_of_mut!() in init(), and afterwards only through the allocator.
#[cfg(not(target_arch = "x86_64"))]
#[allow(static_mut_refs)]
static mut HEAP_MEMORY: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];

/// Size of the static heap (AArch64, RISC-V).
#[cfg(not(target_arch = "x86_64"))]
pub const HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Smallest x86_64 heap: what the boot tests and the BusyBox root need.
pub const MIN_HEAP_SIZE: usize = 32 * 1024 * 1024;

/// Largest x86_64 heap.
pub const MAX_HEAP_SIZE: usize = 1024 * 1024 * 1024;

/// Bytes the heap was given at init (0 before).
static HEAP_BYTES: AtomicUsize = AtomicUsize::new(0);

/// The kernel heap's size in bytes.
pub fn heap_size() -> usize {
    HEAP_BYTES.load(Ordering::Relaxed)
}

/// The x86_64 heap size for `ram` bytes of usable memory: a third of it,
/// rounded down to a power of two (the buddy allocator's unit), clamped to
/// `MIN_HEAP_SIZE..=MAX_HEAP_SIZE`. A third rather than a quarter: usable
/// RAM is a little below the guest's size, and a quarter of it would round
/// down to half the intended heap.
pub fn heap_size_for(ram: usize) -> usize {
    let third = ram / 3;
    let pow2 = if third == 0 {
        0
    } else {
        1usize << (usize::BITS - 1 - third.leading_zeros())
    };
    pow2.clamp(MIN_HEAP_SIZE, MAX_HEAP_SIZE)
}

/// Get current heap statistics.
///
/// Returns (total, used, free) in bytes.
#[cfg(all(
    any(
        target_arch = "x86_64",
        target_arch = "riscv64",
        target_arch = "aarch64"
    ),
    target_os = "none"
))]
pub fn get_heap_stats() -> (usize, usize, usize) {
    // Interrupts off while the heap lock is held (see `IrqSafeHeap`).
    let _irq = crate::arch::disable_interrupts();
    let allocator = crate::get_allocator().lock();
    let total = heap_size();
    let free = allocator.free();
    let used = total.saturating_sub(free);
    (total, used, free)
}

/// Initialize the kernel heap
pub fn init() -> Result<(), crate::error::KernelError> {
    kprintln!("[HEAP] Initializing kernel heap");

    // SAFETY: called exactly once, during single-threaded early boot. The
    // region is exclusively the heap's: the static array on AArch64/RISC-V,
    // and on x86_64 frames taken from the frame allocator and never freed.
    #[allow(unused_unsafe, unused_variables)]
    unsafe {
        #[cfg(not(target_arch = "x86_64"))]
        let (heap_start, heap_size) = (core::ptr::addr_of_mut!(HEAP_MEMORY) as *mut u8, HEAP_SIZE);
        #[cfg(target_arch = "x86_64")]
        let (heap_start, heap_size) = frame_backed_region()?;
        HEAP_BYTES.store(heap_size, Ordering::Relaxed);

        // RISC-V and AArch64: the same LockedHeap as x86_64 (MEM-SEC-03).
        // RISC-V used the never-freeing bump allocator because the
        // linked-list free list was corrupted ("Hole list out of order?");
        // the cause was the frame allocator handing out frames inside the
        // kernel image, where this static heap lives, since fixed (the
        // allocator starts after the kernel end). AArch64 used it because
        // locks need exclusives, which hung with its MMU off (N-28).
        #[cfg(all(
            any(target_arch = "riscv64", target_arch = "aarch64"),
            target_os = "none"
        ))]
        {
            let mut allocator = crate::get_allocator().lock();
            allocator.init(heap_start, heap_size);
            drop(allocator);
        }

        // x86_64: Use LockedHeap
        // Note: The `init` call on LockedHeap is unsafe because it trusts the
        // caller to provide valid memory. `frame_backed_region` returned
        // `heap_size` bytes of frames owned by the heap from now on, mapped
        // through the physical-memory map.
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            let mut allocator = crate::get_allocator().lock();
            allocator.init(heap_start, heap_size);
            drop(allocator);
        }

        println!(
            "[HEAP] Heap initialized: {} KB at {:p}",
            heap_size / 1024,
            heap_start
        );
    }

    Ok(())
}

/// x86_64: take the heap from the frame allocator (N-278): `heap_size_for`
/// the usable RAM, halving down to `MIN_HEAP_SIZE` if no run that long is
/// free. Needs the frame allocator initialized and the boot page-table
/// frames reserved (`mm::init_default` does both first), but no heap.
#[cfg(target_arch = "x86_64")]
fn frame_backed_region() -> Result<(*mut u8, usize), crate::error::KernelError> {
    use crate::mm::{FRAME_ALLOCATOR, FRAME_SIZE};
    let ram = FRAME_ALLOCATOR.lock().get_stats().free_frames as usize * FRAME_SIZE;
    let mut size = heap_size_for(ram);
    loop {
        if let Ok(frame) = FRAME_ALLOCATOR
            .lock()
            .allocate_contiguous(size / FRAME_SIZE)
        {
            let virt = crate::mm::phys_to_virt_addr(frame.as_u64() * FRAME_SIZE as u64);
            return Ok((virt as *mut u8, size));
        }
        if size <= MIN_HEAP_SIZE {
            return Err(crate::error::KernelError::OutOfMemory {
                requested: size,
                available: ram,
            });
        }
        size /= 2;
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use alloc::{boxed::Box, vec::Vec};

    /// N-278: a third of usable RAM, a power of two, within 32 MiB..=1 GiB;
    /// `-m 2048M` (about 1,950 MiB usable) gets 512 MiB, 4 GiB or more the
    /// old 1 GiB.
    #[test]
    fn heap_size_follows_ram() {
        use super::{heap_size_for, MAX_HEAP_SIZE, MIN_HEAP_SIZE};
        const MIB: usize = 1024 * 1024;
        assert_eq!(heap_size_for(1950 * MIB), 512 * MIB);
        assert_eq!(heap_size_for(1500 * MIB), 256 * MIB, "rounded down");
        assert_eq!(heap_size_for(220 * MIB), 64 * MIB);
        assert_eq!(heap_size_for(64 * MIB), MIN_HEAP_SIZE);
        assert_eq!(heap_size_for(0), MIN_HEAP_SIZE);
        assert_eq!(heap_size_for(4096 * MIB), MAX_HEAP_SIZE);
        assert_eq!(heap_size_for(32768 * MIB), MAX_HEAP_SIZE);
    }

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

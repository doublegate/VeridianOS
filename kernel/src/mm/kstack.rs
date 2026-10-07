//! Kernel stacks with guard pages (N-26).
//!
//! On x86_64 each thread's kernel stack is mapped into its own slot of a
//! dedicated kernel-half region, at the top of the slot, with the pages
//! below it left unmapped. A stack overflow then faults on the guard (the
//! page-fault handler runs on its own IST stack) instead of silently
//! overwriting whatever frame lay next to the stack in the direct physical
//! map, which is where stacks used to live.
//!
//! The region's L4 entry is created once at boot, before any process copies
//! the kernel half of the page table, so every address space shares the
//! tables below it and sees each stack as it is mapped.
//!
//! AArch64 and RISC-V run the kernel without paging today (sprint E), so
//! there the stack keeps its direct-map address and has no guard yet.

use super::{FrameNumber, FRAME_ALLOCATOR};
use crate::error::KernelError;

/// A kernel stack: `pages` contiguous frames from `frame`, usable at
/// `[base, base + pages * 4096)`.
#[derive(Debug, Clone, Copy)]
pub struct KernelStack {
    pub base: usize,
    pub frame: FrameNumber,
    pub pages: usize,
}

#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
mod region {
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use spin::Mutex;

    use super::*;
    use crate::mm::{PageFlags, VirtualAddress};

    /// The stacks region (one L4 slot, 512 GiB).
    pub(super) const REGION_BASE: u64 = 0xFFFF_E000_0000_0000;
    /// One stack per slot: up to 252 KiB of stack and at least one guard
    /// page below it.
    pub(super) const SLOT_SIZE: u64 = 256 * 1024;
    /// Slots in the region used (16 GiB of address space).
    const MAX_SLOTS: usize = 1 << 16;

    static NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);
    static FREE_SLOTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    /// Serialises changes to the shared kernel page tables of the region.
    static MAP_LOCK: Mutex<()> = Mutex::new(());

    fn mapper() -> crate::mm::page_table::PageMapper {
        // SAFETY: the kernel's root page table, reached through the physical
        // map; changes under the region are serialised by MAP_LOCK.
        unsafe {
            crate::mm::vas::create_mapper_from_root_pub(crate::mm::get_kernel_page_table() as u64)
        }
    }

    /// Create the region's page tables (its L4 entry in particular) so that
    /// address spaces created from now on share them.
    pub(super) fn init() {
        let _guard = MAP_LOCK.lock();
        let mut m = mapper();
        let probe = VirtualAddress(REGION_BASE);
        // Mapping and unmapping one page builds the L4 -> L1 path.
        // Bound first: a guard temporary in the `if let` would stay locked
        // across map_page, which allocates the tables from the same lock.
        let probe_frame = FRAME_ALLOCATOR.lock().allocate_frames(1, None);
        if let Ok(frame) = probe_frame {
            if m.map_page(
                probe,
                frame,
                PageFlags::PRESENT | PageFlags::WRITABLE,
                &mut crate::mm::vas::VasFrameAllocator,
            )
            .is_ok()
            {
                let _ = m.unmap_page(probe);
            }
            let _ = FRAME_ALLOCATOR.lock().free_frames(frame, 1);
        }
    }

    pub(super) fn map(frame: FrameNumber, pages: usize) -> Result<usize, KernelError> {
        if pages == 0 || pages as u64 * 4096 >= SLOT_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "kernel stack pages",
                value: "too large for a stack slot",
            });
        }
        let slot = match FREE_SLOTS.lock().pop() {
            Some(s) => s,
            None => {
                let s = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
                if s >= MAX_SLOTS {
                    return Err(KernelError::ResourceExhausted {
                        resource: "kernel stack slots",
                    });
                }
                s
            }
        };
        let slot_end = REGION_BASE + (slot as u64 + 1) * SLOT_SIZE;
        let base = slot_end - pages as u64 * 4096;
        let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_EXECUTE;
        let _guard = MAP_LOCK.lock();
        let mut m = mapper();
        for i in 0..pages as u64 {
            let r = m.map_page(
                VirtualAddress(base + i * 4096),
                FrameNumber::new(frame.as_u64() + i),
                flags,
                &mut crate::mm::vas::VasFrameAllocator,
            );
            if let Err(e) = r {
                for j in 0..i {
                    let _ = m.unmap_page(VirtualAddress(base + j * 4096));
                }
                drop(_guard);
                crate::mm::tlb::flush_all();
                FREE_SLOTS.lock().push(slot);
                return Err(e);
            }
        }
        Ok(base as usize)
    }

    pub(super) fn unmap(base: usize, pages: usize) {
        let _guard = MAP_LOCK.lock();
        let mut m = mapper();
        for i in 0..pages as u64 {
            let _ = m.unmap_page(VirtualAddress(base as u64 + i * 4096));
        }
        drop(_guard);
        // Every CPU forgets the stack before its frames are reused.
        crate::mm::tlb::flush_all();
        let slot = ((base as u64 - REGION_BASE) / SLOT_SIZE) as usize;
        FREE_SLOTS.lock().push(slot);
    }

    pub(super) fn contains(addr: u64) -> bool {
        (REGION_BASE..REGION_BASE + MAX_SLOTS as u64 * SLOT_SIZE).contains(&addr)
    }
}

/// Prepare the stack region. Call once during memory init, before the
/// first address space is created.
pub fn init() {
    #[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
    region::init();
}

/// Allocate a kernel stack of `pages` pages.
pub fn allocate(pages: usize) -> Result<KernelStack, KernelError> {
    let frame = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(pages, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: pages * 4096,
            available: 0,
        })?;
    #[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
    let base = match region::map(frame, pages) {
        Ok(base) => base,
        Err(e) => {
            let _ = FRAME_ALLOCATOR.lock().free_frames(frame, pages);
            return Err(e);
        }
    };
    #[cfg(not(all(target_arch = "x86_64", target_os = "none", feature = "alloc")))]
    let base = super::phys_to_virt_addr(frame.as_u64() << 12) as usize;
    // SAFETY: `base` maps exactly the `pages` frames just allocated.
    unsafe { core::ptr::write_bytes(base as *mut u8, 0, pages * 4096) };
    Ok(KernelStack { base, frame, pages })
}

/// Free a stack from [`allocate`]: unmap it (on every CPU), then return
/// its frames.
pub fn free(stack: KernelStack) {
    #[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
    if region::contains(stack.base as u64) {
        region::unmap(stack.base, stack.pages);
    }
    let _ = FRAME_ALLOCATOR.lock().free_frames(stack.frame, stack.pages);
}

/// Whether `addr` is in the guard area of the kernel stack region (below
/// a stack in its slot), i.e. a fault there is a kernel stack overflow.
pub fn is_guard_fault(addr: u64) -> bool {
    #[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
    {
        region::contains(addr)
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none", feature = "alloc")))]
    {
        let _ = addr;
        false
    }
}

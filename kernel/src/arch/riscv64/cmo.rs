//! Cache-block management (Zicbom) for streaming DMA to devices that are
//! not coherent (`mm::dma`; HA-25 on RISC-V, ADR 0019).
//!
//! Most RISC-V systems are coherent; the T-Head C9xx and SpacemiT K1 SoCs
//! mark their devices `dma-noncoherent`. Zicbom's `cbo.clean` and
//! `cbo.flush` act on one cache block (`riscv,cbom-block-size`). Firmware
//! enables them for S-mode (menvcfg.CBCFE, CBIE) when the hart has them.
//! Without Zicbom nothing can be done here: the T-Head vendor instructions
//! (XTheadCmo) are not supported.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static PRESENT: AtomicBool = AtomicBool::new(false);
static BLOCK: AtomicUsize = AtomicUsize::new(64);

/// Record Zicbom and its block size from the device tree's cpu node.
pub fn init_from_device_tree(dtb_pa: u64) {
    // SAFETY: dtb_pa is what firmware passed in a1 (saved by boot.S).
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(dtb_pa) }) else {
        return;
    };
    let has = |prop| {
        fdt.property(&["cpus", "cpu"], prop)
            .is_some_and(|isa| crate::arch::fdt::isa_has_extension(isa, b"zicbom"))
    };
    let block = fdt
        .property_u64(&["cpus", "cpu"], "riscv,cbom-block-size")
        .unwrap_or(64) as usize;
    if (has("riscv,isa-extensions") || has("riscv,isa"))
        && block.is_power_of_two()
        && (16..=4096).contains(&block)
    {
        BLOCK.store(block, Ordering::Relaxed);
        PRESENT.store(true, Ordering::Relaxed);
    }
}

/// Whether the hart has Zicbom.
pub fn present() -> bool {
    PRESENT.load(Ordering::Relaxed)
}

/// Write back the cache blocks of `addr..addr + len` (kernel addresses).
pub fn clean(addr: usize, len: usize) {
    if !present() {
        return;
    }
    for a in crate::mm::page_ops::cache_line_starts(addr, len, BLOCK.load(Ordering::Relaxed)) {
        // SAFETY: `cbo.clean` writes back the block holding `a`, a mapped
        // kernel address; it changes no data. Zicbom is present.
        unsafe {
            core::arch::asm!(
                ".option push",
                ".option arch, +zicbom",
                "cbo.clean ({0})",
                ".option pop",
                in(reg) a,
                options(nostack)
            )
        };
    }
    // SAFETY: a barrier only: the write-backs complete before what follows.
    unsafe { core::arch::asm!("fence rw, rw", options(nostack)) };
}

/// Write back and invalidate the cache blocks of `addr..addr + len`.
pub fn flush(addr: usize, len: usize) {
    if !present() {
        return;
    }
    for a in crate::mm::page_ops::cache_line_starts(addr, len, BLOCK.load(Ordering::Relaxed)) {
        // SAFETY: `cbo.flush` writes back and invalidates the block holding
        // `a`, a mapped kernel address; memory keeps its data.
        unsafe {
            core::arch::asm!(
                ".option push",
                ".option arch, +zicbom",
                "cbo.flush ({0})",
                ".option pop",
                in(reg) a,
                options(nostack)
            )
        };
    }
    // SAFETY: a barrier only.
    unsafe { core::arch::asm!("fence rw, rw", options(nostack)) };
}

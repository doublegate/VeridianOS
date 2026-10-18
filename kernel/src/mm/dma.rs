//! Streaming DMA: cache maintenance for memory a device reads or writes
//! (HA-25, ADR 0019).

/// Which way a transfer goes, as Linux's `enum dma_data_direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    /// The device reads the buffer.
    ToDevice,
    /// The device writes the buffer.
    FromDevice,
    /// Both.
    Bidirectional,
}

/// A cache operation on a buffer's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOp {
    None,
    /// Write dirty lines back to memory (to the point of coherency).
    Clean,
    /// Write back, then drop the lines.
    CleanInvalidate,
}

/// Before the device may use the buffer: on a non-coherent device the
/// CPU's writes go back to memory in every direction, so that no dirty line
/// is evicted over what the device writes later (Linux's arm64 and RISC-V
/// `arch_sync_dma_for_device`).
pub const fn op_for_device(dir: DmaDirection, coherent: bool) -> CacheOp {
    let _ = dir;
    if coherent {
        CacheOp::None
    } else {
        CacheOp::Clean
    }
}

/// Before the CPU reads the buffer after the device used it: on a
/// non-coherent device, drop lines the CPU may have fetched meanwhile
/// (speculatively), unless the device only read it (Linux's
/// `arch_sync_dma_for_cpu`). Clean-and-invalidate rather than invalidate,
/// so that a line the buffer shares with other data loses nothing.
pub const fn op_for_cpu(dir: DmaDirection, coherent: bool) -> CacheOp {
    if coherent || matches!(dir, DmaDirection::ToDevice) {
        CacheOp::None
    } else {
        CacheOp::CleanInvalidate
    }
}

/// Make the CPU's writes to `[addr, addr + len)` (kernel addresses, such as
/// the physical map's alias of a DMA frame) visible to a device about to
/// access it. A barrier for the device's order follows either way.
pub fn sync_for_device(addr: usize, len: usize, dir: DmaDirection, coherent: bool) {
    apply(op_for_device(dir, coherent), addr, len);
    crate::arch::barriers::dma_wmb();
}

/// Make what a device wrote to `[addr, addr + len)` visible to the CPU,
/// once the device is done with it.
pub fn sync_for_cpu(addr: usize, len: usize, dir: DmaDirection, coherent: bool) {
    crate::arch::barriers::dma_rmb();
    apply(op_for_cpu(dir, coherent), addr, len);
}

fn apply(op: CacheOp, addr: usize, len: usize) {
    if len == 0 {
        return;
    }
    match op {
        CacheOp::None => {}
        #[cfg(all(target_arch = "aarch64", target_os = "none"))]
        CacheOp::Clean => crate::arch::aarch64::dcache_clean_poc(addr, len),
        #[cfg(all(target_arch = "aarch64", target_os = "none"))]
        CacheOp::CleanInvalidate => crate::arch::aarch64::dcache_clean_inval_poc(addr, len),
        #[cfg(all(target_arch = "riscv64", target_os = "none"))]
        CacheOp::Clean => crate::arch::riscv64::cmo::clean(addr, len),
        #[cfg(all(target_arch = "riscv64", target_os = "none"))]
        CacheOp::CleanInvalidate => crate::arch::riscv64::cmo::flush(addr, len),
        // x86_64 DMA is coherent; the host has no devices.
        #[cfg(not(all(
            any(target_arch = "aarch64", target_arch = "riscv64"),
            target_os = "none"
        )))]
        CacheOp::Clean | CacheOp::CleanInvalidate => {
            let _ = addr;
        }
    }
}

/// Whether a device the firmware does not describe either way is
/// DMA-coherent, as Linux assumes: not on AArch64 (a device tree must say
/// `dma-coherent`), yes on RISC-V (`dma-noncoherent` says otherwise) and on
/// x86_64.
pub const DEFAULT_COHERENT: bool = !cfg!(target_arch = "aarch64");

/// Whether the device tree's enabled `compat` device at physical `base` is
/// DMA-coherent ([`DEFAULT_COHERENT`] when it does not say, or there is no
/// device tree).
pub fn device_coherent(compat: &[u8], base: u64) -> bool {
    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    let dtb = Some(crate::arch::aarch64::boot::dtb_pa());
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    let dtb = Some(crate::arch::riscv64::dtb_pa());
    #[cfg(not(all(
        any(target_arch = "aarch64", target_arch = "riscv64"),
        target_os = "none"
    )))]
    let dtb: Option<u64> = None;
    dtb.and_then(|pa| {
        // SAFETY: the device tree firmware passed (or QEMU virt's), saved
        // by boot.S; only read.
        unsafe { crate::arch::fdt::Fdt::from_phys(pa) }
    })
    .and_then(|fdt| fdt.dma_coherent(compat, base))
    .unwrap_or(DEFAULT_COHERENT)
}

#[cfg(test)]
mod tests {
    use super::{DmaDirection::*, *};

    /// A coherent device needs nothing. Otherwise the CPU's writes go back
    /// to memory before every transfer, in any direction, so no dirty line
    /// is later evicted over what the device wrote; and the lines are
    /// dropped before the CPU reads what the device wrote, but not after a
    /// transfer the device only read (Linux's arm64 and RISC-V
    /// `arch_sync_dma_for_{device,cpu}`).
    #[test]
    fn maintenance_follows_direction_and_coherency() {
        for dir in [ToDevice, FromDevice, Bidirectional] {
            assert_eq!(op_for_device(dir, true), CacheOp::None);
            assert_eq!(op_for_cpu(dir, true), CacheOp::None);
            assert_eq!(op_for_device(dir, false), CacheOp::Clean);
        }
        assert_eq!(op_for_cpu(ToDevice, false), CacheOp::None);
        assert_eq!(op_for_cpu(FromDevice, false), CacheOp::CleanInvalidate);
        assert_eq!(op_for_cpu(Bidirectional, false), CacheOp::CleanInvalidate);
    }
}

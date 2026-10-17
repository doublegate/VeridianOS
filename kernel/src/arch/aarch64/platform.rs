//! QEMU `virt` device addresses, as kernel pointers.
//!
//! Devices are reached through the physical-memory map (ADR 0015), which
//! boot.S maps at a fixed base with the first GiB (QEMU `virt`'s MMIO) as
//! Device memory, so these are compile-time constants and work in any code,
//! including code that may run before `mm::PHYS_MEM_OFFSET` is set (an early
//! fault). The physical addresses are QEMU `virt`'s; a board port replaces
//! them with ones from the device tree (HA-05, HA-07).

use super::boot::PHYS_MAP_BASE;

/// The kernel address of physical device address `pa`.
pub const fn mmio(pa: usize) -> usize {
    pa + PHYS_MAP_BASE as usize
}

/// PL011 UART (data register at offset 0).
pub const PL011: usize = mmio(0x0900_0000);
/// GICv2 distributor.
pub const GICD: usize = mmio(0x0800_0000);
/// GICv2 CPU interface.
pub const GICC: usize = mmio(0x0801_0000);

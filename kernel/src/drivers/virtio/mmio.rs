//! Virtio MMIO transport (virtio 1.0 legacy-compatible)
//!
//! Implements the virtio-over-MMIO transport layer as defined in the
//! [virtio specification, section 4.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html).
//! This is the transport used on AArch64 and RISC-V QEMU `virt` machines,
//! where virtio devices are memory-mapped rather than behind PCI.
//!
//! # Default MMIO Base Addresses
//!
//! The `DEFAULT_BASES` array lists the first four virtio-mmio device regions
//! for QEMU's `virt` machine. Each region is 0x200 bytes (512 bytes) and
//! contains the standard virtio-mmio register set at the offsets defined in
//! the `regs` module. Valid register offsets range from 0x000 to 0x0A4.
//! Device-specific configuration space starts at offset 0x100 (modern) or
//! STATUS + 0x14 (legacy).
//!
//! # Usage
//!
//! This is a minimal implementation sufficient for virtio-blk using split
//! virtqueues. For x86_64, the PCI transport in `mod.rs` is used instead.
//! See [`super::VirtioTransport`] for the unified transport enum.

#![allow(dead_code)]

use core::ptr;

use crate::{
    arch::barriers::{data_sync_barrier, instruction_sync_barrier},
    error::KernelError,
};

/// Default virtio-mmio base addresses for QEMU's `virt` machine.
///
/// The two architectures use different memory maps:
///
/// - **AArch64**: virtio-mmio devices start at 0x0A00_0000 with 0x200 stride
///   (up to 32 devices, each 512 bytes). See QEMU `hw/arm/virt.c`.
/// - **RISC-V**: virtio-mmio devices start at 0x1000_1000 with 0x1000 stride
///   (up to 8 devices, each 4 KB). See QEMU `hw/riscv/virt.c`.
///
/// We probe the first four slots; this is sufficient for virtio-blk discovery.
#[cfg(target_arch = "aarch64")]
pub const DEFAULT_BASES: [usize; 4] = [0x0a00_0000, 0x0a00_0200, 0x0a00_0400, 0x0a00_0600];

#[cfg(target_arch = "riscv64")]
pub const DEFAULT_BASES: [usize; 4] = [0x1000_1000, 0x1000_2000, 0x1000_3000, 0x1000_4000];

/// Fallback for other architectures (should not be reached; MMIO transport is
/// only used on AArch64 and RISC-V).
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
pub const DEFAULT_BASES: [usize; 4] = [0x0a00_0000, 0x0a00_2000, 0x0a00_4000, 0x0a00_6000];

/// MMIO register offsets (per virtio spec 4.2.2, legacy interface).
///
/// Valid offsets range from 0x000 (MAGIC) to 0x0A4 (QUEUE_USED_HIGH).
/// Each register is 32 bits wide unless noted otherwise. The caller must
/// ensure that `base + offset` falls within the 0x200-byte MMIO region
/// mapped for the device.
mod regs {
    pub const MAGIC: usize = 0x000; // Magic value "virt"
    pub const VERSION: usize = 0x004; // 1 = legacy, 2 = modern
    pub const DEVICE_ID: usize = 0x008;
    pub const VENDOR_ID: usize = 0x00c;
    pub const DEVICE_FEATURES: usize = 0x010;
    pub const DEVICE_FEATURES_SEL: usize = 0x014;
    pub const DRIVER_FEATURES: usize = 0x020;
    pub const DRIVER_FEATURES_SEL: usize = 0x024;
    pub const QUEUE_SEL: usize = 0x030;
    pub const QUEUE_NUM_MAX: usize = 0x034;
    pub const QUEUE_NUM: usize = 0x038;
    pub const QUEUE_READY: usize = 0x044;
    pub const QUEUE_NOTIFY: usize = 0x050;
    pub const INTERRUPT_STATUS: usize = 0x060;
    pub const INTERRUPT_ACK: usize = 0x064;
    pub const STATUS: usize = 0x070;
    // Physical addresses for split virtqueues
    pub const QUEUE_DESC_LOW: usize = 0x080;
    pub const QUEUE_DESC_HIGH: usize = 0x084;
    pub const QUEUE_AVAIL_LOW: usize = 0x090;
    pub const QUEUE_AVAIL_HIGH: usize = 0x094;
    pub const QUEUE_USED_LOW: usize = 0x0a0;
    pub const QUEUE_USED_HIGH: usize = 0x0a4;
}

/// Virtio-mmio status flags (same as PCI transport)
mod status {
    pub const ACKNOWLEDGE: u32 = 1;
    pub const DRIVER: u32 = 2;
    pub const DRIVER_OK: u32 = 4;
    pub const FEATURES_OK: u32 = 8;
    pub const FAILED: u32 = 128;
}

/// Handle for a single virtio-mmio device.
///
/// Wraps the kernel-virtual base address of a virtio-mmio register region
/// and provides typed read/write accessors. The base address must point to
/// a valid 0x200-byte MMIO region that is mapped in the kernel's address
/// space (identity-mapped on AArch64/RISC-V, or via the physical memory
/// window on x86_64).
///
/// # Safety Invariant
///
/// The `base` address must remain valid and mapped for the lifetime of this
/// struct. All register accesses use volatile reads/writes to prevent the
/// compiler from reordering or eliding MMIO operations.
#[derive(Debug, Clone, Copy)]
pub struct VirtioMmioTransport {
    base: usize,
}

impl VirtioMmioTransport {
    pub fn new(base: usize) -> Self {
        Self { base }
    }

    #[inline]
    fn read32(&self, offset: usize) -> u32 {
        // SAFETY: base + offset is an MMIO region mapped in the kernel's phys window.
        unsafe { ptr::read_volatile((self.base + offset) as *const u32) }
    }

    #[inline]
    fn write32(&self, offset: usize, value: u32) {
        // SAFETY: base + offset is an MMIO region mapped in the kernel's phys window.
        unsafe { ptr::write_volatile((self.base + offset) as *mut u32, value) }
    }

    #[inline]
    fn write16(&self, offset: usize, value: u16) {
        // SAFETY: base + offset is an MMIO region mapped in the kernel's phys window.
        unsafe { ptr::write_volatile((self.base + offset) as *mut u16, value) }
    }

    pub fn matches_blk(&self) -> bool {
        self.read32(regs::MAGIC) == 0x7472_6976 // "virt"
            && self.read32(regs::DEVICE_ID) == 2 // 2 = block device
    }

    pub fn begin_init(&self) {
        self.write32(regs::STATUS, 0);
        self.set_status(status::ACKNOWLEDGE | status::DRIVER);
    }

    fn set_status(&self, bits: u32) {
        let cur = self.read32(regs::STATUS);
        self.write32(regs::STATUS, cur | bits);
    }

    pub fn set_failed(&self) {
        self.write32(regs::STATUS, status::FAILED);
    }

    pub fn set_features_ok(&self) -> bool {
        self.set_status(status::FEATURES_OK);
        self.read32(regs::STATUS) & status::FEATURES_OK != 0
    }

    pub fn set_driver_ok(&self) {
        self.set_status(status::DRIVER_OK);
    }

    pub fn read_device_features(&self) -> u32 {
        self.write32(regs::DEVICE_FEATURES_SEL, 0);
        self.read32(regs::DEVICE_FEATURES)
    }

    pub fn write_driver_features(&self, features: u32) {
        self.write32(regs::DRIVER_FEATURES_SEL, 0);
        self.write32(regs::DRIVER_FEATURES, features);
    }

    pub fn select_queue(&self, idx: u16) {
        self.write32(regs::QUEUE_SEL, idx as u32);
    }

    pub fn read_queue_size_max(&self) -> u16 {
        self.read32(regs::QUEUE_NUM_MAX) as u16
    }

    pub fn set_queue_size(&self, size: u16) {
        self.write32(regs::QUEUE_NUM, size as u32);
    }

    pub fn set_queue_ready(&self) {
        self.write32(regs::QUEUE_READY, 1);
    }

    pub fn write_queue_phys(&self, desc: u64, avail: u64, used: u64) {
        self.write32(regs::QUEUE_DESC_LOW, desc as u32);
        self.write32(regs::QUEUE_DESC_HIGH, (desc >> 32) as u32);
        self.write32(regs::QUEUE_AVAIL_LOW, avail as u32);
        self.write32(regs::QUEUE_AVAIL_HIGH, (avail >> 32) as u32);
        self.write32(regs::QUEUE_USED_LOW, used as u32);
        self.write32(regs::QUEUE_USED_HIGH, (used >> 32) as u32);
        data_sync_barrier();
        instruction_sync_barrier();
    }

    pub fn notify_queue(&self, idx: u16) {
        self.write32(regs::QUEUE_NOTIFY, idx as u32);
    }

    pub fn ack_interrupts(&self) {
        let pending = self.read32(regs::INTERRUPT_STATUS);
        if pending != 0 {
            self.write32(regs::INTERRUPT_ACK, pending);
        }
    }

    pub fn read_config_u64(&self, offset: usize) -> u64 {
        let lo = self.read32(regs::STATUS + 0x14 + offset) as u64; // config space follows status+0x14 in legacy mmio
        let hi = self.read32(regs::STATUS + 0x18 + offset) as u64;
        (hi << 32) | lo
    }

    pub fn version(&self) -> u32 {
        self.read32(regs::VERSION)
    }
}

/// Try to initialize a virtio-mmio block device at `base`.
pub fn try_init_mmio_blk(
    base: usize,
) -> Result<crate::drivers::virtio::blk::VirtioBlkDevice, KernelError> {
    let transport = VirtioMmioTransport::new(base);
    if !transport.matches_blk() {
        return Err(KernelError::HardwareError {
            device: "virtio-blk-mmio",
            code: 0xdead0001,
        });
    }

    // Only handle legacy/modern v1+; QEMU virt reports version 2 (modern). We
    // use split virtqueues with 64-bit addresses which are supported in v2.
    let version = transport.version();
    if version < 1 {
        return Err(KernelError::HardwareError {
            device: "virtio-blk-mmio",
            code: 0xdead0002,
        });
    }

    transport.begin_init();

    let device_features = transport.read_device_features();
    let accepted = device_features
        & (super::blk::features::VIRTIO_BLK_F_SIZE_MAX
            | super::blk::features::VIRTIO_BLK_F_SEG_MAX
            | super::blk::features::VIRTIO_BLK_F_RO
            | super::blk::features::VIRTIO_BLK_F_BLK_SIZE
            | super::blk::features::VIRTIO_BLK_F_FLUSH);
    transport.write_driver_features(accepted);

    if !transport.set_features_ok() {
        transport.set_failed();
        return Err(KernelError::HardwareError {
            device: "virtio-blk-mmio",
            code: 0xdead0004,
        });
    }

    // Queue 0 setup
    transport.select_queue(0);
    let qmax = transport.read_queue_size_max();
    if qmax == 0 {
        transport.set_failed();
        return Err(KernelError::HardwareError {
            device: "virtio-blk-mmio",
            code: 0xdead0003,
        });
    }

    let queue = crate::drivers::virtio::queue::VirtQueue::new(qmax)?;
    transport.set_queue_size(queue.size());
    transport.write_queue_phys(queue.phys_desc(), queue.phys_avail(), queue.phys_used());
    transport.set_queue_ready();

    transport.set_driver_ok();

    let capacity_sectors = transport.read_config_u64(0);
    let read_only = (accepted & super::blk::features::VIRTIO_BLK_F_RO) != 0;

    crate::println!(
        "[VIRTIO-BLK/MMIO] Initialized: {} sectors ({} KB) at {:#x}, {}",
        capacity_sectors,
        capacity_sectors * super::blk::BLOCK_SIZE as u64 / 1024,
        base,
        if read_only { "read-only" } else { "read-write" }
    );

    Ok(crate::drivers::virtio::blk::VirtioBlkDevice::from_mmio(
        transport,
        queue,
        capacity_sectors,
        read_only,
        accepted,
    ))
}

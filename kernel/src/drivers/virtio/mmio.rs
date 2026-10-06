//! Virtio MMIO transport (virtio 1.0 legacy-compatible)
//!
//! Implements the virtio-over-MMIO transport layer as defined in the
//! [virtio specification, section 4.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html).
//! This is the transport used on AArch64 and RISC-V QEMU `virt` machines,
//! where virtio devices are memory-mapped rather than behind PCI.
//!
//! # Default MMIO Base Addresses
//!
//! [`slots`] lists every virtio-mmio device region for QEMU's `virt`
//! machine. Each region is 0x200 bytes (512 bytes) and
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

// Virtio MMIO transport -- AArch64/RISC-V device access

use core::ptr;

use crate::{
    arch::barriers::{data_sync_barrier, instruction_sync_barrier},
    error::KernelError,
};

/// Every virtio-mmio slot on QEMU's `virt` machine.
///
/// - **AArch64**: 32 slots from 0x0A00_0000, 0x200 apart (`hw/arm/virt.c`).
/// - **RISC-V**: 8 slots from 0x1000_1000, 0x1000 apart (`hw/riscv/virt.c`).
///
/// QEMU fills the slots from the highest address down, so drivers must scan
/// all of them (a probe of the first four found no disk at all); each
/// driver skips slots holding another device type.
pub fn slots() -> impl Iterator<Item = usize> {
    #[cfg(target_arch = "aarch64")]
    let (base, count, stride) = (0x0a00_0000usize, 32usize, 0x200usize);
    #[cfg(target_arch = "riscv64")]
    let (base, count, stride) = (0x1000_1000usize, 8usize, 0x1000usize);
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    let (base, count, stride) = (0usize, 0usize, 0usize);
    (0..count).map(move |i| base + i * stride)
}

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
    #[allow(dead_code)] // Virtio MMIO register per spec
    pub const VENDOR_ID: usize = 0x00c;
    pub const DEVICE_FEATURES: usize = 0x010;
    pub const DEVICE_FEATURES_SEL: usize = 0x014;
    pub const DRIVER_FEATURES: usize = 0x020;
    pub const DRIVER_FEATURES_SEL: usize = 0x024;
    pub const QUEUE_SEL: usize = 0x030;
    pub const QUEUE_NUM_MAX: usize = 0x034;
    pub const QUEUE_NUM: usize = 0x038;
    /// Legacy (version 1) only: guest page size, queue alignment and the
    /// queue's page frame number.
    pub const GUEST_PAGE_SIZE: usize = 0x028;
    pub const QUEUE_ALIGN: usize = 0x03c;
    pub const QUEUE_PFN: usize = 0x040;
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
    /// Device-specific configuration space (virtio 1.x, 4.2.2).
    pub const CONFIG: usize = 0x100;
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
    #[allow(dead_code)] // Register-width API completeness
    fn write16(&self, offset: usize, value: u16) {
        // SAFETY: base + offset is an MMIO region mapped in the kernel's phys window.
        unsafe { ptr::write_volatile((self.base + offset) as *mut u16, value) }
    }

    pub fn matches_blk(&self) -> bool {
        self.matches_device(2) // 2 = block device
    }

    /// Whether a virtio-mmio device with the given device ID is present.
    pub fn matches_device(&self, device_id: u32) -> bool {
        self.read32(regs::MAGIC) == 0x7472_6976 // "virt"
            && self.read32(regs::DEVICE_ID) == device_id
    }

    pub fn begin_init(&self) {
        self.write32(regs::STATUS, 0);
        self.set_status(status::ACKNOWLEDGE | status::DRIVER);
    }

    fn set_status(&self, bits: u32) {
        let cur = self.read32(regs::STATUS);
        self.write32(regs::STATUS, cur | bits);
    }

    /// Reset the device: it stops all DMA and forgets its queues.
    pub fn reset(&self) {
        self.write32(regs::STATUS, 0);
        data_sync_barrier();
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

    /// Device feature bits 32..63 (VIRTIO_F_VERSION_1 is bit 32).
    pub fn read_device_features_hi(&self) -> u32 {
        self.write32(regs::DEVICE_FEATURES_SEL, 1);
        self.read32(regs::DEVICE_FEATURES)
    }

    /// Accept driver feature bits 32..63.
    pub fn write_driver_features_hi(&self, features: u32) {
        self.write32(regs::DRIVER_FEATURES_SEL, 1);
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
        // Version 1 has no QUEUE_READY: a non-zero QUEUE_PFN activates it.
        if !self.is_legacy() {
            self.write32(regs::QUEUE_READY, 1);
        }
    }

    /// Legacy (version 1) device: queues are addressed by page frame number.
    pub fn is_legacy(&self) -> bool {
        self.version() == 1
    }

    /// Legacy queue setup: 4 KiB pages and alignment (matching VirtQueue's
    /// layout, whose used ring starts on a page boundary), then the PFN.
    /// Modern-only registers are ignored by a legacy device, so without this
    /// a legacy device never saw the rings at all.
    pub fn write_queue_pfn(&self, pfn: u32) {
        self.write32(regs::GUEST_PAGE_SIZE, 4096);
        self.write32(regs::QUEUE_ALIGN, 4096);
        self.write32(regs::QUEUE_PFN, pfn);
        data_sync_barrier();
    }

    pub fn write_queue_phys(&self, desc: u64, avail: u64, used: u64) {
        if self.is_legacy() {
            return; // uses write_queue_pfn
        }
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

    /// Read a 64-bit device config field. Config space starts at 0x100 in
    /// both legacy and modern virtio-mmio; this used to read STATUS + 0x14
    /// (0x84, QUEUE_DESC_HIGH) instead.
    pub fn read_config_u64(&self, offset: usize) -> u64 {
        let lo = self.read32(regs::CONFIG + offset) as u64;
        let hi = self.read32(regs::CONFIG + offset + 4) as u64;
        (hi << 32) | lo
    }

    /// Read one byte of device config space.
    pub fn read_config_u8(&self, offset: usize) -> u8 {
        // SAFETY: base + CONFIG + offset lies in the device's MMIO window.
        unsafe { ptr::read_volatile((self.base + regs::CONFIG + offset) as *const u8) }
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
    if transport.is_legacy() {
        transport.write_queue_pfn(queue.pfn());
    }
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

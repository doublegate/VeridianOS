//! Virtio subsystem -- transport layer and device drivers.
//!
//! This module provides the virtio transport abstraction for VeridianOS,
//! supporting two transport backends:
//!
//! - **PCI transport** ([`VirtioPciTransport`]): Used on **x86_64**, where
//!   virtio devices appear as PCI devices with vendor ID 0x1AF4 (Red Hat). The
//!   driver accesses device registers via BAR0 I/O port space.
//!
//! - **MMIO transport** ([`mmio::VirtioMmioTransport`]): Used on **AArch64**
//!   and **RISC-V**, where QEMU's `virt` machine exposes virtio devices as
//!   memory-mapped regions starting at 0x0A00_0000.
//!
//! Both transports are unified behind the [`VirtioTransport`] enum, which
//! provides a common interface for device initialization, feature negotiation,
//! queue setup, and notification.
//!
//! # Architecture
//!
//! ```text
//!   VirtioTransport (enum)
//!     |-- Pci(VirtioPciTransport)      -- x86_64 via I/O ports (BAR0)
//!     |-- Mmio(VirtioMmioTransport)    -- AArch64/RISC-V via MMIO
//!     |
//!     +-- VirtQueue (queue.rs)         -- split virtqueue (shared)
//!     +-- VirtioBlkDevice (blk.rs)     -- block device driver (shared)
//! ```
//!
//! # Legacy PCI Layout (BAR0 I/O Space)
//!
//! The PCI transport uses the legacy (transitional) virtio PCI interface as
//! described in the virtio 1.0 specification, section 4.1:
//!
//! | Offset | Size | Name            |
//! |--------|------|-----------------|
//! | 0x00   | 4    | device_features |
//! | 0x04   | 4    | guest_features  |
//! | 0x08   | 4    | queue_address   |
//! | 0x0C   | 2    | queue_size      |
//! | 0x0E   | 2    | queue_select    |
//! | 0x10   | 2    | queue_notify    |
//! | 0x12   | 1    | device_status   |
//! | 0x13   | 1    | isr_status      |
//! | 0x14+  | var  | device config   |
//!
//! For the MMIO register layout, see [`mmio`].

#![allow(dead_code)]

pub mod blk;
pub mod mmio;
pub mod queue;

/// Virtio vendor ID (Red Hat, Inc.)
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

/// Virtio-blk PCI device IDs
/// Legacy device ID (virtio 0.9 / transitional)
pub const VIRTIO_BLK_DEVICE_ID_LEGACY: u16 = 0x1001;
/// Modern device ID (virtio 1.0+, transitional)
pub const VIRTIO_BLK_DEVICE_ID_MODERN: u16 = 0x1042;

/// Unified transport enum for virtio-blk
#[derive(Debug, Clone, Copy)]
pub enum VirtioTransport {
    Pci(VirtioPciTransport),
    Mmio(crate::drivers::virtio::mmio::VirtioMmioTransport),
}

impl VirtioTransport {
    pub fn begin_init(&self) {
        match self {
            Self::Pci(p) => p.begin_init(),
            Self::Mmio(m) => m.begin_init(),
        }
    }

    pub fn write_guest_features(&self, features: u32) {
        match self {
            Self::Pci(p) => p.write_guest_features(features),
            Self::Mmio(m) => m.write_driver_features(features),
        }
    }

    pub fn read_device_features(&self) -> u32 {
        match self {
            Self::Pci(p) => p.read_device_features(),
            Self::Mmio(m) => m.read_device_features(),
        }
    }

    pub fn set_features_ok(&self) -> bool {
        match self {
            Self::Pci(p) => p.set_features_ok(),
            Self::Mmio(m) => m.set_features_ok(),
        }
    }

    pub fn select_queue(&self, idx: u16) {
        match self {
            Self::Pci(p) => p.select_queue(idx),
            Self::Mmio(m) => m.select_queue(idx),
        }
    }

    pub fn read_queue_size(&self) -> u16 {
        match self {
            Self::Pci(p) => p.read_queue_size(),
            Self::Mmio(m) => m.read_queue_size_max(),
        }
    }

    pub fn write_queue_address(&self, pfn: u32) {
        match self {
            Self::Pci(p) => p.write_queue_address(pfn),
            Self::Mmio(_) => {} // mmio uses 64-bit phys addresses via write_queue_phys
        }
    }

    pub fn write_queue_phys(&self, desc: u64, avail: u64, used: u64) {
        match self {
            Self::Pci(_) => {}
            Self::Mmio(m) => m.write_queue_phys(desc, avail, used),
        }
    }

    pub fn set_queue_ready(&self) {
        match self {
            Self::Pci(_) => {}
            Self::Mmio(m) => m.set_queue_ready(),
        }
    }

    pub fn set_driver_ok(&self) {
        match self {
            Self::Pci(p) => p.set_driver_ok(),
            Self::Mmio(m) => m.set_driver_ok(),
        }
    }

    pub fn notify_queue(&self, idx: u16) {
        match self {
            Self::Pci(p) => p.notify_queue(idx),
            Self::Mmio(m) => m.notify_queue(idx),
        }
    }

    pub fn read_device_config_u64(&self, offset: u16) -> u64 {
        match self {
            Self::Pci(p) => p.read_device_config_u64(offset),
            Self::Mmio(m) => m.read_config_u64(offset as usize),
        }
    }
}

/// Virtio device status flags (virtio spec 2.1)
pub mod status {
    /// Guest OS has found the device and recognized it as a valid virtio
    /// device.
    pub const ACKNOWLEDGE: u8 = 1;
    /// Guest OS knows how to drive the device.
    pub const DRIVER: u8 = 2;
    /// Driver is ready (feature negotiation complete).
    pub const DRIVER_OK: u8 = 4;
    /// Feature negotiation is complete.
    pub const FEATURES_OK: u8 = 8;
    /// Something went wrong; device has given up on the driver.
    pub const DEVICE_NEEDS_RESET: u8 = 64;
    /// Driver has given up on the device.
    pub const FAILED: u8 = 128;
}

/// Legacy virtio PCI register offsets (I/O space via BAR0)
pub mod regs {
    /// Device features (read-only, 32-bit)
    pub const DEVICE_FEATURES: u16 = 0x00;
    /// Guest (driver) features (read/write, 32-bit)
    pub const GUEST_FEATURES: u16 = 0x04;
    /// Queue address (PFN of virtqueue, 32-bit)
    pub const QUEUE_ADDRESS: u16 = 0x08;
    /// Queue size (number of entries, 16-bit, read-only)
    pub const QUEUE_SIZE: u16 = 0x0C;
    /// Queue select (16-bit, write selects active queue)
    pub const QUEUE_SELECT: u16 = 0x0E;
    /// Queue notify (16-bit, write kicks the selected queue)
    pub const QUEUE_NOTIFY: u16 = 0x10;
    /// Device status (8-bit)
    pub const DEVICE_STATUS: u16 = 0x12;
    /// ISR status (8-bit, read clears)
    pub const ISR_STATUS: u16 = 0x13;
    /// Start of device-specific configuration space
    pub const DEVICE_CONFIG: u16 = 0x14;
}

/// Virtio PCI transport handle.
///
/// Wraps the BAR0 I/O port base address for a legacy virtio PCI device and
/// provides typed accessors for the common virtio register set.
#[derive(Debug, Clone, Copy)]
pub struct VirtioPciTransport {
    /// BAR0 I/O port base address
    io_base: u16,
}

impl VirtioPciTransport {
    /// Create a new transport from the BAR0 I/O base address.
    pub fn new(io_base: u16) -> Self {
        Self { io_base }
    }

    /// Get the I/O base address.
    pub fn io_base(&self) -> u16 {
        self.io_base
    }

    // ---- Register accessors ----

    /// Read the device-offered feature bits (32-bit).
    pub fn read_device_features(&self) -> u32 {
        self.read32(regs::DEVICE_FEATURES)
    }

    /// Write the driver-accepted feature bits (32-bit).
    pub fn write_guest_features(&self, features: u32) {
        self.write32(regs::GUEST_FEATURES, features);
    }

    /// Read the queue size for the currently selected queue.
    pub fn read_queue_size(&self) -> u16 {
        self.read16(regs::QUEUE_SIZE)
    }

    /// Select a virtqueue by index.
    pub fn select_queue(&self, index: u16) {
        self.write16(regs::QUEUE_SELECT, index);
    }

    /// Set the physical page frame number (PFN) of the selected virtqueue.
    ///
    /// The device uses this to locate the virtqueue descriptor table, available
    /// ring, and used ring in guest physical memory. The address is `pfn *
    /// 4096`.
    pub fn write_queue_address(&self, pfn: u32) {
        self.write32(regs::QUEUE_ADDRESS, pfn);
    }

    /// Notify (kick) the device that new buffers are available in the given
    /// queue.
    pub fn notify_queue(&self, queue_index: u16) {
        self.write16(regs::QUEUE_NOTIFY, queue_index);
    }

    /// Read the device status register.
    pub fn read_status(&self) -> u8 {
        self.read8(regs::DEVICE_STATUS)
    }

    /// Write the device status register.
    pub fn write_status(&self, status: u8) {
        self.write8(regs::DEVICE_STATUS, status);
    }

    /// Read the ISR status register (clears interrupt flag on read).
    pub fn read_isr(&self) -> u8 {
        self.read8(regs::ISR_STATUS)
    }

    /// Read a byte from device-specific configuration space.
    pub fn read_device_config_u8(&self, offset: u16) -> u8 {
        self.read8(regs::DEVICE_CONFIG + offset)
    }

    /// Read a 32-bit word from device-specific configuration space.
    pub fn read_device_config_u32(&self, offset: u16) -> u32 {
        self.read32(regs::DEVICE_CONFIG + offset)
    }

    /// Read a 64-bit value from device-specific configuration space (two 32-bit
    /// reads).
    pub fn read_device_config_u64(&self, offset: u16) -> u64 {
        let low = self.read32(regs::DEVICE_CONFIG + offset) as u64;
        let high = self.read32(regs::DEVICE_CONFIG + offset + 4) as u64;
        low | (high << 32)
    }

    // ---- Device initialization protocol (virtio spec 3.1.1) ----

    /// Reset the device by writing zero to the status register.
    pub fn reset(&self) {
        self.write_status(0);
    }

    /// Perform the standard legacy device initialization sequence.
    ///
    /// 1. Reset device
    /// 2. Set ACKNOWLEDGE
    /// 3. Set DRIVER
    ///
    /// After calling this, the driver should read device features, negotiate,
    /// then call `set_features_ok()` and `set_driver_ok()`.
    pub fn begin_init(&self) {
        // Step 1: Reset
        self.reset();

        // Step 2: Acknowledge -- we recognize this as a virtio device
        self.write_status(status::ACKNOWLEDGE);

        // Step 3: Driver -- we know how to drive this device type
        self.write_status(status::ACKNOWLEDGE | status::DRIVER);
    }

    /// Signal that feature negotiation is complete.
    ///
    /// Returns `true` if the device accepted FEATURES_OK; `false` means the
    /// device does not support the selected feature subset and initialization
    /// should be aborted.
    pub fn set_features_ok(&self) -> bool {
        let current = self.read_status();
        self.write_status(current | status::FEATURES_OK);

        // Re-read to confirm the device accepted
        (self.read_status() & status::FEATURES_OK) != 0
    }

    /// Signal that the driver is fully initialized and ready.
    pub fn set_driver_ok(&self) {
        let current = self.read_status();
        self.write_status(current | status::DRIVER_OK);
    }

    /// Mark the device as failed.
    pub fn set_failed(&self) {
        let current = self.read_status();
        self.write_status(current | status::FAILED);
    }

    // ---- Low-level I/O port helpers ----

    fn read8(&self, offset: u16) -> u8 {
        // SAFETY: Reading a virtio PCI I/O register at io_base + offset. The
        // io_base was obtained from a PCI BAR0 I/O space mapping. We are in
        // kernel mode with full I/O privilege.
        unsafe { crate::arch::inb(self.io_base + offset) }
    }

    fn write8(&self, offset: u16, value: u8) {
        // SAFETY: Writing a virtio PCI I/O register. Same invariants as read8.
        unsafe { crate::arch::outb(self.io_base + offset, value) }
    }

    fn read16(&self, offset: u16) -> u16 {
        // SAFETY: Reading a 16-bit virtio PCI I/O register. Same invariants.
        unsafe { crate::arch::inw(self.io_base + offset) }
    }

    fn write16(&self, offset: u16, value: u16) {
        // SAFETY: Writing a 16-bit virtio PCI I/O register. Same invariants.
        unsafe { crate::arch::outw(self.io_base + offset, value) }
    }

    fn read32(&self, offset: u16) -> u32 {
        // SAFETY: Reading a 32-bit virtio PCI I/O register. Same invariants.
        unsafe { crate::arch::inl(self.io_base + offset) }
    }

    fn write32(&self, offset: u16, value: u32) {
        // SAFETY: Writing a 32-bit virtio PCI I/O register. Same invariants.
        unsafe { crate::arch::outl(self.io_base + offset, value) }
    }
}

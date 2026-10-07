//! Virtio-blk device driver
//!
//! Implements a block device driver for virtio-blk PCI devices as described
//! in the virtio specification, section 5.2. Supports read and write operations
//! using the legacy (transitional) PCI transport.
//!
//! # Virtio-blk request format
//!
//! Each request is a three-descriptor chain:
//!
//! 1. **Header** (device-readable): `VirtioBlkReqHeader` with request type +
//!    sector
//! 2. **Data** (device-readable for write, device-writable for read): sector
//!    data
//! 3. **Status** (device-writable): single byte result (0 = OK, 1 = IOERR, 2 =
//!    UNSUPP)
//!
//! # QEMU usage
//!
//! ```text
//! -drive file=disk.img,if=none,id=vd0,format=raw -device virtio-blk-pci,drive=vd0
//! ```

// Virtio-blk driver -- exercised when block device is attached

use core::sync::atomic::{self, Ordering};

use spin::Mutex;

use super::{
    queue::{VirtQueue, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE},
    VirtioPciTransport, VirtioTransport,
};
use crate::{error::KernelError, mm::FRAME_SIZE, sync::once_lock::OnceLock};

/// Block size in bytes (standard sector)
pub const BLOCK_SIZE: usize = 512;

/// Maximum number of sectors per single request
#[allow(dead_code)] // Virtio-blk request size limit per spec
const MAX_SECTORS_PER_REQ: usize = 256;

/// Virtio-blk feature bits (virtio spec 5.2.3)
pub mod features {
    /// Maximum size of any single segment is in `size_max`.
    pub const VIRTIO_BLK_F_SIZE_MAX: u32 = 1 << 1;
    /// Maximum number of segments in a request is in `seg_max`.
    pub const VIRTIO_BLK_F_SEG_MAX: u32 = 1 << 2;
    /// Disk-style geometry specified in geometry.
    pub const VIRTIO_BLK_F_GEOMETRY: u32 = 1 << 4;
    /// Device is read-only.
    pub const VIRTIO_BLK_F_RO: u32 = 1 << 5;
    /// Block size of disk is in `blk_size`.
    pub const VIRTIO_BLK_F_BLK_SIZE: u32 = 1 << 6;
    /// Cache flush command support.
    pub const VIRTIO_BLK_F_FLUSH: u32 = 1 << 9;
}

/// Virtio-blk request types (virtio spec 5.2.6)
mod req_type {
    /// Read sectors from the device
    pub const VIRTIO_BLK_T_IN: u32 = 0;
    /// Write sectors to the device
    pub const VIRTIO_BLK_T_OUT: u32 = 1;
    /// Flush volatile write cache
    #[allow(dead_code)] // Virtio-blk command type per spec
    pub const VIRTIO_BLK_T_FLUSH: u32 = 4;
}

/// Virtio-blk status values (returned in the status byte)
mod blk_status {
    /// Request completed successfully
    pub const VIRTIO_BLK_S_OK: u8 = 0;
    /// I/O error
    pub const VIRTIO_BLK_S_IOERR: u8 = 1;
    /// Unsupported request type
    pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;
}

/// Virtio-blk request header, sent as the first descriptor in each request
/// chain.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioBlkReqHeader {
    /// Request type: VIRTIO_BLK_T_IN (read) or VIRTIO_BLK_T_OUT (write)
    type_: u32,
    /// Reserved field (must be zero)
    reserved: u32,
    /// Starting sector (512-byte units)
    sector: u64,
}

/// Largest transfer per request: one page of data.
pub const MAX_REQUEST_BYTES: usize = FRAME_SIZE;

/// Offset of the status byte within the control frame (after the header).
const STATUS_OFFSET: usize = 64;

/// DMA memory reused by every request (DRV-PERF-02): a control frame
/// holding the request header and status byte, and a data frame. The old
/// driver allocated and zeroed a fresh frame for each 512-byte request.
struct RequestFrames {
    ctrl: crate::drivers::dma_frame::DmaFrame,
    data: crate::drivers::dma_frame::DmaFrame,
}

impl RequestFrames {
    fn new() -> Result<Self, KernelError> {
        Ok(Self {
            ctrl: crate::drivers::dma_frame::DmaFrame::alloc()?,
            data: crate::drivers::dma_frame::DmaFrame::alloc()?,
        })
    }

    fn write_header(&self, type_: u32, sector: u64) {
        let header = VirtioBlkReqHeader {
            type_,
            reserved: 0,
            sector,
        };
        // SAFETY: the control frame is ours and holds the header at 0.
        unsafe { core::ptr::write_volatile(self.ctrl.virt as *mut VirtioBlkReqHeader, header) };
    }

    fn status_phys(&self) -> u64 {
        self.ctrl.phys + STATUS_OFFSET as u64
    }

    fn set_status(&self, value: u8) {
        // SAFETY: STATUS_OFFSET is inside the control frame.
        unsafe { core::ptr::write_volatile((self.ctrl.virt + STATUS_OFFSET) as *mut u8, value) };
    }

    fn status(&self) -> u8 {
        // SAFETY: as set_status; the device writes it (volatile read).
        unsafe { core::ptr::read_volatile((self.ctrl.virt + STATUS_OFFSET) as *const u8) }
    }
}

/// Virtio block device.
///
/// Manages a single virtio-blk PCI device with one request virtqueue (queue 0).
pub struct VirtioBlkDevice {
    /// Transport handle (PCI or MMIO)
    transport: VirtioTransport,
    /// Request virtqueue (queue index 0)
    queue: VirtQueue,
    /// Device capacity in 512-byte sectors
    capacity_sectors: u64,
    /// Whether the device is read-only (VIRTIO_BLK_F_RO)
    read_only: bool,
    /// Negotiated features
    #[allow(dead_code)] // Negotiated feature bits for device capabilities
    features: u32,
    /// Set after a request timed out with the device still owning its
    /// buffer and descriptors. The queue state is then unknown (a late
    /// completion would be taken for the next request's), so every later
    /// request fails instead (N-10).
    failed: bool,
    /// Request DMA memory, allocated on first use and reused.
    frames: Option<RequestFrames>,
}

impl VirtioBlkDevice {
    /// Probe and initialize a virtio-blk device at the given PCI BAR0 I/O base.
    ///
    /// Performs the full legacy virtio initialization sequence:
    /// 1. Reset + ACKNOWLEDGE + DRIVER
    /// 2. Read and negotiate features
    /// 3. Set up virtqueue 0 (request queue)
    /// 4. Set FEATURES_OK + DRIVER_OK
    /// 5. Read device configuration (capacity)
    pub fn new(io_base: u16) -> Result<Self, KernelError> {
        let transport = VirtioTransport::Pci(VirtioPciTransport::new(io_base));

        // Step 1-2: Begin initialization (reset + ACKNOWLEDGE + DRIVER)
        transport.begin_init();

        // Step 3: Read and negotiate features
        let device_features = transport.read_device_features();
        let accepted = device_features
            & (features::VIRTIO_BLK_F_SIZE_MAX
                | features::VIRTIO_BLK_F_SEG_MAX
                | features::VIRTIO_BLK_F_RO
                | features::VIRTIO_BLK_F_BLK_SIZE
                | features::VIRTIO_BLK_F_FLUSH);
        transport.write_guest_features(accepted);

        let read_only = (accepted & features::VIRTIO_BLK_F_RO) != 0;

        // Step 4: Set FEATURES_OK (legacy devices may not support this; proceed anyway)
        let _features_ok = transport.set_features_ok();

        // Step 5: Set up virtqueue 0
        transport.select_queue(0);
        let queue_size = transport.read_queue_size();
        if queue_size == 0 {
            return Err(KernelError::HardwareError {
                device: "virtio-blk",
                code: 0x01, // Queue size is zero -- no queue available
            });
        }

        let queue = VirtQueue::new(queue_size)?;
        transport.write_queue_address(queue.pfn());
        transport.write_queue_phys(queue.phys_desc(), queue.phys_avail(), queue.phys_used());
        transport.set_queue_ready();

        // Step 6: Set DRIVER_OK -- device is live
        transport.set_driver_ok();

        // Step 7: Read device configuration -- capacity in sectors
        // Legacy virtio-blk config starts at offset 0x14 (after common registers):
        //   offset 0x00 (relative to config base): capacity (u64, in 512-byte sectors)
        let capacity_sectors = transport.read_device_config_u64(0);

        crate::println!(
            "[VIRTIO-BLK] Initialized: {} sectors ({} KB), {}",
            capacity_sectors,
            capacity_sectors * BLOCK_SIZE as u64 / 1024,
            if read_only { "read-only" } else { "read-write" }
        );

        Ok(Self {
            transport,
            queue,
            capacity_sectors,
            read_only,
            features: accepted,
            failed: false,
            frames: None,
        })
    }

    /// Construct from an MMIO transport + queue (used on AArch64/RISC-V).
    pub fn from_mmio(
        transport: crate::drivers::virtio::mmio::VirtioMmioTransport,
        queue: VirtQueue,
        capacity_sectors: u64,
        read_only: bool,
        features: u32,
    ) -> Self {
        Self {
            transport: VirtioTransport::Mmio(transport),
            queue,
            capacity_sectors,
            read_only,
            features,
            failed: false,
            frames: None,
        }
    }

    /// Get device capacity in 512-byte sectors.
    pub fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    /// Get device capacity in bytes.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_sectors * BLOCK_SIZE as u64
    }

    /// Check if the device is read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Read a single block (512 bytes) from the device.
    ///
    /// `block_num` is the 0-based sector number. `buf` must be at least 512
    /// bytes.
    pub fn read_block(&mut self, block_num: u64, buf: &mut [u8]) -> Result<(), KernelError> {
        if buf.len() < BLOCK_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "buf",
                value: "buffer must be at least 512 bytes",
            });
        }
        self.read_sectors(block_num, &mut buf[..BLOCK_SIZE])
    }

    /// Write a single block (512 bytes) to the device.
    ///
    /// `block_num` is the 0-based sector number. `data` must be at least 512
    /// bytes.
    pub fn write_block(&mut self, block_num: u64, data: &[u8]) -> Result<(), KernelError> {
        if data.len() < BLOCK_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "data must be at least 512 bytes",
            });
        }
        self.write_sectors(block_num, &data[..BLOCK_SIZE])
    }

    /// Read `buf.len()` bytes (a multiple of 512, at most one page) starting
    /// at `sector`, in one request.
    pub fn read_sectors(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), KernelError> {
        self.check_range(sector, buf.len())?;
        self.do_request(req_type::VIRTIO_BLK_T_IN, sector, buf.len())?;
        let frames = self.frames.as_ref().ok_or(KernelError::InvalidState {
            expected: "request frames",
            actual: "none",
        })?;
        buf.copy_from_slice(&frames.data.bytes()[..buf.len()]);
        Ok(())
    }

    /// Write `data` (a multiple of 512 bytes, at most one page) starting at
    /// `sector`, in one request.
    pub fn write_sectors(&mut self, sector: u64, data: &[u8]) -> Result<(), KernelError> {
        if self.read_only {
            return Err(KernelError::PermissionDenied {
                operation: "write to read-only virtio-blk device",
            });
        }
        self.check_range(sector, data.len())?;
        if self.frames.is_none() {
            self.frames = Some(RequestFrames::new()?);
        }
        if let Some(frames) = self.frames.as_mut() {
            frames.data.bytes_mut()[..data.len()].copy_from_slice(data);
        }
        self.do_request(req_type::VIRTIO_BLK_T_OUT, sector, data.len())
    }

    /// A transfer must be whole sectors, at most one page, inside the disk.
    fn check_range(&self, sector: u64, len: usize) -> Result<(), KernelError> {
        if len == 0 || !len.is_multiple_of(BLOCK_SIZE) || len > MAX_REQUEST_BYTES {
            return Err(KernelError::InvalidArgument {
                name: "len",
                value: "must be 1..=8 whole sectors",
            });
        }
        let end =
            sector
                .checked_add((len / BLOCK_SIZE) as u64)
                .ok_or(KernelError::InvalidArgument {
                    name: "sector",
                    value: "overflows",
                })?;
        if end > self.capacity_sectors {
            return Err(KernelError::InvalidArgument {
                name: "sector",
                value: "range exceeds device capacity",
            });
        }
        Ok(())
    }

    /// Submit one request for `len` data bytes (already in the data frame
    /// for writes; left there for reads) and poll for completion.
    fn do_request(&mut self, type_: u32, sector: u64, len: usize) -> Result<(), KernelError> {
        if self.failed {
            return Err(KernelError::HardwareError {
                device: "virtio-blk",
                code: 0x03, // device quarantined after a request timeout
            });
        }
        if self.frames.is_none() {
            self.frames = Some(RequestFrames::new()?);
        }
        let (header_phys, data_phys, status_phys) = {
            let frames = self.frames.as_ref().ok_or(KernelError::InvalidState {
                expected: "request frames",
                actual: "none",
            })?;
            frames.write_header(type_, sector);
            // Anything but OK if the device never writes the status.
            frames.set_status(0xFF);
            (frames.ctrl.phys, frames.data.phys, frames.status_phys())
        };

        // Build a 3-descriptor chain:
        //   [0] header (device-readable)
        //   [1] data   (device-writable for read, device-readable for write)
        //   [2] status (device-writable)
        let desc_header = self
            .queue
            .alloc_desc()
            .ok_or(KernelError::ResourceExhausted {
                resource: "virtio-blk descriptors",
            })?;
        let desc_data = match self.queue.alloc_desc() {
            Some(d) => d,
            None => {
                self.queue.free_desc(desc_header);
                return Err(KernelError::ResourceExhausted {
                    resource: "virtio-blk descriptors",
                });
            }
        };
        let desc_status = match self.queue.alloc_desc() {
            Some(d) => d,
            None => {
                self.queue.free_desc(desc_header);
                self.queue.free_desc(desc_data);
                return Err(KernelError::ResourceExhausted {
                    resource: "virtio-blk descriptors",
                });
            }
        };

        // SAFETY: the three descriptors were just allocated; the addresses
        // are the device-owned request frames, sized for these lengths
        // (header 16 bytes, data `len` <= one frame, status 1 byte).
        unsafe {
            self.queue.write_desc(
                desc_header,
                header_phys,
                core::mem::size_of::<VirtioBlkReqHeader>() as u32,
                VIRTQ_DESC_F_NEXT,
                desc_data,
            );
            let data_flags = if type_ == req_type::VIRTIO_BLK_T_IN {
                VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT // device writes data
            } else {
                VIRTQ_DESC_F_NEXT // device reads data
            };
            self.queue
                .write_desc(desc_data, data_phys, len as u32, data_flags, desc_status);
            self.queue
                .write_desc(desc_status, status_phys, 1, VIRTQ_DESC_F_WRITE, 0);
        }

        // Ensure all descriptor writes are visible before notifying
        atomic::fence(Ordering::Release);
        self.queue.push_avail(desc_header);
        self.transport.notify_queue(0);

        // Poll for completion
        let mut spins: u32 = 0;
        const MAX_SPINS: u32 = 10_000_000;
        while !self.queue.has_used() {
            core::hint::spin_loop();
            spins += 1;
            if spins >= MAX_SPINS {
                // The device may still DMA into the request frames and still
                // owns the descriptor chain: freeing either would let it
                // corrupt reallocated memory or a later request. Leak both
                // and stop using the device (N-10).
                core::mem::forget(self.frames.take());
                self.failed = true;
                return Err(KernelError::Timeout {
                    operation: "virtio-blk request",
                    duration_ms: 0,
                });
            }
        }

        // Consume the used entry. Only a completion naming our chain head
        // proves the device is done with the chain and the request frames;
        // on a mismatch or an invalid id it may still own both, so quarantine
        // them as the timeout path does (review of the v0.26.0 stack, PR #12).
        let used = self.queue.poll_used();
        if !completion_is_ours(used, desc_header) {
            core::mem::forget(self.frames.take());
            self.failed = true;
            return Err(KernelError::HardwareError {
                device: "virtio-blk",
                code: 0x02, // Used ring entry invalid or not ours
            });
        }
        self.queue.free_chain(desc_header);

        let status = self.frames.as_ref().map_or(0xFF, |f| f.status());
        match status {
            blk_status::VIRTIO_BLK_S_OK => Ok(()),
            blk_status::VIRTIO_BLK_S_IOERR => Err(KernelError::HardwareError {
                device: "virtio-blk",
                code: 0x10, // I/O error
            }),
            blk_status::VIRTIO_BLK_S_UNSUPP => Err(KernelError::OperationNotSupported {
                operation: "virtio-blk unsupported request type",
            }),
            _ => Err(KernelError::HardwareError {
                device: "virtio-blk",
                code: status as u32,
            }),
        }
    }
}

/// Block device trait for generic block I/O operations.
pub trait BlockDevice: Send + Sync {
    /// Read a block (512 bytes) at the given sector number.
    fn read_block(&mut self, block_num: u64, buf: &mut [u8]) -> Result<(), KernelError>;

    /// Write a block (512 bytes) at the given sector number.
    fn write_block(&mut self, block_num: u64, data: &[u8]) -> Result<(), KernelError>;

    /// Get the device capacity in sectors.
    fn capacity_sectors(&self) -> u64;

    /// Get the block size in bytes.
    fn block_size(&self) -> usize {
        BLOCK_SIZE
    }

    /// Check if the device is read-only.
    fn is_read_only(&self) -> bool;
}

impl BlockDevice for VirtioBlkDevice {
    fn read_block(&mut self, block_num: u64, buf: &mut [u8]) -> Result<(), KernelError> {
        VirtioBlkDevice::read_block(self, block_num, buf)
    }

    fn write_block(&mut self, block_num: u64, data: &[u8]) -> Result<(), KernelError> {
        VirtioBlkDevice::write_block(self, block_num, data)
    }

    fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }
}

// ---------------------------------------------------------------------------
// Global driver instance and initialization
// ---------------------------------------------------------------------------

/// Global virtio-blk device instance (if a device was found and initialized).
static VIRTIO_BLK: OnceLock<Mutex<VirtioBlkDevice>> = OnceLock::new();

/// Probe PCI bus for virtio-blk devices and initialize the first one found.
///
/// This is only meaningful on x86_64 where PCI I/O port access works. On
/// AArch64 and RISC-V, this function is a no-op stub (virtio-mmio transport
/// would be needed instead).
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    init_x86_64();

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    init_mmio();
}

/// x86_64 PCI-based virtio-blk initialization.
#[cfg(target_arch = "x86_64")]
fn init_x86_64() {
    use crate::drivers::pci;

    if !pci::is_pci_initialized() {
        crate::println!("[VIRTIO-BLK] PCI bus not initialized, skipping");
        return;
    }

    let pci_bus = pci::get_pci_bus().lock();

    // Search for virtio-blk devices (vendor 0x1AF4, device 0x1001 or 0x1042)
    // We only support one virtio-blk device; stop after the first successful init.
    let all_devices = pci_bus.get_all_devices();
    drop(pci_bus); // Release PCI lock before performing device init

    for device in &all_devices {
        if device.vendor_id != super::VIRTIO_VENDOR_ID {
            continue;
        }
        if device.device_id != super::VIRTIO_BLK_DEVICE_ID_LEGACY
            && device.device_id != super::VIRTIO_BLK_DEVICE_ID_MODERN
        {
            continue;
        }

        crate::println!(
            "[VIRTIO-BLK] Found device at {}:{}:{} (ID {:04x}:{:04x})",
            device.location.bus,
            device.location.device,
            device.location.function,
            device.vendor_id,
            device.device_id,
        );

        // Get BAR0 I/O port address
        let io_base = match device.bars.first() {
            Some(bar) => match bar.get_io_address() {
                Some(addr) => addr as u16,
                None => {
                    // Legacy devices sometimes use I/O BARs,
                    // but QEMU may present an I/O BAR at BAR0.
                    crate::println!("[VIRTIO-BLK] BAR0 is not an I/O BAR, skipping device");
                    continue;
                }
            },
            None => {
                crate::println!("[VIRTIO-BLK] No BAR0 found, skipping device");
                continue;
            }
        };

        // Enable I/O space, memory space, and bus mastering
        enable_bus_master(device);

        match VirtioBlkDevice::new(io_base) {
            Ok(dev) => {
                let _ = VIRTIO_BLK.set(Mutex::new(dev));
                crate::println!("[VIRTIO-BLK] Device initialized and registered");
            }
            Err(e) => {
                crate::println!("[VIRTIO-BLK] Failed to initialize device: {:?}", e);
            }
        }

        // We only support one virtio-blk device for now
        return;
    }

    crate::println!("[VIRTIO-BLK] No virtio-blk devices found on PCI bus");
}

/// AArch64 / RISC-V virtio-mmio initialization.
///
/// Probes the architecture-specific MMIO base addresses for a virtio-blk
/// device. On AArch64, these are at 0x0A00_0000 with 0x200 stride; on
/// RISC-V, at 0x1000_1000 with 0x1000 stride. See
/// [`super::mmio::slots`].
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn init_mmio() {
    use crate::drivers::virtio::mmio::{slots, try_init_mmio_blk};

    // Probe the standard virtio-mmio base addresses exposed by QEMU virt.
    for base in slots() {
        match try_init_mmio_blk(base) {
            Ok(dev) => {
                if VIRTIO_BLK.set(Mutex::new(dev)).is_ok() {
                    crate::println!("[VIRTIO-BLK/MMIO] Device initialized at base {:#x}", base);
                    return;
                }
            }
            Err(_) => continue,
        }
    }

    crate::println!("[VIRTIO-BLK/MMIO] No virtio-blk mmio device detected");
}

/// Enable PCI I/O space, memory space, and bus mastering for a device.
#[cfg(target_arch = "x86_64")]
fn enable_bus_master(device: &crate::drivers::pci::PciDevice) {
    let loc = device.location;
    let config_addr = loc.to_config_address() | (0x04 & 0xFC); // Command register at offset 0x04

    // SAFETY: Reading and writing PCI configuration space via mechanism #1
    // (ports 0xCF8/0xCFC). We are in kernel mode with full I/O privilege.
    unsafe {
        crate::arch::outl(0xCF8, config_addr);
        let cmd = crate::arch::inl(0xCFC);
        // Set bit 0 (I/O Space), bit 1 (Memory Space), bit 2 (Bus Master)
        // Only modify the lower 16 bits (Command register); preserve upper
        // 16 bits (Status register) as zeros to avoid W1C side-effects.
        let new_cmd = (cmd & 0xFFFF) | 0x07;
        crate::arch::outl(0xCF8, config_addr);
        crate::arch::outl(0xCFC, new_cmd);
    }
}

/// Whether a used-ring element completes the request whose chain starts at
/// `head`. Only one request is in flight at a time, so any other id (or an
/// id the queue rejected as out of range, `None`) is a device fault.
fn completion_is_ours(used: Option<(u16, u32)>, head: u16) -> bool {
    matches!(used, Some((id, _)) if id == head)
}

/// Get a reference to the global virtio-blk device, if initialized.
pub fn get_device() -> Option<&'static Mutex<VirtioBlkDevice>> {
    VIRTIO_BLK.get()
}

/// Check if a virtio-blk device has been initialized.
pub fn is_initialized() -> bool {
    VIRTIO_BLK.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_must_name_the_request_head() {
        assert!(completion_is_ours(Some((3, 513)), 3));
        assert!(!completion_is_ours(Some((4, 513)), 3));
        assert!(!completion_is_ours(None, 3));
    }
}

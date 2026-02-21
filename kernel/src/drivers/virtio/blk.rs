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
#![allow(dead_code)]

use core::sync::atomic::{self, Ordering};

use spin::Mutex;

use super::{
    queue::{VirtQueue, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE},
    VirtioPciTransport, VirtioTransport,
};
use crate::{
    error::KernelError,
    mm::{FRAME_ALLOCATOR, FRAME_SIZE},
    sync::once_lock::OnceLock,
};

/// Block size in bytes (standard sector)
pub const BLOCK_SIZE: usize = 512;

/// Maximum number of sectors per single request
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

/// A DMA buffer for a single virtio-blk request.
///
/// Holds the physical memory for the header, data, and status byte so they
/// remain valid while the device processes the request.
struct RequestBuffer {
    /// Physical address of the header
    header_phys: u64,
    /// Virtual address of the header (for writing from CPU)
    header_virt: usize,
    /// Physical address of the data region
    data_phys: u64,
    /// Virtual address of the data region
    data_virt: usize,
    /// Physical address of the status byte
    status_phys: u64,
    /// Virtual address of the status byte
    status_virt: usize,
    /// Frame allocated for the request buffer
    frame: crate::mm::FrameNumber,
}

impl RequestBuffer {
    /// Allocate a request buffer from the frame allocator.
    ///
    /// Layout within a single 4KB frame:
    /// - [0..16): VirtioBlkReqHeader (16 bytes)
    /// - [16..16+data_len): Data buffer
    /// - [16+data_len]: Status byte (1 byte)
    fn new(data_len: usize) -> Result<Self, KernelError> {
        let total = core::mem::size_of::<VirtioBlkReqHeader>() + data_len + 1;
        if total > FRAME_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "data_len",
                value: "request buffer exceeds single frame",
            });
        }

        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: FRAME_SIZE,
                available: 0,
            })?;

        let phys_base = frame.as_u64() * FRAME_SIZE as u64;
        let virt_base = phys_to_kernel_virt(phys_base);

        // Zero the frame
        // SAFETY: virt_base points to a freshly allocated, kernel-accessible
        // frame. No other references exist.
        unsafe {
            core::ptr::write_bytes(virt_base as *mut u8, 0, FRAME_SIZE);
        }

        let header_offset = 0;
        let data_offset = core::mem::size_of::<VirtioBlkReqHeader>();
        let status_offset = data_offset + data_len;

        Ok(Self {
            header_phys: phys_base + header_offset as u64,
            header_virt: virt_base + header_offset,
            data_phys: phys_base + data_offset as u64,
            data_virt: virt_base + data_offset,
            status_phys: phys_base + status_offset as u64,
            status_virt: virt_base + status_offset,
            frame,
        })
    }

    /// Write the request header.
    fn write_header(&self, type_: u32, sector: u64) {
        let header = VirtioBlkReqHeader {
            type_,
            reserved: 0,
            sector,
        };
        // SAFETY: header_virt points to valid memory within our allocated frame.
        // No other references to this memory exist.
        unsafe {
            core::ptr::write_volatile(self.header_virt as *mut VirtioBlkReqHeader, header);
        }
    }

    /// Write data into the data region (for write requests).
    fn write_data(&self, data: &[u8]) {
        // SAFETY: data_virt points to valid memory within our allocated frame,
        // with at least `data.len()` bytes available (checked at construction).
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), self.data_virt as *mut u8, data.len());
        }
    }

    /// Read data from the data region (for read requests).
    fn read_data(&self, buf: &mut [u8]) {
        // SAFETY: data_virt points to valid memory written by the device.
        // buf.len() does not exceed the allocated data region.
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.data_virt as *const u8,
                buf.as_mut_ptr(),
                buf.len(),
            );
        }
    }

    /// Read the status byte.
    fn read_status(&self) -> u8 {
        // SAFETY: status_virt points to a valid byte written by the device.
        unsafe { core::ptr::read_volatile(self.status_virt as *const u8) }
    }
}

impl Drop for RequestBuffer {
    fn drop(&mut self) {
        let _ = FRAME_ALLOCATOR.lock().free_frames(self.frame, 1);
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
    features: u32,
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
        if block_num >= self.capacity_sectors {
            return Err(KernelError::InvalidArgument {
                name: "block_num",
                value: "block number exceeds device capacity",
            });
        }

        self.do_request(req_type::VIRTIO_BLK_T_IN, block_num, Some(buf), None)
    }

    /// Write a single block (512 bytes) to the device.
    ///
    /// `block_num` is the 0-based sector number. `data` must be at least 512
    /// bytes.
    pub fn write_block(&mut self, block_num: u64, data: &[u8]) -> Result<(), KernelError> {
        if self.read_only {
            return Err(KernelError::PermissionDenied {
                operation: "write to read-only virtio-blk device",
            });
        }
        if data.len() < BLOCK_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "data must be at least 512 bytes",
            });
        }
        if block_num >= self.capacity_sectors {
            return Err(KernelError::InvalidArgument {
                name: "block_num",
                value: "block number exceeds device capacity",
            });
        }

        self.do_request(req_type::VIRTIO_BLK_T_OUT, block_num, None, Some(data))
    }

    /// Submit a block request and poll for completion.
    ///
    /// For IN (read): `read_buf` receives the data after completion.
    /// For OUT (write): `write_data` provides the data to write.
    fn do_request(
        &mut self,
        type_: u32,
        sector: u64,
        read_buf: Option<&mut [u8]>,
        write_data: Option<&[u8]>,
    ) -> Result<(), KernelError> {
        let data_len = BLOCK_SIZE;

        // Allocate DMA buffer for the request
        let req_buf = RequestBuffer::new(data_len)?;

        // Fill in the request header
        req_buf.write_header(type_, sector);

        // For write requests, copy data into the DMA buffer
        if let Some(data) = write_data {
            req_buf.write_data(&data[..data_len]);
        }

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

        // Descriptor 0: Header (device-readable, chained to data)
        // SAFETY: desc_header is a valid allocated descriptor index. header_phys
        // points to a valid VirtioBlkReqHeader in DMA-accessible memory.
        unsafe {
            self.queue.write_desc(
                desc_header,
                req_buf.header_phys,
                core::mem::size_of::<VirtioBlkReqHeader>() as u32,
                VIRTQ_DESC_F_NEXT,
                desc_data,
            );
        }

        // Descriptor 1: Data (direction depends on request type)
        let data_flags = if type_ == req_type::VIRTIO_BLK_T_IN {
            VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT // Device writes data
        } else {
            VIRTQ_DESC_F_NEXT // Device reads data (driver-written)
        };
        // SAFETY: desc_data is a valid allocated descriptor. data_phys points
        // to valid DMA memory of at least data_len bytes.
        unsafe {
            self.queue.write_desc(
                desc_data,
                req_buf.data_phys,
                data_len as u32,
                data_flags,
                desc_status,
            );
        }

        // Descriptor 2: Status (device-writable, end of chain)
        // SAFETY: desc_status is valid. status_phys points to 1 byte of DMA memory.
        unsafe {
            self.queue
                .write_desc(desc_status, req_buf.status_phys, 1, VIRTQ_DESC_F_WRITE, 0);
        }

        // Ensure all descriptor writes are visible before notifying
        atomic::fence(Ordering::Release);

        // Push the chain head onto the available ring
        self.queue.push_avail(desc_header);

        // Notify the device
        self.transport.notify_queue(0);

        // Poll for completion
        let mut spins: u32 = 0;
        const MAX_SPINS: u32 = 10_000_000;
        while !self.queue.has_used() {
            core::hint::spin_loop();
            spins += 1;
            if spins >= MAX_SPINS {
                // Free descriptors before returning error
                self.queue.free_chain(desc_header);
                return Err(KernelError::Timeout {
                    operation: "virtio-blk request",
                    duration_ms: 0,
                });
            }
        }

        // Consume the used entry
        let (_used_id, _used_len) = self.queue.poll_used().ok_or(KernelError::HardwareError {
            device: "virtio-blk",
            code: 0x02, // Used ring empty after has_used() returned true
        })?;

        // Check status byte
        let status = req_buf.read_status();
        match status {
            blk_status::VIRTIO_BLK_S_OK => {}
            blk_status::VIRTIO_BLK_S_IOERR => {
                self.queue.free_chain(desc_header);
                return Err(KernelError::HardwareError {
                    device: "virtio-blk",
                    code: 0x10, // I/O error
                });
            }
            blk_status::VIRTIO_BLK_S_UNSUPP => {
                self.queue.free_chain(desc_header);
                return Err(KernelError::OperationNotSupported {
                    operation: "virtio-blk unsupported request type",
                });
            }
            _ => {
                self.queue.free_chain(desc_header);
                return Err(KernelError::HardwareError {
                    device: "virtio-blk",
                    code: status as u32,
                });
            }
        }

        // For read requests, copy data back to the caller's buffer
        if let Some(buf) = read_buf {
            req_buf.read_data(&mut buf[..data_len]);
        }

        // Free the descriptor chain
        self.queue.free_chain(desc_header);

        // req_buf is dropped here, freeing the DMA frame
        Ok(())
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
/// [`super::mmio::DEFAULT_BASES`].
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn init_mmio() {
    use crate::drivers::virtio::mmio::{try_init_mmio_blk, DEFAULT_BASES};

    // Probe the standard virtio-mmio base addresses exposed by QEMU virt.
    for base in DEFAULT_BASES {
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

/// Get a reference to the global virtio-blk device, if initialized.
pub fn get_device() -> Option<&'static Mutex<VirtioBlkDevice>> {
    VIRTIO_BLK.get()
}

/// Check if a virtio-blk device has been initialized.
pub fn is_initialized() -> bool {
    VIRTIO_BLK.get().is_some()
}

/// Convert a physical address to a kernel-accessible virtual address.
fn phys_to_kernel_virt(phys: u64) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if let Some(virt) = crate::arch::x86_64::msr::phys_to_virt(phys as usize) {
            return virt;
        }
        (phys + 0xFFFF_8000_0000_0000) as usize
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        phys as usize
    }
}

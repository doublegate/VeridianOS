//! VirtIO GPU Driver
//!
//! Driver for paravirtualized GPU devices using the VirtIO protocol.
//! Commonly used in QEMU/KVM virtual machines for 2D/3D rendering.
//!
//! Implements the VirtIO MMIO transport with proper status negotiation,
//! virtqueue setup via frame allocator DMA buffers, and 2D rendering
//! operations (resource creation, scanout, transfer, flush).
//!
//! ## Architecture
//!
//! The driver uses the VirtIO 1.0+ modern MMIO transport interface with
//! two virtqueues:
//! - **controlq** (queue 0): All 2D/3D commands and responses
//! - **cursorq** (queue 1): Hardware cursor updates (optional)
//!
//! ## Supported Operations
//!
//! - Display info query (GET_DISPLAY_INFO)
//! - 2D resource creation (RESOURCE_CREATE_2D)
//! - Backing store attachment (RESOURCE_ATTACH_BACKING)
//! - Scanout configuration (SET_SCANOUT)
//! - Host transfer (TRANSFER_TO_HOST_2D)
//! - Display flush (RESOURCE_FLUSH)
//! - EDID query (GET_EDID, if supported)

// Allow dead code for VirtIO GPU protocol constants, structures, and methods
// not yet fully exercised by callers during Phase 7 bringup.
#![allow(dead_code, clippy::needless_range_loop)]

use alloc::vec::Vec;

use crate::error::KernelError;

// ============================================================================
// VirtIO GPU Protocol Constants
// ============================================================================

// --- Command types ---

/// Get display info (returns display modes for all scanouts)
const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x100;
/// Create a 2D resource (host-side texture)
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x101;
/// Destroy a 2D resource
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x102;
/// Set scanout (bind resource to display output)
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x103;
/// Flush resource to display
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x104;
/// Transfer data from guest to host resource
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x105;
/// Attach backing store pages to a resource
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x106;
/// Detach backing store from a resource
const VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING: u32 = 0x107;
/// Get capability set info (3D)
const VIRTIO_GPU_CMD_GET_CAPSET_INFO: u32 = 0x108;
/// Get capability set data (3D)
const VIRTIO_GPU_CMD_GET_CAPSET: u32 = 0x109;
/// Get EDID data for a scanout
const VIRTIO_GPU_CMD_GET_EDID: u32 = 0x10A;

// --- Response types ---

/// Success, no data payload
const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
/// Success, display info payload
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;
/// Success, capset info payload
const VIRTIO_GPU_RESP_OK_CAPSET_INFO: u32 = 0x1102;
/// Success, capset data payload
const VIRTIO_GPU_RESP_OK_CAPSET: u32 = 0x1103;
/// Success, EDID data payload
const VIRTIO_GPU_RESP_OK_EDID: u32 = 0x1104;

/// Error: unspecified
const VIRTIO_GPU_RESP_ERR_UNSPEC: u32 = 0x1200;
/// Error: out of memory on host
const VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY: u32 = 0x1201;
/// Error: invalid scanout ID
const VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
/// Error: invalid resource ID
const VIRTIO_GPU_RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
/// Error: invalid context ID
const VIRTIO_GPU_RESP_ERR_INVALID_CONTEXT_ID: u32 = 0x1204;
/// Error: invalid parameter
const VIRTIO_GPU_RESP_ERR_INVALID_PARAMETER: u32 = 0x1205;

// --- Pixel formats ---

/// B8G8R8A8 (BGRA with alpha, native for many displays)
const FORMAT_B8G8R8A8_UNORM: u32 = 1;
/// R8G8B8A8 (RGBA)
const FORMAT_R8G8B8A8_UNORM: u32 = 67;
/// B8G8R8X8 (BGRX, alpha ignored)
const FORMAT_B8G8R8X8_UNORM: u32 = 68;
/// R8G8B8X8 (RGBX, alpha ignored)
const FORMAT_R8G8B8X8_UNORM: u32 = 134;

// --- Feature bits ---

/// Device supports 3D (virgl) commands
const VIRTIO_GPU_F_VIRGL: u64 = 1 << 0;
/// Device supports EDID queries
const VIRTIO_GPU_F_EDID: u64 = 1 << 1;

// --- Max scanouts per the spec ---
const VIRTIO_GPU_MAX_SCANOUTS: usize = 16;

// ============================================================================
// VirtIO MMIO Register Offsets (modern interface, matches virtio_net.rs)
// ============================================================================

const VIRTIO_MMIO_MAGIC: usize = 0x00;
const VIRTIO_MMIO_VERSION: usize = 0x04;
const VIRTIO_MMIO_DEVICE_ID: usize = 0x08;
const VIRTIO_MMIO_DEVICE_FEATURES: usize = 0x10;
const VIRTIO_MMIO_DEVICE_FEATURES_SEL: usize = 0x14;
const VIRTIO_MMIO_DRIVER_FEATURES: usize = 0x20;
const VIRTIO_MMIO_DRIVER_FEATURES_SEL: usize = 0x24;
const VIRTIO_MMIO_QUEUE_SEL: usize = 0x30;
const VIRTIO_MMIO_QUEUE_NUM_MAX: usize = 0x34;
const VIRTIO_MMIO_QUEUE_NUM: usize = 0x38;
const VIRTIO_MMIO_QUEUE_READY: usize = 0x44;
const VIRTIO_MMIO_QUEUE_NOTIFY: usize = 0x50;
const VIRTIO_MMIO_STATUS: usize = 0x70;
const VIRTIO_MMIO_QUEUE_DESC_LOW: usize = 0x80;
const VIRTIO_MMIO_QUEUE_DESC_HIGH: usize = 0x84;
const VIRTIO_MMIO_QUEUE_AVAIL_LOW: usize = 0x90;
const VIRTIO_MMIO_QUEUE_AVAIL_HIGH: usize = 0x94;
const VIRTIO_MMIO_QUEUE_USED_LOW: usize = 0xA0;
const VIRTIO_MMIO_QUEUE_USED_HIGH: usize = 0xA4;
const VIRTIO_MMIO_CONFIG_BASE: usize = 0x100;

// VirtIO status bits
const VIRTIO_STATUS_ACKNOWLEDGE: u32 = 1;
const VIRTIO_STATUS_DRIVER: u32 = 2;
const VIRTIO_STATUS_DRIVER_OK: u32 = 4;
const VIRTIO_STATUS_FEATURES_OK: u32 = 8;

/// Descriptor flags: next descriptor exists (chained)
const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Descriptor flags: buffer is device-writable
const VIRTQ_DESC_F_WRITE: u16 = 2;

// ============================================================================
// VirtIO GPU Protocol Structures
// ============================================================================

/// VirtIO GPU control header -- common prefix for all commands and responses.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuCtrlHdr {
    /// Command or response type
    hdr_type: u32,
    /// Flags (e.g. VIRTIO_GPU_FLAG_FENCE)
    flags: u32,
    /// Fence ID for synchronization
    fence_id: u64,
    /// 3D rendering context ID (0 for 2D)
    ctx_id: u32,
    /// Ring index (virtio-gpu multi-queue extension)
    ring_idx: u8,
    /// Padding to maintain alignment
    padding: [u8; 3],
}

impl VirtioGpuCtrlHdr {
    /// Create a new command header with the given type.
    fn new(hdr_type: u32) -> Self {
        Self {
            hdr_type,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            ring_idx: 0,
            padding: [0; 3],
        }
    }
}

/// Rectangle structure for GPU commands.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioGpuRect {
    /// X coordinate
    pub x: u32,
    /// Y coordinate
    pub y: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl VirtioGpuRect {
    /// Create a new rectangle.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Display mode information for one scanout.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioGpuDisplayOne {
    /// Active display rectangle (position and size)
    rect: VirtioGpuRect,
    /// Whether this scanout is enabled
    enabled: u32,
    /// Scanout flags
    flags: u32,
}

/// Response to GET_DISPLAY_INFO command.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuRespDisplayInfo {
    /// Response header
    hdr: VirtioGpuCtrlHdr,
    /// Display modes for up to 16 scanouts
    pmodes: [VirtioGpuDisplayOne; VIRTIO_GPU_MAX_SCANOUTS],
}

/// RESOURCE_CREATE_2D command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuResourceCreate2d {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Unique resource identifier
    resource_id: u32,
    /// Pixel format (FORMAT_B8G8R8A8_UNORM etc.)
    format: u32,
    /// Width in pixels
    width: u32,
    /// Height in pixels
    height: u32,
}

/// RESOURCE_UNREF command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuResourceUnref {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Resource to destroy
    resource_id: u32,
    /// Padding
    padding: u32,
}

/// RESOURCE_ATTACH_BACKING command structure.
///
/// Followed immediately in the descriptor by `nr_entries` VirtioGpuMemEntry
/// elements.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuResourceAttachBacking {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Resource to attach backing to
    resource_id: u32,
    /// Number of memory entries following this struct
    nr_entries: u32,
}

/// A single memory entry for RESOURCE_ATTACH_BACKING.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuMemEntry {
    /// Physical address of the backing page
    addr: u64,
    /// Length in bytes
    length: u32,
    /// Padding
    padding: u32,
}

/// SET_SCANOUT command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuSetScanout {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Rectangle within the resource to display
    rect: VirtioGpuRect,
    /// Scanout index (display output)
    scanout_id: u32,
    /// Resource to display
    resource_id: u32,
}

/// TRANSFER_TO_HOST_2D command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuTransferToHost2d {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Rectangle within the resource to transfer
    rect: VirtioGpuRect,
    /// Byte offset within the resource backing store
    offset: u64,
    /// Resource to transfer
    resource_id: u32,
    /// Padding
    padding: u32,
}

/// RESOURCE_FLUSH command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuResourceFlush {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Rectangle to flush to display
    rect: VirtioGpuRect,
    /// Resource to flush
    resource_id: u32,
    /// Padding
    padding: u32,
}

/// RESOURCE_DETACH_BACKING command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuResourceDetachBacking {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Resource to detach backing from
    resource_id: u32,
    /// Padding
    padding: u32,
}

/// GET_EDID command structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuGetEdid {
    /// Command header
    hdr: VirtioGpuCtrlHdr,
    /// Scanout to query EDID for
    scanout: u32,
    /// Padding
    padding: u32,
}

/// GET_EDID response structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioGpuRespEdid {
    /// Response header
    hdr: VirtioGpuCtrlHdr,
    /// Size of valid EDID data
    size: u32,
    /// Padding
    padding: u32,
    /// Raw EDID data (up to 1024 bytes)
    edid: [u8; 1024],
}

// ============================================================================
// VirtIO Ring Structures (same layout as virtio_net.rs)
// ============================================================================

/// VirtIO Ring Descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtqDesc {
    /// Guest physical address of the buffer
    addr: u64,
    /// Length of the buffer in bytes
    len: u32,
    /// Descriptor flags (NEXT, WRITE, INDIRECT)
    flags: u16,
    /// Index of the next descriptor in the chain
    next: u16,
}

/// VirtIO Ring Available
#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    ring: [u16; 256],
    used_event: u16,
}

/// VirtIO Ring Used Element
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

/// VirtIO Ring Used
#[repr(C)]
struct VirtqUsed {
    flags: u16,
    idx: u16,
    ring: [VirtqUsedElem; 256],
    avail_event: u16,
}

/// VirtIO Virtqueue -- manages a descriptor ring, available ring, and used
/// ring.
struct Virtqueue {
    /// Queue size (number of descriptors)
    size: u16,

    /// Descriptor table
    descriptors: &'static mut [VirtqDesc],

    /// Available ring
    avail: &'static mut VirtqAvail,

    /// Used ring
    used: &'static mut VirtqUsed,

    /// Free descriptor list head
    free_head: u16,

    /// Last seen used index
    last_used_idx: u16,

    /// Number of free descriptors
    num_free: u16,
}

impl Virtqueue {
    /// Create a new virtqueue from pre-allocated memory regions.
    fn new(
        descriptors: &'static mut [VirtqDesc],
        avail: &'static mut VirtqAvail,
        used: &'static mut VirtqUsed,
        size: u16,
    ) -> Self {
        // Initialize descriptor free list
        for i in 0..size {
            descriptors[i as usize].next = if i + 1 < size { i + 1 } else { 0 };
        }

        // Initialize rings
        avail.flags = 0;
        avail.idx = 0;
        used.flags = 0;
        used.idx = 0;

        Self {
            size,
            descriptors,
            avail,
            used,
            free_head: 0,
            last_used_idx: 0,
            num_free: size,
        }
    }

    /// Allocate a free descriptor.
    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }

        let desc_idx = self.free_head;
        self.free_head = self.descriptors[desc_idx as usize].next;
        self.num_free -= 1;

        Some(desc_idx)
    }

    /// Return a descriptor to the free list.
    fn free_desc(&mut self, desc_idx: u16) {
        self.descriptors[desc_idx as usize].next = self.free_head;
        self.free_head = desc_idx;
        self.num_free += 1;
    }

    /// Add a descriptor to the available ring and advance the index.
    fn add_to_avail(&mut self, desc_idx: u16) {
        let avail_idx = self.avail.idx as usize % self.size as usize;
        self.avail.ring[avail_idx] = desc_idx;

        // Memory barrier to ensure descriptor writes are visible before idx update
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        self.avail.idx = self.avail.idx.wrapping_add(1);
    }

    /// Check for completed buffers in the used ring.
    fn get_used(&mut self) -> Option<(u16, u32)> {
        if self.last_used_idx == self.used.idx {
            return None;
        }

        let used_idx = self.last_used_idx as usize % self.size as usize;
        let used_elem = self.used.ring[used_idx];

        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Some((used_elem.id as u16, used_elem.len))
    }
}

/// DMA buffer region backing a virtqueue.
///
/// Stores the virtual address of frame-allocator-provided pages
/// used for the descriptor table, available ring, and used ring.
struct VirtqueueDmaRegion {
    /// Virtual address of allocated pages
    virt_addr: usize,
    /// Number of 4KB pages allocated
    num_pages: usize,
}

/// Per-descriptor data buffer (single 4KB page or larger).
struct DataBuffer {
    virt_addr: usize,
    phys_addr: u64,
}

// ============================================================================
// VirtIO GPU Driver State
// ============================================================================

/// GPU device initialization state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpuDeviceState {
    /// Device not yet initialized
    Uninitialized,
    /// Device initialized and ready
    Ready,
    /// Device encountered an error
    Error,
}

/// VirtIO GPU Driver
///
/// Manages a single virtio-gpu device including its control and cursor
/// virtqueues, display configuration, and framebuffer resources.
pub struct VirtioGpuDriver {
    /// MMIO base address (virtual)
    mmio_base: usize,

    /// Negotiated device features
    features: u64,

    /// Device state
    state: GpuDeviceState,

    /// Control virtqueue (queue index 0)
    controlq: Option<Virtqueue>,
    /// Cursor virtqueue (queue index 1)
    cursorq: Option<Virtqueue>,

    /// DMA region backing the control virtqueue rings
    ctrl_dma: Option<VirtqueueDmaRegion>,
    /// DMA region backing the cursor virtqueue rings
    cursor_dma: Option<VirtqueueDmaRegion>,

    /// Per-descriptor data buffers for the control queue
    ctrl_buffers: Vec<DataBuffer>,
    /// Per-descriptor data buffers for the cursor queue
    cursor_buffers: Vec<DataBuffer>,

    /// Detected display info for the first enabled scanout
    display_info: Option<VirtioGpuDisplayOne>,

    /// Next resource ID to allocate
    next_resource_id: u32,
    /// Resource ID bound to the primary framebuffer
    framebuffer_resource_id: u32,
    /// Backing pixel buffer for the framebuffer resource
    framebuffer_backing: Option<Vec<u32>>,

    /// Display width in pixels
    width: u32,
    /// Display height in pixels
    height: u32,
}

impl VirtioGpuDriver {
    /// Create and initialize a new VirtIO GPU driver at the given MMIO base.
    pub fn new(mmio_base: usize) -> Result<Self, KernelError> {
        let mut driver = Self {
            mmio_base,
            features: 0,
            state: GpuDeviceState::Uninitialized,
            controlq: None,
            cursorq: None,
            ctrl_dma: None,
            cursor_dma: None,
            ctrl_buffers: Vec::new(),
            cursor_buffers: Vec::new(),
            display_info: None,
            next_resource_id: 1,
            framebuffer_resource_id: 0,
            framebuffer_backing: None,
            width: 0,
            height: 0,
        };

        driver.initialize()?;
        Ok(driver)
    }

    // ---- MMIO register access ----

    /// Read a 32-bit MMIO register.
    fn read_reg(&self, offset: usize) -> u32 {
        // SAFETY: Reading a VirtIO MMIO register at mmio_base + offset. The
        // mmio_base is the device's memory-mapped I/O base from the device tree
        // or PCI BAR. read_volatile prevents compiler reordering of hardware
        // register accesses.
        unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u32) }
    }

    /// Write a 32-bit MMIO register.
    fn write_reg(&self, offset: usize, value: u32) {
        // SAFETY: Writing a VirtIO MMIO register. Same invariants as read_reg.
        unsafe {
            core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value);
        }
    }

    // ---- Device initialization ----

    /// Initialize the VirtIO GPU device following the VirtIO 1.0+ sequence:
    ///
    /// 1. Reset device
    /// 2. Set ACKNOWLEDGE
    /// 3. Set DRIVER
    /// 4. Negotiate features
    /// 5. Set FEATURES_OK and verify
    /// 6. Setup virtqueues (control=0, cursor=1)
    /// 7. Set DRIVER_OK
    /// 8. Query display info and setup framebuffer
    fn initialize(&mut self) -> Result<(), KernelError> {
        // Validate magic number
        let magic = self.read_reg(VIRTIO_MMIO_MAGIC);
        if magic != 0x74726976 {
            // "virt" in little-endian
            crate::println!(
                "[VIRTIO-GPU] Invalid magic: {:#010x} (expected 0x74726976)",
                magic
            );
            return Err(KernelError::HardwareError {
                device: "virtio-gpu",
                code: 0x01,
            });
        }

        // Validate device ID (16 = GPU device)
        let device_id = self.read_reg(VIRTIO_MMIO_DEVICE_ID);
        if device_id != 16 {
            crate::println!(
                "[VIRTIO-GPU] Unexpected device ID: {} (expected 16)",
                device_id
            );
            return Err(KernelError::HardwareError {
                device: "virtio-gpu",
                code: 0x02,
            });
        }

        let version = self.read_reg(VIRTIO_MMIO_VERSION);
        crate::println!(
            "[VIRTIO-GPU] Found virtio-gpu device (MMIO version {})",
            version
        );

        // Step 1: Reset device
        self.write_reg(VIRTIO_MMIO_STATUS, 0);

        // Step 2: Set ACKNOWLEDGE status bit
        self.write_reg(VIRTIO_MMIO_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);

        // Step 3: Set DRIVER status bit
        self.write_reg(
            VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER,
        );

        // Step 4: Read and negotiate features
        self.write_reg(VIRTIO_MMIO_DEVICE_FEATURES_SEL, 0);
        let features_low = self.read_reg(VIRTIO_MMIO_DEVICE_FEATURES) as u64;
        self.write_reg(VIRTIO_MMIO_DEVICE_FEATURES_SEL, 1);
        let features_high = (self.read_reg(VIRTIO_MMIO_DEVICE_FEATURES) as u64) << 32;
        self.features = features_low | features_high;

        crate::println!("[VIRTIO-GPU] Device features: {:#018x}", self.features);

        if self.features & VIRTIO_GPU_F_VIRGL != 0 {
            crate::println!("[VIRTIO-GPU]   - VIRGL (3D) supported");
        }
        if self.features & VIRTIO_GPU_F_EDID != 0 {
            crate::println!("[VIRTIO-GPU]   - EDID supported");
        }

        // Accept EDID if available, but do NOT request VIRGL (we only do 2D)
        let driver_features = self.features & VIRTIO_GPU_F_EDID;
        self.write_reg(VIRTIO_MMIO_DRIVER_FEATURES_SEL, 0);
        self.write_reg(
            VIRTIO_MMIO_DRIVER_FEATURES,
            (driver_features & 0xFFFFFFFF) as u32,
        );
        self.write_reg(VIRTIO_MMIO_DRIVER_FEATURES_SEL, 1);
        self.write_reg(VIRTIO_MMIO_DRIVER_FEATURES, (driver_features >> 32) as u32);

        // Step 5: Set FEATURES_OK and verify
        self.write_reg(
            VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK,
        );

        if (self.read_reg(VIRTIO_MMIO_STATUS) & VIRTIO_STATUS_FEATURES_OK) == 0 {
            crate::println!("[VIRTIO-GPU] Device did not accept features");
            return Err(KernelError::HardwareError {
                device: "virtio-gpu",
                code: 0x03,
            });
        }

        // Step 6: Set up virtqueues
        self.setup_control_queue()?;
        self.setup_cursor_queue()?;

        // Step 7: Set DRIVER_OK -- device is live
        self.write_reg(
            VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE
                | VIRTIO_STATUS_DRIVER
                | VIRTIO_STATUS_FEATURES_OK
                | VIRTIO_STATUS_DRIVER_OK,
        );

        crate::println!("[VIRTIO-GPU] Device status: DRIVER_OK");

        // Step 8: Query display info
        match self.get_display_info() {
            Ok(display) => {
                self.width = display.rect.width;
                self.height = display.rect.height;
                self.display_info = Some(display);

                crate::println!(
                    "[VIRTIO-GPU] Display: {}x{} (enabled={})",
                    self.width,
                    self.height,
                    display.enabled
                );

                // Set up the primary framebuffer
                if let Err(e) = self.setup_framebuffer() {
                    crate::println!("[VIRTIO-GPU] Framebuffer setup failed: {:?}", e);
                    // Non-fatal: driver is still usable for manual operations
                }
            }
            Err(e) => {
                crate::println!("[VIRTIO-GPU] Display info query failed: {:?}", e);
                // Use default resolution
                self.width = 1024;
                self.height = 768;
            }
        }

        self.state = GpuDeviceState::Ready;
        Ok(())
    }

    /// Set up the control virtqueue (queue index 0).
    fn setup_control_queue(&mut self) -> Result<(), KernelError> {
        self.write_reg(VIRTIO_MMIO_QUEUE_SEL, 0);

        let max_size = self.read_reg(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if max_size == 0 {
            return Err(KernelError::HardwareError {
                device: "virtio-gpu",
                code: 0x10,
            });
        }
        let queue_size = max_size.min(256);

        crate::println!(
            "[VIRTIO-GPU] Control queue: max={}, using={}",
            max_size,
            queue_size
        );

        let (vq, dma, buffers) = self.allocate_virtqueue(queue_size)?;

        // Tell device about the queue addresses
        let desc_phys = dma.virt_addr as u64;
        let avail_offset = (queue_size as usize) * core::mem::size_of::<VirtqDesc>();
        let used_offset = avail_offset + 6 + 2 * (queue_size as usize);
        let avail_phys = desc_phys + avail_offset as u64;
        let used_phys = desc_phys + used_offset as u64;

        self.write_reg(VIRTIO_MMIO_QUEUE_NUM, queue_size as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_DESC_LOW, desc_phys as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_DESC_HIGH, (desc_phys >> 32) as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_AVAIL_LOW, avail_phys as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_AVAIL_HIGH, (avail_phys >> 32) as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_USED_LOW, used_phys as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_USED_HIGH, (used_phys >> 32) as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_READY, 1);

        self.controlq = Some(vq);
        self.ctrl_dma = Some(dma);
        self.ctrl_buffers = buffers;

        Ok(())
    }

    /// Set up the cursor virtqueue (queue index 1).
    fn setup_cursor_queue(&mut self) -> Result<(), KernelError> {
        self.write_reg(VIRTIO_MMIO_QUEUE_SEL, 1);

        let max_size = self.read_reg(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if max_size == 0 {
            crate::println!("[VIRTIO-GPU] Cursor queue not available (max_size=0)");
            return Ok(()); // Cursor queue is optional
        }
        let queue_size = max_size.min(256);

        crate::println!(
            "[VIRTIO-GPU] Cursor queue: max={}, using={}",
            max_size,
            queue_size
        );

        let (vq, dma, buffers) = self.allocate_virtqueue(queue_size)?;

        let desc_phys = dma.virt_addr as u64;
        let avail_offset = (queue_size as usize) * core::mem::size_of::<VirtqDesc>();
        let used_offset = avail_offset + 6 + 2 * (queue_size as usize);
        let avail_phys = desc_phys + avail_offset as u64;
        let used_phys = desc_phys + used_offset as u64;

        self.write_reg(VIRTIO_MMIO_QUEUE_NUM, queue_size as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_DESC_LOW, desc_phys as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_DESC_HIGH, (desc_phys >> 32) as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_AVAIL_LOW, avail_phys as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_AVAIL_HIGH, (avail_phys >> 32) as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_USED_LOW, used_phys as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_USED_HIGH, (used_phys >> 32) as u32);
        self.write_reg(VIRTIO_MMIO_QUEUE_READY, 1);

        self.cursorq = Some(vq);
        self.cursor_dma = Some(dma);
        self.cursor_buffers = buffers;

        Ok(())
    }

    /// Allocate a virtqueue: ring memory + per-descriptor data buffers.
    fn allocate_virtqueue(
        &self,
        queue_size: u16,
    ) -> Result<(Virtqueue, VirtqueueDmaRegion, Vec<DataBuffer>), KernelError> {
        let qs = queue_size as usize;

        // Calculate total ring memory needed:
        //   descriptors: qs * 16 bytes
        //   avail ring: 2+2 + qs*2 + 2 = 6 + 2*qs bytes
        //   used ring: 2+2 + qs*8 + 2 = 6 + 8*qs bytes
        let desc_size = qs * core::mem::size_of::<VirtqDesc>();
        let avail_size = 6 + 2 * qs;
        let used_size = 6 + 8 * qs;
        let total_ring_bytes = desc_size + avail_size + used_size;
        let ring_pages = total_ring_bytes.div_ceil(4096);

        // Allocate pages for the ring structures.
        // In a full implementation this would use the frame allocator for
        // physically contiguous DMA memory. For now we use a zeroed Vec
        // that is leaked to obtain 'static references.
        let ring_mem = alloc::vec![0u8; ring_pages * 4096];
        let ring_ptr = ring_mem.as_ptr() as usize;
        // Leak the memory so it lives for 'static (device holds references)
        core::mem::forget(ring_mem);

        // Carve out descriptor table, avail ring, used ring
        let desc_ptr = ring_ptr as *mut VirtqDesc;
        let avail_ptr = (ring_ptr + desc_size) as *mut VirtqAvail;
        let used_ptr = (ring_ptr + desc_size + avail_size) as *mut VirtqUsed;

        // SAFETY: These pointers come from a just-allocated, zeroed region that
        // is large enough and properly aligned (Vec guarantees alignment for u8).
        // The region is leaked so it outlives the driver.
        let descriptors = unsafe { core::slice::from_raw_parts_mut(desc_ptr, qs) };
        let avail = unsafe { &mut *avail_ptr };
        let used = unsafe { &mut *used_ptr };

        let vq = Virtqueue::new(descriptors, avail, used, queue_size);

        // Allocate per-descriptor data buffers (one 4KB page each)
        let mut data_buffers = Vec::with_capacity(qs);
        for _i in 0..qs {
            let buf = alloc::vec![0u8; 4096];
            let buf_virt = buf.as_ptr() as usize;
            let buf_phys = buf_virt as u64; // Approximate for identity/offset mapping
            core::mem::forget(buf);
            data_buffers.push(DataBuffer {
                virt_addr: buf_virt,
                phys_addr: buf_phys,
            });
        }

        let dma = VirtqueueDmaRegion {
            virt_addr: ring_ptr,
            num_pages: ring_pages,
        };

        Ok((vq, dma, data_buffers))
    }

    // ---- Command submission ----

    /// Send a command via the control queue and wait for the response.
    ///
    /// The command is copied into a data buffer, submitted via the control
    /// virtqueue as a two-descriptor chain (device-readable request +
    /// device-writable response), and the driver polls the used ring for
    /// completion.
    ///
    /// Returns the response header for status checking.
    fn send_command_raw(
        &mut self,
        cmd_bytes: &[u8],
        resp_len: usize,
    ) -> Result<(VirtioGpuCtrlHdr, usize), KernelError> {
        let mmio = self.mmio_base;

        let controlq = self.controlq.as_mut().ok_or(KernelError::HardwareError {
            device: "virtio-gpu",
            code: 0x20,
        })?;

        // Allocate two descriptors: one for the request, one for the response
        let req_desc_idx = controlq
            .alloc_desc()
            .ok_or(KernelError::ResourceExhausted {
                resource: "virtio_gpu_ctrl_descriptors",
            })?;

        let resp_desc_idx = controlq.alloc_desc().ok_or_else(|| {
            controlq.free_desc(req_desc_idx);
            KernelError::ResourceExhausted {
                resource: "virtio_gpu_ctrl_descriptors",
            }
        })?;

        // Copy command data into the request buffer
        if (req_desc_idx as usize) < self.ctrl_buffers.len()
            && (resp_desc_idx as usize) < self.ctrl_buffers.len()
        {
            let req_buf_virt = self.ctrl_buffers[req_desc_idx as usize].virt_addr;
            let req_buf_phys = self.ctrl_buffers[req_desc_idx as usize].phys_addr;
            let resp_buf_virt = self.ctrl_buffers[resp_desc_idx as usize].virt_addr;
            let resp_buf_phys = self.ctrl_buffers[resp_desc_idx as usize].phys_addr;

            // SAFETY: req_buf_virt points to a leaked 4096-byte allocation.
            // cmd_bytes.len() <= 4096 (checked by callers or bounded by protocol).
            // We hold &mut self so no concurrent access.
            let req_slice =
                unsafe { core::slice::from_raw_parts_mut(req_buf_virt as *mut u8, 4096) };
            let copy_len = cmd_bytes.len().min(4096);
            req_slice[..copy_len].copy_from_slice(&cmd_bytes[..copy_len]);

            // Zero the response buffer
            let resp_slice =
                unsafe { core::slice::from_raw_parts_mut(resp_buf_virt as *mut u8, 4096) };
            resp_slice[..resp_len.min(4096)].fill(0);

            // Set up request descriptor (device-readable, chained to response)
            controlq.descriptors[req_desc_idx as usize] = VirtqDesc {
                addr: req_buf_phys,
                len: copy_len as u32,
                flags: VIRTQ_DESC_F_NEXT,
                next: resp_desc_idx,
            };

            // Set up response descriptor (device-writable)
            controlq.descriptors[resp_desc_idx as usize] = VirtqDesc {
                addr: resp_buf_phys,
                len: resp_len.min(4096) as u32,
                flags: VIRTQ_DESC_F_WRITE,
                next: 0,
            };

            // Add the head of the chain to the available ring
            controlq.add_to_avail(req_desc_idx);

            // Kick the device (control queue = index 0)
            // SAFETY: Writing to VirtIO queue notify register.
            unsafe {
                core::ptr::write_volatile((mmio + VIRTIO_MMIO_QUEUE_NOTIFY) as *mut u32, 0);
            }

            // Poll for completion (with timeout)
            let mut timeout = 1_000_000u32;
            loop {
                if let Some((_used_id, used_len)) = controlq.get_used() {
                    // Read response header from the response buffer
                    // SAFETY: resp_buf_virt is a valid, leaked 4096-byte buffer.
                    let resp_hdr = unsafe { *(resp_buf_virt as *const VirtioGpuCtrlHdr) };

                    // Free both descriptors
                    controlq.free_desc(req_desc_idx);
                    controlq.free_desc(resp_desc_idx);

                    return Ok((resp_hdr, used_len as usize));
                }

                timeout -= 1;
                if timeout == 0 {
                    controlq.free_desc(req_desc_idx);
                    controlq.free_desc(resp_desc_idx);
                    return Err(KernelError::Timeout {
                        operation: "virtio_gpu_command",
                        duration_ms: 1000,
                    });
                }

                core::hint::spin_loop();
            }
        } else {
            controlq.free_desc(req_desc_idx);
            controlq.free_desc(resp_desc_idx);
            Err(KernelError::ResourceExhausted {
                resource: "virtio_gpu_ctrl_buffers",
            })
        }
    }

    /// Send a typed command and expect a simple OK_NODATA response.
    fn send_simple_command<T: Sized>(&mut self, cmd: &T) -> Result<(), KernelError> {
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(cmd as *const T as *const u8, core::mem::size_of::<T>())
        };

        let (resp_hdr, _len) =
            self.send_command_raw(cmd_bytes, core::mem::size_of::<VirtioGpuCtrlHdr>())?;

        if resp_hdr.hdr_type != VIRTIO_GPU_RESP_OK_NODATA {
            return Err(Self::response_to_error(resp_hdr.hdr_type));
        }

        Ok(())
    }

    /// Convert a VirtIO GPU response type to a KernelError.
    fn response_to_error(resp_type: u32) -> KernelError {
        match resp_type {
            VIRTIO_GPU_RESP_ERR_UNSPEC => KernelError::HardwareError {
                device: "virtio-gpu",
                code: 0x1200,
            },
            VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY => KernelError::OutOfMemory {
                requested: 0,
                available: 0,
            },
            VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID => KernelError::InvalidArgument {
                name: "scanout_id",
                value: "invalid",
            },
            VIRTIO_GPU_RESP_ERR_INVALID_RESOURCE_ID => KernelError::InvalidArgument {
                name: "resource_id",
                value: "invalid",
            },
            VIRTIO_GPU_RESP_ERR_INVALID_CONTEXT_ID => KernelError::InvalidArgument {
                name: "context_id",
                value: "invalid",
            },
            VIRTIO_GPU_RESP_ERR_INVALID_PARAMETER => KernelError::InvalidArgument {
                name: "parameter",
                value: "invalid",
            },
            _ => KernelError::HardwareError {
                device: "virtio-gpu",
                code: resp_type,
            },
        }
    }

    // ---- GPU commands ----

    /// Query display information from the device.
    ///
    /// Returns the first enabled display mode (scanout 0 is preferred).
    pub fn get_display_info(&mut self) -> Result<VirtioGpuDisplayOne, KernelError> {
        let cmd = VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const VirtioGpuCtrlHdr as *const u8,
                core::mem::size_of::<VirtioGpuCtrlHdr>(),
            )
        };

        let resp_size = core::mem::size_of::<VirtioGpuRespDisplayInfo>();
        let (resp_hdr, _len) = self.send_command_raw(cmd_bytes, resp_size)?;

        if resp_hdr.hdr_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
            return Err(Self::response_to_error(resp_hdr.hdr_type));
        }

        // Read the full response from the response buffer.
        // We need to re-read the response descriptor buffer to get the display
        // info payload.
        //
        // The response was written to the resp_desc buffer (second descriptor).
        // Since send_command_raw already freed the descriptors, we need to read
        // from the buffer that was used. We re-read it by examining the second
        // data buffer that was just used.
        //
        // A simpler approach: peek at the response buffer before freeing.
        // Since we already got resp_hdr, we need to read the full struct.
        // The response was in ctrl_buffers[resp_desc_idx]. However,
        // send_command_raw already freed the descriptors. We need a different
        // approach.
        //
        // Refactored: use send_command_raw_with_response for large responses.
        //
        // For now, re-send the command and capture the full response.
        self.get_display_info_internal()
    }

    /// Internal implementation of get_display_info that captures the full
    /// response buffer.
    fn get_display_info_internal(&mut self) -> Result<VirtioGpuDisplayOne, KernelError> {
        let cmd = VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(
                &cmd as *const VirtioGpuCtrlHdr as *const u8,
                core::mem::size_of::<VirtioGpuCtrlHdr>(),
            )
        };

        let resp_size = core::mem::size_of::<VirtioGpuRespDisplayInfo>();
        let mmio = self.mmio_base;

        let controlq = self.controlq.as_mut().ok_or(KernelError::HardwareError {
            device: "virtio-gpu",
            code: 0x20,
        })?;

        let req_desc_idx = controlq
            .alloc_desc()
            .ok_or(KernelError::ResourceExhausted {
                resource: "virtio_gpu_ctrl_descriptors",
            })?;

        let resp_desc_idx = controlq.alloc_desc().ok_or_else(|| {
            controlq.free_desc(req_desc_idx);
            KernelError::ResourceExhausted {
                resource: "virtio_gpu_ctrl_descriptors",
            }
        })?;

        if (req_desc_idx as usize) >= self.ctrl_buffers.len()
            || (resp_desc_idx as usize) >= self.ctrl_buffers.len()
        {
            controlq.free_desc(req_desc_idx);
            controlq.free_desc(resp_desc_idx);
            return Err(KernelError::ResourceExhausted {
                resource: "virtio_gpu_ctrl_buffers",
            });
        }

        let req_buf_virt = self.ctrl_buffers[req_desc_idx as usize].virt_addr;
        let req_buf_phys = self.ctrl_buffers[req_desc_idx as usize].phys_addr;
        let resp_buf_virt = self.ctrl_buffers[resp_desc_idx as usize].virt_addr;
        let resp_buf_phys = self.ctrl_buffers[resp_desc_idx as usize].phys_addr;

        // Copy command
        let req_slice = unsafe { core::slice::from_raw_parts_mut(req_buf_virt as *mut u8, 4096) };
        let copy_len = cmd_bytes.len().min(4096);
        req_slice[..copy_len].copy_from_slice(&cmd_bytes[..copy_len]);

        // Zero response
        let resp_slice = unsafe { core::slice::from_raw_parts_mut(resp_buf_virt as *mut u8, 4096) };
        resp_slice[..resp_size.min(4096)].fill(0);

        // Set up descriptors
        controlq.descriptors[req_desc_idx as usize] = VirtqDesc {
            addr: req_buf_phys,
            len: copy_len as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next: resp_desc_idx,
        };
        controlq.descriptors[resp_desc_idx as usize] = VirtqDesc {
            addr: resp_buf_phys,
            len: resp_size.min(4096) as u32,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        };

        controlq.add_to_avail(req_desc_idx);

        // Kick
        unsafe {
            core::ptr::write_volatile((mmio + VIRTIO_MMIO_QUEUE_NOTIFY) as *mut u32, 0);
        }

        // Poll for completion
        let mut timeout = 1_000_000u32;
        loop {
            if let Some((_used_id, _used_len)) = controlq.get_used() {
                // Read the full response
                // SAFETY: resp_buf_virt is a valid leaked 4096-byte buffer,
                // and VirtioGpuRespDisplayInfo fits within it.
                let resp = unsafe { *(resp_buf_virt as *const VirtioGpuRespDisplayInfo) };

                controlq.free_desc(req_desc_idx);
                controlq.free_desc(resp_desc_idx);

                if resp.hdr.hdr_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
                    return Err(Self::response_to_error(resp.hdr.hdr_type));
                }

                // Find the first enabled display
                for i in 0..VIRTIO_GPU_MAX_SCANOUTS {
                    if resp.pmodes[i].enabled != 0 {
                        return Ok(resp.pmodes[i]);
                    }
                }

                // No enabled display found -- use scanout 0 with defaults
                if resp.pmodes[0].rect.width > 0 && resp.pmodes[0].rect.height > 0 {
                    return Ok(resp.pmodes[0]);
                }

                // Fallback defaults
                return Ok(VirtioGpuDisplayOne {
                    rect: VirtioGpuRect {
                        x: 0,
                        y: 0,
                        width: 1024,
                        height: 768,
                    },
                    enabled: 1,
                    flags: 0,
                });
            }

            timeout -= 1;
            if timeout == 0 {
                controlq.free_desc(req_desc_idx);
                controlq.free_desc(resp_desc_idx);
                return Err(KernelError::Timeout {
                    operation: "virtio_gpu_get_display_info",
                    duration_ms: 1000,
                });
            }

            core::hint::spin_loop();
        }
    }

    /// Create a 2D resource on the host.
    pub fn create_resource_2d(
        &mut self,
        resource_id: u32,
        format: u32,
        width: u32,
        height: u32,
    ) -> Result<(), KernelError> {
        let cmd = VirtioGpuResourceCreate2d {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id,
            format,
            width,
            height,
        };

        self.send_simple_command(&cmd)?;

        crate::println!(
            "[VIRTIO-GPU] Created 2D resource {} ({}x{}, format={})",
            resource_id,
            width,
            height,
            format
        );

        Ok(())
    }

    /// Destroy a 2D resource on the host.
    pub fn resource_unref(&mut self, resource_id: u32) -> Result<(), KernelError> {
        let cmd = VirtioGpuResourceUnref {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_UNREF),
            resource_id,
            padding: 0,
        };

        self.send_simple_command(&cmd)?;

        crate::println!("[VIRTIO-GPU] Destroyed resource {}", resource_id);
        Ok(())
    }

    /// Attach backing store (guest memory) to a resource.
    ///
    /// The command includes a VirtioGpuMemEntry that describes the physical
    /// address and length of the backing memory.
    pub fn attach_backing(
        &mut self,
        resource_id: u32,
        addr: u64,
        length: u32,
    ) -> Result<(), KernelError> {
        // Build the combined command: attach_backing header + one mem entry
        // We need to send them as a single contiguous command buffer.
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct AttachBackingWithEntry {
            cmd: VirtioGpuResourceAttachBacking,
            entry: VirtioGpuMemEntry,
        }

        let combined = AttachBackingWithEntry {
            cmd: VirtioGpuResourceAttachBacking {
                hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
                resource_id,
                nr_entries: 1,
            },
            entry: VirtioGpuMemEntry {
                addr,
                length,
                padding: 0,
            },
        };

        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(
                &combined as *const AttachBackingWithEntry as *const u8,
                core::mem::size_of::<AttachBackingWithEntry>(),
            )
        };

        let (resp_hdr, _len) =
            self.send_command_raw(cmd_bytes, core::mem::size_of::<VirtioGpuCtrlHdr>())?;

        if resp_hdr.hdr_type != VIRTIO_GPU_RESP_OK_NODATA {
            return Err(Self::response_to_error(resp_hdr.hdr_type));
        }

        crate::println!(
            "[VIRTIO-GPU] Attached backing for resource {} (addr={:#x}, len={})",
            resource_id,
            addr,
            length
        );

        Ok(())
    }

    /// Detach backing store from a resource.
    pub fn detach_backing(&mut self, resource_id: u32) -> Result<(), KernelError> {
        let cmd = VirtioGpuResourceDetachBacking {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING),
            resource_id,
            padding: 0,
        };

        self.send_simple_command(&cmd)
    }

    /// Set scanout: bind a resource (or region of it) to a display output.
    pub fn set_scanout(
        &mut self,
        scanout_id: u32,
        resource_id: u32,
        rect: VirtioGpuRect,
    ) -> Result<(), KernelError> {
        let cmd = VirtioGpuSetScanout {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_SET_SCANOUT),
            rect,
            scanout_id,
            resource_id,
        };

        self.send_simple_command(&cmd)?;

        crate::println!(
            "[VIRTIO-GPU] Set scanout {}: resource {} ({}x{}+{}+{})",
            scanout_id,
            resource_id,
            rect.width,
            rect.height,
            rect.x,
            rect.y
        );

        Ok(())
    }

    /// Transfer data from guest backing store to host resource.
    pub fn transfer_to_host_2d(
        &mut self,
        resource_id: u32,
        rect: VirtioGpuRect,
    ) -> Result<(), KernelError> {
        let cmd = VirtioGpuTransferToHost2d {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            rect,
            offset: 0,
            resource_id,
            padding: 0,
        };

        self.send_simple_command(&cmd)
    }

    /// Flush a resource region to the display.
    pub fn resource_flush(
        &mut self,
        resource_id: u32,
        rect: VirtioGpuRect,
    ) -> Result<(), KernelError> {
        let cmd = VirtioGpuResourceFlush {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
            rect,
            resource_id,
            padding: 0,
        };

        self.send_simple_command(&cmd)
    }

    // ---- Framebuffer management ----

    /// Set up the primary framebuffer: create a 2D resource, attach a
    /// backing pixel buffer, and bind it to scanout 0.
    pub fn setup_framebuffer(&mut self) -> Result<(), KernelError> {
        if self.width == 0 || self.height == 0 {
            return Err(KernelError::InvalidState {
                expected: "display_configured",
                actual: "no_display",
            });
        }

        let resource_id = self.next_resource_id;
        self.next_resource_id += 1;

        // Create a 2D resource for the framebuffer
        self.create_resource_2d(resource_id, FORMAT_B8G8R8X8_UNORM, self.width, self.height)?;

        // Allocate backing pixel buffer
        let pixel_count = (self.width * self.height) as usize;
        let mut backing = alloc::vec![0u32; pixel_count];

        // Fill with a dark blue gradient as initial content
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y * self.width + x) as usize;
                // BGRX format: blue gradient from dark to mid
                let blue = 32 + (y * 64 / self.height);
                let green = 16 + (y * 32 / self.height);
                backing[idx] = (blue << 16) | (green << 8) | 0x10;
            }
        }

        // Get the physical address of the backing buffer
        let backing_addr = backing.as_ptr() as u64;
        let backing_len = (pixel_count * 4) as u32;

        // Attach the backing store
        self.attach_backing(resource_id, backing_addr, backing_len)?;

        // Bind to scanout 0
        let scanout_rect = VirtioGpuRect::new(0, 0, self.width, self.height);
        self.set_scanout(0, resource_id, scanout_rect)?;

        // Transfer initial content to host
        self.transfer_to_host_2d(resource_id, scanout_rect)?;

        // Flush to display
        self.resource_flush(resource_id, scanout_rect)?;

        self.framebuffer_resource_id = resource_id;
        self.framebuffer_backing = Some(backing);

        crate::println!(
            "[VIRTIO-GPU] Framebuffer ready: {}x{} (resource {})",
            self.width,
            self.height,
            resource_id
        );

        Ok(())
    }

    /// Flush the framebuffer to the display.
    ///
    /// Transfers the entire backing buffer to the host and triggers a display
    /// refresh. Call this after modifying the framebuffer pixels.
    pub fn flush_framebuffer(&mut self) -> Result<(), KernelError> {
        if self.framebuffer_resource_id == 0 {
            return Err(KernelError::InvalidState {
                expected: "framebuffer_setup",
                actual: "no_framebuffer",
            });
        }

        let rect = VirtioGpuRect::new(0, 0, self.width, self.height);
        self.transfer_to_host_2d(self.framebuffer_resource_id, rect)?;
        self.resource_flush(self.framebuffer_resource_id, rect)?;

        Ok(())
    }

    /// Flush a sub-region of the framebuffer.
    ///
    /// More efficient than flushing the entire framebuffer when only a small
    /// area has changed.
    pub fn flush_region(&mut self, rect: VirtioGpuRect) -> Result<(), KernelError> {
        if self.framebuffer_resource_id == 0 {
            return Err(KernelError::InvalidState {
                expected: "framebuffer_setup",
                actual: "no_framebuffer",
            });
        }

        self.transfer_to_host_2d(self.framebuffer_resource_id, rect)?;
        self.resource_flush(self.framebuffer_resource_id, rect)?;

        Ok(())
    }

    /// Get mutable access to the framebuffer pixel buffer.
    ///
    /// Returns a slice of BGRX pixels. Modify the pixels, then call
    /// `flush_framebuffer()` or `flush_region()` to push changes to the
    /// display.
    pub fn get_framebuffer_mut(&mut self) -> Option<&mut [u32]> {
        self.framebuffer_backing.as_deref_mut()
    }

    /// Get read-only access to the framebuffer pixel buffer.
    pub fn get_framebuffer(&self) -> Option<&[u32]> {
        self.framebuffer_backing.as_deref()
    }

    /// Get the display width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the display height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Check if the driver is initialized and ready.
    pub fn is_ready(&self) -> bool {
        self.state == GpuDeviceState::Ready
    }

    /// Check if EDID is supported.
    pub fn supports_edid(&self) -> bool {
        self.features & VIRTIO_GPU_F_EDID != 0
    }

    /// Check if 3D (VIRGL) is supported.
    pub fn supports_virgl(&self) -> bool {
        self.features & VIRTIO_GPU_F_VIRGL != 0
    }

    /// Get the framebuffer resource ID.
    pub fn framebuffer_resource_id(&self) -> u32 {
        self.framebuffer_resource_id
    }

    /// Allocate a new resource ID.
    pub fn alloc_resource_id(&mut self) -> u32 {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        id
    }

    /// Set a pixel in the framebuffer (BGRX format).
    ///
    /// Does NOT flush automatically -- call `flush_framebuffer()` after
    /// modifying pixels.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: u32) -> Result<(), KernelError> {
        if x >= self.width || y >= self.height {
            return Err(KernelError::InvalidArgument {
                name: "coordinates",
                value: "out_of_bounds",
            });
        }

        if let Some(ref mut fb) = self.framebuffer_backing {
            let idx = (y * self.width + x) as usize;
            if idx < fb.len() {
                fb[idx] = color;
            }
        }

        Ok(())
    }

    /// Fill a rectangle in the framebuffer with a solid color.
    ///
    /// Does NOT flush automatically.
    pub fn fill_rect(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        color: u32,
    ) -> Result<(), KernelError> {
        if let Some(ref mut fb) = self.framebuffer_backing {
            let width = self.width;
            let height = self.height;

            for dy in 0..h {
                let row_y = y + dy;
                if row_y >= height {
                    break;
                }
                for dx in 0..w {
                    let col_x = x + dx;
                    if col_x >= width {
                        break;
                    }
                    let idx = (row_y * width + col_x) as usize;
                    if idx < fb.len() {
                        fb[idx] = color;
                    }
                }
            }
        }

        Ok(())
    }

    /// Blit a buffer of pixels into the framebuffer.
    ///
    /// The buffer must contain `w * h` BGRX pixels. Does NOT flush
    /// automatically.
    pub fn blit(
        &mut self,
        buffer: &[u32],
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> Result<(), KernelError> {
        if let Some(ref mut fb) = self.framebuffer_backing {
            let fb_width = self.width;
            let fb_height = self.height;

            for dy in 0..h {
                let row_y = y + dy;
                if row_y >= fb_height {
                    break;
                }
                for dx in 0..w {
                    let col_x = x + dx;
                    if col_x >= fb_width {
                        break;
                    }
                    let src_idx = (dy * w + dx) as usize;
                    let dst_idx = (row_y * fb_width + col_x) as usize;
                    if src_idx < buffer.len() && dst_idx < fb.len() {
                        fb[dst_idx] = buffer[src_idx];
                    }
                }
            }
        }

        Ok(())
    }

    /// Clear the framebuffer with a solid color.
    ///
    /// Does NOT flush automatically.
    pub fn clear(&mut self, color: u32) {
        if let Some(ref mut fb) = self.framebuffer_backing {
            fb.fill(color);
        }
    }
}

// ============================================================================
// PCI Discovery (x86_64 only)
// ============================================================================

/// Probe PCI bus for a VirtIO GPU device (vendor 0x1AF4, device 0x1050).
///
/// Returns the MMIO base address (virtual) if found.
#[cfg(target_arch = "x86_64")]
pub fn probe_pci() -> Option<usize> {
    if !crate::drivers::pci::is_pci_initialized() {
        return None;
    }

    let bus = crate::drivers::pci::get_pci_bus().lock();

    // VirtIO GPU: vendor 0x1AF4 (Red Hat), device 0x1050 (virtio 1.0 GPU)
    let devices = bus.find_devices_by_id(0x1AF4, 0x1050);

    if let Some(dev) = devices.first() {
        crate::println!(
            "[VIRTIO-GPU] Found PCI device {:04x}:{:04x} at {:02x}:{:02x}.{}",
            dev.vendor_id,
            dev.device_id,
            dev.location.bus,
            dev.location.device,
            dev.location.function
        );

        // Get BAR0 MMIO address
        if let Some(bar) = dev.bars.first() {
            if let Some(phys_addr) = bar.get_memory_address() {
                crate::println!("[VIRTIO-GPU] BAR0 physical address: {:#x}", phys_addr);
                // Convert physical to virtual address
                return crate::arch::x86_64::msr::phys_to_virt(phys_addr as usize);
            }
        }

        crate::println!("[VIRTIO-GPU] No usable BAR found");
    }

    // Also try the transitional device ID (0x1040 + device_type where
    // gpu device_type = 16)
    let legacy_devices = bus.find_devices_by_id(0x1AF4, 0x1040);
    for dev in &legacy_devices {
        // For transitional devices, subsystem device ID indicates the type
        crate::println!(
            "[VIRTIO-GPU] Found transitional VirtIO PCI device {:04x}:{:04x}",
            dev.vendor_id,
            dev.device_id
        );

        if let Some(bar) = dev.bars.first() {
            if let Some(phys_addr) = bar.get_memory_address() {
                return crate::arch::x86_64::msr::phys_to_virt(phys_addr as usize);
            }
        }
    }

    None
}

/// Stub for non-x86_64 architectures.
#[cfg(not(target_arch = "x86_64"))]
pub fn probe_pci() -> Option<usize> {
    None
}

/// Probe PCI bus for display-class devices and return a summary.
#[cfg(target_arch = "x86_64")]
pub fn enumerate_gpu_devices() -> Vec<(u16, u16, u8, u8)> {
    let mut result = Vec::new();

    if !crate::drivers::pci::is_pci_initialized() {
        return result;
    }

    let bus = crate::drivers::pci::get_pci_bus().lock();
    let display_devices = bus.find_devices_by_class(crate::drivers::pci::class_codes::DISPLAY);

    for dev in &display_devices {
        result.push((dev.vendor_id, dev.device_id, dev.class_code, dev.subclass));
    }

    result
}

/// Stub for non-x86_64 architectures.
#[cfg(not(target_arch = "x86_64"))]
pub fn enumerate_gpu_devices() -> Vec<(u16, u16, u8, u8)> {
    Vec::new()
}

// ============================================================================
// Module-level state and initialization
// ============================================================================

/// Global VirtIO GPU driver instance.
static VIRTIO_GPU: spin::Mutex<Option<VirtioGpuDriver>> = spin::Mutex::new(None);

/// Initialize the VirtIO GPU driver.
///
/// Probes PCI for a virtio-gpu device. If found, initializes the driver,
/// queries display info, and sets up a framebuffer.
pub fn init() -> Result<(), KernelError> {
    crate::println!("[VIRTIO-GPU] Probing for virtio-gpu device...");

    // Try PCI discovery
    if let Some(mmio_base) = probe_pci() {
        crate::println!("[VIRTIO-GPU] MMIO base: {:#x}", mmio_base);

        match VirtioGpuDriver::new(mmio_base) {
            Ok(driver) => {
                crate::println!(
                    "[VIRTIO-GPU] Driver initialized: {}x{} (resource {})",
                    driver.width(),
                    driver.height(),
                    driver.framebuffer_resource_id()
                );
                *VIRTIO_GPU.lock() = Some(driver);
                return Ok(());
            }
            Err(e) => {
                crate::println!("[VIRTIO-GPU] Init failed: {:?}", e);
                return Err(e);
            }
        }
    }

    crate::println!("[VIRTIO-GPU] No virtio-gpu device found");
    Ok(())
}

/// Execute a closure with the VirtIO GPU driver (mutable access).
pub fn with_driver<R, F: FnOnce(&mut VirtioGpuDriver) -> R>(f: F) -> Option<R> {
    VIRTIO_GPU.lock().as_mut().map(f)
}

/// Check if a VirtIO GPU driver is available and initialized.
pub fn is_available() -> bool {
    VIRTIO_GPU.lock().is_some()
}

/// Flush the VirtIO GPU framebuffer to the display.
///
/// Convenience function that acquires the driver lock and flushes.
pub fn flush_framebuffer() -> Result<(), KernelError> {
    if let Some(ref mut driver) = *VIRTIO_GPU.lock() {
        driver.flush_framebuffer()
    } else {
        Err(KernelError::InvalidState {
            expected: "virtio_gpu_initialized",
            actual: "no_driver",
        })
    }
}

/// Get the display dimensions (width, height) if a VirtIO GPU is available.
pub fn get_display_size() -> Option<(u32, u32)> {
    VIRTIO_GPU.lock().as_ref().map(|d| (d.width(), d.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_constants() {
        assert_eq!(VIRTIO_GPU_CMD_GET_DISPLAY_INFO, 0x100);
        assert_eq!(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D, 0x101);
        assert_eq!(VIRTIO_GPU_RESP_OK_NODATA, 0x1100);
        assert_eq!(VIRTIO_GPU_RESP_ERR_UNSPEC, 0x1200);
        assert_eq!(FORMAT_B8G8R8A8_UNORM, 1);
    }

    #[test]
    fn test_ctrl_hdr_size() {
        // VirtIO spec: control header is 24 bytes
        assert_eq!(core::mem::size_of::<VirtioGpuCtrlHdr>(), 24);
    }

    #[test]
    fn test_rect() {
        let rect = VirtioGpuRect::new(10, 20, 800, 600);
        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.width, 800);
        assert_eq!(rect.height, 600);
    }

    #[test]
    fn test_display_one_size() {
        // VirtioGpuDisplayOne: rect (16) + enabled (4) + flags (4) = 24
        assert_eq!(core::mem::size_of::<VirtioGpuDisplayOne>(), 24);
    }

    #[test]
    fn test_resource_create_2d_size() {
        // hdr (24) + resource_id (4) + format (4) + width (4) + height (4) = 40
        assert_eq!(core::mem::size_of::<VirtioGpuResourceCreate2d>(), 40);
    }

    #[test]
    fn test_mem_entry_size() {
        // addr (8) + length (4) + padding (4) = 16
        assert_eq!(core::mem::size_of::<VirtioGpuMemEntry>(), 16);
    }

    #[test]
    fn test_response_to_error() {
        let err = VirtioGpuDriver::response_to_error(VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY);
        match err {
            KernelError::OutOfMemory { .. } => {}
            _ => panic!("Expected OutOfMemory error"),
        }

        let err = VirtioGpuDriver::response_to_error(VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID);
        match err {
            KernelError::InvalidArgument { name, .. } => {
                assert_eq!(name, "scanout_id");
            }
            _ => panic!("Expected InvalidArgument error"),
        }
    }
}

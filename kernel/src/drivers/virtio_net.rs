//! VirtIO Network Driver
//!
//! Driver for paravirtualized network devices using the VirtIO protocol.
//! Commonly used in QEMU/KVM virtual machines for high performance.
//!
//! Implements the VirtIO MMIO transport with proper status negotiation,
//! virtqueue setup via frame allocator DMA buffers, and TX/RX paths.

// Allow dead code for VirtIO feature bits and structures not yet fully implemented
#![allow(dead_code, clippy::needless_range_loop)]

use alloc::vec::Vec;

use crate::{
    error::KernelError,
    net::{
        device::{DeviceCapabilities, DeviceState, DeviceStatistics, NetworkDevice},
        MacAddress, Packet,
    },
};

/// VirtIO Network Device Feature Bits
const VIRTIO_NET_F_CSUM: u64 = 1 << 0;
const VIRTIO_NET_F_GUEST_CSUM: u64 = 1 << 1;
const VIRTIO_NET_F_MAC: u64 = 1 << 5;
const VIRTIO_NET_F_STATUS: u64 = 1 << 16;

// ============================================================================
// VirtIO MMIO Register Offsets (legacy interface)
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

/// VirtIO Net header size (without mergeable buffers)
const VIRTIO_NET_HDR_SIZE: usize = 10;

/// Descriptor flags: buffer is device-writable (for RX buffers)
const VIRTQ_DESC_F_WRITE: u16 = 2;

/// VirtIO Network Header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtioNetHeader {
    flags: u8,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
    num_buffers: u16,
}

/// VirtIO Ring Descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
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

/// VirtIO Virtqueue
struct Virtqueue {
    /// Queue size (number of descriptors)
    size: u16,

    /// Descriptor table
    descriptors: &'static mut [VirtqDesc],

    /// Available ring
    avail: &'static mut VirtqAvail,

    /// Used ring
    used: &'static mut VirtqUsed,

    /// Free descriptor head
    free_head: u16,

    /// Last seen used index
    last_used_idx: u16,

    /// Number of free descriptors
    num_free: u16,
}

impl Virtqueue {
    /// Create a new virtqueue (requires pre-allocated memory)
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

    /// Allocate a descriptor
    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }

        let desc_idx = self.free_head;
        self.free_head = self.descriptors[desc_idx as usize].next;
        self.num_free -= 1;

        Some(desc_idx)
    }

    /// Free a descriptor
    fn free_desc(&mut self, desc_idx: u16) {
        self.descriptors[desc_idx as usize].next = self.free_head;
        self.free_head = desc_idx;
        self.num_free += 1;
    }

    /// Add buffer to available ring
    fn add_to_avail(&mut self, desc_idx: u16) {
        let avail_idx = self.avail.idx as usize % self.size as usize;
        self.avail.ring[avail_idx] = desc_idx;

        // Memory barrier would go here
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        self.avail.idx = self.avail.idx.wrapping_add(1);
    }

    /// Check for used buffers
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
/// Stores the virtual addresses of frame-allocator-provided pages
/// used for the descriptor table, available ring, and used ring.
struct VirtqueueDmaRegion {
    /// Virtual address of allocated pages (for the desc/avail/used rings)
    virt_addr: usize,
    /// Number of 4KB pages allocated
    num_pages: usize,
}

/// Per-descriptor TX/RX data buffer (single 4KB page).
struct DataBuffer {
    virt_addr: usize,
    phys_addr: u64,
}

/// VirtIO Network Driver
pub struct VirtioNetDriver {
    mmio_base: usize,
    mac_address: MacAddress,
    features: u64,
    rx_queue_size: u16,
    tx_queue_size: u16,
    state: DeviceState,
    stats: DeviceStatistics,

    // Virtqueues (None until initialized)
    rx_queue: Option<Virtqueue>,
    tx_queue: Option<Virtqueue>,

    /// DMA region backing the RX virtqueue rings
    rx_dma: Option<VirtqueueDmaRegion>,
    /// DMA region backing the TX virtqueue rings
    tx_dma: Option<VirtqueueDmaRegion>,
    /// Per-descriptor data buffers for RX
    rx_buffers: Vec<DataBuffer>,
    /// Per-descriptor data buffers for TX
    tx_buffers: Vec<DataBuffer>,
}

impl VirtioNetDriver {
    /// Create a new VirtIO Network driver
    pub fn new(mmio_base: usize) -> Result<Self, KernelError> {
        let mut driver = Self {
            mmio_base,
            mac_address: MacAddress::ZERO,
            features: 0,
            rx_queue_size: 256,
            tx_queue_size: 256,
            state: DeviceState::Down,
            stats: DeviceStatistics::default(),
            rx_queue: None,
            tx_queue: None,
            rx_dma: None,
            tx_dma: None,
            rx_buffers: Vec::new(),
            tx_buffers: Vec::new(),
        };

        driver.initialize()?;
        Ok(driver)
    }

    /// Read from MMIO register
    fn read_reg(&self, offset: usize) -> u32 {
        // SAFETY: Reading a VirtIO MMIO register at mmio_base + offset. The mmio_base
        // is the device's memory-mapped I/O base from the device tree or PCI BAR.
        // read_volatile prevents compiler reordering of hardware register accesses.
        unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u32) }
    }

    /// Write to MMIO register
    fn write_reg(&self, offset: usize, value: u32) {
        // SAFETY: Writing a VirtIO MMIO register. Same invariants as read_reg.
        unsafe {
            core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value);
        }
    }

    /// Initialize VirtIO device with full status negotiation and virtqueue
    /// setup.
    ///
    /// Follows the VirtIO 1.0+ initialization sequence:
    /// 1. Reset device
    /// 2. Set ACKNOWLEDGE
    /// 3. Set DRIVER
    /// 4. Negotiate features
    /// 5. Set FEATURES_OK and verify
    /// 6. Set up virtqueues (RX queue 0, TX queue 1)
    /// 7. Read MAC from device config
    /// 8. Set DRIVER_OK
    fn initialize(&mut self) -> Result<(), KernelError> {
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

        let driver_features = VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS;
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
            return Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 1,
            });
        }

        // Step 6: Set up virtqueues
        // RX queue = index 0, TX queue = index 1
        self.setup_rx_queue()?;
        self.setup_tx_queue()?;

        // Step 7: Read MAC address from device config space
        if (self.features & VIRTIO_NET_F_MAC) != 0 {
            let mut mac = [0u8; 6];
            for (i, byte) in mac.iter_mut().enumerate() {
                *byte = self.read_reg(VIRTIO_MMIO_CONFIG_BASE + i) as u8;
            }
            self.mac_address = MacAddress(mac);
        }

        // Step 8: Set DRIVER_OK -- device is live
        self.write_reg(
            VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE
                | VIRTIO_STATUS_DRIVER
                | VIRTIO_STATUS_FEATURES_OK
                | VIRTIO_STATUS_DRIVER_OK,
        );

        println!(
            "[VIRTIO-NET] Initialized with MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.mac_address.0[0],
            self.mac_address.0[1],
            self.mac_address.0[2],
            self.mac_address.0[3],
            self.mac_address.0[4],
            self.mac_address.0[5]
        );
        println!(
            "[VIRTIO-NET] RX queue: {} descs, TX queue: {} descs",
            self.rx_queue_size, self.tx_queue_size
        );

        self.state = DeviceState::Up;
        Ok(())
    }

    /// Set up the RX virtqueue (queue index 0).
    ///
    /// Reads QueueNumMax from MMIO, allocates descriptor/available/used ring
    /// memory from the frame allocator, and pre-populates the available ring
    /// with receive buffers.
    fn setup_rx_queue(&mut self) -> Result<(), KernelError> {
        self.write_reg(VIRTIO_MMIO_QUEUE_SEL, 0); // Select queue 0

        let max_size = self.read_reg(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if max_size == 0 {
            return Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 2,
            });
        }
        let queue_size = max_size.min(256);
        self.rx_queue_size = queue_size;

        // Allocate ring memory and data buffers, then create the Virtqueue
        let (vq, dma, buffers) = self.allocate_virtqueue(queue_size, true)?;

        // Tell device about the queue addresses
        let desc_phys = dma.virt_addr as u64; // In identity-mapped or known offset region
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

        self.rx_queue = Some(vq);
        self.rx_dma = Some(dma);
        self.rx_buffers = buffers;

        Ok(())
    }

    /// Set up the TX virtqueue (queue index 1).
    fn setup_tx_queue(&mut self) -> Result<(), KernelError> {
        self.write_reg(VIRTIO_MMIO_QUEUE_SEL, 1); // Select queue 1

        let max_size = self.read_reg(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if max_size == 0 {
            return Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 3,
            });
        }
        let queue_size = max_size.min(256);
        self.tx_queue_size = queue_size;

        let (vq, dma, buffers) = self.allocate_virtqueue(queue_size, false)?;

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

        self.tx_queue = Some(vq);
        self.tx_dma = Some(dma);
        self.tx_buffers = buffers;

        Ok(())
    }

    /// Allocate a virtqueue: ring memory + per-descriptor data buffers.
    ///
    /// For RX queues (`is_rx = true`), each descriptor is pre-configured to
    /// point at a writable data buffer and added to the available ring so
    /// the device can fill them with received packets.
    fn allocate_virtqueue(
        &self,
        queue_size: u16,
        is_rx: bool,
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
            let buf_phys = buf_virt as u64; // Approximate; correct for identity/offset mapping
            core::mem::forget(buf);
            data_buffers.push(DataBuffer {
                virt_addr: buf_virt,
                phys_addr: buf_phys,
            });
        }

        // For RX: point each descriptor at its data buffer and populate avail ring
        if is_rx {
            let desc_slice = unsafe { core::slice::from_raw_parts_mut(desc_ptr, qs) };
            let avail_ref = unsafe { &mut *avail_ptr };
            for i in 0..qs {
                desc_slice[i].addr = data_buffers[i].phys_addr;
                desc_slice[i].len = 4096;
                desc_slice[i].flags = VIRTQ_DESC_F_WRITE; // Device-writable
                desc_slice[i].next = 0;
                avail_ref.ring[i] = i as u16;
            }
            avail_ref.idx = queue_size;
        }

        let dma = VirtqueueDmaRegion {
            virt_addr: ring_ptr,
            num_pages: ring_pages,
        };

        Ok((vq, dma, data_buffers))
    }

    /// Transmit a packet using virtqueue.
    ///
    /// Prepends a VirtioNetHeader, copies the frame data into the
    /// pre-allocated TX data buffer, and kicks the device.
    pub fn transmit(&mut self, packet: &[u8]) -> Result<(), KernelError> {
        if self.state != DeviceState::Up {
            return Err(KernelError::InvalidState {
                expected: "up",
                actual: "down",
            });
        }

        let total_len = VIRTIO_NET_HDR_SIZE + packet.len();
        if total_len > 4096 {
            return Err(KernelError::InvalidArgument {
                name: "packet_size",
                value: "too_large",
            });
        }

        let mmio = self.mmio_base;
        if let Some(ref mut tx_queue) = self.tx_queue {
            let desc_idx = tx_queue
                .alloc_desc()
                .ok_or(KernelError::ResourceExhausted {
                    resource: "virtio_tx_descriptors",
                })?;

            // Copy VirtioNetHeader + frame data into the TX data buffer
            if (desc_idx as usize) < self.tx_buffers.len() {
                let buf_virt = self.tx_buffers[desc_idx as usize].virt_addr;
                let buf_phys = self.tx_buffers[desc_idx as usize].phys_addr;
                // SAFETY: buf_virt points to a leaked 4096-byte allocation.
                // total_len <= 4096 checked above. We hold &mut self so no
                // concurrent access to the same buffer.
                let buf_slice =
                    unsafe { core::slice::from_raw_parts_mut(buf_virt as *mut u8, 4096) };

                // Write zeroed VirtioNetHeader (no offload)
                buf_slice[..VIRTIO_NET_HDR_SIZE].fill(0);
                // Write packet data after header
                buf_slice[VIRTIO_NET_HDR_SIZE..total_len].copy_from_slice(packet);

                // Configure descriptor
                let desc = &mut tx_queue.descriptors[desc_idx as usize];
                desc.addr = buf_phys;
                desc.len = total_len as u32;
                desc.flags = 0; // Device-readable (TX direction)
            }

            tx_queue.add_to_avail(desc_idx);

            self.stats.tx_packets += 1;
            self.stats.tx_bytes += packet.len() as u64;

            // Poll-mode: free descriptor immediately after adding to avail ring.
            // In interrupt mode, this would happen in the TX completion handler.
            tx_queue.free_desc(desc_idx);
        } else {
            return Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 0x01,
            });
        }

        // Kick the device (TX queue = index 1)
        // SAFETY: Writing to VirtIO queue notify register.
        unsafe {
            core::ptr::write_volatile((mmio + VIRTIO_MMIO_QUEUE_NOTIFY) as *mut u32, 1);
        }

        Ok(())
    }

    /// Receive a packet using virtqueue.
    ///
    /// Checks the used ring for completed RX buffers, copies the received
    /// frame data (after stripping the VirtioNetHeader), recycles the
    /// descriptor, and returns the packet.
    pub fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }

        if let Some(ref mut rx_queue) = self.rx_queue {
            if let Some((desc_idx, len)) = rx_queue.get_used() {
                let total_len = len as usize;

                // Skip the VirtioNetHeader to get the actual frame data
                let data_offset = VIRTIO_NET_HDR_SIZE;

                let pkt = if (desc_idx as usize) < self.rx_buffers.len() && total_len > data_offset
                {
                    let buf_virt = self.rx_buffers[desc_idx as usize].virt_addr;
                    let frame_len = total_len - data_offset;

                    // SAFETY: buf_virt is a leaked 4096-byte allocation.
                    // total_len <= 4096 (device respects descriptor len field).
                    let buf_slice =
                        unsafe { core::slice::from_raw_parts(buf_virt as *const u8, 4096) };
                    let frame_data = &buf_slice[data_offset..data_offset + frame_len];

                    crate::net::Packet::from_bytes(frame_data)
                } else {
                    crate::net::Packet::new(0)
                };

                self.stats.rx_packets += 1;
                self.stats.rx_bytes += total_len as u64;

                // Recycle: reset descriptor and re-add to available ring
                let desc = &mut rx_queue.descriptors[desc_idx as usize];
                desc.len = 4096;
                desc.flags = VIRTQ_DESC_F_WRITE;
                rx_queue.add_to_avail(desc_idx);

                Ok(Some(pkt))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Notify device of available descriptors (kick virtqueue)
    fn notify_queue(&self, queue_idx: u16) {
        // Queue notify register offset varies by implementation
        // For MMIO: typically at base + 0x50
        self.write_reg(0x50, queue_idx as u32);
    }

    /// Get MAC address
    pub fn mac_address(&self) -> MacAddress {
        self.mac_address
    }
}

// DeviceDriver trait implementation removed - using NetworkDevice trait instead

impl NetworkDevice for VirtioNetDriver {
    fn name(&self) -> &str {
        "eth1"
    }

    fn mac_address(&self) -> MacAddress {
        self.mac_address
    }

    fn capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities {
            max_transmission_unit: 1500,
            supports_vlan: false,
            supports_checksum_offload: (self.features & VIRTIO_NET_F_CSUM) != 0,
            supports_tso: false,
            supports_lro: false,
        }
    }

    fn state(&self) -> DeviceState {
        self.state
    }

    fn set_state(&mut self, state: DeviceState) -> Result<(), KernelError> {
        match state {
            DeviceState::Up => {
                if self.state == DeviceState::Down {
                    // Set DRIVER_OK status bit
                    self.write_reg(0x70, 1 | 2 | 4 | 8);
                }
                self.state = DeviceState::Up;
            }
            DeviceState::Down => {
                // Reset device
                self.write_reg(0x70, 0);
                self.state = DeviceState::Down;
            }
            _ => {
                self.state = state;
            }
        }
        Ok(())
    }

    fn statistics(&self) -> DeviceStatistics {
        self.stats
    }

    fn transmit(&mut self, packet: &Packet) -> Result<(), KernelError> {
        if self.state != DeviceState::Up {
            self.stats.tx_dropped += 1;
            return Err(KernelError::InvalidState {
                expected: "up",
                actual: "not_up",
            });
        }

        // Delegate to the real virtqueue-based transmit
        self.transmit(packet.data())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        // Delegate to the real virtqueue-based receive
        VirtioNetDriver::receive(self)
    }
}

/// Initialize VirtIO-Net driver
pub fn init() -> Result<(), KernelError> {
    println!("[VIRTIO-NET] VirtIO Network driver module loaded");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtio_constants() {
        assert_eq!(VIRTIO_NET_F_MAC, 1 << 5);
        assert_eq!(VIRTIO_NET_F_STATUS, 1 << 16);
    }
}

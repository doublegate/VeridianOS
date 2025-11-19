//! VirtIO Network Driver
//!
//! Driver for paravirtualized network devices using the VirtIO protocol.
//! Commonly used in QEMU/KVM virtual machines for high performance.

use crate::error::KernelError;
use crate::net::{MacAddress, Packet};
use crate::net::device::{NetworkDevice, DeviceCapabilities, DeviceStatistics, DeviceState};

/// VirtIO Network Device Feature Bits
const VIRTIO_NET_F_CSUM: u64 = 1 << 0;
const VIRTIO_NET_F_GUEST_CSUM: u64 = 1 << 1;
const VIRTIO_NET_F_MAC: u64 = 1 << 5;
const VIRTIO_NET_F_STATUS: u64 = 1 << 16;

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
    fn new(descriptors: &'static mut [VirtqDesc],
           avail: &'static mut VirtqAvail,
           used: &'static mut VirtqUsed,
           size: u16) -> Self {
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
        };

        driver.initialize()?;
        Ok(driver)
    }

    /// Read from MMIO register
    fn read_reg(&self, offset: usize) -> u32 {
        unsafe {
            core::ptr::read_volatile((self.mmio_base + offset) as *const u32)
        }
    }

    /// Write to MMIO register
    fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value);
        }
    }

    /// Initialize VirtIO device
    fn initialize(&mut self) -> Result<(), KernelError> {
        // Reset device
        self.write_reg(0x70, 0);

        // Set ACKNOWLEDGE status bit
        self.write_reg(0x70, 1);

        // Set DRIVER status bit
        self.write_reg(0x70, 1 | 2);

        // Read device features
        self.write_reg(0x14, 0); // Select features word 0
        let features_low = self.read_reg(0x10) as u64;
        self.write_reg(0x14, 1); // Select features word 1
        let features_high = (self.read_reg(0x10) as u64) << 32;
        self.features = features_low | features_high;

        // Negotiate features
        let driver_features = VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS;
        self.write_reg(0x24, 0); // Select features word 0
        self.write_reg(0x20, (driver_features & 0xFFFFFFFF) as u32);
        self.write_reg(0x24, 1); // Select features word 1
        self.write_reg(0x20, (driver_features >> 32) as u32);

        // Set FEATURES_OK status bit
        self.write_reg(0x70, 1 | 2 | 8);

        // Verify FEATURES_OK
        if (self.read_reg(0x70) & 8) == 0 {
            return Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 1,
            });
        }

        // Read MAC address if supported
        if (self.features & VIRTIO_NET_F_MAC) != 0 {
            let mut mac = [0u8; 6];
            for i in 0..6 {
                mac[i] = self.read_reg(0x100 + i) as u8;
            }
            self.mac_address = MacAddress(mac);
        }

        // Set DRIVER_OK status bit
        self.write_reg(0x70, 1 | 2 | 4 | 8);

        println!("[VIRTIO-NET] Initialized with MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                 self.mac_address.0[0], self.mac_address.0[1], self.mac_address.0[2],
                 self.mac_address.0[3], self.mac_address.0[4], self.mac_address.0[5]);

        // Device is now up
        self.state = DeviceState::Up;

        Ok(())
    }

    /// Transmit a packet using virtqueue
    pub fn transmit(&mut self, packet: &[u8]) -> Result<(), KernelError> {
        if self.state != DeviceState::Up {
            return Err(KernelError::InvalidState {
                expected: "up",
                actual: "down",
            });
        }

        // NOTE: Full implementation requires DMA buffer pools
        // This shows the virtqueue logic but needs proper buffer allocation

        if let Some(ref mut tx_queue) = self.tx_queue {
            // Allocate a descriptor for the packet
            let desc_idx = tx_queue.alloc_desc().ok_or(KernelError::ResourceExhausted {
                resource: "virtio_tx_descriptors",
            })?;

            // TODO: Proper implementation would:
            // 1. Allocate DMA buffer from pool
            // 2. Copy packet data to DMA buffer
            // 3. Set up descriptor with DMA physical address
            // 4. Add to available ring
            // 5. Notify device via MMIO kick

            // For now, just track statistics
            self.stats.tx_packets += 1;
            self.stats.tx_bytes += packet.len() as u64;

            // Free descriptor (would normally be done after TX complete interrupt)
            tx_queue.free_desc(desc_idx);

            println!("[VIRTIO-NET] Transmitted {} bytes (virtqueue stub)", packet.len());
            Ok(())
        } else {
            Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 0x01, // TX queue not initialized
            })
        }
    }

    /// Receive a packet using virtqueue
    pub fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }

        // NOTE: Full implementation requires DMA buffer pools
        // This shows the virtqueue logic but needs proper buffer allocation

        if let Some(ref mut rx_queue) = self.rx_queue {
            // Check if there are any used buffers
            if let Some((desc_idx, len)) = rx_queue.get_used() {
                // TODO: Proper implementation would:
                // 1. Get DMA buffer address from descriptor
                // 2. Create Packet from DMA buffer data
                // 3. Free or recycle DMA buffer
                // 4. Add new buffer to RX queue for next packet

                self.stats.rx_packets += 1;
                self.stats.rx_bytes += len as u64;

                // Free descriptor
                rx_queue.free_desc(desc_idx);

                println!("[VIRTIO-NET] Received {} bytes (virtqueue stub)", len);

                // Would return actual packet here
                Ok(None)
            } else {
                // No packets available
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

        // TODO: Implement virtqueue-based transmission
        // For now, just update statistics
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += packet.len() as u64;

        println!("[VIRTIO-NET] Transmitting {} bytes (stub)", packet.len());
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }

        // TODO: Implement virtqueue-based reception
        Ok(None)
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

    #[test_case]
    fn test_virtio_constants() {
        assert_eq!(VIRTIO_NET_F_MAC, 1 << 5);
        assert_eq!(VIRTIO_NET_F_STATUS, 1 << 16);
    }
}

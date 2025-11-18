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

/// VirtIO Network Driver
pub struct VirtioNetDriver {
    mmio_base: usize,
    mac_address: MacAddress,
    features: u64,
    rx_queue_size: u16,
    tx_queue_size: u16,
    state: DeviceState,
    stats: DeviceStatistics,
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

    /// Transmit a packet
    pub fn transmit(&mut self, packet: &[u8]) -> Result<(), KernelError> {
        // TODO: Implement virtqueue-based transmission
        println!("[VIRTIO-NET] Transmitting {} bytes (stub)", packet.len());
        Ok(())
    }

    /// Receive a packet
    pub fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        // TODO: Implement virtqueue-based reception
        Ok(None)
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

//! Intel E1000 (82540EM) Network Driver
//!
//! This driver supports the Intel E1000 Gigabit Ethernet controller,
//! commonly found in QEMU and VirtualBox virtual machines.

use crate::{
    error::KernelError,
    net::{
        device::{DeviceCapabilities, DeviceState, DeviceStatistics, NetworkDevice},
        MacAddress, Packet,
    },
};

/// E1000 PCI vendor and device IDs
pub const E1000_VENDOR_ID: u16 = 0x8086;
pub const E1000_DEVICE_ID: u16 = 0x100E;

/// E1000 register offsets
const REG_CTRL: usize = 0x0000; // Device Control
const REG_STATUS: usize = 0x0008; // Device Status
const REG_EEPROM: usize = 0x0014; // EEPROM Read
const REG_CTRL_EXT: usize = 0x0018; // Extended Device Control
const REG_ICR: usize = 0x00C0; // Interrupt Cause Read
const REG_IMS: usize = 0x00D0; // Interrupt Mask Set
const REG_RCTL: usize = 0x0100; // Receive Control
const REG_TCTL: usize = 0x0400; // Transmit Control
const REG_RDBAL: usize = 0x2800; // RX Descriptor Base Low
const REG_RDBAH: usize = 0x2804; // RX Descriptor Base High
const REG_RDLEN: usize = 0x2808; // RX Descriptor Length
const REG_RDH: usize = 0x2810; // RX Descriptor Head
const REG_RDT: usize = 0x2818; // RX Descriptor Tail
const REG_TDBAL: usize = 0x3800; // TX Descriptor Base Low
const REG_TDBAH: usize = 0x3804; // TX Descriptor Base High
const REG_TDLEN: usize = 0x3808; // TX Descriptor Length
const REG_TDH: usize = 0x3810; // TX Descriptor Head
const REG_TDT: usize = 0x3818; // TX Descriptor Tail
const REG_MTA: usize = 0x5200; // Multicast Table Array

/// Number of RX/TX descriptors
const NUM_RX_DESC: usize = 32;
const NUM_TX_DESC: usize = 8;

/// Receive Descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RxDescriptor {
    addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

/// Transmit Descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct TxDescriptor {
    addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

/// E1000 Driver State
pub struct E1000Driver {
    mmio_base: usize,
    mac_address: MacAddress,
    rx_descriptors: [RxDescriptor; NUM_RX_DESC],
    tx_descriptors: [TxDescriptor; NUM_TX_DESC],
    rx_buffers: [[u8; 2048]; NUM_RX_DESC],
    tx_buffers: [[u8; 2048]; NUM_TX_DESC],
    rx_current: usize,
    tx_current: usize,
    state: DeviceState,
    stats: DeviceStatistics,
}

impl E1000Driver {
    /// Create a new E1000 driver instance
    pub fn new(mmio_base: usize) -> Result<Self, KernelError> {
        let mut driver = Self {
            mmio_base,
            mac_address: MacAddress::ZERO,
            rx_descriptors: [RxDescriptor {
                addr: 0,
                length: 0,
                checksum: 0,
                status: 0,
                errors: 0,
                special: 0,
            }; NUM_RX_DESC],
            tx_descriptors: [TxDescriptor {
                addr: 0,
                length: 0,
                cso: 0,
                cmd: 0,
                status: 0,
                css: 0,
                special: 0,
            }; NUM_TX_DESC],
            rx_buffers: [[0u8; 2048]; NUM_RX_DESC],
            tx_buffers: [[0u8; 2048]; NUM_TX_DESC],
            rx_current: 0,
            tx_current: 0,
            state: DeviceState::Down,
            stats: DeviceStatistics::default(),
        };

        driver.initialize()?;
        Ok(driver)
    }

    /// Read from MMIO register
    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u32) }
    }

    /// Write to MMIO register
    fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value);
        }
    }

    /// Read MAC address from EEPROM
    fn read_mac_address(&mut self) -> MacAddress {
        let mut mac = [0u8; 6];

        // Read from EEPROM words 0-2
        for i in 0usize..3 {
            let word = self.eeprom_read(i as u8);
            mac[i * 2] = (word & 0xFF) as u8;
            mac[i * 2 + 1] = (word >> 8) as u8;
        }

        MacAddress(mac)
    }

    /// Read from EEPROM
    fn eeprom_read(&self, addr: u8) -> u16 {
        self.write_reg(REG_EEPROM, 1 | ((addr as u32) << 8));

        // Wait for read to complete
        let mut result: u32;
        loop {
            result = self.read_reg(REG_EEPROM);
            if (result & (1 << 4)) != 0 {
                break;
            }
        }

        ((result >> 16) & 0xFFFF) as u16
    }

    /// Initialize the E1000 device
    fn initialize(&mut self) -> Result<(), KernelError> {
        // Read MAC address
        self.mac_address = self.read_mac_address();

        // Enable bus mastering and memory access
        // (Would normally be done via PCI configuration space)

        // Reset the device
        self.write_reg(REG_CTRL, self.read_reg(REG_CTRL) | 0x04000000);

        // Wait for reset
        for _ in 0..1000 {
            if (self.read_reg(REG_CTRL) & 0x04000000) == 0 {
                break;
            }
        }

        // Disable interrupts
        self.write_reg(REG_IMS, 0);
        self.read_reg(REG_ICR); // Clear pending interrupts

        // Initialize RX descriptors
        for i in 0..NUM_RX_DESC {
            self.rx_descriptors[i].addr = &self.rx_buffers[i] as *const _ as u64;
            self.rx_descriptors[i].status = 0;
        }

        // Initialize TX descriptors
        for i in 0..NUM_TX_DESC {
            self.tx_descriptors[i].addr = &self.tx_buffers[i] as *const _ as u64;
            self.tx_descriptors[i].status = 1; // DD bit set
            self.tx_descriptors[i].cmd = 0;
        }

        // Set up RX ring
        let rx_desc_addr = &self.rx_descriptors as *const _ as u64;
        self.write_reg(REG_RDBAL, (rx_desc_addr & 0xFFFFFFFF) as u32);
        self.write_reg(REG_RDBAH, (rx_desc_addr >> 32) as u32);
        self.write_reg(REG_RDLEN, (NUM_RX_DESC * 16) as u32);
        self.write_reg(REG_RDH, 0);
        self.write_reg(REG_RDT, (NUM_RX_DESC - 1) as u32);

        // Set up TX ring
        let tx_desc_addr = &self.tx_descriptors as *const _ as u64;
        self.write_reg(REG_TDBAL, (tx_desc_addr & 0xFFFFFFFF) as u32);
        self.write_reg(REG_TDBAH, (tx_desc_addr >> 32) as u32);
        self.write_reg(REG_TDLEN, (NUM_TX_DESC * 16) as u32);
        self.write_reg(REG_TDH, 0);
        self.write_reg(REG_TDT, 0);

        // Enable receiver
        self.write_reg(REG_RCTL, (1 << 1) | (1 << 2) | (1 << 15));

        // Enable transmitter
        self.write_reg(REG_TCTL, (1 << 1) | (1 << 3) | (0x10 << 4));

        // Clear multicast table
        for i in 0..128 {
            self.write_reg(REG_MTA + i * 4, 0);
        }

        println!(
            "[E1000] Initialized with MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.mac_address.0[0],
            self.mac_address.0[1],
            self.mac_address.0[2],
            self.mac_address.0[3],
            self.mac_address.0[4],
            self.mac_address.0[5]
        );

        // Device is now up
        self.state = DeviceState::Up;

        Ok(())
    }

    /// Transmit a packet (raw implementation)
    fn transmit_raw(&mut self, packet: &[u8]) -> Result<(), KernelError> {
        if packet.len() > 2048 {
            return Err(KernelError::InvalidArgument {
                name: "packet_size",
                value: "too_large",
            });
        }

        let idx = self.tx_current;
        let desc = &mut self.tx_descriptors[idx];

        // Wait for descriptor to be available
        if (desc.status & 1) == 0 {
            self.stats.tx_dropped += 1;
            return Err(KernelError::WouldBlock);
        }

        // Copy packet to TX buffer
        self.tx_buffers[idx][..packet.len()].copy_from_slice(packet);

        // Set up descriptor
        desc.length = packet.len() as u16;
        desc.cmd = (1 << 0) | (1 << 1) | (1 << 3); // EOP | IFCS | RS
        desc.status = 0;

        // Update tail pointer
        self.tx_current = (self.tx_current + 1) % NUM_TX_DESC;
        self.write_reg(REG_TDT, self.tx_current as u32);

        // Update statistics
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += packet.len() as u64;

        Ok(())
    }

    /// Receive a packet (raw implementation)
    fn receive_raw(&mut self) -> Result<Option<Packet>, KernelError> {
        let idx = self.rx_current;
        let desc = &mut self.rx_descriptors[idx];

        // Check if packet is available
        if (desc.status & 1) == 0 {
            return Ok(None);
        }

        // Check for errors
        if desc.errors != 0 {
            self.stats.rx_errors += 1;
            // Reset descriptor
            desc.status = 0;
            self.rx_current = (self.rx_current + 1) % NUM_RX_DESC;
            self.write_reg(REG_RDT, self.rx_current as u32);
            return Ok(None);
        }

        // Get packet data
        let len = desc.length as usize;
        let data = self.rx_buffers[idx][..len].to_vec();
        let packet = Packet::from_bytes(&data);

        // Update statistics
        self.stats.rx_packets += 1;
        self.stats.rx_bytes += len as u64;

        // Reset descriptor
        desc.status = 0;

        // Update tail pointer
        self.rx_current = (self.rx_current + 1) % NUM_RX_DESC;
        self.write_reg(REG_RDT, self.rx_current as u32);

        Ok(Some(packet))
    }

    /// Get MAC address
    pub fn mac_address(&self) -> MacAddress {
        self.mac_address
    }
}

// DeviceDriver trait implementation removed - using NetworkDevice trait instead

impl NetworkDevice for E1000Driver {
    fn name(&self) -> &str {
        "eth0"
    }

    fn mac_address(&self) -> MacAddress {
        self.mac_address
    }

    fn capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities {
            max_transmission_unit: 1500,
            supports_vlan: false,
            supports_checksum_offload: true,
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
                    // Re-enable RX and TX
                    self.write_reg(REG_RCTL, (1 << 1) | (1 << 2) | (1 << 15));
                    self.write_reg(REG_TCTL, (1 << 1) | (1 << 3) | (0x10 << 4));
                }
                self.state = DeviceState::Up;
            }
            DeviceState::Down => {
                // Disable RX and TX
                self.write_reg(REG_RCTL, 0);
                self.write_reg(REG_TCTL, 0);
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

        self.transmit_raw(packet.data())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }

        self.receive_raw()
    }
}

/// Initialize E1000 driver
pub fn init() -> Result<(), KernelError> {
    println!("[E1000] Intel E1000 network driver module loaded");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_e1000_constants() {
        assert_eq!(E1000_VENDOR_ID, 0x8086);
        assert_eq!(E1000_DEVICE_ID, 0x100E);
        assert_eq!(NUM_RX_DESC, 32);
        assert_eq!(NUM_TX_DESC, 8);
    }
}

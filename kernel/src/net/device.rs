//! Network device abstraction layer
//!
//! Defines the [`NetworkDevice`] trait and device registry for hardware
//! network drivers. All network drivers (E1000, VirtIO-Net, etc.)
//! implement this trait to provide a uniform interface for the IP stack.

#![allow(clippy::derivable_impls)]

use alloc::{boxed::Box, string::String, vec::Vec};

use spin::Mutex;

use super::{MacAddress, Packet};
use crate::error::KernelError;

/// Network device capabilities
#[derive(Debug, Clone, Copy)]
pub struct DeviceCapabilities {
    pub max_transmission_unit: usize,
    pub supports_vlan: bool,
    pub supports_checksum_offload: bool,
    pub supports_tso: bool, // TCP Segmentation Offload
    pub supports_lro: bool, // Large Receive Offload
}

impl Default for DeviceCapabilities {
    fn default() -> Self {
        Self {
            max_transmission_unit: 1500,
            supports_vlan: false,
            supports_checksum_offload: false,
            supports_tso: false,
            supports_lro: false,
        }
    }
}

/// Network device statistics
#[derive(Debug, Clone, Copy)]
pub struct DeviceStatistics {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
}

impl Default for DeviceStatistics {
    fn default() -> Self {
        Self {
            rx_packets: 0,
            tx_packets: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_errors: 0,
            tx_errors: 0,
            rx_dropped: 0,
            tx_dropped: 0,
        }
    }
}

/// Network device state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    Down,
    Up,
    Dormant,
    Testing,
}

/// Network device trait
pub trait NetworkDevice: Send {
    /// Get device name
    fn name(&self) -> &str;

    /// Get device MAC address
    fn mac_address(&self) -> MacAddress;

    /// Get device capabilities
    fn capabilities(&self) -> DeviceCapabilities;

    /// Get device state
    fn state(&self) -> DeviceState;

    /// Set device state
    fn set_state(&mut self, state: DeviceState) -> Result<(), KernelError>;

    /// Get device statistics
    fn statistics(&self) -> DeviceStatistics;

    /// Transmit a packet
    fn transmit(&mut self, packet: &Packet) -> Result<(), KernelError>;

    /// Receive a packet (non-blocking)
    fn receive(&mut self) -> Result<Option<Packet>, KernelError>;

    /// Get MTU
    fn mtu(&self) -> usize {
        self.capabilities().max_transmission_unit
    }
}

/// Loopback device implementation
pub struct LoopbackDevice {
    name: String,
    mac: MacAddress,
    state: DeviceState,
    stats: DeviceStatistics,
    queue: Vec<Packet>,
}

impl LoopbackDevice {
    pub fn new() -> Self {
        Self {
            name: String::from("lo0"),
            mac: MacAddress([0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            state: DeviceState::Down,
            stats: DeviceStatistics::default(),
            queue: Vec::new(),
        }
    }
}

impl Default for LoopbackDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkDevice for LoopbackDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    fn capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities {
            max_transmission_unit: 65536,
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
        self.state = state;
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

        // Loopback: immediately queue for receive
        self.queue.push(packet.clone());
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += packet.len() as u64;

        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }

        if let Some(packet) = self.queue.pop() {
            self.stats.rx_packets += 1;
            self.stats.rx_bytes += packet.len() as u64;
            Ok(Some(packet))
        } else {
            Ok(None)
        }
    }
}

/// Ethernet device (placeholder for real hardware)
pub struct EthernetDevice {
    name: String,
    mac: MacAddress,
    state: DeviceState,
    stats: DeviceStatistics,
    capabilities: DeviceCapabilities,
}

impl EthernetDevice {
    pub fn new(name: String, mac: MacAddress) -> Self {
        Self {
            name,
            mac,
            state: DeviceState::Down,
            stats: DeviceStatistics::default(),
            capabilities: DeviceCapabilities::default(),
        }
    }
}

impl NetworkDevice for EthernetDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }

    fn state(&self) -> DeviceState {
        self.state
    }

    fn set_state(&mut self, state: DeviceState) -> Result<(), KernelError> {
        self.state = state;
        // Hardware state is configured via DMA ring allocation in with_mmio() devices
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

        // DMA TX is handled by with_mmio() EthernetDevice instances
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += packet.len() as u64;

        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }

        // DMA RX is handled by with_mmio() EthernetDevice instances
        Ok(None)
    }
}

/// Device registry protected by Mutex for safe concurrent access
static DEVICES: Mutex<Option<Vec<Box<dyn NetworkDevice>>>> = Mutex::new(None);

/// Initialize device subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[NETDEV] Initializing network device subsystem...");

    let mut devices_lock = DEVICES.lock();
    let mut device_list = Vec::new();

    // Create and register loopback device
    let mut lo = LoopbackDevice::new();
    lo.set_state(DeviceState::Up)?;
    device_list.push(Box::new(lo) as Box<dyn NetworkDevice>);

    *devices_lock = Some(device_list);

    println!("[NETDEV] Network device subsystem initialized");
    Ok(())
}

/// Register a network device
pub fn register_device(device: Box<dyn NetworkDevice>) -> Result<(), KernelError> {
    let mut devices_lock = DEVICES.lock();
    if let Some(ref mut devices) = *devices_lock {
        println!("[NETDEV] Registering device: {}", device.name());
        devices.push(device);
        Ok(())
    } else {
        Err(KernelError::InvalidState {
            expected: "initialized",
            actual: "not_initialized",
        })
    }
}

/// Execute a closure with a device by name (immutable access)
pub fn with_device<R, F: FnOnce(&dyn NetworkDevice) -> R>(name: &str, f: F) -> Option<R> {
    let devices_lock = DEVICES.lock();
    if let Some(ref devices) = *devices_lock {
        devices
            .iter()
            .find(|d| d.name() == name)
            .map(|d| f(d.as_ref()))
    } else {
        None
    }
}

/// Execute a closure with a device by name (mutable access)
pub fn with_device_mut<R, F: FnOnce(&mut dyn NetworkDevice) -> R>(name: &str, f: F) -> Option<R> {
    let mut devices_lock = DEVICES.lock();
    if let Some(ref mut devices) = *devices_lock {
        devices
            .iter_mut()
            .find(|d| d.name() == name)
            .map(|d| f(d.as_mut()))
    } else {
        None
    }
}

/// List all device names
pub fn list_devices() -> Vec<String> {
    let devices_lock = DEVICES.lock();
    if let Some(ref devices) = *devices_lock {
        devices.iter().map(|d| String::from(d.name())).collect()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loopback_device() {
        let mut lo = LoopbackDevice::new();
        assert_eq!(lo.state(), DeviceState::Down);

        lo.set_state(DeviceState::Up).unwrap();
        assert_eq!(lo.state(), DeviceState::Up);
    }

    #[test]
    fn test_device_capabilities() {
        let caps = DeviceCapabilities::default();
        assert_eq!(caps.max_transmission_unit, 1500);
        assert!(!caps.supports_vlan);
    }
}

//! Network Device Drivers
//!
//! Implements network device drivers including Ethernet, Wi-Fi, and loopback.

use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};

use spin::{Mutex, RwLock};

use crate::services::driver_framework::{DeviceClass, DeviceInfo, DeviceStatus, Driver};

/// Network packet buffer
#[derive(Debug, Clone)]
pub struct NetworkPacket {
    pub data: Vec<u8>,
    pub length: usize,
    pub timestamp: u64,
}

impl NetworkPacket {
    pub fn new(data: Vec<u8>) -> Self {
        let length = data.len();
        Self {
            data,
            length,
            timestamp: 0, // TODO: Get actual timestamp
        }
    }
}

/// Network interface statistics
#[derive(Debug, Clone, Default)]
pub struct NetworkStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
    pub collisions: u64,
}

/// Network interface state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceState {
    Down,
    Up,
    Testing,
    Unknown,
}

/// Network interface configuration
#[derive(Debug, Clone)]
pub struct InterfaceConfig {
    pub name: String,
    pub mac_address: [u8; 6],
    pub mtu: u16,
    pub state: InterfaceState,
    pub ip_address: Option<[u8; 4]>,
    pub netmask: Option<[u8; 4]>,
    pub gateway: Option<[u8; 4]>,
}

impl InterfaceConfig {
    pub fn new(name: String, mac_address: [u8; 6]) -> Self {
        Self {
            name,
            mac_address,
            mtu: 1500, // Standard Ethernet MTU
            state: InterfaceState::Down,
            ip_address: None,
            netmask: None,
            gateway: None,
        }
    }
}

/// Network device trait
pub trait NetworkDevice: Send + Sync {
    /// Get device name
    fn name(&self) -> &str;

    /// Get interface configuration
    fn get_config(&self) -> InterfaceConfig;

    /// Set interface configuration
    fn set_config(&mut self, config: InterfaceConfig) -> Result<(), &'static str>;

    /// Bring interface up
    fn up(&mut self) -> Result<(), &'static str>;

    /// Bring interface down
    fn down(&mut self) -> Result<(), &'static str>;

    /// Send a packet
    fn send_packet(&mut self, packet: NetworkPacket) -> Result<(), &'static str>;

    /// Receive a packet (non-blocking)
    fn receive_packet(&mut self) -> Result<Option<NetworkPacket>, &'static str>;

    /// Get interface statistics
    fn get_stats(&self) -> NetworkStats;

    /// Reset interface statistics
    fn reset_stats(&mut self);

    /// Check if link is up
    fn link_up(&self) -> bool;

    /// Get link speed in Mbps
    fn link_speed(&self) -> u32;
}

/// Ethernet driver implementation
pub struct EthernetDriver {
    name: String,
    config: Mutex<InterfaceConfig>,
    stats: Mutex<NetworkStats>,
    rx_queue: Mutex<Vec<NetworkPacket>>,
    tx_queue: Mutex<Vec<NetworkPacket>>,
    device_info: DeviceInfo,
    enabled: Mutex<bool>,
}

impl EthernetDriver {
    /// Create a new Ethernet driver
    pub fn new(name: String, mac_address: [u8; 6], device_info: DeviceInfo) -> Self {
        Self {
            config: Mutex::new(InterfaceConfig::new(name.clone(), mac_address)),
            stats: Mutex::new(NetworkStats::default()),
            rx_queue: Mutex::new(Vec::new()),
            tx_queue: Mutex::new(Vec::new()),
            device_info,
            enabled: Mutex::new(false),
            name,
        }
    }

    /// Process transmitted packets
    fn process_tx_queue(&mut self) -> Result<(), &'static str> {
        let mut tx_queue = self.tx_queue.lock();
        let mut stats = self.stats.lock();

        // Simulate packet transmission
        for packet in tx_queue.drain(..) {
            // TODO: Actual hardware transmission
            stats.tx_packets += 1;
            stats.tx_bytes += packet.length as u64;

            // Simulate successful transmission
            crate::println!("[ETH] Transmitted packet ({} bytes)", packet.length);
        }

        Ok(())
    }

    /// Simulate packet reception
    pub fn simulate_receive(&self, data: Vec<u8>) {
        let packet = NetworkPacket::new(data);
        let mut rx_queue = self.rx_queue.lock();
        let mut stats = self.stats.lock();

        stats.rx_packets += 1;
        stats.rx_bytes += packet.length as u64;

        rx_queue.push(packet);
        crate::println!("[ETH] Received packet ({} bytes)", stats.rx_bytes);
    }
}

impl NetworkDevice for EthernetDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn get_config(&self) -> InterfaceConfig {
        self.config.lock().clone()
    }

    fn set_config(&mut self, config: InterfaceConfig) -> Result<(), &'static str> {
        *self.config.lock() = config;
        crate::println!("[ETH] Updated interface configuration for {}", self.name);
        Ok(())
    }

    fn up(&mut self) -> Result<(), &'static str> {
        self.config.lock().state = InterfaceState::Up;
        *self.enabled.lock() = true;
        crate::println!("[ETH] Interface {} is up", self.name);
        Ok(())
    }

    fn down(&mut self) -> Result<(), &'static str> {
        self.config.lock().state = InterfaceState::Down;
        *self.enabled.lock() = false;
        crate::println!("[ETH] Interface {} is down", self.name);
        Ok(())
    }

    fn send_packet(&mut self, packet: NetworkPacket) -> Result<(), &'static str> {
        if !*self.enabled.lock() {
            return Err("Interface is down");
        }

        self.tx_queue.lock().push(packet);
        self.process_tx_queue()?;
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Option<NetworkPacket>, &'static str> {
        if !*self.enabled.lock() {
            return Ok(None);
        }

        Ok(self.rx_queue.lock().pop())
    }

    fn get_stats(&self) -> NetworkStats {
        self.stats.lock().clone()
    }

    fn reset_stats(&mut self) {
        *self.stats.lock() = NetworkStats::default();
        crate::println!("[ETH] Reset statistics for {}", self.name);
    }

    fn link_up(&self) -> bool {
        *self.enabled.lock() && self.config.lock().state == InterfaceState::Up
    }

    fn link_speed(&self) -> u32 {
        if self.link_up() {
            1000 // 1 Gbps
        } else {
            0
        }
    }
}

impl Driver for EthernetDriver {
    fn name(&self) -> &str {
        "ethernet"
    }

    fn supported_classes(&self) -> Vec<DeviceClass> {
        vec![DeviceClass::Network]
    }

    fn supports_device(&self, device: &DeviceInfo) -> bool {
        device.class == DeviceClass::Network
    }

    fn probe(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[ETH] Probing device: {}", device.name);
        // TODO: Check if this is actually an Ethernet device
        Ok(())
    }

    fn attach(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[ETH] Attaching to device: {}", device.name);

        // Initialize hardware
        // TODO: Initialize actual Ethernet hardware

        self.up()?;

        crate::println!("[ETH] Successfully attached to {}", device.name);
        Ok(())
    }

    fn detach(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[ETH] Detaching from device: {}", device.name);

        self.down()?;

        crate::println!("[ETH] Successfully detached from {}", device.name);
        Ok(())
    }

    fn suspend(&mut self) -> Result<(), &'static str> {
        self.down()
    }

    fn resume(&mut self) -> Result<(), &'static str> {
        self.up()
    }

    fn handle_interrupt(&mut self, irq: u8) -> Result<(), &'static str> {
        crate::println!("[ETH] Handling interrupt {} for {}", irq, self.name);

        // TODO: Handle actual hardware interrupts
        // - Check interrupt status
        // - Process received packets
        // - Handle transmission completion
        // - Handle errors

        Ok(())
    }

    fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize, &'static str> {
        // For network devices, reading could return received packets
        if let Some(packet) = self.receive_packet()? {
            let copy_len = buffer.len().min(packet.data.len());
            buffer[..copy_len].copy_from_slice(&packet.data[..copy_len]);
            Ok(copy_len)
        } else {
            Ok(0)
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) -> Result<usize, &'static str> {
        // For network devices, writing sends packets
        let packet = NetworkPacket::new(data.to_vec());
        self.send_packet(packet)?;
        Ok(data.len())
    }

    fn ioctl(&mut self, cmd: u32, arg: u64) -> Result<u64, &'static str> {
        match cmd {
            0x1000 => {
                // Get interface status
                Ok(if self.link_up() { 1 } else { 0 })
            }
            0x1001 => {
                // Get link speed
                Ok(self.link_speed() as u64)
            }
            0x1002 => {
                // Reset statistics
                self.reset_stats();
                Ok(0)
            }
            _ => Err("Unknown ioctl command"),
        }
    }
}

/// Loopback network device
pub struct LoopbackDriver {
    name: String,
    config: Mutex<InterfaceConfig>,
    stats: Mutex<NetworkStats>,
    enabled: Mutex<bool>,
}

impl LoopbackDriver {
    pub fn new() -> Self {
        let loopback_mac = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut config = InterfaceConfig::new(String::from("lo"), loopback_mac);
        config.ip_address = Some([127, 0, 0, 1]);
        config.netmask = Some([255, 0, 0, 0]);
        config.mtu = 65535; // Loopback can have larger MTU (max for u16)

        Self {
            name: String::from("loopback"),
            config: Mutex::new(config),
            stats: Mutex::new(NetworkStats::default()),
            enabled: Mutex::new(false),
        }
    }
}

impl NetworkDevice for LoopbackDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn get_config(&self) -> InterfaceConfig {
        self.config.lock().clone()
    }

    fn set_config(&mut self, config: InterfaceConfig) -> Result<(), &'static str> {
        *self.config.lock() = config;
        Ok(())
    }

    fn up(&mut self) -> Result<(), &'static str> {
        self.config.lock().state = InterfaceState::Up;
        *self.enabled.lock() = true;
        crate::println!("[LOOP] Loopback interface is up");
        Ok(())
    }

    fn down(&mut self) -> Result<(), &'static str> {
        self.config.lock().state = InterfaceState::Down;
        *self.enabled.lock() = false;
        crate::println!("[LOOP] Loopback interface is down");
        Ok(())
    }

    fn send_packet(&mut self, packet: NetworkPacket) -> Result<(), &'static str> {
        if !*self.enabled.lock() {
            return Err("Interface is down");
        }

        // Loopback immediately "receives" what it sends
        let mut stats = self.stats.lock();
        stats.tx_packets += 1;
        stats.tx_bytes += packet.length as u64;
        stats.rx_packets += 1;
        stats.rx_bytes += packet.length as u64;

        crate::println!("[LOOP] Looped packet ({} bytes)", packet.length);
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Option<NetworkPacket>, &'static str> {
        // Loopback doesn't queue packets
        Ok(None)
    }

    fn get_stats(&self) -> NetworkStats {
        self.stats.lock().clone()
    }

    fn reset_stats(&mut self) {
        *self.stats.lock() = NetworkStats::default();
    }

    fn link_up(&self) -> bool {
        *self.enabled.lock()
    }

    fn link_speed(&self) -> u32 {
        u32::MAX // Infinite speed for loopback
    }
}

impl Driver for LoopbackDriver {
    fn name(&self) -> &str {
        "loopback"
    }

    fn supported_classes(&self) -> Vec<DeviceClass> {
        vec![] // Loopback is not a hardware device
    }

    fn supports_device(&self, _device: &DeviceInfo) -> bool {
        false // Loopback doesn't attach to hardware
    }

    fn probe(&mut self, _device: &DeviceInfo) -> Result<(), &'static str> {
        Err("Loopback doesn't probe hardware")
    }

    fn attach(&mut self, _device: &DeviceInfo) -> Result<(), &'static str> {
        Err("Loopback doesn't attach to hardware")
    }

    fn detach(&mut self, _device: &DeviceInfo) -> Result<(), &'static str> {
        Err("Loopback doesn't detach from hardware")
    }

    fn suspend(&mut self) -> Result<(), &'static str> {
        self.down()
    }

    fn resume(&mut self) -> Result<(), &'static str> {
        self.up()
    }

    fn handle_interrupt(&mut self, _irq: u8) -> Result<(), &'static str> {
        Err("Loopback doesn't handle interrupts")
    }

    fn read(&mut self, _offset: u64, _buffer: &mut [u8]) -> Result<usize, &'static str> {
        Ok(0) // No data to read from loopback
    }

    fn write(&mut self, _offset: u64, data: &[u8]) -> Result<usize, &'static str> {
        let packet = NetworkPacket::new(data.to_vec());
        self.send_packet(packet)?;
        Ok(data.len())
    }

    fn ioctl(&mut self, cmd: u32, _arg: u64) -> Result<u64, &'static str> {
        match cmd {
            0x1000 => Ok(if self.link_up() { 1 } else { 0 }),
            0x1001 => Ok(self.link_speed() as u64),
            0x1002 => {
                self.reset_stats();
                Ok(0)
            }
            _ => Err("Unknown ioctl command"),
        }
    }
}

/// Network interface manager
pub struct NetworkManager {
    interfaces: RwLock<BTreeMap<String, Arc<Mutex<dyn NetworkDevice>>>>,
    default_route: RwLock<Option<String>>,
}

impl NetworkManager {
    pub fn new() -> Self {
        Self {
            interfaces: RwLock::new(BTreeMap::new()),
            default_route: RwLock::new(None),
        }
    }

    /// Register a network interface
    pub fn register_interface(
        &self,
        name: String,
        device: Arc<Mutex<dyn NetworkDevice>>,
    ) -> Result<(), &'static str> {
        if self.interfaces.read().contains_key(&name) {
            return Err("Interface already exists");
        }

        self.interfaces.write().insert(name.clone(), device);
        crate::println!("[NET] Registered interface: {}", name);
        Ok(())
    }

    /// Unregister a network interface
    pub fn unregister_interface(&self, name: &str) -> Result<(), &'static str> {
        if self.interfaces.write().remove(name).is_some() {
            crate::println!("[NET] Unregistered interface: {}", name);
            Ok(())
        } else {
            Err("Interface not found")
        }
    }

    /// Get interface by name
    pub fn get_interface(&self, name: &str) -> Option<Arc<Mutex<dyn NetworkDevice>>> {
        self.interfaces.read().get(name).cloned()
    }

    /// List all interfaces
    pub fn list_interfaces(&self) -> Vec<String> {
        self.interfaces.read().keys().cloned().collect()
    }

    /// Set default route
    pub fn set_default_route(&self, interface: String) {
        *self.default_route.write() = Some(interface);
        crate::println!("[NET] Set default route to interface");
    }

    /// Get network statistics for all interfaces
    pub fn get_global_stats(&self) -> NetworkStats {
        let mut total_stats = NetworkStats::default();

        for interface in self.interfaces.read().values() {
            let stats = interface.lock().get_stats();
            total_stats.rx_packets += stats.rx_packets;
            total_stats.tx_packets += stats.tx_packets;
            total_stats.rx_bytes += stats.rx_bytes;
            total_stats.tx_bytes += stats.tx_bytes;
            total_stats.rx_errors += stats.rx_errors;
            total_stats.tx_errors += stats.tx_errors;
            total_stats.rx_dropped += stats.rx_dropped;
            total_stats.tx_dropped += stats.tx_dropped;
            total_stats.collisions += stats.collisions;
        }

        total_stats
    }
}

/// Global network manager instance
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
static NETWORK_MANAGER: spin::Once<NetworkManager> = spin::Once::new();

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static mut NETWORK_MANAGER_STATIC: Option<NetworkManager> = None;

/// Initialize network subsystem
pub fn init() {
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        NETWORK_MANAGER.call_once(|| NetworkManager::new());
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        NETWORK_MANAGER_STATIC = Some(NetworkManager::new());
    }

    // Create and register loopback interface
    let loopback = Arc::new(Mutex::new(LoopbackDriver::new()));
    get_network_manager()
        .register_interface(String::from("lo"), loopback.clone())
        .unwrap();

    // Bring up loopback interface
    loopback.lock().up().unwrap();

    // Register Ethernet driver with driver framework
    let driver_framework = crate::services::driver_framework::get_driver_framework();

    // Create a dummy Ethernet driver for demonstration
    let dummy_device = DeviceInfo {
        id: 0,
        name: String::from("Dummy Ethernet"),
        class: DeviceClass::Network,
        device_id: None,
        driver: None,
        bus: String::from("pci"),
        address: 0,
        irq: Some(11),
        dma_channels: Vec::new(),
        io_ports: Vec::new(),
        memory_regions: Vec::new(),
        status: DeviceStatus::Uninitialized,
    };

    let ethernet_driver = EthernetDriver::new(
        String::from("eth0"),
        [0x52, 0x54, 0x00, 0x12, 0x34, 0x56], // Random MAC
        dummy_device,
    );

    if let Err(e) = driver_framework.register_driver(Box::new(ethernet_driver)) {
        crate::println!("[NET] Failed to register Ethernet driver: {}", e);
    }

    crate::println!("[NET] Network subsystem initialized");
}

/// Get the global network manager
pub fn get_network_manager() -> &'static NetworkManager {
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        NETWORK_MANAGER
            .get()
            .expect("Network manager not initialized")
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        NETWORK_MANAGER_STATIC
            .as_ref()
            .expect("Network manager not initialized")
    }
}

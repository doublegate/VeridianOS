//! PCI Bus Driver
//!
//! Implements PCI bus enumeration and device management.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::{vec, format};
use core::mem;
use crate::services::driver_framework::{
    Bus, DeviceInfo, DeviceClass, DeviceId, DeviceStatus
};

/// PCI configuration space registers
#[repr(u16)]
#[allow(dead_code)]
pub enum PciConfigRegister {
    VendorId = 0x00,
    DeviceId = 0x02,
    Command = 0x04,
    Status = 0x06,
    RevisionId = 0x08,
    ProgIf = 0x09,
    Subclass = 0x0A,
    ClassCode = 0x0B,
    CacheLineSize = 0x0C,
    LatencyTimer = 0x0D,
    HeaderType = 0x0E,
    Bist = 0x0F,
    Bar0 = 0x10,
    Bar1 = 0x14,
    Bar2 = 0x18,
    Bar3 = 0x1C,
    Bar4 = 0x20,
    Bar5 = 0x24,
    CardbusCisPointer = 0x28,
    SubsystemVendorId = 0x2C,
    SubsystemId = 0x2E,
    ExpansionRomBase = 0x30,
    CapabilitiesPointer = 0x34,
    InterruptLine = 0x3C,
    InterruptPin = 0x3D,
    MinGrant = 0x3E,
    MaxLatency = 0x3F,
}

/// PCI class codes
#[allow(dead_code)]
pub mod class_codes {
    pub const UNCLASSIFIED: u8 = 0x00;
    pub const MASS_STORAGE: u8 = 0x01;
    pub const NETWORK: u8 = 0x02;
    pub const DISPLAY: u8 = 0x03;
    pub const MULTIMEDIA: u8 = 0x04;
    pub const MEMORY: u8 = 0x05;
    pub const BRIDGE: u8 = 0x06;
    pub const COMMUNICATION: u8 = 0x07;
    pub const SYSTEM: u8 = 0x08;
    pub const INPUT: u8 = 0x09;
    pub const DOCKING: u8 = 0x0A;
    pub const PROCESSOR: u8 = 0x0B;
    pub const SERIAL_BUS: u8 = 0x0C;
    pub const WIRELESS: u8 = 0x0D;
    pub const INTELLIGENT: u8 = 0x0E;
    pub const SATELLITE: u8 = 0x0F;
    pub const ENCRYPTION: u8 = 0x10;
    pub const DATA_ACQUISITION: u8 = 0x11;
    pub const COPROCESSOR: u8 = 0xFF;
}

/// PCI command register flags
#[allow(dead_code)]
pub mod command_flags {
    pub const IO_SPACE: u16 = 1 << 0;
    pub const MEMORY_SPACE: u16 = 1 << 1;
    pub const BUS_MASTER: u16 = 1 << 2;
    pub const SPECIAL_CYCLES: u16 = 1 << 3;
    pub const MEMORY_WRITE_INVALIDATE: u16 = 1 << 4;
    pub const VGA_PALETTE_SNOOP: u16 = 1 << 5;
    pub const PARITY_ERROR: u16 = 1 << 6;
    pub const STEPPING: u16 = 1 << 7;
    pub const SERR: u16 = 1 << 8;
    pub const FAST_BACK_TO_BACK: u16 = 1 << 9;
    pub const INTERRUPT_DISABLE: u16 = 1 << 10;
}

/// PCI device location
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PciLocation {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciLocation {
    pub fn new(bus: u8, device: u8, function: u8) -> Self {
        Self { bus, device, function }
    }
    
    /// Convert to configuration address
    pub fn to_config_address(&self) -> u32 {
        0x80000000 |
        ((self.bus as u32) << 16) |
        ((self.device as u32) << 11) |
        ((self.function as u32) << 8)
    }
}

/// PCI Base Address Register
#[derive(Debug, Clone)]
pub enum PciBar {
    Memory {
        address: u64,
        size: u64,
        prefetchable: bool,
        is_64bit: bool,
    },
    Io {
        address: u32,
        size: u32,
    },
    None,
}

impl PciBar {
    /// Get memory address if this is a memory BAR
    pub fn get_memory_address(&self) -> Option<u64> {
        match self {
            PciBar::Memory { address, .. } => Some(*address),
            _ => None,
        }
    }

    /// Get I/O port address if this is an I/O BAR
    pub fn get_io_address(&self) -> Option<u32> {
        match self {
            PciBar::Io { address, .. } => Some(*address),
            _ => None,
        }
    }

    /// Check if this is a memory BAR
    pub fn is_memory(&self) -> bool {
        matches!(self, PciBar::Memory { .. })
    }

    /// Check if this is an I/O BAR
    pub fn is_io(&self) -> bool {
        matches!(self, PciBar::Io { .. })
    }
}

/// PCI device representation
#[derive(Debug, Clone)]
pub struct PciDevice {
    pub location: PciLocation,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
    pub bars: Vec<PciBar>,
    pub enabled: bool,
}

impl PciDevice {
    /// Create a new PCI device
    pub fn new(location: PciLocation) -> Self {
        Self {
            location,
            vendor_id: 0,
            device_id: 0,
            class_code: 0,
            subclass: 0,
            prog_if: 0,
            revision: 0,
            header_type: 0,
            interrupt_line: 0,
            interrupt_pin: 0,
            bars: Vec::new(),
            enabled: false,
        }
    }
    
    /// Get device class
    pub fn get_device_class(&self) -> DeviceClass {
        match self.class_code {
            class_codes::MASS_STORAGE => DeviceClass::Storage,
            class_codes::NETWORK => DeviceClass::Network,
            class_codes::DISPLAY => DeviceClass::Display,
            class_codes::MULTIMEDIA => DeviceClass::Audio,
            class_codes::SERIAL_BUS => {
                match self.subclass {
                    0x03 => DeviceClass::USB, // USB controller
                    _ => DeviceClass::Other,
                }
            }
            class_codes::BRIDGE => DeviceClass::PCI,
            _ => DeviceClass::Other,
        }
    }
    
    /// Check if device is multifunction
    pub fn is_multifunction(&self) -> bool {
        self.header_type & 0x80 != 0
    }
}

/// PCI bus implementation
pub struct PciBus {
    /// Discovered PCI devices
    devices: spin::RwLock<BTreeMap<PciLocation, PciDevice>>,
    
    /// Device enumeration complete
    enumerated: core::sync::atomic::AtomicBool,
}

impl PciBus {
    /// Create a new PCI bus
    pub fn new() -> Self {
        Self {
            devices: spin::RwLock::new(BTreeMap::new()),
            enumerated: core::sync::atomic::AtomicBool::new(false),
        }
    }
    
    /// Enumerate all PCI devices
    pub fn enumerate_devices(&self) -> Result<(), &'static str> {
        if self.enumerated.load(core::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        
        crate::println!("[PCI] Enumerating PCI devices...");
        let mut device_count = 0;
        
        // Scan all buses
        for bus in 0..=255 {
            for device in 0..32 {
                // Check function 0
                let location = PciLocation::new(bus, device, 0);
                if let Some(mut pci_device) = self.probe_device(location) {
                    self.read_device_config(&mut pci_device);
                    
                    crate::println!("[PCI] Found device at {}:{}:{} - {:04x}:{:04x} (class {:02x})", 
                        bus, device, 0, pci_device.vendor_id, pci_device.device_id, pci_device.class_code);
                    
                    let is_multifunction = pci_device.is_multifunction();
                    self.devices.write().insert(location, pci_device);
                    device_count += 1;
                    
                    // Check other functions if multifunction
                    if is_multifunction {
                        for function in 1..8 {
                            let func_location = PciLocation::new(bus, device, function);
                            if let Some(mut func_device) = self.probe_device(func_location) {
                                self.read_device_config(&mut func_device);
                                
                                crate::println!("[PCI] Found device at {}:{}:{} - {:04x}:{:04x} (class {:02x})", 
                                    bus, device, function, func_device.vendor_id, func_device.device_id, func_device.class_code);
                                
                                self.devices.write().insert(func_location, func_device);
                                device_count += 1;
                            }
                        }
                    }
                }
            }
        }
        
        self.enumerated.store(true, core::sync::atomic::Ordering::Release);
        crate::println!("[PCI] Enumeration complete: {} devices found", device_count);
        
        Ok(())
    }
    
    /// Probe for device at location
    fn probe_device(&self, location: PciLocation) -> Option<PciDevice> {
        let vendor_id = self.read_config_word(location, PciConfigRegister::VendorId);
        
        // Check if device exists
        if vendor_id == 0xFFFF {
            return None;
        }
        
        let mut device = PciDevice::new(location);
        device.vendor_id = vendor_id;
        device.device_id = self.read_config_word(location, PciConfigRegister::DeviceId);
        
        Some(device)
    }
    
    /// Read full device configuration
    fn read_device_config(&self, device: &mut PciDevice) {
        let location = device.location;
        
        device.class_code = self.read_config_byte(location, PciConfigRegister::ClassCode);
        device.subclass = self.read_config_byte(location, PciConfigRegister::Subclass);
        device.prog_if = self.read_config_byte(location, PciConfigRegister::ProgIf);
        device.revision = self.read_config_byte(location, PciConfigRegister::RevisionId);
        device.header_type = self.read_config_byte(location, PciConfigRegister::HeaderType);
        device.interrupt_line = self.read_config_byte(location, PciConfigRegister::InterruptLine);
        device.interrupt_pin = self.read_config_byte(location, PciConfigRegister::InterruptPin);
        
        // Read BARs
        device.bars = self.read_bars(location, device.header_type & 0x7F);
    }
    
    /// Read Base Address Registers
    fn read_bars(&self, location: PciLocation, header_type: u8) -> Vec<PciBar> {
        let mut bars = Vec::new();
        
        // Standard header has 6 BARs, bridge header has 2
        let bar_count = if header_type == 0 { 6 } else { 2 };
        
        let mut bar_index = 0;
        while bar_index < bar_count {
            let bar_offset = PciConfigRegister::Bar0 as u16 + (bar_index * 4) as u16;
            let bar_value = self.read_config_dword(location, bar_offset);
            
            if bar_value == 0 {
                bars.push(PciBar::None);
                bar_index += 1;
                continue;
            }
            
            if bar_value & 1 == 0 {
                // Memory BAR
                let is_64bit = (bar_value >> 1) & 3 == 2;
                let prefetchable = (bar_value >> 3) & 1 != 0;
                
                // Write all 1s to determine size
                self.write_config_dword(location, bar_offset, 0xFFFFFFFF);
                let size_mask = self.read_config_dword(location, bar_offset);
                self.write_config_dword(location, bar_offset, bar_value);
                
                let size = (!size_mask + 1) & 0xFFFFFFF0;
                let mut address = (bar_value & 0xFFFFFFF0) as u64;
                
                if is_64bit && bar_index + 1 < bar_count {
                    // Read upper 32 bits
                    let upper_bar_offset = bar_offset + 4;
                    let upper_value = self.read_config_dword(location, upper_bar_offset);
                    address |= (upper_value as u64) << 32;
                    
                    bars.push(PciBar::Memory {
                        address,
                        size: size as u64,
                        prefetchable,
                        is_64bit: true,
                    });
                    
                    bars.push(PciBar::None); // Upper 32 bits
                    bar_index += 2;
                } else {
                    bars.push(PciBar::Memory {
                        address,
                        size: size as u64,
                        prefetchable,
                        is_64bit: false,
                    });
                    bar_index += 1;
                }
            } else {
                // I/O BAR
                self.write_config_dword(location, bar_offset, 0xFFFFFFFF);
                let size_mask = self.read_config_dword(location, bar_offset);
                self.write_config_dword(location, bar_offset, bar_value);
                
                let size = (!size_mask + 1) & 0xFFFFFFFC;
                let address = bar_value & 0xFFFFFFFC;
                
                bars.push(PciBar::Io {
                    address,
                    size,
                });
                bar_index += 1;
            }
        }
        
        bars
    }
    
    /// Read configuration byte
    fn read_config_byte(&self, location: PciLocation, register: PciConfigRegister) -> u8 {
        let offset = register as u16;
        let dword = self.read_config_dword(location, offset & !3);
        ((dword >> ((offset & 3) * 8)) & 0xFF) as u8
    }
    
    /// Read configuration word
    fn read_config_word(&self, location: PciLocation, register: PciConfigRegister) -> u16 {
        let offset = register as u16;
        let dword = self.read_config_dword(location, offset & !3);
        ((dword >> ((offset & 3) * 8)) & 0xFFFF) as u16
    }
    
    /// Read configuration dword
    fn read_config_dword(&self, location: PciLocation, offset: u16) -> u32 {
        let address = location.to_config_address() | (offset as u32 & 0xFC);
        
        unsafe {
            // Write configuration address
            crate::arch::outl(0xCF8, address);
            // Read configuration data
            crate::arch::inl(0xCFC)
        }
    }
    
    /// Write configuration dword
    fn write_config_dword(&self, location: PciLocation, offset: u16, value: u32) {
        let address = location.to_config_address() | (offset as u32 & 0xFC);
        
        unsafe {
            // Write configuration address
            crate::arch::outl(0xCF8, address);
            // Write configuration data
            crate::arch::outl(0xCFC, value);
        }
    }
    
    /// Get device by location
    pub fn get_device(&self, location: PciLocation) -> Option<PciDevice> {
        self.devices.read().get(&location).cloned()
    }
    
    /// Get all devices
    pub fn get_all_devices(&self) -> Vec<PciDevice> {
        self.devices.read().values().cloned().collect()
    }
    
    /// Find devices by class
    pub fn find_devices_by_class(&self, class_code: u8) -> Vec<PciDevice> {
        self.devices.read()
            .values()
            .filter(|dev| dev.class_code == class_code)
            .cloned()
            .collect()
    }
    
    /// Find devices by vendor and device ID
    pub fn find_devices_by_id(&self, vendor_id: u16, device_id: u16) -> Vec<PciDevice> {
        self.devices.read()
            .values()
            .filter(|dev| dev.vendor_id == vendor_id && dev.device_id == device_id)
            .cloned()
            .collect()
    }
}

impl Bus for PciBus {
    fn name(&self) -> &str {
        "pci"
    }
    
    fn scan(&mut self) -> Vec<DeviceInfo> {
        // Enumerate devices if not done already
        self.enumerate_devices().unwrap_or_else(|e| {
            crate::println!("[PCI] Enumeration failed: {}", e);
        });
        
        let devices = self.devices.read();
        let mut device_infos = Vec::new();
        
        for (location, pci_device) in devices.iter() {
            let device_id = DeviceId {
                vendor_id: pci_device.vendor_id,
                device_id: pci_device.device_id,
                class_code: pci_device.class_code,
                subclass: pci_device.subclass,
                prog_if: pci_device.prog_if,
                revision: pci_device.revision,
            };
            
            let mut io_ports = Vec::new();
            let mut memory_regions = Vec::new();
            
            for bar in &pci_device.bars {
                match bar {
                    PciBar::Memory { address, size, .. } => {
                        memory_regions.push((*address, *size));
                    }
                    PciBar::Io { address, size } => {
                        io_ports.push((*address as u16, (*address + *size) as u16));
                    }
                    PciBar::None => {}
                }
            }
            
            let device_info = DeviceInfo {
                id: (location.bus as u64) << 16 | (location.device as u64) << 8 | (location.function as u64),
                name: format!("PCI Device {:04x}:{:04x}", pci_device.vendor_id, pci_device.device_id),
                class: pci_device.get_device_class(),
                device_id: Some(device_id),
                driver: None,
                bus: String::from("pci"),
                address: location.to_config_address() as u64,
                irq: if pci_device.interrupt_line != 0xFF { 
                    Some(pci_device.interrupt_line) 
                } else { 
                    None 
                },
                dma_channels: Vec::new(), // PCI devices don't use ISA DMA
                io_ports,
                memory_regions,
                status: DeviceStatus::Uninitialized,
            };
            
            device_infos.push(device_info);
        }
        
        device_infos
    }
    
    fn read_config(&self, device: &DeviceInfo, offset: u16, size: u8) -> Result<u32, &'static str> {
        // Extract location from device address
        let address = device.address as u32;
        let bus = ((address >> 16) & 0xFF) as u8;
        let dev = ((address >> 11) & 0x1F) as u8;
        let func = ((address >> 8) & 0x07) as u8;
        let location = PciLocation::new(bus, dev, func);
        
        match size {
            1 => Ok(self.read_config_byte(location, unsafe { mem::transmute(offset) }) as u32),
            2 => Ok(self.read_config_word(location, unsafe { mem::transmute(offset) }) as u32),
            4 => Ok(self.read_config_dword(location, offset)),
            _ => Err("Invalid size"),
        }
    }
    
    fn write_config(&mut self, device: &DeviceInfo, offset: u16, value: u32, size: u8) -> Result<(), &'static str> {
        // Extract location from device address
        let address = device.address as u32;
        let bus = ((address >> 16) & 0xFF) as u8;
        let dev = ((address >> 11) & 0x1F) as u8;
        let func = ((address >> 8) & 0x07) as u8;
        let location = PciLocation::new(bus, dev, func);
        
        match size {
            1 => {
                let current = self.read_config_dword(location, offset & !3);
                let shift = (offset & 3) * 8;
                let mask = !(0xFF << shift);
                let new_value = (current & mask) | ((value & 0xFF) << shift);
                self.write_config_dword(location, offset & !3, new_value);
            }
            2 => {
                let current = self.read_config_dword(location, offset & !3);
                let shift = (offset & 3) * 8;
                let mask = !(0xFFFF << shift);
                let new_value = (current & mask) | ((value & 0xFFFF) << shift);
                self.write_config_dword(location, offset & !3, new_value);
            }
            4 => self.write_config_dword(location, offset, value),
            _ => return Err("Invalid size"),
        }
        
        Ok(())
    }
    
    fn enable_device(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        // Enable I/O and memory space, bus mastering
        let current_command = self.read_config(device, PciConfigRegister::Command as u16, 2)?;
        let new_command = current_command | 
            command_flags::IO_SPACE as u32 |
            command_flags::MEMORY_SPACE as u32 |
            command_flags::BUS_MASTER as u32;
        
        self.write_config(device, PciConfigRegister::Command as u16, new_command, 2)?;
        
        crate::println!("[PCI] Enabled device {}", device.name);
        Ok(())
    }
    
    fn disable_device(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        // Disable I/O and memory space, bus mastering
        let current_command = self.read_config(device, PciConfigRegister::Command as u16, 2)?;
        let new_command = current_command & 
            !(command_flags::IO_SPACE as u32 |
              command_flags::MEMORY_SPACE as u32 |
              command_flags::BUS_MASTER as u32);
        
        self.write_config(device, PciConfigRegister::Command as u16, new_command, 2)?;
        
        crate::println!("[PCI] Disabled device {}", device.name);
        Ok(())
    }
}

/// Global PCI bus instance
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
static PCI_BUS: spin::Once<spin::Mutex<PciBus>> = spin::Once::new();

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static mut PCI_BUS_STATIC: Option<spin::Mutex<PciBus>> = None;

/// Initialize PCI bus
pub fn init() {
    let pci_bus = PciBus::new();
    
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        PCI_BUS.call_once(|| spin::Mutex::new(pci_bus));
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        PCI_BUS_STATIC = Some(spin::Mutex::new(pci_bus));
    }
    
    // Register with driver framework
    let driver_framework = crate::services::driver_framework::get_driver_framework();
    
    // Create a new PciBus instance for registration (since we can't clone the mutex guard)
    let bus_instance = PciBus::new();
    
    if let Err(e) = driver_framework.register_bus(alloc::boxed::Box::new(bus_instance)) {
        crate::println!("[PCI] Failed to register PCI bus: {}", e);
    } else {
        crate::println!("[PCI] PCI bus driver initialized");
    }
}

/// Get the global PCI bus
pub fn get_pci_bus() -> &'static spin::Mutex<PciBus> {
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        PCI_BUS.get().expect("PCI bus not initialized")
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        PCI_BUS_STATIC.as_ref().expect("PCI bus not initialized")
    }
}
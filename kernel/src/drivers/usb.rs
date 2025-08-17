//! USB Bus Driver
//!
//! Implements USB host controller and device management.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::boxed::Box;
use alloc::{vec, format};
use core::mem;
use spin::{Mutex, RwLock};
use crate::services::driver_framework::{
    Bus, DeviceInfo, DeviceClass, DeviceId, DeviceStatus
};

/// USB device speeds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,    // 1.5 Mbps
    Full,   // 12 Mbps
    High,   // 480 Mbps
    Super,  // 5 Gbps
    SuperPlus, // 10 Gbps
}

/// USB device classes
#[allow(dead_code)]
pub mod usb_classes {
    pub const AUDIO: u8 = 0x01;
    pub const CDC: u8 = 0x02;          // Communications and CDC Control
    pub const HID: u8 = 0x03;          // Human Interface Device
    pub const PHYSICAL: u8 = 0x05;     // Physical
    pub const IMAGE: u8 = 0x06;        // Image
    pub const PRINTER: u8 = 0x07;      // Printer
    pub const MASS_STORAGE: u8 = 0x08; // Mass Storage
    pub const HUB: u8 = 0x09;          // Hub
    pub const CDC_DATA: u8 = 0x0A;     // CDC-Data
    pub const SMART_CARD: u8 = 0x0B;   // Smart Card
    pub const CONTENT_SECURITY: u8 = 0x0D; // Content Security
    pub const VIDEO: u8 = 0x0E;        // Video
    pub const HEALTHCARE: u8 = 0x0F;   // Personal Healthcare
    pub const DIAGNOSTIC: u8 = 0xDC;   // Diagnostic Device
    pub const WIRELESS: u8 = 0xE0;     // Wireless Controller
    pub const MISC: u8 = 0xEF;         // Miscellaneous
    pub const APP_SPECIFIC: u8 = 0xFE; // Application Specific
    pub const VENDOR_SPECIFIC: u8 = 0xFF; // Vendor Specific
}

/// USB endpoint types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbEndpointType {
    Control = 0,
    Isochronous = 1,
    Bulk = 2,
    Interrupt = 3,
}

/// USB endpoint direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDirection {
    Out = 0,
    In = 1,
}

/// USB endpoint descriptor
#[derive(Debug, Clone)]
pub struct UsbEndpoint {
    pub address: u8,
    pub direction: UsbDirection,
    pub endpoint_type: UsbEndpointType,
    pub max_packet_size: u16,
    pub interval: u8,
}

impl UsbEndpoint {
    pub fn new(address: u8) -> Self {
        Self {
            address: address & 0x7F,
            direction: if address & 0x80 != 0 { UsbDirection::In } else { UsbDirection::Out },
            endpoint_type: UsbEndpointType::Control,
            max_packet_size: 8,
            interval: 0,
        }
    }
}

/// USB interface descriptor
#[derive(Debug, Clone)]
pub struct UsbInterface {
    pub number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<UsbEndpoint>,
}

/// USB configuration descriptor
#[derive(Debug, Clone)]
pub struct UsbConfiguration {
    pub value: u8,
    pub max_power: u16, // in mA
    pub self_powered: bool,
    pub remote_wakeup: bool,
    pub interfaces: Vec<UsbInterface>,
}

/// USB device descriptor
#[derive(Debug, Clone)]
pub struct UsbDeviceDescriptor {
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_release: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub max_packet_size: u8,
    pub manufacturer: String,
    pub product: String,
    pub serial_number: String,
    pub configurations: Vec<UsbConfiguration>,
}

/// USB device representation
#[derive(Debug, Clone)]
pub struct UsbDevice {
    pub address: u8,
    pub port: u8,
    pub speed: UsbSpeed,
    pub descriptor: UsbDeviceDescriptor,
    pub current_configuration: Option<u8>,
    pub connected: bool,
}

impl UsbDevice {
    /// Create a new USB device
    pub fn new(address: u8, port: u8, speed: UsbSpeed) -> Self {
        Self {
            address,
            port,
            speed,
            descriptor: UsbDeviceDescriptor {
                vendor_id: 0,
                product_id: 0,
                device_release: 0,
                class: 0,
                subclass: 0,
                protocol: 0,
                max_packet_size: 8,
                manufacturer: String::new(),
                product: String::new(),
                serial_number: String::new(),
                configurations: Vec::new(),
            },
            current_configuration: None,
            connected: false,
        }
    }
    
    /// Get device class
    pub fn get_device_class(&self) -> DeviceClass {
        match self.descriptor.class {
            usb_classes::AUDIO => DeviceClass::Audio,
            usb_classes::HID => DeviceClass::Input,
            usb_classes::MASS_STORAGE => DeviceClass::Storage,
            usb_classes::HUB => DeviceClass::USB,
            usb_classes::VIDEO => DeviceClass::Display,
            usb_classes::CDC | usb_classes::CDC_DATA => DeviceClass::Network,
            _ => DeviceClass::Other,
        }
    }
}

/// USB transfer types
#[derive(Debug, Clone)]
pub enum UsbTransfer {
    Setup {
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: Vec<u8>,
    },
    In {
        endpoint: u8,
        length: usize,
    },
    Out {
        endpoint: u8,
        data: Vec<u8>,
    },
}

/// USB host controller trait
pub trait UsbHostController: Send + Sync {
    /// Get controller name
    fn name(&self) -> &str;
    
    /// Initialize the controller
    fn init(&mut self) -> Result<(), &'static str>;
    
    /// Reset the controller
    fn reset(&mut self) -> Result<(), &'static str>;
    
    /// Get number of ports
    fn get_port_count(&self) -> u8;
    
    /// Check port status
    fn get_port_status(&self, port: u8) -> Result<UsbPortStatus, &'static str>;
    
    /// Reset port
    fn reset_port(&mut self, port: u8) -> Result<(), &'static str>;
    
    /// Enable port
    fn enable_port(&mut self, port: u8) -> Result<(), &'static str>;
    
    /// Disable port
    fn disable_port(&mut self, port: u8) -> Result<(), &'static str>;
    
    /// Perform USB transfer
    fn transfer(&mut self, device_address: u8, transfer: UsbTransfer) -> Result<Vec<u8>, &'static str>;
    
    /// Set device address
    fn set_device_address(&mut self, old_address: u8, new_address: u8) -> Result<(), &'static str>;
}

/// USB port status
#[derive(Debug, Clone, Copy)]
pub struct UsbPortStatus {
    pub connected: bool,
    pub enabled: bool,
    pub suspended: bool,
    pub reset: bool,
    pub speed: UsbSpeed,
    pub power: bool,
}

/// USB bus implementation
pub struct UsbBus {
    /// Host controllers
    controllers: RwLock<Vec<Box<dyn UsbHostController>>>,
    
    /// Connected devices
    devices: RwLock<BTreeMap<u8, UsbDevice>>, // address -> device
    
    /// Next device address
    next_address: core::sync::atomic::AtomicU8,
    
    /// Port to device mapping
    port_devices: RwLock<BTreeMap<(usize, u8), u8>>, // (controller_index, port) -> address
}

impl UsbBus {
    /// Create a new USB bus
    pub fn new() -> Self {
        Self {
            controllers: RwLock::new(Vec::new()),
            devices: RwLock::new(BTreeMap::new()),
            next_address: core::sync::atomic::AtomicU8::new(1),
            port_devices: RwLock::new(BTreeMap::new()),
        }
    }
    
    /// Add a host controller
    pub fn add_controller(&self, mut controller: Box<dyn UsbHostController>) -> Result<(), &'static str> {
        // Initialize the controller
        controller.init()?;
        
        let controller_name: String = controller.name().into();
        let controller_index = self.controllers.read().len();
        
        // Scan ports for devices
        let port_count = controller.get_port_count();
        crate::println!("[USB] Controller {} has {} ports", controller_name, port_count);
        
        self.controllers.write().push(controller);
        
        // Scan for connected devices
        self.scan_controller_ports(controller_index)?;
        
        crate::println!("[USB] Added USB host controller: {}", controller_name);
        Ok(())
    }
    
    /// Scan controller ports for devices
    fn scan_controller_ports(&self, controller_index: usize) -> Result<(), &'static str> {
        let port_count = {
            let controllers = self.controllers.read();
            controllers.get(controller_index)
                .ok_or("Controller not found")?
                .get_port_count()
        };
        
        for port in 1..=port_count {
            if let Err(e) = self.scan_port(controller_index, port) {
                crate::println!("[USB] Failed to scan port {}: {}", port, e);
            }
        }
        
        Ok(())
    }
    
    /// Scan a specific port
    fn scan_port(&self, controller_index: usize, port: u8) -> Result<(), &'static str> {
        let status = {
            let controllers = self.controllers.read();
            controllers.get(controller_index)
                .ok_or("Controller not found")?
                .get_port_status(port)?
        };
        
        if !status.connected {
            return Ok(()); // No device connected
        }
        
        crate::println!("[USB] Device detected on controller {} port {}", controller_index, port);
        
        // Reset and enable port
        {
            let mut controllers = self.controllers.write();
            let controller = controllers.get_mut(controller_index)
                .ok_or("Controller not found")?;
            
            controller.reset_port(port)?;
            controller.enable_port(port)?;
        }
        
        // Enumerate device
        let device_address = self.enumerate_device(controller_index, port, status.speed)?;
        
        // Store port mapping
        self.port_devices.write().insert((controller_index, port), device_address);
        
        Ok(())
    }
    
    /// Enumerate a device
    fn enumerate_device(&self, controller_index: usize, port: u8, speed: UsbSpeed) -> Result<u8, &'static str> {
        // Allocate device address
        let device_address = self.next_address.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        
        // Create device
        let mut device = UsbDevice::new(device_address, port, speed);
        
        // Get device descriptor
        self.read_device_descriptor(controller_index, 0, &mut device)?;
        
        // Set device address
        {
            let mut controllers = self.controllers.write();
            let controller = controllers.get_mut(controller_index)
                .ok_or("Controller not found")?;
            
            controller.set_device_address(0, device_address)?;
        }
        
        // Read full device descriptor with new address
        self.read_device_descriptor(controller_index, device_address, &mut device)?;
        
        // Read configurations
        self.read_configurations(controller_index, device_address, &mut device)?;
        
        device.connected = true;
        
        crate::println!("[USB] Enumerated device {:04x}:{:04x} at address {}", 
            device.descriptor.vendor_id, device.descriptor.product_id, device_address);
        
        // Store device
        self.devices.write().insert(device_address, device);
        
        Ok(device_address)
    }
    
    /// Read device descriptor
    fn read_device_descriptor(&self, controller_index: usize, address: u8, device: &mut UsbDevice) -> Result<(), &'static str> {
        let transfer = UsbTransfer::Setup {
            request_type: 0x80, // Device to host, standard, device
            request: 0x06,      // GET_DESCRIPTOR
            value: 0x0100,      // Device descriptor
            index: 0,
            data: vec![0; 18],  // Device descriptor is 18 bytes
        };
        
        let response = {
            let mut controllers = self.controllers.write();
            let controller = controllers.get_mut(controller_index)
                .ok_or("Controller not found")?;
            
            controller.transfer(address, transfer)?
        };
        
        if response.len() < 18 {
            return Err("Device descriptor too short");
        }
        
        // Parse device descriptor
        device.descriptor.vendor_id = u16::from_le_bytes([response[8], response[9]]);
        device.descriptor.product_id = u16::from_le_bytes([response[10], response[11]]);
        device.descriptor.device_release = u16::from_le_bytes([response[12], response[13]]);
        device.descriptor.class = response[4];
        device.descriptor.subclass = response[5];
        device.descriptor.protocol = response[6];
        device.descriptor.max_packet_size = response[7];
        
        // Read string descriptors if available
        if response[14] != 0 { // Manufacturer string index
            if let Ok(manufacturer) = self.read_string_descriptor(controller_index, address, response[14]) {
                device.descriptor.manufacturer = manufacturer;
            }
        }
        
        if response[15] != 0 { // Product string index
            if let Ok(product) = self.read_string_descriptor(controller_index, address, response[15]) {
                device.descriptor.product = product;
            }
        }
        
        if response[16] != 0 { // Serial number string index
            if let Ok(serial) = self.read_string_descriptor(controller_index, address, response[16]) {
                device.descriptor.serial_number = serial;
            }
        }
        
        Ok(())
    }
    
    /// Read string descriptor
    fn read_string_descriptor(&self, controller_index: usize, address: u8, index: u8) -> Result<String, &'static str> {
        let transfer = UsbTransfer::Setup {
            request_type: 0x80,
            request: 0x06,
            value: 0x0300 | (index as u16),
            index: 0x0409, // English (US)
            data: vec![0; 255],
        };
        
        let response = {
            let mut controllers = self.controllers.write();
            let controller = controllers.get_mut(controller_index)
                .ok_or("Controller not found")?;
            
            controller.transfer(address, transfer)?
        };
        
        if response.len() < 2 {
            return Err("String descriptor too short");
        }
        
        let length = response[0] as usize;
        if length < 2 || response[1] != 0x03 {
            return Err("Invalid string descriptor");
        }
        
        // Convert UTF-16LE to UTF-8
        let mut result = String::new();
        for i in (2..length.min(response.len())).step_by(2) {
            if i + 1 < response.len() {
                let code_unit = u16::from_le_bytes([response[i], response[i + 1]]);
                if let Some(ch) = char::from_u32(code_unit as u32) {
                    result.push(ch);
                }
            }
        }
        
        Ok(result)
    }
    
    /// Read configurations
    fn read_configurations(&self, controller_index: usize, address: u8, device: &mut UsbDevice) -> Result<(), &'static str> {
        // For now, just create a default configuration
        let config = UsbConfiguration {
            value: 1,
            max_power: 100, // 100 mA
            self_powered: false,
            remote_wakeup: false,
            interfaces: Vec::new(),
        };
        
        device.descriptor.configurations.push(config);
        Ok(())
    }
    
    /// Get device by address
    pub fn get_device(&self, address: u8) -> Option<UsbDevice> {
        self.devices.read().get(&address).cloned()
    }
    
    /// Get all devices
    pub fn get_all_devices(&self) -> Vec<UsbDevice> {
        self.devices.read().values().cloned().collect()
    }
    
    /// Find devices by class
    pub fn find_devices_by_class(&self, class: u8) -> Vec<UsbDevice> {
        self.devices.read()
            .values()
            .filter(|dev| dev.descriptor.class == class)
            .cloned()
            .collect()
    }
}

impl Bus for UsbBus {
    fn name(&self) -> &str {
        "usb"
    }
    
    fn scan(&mut self) -> Vec<DeviceInfo> {
        let devices = self.devices.read();
        let mut device_infos = Vec::new();
        
        for (address, usb_device) in devices.iter() {
            let device_id = DeviceId {
                vendor_id: usb_device.descriptor.vendor_id,
                device_id: usb_device.descriptor.product_id,
                class_code: usb_device.descriptor.class,
                subclass: usb_device.descriptor.subclass,
                prog_if: usb_device.descriptor.protocol,
                revision: (usb_device.descriptor.device_release & 0xFF) as u8,
            };
            
            let device_info = DeviceInfo {
                id: *address as u64,
                name: if !usb_device.descriptor.product.is_empty() {
                    usb_device.descriptor.product.clone()
                } else {
                    format!("USB Device {:04x}:{:04x}", 
                        usb_device.descriptor.vendor_id, 
                        usb_device.descriptor.product_id)
                },
                class: usb_device.get_device_class(),
                device_id: Some(device_id),
                driver: None,
                bus: String::from("usb"),
                address: *address as u64,
                irq: None, // USB devices use the host controller's IRQ
                dma_channels: Vec::new(),
                io_ports: Vec::new(),
                memory_regions: Vec::new(),
                status: if usb_device.connected {
                    DeviceStatus::Uninitialized
                } else {
                    DeviceStatus::Removed
                },
            };
            
            device_infos.push(device_info);
        }
        
        device_infos
    }
    
    fn read_config(&self, device: &DeviceInfo, offset: u16, size: u8) -> Result<u32, &'static str> {
        // USB devices don't have traditional config space
        // This could be used for reading descriptors
        Err("USB devices don't support config space reads")
    }
    
    fn write_config(&mut self, device: &DeviceInfo, offset: u16, value: u32, size: u8) -> Result<(), &'static str> {
        // USB devices don't have traditional config space
        Err("USB devices don't support config space writes")
    }
    
    fn enable_device(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        let address = device.address as u8;
        
        if let Some(mut usb_device) = self.devices.write().get_mut(&address) {
            // Set configuration 1 if available
            if !usb_device.descriptor.configurations.is_empty() {
                usb_device.current_configuration = Some(1);
                crate::println!("[USB] Enabled device at address {}", address);
            }
        }
        
        Ok(())
    }
    
    fn disable_device(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        let address = device.address as u8;
        
        if let Some(mut usb_device) = self.devices.write().get_mut(&address) {
            usb_device.current_configuration = None;
            crate::println!("[USB] Disabled device at address {}", address);
        }
        
        Ok(())
    }
}

/// Simple UHCI host controller implementation
pub struct UhciController {
    base_address: u32,
    port_count: u8,
    name: String,
}

impl UhciController {
    pub fn new(base_address: u32) -> Self {
        Self {
            base_address,
            port_count: 2, // UHCI typically has 2 ports
            name: String::from("UHCI"),
        }
    }
}

impl UsbHostController for UhciController {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn init(&mut self) -> Result<(), &'static str> {
        crate::println!("[USB] Initializing UHCI controller at 0x{:x}", self.base_address);
        
        // TODO: Implement actual UHCI initialization
        // - Reset controller
        // - Set up frame list
        // - Enable controller
        // - Enable ports
        
        Ok(())
    }
    
    fn reset(&mut self) -> Result<(), &'static str> {
        // TODO: Implement UHCI reset
        Ok(())
    }
    
    fn get_port_count(&self) -> u8 {
        self.port_count
    }
    
    fn get_port_status(&self, port: u8) -> Result<UsbPortStatus, &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }
        
        // TODO: Read actual port status from UHCI registers
        Ok(UsbPortStatus {
            connected: false, // No devices for now
            enabled: false,
            suspended: false,
            reset: false,
            speed: UsbSpeed::Full,
            power: true,
        })
    }
    
    fn reset_port(&mut self, port: u8) -> Result<(), &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }
        
        // TODO: Implement port reset
        Ok(())
    }
    
    fn enable_port(&mut self, port: u8) -> Result<(), &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }
        
        // TODO: Implement port enable
        Ok(())
    }
    
    fn disable_port(&mut self, port: u8) -> Result<(), &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }
        
        // TODO: Implement port disable
        Ok(())
    }
    
    fn transfer(&mut self, device_address: u8, transfer: UsbTransfer) -> Result<Vec<u8>, &'static str> {
        // TODO: Implement actual USB transfers
        match transfer {
            UsbTransfer::Setup { data, .. } => {
                // Return dummy device descriptor for now
                if data.len() >= 18 {
                    let mut response = vec![0; 18];
                    response[0] = 18;   // bLength
                    response[1] = 1;    // bDescriptorType (device)
                    response[2] = 0x10; // bcdUSB (1.0)
                    response[3] = 0x01;
                    response[4] = 0;    // bDeviceClass
                    response[5] = 0;    // bDeviceSubClass
                    response[6] = 0;    // bDeviceProtocol
                    response[7] = 8;    // bMaxPacketSize0
                    response[8] = 0x34; // idVendor (dummy)
                    response[9] = 0x12;
                    response[10] = 0x78; // idProduct (dummy)
                    response[11] = 0x56;
                    return Ok(response);
                }
            }
            _ => {}
        }
        
        Err("Transfer not implemented")
    }
    
    fn set_device_address(&mut self, old_address: u8, new_address: u8) -> Result<(), &'static str> {
        // TODO: Implement SET_ADDRESS
        Ok(())
    }
}

/// Global USB bus instance
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
static USB_BUS: spin::Once<Mutex<UsbBus>> = spin::Once::new();

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static mut USB_BUS_STATIC: Option<Mutex<UsbBus>> = None;

/// Initialize USB subsystem
pub fn init() {
    let usb_bus = UsbBus::new();
    
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        USB_BUS.call_once(|| Mutex::new(usb_bus));
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        USB_BUS_STATIC = Some(Mutex::new(usb_bus));
    }
    
    // Add UHCI controller (placeholder)
    let uhci = UhciController::new(0); // No actual hardware for now
    if let Err(e) = get_usb_bus().lock().add_controller(Box::new(uhci)) {
        crate::println!("[USB] Failed to add UHCI controller: {}", e);
    }
    
    // Register with driver framework
    // Note: We create a new instance for the driver framework since Bus trait requires mut
    let driver_framework = crate::services::driver_framework::get_driver_framework();
    let bus_instance = UsbBus::new();
    
    if let Err(e) = driver_framework.register_bus(Box::new(bus_instance)) {
        crate::println!("[USB] Failed to register USB bus: {}", e);
    } else {
        crate::println!("[USB] USB bus driver initialized");
    }
}

/// Get the global USB bus
pub fn get_usb_bus() -> &'static Mutex<UsbBus> {
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        USB_BUS.get().expect("USB bus not initialized")
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        USB_BUS_STATIC.as_ref().expect("USB bus not initialized")
    }
}
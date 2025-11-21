//! USB Bus Driver
//!
//! Implements USB host controller and device management.

#![allow(static_mut_refs)]

use alloc::{boxed::Box, collections::BTreeMap, format, string::String, vec, vec::Vec};

use spin::{Mutex, RwLock};

use crate::services::driver_framework::{Bus, DeviceClass, DeviceId, DeviceInfo, DeviceStatus};

/// USB device speeds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,       // 1.5 Mbps
    Full,      // 12 Mbps
    High,      // 480 Mbps
    Super,     // 5 Gbps
    SuperPlus, // 10 Gbps
}

/// USB device classes
#[allow(dead_code)]
pub mod usb_classes {
    pub const AUDIO: u8 = 0x01;
    pub const CDC: u8 = 0x02; // Communications and CDC Control
    pub const HID: u8 = 0x03; // Human Interface Device
    pub const PHYSICAL: u8 = 0x05; // Physical
    pub const IMAGE: u8 = 0x06; // Image
    pub const PRINTER: u8 = 0x07; // Printer
    pub const MASS_STORAGE: u8 = 0x08; // Mass Storage
    pub const HUB: u8 = 0x09; // Hub
    pub const CDC_DATA: u8 = 0x0A; // CDC-Data
    pub const SMART_CARD: u8 = 0x0B; // Smart Card
    pub const CONTENT_SECURITY: u8 = 0x0D; // Content Security
    pub const VIDEO: u8 = 0x0E; // Video
    pub const HEALTHCARE: u8 = 0x0F; // Personal Healthcare
    pub const DIAGNOSTIC: u8 = 0xDC; // Diagnostic Device
    pub const WIRELESS: u8 = 0xE0; // Wireless Controller
    pub const MISC: u8 = 0xEF; // Miscellaneous
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
            direction: if address & 0x80 != 0 {
                UsbDirection::In
            } else {
                UsbDirection::Out
            },
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
    fn transfer(
        &mut self,
        device_address: u8,
        transfer: UsbTransfer,
    ) -> Result<Vec<u8>, &'static str>;

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

impl Default for UsbBus {
    fn default() -> Self {
        Self::new()
    }
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
    pub fn add_controller(
        &self,
        mut controller: Box<dyn UsbHostController>,
    ) -> Result<(), &'static str> {
        // Initialize the controller
        controller.init()?;

        let _controller_name: String = controller.name().into();
        let controller_index = self.controllers.read().len();

        // Scan ports for devices
        let _port_count = controller.get_port_count();
        crate::println!(
            "[USB] Controller {} has {} ports",
            _controller_name,
            _port_count
        );

        self.controllers.write().push(controller);

        // Scan for connected devices
        self.scan_controller_ports(controller_index)?;

        crate::println!("[USB] Added USB host controller: {}", _controller_name);
        Ok(())
    }

    /// Scan controller ports for devices
    fn scan_controller_ports(&self, controller_index: usize) -> Result<(), &'static str> {
        let port_count = {
            let controllers = self.controllers.read();
            controllers
                .get(controller_index)
                .ok_or("Controller not found")?
                .get_port_count()
        };

        for port in 1..=port_count {
            if let Err(_e) = self.scan_port(controller_index, port) {
                crate::println!("[USB] Failed to scan port {}: {}", port, _e);
            }
        }

        Ok(())
    }

    /// Scan a specific port
    fn scan_port(&self, controller_index: usize, port: u8) -> Result<(), &'static str> {
        let status = {
            let controllers = self.controllers.read();
            controllers
                .get(controller_index)
                .ok_or("Controller not found")?
                .get_port_status(port)?
        };

        if !status.connected {
            return Ok(()); // No device connected
        }

        crate::println!(
            "[USB] Device detected on controller {} port {}",
            controller_index,
            port
        );

        // Reset and enable port
        {
            let mut controllers = self.controllers.write();
            let controller = controllers
                .get_mut(controller_index)
                .ok_or("Controller not found")?;

            controller.reset_port(port)?;
            controller.enable_port(port)?;
        }

        // Enumerate device
        let device_address = self.enumerate_device(controller_index, port, status.speed)?;

        // Store port mapping
        self.port_devices
            .write()
            .insert((controller_index, port), device_address);

        Ok(())
    }

    /// Enumerate a device
    fn enumerate_device(
        &self,
        controller_index: usize,
        port: u8,
        speed: UsbSpeed,
    ) -> Result<u8, &'static str> {
        // Allocate device address
        let device_address = self
            .next_address
            .fetch_add(1, core::sync::atomic::Ordering::SeqCst);

        // Create device
        let mut device = UsbDevice::new(device_address, port, speed);

        // Get device descriptor
        self.read_device_descriptor(controller_index, 0, &mut device)?;

        // Set device address
        {
            let mut controllers = self.controllers.write();
            let controller = controllers
                .get_mut(controller_index)
                .ok_or("Controller not found")?;

            controller.set_device_address(0, device_address)?;
        }

        // Read full device descriptor with new address
        self.read_device_descriptor(controller_index, device_address, &mut device)?;

        // Read configurations
        self.read_configurations(controller_index, device_address, &mut device)?;

        device.connected = true;

        crate::println!(
            "[USB] Enumerated device {:04x}:{:04x} at address {}",
            device.descriptor.vendor_id,
            device.descriptor.product_id,
            device_address
        );

        // Store device
        self.devices.write().insert(device_address, device);

        Ok(device_address)
    }

    /// Read device descriptor
    fn read_device_descriptor(
        &self,
        controller_index: usize,
        address: u8,
        device: &mut UsbDevice,
    ) -> Result<(), &'static str> {
        let transfer = UsbTransfer::Setup {
            request_type: 0x80, // Device to host, standard, device
            request: 0x06,      // GET_DESCRIPTOR
            value: 0x0100,      // Device descriptor
            index: 0,
            data: vec![0; 18], // Device descriptor is 18 bytes
        };

        let response = {
            let mut controllers = self.controllers.write();
            let controller = controllers
                .get_mut(controller_index)
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
        if response[14] != 0 {
            // Manufacturer string index
            if let Ok(manufacturer) =
                self.read_string_descriptor(controller_index, address, response[14])
            {
                device.descriptor.manufacturer = manufacturer;
            }
        }

        if response[15] != 0 {
            // Product string index
            if let Ok(product) =
                self.read_string_descriptor(controller_index, address, response[15])
            {
                device.descriptor.product = product;
            }
        }

        if response[16] != 0 {
            // Serial number string index
            if let Ok(serial) = self.read_string_descriptor(controller_index, address, response[16])
            {
                device.descriptor.serial_number = serial;
            }
        }

        Ok(())
    }

    /// Read string descriptor
    fn read_string_descriptor(
        &self,
        controller_index: usize,
        address: u8,
        index: u8,
    ) -> Result<String, &'static str> {
        let transfer = UsbTransfer::Setup {
            request_type: 0x80,
            request: 0x06,
            value: 0x0300 | (index as u16),
            index: 0x0409, // English (US)
            data: vec![0; 255],
        };

        let response = {
            let mut controllers = self.controllers.write();
            let controller = controllers
                .get_mut(controller_index)
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
    fn read_configurations(
        &self,
        _controller_index: usize,
        _address: u8,
        device: &mut UsbDevice,
    ) -> Result<(), &'static str> {
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
        self.devices
            .read()
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
                    format!(
                        "USB Device {:04x}:{:04x}",
                        usb_device.descriptor.vendor_id, usb_device.descriptor.product_id
                    )
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

    fn read_config(
        &self,
        _device: &DeviceInfo,
        _offset: u16,
        _size: u8,
    ) -> Result<u32, &'static str> {
        // USB devices don't have traditional config space
        // This could be used for reading descriptors
        Err("USB devices don't support config space reads")
    }

    fn write_config(
        &mut self,
        _device: &DeviceInfo,
        _offset: u16,
        _value: u32,
        _size: u8,
    ) -> Result<(), &'static str> {
        // USB devices don't have traditional config space
        Err("USB devices don't support config space writes")
    }

    fn enable_device(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        let address = device.address as u8;

        if let Some(usb_device) = self.devices.write().get_mut(&address) {
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

        if let Some(usb_device) = self.devices.write().get_mut(&address) {
            usb_device.current_configuration = None;
            crate::println!("[USB] Disabled device at address {}", address);
        }

        Ok(())
    }
}

/// UHCI register offsets
#[allow(dead_code)]
mod uhci_regs {
    pub const USBCMD: u16 = 0x00; // USB Command Register
    pub const USBSTS: u16 = 0x02; // USB Status Register
    pub const USBINTR: u16 = 0x04; // USB Interrupt Enable Register
    pub const FRNUM: u16 = 0x06; // Frame Number Register
    pub const FLBASEADD: u16 = 0x08; // Frame List Base Address Register
    pub const SOFMOD: u16 = 0x0C; // Start of Frame Modify Register
    pub const PORTSC1: u16 = 0x10; // Port 1 Status/Control
    pub const PORTSC2: u16 = 0x12; // Port 2 Status/Control

    // USBCMD bits
    pub const CMD_RUN: u16 = 0x0001;
    pub const CMD_HCRESET: u16 = 0x0002;
    pub const CMD_GRESET: u16 = 0x0004;
    pub const CMD_EGSM: u16 = 0x0008;
    pub const CMD_FGR: u16 = 0x0010;
    pub const CMD_SWDBG: u16 = 0x0020;
    pub const CMD_CF: u16 = 0x0040;
    pub const CMD_MAXP: u16 = 0x0080;

    // USBSTS bits
    pub const STS_USBINT: u16 = 0x0001;
    pub const STS_ERROR: u16 = 0x0002;
    pub const STS_RD: u16 = 0x0004;
    pub const STS_HSE: u16 = 0x0008;
    pub const STS_HCPE: u16 = 0x0010;
    pub const STS_HCH: u16 = 0x0020;

    // Port status bits
    pub const PORT_CCS: u16 = 0x0001; // Current Connect Status
    pub const PORT_CSC: u16 = 0x0002; // Connect Status Change
    pub const PORT_PE: u16 = 0x0004; // Port Enable
    pub const PORT_PEC: u16 = 0x0008; // Port Enable Change
    pub const PORT_LS: u16 = 0x0030; // Line Status
    pub const PORT_RD: u16 = 0x0040; // Resume Detect
    pub const PORT_LSDA: u16 = 0x0100; // Low Speed Device Attached
    pub const PORT_PR: u16 = 0x0200; // Port Reset
    pub const PORT_SUSP: u16 = 0x1000; // Suspend
}

/// UHCI Transfer Descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UhciTd {
    /// Link pointer to next TD or QH
    pub link_pointer: u32,
    /// Control and status
    pub control_status: u32,
    /// Token (PID, device address, endpoint, data toggle)
    pub token: u32,
    /// Buffer pointer
    pub buffer_pointer: u32,
}

impl Default for UhciTd {
    fn default() -> Self {
        Self::new()
    }
}

impl UhciTd {
    pub const fn new() -> Self {
        Self {
            link_pointer: 1, // Terminate
            control_status: 0,
            token: 0,
            buffer_pointer: 0,
        }
    }

    /// Set up a SETUP packet TD
    pub fn setup_packet(&mut self, device_addr: u8, endpoint: u8, data_toggle: bool, max_len: u16) {
        self.token = 0x2D  // SETUP PID
            | ((device_addr as u32 & 0x7F) << 8)
            | ((endpoint as u32 & 0xF) << 15)
            | (if data_toggle { 1 << 19 } else { 0 })
            | ((max_len.saturating_sub(1) as u32 & 0x7FF) << 21);

        self.control_status = (3 << 27)  // 3 errors allowed
            | (1 << 23); // Active
    }

    /// Set up an IN packet TD
    pub fn in_packet(&mut self, device_addr: u8, endpoint: u8, data_toggle: bool, max_len: u16) {
        self.token = 0x69  // IN PID
            | ((device_addr as u32 & 0x7F) << 8)
            | ((endpoint as u32 & 0xF) << 15)
            | (if data_toggle { 1 << 19 } else { 0 })
            | ((max_len.saturating_sub(1) as u32 & 0x7FF) << 21);

        self.control_status = (3 << 27)  // 3 errors allowed
            | (1 << 23); // Active
    }

    /// Set up an OUT packet TD
    pub fn out_packet(&mut self, device_addr: u8, endpoint: u8, data_toggle: bool, max_len: u16) {
        self.token = 0xE1  // OUT PID
            | ((device_addr as u32 & 0x7F) << 8)
            | ((endpoint as u32 & 0xF) << 15)
            | (if data_toggle { 1 << 19 } else { 0 })
            | ((max_len.saturating_sub(1) as u32 & 0x7FF) << 21);

        self.control_status = (3 << 27)  // 3 errors allowed
            | (1 << 23); // Active
    }

    /// Check if TD is still active
    pub fn is_active(&self) -> bool {
        (self.control_status & (1 << 23)) != 0
    }

    /// Check if TD completed with error
    pub fn has_error(&self) -> bool {
        (self.control_status & (1 << 22)) != 0  // Stalled
            || (self.control_status & (1 << 21)) != 0  // Data Buffer Error
            || (self.control_status & (1 << 20)) != 0  // Babble Detected
            || (self.control_status & (1 << 19)) != 0  // NAK Received
            || (self.control_status & (1 << 18)) != 0  // CRC/Timeout Error
            || (self.control_status & (1 << 17)) != 0 // Bitstuff Error
    }

    /// Get actual length transferred
    pub fn actual_length(&self) -> u16 {
        let len = ((self.control_status + 1) & 0x7FF) as u16;
        if len == 0x7FF {
            0
        } else {
            len
        }
    }
}

/// UHCI Queue Head
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UhciQh {
    /// Horizontal link pointer (next QH)
    pub head_link: u32,
    /// Vertical link pointer (first TD)
    pub element_link: u32,
}

impl Default for UhciQh {
    fn default() -> Self {
        Self::new()
    }
}

impl UhciQh {
    pub const fn new() -> Self {
        Self {
            head_link: 1,    // Terminate
            element_link: 1, // Terminate
        }
    }
}

/// Simple UHCI host controller implementation
pub struct UhciController {
    base_address: u32,
    port_count: u8,
    name: String,
    /// Frame list physical address (must be 4KB aligned)
    frame_list_phys: u64,
    /// Control QH for control transfers
    control_qh_phys: u64,
    /// Transfer descriptors buffer
    td_buffer_phys: u64,
    /// Data buffer for transfers
    data_buffer_phys: u64,
    /// Whether controller is initialized
    initialized: bool,
}

impl UhciController {
    pub fn new(base_address: u32) -> Self {
        Self {
            base_address,
            port_count: 2, // UHCI typically has 2 ports
            name: String::from("UHCI"),
            frame_list_phys: 0,
            control_qh_phys: 0,
            td_buffer_phys: 0,
            data_buffer_phys: 0,
            initialized: false,
        }
    }

    /// Read a 16-bit register
    #[cfg(target_arch = "x86_64")]
    fn read_reg16(&self, offset: u16) -> u16 {
        use crate::arch::x86_64::inw;
        unsafe { inw((self.base_address as u16).wrapping_add(offset)) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn read_reg16(&self, offset: u16) -> u16 {
        // MMIO access for non-x86 architectures
        let addr = (self.base_address as usize) + (offset as usize);
        unsafe { core::ptr::read_volatile(addr as *const u16) }
    }

    /// Write a 16-bit register
    #[cfg(target_arch = "x86_64")]
    fn write_reg16(&self, offset: u16, value: u16) {
        use crate::arch::x86_64::outw;
        unsafe { outw((self.base_address as u16).wrapping_add(offset), value) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn write_reg16(&self, offset: u16, value: u16) {
        // MMIO access for non-x86 architectures
        let addr = (self.base_address as usize) + (offset as usize);
        unsafe { core::ptr::write_volatile(addr as *mut u16, value) }
    }

    /// Read a 32-bit register
    #[allow(dead_code)]
    #[cfg(target_arch = "x86_64")]
    fn read_reg32(&self, offset: u16) -> u32 {
        use crate::arch::x86_64::inl;
        unsafe { inl((self.base_address as u16).wrapping_add(offset)) }
    }

    #[allow(dead_code)]
    #[cfg(not(target_arch = "x86_64"))]
    fn read_reg32(&self, offset: u16) -> u32 {
        let addr = (self.base_address as usize) + (offset as usize);
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    /// Write a 32-bit register
    #[cfg(target_arch = "x86_64")]
    fn write_reg32(&self, offset: u16, value: u32) {
        use crate::arch::x86_64::outl;
        unsafe { outl((self.base_address as u16).wrapping_add(offset), value) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn write_reg32(&self, offset: u16, value: u32) {
        let addr = (self.base_address as usize) + (offset as usize);
        unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
    }

    /// Read port status register
    fn read_port_status(&self, port: u8) -> u16 {
        let offset = if port == 1 {
            uhci_regs::PORTSC1
        } else {
            uhci_regs::PORTSC2
        };
        self.read_reg16(offset)
    }

    /// Write port status register
    fn write_port_status(&self, port: u8, value: u16) {
        let offset = if port == 1 {
            uhci_regs::PORTSC1
        } else {
            uhci_regs::PORTSC2
        };
        self.write_reg16(offset, value);
    }

    /// Allocate controller data structures
    fn allocate_structures(&mut self) -> Result<(), &'static str> {
        // Allocate frame list (1024 entries * 4 bytes = 4KB, must be 4KB aligned)
        let frame_list_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| "Failed to allocate frame list")?;
        let frame_list_frame = frame_list_pages.first().ok_or("Empty allocation")?;
        self.frame_list_phys = frame_list_frame.as_addr().as_u64();

        // Clear frame list
        let frame_list_ptr = self.frame_list_phys as *mut u32;
        unsafe {
            for i in 0..1024 {
                core::ptr::write_volatile(frame_list_ptr.add(i), 1); // Terminate
            }
        }

        // Allocate control QH
        let qh_pages = crate::mm::allocate_pages(1, None).map_err(|_| "Failed to allocate QH")?;
        let qh_frame = qh_pages.first().ok_or("Empty allocation")?;
        self.control_qh_phys = qh_frame.as_addr().as_u64();

        // Initialize control QH
        let qh_ptr = self.control_qh_phys as *mut UhciQh;
        unsafe {
            core::ptr::write_volatile(qh_ptr, UhciQh::new());
        }

        // Point frame list entries to control QH
        unsafe {
            for i in 0..1024 {
                // QH pointer with QH bit set (bit 1)
                core::ptr::write_volatile(frame_list_ptr.add(i), (self.control_qh_phys as u32) | 2);
            }
        }

        // Allocate TD buffer (4KB for multiple TDs)
        let td_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| "Failed to allocate TD buffer")?;
        let td_frame = td_pages.first().ok_or("Empty allocation")?;
        self.td_buffer_phys = td_frame.as_addr().as_u64();

        // Clear TD buffer
        unsafe {
            core::ptr::write_bytes(self.td_buffer_phys as *mut u8, 0, 4096);
        }

        // Allocate data buffer for transfers
        let data_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| "Failed to allocate data buffer")?;
        let data_frame = data_pages.first().ok_or("Empty allocation")?;
        self.data_buffer_phys = data_frame.as_addr().as_u64();

        Ok(())
    }

    /// Wait for transfer completion with timeout
    fn wait_for_transfer(&self, td_ptr: *mut UhciTd, timeout_ms: u32) -> Result<(), &'static str> {
        let mut elapsed = 0u32;
        let poll_interval = 1; // 1ms per poll

        while elapsed < timeout_ms {
            let td = unsafe { core::ptr::read_volatile(td_ptr) };

            if !td.is_active() {
                if td.has_error() {
                    return Err("Transfer error");
                }
                return Ok(());
            }

            // Simple delay (should use proper timer in production)
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
            elapsed += poll_interval;
        }

        Err("Transfer timeout")
    }
}

impl UsbHostController for UhciController {
    fn name(&self) -> &str {
        &self.name
    }

    fn init(&mut self) -> Result<(), &'static str> {
        crate::println!(
            "[USB] Initializing UHCI controller at 0x{:x}",
            self.base_address
        );

        // Skip initialization if base address is 0 (no hardware)
        if self.base_address == 0 {
            crate::println!("[USB] UHCI: No hardware present, using software emulation");
            self.initialized = true;
            return Ok(());
        }

        // Allocate data structures
        self.allocate_structures()?;

        // Reset controller
        self.reset()?;

        // Set frame list base address
        self.write_reg32(uhci_regs::FLBASEADD, self.frame_list_phys as u32);

        // Set SOF timing (default value)
        self.write_reg16(uhci_regs::SOFMOD, 64);

        // Clear frame number
        self.write_reg16(uhci_regs::FRNUM, 0);

        // Clear status register by writing 1s to clear bits
        self.write_reg16(uhci_regs::USBSTS, 0x003F);

        // Enable interrupts (USB interrupt, error, resume detect)
        self.write_reg16(uhci_regs::USBINTR, 0x000F);

        // Start the controller (run, configure flag, max packet = 64)
        self.write_reg16(
            uhci_regs::USBCMD,
            uhci_regs::CMD_RUN | uhci_regs::CMD_CF | uhci_regs::CMD_MAXP,
        );

        // Wait for controller to start
        for _ in 0..100 {
            let status = self.read_reg16(uhci_regs::USBSTS);
            if (status & uhci_regs::STS_HCH) == 0 {
                break; // Controller is running
            }
            core::hint::spin_loop();
        }

        // Enable power on all ports
        for port in 1..=self.port_count {
            let _status = self.read_port_status(port);
            // Port power is always on in UHCI (no explicit control)
            crate::println!("[USB] Port {} initial status: 0x{:04x}", port, _status);
        }

        self.initialized = true;
        crate::println!("[USB] UHCI controller initialized");

        Ok(())
    }

    fn reset(&mut self) -> Result<(), &'static str> {
        if self.base_address == 0 {
            return Ok(());
        }

        // Stop the controller first
        let cmd = self.read_reg16(uhci_regs::USBCMD);
        self.write_reg16(uhci_regs::USBCMD, cmd & !uhci_regs::CMD_RUN);

        // Wait for halt
        for _ in 0..100 {
            let status = self.read_reg16(uhci_regs::USBSTS);
            if (status & uhci_regs::STS_HCH) != 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Global reset
        self.write_reg16(uhci_regs::USBCMD, uhci_regs::CMD_GRESET);

        // Wait at least 10ms for reset
        for _ in 0..10000 {
            core::hint::spin_loop();
        }

        // Clear global reset
        self.write_reg16(uhci_regs::USBCMD, 0);

        // Host controller reset
        self.write_reg16(uhci_regs::USBCMD, uhci_regs::CMD_HCRESET);

        // Wait for reset to complete
        for _ in 0..100 {
            let cmd = self.read_reg16(uhci_regs::USBCMD);
            if (cmd & uhci_regs::CMD_HCRESET) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        crate::println!("[USB] UHCI controller reset complete");
        Ok(())
    }

    fn get_port_count(&self) -> u8 {
        self.port_count
    }

    fn get_port_status(&self, port: u8) -> Result<UsbPortStatus, &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }

        // Return empty status for software emulation mode
        if self.base_address == 0 {
            return Ok(UsbPortStatus {
                connected: false,
                enabled: false,
                suspended: false,
                reset: false,
                speed: UsbSpeed::Full,
                power: true,
            });
        }

        let status = self.read_port_status(port);

        Ok(UsbPortStatus {
            connected: (status & uhci_regs::PORT_CCS) != 0,
            enabled: (status & uhci_regs::PORT_PE) != 0,
            suspended: (status & uhci_regs::PORT_SUSP) != 0,
            reset: (status & uhci_regs::PORT_PR) != 0,
            speed: if (status & uhci_regs::PORT_LSDA) != 0 {
                UsbSpeed::Low
            } else {
                UsbSpeed::Full
            },
            power: true, // UHCI ports are always powered
        })
    }

    fn reset_port(&mut self, port: u8) -> Result<(), &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }

        if self.base_address == 0 {
            return Ok(());
        }

        let current = self.read_port_status(port);

        // Set port reset bit
        self.write_port_status(port, current | uhci_regs::PORT_PR);

        // Wait at least 50ms for reset
        for _ in 0..50000 {
            core::hint::spin_loop();
        }

        // Clear port reset bit
        let current = self.read_port_status(port);
        self.write_port_status(port, current & !uhci_regs::PORT_PR);

        // Wait for reset to complete
        for _ in 0..1000 {
            core::hint::spin_loop();
        }

        // Clear status change bits by writing 1s
        let status = self.read_port_status(port);
        self.write_port_status(port, status | uhci_regs::PORT_CSC | uhci_regs::PORT_PEC);

        crate::println!("[USB] Port {} reset complete", port);
        Ok(())
    }

    fn enable_port(&mut self, port: u8) -> Result<(), &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }

        if self.base_address == 0 {
            return Ok(());
        }

        let current = self.read_port_status(port);

        // Enable port
        self.write_port_status(port, current | uhci_regs::PORT_PE);

        // Verify port is enabled
        let status = self.read_port_status(port);
        if (status & uhci_regs::PORT_PE) != 0 {
            crate::println!("[USB] Port {} enabled", port);
            Ok(())
        } else {
            Err("Failed to enable port")
        }
    }

    fn disable_port(&mut self, port: u8) -> Result<(), &'static str> {
        if port == 0 || port > self.port_count {
            return Err("Invalid port number");
        }

        if self.base_address == 0 {
            return Ok(());
        }

        let current = self.read_port_status(port);

        // Disable port
        self.write_port_status(port, current & !uhci_regs::PORT_PE);

        crate::println!("[USB] Port {} disabled", port);
        Ok(())
    }

    fn transfer(
        &mut self,
        device_address: u8,
        transfer: UsbTransfer,
    ) -> Result<Vec<u8>, &'static str> {
        // Software emulation mode - return simulated responses
        if self.base_address == 0 || !self.initialized {
            return match transfer {
                UsbTransfer::Setup { data, value, .. } => {
                    // Simulate GET_DESCRIPTOR response
                    if (value >> 8) == 1 {
                        // Device descriptor
                        let mut response = vec![0u8; data.len().min(18)];
                        if response.len() >= 18 {
                            response[0] = 18; // bLength
                            response[1] = 1; // bDescriptorType
                            response[2] = 0x10; // bcdUSB low
                            response[3] = 0x02; // bcdUSB high (USB 2.0)
                            response[4] = 0; // bDeviceClass
                            response[5] = 0; // bDeviceSubClass
                            response[6] = 0; // bDeviceProtocol
                            response[7] = 64; // bMaxPacketSize0
                            response[8] = 0x34; // idVendor low
                            response[9] = 0x12; // idVendor high
                            response[10] = 0x78; // idProduct low
                            response[11] = 0x56; // idProduct high
                            response[12] = 0x00; // bcdDevice low
                            response[13] = 0x01; // bcdDevice high
                            response[14] = 0; // iManufacturer
                            response[15] = 0; // iProduct
                            response[16] = 0; // iSerialNumber
                            response[17] = 1; // bNumConfigurations
                        }
                        Ok(response)
                    } else {
                        Ok(vec![0u8; data.len()])
                    }
                }
                UsbTransfer::In { length, .. } => Ok(vec![0u8; length]),
                UsbTransfer::Out { .. } => Ok(Vec::new()),
            };
        }

        // Hardware transfer implementation
        let td_ptr = self.td_buffer_phys as *mut UhciTd;
        let data_ptr = self.data_buffer_phys as *mut u8;
        let qh_ptr = self.control_qh_phys as *mut UhciQh;

        match transfer {
            UsbTransfer::Setup {
                request_type,
                request,
                value,
                index,
                data,
            } => {
                // Build setup packet (8 bytes)
                let setup_packet = [
                    request_type,
                    request,
                    (value & 0xFF) as u8,
                    ((value >> 8) & 0xFF) as u8,
                    (index & 0xFF) as u8,
                    ((index >> 8) & 0xFF) as u8,
                    (data.len() & 0xFF) as u8,
                    ((data.len() >> 8) & 0xFF) as u8,
                ];

                unsafe {
                    // Copy setup packet to data buffer
                    core::ptr::copy_nonoverlapping(setup_packet.as_ptr(), data_ptr, 8);

                    // Setup TD for SETUP phase
                    let setup_td = &mut *td_ptr;
                    setup_td.setup_packet(device_address, 0, false, 8);
                    setup_td.buffer_pointer = self.data_buffer_phys as u32;
                    setup_td.link_pointer = (self.td_buffer_phys as u32 + 16) | 4; // Next TD, depth first

                    // Data TD for IN phase (if reading)
                    let data_td = &mut *td_ptr.add(1);
                    let response_len = data.len().min(64) as u16;
                    data_td.in_packet(device_address, 0, true, response_len);
                    data_td.buffer_pointer = (self.data_buffer_phys + 64) as u32;
                    data_td.link_pointer = (self.td_buffer_phys as u32 + 32) | 4; // Next TD

                    // Status TD for OUT phase (zero-length OUT)
                    let status_td = &mut *td_ptr.add(2);
                    status_td.out_packet(device_address, 0, true, 0);
                    status_td.buffer_pointer = 0;
                    status_td.link_pointer = 1; // Terminate

                    // Point QH to first TD
                    let qh = &mut *qh_ptr;
                    qh.element_link = self.td_buffer_phys as u32; // TD pointer (bit 1 = 0 for TD)

                    // Wait for transfer completion
                    self.wait_for_transfer(td_ptr.add(2), 1000)?;

                    // Read response data
                    let actual_len = (*td_ptr.add(1)).actual_length() as usize;
                    let mut response = vec![0u8; actual_len.min(data.len())];
                    core::ptr::copy_nonoverlapping(
                        (self.data_buffer_phys + 64) as *const u8,
                        response.as_mut_ptr(),
                        response.len(),
                    );

                    Ok(response)
                }
            }

            UsbTransfer::In { endpoint, length } => {
                let transfer_len = length.min(4096) as u16;

                unsafe {
                    let in_td = &mut *td_ptr;
                    in_td.in_packet(device_address, endpoint, false, transfer_len);
                    in_td.buffer_pointer = self.data_buffer_phys as u32;
                    in_td.link_pointer = 1; // Terminate

                    let qh = &mut *qh_ptr;
                    qh.element_link = self.td_buffer_phys as u32;

                    self.wait_for_transfer(td_ptr, 1000)?;

                    let actual_len = (*td_ptr).actual_length() as usize;
                    let mut response = vec![0u8; actual_len];
                    core::ptr::copy_nonoverlapping(data_ptr, response.as_mut_ptr(), actual_len);

                    Ok(response)
                }
            }

            UsbTransfer::Out { endpoint, data } => {
                let transfer_len = data.len().min(4096);

                unsafe {
                    // Copy data to buffer
                    core::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, transfer_len);

                    let out_td = &mut *td_ptr;
                    out_td.out_packet(device_address, endpoint, false, transfer_len as u16);
                    out_td.buffer_pointer = self.data_buffer_phys as u32;
                    out_td.link_pointer = 1; // Terminate

                    let qh = &mut *qh_ptr;
                    qh.element_link = self.td_buffer_phys as u32;

                    self.wait_for_transfer(td_ptr, 1000)?;

                    Ok(Vec::new())
                }
            }
        }
    }

    fn set_device_address(&mut self, old_address: u8, new_address: u8) -> Result<(), &'static str> {
        // SET_ADDRESS is a control transfer
        let transfer = UsbTransfer::Setup {
            request_type: 0x00, // Host to device, standard, device
            request: 0x05,      // SET_ADDRESS
            value: new_address as u16,
            index: 0,
            data: Vec::new(),
        };

        // Send to old address (or address 0 for new devices)
        self.transfer(old_address, transfer)?;

        // Wait for device to switch addresses (at least 2ms per USB spec)
        for _ in 0..2000 {
            core::hint::spin_loop();
        }

        crate::println!(
            "[USB] Device address changed from {} to {}",
            old_address,
            new_address
        );

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
    if let Err(_e) = get_usb_bus().lock().add_controller(Box::new(uhci)) {
        crate::println!("[USB] Failed to add UHCI controller: {}", _e);
    }

    // Register with driver framework
    // Note: We create a new instance for the driver framework since Bus trait
    // requires mut
    let driver_framework = crate::services::driver_framework::get_driver_framework();
    let bus_instance = UsbBus::new();

    if let Err(_e) = driver_framework.register_bus(Box::new(bus_instance)) {
        crate::println!("[USB] Failed to register USB bus: {}", _e);
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

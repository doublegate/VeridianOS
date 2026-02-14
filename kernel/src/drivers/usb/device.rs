//! USB device types, descriptors, and bus-level device management
//!
//! Contains USB device representations, descriptor structures, and the
//! [`UsbBus`] implementation that manages device enumeration and the
//! [`Bus`] trait for integration with the driver framework.

use alloc::{boxed::Box, collections::BTreeMap, format, string::String, vec, vec::Vec};

use spin::RwLock;

use super::{host::UsbHostController, transfer::UsbTransfer};
use crate::{
    error::KernelError,
    services::driver_framework::{Bus, DeviceClass, DeviceId, DeviceInfo, DeviceStatus},
};

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
    ) -> Result<(), KernelError> {
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
    fn scan_controller_ports(&self, controller_index: usize) -> Result<(), KernelError> {
        let port_count = {
            let controllers = self.controllers.read();
            controllers
                .get(controller_index)
                .ok_or(KernelError::NotFound {
                    resource: "usb controller",
                    id: controller_index as u64,
                })?
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
    fn scan_port(&self, controller_index: usize, port: u8) -> Result<(), KernelError> {
        let status = {
            let controllers = self.controllers.read();
            controllers
                .get(controller_index)
                .ok_or(KernelError::NotFound {
                    resource: "usb controller",
                    id: controller_index as u64,
                })?
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
            let controller =
                controllers
                    .get_mut(controller_index)
                    .ok_or(KernelError::NotFound {
                        resource: "usb controller",
                        id: controller_index as u64,
                    })?;

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
    ) -> Result<u8, KernelError> {
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
            let controller =
                controllers
                    .get_mut(controller_index)
                    .ok_or(KernelError::NotFound {
                        resource: "usb controller",
                        id: controller_index as u64,
                    })?;

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
    ) -> Result<(), KernelError> {
        let transfer = UsbTransfer::Setup {
            request_type: 0x80, // Device to host, standard, device
            request: 0x06,      // GET_DESCRIPTOR
            value: 0x0100,      // Device descriptor
            index: 0,
            data: vec![0; 18], // Device descriptor is 18 bytes
        };

        let response = {
            let mut controllers = self.controllers.write();
            let controller =
                controllers
                    .get_mut(controller_index)
                    .ok_or(KernelError::NotFound {
                        resource: "usb controller",
                        id: controller_index as u64,
                    })?;

            controller.transfer(address, transfer)?
        };

        if response.len() < 18 {
            return Err(KernelError::HardwareError {
                device: "usb",
                code: 3,
            });
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
    ) -> Result<String, KernelError> {
        let transfer = UsbTransfer::Setup {
            request_type: 0x80,
            request: 0x06,
            value: 0x0300 | (index as u16),
            index: 0x0409, // English (US)
            data: vec![0; 255],
        };

        let response = {
            let mut controllers = self.controllers.write();
            let controller =
                controllers
                    .get_mut(controller_index)
                    .ok_or(KernelError::NotFound {
                        resource: "usb controller",
                        id: controller_index as u64,
                    })?;

            controller.transfer(address, transfer)?
        };

        if response.len() < 2 {
            return Err(KernelError::HardwareError {
                device: "usb",
                code: 4,
            });
        }

        let length = response[0] as usize;
        if length < 2 || response[1] != 0x03 {
            return Err(KernelError::HardwareError {
                device: "usb",
                code: 5,
            });
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
    ) -> Result<(), KernelError> {
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
    ) -> Result<u32, KernelError> {
        // USB devices don't have traditional config space
        // This could be used for reading descriptors
        Err(KernelError::OperationNotSupported {
            operation: "config space read on USB device",
        })
    }

    fn write_config(
        &mut self,
        _device: &DeviceInfo,
        _offset: u16,
        _value: u32,
        _size: u8,
    ) -> Result<(), KernelError> {
        // USB devices don't have traditional config space
        Err(KernelError::OperationNotSupported {
            operation: "config space write on USB device",
        })
    }

    fn enable_device(&mut self, device: &DeviceInfo) -> Result<(), KernelError> {
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

    fn disable_device(&mut self, device: &DeviceInfo) -> Result<(), KernelError> {
        let address = device.address as u8;

        if let Some(usb_device) = self.devices.write().get_mut(&address) {
            usb_device.current_configuration = None;
            crate::println!("[USB] Disabled device at address {}", address);
        }

        Ok(())
    }
}

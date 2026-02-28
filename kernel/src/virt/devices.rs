//! Virtual device emulation for guest VMs

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};

use super::VmError;

pub trait VirtualDevice: Send {
    fn handle_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> Result<(), VmError>;
    fn name(&self) -> &str;
    fn base_port(&self) -> u16;
    fn port_count(&self) -> u16;
}

const UART_BUFFER_SIZE: usize = 256;

pub struct VirtualUart {
    base: u16,
    #[cfg(feature = "alloc")]
    output_buffer: Vec<u8>,
    line_status: u8,
}

impl VirtualUart {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            #[cfg(feature = "alloc")]
            output_buffer: Vec::with_capacity(UART_BUFFER_SIZE),
            line_status: 0x60,
        }
    }

    fn write_byte(&mut self, byte: u8) {
        #[cfg(feature = "alloc")]
        {
            self.output_buffer.push(byte);
            if byte == b'\n' || self.output_buffer.len() >= UART_BUFFER_SIZE {
                self.flush();
            }
        }
        #[cfg(not(feature = "alloc"))]
        {
            let _ = byte;
        }
    }

    fn read_status(&self) -> u8 {
        self.line_status
    }

    #[cfg(feature = "alloc")]
    fn flush(&mut self) {
        if !self.output_buffer.is_empty() {
            for &b in &self.output_buffer {
                if b.is_ascii() {
                    crate::print!("{}", b as char);
                }
            }
            self.output_buffer.clear();
        }
    }
}

impl VirtualDevice for VirtualUart {
    fn handle_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> Result<(), VmError> {
        let offset = port - self.base;
        match offset {
            0 => {
                if is_write {
                    if let Some(&b) = data.first() {
                        self.write_byte(b);
                    }
                } else if let Some(d) = data.first_mut() {
                    *d = 0xFF;
                }
            }
            5 => {
                if !is_write {
                    if let Some(d) = data.first_mut() {
                        *d = self.read_status();
                    }
                }
            }
            1..=4 | 6 | 7 => {
                if !is_write {
                    if let Some(d) = data.first_mut() {
                        *d = 0x00;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn name(&self) -> &str {
        "8250 UART"
    }
    fn base_port(&self) -> u16 {
        self.base
    }
    fn port_count(&self) -> u16 {
        8
    }
}

pub struct VirtualPic {
    base: u16,
    mask: u8,
    isr: u8,
    is_master: bool,
}

impl VirtualPic {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            mask: 0xFF,
            isr: 0,
            is_master: base == 0x20,
        }
    }
}

impl VirtualDevice for VirtualPic {
    fn handle_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> Result<(), VmError> {
        let offset = port - self.base;
        match offset {
            0 => {
                if is_write {
                    if let Some(&b) = data.first() {
                        if b & 0x10 != 0 {
                            self.mask = 0xFF;
                            self.isr = 0;
                        } else if b == 0x20 {
                            self.isr = 0;
                        }
                    }
                } else if let Some(d) = data.first_mut() {
                    *d = self.isr;
                }
            }
            1 => {
                if is_write {
                    if let Some(&b) = data.first() {
                        self.mask = b;
                    }
                } else if let Some(d) = data.first_mut() {
                    *d = self.mask;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn name(&self) -> &str {
        if self.is_master {
            "8259A PIC (master)"
        } else {
            "8259A PIC (slave)"
        }
    }
    fn base_port(&self) -> u16 {
        self.base
    }
    fn port_count(&self) -> u16 {
        2
    }
}

#[cfg(feature = "alloc")]
struct DeviceRegistration {
    base: u16,
    size: u16,
    device: Box<dyn VirtualDevice>,
}

#[cfg(feature = "alloc")]
pub struct DeviceManager {
    devices: BTreeMap<u16, DeviceRegistration>,
}

#[cfg(feature = "alloc")]
impl DeviceManager {
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
        }
    }

    pub fn register_device(&mut self, base: u16, size: u16, device: Box<dyn VirtualDevice>) {
        self.devices
            .insert(base, DeviceRegistration { base, size, device });
    }

    pub fn handle_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> Result<(), VmError> {
        for reg in self.devices.values_mut() {
            if port >= reg.base && port < reg.base + reg.size {
                return reg.device.handle_io(port, is_write, data);
            }
        }
        if !is_write {
            for b in data.iter_mut() {
                *b = 0xFF;
            }
        }
        Ok(())
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn list_devices(&self) {
        for reg in self.devices.values() {
            crate::println!(
                "  [vdev] {} at ports 0x{:04x}-0x{:04x}",
                reg.device.name(),
                reg.base,
                reg.base + reg.size - 1
            );
        }
    }

    pub fn with_standard_devices() -> Self {
        let mut mgr = Self::new();
        mgr.register_device(0x3F8, 8, Box::new(VirtualUart::new(0x3F8)));
        mgr.register_device(0x2F8, 8, Box::new(VirtualUart::new(0x2F8)));
        mgr.register_device(0x20, 2, Box::new(VirtualPic::new(0x20)));
        mgr.register_device(0xA0, 2, Box::new(VirtualPic::new(0xA0)));
        mgr
    }
}

#[cfg(feature = "alloc")]
impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uart_write() {
        let mut uart = VirtualUart::new(0x3F8);
        let mut data = [b'A'];
        assert!(uart.handle_io(0x3F8, true, &mut data).is_ok());
    }

    #[test]
    fn test_uart_status() {
        let mut uart = VirtualUart::new(0x3F8);
        let mut data = [0u8];
        assert!(uart.handle_io(0x3FD, false, &mut data).is_ok());
        assert_eq!(data[0], 0x60);
    }

    #[test]
    fn test_pic_eoi() {
        let mut pic = VirtualPic::new(0x20);
        pic.isr = 1;
        let mut data = [0x20];
        assert!(pic.handle_io(0x20, true, &mut data).is_ok());
        assert_eq!(pic.isr, 0);
    }

    #[test]
    fn test_device_manager() {
        let mut mgr = DeviceManager::new();
        mgr.register_device(0x3F8, 8, Box::new(VirtualUart::new(0x3F8)));
        assert_eq!(mgr.device_count(), 1);
        let mut data = [b'X'];
        assert!(mgr.handle_io(0x3F8, true, &mut data).is_ok());
    }

    #[test]
    fn test_standard_devices() {
        let mgr = DeviceManager::with_standard_devices();
        assert_eq!(mgr.device_count(), 4);
    }
}

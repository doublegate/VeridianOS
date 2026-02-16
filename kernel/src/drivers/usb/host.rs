//! USB host controller trait and UHCI controller implementation
//!
//! Contains the [`UsbHostController`] trait that all USB host controllers must
//! implement, along with the [`UhciController`] (Universal Host Controller
//! Interface) implementation including register definitions and hardware
//! transfer logic.

use alloc::{string::String, vec, vec::Vec};

use super::{
    device::UsbPortStatus,
    transfer::{UhciQh, UhciTd, UsbTransfer},
    UsbSpeed,
};
use crate::error::KernelError;

/// USB host controller trait
pub trait UsbHostController: Send + Sync {
    /// Get controller name
    fn name(&self) -> &str;

    /// Initialize the controller
    fn init(&mut self) -> Result<(), KernelError>;

    /// Reset the controller
    fn reset(&mut self) -> Result<(), KernelError>;

    /// Get number of ports
    fn get_port_count(&self) -> u8;

    /// Check port status
    fn get_port_status(&self, port: u8) -> Result<UsbPortStatus, KernelError>;

    /// Reset port
    fn reset_port(&mut self, port: u8) -> Result<(), KernelError>;

    /// Enable port
    fn enable_port(&mut self, port: u8) -> Result<(), KernelError>;

    /// Disable port
    fn disable_port(&mut self, port: u8) -> Result<(), KernelError>;

    /// Perform USB transfer
    fn transfer(
        &mut self,
        device_address: u8,
        transfer: UsbTransfer,
    ) -> Result<Vec<u8>, KernelError>;

    /// Set device address
    fn set_device_address(&mut self, old_address: u8, new_address: u8) -> Result<(), KernelError>;
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
        // SAFETY: Reading a 16-bit I/O port at base_address + offset. The UHCI
        // controller's I/O base address was obtained from PCI configuration space.
        // We are in kernel mode with full I/O privilege.
        unsafe { inw((self.base_address as u16).wrapping_add(offset)) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn read_reg16(&self, offset: u16) -> u16 {
        // MMIO access for non-x86 architectures
        let addr = (self.base_address as usize) + (offset as usize);
        // SAFETY: MMIO read from the UHCI controller's memory-mapped register space.
        // The base address was provided during controller initialization from PCI BAR.
        // read_volatile prevents the compiler from reordering or eliding the access.
        unsafe { core::ptr::read_volatile(addr as *const u16) }
    }

    /// Write a 16-bit register
    #[cfg(target_arch = "x86_64")]
    fn write_reg16(&self, offset: u16, value: u16) {
        use crate::arch::x86_64::outw;
        // SAFETY: Writing a 16-bit I/O port at base_address + offset. Same invariants
        // as read_reg16: valid UHCI I/O base from PCI, kernel mode with I/O privilege.
        unsafe { outw((self.base_address as u16).wrapping_add(offset), value) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn write_reg16(&self, offset: u16, value: u16) {
        // MMIO access for non-x86 architectures
        let addr = (self.base_address as usize) + (offset as usize);
        // SAFETY: MMIO write to the UHCI controller's register space. Same invariants
        // as read_reg16: valid base from PCI BAR, write_volatile for proper ordering.
        unsafe { core::ptr::write_volatile(addr as *mut u16, value) }
    }

    /// Read a 32-bit register
    /// Completes the register-width API alongside
    /// read_reg16/write_reg16/write_reg32.
    #[allow(dead_code)]
    #[cfg(target_arch = "x86_64")]
    fn read_reg32(&self, offset: u16) -> u32 {
        use crate::arch::x86_64::inl;
        // SAFETY: Reading a 32-bit I/O port. Same invariants as read_reg16.
        unsafe { inl((self.base_address as u16).wrapping_add(offset)) }
    }

    /// Completes the register-width API alongside
    /// read_reg16/write_reg16/write_reg32.
    #[allow(dead_code)]
    #[cfg(not(target_arch = "x86_64"))]
    fn read_reg32(&self, offset: u16) -> u32 {
        let addr = (self.base_address as usize) + (offset as usize);
        // SAFETY: MMIO read of 32-bit register. Same invariants as read_reg16.
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    /// Write a 32-bit register
    #[cfg(target_arch = "x86_64")]
    fn write_reg32(&self, offset: u16, value: u32) {
        use crate::arch::x86_64::outl;
        // SAFETY: Writing a 32-bit I/O port. Same invariants as write_reg16.
        unsafe { outl((self.base_address as u16).wrapping_add(offset), value) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn write_reg32(&self, offset: u16, value: u32) {
        let addr = (self.base_address as usize) + (offset as usize);
        // SAFETY: MMIO write of 32-bit register. Same invariants as write_reg16.
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
    fn allocate_structures(&mut self) -> Result<(), KernelError> {
        // Allocate frame list (1024 entries * 4 bytes = 4KB, must be 4KB aligned)
        let frame_list_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| KernelError::OutOfMemory {
                requested: 4096,
                available: 0,
            })?;
        let frame_list_frame = frame_list_pages.first().ok_or(KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;
        self.frame_list_phys = frame_list_frame.as_addr().as_u64();

        // Clear frame list
        let frame_list_ptr = self.frame_list_phys as *mut u32;
        // SAFETY: frame_list_phys was just allocated as a full 4KB page (1024 u32
        // entries). write_volatile ensures proper ordering for hardware-visible
        // memory.
        unsafe {
            for i in 0..1024 {
                core::ptr::write_volatile(frame_list_ptr.add(i), 1); // Terminate
            }
        }

        // Allocate control QH
        let qh_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| KernelError::OutOfMemory {
                requested: 4096,
                available: 0,
            })?;
        let qh_frame = qh_pages.first().ok_or(KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;
        self.control_qh_phys = qh_frame.as_addr().as_u64();

        // Initialize control QH
        let qh_ptr = self.control_qh_phys as *mut UhciQh;
        // SAFETY: control_qh_phys points to a freshly allocated 4KB page, which is
        // large enough for a UhciQh (8 bytes). write_volatile for
        // hardware-visible memory.
        unsafe {
            core::ptr::write_volatile(qh_ptr, UhciQh::new());
        }

        // Point frame list entries to control QH
        // SAFETY: frame_list_ptr still points to the same 4KB-aligned page allocated
        // above. We write the physical address of the control QH with the QH
        // type bit (bit 1) set per the UHCI specification.
        unsafe {
            for i in 0..1024 {
                // QH pointer with QH bit set (bit 1)
                core::ptr::write_volatile(frame_list_ptr.add(i), (self.control_qh_phys as u32) | 2);
            }
        }

        // Allocate TD buffer (4KB for multiple TDs)
        let td_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| KernelError::OutOfMemory {
                requested: 4096,
                available: 0,
            })?;
        let td_frame = td_pages.first().ok_or(KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;
        self.td_buffer_phys = td_frame.as_addr().as_u64();

        // Clear TD buffer
        // SAFETY: td_buffer_phys was just allocated as a full 4KB page. write_bytes
        // zeroes the entire page to initialize all transfer descriptors.
        unsafe {
            core::ptr::write_bytes(self.td_buffer_phys as *mut u8, 0, 4096);
        }

        // Allocate data buffer for transfers
        let data_pages =
            crate::mm::allocate_pages(1, None).map_err(|_| KernelError::OutOfMemory {
                requested: 4096,
                available: 0,
            })?;
        let data_frame = data_pages.first().ok_or(KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;
        self.data_buffer_phys = data_frame.as_addr().as_u64();

        Ok(())
    }

    /// Wait for transfer completion with timeout
    fn wait_for_transfer(&self, td_ptr: *mut UhciTd, timeout_ms: u32) -> Result<(), KernelError> {
        let mut elapsed = 0u32;
        let poll_interval = 1; // 1ms per poll

        while elapsed < timeout_ms {
            // SAFETY: td_ptr points into the TD buffer page allocated by
            // allocate_structures. read_volatile ensures we see the latest
            // hardware-written status bits.
            let td = unsafe { core::ptr::read_volatile(td_ptr) };

            if !td.is_active() {
                if td.has_error() {
                    return Err(KernelError::HardwareError {
                        device: "uhci",
                        code: 1,
                    });
                }
                return Ok(());
            }

            // Simple delay (should use proper timer in production)
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
            elapsed += poll_interval;
        }

        Err(KernelError::Timeout {
            operation: "usb transfer",
            duration_ms: timeout_ms as u64,
        })
    }
}

impl UsbHostController for UhciController {
    fn name(&self) -> &str {
        &self.name
    }

    fn init(&mut self) -> Result<(), KernelError> {
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

    fn reset(&mut self) -> Result<(), KernelError> {
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

    fn get_port_status(&self, port: u8) -> Result<UsbPortStatus, KernelError> {
        if port == 0 || port > self.port_count {
            return Err(KernelError::InvalidArgument {
                name: "port",
                value: "out of range",
            });
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

    fn reset_port(&mut self, port: u8) -> Result<(), KernelError> {
        if port == 0 || port > self.port_count {
            return Err(KernelError::InvalidArgument {
                name: "port",
                value: "out of range",
            });
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

    fn enable_port(&mut self, port: u8) -> Result<(), KernelError> {
        if port == 0 || port > self.port_count {
            return Err(KernelError::InvalidArgument {
                name: "port",
                value: "out of range",
            });
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
            Err(KernelError::HardwareError {
                device: "uhci",
                code: 2,
            })
        }
    }

    fn disable_port(&mut self, port: u8) -> Result<(), KernelError> {
        if port == 0 || port > self.port_count {
            return Err(KernelError::InvalidArgument {
                name: "port",
                value: "out of range",
            });
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
    ) -> Result<Vec<u8>, KernelError> {
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

                // SAFETY: td_ptr, data_ptr, and qh_ptr all point into pages allocated
                // by allocate_structures(). The TD buffer can hold at least 3 TDs (48 bytes
                // out of 4096). The data buffer is a full 4KB page. We write the setup
                // packet (8 bytes) to data_ptr, configure 3 TDs for the SETUP/DATA/STATUS
                // phases per the UHCI specification, then read the response from the data
                // buffer at offset 64. copy_nonoverlapping is safe because source and
                // destination do not overlap and lengths are within allocated bounds.
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

                // SAFETY: Same allocated pages as the Setup case. We configure a single
                // IN TD pointing to the data buffer, wait for completion, then copy the
                // received data. actual_len is capped at 4096 by transfer_len.
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

                // SAFETY: Same allocated pages as Setup/In cases. We copy outgoing data
                // to the data buffer (transfer_len capped at 4096), configure the OUT TD,
                // and wait for hardware completion.
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

    fn set_device_address(&mut self, old_address: u8, new_address: u8) -> Result<(), KernelError> {
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

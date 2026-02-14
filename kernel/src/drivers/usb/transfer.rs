//! USB transfer types and UHCI transfer descriptors
//!
//! Contains data structures for USB data transfers including:
//! - USB transfer request types (Setup, In, Out)
//! - UHCI Transfer Descriptors (TD) for hardware-level transfers
//! - UHCI Queue Heads (QH) for scheduling transfers

use alloc::vec::Vec;

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

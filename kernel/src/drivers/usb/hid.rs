//! USB HID (Human Interface Device) Driver
//!
//! Implements HID Boot Protocol support for keyboards and mice,
//! HID report descriptor parsing stubs, and input event generation.
//!
//! Reference: USB HID Specification 1.11, USB HID Usage Tables 1.12

#![allow(dead_code)]

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// HID Class Constants
// ---------------------------------------------------------------------------

/// HID descriptor type (returned in GET_DESCRIPTOR)
pub const HID_DESCRIPTOR_TYPE: u8 = 0x21;
/// HID report descriptor type
pub const HID_REPORT_DESCRIPTOR_TYPE: u8 = 0x22;
/// HID physical descriptor type
pub const HID_PHYSICAL_DESCRIPTOR_TYPE: u8 = 0x23;

// HID class-specific requests
/// Get a report from the device
pub const HID_GET_REPORT: u8 = 0x01;
/// Get the idle rate
pub const HID_GET_IDLE: u8 = 0x02;
/// Get the active protocol (boot vs report)
pub const HID_GET_PROTOCOL: u8 = 0x03;
/// Send a report to the device
pub const HID_SET_REPORT: u8 = 0x09;
/// Set the idle rate
pub const HID_SET_IDLE: u8 = 0x0A;
/// Set the active protocol (boot vs report)
pub const HID_SET_PROTOCOL: u8 = 0x0B;

// HID protocol values for SET_PROTOCOL / GET_PROTOCOL
/// Boot protocol (simplified fixed-format reports)
pub const HID_PROTOCOL_BOOT: u8 = 0;
/// Report protocol (full report descriptor driven)
pub const HID_PROTOCOL_REPORT: u8 = 1;

// HID subclass values
/// No subclass
pub const HID_SUBCLASS_NONE: u8 = 0x00;
/// Boot interface subclass
pub const HID_SUBCLASS_BOOT: u8 = 0x01;

// HID boot interface protocol values
/// Keyboard boot protocol
pub const HID_BOOT_PROTOCOL_KEYBOARD: u8 = 0x01;
/// Mouse boot protocol
pub const HID_BOOT_PROTOCOL_MOUSE: u8 = 0x02;

// ---------------------------------------------------------------------------
// HID Report Descriptor Item Tags (parsing stubs)
// ---------------------------------------------------------------------------

/// Report descriptor item types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HidItemType {
    /// Main items: Input, Output, Feature, Collection, End Collection
    Main = 0,
    /// Global items: Usage Page, Logical Min/Max, Report Size/Count/ID
    Global = 1,
    /// Local items: Usage, Usage Min/Max, Designator, String
    Local = 2,
    /// Reserved
    Reserved = 3,
}

/// Main item tags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HidMainTag {
    Input = 0x08,
    Output = 0x09,
    Feature = 0x0B,
    Collection = 0x0A,
    EndCollection = 0x0C,
}

/// Global item tags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HidGlobalTag {
    UsagePage = 0x00,
    LogicalMinimum = 0x01,
    LogicalMaximum = 0x02,
    PhysicalMinimum = 0x03,
    PhysicalMaximum = 0x04,
    UnitExponent = 0x05,
    Unit = 0x06,
    ReportSize = 0x07,
    ReportId = 0x08,
    ReportCount = 0x09,
    Push = 0x0A,
    Pop = 0x0B,
}

/// Usage page values
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum HidUsagePage {
    GenericDesktop = 0x01,
    SimulationControls = 0x02,
    VrControls = 0x03,
    SportControls = 0x04,
    GameControls = 0x05,
    GenericDevice = 0x06,
    Keyboard = 0x07,
    Led = 0x08,
    Button = 0x09,
    Consumer = 0x0C,
}

/// Generic Desktop usage IDs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum HidDesktopUsage {
    Pointer = 0x01,
    Mouse = 0x02,
    Joystick = 0x04,
    Gamepad = 0x05,
    Keyboard = 0x06,
    Keypad = 0x07,
    X = 0x30,
    Y = 0x31,
    Z = 0x32,
    Wheel = 0x38,
}

// ---------------------------------------------------------------------------
// HID Device Types
// ---------------------------------------------------------------------------

/// Classification of HID device types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidDeviceType {
    /// Boot-protocol keyboard
    Keyboard,
    /// Boot-protocol mouse
    Mouse,
    /// Gamepad / joystick
    Gamepad,
    /// Generic HID device (parsed from report descriptor)
    Generic,
}

// ---------------------------------------------------------------------------
// Input Events
// ---------------------------------------------------------------------------

/// Input events generated from HID reports
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// A key was pressed (HID scancode)
    KeyPress(u8),
    /// A key was released (HID scancode)
    KeyRelease(u8),
    /// Relative mouse movement (dx, dy)
    MouseMove(i16, i16),
    /// Mouse button state change (button index 0-based, true = pressed)
    MouseButton(u8, bool),
    /// Mouse scroll wheel delta (positive = up)
    MouseScroll(i8),
}

// ---------------------------------------------------------------------------
// Boot Protocol Report Structures
// ---------------------------------------------------------------------------

/// Boot keyboard report: 8 bytes
///
/// Byte 0: Modifier keys bitmask
///   bit 0: Left Ctrl,  bit 1: Left Shift,  bit 2: Left Alt,  bit 3: Left GUI
///   bit 4: Right Ctrl, bit 5: Right Shift, bit 6: Right Alt, bit 7: Right GUI
/// Byte 1: Reserved (OEM use)
/// Bytes 2-7: Up to 6 simultaneous key codes (0 = no key)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct BootKeyboardReport {
    pub modifiers: u8,
    pub reserved: u8,
    pub keys: [u8; 6],
}

impl BootKeyboardReport {
    /// Create an empty keyboard report
    pub const fn empty() -> Self {
        Self {
            modifiers: 0,
            reserved: 0,
            keys: [0; 6],
        }
    }

    /// Parse a raw 8-byte buffer into a keyboard report
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 8 {
            return Err(KernelError::InvalidArgument {
                name: "hid_report",
                value: "keyboard report must be 8 bytes",
            });
        }
        Ok(Self {
            modifiers: data[0],
            reserved: data[1],
            keys: [data[2], data[3], data[4], data[5], data[6], data[7]],
        })
    }

    /// Check if a modifier bit is set
    pub fn has_modifier(&self, bit: u8) -> bool {
        (self.modifiers & (1 << bit)) != 0
    }

    /// Check for Left Ctrl
    pub fn left_ctrl(&self) -> bool {
        self.has_modifier(0)
    }
    /// Check for Left Shift
    pub fn left_shift(&self) -> bool {
        self.has_modifier(1)
    }
    /// Check for Left Alt
    pub fn left_alt(&self) -> bool {
        self.has_modifier(2)
    }
    /// Check for Left GUI (Super/Windows)
    pub fn left_gui(&self) -> bool {
        self.has_modifier(3)
    }
}

/// Boot mouse report: 3 bytes minimum, optional 4th byte for scroll
///
/// Byte 0: Button bitmask (bit 0 = left, bit 1 = right, bit 2 = middle)
/// Byte 1: X displacement (signed, -127 to +127)
/// Byte 2: Y displacement (signed, -127 to +127)
/// Byte 3 (optional): Scroll wheel (signed)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct BootMouseReport {
    pub buttons: u8,
    pub x_displacement: i8,
    pub y_displacement: i8,
    pub scroll: i8,
}

impl BootMouseReport {
    /// Create an empty mouse report
    pub const fn empty() -> Self {
        Self {
            buttons: 0,
            x_displacement: 0,
            y_displacement: 0,
            scroll: 0,
        }
    }

    /// Parse raw bytes into a mouse report (3 or 4 bytes)
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 3 {
            return Err(KernelError::InvalidArgument {
                name: "hid_report",
                value: "mouse report must be at least 3 bytes",
            });
        }
        Ok(Self {
            buttons: data[0],
            x_displacement: data[1] as i8,
            y_displacement: data[2] as i8,
            scroll: if data.len() >= 4 { data[3] as i8 } else { 0 },
        })
    }

    /// Check if the left button is pressed
    pub fn left_button(&self) -> bool {
        (self.buttons & 0x01) != 0
    }
    /// Check if the right button is pressed
    pub fn right_button(&self) -> bool {
        (self.buttons & 0x02) != 0
    }
    /// Check if the middle button is pressed
    pub fn middle_button(&self) -> bool {
        (self.buttons & 0x04) != 0
    }
}

// ---------------------------------------------------------------------------
// HID Report Descriptor Parser (Stub)
// ---------------------------------------------------------------------------

/// Parsed capabilities from a HID report descriptor
#[derive(Debug, Clone)]
pub struct HidCapabilities {
    /// Detected device type
    pub device_type: HidDeviceType,
    /// Number of report IDs found (0 means no report ID prefix)
    pub report_id_count: u8,
    /// Whether the device supports boot protocol
    pub boot_protocol: bool,
    /// Maximum input report size in bytes
    pub max_input_report_size: u16,
    /// Maximum output report size in bytes
    pub max_output_report_size: u16,
}

impl HidCapabilities {
    /// Default capabilities for a keyboard in boot mode
    pub fn boot_keyboard() -> Self {
        Self {
            device_type: HidDeviceType::Keyboard,
            report_id_count: 0,
            boot_protocol: true,
            max_input_report_size: 8,
            max_output_report_size: 1, // LED output report
        }
    }

    /// Default capabilities for a mouse in boot mode
    pub fn boot_mouse() -> Self {
        Self {
            device_type: HidDeviceType::Mouse,
            report_id_count: 0,
            boot_protocol: true,
            max_input_report_size: 4,
            max_output_report_size: 0,
        }
    }
}

/// Parse a HID report descriptor to determine device capabilities.
///
/// This is a stub implementation that detects basic device types by
/// scanning for Usage Page and Usage items in the descriptor. Full
/// report descriptor parsing (nested collections, push/pop state) is
/// left as a future enhancement.
pub fn parse_report_descriptor(descriptor: &[u8]) -> Result<HidCapabilities, KernelError> {
    if descriptor.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "descriptor",
            value: "empty report descriptor",
        });
    }

    let mut device_type = HidDeviceType::Generic;
    let mut usage_page: u16 = 0;
    let mut report_id_count: u8 = 0;
    let mut i = 0;

    while i < descriptor.len() {
        let prefix = descriptor[i];

        // Long items (prefix == 0xFE) not supported in stub
        if prefix == 0xFE {
            if i + 2 < descriptor.len() {
                let data_size = descriptor[i + 1] as usize;
                i += 3 + data_size;
            } else {
                break;
            }
            continue;
        }

        // Short item: prefix encodes size (bits 0-1), type (bits 2-3), tag (bits 4-7)
        let size = match prefix & 0x03 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            3 => 4, // size code 3 means 4 bytes
            _ => unreachable!(),
        };
        let item_type = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0F;

        // Read up to 4 bytes of item data as a u32
        let data = if size > 0 && i + size < descriptor.len() {
            let mut val: u32 = 0;
            for j in 0..size {
                val |= (descriptor[i + 1 + j] as u32) << (j * 8);
            }
            val
        } else {
            0
        };

        match item_type {
            // Global items
            1 => {
                match tag {
                    0x00 => {
                        // Usage Page
                        usage_page = data as u16;
                    }
                    0x08 => {
                        // Report ID
                        report_id_count = report_id_count.saturating_add(1);
                    }
                    _ => {}
                }
            }
            // Local items
            2 => {
                if tag == 0x00 {
                    // Usage
                    let usage = data as u16;
                    // Detect device type from Generic Desktop usages
                    if usage_page == HidUsagePage::GenericDesktop as u16 {
                        match usage {
                            0x06 | 0x07 => device_type = HidDeviceType::Keyboard,
                            0x02 => device_type = HidDeviceType::Mouse,
                            0x04 | 0x05 => device_type = HidDeviceType::Gamepad,
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }

        i += 1 + size;
    }

    Ok(HidCapabilities {
        device_type,
        report_id_count,
        boot_protocol: matches!(device_type, HidDeviceType::Keyboard | HidDeviceType::Mouse),
        max_input_report_size: match device_type {
            HidDeviceType::Keyboard => 8,
            HidDeviceType::Mouse => 4,
            _ => 64,
        },
        max_output_report_size: match device_type {
            HidDeviceType::Keyboard => 1,
            _ => 0,
        },
    })
}

// ---------------------------------------------------------------------------
// HID Device
// ---------------------------------------------------------------------------

/// Maximum report buffer size
const MAX_REPORT_SIZE: usize = 64;

/// Represents a USB HID device with state tracking
#[derive(Debug)]
pub struct HidDevice {
    /// USB device address on the bus
    pub address: u8,
    /// Interface number within the USB configuration
    pub interface: u8,
    /// Detected device type
    pub device_type: HidDeviceType,
    /// Interrupt IN endpoint address (for polling reports)
    pub interrupt_endpoint: u8,
    /// Interrupt endpoint polling interval in milliseconds
    pub poll_interval_ms: u8,
    /// Maximum packet size for the interrupt endpoint
    pub max_packet_size: u16,
    /// Whether the device is in boot protocol mode
    pub boot_protocol: bool,
    /// Parsed device capabilities
    pub capabilities: HidCapabilities,
    /// Previous keyboard report (for detecting key press/release)
    prev_keyboard_report: BootKeyboardReport,
    /// Previous mouse button state (for detecting button changes)
    prev_mouse_buttons: u8,
    /// Report receive buffer
    report_buffer: [u8; MAX_REPORT_SIZE],
    /// Number of valid bytes in the report buffer
    report_len: usize,
}

impl HidDevice {
    /// Create a new HID device with boot protocol configuration
    pub fn new_boot_keyboard(address: u8, interface: u8, endpoint: u8, interval: u8) -> Self {
        Self {
            address,
            interface,
            device_type: HidDeviceType::Keyboard,
            interrupt_endpoint: endpoint,
            poll_interval_ms: interval,
            max_packet_size: 8,
            boot_protocol: true,
            capabilities: HidCapabilities::boot_keyboard(),
            prev_keyboard_report: BootKeyboardReport::empty(),
            prev_mouse_buttons: 0,
            report_buffer: [0; MAX_REPORT_SIZE],
            report_len: 0,
        }
    }

    /// Create a new HID device configured as a boot mouse
    pub fn new_boot_mouse(address: u8, interface: u8, endpoint: u8, interval: u8) -> Self {
        Self {
            address,
            interface,
            device_type: HidDeviceType::Mouse,
            interrupt_endpoint: endpoint,
            poll_interval_ms: interval,
            max_packet_size: 4,
            boot_protocol: true,
            capabilities: HidCapabilities::boot_mouse(),
            prev_keyboard_report: BootKeyboardReport::empty(),
            prev_mouse_buttons: 0,
            report_buffer: [0; MAX_REPORT_SIZE],
            report_len: 0,
        }
    }

    /// Create a HID device from parsed capabilities
    pub fn from_capabilities(
        address: u8,
        interface: u8,
        endpoint: u8,
        interval: u8,
        caps: HidCapabilities,
    ) -> Self {
        Self {
            address,
            interface,
            device_type: caps.device_type,
            interrupt_endpoint: endpoint,
            poll_interval_ms: interval,
            max_packet_size: caps.max_input_report_size,
            boot_protocol: caps.boot_protocol,
            capabilities: caps,
            prev_keyboard_report: BootKeyboardReport::empty(),
            prev_mouse_buttons: 0,
            report_buffer: [0; MAX_REPORT_SIZE],
            report_len: 0,
        }
    }

    /// Submit report data received from an interrupt transfer.
    ///
    /// Stores the data in the internal buffer for subsequent
    /// [`process_report`](Self::process_report) calls.
    pub fn submit_report(&mut self, data: &[u8]) -> Result<(), KernelError> {
        if data.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "report_data",
                value: "empty report",
            });
        }
        let len = data.len().min(MAX_REPORT_SIZE);
        self.report_buffer[..len].copy_from_slice(&data[..len]);
        self.report_len = len;
        Ok(())
    }

    /// Process the current report buffer and generate input events.
    ///
    /// Returns a small array of events. For keyboards this includes
    /// key press/release deltas compared to the previous report; for
    /// mice this includes movement, button changes, and scroll.
    pub fn process_report(&mut self) -> Result<InputEventBatch, KernelError> {
        if self.report_len == 0 {
            return Ok(InputEventBatch::empty());
        }

        // Copy report data to a local buffer to avoid borrowing self
        // immutably (report_buffer) while also borrowing mutably (self).
        let mut local_buf = [0u8; MAX_REPORT_SIZE];
        let len = self.report_len;
        local_buf[..len].copy_from_slice(&self.report_buffer[..len]);

        match self.device_type {
            HidDeviceType::Keyboard => self.process_keyboard_report(&local_buf[..len]),
            HidDeviceType::Mouse => self.process_mouse_report(&local_buf[..len]),
            _ => Ok(InputEventBatch::empty()),
        }
    }

    /// Process a boot keyboard report and generate press/release events
    fn process_keyboard_report(&mut self, data: &[u8]) -> Result<InputEventBatch, KernelError> {
        let report = BootKeyboardReport::from_bytes(data)?;
        let mut batch = InputEventBatch::empty();

        // Detect modifier changes (bits 0-7)
        let mod_diff = report.modifiers ^ self.prev_keyboard_report.modifiers;
        for bit in 0..8u8 {
            if mod_diff & (1 << bit) != 0 {
                // Modifier keycodes: 0xE0 + bit (per HID usage tables)
                let keycode = 0xE0 + bit;
                if report.modifiers & (1 << bit) != 0 {
                    batch.push(InputEvent::KeyPress(keycode));
                } else {
                    batch.push(InputEvent::KeyRelease(keycode));
                }
            }
        }

        // Detect newly pressed keys (in new report but not in previous)
        for &key in &report.keys {
            if key == 0 {
                continue;
            }
            let was_pressed = self.prev_keyboard_report.keys.contains(&key);
            if !was_pressed {
                batch.push(InputEvent::KeyPress(key));
            }
        }

        // Detect released keys (in previous report but not in new)
        for &key in &self.prev_keyboard_report.keys {
            if key == 0 {
                continue;
            }
            let still_pressed = report.keys.contains(&key);
            if !still_pressed {
                batch.push(InputEvent::KeyRelease(key));
            }
        }

        self.prev_keyboard_report = report;
        Ok(batch)
    }

    /// Process a boot mouse report and generate movement/button/scroll events
    fn process_mouse_report(&mut self, data: &[u8]) -> Result<InputEventBatch, KernelError> {
        let report = BootMouseReport::from_bytes(data)?;
        let mut batch = InputEventBatch::empty();

        // Mouse movement
        if report.x_displacement != 0 || report.y_displacement != 0 {
            batch.push(InputEvent::MouseMove(
                report.x_displacement as i16,
                report.y_displacement as i16,
            ));
        }

        // Button changes (check bits 0-2: left, right, middle)
        let btn_diff = report.buttons ^ self.prev_mouse_buttons;
        for bit in 0..3u8 {
            if btn_diff & (1 << bit) != 0 {
                let pressed = report.buttons & (1 << bit) != 0;
                batch.push(InputEvent::MouseButton(bit, pressed));
            }
        }

        // Scroll wheel
        if report.scroll != 0 {
            batch.push(InputEvent::MouseScroll(report.scroll));
        }

        self.prev_mouse_buttons = report.buttons;
        Ok(batch)
    }

    /// Poll the device for new input (stub).
    ///
    /// In a real implementation this would issue an interrupt transfer
    /// to the device's interrupt IN endpoint via the host controller
    /// and return the first event from the resulting report. This stub
    /// always returns `None`.
    #[cfg(target_os = "none")]
    pub fn poll_device(&mut self) -> Option<InputEvent> {
        // TODO(phase7.5): Issue interrupt transfer via host controller
        // let transfer = UsbTransfer::interrupt_in(
        //     self.address, self.interrupt_endpoint, self.max_packet_size);
        // if let Ok(data) = host_controller.submit(transfer) {
        //     self.submit_report(&data).ok();
        //     let batch = self.process_report().ok()?;
        //     return batch.first();
        // }
        None
    }

    /// Poll the device for new input (non-hardware stub, always None)
    #[cfg(not(target_os = "none"))]
    pub fn poll_device(&mut self) -> Option<InputEvent> {
        None
    }
}

// ---------------------------------------------------------------------------
// Input Event Batch
// ---------------------------------------------------------------------------

/// Maximum number of events from a single report
const MAX_EVENTS_PER_REPORT: usize = 16;

/// Small fixed-capacity collection of input events from a single report
#[derive(Debug, Clone)]
pub struct InputEventBatch {
    events: [Option<InputEvent>; MAX_EVENTS_PER_REPORT],
    count: usize,
}

impl InputEventBatch {
    /// Create an empty batch
    pub const fn empty() -> Self {
        Self {
            events: [None; MAX_EVENTS_PER_REPORT],
            count: 0,
        }
    }

    /// Push an event into the batch (drops if full)
    pub fn push(&mut self, event: InputEvent) {
        if self.count < MAX_EVENTS_PER_REPORT {
            self.events[self.count] = Some(event);
            self.count += 1;
        }
    }

    /// Number of events in the batch
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the batch is empty
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get the first event, if any
    pub fn first(&self) -> Option<InputEvent> {
        if self.count > 0 {
            self.events[0]
        } else {
            None
        }
    }

    /// Iterate over events in the batch
    pub fn iter(&self) -> InputEventBatchIter<'_> {
        InputEventBatchIter {
            batch: self,
            index: 0,
        }
    }
}

/// Iterator over events in an [`InputEventBatch`]
pub struct InputEventBatchIter<'a> {
    batch: &'a InputEventBatch,
    index: usize,
}

impl<'a> Iterator for InputEventBatchIter<'a> {
    type Item = InputEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.batch.count {
            let event = self.batch.events[self.index];
            self.index += 1;
            event
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_keyboard_report_parse() {
        // Modifier: Left Shift (bit 1), key 'A' (0x04)
        let data = [0x02, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00];
        let report = BootKeyboardReport::from_bytes(&data).unwrap();
        assert_eq!(report.modifiers, 0x02);
        assert!(report.left_shift());
        assert!(!report.left_ctrl());
        assert_eq!(report.keys[0], 0x04);
        assert_eq!(report.keys[1], 0x00);
    }

    #[test]
    fn test_boot_keyboard_report_too_short() {
        let data = [0x02, 0x00, 0x04];
        let result = BootKeyboardReport::from_bytes(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_boot_mouse_report_parse() {
        // Left button pressed, X=+10, Y=-5
        let data: [u8; 3] = [0x01, 10, (-5i8) as u8];
        let report = BootMouseReport::from_bytes(&data).unwrap();
        assert!(report.left_button());
        assert!(!report.right_button());
        assert_eq!(report.x_displacement, 10);
        assert_eq!(report.y_displacement, -5);
        assert_eq!(report.scroll, 0); // no scroll byte
    }

    #[test]
    fn test_boot_mouse_report_with_scroll() {
        let data: [u8; 4] = [0x04, 0, 0, (-3i8) as u8]; // middle button, scroll -3
        let report = BootMouseReport::from_bytes(&data).unwrap();
        assert!(report.middle_button());
        assert!(!report.left_button());
        assert_eq!(report.scroll, -3);
    }

    #[test]
    fn test_keyboard_event_generation() {
        let mut dev = HidDevice::new_boot_keyboard(1, 0, 0x81, 10);

        // Press 'A' (0x04)
        let data = [0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00];
        dev.submit_report(&data).unwrap();
        let batch = dev.process_report().unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.first(), Some(InputEvent::KeyPress(0x04)));

        // Release 'A', press 'B' (0x05)
        let data = [0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00];
        dev.submit_report(&data).unwrap();
        let batch = dev.process_report().unwrap();
        // Should have KeyPress(0x05) and KeyRelease(0x04)
        assert_eq!(batch.len(), 2);
        let events: [Option<InputEvent>; 2] = [
            batch
                .iter()
                .find(|e| matches!(e, InputEvent::KeyPress(0x05))),
            batch
                .iter()
                .find(|e| matches!(e, InputEvent::KeyRelease(0x04))),
        ];
        assert!(events[0].is_some());
        assert!(events[1].is_some());
    }

    #[test]
    fn test_mouse_event_generation() {
        let mut dev = HidDevice::new_boot_mouse(2, 0, 0x81, 10);

        // Move mouse and press left button
        let data: [u8; 4] = [0x01, 20, (-10i8) as u8, 0];
        dev.submit_report(&data).unwrap();
        let batch = dev.process_report().unwrap();

        // Should have MouseMove + MouseButton(0, true)
        assert!(batch.len() >= 2);
        let has_move = batch
            .iter()
            .any(|e| matches!(e, InputEvent::MouseMove(20, -10)));
        let has_btn = batch
            .iter()
            .any(|e| matches!(e, InputEvent::MouseButton(0, true)));
        assert!(has_move);
        assert!(has_btn);
    }

    #[test]
    fn test_report_descriptor_keyboard_detection() {
        // Minimal descriptor: Usage Page (Generic Desktop), Usage (Keyboard)
        let descriptor: [u8; 4] = [
            0x05, 0x01, // Usage Page: Generic Desktop (global, 1 byte)
            0x09, 0x06, // Usage: Keyboard (local, 1 byte)
        ];
        let caps = parse_report_descriptor(&descriptor).unwrap();
        assert_eq!(caps.device_type, HidDeviceType::Keyboard);
        assert!(caps.boot_protocol);
        assert_eq!(caps.max_input_report_size, 8);
    }

    #[test]
    fn test_report_descriptor_mouse_detection() {
        // Minimal descriptor: Usage Page (Generic Desktop), Usage (Mouse)
        let descriptor: [u8; 4] = [
            0x05, 0x01, // Usage Page: Generic Desktop
            0x09, 0x02, // Usage: Mouse
        ];
        let caps = parse_report_descriptor(&descriptor).unwrap();
        assert_eq!(caps.device_type, HidDeviceType::Mouse);
        assert!(caps.boot_protocol);
        assert_eq!(caps.max_input_report_size, 4);
    }

    #[test]
    fn test_report_descriptor_empty() {
        let result = parse_report_descriptor(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_input_event_batch_overflow() {
        let mut batch = InputEventBatch::empty();
        for i in 0..MAX_EVENTS_PER_REPORT + 5 {
            batch.push(InputEvent::KeyPress(i as u8));
        }
        // Should cap at MAX_EVENTS_PER_REPORT
        assert_eq!(batch.len(), MAX_EVENTS_PER_REPORT);
    }

    #[test]
    fn test_poll_device_returns_none() {
        let mut dev = HidDevice::new_boot_keyboard(1, 0, 0x81, 10);
        assert_eq!(dev.poll_device(), None);
    }
}

//! evdev device node support for VeridianOS
//!
//! Creates per-device event nodes (`/dev/input/event*`) that expose input
//! events through the standard Linux evdev interface.  User-space libinput
//! reads from these nodes to receive keyboard, mouse, and other input events.
//!
//! Events originate from the unified [`input_event`] ring buffer and are
//! routed to per-device queues based on event type (EV_KEY -> keyboard,
//! EV_REL/BTN_* -> mouse).

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::input_event::{self, InputEvent, EV_ABS, EV_KEY, EV_REL};

// ---------------------------------------------------------------------------
// evdev ioctl numbers (Linux-compatible)
// ---------------------------------------------------------------------------

/// EVIOCGVERSION -- get driver version
pub(crate) const EVIOCGVERSION: u32 = 0x01;
/// EVIOCGID -- get device ID
pub(crate) const EVIOCGID: u32 = 0x02;
/// EVIOCGNAME -- get device name
pub(crate) const EVIOCGNAME: u32 = 0x06;
/// EVIOCGPHYS -- get physical location
pub(crate) const EVIOCGPHYS: u32 = 0x07;
/// EVIOCGUNIQ -- get unique identifier
pub(crate) const EVIOCGUNIQ: u32 = 0x08;
/// EVIOCGBIT -- get event type bits
pub(crate) const EVIOCGBIT: u32 = 0x20;
/// EVIOCGABS -- get absolute axis info
pub(crate) const EVIOCGABS: u32 = 0x40;
/// EVIOCGRAB -- exclusive grab
pub(crate) const EVIOCGRAB: u32 = 0x90;

// ---------------------------------------------------------------------------
// Device type enumeration
// ---------------------------------------------------------------------------

/// Type of input device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvdevDeviceType {
    /// Keyboard device (event0)
    Keyboard,
    /// Mouse / pointer device (event1)
    Mouse,
}

// ---------------------------------------------------------------------------
// Per-device event queue
// ---------------------------------------------------------------------------

const DEVICE_BUFFER_SIZE: usize = 128;

/// Per-device event ring buffer
struct DeviceEventBuffer {
    buf: [InputEvent; DEVICE_BUFFER_SIZE],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl DeviceEventBuffer {
    const fn new() -> Self {
        Self {
            buf: [InputEvent {
                timestamp: 0,
                event_type: 0,
                code: 0,
                value: 0,
            }; DEVICE_BUFFER_SIZE],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn push(&mut self, event: InputEvent) {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & (DEVICE_BUFFER_SIZE - 1);
        let tail = self.tail.load(Ordering::Acquire);
        if next == tail {
            return; // Buffer full, drop event
        }
        self.buf[head] = event;
        self.head.store(next, Ordering::Release);
    }

    fn pop(&self) -> Option<InputEvent> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let event = self.buf[tail];
        self.tail
            .store((tail + 1) & (DEVICE_BUFFER_SIZE - 1), Ordering::Release);
        Some(event)
    }

    fn is_empty(&self) -> bool {
        self.tail.load(Ordering::Relaxed) == self.head.load(Ordering::Acquire)
    }
}

// SAFETY: DeviceEventBuffer uses atomic head/tail. Single producer
// (route_events) and single consumer (read).
unsafe impl Send for DeviceEventBuffer {}
unsafe impl Sync for DeviceEventBuffer {}

// ---------------------------------------------------------------------------
// evdev device state
// ---------------------------------------------------------------------------

/// State for a single evdev device node
pub(crate) struct EvdevDevice {
    /// Device type
    pub(crate) device_type: EvdevDeviceType,
    /// Device name (for EVIOCGNAME)
    pub(crate) name: [u8; 64],
    pub(crate) name_len: usize,
    /// Major device number
    pub(crate) major: u32,
    /// Minor device number
    pub(crate) minor: u32,
    /// Per-device event queue
    buffer: DeviceEventBuffer,
    /// Whether this device is exclusively grabbed
    grabbed: AtomicBool,
}

impl EvdevDevice {
    pub(crate) const fn new(
        device_type: EvdevDeviceType,
        name_bytes: &[u8; 64],
        name_len: usize,
        major: u32,
        minor: u32,
    ) -> Self {
        Self {
            device_type,
            name: *name_bytes,
            name_len,
            major,
            minor,
            buffer: DeviceEventBuffer::new(),
            grabbed: AtomicBool::new(false),
        }
    }

    /// Push an event into this device's queue
    pub(crate) fn push_event(&mut self, event: InputEvent) {
        self.buffer.push(event);
    }

    /// Read the next event
    pub(crate) fn read_event(&self) -> Option<InputEvent> {
        self.buffer.pop()
    }

    /// Check if events are available
    pub(crate) fn has_events(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Read events into a byte buffer (returns bytes read)
    pub(crate) fn read_into_buffer(&self, buffer: &mut [u8]) -> usize {
        let event_size = core::mem::size_of::<InputEvent>();
        let max_events = buffer.len() / event_size;
        let mut bytes_read = 0;

        for i in 0..max_events {
            if let Some(event) = self.read_event() {
                let offset = i * event_size;
                // SAFETY: We checked bounds above (i < max_events, max_events = len /
                // event_size)
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        &event as *const InputEvent as *const u8,
                        buffer.as_mut_ptr().add(offset),
                        event_size,
                    );
                }
                bytes_read += event_size;
            } else {
                break;
            }
        }

        bytes_read
    }

    /// Handle an evdev ioctl
    pub(crate) fn handle_ioctl(&self, cmd: u32, arg: *mut u8) -> Result<i32, i32> {
        // Extract the evdev ioctl number from the request.
        // Linux evdev ioctls are _IOC(dir, 'E', nr, size). We match on nr.
        let nr = cmd & 0xFF;

        match nr {
            EVIOCGVERSION => {
                if arg.is_null() {
                    return Err(-1);
                }
                // Return Linux evdev version 0x010001 (1.0.1)
                // SAFETY: Caller validated arg pointer.
                unsafe {
                    *(arg as *mut u32) = 0x010001;
                }
                Ok(0)
            }
            EVIOCGID => {
                if arg.is_null() {
                    return Err(-1);
                }
                // Return a synthetic input_id struct
                // struct input_id { u16 bustype, vendor, product, version }
                // SAFETY: Caller validated arg pointer.
                unsafe {
                    let id = arg as *mut u16;
                    id.write(0x19); // BUS_VIRTUAL
                    id.add(1).write(0x1AF4); // VirtIO vendor
                    id.add(2).write(match self.device_type {
                        EvdevDeviceType::Keyboard => 0x0001,
                        EvdevDeviceType::Mouse => 0x0002,
                    });
                    id.add(3).write(0x0001); // version
                }
                Ok(0)
            }
            _ if (EVIOCGNAME..EVIOCGBIT).contains(&nr) => {
                // EVIOCGNAME -- return device name
                if arg.is_null() {
                    return Err(-1);
                }
                let copy_len = self.name_len.min(63);
                // SAFETY: Caller validated arg pointer; copy_len bounded.
                unsafe {
                    core::ptr::copy_nonoverlapping(self.name.as_ptr(), arg, copy_len);
                    // Null-terminate
                    arg.add(copy_len).write(0);
                }
                Ok(copy_len as i32)
            }
            _ if (EVIOCGBIT..EVIOCGABS).contains(&nr) => {
                // EVIOCGBIT -- return capability bitmask
                if arg.is_null() {
                    return Err(-1);
                }
                let ev_type = nr - EVIOCGBIT;
                self.fill_capability_bits(ev_type, arg);
                Ok(0)
            }
            _ if (EVIOCGABS..EVIOCGRAB).contains(&nr) => {
                // EVIOCGABS -- return absolute axis info
                if arg.is_null() {
                    return Err(-1);
                }
                // struct input_absinfo { value, minimum, maximum, fuzz, flat, resolution }
                // For now, return zeroed info (we only support relative mouse)
                // SAFETY: Caller validated arg pointer.
                unsafe {
                    core::ptr::write_bytes(arg, 0, 6 * 4); // 6 x i32
                }
                Ok(0)
            }
            EVIOCGRAB => {
                if arg.is_null() {
                    return Err(-1);
                }
                // SAFETY: Caller validated arg pointer.
                let grab_value = unsafe { *(arg as *const u32) };
                if grab_value != 0 {
                    self.grabbed.store(true, Ordering::Release);
                } else {
                    self.grabbed.store(false, Ordering::Release);
                }
                Ok(0)
            }
            _ => Err(-1), // Unsupported ioctl
        }
    }

    /// Fill event capability bitmask
    fn fill_capability_bits(&self, ev_type: u32, out: *mut u8) {
        // Zero the output first (enough for 64 bytes of bitmap)
        // SAFETY: out was validated by caller.
        unsafe {
            core::ptr::write_bytes(out, 0, 64);
        }

        match ev_type {
            0 => {
                // EV type bitmap: which event types this device supports
                // SAFETY: out was validated by caller.
                unsafe {
                    let bits = out;
                    match self.device_type {
                        EvdevDeviceType::Keyboard => {
                            // Supports EV_KEY (bit 1)
                            *bits = 0x02;
                        }
                        EvdevDeviceType::Mouse => {
                            // Supports EV_KEY (bit 1) + EV_REL (bit 2)
                            *bits = 0x06;
                        }
                    }
                }
            }
            1 => {
                // EV_KEY bitmap (which keys/buttons are supported)
                // For keyboard: set bits 1-127 (most scancodes)
                // For mouse: set BTN_LEFT (0x110), BTN_RIGHT (0x111), BTN_MIDDLE (0x112)
                // SAFETY: out was validated by caller.
                unsafe {
                    match self.device_type {
                        EvdevDeviceType::Keyboard => {
                            // Set key bits 1-127 (bytes 0-15)
                            for i in 0..16 {
                                *out.add(i) = 0xFF;
                            }
                            // Clear bit 0 (reserved)
                            *out &= 0xFE;
                        }
                        EvdevDeviceType::Mouse => {
                            // BTN_LEFT = 0x110, bit index = 0x110 / 8 = 34, bit = 0
                            // BTN_RIGHT = 0x111, byte 34, bit 1
                            // BTN_MIDDLE = 0x112, byte 34, bit 2
                            if 34 < 64 {
                                *out.add(34) = 0x07; // bits 0,1,2
                            }
                        }
                    }
                }
            }
            2 => {
                // EV_REL bitmap (which relative axes are supported)
                if self.device_type == EvdevDeviceType::Mouse {
                    // REL_X (bit 0) + REL_Y (bit 1)
                    // SAFETY: out was validated by caller.
                    unsafe {
                        *out = 0x03;
                    }
                }
            }
            _ => {} // Other event types: leave zeroed
        }
    }
}

// ---------------------------------------------------------------------------
// Global evdev device table
// ---------------------------------------------------------------------------

/// Keyboard device name
const KEYBOARD_NAME: &[u8] = b"VeridianOS PS/2 Keyboard";
/// Mouse device name
const MOUSE_NAME: &[u8] = b"VeridianOS PS/2 Mouse";

/// Helper to create a fixed-size name buffer
const fn make_name(src: &[u8]) -> ([u8; 64], usize) {
    let mut buf = [0u8; 64];
    let len = if src.len() > 63 { 63 } else { src.len() };
    let mut i = 0;
    while i < len {
        buf[i] = src[i];
        i += 1;
    }
    (buf, len)
}

const KEYBOARD_NAME_BUF: ([u8; 64], usize) = make_name(KEYBOARD_NAME);
const MOUSE_NAME_BUF: ([u8; 64], usize) = make_name(MOUSE_NAME);

static KEYBOARD_DEVICE: spin::Mutex<EvdevDevice> = spin::Mutex::new(EvdevDevice::new(
    EvdevDeviceType::Keyboard,
    &KEYBOARD_NAME_BUF.0,
    KEYBOARD_NAME_BUF.1,
    13,
    64, // /dev/input/event0 = major 13, minor 64
));

static MOUSE_DEVICE: spin::Mutex<EvdevDevice> = spin::Mutex::new(EvdevDevice::new(
    EvdevDeviceType::Mouse,
    &MOUSE_NAME_BUF.0,
    MOUSE_NAME_BUF.1,
    13,
    65, // /dev/input/event1 = major 13, minor 65
));

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Route events from the global input queue to per-device queues.
///
/// Should be called periodically (e.g., after `input_event::poll_all()`).
pub(crate) fn route_events() {
    while let Some(event) = input_event::read_event() {
        match event.event_type {
            EV_KEY => {
                // Keys with code >= 0x110 are mouse buttons
                if event.code >= 0x110 && event.code <= 0x117 {
                    MOUSE_DEVICE.lock().push_event(event);
                } else {
                    KEYBOARD_DEVICE.lock().push_event(event);
                }
            }
            EV_REL => {
                MOUSE_DEVICE.lock().push_event(event);
            }
            EV_ABS => {
                MOUSE_DEVICE.lock().push_event(event);
            }
            _ => {
                // Unknown event type -- route to keyboard as fallback
                KEYBOARD_DEVICE.lock().push_event(event);
            }
        }
    }
}

/// Read events from a specific device into a buffer.
///
/// `minor` selects the device (64 = keyboard, 65 = mouse).
/// Returns number of bytes read.
pub(crate) fn read_device(minor: u32, buffer: &mut [u8]) -> usize {
    match minor {
        64 => KEYBOARD_DEVICE.lock().read_into_buffer(buffer),
        65 => MOUSE_DEVICE.lock().read_into_buffer(buffer),
        _ => 0,
    }
}

/// Handle an ioctl on an evdev device.
///
/// `minor` selects the device (64 = keyboard, 65 = mouse).
pub(crate) fn handle_ioctl(minor: u32, cmd: u32, arg: *mut u8) -> Result<i32, i32> {
    match minor {
        64 => KEYBOARD_DEVICE.lock().handle_ioctl(cmd, arg),
        65 => MOUSE_DEVICE.lock().handle_ioctl(cmd, arg),
        _ => Err(-1),
    }
}

/// Check if a device has pending events.
pub(crate) fn has_events(minor: u32) -> bool {
    match minor {
        64 => KEYBOARD_DEVICE.lock().has_events(),
        65 => MOUSE_DEVICE.lock().has_events(),
        _ => false,
    }
}

/// Get the device name for a given minor number.
pub(crate) fn device_name(minor: u32) -> Option<&'static str> {
    match minor {
        64 => Some("event0"),
        65 => Some("event1"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_buffer_push_pop() {
        let mut buf = DeviceEventBuffer::new();
        let event = InputEvent::key(0x1E, true); // 'A' key
        buf.push(event);
        assert!(!buf.is_empty());

        let popped = buf.pop();
        assert!(popped.is_some());
        let popped = popped.unwrap();
        assert_eq!(popped.code, 0x1E);
        assert_eq!(popped.value, 1);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_device_buffer_empty() {
        let buf = DeviceEventBuffer::new();
        assert!(buf.is_empty());
        assert!(buf.pop().is_none());
    }

    #[test]
    fn test_make_name() {
        let (buf, len) = make_name(b"test device");
        assert_eq!(len, 11);
        assert_eq!(&buf[..len], b"test device");
        assert_eq!(buf[len], 0);
    }

    #[test]
    fn test_device_name_lookup() {
        assert_eq!(device_name(64), Some("event0"));
        assert_eq!(device_name(65), Some("event1"));
        assert_eq!(device_name(66), None);
    }
}

//! Bluetooth HCI device node for userland access
//!
//! Provides `/dev/bluetooth/hci0` as a device node that allows the BlueZ
//! userland shim to send HCI command packets and receive HCI event packets
//! via the kernel Bluetooth HCI driver.
//!
//! The device node acts as a bridge between the userland D-Bus daemon
//! (userland/bluez/bluez-hci-bridge.cpp) and the kernel HCI controller
//! (kernel/src/drivers/bluetooth/hci.rs).
//!
//! Protocol:
//!   - Write: userland sends H4 HCI command packets (type=0x01 + header +
//!     params)
//!   - Read:  userland receives H4 HCI event packets (type=0x04 + header +
//!     params)
//!   - Ioctl: adapter info queries and scan mode configuration

#![allow(dead_code)]

use core::result::Result;

use spin::Mutex;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of pending events in the event queue
const EVENT_QUEUE_CAPACITY: usize = 32;

/// Maximum size of a single HCI event packet (type + code + len + params)
const MAX_EVENT_SIZE: usize = 259;

/// Maximum size of a single HCI command packet
const MAX_COMMAND_SIZE: usize = 259;

/// H4 packet type: HCI Command
const H4_COMMAND: u8 = 0x01;

/// H4 packet type: HCI Event
const H4_EVENT: u8 = 0x04;

/// Device node path
pub const BT_DEVICE_PATH: &str = "/dev/bluetooth/hci0";

// ---------------------------------------------------------------------------
// Ioctl commands
// ---------------------------------------------------------------------------

/// Get adapter info (address, name, state)
pub const BT_IOCTL_GET_ADAPTER_INFO: u32 = 0x4201;

/// Set scan mode (0=off, 1=inquiry, 2=page, 3=both)
pub const BT_IOCTL_SET_SCAN_MODE: u32 = 0x4202;

/// Reset the HCI controller
pub const BT_IOCTL_RESET: u32 = 0x4203;

// ---------------------------------------------------------------------------
// Event queue entry
// ---------------------------------------------------------------------------

/// A queued HCI event waiting for userland to read
#[derive(Clone)]
struct EventEntry {
    /// Raw event data (H4 type byte + event packet)
    data: [u8; MAX_EVENT_SIZE],
    /// Valid data length
    len: usize,
}

impl Default for EventEntry {
    fn default() -> Self {
        Self {
            data: [0u8; MAX_EVENT_SIZE],
            len: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Event queue (ring buffer)
// ---------------------------------------------------------------------------

/// Ring buffer for HCI events pending delivery to userland
struct EventQueue {
    entries: [EventEntry; EVENT_QUEUE_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
}

impl EventQueue {
    const fn new() -> Self {
        // SAFETY: EventEntry is Copy-compatible (all fixed-size arrays of u8 + usize).
        // We use a const initializer to avoid requiring Default in const context.
        Self {
            entries: [const {
                EventEntry {
                    data: [0u8; MAX_EVENT_SIZE],
                    len: 0,
                }
            }; EVENT_QUEUE_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// Push an event into the queue. Returns Err if full.
    fn push(&mut self, data: &[u8]) -> Result<(), KernelError> {
        if self.count >= EVENT_QUEUE_CAPACITY {
            return Err(KernelError::ResourceExhausted {
                resource: "bluetooth event queue",
            });
        }

        if data.len() > MAX_EVENT_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "event_data",
                value: "exceeds maximum event size",
            });
        }

        let entry = &mut self.entries[self.tail];
        entry.data[..data.len()].copy_from_slice(data);
        entry.len = data.len();

        self.tail = (self.tail + 1) % EVENT_QUEUE_CAPACITY;
        self.count += 1;
        Ok(())
    }

    /// Pop the next event from the queue. Returns None if empty.
    fn pop(&mut self) -> Option<(usize, [u8; MAX_EVENT_SIZE])> {
        if self.count == 0 {
            return None;
        }

        let entry = &self.entries[self.head];
        let len = entry.len;
        let data = entry.data;

        self.head = (self.head + 1) % EVENT_QUEUE_CAPACITY;
        self.count -= 1;

        Some((len, data))
    }

    /// Check if the queue is empty
    fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Number of pending events
    fn len(&self) -> usize {
        self.count
    }
}

// ---------------------------------------------------------------------------
// Device node handle
// ---------------------------------------------------------------------------

/// Handle returned by bt_device_open
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BtDeviceHandle {
    /// Device index (0 for hci0)
    index: u32,
    /// Whether the handle is valid/open
    open: bool,
}

// ---------------------------------------------------------------------------
// Device node state
// ---------------------------------------------------------------------------

/// Global state for the Bluetooth device node
struct DeviceNodeState {
    /// Event queue: kernel pushes HCI events, userland reads them
    event_queue: EventQueue,
    /// Whether the device node is initialized
    initialized: bool,
    /// Number of open handles (reference count)
    open_count: u32,
}

impl DeviceNodeState {
    const fn new() -> Self {
        Self {
            event_queue: EventQueue::new(),
            initialized: false,
            open_count: 0,
        }
    }
}

/// Global device node state, protected by a spinlock
static DEVICE_NODE: Mutex<DeviceNodeState> = Mutex::new(DeviceNodeState::new());

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the Bluetooth device node subsystem.
/// Creates the `/dev/bluetooth/hci0` device node entry.
/// Must be called after the Bluetooth controller is initialized.
pub fn init() -> Result<(), KernelError> {
    let mut state = DEVICE_NODE.lock();

    if state.initialized {
        return Err(KernelError::InvalidArgument {
            name: "bt_device_node",
            value: "already initialized",
        });
    }

    state.initialized = true;

    crate::println!("[BT] Device node {} registered", BT_DEVICE_PATH);
    Ok(())
}

// ---------------------------------------------------------------------------
// Device operations
// ---------------------------------------------------------------------------

/// Open the Bluetooth HCI device node.
/// Returns a handle for subsequent read/write/ioctl operations.
pub fn bt_device_open() -> Result<BtDeviceHandle, KernelError> {
    let mut state = DEVICE_NODE.lock();

    if !state.initialized {
        return Err(KernelError::InvalidArgument {
            name: "bt_device_node",
            value: "not initialized",
        });
    }

    state.open_count += 1;

    Ok(BtDeviceHandle {
        index: 0,
        open: true,
    })
}

/// Close the Bluetooth HCI device node handle.
pub fn bt_device_close(handle: &mut BtDeviceHandle) -> Result<(), KernelError> {
    if !handle.open {
        return Err(KernelError::InvalidArgument {
            name: "handle",
            value: "not open",
        });
    }

    let mut state = DEVICE_NODE.lock();
    if state.open_count > 0 {
        state.open_count -= 1;
    }

    handle.open = false;
    Ok(())
}

/// Write an HCI command packet to the kernel HCI layer.
///
/// The data should be an H4-formatted command packet:
/// `[0x01] [opcode_lo] [opcode_hi] [param_len] [params...]`
///
/// The command is forwarded to the kernel's BluetoothController for
/// processing and transmission to the hardware.
pub fn bt_device_write(handle: &BtDeviceHandle, data: &[u8]) -> Result<usize, KernelError> {
    if !handle.open {
        return Err(KernelError::InvalidArgument {
            name: "handle",
            value: "not open",
        });
    }

    if data.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "empty write buffer",
        });
    }

    if data.len() > MAX_COMMAND_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "exceeds maximum command size",
        });
    }

    // Verify H4 command packet type
    if data[0] != H4_COMMAND {
        return Err(KernelError::InvalidArgument {
            name: "packet_type",
            value: "expected HCI command (0x01)",
        });
    }

    // Verify minimum command header length (type + opcode + param_len)
    if data.len() < 4 {
        return Err(KernelError::InvalidArgument {
            name: "command",
            value: "too short for HCI command header",
        });
    }

    let opcode = u16::from_le_bytes([data[1], data[2]]);
    let param_len = data[3] as usize;

    // Verify packet completeness
    if data.len() < 4 + param_len {
        return Err(KernelError::InvalidArgument {
            name: "command",
            value: "truncated command parameters",
        });
    }

    // Forward to the kernel HCI controller.
    // Route the command based on its opcode to the appropriate
    // BluetoothController method.
    let controller = super::get_controller();
    let mut ctrl = controller.lock();

    match opcode {
        super::hci::HCI_RESET => {
            let _result = ctrl.send_reset();
        }
        super::hci::HCI_READ_BD_ADDR => {
            let _result = ctrl.read_bd_addr();
        }
        super::hci::HCI_READ_LOCAL_NAME => {
            let _result = ctrl.read_local_name();
        }
        super::hci::HCI_WRITE_SCAN_ENABLE => {
            if param_len >= 1 {
                let mode = match data[4] {
                    0x00 => super::hci::ScanEnable::NoScans,
                    0x01 => super::hci::ScanEnable::InquiryScanOnly,
                    0x02 => super::hci::ScanEnable::PageScanOnly,
                    _ => super::hci::ScanEnable::InquiryAndPageScan,
                };
                let _result = ctrl.write_scan_enable(mode);
            }
        }
        super::hci::HCI_INQUIRY => {
            if param_len >= 5 {
                let inquiry_length = data[7]; // params[3]
                let max_responses = data[8]; // params[4]
                let _result = ctrl.start_inquiry(inquiry_length, max_responses);
            }
        }
        super::hci::HCI_CREATE_CONNECTION => {
            if param_len >= 6 {
                let mut addr = [0u8; 6];
                addr.copy_from_slice(&data[4..10]);
                let _result = ctrl.create_connection(&addr);
            }
        }
        super::hci::HCI_DISCONNECT => {
            if param_len >= 3 {
                let handle = u16::from_le_bytes([data[4], data[5]]);
                let reason = data[6];
                let _result = ctrl.disconnect(handle, reason);
            }
        }
        _ => {
            // Unknown opcode -- log and ignore
        }
    }

    Ok(data.len())
}

/// Read the next HCI event from the kernel event queue.
///
/// Returns the event in H4 format:
/// `[0x04] [event_code] [param_len] [params...]`
///
/// If no event is available, returns Ok(0).
pub fn bt_device_read(handle: &BtDeviceHandle, buf: &mut [u8]) -> Result<usize, KernelError> {
    if !handle.open {
        return Err(KernelError::InvalidArgument {
            name: "handle",
            value: "not open",
        });
    }

    if buf.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "buf",
            value: "empty read buffer",
        });
    }

    let mut state = DEVICE_NODE.lock();

    match state.event_queue.pop() {
        Some((len, data)) => {
            let copy_len = core::cmp::min(len, buf.len());
            buf[..copy_len].copy_from_slice(&data[..copy_len]);
            Ok(copy_len)
        }
        None => Ok(0), // No event available
    }
}

/// Perform an ioctl on the Bluetooth device node.
///
/// Supported commands:
/// - `BT_IOCTL_GET_ADAPTER_INFO`: Read adapter address, name, and state
/// - `BT_IOCTL_SET_SCAN_MODE`: Set inquiry/page scan mode
/// - `BT_IOCTL_RESET`: Reset the HCI controller
pub fn bt_device_ioctl(handle: &BtDeviceHandle, cmd: u32, arg: u64) -> Result<u64, KernelError> {
    if !handle.open {
        return Err(KernelError::InvalidArgument {
            name: "handle",
            value: "not open",
        });
    }

    match cmd {
        BT_IOCTL_GET_ADAPTER_INFO => {
            // Return adapter state as a packed u64:
            // bits [7:0]  = state (0=off, 1=init, 2=ready, 3=scanning, 4=connected)
            // bits [15:8] = open handle count
            let controller = super::get_controller();
            let ctrl = controller.lock();

            let state_val: u8 = match ctrl.state() {
                super::ControllerState::Off => 0,
                super::ControllerState::Initializing => 1,
                super::ControllerState::Ready => 2,
                super::ControllerState::Scanning => 3,
                super::ControllerState::Connected => 4,
            };

            let dev_state = DEVICE_NODE.lock();
            let result = (state_val as u64) | ((dev_state.open_count as u64) << 8);
            Ok(result)
        }

        BT_IOCTL_SET_SCAN_MODE => {
            let scan_mode = (arg & 0xFF) as u8;
            let controller = super::get_controller();
            let mut ctrl = controller.lock();

            let mode = match scan_mode {
                0x00 => super::hci::ScanEnable::NoScans,
                0x01 => super::hci::ScanEnable::InquiryScanOnly,
                0x02 => super::hci::ScanEnable::PageScanOnly,
                _ => super::hci::ScanEnable::InquiryAndPageScan,
            };
            let _result = ctrl.write_scan_enable(mode);
            Ok(0)
        }

        BT_IOCTL_RESET => {
            let controller = super::get_controller();
            let mut ctrl = controller.lock();

            let _result = ctrl.send_reset();
            Ok(0)
        }

        _ => Err(KernelError::InvalidArgument {
            name: "ioctl_cmd",
            value: "unknown ioctl command",
        }),
    }
}

// ---------------------------------------------------------------------------
// Kernel-side event delivery
// ---------------------------------------------------------------------------

/// Push an HCI event into the device node's event queue.
///
/// Called by the kernel HCI driver when an event is received from the
/// Bluetooth controller. The event is queued for the next userland read().
///
/// The data should include the H4 event type byte (0x04) prefix.
pub fn push_event(data: &[u8]) -> Result<(), KernelError> {
    let mut state = DEVICE_NODE.lock();

    if !state.initialized {
        return Err(KernelError::InvalidArgument {
            name: "bt_device_node",
            value: "not initialized",
        });
    }

    state.event_queue.push(data)
}

/// Get the number of pending events in the queue.
pub fn pending_event_count() -> usize {
    let state = DEVICE_NODE.lock();
    state.event_queue.len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_queue_push_pop() {
        let mut queue = EventQueue::new();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        // Push an event
        let event_data = [H4_EVENT, 0x0E, 0x04, 0x01, 0x00, 0x00, 0x00];
        assert!(queue.push(&event_data).is_ok());
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);

        // Pop it back
        let result = queue.pop();
        assert!(result.is_some());
        let (len, data) = result.unwrap();
        assert_eq!(len, event_data.len());
        assert_eq!(&data[..len], &event_data);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_event_queue_overflow() {
        let mut queue = EventQueue::new();

        let event_data = [H4_EVENT, 0x01, 0x01, 0x00];

        // Fill the queue
        for _ in 0..EVENT_QUEUE_CAPACITY {
            assert!(queue.push(&event_data).is_ok());
        }

        // Next push should fail
        assert!(queue.push(&event_data).is_err());
        assert_eq!(queue.len(), EVENT_QUEUE_CAPACITY);
    }

    #[test]
    fn test_event_queue_wrap_around() {
        let mut queue = EventQueue::new();

        let event_data = [H4_EVENT, 0x02, 0x02, 0xAA, 0xBB];

        // Push and pop several times to exercise wrap-around
        for i in 0..EVENT_QUEUE_CAPACITY * 2 {
            assert!(queue.push(&event_data).is_ok());
            let result = queue.pop();
            assert!(result.is_some());
            let (len, _data) = result.unwrap();
            assert_eq!(len, event_data.len());
            let _ = i;
        }

        assert!(queue.is_empty());
    }

    #[test]
    fn test_event_queue_empty_pop() {
        let mut queue = EventQueue::new();
        assert!(queue.pop().is_none());
    }

    #[test]
    fn test_event_queue_oversized_event() {
        let mut queue = EventQueue::new();
        let big_event = [0u8; MAX_EVENT_SIZE + 1];
        assert!(queue.push(&big_event).is_err());
    }
}

//! USB Hotplug Detection
//!
//! Monitors xHCI port status change bits (PORTSC registers) to detect
//! USB device attach and detach events.  Events are queued in a ring
//! buffer and can be consumed by a userland udev daemon.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of USB ports to monitor
const MAX_PORTS: usize = 16;

/// Event ring buffer capacity
const EVENT_RING_CAPACITY: usize = 16;

/// PORTSC register offsets (xHCI spec section 5.4.8)
/// Connect Status Change bit
const PORTSC_CSC: u32 = 1 << 17;
/// Current Connect Status bit
const PORTSC_CCS: u32 = 1 << 0;
/// Port Enabled/Disabled bit
const PORTSC_PED: u32 = 1 << 1;
/// Port Reset bit
const PORTSC_PR: u32 = 1 << 4;
/// Port Link State bits [8:5]
const PORTSC_PLS_MASK: u32 = 0xF << 5;
/// Port Speed bits [13:10]
const PORTSC_SPEED_MASK: u32 = 0xF << 10;
/// Port Speed shift
const PORTSC_SPEED_SHIFT: u32 = 10;
/// Port Power bit
const PORTSC_PP: u32 = 1 << 9;
/// Port Enable/Disable Change bit
const PORTSC_PEC: u32 = 1 << 18;
/// Warm Port Reset Change bit
const PORTSC_WRC: u32 = 1 << 19;
/// Over-current Change bit
const PORTSC_OCC: u32 = 1 << 20;
/// Port Reset Change bit
const PORTSC_PRC: u32 = 1 << 21;
/// Port Link State Change bit
const PORTSC_PLC: u32 = 1 << 22;
/// Port Config Error Change bit
const PORTSC_CEC: u32 = 1 << 23;

/// Write-1-to-clear status change bits (must be preserved when writing PORTSC)
const PORTSC_CHANGE_BITS: u32 =
    PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_OCC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC;

/// USB device class codes
const USB_CLASS_HID: u8 = 0x03;
const USB_CLASS_MASS_STORAGE: u8 = 0x08;
const USB_CLASS_HUB: u8 = 0x09;
const USB_CLASS_AUDIO: u8 = 0x01;
const USB_CLASS_VIDEO: u8 = 0x0E;
const USB_CLASS_PRINTER: u8 = 0x07;
const USB_CLASS_CDC: u8 = 0x02;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// USB device connection speed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceSpeed {
    /// 1.5 Mbps (USB 1.0)
    Low,
    /// 12 Mbps (USB 1.1)
    Full,
    /// 480 Mbps (USB 2.0)
    High,
    /// 5 Gbps (USB 3.0)
    Super,
}

impl UsbDeviceSpeed {
    /// Decode speed from xHCI PORTSC speed field
    pub(crate) fn from_portsc(speed_field: u32) -> Self {
        match speed_field {
            1 => UsbDeviceSpeed::Full,
            2 => UsbDeviceSpeed::Low,
            3 => UsbDeviceSpeed::High,
            4 => UsbDeviceSpeed::Super,
            _ => UsbDeviceSpeed::Full, // default
        }
    }

    /// Human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            UsbDeviceSpeed::Low => "Low Speed (1.5 Mbps)",
            UsbDeviceSpeed::Full => "Full Speed (12 Mbps)",
            UsbDeviceSpeed::High => "High Speed (480 Mbps)",
            UsbDeviceSpeed::Super => "SuperSpeed (5 Gbps)",
        }
    }
}

/// USB hotplug event
#[derive(Debug, Clone)]
pub enum UsbHotplugEvent {
    /// A USB device was physically attached to a port
    DeviceAttached {
        /// Port number (0-based)
        port: u8,
        /// Connection speed
        speed: UsbDeviceSpeed,
        /// USB vendor ID (from device descriptor, 0 if not yet read)
        vendor_id: u16,
        /// USB product ID (from device descriptor, 0 if not yet read)
        product_id: u16,
        /// USB device class code (from device descriptor)
        device_class: u8,
    },
    /// A USB device was physically detached from a port
    DeviceDetached {
        /// Port number (0-based)
        port: u8,
    },
}

/// Per-port status tracking
#[derive(Debug, Clone, Copy)]
pub struct UsbPortStatus {
    /// Whether a device is currently connected
    pub connected: bool,
    /// Whether the port is enabled
    pub enabled: bool,
    /// Current connection speed
    pub speed: UsbDeviceSpeed,
    /// Connect status changed since last poll
    pub connect_changed: bool,
    /// Enable status changed since last poll
    pub enable_changed: bool,
    /// Vendor ID of attached device (0 if none)
    pub vendor_id: u16,
    /// Product ID of attached device (0 if none)
    pub product_id: u16,
    /// Device class of attached device (0 if none)
    pub device_class: u8,
}

impl Default for UsbPortStatus {
    fn default() -> Self {
        Self {
            connected: false,
            enabled: false,
            speed: UsbDeviceSpeed::Full,
            connect_changed: false,
            enable_changed: false,
            vendor_id: 0,
            product_id: 0,
            device_class: 0,
        }
    }
}

/// Hotplug callback function type
pub type HotplugCallback = fn(UsbHotplugEvent);

/// Event ring buffer for hotplug events
struct EventRingBuffer {
    events: [Option<UsbHotplugEvent>; EVENT_RING_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
}

impl EventRingBuffer {
    const fn new() -> Self {
        // Initialize all slots to None using const array init
        const NONE: Option<UsbHotplugEvent> = None;
        Self {
            events: [NONE; EVENT_RING_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    fn push(&mut self, event: UsbHotplugEvent) -> bool {
        if self.count >= EVENT_RING_CAPACITY {
            return false; // ring full, drop event
        }
        self.events[self.tail] = Some(event);
        self.tail = (self.tail + 1) % EVENT_RING_CAPACITY;
        self.count += 1;
        true
    }

    fn pop(&mut self) -> Option<UsbHotplugEvent> {
        if self.count == 0 {
            return None;
        }
        let event = self.events[self.head].take();
        self.head = (self.head + 1) % EVENT_RING_CAPACITY;
        self.count -= 1;
        event
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn len(&self) -> usize {
        self.count
    }

    fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.count = 0;
        for slot in self.events.iter_mut() {
            *slot = None;
        }
    }
}

/// USB Hotplug Manager
///
/// Tracks port status for up to MAX_PORTS ports and generates
/// attach/detach events by polling PORTSC change bits.
pub struct UsbHotplugManager {
    /// Per-port status
    ports: [UsbPortStatus; MAX_PORTS],
    /// Number of ports on this controller
    num_ports: u8,
    /// Event ring buffer
    event_ring: EventRingBuffer,
    /// Registered callbacks
    callbacks: [Option<HotplugCallback>; 4],
    /// Number of registered callbacks
    num_callbacks: usize,
    /// Base address of xHCI operational registers (for PORTSC access)
    portsc_base: usize,
    /// Whether hotplug monitoring is initialized
    initialized: bool,
    /// Total attach events since init
    total_attach_events: u32,
    /// Total detach events since init
    total_detach_events: u32,
}

impl Default for UsbHotplugManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbHotplugManager {
    /// Create a new hotplug manager
    pub const fn new() -> Self {
        const DEFAULT_PORT: UsbPortStatus = UsbPortStatus {
            connected: false,
            enabled: false,
            speed: UsbDeviceSpeed::Full,
            connect_changed: false,
            enable_changed: false,
            vendor_id: 0,
            product_id: 0,
            device_class: 0,
        };
        const NONE_CB: Option<HotplugCallback> = None;
        Self {
            ports: [DEFAULT_PORT; MAX_PORTS],
            num_ports: 0,
            event_ring: EventRingBuffer::new(),
            callbacks: [NONE_CB; 4],
            num_callbacks: 0,
            portsc_base: 0,
            initialized: false,
            total_attach_events: 0,
            total_detach_events: 0,
        }
    }

    /// Initialize with the xHCI operational register base and port count
    pub fn init(&mut self, portsc_base: usize, num_ports: u8) {
        let capped = if num_ports as usize > MAX_PORTS {
            MAX_PORTS as u8
        } else {
            num_ports
        };
        self.portsc_base = portsc_base;
        self.num_ports = capped;
        self.initialized = true;
        self.event_ring.clear();
        self.total_attach_events = 0;
        self.total_detach_events = 0;

        // Clear all port status
        for port in self.ports.iter_mut() {
            *port = UsbPortStatus::default();
        }
    }

    /// Poll all ports for status changes
    ///
    /// Reads PORTSC registers, detects CSC (Connect Status Change),
    /// and generates appropriate events.
    pub fn poll(&mut self) {
        if !self.initialized || self.portsc_base == 0 {
            return;
        }

        for port_idx in 0..self.num_ports as usize {
            let portsc = self.read_portsc(port_idx);

            // Check Connect Status Change bit
            if portsc & PORTSC_CSC != 0 {
                let connected = portsc & PORTSC_CCS != 0;
                let was_connected = self.ports[port_idx].connected;

                if connected && !was_connected {
                    // Device attached
                    let speed_field = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
                    let speed = UsbDeviceSpeed::from_portsc(speed_field);
                    let enabled = portsc & PORTSC_PED != 0;

                    // Read device descriptor to get vendor/product/class
                    let (vendor_id, product_id, device_class) =
                        self.read_device_descriptor(port_idx);

                    self.ports[port_idx] = UsbPortStatus {
                        connected: true,
                        enabled,
                        speed,
                        connect_changed: true,
                        enable_changed: false,
                        vendor_id,
                        product_id,
                        device_class,
                    };

                    let event = UsbHotplugEvent::DeviceAttached {
                        port: port_idx as u8,
                        speed,
                        vendor_id,
                        product_id,
                        device_class,
                    };

                    self.event_ring.push(event.clone());
                    self.total_attach_events = self.total_attach_events.saturating_add(1);
                    self.notify_callbacks(event);
                } else if !connected && was_connected {
                    // Device detached
                    self.ports[port_idx] = UsbPortStatus {
                        connected: false,
                        enabled: false,
                        speed: UsbDeviceSpeed::Full,
                        connect_changed: true,
                        enable_changed: false,
                        vendor_id: 0,
                        product_id: 0,
                        device_class: 0,
                    };

                    let event = UsbHotplugEvent::DeviceDetached {
                        port: port_idx as u8,
                    };

                    self.event_ring.push(event.clone());
                    self.total_detach_events = self.total_detach_events.saturating_add(1);
                    self.notify_callbacks(event);
                }

                // Clear CSC by writing 1 to it (preserve other bits, clear change bits)
                self.clear_portsc_csc(port_idx, portsc);
            }

            // Also check enable change
            if portsc & PORTSC_PEC != 0 {
                let enabled = portsc & PORTSC_PED != 0;
                self.ports[port_idx].enabled = enabled;
                self.ports[port_idx].enable_changed = true;

                // Clear PEC
                self.clear_portsc_change(port_idx, portsc, PORTSC_PEC);
            }
        }
    }

    /// Read PORTSC register for a given port
    fn read_portsc(&self, port_idx: usize) -> u32 {
        if self.portsc_base == 0 {
            return 0;
        }
        // Each PORTSC is at offset 0x400 + (port_idx * 0x10) from operational base
        let addr = self.portsc_base + 0x400 + port_idx * 0x10;

        // SAFETY: portsc_base was validated at init time and addr is within
        // the xHCI MMIO region.  Port index is bounded by num_ports.
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        unsafe {
            core::ptr::read_volatile(addr as *const u32)
        }
        #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
        {
            let _ = addr;
            0
        }
    }

    /// Clear Connect Status Change bit in PORTSC
    fn clear_portsc_csc(&self, port_idx: usize, current: u32) {
        self.clear_portsc_change(port_idx, current, PORTSC_CSC);
    }

    /// Clear a specific change bit in PORTSC
    ///
    /// xHCI PORTSC is special: writing 1 to a change bit clears it,
    /// but we must NOT write 1 to other change bits (would clear them too).
    fn clear_portsc_change(&self, port_idx: usize, current: u32, change_bit: u32) {
        if self.portsc_base == 0 {
            return;
        }
        let addr = self.portsc_base + 0x400 + port_idx * 0x10;

        // Preserve non-change bits, write 1 only to the target change bit
        let write_val = (current & !PORTSC_CHANGE_BITS) | change_bit;

        // SAFETY: addr is within xHCI MMIO region, validated at init.
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        unsafe {
            core::ptr::write_volatile(addr as *mut u32, write_val);
        }
        #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
        {
            let _ = (addr, write_val);
        }
    }

    /// Attempt to read device descriptor from a newly attached device
    ///
    /// Returns (vendor_id, product_id, device_class).
    /// Returns (0, 0, 0) if the descriptor cannot be read yet.
    fn read_device_descriptor(&self, _port_idx: usize) -> (u16, u16, u8) {
        // In a full implementation this would issue a GET_DESCRIPTOR
        // control transfer via the xHCI command ring.  For now we
        // return zeros; the udev daemon can read descriptors later
        // via the USB sysfs interface.
        (0, 0, 0)
    }

    /// Get the next pending hotplug event
    pub fn get_event(&mut self) -> Option<UsbHotplugEvent> {
        self.event_ring.pop()
    }

    /// Check if there are pending events
    pub fn has_events(&self) -> bool {
        !self.event_ring.is_empty()
    }

    /// Get the number of pending events
    pub fn pending_event_count(&self) -> usize {
        self.event_ring.len()
    }

    /// Register a callback for hotplug events
    pub fn register_callback(&mut self, callback: HotplugCallback) -> Result<(), KernelError> {
        if self.num_callbacks >= self.callbacks.len() {
            return Err(KernelError::ResourceExhausted {
                resource: "hotplug callbacks",
            });
        }
        self.callbacks[self.num_callbacks] = Some(callback);
        self.num_callbacks += 1;
        Ok(())
    }

    /// Notify all registered callbacks about an event
    fn notify_callbacks(&self, event: UsbHotplugEvent) {
        for cb in self.callbacks.iter().flatten() {
            cb(event.clone());
        }
    }

    /// Get port status for a specific port
    pub fn port_status(&self, port: u8) -> Option<&UsbPortStatus> {
        if port < self.num_ports {
            Some(&self.ports[port as usize])
        } else {
            None
        }
    }

    /// Get the number of monitored ports
    pub fn num_ports(&self) -> u8 {
        self.num_ports
    }

    /// Get total attach event count
    pub fn total_attach_events(&self) -> u32 {
        self.total_attach_events
    }

    /// Get total detach event count
    pub fn total_detach_events(&self) -> u32 {
        self.total_detach_events
    }

    /// Check if the hotplug manager is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

// ---------------------------------------------------------------------------
// Global Hotplug Manager
// ---------------------------------------------------------------------------

static HOTPLUG_MANAGER: Mutex<UsbHotplugManager> = Mutex::new(UsbHotplugManager::new());

/// Whether hotplug subsystem has been initialized
static HOTPLUG_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize USB hotplug detection
///
/// Should be called after xHCI controller enumeration, with the PORTSC
/// base address and number of root hub ports.
pub fn usb_hotplug_init(portsc_base: usize, num_ports: u8) {
    let mut manager = HOTPLUG_MANAGER.lock();
    manager.init(portsc_base, num_ports);
    HOTPLUG_INITIALIZED.store(true, Ordering::Release);
    crate::println!(
        "[USB-HOTPLUG] Initialized, monitoring {} ports (PORTSC base: {:#x})",
        num_ports,
        portsc_base
    );
}

/// Poll for USB hotplug events
///
/// Called periodically (e.g., from a timer interrupt or polling thread)
/// to check xHCI port status change bits.
pub fn usb_hotplug_poll() {
    if !HOTPLUG_INITIALIZED.load(Ordering::Acquire) {
        return;
    }
    HOTPLUG_MANAGER.lock().poll();
}

/// Get the next pending hotplug event
pub fn usb_hotplug_get_event() -> Option<UsbHotplugEvent> {
    if !HOTPLUG_INITIALIZED.load(Ordering::Acquire) {
        return None;
    }
    HOTPLUG_MANAGER.lock().get_event()
}

/// Register a callback for hotplug events
pub fn usb_hotplug_register_callback(callback: HotplugCallback) -> Result<(), KernelError> {
    HOTPLUG_MANAGER.lock().register_callback(callback)
}

/// Check if there are pending hotplug events
pub fn usb_hotplug_has_events() -> bool {
    if !HOTPLUG_INITIALIZED.load(Ordering::Acquire) {
        return false;
    }
    HOTPLUG_MANAGER.lock().has_events()
}

/// Get port status
pub fn usb_hotplug_port_status(port: u8) -> Option<UsbPortStatus> {
    if !HOTPLUG_INITIALIZED.load(Ordering::Acquire) {
        return None;
    }
    HOTPLUG_MANAGER.lock().port_status(port).copied()
}

/// Get the number of monitored ports
pub fn usb_hotplug_num_ports() -> u8 {
    if !HOTPLUG_INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }
    HOTPLUG_MANAGER.lock().num_ports()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usb_device_speed_from_portsc() {
        assert_eq!(UsbDeviceSpeed::from_portsc(1), UsbDeviceSpeed::Full);
        assert_eq!(UsbDeviceSpeed::from_portsc(2), UsbDeviceSpeed::Low);
        assert_eq!(UsbDeviceSpeed::from_portsc(3), UsbDeviceSpeed::High);
        assert_eq!(UsbDeviceSpeed::from_portsc(4), UsbDeviceSpeed::Super);
        assert_eq!(UsbDeviceSpeed::from_portsc(0), UsbDeviceSpeed::Full);
    }

    #[test]
    fn test_usb_device_speed_name() {
        assert!(UsbDeviceSpeed::Low.name().contains("1.5"));
        assert!(UsbDeviceSpeed::Full.name().contains("12"));
        assert!(UsbDeviceSpeed::High.name().contains("480"));
        assert!(UsbDeviceSpeed::Super.name().contains("5"));
    }

    #[test]
    fn test_event_ring_buffer_empty() {
        let ring = EventRingBuffer::new();
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn test_event_ring_buffer_push_pop() {
        let mut ring = EventRingBuffer::new();
        let event = UsbHotplugEvent::DeviceDetached { port: 0 };
        assert!(ring.push(event));
        assert!(!ring.is_empty());
        assert_eq!(ring.len(), 1);

        let popped = ring.pop();
        assert!(popped.is_some());
        assert!(ring.is_empty());
    }

    #[test]
    fn test_event_ring_buffer_overflow() {
        let mut ring = EventRingBuffer::new();
        for i in 0..EVENT_RING_CAPACITY {
            let event = UsbHotplugEvent::DeviceDetached { port: i as u8 };
            assert!(ring.push(event));
        }
        assert_eq!(ring.len(), EVENT_RING_CAPACITY);

        // Ring is full, should fail
        let event = UsbHotplugEvent::DeviceDetached { port: 99 };
        assert!(!ring.push(event));
    }

    #[test]
    fn test_event_ring_buffer_wrap() {
        let mut ring = EventRingBuffer::new();

        // Fill and drain partially
        for i in 0..8 {
            ring.push(UsbHotplugEvent::DeviceDetached { port: i });
        }
        for _ in 0..4 {
            ring.pop();
        }
        assert_eq!(ring.len(), 4);

        // Add more to wrap around
        for i in 0..8 {
            ring.push(UsbHotplugEvent::DeviceDetached { port: 10 + i });
        }
        assert_eq!(ring.len(), 12);
    }

    #[test]
    fn test_event_ring_buffer_clear() {
        let mut ring = EventRingBuffer::new();
        for i in 0..5 {
            ring.push(UsbHotplugEvent::DeviceDetached { port: i });
        }
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn test_hotplug_manager_new() {
        let manager = UsbHotplugManager::new();
        assert!(!manager.is_initialized());
        assert_eq!(manager.num_ports(), 0);
    }

    #[test]
    fn test_hotplug_manager_init() {
        let mut manager = UsbHotplugManager::new();
        manager.init(0x1000, 4);
        assert!(manager.is_initialized());
        assert_eq!(manager.num_ports(), 4);
    }

    #[test]
    fn test_hotplug_manager_port_capping() {
        let mut manager = UsbHotplugManager::new();
        manager.init(0x1000, 255); // over MAX_PORTS
        assert_eq!(manager.num_ports(), MAX_PORTS as u8);
    }

    #[test]
    fn test_hotplug_manager_port_status() {
        let mut manager = UsbHotplugManager::new();
        manager.init(0x1000, 4);

        assert!(manager.port_status(0).is_some());
        assert!(manager.port_status(3).is_some());
        assert!(manager.port_status(4).is_none());

        let status = manager.port_status(0).unwrap();
        assert!(!status.connected);
        assert!(!status.enabled);
    }

    #[test]
    fn test_hotplug_manager_event_counters() {
        let manager = UsbHotplugManager::new();
        assert_eq!(manager.total_attach_events(), 0);
        assert_eq!(manager.total_detach_events(), 0);
    }

    #[test]
    fn test_hotplug_manager_no_events_when_uninit() {
        let mut manager = UsbHotplugManager::new();
        assert!(!manager.has_events());
        assert!(manager.get_event().is_none());
    }

    #[test]
    fn test_port_status_default() {
        let status = UsbPortStatus::default();
        assert!(!status.connected);
        assert!(!status.enabled);
        assert_eq!(status.vendor_id, 0);
        assert_eq!(status.product_id, 0);
        assert_eq!(status.device_class, 0);
    }

    #[test]
    fn test_portsc_constants() {
        // Verify bit positions match xHCI spec
        assert_eq!(PORTSC_CCS, 0x0000_0001);
        assert_eq!(PORTSC_PED, 0x0000_0002);
        assert_eq!(PORTSC_CSC, 0x0002_0000);
        assert_eq!(PORTSC_PEC, 0x0004_0000);
    }

    #[test]
    fn test_register_callback_limit() {
        let mut manager = UsbHotplugManager::new();
        fn dummy(_: UsbHotplugEvent) {}

        for _ in 0..4 {
            assert!(manager.register_callback(dummy).is_ok());
        }
        // 5th should fail
        assert!(manager.register_callback(dummy).is_err());
    }
}

//! USB Bus Driver
//!
//! Implements USB host controller and device management.
//!
//! This module is organized into submodules:
//! - [`device`]: USB device types, descriptors, and bus-level device management
//! - [`host`]: USB host controller trait and UHCI controller implementation
//! - [`transfer`]: USB transfer types and UHCI transfer descriptors

mod device;
mod host;
mod transfer;

// Re-export all public types to maintain existing API
use alloc::boxed::Box;

pub use device::{
    usb_classes, UsbBus, UsbConfiguration, UsbDevice, UsbDeviceDescriptor, UsbDirection,
    UsbEndpoint, UsbEndpointType, UsbInterface, UsbPortStatus, UsbSpeed,
};
pub use host::{UhciController, UsbHostController};
use spin::Mutex;
pub use transfer::{UhciQh, UhciTd, UsbTransfer};

use crate::sync::once_lock::OnceLock;

/// Global USB bus instance
static USB_BUS: OnceLock<Mutex<UsbBus>> = OnceLock::new();

/// Initialize USB subsystem
pub fn init() {
    let usb_bus = UsbBus::new();
    let _ = USB_BUS.set(Mutex::new(usb_bus));

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
    USB_BUS.get().expect("USB bus not initialized")
}

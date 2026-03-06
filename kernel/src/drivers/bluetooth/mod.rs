//! Bluetooth driver subsystem
//!
//! Provides HCI (Host Controller Interface) transport, command/event protocol,
//! L2CAP basics, and SDP service discovery stubs.

#![allow(dead_code)]

pub mod hci;

// Phase 8 Wave 2: Bluetooth L2CAP/RFCOMM/Profiles
pub mod l2cap;
pub mod profiles;
pub mod rfcomm;

pub use hci::{BluetoothController, ControllerState};
use spin::Mutex;

use crate::sync::once_lock::OnceLock;

/// Global Bluetooth controller instance
static BT_CONTROLLER: OnceLock<Mutex<BluetoothController>> = OnceLock::new();

/// Initialize the Bluetooth subsystem
pub fn init() {
    let controller = BluetoothController::new();
    let _ = BT_CONTROLLER.set(Mutex::new(controller));
    crate::println!("[BT] Bluetooth subsystem initialized");
}

/// Access the global Bluetooth controller
pub fn get_controller() -> &'static Mutex<BluetoothController> {
    BT_CONTROLLER
        .get()
        .expect("Bluetooth controller not initialized")
}

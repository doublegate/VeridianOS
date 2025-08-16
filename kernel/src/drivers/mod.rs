//! Device drivers module
//!
//! Contains all device drivers including bus drivers, network drivers, and device-specific drivers.

pub mod pci;
pub mod usb;
pub mod network;
pub mod console;
pub mod storage;

pub use pci::{PciBus, PciDevice};
pub use usb::{UsbBus, UsbDevice};
pub use network::{NetworkDevice, EthernetDriver, LoopbackDriver};
pub use console::{ConsoleDevice, ConsoleDriver, VgaConsole, SerialConsole};
pub use storage::{StorageDevice, AtaDriver};

/// Initialize all drivers
pub fn init() {
    crate::println!("[DRIVERS] Initializing device drivers...");
    
    // Initialize bus drivers
    pci::init();
    usb::init();
    
    // Initialize device drivers
    network::init();
    console::init();
    storage::init();
    
    crate::println!("[DRIVERS] Device drivers initialized");
}
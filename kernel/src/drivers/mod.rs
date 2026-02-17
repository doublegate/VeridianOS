//! Device drivers module
//!
//! Contains all device drivers including bus drivers, network drivers, and
//! device-specific drivers.

pub mod console;
pub mod e1000;
pub mod gpu;
pub mod input;
pub mod keyboard;
pub mod network;
pub mod nvme;
pub mod pci;
pub mod ramfb;
pub mod storage;
pub mod usb;
pub mod virtio;
pub mod virtio_net;

pub use console::{ConsoleDevice, ConsoleDriver, SerialConsole, VgaConsole};
pub use gpu::{GpuDriver, PixelFormat};
pub use network::{EthernetDriver, LoopbackDriver, NetworkDevice};
pub use pci::{PciBus, PciDevice};
pub use storage::{AtaDriver, StorageDevice};
pub use usb::{UsbBus, UsbDevice};

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
    virtio::blk::init();
    if let Err(_e) = gpu::init() {
        crate::println!("[DRIVERS] Warning: GPU init failed: {:?}", _e);
    }

    crate::println!("[DRIVERS] Device drivers initialized");
}

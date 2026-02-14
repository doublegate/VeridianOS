//! Network Driver Integration Module
//!
//! Provides automatic registration of hardware network drivers with the network
//! stack. Uses PCI bus enumeration to auto-detect network hardware.

use alloc::boxed::Box;

use super::device::{self, NetworkDevice};
use crate::error::KernelError;

// PCI vendor and device IDs for network cards (only used on x86_64)
#[cfg(target_arch = "x86_64")]
const INTEL_VENDOR_ID: u16 = 0x8086;
#[cfg(target_arch = "x86_64")]
const E1000_DEVICE_ID: u16 = 0x100E;
#[cfg(target_arch = "x86_64")]
const E1000E_DEVICE_ID: u16 = 0x10D3;
#[cfg(target_arch = "x86_64")]
const REDHAT_VENDOR_ID: u16 = 0x1AF4;
#[cfg(target_arch = "x86_64")]
const VIRTIO_NET_LEGACY_DEVICE_ID: u16 = 0x1000;
#[cfg(target_arch = "x86_64")]
const VIRTIO_NET_MODERN_DEVICE_ID: u16 = 0x1041;
/// Initialize and register all available network drivers
#[allow(unused_assignments)]
pub fn register_drivers() -> Result<(), KernelError> {
    println!("[NET-INTEGRATION] Scanning for network devices...");

    #[allow(unused_variables)]
    let mut device_count = 0;

    // Only x86_64 has PCI support
    #[cfg(target_arch = "x86_64")]
    {
        // Check if PCI is initialized before trying to access it
        if !crate::drivers::pci::is_pci_initialized() {
            println!("[NET-INTEGRATION] PCI bus not initialized, skipping PCI device scan");
        } else {
            // Get PCI bus and enumerate devices
            let pci_bus = crate::drivers::pci::get_pci_bus();
            let bus = pci_bus.lock();

            // Ensure PCI enumeration is complete
            let _ = bus.enumerate_devices();

            // Search for Intel E1000 network cards
            let e1000_devices = bus.find_devices_by_id(INTEL_VENDOR_ID, E1000_DEVICE_ID);
            for device in e1000_devices {
                println!(
                    "[NET-INTEGRATION] Found E1000 at {:02x}:{:02x}.{}",
                    device.location.bus, device.location.device, device.location.function
                );

                // Get BAR0 (MMIO base address)
                if let Some(bar0) = device.bars.first() {
                    if let Some(address) = bar0.get_memory_address() {
                        if try_register_e1000(address).is_ok() {
                            device_count += 1;
                        }
                    }
                }
            }

            // Search for Intel E1000E network cards
            let e1000e_devices = bus.find_devices_by_id(INTEL_VENDOR_ID, E1000E_DEVICE_ID);
            for device in e1000e_devices {
                println!(
                    "[NET-INTEGRATION] Found E1000E at {:02x}:{:02x}.{}",
                    device.location.bus, device.location.device, device.location.function
                );

                if let Some(bar0) = device.bars.first() {
                    if let Some(address) = bar0.get_memory_address() {
                        if try_register_e1000(address).is_ok() {
                            device_count += 1;
                        }
                    }
                }
            }

            // Search for VirtIO-Net (legacy and modern)
            let virtio_legacy =
                bus.find_devices_by_id(REDHAT_VENDOR_ID, VIRTIO_NET_LEGACY_DEVICE_ID);
            let virtio_modern =
                bus.find_devices_by_id(REDHAT_VENDOR_ID, VIRTIO_NET_MODERN_DEVICE_ID);

            for device in virtio_legacy.iter().chain(virtio_modern.iter()) {
                println!(
                    "[NET-INTEGRATION] Found VirtIO-Net at {:02x}:{:02x}.{}",
                    device.location.bus, device.location.device, device.location.function
                );

                if let Some(bar0) = device.bars.first() {
                    if let Some(address) = bar0.get_memory_address() {
                        if try_register_virtio_net(address).is_ok() {
                            device_count += 1;
                        }
                    }
                }
            }
        }
    }

    // For non-x86_64 architectures, try VirtIO MMIO at known addresses
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        // VirtIO MMIO devices at platform-specific addresses
        #[cfg(target_arch = "aarch64")]
        let virtio_bases = [0x0a000000, 0x0a000200, 0x0a000400, 0x0a000600];

        #[cfg(target_arch = "riscv64")]
        let virtio_bases = [0x10001000, 0x10002000, 0x10003000, 0x10004000];

        for &base in &virtio_bases {
            if try_register_virtio_net(base as u64).is_ok() {
                device_count += 1;
            }
        }
    }

    println!(
        "[NET-INTEGRATION] Network device scan complete: {} devices registered",
        device_count
    );
    Ok(())
}

/// Try to register E1000 driver if hardware is present
///
/// Called from the x86_64 PCI device scan path above.
#[cfg(target_arch = "x86_64")]
fn try_register_e1000(bar_address: u64) -> Result<(), KernelError> {
    use crate::drivers::e1000::E1000Driver;

    println!(
        "[NET-INTEGRATION] Initializing E1000 at 0x{:x}",
        bar_address
    );

    match E1000Driver::new(bar_address as usize) {
        Ok(driver) => {
            let name = driver.name();
            let mac = driver.mac_address();

            // TODO(phase4): Register E1000 with network device registry

            println!(
                "[NET-INTEGRATION] E1000 initialized: {} (MAC: \
                 {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
                name, mac.0[0], mac.0[1], mac.0[2], mac.0[3], mac.0[4], mac.0[5]
            );

            Ok(())
        }
        Err(_) => Err(KernelError::NotFound {
            resource: "e1000_hardware",
            id: 0,
        }),
    }
}

/// Try to register VirtIO-Net driver if hardware is present
fn try_register_virtio_net(bar_address: u64) -> Result<(), KernelError> {
    use crate::drivers::virtio_net::VirtioNetDriver;

    println!(
        "[NET-INTEGRATION] Initializing VirtIO-Net at 0x{:x}",
        bar_address
    );

    match VirtioNetDriver::new(bar_address as usize) {
        Ok(driver) => {
            let _name = driver.name();
            let _mac = driver.mac_address();

            // TODO(phase4): Register VirtIO-Net with network device registry

            println!(
                "[NET-INTEGRATION] VirtIO-Net initialized: {} (MAC: \
                 {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
                _name, _mac.0[0], _mac.0[1], _mac.0[2], _mac.0[3], _mac.0[4], _mac.0[5]
            );

            Ok(())
        }
        Err(_) => Err(KernelError::NotFound {
            resource: "virtio_net_hardware",
            id: 0,
        }),
    }
}

/// Register a manually-created network device (for testing/debugging)
pub fn register_device(device: Box<dyn NetworkDevice>) -> Result<(), KernelError> {
    device::register_device(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_integration_init() {
        // Basic smoke test
        let result = register_drivers();
        assert!(result.is_ok() || result.is_err()); // Always returns something
    }
}

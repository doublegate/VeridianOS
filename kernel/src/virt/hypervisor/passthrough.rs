//! VirtIO Device Passthrough
//!
//! Device assignment, MMIO mapping, interrupt forwarding for passthrough
//! devices.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::MAX_MSIX_VECTORS;
use crate::virt::VmError;

// ---------------------------------------------------------------------------
// 2. VirtIO Device Passthrough
// ---------------------------------------------------------------------------

/// Passthrough device type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughDeviceType {
    /// VirtIO network device
    VirtioNet,
    /// VirtIO block device
    VirtioBlk,
    /// VirtIO GPU device
    VirtioGpu,
    /// VirtIO sound device
    VirtioSound,
    /// Generic PCI device
    GenericPci,
}

/// MSI-X vector remapping entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MsixRemap {
    /// Host vector number
    pub host_vector: u16,
    /// Guest vector number
    pub guest_vector: u16,
    /// Target vCPU for delivery
    pub target_vcpu: u8,
    /// Whether this remap is active
    pub active: bool,
}

/// MMIO region mapping for passthrough device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioRegion {
    /// Host physical address
    pub host_phys: u64,
    /// Guest physical address (mapped into guest EPT)
    pub guest_phys: u64,
    /// Region size in bytes
    pub size: u64,
    /// Whether the region is currently mapped
    pub mapped: bool,
}

/// PCI BAR (Base Address Register) info
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciBar {
    /// BAR index (0-5)
    pub index: u8,
    /// Base address
    pub address: u64,
    /// Size in bytes
    pub size: u64,
    /// Whether this is a memory BAR (vs I/O)
    pub is_memory: bool,
    /// Whether this is a 64-bit BAR
    pub is_64bit: bool,
    /// Whether this is prefetchable
    pub prefetchable: bool,
}

/// PCI configuration space passthrough
#[derive(Debug, Clone)]
pub struct PciConfigPassthrough {
    /// Vendor ID
    pub vendor_id: u16,
    /// Device ID
    pub device_id: u16,
    /// BDF (bus:device:function)
    pub bdf: u32,
    /// BARs
    #[cfg(feature = "alloc")]
    pub bars: Vec<PciBar>,
    /// Emulated config space (256 bytes for type 0)
    pub config_space: [u8; 256],
    /// Which config registers are writable by guest
    pub writable_mask: [u8; 256],
}

impl PciConfigPassthrough {
    #[cfg(feature = "alloc")]
    pub fn new(vendor_id: u16, device_id: u16, bdf: u32) -> Self {
        let mut config_space = [0u8; 256];
        // Set vendor/device ID
        config_space[0] = vendor_id as u8;
        config_space[1] = (vendor_id >> 8) as u8;
        config_space[2] = device_id as u8;
        config_space[3] = (device_id >> 8) as u8;

        let mut writable_mask = [0u8; 256];
        // Command register is writable
        writable_mask[4] = 0xFF;
        writable_mask[5] = 0xFF;
        // BAR registers (0x10-0x27)
        for item in writable_mask.iter_mut().take(0x27 + 1).skip(0x10) {
            *item = 0xFF;
        }

        Self {
            vendor_id,
            device_id,
            bdf,
            bars: Vec::new(),
            config_space,
            writable_mask,
        }
    }

    /// Read from config space
    pub fn read_config(&self, offset: u8) -> u8 {
        self.config_space[offset as usize]
    }

    /// Write to config space (respecting writable mask)
    pub fn write_config(&mut self, offset: u8, value: u8) {
        let mask = self.writable_mask[offset as usize];
        let idx = offset as usize;
        self.config_space[idx] = (self.config_space[idx] & !mask) | (value & mask);
    }

    /// Read 32-bit config space value
    pub fn read_config32(&self, offset: u8) -> u32 {
        let idx = (offset & 0xFC) as usize;
        u32::from_le_bytes([
            self.config_space[idx],
            self.config_space[idx + 1],
            self.config_space[idx + 2],
            self.config_space[idx + 3],
        ])
    }
}

/// A passthrough device assigned to a guest VM
#[cfg(feature = "alloc")]
pub struct PassthroughDevice {
    /// Device type
    pub device_type: PassthroughDeviceType,
    /// PCI configuration space
    pub pci_config: PciConfigPassthrough,
    /// MMIO regions mapped into guest
    pub mmio_regions: Vec<MmioRegion>,
    /// MSI-X vector remappings
    pub msix_remaps: Vec<MsixRemap>,
    /// Whether the device is currently assigned to a guest
    pub assigned: bool,
    /// VM ID that owns this device
    pub owner_vm_id: u64,
}

#[cfg(feature = "alloc")]
impl PassthroughDevice {
    pub fn new(
        device_type: PassthroughDeviceType,
        vendor_id: u16,
        device_id: u16,
        bdf: u32,
    ) -> Self {
        Self {
            device_type,
            pci_config: PciConfigPassthrough::new(vendor_id, device_id, bdf),
            mmio_regions: Vec::new(),
            msix_remaps: Vec::new(),
            assigned: false,
            owner_vm_id: 0,
        }
    }

    /// Assign device to a VM
    pub fn assign_to_vm(&mut self, vm_id: u64) -> Result<(), VmError> {
        if self.assigned {
            return Err(VmError::DeviceError);
        }
        self.assigned = true;
        self.owner_vm_id = vm_id;
        Ok(())
    }

    /// Unassign device from VM (reset on guest exit)
    pub fn unassign(&mut self) {
        self.assigned = false;
        self.owner_vm_id = 0;
        // Reset MSI-X remaps
        for remap in &mut self.msix_remaps {
            remap.active = false;
        }
        // Unmap MMIO regions
        for region in &mut self.mmio_regions {
            region.mapped = false;
        }
    }

    /// Add an MMIO region mapping
    pub fn add_mmio_region(&mut self, host_phys: u64, guest_phys: u64, size: u64) {
        self.mmio_regions.push(MmioRegion {
            host_phys,
            guest_phys,
            size,
            mapped: true,
        });
    }

    /// Add an MSI-X remap entry
    pub fn add_msix_remap(&mut self, host_vector: u16, guest_vector: u16, target_vcpu: u8) {
        if self.msix_remaps.len() < MAX_MSIX_VECTORS {
            self.msix_remaps.push(MsixRemap {
                host_vector,
                guest_vector,
                target_vcpu,
                active: true,
            });
        }
    }

    /// Look up guest vector for a host interrupt
    pub fn remap_interrupt(&self, host_vector: u16) -> Option<(u16, u8)> {
        for remap in &self.msix_remaps {
            if remap.active && remap.host_vector == host_vector {
                return Some((remap.guest_vector, remap.target_vcpu));
            }
        }
        None
    }

    /// Reset device to initial state
    pub fn reset(&mut self) {
        self.pci_config.config_space[4] = 0; // Command register
        self.pci_config.config_space[5] = 0;
        for remap in &mut self.msix_remaps {
            remap.active = false;
        }
    }

    pub fn is_assigned(&self) -> bool {
        self.assigned
    }

    pub fn mmio_region_count(&self) -> usize {
        self.mmio_regions.len()
    }

    pub fn msix_remap_count(&self) -> usize {
        self.msix_remaps.len()
    }
}

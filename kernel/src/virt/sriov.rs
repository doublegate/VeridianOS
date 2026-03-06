//! SR-IOV (Single Root I/O Virtualization) support
//!
//! Implements SR-IOV capability parsing, VF enable/disable, and VF-to-VM
//! assignment for high-performance device sharing.
//!
//! Sprint W5-S8.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

use super::{vfio::PciAddress, VmError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// SR-IOV PCI Extended Capability ID
const SRIOV_CAP_ID: u16 = 0x0010;

/// Maximum VFs per physical function
const MAX_VFS: usize = 256;

/// SR-IOV capability register offsets (from capability start)
const SRIOV_CAP_OFFSET: u16 = 0x04;
const SRIOV_CTRL_OFFSET: u16 = 0x08;
const SRIOV_TOTAL_VFS_OFFSET: u16 = 0x0E;
const SRIOV_NUM_VFS_OFFSET: u16 = 0x10;
const SRIOV_VF_OFFSET_OFFSET: u16 = 0x14;
const SRIOV_VF_STRIDE_OFFSET: u16 = 0x16;
const SRIOV_VF_DEVICE_ID_OFFSET: u16 = 0x1A;

/// SR-IOV control register bits
const SRIOV_CTRL_VF_ENABLE: u16 = 0x0001;
const SRIOV_CTRL_VF_MIGRATION: u16 = 0x0002;
const SRIOV_CTRL_ARI_CAPABLE: u16 = 0x0010;

// ---------------------------------------------------------------------------
// SR-IOV Capability
// ---------------------------------------------------------------------------

/// Parsed SR-IOV capability from PCI config space
#[derive(Debug, Clone, Copy)]
pub struct SriovCapability {
    /// Offset of the SR-IOV capability in PCI config space
    pub offset: u16,
    /// Total number of VFs supported by hardware
    pub total_vfs: u16,
    /// Currently enabled number of VFs
    pub num_vfs: u16,
    /// First VF offset (RID offset from PF)
    pub vf_offset: u16,
    /// VF stride (RID stride between consecutive VFs)
    pub vf_stride: u16,
    /// VF device ID
    pub vf_device_id: u16,
    /// SR-IOV capability flags
    pub capabilities: u32,
    /// Whether VF migration is supported
    pub migration_capable: bool,
    /// Whether ARI (Alternative Routing-ID Interpretation) is capable
    pub ari_capable: bool,
}

impl SriovCapability {
    /// Parse SR-IOV capability from a config space data buffer
    ///
    /// `data` should contain the SR-IOV capability structure starting at index
    /// 0.
    pub fn parse(data: &[u8], offset: u16) -> Result<Self, VmError> {
        if data.len() < 0x24 {
            return Err(VmError::DeviceError);
        }

        let capabilities = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let ctrl = u16::from_le_bytes([data[8], data[9]]);
        let total_vfs = u16::from_le_bytes([data[0x0E], data[0x0F]]);
        let num_vfs = u16::from_le_bytes([data[0x10], data[0x11]]);
        let vf_offset = u16::from_le_bytes([data[0x14], data[0x15]]);
        let vf_stride = u16::from_le_bytes([data[0x16], data[0x17]]);
        let vf_device_id = u16::from_le_bytes([data[0x1A], data[0x1B]]);

        Ok(Self {
            offset,
            total_vfs,
            num_vfs,
            vf_offset,
            vf_stride,
            vf_device_id,
            capabilities,
            migration_capable: ctrl & SRIOV_CTRL_VF_MIGRATION != 0,
            ari_capable: ctrl & SRIOV_CTRL_ARI_CAPABLE != 0,
        })
    }

    /// Calculate the PCI address of a specific VF
    pub fn vf_address(&self, pf: &PciAddress, vf_index: u16) -> Option<PciAddress> {
        if vf_index >= self.total_vfs {
            return None;
        }
        let pf_bdf = pf.to_bdf();
        let vf_bdf = pf_bdf
            .checked_add(self.vf_offset)?
            .checked_add(self.vf_stride.checked_mul(vf_index)?)?;
        Some(PciAddress::from_bdf(vf_bdf))
    }

    /// Get the SR-IOV extended capability ID
    pub fn cap_id() -> u16 {
        SRIOV_CAP_ID
    }
}

// ---------------------------------------------------------------------------
// Virtual Function
// ---------------------------------------------------------------------------

/// State of a Virtual Function
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VfState {
    /// VF is not enabled
    #[default]
    Disabled,
    /// VF is enabled but not assigned
    Enabled,
    /// VF is assigned to a VM
    Assigned,
}

/// A single Virtual Function instance
#[derive(Debug, Clone, Copy)]
pub struct VirtualFunction {
    /// VF index within the PF's VF space
    pub vf_index: u16,
    /// PCI address of this VF
    pub pci_address: PciAddress,
    /// Whether this VF is enabled
    pub enabled: bool,
    /// VM this VF is assigned to (None = not assigned)
    pub assigned_vm: Option<u32>,
    /// Current state
    pub state: VfState,
}

impl VirtualFunction {
    /// Create a new VF
    pub fn new(vf_index: u16, pci_address: PciAddress) -> Self {
        Self {
            vf_index,
            pci_address,
            enabled: false,
            assigned_vm: None,
            state: VfState::Disabled,
        }
    }

    /// Enable this VF
    pub fn enable(&mut self) {
        self.enabled = true;
        self.state = VfState::Enabled;
    }

    /// Disable this VF
    pub fn disable(&mut self) {
        self.enabled = false;
        self.assigned_vm = None;
        self.state = VfState::Disabled;
    }

    /// Assign this VF to a VM
    pub fn assign(&mut self, vm_id: u32) -> Result<(), VmError> {
        if !self.enabled {
            return Err(VmError::DeviceError);
        }
        if self.assigned_vm.is_some() {
            return Err(VmError::DeviceError);
        }
        self.assigned_vm = Some(vm_id);
        self.state = VfState::Assigned;
        Ok(())
    }

    /// Unassign this VF from its VM
    pub fn unassign(&mut self) {
        self.assigned_vm = None;
        if self.enabled {
            self.state = VfState::Enabled;
        } else {
            self.state = VfState::Disabled;
        }
    }

    /// Check if this VF is available for assignment
    pub fn is_available(&self) -> bool {
        self.enabled && self.assigned_vm.is_none()
    }
}

// ---------------------------------------------------------------------------
// SR-IOV Device (Physical Function)
// ---------------------------------------------------------------------------

/// An SR-IOV physical function with its virtual functions
#[cfg(feature = "alloc")]
pub struct SriovDevice {
    /// PCI address of the physical function
    pub pf_address: PciAddress,
    /// Parsed SR-IOV capability
    pub capability: SriovCapability,
    /// Virtual functions
    pub vfs: Vec<VirtualFunction>,
    /// Whether VFs are currently enabled
    pub vfs_enabled: bool,
}

#[cfg(feature = "alloc")]
impl SriovDevice {
    /// Create a new SR-IOV device from a PF and capability
    pub fn new(pf_address: PciAddress, capability: SriovCapability) -> Self {
        Self {
            pf_address,
            capability,
            vfs: Vec::new(),
            vfs_enabled: false,
        }
    }

    /// Parse capability from config space data
    pub fn parse_capability(
        pf_address: PciAddress,
        data: &[u8],
        offset: u16,
    ) -> Result<Self, VmError> {
        let cap = SriovCapability::parse(data, offset)?;
        Ok(Self::new(pf_address, cap))
    }

    /// Enable VFs (creates VF entries)
    pub fn enable_vfs(&mut self, num_vfs: u16) -> Result<(), VmError> {
        if num_vfs > self.capability.total_vfs || num_vfs as usize > MAX_VFS {
            return Err(VmError::DeviceError);
        }
        if self.vfs_enabled {
            return Err(VmError::VmxAlreadyEnabled);
        }

        self.vfs.clear();
        for i in 0..num_vfs {
            let vf_addr = self
                .capability
                .vf_address(&self.pf_address, i)
                .ok_or(VmError::DeviceError)?;
            let mut vf = VirtualFunction::new(i, vf_addr);
            vf.enable();
            self.vfs.push(vf);
        }
        self.capability.num_vfs = num_vfs;
        self.vfs_enabled = true;
        Ok(())
    }

    /// Disable all VFs
    pub fn disable_vfs(&mut self) {
        for vf in &mut self.vfs {
            vf.disable();
        }
        self.vfs.clear();
        self.capability.num_vfs = 0;
        self.vfs_enabled = false;
    }

    /// Assign a VF to a VM
    pub fn assign_vf(&mut self, vf_index: u16, vm_id: u32) -> Result<(), VmError> {
        let vf = self
            .vfs
            .iter_mut()
            .find(|v| v.vf_index == vf_index)
            .ok_or(VmError::DeviceError)?;
        vf.assign(vm_id)
    }

    /// Unassign a VF from its VM
    pub fn unassign_vf(&mut self, vf_index: u16) -> Result<(), VmError> {
        let vf = self
            .vfs
            .iter_mut()
            .find(|v| v.vf_index == vf_index)
            .ok_or(VmError::DeviceError)?;
        vf.unassign();
        Ok(())
    }

    /// Get a VF by index
    pub fn vf(&self, vf_index: u16) -> Option<&VirtualFunction> {
        self.vfs.iter().find(|v| v.vf_index == vf_index)
    }

    /// Get number of enabled VFs
    pub fn num_enabled_vfs(&self) -> usize {
        self.vfs.iter().filter(|v| v.enabled).count()
    }

    /// Get number of assigned VFs
    pub fn num_assigned_vfs(&self) -> usize {
        self.vfs.iter().filter(|v| v.assigned_vm.is_some()).count()
    }

    /// List available (unassigned) VFs
    pub fn available_vfs(&self) -> Vec<u16> {
        self.vfs
            .iter()
            .filter(|v| v.is_available())
            .map(|v| v.vf_index)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SR-IOV Manager
// ---------------------------------------------------------------------------

/// Manager for all SR-IOV devices in the system
#[cfg(feature = "alloc")]
pub struct SriovManager {
    /// Known SR-IOV devices keyed by PF BDF
    devices: BTreeMap<u16, SriovDevice>,
}

#[cfg(feature = "alloc")]
impl Default for SriovManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl SriovManager {
    /// Create a new SR-IOV manager
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
        }
    }

    /// Discover and register an SR-IOV device
    pub fn discover(&mut self, device: SriovDevice) {
        let bdf = device.pf_address.to_bdf();
        self.devices.insert(bdf, device);
    }

    /// Get a device by PF address
    pub fn get_device(&self, pf: &PciAddress) -> Option<&SriovDevice> {
        self.devices.get(&pf.to_bdf())
    }

    /// Get a mutable device by PF address
    pub fn get_device_mut(&mut self, pf: &PciAddress) -> Option<&mut SriovDevice> {
        self.devices.get_mut(&pf.to_bdf())
    }

    /// List all VFs across all devices
    pub fn list_vfs(&self) -> Vec<(PciAddress, &VirtualFunction)> {
        let mut result = Vec::new();
        for dev in self.devices.values() {
            for vf in &dev.vfs {
                result.push((dev.pf_address, vf));
            }
        }
        result
    }

    /// Get total number of registered SR-IOV devices
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Get total number of enabled VFs
    pub fn total_vfs(&self) -> usize {
        self.devices.values().map(|d| d.num_enabled_vfs()).sum()
    }

    /// Get total number of assigned VFs
    pub fn total_assigned_vfs(&self) -> usize {
        self.devices.values().map(|d| d.num_assigned_vfs()).sum()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a fake SR-IOV capability config space block
    fn make_sriov_config(
        total_vfs: u16,
        vf_offset: u16,
        vf_stride: u16,
        vf_dev_id: u16,
    ) -> [u8; 0x24] {
        let mut data = [0u8; 0x24];
        // Capabilities at offset 4 (u32)
        data[4] = 0x01; // Some caps
                        // Control at offset 8 (u16)
        data[8] = 0x00;
        // Total VFs at offset 0x0E
        data[0x0E] = total_vfs as u8;
        data[0x0F] = (total_vfs >> 8) as u8;
        // Num VFs at offset 0x10
        data[0x10] = 0;
        // VF offset at 0x14
        data[0x14] = vf_offset as u8;
        data[0x15] = (vf_offset >> 8) as u8;
        // VF stride at 0x16
        data[0x16] = vf_stride as u8;
        data[0x17] = (vf_stride >> 8) as u8;
        // VF device ID at 0x1A
        data[0x1A] = vf_dev_id as u8;
        data[0x1B] = (vf_dev_id >> 8) as u8;
        data
    }

    #[test]
    fn test_sriov_capability_parse() {
        let config = make_sriov_config(8, 1, 1, 0x1234);
        let cap = SriovCapability::parse(&config, 0x100).unwrap();
        assert_eq!(cap.total_vfs, 8);
        assert_eq!(cap.vf_offset, 1);
        assert_eq!(cap.vf_stride, 1);
        assert_eq!(cap.vf_device_id, 0x1234);
        assert_eq!(cap.offset, 0x100);
    }

    #[test]
    fn test_sriov_capability_parse_too_short() {
        let data = [0u8; 10];
        assert!(SriovCapability::parse(&data, 0).is_err());
    }

    #[test]
    fn test_sriov_vf_address() {
        let config = make_sriov_config(4, 2, 1, 0);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let pf = PciAddress::new(0, 3, 0);
        // PF BDF = 0x0018, VF0 = 0x0018+2 = 0x001A, VF1 = 0x001B, etc.
        let vf0 = cap.vf_address(&pf, 0).unwrap();
        assert_eq!(vf0.to_bdf(), pf.to_bdf() + 2);
        let vf1 = cap.vf_address(&pf, 1).unwrap();
        assert_eq!(vf1.to_bdf(), pf.to_bdf() + 3);
    }

    #[test]
    fn test_virtual_function_lifecycle() {
        let mut vf = VirtualFunction::new(0, PciAddress::new(0, 3, 2));
        assert_eq!(vf.state, VfState::Disabled);
        assert!(!vf.is_available());

        vf.enable();
        assert_eq!(vf.state, VfState::Enabled);
        assert!(vf.is_available());

        vf.assign(1).unwrap();
        assert_eq!(vf.state, VfState::Assigned);
        assert!(!vf.is_available());
        assert_eq!(vf.assigned_vm, Some(1));

        vf.unassign();
        assert_eq!(vf.state, VfState::Enabled);
        assert!(vf.is_available());

        vf.disable();
        assert_eq!(vf.state, VfState::Disabled);
    }

    #[test]
    fn test_vf_assign_disabled() {
        let mut vf = VirtualFunction::new(0, PciAddress::new(0, 0, 0));
        assert!(vf.assign(1).is_err());
    }

    #[test]
    fn test_vf_double_assign() {
        let mut vf = VirtualFunction::new(0, PciAddress::new(0, 0, 0));
        vf.enable();
        vf.assign(1).unwrap();
        assert!(vf.assign(2).is_err());
    }

    #[test]
    fn test_sriov_device_enable_vfs() {
        let config = make_sriov_config(8, 1, 1, 0x5678);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let pf = PciAddress::new(0, 5, 0);
        let mut dev = SriovDevice::new(pf, cap);

        dev.enable_vfs(4).unwrap();
        assert_eq!(dev.num_enabled_vfs(), 4);
        assert!(dev.vfs_enabled);
    }

    #[test]
    fn test_sriov_device_enable_too_many() {
        let config = make_sriov_config(4, 1, 1, 0);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let mut dev = SriovDevice::new(PciAddress::new(0, 0, 0), cap);
        assert!(dev.enable_vfs(5).is_err());
    }

    #[test]
    fn test_sriov_device_disable_vfs() {
        let config = make_sriov_config(8, 1, 1, 0);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let mut dev = SriovDevice::new(PciAddress::new(0, 0, 0), cap);
        dev.enable_vfs(4).unwrap();
        dev.disable_vfs();
        assert_eq!(dev.num_enabled_vfs(), 0);
        assert!(!dev.vfs_enabled);
    }

    #[test]
    fn test_sriov_device_assign_vf() {
        let config = make_sriov_config(8, 1, 1, 0);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let mut dev = SriovDevice::new(PciAddress::new(0, 0, 0), cap);
        dev.enable_vfs(4).unwrap();
        dev.assign_vf(0, 42).unwrap();
        assert_eq!(dev.num_assigned_vfs(), 1);
        assert_eq!(dev.vf(0).unwrap().assigned_vm, Some(42));
    }

    #[test]
    fn test_sriov_device_unassign_vf() {
        let config = make_sriov_config(8, 1, 1, 0);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let mut dev = SriovDevice::new(PciAddress::new(0, 0, 0), cap);
        dev.enable_vfs(4).unwrap();
        dev.assign_vf(1, 10).unwrap();
        dev.unassign_vf(1).unwrap();
        assert!(dev.vf(1).unwrap().is_available());
    }

    #[test]
    fn test_sriov_manager() {
        let config = make_sriov_config(4, 1, 1, 0);
        let cap = SriovCapability::parse(&config, 0).unwrap();
        let pf = PciAddress::new(0, 5, 0);
        let mut dev = SriovDevice::new(pf, cap);
        dev.enable_vfs(2).unwrap();

        let mut mgr = SriovManager::new();
        mgr.discover(dev);
        assert_eq!(mgr.device_count(), 1);
        assert_eq!(mgr.total_vfs(), 2);
        assert_eq!(mgr.total_assigned_vfs(), 0);

        let vfs = mgr.list_vfs();
        assert_eq!(vfs.len(), 2);
    }
}

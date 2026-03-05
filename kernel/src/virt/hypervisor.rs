//! Advanced Hypervisor Enhancements
//!
//! Implements 6 hypervisor features for Phase 7.5 Wave 7:
//! 1. Nested Virtualization -- L2 VMCS shadowing with field forwarding
//! 2. VirtIO Device Passthrough -- device assignment, MMIO mapping, interrupt
//!    forwarding
//! 3. Live Migration -- VMCS serialization, dirty page pre-copy, stop-and-copy
//! 4. Guest SMP -- multi-vCPU VMs with per-vCPU VMCS, IPI, SIPI emulation
//! 5. Virtual LAPIC -- full LAPIC register emulation with timer modes
//! 6. VM Snapshots -- complete state capture/restore with memory and device
//!    state

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use super::{vmx::VmcsFields, VmError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum vCPUs per VM
const MAX_VCPUS: usize = 16;

/// Maximum VMs tracked by the hypervisor
const _MAX_VMS: usize = 64;

/// LAPIC base MMIO address (standard x86 location)
const LAPIC_BASE_ADDR: u64 = 0xFEE0_0000;

/// LAPIC register space size (4 KiB)
const LAPIC_REGION_SIZE: u64 = 0x1000;

/// Page size constant
const PAGE_SIZE: u64 = 4096;

/// Number of VMCS field groups for serialization
const _VMCS_FIELD_GROUP_COUNT: usize = 7;

/// Maximum pages per pre-copy iteration
const PRECOPY_BATCH_SIZE: u64 = 256;

/// Dirty page bitmap granularity: bits per u64
const BITS_PER_U64: u64 = 64;

/// Snapshot magic number
const SNAPSHOT_MAGIC: u32 = 0x564D_534E; // "VMSN"

/// Snapshot format version
const SNAPSHOT_VERSION: u32 = 1;

/// Maximum passthrough devices per VM
const _MAX_PASSTHROUGH_DEVICES: usize = 32;

/// Maximum MSI-X vectors
const MAX_MSIX_VECTORS: usize = 64;

// ---------------------------------------------------------------------------
// 1. Nested Virtualization
// ---------------------------------------------------------------------------

/// Shadow VMCS for L2 (nested) guest
#[cfg(feature = "alloc")]
pub struct ShadowVmcs {
    /// Cached field values from L1's perspective
    fields: BTreeMap<u32, u64>,
    /// Whether the shadow VMCS is active
    active: bool,
    /// L1 VMCS link pointer (set to shadow VMCS physical address)
    link_pointer: u64,
}

#[cfg(feature = "alloc")]
impl Default for ShadowVmcs {
    fn default() -> Self {
        Self::new()
    }
}

impl ShadowVmcs {
    pub fn new() -> Self {
        Self {
            fields: BTreeMap::new(),
            active: false,
            link_pointer: 0xFFFF_FFFF_FFFF_FFFF,
        }
    }

    /// Write a field into the shadow VMCS
    pub fn write_field(&mut self, field: u32, value: u64) {
        self.fields.insert(field, value);
    }

    /// Read a field from the shadow VMCS
    pub fn read_field(&self, field: u32) -> Option<u64> {
        self.fields.get(&field).copied()
    }

    /// Activate the shadow VMCS for nested operation
    pub fn activate(&mut self, link_pointer: u64) {
        self.active = true;
        self.link_pointer = link_pointer;
    }

    /// Deactivate the shadow VMCS
    pub fn deactivate(&mut self) {
        self.active = false;
        self.link_pointer = 0xFFFF_FFFF_FFFF_FFFF;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn link_pointer(&self) -> u64 {
        self.link_pointer
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Clear all cached fields
    pub fn clear(&mut self) {
        self.fields.clear();
        self.active = false;
        self.link_pointer = 0xFFFF_FFFF_FFFF_FFFF;
    }
}

/// Nested virtualization state for L1/L2 management
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NestingLevel {
    /// Running at L0 (host hypervisor)
    #[default]
    L0,
    /// Running at L1 (guest hypervisor)
    L1,
    /// Running at L2 (nested guest)
    L2,
}

/// Nested VM entry/exit reason for L1<->L2 transitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedExitReason {
    /// L2 executed VMCALL -- forward to L1
    Vmcall,
    /// L2 EPT violation -- may need L1 handling
    EptViolation,
    /// L2 I/O instruction -- check L1 bitmap
    IoInstruction,
    /// L2 MSR access -- check L1 bitmap
    MsrAccess,
    /// L2 CPUID -- emulate or forward
    Cpuid,
    /// L2 executed VMX instruction -- must forward to L1
    VmxInstruction,
    /// L2 external interrupt -- may deliver to L1
    ExternalInterrupt,
    /// L2 triple fault
    TripleFault,
    /// L2 HLT
    Hlt,
    /// Other reason
    Other(u32),
}

/// VMCS field forwarding: determines how L1 reads/writes to shadow VMCS
/// map onto actual hardware VMCS fields
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldForwardPolicy {
    /// Field is directly passed through to hardware
    Passthrough,
    /// Field is intercepted and emulated
    Emulated,
    /// Field is read-only from L1
    ReadOnly,
    /// Field is hidden from L1
    Hidden,
}

/// Nested virtualization controller
#[cfg(feature = "alloc")]
pub struct NestedVirtController {
    /// Current nesting level
    level: NestingLevel,
    /// Shadow VMCS for L2
    shadow_vmcs: ShadowVmcs,
    /// L1 guest state saved during L2 execution
    l1_saved_state: GuestRegisters,
    /// Field forwarding policies
    field_policies: BTreeMap<u32, FieldForwardPolicy>,
    /// Whether nested VMX is enabled for the guest
    nested_vmx_enabled: bool,
}

#[cfg(feature = "alloc")]
impl Default for NestedVirtController {
    fn default() -> Self {
        Self::new()
    }
}

impl NestedVirtController {
    pub fn new() -> Self {
        let mut policies = BTreeMap::new();
        // Guest RIP/RSP always emulated (L0 controls actual execution)
        policies.insert(VmcsFields::GUEST_RIP, FieldForwardPolicy::Emulated);
        policies.insert(VmcsFields::GUEST_RSP, FieldForwardPolicy::Emulated);
        policies.insert(VmcsFields::GUEST_RFLAGS, FieldForwardPolicy::Emulated);
        // Control registers pass through
        policies.insert(VmcsFields::GUEST_CR0, FieldForwardPolicy::Passthrough);
        policies.insert(VmcsFields::GUEST_CR3, FieldForwardPolicy::Passthrough);
        policies.insert(VmcsFields::GUEST_CR4, FieldForwardPolicy::Passthrough);
        // Exit reason is read-only
        policies.insert(VmcsFields::VM_EXIT_REASON, FieldForwardPolicy::ReadOnly);
        // Host state is hidden from L1 VMREAD
        policies.insert(VmcsFields::HOST_RIP, FieldForwardPolicy::Hidden);
        policies.insert(VmcsFields::HOST_RSP, FieldForwardPolicy::Hidden);

        Self {
            level: NestingLevel::L0,
            shadow_vmcs: ShadowVmcs::new(),
            l1_saved_state: GuestRegisters::default(),
            field_policies: policies,
            nested_vmx_enabled: false,
        }
    }

    /// Enable nested VMX for the guest
    pub fn enable_nested_vmx(&mut self) {
        self.nested_vmx_enabled = true;
    }

    /// Handle L1 VMWRITE to shadow VMCS
    pub fn handle_l1_vmwrite(&mut self, field: u32, value: u64) -> Result<(), VmError> {
        let policy = self
            .field_policies
            .get(&field)
            .copied()
            .unwrap_or(FieldForwardPolicy::Passthrough);

        match policy {
            FieldForwardPolicy::Passthrough | FieldForwardPolicy::Emulated => {
                self.shadow_vmcs.write_field(field, value);
                Ok(())
            }
            FieldForwardPolicy::ReadOnly | FieldForwardPolicy::Hidden => {
                Err(VmError::VmcsFieldError)
            }
        }
    }

    /// Handle L1 VMREAD from shadow VMCS
    pub fn handle_l1_vmread(&self, field: u32) -> Result<u64, VmError> {
        let policy = self
            .field_policies
            .get(&field)
            .copied()
            .unwrap_or(FieldForwardPolicy::Passthrough);

        match policy {
            FieldForwardPolicy::Hidden => Err(VmError::VmcsFieldError),
            _ => self
                .shadow_vmcs
                .read_field(field)
                .ok_or(VmError::VmcsFieldError),
        }
    }

    /// Enter L2 from L1 (nested VM entry)
    pub fn enter_l2(&mut self, l1_state: &GuestRegisters) -> Result<(), VmError> {
        if !self.nested_vmx_enabled {
            return Err(VmError::VmxNotSupported);
        }
        if self.level != NestingLevel::L1 {
            // Must be at L1 to enter L2
            if self.level == NestingLevel::L0 {
                // L0 -> L1 is implicit; let's allow L0 to go to L2 through L1
                self.level = NestingLevel::L1;
            }
        }

        // Save L1 state
        self.l1_saved_state = *l1_state;

        // Validate shadow VMCS has minimum required fields
        if self.shadow_vmcs.read_field(VmcsFields::GUEST_RIP).is_none() {
            return Err(VmError::InvalidGuestState);
        }

        self.level = NestingLevel::L2;
        self.shadow_vmcs.activate(0);
        Ok(())
    }

    /// Exit from L2 back to L1 (nested VM exit)
    pub fn exit_l2(&mut self, exit_reason: NestedExitReason) -> Result<GuestRegisters, VmError> {
        if self.level != NestingLevel::L2 {
            return Err(VmError::InvalidVmState);
        }

        // Store exit reason in shadow VMCS for L1 to read
        let reason_code = match exit_reason {
            NestedExitReason::Vmcall => 18,
            NestedExitReason::EptViolation => 48,
            NestedExitReason::IoInstruction => 30,
            NestedExitReason::MsrAccess => 31,
            NestedExitReason::Cpuid => 10,
            NestedExitReason::VmxInstruction => 18,
            NestedExitReason::ExternalInterrupt => 1,
            NestedExitReason::TripleFault => 2,
            NestedExitReason::Hlt => 12,
            NestedExitReason::Other(code) => code,
        };
        self.shadow_vmcs
            .write_field(VmcsFields::VM_EXIT_REASON, reason_code as u64);

        self.level = NestingLevel::L1;
        self.shadow_vmcs.deactivate();

        // Return saved L1 state
        Ok(self.l1_saved_state)
    }

    pub fn nesting_level(&self) -> NestingLevel {
        self.level
    }

    pub fn is_nested_vmx_enabled(&self) -> bool {
        self.nested_vmx_enabled
    }

    /// Check if a VM exit from L2 should be forwarded to L1
    pub fn should_forward_to_l1(&self, exit_reason: NestedExitReason) -> bool {
        match exit_reason {
            // VMX instructions in L2 always go to L1
            NestedExitReason::VmxInstruction => true,
            // VMCALL always forwarded
            NestedExitReason::Vmcall => true,
            // Triple fault always forwarded
            NestedExitReason::TripleFault => true,
            // EPT violations may be handled by L0 or forwarded
            NestedExitReason::EptViolation => true,
            // I/O depends on L1 bitmap
            NestedExitReason::IoInstruction => true,
            // External interrupts go to L0 first
            NestedExitReason::ExternalInterrupt => false,
            _ => true,
        }
    }
}

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

// ---------------------------------------------------------------------------
// 3. Live Migration
// ---------------------------------------------------------------------------

/// Migration state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MigrationState {
    /// Not migrating
    #[default]
    Idle,
    /// Initial setup phase
    Setup,
    /// Pre-copy: iteratively send dirty pages
    PreCopy,
    /// Stop-and-copy: VM paused, final state transfer
    StopAndCopy,
    /// Completing migration
    Completing,
    /// Migration complete
    Complete,
    /// Migration failed
    Failed,
}

/// VMCS field group for serialization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmcsFieldGroup {
    /// Guest register state (RIP, RSP, RFLAGS, etc.)
    GuestRegisterState,
    /// Guest segment state (selectors, bases, limits, AR)
    GuestSegmentState,
    /// Guest control state (CR0, CR3, CR4, DR7)
    GuestControlState,
    /// Host state fields
    HostState,
    /// Execution control fields
    ExecutionControls,
    /// Exit/entry control fields
    ExitEntryControls,
    /// Read-only data fields
    ReadOnlyData,
}

/// Serialized VMCS field
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerializedVmcsField {
    pub encoding: u32,
    pub value: u64,
}

/// Serialized VMCS state for migration
#[cfg(feature = "alloc")]
pub struct SerializedVmcs {
    pub fields: Vec<SerializedVmcsField>,
}

#[cfg(feature = "alloc")]
impl Default for SerializedVmcs {
    fn default() -> Self {
        Self::new()
    }
}

impl SerializedVmcs {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn add_field(&mut self, encoding: u32, value: u64) {
        self.fields.push(SerializedVmcsField { encoding, value });
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn find_field(&self, encoding: u32) -> Option<u64> {
        self.fields
            .iter()
            .find(|f| f.encoding == encoding)
            .map(|f| f.value)
    }

    /// Get all guest register state fields for serialization
    pub fn serialize_guest_registers() -> &'static [u32] {
        &[
            VmcsFields::GUEST_RIP,
            VmcsFields::GUEST_RSP,
            VmcsFields::GUEST_RFLAGS,
            VmcsFields::GUEST_CR0,
            VmcsFields::GUEST_CR3,
            VmcsFields::GUEST_CR4,
            VmcsFields::GUEST_DR7,
            VmcsFields::GUEST_SYSENTER_CS,
            VmcsFields::GUEST_SYSENTER_ESP,
            VmcsFields::GUEST_SYSENTER_EIP,
            VmcsFields::GUEST_IA32_EFER,
            VmcsFields::GUEST_IA32_PAT,
        ]
    }

    /// Get all guest segment state fields
    pub fn serialize_guest_segments() -> &'static [u32] {
        &[
            VmcsFields::GUEST_CS_SELECTOR,
            VmcsFields::GUEST_CS_BASE,
            VmcsFields::GUEST_CS_LIMIT,
            VmcsFields::GUEST_CS_ACCESS_RIGHTS,
            VmcsFields::GUEST_SS_SELECTOR,
            VmcsFields::GUEST_SS_BASE,
            VmcsFields::GUEST_SS_LIMIT,
            VmcsFields::GUEST_SS_ACCESS_RIGHTS,
            VmcsFields::GUEST_DS_SELECTOR,
            VmcsFields::GUEST_DS_BASE,
            VmcsFields::GUEST_DS_LIMIT,
            VmcsFields::GUEST_DS_ACCESS_RIGHTS,
            VmcsFields::GUEST_ES_SELECTOR,
            VmcsFields::GUEST_ES_BASE,
            VmcsFields::GUEST_ES_LIMIT,
            VmcsFields::GUEST_ES_ACCESS_RIGHTS,
            VmcsFields::GUEST_FS_SELECTOR,
            VmcsFields::GUEST_FS_BASE,
            VmcsFields::GUEST_FS_LIMIT,
            VmcsFields::GUEST_FS_ACCESS_RIGHTS,
            VmcsFields::GUEST_GS_SELECTOR,
            VmcsFields::GUEST_GS_BASE,
            VmcsFields::GUEST_GS_LIMIT,
            VmcsFields::GUEST_GS_ACCESS_RIGHTS,
            VmcsFields::GUEST_TR_SELECTOR,
            VmcsFields::GUEST_TR_BASE,
            VmcsFields::GUEST_TR_LIMIT,
            VmcsFields::GUEST_TR_ACCESS_RIGHTS,
            VmcsFields::GUEST_LDTR_SELECTOR,
            VmcsFields::GUEST_LDTR_BASE,
            VmcsFields::GUEST_LDTR_LIMIT,
            VmcsFields::GUEST_LDTR_ACCESS_RIGHTS,
            VmcsFields::GUEST_GDTR_BASE,
            VmcsFields::GUEST_GDTR_LIMIT,
            VmcsFields::GUEST_IDTR_BASE,
            VmcsFields::GUEST_IDTR_LIMIT,
        ]
    }
}

/// Dirty page bitmap for tracking modified guest pages during migration
#[cfg(feature = "alloc")]
pub struct DirtyPageBitmap {
    /// Bitmap: 1 bit per page
    bitmap: Vec<u64>,
    /// Total number of pages tracked
    total_pages: u64,
    /// Count of currently dirty pages
    dirty_count: u64,
}

#[cfg(feature = "alloc")]
impl DirtyPageBitmap {
    pub fn new(total_pages: u64) -> Self {
        let words = total_pages.div_ceil(BITS_PER_U64) as usize;
        Self {
            bitmap: vec![0u64; words],
            total_pages,
            dirty_count: 0,
        }
    }

    /// Mark a page as dirty
    pub fn set_dirty(&mut self, page_index: u64) {
        if page_index >= self.total_pages {
            return;
        }
        let word = (page_index / BITS_PER_U64) as usize;
        let bit = page_index % BITS_PER_U64;
        if self.bitmap[word] & (1u64 << bit) == 0 {
            self.bitmap[word] |= 1u64 << bit;
            self.dirty_count += 1;
        }
    }

    /// Check if a page is dirty
    pub fn is_dirty(&self, page_index: u64) -> bool {
        if page_index >= self.total_pages {
            return false;
        }
        let word = (page_index / BITS_PER_U64) as usize;
        let bit = page_index % BITS_PER_U64;
        self.bitmap[word] & (1u64 << bit) != 0
    }

    /// Clear a page's dirty bit
    pub fn clear_dirty(&mut self, page_index: u64) {
        if page_index >= self.total_pages {
            return;
        }
        let word = (page_index / BITS_PER_U64) as usize;
        let bit = page_index % BITS_PER_U64;
        if self.bitmap[word] & (1u64 << bit) != 0 {
            self.bitmap[word] &= !(1u64 << bit);
            if self.dirty_count > 0 {
                self.dirty_count -= 1;
            }
        }
    }

    /// Clear all dirty bits and return previous dirty count
    pub fn clear_all(&mut self) -> u64 {
        let count = self.dirty_count;
        for word in &mut self.bitmap {
            *word = 0;
        }
        self.dirty_count = 0;
        count
    }

    /// Iterate over dirty page indices
    pub fn dirty_pages(&self) -> DirtyPageIter<'_> {
        DirtyPageIter {
            bitmap: self,
            current_word: 0,
            current_bit: 0,
        }
    }

    pub fn dirty_count(&self) -> u64 {
        self.dirty_count
    }

    pub fn total_pages(&self) -> u64 {
        self.total_pages
    }
}

/// Iterator over dirty pages
#[cfg(feature = "alloc")]
pub struct DirtyPageIter<'a> {
    bitmap: &'a DirtyPageBitmap,
    current_word: usize,
    current_bit: u64,
}

#[cfg(feature = "alloc")]
impl<'a> Iterator for DirtyPageIter<'a> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        while self.current_word < self.bitmap.bitmap.len() {
            let word = self.bitmap.bitmap[self.current_word];
            while self.current_bit < BITS_PER_U64 {
                let bit = self.current_bit;
                self.current_bit += 1;
                if word & (1u64 << bit) != 0 {
                    let page_idx = (self.current_word as u64)
                        .checked_mul(BITS_PER_U64)
                        .and_then(|v| v.checked_add(bit));
                    if let Some(idx) = page_idx {
                        if idx < self.bitmap.total_pages {
                            return Some(idx);
                        }
                    }
                }
            }
            self.current_word += 1;
            self.current_bit = 0;
        }
        None
    }
}

/// Migration progress tracking (integer-only bandwidth/progress estimation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MigrationProgress {
    /// Total bytes to transfer
    pub total_bytes: u64,
    /// Bytes transferred so far
    pub transferred_bytes: u64,
    /// Current iteration number
    pub iteration: u32,
    /// Dirty pages in current iteration
    pub current_dirty_pages: u64,
    /// Dirty pages from previous iteration
    pub previous_dirty_pages: u64,
    /// Estimated bandwidth in bytes per millisecond (integer)
    pub bandwidth_bytes_per_ms: u64,
    /// Estimated remaining time in milliseconds
    pub estimated_remaining_ms: u64,
}

impl MigrationProgress {
    /// Update bandwidth estimate (integer math)
    /// `bytes_sent`: bytes transferred in this iteration
    /// `elapsed_ms`: time for this iteration in milliseconds
    pub fn update_bandwidth(&mut self, bytes_sent: u64, elapsed_ms: u64) {
        if elapsed_ms > 0 {
            self.bandwidth_bytes_per_ms = bytes_sent / elapsed_ms;
        }
        self.transferred_bytes = self.transferred_bytes.saturating_add(bytes_sent);
    }

    /// Estimate remaining transfer time
    pub fn estimate_remaining(&mut self) {
        if self.bandwidth_bytes_per_ms > 0 {
            let remaining = self.total_bytes.saturating_sub(self.transferred_bytes);
            self.estimated_remaining_ms = remaining / self.bandwidth_bytes_per_ms;
        }
    }

    /// Calculate completion percentage (0-100, integer)
    pub fn completion_percent(&self) -> u32 {
        if self.total_bytes == 0 {
            return 100;
        }
        // Use checked_mul to avoid overflow on large memory sizes
        let percent = self
            .transferred_bytes
            .checked_mul(100)
            .map(|v| v / self.total_bytes)
            .unwrap_or(100);
        if percent > 100 {
            100
        } else {
            percent as u32
        }
    }

    /// Check if dirty page convergence threshold is met
    /// Returns true if dirty pages decreased by at least the given percentage
    pub fn has_converged(&self, threshold_percent: u32) -> bool {
        if self.previous_dirty_pages == 0 {
            return true;
        }
        // current_dirty < previous_dirty * (100 - threshold) / 100
        let threshold_pages = self
            .previous_dirty_pages
            .checked_mul((100 - threshold_percent) as u64)
            .map(|v| v / 100)
            .unwrap_or(0);
        self.current_dirty_pages <= threshold_pages
    }
}

/// Live migration controller
#[cfg(feature = "alloc")]
pub struct MigrationController {
    /// Current migration state
    state: MigrationState,
    /// Progress tracking
    progress: MigrationProgress,
    /// Dirty page bitmap
    dirty_bitmap: Option<DirtyPageBitmap>,
    /// Serialized VMCS state
    vmcs_state: Option<SerializedVmcs>,
    /// Source VM ID
    source_vm_id: u64,
    /// Convergence threshold (percent reduction in dirty pages)
    convergence_threshold: u32,
    /// Maximum pre-copy iterations before stop-and-copy
    max_precopy_iterations: u32,
}

#[cfg(feature = "alloc")]
impl MigrationController {
    pub fn new(source_vm_id: u64) -> Self {
        Self {
            state: MigrationState::Idle,
            progress: MigrationProgress::default(),
            dirty_bitmap: None,
            vmcs_state: None,
            source_vm_id,
            convergence_threshold: 20, // 20% reduction required
            max_precopy_iterations: 30,
        }
    }

    /// Begin migration setup
    pub fn begin_setup(&mut self, total_memory_pages: u64) -> Result<(), VmError> {
        if self.state != MigrationState::Idle {
            return Err(VmError::InvalidVmState);
        }

        self.dirty_bitmap = Some(DirtyPageBitmap::new(total_memory_pages));
        self.progress.total_bytes = total_memory_pages
            .checked_mul(PAGE_SIZE)
            .ok_or(VmError::GuestMemoryError)?;
        self.state = MigrationState::Setup;
        Ok(())
    }

    /// Transition to pre-copy phase
    pub fn begin_precopy(&mut self) -> Result<(), VmError> {
        if self.state != MigrationState::Setup {
            return Err(VmError::InvalidVmState);
        }

        // Mark all pages dirty for initial transfer
        if let Some(ref mut bitmap) = self.dirty_bitmap {
            let total = bitmap.total_pages();
            for i in 0..total {
                bitmap.set_dirty(i);
            }
        }

        self.progress.iteration = 0;
        self.state = MigrationState::PreCopy;
        Ok(())
    }

    /// Perform one pre-copy iteration: returns list of dirty page indices to
    /// send
    pub fn precopy_iteration(&mut self) -> Result<Vec<u64>, VmError> {
        if self.state != MigrationState::PreCopy {
            return Err(VmError::InvalidVmState);
        }

        let dirty_pages: Vec<u64> = if let Some(ref bitmap) = self.dirty_bitmap {
            bitmap
                .dirty_pages()
                .take(PRECOPY_BATCH_SIZE as usize)
                .collect()
        } else {
            return Err(VmError::InvalidVmState);
        };

        // Update progress
        self.progress.previous_dirty_pages = self.progress.current_dirty_pages;
        self.progress.current_dirty_pages = if let Some(ref bitmap) = self.dirty_bitmap {
            bitmap.dirty_count()
        } else {
            0
        };
        self.progress.iteration += 1;

        // Clear sent pages from bitmap
        if let Some(ref mut bitmap) = self.dirty_bitmap {
            for &page_idx in &dirty_pages {
                bitmap.clear_dirty(page_idx);
            }
        }

        // Check convergence or max iterations
        if self.progress.has_converged(self.convergence_threshold)
            || self.progress.iteration >= self.max_precopy_iterations
        {
            // Time to stop and copy
            self.state = MigrationState::StopAndCopy;
        }

        Ok(dirty_pages)
    }

    /// Begin stop-and-copy phase (VM must be paused)
    pub fn begin_stop_and_copy(&mut self) -> Result<(), VmError> {
        if self.state != MigrationState::PreCopy && self.state != MigrationState::StopAndCopy {
            return Err(VmError::InvalidVmState);
        }
        self.state = MigrationState::StopAndCopy;
        Ok(())
    }

    /// Serialize VMCS state for transfer
    pub fn serialize_vmcs(&mut self, fields: &[(u32, u64)]) -> Result<(), VmError> {
        let mut vmcs = SerializedVmcs::new();
        for &(encoding, value) in fields {
            vmcs.add_field(encoding, value);
        }
        self.vmcs_state = Some(vmcs);
        Ok(())
    }

    /// Get remaining dirty pages for stop-and-copy final transfer
    pub fn final_dirty_pages(&self) -> Result<Vec<u64>, VmError> {
        if self.state != MigrationState::StopAndCopy {
            return Err(VmError::InvalidVmState);
        }
        if let Some(ref bitmap) = self.dirty_bitmap {
            Ok(bitmap.dirty_pages().collect())
        } else {
            Err(VmError::InvalidVmState)
        }
    }

    /// Complete the migration
    pub fn complete(&mut self) -> Result<(), VmError> {
        if self.state != MigrationState::StopAndCopy {
            return Err(VmError::InvalidVmState);
        }
        self.state = MigrationState::Complete;
        Ok(())
    }

    /// Mark migration as failed
    pub fn fail(&mut self) {
        self.state = MigrationState::Failed;
    }

    pub fn state(&self) -> MigrationState {
        self.state
    }

    pub fn progress(&self) -> &MigrationProgress {
        &self.progress
    }

    pub fn source_vm_id(&self) -> u64 {
        self.source_vm_id
    }

    pub fn vmcs_state(&self) -> Option<&SerializedVmcs> {
        self.vmcs_state.as_ref()
    }
}

// ---------------------------------------------------------------------------
// 4. Guest SMP Support
// ---------------------------------------------------------------------------

/// General-purpose register state for a vCPU
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct GuestRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

/// vCPU execution state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VcpuState {
    /// Not yet started
    #[default]
    Created,
    /// Running guest code
    Running,
    /// Halted (HLT instruction)
    Halted,
    /// Waiting for SIPI
    WaitingForSipi,
    /// Paused by hypervisor
    Paused,
    /// Stopped / destroyed
    Stopped,
}

/// Inter-Processor Interrupt delivery mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpiDeliveryMode {
    /// Fixed: deliver to specific vCPU
    Fixed,
    /// Lowest priority: deliver to lowest-priority vCPU
    LowestPriority,
    /// NMI: deliver NMI
    Nmi,
    /// INIT: send INIT signal
    Init,
    /// SIPI: Startup IPI (with vector for real-mode entry point)
    Sipi,
    /// ExtINT: external interrupt
    ExtInt,
}

/// IPI message between vCPUs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpiMessage {
    /// Source vCPU ID
    pub source: u8,
    /// Destination vCPU ID (0xFF = broadcast)
    pub destination: u8,
    /// Delivery mode
    pub delivery_mode: IpiDeliveryMode,
    /// Vector number (for Fixed/SIPI)
    pub vector: u8,
    /// Level (0 = deassert, 1 = assert)
    pub level: bool,
    /// Trigger mode (true = level, false = edge)
    pub trigger_level: bool,
}

/// Virtual CPU
#[cfg(feature = "alloc")]
pub struct VirtualCpu {
    /// vCPU ID (0 = BSP, 1+ = APs)
    pub id: u8,
    /// Current execution state
    pub state: VcpuState,
    /// General-purpose registers
    pub registers: GuestRegisters,
    /// LAPIC ID for this vCPU
    pub apic_id: u8,
    /// Pending IPIs (queue)
    pub pending_ipis: Vec<IpiMessage>,
    /// Whether this is the bootstrap processor
    pub is_bsp: bool,
    /// Host thread affinity (which host CPU to schedule on)
    pub host_affinity: Option<u32>,
    /// SIPI vector (real-mode entry = vector * 0x1000)
    pub sipi_vector: u8,
    /// VMCS field values for this vCPU
    pub vmcs_fields: BTreeMap<u32, u64>,
}

#[cfg(feature = "alloc")]
impl VirtualCpu {
    pub fn new(id: u8, is_bsp: bool) -> Self {
        let initial_state = if is_bsp {
            VcpuState::Created
        } else {
            VcpuState::WaitingForSipi
        };

        Self {
            id,
            state: initial_state,
            registers: GuestRegisters::default(),
            apic_id: id,
            pending_ipis: Vec::new(),
            is_bsp,
            host_affinity: None,
            sipi_vector: 0,
            vmcs_fields: BTreeMap::new(),
        }
    }

    /// Deliver an IPI to this vCPU
    pub fn deliver_ipi(&mut self, ipi: IpiMessage) {
        match ipi.delivery_mode {
            IpiDeliveryMode::Init => {
                // INIT resets vCPU to wait-for-SIPI state
                self.state = VcpuState::WaitingForSipi;
                self.registers = GuestRegisters::default();
            }
            IpiDeliveryMode::Sipi => {
                if self.state == VcpuState::WaitingForSipi {
                    // SIPI: entry point = vector * 0x1000 in real mode
                    self.sipi_vector = ipi.vector;
                    self.registers.rip = (ipi.vector as u64) << 12;
                    self.state = VcpuState::Running;
                }
                // Ignore SIPI if not in wait-for-SIPI state
            }
            IpiDeliveryMode::Nmi => {
                // Wake from HLT for NMI
                if self.state == VcpuState::Halted {
                    self.state = VcpuState::Running;
                }
                self.pending_ipis.push(ipi);
            }
            _ => {
                if self.state == VcpuState::Halted {
                    self.state = VcpuState::Running;
                }
                self.pending_ipis.push(ipi);
            }
        }
    }

    /// Pop next pending IPI
    pub fn pop_ipi(&mut self) -> Option<IpiMessage> {
        if self.pending_ipis.is_empty() {
            None
        } else {
            Some(self.pending_ipis.remove(0))
        }
    }

    /// Set host CPU affinity for scheduling
    pub fn set_affinity(&mut self, host_cpu: u32) {
        self.host_affinity = Some(host_cpu);
    }

    pub fn pending_ipi_count(&self) -> usize {
        self.pending_ipis.len()
    }

    /// Halt the vCPU (from HLT instruction)
    pub fn halt(&mut self) {
        self.state = VcpuState::Halted;
    }

    /// Pause the vCPU (hypervisor request)
    pub fn pause(&mut self) {
        if self.state == VcpuState::Running {
            self.state = VcpuState::Paused;
        }
    }

    /// Resume the vCPU
    pub fn resume(&mut self) {
        if self.state == VcpuState::Paused {
            self.state = VcpuState::Running;
        }
    }

    /// Stop the vCPU permanently
    pub fn stop(&mut self) {
        self.state = VcpuState::Stopped;
    }
}

/// Multi-vCPU VM
#[cfg(feature = "alloc")]
pub struct SmpVm {
    /// VM identifier
    pub vm_id: u64,
    /// Virtual CPUs
    pub vcpus: Vec<VirtualCpu>,
    /// Maximum vCPUs allowed
    pub max_vcpus: usize,
}

#[cfg(feature = "alloc")]
impl SmpVm {
    pub fn new(vm_id: u64, vcpu_count: usize) -> Result<Self, VmError> {
        if vcpu_count == 0 || vcpu_count > MAX_VCPUS {
            return Err(VmError::InvalidVmState);
        }

        let mut vcpus = Vec::with_capacity(vcpu_count);
        for i in 0..vcpu_count {
            vcpus.push(VirtualCpu::new(i as u8, i == 0));
        }

        Ok(Self {
            vm_id,
            vcpus,
            max_vcpus: vcpu_count,
        })
    }

    /// Send IPI from one vCPU to another
    pub fn send_ipi(
        &mut self,
        source: u8,
        dest: u8,
        mode: IpiDeliveryMode,
        vector: u8,
    ) -> Result<(), VmError> {
        if source as usize >= self.vcpus.len() {
            return Err(VmError::InvalidVmState);
        }

        let ipi = IpiMessage {
            source,
            destination: dest,
            delivery_mode: mode,
            vector,
            level: true,
            trigger_level: false,
        };

        if dest == 0xFF {
            // Broadcast (excluding self)
            for vcpu in &mut self.vcpus {
                if vcpu.id != source {
                    vcpu.deliver_ipi(ipi);
                }
            }
        } else {
            let target = self.vcpus.iter_mut().find(|v| v.apic_id == dest);
            if let Some(vcpu) = target {
                vcpu.deliver_ipi(ipi);
            } else {
                return Err(VmError::InvalidVmState);
            }
        }

        Ok(())
    }

    /// Emulate the AP startup sequence: BSP sends INIT then SIPI
    pub fn startup_ap(&mut self, ap_id: u8, sipi_vector: u8) -> Result<(), VmError> {
        // Send INIT
        self.send_ipi(0, ap_id, IpiDeliveryMode::Init, 0)?;
        // Send SIPI
        self.send_ipi(0, ap_id, IpiDeliveryMode::Sipi, sipi_vector)?;
        Ok(())
    }

    pub fn vcpu_count(&self) -> usize {
        self.vcpus.len()
    }

    pub fn running_vcpu_count(&self) -> usize {
        self.vcpus
            .iter()
            .filter(|v| v.state == VcpuState::Running)
            .count()
    }

    /// Get a vCPU by ID
    pub fn vcpu(&self, id: u8) -> Option<&VirtualCpu> {
        self.vcpus.iter().find(|v| v.id == id)
    }

    /// Get a mutable reference to a vCPU by ID
    pub fn vcpu_mut(&mut self, id: u8) -> Option<&mut VirtualCpu> {
        self.vcpus.iter_mut().find(|v| v.id == id)
    }

    /// Pause all vCPUs
    pub fn pause_all(&mut self) {
        for vcpu in &mut self.vcpus {
            vcpu.pause();
        }
    }

    /// Resume all vCPUs
    pub fn resume_all(&mut self) {
        for vcpu in &mut self.vcpus {
            vcpu.resume();
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Virtual LAPIC Emulation
// ---------------------------------------------------------------------------

/// LAPIC timer mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LapicTimerMode {
    /// One-shot: fires once, then stops
    #[default]
    OneShot,
    /// Periodic: fires repeatedly at interval
    Periodic,
    /// TSC-deadline: fires when TSC >= deadline
    TscDeadline,
}

/// Local Vector Table (LVT) entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LvtEntry {
    /// Raw 32-bit register value
    pub raw: u32,
}

impl Default for LvtEntry {
    fn default() -> Self {
        Self { raw: 0x0001_0000 } // Masked by default
    }
}

impl LvtEntry {
    pub fn vector(&self) -> u8 {
        (self.raw & 0xFF) as u8
    }

    pub fn delivery_mode(&self) -> u8 {
        ((self.raw >> 8) & 0x7) as u8
    }

    pub fn is_masked(&self) -> bool {
        self.raw & (1 << 16) != 0
    }

    pub fn trigger_mode(&self) -> bool {
        self.raw & (1 << 15) != 0
    }

    pub fn timer_mode(&self) -> LapicTimerMode {
        match (self.raw >> 17) & 0x3 {
            0 => LapicTimerMode::OneShot,
            1 => LapicTimerMode::Periodic,
            2 => LapicTimerMode::TscDeadline,
            _ => LapicTimerMode::OneShot,
        }
    }
}

/// Virtual LAPIC register offsets
#[allow(unused)]
pub struct LapicRegs;

#[allow(unused)]
impl LapicRegs {
    pub const ID: u32 = 0x020;
    pub const VERSION: u32 = 0x030;
    pub const TPR: u32 = 0x080;
    pub const APR: u32 = 0x090;
    pub const PPR: u32 = 0x0A0;
    pub const EOI: u32 = 0x0B0;
    pub const RRD: u32 = 0x0C0;
    pub const LDR: u32 = 0x0D0;
    pub const DFR: u32 = 0x0E0;
    pub const SVR: u32 = 0x0F0;
    pub const ISR_BASE: u32 = 0x100;
    pub const TMR_BASE: u32 = 0x180;
    pub const IRR_BASE: u32 = 0x200;
    pub const ESR: u32 = 0x280;
    pub const ICR_LOW: u32 = 0x300;
    pub const ICR_HIGH: u32 = 0x310;
    pub const LVT_TIMER: u32 = 0x320;
    pub const LVT_THERMAL: u32 = 0x330;
    pub const LVT_PERFMON: u32 = 0x340;
    pub const LVT_LINT0: u32 = 0x350;
    pub const LVT_LINT1: u32 = 0x360;
    pub const LVT_ERROR: u32 = 0x370;
    pub const TIMER_INITIAL_COUNT: u32 = 0x380;
    pub const TIMER_CURRENT_COUNT: u32 = 0x390;
    pub const TIMER_DIVIDE_CONFIG: u32 = 0x3E0;
}

/// Virtual LAPIC state
pub struct VirtualLapic {
    /// LAPIC ID
    pub id: u32,
    /// Task Priority Register
    pub tpr: u32,
    /// Spurious Interrupt Vector Register
    pub svr: u32,
    /// In-Service Register (256 bits = 8 x u32)
    pub isr: [u32; 8],
    /// Interrupt Request Register (256 bits = 8 x u32)
    pub irr: [u32; 8],
    /// Trigger Mode Register (256 bits = 8 x u32)
    pub tmr: [u32; 8],
    /// LVT Timer entry
    pub lvt_timer: LvtEntry,
    /// LVT Thermal entry
    pub lvt_thermal: LvtEntry,
    /// LVT Performance Monitor entry
    pub lvt_perfmon: LvtEntry,
    /// LVT LINT0 entry
    pub lvt_lint0: LvtEntry,
    /// LVT LINT1 entry
    pub lvt_lint1: LvtEntry,
    /// LVT Error entry
    pub lvt_error: LvtEntry,
    /// Timer initial count
    pub timer_initial_count: u32,
    /// Timer current count (decrements)
    pub timer_current_count: u32,
    /// Timer divide configuration
    pub timer_divide_config: u32,
    /// TSC deadline value
    pub tsc_deadline: u64,
    /// Error status register
    pub esr: u32,
    /// Interrupt Command Register (low 32 bits)
    pub icr_low: u32,
    /// Interrupt Command Register (high 32 bits)
    pub icr_high: u32,
    /// Logical Destination Register
    pub ldr: u32,
    /// Destination Format Register
    pub dfr: u32,
    /// Whether the LAPIC is enabled (via SVR bit 8)
    pub enabled: bool,
}

impl VirtualLapic {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            tpr: 0,
            svr: 0xFF, // Disabled by default (bit 8 = 0)
            isr: [0; 8],
            irr: [0; 8],
            tmr: [0; 8],
            lvt_timer: LvtEntry::default(),
            lvt_thermal: LvtEntry::default(),
            lvt_perfmon: LvtEntry::default(),
            lvt_lint0: LvtEntry::default(),
            lvt_lint1: LvtEntry::default(),
            lvt_error: LvtEntry::default(),
            timer_initial_count: 0,
            timer_current_count: 0,
            timer_divide_config: 0,
            tsc_deadline: 0,
            esr: 0,
            icr_low: 0,
            icr_high: 0,
            ldr: 0,
            dfr: 0xFFFF_FFFF,
            enabled: false,
        }
    }

    /// Handle MMIO read from LAPIC register space
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset {
            LapicRegs::ID => self.id << 24,
            LapicRegs::VERSION => 0x0005_0014, // version 0x14, max LVT 5
            LapicRegs::TPR => self.tpr,
            LapicRegs::PPR => self.compute_ppr(),
            LapicRegs::LDR => self.ldr,
            LapicRegs::DFR => self.dfr,
            LapicRegs::SVR => self.svr,
            LapicRegs::ESR => self.esr,
            LapicRegs::ICR_LOW => self.icr_low,
            LapicRegs::ICR_HIGH => self.icr_high,
            LapicRegs::LVT_TIMER => self.lvt_timer.raw,
            LapicRegs::LVT_THERMAL => self.lvt_thermal.raw,
            LapicRegs::LVT_PERFMON => self.lvt_perfmon.raw,
            LapicRegs::LVT_LINT0 => self.lvt_lint0.raw,
            LapicRegs::LVT_LINT1 => self.lvt_lint1.raw,
            LapicRegs::LVT_ERROR => self.lvt_error.raw,
            LapicRegs::TIMER_INITIAL_COUNT => self.timer_initial_count,
            LapicRegs::TIMER_CURRENT_COUNT => self.timer_current_count,
            LapicRegs::TIMER_DIVIDE_CONFIG => self.timer_divide_config,
            // ISR/IRR/TMR: 8 registers each at 0x10 intervals
            off if (LapicRegs::ISR_BASE..LapicRegs::ISR_BASE + 0x80).contains(&off) => {
                let idx = ((off - LapicRegs::ISR_BASE) / 0x10) as usize;
                if idx < 8 {
                    self.isr[idx]
                } else {
                    0
                }
            }
            off if (LapicRegs::TMR_BASE..LapicRegs::TMR_BASE + 0x80).contains(&off) => {
                let idx = ((off - LapicRegs::TMR_BASE) / 0x10) as usize;
                if idx < 8 {
                    self.tmr[idx]
                } else {
                    0
                }
            }
            off if (LapicRegs::IRR_BASE..LapicRegs::IRR_BASE + 0x80).contains(&off) => {
                let idx = ((off - LapicRegs::IRR_BASE) / 0x10) as usize;
                if idx < 8 {
                    self.irr[idx]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// Handle MMIO write to LAPIC register space
    pub fn write_register(&mut self, offset: u32, value: u32) {
        match offset {
            LapicRegs::ID => self.id = value >> 24,
            LapicRegs::TPR => self.tpr = value & 0xFF,
            LapicRegs::LDR => self.ldr = value & 0xFF00_0000,
            LapicRegs::DFR => self.dfr = value | 0x0FFF_FFFF,
            LapicRegs::SVR => {
                self.svr = value;
                self.enabled = value & (1 << 8) != 0;
            }
            LapicRegs::EOI => {
                // End of interrupt: clear highest-priority bit in ISR
                self.handle_eoi();
            }
            LapicRegs::ESR => {
                // Write clears ESR
                self.esr = 0;
            }
            LapicRegs::ICR_LOW => {
                self.icr_low = value;
                // Writing ICR low triggers IPI send
            }
            LapicRegs::ICR_HIGH => {
                self.icr_high = value;
            }
            LapicRegs::LVT_TIMER => {
                self.lvt_timer = LvtEntry { raw: value };
            }
            LapicRegs::LVT_THERMAL => {
                self.lvt_thermal = LvtEntry { raw: value };
            }
            LapicRegs::LVT_PERFMON => {
                self.lvt_perfmon = LvtEntry { raw: value };
            }
            LapicRegs::LVT_LINT0 => {
                self.lvt_lint0 = LvtEntry { raw: value };
            }
            LapicRegs::LVT_LINT1 => {
                self.lvt_lint1 = LvtEntry { raw: value };
            }
            LapicRegs::LVT_ERROR => {
                self.lvt_error = LvtEntry { raw: value };
            }
            LapicRegs::TIMER_INITIAL_COUNT => {
                self.timer_initial_count = value;
                self.timer_current_count = value;
            }
            LapicRegs::TIMER_DIVIDE_CONFIG => {
                self.timer_divide_config = value & 0xB;
            }
            _ => {}
        }
    }

    /// Compute the processor priority register
    fn compute_ppr(&self) -> u32 {
        let isrv = self.highest_isr_priority();
        let tpr_class = self.tpr >> 4;
        let isr_class = isrv >> 4;
        if tpr_class >= isr_class {
            self.tpr
        } else {
            isrv
        }
    }

    /// Find highest-priority bit set in ISR
    fn highest_isr_priority(&self) -> u32 {
        for i in (0..8).rev() {
            if self.isr[i] != 0 {
                let bit = 31 - self.isr[i].leading_zeros();
                return (i as u32) * 32 + bit;
            }
        }
        0
    }

    /// Find highest-priority bit set in IRR
    fn highest_irr_priority(&self) -> Option<u32> {
        for i in (0..8).rev() {
            if self.irr[i] != 0 {
                let bit = 31 - self.irr[i].leading_zeros();
                return Some((i as u32) * 32 + bit);
            }
        }
        None
    }

    /// Handle End-Of-Interrupt: clear highest ISR bit
    fn handle_eoi(&mut self) {
        for i in (0..8).rev() {
            if self.isr[i] != 0 {
                let bit = 31 - self.isr[i].leading_zeros();
                self.isr[i] &= !(1 << bit);
                return;
            }
        }
    }

    /// Accept an interrupt: set IRR bit
    pub fn accept_interrupt(&mut self, vector: u8) {
        let idx = (vector / 32) as usize;
        let bit = (vector % 32) as u32;
        if idx < 8 {
            self.irr[idx] |= 1 << bit;
        }
    }

    /// Try to deliver next pending interrupt (IRR -> ISR)
    pub fn deliver_pending_interrupt(&mut self) -> Option<u8> {
        if !self.enabled {
            return None;
        }

        let ppr = self.compute_ppr();
        let ppr_class = ppr >> 4;

        if let Some(vector) = self.highest_irr_priority() {
            let vector_class = vector >> 4;
            if vector_class > ppr_class {
                // Move from IRR to ISR
                let idx = (vector / 32) as usize;
                let bit = vector % 32;
                self.irr[idx] &= !(1 << bit);
                self.isr[idx] |= 1 << bit;
                return Some(vector as u8);
            }
        }
        None
    }

    /// Tick the LAPIC timer (called periodically by hypervisor)
    /// Returns true if timer interrupt should fire
    pub fn tick_timer(&mut self, ticks: u32) -> bool {
        if self.timer_initial_count == 0 || self.lvt_timer.is_masked() {
            return false;
        }

        let mode = self.lvt_timer.timer_mode();
        match mode {
            LapicTimerMode::OneShot => {
                if self.timer_current_count > 0 {
                    if self.timer_current_count <= ticks {
                        self.timer_current_count = 0;
                        return true;
                    }
                    self.timer_current_count -= ticks;
                }
                false
            }
            LapicTimerMode::Periodic => {
                if self.timer_current_count <= ticks {
                    self.timer_current_count = self.timer_initial_count;
                    true
                } else {
                    self.timer_current_count -= ticks;
                    false
                }
            }
            LapicTimerMode::TscDeadline => {
                // TSC deadline mode handled separately
                false
            }
        }
    }

    /// Get the timer divide value from config register
    pub fn timer_divide_value(&self) -> u32 {
        let bits = ((self.timer_divide_config & 0x8) >> 1) | (self.timer_divide_config & 0x3);
        match bits {
            0b000 => 2,
            0b001 => 4,
            0b010 => 8,
            0b011 => 16,
            0b100 => 32,
            0b101 => 64,
            0b110 => 128,
            0b111 => 1,
            _ => 1,
        }
    }

    /// Extract IPI delivery info from ICR
    pub fn extract_ipi(&self) -> IpiMessage {
        let vector = (self.icr_low & 0xFF) as u8;
        let delivery = match (self.icr_low >> 8) & 0x7 {
            0 => IpiDeliveryMode::Fixed,
            1 => IpiDeliveryMode::LowestPriority,
            4 => IpiDeliveryMode::Nmi,
            5 => IpiDeliveryMode::Init,
            6 => IpiDeliveryMode::Sipi,
            7 => IpiDeliveryMode::ExtInt,
            _ => IpiDeliveryMode::Fixed,
        };
        let level = self.icr_low & (1 << 14) != 0;
        let trigger = self.icr_low & (1 << 15) != 0;
        let dest = (self.icr_high >> 24) as u8;

        IpiMessage {
            source: self.id as u8,
            destination: dest,
            delivery_mode: delivery,
            vector,
            level,
            trigger_level: trigger,
        }
    }

    /// Check if the LAPIC is software-enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the LAPIC base address
    pub fn base_address() -> u64 {
        LAPIC_BASE_ADDR
    }

    /// Get the LAPIC region size
    pub fn region_size() -> u64 {
        LAPIC_REGION_SIZE
    }
}

// ---------------------------------------------------------------------------
// 6. VM Snapshots
// ---------------------------------------------------------------------------

/// Snapshot region type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SnapshotRegionType {
    /// VMCS state
    VmcsState = 1,
    /// General-purpose registers
    GeneralRegisters = 2,
    /// MSR values
    MsrValues = 3,
    /// Guest memory page
    MemoryPage = 4,
    /// Virtual device state
    DeviceState = 5,
    /// LAPIC state
    LapicState = 6,
    /// vCPU state
    VcpuState = 7,
}

/// Snapshot file header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SnapshotHeader {
    /// Magic number (SNAPSHOT_MAGIC)
    pub magic: u32,
    /// Format version
    pub version: u32,
    /// VM ID
    pub vm_id: u64,
    /// Number of vCPUs
    pub vcpu_count: u32,
    /// Total memory pages
    pub memory_pages: u64,
    /// Number of regions in the snapshot
    pub region_count: u32,
    /// Total snapshot size in bytes
    pub total_size: u64,
    /// Timestamp (kernel ticks at snapshot time)
    pub timestamp: u64,
    /// Checksum (simple XOR-based)
    pub checksum: u64,
}

impl Default for SnapshotHeader {
    fn default() -> Self {
        Self {
            magic: SNAPSHOT_MAGIC,
            version: SNAPSHOT_VERSION,
            vm_id: 0,
            vcpu_count: 0,
            memory_pages: 0,
            region_count: 0,
            total_size: 0,
            timestamp: 0,
            checksum: 0,
        }
    }
}

impl SnapshotHeader {
    pub fn is_valid(&self) -> bool {
        self.magic == SNAPSHOT_MAGIC && self.version == SNAPSHOT_VERSION
    }

    /// Compute simple XOR checksum over header fields (excluding checksum
    /// itself)
    pub fn compute_checksum(&self) -> u64 {
        let mut ck: u64 = 0;
        ck ^= self.magic as u64;
        ck ^= self.version as u64;
        ck ^= self.vm_id;
        ck ^= self.vcpu_count as u64;
        ck ^= self.memory_pages;
        ck ^= self.region_count as u64;
        ck ^= self.total_size;
        ck ^= self.timestamp;
        ck
    }
}

/// Snapshot region descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SnapshotRegionDescriptor {
    /// Region type
    pub region_type: u32,
    /// vCPU index (for per-CPU state)
    pub vcpu_index: u32,
    /// Offset from start of snapshot data
    pub offset: u64,
    /// Size of this region in bytes
    pub size: u64,
}

/// MSR entry for snapshot
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MsrEntry {
    pub index: u32,
    pub _reserved: u32,
    pub value: u64,
}

/// Common MSR indices for snapshot
#[allow(unused)]
pub struct CommonMsrs;

#[allow(unused)]
impl CommonMsrs {
    pub const IA32_EFER: u32 = 0xC000_0080;
    pub const IA32_STAR: u32 = 0xC000_0081;
    pub const IA32_LSTAR: u32 = 0xC000_0082;
    pub const IA32_CSTAR: u32 = 0xC000_0083;
    pub const IA32_FMASK: u32 = 0xC000_0084;
    pub const IA32_FS_BASE: u32 = 0xC000_0100;
    pub const IA32_GS_BASE: u32 = 0xC000_0101;
    pub const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
    pub const IA32_TSC_AUX: u32 = 0xC000_0103;
    pub const IA32_SYSENTER_CS: u32 = 0x174;
    pub const IA32_SYSENTER_ESP: u32 = 0x175;
    pub const IA32_SYSENTER_EIP: u32 = 0x176;
    pub const IA32_PAT: u32 = 0x277;
    pub const IA32_DEBUGCTL: u32 = 0x1D9;
    pub const IA32_APIC_BASE: u32 = 0x1B;
}

/// Complete VM snapshot (in-memory representation)
#[cfg(feature = "alloc")]
pub struct VmSnapshot {
    /// Snapshot header
    pub header: SnapshotHeader,
    /// Region descriptors
    pub regions: Vec<SnapshotRegionDescriptor>,
    /// Serialized VMCS per vCPU
    pub vmcs_states: Vec<SerializedVmcs>,
    /// General register state per vCPU
    pub register_states: Vec<GuestRegisters>,
    /// MSR values per vCPU
    pub msr_states: Vec<Vec<MsrEntry>>,
    /// LAPIC state per vCPU (serialized as raw register values)
    pub lapic_states: Vec<LapicSnapshot>,
    /// Dirty page indices (pages included in snapshot)
    pub memory_page_indices: Vec<u64>,
    /// Device state blobs
    pub device_states: Vec<DeviceStateBlob>,
}

/// Serialized LAPIC state for snapshot
#[derive(Debug, Clone)]
pub struct LapicSnapshot {
    pub id: u32,
    pub tpr: u32,
    pub svr: u32,
    pub isr: [u32; 8],
    pub irr: [u32; 8],
    pub tmr: [u32; 8],
    pub lvt_timer_raw: u32,
    pub lvt_thermal_raw: u32,
    pub lvt_perfmon_raw: u32,
    pub lvt_lint0_raw: u32,
    pub lvt_lint1_raw: u32,
    pub lvt_error_raw: u32,
    pub timer_initial_count: u32,
    pub timer_current_count: u32,
    pub timer_divide_config: u32,
    pub tsc_deadline: u64,
    pub icr_low: u32,
    pub icr_high: u32,
    pub ldr: u32,
    pub dfr: u32,
    pub enabled: bool,
}

impl LapicSnapshot {
    pub fn from_lapic(lapic: &VirtualLapic) -> Self {
        Self {
            id: lapic.id,
            tpr: lapic.tpr,
            svr: lapic.svr,
            isr: lapic.isr,
            irr: lapic.irr,
            tmr: lapic.tmr,
            lvt_timer_raw: lapic.lvt_timer.raw,
            lvt_thermal_raw: lapic.lvt_thermal.raw,
            lvt_perfmon_raw: lapic.lvt_perfmon.raw,
            lvt_lint0_raw: lapic.lvt_lint0.raw,
            lvt_lint1_raw: lapic.lvt_lint1.raw,
            lvt_error_raw: lapic.lvt_error.raw,
            timer_initial_count: lapic.timer_initial_count,
            timer_current_count: lapic.timer_current_count,
            timer_divide_config: lapic.timer_divide_config,
            tsc_deadline: lapic.tsc_deadline,
            icr_low: lapic.icr_low,
            icr_high: lapic.icr_high,
            ldr: lapic.ldr,
            dfr: lapic.dfr,
            enabled: lapic.enabled,
        }
    }

    /// Restore LAPIC from snapshot
    pub fn restore_to_lapic(&self, lapic: &mut VirtualLapic) {
        lapic.id = self.id;
        lapic.tpr = self.tpr;
        lapic.svr = self.svr;
        lapic.isr = self.isr;
        lapic.irr = self.irr;
        lapic.tmr = self.tmr;
        lapic.lvt_timer = LvtEntry {
            raw: self.lvt_timer_raw,
        };
        lapic.lvt_thermal = LvtEntry {
            raw: self.lvt_thermal_raw,
        };
        lapic.lvt_perfmon = LvtEntry {
            raw: self.lvt_perfmon_raw,
        };
        lapic.lvt_lint0 = LvtEntry {
            raw: self.lvt_lint0_raw,
        };
        lapic.lvt_lint1 = LvtEntry {
            raw: self.lvt_lint1_raw,
        };
        lapic.lvt_error = LvtEntry {
            raw: self.lvt_error_raw,
        };
        lapic.timer_initial_count = self.timer_initial_count;
        lapic.timer_current_count = self.timer_current_count;
        lapic.timer_divide_config = self.timer_divide_config;
        lapic.tsc_deadline = self.tsc_deadline;
        lapic.icr_low = self.icr_low;
        lapic.icr_high = self.icr_high;
        lapic.ldr = self.ldr;
        lapic.dfr = self.dfr;
        lapic.enabled = self.enabled;
    }
}

/// Serialized device state blob
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct DeviceStateBlob {
    /// Device name
    pub name: String,
    /// Serialized state bytes
    pub data: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl VmSnapshot {
    /// Create a new empty snapshot
    pub fn new(vm_id: u64, vcpu_count: u32, memory_pages: u64, timestamp: u64) -> Self {
        let header = SnapshotHeader {
            vm_id,
            vcpu_count,
            memory_pages,
            timestamp,
            ..Default::default()
        };

        Self {
            header,
            regions: Vec::new(),
            vmcs_states: Vec::new(),
            register_states: Vec::new(),
            msr_states: Vec::new(),
            lapic_states: Vec::new(),
            memory_page_indices: Vec::new(),
            device_states: Vec::new(),
        }
    }

    /// Add register state for a vCPU
    pub fn add_register_state(&mut self, vcpu_idx: u32, regs: GuestRegisters) {
        self.register_states.push(regs);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::GeneralRegisters as u32,
            vcpu_index: vcpu_idx,
            offset: 0, // Computed during serialization
            size: core::mem::size_of::<GuestRegisters>() as u64,
        });
        self.header.region_count += 1;
    }

    /// Add VMCS state for a vCPU
    pub fn add_vmcs_state(&mut self, vcpu_idx: u32, vmcs: SerializedVmcs) {
        let field_size = vmcs.field_count() as u64 * 12; // 4 bytes encoding + 8 bytes value
        self.vmcs_states.push(vmcs);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::VmcsState as u32,
            vcpu_index: vcpu_idx,
            offset: 0,
            size: field_size,
        });
        self.header.region_count += 1;
    }

    /// Add MSR state for a vCPU
    pub fn add_msr_state(&mut self, vcpu_idx: u32, msrs: Vec<MsrEntry>) {
        let size = msrs.len() as u64 * 16; // 16 bytes per MsrEntry
        self.msr_states.push(msrs);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::MsrValues as u32,
            vcpu_index: vcpu_idx,
            offset: 0,
            size,
        });
        self.header.region_count += 1;
    }

    /// Add LAPIC state for a vCPU
    pub fn add_lapic_state(&mut self, vcpu_idx: u32, lapic: &VirtualLapic) {
        self.lapic_states.push(LapicSnapshot::from_lapic(lapic));
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::LapicState as u32,
            vcpu_index: vcpu_idx,
            offset: 0,
            size: core::mem::size_of::<LapicSnapshot>() as u64,
        });
        self.header.region_count += 1;
    }

    /// Add a memory page to the snapshot
    pub fn add_memory_page(&mut self, page_index: u64) {
        self.memory_page_indices.push(page_index);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::MemoryPage as u32,
            vcpu_index: 0,
            offset: 0,
            size: PAGE_SIZE,
        });
        self.header.region_count += 1;
    }

    /// Add device state blob
    pub fn add_device_state(&mut self, name: String, data: Vec<u8>) {
        let size = data.len() as u64;
        self.device_states.push(DeviceStateBlob { name, data });
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::DeviceState as u32,
            vcpu_index: 0,
            offset: 0,
            size,
        });
        self.header.region_count += 1;
    }

    /// Finalize snapshot: compute total size and checksum
    pub fn finalize(&mut self) {
        let mut total: u64 = core::mem::size_of::<SnapshotHeader>() as u64;

        // Region descriptors
        total = total.saturating_add(
            (self.regions.len() as u64)
                .checked_mul(core::mem::size_of::<SnapshotRegionDescriptor>() as u64)
                .unwrap_or(0),
        );

        // Region data
        for region in &self.regions {
            total = total.saturating_add(region.size);
        }

        self.header.total_size = total;
        self.header.checksum = self.header.compute_checksum();
    }

    /// Validate snapshot header
    pub fn validate(&self) -> bool {
        self.header.is_valid() && self.header.checksum == self.header.compute_checksum()
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn memory_page_count(&self) -> usize {
        self.memory_page_indices.len()
    }

    pub fn vcpu_state_count(&self) -> usize {
        self.register_states.len()
    }

    pub fn device_state_count(&self) -> usize {
        self.device_states.len()
    }
}

/// Restore a VirtualLapic from a snapshot
#[cfg(feature = "alloc")]
pub fn restore_lapic_from_snapshot(snapshot: &VmSnapshot, vcpu_idx: usize) -> Option<VirtualLapic> {
    let lapic_snap = snapshot.lapic_states.get(vcpu_idx)?;
    let mut lapic = VirtualLapic::new(lapic_snap.id);
    lapic_snap.restore_to_lapic(&mut lapic);
    Some(lapic)
}

// ---------------------------------------------------------------------------
// Hypervisor Manager (ties everything together)
// ---------------------------------------------------------------------------

/// Hypervisor statistics
#[derive(Debug, Default)]
pub struct HypervisorStats {
    pub total_vm_entries: AtomicU64,
    pub total_vm_exits: AtomicU64,
    pub total_ipis_sent: AtomicU64,
    pub total_lapic_timer_fires: AtomicU64,
    pub total_ept_violations: AtomicU64,
    pub total_snapshots_taken: AtomicU64,
    pub total_migrations_started: AtomicU64,
    pub total_migrations_completed: AtomicU64,
}

impl HypervisorStats {
    pub const fn new() -> Self {
        Self {
            total_vm_entries: AtomicU64::new(0),
            total_vm_exits: AtomicU64::new(0),
            total_ipis_sent: AtomicU64::new(0),
            total_lapic_timer_fires: AtomicU64::new(0),
            total_ept_violations: AtomicU64::new(0),
            total_snapshots_taken: AtomicU64::new(0),
            total_migrations_started: AtomicU64::new(0),
            total_migrations_completed: AtomicU64::new(0),
        }
    }

    pub fn record_vm_entry(&self) {
        self.total_vm_entries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_vm_exit(&self) {
        self.total_vm_exits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ipi(&self) {
        self.total_ipis_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_timer_fire(&self) {
        self.total_lapic_timer_fires.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ept_violation(&self) {
        self.total_ept_violations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_snapshot(&self) {
        self.total_snapshots_taken.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_migration_start(&self) {
        self.total_migrations_started
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_migration_complete(&self) {
        self.total_migrations_completed
            .fetch_add(1, Ordering::Relaxed);
    }
}

static HYPERVISOR_STATS: HypervisorStats = HypervisorStats::new();

/// Get global hypervisor statistics
pub fn get_stats() -> &'static HypervisorStats {
    &HYPERVISOR_STATS
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- Nested Virtualization Tests ---

    #[test]
    fn test_shadow_vmcs_read_write() {
        let mut shadow = ShadowVmcs::new();
        shadow.write_field(VmcsFields::GUEST_RIP, 0x1000);
        assert_eq!(shadow.read_field(VmcsFields::GUEST_RIP), Some(0x1000));
        assert_eq!(shadow.read_field(VmcsFields::GUEST_RSP), None);
        assert_eq!(shadow.field_count(), 1);
    }

    #[test]
    fn test_shadow_vmcs_activate_deactivate() {
        let mut shadow = ShadowVmcs::new();
        assert!(!shadow.is_active());
        shadow.activate(0x2000);
        assert!(shadow.is_active());
        assert_eq!(shadow.link_pointer(), 0x2000);
        shadow.deactivate();
        assert!(!shadow.is_active());
        assert_eq!(shadow.link_pointer(), 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn test_shadow_vmcs_clear() {
        let mut shadow = ShadowVmcs::new();
        shadow.write_field(0x100, 42);
        shadow.write_field(0x200, 84);
        shadow.activate(0x3000);
        shadow.clear();
        assert_eq!(shadow.field_count(), 0);
        assert!(!shadow.is_active());
    }

    #[test]
    fn test_nested_controller_l1_vmwrite_passthrough() {
        let mut ctrl = NestedVirtController::new();
        assert!(ctrl
            .handle_l1_vmwrite(VmcsFields::GUEST_CR0, 0x80000011)
            .is_ok());
        assert_eq!(ctrl.handle_l1_vmread(VmcsFields::GUEST_CR0), Ok(0x80000011));
    }

    #[test]
    fn test_nested_controller_l1_vmwrite_hidden_field() {
        let ctrl = NestedVirtController::new();
        assert_eq!(
            ctrl.handle_l1_vmread(VmcsFields::HOST_RIP),
            Err(VmError::VmcsFieldError)
        );
    }

    #[test]
    fn test_nested_controller_l1_vmwrite_readonly_field() {
        let mut ctrl = NestedVirtController::new();
        assert_eq!(
            ctrl.handle_l1_vmwrite(VmcsFields::VM_EXIT_REASON, 42),
            Err(VmError::VmcsFieldError)
        );
    }

    #[test]
    fn test_nested_enter_exit_l2() {
        let mut ctrl = NestedVirtController::new();
        ctrl.enable_nested_vmx();
        ctrl.handle_l1_vmwrite(VmcsFields::GUEST_RIP, 0x5000)
            .unwrap();

        let l1_regs = GuestRegisters {
            rax: 1,
            rip: 0x4000,
            ..Default::default()
        };
        assert!(ctrl.enter_l2(&l1_regs).is_ok());
        assert_eq!(ctrl.nesting_level(), NestingLevel::L2);

        let l1_restored = ctrl.exit_l2(NestedExitReason::Vmcall).unwrap();
        assert_eq!(l1_restored.rax, 1);
        assert_eq!(l1_restored.rip, 0x4000);
        assert_eq!(ctrl.nesting_level(), NestingLevel::L1);
    }

    #[test]
    fn test_nested_exit_l2_stores_reason() {
        let mut ctrl = NestedVirtController::new();
        ctrl.enable_nested_vmx();
        ctrl.handle_l1_vmwrite(VmcsFields::GUEST_RIP, 0x1000)
            .unwrap();
        ctrl.enter_l2(&GuestRegisters::default()).unwrap();
        ctrl.exit_l2(NestedExitReason::EptViolation).unwrap();
        // Exit reason 48 stored in shadow VMCS
        assert_eq!(
            ctrl.shadow_vmcs.read_field(VmcsFields::VM_EXIT_REASON),
            Some(48)
        );
    }

    #[test]
    fn test_nested_should_forward() {
        let ctrl = NestedVirtController::new();
        assert!(ctrl.should_forward_to_l1(NestedExitReason::VmxInstruction));
        assert!(ctrl.should_forward_to_l1(NestedExitReason::Vmcall));
        assert!(!ctrl.should_forward_to_l1(NestedExitReason::ExternalInterrupt));
    }

    // --- VirtIO Device Passthrough Tests ---

    #[test]
    fn test_passthrough_device_assign_unassign() {
        let mut dev = PassthroughDevice::new(
            PassthroughDeviceType::VirtioNet,
            0x1AF4,
            0x1041,
            0x0000_0800,
        );
        assert!(!dev.is_assigned());
        assert!(dev.assign_to_vm(1).is_ok());
        assert!(dev.is_assigned());
        assert_eq!(dev.owner_vm_id, 1);
        // Double assign fails
        assert_eq!(dev.assign_to_vm(2), Err(VmError::DeviceError));
        dev.unassign();
        assert!(!dev.is_assigned());
    }

    #[test]
    fn test_passthrough_msix_remap() {
        let mut dev = PassthroughDevice::new(
            PassthroughDeviceType::VirtioBlk,
            0x1AF4,
            0x1042,
            0x0000_1000,
        );
        dev.add_msix_remap(32, 64, 0);
        dev.add_msix_remap(33, 65, 1);
        assert_eq!(dev.msix_remap_count(), 2);
        assert_eq!(dev.remap_interrupt(32), Some((64, 0)));
        assert_eq!(dev.remap_interrupt(33), Some((65, 1)));
        assert_eq!(dev.remap_interrupt(99), None);
    }

    #[test]
    fn test_passthrough_mmio_region() {
        let mut dev = PassthroughDevice::new(
            PassthroughDeviceType::VirtioGpu,
            0x1AF4,
            0x1050,
            0x0000_1800,
        );
        dev.add_mmio_region(0xFE00_0000, 0xC000_0000, 0x1000);
        assert_eq!(dev.mmio_region_count(), 1);
        assert!(dev.mmio_regions[0].mapped);
    }

    #[test]
    fn test_pci_config_passthrough() {
        let mut pci = PciConfigPassthrough::new(0x1AF4, 0x1041, 0);
        assert_eq!(pci.read_config(0), 0xF4); // vendor low
        assert_eq!(pci.read_config(1), 0x1A); // vendor high
        assert_eq!(pci.read_config(2), 0x41); // device low
        assert_eq!(pci.read_config(3), 0x10); // device high
                                              // Write to command register (writable)
        pci.write_config(4, 0x07);
        assert_eq!(pci.read_config(4), 0x07);
        // Write to read-only area (should be masked)
        pci.write_config(0, 0xFF);
        assert_eq!(pci.read_config(0), 0xF4); // Unchanged
    }

    // --- Live Migration Tests ---

    #[test]
    fn test_dirty_page_bitmap() {
        let mut bm = DirtyPageBitmap::new(256);
        assert_eq!(bm.dirty_count(), 0);
        assert_eq!(bm.total_pages(), 256);

        bm.set_dirty(0);
        bm.set_dirty(63);
        bm.set_dirty(64);
        bm.set_dirty(255);
        assert_eq!(bm.dirty_count(), 4);
        assert!(bm.is_dirty(0));
        assert!(bm.is_dirty(63));
        assert!(bm.is_dirty(64));
        assert!(bm.is_dirty(255));
        assert!(!bm.is_dirty(1));

        bm.clear_dirty(63);
        assert_eq!(bm.dirty_count(), 3);
        assert!(!bm.is_dirty(63));
    }

    #[test]
    fn test_dirty_page_bitmap_idempotent() {
        let mut bm = DirtyPageBitmap::new(128);
        bm.set_dirty(10);
        bm.set_dirty(10); // Double set
        assert_eq!(bm.dirty_count(), 1);
        bm.clear_dirty(10);
        bm.clear_dirty(10); // Double clear
        assert_eq!(bm.dirty_count(), 0);
    }

    #[test]
    fn test_dirty_page_iterator() {
        let mut bm = DirtyPageBitmap::new(200);
        bm.set_dirty(5);
        bm.set_dirty(100);
        bm.set_dirty(199);
        let pages: Vec<u64> = bm.dirty_pages().collect();
        assert_eq!(pages, vec![5, 100, 199]);
    }

    #[test]
    fn test_dirty_page_clear_all() {
        let mut bm = DirtyPageBitmap::new(128);
        bm.set_dirty(0);
        bm.set_dirty(50);
        bm.set_dirty(127);
        let old_count = bm.clear_all();
        assert_eq!(old_count, 3);
        assert_eq!(bm.dirty_count(), 0);
    }

    #[test]
    fn test_migration_progress_bandwidth() {
        let mut progress = MigrationProgress::default();
        progress.total_bytes = 1_000_000;
        progress.update_bandwidth(500_000, 100); // 5000 bytes/ms
        assert_eq!(progress.bandwidth_bytes_per_ms, 5000);
        assert_eq!(progress.transferred_bytes, 500_000);
        progress.estimate_remaining();
        assert_eq!(progress.estimated_remaining_ms, 100); // 500000 / 5000
    }

    #[test]
    fn test_migration_progress_completion() {
        let mut progress = MigrationProgress::default();
        progress.total_bytes = 1000;
        progress.transferred_bytes = 750;
        assert_eq!(progress.completion_percent(), 75);
    }

    #[test]
    fn test_migration_convergence() {
        let mut progress = MigrationProgress::default();
        progress.previous_dirty_pages = 1000;
        progress.current_dirty_pages = 700;
        // 30% reduction, threshold 20% -> converged
        assert!(progress.has_converged(20));
        progress.current_dirty_pages = 900;
        // 10% reduction, threshold 20% -> not converged
        assert!(!progress.has_converged(20));
    }

    #[test]
    fn test_migration_state_machine() {
        let mut ctrl = MigrationController::new(1);
        assert_eq!(ctrl.state(), MigrationState::Idle);

        ctrl.begin_setup(100).unwrap();
        assert_eq!(ctrl.state(), MigrationState::Setup);

        ctrl.begin_precopy().unwrap();
        assert_eq!(ctrl.state(), MigrationState::PreCopy);

        let pages = ctrl.precopy_iteration().unwrap();
        assert!(!pages.is_empty());
    }

    #[test]
    fn test_serialized_vmcs() {
        let mut vmcs = SerializedVmcs::new();
        vmcs.add_field(VmcsFields::GUEST_RIP, 0x1000);
        vmcs.add_field(VmcsFields::GUEST_RSP, 0x7FF0);
        assert_eq!(vmcs.field_count(), 2);
        assert_eq!(vmcs.find_field(VmcsFields::GUEST_RIP), Some(0x1000));
        assert_eq!(vmcs.find_field(VmcsFields::GUEST_CR0), None);
    }

    // --- Guest SMP Tests ---

    #[test]
    fn test_smp_vm_creation() {
        let vm = SmpVm::new(1, 4).unwrap();
        assert_eq!(vm.vcpu_count(), 4);
        assert!(vm.vcpu(0).unwrap().is_bsp);
        assert!(!vm.vcpu(1).unwrap().is_bsp);
        assert_eq!(vm.vcpu(1).unwrap().state, VcpuState::WaitingForSipi);
    }

    #[test]
    fn test_smp_vm_max_vcpu_limit() {
        assert!(SmpVm::new(1, 0).is_err());
        assert!(SmpVm::new(1, MAX_VCPUS + 1).is_err());
        assert!(SmpVm::new(1, MAX_VCPUS).is_ok());
    }

    #[test]
    fn test_vcpu_sipi_startup() {
        let mut vm = SmpVm::new(1, 2).unwrap();
        // AP starts in WaitingForSipi
        assert_eq!(vm.vcpu(1).unwrap().state, VcpuState::WaitingForSipi);

        // BSP sends INIT + SIPI to AP
        vm.startup_ap(1, 0x10).unwrap(); // Entry at 0x10000

        let ap = vm.vcpu(1).unwrap();
        assert_eq!(ap.state, VcpuState::Running);
        assert_eq!(ap.registers.rip, 0x10000);
        assert_eq!(ap.sipi_vector, 0x10);
    }

    #[test]
    fn test_vcpu_ipi_delivery() {
        let mut vm = SmpVm::new(1, 4).unwrap();
        // Start all APs
        for i in 1..4 {
            vm.startup_ap(i, 0x20).unwrap();
        }

        // Send fixed IPI from vCPU 0 to vCPU 2
        vm.send_ipi(0, 2, IpiDeliveryMode::Fixed, 0x30).unwrap();
        assert_eq!(vm.vcpu(2).unwrap().pending_ipi_count(), 1);
        let ipi = vm.vcpu_mut(2).unwrap().pop_ipi().unwrap();
        assert_eq!(ipi.vector, 0x30);
        assert_eq!(ipi.source, 0);
    }

    #[test]
    fn test_vcpu_broadcast_ipi() {
        let mut vm = SmpVm::new(1, 4).unwrap();
        for i in 1..4 {
            vm.startup_ap(i, 0x20).unwrap();
        }

        // Broadcast from vCPU 0
        vm.send_ipi(0, 0xFF, IpiDeliveryMode::Fixed, 0x40).unwrap();
        // All except sender should receive
        assert_eq!(vm.vcpu(0).unwrap().pending_ipi_count(), 0);
        assert_eq!(vm.vcpu(1).unwrap().pending_ipi_count(), 1);
        assert_eq!(vm.vcpu(2).unwrap().pending_ipi_count(), 1);
        assert_eq!(vm.vcpu(3).unwrap().pending_ipi_count(), 1);
    }

    #[test]
    fn test_vcpu_halt_and_nmi_wake() {
        let mut vcpu = VirtualCpu::new(0, true);
        vcpu.state = VcpuState::Running;
        vcpu.halt();
        assert_eq!(vcpu.state, VcpuState::Halted);

        vcpu.deliver_ipi(IpiMessage {
            source: 1,
            destination: 0,
            delivery_mode: IpiDeliveryMode::Nmi,
            vector: 0,
            level: true,
            trigger_level: false,
        });
        assert_eq!(vcpu.state, VcpuState::Running);
    }

    #[test]
    fn test_vcpu_pause_resume() {
        let mut vm = SmpVm::new(1, 2).unwrap();
        vm.vcpu_mut(0).unwrap().state = VcpuState::Running;
        vm.startup_ap(1, 0x10).unwrap();
        assert_eq!(vm.running_vcpu_count(), 2);

        vm.pause_all();
        assert_eq!(vm.vcpu(0).unwrap().state, VcpuState::Paused);
        // AP was Running, now Paused
        assert_eq!(vm.vcpu(1).unwrap().state, VcpuState::Paused);

        vm.resume_all();
        assert_eq!(vm.running_vcpu_count(), 2);
    }

    // --- Virtual LAPIC Tests ---

    #[test]
    fn test_lapic_register_rw() {
        let mut lapic = VirtualLapic::new(0);
        // Write TPR
        lapic.write_register(LapicRegs::TPR, 0x20);
        assert_eq!(lapic.read_register(LapicRegs::TPR), 0x20);
        // Read version
        assert_eq!(lapic.read_register(LapicRegs::VERSION), 0x0005_0014);
        // Read ID
        assert_eq!(lapic.read_register(LapicRegs::ID), 0);
    }

    #[test]
    fn test_lapic_enable_via_svr() {
        let mut lapic = VirtualLapic::new(0);
        assert!(!lapic.is_enabled());
        lapic.write_register(LapicRegs::SVR, 0x1FF); // bit 8 set
        assert!(lapic.is_enabled());
    }

    #[test]
    fn test_lapic_accept_and_deliver_interrupt() {
        let mut lapic = VirtualLapic::new(0);
        lapic.write_register(LapicRegs::SVR, 0x1FF); // Enable
        lapic.accept_interrupt(0x30);
        // IRR should have bit 0x30
        assert!(lapic.irr[1] & (1 << 16) != 0); // 0x30 = 48 = word 1, bit 16
        let vec = lapic.deliver_pending_interrupt();
        assert_eq!(vec, Some(0x30));
        // Now in ISR
        assert!(lapic.isr[1] & (1 << 16) != 0);
    }

    #[test]
    fn test_lapic_eoi() {
        let mut lapic = VirtualLapic::new(0);
        lapic.write_register(LapicRegs::SVR, 0x1FF);
        lapic.accept_interrupt(0x30);
        lapic.deliver_pending_interrupt();
        // ISR has 0x30
        lapic.write_register(LapicRegs::EOI, 0);
        // ISR should be cleared
        assert_eq!(lapic.isr[1] & (1 << 16), 0);
    }

    #[test]
    fn test_lapic_timer_oneshot() {
        let mut lapic = VirtualLapic::new(0);
        lapic.lvt_timer = LvtEntry { raw: 0x0000_0020 }; // vector 0x20, one-shot, unmasked
        lapic.write_register(LapicRegs::TIMER_INITIAL_COUNT, 100);
        assert!(!lapic.tick_timer(50));
        assert_eq!(lapic.timer_current_count, 50);
        assert!(lapic.tick_timer(60)); // Fires
        assert_eq!(lapic.timer_current_count, 0);
        assert!(!lapic.tick_timer(10)); // No more fires
    }

    #[test]
    fn test_lapic_timer_periodic() {
        let mut lapic = VirtualLapic::new(0);
        // Periodic mode: bits 17 = 1
        lapic.lvt_timer = LvtEntry { raw: 0x0002_0020 }; // vector 0x20, periodic
        lapic.write_register(LapicRegs::TIMER_INITIAL_COUNT, 100);
        assert!(lapic.tick_timer(110)); // Fires and reloads
        assert_eq!(lapic.timer_current_count, 100); // Reloaded
    }

    #[test]
    fn test_lapic_timer_divide_value() {
        let mut lapic = VirtualLapic::new(0);
        lapic.timer_divide_config = 0b0000; // divide by 2
        assert_eq!(lapic.timer_divide_value(), 2);
        lapic.timer_divide_config = 0b0011; // divide by 16
        assert_eq!(lapic.timer_divide_value(), 16);
        lapic.timer_divide_config = 0b1011; // divide by 1
        assert_eq!(lapic.timer_divide_value(), 1);
    }

    #[test]
    fn test_lapic_extract_ipi() {
        let mut lapic = VirtualLapic::new(0);
        lapic.icr_low = 0x0000_4030; // vector 0x30, INIT mode (5 << 8)
                                     // Wait, INIT = 5 << 8 = 0x500. Let's set that properly.
        lapic.icr_low = 0x0000_0530; // vector 0x30, INIT delivery mode (5 << 8)
        lapic.icr_high = 0x0200_0000; // dest APIC ID 2
        let ipi = lapic.extract_ipi();
        assert_eq!(ipi.vector, 0x30);
        assert_eq!(ipi.destination, 2);
        assert_eq!(ipi.delivery_mode, IpiDeliveryMode::Init);
    }

    #[test]
    fn test_lapic_priority() {
        let mut lapic = VirtualLapic::new(0);
        lapic.write_register(LapicRegs::SVR, 0x1FF);
        lapic.write_register(LapicRegs::TPR, 0x40); // Priority class 4

        // Interrupt with vector 0x30 (class 3) should NOT be delivered
        // because TPR class (4) > vector class (3)
        lapic.accept_interrupt(0x30);
        assert_eq!(lapic.deliver_pending_interrupt(), None);

        // Interrupt with vector 0x50 (class 5) should be delivered
        lapic.accept_interrupt(0x50);
        assert_eq!(lapic.deliver_pending_interrupt(), Some(0x50));
    }

    // --- Snapshot Tests ---

    #[test]
    fn test_snapshot_header_validation() {
        let mut header = SnapshotHeader::default();
        header.vm_id = 42;
        header.vcpu_count = 4;
        header.memory_pages = 1024;
        header.checksum = header.compute_checksum();
        assert!(header.is_valid());

        // Corrupt magic
        header.magic = 0;
        assert!(!header.is_valid());
    }

    #[test]
    fn test_snapshot_creation_and_finalize() {
        let mut snap = VmSnapshot::new(1, 2, 1024, 123456);
        snap.add_register_state(
            0,
            GuestRegisters {
                rip: 0x1000,
                ..Default::default()
            },
        );
        snap.add_register_state(
            1,
            GuestRegisters {
                rip: 0x2000,
                ..Default::default()
            },
        );
        snap.add_memory_page(0);
        snap.add_memory_page(100);
        snap.finalize();

        assert!(snap.validate());
        assert_eq!(snap.vcpu_state_count(), 2);
        assert_eq!(snap.memory_page_count(), 2);
        assert!(snap.header.total_size > 0);
    }

    #[test]
    fn test_snapshot_lapic_roundtrip() {
        let mut lapic = VirtualLapic::new(3);
        lapic.write_register(LapicRegs::SVR, 0x1FF);
        lapic.write_register(LapicRegs::TPR, 0x50);
        lapic.accept_interrupt(0x80);
        lapic.timer_initial_count = 5000;
        lapic.timer_current_count = 2500;

        let snap = LapicSnapshot::from_lapic(&lapic);
        let mut restored = VirtualLapic::new(0);
        snap.restore_to_lapic(&mut restored);

        assert_eq!(restored.id, 3);
        assert_eq!(restored.tpr, 0x50);
        assert!(restored.is_enabled());
        assert_eq!(restored.timer_initial_count, 5000);
        assert_eq!(restored.timer_current_count, 2500);
        // IRR should be preserved
        assert!(restored.irr[4] & 1 != 0); // vector 0x80 = word 4, bit 0
    }

    #[test]
    fn test_snapshot_device_state() {
        let mut snap = VmSnapshot::new(1, 1, 256, 0);
        snap.add_device_state(String::from("uart0"), vec![0x60, 0x00, 0x00, 0x00]);
        assert_eq!(snap.device_state_count(), 1);
        assert_eq!(snap.device_states[0].name, "uart0");
        assert_eq!(snap.device_states[0].data.len(), 4);
    }

    // --- Hypervisor Stats Tests ---

    #[test]
    fn test_hypervisor_stats() {
        let stats = HypervisorStats::new();
        stats.record_vm_entry();
        stats.record_vm_entry();
        stats.record_vm_exit();
        stats.record_ipi();
        assert_eq!(stats.total_vm_entries.load(Ordering::Relaxed), 2);
        assert_eq!(stats.total_vm_exits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_ipis_sent.load(Ordering::Relaxed), 1);
    }

    // --- LVT Entry Tests ---

    #[test]
    fn test_lvt_entry_fields() {
        let entry = LvtEntry { raw: 0x0002_0030 }; // periodic, vector 0x30
        assert_eq!(entry.vector(), 0x30);
        assert!(!entry.is_masked());
        assert_eq!(entry.timer_mode(), LapicTimerMode::Periodic);

        let masked = LvtEntry { raw: 0x0001_0020 }; // masked
        assert!(masked.is_masked());
    }

    #[test]
    fn test_nesting_level_default() {
        let level = NestingLevel::default();
        assert_eq!(level, NestingLevel::L0);
    }

    #[test]
    fn test_migration_state_default() {
        let state = MigrationState::default();
        assert_eq!(state, MigrationState::Idle);
    }
}

//! Nested Virtualization
//!
//! L2 VMCS shadowing with field forwarding for nested hypervisor support.

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

use super::GuestRegisters;
use crate::virt::{vmx::VmcsFields, VmError};

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
    pub(crate) shadow_vmcs: ShadowVmcs,
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

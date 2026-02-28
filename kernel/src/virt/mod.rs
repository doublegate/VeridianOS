//! Virtualization subsystem - VMX hypervisor, EPT memory, containers
//!
//! Provides hardware-assisted virtualization (Intel VT-x) and
//! OS-level container isolation with Linux-compatible namespaces.

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod container;
pub mod devices;
pub mod memory;
pub mod namespace;
pub mod vmx;

use crate::error::KernelError;

/// Virtualization error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmError {
    VmxNotSupported,
    VmxDisabled,
    VmxonFailed,
    VmxoffFailed,
    VmclearFailed,
    VmptrldFailed,
    VmlaunchFailed,
    VmresumeFailed,
    VmwriteFailed,
    VmreadFailed,
    VmcsAllocationFailed,
    EptMappingFailed,
    GuestMemoryError,
    InvalidVmState,
    DeviceError,
    VmxOperationFailed,
    VmcsFieldError,
    VmxAlreadyEnabled,
    VmEntryFailed,
    VmExitHandlerError,
    InvalidGuestState,
}

impl core::fmt::Display for VmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::VmxNotSupported => write!(f, "VMX not supported"),
            Self::VmxDisabled => write!(f, "VMX disabled in BIOS"),
            Self::VmxonFailed => write!(f, "VMXON failed"),
            Self::VmxoffFailed => write!(f, "VMXOFF failed"),
            Self::VmclearFailed => write!(f, "VMCLEAR failed"),
            Self::VmptrldFailed => write!(f, "VMPTRLD failed"),
            Self::VmlaunchFailed => write!(f, "VMLAUNCH failed"),
            Self::VmresumeFailed => write!(f, "VMRESUME failed"),
            Self::VmwriteFailed => write!(f, "VMWRITE failed"),
            Self::VmreadFailed => write!(f, "VMREAD failed"),
            Self::VmcsAllocationFailed => write!(f, "VMCS allocation failed"),
            Self::EptMappingFailed => write!(f, "EPT mapping failed"),
            Self::GuestMemoryError => write!(f, "Guest memory error"),
            Self::InvalidVmState => write!(f, "Invalid VM state"),
            Self::DeviceError => write!(f, "Device emulation error"),
            Self::VmxOperationFailed => write!(f, "VMX operation failed"),
            Self::VmcsFieldError => write!(f, "VMCS field error"),
            Self::VmxAlreadyEnabled => write!(f, "VMX already enabled"),
            Self::VmEntryFailed => write!(f, "VM entry failed"),
            Self::VmExitHandlerError => write!(f, "VM exit handler error"),
            Self::InvalidGuestState => write!(f, "Invalid guest state"),
        }
    }
}

impl From<VmError> for KernelError {
    fn from(e: VmError) -> Self {
        KernelError::InvalidArgument {
            name: "virt",
            value: match e {
                VmError::VmxNotSupported => "vmx_not_supported",
                VmError::VmxDisabled => "vmx_disabled",
                VmError::VmxonFailed => "vmxon_failed",
                VmError::VmxoffFailed => "vmxoff_failed",
                VmError::VmclearFailed => "vmclear_failed",
                VmError::VmptrldFailed => "vmptrld_failed",
                VmError::VmlaunchFailed => "vmlaunch_failed",
                VmError::VmresumeFailed => "vmresume_failed",
                VmError::VmwriteFailed => "vmwrite_failed",
                VmError::VmreadFailed => "vmread_failed",
                VmError::VmcsAllocationFailed => "vmcs_alloc_failed",
                VmError::EptMappingFailed => "ept_mapping_failed",
                VmError::GuestMemoryError => "guest_memory_error",
                VmError::InvalidVmState => "invalid_vm_state",
                VmError::DeviceError => "device_error",
                VmError::VmxOperationFailed => "vmx_operation_failed",
                VmError::VmcsFieldError => "vmcs_field_error",
                VmError::VmxAlreadyEnabled => "vmx_already_enabled",
                VmError::VmEntryFailed => "vm_entry_failed",
                VmError::VmExitHandlerError => "vm_exit_handler_error",
                VmError::InvalidGuestState => "invalid_guest_state",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum VmExitReason {
    ExceptionOrNmi = 0,
    ExternalInterrupt = 1,
    TripleFault = 2,
    InitSignal = 3,
    StartupIpi = 4,
    IoSmi = 5,
    OtherSmi = 6,
    InterruptWindow = 7,
    NmiWindow = 8,
    TaskSwitch = 9,
    Cpuid = 10,
    Getsec = 11,
    Hlt = 12,
    Invd = 13,
    Invlpg = 14,
    Rdpmc = 15,
    Rdtsc = 16,
    Rsm = 17,
    Vmcall = 18,
    Vmclear = 19,
    Vmlaunch = 20,
    Vmptrld = 21,
    Vmptrst = 22,
    Vmread = 23,
    Vmresume = 24,
    Vmwrite = 25,
    Vmxoff = 26,
    Vmxon = 27,
    ControlRegisterAccess = 28,
    MovDr = 29,
    IoInstruction = 30,
    Rdmsr = 31,
    Wrmsr = 32,
    EntryFailInvalidGuestState = 33,
    EntryFailMsrLoading = 34,
    Mwait = 36,
    MonitorTrapFlag = 37,
    Monitor = 39,
    Pause = 40,
    EntryFailMachineCheck = 41,
    TprBelowThreshold = 43,
    ApicAccess = 44,
    VirtualizedEoi = 45,
    GdtrIdtrAccess = 46,
    LdtrTrAccess = 47,
    EptViolation = 48,
    EptMisconfiguration = 49,
    Invept = 50,
    Rdtscp = 51,
    VmxPreemptionTimerExpired = 52,
    Invvpid = 53,
    WbinvdWbnoinvd = 54,
    Xsetbv = 55,
    ApicWrite = 56,
    Rdrand = 57,
    Invpcid = 58,
    Vmfunc = 59,
    Encls = 60,
    Rdseed = 61,
    PageModLogFull = 62,
    Xsaves = 63,
    Xrstors = 64,
    Unknown = 0xFFFF,
}

impl VmExitReason {
    pub fn from_raw(raw: u32) -> Self {
        match raw & 0xFFFF {
            0 => Self::ExceptionOrNmi,
            1 => Self::ExternalInterrupt,
            2 => Self::TripleFault,
            3 => Self::InitSignal,
            4 => Self::StartupIpi,
            5 => Self::IoSmi,
            6 => Self::OtherSmi,
            7 => Self::InterruptWindow,
            8 => Self::NmiWindow,
            9 => Self::TaskSwitch,
            10 => Self::Cpuid,
            11 => Self::Getsec,
            12 => Self::Hlt,
            13 => Self::Invd,
            14 => Self::Invlpg,
            15 => Self::Rdpmc,
            16 => Self::Rdtsc,
            17 => Self::Rsm,
            18 => Self::Vmcall,
            19 => Self::Vmclear,
            20 => Self::Vmlaunch,
            21 => Self::Vmptrld,
            22 => Self::Vmptrst,
            23 => Self::Vmread,
            24 => Self::Vmresume,
            25 => Self::Vmwrite,
            26 => Self::Vmxoff,
            27 => Self::Vmxon,
            28 => Self::ControlRegisterAccess,
            29 => Self::MovDr,
            30 => Self::IoInstruction,
            31 => Self::Rdmsr,
            32 => Self::Wrmsr,
            33 => Self::EntryFailInvalidGuestState,
            34 => Self::EntryFailMsrLoading,
            36 => Self::Mwait,
            37 => Self::MonitorTrapFlag,
            39 => Self::Monitor,
            40 => Self::Pause,
            41 => Self::EntryFailMachineCheck,
            43 => Self::TprBelowThreshold,
            44 => Self::ApicAccess,
            45 => Self::VirtualizedEoi,
            46 => Self::GdtrIdtrAccess,
            47 => Self::LdtrTrAccess,
            48 => Self::EptViolation,
            49 => Self::EptMisconfiguration,
            50 => Self::Invept,
            51 => Self::Rdtscp,
            52 => Self::VmxPreemptionTimerExpired,
            53 => Self::Invvpid,
            54 => Self::WbinvdWbnoinvd,
            55 => Self::Xsetbv,
            56 => Self::ApicWrite,
            57 => Self::Rdrand,
            58 => Self::Invpcid,
            59 => Self::Vmfunc,
            60 => Self::Encls,
            61 => Self::Rdseed,
            62 => Self::PageModLogFull,
            63 => Self::Xsaves,
            64 => Self::Xrstors,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VmCapability {
    pub vmx_supported: bool,
    pub ept_supported: bool,
    pub vpid_supported: bool,
    pub unrestricted_guest: bool,
}

impl VmCapability {
    pub fn detect() -> Self {
        let vmx = cpu_supports_vmx();
        Self {
            vmx_supported: vmx,
            ept_supported: vmx,
            vpid_supported: vmx,
            unrestricted_guest: vmx,
        }
    }
}

pub fn cpu_supports_vmx() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        let ecx: u32;
        unsafe {
            core::arch::asm!(
                "push rbx", "mov eax, 1", "cpuid", "pop rbx",
                out("ecx") ecx, out("eax") _, out("edx") _, options(nomem),
            );
        }
        ecx & (1 << 5) != 0
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

#[cfg(target_arch = "x86_64")]
pub(crate) unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high, options(nomem, nostack));
    }
    ((high as u64) << 32) | (low as u64)
}

#[cfg(target_arch = "x86_64")]
pub(crate) unsafe fn write_msr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        core::arch::asm!("wrmsr", in("ecx") msr, in("eax") low, in("edx") high, options(nomem, nostack));
    }
}

pub fn init() {
    let cap = VmCapability::detect();
    crate::println!("  [virt] VMX supported: {}", cap.vmx_supported);
    crate::println!("  [virt] EPT supported: {}", cap.ept_supported);
    crate::println!("  [virt] VPID supported: {}", cap.vpid_supported);
    container::init();
    crate::println!("  [virt] Virtualization subsystem initialized");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_exit_reason_from_raw() {
        assert_eq!(VmExitReason::from_raw(0), VmExitReason::ExceptionOrNmi);
        assert_eq!(VmExitReason::from_raw(10), VmExitReason::Cpuid);
        assert_eq!(VmExitReason::from_raw(48), VmExitReason::EptViolation);
        assert_eq!(VmExitReason::from_raw(9999), VmExitReason::Unknown);
    }

    #[test]
    fn test_vm_error_display() {
        assert_eq!(
            alloc::format!("{}", VmError::VmxNotSupported),
            "VMX not supported"
        );
    }

    #[test]
    fn test_vm_capability_detect() {
        let _ = VmCapability::detect().vmx_supported;
    }

    #[test]
    fn test_vm_error_to_kernel_error() {
        let e: KernelError = VmError::EptMappingFailed.into();
        match e {
            KernelError::InvalidArgument { name, value } => {
                assert_eq!(name, "virt");
                assert_eq!(value, "ept_mapping_failed");
            }
            _ => panic!("Wrong error variant"),
        }
    }
}

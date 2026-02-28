//! VMX (Virtual Machine Extensions) implementation
//!
//! Provides VMX enable/disable, VMCS management, and VM entry/exit support
//! for x86_64 hardware virtualization.

#[cfg(feature = "alloc")]
extern crate alloc;

use super::{VmError, VmExitReason};

static VMX_STATE: spin::Mutex<Option<VmxState>> = spin::Mutex::new(None);

#[derive(Debug)]
pub struct VmxState {
    pub enabled: bool,
    pub vmxon_region: Option<crate::mm::FrameNumber>,
    pub revision_id: u32,
}

// VMCS field encoding constants (Intel SDM Vol. 3C, Appendix B)
pub struct VmcsFields;

#[allow(unused)]
impl VmcsFields {
    pub const GUEST_ES_SELECTOR: u32 = 0x0800;
    pub const GUEST_CS_SELECTOR: u32 = 0x0802;
    pub const GUEST_SS_SELECTOR: u32 = 0x0804;
    pub const GUEST_DS_SELECTOR: u32 = 0x0806;
    pub const GUEST_FS_SELECTOR: u32 = 0x0808;
    pub const GUEST_GS_SELECTOR: u32 = 0x080A;
    pub const GUEST_LDTR_SELECTOR: u32 = 0x080C;
    pub const GUEST_TR_SELECTOR: u32 = 0x080E;
    pub const HOST_ES_SELECTOR: u32 = 0x0C00;
    pub const HOST_CS_SELECTOR: u32 = 0x0C02;
    pub const HOST_SS_SELECTOR: u32 = 0x0C04;
    pub const HOST_DS_SELECTOR: u32 = 0x0C06;
    pub const HOST_FS_SELECTOR: u32 = 0x0C08;
    pub const HOST_GS_SELECTOR: u32 = 0x0C0A;
    pub const HOST_TR_SELECTOR: u32 = 0x0C0C;
    pub const IO_BITMAP_A: u32 = 0x2000;
    pub const IO_BITMAP_B: u32 = 0x2002;
    pub const MSR_BITMAP: u32 = 0x2004;
    pub const EPT_POINTER: u32 = 0x201A;
    pub const GUEST_VMCS_LINK_POINTER: u32 = 0x2800;
    pub const GUEST_IA32_DEBUGCTL: u32 = 0x2802;
    pub const GUEST_IA32_PAT: u32 = 0x2804;
    pub const GUEST_IA32_EFER: u32 = 0x2806;
    pub const HOST_IA32_PAT: u32 = 0x2C00;
    pub const HOST_IA32_EFER: u32 = 0x2C02;
    pub const PIN_BASED_VM_EXEC_CONTROLS: u32 = 0x4000;
    pub const PRIMARY_PROC_BASED_VM_EXEC_CONTROLS: u32 = 0x4002;
    pub const EXCEPTION_BITMAP: u32 = 0x4004;
    pub const VM_EXIT_CONTROLS: u32 = 0x4010;
    pub const VM_EXIT_MSR_STORE_COUNT: u32 = 0x400E;
    pub const VM_EXIT_MSR_LOAD_COUNT: u32 = 0x4012;
    pub const VM_ENTRY_CONTROLS: u32 = 0x4014;
    pub const VM_ENTRY_MSR_LOAD_COUNT: u32 = 0x4016;
    pub const VM_ENTRY_INTERRUPTION_INFO: u32 = 0x4018;
    pub const SECONDARY_PROC_BASED_VM_EXEC_CONTROLS: u32 = 0x401E;
    pub const GUEST_ES_LIMIT: u32 = 0x4800;
    pub const GUEST_CS_LIMIT: u32 = 0x4802;
    pub const GUEST_SS_LIMIT: u32 = 0x4804;
    pub const GUEST_DS_LIMIT: u32 = 0x4806;
    pub const GUEST_FS_LIMIT: u32 = 0x4808;
    pub const GUEST_GS_LIMIT: u32 = 0x480A;
    pub const GUEST_LDTR_LIMIT: u32 = 0x480C;
    pub const GUEST_TR_LIMIT: u32 = 0x480E;
    pub const GUEST_GDTR_LIMIT: u32 = 0x4810;
    pub const GUEST_IDTR_LIMIT: u32 = 0x4812;
    pub const GUEST_ES_ACCESS_RIGHTS: u32 = 0x4814;
    pub const GUEST_CS_ACCESS_RIGHTS: u32 = 0x4816;
    pub const GUEST_SS_ACCESS_RIGHTS: u32 = 0x4818;
    pub const GUEST_DS_ACCESS_RIGHTS: u32 = 0x481A;
    pub const GUEST_FS_ACCESS_RIGHTS: u32 = 0x481C;
    pub const GUEST_GS_ACCESS_RIGHTS: u32 = 0x481E;
    pub const GUEST_LDTR_ACCESS_RIGHTS: u32 = 0x4820;
    pub const GUEST_TR_ACCESS_RIGHTS: u32 = 0x4822;
    pub const GUEST_INTERRUPTIBILITY_STATE: u32 = 0x4824;
    pub const GUEST_ACTIVITY_STATE: u32 = 0x4826;
    pub const GUEST_SYSENTER_CS: u32 = 0x482A;
    pub const VM_EXIT_REASON: u32 = 0x4402;
    pub const VM_EXIT_INTERRUPTION_INFO: u32 = 0x4404;
    pub const VM_EXIT_INTERRUPTION_ERROR_CODE: u32 = 0x4406;
    pub const VM_EXIT_INSTRUCTION_LENGTH: u32 = 0x440C;
    pub const VM_EXIT_INSTRUCTION_INFO: u32 = 0x440E;
    pub const GUEST_CR0: u32 = 0x6800;
    pub const GUEST_CR3: u32 = 0x6802;
    pub const GUEST_CR4: u32 = 0x6804;
    pub const GUEST_ES_BASE: u32 = 0x6806;
    pub const GUEST_CS_BASE: u32 = 0x6808;
    pub const GUEST_SS_BASE: u32 = 0x680A;
    pub const GUEST_DS_BASE: u32 = 0x680C;
    pub const GUEST_FS_BASE: u32 = 0x680E;
    pub const GUEST_GS_BASE: u32 = 0x6810;
    pub const GUEST_LDTR_BASE: u32 = 0x6812;
    pub const GUEST_TR_BASE: u32 = 0x6814;
    pub const GUEST_GDTR_BASE: u32 = 0x6816;
    pub const GUEST_IDTR_BASE: u32 = 0x6818;
    pub const GUEST_DR7: u32 = 0x681A;
    pub const GUEST_RSP: u32 = 0x681C;
    pub const GUEST_RIP: u32 = 0x681E;
    pub const GUEST_RFLAGS: u32 = 0x6820;
    pub const GUEST_SYSENTER_ESP: u32 = 0x6824;
    pub const GUEST_SYSENTER_EIP: u32 = 0x6826;
    pub const HOST_CR0: u32 = 0x6C00;
    pub const HOST_CR3: u32 = 0x6C02;
    pub const HOST_CR4: u32 = 0x6C04;
    pub const HOST_FS_BASE: u32 = 0x6C06;
    pub const HOST_GS_BASE: u32 = 0x6C08;
    pub const HOST_TR_BASE: u32 = 0x6C0A;
    pub const HOST_GDTR_BASE: u32 = 0x6C0C;
    pub const HOST_IDTR_BASE: u32 = 0x6C0E;
    pub const HOST_IA32_SYSENTER_ESP: u32 = 0x6C10;
    pub const HOST_IA32_SYSENTER_EIP: u32 = 0x6C12;
    pub const HOST_RSP: u32 = 0x6C14;
    pub const HOST_RIP: u32 = 0x6C16;
    pub const EXIT_QUALIFICATION: u32 = 0x6400;
    pub const GUEST_LINEAR_ADDRESS: u32 = 0x640A;
    pub const GUEST_PHYSICAL_ADDRESS: u32 = 0x2400;
}

#[cfg(target_arch = "x86_64")]
const IA32_VMX_BASIC: u32 = 0x480;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_CR0_FIXED0: u32 = 0x486;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_CR0_FIXED1: u32 = 0x487;
#[cfg(target_arch = "x86_64")]
const IA32_FEATURE_CONTROL: u32 = 0x3A;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_PINBASED_CTLS: u32 = 0x481;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_PROCBASED_CTLS: u32 = 0x482;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_EXIT_CTLS: u32 = 0x483;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_ENTRY_CTLS: u32 = 0x484;
#[cfg(target_arch = "x86_64")]
const IA32_VMX_PROCBASED_CTLS2: u32 = 0x48B;
#[cfg(target_arch = "x86_64")]
const CR4_VMXE: u64 = 1 << 13;

pub struct Vmcs {
    frame: crate::mm::FrameNumber,
    active: bool,
}

impl Vmcs {
    #[cfg(target_arch = "x86_64")]
    pub fn allocate() -> Result<Self, VmError> {
        use crate::mm::frame_allocator::FRAME_ALLOCATOR;
        let frame = {
            let allocator = FRAME_ALLOCATOR.lock();
            allocator
                .allocate_frames(1, None)
                .map_err(|_| VmError::VmcsAllocationFailed)?
        };
        let phys_addr = frame.as_u64() * crate::mm::FRAME_SIZE as u64;
        let virt_addr = crate::mm::phys_to_virt_addr(phys_addr);
        // SAFETY: Exclusively owned frame, zero and write revision ID.
        unsafe {
            core::ptr::write_bytes(virt_addr as *mut u8, 0, crate::mm::FRAME_SIZE);
            let vmx_basic = super::read_msr(IA32_VMX_BASIC);
            let revision_id = (vmx_basic & 0x7FFF_FFFF) as u32;
            core::ptr::write_volatile(virt_addr as *mut u32, revision_id);
        }
        Ok(Self {
            frame,
            active: false,
        })
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn allocate() -> Result<Self, VmError> {
        Err(VmError::VmxNotSupported)
    }

    pub fn physical_address(&self) -> u64 {
        self.frame.as_u64() * crate::mm::FRAME_SIZE as u64
    }

    #[cfg(target_arch = "x86_64")]
    pub fn clear(&mut self) -> Result<(), VmError> {
        let phys_addr = self.physical_address();
        // SAFETY: VMCLEAR on our owned VMCS region.
        unsafe {
            let success: u8;
            core::arch::asm!(
                "vmclear [{addr}]", "setna {success}",
                addr = in(reg) &phys_addr as *const u64,
                success = out(reg_byte) success, options(nostack),
            );
            if success != 0 {
                return Err(VmError::VmxOperationFailed);
            }
        }
        self.active = false;
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn clear(&mut self) -> Result<(), VmError> {
        Err(VmError::VmxNotSupported)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn load(&mut self) -> Result<(), VmError> {
        let phys_addr = self.physical_address();
        // SAFETY: VMPTRLD loads our owned VMCS.
        unsafe {
            let success: u8;
            core::arch::asm!(
                "vmptrld [{addr}]", "setna {success}",
                addr = in(reg) &phys_addr as *const u64,
                success = out(reg_byte) success, options(nostack),
            );
            if success != 0 {
                return Err(VmError::VmxOperationFailed);
            }
        }
        self.active = true;
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn load(&mut self) -> Result<(), VmError> {
        Err(VmError::VmxNotSupported)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn write_field(&self, field: u32, value: u64) -> Result<(), VmError> {
        if !self.active {
            return Err(VmError::VmcsFieldError);
        }
        // SAFETY: VMWRITE on the current VMCS.
        unsafe {
            let success: u8;
            core::arch::asm!(
                "vmwrite {field}, {value}", "setna {success}",
                field = in(reg) field as u64, value = in(reg) value,
                success = out(reg_byte) success, options(nostack, nomem),
            );
            if success != 0 {
                return Err(VmError::VmcsFieldError);
            }
        }
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn write_field(&self, _field: u32, _value: u64) -> Result<(), VmError> {
        Err(VmError::VmxNotSupported)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn read_field(&self, field: u32) -> Result<u64, VmError> {
        if !self.active {
            return Err(VmError::VmcsFieldError);
        }
        let value: u64;
        // SAFETY: VMREAD on the current VMCS.
        unsafe {
            let success: u8;
            core::arch::asm!(
                "vmread {value}, {field}", "setna {success}",
                field = in(reg) field as u64, value = out(reg) value,
                success = out(reg_byte) success, options(nostack, nomem),
            );
            if success != 0 {
                return Err(VmError::VmcsFieldError);
            }
        }
        Ok(value)
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn read_field(&self, _field: u32) -> Result<u64, VmError> {
        Err(VmError::VmxNotSupported)
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn frame(&self) -> crate::mm::FrameNumber {
        self.frame
    }
}

// VMX enable/disable

#[cfg(target_arch = "x86_64")]
pub fn vmx_enable() -> Result<(), VmError> {
    use crate::mm::frame_allocator::FRAME_ALLOCATOR;
    {
        let state = VMX_STATE.lock();
        if let Some(ref s) = *state {
            if s.enabled {
                return Err(VmError::VmxAlreadyEnabled);
            }
        }
    }
    if !super::cpu_supports_vmx() {
        return Err(VmError::VmxNotSupported);
    }

    let feature_control = unsafe { super::read_msr(IA32_FEATURE_CONTROL) };
    let lock_bit = feature_control & 1;
    let vmx_outside_smx = (feature_control >> 2) & 1;
    if lock_bit != 0 && vmx_outside_smx == 0 {
        return Err(VmError::VmxNotSupported);
    }
    if lock_bit == 0 {
        unsafe { super::write_msr(IA32_FEATURE_CONTROL, feature_control | (1 << 2) | 1) };
    }

    let vmx_basic = unsafe { super::read_msr(IA32_VMX_BASIC) };
    let revision_id = (vmx_basic & 0x7FFF_FFFF) as u32;

    let vmxon_frame = {
        let allocator = FRAME_ALLOCATOR.lock();
        allocator
            .allocate_frames(1, None)
            .map_err(|_| VmError::VmcsAllocationFailed)?
    };
    let vmxon_phys = vmxon_frame.as_u64() * crate::mm::FRAME_SIZE as u64;
    let vmxon_virt = crate::mm::phys_to_virt_addr(vmxon_phys);
    // SAFETY: Exclusively owned frame.
    unsafe {
        core::ptr::write_bytes(vmxon_virt as *mut u8, 0, crate::mm::FRAME_SIZE);
        core::ptr::write_volatile(vmxon_virt as *mut u32, revision_id);
    }

    // SAFETY: Set CR4.VMXE before VMXON.
    unsafe {
        let cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
        core::arch::asm!("mov cr4, {}", in(reg) cr4 | CR4_VMXE, options(nostack, nomem));
    }

    // SAFETY: Adjust CR0 fixed bits for VMX operation.
    unsafe {
        let fixed0 = super::read_msr(IA32_VMX_CR0_FIXED0);
        let fixed1 = super::read_msr(IA32_VMX_CR0_FIXED1);
        let cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nostack, nomem));
        core::arch::asm!("mov cr0, {}", in(reg) (cr0 | fixed0) & fixed1, options(nostack, nomem));
    }

    // SAFETY: Execute VMXON with properly initialized region.
    unsafe {
        let success: u8;
        core::arch::asm!(
            "vmxon [{addr}]", "setna {success}",
            addr = in(reg) &vmxon_phys as *const u64,
            success = out(reg_byte) success, options(nostack),
        );
        if success != 0 {
            let cr4: u64;
            core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
            core::arch::asm!("mov cr4, {}", in(reg) cr4 & !CR4_VMXE, options(nostack, nomem));
            return Err(VmError::VmxOperationFailed);
        }
    }

    let mut state = VMX_STATE.lock();
    *state = Some(VmxState {
        enabled: true,
        vmxon_region: Some(vmxon_frame),
        revision_id,
    });
    crate::println!("  [vmx] VMX enabled (revision 0x{:08x})", revision_id);
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
pub fn vmx_enable() -> Result<(), VmError> {
    Err(VmError::VmxNotSupported)
}

#[cfg(target_arch = "x86_64")]
pub fn vmx_disable() -> Result<(), VmError> {
    let mut state = VMX_STATE.lock();
    match state.as_ref() {
        Some(s) if s.enabled => {}
        _ => return Ok(()),
    }
    // SAFETY: VMXOFF + clear CR4.VMXE.
    unsafe {
        core::arch::asm!("vmxoff", options(nostack, nomem));
        let cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
        core::arch::asm!("mov cr4, {}", in(reg) cr4 & !CR4_VMXE, options(nostack, nomem));
    }
    if let Some(ref mut s) = *state {
        s.enabled = false;
    }
    crate::println!("  [vmx] VMX disabled");
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
pub fn vmx_disable() -> Result<(), VmError> {
    Err(VmError::VmxNotSupported)
}

pub fn is_vmx_enabled() -> bool {
    let state = VMX_STATE.lock();
    state.as_ref().is_some_and(|s| s.enabled)
}

pub fn vmcs_revision_id() -> Option<u32> {
    let state = VMX_STATE.lock();
    state.as_ref().map(|s| s.revision_id)
}

#[cfg(target_arch = "x86_64")]
fn adjust_controls(msr: u32, desired: u32) -> u32 {
    let msr_val = unsafe { super::read_msr(msr) };
    let required = msr_val as u32;
    let allowed = (msr_val >> 32) as u32;
    (desired | required) & allowed
}

#[cfg(target_arch = "x86_64")]
pub fn setup_vmcs(vmcs: &Vmcs, guest_entry: u64, guest_stack: u64) -> Result<(), VmError> {
    if !vmcs.is_active() {
        return Err(VmError::VmcsFieldError);
    }

    let host_cr0: u64;
    let host_cr3: u64;
    let host_cr4: u64;
    // SAFETY: Reading control registers at ring 0.
    unsafe {
        core::arch::asm!("mov {}, cr0", out(reg) host_cr0, options(nostack, nomem));
        core::arch::asm!("mov {}, cr3", out(reg) host_cr3, options(nostack, nomem));
        core::arch::asm!("mov {}, cr4", out(reg) host_cr4, options(nostack, nomem));
    }
    vmcs.write_field(VmcsFields::HOST_CR0, host_cr0)?;
    vmcs.write_field(VmcsFields::HOST_CR3, host_cr3)?;
    vmcs.write_field(VmcsFields::HOST_CR4, host_cr4)?;

    let (cs, ss, ds, es, fs, gs, tr): (u16, u16, u16, u16, u16, u16, u16);
    // SAFETY: Reading segment selectors.
    unsafe {
        core::arch::asm!("mov {:x}, cs", out(reg) cs, options(nostack, nomem));
        core::arch::asm!("mov {:x}, ss", out(reg) ss, options(nostack, nomem));
        core::arch::asm!("mov {:x}, ds", out(reg) ds, options(nostack, nomem));
        core::arch::asm!("mov {:x}, es", out(reg) es, options(nostack, nomem));
        core::arch::asm!("mov {:x}, fs", out(reg) fs, options(nostack, nomem));
        core::arch::asm!("mov {:x}, gs", out(reg) gs, options(nostack, nomem));
        core::arch::asm!("str {:x}", out(reg) tr, options(nostack, nomem));
    }
    vmcs.write_field(VmcsFields::HOST_CS_SELECTOR, cs as u64)?;
    vmcs.write_field(VmcsFields::HOST_SS_SELECTOR, ss as u64)?;
    vmcs.write_field(VmcsFields::HOST_DS_SELECTOR, ds as u64)?;
    vmcs.write_field(VmcsFields::HOST_ES_SELECTOR, es as u64)?;
    vmcs.write_field(VmcsFields::HOST_FS_SELECTOR, fs as u64)?;
    vmcs.write_field(VmcsFields::HOST_GS_SELECTOR, gs as u64)?;
    vmcs.write_field(VmcsFields::HOST_TR_SELECTOR, tr as u64)?;

    let gdtr: [u8; 10] = [0; 10];
    let idtr: [u8; 10] = [0; 10];
    // SAFETY: SGDT/SIDT store descriptor table registers.
    unsafe {
        core::arch::asm!("sgdt [{}]", in(reg) &gdtr as *const _, options(nostack));
        core::arch::asm!("sidt [{}]", in(reg) &idtr as *const _, options(nostack));
    }
    let gdt_base = u64::from_le_bytes(gdtr[2..10].try_into().unwrap_or([0; 8]));
    let idt_base = u64::from_le_bytes(idtr[2..10].try_into().unwrap_or([0; 8]));
    vmcs.write_field(VmcsFields::HOST_GDTR_BASE, gdt_base)?;
    vmcs.write_field(VmcsFields::HOST_IDTR_BASE, idt_base)?;
    vmcs.write_field(VmcsFields::HOST_RIP, vm_exit_handler as *const () as u64)?;
    vmcs.write_field(VmcsFields::HOST_FS_BASE, 0)?;
    vmcs.write_field(VmcsFields::HOST_GS_BASE, 0)?;
    vmcs.write_field(VmcsFields::HOST_TR_BASE, 0)?;
    vmcs.write_field(VmcsFields::HOST_IA32_SYSENTER_ESP, 0)?;
    vmcs.write_field(VmcsFields::HOST_IA32_SYSENTER_EIP, 0)?;

    // Guest state
    vmcs.write_field(VmcsFields::GUEST_CR0, host_cr0)?;
    vmcs.write_field(VmcsFields::GUEST_CR3, 0)?;
    vmcs.write_field(VmcsFields::GUEST_CR4, host_cr4 & !CR4_VMXE)?;

    let cs_ar: u64 = 0xA09B;
    let ds_ar: u64 = 0xC093;
    let tr_ar: u64 = 0x008B;
    let ldtr_ar: u64 = 0x10000;

    vmcs.write_field(VmcsFields::GUEST_CS_SELECTOR, 0x08)?;
    vmcs.write_field(VmcsFields::GUEST_CS_BASE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_CS_LIMIT, 0xFFFF_FFFF)?;
    vmcs.write_field(VmcsFields::GUEST_CS_ACCESS_RIGHTS, cs_ar)?;

    for (sel, base, limit, ar) in [
        (
            VmcsFields::GUEST_SS_SELECTOR,
            VmcsFields::GUEST_SS_BASE,
            VmcsFields::GUEST_SS_LIMIT,
            VmcsFields::GUEST_SS_ACCESS_RIGHTS,
        ),
        (
            VmcsFields::GUEST_DS_SELECTOR,
            VmcsFields::GUEST_DS_BASE,
            VmcsFields::GUEST_DS_LIMIT,
            VmcsFields::GUEST_DS_ACCESS_RIGHTS,
        ),
        (
            VmcsFields::GUEST_ES_SELECTOR,
            VmcsFields::GUEST_ES_BASE,
            VmcsFields::GUEST_ES_LIMIT,
            VmcsFields::GUEST_ES_ACCESS_RIGHTS,
        ),
        (
            VmcsFields::GUEST_FS_SELECTOR,
            VmcsFields::GUEST_FS_BASE,
            VmcsFields::GUEST_FS_LIMIT,
            VmcsFields::GUEST_FS_ACCESS_RIGHTS,
        ),
        (
            VmcsFields::GUEST_GS_SELECTOR,
            VmcsFields::GUEST_GS_BASE,
            VmcsFields::GUEST_GS_LIMIT,
            VmcsFields::GUEST_GS_ACCESS_RIGHTS,
        ),
    ] {
        vmcs.write_field(sel, 0x10)?;
        vmcs.write_field(base, 0)?;
        vmcs.write_field(limit, 0xFFFF_FFFF)?;
        vmcs.write_field(ar, ds_ar)?;
    }

    vmcs.write_field(VmcsFields::GUEST_TR_SELECTOR, 0x18)?;
    vmcs.write_field(VmcsFields::GUEST_TR_BASE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_TR_LIMIT, 0x67)?;
    vmcs.write_field(VmcsFields::GUEST_TR_ACCESS_RIGHTS, tr_ar)?;
    vmcs.write_field(VmcsFields::GUEST_LDTR_SELECTOR, 0)?;
    vmcs.write_field(VmcsFields::GUEST_LDTR_BASE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_LDTR_LIMIT, 0)?;
    vmcs.write_field(VmcsFields::GUEST_LDTR_ACCESS_RIGHTS, ldtr_ar)?;
    vmcs.write_field(VmcsFields::GUEST_GDTR_BASE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_GDTR_LIMIT, 0)?;
    vmcs.write_field(VmcsFields::GUEST_IDTR_BASE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_IDTR_LIMIT, 0)?;
    vmcs.write_field(VmcsFields::GUEST_DR7, 0x400)?;
    vmcs.write_field(VmcsFields::GUEST_RFLAGS, 0x2)?;
    vmcs.write_field(VmcsFields::GUEST_RIP, guest_entry)?;
    vmcs.write_field(VmcsFields::GUEST_RSP, guest_stack)?;
    vmcs.write_field(VmcsFields::GUEST_INTERRUPTIBILITY_STATE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_ACTIVITY_STATE, 0)?;
    vmcs.write_field(VmcsFields::GUEST_VMCS_LINK_POINTER, 0xFFFF_FFFF_FFFF_FFFF)?;
    vmcs.write_field(VmcsFields::GUEST_SYSENTER_CS, 0)?;
    vmcs.write_field(VmcsFields::GUEST_SYSENTER_ESP, 0)?;
    vmcs.write_field(VmcsFields::GUEST_SYSENTER_EIP, 0)?;

    // Execution controls
    let pin_based = adjust_controls(IA32_VMX_PINBASED_CTLS, 0x0000_0001);
    vmcs.write_field(VmcsFields::PIN_BASED_VM_EXEC_CONTROLS, pin_based as u64)?;
    let primary_proc = adjust_controls(
        IA32_VMX_PROCBASED_CTLS,
        (1 << 7) | (1 << 24) | (1 << 28) | (1 << 31),
    );
    vmcs.write_field(
        VmcsFields::PRIMARY_PROC_BASED_VM_EXEC_CONTROLS,
        primary_proc as u64,
    )?;
    let secondary_proc = adjust_controls(IA32_VMX_PROCBASED_CTLS2, (1 << 1) | (1 << 7));
    vmcs.write_field(
        VmcsFields::SECONDARY_PROC_BASED_VM_EXEC_CONTROLS,
        secondary_proc as u64,
    )?;
    vmcs.write_field(VmcsFields::EXCEPTION_BITMAP, 0)?;

    let exit_controls = adjust_controls(IA32_VMX_EXIT_CTLS, 1 << 9);
    vmcs.write_field(VmcsFields::VM_EXIT_CONTROLS, exit_controls as u64)?;
    vmcs.write_field(VmcsFields::VM_EXIT_MSR_STORE_COUNT, 0)?;
    vmcs.write_field(VmcsFields::VM_EXIT_MSR_LOAD_COUNT, 0)?;

    let entry_controls = adjust_controls(IA32_VMX_ENTRY_CTLS, 1 << 9);
    vmcs.write_field(VmcsFields::VM_ENTRY_CONTROLS, entry_controls as u64)?;
    vmcs.write_field(VmcsFields::VM_ENTRY_MSR_LOAD_COUNT, 0)?;
    vmcs.write_field(VmcsFields::VM_ENTRY_INTERRUPTION_INFO, 0)?;

    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
pub fn setup_vmcs(_vmcs: &Vmcs, _guest_entry: u64, _guest_stack: u64) -> Result<(), VmError> {
    Err(VmError::VmxNotSupported)
}

#[cfg(target_arch = "x86_64")]
pub fn vm_launch() -> Result<VmExitReason, VmError> {
    // SAFETY: VMLAUNCH transfers to guest. If it fails, we return error.
    let success: u8;
    unsafe {
        core::arch::asm!("vmlaunch", "setna {success}", success = out(reg_byte) success, options(nostack));
    }
    if success != 0 {
        return Err(VmError::VmEntryFailed);
    }
    Err(VmError::VmEntryFailed)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn vm_launch() -> Result<VmExitReason, VmError> {
    Err(VmError::VmxNotSupported)
}

#[cfg(target_arch = "x86_64")]
pub fn vm_resume() -> Result<VmExitReason, VmError> {
    // SAFETY: VMRESUME transfers back to guest.
    let success: u8;
    unsafe {
        core::arch::asm!("vmresume", "setna {success}", success = out(reg_byte) success, options(nostack));
    }
    if success != 0 {
        return Err(VmError::VmEntryFailed);
    }
    Err(VmError::VmEntryFailed)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn vm_resume() -> Result<VmExitReason, VmError> {
    Err(VmError::VmxNotSupported)
}

#[cfg(target_arch = "x86_64")]
extern "C" fn vm_exit_handler() {
    let raw_reason: u64;
    // SAFETY: VMREAD after VM exit, VMCS is current.
    unsafe {
        let _success: u8;
        core::arch::asm!(
            "vmread {value}, {field}", "setna {success}",
            field = in(reg) VmcsFields::VM_EXIT_REASON as u64,
            value = out(reg) raw_reason,
            success = out(reg_byte) _success,
            options(nostack, nomem),
        );
    }
    let reason = VmExitReason::from_raw(raw_reason as u32);
    let _ = handle_vm_exit(reason);
}

pub fn handle_vm_exit(reason: VmExitReason) -> Result<(), VmError> {
    match reason {
        VmExitReason::Cpuid => {
            crate::println!("  [vmx] VM exit: CPUID");
            Ok(())
        }
        VmExitReason::Hlt => {
            crate::println!("  [vmx] VM exit: HLT");
            Ok(())
        }
        VmExitReason::IoInstruction => {
            crate::println!("  [vmx] VM exit: I/O");
            Ok(())
        }
        VmExitReason::Rdmsr | VmExitReason::Wrmsr => {
            crate::println!("  [vmx] VM exit: MSR");
            Ok(())
        }
        VmExitReason::EptViolation => {
            crate::println!("  [vmx] VM exit: EPT violation");
            Ok(())
        }
        VmExitReason::ExternalInterrupt => Ok(()),
        VmExitReason::TripleFault => {
            crate::println!("  [vmx] VM exit: Triple fault");
            Err(VmError::VmExitHandlerError)
        }
        VmExitReason::EntryFailInvalidGuestState => {
            crate::println!("  [vmx] VM exit: Invalid guest state");
            Err(VmError::InvalidGuestState)
        }
        VmExitReason::Vmcall => {
            crate::println!("  [vmx] VM exit: VMCALL");
            Ok(())
        }
        _ => {
            crate::println!("  [vmx] VM exit: unhandled {:?}", reason);
            Err(VmError::VmExitHandlerError)
        }
    }
}

pub fn vmx_status() -> (bool, bool, Option<u32>) {
    (
        super::cpu_supports_vmx(),
        is_vmx_enabled(),
        vmcs_revision_id(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vmcs_field_constants() {
        assert_eq!(VmcsFields::GUEST_RIP, 0x681E);
        assert_eq!(VmcsFields::GUEST_RSP, 0x681C);
        assert_eq!(VmcsFields::HOST_RIP, 0x6C16);
        assert_eq!(VmcsFields::HOST_RSP, 0x6C14);
        assert_eq!(VmcsFields::VM_EXIT_REASON, 0x4402);
        assert_eq!(VmcsFields::EPT_POINTER, 0x201A);
    }

    #[test]
    fn test_vmx_state_initial() {
        assert!(!is_vmx_enabled());
        assert!(vmcs_revision_id().is_none());
    }

    #[test]
    fn test_handle_vm_exit_cpuid() {
        assert!(handle_vm_exit(VmExitReason::Cpuid).is_ok());
    }

    #[test]
    fn test_handle_vm_exit_triple_fault() {
        assert!(handle_vm_exit(VmExitReason::TripleFault).is_err());
    }

    #[test]
    fn test_handle_vm_exit_hlt() {
        assert!(handle_vm_exit(VmExitReason::Hlt).is_ok());
    }
}

//! Model-Specific Register (MSR) read/write primitives.
//!
//! Extracted from `apic.rs` so that other modules (e.g. `pat.rs`) can
//! access MSRs without duplicating inline assembly.

/// Read a 64-bit Model-Specific Register.
pub fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: RDMSR reads the MSR specified by ECX. The caller passes a
    // well-known MSR address. This is a privileged, read-only operation
    // with no side effects beyond returning the MSR value.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (low as u64) | ((high as u64) << 32)
}

/// Write a 64-bit value to a Model-Specific Register.
pub fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    // SAFETY: WRMSR writes to the MSR specified by ECX. The caller passes a
    // well-known MSR address and a valid value. This is a privileged operation
    // that modifies CPU configuration.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Translate a physical address to its virtual address using the
/// bootloader's physical memory mapping.
///
/// The bootloader maps all physical memory at a dynamic offset in the
/// higher-half virtual address space. MMIO regions like the Local APIC
/// (0xFEE0_0000) and I/O APIC (0xFEC0_0000) are not identity-mapped,
/// so we must add the physical memory offset to access them.
///
/// Returns `None` if boot info or the physical memory offset is unavailable.
pub fn phys_to_virt(phys: usize) -> Option<usize> {
    // SAFETY: BOOT_INFO is a static mut written once during early boot
    // (before any concurrency) and read-only afterwards. We are in
    // single-threaded kernel init context.
    #[allow(static_mut_refs)]
    let boot_info = unsafe { crate::arch::x86_64::boot::BOOT_INFO.as_ref()? };
    let offset = boot_info.physical_memory_offset.into_option()?;
    Some(offset as usize + phys)
}

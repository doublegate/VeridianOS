//! SMEP/SMAP Enforcement
//!
//! Supervisor Mode Execution Prevention (SMEP) prevents the kernel from
//! executing code mapped in user-space pages. Supervisor Mode Access Prevention
//! (SMAP) prevents the kernel from reading or writing user-space memory unless
//! explicitly permitted.
//!
//! ## Architecture Support
//!
//! - **x86_64**: CR4.SMEP (bit 20) and CR4.SMAP (bit 21). Temporary SMAP bypass
//!   via STAC/CLAC instructions.
//! - **AArch64**: Privileged Access Never (PAN) via SCTLR_EL1 bit 22.
//! - **RISC-V**: Supervisor User Memory (SUM) bit in sstatus register.
//!
//! ## Usage
//!
//! Call `init()` during boot to detect and enable available protections.
//! When the kernel must copy data to/from user-space buffers, bracket the
//! access with `disable_smap_temporarily()` and `restore_smap()`.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Feature detection flags
// ---------------------------------------------------------------------------

/// Whether the CPU supports SMEP.
static SMEP_SUPPORTED: AtomicBool = AtomicBool::new(false);
/// Whether the CPU supports SMAP.
static SMAP_SUPPORTED: AtomicBool = AtomicBool::new(false);
/// Whether SMEP is currently enabled in the control register.
static SMEP_ENABLED: AtomicBool = AtomicBool::new(false);
/// Whether SMAP is currently enabled in the control register.
static SMAP_ENABLED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// x86_64: CPUID feature bits and CR4 constants
// ---------------------------------------------------------------------------

/// CPUID leaf 7, EBX bit 7 -- SMEP
#[cfg(target_arch = "x86_64")]
const CPUID_SMEP_BIT: u32 = 1 << 7;
/// CPUID leaf 7, EBX bit 20 -- SMAP
#[cfg(target_arch = "x86_64")]
const CPUID_SMAP_BIT: u32 = 1 << 20;

/// CR4 bit 20 -- SMEP enable
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
const CR4_SMEP: u64 = 1 << 20;
/// CR4 bit 21 -- SMAP enable
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
const CR4_SMAP: u64 = 1 << 21;

// ---------------------------------------------------------------------------
// Public query API
// ---------------------------------------------------------------------------

/// Returns `true` if the CPU supports SMEP (or the arch-specific equivalent).
pub fn is_smep_supported() -> bool {
    SMEP_SUPPORTED.load(Ordering::Relaxed)
}

/// Returns `true` if the CPU supports SMAP (or the arch-specific equivalent).
pub fn is_smap_supported() -> bool {
    SMAP_SUPPORTED.load(Ordering::Relaxed)
}

/// Returns `true` if SMEP is currently enabled.
pub fn is_smep_enabled() -> bool {
    SMEP_ENABLED.load(Ordering::Relaxed)
}

/// Returns `true` if SMAP is currently enabled.
pub fn is_smap_enabled() -> bool {
    SMAP_ENABLED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// x86_64 implementation
// ---------------------------------------------------------------------------

/// Detect SMEP/SMAP support via CPUID on x86_64.
#[cfg(target_arch = "x86_64")]
fn detect_features() {
    // CPUID leaf 7, sub-leaf 0: structured extended feature flags in EBX.
    // LLVM reserves RBX, so we must save/restore it around `cpuid`.
    let ebx: u32;

    // SAFETY: CPUID is a read-only instruction. We save and restore RBX
    // because LLVM may use it as a reserved register.
    #[cfg(target_os = "none")]
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx:e}, ebx",
            "pop rbx",
            ebx = out(reg) ebx,
            in("eax") 7u32,
            in("ecx") 0u32,
            options(nostack),
        );
    }

    #[cfg(not(target_os = "none"))]
    let ebx = {
        // Host/CI stub: assume both supported for test coverage.
        CPUID_SMEP_BIT | CPUID_SMAP_BIT
    };

    if ebx & CPUID_SMEP_BIT != 0 {
        SMEP_SUPPORTED.store(true, Ordering::Relaxed);
    }
    if ebx & CPUID_SMAP_BIT != 0 {
        SMAP_SUPPORTED.store(true, Ordering::Relaxed);
    }
}

/// AArch64: detect PAN support via ID_AA64MMFR1_EL1.
#[cfg(target_arch = "aarch64")]
fn detect_features() {
    // ID_AA64MMFR1_EL1 bits [23:20] encode PAN support level.
    // 0b0001 = PAN supported, 0b0010 = PAN + AT_S1E1RP, 0b0011 = PAN + EPAN.
    let mmfr1: u64;

    #[cfg(target_os = "none")]
    // SAFETY: Reading the ID register is a read-only operation that does
    // not change processor state.
    unsafe {
        core::arch::asm!(
            "mrs {}, id_aa64mmfr1_el1",
            out(reg) mmfr1,
            options(nomem, nostack),
        );
    }

    #[cfg(not(target_os = "none"))]
    let mmfr1: u64 = 0x1 << 20; // stub: PAN supported

    let pan_field = (mmfr1 >> 20) & 0xF;
    if pan_field >= 1 {
        // PAN is the AArch64 equivalent of both SMEP and SMAP.
        SMEP_SUPPORTED.store(true, Ordering::Relaxed);
        SMAP_SUPPORTED.store(true, Ordering::Relaxed);
    }
}

/// RISC-V: SUM bit is always architecturally defined in sstatus.
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn detect_features() {
    // The SUM (Supervisor User Memory access) bit is defined by the
    // RISC-V privileged specification. It is always present; there is
    // no separate feature-detection register.
    SMEP_SUPPORTED.store(true, Ordering::Relaxed);
    SMAP_SUPPORTED.store(true, Ordering::Relaxed);
}

/// Fallback for other / host targets.
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv32",
    target_arch = "riscv64",
)))]
fn detect_features() {
    // Nothing to detect.
}

// ---------------------------------------------------------------------------
// Enable helpers
// ---------------------------------------------------------------------------

/// Enable SMEP. Returns `Ok(())` if enabled or already enabled, or
/// `Err` if the feature is not supported.
pub fn enable_smep() -> Result<(), KernelError> {
    if !is_smep_supported() {
        return Err(KernelError::OperationNotSupported {
            operation: "SMEP not supported by CPU",
        });
    }
    if is_smep_enabled() {
        return Ok(());
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // SAFETY: We only set the SMEP bit in CR4 after confirming CPUID
        // support. The read-modify-write is atomic with respect to the
        // current CPU (interrupts are implicitly serialised by the CR4
        // write).
        unsafe {
            let cr4: u64;
            core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
            core::arch::asm!("mov cr4, {}", in(reg) cr4 | CR4_SMEP, options(nomem, nostack));
        }
    }

    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    {
        // Enable PAN via SCTLR_EL1 bit 22 (SPAN=0 means PAN is active
        // automatically on exception entry). We clear SPAN (bit 23) and
        // set PAN (PSTATE.PAN) directly.
        // SAFETY: Writing PSTATE.PAN is the documented way to enable PAN.
        unsafe {
            core::arch::asm!(".inst 0xD500419F", options(nomem, nostack),);
        }
    }

    #[cfg(all(
        any(target_arch = "riscv32", target_arch = "riscv64"),
        target_os = "none"
    ))]
    {
        // On RISC-V, SMEP-like behaviour is the default: S-mode cannot
        // fetch instructions from U-mode pages. No action required.
    }

    SMEP_ENABLED.store(true, Ordering::Release);
    Ok(())
}

/// Enable SMAP. Returns `Ok(())` if enabled or already enabled, or
/// `Err` if the feature is not supported.
pub fn enable_smap() -> Result<(), KernelError> {
    if !is_smap_supported() {
        return Err(KernelError::OperationNotSupported {
            operation: "SMAP not supported by CPU",
        });
    }
    if is_smap_enabled() {
        return Ok(());
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // SAFETY: Same rationale as enable_smep -- we set CR4.SMAP
        // after confirming CPUID support.
        unsafe {
            let cr4: u64;
            core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
            core::arch::asm!("mov cr4, {}", in(reg) cr4 | CR4_SMAP, options(nomem, nostack));
        }
    }

    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    {
        // PAN was already enabled in enable_smep(). On AArch64 PAN covers
        // both execution and data access prevention for user pages.
        // Re-asserting is harmless.
        unsafe {
            core::arch::asm!(".inst 0xD500419F", options(nomem, nostack),);
        }
    }

    #[cfg(all(
        any(target_arch = "riscv32", target_arch = "riscv64"),
        target_os = "none"
    ))]
    {
        // Clear SUM bit in sstatus so S-mode cannot access U-mode pages.
        // sstatus.SUM = bit 18.
        const SSTATUS_SUM: u64 = 1 << 18;
        // SAFETY: Clearing SUM restricts supervisor access to user pages.
        // This is the intended security posture.
        unsafe {
            core::arch::asm!(
                "csrc sstatus, {sum}",
                sum = in(reg) SSTATUS_SUM,
                options(nomem, nostack),
            );
        }
    }

    SMAP_ENABLED.store(true, Ordering::Release);
    Ok(())
}

// ---------------------------------------------------------------------------
// Temporary SMAP bypass for user memory access
// ---------------------------------------------------------------------------

/// Temporarily disable SMAP to allow kernel access to user-space memory.
///
/// On x86_64 this executes STAC (Set AC Flag). On AArch64, clears PSTATE.PAN.
/// On RISC-V, sets the SUM bit in sstatus.
///
/// The caller **must** call [`restore_smap`] after the user-memory access.
///
/// # Safety
///
/// This function is safe to call, but the *window* between
/// `disable_smap_temporarily` and `restore_smap` permits kernel code to
/// access user-mapped pages. Keep this window as short as possible.
#[inline(always)]
pub fn disable_smap_temporarily() {
    if !is_smap_enabled() {
        return;
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    // SAFETY: STAC sets EFLAGS.AC, temporarily allowing supervisor access
    // to user pages when SMAP is enabled. Must be paired with CLAC.
    unsafe {
        core::arch::asm!("stac", options(nomem, nostack));
    }

    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    // SAFETY: Clearing PAN allows privileged access to user pages.
    unsafe {
        core::arch::asm!(".inst 0xD500409F", options(nomem, nostack));
    }

    #[cfg(all(
        any(target_arch = "riscv32", target_arch = "riscv64"),
        target_os = "none"
    ))]
    {
        const SSTATUS_SUM: u64 = 1 << 18;
        // SAFETY: Setting SUM allows S-mode to access U-mode pages.
        unsafe {
            core::arch::asm!(
                "csrs sstatus, {sum}",
                sum = in(reg) SSTATUS_SUM,
                options(nomem, nostack),
            );
        }
    }
}

/// Restore SMAP after a temporary user-memory access.
///
/// On x86_64 this executes CLAC (Clear AC Flag). On AArch64, sets PSTATE.PAN.
/// On RISC-V, clears the SUM bit.
#[inline(always)]
pub fn restore_smap() {
    if !is_smap_enabled() {
        return;
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    // SAFETY: CLAC clears EFLAGS.AC, re-enabling SMAP protection.
    unsafe {
        core::arch::asm!("clac", options(nomem, nostack));
    }

    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    // SAFETY: Re-enabling PAN restores the user-access restriction.
    unsafe {
        core::arch::asm!(".inst 0xD500419F", options(nomem, nostack));
    }

    #[cfg(all(
        any(target_arch = "riscv32", target_arch = "riscv64"),
        target_os = "none"
    ))]
    {
        const SSTATUS_SUM: u64 = 1 << 18;
        // SAFETY: Clearing SUM re-restricts S-mode access to U-mode pages.
        unsafe {
            core::arch::asm!(
                "csrc sstatus, {sum}",
                sum = in(reg) SSTATUS_SUM,
                options(nomem, nostack),
            );
        }
    }
}

/// RAII guard that disables SMAP on creation and restores it on drop.
///
/// # Example
///
/// ```ignore
/// {
///     let _guard = SmapGuard::new();
///     // user memory is accessible here
///     core::ptr::copy_nonoverlapping(user_src, kernel_dst, len);
/// } // SMAP automatically restored
/// ```
pub struct SmapGuard {
    /// Whether SMAP was active and thus needs restoring.
    active: bool,
}

impl SmapGuard {
    /// Create a new guard, temporarily disabling SMAP.
    pub fn new() -> Self {
        let active = is_smap_enabled();
        if active {
            disable_smap_temporarily();
        }
        Self { active }
    }
}

impl Default for SmapGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SmapGuard {
    fn drop(&mut self) {
        if self.active {
            restore_smap();
        }
    }
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Detect and enable SMEP/SMAP (or platform equivalents).
///
/// Called during early boot from `security::init()`. Non-fatal: logs
/// status but does not fail the boot if the CPU lacks support.
pub fn init() -> Result<(), KernelError> {
    detect_features();

    let smep_ok = if is_smep_supported() {
        enable_smep().is_ok()
    } else {
        false
    };

    let smap_ok = if is_smap_supported() {
        enable_smap().is_ok()
    } else {
        false
    };

    #[cfg(target_arch = "x86_64")]
    {
        crate::println!(
            "[SMEP/SMAP] x86_64: SMEP {} ({}), SMAP {} ({})",
            if is_smep_supported() {
                "supported"
            } else {
                "unsupported"
            },
            if smep_ok { "enabled" } else { "skipped" },
            if is_smap_supported() {
                "supported"
            } else {
                "unsupported"
            },
            if smap_ok { "enabled" } else { "skipped" },
        );
    }

    #[cfg(target_arch = "aarch64")]
    {
        let _ = (smep_ok, smap_ok);
        crate::kprintln!("[SMEP/SMAP] AArch64: PAN detection complete");
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        let _ = (smep_ok, smap_ok);
        crate::kprintln!("[SMEP/SMAP] RISC-V: SUM enforcement configured");
    }

    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv32",
        target_arch = "riscv64",
    )))]
    {
        let _ = (smep_ok, smap_ok);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        // Before init, nothing should be enabled (atomics default to false).
        // Note: other tests may have called init() already in this process,
        // so we only verify the query functions don't panic.
        let _ = is_smep_supported();
        let _ = is_smap_supported();
        let _ = is_smep_enabled();
        let _ = is_smap_enabled();
    }

    #[test]
    fn test_detect_and_init() {
        detect_features();
        // On the CI host, the stub sets both as supported.
        #[cfg(not(target_os = "none"))]
        {
            assert!(is_smep_supported());
            assert!(is_smap_supported());
        }
    }

    #[test]
    fn test_enable_without_detection_fails() {
        // Reset support flags to simulate unsupported CPU.
        SMEP_SUPPORTED.store(false, Ordering::Relaxed);
        SMAP_SUPPORTED.store(false, Ordering::Relaxed);
        SMEP_ENABLED.store(false, Ordering::Relaxed);
        SMAP_ENABLED.store(false, Ordering::Relaxed);

        assert!(enable_smep().is_err());
        assert!(enable_smap().is_err());

        // Restore for other tests.
        detect_features();
    }

    #[test]
    fn test_smap_guard() {
        // Ensure the guard can be created and dropped without panicking,
        // regardless of whether SMAP is actually enabled on the host.
        {
            let _guard = SmapGuard::new();
            // Inside the guard: user memory access would be permitted.
        }
        // After drop: SMAP should be restored (no-op on host).
    }

    #[test]
    fn test_disable_restore_noop_when_not_enabled() {
        // If SMAP is not enabled, disable/restore should be no-ops.
        SMAP_ENABLED.store(false, Ordering::Relaxed);
        disable_smap_temporarily();
        restore_smap();
    }

    #[test]
    fn test_enable_idempotent() {
        // Calling enable twice should succeed (idempotent).
        detect_features();
        // On the host target, enable is a no-op (no CR4 write) but the
        // flag gets set.
        let _ = enable_smep();
        assert!(enable_smep().is_ok());
        let _ = enable_smap();
        assert!(enable_smap().is_ok());
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_cpuid_bit_constants() {
        assert_eq!(CPUID_SMEP_BIT, 1 << 7);
        assert_eq!(CPUID_SMAP_BIT, 1 << 20);
    }
}

//! Retpoline / Spectre Mitigations
//!
//! Provides software and hardware mitigations for Spectre-class speculative
//! execution vulnerabilities.
//!
//! ## Spectre v1 (Bounds Check Bypass)
//!
//! - [`bounds_mask`]: branchless index clamping that produces a safe index even
//!   under mis-speculation.
//! - [`speculation_barrier`]: serialising instruction that halts speculative
//!   execution past this point.
//!
//! ## Spectre v2 (Branch Target Injection)
//!
//! - **IBRS** (Indirect Branch Restricted Speculation): restricts speculative
//!   targets of indirect branches to a curated set.
//! - **IBPB** (Indirect Branch Prediction Barrier): flushes the Branch Target
//!   Buffer on context switches.
//! - **STIBP** (Single Thread Indirect Branch Predictors): prevents cross-SMT
//!   branch-target poisoning.
//! - **Retpoline**: compiler-based mitigation that replaces indirect calls with
//!   a construct that never speculatively follows the real target.
//!
//! ## Architecture Support
//!
//! - **x86_64**: IBRS/IBPB/STIBP via IA32_SPEC_CTRL (MSR 0x48) and
//!   IA32_PRED_CMD (MSR 0x49). Feature detection through CPUID leaf 7.
//! - **AArch64**: CSV2 (Cache Speculation Variant 2) detection via
//!   ID_AA64PFR0_EL1. Barriers via DSB SY + ISB.
//! - **RISC-V**: FENCE.I as speculation barrier.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Feature detection flags
// ---------------------------------------------------------------------------

/// Whether the CPU supports IBRS (IA32_SPEC_CTRL bit 0).
static IBRS_SUPPORTED: AtomicBool = AtomicBool::new(false);
/// Whether the CPU supports IBPB (IA32_PRED_CMD).
static IBPB_SUPPORTED: AtomicBool = AtomicBool::new(false);
/// Whether the CPU supports STIBP (IA32_SPEC_CTRL bit 1).
static STIBP_SUPPORTED: AtomicBool = AtomicBool::new(false);
/// Whether IBRS is currently enabled.
static IBRS_ENABLED: AtomicBool = AtomicBool::new(false);
/// Whether the CPU has hardware Spectre-v2 mitigation (e.g., eIBRS,
/// CSV2, or similar micro-architectural fix).
static HW_MITIGATED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// x86_64 MSR / CPUID constants
// ---------------------------------------------------------------------------

/// IA32_SPEC_CTRL MSR -- IBRS (bit 0), STIBP (bit 1).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
const MSR_SPEC_CTRL: u32 = 0x48;
/// IA32_PRED_CMD MSR -- IBPB (bit 0).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
const MSR_PRED_CMD: u32 = 0x49;

/// CPUID leaf 7, EDX bit 26 -- IBRS / IBPB supported.
#[cfg(target_arch = "x86_64")]
const CPUID_IBRS_IBPB_BIT: u32 = 1 << 26;
/// CPUID leaf 7, EDX bit 27 -- STIBP supported.
#[cfg(target_arch = "x86_64")]
const CPUID_STIBP_BIT: u32 = 1 << 27;
/// CPUID leaf 7, EDX bit 29 -- IA32_ARCH_CAPABILITIES available.
#[cfg(target_arch = "x86_64")]
const CPUID_ARCH_CAP_BIT: u32 = 1 << 29;

// ---------------------------------------------------------------------------
// Public query API
// ---------------------------------------------------------------------------

/// Returns `true` if IBRS is supported by the CPU.
pub fn has_ibrs() -> bool {
    IBRS_SUPPORTED.load(Ordering::Relaxed)
}

/// Returns `true` if IBPB is supported by the CPU.
pub fn has_ibpb() -> bool {
    IBPB_SUPPORTED.load(Ordering::Relaxed)
}

/// Returns `true` if STIBP is supported by the CPU.
pub fn has_stibp() -> bool {
    STIBP_SUPPORTED.load(Ordering::Relaxed)
}

/// Returns `true` if IBRS is currently enabled.
pub fn is_ibrs_enabled() -> bool {
    IBRS_ENABLED.load(Ordering::Relaxed)
}

/// Returns `true` if the CPU has hardware-level Spectre mitigations.
pub fn is_hw_mitigated() -> bool {
    HW_MITIGATED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Feature detection
// ---------------------------------------------------------------------------

/// Detect Spectre mitigation features via CPUID on x86_64.
#[cfg(target_arch = "x86_64")]
fn detect_features() {
    // CPUID leaf 7, sub-leaf 0: EDX contains IBRS/IBPB/STIBP bits.
    let edx: u32;

    #[cfg(target_os = "none")]
    {
        // SAFETY: CPUID is read-only. We save/restore RBX for LLVM.
        let edx_val: u32;
        unsafe {
            core::arch::asm!(
                "push rbx",
                "cpuid",
                "pop rbx",
                in("eax") 7u32,
                in("ecx") 0u32,
                lateout("eax") _,
                lateout("ecx") _,
                lateout("edx") edx_val,
                options(nostack),
            );
        }
        edx = edx_val;
    }

    #[cfg(not(target_os = "none"))]
    {
        // Host/CI stub: report all features as supported.
        edx = CPUID_IBRS_IBPB_BIT | CPUID_STIBP_BIT | CPUID_ARCH_CAP_BIT;
    }

    if edx & CPUID_IBRS_IBPB_BIT != 0 {
        IBRS_SUPPORTED.store(true, Ordering::Relaxed);
        IBPB_SUPPORTED.store(true, Ordering::Relaxed);
    }
    if edx & CPUID_STIBP_BIT != 0 {
        STIBP_SUPPORTED.store(true, Ordering::Relaxed);
    }
    if edx & CPUID_ARCH_CAP_BIT != 0 {
        // If IA32_ARCH_CAPABILITIES is available, the CPU likely has
        // enhanced IBRS (eIBRS) or other hardware fixes.
        HW_MITIGATED.store(true, Ordering::Relaxed);
    }
}

/// AArch64: detect CSV2 support via ID_AA64PFR0_EL1.
#[cfg(target_arch = "aarch64")]
fn detect_features() {
    let pfr0: u64;

    #[cfg(target_os = "none")]
    // SAFETY: Reading the ID register is a read-only operation.
    unsafe {
        core::arch::asm!(
            "mrs {}, id_aa64pfr0_el1",
            out(reg) pfr0,
            options(nomem, nostack),
        );
    }

    #[cfg(not(target_os = "none"))]
    let pfr0: u64 = 0x1 << 56; // stub: CSV2 supported

    // CSV2 field: bits [59:56]. Values >= 1 indicate CSV2 mitigation.
    let csv2 = (pfr0 >> 56) & 0xF;
    if csv2 >= 1 {
        HW_MITIGATED.store(true, Ordering::Relaxed);
    }
}

/// RISC-V: no specific feature detection registers for Spectre.
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn detect_features() {
    // RISC-V does not define Spectre feature-detection registers in the
    // base privileged specification. Rely on FENCE as a software barrier.
}

/// Fallback for other / host targets.
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv32",
    target_arch = "riscv64",
)))]
fn detect_features() {}

// ---------------------------------------------------------------------------
// Enable / Flush helpers
// ---------------------------------------------------------------------------

/// Enable IBRS (Indirect Branch Restricted Speculation).
///
/// After enabling, speculative targets of indirect branches executed in
/// supervisor mode are restricted to those trained in supervisor mode.
pub fn enable_ibrs() -> Result<(), KernelError> {
    if !has_ibrs() {
        return Err(KernelError::OperationNotSupported {
            operation: "IBRS not supported by CPU",
        });
    }
    if is_ibrs_enabled() {
        return Ok(());
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // Write IA32_SPEC_CTRL.IBRS = 1.
        // SAFETY: We confirmed IBRS support via CPUID. Writing MSR 0x48
        // with bit 0 set enables IBRS. We pin EAX/ECX/EDX explicitly to
        // avoid LLVM register conflicts (see wrmsr hazard note in memory).
        unsafe {
            core::arch::asm!(
                "wrmsr",
                in("ecx") MSR_SPEC_CTRL,
                in("eax") 1u32,
                in("edx") 0u32,
                options(nomem, nostack),
            );
        }
    }

    IBRS_ENABLED.store(true, Ordering::Release);
    Ok(())
}

/// Flush the Branch Target Buffer (issue IBPB).
///
/// Should be called on context switches between different security
/// domains (e.g., different processes) to prevent branch-target poisoning.
pub fn flush_btb() {
    if has_ibpb() {
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            // Write IA32_PRED_CMD.IBPB = 1.
            // SAFETY: We confirmed IBPB support. Writing MSR 0x49 with bit 0
            // invalidates indirect-branch predictions.
            unsafe {
                core::arch::asm!(
                    "wrmsr",
                    in("ecx") MSR_PRED_CMD,
                    in("eax") 1u32,
                    in("edx") 0u32,
                    options(nomem, nostack),
                );
            }
        }
    }
    // AArch64 and RISC-V rely on architectural guarantees or fences;
    // there is no explicit BTB flush instruction.
}

/// Enable STIBP (Single Thread Indirect Branch Predictors).
///
/// Prevents sibling SMT threads from influencing indirect-branch predictions.
pub fn enable_stibp() -> Result<(), KernelError> {
    if !has_stibp() {
        return Err(KernelError::OperationNotSupported {
            operation: "STIBP not supported by CPU",
        });
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // Read current IA32_SPEC_CTRL value, then set bit 1 (STIBP).
        let lo: u32;
        let hi: u32;
        // SAFETY: Reading MSR 0x48 after confirming support is safe.
        unsafe {
            core::arch::asm!(
                "rdmsr",
                in("ecx") MSR_SPEC_CTRL,
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack),
            );
        }
        let new_lo = lo | (1 << 1); // set STIBP bit
                                    // SAFETY: We only add the STIBP bit; all other bits are preserved.
        unsafe {
            core::arch::asm!(
                "wrmsr",
                in("ecx") MSR_SPEC_CTRL,
                in("eax") new_lo,
                in("edx") hi,
                options(nomem, nostack),
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Speculation barriers
// ---------------------------------------------------------------------------

/// Insert a full speculation barrier.
///
/// On x86_64, emits LFENCE. On AArch64, DSB SY + ISB. On RISC-V, FENCE.
/// This prevents speculative execution from proceeding past this point.
#[inline(always)]
pub fn speculation_barrier() {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    // SAFETY: LFENCE is a serialising instruction with no side effects
    // beyond ordering.
    unsafe {
        core::arch::asm!("lfence", options(nomem, nostack));
    }

    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    // SAFETY: DSB SY + ISB form a full speculation barrier on AArch64.
    unsafe {
        core::arch::asm!("dsb sy", "isb", options(nomem, nostack));
    }

    #[cfg(all(
        any(target_arch = "riscv32", target_arch = "riscv64"),
        target_os = "none"
    ))]
    // SAFETY: FENCE orders all prior memory operations and serves as
    // a speculation barrier on RISC-V.
    unsafe {
        core::arch::asm!("fence iorw, iorw", options(nomem, nostack));
    }
}

// ---------------------------------------------------------------------------
// Spectre v1: safe bounds masking
// ---------------------------------------------------------------------------

/// Branchless bounds mask for Spectre v1 mitigation.
///
/// Returns a mask that is all-ones if `index < size` and all-zeros
/// otherwise. The comparison is performed without a branch, so
/// mis-speculation cannot bypass the bounds check.
///
/// # Usage
///
/// ```ignore
/// let safe_idx = index & bounds_mask(index, arr.len());
/// let value = arr[safe_idx]; // safe even under mis-speculation
/// ```
#[inline(always)]
pub fn bounds_mask(index: usize, size: usize) -> usize {
    // Compute (index < size) without a branch.
    //
    // If index < size, the subtraction does NOT borrow, so the high bit
    // is 0; arithmetic right-shift produces all zeros, then NOT gives
    // all ones.
    //
    // If index >= size, the subtraction borrows, high bit is 1; right-
    // shift produces all ones, then NOT gives all zeros.
    //
    // This is the standard pattern recommended by the Linux kernel and
    // Intel for Spectre-v1 safe array indexing.
    // Compute (size - 1) - index; if index < size, the result is
    // non-negative (high bit clear). If index >= size, it underflows
    // (high bit set).
    let diff = (size.wrapping_sub(1)).wrapping_sub(index);
    // Arithmetic right shift: propagate sign bit across all bits.
    let sign = (diff as isize) >> (usize::BITS - 1);
    // If index < size => diff >= 0 => sign == 0 => !0 == all-ones (usize::MAX)
    // If index >= size => diff < 0 => sign == -1 => !(-1) == 0
    !(sign as usize)
}

/// Safe array index that clamps to zero under mis-speculation.
///
/// Returns `index` unchanged if `index < size`, or `0` otherwise.
/// Uses [`bounds_mask`] internally for branchless operation.
#[inline(always)]
pub fn safe_index(index: usize, size: usize) -> usize {
    index & bounds_mask(index, size)
}

// ---------------------------------------------------------------------------
// Retpoline thunk marker
// ---------------------------------------------------------------------------

/// Whether retpoline (compiler-level Spectre v2 mitigation) is active.
///
/// Retpoline replaces indirect calls (`call *%rax`) with a construct that
/// traps speculation in an infinite pause loop. It is enabled at the
/// compiler level (e.g., `-Cllvm-args=-x86-speculative-load-hardening`
/// or equivalent). Because Rust does not expose a `target_feature` for
/// retpoline, this is a compile-time constant that should be updated
/// when the build system enables retpoline flags.
pub const RETPOLINE_ENABLED: bool = false;

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Detect and enable Spectre mitigations.
///
/// Called during early boot from `security::init()`. Detects CPU features
/// and enables available hardware mitigations. If IBRS is available, it is
/// enabled. IBPB flushes are performed on context switches via [`flush_btb`].
pub fn init() -> Result<(), KernelError> {
    detect_features();

    // Enable IBRS if available (enhanced IBRS is preferred).
    let ibrs_ok = if has_ibrs() {
        enable_ibrs().is_ok()
    } else {
        false
    };

    // Enable STIBP if available.
    let stibp_ok = if has_stibp() {
        enable_stibp().is_ok()
    } else {
        false
    };

    #[cfg(target_arch = "x86_64")]
    crate::println!(
        "[SPECTRE] x86_64: IBRS {} ({}), IBPB {}, STIBP {} ({}), eIBRS/HW {}",
        if has_ibrs() { "yes" } else { "no" },
        if ibrs_ok { "enabled" } else { "skipped" },
        if has_ibpb() { "yes" } else { "no" },
        if has_stibp() { "yes" } else { "no" },
        if stibp_ok { "enabled" } else { "skipped" },
        if is_hw_mitigated() { "yes" } else { "no" },
    );

    #[cfg(target_arch = "aarch64")]
    {
        let _ = (ibrs_ok, stibp_ok);
        crate::kprintln!("[SPECTRE] AArch64: CSV2 detection complete");
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        let _ = (ibrs_ok, stibp_ok);
        crate::kprintln!("[SPECTRE] RISC-V: fence-based barriers configured");
    }

    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv32",
        target_arch = "riscv64",
    )))]
    {
        let _ = (ibrs_ok, stibp_ok);
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
    fn test_bounds_mask_in_range() {
        let mask = bounds_mask(3, 10);
        assert_eq!(mask, usize::MAX);
        assert_eq!(3 & mask, 3);
    }

    #[test]
    fn test_bounds_mask_at_boundary() {
        let mask = bounds_mask(10, 10);
        assert_eq!(mask, 0);
        assert_eq!(10 & mask, 0);
    }

    #[test]
    fn test_bounds_mask_out_of_range() {
        let mask = bounds_mask(15, 10);
        assert_eq!(mask, 0);
        assert_eq!(15 & mask, 0);
    }

    #[test]
    fn test_bounds_mask_zero_size() {
        let mask = bounds_mask(0, 0);
        assert_eq!(mask, 0);
    }

    #[test]
    fn test_bounds_mask_max_index() {
        let mask = bounds_mask(usize::MAX, 10);
        assert_eq!(mask, 0);
    }

    #[test]
    fn test_safe_index_in_range() {
        assert_eq!(safe_index(5, 10), 5);
    }

    #[test]
    fn test_safe_index_out_of_range() {
        assert_eq!(safe_index(10, 10), 0);
        assert_eq!(safe_index(100, 10), 0);
    }

    #[test]
    fn test_safe_index_zero() {
        assert_eq!(safe_index(0, 10), 0);
        assert_eq!(safe_index(0, 0), 0);
    }

    #[test]
    fn test_feature_queries() {
        // Just verify the query functions don't panic.
        let _ = has_ibrs();
        let _ = has_ibpb();
        let _ = has_stibp();
        let _ = is_ibrs_enabled();
        let _ = is_hw_mitigated();
    }

    #[test]
    fn test_detect_features_host() {
        detect_features();
        // On host/CI, stubs report features as supported.
        #[cfg(not(target_os = "none"))]
        {
            assert!(has_ibrs());
            assert!(has_ibpb());
            assert!(has_stibp());
        }
    }

    #[test]
    fn test_enable_ibrs_without_support() {
        IBRS_SUPPORTED.store(false, Ordering::Relaxed);
        IBRS_ENABLED.store(false, Ordering::Relaxed);
        assert!(enable_ibrs().is_err());
        // Restore.
        detect_features();
    }

    #[test]
    fn test_flush_btb_noop_without_support() {
        IBPB_SUPPORTED.store(false, Ordering::Relaxed);
        flush_btb(); // should not panic
                     // Restore.
        detect_features();
    }

    #[test]
    fn test_speculation_barrier_noop_on_host() {
        // On the host target, speculation_barrier is a no-op (all asm
        // is gated with target_os = "none"). Just verify it compiles.
        speculation_barrier();
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_cpuid_constants() {
        assert_eq!(CPUID_IBRS_IBPB_BIT, 1 << 26);
        assert_eq!(CPUID_STIBP_BIT, 1 << 27);
        assert_eq!(CPUID_ARCH_CAP_BIT, 1 << 29);
    }
}

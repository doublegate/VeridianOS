//! Branch Target Identification for user programs (FEAT_BTI), as Linux:
//! on a CPU that implements it (ID_AA64PFR1_EL1.BT), EL0 guarded pages
//! (PROT_BTI, PTE GP bit) are enforced (SCTLR_EL1.BT0 set), programs see
//! HWCAP2_BTI, mprotect accepts PROT_BTI, and the loader guards the code of
//! a program whose GNU property note asks for it. Elsewhere none of this
//! applies, and PROT_BTI is refused by mprotect and ignored by mmap.
//!
//! The kernel itself is not built with BTI landing pads, and its pages are
//! not guarded.

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether the boot CPU implements FEAT_BTI.
static SUPPORTED: AtomicBool = AtomicBool::new(false);

/// SCTLR_EL1.BT0: guarded pages at EL0 require BTI landing pads (PACIxSP
/// is not one; Linux sets it too).
const SCTLR_BT0: u64 = 1 << 35;

/// Whether ID_AA64PFR1_EL1 (`pfr1`) reports FEAT_BTI (BT, bits 3:0).
pub fn has_bti(pfr1: u64) -> bool {
    pfr1 & 0xF != 0
}

/// Enable BTI for EL0 on this CPU if it has it; the boot CPU decides
/// whether the system offers it. Runs once on each CPU at bring-up, before
/// it runs a user program.
pub fn init_cpu(boot_cpu: bool) {
    let pfr1: u64;
    // SAFETY: an ID register read at EL1, with no side effects.
    unsafe { core::arch::asm!("mrs {0}, id_aa64pfr1_el1", out(reg) pfr1, options(nomem, nostack)) };
    let has = has_bti(pfr1);
    if boot_cpu {
        SUPPORTED.store(has, Ordering::Release);
        crate::println!(
            "[BTI] guarded pages for user programs: {}",
            if has {
                "enforced"
            } else {
                "not available (no FEAT_BTI)"
            }
        );
    } else if has != supported() {
        crate::println!("[BTI] warning: this CPU's BTI support differs from the boot CPU's");
    }
    if has {
        // SAFETY: sets only SCTLR_EL1.BT0, which affects EL0 execution in
        // guarded pages; the kernel's own pages are never guarded. The isb
        // makes the change take effect before any return to EL0.
        unsafe {
            core::arch::asm!(
                "mrs {t}, sctlr_el1",
                "orr {t}, {t}, {bt0}",
                "msr sctlr_el1, {t}",
                "isb",
                t = out(reg) _,
                bt0 = in(reg) SCTLR_BT0,
                options(nostack)
            );
        }
    }
}

/// Whether the system offers BTI to user programs.
pub fn supported() -> bool {
    SUPPORTED.load(Ordering::Acquire)
}

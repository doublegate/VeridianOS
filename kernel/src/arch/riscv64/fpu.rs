//! User floating-point state on RISC-V (sprint E).
//!
//! The kernel is soft-float (N-178), so the F/D registers hold only user
//! state, switched eagerly with the task (`sched::dispatch::arch_switch`,
//! as XSAVE on x86_64). `sstatus.FS` is Off while the kernel runs, so a
//! stray FP instruction in kernel code traps (HR-20); it is turned on only
//! around the save and restore, and set to Clean for the return to U-mode
//! (`switch::user_sstatus`). Whether the harts have the D extension comes
//! from the device tree's `riscv,isa`: `misa` is an M-mode CSR, which
//! S-mode code cannot read (HR-21).

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::arch::riscv::context::FpuState;

static PRESENT: AtomicBool = AtomicBool::new(false);
/// AT_HWCAP for user programs (`elf::dynamic::riscv_hwcap`), from the same
/// device-tree ISA description.
static HWCAP: AtomicU64 = AtomicU64::new(0);

/// sstatus.FS.
const FS: usize = 3 << 13;

/// Record whether the harts have F and D (device tree at physical
/// `dtb_pa`). Without them user FP stays off and an FP instruction is
/// SIGILL.
pub fn init_from_device_tree(dtb_pa: u64) {
    // SAFETY: dtb_pa is what firmware passed in a1 (saved by boot.S).
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(dtb_pa) }) else {
        return;
    };
    let has = |l: u8| {
        fdt.property(&["cpus", "cpu"], "riscv,isa-extensions")
            .is_some_and(|isa| crate::arch::fdt::isa_has_base_extension(isa, l))
            || fdt
                .property(&["cpus", "cpu"], "riscv,isa")
                .is_some_and(|isa| crate::arch::fdt::isa_has_base_extension(isa, l))
    };
    let fp = has(b'f') && has(b'd');
    PRESENT.store(fp, Ordering::Relaxed);
    HWCAP.store(crate::elf::dynamic::riscv_hwcap(has, fp), Ordering::Relaxed);
}

/// The AT_HWCAP word user programs get (0 before `init_from_device_tree`).
pub fn hwcap() -> u64 {
    HWCAP.load(Ordering::Relaxed)
}

/// Whether user programs have F/D registers to switch.
pub fn present() -> bool {
    PRESENT.load(Ordering::Relaxed)
}

/// The FP state a new program starts with: every register zero, round to
/// nearest, no flags.
pub fn initial() -> alloc::boxed::Box<FpuState> {
    alloc::boxed::Box::new(FpuState {
        f: [0.0; 32],
        fcsr: 0,
    })
}

/// Save the live user FP registers into `state` (FS on just for the save,
/// then off again).
pub fn save(state: &mut FpuState) {
    if !present() {
        return;
    }
    // SAFETY: sets sstatus.FS so the FP instructions of the save do not
    // trap; cleared again below.
    unsafe { core::arch::asm!("csrs sstatus, {0}", in(reg) FS, options(nomem, nostack)) };
    crate::arch::riscv::context::save_fpu_state(state);
    // SAFETY: back to FP off in the kernel.
    unsafe { core::arch::asm!("csrc sstatus, {0}", in(reg) FS, options(nomem, nostack)) };
}

/// Load `state` into the FP registers for the task about to run (FS on just
/// for the restore).
pub fn restore(state: &FpuState) {
    if !present() {
        return;
    }
    // SAFETY: as in `save`.
    unsafe { core::arch::asm!("csrs sstatus, {0}", in(reg) FS, options(nomem, nostack)) };
    crate::arch::riscv::context::restore_fpu_state(state);
    // SAFETY: as in `save`.
    unsafe { core::arch::asm!("csrc sstatus, {0}", in(reg) FS, options(nomem, nostack)) };
}

//! RISC-V secondary hart bring-up (SBI HSM), for `arch::smp_boot`.
//!
//! Harts come from the device tree's `/cpus/cpu@N` nodes (`reg` = hart
//! id). OpenSBI keeps every hart but the boot hart STOPPED until the kernel
//! calls `hart_start`, which enters `veridian_hart_entry` (boot.S) with the
//! MMU off. The kernel itself runs with `satp = 0`, so the entry address and
//! the boot block are passed as their (identical) virtual addresses.

use alloc::vec::Vec;

use crate::arch::smp_boot::ApBootArgs;

extern "C" {
    fn veridian_hart_entry();
    static veridian_dtb_pa: u64;
}

/// Hart ids of the usable secondary harts, in device tree order.
pub fn enumerate_secondaries() -> Vec<u32> {
    let mut harts = Vec::new();
    if !crate::arch::riscv::sbi::has_hsm() {
        crate::println!("[SMP] SBI has no HSM extension: secondary harts cannot be started");
        return harts;
    }
    let boot = super::boot::boot_hartid();
    // SAFETY: written once by boot.S from a1 before any Rust code ran.
    let dtb_pa = unsafe { core::ptr::addr_of!(veridian_dtb_pa).read_volatile() };
    // SAFETY: dtb_pa is the blob firmware passed (saved by boot.S).
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(dtb_pa) }) else {
        return harts;
    };
    fdt.cpus(|cpu| {
        let Some(hart) = cpu.reg else { return };
        if hart == boot || !cpu.is_available() || cpu.mmu_type == Some(&b"riscv,none"[..]) {
            return;
        }
        if let Ok(hart) = u32::try_from(hart) {
            harts.push(hart);
        }
    });
    harts
}

/// Nothing extra on RISC-V: the entry stub needs only the stack and the
/// per-CPU block.
pub fn prepare_boot_args(_args: &ApBootArgs) {}

/// Ask the firmware to start hart `hw` at the entry stub.
pub fn start_ap(hw: u32, args: &'static ApBootArgs) -> Result<(), &'static str> {
    let ret = crate::arch::riscv::sbi::hart_start(
        hw as usize,
        veridian_hart_entry as *const () as usize,
        args as *const ApBootArgs as usize,
    );
    if ret.is_ok() {
        Ok(())
    } else {
        Err("SBI hart_start failed")
    }
}

/// This hart's local setup: its tick and IPIs (the trap vector is already
/// installed by the entry stub).
pub fn ap_init() {
    crate::arch::smp_boot::ap_stage(1);
    crate::arch::riscv::timer::start_secondary();
    // SAFETY: sets sie.SSIE so IPIs (SBI send_ipi) are delivered here.
    unsafe { core::arch::asm!("csrs sie, {0}", in(reg) 2usize, options(nomem, nostack)) };
    crate::arch::smp_boot::ap_stage(2);
}

/// Park with interrupts enabled.
pub fn idle() -> ! {
    loop {
        // SAFETY: enables supervisor interrupts on this hart, whose trap
        // vector and handlers are installed, then waits for one.
        unsafe { core::arch::asm!("csrsi sstatus, 2", "wfi", options(nomem, nostack)) };
    }
}

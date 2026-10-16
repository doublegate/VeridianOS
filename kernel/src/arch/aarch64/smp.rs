//! AArch64 secondary CPU bring-up (PSCI CPU_ON), for `arch::smp_boot`.
//!
//! CPUs come from the device tree's `/cpus/cpu@N` nodes (`reg` = MPIDR
//! affinity, `enable-method = "psci"`); QEMU places the blob at the start of
//! RAM for a bare-metal ELF. A secondary enters `veridian_secondary_entry`
//! (boot.S) with the MMU and caches off.
//!
//! A secondary switches its MMU on in boot.S (ADR 0015), sets up its GIC
//! CPU interface and its own timer tick, and dispatches (its boot flow
//! becomes its idle task). Until its MMU was on (N-28) a secondary could
//! take no lock and ran a reduced stage that parked with interrupts masked.

use alloc::vec::Vec;

use super::psci::{self, Conduit};
use crate::arch::smp_boot::ApBootArgs;

extern "C" {
    fn veridian_secondary_entry();
}

/// PSCI conduit found during enumeration.
static CONDUIT: spin::Once<Conduit> = spin::Once::new();

/// MPIDR affinity bits (Aff2..Aff0) used as hardware ids.
const MPIDR_AFF_MASK: u64 = 0x00FF_FFFF;

/// MPIDR affinities of the usable secondary CPUs, in device tree order.
pub fn enumerate_secondaries() -> Vec<u32> {
    let mut cpus = Vec::new();
    // SAFETY: the blob firmware passed (or QEMU virt's), reached through
    // the physical map; from_phys validates the header before reading on.
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(super::boot::dtb_pa()) }) else {
        crate::println!("[SMP] No device tree found: secondary CPUs not started");
        return cpus;
    };
    let Some(conduit) = fdt
        .property(&["psci"], "method")
        .and_then(psci::conduit_from_method)
    else {
        crate::println!("[SMP] No PSCI conduit in the device tree: secondary CPUs not started");
        return cpus;
    };
    CONDUIT.call_once(|| conduit);
    let mpidr: u64;
    // SAFETY: reading MPIDR_EL1 at EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack)) };
    let boot = mpidr & MPIDR_AFF_MASK;
    fdt.cpus(|cpu| {
        let Some(reg) = cpu.reg else { return };
        let psci_enabled = cpu.enable_method.is_none_or(|m| m == b"psci");
        if reg & MPIDR_AFF_MASK == boot || !cpu.is_available() || !psci_enabled {
            return;
        }
        cpus.push((reg & MPIDR_AFF_MASK) as u32);
    });
    cpus
}

/// Nothing extra on AArch64. The boot block is written with plain stores;
/// the secondary reads it once its MMU is on, which `start_ap` orders.
pub fn prepare_boot_args(_args: &ApBootArgs) {}

/// Power on CPU `hw` at the entry stub.
pub fn start_ap(hw: u32, args: &'static ApBootArgs) -> Result<(), &'static str> {
    let conduit = *CONDUIT.get().ok_or("no PSCI conduit")?;
    // The secondary starts with its MMU off, at physical addresses: its
    // entry and its boot block are passed as such (ADR 0015). It reads the
    // block only once its MMU is on, so the block needs no cache cleaning.
    // SAFETY: barriers only.
    unsafe { core::arch::asm!("dsb sy", "isb", options(nostack)) };
    let ret = psci::cpu_on(
        conduit,
        u64::from(hw),
        super::boot::kernel_phys(veridian_secondary_entry as *const () as u64),
        super::boot::kernel_phys(args as *const ApBootArgs as u64),
    );
    if ret == 0 {
        Ok(())
    } else {
        Err(psci::error_name(ret))
    }
}

/// This CPU's local setup: the GIC CPU interface and its timer tick (boot.S
/// installed the exception vectors and turned the MMU on).
pub fn ap_init() -> Result<(), &'static str> {
    crate::arch::smp_boot::ap_stage(1);
    super::gic::init_secondary_cpu_interface();
    super::timer::start_secondary().map_err(|_| "timer start failed")?;
    crate::arch::smp_boot::ap_stage(2);
    Ok(())
}

/// No clock check: the generic timer counter (AArch64) and the `time` CSR
/// (RISC-V) are synchronised across CPUs by the architecture.
pub fn serve_boot_handshake() {}

/// Dispatch on this CPU: its boot flow becomes its idle task (as on x86_64
/// and RISC-V), with interrupts enabled.
pub fn idle() -> ! {
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::run_secondary();
    #[cfg(not(feature = "alloc"))]
    loop {
        // SAFETY: unmasks IRQs on this CPU, whose vectors, GIC CPU interface
        // and timer are set up, then waits for one.
        unsafe { core::arch::asm!("msr daifclr, #2", "wfi", options(nomem, nostack)) };
    }
}

//! AArch64 secondary CPU bring-up (PSCI CPU_ON), for `arch::smp_boot`.
//!
//! CPUs come from the device tree's `/cpus/cpu@N` nodes (`reg` = MPIDR
//! affinity, `enable-method = "psci"`); QEMU places the blob at the start of
//! RAM for a bare-metal ELF. A secondary enters `veridian_secondary_entry`
//! (boot.S) with the MMU and caches off.
//!
//! Reduced stage S1 (ADR 0004): until the MMU is on (N-28), load/store
//! exclusive -- every lock and atomic read-modify-write -- is not
//! architecturally reliable across CPUs on the Device memory the kernel
//! runs from. A secondary therefore only installs its vectors and GIC CPU
//! interface, reports ONLINE with plain stores, and parks with interrupts
//! masked: no lock, no timer, no shared state.

use alloc::vec::Vec;

use super::psci::{self, Conduit};
use crate::arch::smp_boot::ApBootArgs;

extern "C" {
    fn veridian_secondary_entry();
}

/// Where QEMU puts the device tree for a bare-metal ELF on `virt`.
const QEMU_VIRT_DTB_PA: u64 = 0x4000_0000;

/// PSCI conduit found during enumeration.
static CONDUIT: spin::Once<Conduit> = spin::Once::new();

/// MPIDR affinity bits (Aff2..Aff0) used as hardware ids.
const MPIDR_AFF_MASK: u64 = 0x00FF_FFFF;

/// MPIDR affinities of the usable secondary CPUs, in device tree order.
pub fn enumerate_secondaries() -> Vec<u32> {
    let mut cpus = Vec::new();
    // SAFETY: on QEMU virt the blob is at the start of RAM, identity
    // mapped; from_phys validates the header before reading further.
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(QEMU_VIRT_DTB_PA) }) else {
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
/// the secondary reads it with its MMU off, which `start_ap` orders.
pub fn prepare_boot_args(_args: &ApBootArgs) {}

/// Power on CPU `hw` at the entry stub.
pub fn start_ap(hw: u32, args: &'static ApBootArgs) -> Result<(), &'static str> {
    let conduit = *CONDUIT.get().ok_or("no PSCI conduit")?;
    // The secondary reads the boot block with its MMU and caches off.
    // SAFETY: barriers only.
    unsafe { core::arch::asm!("dsb sy", "isb", options(nostack)) };
    let ret = psci::cpu_on(
        conduit,
        u64::from(hw),
        veridian_secondary_entry as *const () as u64,
        args as *const ApBootArgs as u64,
    );
    if ret == 0 {
        Ok(())
    } else {
        Err(psci::error_name(ret))
    }
}

/// This CPU's local setup: exception vectors and GIC CPU interface. No
/// timer and no lock before the MMU is on (see the module comment).
pub fn ap_init() {
    super::exceptions::install();
    crate::arch::smp_boot::ap_stage(1);
    super::gic::init_secondary_cpu_interface();
    crate::arch::smp_boot::ap_stage(2);
}

/// Park with interrupts masked (S1 before N-28).
pub fn idle() -> ! {
    loop {
        // SAFETY: waits for an event; interrupts stay masked (DAIF set at
        // entry).
        unsafe { core::arch::asm!("wfe", options(nomem, nostack)) };
    }
}

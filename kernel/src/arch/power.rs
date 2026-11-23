//! Restarting, halting and powering off the machine (reboot(2); ADR 0027
//! decision 29). The other CPUs are stopped first, as Linux's
//! smp_send_stop: each is sent the reschedule interrupt, and its interrupt
//! handler sees [`stopping`] and halts with interrupts masked. A CPU that
//! does not answer within a second (one spinning with interrupts off) is
//! left, as Linux without an NMI.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The machine is being stopped: every CPU that takes an interrupt halts.
static STOPPING: AtomicBool = AtomicBool::new(false);
/// How many CPUs have halted for it.
static STOPPED: AtomicUsize = AtomicUsize::new(0);

/// Whether the machine is being stopped (checked by the IPI handler).
pub fn stopping() -> bool {
    STOPPING.load(Ordering::Acquire)
}

/// Halt this CPU for good, interrupts masked (stop_this_cpu).
pub fn stop_this_cpu() -> ! {
    let _irq = crate::arch::disable_interrupts();
    STOPPED.fetch_add(1, Ordering::AcqRel);
    crate::arch::halt();
}

/// Stop every other CPU, waiting up to a second for them, and leave
/// interrupts masked on this one for good (nothing is scheduled any more).
/// Whatever needs a lock another CPU might hold -- printing, the heap -- is
/// done before this, as Linux prints before machine_restart.
fn stop_other_cpus() {
    core::mem::forget(crate::arch::disable_interrupts());
    if STOPPING.swap(true, Ordering::AcqRel) {
        // Another CPU is stopping the machine: this one is stopped too.
        stop_this_cpu();
    }
    let others = (crate::arch::smp_boot::online_cpus() as usize).saturating_sub(1);
    if others == 0 {
        return;
    }
    crate::sched::dispatch::kick_others();
    let deadline = crate::timer::monotonic_ns().saturating_add(1_000_000_000);
    while STOPPED.load(Ordering::Acquire) < others && crate::timer::monotonic_ns() < deadline {
        core::hint::spin_loop();
    }
}

/// Whether the machine can power itself off (kernel_can_power_off): without
/// it, reboot's POWER_OFF halts.
pub fn can_power_off() -> bool {
    cfg!(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    ))
}

/// kernel_restart: say so, stop the other CPUs, reset the machine.
pub fn restart(cmd: Option<&str>) -> ! {
    match cmd {
        Some(c) => crate::println!("reboot: Restarting system with command '{}'", c),
        None => crate::println!("reboot: Restarting system"),
    }
    stop_other_cpus();
    machine_restart()
}

/// kernel_halt: say so, stop the other CPUs, then this one.
pub fn halt() -> ! {
    crate::println!("reboot: System halted");
    stop_other_cpus();
    stop_this_cpu()
}

/// kernel_power_off: say so, stop the other CPUs, power off; this CPU
/// halts if that does not take.
pub fn power_off() -> ! {
    crate::println!("reboot: Power down");
    let prepared = prepare_power_off();
    stop_other_cpus();
    machine_power_off(prepared);
    stop_this_cpu()
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn machine_restart() -> ! {
    use x86_64::instructions::port::Port;
    // The PCI reset control register (0xCF9): a full reset; then the
    // keyboard controller's reset line; then a triple fault, which resets
    // any PC (Linux's reboot=pci,kbd,triple order of fallbacks).
    // SAFETY: port writes that reset the machine; nothing runs after them
    // but the fallbacks below.
    unsafe {
        let mut cf9: Port<u8> = Port::new(0xcf9);
        cf9.write(0x02);
        cf9.write(0x06);
        let mut kbd: Port<u8> = Port::new(0x64);
        kbd.write(0xfe);
    }
    // SAFETY: an empty IDT and an interrupt: a triple fault, a reset.
    unsafe {
        let idt = x86_64::structures::DescriptorTablePointer {
            limit: 0,
            base: x86_64::VirtAddr::new(0),
        };
        x86_64::instructions::tables::lidt(&idt);
        core::arch::asm!("int3", options(nomem, nostack));
    }
    stop_this_cpu()
}

/// What powering off needs that takes locks or prints: the ACPI tables and
/// the S5 register writes, read while the other CPUs still run, since one
/// of them may be stopped holding the lock (PR #49 review).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn prepare_power_off() -> Option<crate::arch::x86_64::acpi_pm::S5> {
    use crate::arch::x86_64::acpi_pm;
    if !acpi_pm::is_initialized() {
        let _ = acpi_pm::acpi_pm_init();
    }
    acpi_pm::prepare_s5()
}

/// Nothing to prepare: the firmware's power-off call takes no lock.
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
struct NothingPrepared;

#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
fn prepare_power_off() -> NothingPrepared {
    NothingPrepared
}

/// Port writes only (the other CPUs are stopped).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn machine_power_off(s5: Option<crate::arch::x86_64::acpi_pm::S5>) {
    if let Some(s5) = s5 {
        s5.enter();
    }
}

/// The PSCI conduit the device tree names: read at SMP bring-up, or here
/// in a kernel built without SMP; HVC (QEMU virt's) only when the tree names
/// none (PR #49 review).
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
fn psci_conduit() -> crate::arch::aarch64::psci::Conduit {
    #[cfg(feature = "smp")]
    {
        crate::arch::aarch64::smp::conduit()
    }
    #[cfg(not(feature = "smp"))]
    {
        crate::arch::aarch64::psci::conduit_from_device_tree()
            .unwrap_or(crate::arch::aarch64::psci::Conduit::Hvc)
    }
}

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
fn machine_restart() -> ! {
    crate::arch::aarch64::psci::system_reset(psci_conduit());
    stop_this_cpu()
}

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
fn machine_power_off(_: NothingPrepared) {
    crate::arch::aarch64::psci::system_off(psci_conduit());
}

#[cfg(all(target_arch = "riscv64", target_os = "none"))]
fn machine_restart() -> ! {
    // SBI SRST: a cold reboot.
    crate::arch::riscv::sbi::system_reset(1, 0);
    stop_this_cpu()
}

#[cfg(all(target_arch = "riscv64", target_os = "none"))]
fn machine_power_off(_: NothingPrepared) {
    // SBI SRST shutdown, or the legacy shutdown call without SRST.
    crate::arch::riscv::sbi::system_reset(0, 0);
    crate::arch::riscv::sbi::legacy_shutdown();
}

#[cfg(not(target_os = "none"))]
fn machine_restart() -> ! {
    stop_this_cpu()
}

#[cfg(not(target_os = "none"))]
fn machine_power_off(_: NothingPrepared) {}

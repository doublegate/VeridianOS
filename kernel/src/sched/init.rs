//! Scheduler initialization and timer setup
//!
//! Contains the bootstrap initialization path (`init_with_bootstrap`) used
//! during early kernel boot, the normal initialization path (`init`), and
//! architecture-specific preemption timer configuration.

use core::ptr::NonNull;

use super::{smp, task::Task};
use crate::error::KernelResult;

/// Initialize scheduler with bootstrap task
///
/// This is used during early boot to initialize the scheduler with a
/// bootstrap task that will complete kernel initialization.
pub fn init_with_bootstrap(bootstrap_task: NonNull<Task>) -> KernelResult<()> {
    kprintln!("[SCHED] Initializing scheduler with bootstrap task...");

    // Initialize SMP support
    kprintln!("[SCHED] About to initialize SMP...");
    smp::init();
    kprintln!("[SCHED] SMP initialization complete");

    // Initialize scheduler with bootstrap task
    kprintln!("[SCHED] About to get scheduler lock...");
    super::SCHEDULER.lock().init(bootstrap_task);
    kprintln!("[SCHED] Scheduler init complete");

    // Set up timer interrupt for preemption
    kprintln!("[SCHED] About to setup preemption timer...");
    setup_preemption_timer();
    kprintln!("[SCHED] Preemption timer setup complete");

    kprintln!("[SCHED] Scheduler initialized with bootstrap task");

    Ok(())
}

/// Initialize scheduler normally (after bootstrap)
pub fn init() {
    kprintln!("[SCHED] Initializing scheduler...");

    // Initialize SMP support
    smp::init();

    // Skip complex scheduler setup on all architectures for now.
    // kernel_init_main() tests run before sched::init() and don't need the
    // scheduler. The idle task creation and PIT timer setup can hang or panic
    // during early boot.
    kprintln!("[SCHED] Scheduler initialized (minimal)");
}

/// Set up preemption timer
fn setup_preemption_timer() {
    #[cfg(target_arch = "x86_64")]
    {
        // Configure timer for 10ms tick (100Hz)
        crate::arch::x86_64::timer::setup_timer(10);
        kprintln!("[SCHED] x86_64 timer configured for preemptive scheduling");
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Configure generic timer for 10ms tick
        crate::arch::aarch64::timer::setup_timer(10);
        kprintln!("[SCHED] AArch64 timer configured for preemptive scheduling");
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // Configure RISC-V timer for 10ms tick.
        // NOTE: setup_timer() configures the SBI timer but does NOT enable
        // STIE (supervisor timer interrupt enable) because no trap handler
        // (stvec) is registered yet. Enabling STIE without stvec causes
        // the CPU to jump to address 0 on timer fire, rebooting the system.
        crate::arch::riscv::timer::setup_timer(10);
        kprintln!("[SCHED] RISC-V timer configured for preemptive scheduling");
    }
}

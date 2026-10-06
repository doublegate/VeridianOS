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
    // The periodic tick is started by each architecture's init (LAPIC
    // TSC-deadline/periodic on x86_64, the EL1 virtual timer on AArch64,
    // stimecmp/SBI on RISC-V); see arch::*::timer.

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

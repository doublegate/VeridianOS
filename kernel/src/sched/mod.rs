//! Scheduling.
//!
//! The scheduler is the dispatcher ([`dispatch`], ADR 0006) with its policy
//! classes ([`policy`], ADR 0007): every user thread and kernel thread is a
//! dispatcher task. The older scheduler (one global ready queue, a `Task`
//! control block of its own, a separate context switch, process
//! "adapters" and pid-keyed IPC blocking) never ran once the dispatcher
//! started and was removed in sprint D4.
//!
//! - [`dispatch`] - tasks, run queues, wait queues, sleeping and waking
//! - [`policy`] - deadline, real-time and fair classes
//! - [`smp`] - CPU topology and per-CPU data
//! - [`cputime`], [`loadavg`], [`metrics`] - accounting

pub mod cputime;
#[cfg(feature = "alloc")]
pub mod dispatch;
pub mod loadavg;
pub mod metrics;
#[cfg(feature = "alloc")]
pub mod policy;
pub mod smp;

pub(crate) use crate::process::ProcessId;

/// Initialize CPU bookkeeping (SMP topology). The dispatcher itself is
/// started by `dispatch::start` (x86_64; the other architectures get it in
/// sprint E).
pub fn init() {
    kprintln!("[SCHED] Initializing scheduler...");
    smp::init();
    kprintln!("[SCHED] Scheduler initialized");
}

/// Idle forever (the boot flow's end when it has nothing left to run).
pub fn start() -> ! {
    kprintln!("[SCHED] Entering idle loop");
    loop {
        crate::arch::idle();
    }
}

/// Give the CPU to another runnable task, if any.
pub fn yield_cpu() {
    #[cfg(feature = "alloc")]
    if dispatch::started() {
        dispatch::yield_now();
        return;
    }
    core::hint::spin_loop();
}

/// From inside a system call that polls for a condition, give the CPU away
/// until something may have changed. Returns true if a signal must be
/// acted on (the caller fails with EINTR).
///
/// A dispatched thread lets other tasks run. Outside one (no user thread
/// is, without the dispatcher) the CPU waits for the next interrupt.
#[must_use]
pub fn wait_for_interrupt_in_syscall() -> bool {
    #[cfg(feature = "alloc")]
    if dispatch::current_owner().is_some() {
        return dispatch::wait_in_syscall();
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: enables interrupts for exactly one halt and disables them
    // again.
    unsafe {
        core::arch::asm!("sti; hlt; cli", options(nomem, nostack));
    }
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    // SAFETY: WFI only waits for an interrupt or event.
    unsafe {
        core::arch::asm!("wfi", options(nomem, nostack));
    }
    false
}

/// The calling user thread's id, or 0 for kernel threads and the boot task.
pub fn get_current_thread_id() -> u64 {
    crate::process::current_thread().map_or(0, |t| t.tid.0)
}

/// The calling user process's id, or 0 for kernel threads and the boot
/// task (the old fallback answered 1, init, which it was not).
pub fn current_process_id() -> ProcessId {
    crate::process::current_process().map_or(ProcessId(0), |p| p.pid)
}

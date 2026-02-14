//! Scheduler runtime loop and idle task management
//!
//! Contains the main scheduler execution loop (`run`), the idle task entry
//! point, timer tick handling, and scheduler start/query functions.

use super::scheduler;

/// Start the scheduler
///
/// This transfers control to the scheduler, which will run the current task
/// (bootstrap or idle) and never return.
pub fn start() -> ! {
    kprintln!("[SCHED] Starting scheduler execution");

    #[cfg(target_arch = "aarch64")]
    {
        // Enter idle loop
        loop {
            // SAFETY: WFI (Wait For Interrupt) is a hint instruction that
            // puts the CPU into a low-power state until an interrupt arrives.
            // It does not modify any memory or registers beyond the PC. The
            // nomem/nostack/preserves_flags options correctly reflect this.
            unsafe {
                core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
            }
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // Enter idle loop
        loop {
            // SAFETY: WFI (Wait For Interrupt) is a hint instruction on RISC-V
            // that suspends execution until an interrupt occurs. It does not
            // modify memory or registers. The options correctly reflect this.
            unsafe {
                core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
            }
        }
    }

    // x86_64: Enter HLT idle loop (matches AArch64 WFI and RISC-V WFI)
    #[cfg(target_arch = "x86_64")]
    {
        println!("[SCHED] Entering idle loop");
        loop {
            crate::arch::idle();
        }
    }
}

/// Check if there are ready tasks
pub fn has_ready_tasks() -> bool {
    #[cfg(not(target_arch = "riscv64"))]
    {
        super::READY_QUEUE.lock().has_ready_tasks()
    }
    #[cfg(target_arch = "riscv64")]
    {
        super::queue::get_ready_queue().has_ready_tasks()
    }
}

/// Run scheduler main loop (called by idle task)
pub fn run() -> ! {
    kprintln!("[SCHED] Entering scheduler main loop");

    let mut balance_counter = 0u64;

    loop {
        // Check for ready tasks
        #[cfg(not(target_arch = "riscv64"))]
        {
            if super::READY_QUEUE.lock().has_ready_tasks() {
                super::SCHEDULER.lock().schedule();
            }
        }
        #[cfg(target_arch = "riscv64")]
        {
            if super::queue::get_ready_queue().has_ready_tasks() {
                super::SCHEDULER.lock().schedule();
            }
        }

        // Periodically perform load balancing and cleanup
        balance_counter = balance_counter.wrapping_add(1);
        if balance_counter.is_multiple_of(1000) {
            #[cfg(feature = "alloc")]
            {
                super::load_balance::balance_load();

                // Also clean up dead tasks
                if balance_counter.is_multiple_of(10000) {
                    super::load_balance::cleanup_dead_tasks();
                }
            }
        }

        // Enter low power state
        crate::arch::idle();
    }
}

/// Idle task entry point
pub extern "C" fn idle_task_entry() -> ! {
    run()
}

/// Handle timer tick
pub fn timer_tick() {
    scheduler::current_scheduler().lock().tick();
}

/// Set scheduling algorithm
pub fn set_algorithm(algorithm: super::SchedAlgorithm) {
    super::SCHEDULER.lock().algorithm = algorithm;
}

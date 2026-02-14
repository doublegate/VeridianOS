//! Process and thread scheduling module
//!
//! Implements a multi-level scheduler with support for:
//! - Multiple scheduling algorithms (round-robin, priority, CFS)
//! - Real-time scheduling classes
//! - SMP load balancing
//! - CPU affinity
//! - Context switching for x86_64, AArch64, and RISC-V
//!
//! This module is a facade that re-exports the public API from submodules:
//! - [`scheduler`] - Core scheduler algorithm and state
//! - [`task`] - Task control block and priority types
//! - [`metrics`] - Performance metrics and measurement
//! - [`smp`] - Symmetric multiprocessing support
//! - [`queue`] - Ready queue management
//! - [`numa`] - NUMA-aware scheduling
//! - [`init`] - Initialization and timer setup
//! - [`runtime`] - Scheduler execution loop and idle task
//! - [`process_compat`] - Process compatibility wrapper types
//! - [`ipc_blocking`] - IPC blocking/waking and wait queues
//! - [`task_management`] - Task creation, exit, and thread scheduling
//! - [`load_balance`] - Load balancing and task migration

#![allow(dead_code, function_casts_as_integer)]

// ---- Submodule declarations ----

pub mod init;
pub mod ipc_blocking;
pub mod load_balance;
pub mod metrics;
pub mod numa;
pub mod process_compat;
pub mod queue;
pub mod runtime;
pub mod scheduler;
pub mod smp;
pub mod task;
pub mod task_management;
pub mod task_ptr;

#[cfg(target_arch = "riscv64")]
pub mod riscv_scheduler;

// ---- Re-exports: scheduler types ----

#[cfg(not(target_arch = "riscv64"))]
pub use queue::READY_QUEUE;
#[cfg(target_arch = "riscv64")]
pub use scheduler::SchedAlgorithm;
#[cfg(not(target_arch = "riscv64"))]
pub use scheduler::{SchedAlgorithm, SCHEDULER};

#[cfg(target_arch = "riscv64")]
pub static SCHEDULER: riscv_scheduler::RiscvScheduler = riscv_scheduler::RiscvScheduler::new();

// ---- Re-exports: initialization ----
pub use init::{init, init_with_bootstrap};
// ---- Re-exports: IPC blocking/waking ----
pub use ipc_blocking::{
    block_on_ipc, block_process, wake_up_endpoint_waiters, wake_up_process, yield_cpu,
};
// ---- Re-exports: load balancing ----
#[cfg(feature = "alloc")]
pub use load_balance::{balance_load, cleanup_dead_tasks};
// ---- Re-exports: process compatibility ----
pub use process_compat::{alloc_pid, current_process, find_process, switch_to_process, Process};
// ---- Re-exports: runtime ----
pub use runtime::{has_ready_tasks, idle_task_entry, run, set_algorithm, start, timer_tick};
pub use task::{Priority, SchedClass, SchedPolicy, Task};
pub use task_management::exit_task;
// ---- Re-exports: task management ----
#[cfg(feature = "alloc")]
pub use task_management::{create_task, create_task_from_thread, schedule_thread};

// Export functions needed by tests
#[allow(unused_imports)]
pub use self::scheduler::should_preempt;
// ---- Remaining items that stay in mod.rs ----

// Import ProcessState from process module (used by submodules via super::)
pub(crate) use crate::process::ProcessState;
// Use process module types (used by submodules via super::)
pub(crate) use crate::process::{ProcessId, ThreadId};

/// Get the current thread ID
/// TODO: Implement proper current thread tracking
pub fn get_current_thread_id() -> u64 {
    // For now, return a dummy value
    // In a real implementation, this would track the currently running thread
    1
}

/// Set current task (for testing)
///
/// # Safety
/// The caller must ensure that the task pointer is valid and properly
/// initialized
pub unsafe fn set_current_task(task: *mut Task) {
    // This is a test helper function
    let scheduler = scheduler::current_scheduler();
    let mut sched = scheduler.lock();
    if !task.is_null() {
        // SAFETY: Caller guarantees `task` is a valid, non-null pointer to a
        // properly initialized Task. `NonNull::new_unchecked` is safe here
        // because we just checked `!task.is_null()`.
        sched.current = Some(task_ptr::TaskPtr::new(core::ptr::NonNull::new_unchecked(
            task,
        )));
    } else {
        sched.current = None;
    }
}

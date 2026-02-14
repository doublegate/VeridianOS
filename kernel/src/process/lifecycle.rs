//! Process lifecycle management
//!
//! This module handles process creation, termination, and state transitions.
//! The implementation is split into focused submodules:
//! - [`creation`](super::creation): Process creation, exec, and stack setup
//! - [`fork`](super::fork): Process forking with address space cloning
//! - [`exit`](super::exit): Process exit, cleanup, signals, wait, and
//!   statistics

// Re-export everything from submodules to maintain existing API
// Re-export wait_process (used by parent module re-exports)
#[cfg(feature = "alloc")]
pub use super::exit::wait_process;
pub use super::{
    creation::{
        create_process, create_process_with_options, exec_process, ProcessCreateOptions,
        DEFAULT_KERNEL_STACK_SIZE, DEFAULT_USER_STACK_SIZE,
    },
    exit::{
        cleanup_thread, default_signal_action, exit_process, get_process_stats, kill_process,
        reap_zombie_threads, signals, ProcessStats, SignalAction, WaitOptions,
    },
    fork::fork_process,
};
use super::{pcb::Process, thread::Thread, ProcessPriority};
#[allow(unused_imports)]
use crate::{arch::context::ThreadContext, println, sched};

/// Create scheduler task for thread
///
/// This is a shared helper used by both process creation and forking.
/// It bridges the process/thread model with the scheduler's task model.
pub(super) fn create_scheduler_task(
    process: &Process,
    thread: &Thread,
) -> Result<(), &'static str> {
    // Create a sched::Task from our Thread
    // Get thread context info
    let ctx = thread.context.lock();
    let instruction_pointer = ctx.get_instruction_pointer();
    let stack_pointer = ctx.get_stack_pointer();
    drop(ctx);

    // Create a sched::Task using the constructor
    #[cfg(feature = "alloc")]
    let mut task = sched::task::Task::new(
        process.pid,
        thread.tid,
        process.name.clone(),
        instruction_pointer,
        stack_pointer,
        process.memory_space.lock().get_page_table() as usize,
    );

    // Update task fields based on thread/process state
    task.priority = match *process.priority.lock() {
        ProcessPriority::RealTime => sched::task::Priority::RealTimeHigh,
        ProcessPriority::System => sched::task::Priority::SystemHigh,
        ProcessPriority::Normal => sched::task::Priority::UserNormal,
        ProcessPriority::Low => sched::task::Priority::UserLow,
        ProcessPriority::Idle => sched::task::Priority::Idle,
    };

    task.sched_class = match *process.priority.lock() {
        ProcessPriority::RealTime => sched::task::SchedClass::RealTime,
        _ => sched::task::SchedClass::Normal,
    };

    task.time_slice = thread
        .time_slice
        .load(core::sync::atomic::Ordering::Acquire);

    // Get task pointer
    let task_ptr = core::ptr::NonNull::new(&task as *const _ as *mut _)
        .ok_or("Failed to create task pointer")?;

    // Add to scheduler
    let scheduler = sched::SCHEDULER.lock();
    scheduler.enqueue(task_ptr);

    Ok(())
}

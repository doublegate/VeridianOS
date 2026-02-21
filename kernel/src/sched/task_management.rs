//! Task creation, exit, and thread scheduling
//!
//! Provides functions to create scheduler tasks from process threads,
//! schedule them on appropriate CPUs, and handle task exit with deferred
//! cleanup.

// Task lifecycle management -- exercised via process creation/exit paths
#![allow(dead_code)]

use core::{ptr::NonNull, sync::atomic::Ordering};

#[cfg(feature = "alloc")]
use super::task;
use super::{
    scheduler, smp,
    task::{CpuSet, Priority, SchedClass, Task, TaskContext},
};
use crate::{
    arch::context::ThreadContext,
    error::KernelError,
    process::{thread::ThreadState, ProcessId as ProcId, ProcessState, ThreadId as ThrId},
};

/// Create new user task
#[cfg(feature = "alloc")]
pub fn create_task(
    name: &str,
    _entry_point: usize,
    stack_size: usize,
    priority: Priority,
) -> Result<ProcId, KernelError> {
    extern crate alloc;
    use alloc::string::String;

    // Allocate PID and TID
    let pid = super::process_compat::alloc_pid();
    let tid = task::alloc_tid();

    // TODO(phase5): Allocate stack
    let stack_base = 0; // Placeholder

    // TODO(phase5): Create page table
    let page_table = 0; // Placeholder

    // Create task
    let mut new_task = Task::new(
        pid,
        tid,
        String::from(name),
        _entry_point,
        stack_base + stack_size,
        page_table,
    );

    new_task.priority = priority;

    // TODO(phase5): Add to task table
    // For now, just enqueue it
    // let task_ptr = NonNull::new(&mut new_task as *mut _).unwrap();
    // SCHEDULER.enqueue(task_ptr);

    Ok(pid)
}

/// Exit current task
#[allow(unused_variables)]
pub fn exit_task(exit_code: i32) {
    #[cfg(feature = "alloc")]
    extern crate alloc;
    #[cfg(feature = "alloc")]
    use alloc::vec::Vec;

    #[cfg(feature = "alloc")]
    use spin::Lazy;

    /// Wrapper to make NonNull<Task> Send/Sync for the cleanup queue.
    ///
    /// # Safety
    ///
    /// The cleanup queue is protected by a spin::Mutex, ensuring exclusive
    /// access. Task pointers in the queue are only deallocated after a
    /// sufficient tick delay to ensure no other CPU holds a reference.
    #[derive(Clone, Copy)]
    struct CleanupTaskPtr(core::ptr::NonNull<Task>);

    // SAFETY: CleanupTaskPtr is only accessed under the CLEANUP_QUEUE mutex.
    // Task memory outlives the queue entry due to the deferred cleanup delay.
    unsafe impl Send for CleanupTaskPtr {}
    // SAFETY: Same as Send -- all access synchronized via mutex.
    unsafe impl Sync for CleanupTaskPtr {}

    let mut scheduler = super::SCHEDULER.lock();

    if let Some(current_task) = scheduler.current() {
        // SAFETY: `current_task` is a valid NonNull<Task> from the scheduler.
        // We hold the scheduler lock ensuring exclusive access. We update
        // the task's state to Dead, clean up thread references, and clear
        // scheduler data structure links.
        unsafe {
            let task_mut = current_task.as_ptr();
            let task_ref = &*task_mut;

            // Mark task as dead
            (*task_mut).state = ProcessState::Dead;

            // Clean up thread reference if exists
            if let Some(thread_ptr) = task_ref.thread_ref {
                // SAFETY: thread_ptr was set during task creation and points
                // to a valid Thread. We update its state and exit code.
                let thread = thread_ptr.as_ref();

                // Remove task pointer from thread
                thread.set_task_ptr(None);

                // Mark thread as dead
                thread.set_state(ThreadState::Dead);

                // Store exit code
                thread.exit_code.store(exit_code as u32, Ordering::Release);
            }

            // Clean up scheduler data structures
            // Remove from ready queue if present
            if let Some(_ready_link) = (*task_mut).ready_link {
                // TODO(phase5): Remove from ready queue
                (*task_mut).ready_link = None;
            }

            // Remove from wait queue if blocked
            if let Some(_wait_link) = (*task_mut).wait_link {
                // TODO(phase5): Remove from wait queue
                (*task_mut).wait_link = None;
            }

            // Clear current CPU assignment
            (*task_mut).current_cpu = None;

            // Mark task for deferred cleanup
            // We can't free immediately as other CPUs might have references
            #[cfg(feature = "alloc")]
            {
                // Add to cleanup queue for deferred deallocation
                static CLEANUP_QUEUE: Lazy<spin::Mutex<Vec<(CleanupTaskPtr, u64)>>> =
                    Lazy::new(|| spin::Mutex::new(Vec::new()));

                // Get current tick count for deferred cleanup
                let cleanup_tick = crate::arch::timer::get_ticks() + 100; // Cleanup after 100 ticks
                CLEANUP_QUEUE
                    .lock()
                    .push((CleanupTaskPtr(current_task), cleanup_tick));
            }
        }

        // Schedule another task
        scheduler.schedule();
    }

    // Should not return
    loop {
        crate::arch::idle();
    }
}

/// Create task from process thread
#[cfg(feature = "alloc")]
pub fn create_task_from_thread(
    process_id: ProcId,
    thread_id: ThrId,
    thread: &crate::process::Thread,
) -> Result<NonNull<Task>, KernelError> {
    extern crate alloc;
    use alloc::{boxed::Box, string::String};

    // Get thread context to extract entry point and stack
    let ctx = thread.context.lock();
    let entry_point = ctx.get_instruction_pointer();
    let kernel_stack_top = thread.kernel_stack.top();
    drop(ctx);

    // Create scheduler task from process thread
    let mut new_task = Box::new(Task::new(
        process_id,
        thread_id,
        String::from(&thread.name),
        entry_point,
        kernel_stack_top,
        0, // Will be set to process page table
    ));

    // Set priority based on thread priority (numeric value)
    new_task.priority = match thread.priority {
        0..=10 => Priority::RealTimeHigh,
        11..=20 => Priority::RealTimeNormal,
        21..=30 => Priority::RealTimeLow,
        31..=40 => Priority::SystemHigh,
        41..=50 => Priority::SystemNormal,
        51..=60 => Priority::UserHigh,
        61..=70 => Priority::UserNormal,
        71..=80 => Priority::UserLow,
        _ => Priority::Idle,
    };

    // Set scheduling class
    new_task.sched_class = if new_task.priority <= Priority::RealTimeLow {
        SchedClass::RealTime
    } else if new_task.priority == Priority::Idle {
        SchedClass::Idle
    } else {
        SchedClass::Normal
    };

    // Set CPU affinity
    new_task.cpu_affinity = CpuSet::from_mask(thread.cpu_affinity.load(Ordering::Relaxed) as u64);

    // Copy thread context - create new task context from thread context
    let thread_ctx = thread.context.lock();
    new_task.context = TaskContext::new(entry_point, kernel_stack_top);
    new_task.tls_base = thread_ctx.tls_base();
    drop(thread_ctx);

    // Set user stack
    new_task.user_stack = thread.user_stack.top();

    // Get thread pointer
    let thread_ptr = NonNull::new(thread as *const _ as *mut _);
    new_task.thread_ref = thread_ptr;

    // Get the task pointer
    // Box::leak always returns a non-null pointer
    let task_ptr =
        NonNull::new(Box::leak(new_task) as *mut _).expect("Box::leak returned null (impossible)");

    // Link thread and task bidirectionally
    thread.set_task_ptr(Some(task_ptr));

    // Return pointer to leaked task
    Ok(task_ptr)
}

/// Schedule a process thread
#[cfg(feature = "alloc")]
pub fn schedule_thread(
    process_id: ProcId,
    thread_id: ThrId,
    thread: &crate::process::Thread,
) -> Result<(), KernelError> {
    let task_ptr = create_task_from_thread(process_id, thread_id, thread)?;

    // Find best CPU for this task
    let target_cpu = if thread.cpu_affinity.load(Ordering::Relaxed) == !0usize {
        // No affinity restriction, use least loaded CPU
        smp::find_least_loaded_cpu()
    } else {
        // Find least loaded CPU that matches affinity
        let mut best_cpu = 0;
        let mut min_load = 100;
        let affinity = thread.cpu_affinity.load(Ordering::Relaxed) as u64;

        for cpu in 0..8 {
            // Check first 8 CPUs
            if (affinity & (1 << cpu)) != 0 {
                if let Some(cpu_data) = smp::per_cpu(cpu) {
                    if cpu_data.cpu_info.is_online() {
                        let load = cpu_data.cpu_info.load.load(Ordering::Relaxed);
                        if load < min_load {
                            min_load = load;
                            best_cpu = cpu;
                        }
                    }
                }
            }
        }
        best_cpu
    };

    // Schedule on target CPU
    scheduler::schedule_on_cpu(target_cpu, task_ptr);
    Ok(())
}

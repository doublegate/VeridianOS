//! IPC blocking and waking operations
//!
//! Manages task blocking and waking for IPC endpoints, including per-endpoint
//! wait queues. Tasks blocked on IPC are tracked here and woken when messages
//! arrive or endpoints become available.

use core::sync::atomic::Ordering;

use super::{metrics, scheduler, smp, task::Task};
use crate::process::{ProcessId, ProcessState};

/// Yield CPU to scheduler
pub fn yield_cpu() {
    super::SCHEDULER.lock().schedule();
}

/// Block current process on IPC
pub fn block_on_ipc(endpoint: u64) {
    let scheduler = scheduler::current_scheduler();
    let mut sched = scheduler.lock();

    if let Some(current_task) = sched.current() {
        // SAFETY: `current_task` is a valid NonNull<Task> from the scheduler.
        // We hold the scheduler lock, so no other code can concurrently modify
        // this task. We update the task's state and blocked_on fields, and
        // optionally update the linked thread's state.
        unsafe {
            let task_mut = current_task.as_ptr();
            (*task_mut).state = ProcessState::Blocked;
            (*task_mut).blocked_on = Some(endpoint);

            // Update thread state if linked
            if let Some(thread_ptr) = (*task_mut).thread_ref {
                // SAFETY: thread_ptr was set during task creation from a valid
                // Thread reference and remains valid for the task's lifetime.
                thread_ptr
                    .as_ref()
                    .set_state(crate::process::thread::ThreadState::Blocked);
            }
        }

        // Add task to wait queue for this endpoint
        add_to_wait_queue(current_task, endpoint);

        // Record IPC block metric
        metrics::SCHEDULER_METRICS.record_ipc_block();

        // Force a reschedule
        sched.schedule();
    }
}

/// Block a process (for signal handling like SIGSTOP)
/// Sets process and thread states to Blocked and triggers reschedule
pub fn block_process(pid: ProcessId) {
    // First try to find the process in wait queues or as current task
    #[cfg(feature = "alloc")]
    {
        // Check if this is the current task
        let scheduler = scheduler::current_scheduler();
        let sched = scheduler.lock();

        if let Some(current_task) = sched.current() {
            // SAFETY: `current_task` is a valid NonNull<Task> from the
            // scheduler. We hold the scheduler lock ensuring exclusive access
            // to the task. We read pid for comparison and potentially update
            // state and thread_ref fields.
            unsafe {
                if (*current_task.as_ptr()).pid == pid {
                    // This is the current task - block it
                    let task_mut = current_task.as_ptr();
                    (*task_mut).state = ProcessState::Blocked;

                    // Update thread state if linked
                    if let Some(thread_ptr) = (*task_mut).thread_ref {
                        // SAFETY: thread_ptr is valid for the task's lifetime.
                        thread_ptr
                            .as_ref()
                            .set_state(crate::process::thread::ThreadState::Blocked);
                    }

                    drop(sched);
                    // Force a reschedule
                    super::SCHEDULER.lock().schedule();
                    return;
                }
            }
        }
        drop(sched);

        // Look up process in the process table and block all its threads
        if let Some(process) = crate::process::table::get_process_mut(pid) {
            // Update process state
            process
                .state
                .store(ProcessState::Blocked as u32, Ordering::Release);

            // Block all threads in the process
            let threads = process.threads.lock();
            for (_tid, thread) in threads.iter() {
                thread.set_state(crate::process::thread::ThreadState::Blocked);

                // If thread has a task, update task state too
                if let Some(task_ptr) = thread.get_task_ptr() {
                    // SAFETY: task_ptr was set via Thread::set_task_ptr during
                    // task creation and points to a valid heap-allocated Task.
                    // We hold the process threads lock for synchronization.
                    unsafe {
                        (*task_ptr.as_ptr()).state = ProcessState::Blocked;
                    }
                }
            }

            kprintln!("[SCHED] Blocked process and all its threads");
        }
    }

    #[cfg(not(feature = "alloc"))]
    {
        let _ = pid;
    }
}

/// Wake up process blocked on IPC
pub fn wake_up_process(pid: ProcessId) {
    // First check if task is in any wait queue
    if let Some(task_ptr) = remove_from_wait_queue(pid) {
        // SAFETY: `task_ptr` was stored in the wait queue during
        // `block_on_ipc` when it was a valid NonNull<Task>. Tasks in wait
        // queues are not deallocated until explicitly removed and cleaned up.
        // We hold no other locks that could cause a deadlock.
        unsafe {
            let task_mut = task_ptr.as_ptr();
            let previous_state = (*task_mut).state;
            (*task_mut).state = ProcessState::Ready;
            (*task_mut).blocked_on = None;

            // Update thread state if linked
            if let Some(thread_ptr) = (*task_mut).thread_ref {
                // SAFETY: thread_ptr is valid for the task's lifetime.
                thread_ptr
                    .as_ref()
                    .set_state(crate::process::thread::ThreadState::Ready);
            }

            // Record IPC wakeup metric if it was blocked on IPC
            if previous_state == ProcessState::Blocked {
                metrics::SCHEDULER_METRICS.record_ipc_wakeup();
            }

            // Find the best CPU to schedule on
            let target_cpu = if (*task_mut).cpu_affinity.mask() != 0 {
                // Find least loaded CPU that matches affinity
                smp::find_least_loaded_cpu_with_affinity((*task_mut).cpu_affinity.mask())
            } else {
                // No affinity restriction, use least loaded CPU
                smp::find_least_loaded_cpu()
            };

            // Schedule on target CPU
            scheduler::schedule_on_cpu(target_cpu, task_ptr);
            return;
        }
    }

    // If not in wait queue, search all CPU ready queues
    for cpu_id in 0..smp::MAX_CPUS as u8 {
        if let Some(cpu_data) = smp::per_cpu(cpu_id) {
            if cpu_data.cpu_info.is_online() {
                // Per-CPU schedulers not yet implemented -- use global scheduler.
                // TODO(phase7): Per-CPU ready queues for O(1) wake-up.
                let sched = super::SCHEDULER.lock();

                // Search through the scheduler's tasks
                if let Some(current) = sched.current() {
                    // SAFETY: `current` is a valid NonNull<Task> from the
                    // scheduler. We hold the scheduler lock so the task won't
                    // be modified concurrently. We only read/write task fields.
                    unsafe {
                        if (*current.as_ptr()).pid == pid {
                            // Found it as current task - just update state
                            (*current.as_ptr()).state = ProcessState::Ready;
                            if let Some(thread_ptr) = (*current.as_ptr()).thread_ref {
                                thread_ptr
                                    .as_ref()
                                    .set_state(crate::process::thread::ThreadState::Ready);
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    // If still not found, try to look up in process table and create task if needed
    #[cfg(feature = "alloc")]
    {
        if let Some(process) = crate::process::table::get_process_mut(pid) {
            // Update process state
            process
                .state
                .store(ProcessState::Ready as u32, Ordering::Release);

            // Find main thread and wake it
            if let Some(main_tid) = process.get_main_thread_id() {
                // Update thread state through process
                let threads = process.threads.lock();
                if let Some(thread) = threads.get(&main_tid) {
                    thread.set_state(crate::process::thread::ThreadState::Ready);

                    // Try to schedule the thread if it has a task
                    if let Some(task_ptr) = thread.get_task_ptr() {
                        let target_cpu = smp::find_least_loaded_cpu();
                        scheduler::schedule_on_cpu(target_cpu, task_ptr);
                    }
                }
            }
        }
    }
}

/// Wake up all processes blocked on a specific endpoint
pub fn wake_up_endpoint_waiters(endpoint: u64) {
    #[cfg(feature = "alloc")]
    {
        let waiters = get_endpoint_waiters(endpoint);
        for task_ptr in waiters {
            // SAFETY: task_ptr was retrieved from the wait queue where it was
            // stored as a valid NonNull<Task>. We only read the pid field to
            // pass to wake_up_process.
            unsafe {
                let task = task_ptr.as_ref();
                wake_up_process(task.pid);
            }
        }
    }
    #[cfg(not(feature = "alloc"))]
    {
        // Without alloc, we can't maintain wait queues
        let _ = endpoint;
    }
}

// ---------------------------------------------------------------------------
// Wait queue management
// ---------------------------------------------------------------------------

/// Wait queue for blocked tasks
#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[cfg(feature = "alloc")]
use spin::Lazy;

/// Wrapper to make NonNull<Task> Send/Sync for use in wait queues.
///
/// # Safety
///
/// Tasks stored in wait queues are only accessed while holding the wait queue
/// lock (WAIT_QUEUES Mutex). The scheduler ensures tasks are not deallocated
/// while present in any wait queue.
#[derive(Clone, Copy)]
struct WaitQueueTaskPtr(core::ptr::NonNull<Task>);

// SAFETY: WaitQueueTaskPtr wraps a NonNull<Task> that is only accessed under
// the WAIT_QUEUES mutex lock, ensuring no data races. Task memory is managed
// by the kernel allocator and outlives the wait queue entry.
unsafe impl Send for WaitQueueTaskPtr {}
// SAFETY: Same as Send -- all access is synchronized through the WAIT_QUEUES
// mutex.
unsafe impl Sync for WaitQueueTaskPtr {}

#[cfg(feature = "alloc")]
static WAIT_QUEUES: Lazy<spin::Mutex<BTreeMap<u64, Vec<WaitQueueTaskPtr>>>> =
    Lazy::new(|| spin::Mutex::new(BTreeMap::new()));

#[cfg(feature = "alloc")]
fn wait_queues() -> &'static spin::Mutex<BTreeMap<u64, Vec<WaitQueueTaskPtr>>> {
    &WAIT_QUEUES
}

/// Add task to wait queue for endpoint
#[cfg(feature = "alloc")]
pub(super) fn add_to_wait_queue(task: core::ptr::NonNull<Task>, endpoint: u64) {
    let mut queues = wait_queues().lock();
    queues
        .entry(endpoint)
        .or_default()
        .push(WaitQueueTaskPtr(task));
}

/// Remove task from wait queue by PID
#[cfg(feature = "alloc")]
pub(super) fn remove_from_wait_queue(pid: ProcessId) -> Option<core::ptr::NonNull<Task>> {
    let mut queues = wait_queues().lock();

    for (_endpoint, waiters) in queues.iter_mut() {
        if let Some(pos) = waiters.iter().position(|&WaitQueueTaskPtr(task_ptr)| {
            // SAFETY: task_ptr was inserted into the wait queue as a valid
            // NonNull<Task> during block_on_ipc. Tasks in wait queues are not
            // deallocated. We only read the pid field for comparison.
            unsafe { task_ptr.as_ref().pid == pid }
        }) {
            return Some(waiters.remove(pos).0);
        }
    }

    None
}

/// Get all waiters for an endpoint
#[cfg(feature = "alloc")]
pub(super) fn get_endpoint_waiters(endpoint: u64) -> Vec<core::ptr::NonNull<Task>> {
    let mut queues = wait_queues().lock();
    queues
        .remove(&endpoint)
        .unwrap_or_default()
        .into_iter()
        .map(|WaitQueueTaskPtr(ptr)| ptr)
        .collect()
}

// Stub implementations for no_std without alloc
#[cfg(not(feature = "alloc"))]
pub(super) fn add_to_wait_queue(_task: core::ptr::NonNull<Task>, _endpoint: u64) {
    // No-op without alloc
}

#[cfg(not(feature = "alloc"))]
pub(super) fn remove_from_wait_queue(_pid: ProcessId) -> Option<core::ptr::NonNull<Task>> {
    None
}

#[cfg(not(feature = "alloc"))]
pub(super) fn get_endpoint_waiters(_endpoint: u64) -> [core::ptr::NonNull<Task>; 0] {
    []
}

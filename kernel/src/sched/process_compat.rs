//! Process compatibility wrapper types
//!
//! Provides a `Process` struct that wraps the scheduler's `Task` representation
//! to present a process-oriented view used by IPC, syscalls, and other kernel
//! subsystems.

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicPtr, AtomicU64, Ordering},
};

use super::task::Task;
use crate::process::{ProcessId, ProcessState};

/// Process structure (compatibility wrapper)
///
/// Bridges the scheduler's task-centric model with the rest of the kernel's
/// process-centric view. Contains a reference back to the underlying `Task`.
pub struct Process {
    pub pid: ProcessId,
    pub state: ProcessState,
    pub blocked_on: Option<u64>,
    /// Underlying task
    pub(super) task: Option<NonNull<Task>>,
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Allocate new process ID
pub fn alloc_pid() -> ProcessId {
    ProcessId(NEXT_PID.fetch_add(1, Ordering::Relaxed))
}

// Thread-safe current process storage using atomic pointer
static CURRENT_PROCESS_PTR: AtomicPtr<Process> = AtomicPtr::new(core::ptr::null_mut());

/// Map a task state to a process state
fn task_state_to_process_state(state: ProcessState) -> ProcessState {
    match state {
        ProcessState::Creating => ProcessState::Ready,
        ProcessState::Ready => ProcessState::Ready,
        ProcessState::Running => ProcessState::Running,
        ProcessState::Blocked => ProcessState::Blocked,
        ProcessState::Sleeping => ProcessState::Sleeping,
        ProcessState::Zombie => ProcessState::Dead,
        ProcessState::Dead => ProcessState::Dead,
    }
}

/// Get the current process
///
/// Returns a static mutable reference to a `Process` wrapper around the
/// currently scheduled task. If no task is running, returns a dummy process.
///
/// This function reuses a cached heap allocation to avoid allocating a new
/// `Box<Process>` on every call. The first call allocates; subsequent calls
/// update the existing allocation in-place.
pub fn current_process() -> &'static mut Process {
    // Get from per-CPU scheduler
    if let Some(task_ptr) = super::SCHEDULER.lock().current() {
        // SAFETY: `task_ptr` is a valid NonNull<Task> returned by the scheduler
        // which owns and manages task lifetimes. The task remains valid while it
        // is the current task. We read task fields (pid, state, blocked_on) to
        // populate a Process wrapper.
        unsafe {
            let task = task_ptr.as_ref();

            #[cfg(feature = "alloc")]
            {
                use alloc::boxed::Box;

                // Reuse the existing allocation if available, otherwise allocate once.
                // SAFETY: CURRENT_PROCESS_PTR is only modified by this function.
                // The pointer, if non-null, was created by Box::into_raw in a
                // previous call and has not been freed. We update it in-place to
                // avoid per-call allocation/deallocation churn.
                let process_ptr = CURRENT_PROCESS_PTR.load(Ordering::SeqCst);
                if !process_ptr.is_null() {
                    // Update the existing allocation in-place
                    let process = &mut *process_ptr;
                    process.pid = task.pid;
                    process.state = task_state_to_process_state(task.state);
                    process.blocked_on = task.blocked_on;
                    process.task = Some(task_ptr);
                    process
                } else {
                    // First call: allocate and leak a Process on the heap
                    let process = Box::new(Process {
                        pid: task.pid,
                        state: task_state_to_process_state(task.state),
                        blocked_on: task.blocked_on,
                        task: Some(task_ptr),
                    });
                    let new_ptr = Box::into_raw(process);
                    CURRENT_PROCESS_PTR.store(new_ptr, Ordering::SeqCst);
                    &mut *new_ptr
                }
            }

            #[cfg(not(feature = "alloc"))]
            {
                // Without alloc, fall back to static storage
                static mut CURRENT_PROCESS: Process = Process {
                    pid: ProcessId(0),
                    state: ProcessState::Running,
                    blocked_on: None,
                    task: None,
                };

                // SAFETY: This static is only accessed from the scheduler path
                // which runs with interrupts disabled on a single CPU during
                // early boot (no alloc). `addr_of_mut!` avoids creating an
                // intermediate reference to the static.
                let current_ref = &mut *core::ptr::addr_of_mut!(CURRENT_PROCESS);
                current_ref.pid = task.pid;
                current_ref.state = task_state_to_process_state(task.state);
                current_ref.blocked_on = task.blocked_on;
                current_ref.task = Some(task_ptr);

                current_ref
            }
        }
    } else {
        // No current task, return dummy process.
        // SAFETY: We reuse the cached allocation if available, or allocate once.
        // The pointer is managed through CURRENT_PROCESS_PTR and is never freed
        // during normal operation (only at kernel shutdown via Drop if applicable).
        unsafe {
            #[cfg(feature = "alloc")]
            {
                use alloc::boxed::Box;

                let process_ptr = CURRENT_PROCESS_PTR.load(Ordering::SeqCst);
                if !process_ptr.is_null() {
                    // Reuse existing allocation with dummy values
                    let process = &mut *process_ptr;
                    process.pid = ProcessId(0);
                    process.state = ProcessState::Running;
                    process.blocked_on = None;
                    process.task = None;
                    process
                } else {
                    // First call: allocate and leak a dummy Process
                    let dummy = Box::new(Process {
                        pid: ProcessId(0),
                        state: ProcessState::Running,
                        blocked_on: None,
                        task: None,
                    });
                    let new_ptr = Box::into_raw(dummy);
                    CURRENT_PROCESS_PTR.store(new_ptr, Ordering::SeqCst);
                    &mut *new_ptr
                }
            }

            #[cfg(not(feature = "alloc"))]
            {
                static mut DUMMY_PROCESS: Process = Process {
                    pid: ProcessId(0),
                    state: ProcessState::Running,
                    blocked_on: None,
                    task: None,
                };
                // SAFETY: Accessed only during early boot without alloc, single-
                // threaded context. `addr_of_mut!` avoids UB from direct static
                // mut reference.
                &mut *core::ptr::addr_of_mut!(DUMMY_PROCESS)
            }
        }
    }
}

/// Switch to another process
pub fn switch_to_process(target: &Process) {
    if let Some(task_ptr) = target.task {
        let mut scheduler = super::SCHEDULER.lock();
        scheduler.enqueue(task_ptr);
        scheduler.schedule();
    }
}

// Thread-safe found process storage using atomic pointer
static FOUND_PROCESS_PTR: AtomicPtr<Process> = AtomicPtr::new(core::ptr::null_mut());

/// Find process by PID
pub fn find_process(pid: ProcessId) -> Option<&'static mut Process> {
    // First check if it's the current process (fast path)
    let current = current_process();
    if current.pid == pid {
        return Some(current);
    }

    // Otherwise, look it up in the process table
    #[cfg(feature = "alloc")]
    {
        // Get the actual process from the process table
        if let Some(process) = crate::process::table::get_process_mut(pid) {
            use alloc::boxed::Box;

            // Create a Process wrapper for the scheduler
            let found = Box::new(Process {
                pid: process.pid,
                state: process.get_state(),
                blocked_on: None, // Would need to be tracked
                task: None,       // Would need task mapping
            });

            // SAFETY: `found` is a freshly heap-allocated Box. We leak it via
            // `into_raw` and store the pointer atomically. The old pointer
            // (from a previous call) is reclaimed via `Box::from_raw`. This is
            // safe because the atomic swap provides exclusive ownership of the
            // old allocation.
            unsafe {
                let found_ptr = Box::into_raw(found);
                let old_ptr = FOUND_PROCESS_PTR.swap(found_ptr, Ordering::SeqCst);

                if !old_ptr.is_null() {
                    drop(Box::from_raw(old_ptr));
                }

                Some(&mut *found_ptr)
            }
        } else {
            None
        }
    }

    #[cfg(not(feature = "alloc"))]
    None
}

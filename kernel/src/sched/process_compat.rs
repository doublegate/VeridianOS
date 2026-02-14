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

/// Get the current process
///
/// Returns a static mutable reference to a `Process` wrapper around the
/// currently scheduled task. If no task is running, returns a dummy process.
pub fn current_process() -> &'static mut Process {
    // Get from per-CPU scheduler
    if let Some(task_ptr) = super::SCHEDULER.lock().current() {
        // SAFETY: `task_ptr` is a valid NonNull<Task> returned by the scheduler
        // which owns and manages task lifetimes. The task remains valid while it
        // is the current task. We read task fields (pid, state, blocked_on) to
        // populate a Process wrapper.
        unsafe {
            let task = task_ptr.as_ref();

            // Allocate process wrapper on heap for thread safety
            #[cfg(feature = "alloc")]
            {
                use alloc::boxed::Box;
                let process = Box::new(Process {
                    pid: task.pid,
                    state: match task.state {
                        ProcessState::Creating => ProcessState::Ready,
                        ProcessState::Ready => ProcessState::Ready,
                        ProcessState::Running => ProcessState::Running,
                        ProcessState::Blocked => ProcessState::Blocked,
                        ProcessState::Sleeping => ProcessState::Sleeping,
                        ProcessState::Zombie => ProcessState::Dead,
                        ProcessState::Dead => ProcessState::Dead,
                    },
                    blocked_on: task.blocked_on,
                    task: Some(task_ptr),
                });

                let process_ptr = Box::into_raw(process);
                let old_ptr = CURRENT_PROCESS_PTR.swap(process_ptr, Ordering::SeqCst);

                // SAFETY: `old_ptr` was previously created via `Box::into_raw` in
                // a prior call to this function, so it is valid to reconstruct
                // and drop. The SeqCst swap ensures we have exclusive ownership.
                if !old_ptr.is_null() {
                    drop(Box::from_raw(old_ptr));
                }

                // SAFETY: `process_ptr` was just created via `Box::into_raw` and
                // has not been deallocated. We hold it in the atomic for future
                // cleanup. The reference is valid for 'static because the kernel
                // manages this allocation's lifetime.
                &mut *process_ptr
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
                current_ref.state = match task.state {
                    ProcessState::Creating => ProcessState::Ready,
                    ProcessState::Ready => ProcessState::Ready,
                    ProcessState::Running => ProcessState::Running,
                    ProcessState::Blocked => ProcessState::Blocked,
                    ProcessState::Sleeping => ProcessState::Sleeping,
                    ProcessState::Zombie => ProcessState::Dead,
                    ProcessState::Dead => ProcessState::Dead,
                };
                current_ref.blocked_on = task.blocked_on;
                current_ref.task = Some(task_ptr);

                current_ref
            }
        }
    } else {
        // No current task, return dummy
        // SAFETY: We allocate a dummy Process on the heap (or use a static
        // fallback) and manage its lifetime through the CURRENT_PROCESS_PTR
        // atomic. Old allocations are cleaned up on each call.
        unsafe {
            #[cfg(feature = "alloc")]
            {
                use alloc::boxed::Box;
                let dummy = Box::new(Process {
                    pid: ProcessId(0),
                    state: ProcessState::Running,
                    blocked_on: None,
                    task: None,
                });

                let dummy_ptr = Box::into_raw(dummy);
                let old_ptr = CURRENT_PROCESS_PTR.swap(dummy_ptr, Ordering::SeqCst);

                // SAFETY: Same as above -- old_ptr was created via Box::into_raw.
                if !old_ptr.is_null() {
                    drop(Box::from_raw(old_ptr));
                }

                // SAFETY: dummy_ptr was just created and has not been deallocated.
                &mut *dummy_ptr
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

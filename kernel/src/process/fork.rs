//! Process forking (copy-on-write)
//!
//! Implements the fork system call which creates a child process as a copy
//! of the current process. Currently uses full copy; copy-on-write (CoW)
//! optimization is deferred to Phase 5 (Performance Optimization).

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::format;

use super::{
    lifecycle::create_scheduler_task,
    pcb::{ProcessBuilder, ProcessState},
    table,
    thread::ThreadBuilder,
    ProcessId,
};
#[allow(unused_imports)]
use crate::{arch::context::ThreadContext, error::KernelError, println};

/// Fork current process
#[cfg(feature = "alloc")]
pub fn fork_process() -> Result<ProcessId, KernelError> {
    let current_process =
        super::current_process().ok_or(KernelError::ProcessNotFound { pid: 0 })?;

    let current_thread = super::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // Create new process as copy of current
    let new_process = ProcessBuilder::new(format!("{}-fork", current_process.name))
        .parent(current_process.pid)
        .priority(*current_process.priority.lock())
        .build();

    let new_pid = new_process.pid;

    // Clone address space
    {
        let current_space = current_process.memory_space.lock();
        let mut new_space = new_process.memory_space.lock();

        // Note: Currently using full copy instead of copy-on-write (CoW).
        // CoW optimization deferred to Phase 5 (Performance Optimization) as it
        // requires:
        // - Page table flags for CoW pages (read-only + CoW marker)
        // - Page fault handler integration for CoW page faults
        // - Reference counting for shared physical pages
        // - Memory zone integration for CoW tracking
        // The current implementation is correct, just less memory efficient.
        new_space.clone_from(&current_space)?;
    }

    // Clone capabilities
    {
        let current_caps = current_process.capability_space.lock();
        let new_caps = new_process.capability_space.lock();

        // Clone capability space so child has same capabilities as parent
        new_caps.clone_from(&current_caps)?;
    }

    // Clone file table so child inherits stdin/stdout/stderr and pipes
    {
        let parent_ft = current_process.file_table.lock();
        let child_ft = parent_ft.clone_for_fork();
        *new_process.file_table.lock() = child_ft;
    }

    // Inherit environment variables from parent
    #[cfg(feature = "alloc")]
    {
        let parent_env = current_process.env_vars.lock();
        let mut child_env = new_process.env_vars.lock();
        for (key, value) in parent_env.iter() {
            child_env.insert(key.clone(), value.clone());
        }
    }

    // Inherit uid, gid, pgid, sid from parent
    // (ProcessBuilder doesn't copy these, so do it manually)
    // uid/gid are non-atomic, but the new_process is not yet visible
    // to other threads, so this is safe.
    // SAFETY: new_process is not yet added to the process table, so no
    // other thread can access it concurrently.
    {
        // pgid and sid are inherited from parent per POSIX
        let parent_pgid = current_process
            .pgid
            .load(core::sync::atomic::Ordering::Acquire);
        let parent_sid = current_process
            .sid
            .load(core::sync::atomic::Ordering::Acquire);
        new_process
            .pgid
            .store(parent_pgid, core::sync::atomic::Ordering::Release);
        new_process
            .sid
            .store(parent_sid, core::sync::atomic::Ordering::Release);
    }

    // Create thread in new process matching current thread
    let new_thread = {
        let ctx = current_thread.context.lock();
        let thread = ThreadBuilder::new(
            new_pid,
            current_thread.name.clone(),
            ctx.get_instruction_pointer(),
        )
        .user_stack_size(current_thread.user_stack.size)
        .kernel_stack_size(current_thread.kernel_stack.size)
        .priority(current_thread.priority)
        .cpu_affinity(current_thread.get_affinity())
        .build()?;

        // Copy thread context and set return value to 0 for child
        {
            let mut new_ctx = thread.context.lock();
            // Clone context manually
            *new_ctx = (*ctx).clone();

            // Set return value to 0 for child
            new_ctx.set_return_value(0);
        } // Drop lock here

        thread
    };

    let new_tid = new_thread.tid;
    new_process.add_thread(new_thread)?;

    // Add to parent's children list
    #[cfg(feature = "alloc")]
    {
        current_process.children.lock().push(new_pid);
    }

    // Add process to table
    table::add_process(new_process)?;

    // Mark as ready and add to scheduler
    if let Some(process) = table::get_process(new_pid) {
        process.set_state(ProcessState::Ready);

        if let Some(thread) = process.get_thread(new_tid) {
            create_scheduler_task(process, thread)?;
        }
    }

    println!(
        "[PROCESS] Forked process {} from {}",
        new_pid.0, current_process.pid.0
    );

    // Return child PID to parent
    Ok(new_pid)
}

//! Process management module
//!
//! This module provides the core process and thread management functionality
//! for the VeridianOS microkernel, including:
//! - Process Control Block (PCB) management
//! - Thread creation and management
//! - Process lifecycle (creation, termination, state transitions)
//! - Global process table
//! - Memory space management
//! - Capability integration

#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

// Import println! macro
use crate::println;

// Re-export submodules
pub mod lifecycle;
pub mod memory;
pub mod pcb;
pub mod sync;
pub mod table;
pub mod thread;

// Re-export common types
pub use lifecycle::{exec_process, fork_process, wait_process as wait_for_child};
pub use pcb::{Process, ProcessId, ProcessPriority, ProcessState};
pub use thread::{Thread, ThreadId};

/// Maximum number of processes
pub const MAX_PROCESSES: usize = 4096;

/// Maximum threads per process
pub const MAX_THREADS_PER_PROCESS: usize = 256;

/// Process ID allocator
static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Thread ID allocator
static NEXT_TID: AtomicU64 = AtomicU64::new(1);

/// Allocate a new process ID
pub fn alloc_pid() -> ProcessId {
    ProcessId(NEXT_PID.fetch_add(1, Ordering::Relaxed))
}

/// Allocate a new thread ID
pub fn alloc_tid() -> ThreadId {
    ThreadId(NEXT_TID.fetch_add(1, Ordering::Relaxed))
}

/// Initialize process management subsystem
pub fn init() {
    println!("[PROCESS] Initializing process management...");

    // Initialize process table
    table::init();

    // Create init process (PID 1)
    #[cfg(feature = "alloc")]
    {
        use alloc::string::String;
        match lifecycle::create_process(String::from("init"), 0) {
            Ok(_pid) => {
                println!("[PROCESS] Created init process with PID {}", _pid.0);
            }
            Err(e) => panic!("[PROCESS] Failed to create init process: {}", e),
        }
    }

    println!("[PROCESS] Process management initialized");
}

/// Get current process
pub fn current_process() -> Option<&'static Process> {
    // Get from current CPU's scheduler
    if let Some(task) = crate::sched::SCHEDULER.lock().current() {
        unsafe {
            let task_ref = task.as_ref();
            table::get_process(ProcessId(task_ref.pid))
        }
    } else {
        None
    }
}

/// Get current process (alias for compatibility)
pub fn get_current_process() -> Option<&'static Process> {
    current_process()
}

/// Get current thread
pub fn current_thread() -> Option<&'static Thread> {
    // Get from current CPU's scheduler
    if let Some(task) = crate::sched::SCHEDULER.lock().current() {
        unsafe {
            let task_ref = task.as_ref();
            if let Some(process) = table::get_process(ProcessId(task_ref.pid)) {
                process.get_thread(ThreadId(task_ref.tid))
            } else {
                None
            }
        }
    } else {
        None
    }
}

/// Yield current thread
pub fn yield_thread() {
    crate::sched::yield_cpu();
}

/// Exit current thread
pub fn exit_thread(exit_code: i32) {
    if let Some(thread) = current_thread() {
        println!(
            "[PROCESS] Thread {} exiting with code {}",
            thread.tid.0, exit_code
        );

        // Mark thread as exited with state synchronization
        thread.set_exited(exit_code);

        // Never return - schedule another thread
        crate::sched::exit_task(exit_code);
    }
}

/// Block current thread
pub fn block_thread() {
    if let Some(thread) = current_thread() {
        // Update thread state to blocked
        thread.set_blocked(None);
        crate::sched::yield_cpu();
    }
}

/// Wake up a thread
pub fn wake_thread(tid: ThreadId) {
    println!("[PROCESS] Waking thread {}", tid.0);

    // Find thread in current process
    if let Some(current_process) = get_current_process() {
        let threads = current_process.threads.lock();
        if let Some(thread) = threads.get(&tid) {
            // Mark thread as ready
            thread.set_ready();

            // Wake up in scheduler if it has a task
            if let Some(task_ptr) = thread.get_task_ptr() {
                unsafe {
                    let task = task_ptr.as_ptr();
                    crate::sched::wake_up_process((*task).pid);
                }
            }
        }
    }
}

/// Create a new thread in the current process
pub fn create_thread(
    entry_point: usize,
    stack_ptr: usize,
    arg: usize,
    tls_ptr: usize,
) -> Result<ThreadId, &'static str> {
    if let Some(process) = current_process() {
        let tid = alloc_tid();

        #[cfg(feature = "alloc")]
        {
            use alloc::string::String;
            // Create thread with provided parameters
            // Note: Thread::new requires more parameters including stack info
            // For now, use default stack sizes
            let user_stack_base = 0x1000_0000; // Placeholder - should allocate
            let user_stack_size = 1024 * 1024; // 1MB
            let kernel_stack_base = 0x2000_0000; // Placeholder - should allocate
            let kernel_stack_size = 64 * 1024; // 64KB

            let thread = Thread::new(
                tid,
                process.pid,
                String::from("user_thread"),
                entry_point,
                user_stack_base,
                user_stack_size,
                kernel_stack_base,
                kernel_stack_size,
            );

            // Override the stack pointer if provided
            if stack_ptr != 0 {
                thread.user_stack.set_sp(stack_ptr);
            }

            // Set up thread-local storage if provided
            if tls_ptr != 0 {
                thread.tls.lock().base = tls_ptr;
            }

            // Store argument in a register (architecture-specific)
            // For now, we'll skip this as it requires arch-specific code
            let _ = arg;

            // Add thread to process
            process.add_thread(thread)?;
        }

        Ok(tid)
    } else {
        Err("No current process")
    }
}

/// Set thread CPU affinity
pub fn set_thread_affinity(tid: ThreadId, cpu_mask: u64) -> Result<(), &'static str> {
    if let Some(process) = current_process() {
        if let Some(thread) = process.get_thread(tid) {
            thread
                .cpu_affinity
                .store(cpu_mask as usize, Ordering::SeqCst);
            Ok(())
        } else {
            Err("Thread not found")
        }
    } else {
        Err("No current process")
    }
}

/// Get current thread ID
pub fn get_thread_tid() -> ThreadId {
    if let Some(thread) = current_thread() {
        thread.tid
    } else {
        // Fallback to main thread ID
        ThreadId(0)
    }
}

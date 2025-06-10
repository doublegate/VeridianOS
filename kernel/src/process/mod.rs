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

// Re-export submodules
pub mod pcb;
pub mod table;
pub mod thread;
pub mod lifecycle;
pub mod memory;
pub mod sync;

// Re-export common types
pub use pcb::{Process, ProcessId, ProcessState};
pub use thread::{Thread, ThreadId, ThreadState};
pub use table::PROCESS_TABLE;

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
            Ok(pid) => println!("[PROCESS] Created init process with PID {}", pid.0),
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
        println!("[PROCESS] Thread {} exiting with code {}", thread.tid.0, exit_code);
        
        // Mark thread as exited
        // TODO: Proper cleanup
        
        // Never return - schedule another thread
        crate::sched::exit_task(exit_code);
    }
}

/// Block current thread
pub fn block_thread() {
    if let Some(thread) = current_thread() {
        // TODO: Update thread state to blocked
        crate::sched::yield_cpu();
    }
}

/// Wake up a thread
pub fn wake_thread(tid: ThreadId) {
    // TODO: Find thread and wake it up
    println!("[PROCESS] Waking thread {}", tid.0);
}
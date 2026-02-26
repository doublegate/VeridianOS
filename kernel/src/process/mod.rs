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

// Process management is fully implemented but many functions are not yet
// called from user-space syscall paths. Will be exercised once the process
// lifecycle is driven by real user-space programs.

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

// Import println! macro - may be no-op on some architectures
#[allow(unused_imports)]
use crate::println;

// Re-export submodules
pub mod creation;
pub mod cwd;
pub mod exit;
pub mod fork;
pub mod lifecycle;
pub mod loader;
pub mod memory;
pub mod pcb;
pub mod signal_delivery;
pub mod sync;
pub mod table;
pub mod thread;
pub mod wait;

// Re-export common types
pub use lifecycle::{exec_process, fork_process, wait_process as wait_for_child};
pub use pcb::{Process, ProcessId, ProcessPriority, ProcessState};
pub use table::get_process;
pub use thread::{Thread, ThreadId, ThreadState};

// Re-export thread context types for compatibility
pub use crate::arch::context::{ArchThreadContext, ThreadContext};

/// Maximum number of concurrent processes (including zombies awaiting reaping).
///
/// This limit is enforced in fork() to prevent unbounded process table growth
/// during workloads like BusyBox native compilation (213+ sequential gcc
/// invocations). Zombie processes count against this limit until reaped by
/// their parent via waitpid().
pub const MAX_PROCESSES: usize = 1024;

/// Maximum threads per process
pub const MAX_THREADS_PER_PROCESS: usize = 256;

/// Process ID allocator
static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Thread ID allocator
static NEXT_TID: AtomicU64 = AtomicU64::new(1);

/// Boot-launched process tracking.
///
/// During bootstrap, user processes are launched via
/// `enter_usermode_returnable()` without registering them in the scheduler.
/// When these processes make syscalls (e.g., fork), `current_process()` queries
/// the scheduler which only knows about the idle task (pid=0). These atomics
/// provide a fallback: the bootstrap wrapper sets them before entering user
/// mode, and `current_process()`/`current_thread()` check them when the
/// scheduler returns no valid process.
///
/// Using atomics avoids the SCHEDULER lock entirely, which is critical because
/// acquiring the lock from the bootstrap stack corrupts SSE alignment (movaps
/// GP fault).
static BOOT_CURRENT_PID: AtomicU64 = AtomicU64::new(0);
static BOOT_CURRENT_TID: AtomicU64 = AtomicU64::new(0);

/// Register a boot-launched process as the current process.
///
/// Called from the bootstrap wrapper before entering user mode via
/// `enter_usermode_returnable()`. This allows `current_process()` and
/// `current_thread()` to return the correct process/thread during syscalls.
pub fn set_boot_current(pid: ProcessId, tid: ThreadId) {
    BOOT_CURRENT_PID.store(pid.0, Ordering::Release);
    BOOT_CURRENT_TID.store(tid.0, Ordering::Release);
}

/// Clear the boot-launched process tracking.
///
/// Called from the bootstrap wrapper after the user process exits and control
/// returns to the kernel bootstrap code.
pub fn clear_boot_current() {
    BOOT_CURRENT_PID.store(0, Ordering::Release);
    BOOT_CURRENT_TID.store(0, Ordering::Release);
}

/// Allocate a new process ID
pub fn alloc_pid() -> ProcessId {
    ProcessId(NEXT_PID.fetch_add(1, Ordering::Relaxed))
}

/// Allocate a new thread ID
pub fn alloc_tid() -> ThreadId {
    ThreadId(NEXT_TID.fetch_add(1, Ordering::Relaxed))
}

/// Initialize process management subsystem without creating init process
///
/// This is used during bootstrap to initialize process structures
/// without creating the init process (which requires scheduler).
pub fn init_without_init_process() -> crate::error::KernelResult<()> {
    println!("[PROCESS] Initializing process management structures...");

    // Initialize process table
    table::init();

    println!("[PROCESS] Process management structures initialized");
    Ok(())
}

/// Initialize process management subsystem (legacy)
///
/// This creates the init process, so scheduler must be initialized first.
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
            Err(_e) => {
                // Log the error but do not panic. The bootstrap sequence
                // creates its own init process as a fallback, so this path
                // is recoverable. The legacy init() path is rarely used.
                println!("[PROCESS] WARNING: Failed to create init process: {}", _e);
            }
        }
    }

    println!("[PROCESS] Process management initialized");
}

/// Get current process
pub fn current_process() -> Option<&'static Process> {
    // Get from current CPU's scheduler
    if let Some(task) = crate::sched::SCHEDULER.lock().current() {
        // SAFETY: `task` is a NonNull<Task> returned by the scheduler's
        // current() method. The scheduler guarantees the pointer is valid
        // for the lifetime of the lock. We read pid to look up the process.
        unsafe {
            let task_ref = task.as_ref();
            if let Some(proc) = table::get_process(task_ref.pid) {
                return Some(proc);
            }
        }
    }

    // Fallback: check boot-launched process atomics.
    // During bootstrap, user processes run via enter_usermode_returnable()
    // without scheduler registration. The bootstrap wrapper sets these
    // atomics so syscalls (fork, wait, etc.) can find the calling process.
    let boot_pid = BOOT_CURRENT_PID.load(Ordering::Acquire);
    if boot_pid != 0 {
        return table::get_process(ProcessId(boot_pid));
    }

    None
}

/// Find process by ID
pub fn find_process(pid: ProcessId) -> Option<&'static Process> {
    table::get_process(pid)
}

/// Get current process (alias for compatibility)
pub fn get_current_process() -> Option<&'static Process> {
    current_process()
}

/// Get current thread
pub fn current_thread() -> Option<&'static Thread> {
    // Get from current CPU's scheduler
    if let Some(task) = crate::sched::SCHEDULER.lock().current() {
        // SAFETY: `task` is a NonNull<Task> returned by the scheduler's
        // current() method. The scheduler guarantees the pointer is valid
        // for the lifetime of the lock. We read pid and tid to look up
        // the thread via the process table.
        unsafe {
            let task_ref = task.as_ref();
            if let Some(process) = table::get_process(task_ref.pid) {
                if let Some(thread) = process.get_thread(task_ref.tid) {
                    return Some(thread);
                }
            }
        }
    }

    // Fallback: check boot-launched process atomics.
    let boot_pid = BOOT_CURRENT_PID.load(Ordering::Acquire);
    let boot_tid = BOOT_CURRENT_TID.load(Ordering::Acquire);
    if boot_pid != 0 {
        if let Some(process) = table::get_process(ProcessId(boot_pid)) {
            return process.get_thread(ThreadId(boot_tid));
        }
    }

    None
}

/// Yield current thread
pub fn yield_thread() {
    crate::sched::yield_cpu();
}

/// Exit current thread
pub fn exit_thread(exit_code: i32) {
    if let (Some(thread), Some(process)) = (current_thread(), current_process()) {
        println!(
            "[PROCESS] Thread {} exiting with code {}",
            thread.tid.0, exit_code
        );

        // Mark thread as exited with state synchronization
        thread.set_exited(exit_code);

        // If detached, clean up immediately (no join will occur)
        if thread.detached.load(core::sync::atomic::Ordering::Acquire) {
            let _ = crate::process::exit::cleanup_thread(process, thread.tid);
        }

        // Handle CLONE_CHILD_CLEARTID: clear *clear_tid and futex wake
        let clear_ptr = thread.clear_tid.load(core::sync::atomic::Ordering::Acquire);
        if clear_ptr != 0 {
            unsafe {
                // Ignore copy_to_user errors here; best effort
                let _ = crate::syscall::copy_to_user(clear_ptr, &0u32);
            }
            // Wake futex waiters on that address
            let _ = crate::syscall::sys_futex_wake(clear_ptr, 1, 0);
        }

        // Never return - schedule another thread
        crate::sched::exit_task(exit_code);
    }
}

/// Terminate a specific thread
pub fn terminate_thread(pid: ProcessId, tid: ThreadId) -> crate::error::KernelResult<()> {
    if let Some(process) = find_process(pid) {
        if let Some(thread) = process.get_thread(tid) {
            println!(
                "[PROCESS] Terminating thread {} in process {}",
                tid.0, pid.0
            );

            // Mark thread as dead
            thread.set_state(thread::ThreadState::Dead);

            // Remove from scheduler if it has a task
            if let Some(task_ptr) = thread.get_task_ptr() {
                // SAFETY: task_ptr is a NonNull<Task> stored in the thread.
                // We set the task state to Dead so the scheduler will not
                // run this task again. The thread was found via a valid
                // process/thread lookup above.
                unsafe {
                    let task = task_ptr.as_ptr();
                    (*task).state = ProcessState::Dead;
                }
            }

            Ok(())
        } else {
            Err(crate::error::KernelError::ThreadNotFound { tid: tid.0 })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: pid.0 })
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
                // SAFETY: task_ptr is a NonNull<Task> stored in the thread.
                // We read the pid field to wake the process in the
                // scheduler. The thread was found via the threads lock.
                unsafe {
                    let task = task_ptr.as_ptr();
                    crate::sched::wake_up_process((*task).pid);
                }
            }
        }
    }
}

/// Create a new thread in the current process
///
/// Allocates real stack frames for the thread via the frame allocator using
/// [`ThreadBuilder`]. If `stack_ptr` is non-zero, it overrides the user stack
/// pointer. If `tls_ptr` is non-zero, it sets the TLS base address.
pub fn create_thread(
    entry_point: usize,
    stack_ptr: usize,
    arg: usize,
    tls_ptr: usize,
) -> crate::error::KernelResult<ThreadId> {
    if let Some(process) = current_process() {
        #[cfg(feature = "alloc")]
        {
            use alloc::string::String;

            use thread::ThreadBuilder;

            // Build thread with real stack allocation via ThreadBuilder
            let thread = ThreadBuilder::new(process.pid, String::from("user_thread"), entry_point)
                .user_stack_size(1024 * 1024) // 1MB user stack
                .kernel_stack_size(64 * 1024) // 64KB kernel stack
                .build()?;

            let tid = thread.tid;

            // Override the stack pointer if provided by caller
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

            Ok(tid)
        }

        #[cfg(not(feature = "alloc"))]
        {
            let _ = (entry_point, stack_ptr, arg, tls_ptr);
            Err(crate::error::KernelError::NotImplemented {
                feature: "create_thread (requires alloc)",
            })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: 0 })
    }
}

/// Set thread CPU affinity
pub fn set_thread_affinity(tid: ThreadId, cpu_mask: u64) -> crate::error::KernelResult<()> {
    if let Some(process) = current_process() {
        if let Some(thread) = process.get_thread(tid) {
            thread
                .cpu_affinity
                .store(cpu_mask as usize, Ordering::SeqCst);
            Ok(())
        } else {
            Err(crate::error::KernelError::ThreadNotFound { tid: tid.0 })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: 0 })
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

/// Get a list of all process IDs
pub fn get_process_list() -> Option<alloc::vec::Vec<u64>> {
    #[cfg(feature = "alloc")]
    {
        use table::PROCESS_TABLE;
        let mut pids = alloc::vec::Vec::new();

        // Iterate through all processes
        PROCESS_TABLE.for_each(|process| {
            pids.push(process.pid.0);
        });

        if pids.is_empty() {
            None
        } else {
            Some(pids)
        }
    }
    #[cfg(not(feature = "alloc"))]
    None
}

// get_process is already re-exported at the top of the module

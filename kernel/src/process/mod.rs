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

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

// Import println! macro - may be no-op on some architectures
#[allow(unused_imports)]
use crate::println;

// Re-export submodules
pub mod creation;
pub mod creds;
pub mod cwd;
pub mod exit;
pub mod fork;
pub mod lifecycle;
pub mod memory;
pub mod pcb;
pub mod session;
pub mod signal_delivery;
pub mod signals;
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
pub(crate) static BOOT_CURRENT_PID: AtomicU64 = AtomicU64::new(0);
pub(crate) static BOOT_CURRENT_TID: AtomicU64 = AtomicU64::new(0);
/// The scheduler task (`sched::scheduler::RUNNING_TASK`) that registered the
/// boot-launched process. The registration applies only while that task is
/// still the one running: any other task the scheduler runs in the meantime
/// keeps its own identity (W-8).
static BOOT_DISPATCH_TASK: AtomicUsize = AtomicUsize::new(0);

/// The boot-launched (pid, tid), if one is registered *and* we are running
/// in the context that registered it. Lock-free (see above).
fn boot_context() -> Option<(u64, u64)> {
    let pid = BOOT_CURRENT_PID.load(Ordering::Acquire);
    if pid == 0 {
        return None;
    }
    let running = crate::sched::scheduler::RUNNING_TASK.load(Ordering::Acquire);
    if running != BOOT_DISPATCH_TASK.load(Ordering::Acquire) {
        return None;
    }
    Some((pid, BOOT_CURRENT_TID.load(Ordering::Acquire)))
}

/// Register a boot-launched process as the current process.
///
/// Called from the bootstrap wrapper before entering user mode via
/// `enter_usermode_returnable()`. This allows `current_process()` and
/// `current_thread()` to return the correct process/thread during syscalls.
pub fn set_boot_current(pid: ProcessId, tid: ThreadId) {
    BOOT_DISPATCH_TASK.store(
        crate::sched::scheduler::RUNNING_TASK.load(Ordering::Acquire),
        Ordering::Release,
    );
    BOOT_CURRENT_TID.store(tid.0, Ordering::Release);
    BOOT_CURRENT_PID.store(pid.0, Ordering::Release);
}

/// Clear the boot-launched process tracking.
///
/// Called from the bootstrap wrapper after the user process exits and control
/// returns to the kernel bootstrap code.
pub fn clear_boot_current() {
    BOOT_CURRENT_PID.store(0, Ordering::Release);
    BOOT_CURRENT_TID.store(0, Ordering::Release);
    BOOT_DISPATCH_TASK.store(0, Ordering::Release);
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

/// The (pid, tid) of the user thread the running dispatcher task belongs
/// to (ADR 0006 stage D2). Lock-free.
fn dispatched_context() -> Option<(u64, u64)> {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::current_owner()
    }
    #[cfg(not(feature = "alloc"))]
    {
        None
    }
}

/// Get current process
pub fn current_process() -> Option<alloc::sync::Arc<Process>> {
    // A user thread running as its own dispatcher task.
    if let Some((pid, _)) = dispatched_context() {
        return table::get_process(ProcessId(pid));
    }

    // A process launched directly by the boot/cooperative dispatcher
    // (enter_usermode_returnable) is not the scheduler's current task -- the
    // scheduler still reports the dispatching task. While that dispatching
    // task is the one running, the registered boot process is the caller.
    // This check takes no locks: taking the SCHEDULER lock on the bootstrap
    // stack misaligns SSE state.
    if let Some((boot_pid, _)) = boot_context() {
        if let Some(proc) = table::get_process(ProcessId(boot_pid)) {
            return Some(proc);
        }
    }

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

    None
}

/// Like [`current_process`], but never waits for the scheduler lock:
/// returns `None` if it is held. For fault handlers that may run while the
/// interrupted code holds that lock.
pub fn try_current_process() -> Option<alloc::sync::Arc<Process>> {
    if let Some((pid, _)) = dispatched_context() {
        return table::get_process(ProcessId(pid));
    }
    if let Some((boot_pid, _)) = boot_context() {
        return table::get_process(ProcessId(boot_pid));
    }
    // The riscv64 scheduler has no try_lock; nothing on riscv64 calls this
    // from a fault handler (the user-copy fixup is x86_64-only).
    #[cfg(target_arch = "riscv64")]
    {
        current_process()
    }
    #[cfg(not(target_arch = "riscv64"))]
    {
        let sched = crate::sched::SCHEDULER.try_lock()?;
        let task = sched.current()?;
        // SAFETY: as in current_process(): the scheduler's current task
        // pointer is valid while its lock is held; only the pid is read.
        let pid = unsafe { task.as_ref().pid };
        drop(sched);
        table::get_process(pid)
    }
}

/// Find process by ID
pub fn find_process(pid: ProcessId) -> Option<alloc::sync::Arc<Process>> {
    table::get_process(pid)
}

/// Get current process (alias for compatibility)
pub fn get_current_process() -> Option<alloc::sync::Arc<Process>> {
    current_process()
}

/// Get current thread
pub fn current_thread() -> Option<alloc::sync::Arc<Thread>> {
    // Same rule as current_process(), so the two never name different
    // processes.
    if let Some((pid, tid)) = dispatched_context() {
        return table::get_process(ProcessId(pid))?.get_thread(ThreadId(tid));
    }
    if let Some((boot_pid, boot_tid)) = boot_context() {
        if let Some(process) = table::get_process(ProcessId(boot_pid)) {
            if let Some(thread) = process.get_thread(ThreadId(boot_tid)) {
                return Some(thread);
            }
        }
    }

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

    None
}

/// Start `thread` of `process` as a dispatcher task (ADR 0006 stage D2):
/// it enters ring 3 with the registers in its saved context. `inherit_fpu`
/// gives it a copy of the calling thread's live vector registers (fork,
/// clone) instead of the initial state (a new program).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub fn start_thread(
    process: &Process,
    thread: &Thread,
    inherit_fpu: bool,
) -> crate::error::KernelResult<()> {
    use crate::arch::x86_64::switch::{xsave::Area, UserFrame};

    let (frame, ctx_tls) = {
        let ctx = thread.context.lock();
        let frame =
            UserFrame::from_context(&ctx).ok_or(crate::error::KernelError::InvalidArgument {
                name: "thread context",
                value: "entry point or stack outside user space",
            })?;
        (frame, ctx.tls_base)
    };
    let cr3 = process.memory_space.lock().get_page_table();
    if cr3 == 0 {
        return Err(crate::error::KernelError::InvalidState {
            expected: "address space",
            actual: "no page table",
        });
    }
    let fs_base = if ctx_tls != 0 {
        ctx_tls
    } else {
        process.tls_fs_base.load(Ordering::Acquire)
    };
    // Loaded into IA32_FS_BASE at every switch: it must be a user address
    // (a non-canonical value faults in ring 0).
    if !crate::arch::x86_64::trap::is_user_address(fs_base) {
        return Err(crate::error::KernelError::InvalidArgument {
            name: "thread TLS base",
            value: "outside user space",
        });
    }
    let mut area = Area::new().ok_or(crate::error::KernelError::OutOfMemory {
        requested: crate::arch::x86_64::switch::xsave::size(),
        available: 0,
    })?;
    if inherit_fpu {
        // The kernel is soft-float, so the CPU still holds the calling
        // thread's user vector state.
        area.save();
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    crate::sched::dispatch::spawn_user((process.pid.0, thread.tid.0), &frame, cr3, fs_base, area)?;
    Ok(())
}

/// Run a freshly created (or exec'd) process as dispatcher tasks and wait
/// for it to end (stage D2): its first thread is started, the caller blocks
/// until the process is a zombie whose last task is off the CPU, and the
/// zombie is reaped. Returns the exit code, or 128 + the signal that killed
/// it (the shell convention). `None` if the process could not be started.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub fn run_and_wait(pid: ProcessId) -> Option<i32> {
    let process = table::get_process(pid)?;
    let thread = process.threads.lock().values().next().cloned()?;
    if start_thread(&process, &thread, false).is_err() {
        return None;
    }
    drop(thread);
    crate::sched::dispatch::PROCESS_EVENTS.wait_until(|| {
        process.get_state() == ProcessState::Zombie
            && crate::sched::dispatch::tasks_of(pid.0).is_empty()
    });
    let sig = process.term_signal.load(Ordering::Acquire);
    let code = if sig != 0 {
        128 + sig as i32
    } else {
        process.get_exit_code()
    };
    drop(process);
    table::remove_process(pid);
    Some(code)
}

/// A dispatcher task of thread (pid, tid) has been reaped (it is off every
/// CPU). Once a zombie process has no task left, its page tables -- which a
/// running thread was still using when it tore the rest down -- are freed.
#[cfg(feature = "alloc")]
pub fn user_task_reaped(pid: u64, _tid: u64) {
    let Some(process) = table::get_process(ProcessId(pid)) else {
        return;
    };
    if process.get_state() != ProcessState::Zombie
        || !crate::sched::dispatch::tasks_of(pid).is_empty()
    {
        return;
    }
    let root = {
        let vas = process.memory_space.lock();
        let root = vas.get_page_table();
        vas.set_page_table(0);
        root
    };
    if root != 0 {
        crate::mm::vas::free_user_page_table_frames(root);
    }
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
}

/// Whether a dispatched thread waiting in a system call must stop waiting
/// and fail with EINTR: its process has a fatal signal to act on. The
/// signal is acted on at the system-call exit, once the call has unwound.
pub fn wait_interrupted() -> bool {
    if dispatched_context().is_none() {
        return false;
    }
    let (Some(process), Some(thread)) = (current_process(), current_thread()) else {
        return false;
    };
    process.kill_pending.load(Ordering::Acquire) != 0
        || signals::deliverable(&process, &thread) != 0
}

/// Run on every return to user mode of a dispatched thread: a thread whose
/// process received a fatal signal exits here instead (the signal is acted
/// on by the process's own threads, never torn down under them).
pub extern "C" fn user_return_check() {
    #[cfg(feature = "alloc")]
    if dispatched_context().is_some() {
        if let Some(process) = current_process() {
            let sig = process.kill_pending.load(Ordering::Acquire);
            if sig == exit::GROUP_EXIT_PENDING {
                drop(process);
                exit::exit_dispatched(0, true);
            } else if sig != 0 {
                drop(process);
                let _ = crate::syscall::process::exit_current(0, sig);
            }
        }
    }
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

        // Handle CLONE_CHILD_CLEARTID first, while the thread is intact:
        // clear *clear_tid and wake one futex waiter (best effort).
        let clear_ptr = thread.clear_tid.load(core::sync::atomic::Ordering::Acquire);
        if clear_ptr != 0 {
            let _ = crate::syscall::userspace::write_user(clear_ptr, 0u32);
            let _ = crate::syscall::sys_futex_wake(clear_ptr, 1, 0);
        }

        // Mark thread as exited with state synchronization
        thread.set_exited(exit_code);

        // A detached thread is never joined, so it must be reaped -- but not
        // here: cleanup_thread frees the kernel stack this code is running
        // on and drops `thread` (PROC-SEC-02). Defer it to the idle loop.
        #[cfg(feature = "alloc")]
        if thread.detached.load(core::sync::atomic::Ordering::Acquire) {
            crate::sched::load_balance::defer_thread_reap(process.pid, thread.tid);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sched::scheduler::RUNNING_TASK;

    /// W-8: a boot-launched registration applies only while the task that
    /// registered it is running; any other scheduled task keeps its own
    /// identity, and clearing removes it.
    #[test]
    fn boot_context_is_scoped_to_the_registering_task() {
        let saved = RUNNING_TASK.load(Ordering::Acquire);

        RUNNING_TASK.store(0x1000, Ordering::Release);
        set_boot_current(ProcessId(77), ThreadId(78));
        assert_eq!(boot_context(), Some((77, 78)));

        RUNNING_TASK.store(0x2000, Ordering::Release); // another task runs
        assert_eq!(boot_context(), None);

        RUNNING_TASK.store(0x1000, Ordering::Release); // dispatcher resumes
        assert_eq!(boot_context(), Some((77, 78)));

        clear_boot_current();
        assert_eq!(boot_context(), None);

        RUNNING_TASK.store(saved, Ordering::Release);
    }
}

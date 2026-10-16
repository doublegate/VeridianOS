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
pub mod creds;
pub mod cwd;
pub mod exit;
pub mod fork;
pub mod lifecycle;
pub mod memory;
pub mod pcb;
pub mod rlimit;
pub mod robust_list;
pub mod session;
pub mod signal_delivery;
pub mod signals;
pub mod table;
pub mod thread;

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

/// Process and thread IDs share one space, as on Linux: a process's first
/// thread has the process's ID as its thread ID (`ThreadBuilder::tid`), and
/// every other thread an ID no process has, so gettid, tkill and tgkill name
/// threads and processes without ambiguity (N-217).
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a new process ID
pub fn alloc_pid() -> ProcessId {
    ProcessId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// Allocate a new thread ID for a thread other than a process's first
/// (from the same space as process IDs).
pub fn alloc_tid() -> ThreadId {
    ThreadId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
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

/// Get current process: the process of the user thread running as the
/// current dispatcher task. `None` for kernel threads and the boot task (the
/// old scheduler's current task and the boot-launched registration went in
/// sprint D4).
pub fn current_process() -> Option<alloc::sync::Arc<Process>> {
    let (pid, _) = dispatched_context()?;
    table::get_process(ProcessId(pid))
}

/// Like [`current_process`]; takes no scheduler lock, so fault handlers may
/// use it whatever the interrupted code holds.
pub fn try_current_process() -> Option<alloc::sync::Arc<Process>> {
    current_process()
}

/// Find process by ID
pub fn find_process(pid: ProcessId) -> Option<alloc::sync::Arc<Process>> {
    table::get_process(pid)
}

/// Get current process (alias for compatibility)
pub fn get_current_process() -> Option<alloc::sync::Arc<Process>> {
    current_process()
}

/// Get current thread (same rule as [`current_process`], so the two never
/// name different processes).
pub fn current_thread() -> Option<alloc::sync::Arc<Thread>> {
    let (pid, tid) = dispatched_context()?;
    table::get_process(ProcessId(pid))?.get_thread(ThreadId(tid))
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
    // A new thread or a fork child inherits the caller's GS base (Linux
    // copies it with the thread); a fresh program starts with 0.
    let mut gs_base = 0;
    if inherit_fpu {
        // The kernel is soft-float, so the CPU still holds the calling
        // thread's user vector state.
        area.save();
        gs_base = crate::arch::x86_64::switch::read_user_gs_base();
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    let policy = thread.sched.lock().to_policy();
    crate::sched::dispatch::spawn_user(
        (process.pid.0, thread.tid.0),
        &frame,
        cr3,
        fs_base,
        area,
        gs_base,
        policy,
        thread.get_affinity() as u64,
    )?;
    Ok(())
}

/// RISC-V: start `thread` as a dispatcher task entering user mode with the
/// registers of its saved context (pc, sp, gp, tp, the argument registers).
/// User FP state is not switched yet, so `inherit_fpu` has nothing to copy.
#[cfg(all(feature = "alloc", target_arch = "riscv64", target_os = "none"))]
pub fn start_thread(
    process: &Process,
    thread: &Thread,
    inherit_fpu: bool,
) -> crate::error::KernelResult<()> {
    use crate::arch::riscv64::switch::UserFrame;
    let _ = inherit_fpu;
    let frame = {
        let ctx = thread.context.lock();
        let user = crate::syscall::userspace::is_user_address;
        if !user(ctx.pc) || !user(ctx.sp) {
            return Err(crate::error::KernelError::InvalidArgument {
                name: "thread context",
                value: "entry point or stack outside user space",
            });
        }
        let mut f = UserFrame::default();
        let regs = [
            ctx.ra, ctx.sp, ctx.gp, ctx.tp, ctx.t0, ctx.t1, ctx.t2, ctx.s0, ctx.s1, ctx.a0, ctx.a1,
            ctx.a2, ctx.a3, ctx.a4, ctx.a5, ctx.a6, ctx.a7, ctx.s2, ctx.s3, ctx.s4, ctx.s5, ctx.s6,
            ctx.s7, ctx.s8, ctx.s9, ctx.s10, ctx.s11, ctx.t3, ctx.t4, ctx.t5, ctx.t6,
        ];
        for (i, r) in regs.iter().enumerate() {
            f.x[i + 1] = *r as u64;
        }
        f.sepc = ctx.pc as u64;
        f
    };
    let root = process.memory_space.lock().get_page_table();
    if root == 0 {
        return Err(crate::error::KernelError::InvalidState {
            expected: "address space",
            actual: "no page table",
        });
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    let policy = thread.sched.lock().to_policy();
    crate::sched::dispatch::spawn_user(
        (process.pid.0, thread.tid.0),
        &frame,
        root,
        policy,
        thread.get_affinity() as u64,
    )?;
    Ok(())
}

/// AArch64: start `thread` as a dispatcher task entering EL0 with the
/// registers of its saved context (x0-x30, sp, pc) and its TLS register
/// (TPIDR_EL0). User FP state is not switched yet (FP is trapped at EL0),
/// so `inherit_fpu` has nothing to copy.
#[cfg(all(feature = "alloc", target_arch = "aarch64", target_os = "none"))]
pub fn start_thread(
    process: &Process,
    thread: &Thread,
    inherit_fpu: bool,
) -> crate::error::KernelResult<()> {
    use crate::arch::aarch64::switch::UserFrame;
    let _ = inherit_fpu;
    let (frame, tls) = {
        let ctx = thread.context.lock();
        let user = crate::syscall::userspace::is_user_address;
        if !user(ctx.pc as usize) || !user(ctx.sp as usize) {
            return Err(crate::error::KernelError::InvalidArgument {
                name: "thread context",
                value: "entry point or stack outside user space",
            });
        }
        let f = UserFrame {
            x: ctx.x,
            sp: ctx.sp,
            elr: ctx.pc,
            spsr: 0,
        };
        (f, ctx.tpidr_el0)
    };
    let root = process.memory_space.lock().get_page_table();
    if root == 0 {
        return Err(crate::error::KernelError::InvalidState {
            expected: "address space",
            actual: "no page table",
        });
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    let policy = thread.sched.lock().to_policy();
    crate::sched::dispatch::spawn_user(
        (process.pid.0, thread.tid.0),
        &frame,
        root,
        tls,
        policy,
        thread.get_affinity() as u64,
    )?;
    Ok(())
}

/// Run a freshly created (or exec'd) process as dispatcher tasks and wait
/// for it to end (stage D2): its first thread is started, the caller blocks
/// until the process is a zombie whose last task is off the CPU, and the
/// zombie is reaped. Returns the exit code, or 128 + the signal that killed
/// it (the shell convention). `None` if the process could not be started.
#[cfg(all(
    feature = "alloc",
    any(
        target_arch = "x86_64",
        all(
            any(target_arch = "riscv64", target_arch = "aarch64"),
            target_os = "none"
        )
    )
))]
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

/// The process of a dispatcher task about to be reaped, looked up before
/// the task leaves the task table: once it has, a waiting parent may remove
/// the process, and [`user_task_reaped`] must still free its page tables.
#[cfg(feature = "alloc")]
pub fn process_for_reap(pid: u64) -> Option<alloc::sync::Arc<Process>> {
    table::get_process(ProcessId(pid))
}

/// A dispatcher task of `process` has been reaped (it is off every CPU and
/// out of the task table). Once a zombie process has no task left, its page
/// tables -- which a running thread was still using when it tore the rest
/// down -- are freed. Runs in the reaper thread: it may sleep.
#[cfg(feature = "alloc")]
pub fn user_task_reaped(process: Option<alloc::sync::Arc<Process>>, tid: u64) {
    // A thread other than the main one that has exited while its process
    // lives on is gone for good, as on Linux: its entry (and the kernel
    // stack its builder allocated) is freed now. C libraries join through
    // the CLONE_CHILD_CLEARTID futex, not the kernel, so nothing else ever
    // removed it, and a process stopped creating threads after 255.
    if let Some(p) = &process {
        let exited = p.get_thread(ThreadId(tid)).is_some_and(|t| {
            matches!(
                t.get_state(),
                thread::ThreadState::Zombie | thread::ThreadState::Dead
            )
        });
        if tid != p.pid.0 && exited && p.get_state() != ProcessState::Zombie {
            let _ = exit::cleanup_thread(p, ThreadId(tid));
        }
    }
    // Every reap: an exec waits for its process's other tasks to go
    // (N-101), not only for the whole process.
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
    let Some(process) = process else {
        return;
    };
    if process.get_state() != ProcessState::Zombie
        || !crate::sched::dispatch::tasks_of(process.pid.0).is_empty()
    {
        return;
    }
    free_page_tables(&process);
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
}

/// Free a process's page tables (its root and what is left below it). Only
/// once no CPU can be running in its address space.
#[cfg(feature = "alloc")]
pub(crate) fn free_page_tables(process: &Process) {
    let root = {
        let vas = process.memory_space.lock();
        let root = vas.get_page_table();
        vas.set_page_table(0);
        root
    };
    if root != 0 {
        crate::mm::vas::free_user_page_table_frames(root);
    }
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
    match process.kill_pending.load(Ordering::Acquire) {
        0 => signals::deliverable(&process, &thread) != 0,
        // An exec in progress interrupts every thread but the exec'ing one.
        exit::EXEC_PENDING => process.exec_tid.load(Ordering::Acquire) != thread.tid.0,
        _ => true,
    }
}

/// Run on every return to user mode of a dispatched thread: a thread whose
/// process received a fatal signal exits here instead (the signal is acted
/// on by the process's own threads, never torn down under them).
pub extern "C" fn user_return_check() {
    #[cfg(feature = "alloc")]
    if dispatched_context().is_some() {
        if let Some(process) = current_process() {
            // RLIMIT_CPU: SIGXCPU or SIGKILL, acted on just below or by
            // the signal delivery that follows.
            rlimit::check_cpu(&process);
            let sig = process.kill_pending.load(Ordering::Acquire);
            if sig == exit::GROUP_EXIT_PENDING {
                drop(process);
                exit::exit_dispatched(0, true);
            } else if sig == exit::EXEC_PENDING {
                // Another thread is exec'ing: this one leaves.
                let me = current_thread().map(|t| t.tid.0);
                if me != Some(process.exec_tid.load(Ordering::Acquire)) {
                    drop(process);
                    exit::exit_dispatched(0, false);
                }
            } else if sig != 0 {
                drop(process);
                let _ = crate::syscall::process::exit_current(0, sig);
            }
        }
    }
}

/// Exit the calling thread (pthread_exit). Every user thread is a
/// dispatcher task, which leaves through `exit::exit_dispatched` (robust
/// futexes, CLONE_CHILD_CLEARTID, CPU time, and the process's teardown if it
/// is the last thread). Returns only when called outside a user thread.
pub fn exit_thread(exit_code: i32) {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        exit::exit_dispatched(exit_code, false);
    }
    let _ = exit_code;
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
            Ok(())
        } else {
            Err(crate::error::KernelError::ThreadNotFound { tid: tid.0 })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: pid.0 })
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
            let sched = current_thread()
                .map(|t| t.sched.lock().for_child())
                .unwrap_or_default();
            let thread = ThreadBuilder::new(process.pid, String::from("user_thread"), entry_point)
                .sched(sched)
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

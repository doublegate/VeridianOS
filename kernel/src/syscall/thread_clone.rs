//! Thread clone syscall implementation (CLONE_VM | CLONE_FILES | CLONE_SIGHAND
//! | CLONE_FS | CLONE_THREAD).
//!
//! Creates a new thread in the current process sharing address space and
//! resources.

#[cfg(feature = "alloc")]
use crate::process::thread::ThreadFs;
use crate::{
    arch::context::ThreadContext,
    process::{self, thread::ThreadBuilder, ProcessState},
    sched,
    syscall::{userspace::validate_user_ptr, SyscallError},
};

// Allowed clone flags (subset of Linux, thread-oriented)
const CLONE_VM: usize = 0x0000_0100;
const CLONE_FS: usize = 0x0000_0200;
const CLONE_FILES: usize = 0x0000_0400;
const CLONE_SIGHAND: usize = 0x0000_0800;
const CLONE_THREAD: usize = 0x0001_0000;
/// Share SysV semaphore undo lists (musl always sets this for pthreads).
/// Accepted but ignored -- VeridianOS has no SysV semaphore undo tracking.
const CLONE_SYSVSEM: usize = 0x0004_0000;
const CLONE_SETTLS: usize = 0x0008_0000;
const CLONE_PARENT_SETTID: usize = 0x0010_0000;
const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
/// Deprecated flag set by musl's pthread_create (ignored on Linux 2.6+).
/// Accepted but ignored for compatibility.
const CLONE_DETACHED: usize = 0x0040_0000;
const CLONE_CHILD_SETTID: usize = 0x0100_0000;

/// Upper bound of the user-space address range.  Any stack pointer must be
/// strictly below this address.  The value matches the canonical user-space
/// limit on x86_64; AArch64 and RISC-V use the same logical limit via
/// `validate_user_ptr`.
use crate::mm::user_layout::USER_SPACE_END;

/// Create a new thread sharing the current process's address space and
/// resources, following the Linux `clone(2)` semantics for thread creation.
///
/// The new thread shares the parent's virtual memory (`CLONE_VM`), file
/// descriptor table (`CLONE_FILES`), signal handlers (`CLONE_SIGHAND`), and
/// thread group (`CLONE_THREAD`).  These four flags are mandatory.  Optional
/// flags include:
///
/// - `CLONE_FS` — share current working directory and umask.  If not set the
///   child gets an independent copy.
/// - `CLONE_SETTLS` — set the TLS base register (FS on x86_64, TPIDR_EL0 on
///   AArch64, `tp` on RISC-V) to `tls`.
/// - `CLONE_PARENT_SETTID` — write the child's TID to `*parent_tid_ptr` in the
///   parent's address space before the child starts running.
/// - `CLONE_CHILD_SETTID` — write the child's TID to `*child_tid_ptr` in the
///   child's address space.
/// - `CLONE_CHILD_CLEARTID` — register `child_tid_ptr` so that when the child
///   exits, the kernel zeroes `*child_tid_ptr` and performs a `FUTEX_WAKE` on
///   that address (used by `pthread_join`).
///
/// # Arguments
///
/// * `flags`          - Combination of `CLONE_*` flags.
/// * `newsp`          - User-space stack pointer for the new thread (must be
///   non-zero and within user space).
/// * `parent_tid_ptr` - Address to write child TID if `CLONE_PARENT_SETTID`.
/// * `child_tid_ptr`  - Address for `CLONE_CHILD_SETTID` /
///   `CLONE_CHILD_CLEARTID`.
/// * `tls`            - TLS base if `CLONE_SETTLS` is set.
///
/// # Returns
///
/// `Ok(tid)` with the new thread's TID on success, or a `SyscallError` on
/// failure.
pub fn sys_thread_clone(
    flags: usize,
    newsp: usize,
    parent_tid_ptr: usize,
    child_tid_ptr: usize,
    tls: usize,
) -> Result<usize, SyscallError> {
    // Validate flags
    let required = CLONE_VM | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;
    if flags & required != required {
        return Err(SyscallError::InvalidArgument);
    }
    // Reject unsupported flags
    let unsupported = flags
        & !(CLONE_VM
            | CLONE_FS
            | CLONE_FILES
            | CLONE_SIGHAND
            | CLONE_THREAD
            | CLONE_SYSVSEM
            | CLONE_SETTLS
            | CLONE_PARENT_SETTID
            | CLONE_CHILD_CLEARTID
            | CLONE_DETACHED
            | CLONE_CHILD_SETTID);
    if unsupported != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // Share FS? (CWD and umask). If not set, clone gets its own copy.
    let share_fs = flags & CLONE_FS != 0;

    // The TLS base is loaded into IA32_FS_BASE whenever the thread runs; a
    // non-canonical value would fault (#GP) in ring 0 at every switch.
    #[cfg(target_arch = "x86_64")]
    if flags & CLONE_SETTLS != 0 && !crate::arch::x86_64::trap::is_user_address(tls as u64) {
        return Err(SyscallError::InvalidArgument);
    }

    // Basic user stack validation
    if newsp == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    // Ensure the stack pointer is within the user-space address range (a
    // stack top may equal the exclusive end). Kernel addresses must be
    // rejected to prevent a malicious caller from running a thread with
    // a kernel-space stack.
    if newsp > USER_SPACE_END {
        return Err(SyscallError::InvalidPointer);
    }

    // Validate TID pointers if requested
    if flags & CLONE_PARENT_SETTID != 0 {
        validate_user_ptr(parent_tid_ptr as *const u32, core::mem::size_of::<u32>())?;
    }
    if flags & CLONE_CHILD_SETTID != 0 {
        validate_user_ptr(child_tid_ptr as *const u32, core::mem::size_of::<u32>())?;
    }
    if flags & CLONE_CHILD_CLEARTID != 0 {
        validate_user_ptr(child_tid_ptr as *const u32, core::mem::size_of::<u32>())?;
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Clone current user context so child resumes at same PC with retval=0.
    // The caller pushed start_routine/arg on the user stack before invoking clone.
    let current_thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let current_ctx = current_thread.context.lock();
    let mut builder = ThreadBuilder::new(
        proc.pid,
        "clone-thread".into(),
        current_ctx.get_instruction_pointer(),
    )
    .kernel_stack_size(process::creation::DEFAULT_KERNEL_STACK_SIZE)
    // The thread runs on the caller's stack (`newsp`, required above): no
    // kernel-chosen stack is mapped for it. One used to be, 256 KiB per
    // thread that nothing ran on (N-114).
    .user_stack_size(0);

    #[cfg(feature = "alloc")]
    {
        let parent_fs = current_thread.fs();
        let child_fs = if share_fs {
            ThreadFs::clone_shared(&parent_fs)
        } else {
            ThreadFs::clone_copy(&parent_fs)
        };
        builder = builder.fs(child_fs);
    }

    if flags & CLONE_SETTLS != 0 {
        builder = builder.tls_base(tls);
    }
    if flags & CLONE_CHILD_CLEARTID != 0 {
        builder = builder.clear_tid(child_tid_ptr);
    }

    let thread = builder.build().map_err(|_| SyscallError::InvalidState)?;
    let tid = thread.tid;
    // A new thread starts with its creator's blocked mask (N-109).
    thread.sigmask.store(
        current_thread
            .sigmask
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );

    // Override context with cloned registers so the child returns 0 from clone.
    //
    // The thread's context was initialized from the exec/load entry point, but
    // the parent is mid-syscall: the *live* registers (RIP = return address
    // after SYSCALL, callee-saved GPRs) are on the kernel stack in the
    // SyscallFrame, NOT in the stale ThreadContext.  We clone the stale
    // context as a baseline, then overlay the live values from SyscallFrame.
    {
        let mut child_ctx = thread.context.lock();
        ThreadContext::clone_from(&mut *child_ctx, &*current_ctx);

        // Overlay live registers from the SyscallFrame saved by syscall_entry.
        // Without this, the child's RIP is the process entry point (from exec)
        // instead of the instruction after the clone() syscall.
        #[cfg(target_arch = "x86_64")]
        if let Some(frame) = crate::arch::x86_64::syscall::get_syscall_frame() {
            child_ctx.rip = frame.rip;
            child_ctx.rflags = frame.rflags;
            child_ctx.rbx = frame.rbx;
            child_ctx.rbp = frame.rbp;
            child_ctx.r12 = frame.r12;
            child_ctx.r13 = frame.r13;
            child_ctx.r14 = frame.r14;
            child_ctx.r15 = frame.r15;
            child_ctx.rdi = frame.rdi;
            child_ctx.rsi = frame.rsi;
            child_ctx.rdx = frame.rdx;
            child_ctx.r8 = frame.r8;
            child_ctx.r9 = frame.r9;
            child_ctx.r10 = frame.r10;
        }

        child_ctx.set_stack_pointer(newsp);
        child_ctx.set_return_value(0); // child sees 0 return
                                       // Apply requested TLS base after cloning parent context
        if flags & CLONE_SETTLS != 0 {
            child_ctx.set_tls_base(tls as u64);
        }
    }
    drop(current_ctx);

    // Both TID writes happen before the thread joins the process and before
    // its task exists: either can fault, and failing after the enqueue left
    // a runnable child behind an error return (review of the v0.26.0 stack,
    // PR #14). Here a failure only drops the unregistered `thread`.
    //
    // CLONE_PARENT_SETTID: write the child's TID into the parent's address
    // space at `parent_tid_ptr`, so the parent can observe the TID
    // immediately after clone returns.
    if flags & CLONE_PARENT_SETTID != 0 {
        // SAFETY: `parent_tid_ptr` was validated above via `validate_user_ptr`.
        // `copy_to_user` performs its own bounds check as a defence-in-depth
        // measure.  The write is a single u32 which is naturally aligned.
        unsafe {
            crate::syscall::userspace::copy_to_user(parent_tid_ptr, &(tid.0 as u32))
                .map_err(|_| SyscallError::InvalidPointer)?;
        }
    }

    // CLONE_CHILD_SETTID: write the child's TID into `child_tid_ptr`.
    // TODO(tier7): On Linux this write happens in the *child's* address
    // space after the child begins execution.  Because CLONE_VM is
    // mandatory here (shared address space), writing from the parent
    // context is equivalent.  For full fork() support (separate address
    // spaces) this would need to be deferred to the child's first
    // scheduling quantum.
    if flags & CLONE_CHILD_SETTID != 0 {
        // SAFETY: `child_tid_ptr` was validated above via `validate_user_ptr`.
        // The pointer targets shared user memory (CLONE_VM is set).
        unsafe {
            crate::syscall::userspace::copy_to_user(child_tid_ptr, &(tid.0 as u32))
                .map_err(|_| SyscallError::InvalidPointer)?;
        }
    }

    // Add to the process first: the task keeps a pointer to the thread, so
    // it must be created from the thread's final, shared (Arc) location.
    // It used to be created from this local, which add_thread() then moved,
    // leaving the task's thread_ref pointing at a dead stack slot.
    let thread = proc
        .add_thread(thread)
        .map_err(|_| SyscallError::InvalidState)?;

    // A dispatched process's new thread runs as its own task (stage D2),
    // starting from the registers prepared above and a copy of the caller's
    // vector state. It must not also go on the old scheduler's queue, where
    // the old nested dispatch would run a second copy of it.
    #[cfg(target_arch = "x86_64")]
    if sched::dispatch::current_owner().is_some() {
        crate::process::start_thread(&proc, &thread, true)
            .map_err(|_| SyscallError::OutOfMemory)?;
        return Ok(tid.0 as usize);
    }

    // Create scheduler task
    let task_ptr = sched::create_task_from_thread(proc.pid, tid, &thread)
        .map_err(|_| SyscallError::InvalidState)?;

    // Mark ready
    proc.set_state(ProcessState::Ready);

    // Enqueue the child thread in the scheduler so it actually gets CPU time.
    // Without this, the child task exists but is never placed on a ready queue,
    // causing the parent to spin forever in futex_wait (pthread_create waits
    // for the child to write its TID).
    sched::SCHEDULER.lock().enqueue(task_ptr);

    // TODO(tier7): CLONE_CHILD_CLEARTID is registered via `builder.clear_tid()`
    // above.  On thread exit the kernel should:
    //   1. Write 0 to `*child_tid_ptr` (in the thread's address space).
    //   2. Perform a FUTEX_WAKE on `child_tid_ptr` to unblock any `pthread_join`
    //      waiters.
    // This is currently handled by the thread exit path if the clear_tid
    // field is set; verify integration in the exit syscall.

    Ok(tid.0 as usize)
}

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
const CLONE_SETTLS: usize = 0x0008_0000;
const CLONE_PARENT_SETTID: usize = 0x0010_0000;
const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
const CLONE_CHILD_SETTID: usize = 0x0100_0000;

/// Create a new thread sharing the current process resources.
///
/// Args:
/// - flags: must include CLONE_VM|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD
///   (optional CLONE_FS)
/// - newsp: user stack pointer for new thread
/// - parent_tid: (ignored for now)
/// - child_tid: (ignored for now)
/// - tls: TLS base pointer for new thread (arch-specific FS/TP base)
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
            | CLONE_SETTLS
            | CLONE_PARENT_SETTID
            | CLONE_CHILD_CLEARTID
            | CLONE_CHILD_SETTID);
    if unsupported != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // Share FS? (CWD and umask). If not set, clone gets its own copy.
    let share_fs = flags & CLONE_FS != 0;

    // Basic user stack validation
    if newsp == 0 {
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
    .user_stack_size(process::creation::DEFAULT_USER_STACK_SIZE);

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

    // Override context with cloned registers so the child returns 0 from clone
    {
        let mut child_ctx = thread.context.lock();
        ThreadContext::clone_from(&mut *child_ctx, &*current_ctx);
        child_ctx.set_stack_pointer(newsp);
        child_ctx.set_return_value(0); // child sees 0 return
                                       // Apply requested TLS base after cloning parent context
        if flags & CLONE_SETTLS != 0 {
            child_ctx.set_tls_base(tls as u64);
        }
    }
    drop(current_ctx);

    // Map user stack pages for the new thread
    {
        let mut vas = proc.memory_space.lock();
        let stack_base = thread.user_stack.base;
        let stack_size = thread.user_stack.size;
        let flags = crate::mm::PageFlags::PRESENT
            | crate::mm::PageFlags::USER
            | crate::mm::PageFlags::WRITABLE
            | crate::mm::PageFlags::NO_EXECUTE;
        let pages = stack_size / 4096;
        for i in 0..pages {
            let vaddr = stack_base + i * 4096;
            vas.map_page(vaddr, flags)
                .map_err(|_| SyscallError::InvalidState)?;
        }
    }

    // Create scheduler task
    let _task_ptr = sched::create_task_from_thread(proc.pid, tid, &thread)
        .map_err(|_| SyscallError::InvalidState)?;

    // Add to process
    proc.add_thread(thread)
        .map_err(|_| SyscallError::InvalidState)?;

    // Mark ready
    proc.set_state(ProcessState::Ready);

    // Parent_tid write
    if flags & CLONE_PARENT_SETTID != 0 {
        unsafe {
            crate::syscall::userspace::copy_to_user(parent_tid_ptr, &(tid.0 as u32))
                .map_err(|_| SyscallError::InvalidPointer)?;
        }
    }

    if flags & CLONE_CHILD_SETTID != 0 {
        unsafe {
            crate::syscall::userspace::copy_to_user(child_tid_ptr, &(tid.0 as u32))
                .map_err(|_| SyscallError::InvalidPointer)?;
        }
    }

    Ok(tid.0 as usize)
}

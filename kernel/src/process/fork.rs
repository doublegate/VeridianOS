//! Process forking (copy-on-write)
//!
//! Implements the fork system call, which creates a child process as a copy
//! of the current process. The address space is shared copy-on-write
//! (`VirtualAddressSpace::clone_from`): writable pages become read-only in
//! both parent and child, and the page fault handler gives the first writer
//! a private copy.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::format;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::{
    lifecycle::create_scheduler_task,
    pcb::{ProcessBuilder, ProcessState},
    table,
    thread::ThreadBuilder,
    ProcessId,
};
#[allow(unused_imports)]
use crate::{arch::context::ThreadContext, error::KernelError, println};

/// How a clone that makes a new process (not a thread) differs from a
/// plain fork (N-210). `Default` is fork.
#[derive(Debug, Clone, Copy)]
pub struct ForkOptions {
    /// CLONE_VM: the child maps the parent's memory itself
    /// (`VirtualAddressSpace::share_from`), not a copy-on-write copy.
    pub share_vm: bool,
    /// CLONE_FS: the child shares the working directory, root and umask.
    pub share_fs: bool,
    /// CLONE_VFORK: the parent waits until the child execs or exits
    /// (`Process::vfork_pending`; the caller waits).
    pub vfork: bool,
    /// The child's stack pointer, when not the parent's (clone's stack).
    pub stack: Option<usize>,
    /// CLONE_SETTLS: the child's TLS base.
    pub tls: Option<usize>,
    /// The signal the parent gets when the child exits (0: none).
    pub exit_signal: u32,
    /// CLONE_PARENT_SETTID: where to store the child's TID in the parent.
    pub parent_settid: Option<usize>,
    /// CLONE_CHILD_SETTID: where to store it in the child.
    pub child_settid: Option<usize>,
    /// CLONE_CHILD_CLEARTID: cleared and futex-woken when the child exits.
    pub child_cleartid: Option<usize>,
}

impl Default for ForkOptions {
    fn default() -> Self {
        Self {
            share_vm: false,
            share_fs: false,
            vfork: false,
            stack: None,
            tls: None,
            exit_signal: super::signals::SIGCHLD as u32,
            parent_settid: None,
            child_settid: None,
            child_cleartid: None,
        }
    }
}

/// Fork current process
#[cfg(feature = "alloc")]
pub fn fork_process() -> Result<ProcessId, KernelError> {
    fork_process_with(&ForkOptions::default()).map(|(pid, _)| pid)
}

/// Make a new process from the calling thread, as fork or a non-thread
/// clone (`opts`). Returns the child's pid and the TID of its thread.
#[cfg(feature = "alloc")]
pub fn fork_process_with(opts: &ForkOptions) -> Result<(ProcessId, super::ThreadId), KernelError> {
    // Enforce process count limit (includes zombies awaiting reap).
    // This prevents unbounded process table growth during workloads
    // like BusyBox native compilation (213+ sequential fork+exec+wait).
    let current_count = table::PROCESS_TABLE.count();
    if current_count >= super::MAX_PROCESSES {
        println!(
            "[PROCESS] fork: process limit reached ({}/{})",
            current_count,
            super::MAX_PROCESSES
        );
        return Err(KernelError::ResourceExhausted {
            resource: "process table",
        });
    }

    let current_process =
        super::current_process().ok_or(KernelError::ProcessNotFound { pid: 0 })?;

    let current_thread = super::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // Create new process as copy of current
    // The child has its parent's credentials: a process that dropped to
    // an unprivileged uid used to fork children running as root (N-93).
    let new_process = ProcessBuilder::new(format!("{}-fork", current_process.name))
        .parent(current_process.pid)
        .priority(*current_process.priority.lock())
        .credentials(current_process.credentials())
        .build();

    let new_pid = new_process.pid;

    // Clone address space with COW (copy-on-write) optimization.
    // Pages are shared read-only between parent and child; physical copies
    // are deferred to the page fault handler when either process writes.
    {
        let current_space = current_process.memory_space.lock();
        let mut new_space = new_process.memory_space.lock();

        // Share the address space copy-on-write: clone_from maps the
        // parent's frames read-only + COW in both, with per-frame owner
        // counts (mm::frame_refs); the first write takes a private copy.
        // A CLONE_VM child maps the same frames writable instead.
        if opts.share_vm {
            new_space.share_from(&current_space)?;
        } else {
            new_space.clone_from(&current_space)?;
        }
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
        *new_process.exe_path.lock() = current_process.exe_path.lock().clone();
    }

    // Inherit container membership from parent so forked children
    // stay inside the same container namespace.
    {
        let cid = current_process
            .container_id
            .load(core::sync::atomic::Ordering::Acquire);
        new_process
            .container_id
            .store(cid, core::sync::atomic::Ordering::Release);
        if cid != 0 {
            // Register child in the container's PID namespace
            #[cfg(target_arch = "x86_64")]
            {
                let mut mgr = crate::virt::container::CONTAINER_MGR.lock();
                if let Some(ref mut mgr) = *mgr {
                    if let Some(container) = mgr.get_mut(cid) {
                        container.namespaces.pid.add_process(new_pid);
                    }
                }
            }
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

    // Signal dispositions and the blocked mask are inherited (POSIX fork);
    // pending signals are not.
    *new_process.signal_handlers.lock() = *current_process.signal_handlers.lock();
    *new_process.signal_action_extra.lock() = *current_process.signal_action_extra.lock();
    new_process.signal_mask.store(
        current_process
            .signal_mask
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );

    // Create thread in new process matching current thread
    let new_thread = {
        let ctx = current_thread.context.lock();
        let thread = ThreadBuilder::new(
            new_pid,
            current_thread.name.clone(),
            ctx.get_instruction_pointer(),
        )
        // The child's only thread is its first: its ID is the child's
        // (N-217).
        .tid(super::ThreadId(new_pid.0))
        // Scheduling parameters as Linux's sched_fork (N-221).
        .sched(current_thread.sched.lock().for_child())
        // A clone thread has no kernel-chosen stack (it runs on its own);
        // the child's main thread still needs a size for a later exec.
        .user_stack_size(
            current_thread
                .user_stack
                .size
                .max(super::creation::DEFAULT_USER_STACK_SIZE),
        )
        .kernel_stack_size(current_thread.kernel_stack.size)
        .priority(current_thread.priority)
        .cpu_affinity(current_thread.get_affinity())
        // Working directory and umask are copied, not reset to "/" and 022
        // (N-93); a copy, because the child's later chdir is its own --
        // unless CLONE_FS shares them.
        .fs(if opts.share_fs {
            super::thread::ThreadFs::clone_shared(&current_thread.fs)
        } else {
            super::thread::ThreadFs::clone_copy(&current_thread.fs)
        });
        let thread = match opts.child_cleartid {
            Some(ptr) => thread.clear_tid(ptr),
            None => thread,
        };
        let thread = match opts.tls {
            Some(tls) => thread.tls_base(tls),
            None => thread,
        }
        .build()?;

        // Copy thread context for child process.
        //
        // On x86_64, we capture the LIVE register state from the syscall
        // frame on the kernel stack (saved by syscall_entry assembly). This
        // gives the child the parent's actual CPU registers at the moment of
        // fork(), so the child resumes at the instruction after fork() with
        // RAX=0 (fork return value), not from main().
        //
        // On other architectures (or if no syscall frame is available), we
        // fall back to cloning the parent's ThreadContext from exec/load time.
        {
            let mut new_ctx = thread.context.lock();

            #[cfg(target_arch = "x86_64")]
            {
                use crate::arch::x86_64::syscall::get_syscall_frame;

                if let Some(frame) = get_syscall_frame() {
                    // Populate child context from live parent registers.
                    // Start with a clone for fields not in the frame (cr3, segments, etc.)
                    *new_ctx = (*ctx).clone();

                    // User RIP and RSP: where this syscall returns to.
                    new_ctx.set_instruction_pointer(frame.rip as usize);
                    new_ctx.set_stack_pointer(frame.rsp as usize);

                    // Return value: fork returns 0 in child
                    new_ctx.set_return_value(0);

                    // Copy all general-purpose registers from the live frame.
                    // The X86_64Context fields are accessed directly since we
                    // know the concrete type on x86_64.
                    new_ctx.rbx = frame.rbx;
                    new_ctx.rbp = frame.rbp;
                    new_ctx.r12 = frame.r12;
                    new_ctx.r13 = frame.r13;
                    new_ctx.r14 = frame.r14;
                    new_ctx.r15 = frame.r15;
                    new_ctx.rdi = frame.rdi;
                    new_ctx.rsi = frame.rsi;
                    new_ctx.rdx = frame.rdx;
                    new_ctx.r8 = frame.r8;
                    new_ctx.r9 = frame.r9;
                    new_ctx.r10 = frame.r10;

                    // R11 as the parent will see it (SYSCALL put RFLAGS there)
                    // and the RFLAGS the syscall returns with.
                    new_ctx.r11 = frame.r11;
                    new_ctx.rflags = frame.rflags;

                    // RCX holds user RIP (already set via set_instruction_pointer)
                    new_ctx.rcx = frame.rcx;
                } else {
                    // No syscall frame (called outside syscall context).
                    // Fall back to cloning parent's stored context.
                    *new_ctx = (*ctx).clone();
                    new_ctx.set_return_value(0);
                }
            }

            #[cfg(not(target_arch = "x86_64"))]
            {
                *new_ctx = (*ctx).clone();
                new_ctx.set_return_value(0);
            }

            // clone(2)'s stack and TLS for the child.
            if let Some(sp) = opts.stack {
                new_ctx.set_stack_pointer(sp);
            }
            if let Some(tls) = opts.tls {
                new_ctx.set_tls_base(tls as u64);
            }
        } // Drop lock here

        thread
    };

    let new_tid = new_thread.tid;
    new_process
        .exit_signal
        .store(opts.exit_signal, core::sync::atomic::Ordering::Release);
    new_process
        .vfork_pending
        .store(opts.vfork, core::sync::atomic::Ordering::Release);

    // The TID stores happen before the child can run. CLONE_CHILD_SETTID
    // writes the child's memory only: a copy-on-write page is copied first
    // (a CLONE_VM child shares the frame, so the parent sees it too, as on
    // Linux). They are ordinary user stores, so page protection applies,
    // and as on Linux (put_user, result unused) a store that cannot be made
    // is skipped rather than failing a clone whose child already exists.
    let tid_bytes = (new_tid.0 as u32).to_ne_bytes();
    if let Some(ptr) = opts.child_settid {
        let _ = new_process
            .memory_space
            .lock()
            .write_bytes_private(ptr as u64, &tid_bytes, false);
    }
    if let Some(ptr) = opts.parent_settid {
        let _ = current_process
            .memory_space
            .lock()
            .write_bytes_private(ptr as u64, &tid_bytes, false);
    }

    // The calling thread's blocked mask (per thread, N-109) and alternate
    // signal stack (N-222; a fork or vfork child keeps both).
    new_thread.sigmask.store(
        current_thread
            .sigmask
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );
    *new_thread.altstack.lock() = *current_thread.altstack.lock();
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
        // The child inherited the parent's shared-region mappings; tell
        // each region, so its mapping count includes the child. Only now,
        // once nothing before the table insert can fail: a registration
        // for a child that was then dropped kept the region busy forever
        // (review of the v0.26.0 stack, PR #15).
        register_inherited_regions(&process);
        process.set_state(ProcessState::Ready);

        if let Some(thread) = process.get_thread(new_tid) {
            create_scheduler_task(&process, &thread)?;
        }
    }

    // Return child PID to parent
    Ok((new_pid, new_tid))
}

/// Register each `MappingType::SharedRegion` mapping the child inherited
/// with its region (keyed by the region's physical base, its first frame).
///
/// `clone_from` copies such a mapping's page-table entries to the same
/// frames but nothing told the region, so the child was missing from its
/// mapping count and `unregister_region` could consider a region the child
/// still maps unused (review of the v0.26.0 stack, PR #13).
#[cfg(feature = "alloc")]
fn register_inherited_regions(child: &super::pcb::Process) {
    use crate::{
        ipc::shared_memory::{lookup_region, Permission},
        mm::{vas::MappingType, PageFlags},
    };

    // Collect first: SharedRegion::map takes the region lock before the
    // address-space lock, so never hold the latter while taking the former.
    let inherited: Vec<_> = {
        let space = child.memory_space.lock();
        let mappings = space.mappings_ref().lock();
        mappings
            .values()
            .filter(|m| m.mapping_type == MappingType::SharedRegion)
            .filter_map(|m| {
                let base = m.physical_frames.first()?.as_u64() * 4096;
                Some((base, m.start, m.flags))
            })
            .collect()
    };

    for (base, start, flags) in inherited {
        let Some(region) = lookup_region(base) else {
            continue;
        };
        let write = flags.contains(PageFlags::WRITABLE);
        let exec = !flags.contains(PageFlags::NO_EXECUTE);
        let permissions = match (write, exec) {
            (true, true) => Permission::ReadWriteExecute,
            (true, false) => Permission::Write,
            (false, true) => Permission::ReadExecute,
            (false, false) => Permission::Read,
        };
        // Err only if the child is already registered, which a fresh child
        // cannot be: logged, not dropped, if that invariant ever breaks.
        if let Err(e) = region.register_inherited(child.pid, start, permissions) {
            crate::println!(
                "[FORK] pid {}: registering an inherited region failed: {:?}",
                child.pid.0,
                e
            );
        }
    }
}

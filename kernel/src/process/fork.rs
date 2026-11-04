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
    pcb::{ProcessBuilder, ProcessState},
    table,
    thread::ThreadBuilder,
    ProcessId,
};
#[allow(unused_imports)]
use crate::{arch::context::ThreadContext, error::KernelError, println};

/// How a clone that makes a new process (not a thread) differs from a
/// plain fork (N-210). `Default` is fork.
#[derive(Debug, Clone)]
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
    /// CLONE_PIDFD: where to store a pidfd for the child in the parent
    /// (installed after the child's file table is copied, so the child
    /// does not have it).
    pub pidfd: Option<usize>,
    /// CLONE_PIDFD_AUTOKILL: that pidfd SIGKILLs the child when it is last
    /// closed.
    pub pidfd_autokill: bool,
    /// clone3's set_tid: the child's ID, claimed (`process::IdClaim`).
    pub pid: Option<ProcessId>,
    /// Its numbers in the PID namespaces below the initial one, reserved
    /// by the clone (with set_tid's choices); the fork reserves them
    /// itself when None.
    pub pid_numbers: Option<super::pidns::Pending>,
    /// CLONE_CLEAR_SIGHAND: handlers other than SIG_IGN start as SIG_DFL,
    /// and every action's flags and mask are cleared (Linux's
    /// flush_signal_handlers).
    pub clear_sighand: bool,
    /// CLONE_NNP: the child starts with no_new_privs set.
    pub no_new_privs: bool,
    /// CLONE_AUTOREAP: the child is reaped as it exits
    /// (`Process::autoreap`).
    pub autoreap: bool,
    /// The child's namespaces (`process::ns::for_clone`); the caller's
    /// when None.
    pub namespaces: Option<alloc::sync::Arc<super::ns::Namespaces>>,
    /// CLONE_EMPTY_MNTNS: the child's root and working directory are the
    /// root of its new, empty mount namespace.
    pub fs_at_root: bool,
    /// CLONE_NEWUSER: the child's new user namespace, in which it starts
    /// with every capability (Linux's set_cred_user_ns); the parent's
    /// when None.
    pub user_ns: Option<alloc::sync::Arc<super::userns::UserNs>>,
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
            pidfd: None,
            pidfd_autokill: false,
            pid: None,
            pid_numbers: None,
            clear_sighand: false,
            no_new_privs: false,
            autoreap: false,
            namespaces: None,
            fs_at_root: false,
            user_ns: None,
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

    // RLIMIT_NPROC: EAGAIN, as Linux.
    // Held until the child is in the process table.
    let _nproc =
        super::rlimit::check_nproc(&current_process).map_err(|_| KernelError::WouldBlock)?;

    let current_thread = super::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // The child goes in the PID namespace for its parent's children, with
    // a number at each level (alloc_pid): one whose init has exited takes
    // no more (ENOMEM).
    let pid_ns = opts.namespaces.as_ref().map_or_else(
        || current_thread.namespaces().pid_for_children.clone(),
        |n| n.pid_for_children.clone(),
    );
    let numbers = match opts.pid_numbers.as_ref().and_then(|p| p.take()) {
        Some(r) => r,
        None => super::pidns::reserve(&pid_ns, &[], |_| false).map_err(|e| match e {
            super::pidns::AllocError::Exhausted => KernelError::WouldBlock,
            _ => KernelError::OutOfMemory {
                requested: 0,
                available: 0,
            },
        })?,
    };

    // Create new process as copy of current
    // The child has its parent's credentials: a process that dropped to
    // an unprivileged uid used to fork children running as root (N-93). The
    // IDs and the calling thread's capability sets are read as one
    // snapshot, as a concurrent set*id call changes them together.
    let (parent_creds, parent_caps) = current_process.creds_and_caps(&current_thread);
    let new_process = ProcessBuilder::new(format!("{}-fork", current_process.name))
        .parent(current_process.pid)
        .priority(*current_process.priority.lock())
        .credentials(parent_creds)
        .pid(opts.pid)
        .build();
    new_process.set_user_ns(
        opts.user_ns
            .clone()
            .unwrap_or_else(|| current_process.user_ns()),
    );
    // The memory is the parent's copy, with its namespace (dup_mm).
    new_process.set_mm_user_ns(current_process.mm_user_ns());

    let new_pid = new_process.pid;
    new_process.set_pid_ns(pid_ns.clone());
    // Released again by `discard_unstarted` if the fork fails.
    numbers.commit(new_pid.0);

    // Everything that can fail once the child exists. On failure the child
    // is freed whole (N-46): its address space, frames, page tables, kernel
    // stack, capabilities and files used to leak, since dropping a Process
    // frees nothing.
    let populate = || -> Result<super::ThreadId, KernelError> {
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
            // The child runs the same program, so its writes stay denied
            // until both are gone (Linux's dup_mm keeps mm->exe_file).
            *new_process.exe_denial.lock() = current_process.exe_denial.lock().clone();
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
            // So are the resource limits (N-224).
            new_process.inherit_limits(current_process.limits());
            // So is the dumpable flag (the memory it protects is copied).
            new_process.dumpable.store(
                current_process
                    .dumpable
                    .load(core::sync::atomic::Ordering::Acquire),
                core::sync::atomic::Ordering::Release,
            );
        }

        // Signal dispositions and the blocked mask are inherited (POSIX fork);
        // pending signals are not.
        {
            // Both tables together, in their lock order (handler, then the
            // rest), so no action is copied half old and half new.
            let handlers = current_process.signal_handlers.lock();
            let extra = current_process.signal_action_extra.lock();
            let (mut child_handlers, mut child_extra) = (*handlers, *extra);
            if opts.clear_sighand {
                const SIG_IGN: u64 = 1;
                for handler in child_handlers.iter_mut().filter(|h| **h != SIG_IGN) {
                    *handler = 0;
                }
                child_extra = [(0, 0, 0); super::signals::NSIG + 1];
            }
            *new_process.signal_handlers.lock() = child_handlers;
            *new_process.signal_action_extra.lock() = child_extra;
        }
        new_process.signal_mask.store(
            current_process
                .signal_mask
                .load(core::sync::atomic::Ordering::Acquire),
            core::sync::atomic::Ordering::Release,
        );
        // Linux uname mode is inherited (ADR 0021).
        new_process
            .uname_linux
            .inherit(&current_process.uname_linux);

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
                super::thread::ThreadFs::clone_shared(&current_thread.fs())
            } else {
                let fs = super::thread::ThreadFs::clone_copy(&current_thread.fs());
                if opts.fs_at_root {
                    fs.move_to_root();
                }
                fs
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
                    // The live registers of the system call in progress:
                    // the context is the one saved when the program was
                    // loaded, so a child started from it ran the program
                    // again from its entry. (Gated as the function is
                    // defined: bare-metal AArch64 and RISC-V.)
                    #[cfg(all(
                        target_os = "none",
                        any(target_arch = "aarch64", target_arch = "riscv64")
                    ))]
                    super::load_live_user_registers(&mut new_ctx);
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
        new_process
            .autoreap
            .store(opts.autoreap, core::sync::atomic::Ordering::Release);
        if opts.share_vm {
            new_process.vm_id.store(
                current_process
                    .vm_id
                    .load(core::sync::atomic::Ordering::Acquire),
                core::sync::atomic::Ordering::Release,
            );
        }

        // The TID stores happen before the child can run. CLONE_CHILD_SETTID
        // writes the child's memory only: a copy-on-write page is copied first
        // (a CLONE_VM child shares the frame, so the parent sees it too, as on
        // Linux). They are ordinary user stores, so page protection applies,
        // and as on Linux (put_user, result unused) a store that cannot be made
        // is skipped rather than failing a clone whose child already exists.
        // Each stores the TID as its reader sees it: the child its number
        // in its own namespace (schedule_tail's task_pid_vnr), the parent
        // its number in the parent's (kernel_clone's pid_vnr).
        if let Some(ptr) = opts.child_settid {
            let nr = super::pidns::nr_in(new_tid.0, &pid_ns) as u32;
            let _ = new_process.memory_space.lock().write_bytes_private(
                ptr as u64,
                &nr.to_ne_bytes(),
                false,
            );
        }
        if let Some(ptr) = opts.parent_settid {
            let nr = super::pidns::nr_in(new_tid.0, &current_process.pid_ns()) as u32;
            let _ = current_process.memory_space.lock().write_bytes_private(
                ptr as u64,
                &nr.to_ne_bytes(),
                false,
            );
        }

        // The calling thread's blocked mask (per thread, N-109), alternate
        // signal stack (N-222), command name (N-227), no_new_privs and
        // capability sets (ADR 0020): a fork or vfork child keeps them all.
        new_thread.sigmask.store(
            current_thread
                .sigmask
                .load(core::sync::atomic::Ordering::Acquire),
            core::sync::atomic::Ordering::Release,
        );
        *new_thread.altstack.lock() = *current_thread.altstack.lock();
        *new_thread.comm.lock() = *current_thread.comm.lock();
        new_thread.no_new_privs.store(
            opts.no_new_privs
                || current_thread
                    .no_new_privs
                    .load(core::sync::atomic::Ordering::Acquire),
            core::sync::atomic::Ordering::Release,
        );
        new_thread.personality.store(
            current_thread
                .personality
                .load(core::sync::atomic::Ordering::Acquire),
            core::sync::atomic::Ordering::Release,
        );
        new_thread.set_namespaces(opts.namespaces.clone().unwrap_or_else(|| {
            // fork and vfork: Linux's copy_namespaces with no namespace
            // flag, so the child of an unshare(CLONE_NEWTIME) caller enters
            // the new time namespace (a vfork child, sharing memory, does
            // not). Without namespace flags it cannot fail.
            let current = current_thread.namespaces();
            let flags = if opts.share_vm { 0x100 } else { 0 };
            super::ns::for_clone(
                &current,
                flags,
                false,
                &current_process.user_ns(),
                &current_process.pid_ns(),
                0,
            )
            .unwrap_or(current)
        }));
        // The sets read with the credentials (above), not re-read here: a
        // set*id call on another thread in between would pair root IDs
        // with dropped sets, which exec turns back into full privilege.
        // In a new user namespace it holds every capability, which count
        // only there and below (set_cred_user_ns).
        *new_thread.caps.lock() = if opts.user_ns.is_some() {
            super::linux_caps::CapSets::ROOT
        } else {
            parent_caps
        };
        new_process.add_thread(new_thread)?;
        Ok(new_tid)
    };
    let new_tid = match populate() {
        Ok(tid) => tid,
        Err(e) => {
            #[cfg(feature = "alloc")]
            super::exit::discard_unstarted(&new_process);
            return Err(e);
        }
    };

    // Add to parent's children list
    #[cfg(feature = "alloc")]
    {
        current_process.children.lock().push(new_pid);
    }

    // Inherit container membership from parent so forked children
    // stay inside the same container namespace.
    let cid = current_process
        .container_id
        .load(core::sync::atomic::Ordering::Acquire);
    new_process
        .container_id
        .store(cid, core::sync::atomic::Ordering::Release);

    // Add process to table. A fresh pid cannot already be there.
    table::add_process(new_process)?;
    // Its namespace's init began exiting meanwhile: its kill of the
    // namespace may have missed this child, which goes again (ENOMEM), as
    // Linux's alloc_pid checks PIDNS_ADDING with the number's insert.
    if !pid_ns.adding() {
        if let Some(child) = table::get_process(new_pid) {
            super::exit::discard_unstarted(&child);
        }
        return Err(KernelError::OutOfMemory {
            requested: 0,
            available: 0,
        });
    }

    // Registered in its container's PID namespace only now, so a fork that
    // failed above leaves nothing there.
    {
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

    // Mark as ready. The caller starts its thread (sys_fork_with), and
    // discards it whole if that fails.
    if let Some(process) = table::get_process(new_pid) {
        // The child inherited the parent's shared-region mappings; tell
        // each region, so its mapping count includes the child. Only now,
        // once nothing before the table insert can fail: a registration
        // for a child that was then dropped kept the region busy forever
        // (review of the v0.26.0 stack, PR #15).
        register_inherited_regions(&process);
        process.set_state(ProcessState::Ready);
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

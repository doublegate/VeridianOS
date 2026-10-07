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

/// Fork current process
#[cfg(feature = "alloc")]
pub fn fork_process() -> Result<ProcessId, KernelError> {
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
        .uid(current_process.uid())
        .gid(current_process.gid())
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

    // The child keeps its parent's syscall ABI (SYS-INC-01).
    new_process.linux_abi.store(
        current_process
            .linux_abi
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );

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
        .user_stack_size(current_thread.user_stack.size)
        .kernel_stack_size(current_thread.kernel_stack.size)
        .priority(current_thread.priority)
        .cpu_affinity(current_thread.get_affinity())
        // Working directory and umask are copied, not reset to "/" and 022
        // (N-93); a copy, because the child's later chdir is its own.
        .fs(super::thread::ThreadFs::clone_copy(&current_thread.fs))
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
        } // Drop lock here

        thread
    };

    let new_tid = new_thread.tid;
    // The calling thread's blocked mask (per thread, N-109).
    new_thread.sigmask.store(
        current_thread
            .sigmask
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );
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
    Ok(new_pid)
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

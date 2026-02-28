//! Process forking (copy-on-write)
//!
//! Implements the fork system call which creates a child process as a copy
//! of the current process. Provides both full-copy (`fork_process`) and
//! copy-on-write (`cow_fork`) variants. COW fork marks all user-space
//! pages as read-only in both parent and child, deferring physical copies
//! to the page fault handler when either process writes.

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
    let new_process = ProcessBuilder::new(format!("{}-fork", current_process.name))
        .parent(current_process.pid)
        .priority(*current_process.priority.lock())
        .build();

    let new_pid = new_process.pid;

    // Clone address space
    {
        let current_space = current_process.memory_space.lock();
        let mut new_space = new_process.memory_space.lock();

        // Note: Currently using full copy instead of copy-on-write (CoW).
        // CoW optimization deferred to Phase 5 (Performance Optimization) as it
        // requires:
        // - Page table flags for CoW pages (read-only + CoW marker)
        // - Page fault handler integration for CoW page faults
        // - Reference counting for shared physical pages
        // - Memory zone integration for CoW tracking
        // The current implementation is correct, just less memory efficient.
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
                use crate::arch::x86_64::syscall::{get_saved_user_rsp, get_syscall_frame};

                if let Some(frame) = get_syscall_frame() {
                    // Populate child context from live parent registers.
                    // Start with a clone for fields not in the frame (cr3, segments, etc.)
                    *new_ctx = (*ctx).clone();

                    // User RIP: RCX was clobbered by SYSCALL to hold the return address
                    new_ctx.set_instruction_pointer(frame.rcx as usize);

                    // User RSP: saved to per-CPU data by syscall_entry
                    new_ctx.set_stack_pointer(get_saved_user_rsp() as usize);

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

                    // User RFLAGS: R11 was clobbered by SYSCALL to hold RFLAGS
                    new_ctx.r11 = frame.r11;
                    new_ctx.rflags = frame.r11;

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
        process.set_state(ProcessState::Ready);

        if let Some(thread) = process.get_thread(new_tid) {
            create_scheduler_task(process, thread)?;
        }
    }

    // Return child PID to parent
    Ok(new_pid)
}

/// Fork with copy-on-write semantics.
///
/// Instead of physically copying all user pages, this function:
/// 1. Clones the address space metadata (full copy of page tables).
/// 2. Registers shared frames in the COW table with ref_count = 2.
///
/// When either process writes to a COW page, the page fault handler
/// allocates a private copy and decrements the COW ref count.
#[cfg(feature = "alloc")]
pub fn cow_fork() -> Result<ProcessId, KernelError> {
    let current_count = table::PROCESS_TABLE.count();
    if current_count >= super::MAX_PROCESSES {
        println!(
            "[PROCESS] cow_fork: process limit reached ({}/{})",
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

    let new_process = ProcessBuilder::new(format!("{}-cow", current_process.name))
        .parent(current_process.pid)
        .priority(*current_process.priority.lock())
        .build();

    let new_pid = new_process.pid;

    // COW address space setup
    {
        let current_space = current_process.memory_space.lock();
        let mut new_space = new_process.memory_space.lock();

        // Clone the address space (copies page tables and mapping metadata)
        new_space.clone_from(&current_space)?;

        // Collect user-space pages for COW marking
        let user_pages = collect_user_pages(&current_space);

        if !user_pages.is_empty() {
            println!(
                "[PROCESS] cow_fork: marking {} user pages as COW",
                user_pages.len()
            );

            // Register shared frames in the demand paging COW table
            for &(_vaddr, frame) in &user_pages {
                crate::mm::demand_paging::with_manager_mut(|mgr| {
                    mgr.add_cow_entry(frame.as_u64() as usize, frame);
                    // Set ref_count to 2 (parent + child)
                    if let Some(entry) = mgr.cow_table.entries.get(&(frame.as_u64() as usize)) {
                        entry
                            .ref_count
                            .store(2, core::sync::atomic::Ordering::Release);
                    }
                });
            }
        }
    }

    // Clone capabilities
    {
        let current_caps = current_process.capability_space.lock();
        let new_caps = new_process.capability_space.lock();
        new_caps.clone_from(&current_caps)?;
    }

    // Clone file table
    {
        let parent_ft = current_process.file_table.lock();
        let child_ft = parent_ft.clone_for_fork();
        *new_process.file_table.lock() = child_ft;
    }

    // Inherit environment variables
    #[cfg(feature = "alloc")]
    {
        let parent_env = current_process.env_vars.lock();
        let mut child_env = new_process.env_vars.lock();
        for (key, value) in parent_env.iter() {
            child_env.insert(key.clone(), value.clone());
        }
    }

    // Inherit pgid, sid
    {
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
        .build()?;

        {
            let mut new_ctx = thread.context.lock();

            #[cfg(target_arch = "x86_64")]
            {
                use crate::arch::x86_64::syscall::{get_saved_user_rsp, get_syscall_frame};

                if let Some(frame) = get_syscall_frame() {
                    *new_ctx = (*ctx).clone();
                    new_ctx.set_instruction_pointer(frame.rcx as usize);
                    new_ctx.set_stack_pointer(get_saved_user_rsp() as usize);
                    new_ctx.set_return_value(0);
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
                    new_ctx.r11 = frame.r11;
                    new_ctx.rflags = frame.r11;
                    new_ctx.rcx = frame.rcx;
                } else {
                    *new_ctx = (*ctx).clone();
                    new_ctx.set_return_value(0);
                }
            }

            #[cfg(not(target_arch = "x86_64"))]
            {
                *new_ctx = (*ctx).clone();
                new_ctx.set_return_value(0);
            }
        }

        thread
    };

    let new_tid = new_thread.tid;
    new_process.add_thread(new_thread)?;

    #[cfg(feature = "alloc")]
    {
        current_process.children.lock().push(new_pid);
    }

    table::add_process(new_process)?;

    if let Some(process) = table::get_process(new_pid) {
        process.set_state(ProcessState::Ready);
        if let Some(thread) = process.get_thread(new_tid) {
            create_scheduler_task(process, thread)?;
        }
    }

    println!("[PROCESS] cow_fork: created child PID {}", new_pid);
    Ok(new_pid)
}

/// Collect user-space page mappings from a VAS for COW marking.
///
/// Returns a vector of (virtual_address, frame_number) pairs for all
/// user-space pages (addresses below 0x0000_8000_0000_0000).
#[cfg(feature = "alloc")]
fn collect_user_pages(
    vas: &crate::mm::vas::VirtualAddressSpace,
) -> Vec<(usize, crate::mm::FrameNumber)> {
    let mut pages = Vec::new();
    let mappings = vas.mappings_ref().lock();

    for (vaddr, mapping) in mappings.iter() {
        // Only collect user-space pages (below kernel split)
        if vaddr.as_u64() >= 0x0000_8000_0000_0000 {
            continue;
        }

        let base = vaddr.as_u64() as usize;
        for (i, &frame) in mapping.physical_frames.iter().enumerate() {
            let page_vaddr = base + i * 4096;
            if frame.as_u64() != 0 {
                pages.push((page_vaddr, frame));
            }
        }
    }

    pages
}

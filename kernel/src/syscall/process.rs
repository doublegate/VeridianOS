//! Process management system calls
//!
//! Implements system calls for process and thread management including
//! creation, termination, and state management.

use core::slice;

use super::{validate_user_buffer, validate_user_string_ptr, SyscallError, SyscallResult};
#[cfg(target_arch = "x86_64")]
use crate::arch::context::ThreadContext;
use crate::process::{
    create_thread, current_process, exec_process, exit::exit_process, exit_thread, fork_process,
    get_thread_tid, set_thread_affinity, ProcessId, ProcessPriority, ThreadId,
};

/// Fork the current process
///
/// Creates a new process that is a copy of the current process.
/// Returns the PID of the child in the parent, and 0 in the child.
pub fn sys_fork() -> SyscallResult {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[FORK] start\n");
    }

    // Get current process before forking
    let current = current_process().ok_or(SyscallError::InvalidState)?;

    match fork_process() {
        Ok(child_pid) => {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                crate::arch::x86_64::idt::raw_serial_str(b"[FORK] ok pid=");
                crate::arch::x86_64::idt::raw_serial_hex(child_pid.0 as u64);
                crate::arch::x86_64::idt::raw_serial_str(b"\n");
            }
            // In parent process, inherit capabilities to child
            if let Some(child_process) = crate::process::get_process(child_pid) {
                let parent_cap_space = current.capability_space.lock();
                let child_cap_space = child_process.capability_space.lock();

                // Inherit capabilities from parent to child
                if let Err(_e) = crate::cap::inheritance::fork_inherit_capabilities(
                    &parent_cap_space,
                    &child_cap_space,
                ) {
                    // Log error but don't fail the fork
                    println!("[WARN] Failed to inherit capabilities to child process");
                }
            }

            // In parent process, return child PID
            Ok(child_pid.0 as usize)
        }
        Err(_) => Err(SyscallError::OutOfMemory),
    }
}

/// Execute a new program
///
/// # Arguments
/// - path_ptr: Pointer to null-terminated path string
/// - argv_ptr: Pointer to argument array
/// - envp_ptr: Pointer to environment array
pub fn sys_exec(path_ptr: usize, argv_ptr: usize, envp_ptr: usize) -> SyscallResult {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[SYS_EXEC] path_ptr=0x");
        crate::arch::x86_64::idt::raw_serial_hex(path_ptr as u64);
        crate::arch::x86_64::idt::raw_serial_str(b"\n");
    }

    use crate::syscall::userspace::{copy_string_array_from_user, copy_string_from_user};

    // Validate path pointer is in user space
    validate_user_string_ptr(path_ptr)?;

    // Copy path from user space
    // SAFETY: path_ptr was validated as non-null and in user-space above.
    // copy_string_from_user reads a null-terminated string from the user-space
    // pointer with length bounds checking. The caller must provide valid mapped
    // user memory.
    let path = unsafe { copy_string_from_user(path_ptr)? };

    // Diagnostic: print the actual exec path string
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[SYS_EXEC] path=\"");
        for &b in path.as_bytes() {
            crate::arch::x86_64::idt::raw_serial_str(&[b]);
        }
        crate::arch::x86_64::idt::raw_serial_str(b"\"\n");
    }

    // Parse argv and envp arrays from user space
    // SAFETY: argv_ptr and envp_ptr are user-space pointers to null-terminated
    // arrays of string pointers. copy_string_array_from_user handles null
    // pointer checks internally and bounds-checks string lengths.
    let argv = unsafe { copy_string_array_from_user(argv_ptr)? };

    // Diagnostic: dump argc and full argv for cc1/as/ld
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[SYS_EXEC] argc=0x");
        crate::arch::x86_64::idt::raw_serial_hex(argv.len() as u64);
        crate::arch::x86_64::idt::raw_serial_str(b"\n");
        // Dump all argv entries for tool invocations
        if path.contains("cc1") || path.contains("/as") || path.contains("/ld") {
            for (i, arg) in argv.iter().enumerate() {
                crate::arch::x86_64::idt::raw_serial_str(b"  argv[");
                crate::arch::x86_64::idt::raw_serial_hex(i as u64);
                crate::arch::x86_64::idt::raw_serial_str(b"]=\"");
                for &b in arg.as_bytes() {
                    crate::arch::x86_64::idt::raw_serial_str(&[b]);
                }
                crate::arch::x86_64::idt::raw_serial_str(b"\"\n");
            }
        }
    }

    let envp = unsafe { copy_string_array_from_user(envp_ptr)? };

    // Get current process capability space before exec
    let current = current_process().ok_or(SyscallError::InvalidState)?;
    let old_cap_space = current.capability_space.lock();

    // Create new capability space for exec'd process
    let new_cap_space = crate::cap::CapabilitySpace::new();

    // Inherit only capabilities marked for exec preservation
    if let Err(_e) =
        crate::cap::inheritance::exec_inherit_capabilities(&old_cap_space, &new_cap_space)
    {
        println!("[WARN] Failed to inherit capabilities during exec");
    }

    // Convert to slices for exec_process
    #[cfg(feature = "alloc")]
    extern crate alloc;
    #[cfg(feature = "alloc")]
    use alloc::vec::Vec;
    let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let envp_refs: Vec<&str> = envp.iter().map(|s| s.as_str()).collect();

    // Drop the capability space lock before exec_process, which diverges
    // via enter_usermode (-> !) on success, leaking any held lock guards.
    drop(old_cap_space);

    match exec_process(&path, &argv_refs, &envp_refs) {
        Ok(_) => {
            // exec succeeded. The current process's address space has been
            // replaced with the new program image. We cannot return via
            // SYSRET because the kernel stack holds pre-exec register values
            // (old RIP, RSP). Enter user mode directly via iretq.
            #[cfg(target_arch = "x86_64")]
            // SAFETY: current_thread() returns the thread that called exec.
            // Its context was updated by exec_process with the new entry
            // point and stack pointer. swapgs undoes the swapgs from
            // syscall_entry so GS_BASE/KERNEL_GS_BASE are correct for
            // user mode. enter_usermode builds an iretq frame and
            // transitions to Ring 3.
            unsafe {
                let current_thread =
                    crate::process::current_thread().expect("no current thread after exec");
                let ctx = current_thread.context.lock();
                let entry = ctx.get_instruction_pointer() as u64;
                let stack = ctx.get_stack_pointer() as u64;
                drop(ctx);

                // Switch CR3 to the new page tables created by exec_process.
                // exec_process calls memory_space.init() which creates a new
                // L4 page table with kernel mappings, then maps ELF segments
                // and user stack into it. But it does NOT switch CR3 — the
                // CPU still uses the old (cleared) page tables. We must load
                // the new CR3 before iretq, otherwise the user entry point
                // is unmapped and causes a page fault.
                if let Some(proc) = crate::process::current_process() {
                    let memory_space = proc.memory_space.lock();
                    let new_cr3 = memory_space.get_page_table();
                    drop(memory_space);
                    if new_cr3 != 0 {
                        core::arch::asm!("mov cr3, {}", in(reg) new_cr3);
                    }
                }

                // Diagnostic: print entry/stack/CR3 before usermode transition
                // so multi-LOAD ELF GP faults can be correlated.
                crate::arch::x86_64::idt::raw_serial_str(b"[EXEC] entry=0x");
                crate::arch::x86_64::idt::raw_serial_hex(entry);
                crate::arch::x86_64::idt::raw_serial_str(b" stack=0x");
                crate::arch::x86_64::idt::raw_serial_hex(stack);
                let diag_cr3: u64;
                core::arch::asm!("mov {}, cr3", out(reg) diag_cr3);
                crate::arch::x86_64::idt::raw_serial_str(b" cr3=0x");
                crate::arch::x86_64::idt::raw_serial_hex(diag_cr3);
                crate::arch::x86_64::idt::raw_serial_str(b"\n");

                // Set FS_BASE (MSR 0xC0000100) for TLS if the process has one.
                // Must be done BEFORE enter_usermode since iretq doesn't
                // restore FS_BASE. wrmsr uses ECX=MSR, EDX:EAX=value.
                if let Some(proc) = crate::process::current_process() {
                    let fs_base = proc.tls_fs_base.load(core::sync::atomic::Ordering::Acquire);
                    if fs_base != 0 {
                        crate::arch::x86_64::idt::raw_serial_str(b"[EXEC] FS_BASE=0x");
                        crate::arch::x86_64::idt::raw_serial_hex(fs_base);
                        crate::arch::x86_64::idt::raw_serial_str(b"\n");
                        let lo = fs_base as u32;
                        let hi = (fs_base >> 32) as u32;
                        core::arch::asm!(
                            "wrmsr",
                            in("ecx") 0xC000_0100u32, // IA32_FS_BASE
                            in("eax") lo,
                            in("edx") hi,
                        );
                    }
                }

                // Undo the swapgs from syscall_entry so GS_BASE and
                // KERNEL_GS_BASE are correct for user mode.
                core::arch::asm!("swapgs");

                crate::arch::x86_64::usermode::enter_usermode(
                    entry, stack, 0x33, // User CS (Ring 3)
                    0x2B, // User SS (Ring 3)
                );
            }

            // Non-x86_64: exec not yet supported for user-mode entry
            #[cfg(not(target_arch = "x86_64"))]
            Err(SyscallError::InvalidState)
        }
        Err(_e) => {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                crate::arch::x86_64::idt::raw_serial_str(b"[SYS_EXEC] FAIL: ");
                for &b in path.as_bytes() {
                    crate::arch::x86_64::idt::raw_serial_str(&[b]);
                }
                crate::arch::x86_64::idt::raw_serial_str(b"\n");
            }
            Err(SyscallError::ResourceNotFound)
        }
    }
}

/// Exit the current process
///
/// # Arguments
/// - exit_code: Process exit code
///
/// Marks the current thread as exited and removes it from the scheduler.
/// If no current thread is found (e.g., called from a context without a
/// proper task), halts the CPU as a last resort.
pub fn sys_exit(exit_code: usize) -> SyscallResult {
    // Check for boot return context FIRST, before exit_thread/exit_process.
    // Both of those call sched::exit_task() which does a context switch and
    // never returns, preventing boot_return_to_kernel from being reached.
    #[cfg(target_arch = "x86_64")]
    {
        let has_ctx = crate::arch::x86_64::usermode::has_boot_return_context();
        crate::println!("[SYS_EXIT] code={}, boot_ctx={}", exit_code, has_ctx);
        if has_ctx {
            // Mark the process as Zombie and notify parent before returning
            // to the boot context. This is needed for nested child execution:
            // when a forked child exits, the parent's wait loop must find it
            // as a Zombie to reap it.
            if let Some(process) = current_process() {
                process.set_exit_code(exit_code as i32);

                #[cfg(feature = "alloc")]
                {
                    let threads = process.threads.lock();
                    for (_, thread) in threads.iter() {
                        thread.set_state(crate::process::thread::ThreadState::Zombie);
                    }
                }

                process.set_state(crate::process::pcb::ProcessState::Zombie);

                // Wake parent if blocked in waitpid
                if let Some(parent_pid) = process.parent {
                    if let Some(parent) = crate::process::table::get_process(parent_pid) {
                        if parent.get_state() == crate::process::pcb::ProcessState::Blocked {
                            parent.set_state(crate::process::pcb::ProcessState::Ready);
                            crate::sched::wake_up_process(parent_pid);
                        }
                    }
                }
            }

            // SAFETY: The boot return context was saved by
            // enter_usermode_returnable (or enter_forked_child_returnable)
            // and is still valid on the kernel stack.
            unsafe {
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
    }

    // Normal path: exit_thread calls sched::exit_task (never returns).
    exit_thread(exit_code as i32);

    // exit_thread returned, meaning current_thread() was None.
    // Fall back to exit_process which calls sched::exit_task too.
    exit_process(exit_code as i32);

    // No boot context, no scheduler — halt as a last resort.
    loop {
        core::hint::spin_loop();
    }
}

/// Wait for a child process to terminate
///
/// # Arguments
/// - pid: PID of child to wait for (-1 for any child)
/// - status_ptr: Pointer to store exit status
/// - options: Wait options bitmask (WNOHANG=1, WUNTRACED=2, WCONTINUED=8)
pub fn sys_wait(pid: isize, status_ptr: usize, options: usize) -> SyscallResult {
    use crate::{process::exit::WaitOptions, syscall::userspace::copy_to_user};

    let wait_pid = if pid == -1 {
        None
    } else if pid > 0 {
        Some(ProcessId(pid as u64))
    } else {
        return Err(SyscallError::InvalidArgument);
    };

    // Parse options bitmask into WaitOptions struct
    const WNOHANG: usize = 1;
    const WUNTRACED: usize = 2;
    const WCONTINUED: usize = 8;

    let wait_options = WaitOptions {
        no_hang: (options & WNOHANG) != 0,
        untraced: (options & WUNTRACED) != 0,
        continued: (options & WCONTINUED) != 0,
    };

    let wait_result = crate::process::exit::wait_process_with_options(wait_pid, wait_options);

    match wait_result {
        Ok((child_pid, exit_status)) => {
            // Write exit status to user space if pointer provided
            if status_ptr != 0 {
                unsafe {
                    copy_to_user(status_ptr, &exit_status)?;
                }
            }
            Ok(child_pid.0 as usize)
        }
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// Get the current process ID
pub fn sys_getpid() -> SyscallResult {
    if let Some(process) = current_process() {
        Ok(process.pid.0 as usize)
    } else {
        Err(SyscallError::ResourceNotFound)
    }
}

/// Get the parent process ID
pub fn sys_getppid() -> SyscallResult {
    if let Some(process) = current_process() {
        if let Some(parent_pid) = process.parent {
            Ok(parent_pid.0 as usize)
        } else {
            Ok(0) // Init process has no parent
        }
    } else {
        Err(SyscallError::ResourceNotFound)
    }
}

/// Create a new thread
///
/// # Arguments
/// - entry_point: Thread entry point function
/// - stack_ptr: Stack pointer for new thread
/// - arg: Argument to pass to thread
/// - tls_ptr: Thread-local storage pointer
pub fn sys_thread_create(
    entry_point: usize,
    stack_ptr: usize,
    arg: usize,
    tls_ptr: usize,
) -> SyscallResult {
    // Validate entry point and stack pointer are in user space.
    // Entry point needs at least 1 byte (code); stack needs at least
    // pointer-sized space to hold a return address.
    validate_user_buffer(entry_point, 1)?;
    validate_user_buffer(stack_ptr, core::mem::size_of::<usize>())?;

    // TLS pointer is optional (0 means none)
    if tls_ptr != 0 {
        validate_user_buffer(tls_ptr, 1)?;
    }

    match create_thread(entry_point, stack_ptr, arg, tls_ptr) {
        Ok(tid) => Ok(tid.0 as usize),
        Err(_) => Err(SyscallError::OutOfMemory),
    }
}

/// Exit the current thread
///
/// # Arguments
/// - exit_code: Thread exit code
pub fn sys_thread_exit(exit_code: usize) -> SyscallResult {
    exit_thread(exit_code as i32);
    // Should never reach here
    unreachable!("exit_thread returned");
}

/// Get the current thread ID
pub fn sys_gettid() -> SyscallResult {
    Ok(get_thread_tid().0 as usize)
}

/// Join with a thread, waiting for its termination
///
/// # Arguments
/// - tid: Thread ID to join
/// - retval_ptr: Pointer to store thread return value
pub fn sys_thread_join(tid: usize, retval_ptr: usize) -> SyscallResult {
    use crate::syscall::userspace::copy_to_user;

    let target_tid = ThreadId(tid as u64);

    // Get current process
    let current = current_process().ok_or(SyscallError::InvalidState)?;

    // Find target thread in current process
    loop {
        // Check if thread exists and get its state
        let thread_state = {
            let threads = current.threads.lock();
            threads.get(&target_tid).map(|thread| {
                (
                    thread.get_state(),
                    thread.exit_code.load(core::sync::atomic::Ordering::Acquire),
                    thread.detached.load(core::sync::atomic::Ordering::Acquire),
                )
            })
        };

        match thread_state {
            Some((crate::process::thread::ThreadState::Zombie, exit_code, detached)) => {
                if detached {
                    return Err(SyscallError::InvalidState); // cannot join
                                                            // detached thread
                }
                // Thread has exited, clean it up
                if crate::process::lifecycle::cleanup_thread(current, target_tid).is_err() {
                    return Err(SyscallError::InvalidState);
                }

                // Return exit code to user
                if retval_ptr != 0 {
                    let exit_value = exit_code as usize;
                    // SAFETY: retval_ptr is a non-zero user-space pointer.
                    // copy_to_user validates the pointer and copies the
                    // usize exit value to the user buffer.
                    unsafe {
                        copy_to_user(retval_ptr, &exit_value)?;
                    }
                }

                return Ok(0);
            }
            Some(_) => {
                // Thread still running, yield and try again
                crate::sched::yield_cpu();
            }
            None => {
                // Thread doesn't exist
                return Err(SyscallError::ResourceNotFound);
            }
        }
    }
}

/// Set thread CPU affinity
///
/// # Arguments
/// - tid: Thread ID (0 for current thread)
/// - cpuset_ptr: Pointer to CPU set
/// - cpuset_size: Size of CPU set
pub fn sys_thread_setaffinity(tid: usize, cpuset_ptr: usize, cpuset_size: usize) -> SyscallResult {
    if cpuset_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(cpuset_ptr, cpuset_size)?;

    let target_tid = if tid == 0 {
        get_thread_tid()
    } else {
        ThreadId(tid as u64)
    };

    // Read CPU set from user space
    // SAFETY: cpuset_ptr was validated as non-zero and cpuset_size > 0
    // above. The caller must provide a valid, readable user-space buffer
    // of at least cpuset_size bytes containing the CPU affinity mask.
    let cpuset = unsafe { slice::from_raw_parts(cpuset_ptr as *const u8, cpuset_size) };

    // Extract CPU mask from cpuset (simplified)
    let cpu_mask = if cpuset_size >= 8 {
        // Slice is exactly 8 bytes (cpuset_size >= 8 checked above)
        let bytes: [u8; 8] = match cpuset[0..8].try_into() {
            Ok(b) => b,
            Err(_) => return Err(SyscallError::InvalidArgument),
        };
        u64::from_le_bytes(bytes)
    } else {
        return Err(SyscallError::InvalidArgument);
    };

    match set_thread_affinity(target_tid, cpu_mask) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// Get thread CPU affinity
///
/// # Arguments
/// - tid: Thread ID (0 for current thread)
/// - cpuset_ptr: Pointer to store CPU set
/// - cpuset_size: Size of CPU set buffer
pub fn sys_thread_getaffinity(tid: usize, cpuset_ptr: usize, cpuset_size: usize) -> SyscallResult {
    use crate::syscall::userspace::{copy_slice_to_user, validate_user_ptr};

    if cpuset_ptr == 0 || cpuset_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Validate user pointer
    validate_user_ptr(cpuset_ptr as *const u8, cpuset_size)?;

    let target_tid = if tid == 0 {
        get_thread_tid()
    } else {
        ThreadId(tid as u64)
    };

    // Get actual CPU affinity from thread
    let cpu_mask = if let Some(process) = current_process() {
        if let Some(thread) = process.get_thread(target_tid) {
            thread
                .cpu_affinity
                .load(core::sync::atomic::Ordering::Acquire) as u64
        } else {
            return Err(SyscallError::ResourceNotFound);
        }
    } else {
        return Err(SyscallError::InvalidState);
    };

    // Write CPU set to user space
    let mask_bytes = cpu_mask.to_le_bytes();
    let bytes_to_copy = cpuset_size.min(8);

    // SAFETY: cpuset_ptr was validated as non-zero and checked via
    // validate_user_ptr above. copy_slice_to_user copies the CPU mask
    // bytes to the user-space buffer.
    unsafe {
        copy_slice_to_user(cpuset_ptr, &mask_bytes[..bytes_to_copy])?;
    }

    Ok(0)
}

/// Change process priority
///
/// # Arguments
/// - which: Target type (PRIO_PROCESS, PRIO_PGRP, PRIO_USER)
/// - who: Target ID
/// - priority: New priority value
pub fn sys_setpriority(which: usize, who: usize, priority: usize) -> SyscallResult {
    // For now, only support PRIO_PROCESS (which == 0)
    if which != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let current = current_process().ok_or(SyscallError::InvalidState)?;

    let pid = if who == 0 {
        current.pid
    } else {
        let target_pid = ProcessId(who as u64);

        // Modifying another process's priority requires MODIFY right
        // on a Process capability for that process
        if target_pid != current.pid {
            let cap_space = current.capability_space.lock();
            let has_permission = {
                let mut found = false;
                #[cfg(feature = "alloc")]
                {
                    let _ = cap_space.iter_capabilities(|entry| {
                        if let crate::cap::ObjectRef::Process { pid: cap_pid } = &entry.object {
                            if *cap_pid == target_pid
                                && entry.rights.contains(crate::cap::Rights::MODIFY)
                            {
                                found = true;
                                return false; // stop iteration
                            }
                        }
                        true // continue
                    });
                }
                found
            };
            if !has_permission {
                return Err(SyscallError::PermissionDenied);
            }
        }

        target_pid
    };

    // Convert priority to our internal representation
    let new_priority = match priority {
        0..=39 => ProcessPriority::RealTime,
        40..=79 => ProcessPriority::System,
        80..=119 => ProcessPriority::Normal,
        120..=139 => ProcessPriority::Low,
        _ => ProcessPriority::Idle,
    };

    // Actually set the priority
    if let Some(process) = crate::process::table::get_process(pid) {
        process.set_priority(new_priority);

        // Update scheduler tasks for all threads
        #[cfg(feature = "alloc")]
        {
            let threads = process.threads.lock();
            for (_, thread) in threads.iter() {
                if let Some(task_ptr) = thread.get_task_ptr() {
                    // SAFETY: task_ptr is a valid NonNull<Task> obtained from
                    // the thread's stored task pointer. We modify the priority
                    // field while holding the threads lock, preventing
                    // concurrent modification.
                    unsafe {
                        let task = task_ptr.as_ptr();
                        (*task).priority = match new_priority {
                            ProcessPriority::RealTime => crate::sched::task::Priority::RealTimeHigh,
                            ProcessPriority::System => crate::sched::task::Priority::SystemHigh,
                            ProcessPriority::Normal => crate::sched::task::Priority::UserNormal,
                            ProcessPriority::Low => crate::sched::task::Priority::UserLow,
                            ProcessPriority::Idle => crate::sched::task::Priority::Idle,
                        };
                    }
                }
            }
        }

        Ok(0)
    } else {
        Err(SyscallError::ResourceNotFound)
    }
}

// ============================================================================
// Identity syscalls (170-175)
// ============================================================================

/// Get real user ID (SYS_GETUID = 170)
pub fn sys_getuid() -> SyscallResult {
    let proc = current_process().ok_or(SyscallError::InvalidState)?;
    Ok(proc.uid as usize)
}

/// Get effective user ID (SYS_GETEUID = 171)
///
/// VeridianOS does not yet distinguish real/effective UIDs, so this returns
/// the same value as getuid.
pub fn sys_geteuid() -> SyscallResult {
    sys_getuid()
}

/// Get real group ID (SYS_GETGID = 172)
pub fn sys_getgid() -> SyscallResult {
    let proc = current_process().ok_or(SyscallError::InvalidState)?;
    Ok(proc.gid as usize)
}

/// Get effective group ID (SYS_GETEGID = 173)
pub fn sys_getegid() -> SyscallResult {
    sys_getgid()
}

/// Set user ID (SYS_SETUID = 174)
///
/// Only uid 0 (root) can change to a different UID. Non-root processes
/// may only "set" their uid to the current value (a no-op).
pub fn sys_setuid(uid: usize) -> SyscallResult {
    let proc = current_process().ok_or(SyscallError::InvalidState)?;
    let current_uid = proc.uid;
    let new_uid = uid as u32;

    // Non-root can only set uid to current value (no-op)
    if current_uid != 0 && new_uid != current_uid {
        return Err(SyscallError::PermissionDenied);
    }

    // If already the requested uid, nothing to do
    if new_uid == current_uid {
        return Ok(0);
    }

    // Root changing uid: get mutable reference via process table
    let pid = proc.pid;
    if let Some(proc_mut) = crate::process::table::get_process_mut(pid) {
        proc_mut.uid = new_uid;
        Ok(0)
    } else {
        Err(SyscallError::InvalidState)
    }
}

/// Set group ID (SYS_SETGID = 175)
///
/// Only uid 0 (root) can change to a different GID. Non-root processes
/// may only "set" their gid to the current value (a no-op).
pub fn sys_setgid(gid: usize) -> SyscallResult {
    let proc = current_process().ok_or(SyscallError::InvalidState)?;
    let current_uid = proc.uid;
    let current_gid = proc.gid;
    let new_gid = gid as u32;

    // Non-root can only set gid to current value (no-op)
    if current_uid != 0 && new_gid != current_gid {
        return Err(SyscallError::PermissionDenied);
    }

    // If already the requested gid, nothing to do
    if new_gid == current_gid {
        return Ok(0);
    }

    // Root changing gid: get mutable reference via process table
    let pid = proc.pid;
    if let Some(proc_mut) = crate::process::table::get_process_mut(pid) {
        proc_mut.gid = new_gid;
        Ok(0)
    } else {
        Err(SyscallError::InvalidState)
    }
}

// ============================================================================
// Process group / session syscalls (176-180)
// ============================================================================

/// Set process group ID (SYS_SETPGID = 176)
///
/// # Arguments
/// - `pid`: Target process (0 = calling process)
/// - `pgid`: New process group (0 = use pid as pgid)
pub fn sys_setpgid(pid: usize, pgid: usize) -> SyscallResult {
    let proc = current_process().ok_or(SyscallError::InvalidState)?;

    let target_pid = if pid == 0 {
        proc.pid
    } else {
        ProcessId(pid as u64)
    };
    let new_pgid = if pgid == 0 { target_pid.0 } else { pgid as u64 };

    // Can only set pgid for self or children
    if target_pid != proc.pid {
        // Check if target is a child
        #[cfg(feature = "alloc")]
        {
            let children = proc.children.lock();
            if !children.contains(&target_pid) {
                return Err(SyscallError::PermissionDenied);
            }
        }
    }

    // Apply to the target process
    if let Some(target) = crate::process::table::get_process(target_pid) {
        target
            .pgid
            .store(new_pgid, core::sync::atomic::Ordering::Release);
        Ok(0)
    } else {
        Err(SyscallError::ProcessNotFound)
    }
}

/// Get process group ID (SYS_GETPGID = 177)
///
/// # Arguments
/// - `pid`: Target process (0 = calling process)
pub fn sys_getpgid(pid: usize) -> SyscallResult {
    let target_pid = if pid == 0 {
        let proc = current_process().ok_or(SyscallError::InvalidState)?;
        proc.pid
    } else {
        ProcessId(pid as u64)
    };

    if let Some(target) = crate::process::table::get_process(target_pid) {
        Ok(target.pgid.load(core::sync::atomic::Ordering::Acquire) as usize)
    } else {
        Err(SyscallError::ProcessNotFound)
    }
}

/// Get process group ID of calling process (SYS_GETPGRP = 178)
pub fn sys_getpgrp() -> SyscallResult {
    sys_getpgid(0)
}

/// Create a new session (SYS_SETSID = 179)
///
/// Makes the calling process the session leader and process group leader
/// of a new session. The process must not already be a process group leader.
pub fn sys_setsid() -> SyscallResult {
    let proc = current_process().ok_or(SyscallError::InvalidState)?;

    // Process must not already be a process group leader
    let current_pgid = proc.pgid.load(core::sync::atomic::Ordering::Acquire);
    if current_pgid == proc.pid.0 {
        return Err(SyscallError::PermissionDenied);
    }

    // Set both pgid and sid to our pid (new session + group leader)
    proc.pgid
        .store(proc.pid.0, core::sync::atomic::Ordering::Release);
    proc.sid
        .store(proc.pid.0, core::sync::atomic::Ordering::Release);

    Ok(proc.pid.0 as usize)
}

/// Get session ID (SYS_GETSID = 180)
///
/// # Arguments
/// - `pid`: Target process (0 = calling process)
pub fn sys_getsid(pid: usize) -> SyscallResult {
    let target_pid = if pid == 0 {
        let proc = current_process().ok_or(SyscallError::InvalidState)?;
        proc.pid
    } else {
        ProcessId(pid as u64)
    };

    if let Some(target) = crate::process::table::get_process(target_pid) {
        Ok(target.sid.load(core::sync::atomic::Ordering::Acquire) as usize)
    } else {
        Err(SyscallError::ProcessNotFound)
    }
}

/// Get process priority
///
/// # Arguments
/// - which: Target type (PRIO_PROCESS, PRIO_PGRP, PRIO_USER)
/// - who: Target ID
pub fn sys_getpriority(which: usize, who: usize) -> SyscallResult {
    // For now, only support PRIO_PROCESS (which == 0)
    if which != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let pid = if who == 0 {
        // Current process
        if let Some(process) = current_process() {
            process.pid
        } else {
            return Err(SyscallError::ResourceNotFound);
        }
    } else {
        ProcessId(who as u64)
    };

    // Get actual priority from process
    if let Some(process) = crate::process::table::get_process(pid) {
        let priority_value = match *process.priority.lock() {
            ProcessPriority::RealTime => 20, // Highest priority
            ProcessPriority::System => 60,   // High priority
            ProcessPriority::Normal => 100,  // Normal priority
            ProcessPriority::Low => 130,     // Low priority
            ProcessPriority::Idle => 140,    // Lowest priority
        };
        Ok(priority_value)
    } else {
        Err(SyscallError::ResourceNotFound)
    }
}

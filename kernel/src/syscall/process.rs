//! Process management system calls
//!
//! Implements system calls for process and thread management including
//! creation, termination, and state management.

use core::slice;

use super::{SyscallError, SyscallResult};
use crate::process::{
    create_thread, current_process, exec_process, exit_thread, fork_process, get_thread_tid,
    set_thread_affinity, wait_for_child, ProcessId, ProcessPriority, ThreadId,
};

/// Fork the current process
///
/// Creates a new process that is a copy of the current process.
/// Returns the PID of the child in the parent, and 0 in the child.
pub fn sys_fork() -> SyscallResult {
    // Get current process before forking
    let current = current_process().ok_or(SyscallError::InvalidState)?;

    match fork_process() {
        Ok(child_pid) => {
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
    // Validate pointers
    if path_ptr == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // TODO: Validate and copy path from user space
    // For now, we'll use a placeholder
    let path = "placeholder";

    // Parse argv and envp arrays (simplified for now)
    let argv: &[&str] = if argv_ptr != 0 {
        // TODO: Copy from user space
        &[]
    } else {
        &[]
    };

    let envp: &[&str] = if envp_ptr != 0 {
        // TODO: Copy from user space
        &[]
    } else {
        &[]
    };

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

    match exec_process(path, argv, envp) {
        Ok(_) => {
            // exec should not return on success
            unreachable!("exec_process returned on success");
        }
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// Exit the current process
///
/// # Arguments
/// - exit_code: Process exit code
pub fn sys_exit(exit_code: usize) -> SyscallResult {
    exit_thread(exit_code as i32);
    // Should never reach here
    unreachable!("exit_thread returned");
}

/// Wait for a child process to terminate
///
/// # Arguments
/// - pid: PID of child to wait for (-1 for any child)
/// - status_ptr: Pointer to store exit status
/// - options: Wait options (WNOHANG, etc.)
pub fn sys_wait(pid: isize, status_ptr: usize, _options: usize) -> SyscallResult {
    let wait_pid = if pid == -1 {
        None
    } else if pid > 0 {
        Some(ProcessId(pid as u64))
    } else {
        return Err(SyscallError::InvalidArgument);
    };

    match wait_for_child(wait_pid) {
        Ok((child_pid, exit_status)) => {
            // Write exit status to user space if pointer provided
            if status_ptr != 0 {
                unsafe {
                    *(status_ptr as *mut i32) = exit_status;
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
    if entry_point == 0 || stack_ptr == 0 {
        return Err(SyscallError::InvalidArgument);
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
    let _target_tid = ThreadId(tid as u64);

    // TODO: Implement thread join
    // This would involve:
    // 1. Find target thread
    // 2. Check if it's joinable
    // 3. Wait for it to terminate
    // 4. Get its exit value

    if retval_ptr != 0 {
        unsafe {
            *(retval_ptr as *mut usize) = 0; // Placeholder
        }
    }

    Ok(0)
}

/// Set thread CPU affinity
///
/// # Arguments
/// - tid: Thread ID (0 for current thread)
/// - cpuset_ptr: Pointer to CPU set
/// - cpuset_size: Size of CPU set
pub fn sys_thread_setaffinity(tid: usize, cpuset_ptr: usize, cpuset_size: usize) -> SyscallResult {
    if cpuset_ptr == 0 || cpuset_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let target_tid = if tid == 0 {
        get_thread_tid()
    } else {
        ThreadId(tid as u64)
    };

    // Read CPU set from user space
    let cpuset = unsafe { slice::from_raw_parts(cpuset_ptr as *const u8, cpuset_size) };

    // Extract CPU mask from cpuset (simplified)
    let cpu_mask = if cpuset_size >= 8 {
        u64::from_le_bytes(cpuset[0..8].try_into().unwrap())
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
    if cpuset_ptr == 0 || cpuset_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let _target_tid = if tid == 0 {
        get_thread_tid()
    } else {
        ThreadId(tid as u64)
    };

    // TODO: Get actual CPU affinity from thread
    let cpu_mask: u64 = 0xFFFFFFFFFFFFFFFF; // All CPUs for now

    // Write CPU set to user space
    let cpuset = unsafe { slice::from_raw_parts_mut(cpuset_ptr as *mut u8, cpuset_size) };

    if cpuset_size >= 8 {
        cpuset[0..8].copy_from_slice(&cpu_mask.to_le_bytes());
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

    // Convert priority to our internal representation
    let new_priority = match priority {
        0..=39 => ProcessPriority::RealTime,
        40..=79 => ProcessPriority::System,
        80..=119 => ProcessPriority::Normal,
        120..=139 => ProcessPriority::Low,
        _ => ProcessPriority::Idle,
    };

    // TODO: Actually set the priority
    // This would involve finding the process and updating its priority
    let _ = (pid, new_priority);

    Ok(0)
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

    let _pid = if who == 0 {
        // Current process
        if let Some(process) = current_process() {
            process.pid
        } else {
            return Err(SyscallError::ResourceNotFound);
        }
    } else {
        ProcessId(who as u64)
    };

    // TODO: Get actual priority from process
    // For now, return normal priority
    Ok(100) // Normal priority
}

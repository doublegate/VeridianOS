//! Process operations for VeridianOS.
//!
//! Maps Rust process operations to VeridianOS syscalls:
//! - `exit` -> SYS_PROCESS_EXIT (11)
//! - `fork` -> SYS_PROCESS_FORK (12)
//! - `waitpid` -> SYS_PROCESS_WAIT (14)
//! - `getpid` -> SYS_PROCESS_GETPID (15)
//! - `getppid` -> SYS_PROCESS_GETPPID (16)
//! - `getcwd` -> SYS_PROCESS_GETCWD (110)
//! - `chdir` -> SYS_PROCESS_CHDIR (111)
//! - `kill` -> SYS_PROCESS_KILL (113)

use super::{
    syscall0, syscall1, syscall2, syscall3, syscall_result, SyscallError, SYS_PROCESS_CHDIR,
    SYS_PROCESS_EXIT, SYS_PROCESS_EXEC, SYS_PROCESS_FORK, SYS_PROCESS_GETCWD,
    SYS_PROCESS_GETPID, SYS_PROCESS_GETPPID, SYS_PROCESS_KILL, SYS_PROCESS_WAIT,
    SYS_PROCESS_YIELD,
};

/// Exit the current process with the given status code.
///
/// This function never returns.
pub fn exit(status: i32) -> ! {
    // SAFETY: SYS_PROCESS_EXIT terminates the current process.
    // The kernel will never return from this syscall.
    unsafe {
        syscall1(SYS_PROCESS_EXIT, status as usize);
    }
    // The kernel should never return, but just in case:
    loop {
        core::hint::spin_loop();
    }
}

/// Fork the current process.
///
/// # Returns
/// - In the parent: child PID (> 0)
/// - In the child: 0
/// - On error: SyscallError
pub fn fork() -> Result<usize, SyscallError> {
    // SAFETY: fork() is a standard process creation syscall.
    let ret = unsafe { syscall0(SYS_PROCESS_FORK) };
    syscall_result(ret)
}

/// Replace the current process image with a new program.
///
/// # Arguments
/// - `path`: Null-terminated path to the binary
/// - `argv`: Null-terminated array of argument pointers
/// - `envp`: Null-terminated array of environment variable pointers
///
/// On success, this function does not return.
pub fn execve(
    path: *const u8,
    argv: *const *const u8,
    envp: *const *const u8,
) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide valid null-terminated path and
    // null-terminated arrays of null-terminated strings.
    let ret = unsafe { syscall3(SYS_PROCESS_EXEC, path as usize, argv as usize, envp as usize) };
    syscall_result(ret)
}

/// Wait for a child process to change state.
///
/// # Arguments
/// - `pid`: Process ID to wait for (-1 for any child)
/// - `wstatus`: Pointer to store exit status
/// - `options`: Wait options (0 for blocking)
///
/// # Returns
/// PID of the child that changed state.
pub fn waitpid(
    pid: isize,
    wstatus: *mut i32,
    options: usize,
) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid wstatus pointer (or null).
    let ret = unsafe {
        syscall3(
            SYS_PROCESS_WAIT,
            pid as usize,
            wstatus as usize,
            options,
        )
    };
    syscall_result(ret)
}

/// Get the current process ID.
pub fn getpid() -> usize {
    // SAFETY: getpid never fails.
    unsafe { syscall0(SYS_PROCESS_GETPID) as usize }
}

/// Get the parent process ID.
pub fn getppid() -> usize {
    // SAFETY: getppid never fails.
    unsafe { syscall0(SYS_PROCESS_GETPPID) as usize }
}

/// Yield the CPU to another process.
pub fn sched_yield() -> Result<usize, SyscallError> {
    // SAFETY: yield is always safe.
    let ret = unsafe { syscall0(SYS_PROCESS_YIELD) };
    syscall_result(ret)
}

/// Get the current working directory.
///
/// # Arguments
/// - `buf`: Buffer to store the path
/// - `size`: Size of the buffer
///
/// # Returns
/// Length of the path on success.
pub fn getcwd(buf: *mut u8, size: usize) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid buffer of at least `size` bytes.
    let ret = unsafe { syscall2(SYS_PROCESS_GETCWD, buf as usize, size) };
    syscall_result(ret)
}

/// Change the current working directory.
pub fn chdir(path: *const u8) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid null-terminated path.
    let ret = unsafe { syscall1(SYS_PROCESS_CHDIR, path as usize) };
    syscall_result(ret)
}

/// Send a signal to a process.
///
/// # Arguments
/// - `pid`: Target process ID
/// - `sig`: Signal number
pub fn kill(pid: usize, sig: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates pid and signal number.
    let ret = unsafe { syscall2(SYS_PROCESS_KILL, pid, sig) };
    syscall_result(ret)
}

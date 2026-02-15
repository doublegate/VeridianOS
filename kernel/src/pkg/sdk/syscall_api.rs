//! Syscall Wrapper Types for VeridianOS
//!
//! Type-safe wrappers defining the VeridianOS system call interface. These are
//! contract definitions for user-space libraries; the actual implementations
//! use architecture-specific syscall instructions at runtime.

#[cfg(feature = "alloc")]
use alloc::string::String;
use core::fmt;

/// Error codes returned by VeridianOS system calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SyscallError {
    /// One or more arguments are invalid.
    InvalidArgument,
    /// Caller lacks the required capability or permission.
    PermissionDenied,
    /// The requested resource was not found.
    NotFound,
    /// Insufficient memory to complete the operation.
    OutOfMemory,
    /// The resource already exists.
    AlreadyExists,
    /// The operation timed out.
    Timeout,
    /// The operation would block and non-blocking mode was requested.
    WouldBlock,
    /// The operation is not supported on this object.
    NotSupported,
    /// The system call is not yet implemented.
    NotImplemented,
    /// An I/O error occurred.
    IoError,
    /// The file descriptor or handle is invalid.
    BadDescriptor,
    /// The buffer provided is too small.
    BufferTooSmall,
    /// An internal kernel error occurred.
    InternalError,
}

impl fmt::Display for SyscallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument => write!(f, "invalid argument"),
            Self::PermissionDenied => write!(f, "permission denied"),
            Self::NotFound => write!(f, "not found"),
            Self::OutOfMemory => write!(f, "out of memory"),
            Self::AlreadyExists => write!(f, "already exists"),
            Self::Timeout => write!(f, "operation timed out"),
            Self::WouldBlock => write!(f, "operation would block"),
            Self::NotSupported => write!(f, "not supported"),
            Self::NotImplemented => write!(f, "not implemented"),
            Self::IoError => write!(f, "I/O error"),
            Self::BadDescriptor => write!(f, "bad descriptor"),
            Self::BufferTooSmall => write!(f, "buffer too small"),
            Self::InternalError => write!(f, "internal error"),
        }
    }
}

/// Result type for system call operations.
#[allow(dead_code)]
pub type SyscallResult<T> = Result<T, SyscallError>;

/// Basic package information returned by `sys_pkg_query`.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PackageInfo {
    /// Package name.
    pub name: String,
    /// Version string (semver).
    pub version: String,
    /// Whether the package is currently installed.
    pub installed: bool,
}

// ============================================================================
// Process Syscalls
// ============================================================================

/// Fork the current process, returning the child PID to the parent.
///
/// # Returns
/// - `Ok(0)` in the child process
/// - `Ok(child_pid)` in the parent process
#[allow(dead_code)]
pub fn sys_fork() -> SyscallResult<u64> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Replace the current process image with a new program.
///
/// # Arguments
/// - `path`: Path to the executable to load.
/// - `args`: Command-line arguments for the new program.
#[allow(dead_code)]
pub fn sys_exec(_path: &str, _args: &[&str]) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Terminate the current process with the given exit code.
///
/// This function never returns.
#[allow(dead_code)]
pub fn sys_exit(_code: i32) -> ! {
    // Actual implementation via architecture-specific syscall instruction.
    // In the stub we loop forever since this is a diverging function.
    loop {
        core::hint::spin_loop();
    }
}

/// Wait for a child process to exit.
///
/// # Arguments
/// - `pid`: Process ID of the child to wait for.
///
/// # Returns
/// The exit code of the child process.
#[allow(dead_code)]
pub fn sys_wait(_pid: u64) -> SyscallResult<i32> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Return the PID of the calling process.
#[allow(dead_code)]
pub fn sys_getpid() -> u64 {
    // Actual implementation via architecture-specific syscall instruction
    0
}

// ============================================================================
// Memory Syscalls
// ============================================================================

/// Map memory into the calling process's address space.
///
/// # Arguments
/// - `addr`: Preferred virtual address (0 to let the kernel choose).
/// - `len`: Number of bytes to map.
/// - `prot`: Protection flags (read/write/execute bits).
///
/// # Returns
/// The base address of the newly mapped region.
#[allow(dead_code)]
pub fn sys_mmap(_addr: usize, _len: usize, _prot: u32) -> SyscallResult<usize> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Unmap a previously mapped memory region.
///
/// # Arguments
/// - `addr`: Start address of the region (must be page-aligned).
/// - `len`: Number of bytes to unmap.
#[allow(dead_code)]
pub fn sys_munmap(_addr: usize, _len: usize) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// IPC Syscalls
// ============================================================================

/// Send a message to an IPC endpoint.
///
/// # Arguments
/// - `endpoint`: Capability token for the target endpoint.
/// - `msg`: Message payload (must fit within the endpoint's maximum message
///   size).
#[allow(dead_code)]
pub fn sys_ipc_send(_endpoint: u64, _msg: &[u8]) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Receive a message from an IPC endpoint.
///
/// # Arguments
/// - `endpoint`: Capability token for the endpoint to receive from.
/// - `buf`: Buffer to receive the message into.
///
/// # Returns
/// The number of bytes received.
#[allow(dead_code)]
pub fn sys_ipc_receive(_endpoint: u64, _buf: &mut [u8]) -> SyscallResult<usize> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Filesystem Syscalls
// ============================================================================

/// Open a file or directory.
///
/// # Arguments
/// - `path`: Path to the file.
/// - `flags`: Open flags (O_RDONLY, O_WRONLY, O_RDWR, O_CREAT, etc.).
///
/// # Returns
/// A file descriptor for the opened file.
#[allow(dead_code)]
pub fn sys_open(_path: &str, _flags: u32) -> SyscallResult<u64> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Read data from an open file descriptor.
///
/// # Arguments
/// - `fd`: File descriptor returned by `sys_open`.
/// - `buf`: Buffer to read data into.
///
/// # Returns
/// The number of bytes read (0 indicates end-of-file).
#[allow(dead_code)]
pub fn sys_read(_fd: u64, _buf: &mut [u8]) -> SyscallResult<usize> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Write data to an open file descriptor.
///
/// # Arguments
/// - `fd`: File descriptor returned by `sys_open`.
/// - `data`: Data to write.
///
/// # Returns
/// The number of bytes written.
#[allow(dead_code)]
pub fn sys_write(_fd: u64, _data: &[u8]) -> SyscallResult<usize> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Close an open file descriptor.
///
/// # Arguments
/// - `fd`: File descriptor to close.
#[allow(dead_code)]
pub fn sys_close(_fd: u64) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Capability Syscalls
// ============================================================================

/// Create a new capability with the specified rights.
///
/// # Arguments
/// - `rights`: Bitmask of rights to grant (read, write, execute, grant, etc.).
///
/// # Returns
/// The capability token.
#[allow(dead_code)]
pub fn sys_cap_create(_rights: u64) -> SyscallResult<u64> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Grant a capability to another process.
///
/// # Arguments
/// - `cap`: Capability token to grant.
/// - `target`: PID of the target process.
#[allow(dead_code)]
pub fn sys_cap_grant(_cap: u64, _target: u64) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Revoke a previously granted capability.
///
/// # Arguments
/// - `cap`: Capability token to revoke.
#[allow(dead_code)]
pub fn sys_cap_revoke(_cap: u64) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Package Syscalls
// ============================================================================

/// Request installation of a package by name.
///
/// # Arguments
/// - `name`: Package name to install.
#[allow(dead_code)]
pub fn sys_pkg_install(_name: &str) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Request removal of an installed package.
///
/// # Arguments
/// - `name`: Package name to remove.
#[allow(dead_code)]
pub fn sys_pkg_remove(_name: &str) -> SyscallResult<()> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

/// Query information about a package.
///
/// # Arguments
/// - `name`: Package name to query.
///
/// # Returns
/// Package metadata if the package is known.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub fn sys_pkg_query(_name: &str) -> SyscallResult<PackageInfo> {
    // Actual implementation via architecture-specific syscall instruction
    Err(SyscallError::NotImplemented)
}

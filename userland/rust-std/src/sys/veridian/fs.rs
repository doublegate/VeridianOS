//! File system operations for VeridianOS.
//!
//! Maps Rust file I/O operations to VeridianOS syscalls:
//! - `open` -> SYS_FILE_OPEN (50)
//! - `close` -> SYS_FILE_CLOSE (51)
//! - `read` -> SYS_FILE_READ (52)
//! - `write` -> SYS_FILE_WRITE (53)
//! - `seek` -> SYS_FILE_SEEK (54)
//! - `stat` -> SYS_FILE_STAT (55)
//! - `unlink` -> SYS_FILE_UNLINK (157)
//! - `rename` -> SYS_FILE_RENAME (154)
//! - `mkdir` -> SYS_DIR_MKDIR (60)
//! - `rmdir` -> SYS_DIR_RMDIR (61)

use super::{
    syscall1, syscall2, syscall3, syscall_result, SyscallError, SYS_DIR_MKDIR, SYS_DIR_RMDIR,
    SYS_FILE_CLOSE, SYS_FILE_DUP, SYS_FILE_DUP2, SYS_FILE_OPEN, SYS_FILE_PIPE, SYS_FILE_READ,
    SYS_FILE_RENAME, SYS_FILE_SEEK, SYS_FILE_STAT, SYS_FILE_STAT_PATH, SYS_FILE_TRUNCATE,
    SYS_FILE_UNLINK, SYS_FILE_WRITE,
};

// ============================================================================
// Open flags (must match kernel/toolchain definitions)
// ============================================================================

/// Open for reading only.
pub const O_RDONLY: usize = 0;
/// Open for writing only.
pub const O_WRONLY: usize = 1;
/// Open for reading and writing.
pub const O_RDWR: usize = 2;
/// Create file if it does not exist.
pub const O_CREAT: usize = 0x40;
/// Truncate file to zero length.
pub const O_TRUNC: usize = 0x200;
/// Append on each write.
pub const O_APPEND: usize = 0x400;

// ============================================================================
// Seek whence values
// ============================================================================

/// Seek from beginning of file.
pub const SEEK_SET: usize = 0;
/// Seek from current position.
pub const SEEK_CUR: usize = 1;
/// Seek from end of file.
pub const SEEK_END: usize = 2;

// ============================================================================
// Stat structure (matches kernel layout)
// ============================================================================

/// File status information.
///
/// Layout must match the kernel's stat structure passed via syscall.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_nsec: i64,
    pub st_mtime: i64,
    pub st_mtime_nsec: i64,
    pub st_ctime: i64,
    pub st_ctime_nsec: i64,
}

// ============================================================================
// File Operations
// ============================================================================

/// Open a file.
///
/// # Arguments
/// - `path`: Null-terminated path string
/// - `flags`: Open flags (O_RDONLY, O_WRONLY, O_RDWR, O_CREAT, etc.)
/// - `mode`: File permissions (used with O_CREAT)
///
/// # Returns
/// File descriptor on success, error on failure.
pub fn open(path: *const u8, flags: usize, mode: usize) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid null-terminated path pointer.
    // The kernel validates the pointer before dereferencing.
    let ret = unsafe { syscall3(SYS_FILE_OPEN, path as usize, flags, mode) };
    syscall_result(ret)
}

/// Close a file descriptor.
pub fn close(fd: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the file descriptor.
    let ret = unsafe { syscall1(SYS_FILE_CLOSE, fd) };
    syscall_result(ret)
}

/// Read from a file descriptor.
///
/// # Arguments
/// - `fd`: File descriptor
/// - `buf`: Buffer to read into
/// - `count`: Maximum number of bytes to read
///
/// # Returns
/// Number of bytes read on success.
pub fn read(fd: usize, buf: *mut u8, count: usize) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid buffer of at least `count` bytes.
    // The kernel validates the pointer range.
    let ret = unsafe { syscall3(SYS_FILE_READ, fd, buf as usize, count) };
    syscall_result(ret)
}

/// Write to a file descriptor.
///
/// # Arguments
/// - `fd`: File descriptor
/// - `buf`: Buffer to write from
/// - `count`: Number of bytes to write
///
/// # Returns
/// Number of bytes written on success.
pub fn write(fd: usize, buf: *const u8, count: usize) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid buffer of at least `count` bytes.
    // The kernel validates the pointer range.
    let ret = unsafe { syscall3(SYS_FILE_WRITE, fd, buf as usize, count) };
    syscall_result(ret)
}

/// Convenience: write a byte slice to a file descriptor.
pub fn write_bytes(fd: usize, data: &[u8]) -> Result<usize, SyscallError> {
    write(fd, data.as_ptr(), data.len())
}

/// Seek within a file.
///
/// # Arguments
/// - `fd`: File descriptor
/// - `offset`: Offset in bytes
/// - `whence`: SEEK_SET, SEEK_CUR, or SEEK_END
///
/// # Returns
/// New file offset from beginning on success.
pub fn seek(fd: usize, offset: isize, whence: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the fd and whence value.
    let ret = unsafe { syscall3(SYS_FILE_SEEK, fd, offset as usize, whence) };
    syscall_result(ret)
}

/// Get file status by file descriptor.
pub fn fstat(fd: usize, stat_buf: *mut Stat) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid Stat buffer pointer.
    // The kernel validates the pointer and writes the stat data.
    let ret = unsafe { syscall2(SYS_FILE_STAT, fd, stat_buf as usize) };
    syscall_result(ret)
}

/// Get file status by path.
pub fn stat(path: *const u8, stat_buf: *mut Stat) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide valid path and stat buffer pointers.
    let ret = unsafe { syscall2(SYS_FILE_STAT_PATH, path as usize, stat_buf as usize) };
    syscall_result(ret)
}

/// Truncate a file to a specified length.
pub fn ftruncate(fd: usize, length: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the fd.
    let ret = unsafe { syscall2(SYS_FILE_TRUNCATE, fd, length) };
    syscall_result(ret)
}

/// Duplicate a file descriptor.
pub fn dup(oldfd: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates the fd.
    let ret = unsafe { syscall1(SYS_FILE_DUP, oldfd) };
    syscall_result(ret)
}

/// Duplicate a file descriptor to a specific fd number.
pub fn dup2(oldfd: usize, newfd: usize) -> Result<usize, SyscallError> {
    // SAFETY: The kernel validates both fds.
    let ret = unsafe { syscall2(SYS_FILE_DUP2, oldfd, newfd) };
    syscall_result(ret)
}

/// Create a pipe.
///
/// # Arguments
/// - `pipefd`: Pointer to array of two ints [read_fd, write_fd]
pub fn pipe(pipefd: *mut [i32; 2]) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid pointer to a 2-element i32 array.
    let ret = unsafe { syscall1(SYS_FILE_PIPE, pipefd as usize) };
    syscall_result(ret)
}

/// Unlink (delete) a file.
pub fn unlink(path: *const u8) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid null-terminated path.
    let ret = unsafe { syscall1(SYS_FILE_UNLINK, path as usize) };
    syscall_result(ret)
}

/// Rename a file.
pub fn rename(oldpath: *const u8, newpath: *const u8) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide valid null-terminated path pointers.
    let ret = unsafe { syscall2(SYS_FILE_RENAME, oldpath as usize, newpath as usize) };
    syscall_result(ret)
}

// ============================================================================
// Directory Operations
// ============================================================================

/// Create a directory.
pub fn mkdir(path: *const u8, mode: usize) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid null-terminated path.
    let ret = unsafe { syscall2(SYS_DIR_MKDIR, path as usize, mode) };
    syscall_result(ret)
}

/// Remove a directory.
pub fn rmdir(path: *const u8) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid null-terminated path.
    let ret = unsafe { syscall1(SYS_DIR_RMDIR, path as usize) };
    syscall_result(ret)
}

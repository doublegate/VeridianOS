//! Syscall Wrapper Types for VeridianOS
//!
//! Type-safe wrappers defining the VeridianOS system call interface. These are
//! contract definitions for user-space libraries; the actual implementations
//! use architecture-specific syscall instructions at runtime.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

// User-space SDK forward declarations -- see module doc TODO(user-space)

#[cfg(feature = "alloc")]
use alloc::string::String;
use core::fmt;

/// Error codes returned by VeridianOS system calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub type SyscallResult<T> = Result<T, SyscallError>;

/// Basic package information returned by `sys_pkg_query`.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
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

/// Fork the current process, creating a new child process.
///
/// Creates a near-identical copy of the calling process. The child process
/// receives a copy of the parent's address space (using copy-on-write pages
/// for efficiency), file descriptor table, and capability set. The child
/// inherits all capabilities that have the `INHERIT` flag set on them.
///
/// Both the parent and child return from this call, but with different return
/// values so they can distinguish themselves. The child process receives a
/// new unique PID and has its parent PID set to the calling process.
///
/// # Returns
///
/// - `Ok(0)` in the child process.
/// - `Ok(child_pid)` in the parent process, where `child_pid` is the PID of the
///   newly created child.
///
/// # Errors
///
/// - [`SyscallError::OutOfMemory`] - Insufficient memory to create the child
///   process control block or initial page tables.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the capability to
///   create new processes.
/// - [`SyscallError::InternalError`] - The process table is full or an
///   unexpected kernel error occurred.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_exit, sys_fork};
///
/// let result = sys_fork();
/// match result {
///     Ok(0) => {
///         // Child process: execute child-specific logic
///         sys_exit(0);
///     }
///     Ok(child_pid) => {
///         // Parent process: child_pid contains the new child's PID
///         // Optionally wait for the child with sys_wait(child_pid)
///     }
///     Err(e) => {
///         // Fork failed, handle the error
///     }
/// }
/// ```
pub fn sys_fork() -> SyscallResult<u64> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Replace the current process image with a new program loaded from an
/// ELF executable.
///
/// Loads the ELF binary at the given `path`, replaces the calling process's
/// address space with the new program's segments, and begins execution at the
/// ELF entry point. The process PID remains the same, but the virtual memory
/// layout, stack, and instruction pointer are all replaced.
///
/// Capabilities marked with the `INHERIT` flag are preserved across the exec
/// boundary; all other capabilities are dropped. File descriptors marked
/// close-on-exec are closed; remaining descriptors are inherited by the new
/// program image.
///
/// On success this function does not return to the caller because the process
/// image has been replaced. On failure, the original process continues
/// execution and an error is returned.
///
/// # Arguments
///
/// * `path` - Absolute or relative path to the ELF executable to load. The path
///   is resolved through the VFS and must point to a regular file with execute
///   permission. Supports statically linked and dynamically linked ELF binaries
///   (the dynamic linker is invoked automatically for the latter).
/// * `args` - Slice of string arguments to pass to the new program. These are
///   placed on the new program's initial stack and made available through the
///   standard `argc`/`argv` mechanism. The first element is conventionally the
///   program name.
///
/// # Returns
///
/// - `Ok(())` - Never actually returned; on success the process image is
///   replaced and execution continues at the new entry point.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - The file at `path` does not exist.
/// - [`SyscallError::PermissionDenied`] - The caller lacks execute permission
///   on the file or lacks the required capability.
/// - [`SyscallError::InvalidArgument`] - The file is not a valid ELF binary,
///   targets an incompatible architecture, or `args` exceeds the maximum
///   argument size (typically 2 MiB).
/// - [`SyscallError::OutOfMemory`] - Insufficient memory to load the new
///   program segments.
/// - [`SyscallError::IoError`] - An I/O error occurred reading the executable.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_exec, sys_exit, sys_fork, sys_wait};
///
/// let pid = sys_fork().expect("fork failed");
/// if pid == 0 {
///     // Child: replace image with /bin/ls
///     let result = sys_exec("/bin/ls", &["/bin/ls", "-l", "/home"]);
///     // If exec returns, it failed
///     sys_exit(1);
/// } else {
///     // Parent: wait for child to finish
///     let exit_code = sys_wait(pid).expect("wait failed");
/// }
/// ```
pub fn sys_exec(_path: &str, _args: &[&str]) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Terminate the current process with the given exit code.
///
/// Immediately terminates the calling process. All resources held by the
/// process are released in the following order:
///
/// 1. All open file descriptors are closed.
/// 2. All memory mappings are unmapped and physical frames freed.
/// 3. All capabilities not shared with other processes are revoked.
/// 4. The process is moved to the zombie state until the parent calls
///    `sys_wait` to collect the exit code.
/// 5. If the process has children, they are re-parented to the init process
///    (PID 1).
/// 6. A `SIGCHLD`-equivalent notification is sent to the parent process.
///
/// This function never returns. After the cleanup sequence completes, the
/// scheduler selects the next runnable process.
///
/// # Arguments
///
/// * `code` - Exit status code. By convention, `0` indicates success and
///   non-zero values indicate an error. The low 8 bits are made available to
///   the parent via `sys_wait`.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_exit;
///
/// // Exit successfully
/// sys_exit(0);
///
/// // Exit with error code (this line is unreachable)
/// ```
pub fn sys_exit(_code: i32) -> ! {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    // In the stub we loop forever since this is a diverging function.
    loop {
        core::hint::spin_loop();
    }
}

/// Wait for a child process to exit and collect its exit status.
///
/// Blocks the calling process until the specified child process has
/// terminated. Once the child exits, its exit code is returned and the
/// child's zombie entry is removed from the process table (reaped).
///
/// If the child has already exited before this call, the exit code is
/// returned immediately without blocking (zombie reaping). If the specified
/// PID does not correspond to a child of the calling process, an error is
/// returned.
///
/// Future implementations may support `WNOHANG`-style behavior by passing
/// special PID values:
/// - `pid > 0`: Wait for the specific child with that PID.
/// - `pid == 0`: Wait for any child in the same process group (future).
/// - `pid == u64::MAX`: Wait for any child process (future).
///
/// # Arguments
///
/// * `pid` - Process ID of the child to wait for. Must be a direct child of the
///   calling process.
///
/// # Returns
///
/// - `Ok(exit_code)` - The exit status code of the terminated child process. By
///   convention, `0` indicates success and non-zero values indicate an error or
///   signal termination.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - No child process with the given `pid` exists,
///   or the specified process is not a child of the caller.
/// - [`SyscallError::InvalidArgument`] - The `pid` value is invalid (e.g., 0
///   when process group waiting is not yet supported).
/// - [`SyscallError::InternalError`] - An unexpected kernel error occurred
///   during the wait operation.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_exit, sys_fork, sys_wait};
///
/// let child_pid = sys_fork().expect("fork failed");
/// if child_pid == 0 {
///     // Child process work
///     sys_exit(42);
/// } else {
///     // Parent waits for child and retrieves exit code
///     let exit_code = sys_wait(child_pid).expect("wait failed");
///     // exit_code == 42
/// }
/// ```
pub fn sys_wait(_pid: u64) -> SyscallResult<i32> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Return the PID of the calling process.
///
/// Retrieves the unique process identifier for the currently executing
/// process. This is a lightweight operation that always succeeds; the PID
/// is read directly from the current process control block and requires no
/// capability checks.
///
/// The PID is assigned at process creation time by `sys_fork` and remains
/// constant for the lifetime of the process. PID 0 is reserved for the
/// idle/swapper process and PID 1 is reserved for the init process.
///
/// # Returns
///
/// The process ID of the calling process as an unsigned 64-bit integer.
/// This call always succeeds and never returns an error.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_getpid;
///
/// let my_pid = sys_getpid();
/// // my_pid is the unique identifier for this process
/// ```
pub fn sys_getpid() -> u64 {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    0
}

// ============================================================================
// Memory Syscalls
// ============================================================================

/// Map memory into the calling process's virtual address space.
///
/// Allocates a contiguous region of virtual memory and optionally backs it
/// with physical frames. The mapping is anonymous (zero-initialized, not
/// file-backed). Both the requested address and length are rounded to page
/// boundaries (4 KiB on all supported architectures).
///
/// The protection flags control access permissions on the mapped region.
/// The kernel enforces W^X (write XOR execute) policy: a region cannot be
/// both writable and executable simultaneously. To load executable code,
/// first map as writable, write the code, then use a separate call to
/// change protection to read-execute.
///
/// # Arguments
///
/// * `addr` - Preferred virtual address for the mapping. Pass `0` to let the
///   kernel choose a suitable address (recommended). If non-zero, the address
///   must be page-aligned; the kernel may adjust it or return an error if the
///   region is already in use.
/// * `len` - Number of bytes to map. Will be rounded up to the nearest page
///   boundary. Must be greater than zero.
/// * `prot` - Protection flags as a bitmask:
///   - `PROT_READ  (0x1)` - Pages may be read.
///   - `PROT_WRITE (0x2)` - Pages may be written.
///   - `PROT_EXEC  (0x4)` - Pages may be executed.
///   - `0` (PROT_NONE) - Pages cannot be accessed (guard pages).
///
/// # Returns
///
/// - `Ok(base_addr)` - The starting virtual address of the mapped region. When
///   `addr` was `0`, this is the kernel-chosen address.
///
/// # Errors
///
/// - [`SyscallError::InvalidArgument`] - `len` is zero, `addr` is not
///   page-aligned, or `prot` contains both `PROT_WRITE` and `PROT_EXEC` (W^X
///   violation).
/// - [`SyscallError::OutOfMemory`] - Insufficient virtual address space or
///   physical memory to satisfy the mapping.
/// - [`SyscallError::AlreadyExists`] - The requested fixed address range
///   overlaps an existing mapping.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the memory
///   management capability.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_mmap;
///
/// // Allocate 4 KiB of read-write memory at a kernel-chosen address
/// let prot_rw = 0x1 | 0x2; // PROT_READ | PROT_WRITE
/// let addr = sys_mmap(0, 4096, prot_rw).expect("mmap failed");
///
/// // Allocate a 64 KiB guard-page-bounded region
/// let region = sys_mmap(0, 65536, prot_rw).expect("mmap failed");
/// ```
pub fn sys_mmap(_addr: usize, _len: usize, _prot: u32) -> SyscallResult<usize> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Unmap a previously mapped memory region from the calling process's
/// address space.
///
/// Removes the virtual memory mapping starting at `addr` for `len` bytes.
/// Any physical frames backing the region are freed (unless shared with
/// another process via copy-on-write or explicit sharing). Both `addr` and
/// `len` must be page-aligned.
///
/// Partial unmapping is supported: if the specified range covers only part
/// of an existing mapping, that mapping is split and only the requested
/// portion is removed. Accessing unmapped addresses after this call will
/// result in a page fault.
///
/// # Arguments
///
/// * `addr` - Start address of the region to unmap. Must be page-aligned (4 KiB
///   boundary).
/// * `len` - Number of bytes to unmap. Must be page-aligned and greater than
///   zero. The actual unmapped range is `[addr, addr + len)`.
///
/// # Returns
///
/// - `Ok(())` on successful unmapping.
///
/// # Errors
///
/// - [`SyscallError::InvalidArgument`] - `addr` or `len` is not page-aligned,
///   or `len` is zero.
/// - [`SyscallError::NotFound`] - No mapping exists at the specified address
///   range.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the memory
///   management capability.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_mmap, sys_munmap};
///
/// // Map and then unmap a 4 KiB region
/// let addr = sys_mmap(0, 4096, 0x1 | 0x2).expect("mmap failed");
/// sys_munmap(addr, 4096).expect("munmap failed");
/// // Accessing `addr` after this point causes a page fault
/// ```
pub fn sys_munmap(_addr: usize, _len: usize) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// IPC Syscalls
// ============================================================================

/// Send a message to an IPC endpoint.
///
/// Transmits a message to the specified IPC endpoint. The endpoint is
/// identified by a capability token that the sender must possess with
/// `WRITE` rights. The kernel validates the capability before delivering
/// the message.
///
/// For small messages (64 bytes or fewer), the kernel uses a fast-path
/// register-based transfer achieving sub-microsecond latency. For larger
/// messages, the kernel uses zero-copy delivery by remapping the sender's
/// pages into the receiver's address space, avoiding any data copying.
///
/// This call blocks until the receiver has accepted the message or until
/// the endpoint's send queue has space (for asynchronous endpoints). If the
/// endpoint is a synchronous rendezvous endpoint, the sender blocks until a
/// receiver calls `sys_ipc_receive` on the same endpoint.
///
/// # Arguments
///
/// * `endpoint` - Capability token identifying the target IPC endpoint. The
///   caller must hold this capability with at least `WRITE` rights. The token
///   is a 64-bit value obtained from the capability system.
/// * `msg` - Message payload as a byte slice. Must not exceed the endpoint's
///   maximum message size (default 64 KiB). For optimal performance, keep
///   messages at or below 64 bytes to use the register-based fast path.
///
/// # Returns
///
/// - `Ok(())` on successful delivery of the message.
///
/// # Errors
///
/// - [`SyscallError::InvalidArgument`] - `msg` exceeds the endpoint's maximum
///   message size or is empty.
/// - [`SyscallError::PermissionDenied`] - The caller does not hold the endpoint
///   capability or lacks `WRITE` rights on it.
/// - [`SyscallError::NotFound`] - The endpoint capability token is invalid or
///   has been revoked.
/// - [`SyscallError::WouldBlock`] - The endpoint's send queue is full and
///   non-blocking mode was requested.
/// - [`SyscallError::Timeout`] - The operation timed out waiting for the
///   receiver.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_ipc_send;
///
/// let endpoint: u64 = 0x1234; // capability token for the endpoint
/// let message = b"hello from sender";
/// sys_ipc_send(endpoint, message).expect("ipc send failed");
/// ```
pub fn sys_ipc_send(_endpoint: u64, _msg: &[u8]) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Receive a message from an IPC endpoint.
///
/// Blocks the calling process until a message is available on the specified
/// IPC endpoint, then copies the message into the provided buffer. The
/// endpoint is identified by a capability token that the receiver must
/// possess with `READ` rights.
///
/// For small messages (64 bytes or fewer), the data is transferred via
/// registers on the fast path. For larger messages, the kernel uses
/// zero-copy page remapping so the receiver sees the sender's data without
/// any memory copy overhead.
///
/// The caller must provide a buffer large enough to hold the incoming
/// message. If the buffer is too small, the message is truncated and a
/// `BufferTooSmall` error is returned (the message remains consumed from
/// the queue).
///
/// # Arguments
///
/// * `endpoint` - Capability token identifying the IPC endpoint to receive
///   from. The caller must hold this capability with at least `READ` rights.
/// * `buf` - Mutable buffer to receive the message data. Should be at least as
///   large as the expected maximum message size for the endpoint.
///
/// # Returns
///
/// - `Ok(bytes_received)` - The number of bytes written into `buf`.
///
/// # Errors
///
/// - [`SyscallError::PermissionDenied`] - The caller does not hold the endpoint
///   capability or lacks `READ` rights on it.
/// - [`SyscallError::NotFound`] - The endpoint capability token is invalid or
///   has been revoked.
/// - [`SyscallError::BufferTooSmall`] - The provided `buf` is smaller than the
///   incoming message. The message is truncated to fit.
/// - [`SyscallError::WouldBlock`] - No message is available and non-blocking
///   mode was requested.
/// - [`SyscallError::Timeout`] - The operation timed out waiting for a message.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_ipc_receive;
///
/// let endpoint: u64 = 0x1234; // capability token for the endpoint
/// let mut buf = [0u8; 4096];
/// let bytes = sys_ipc_receive(endpoint, &mut buf).expect("ipc receive failed");
/// let received = &buf[..bytes];
/// // Process the received message data
/// ```
pub fn sys_ipc_receive(_endpoint: u64, _buf: &mut [u8]) -> SyscallResult<usize> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Filesystem Syscalls
// ============================================================================

/// Open a file or directory, returning a file descriptor.
///
/// Resolves the given path through the VeridianOS VFS (Virtual File System)
/// and returns a new file descriptor for the opened file. The file descriptor
/// is an index into the per-process file descriptor table and is valid until
/// closed with `sys_close`.
///
/// The caller must hold a capability granting access to the target file or
/// its parent directory. The required capability rights depend on the open
/// flags: `READ` for `O_RDONLY`, `WRITE` for `O_WRONLY` or `O_RDWR`, and
/// both for `O_RDWR`.
///
/// # Arguments
///
/// * `path` - Path to the file or directory to open. Can be absolute (starting
///   with `/`) or relative to the process's current working directory. The path
///   is resolved through the VFS layer (RamFS, DevFS, ProcFS, or BlockFS
///   depending on the mount point).
/// * `flags` - Bitwise OR of open mode flags:
///   - `O_RDONLY  (0x000)` - Open for reading only.
///   - `O_WRONLY  (0x001)` - Open for writing only.
///   - `O_RDWR    (0x002)` - Open for reading and writing.
///   - `O_CREAT   (0x100)` - Create the file if it does not exist.
///   - `O_TRUNC   (0x200)` - Truncate the file to zero length if it exists.
///   - `O_APPEND  (0x400)` - Writes always append to end of file.
///   - `O_EXCL    (0x800)` - With `O_CREAT`, fail if the file already exists.
///
/// # Returns
///
/// - `Ok(fd)` - A file descriptor (unsigned 64-bit integer) for the opened
///   file. The descriptor is the lowest available unused fd in the process's
///   table.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - The file does not exist and `O_CREAT` was not
///   specified, or a path component does not exist.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the required
///   capability for the requested access mode.
/// - [`SyscallError::AlreadyExists`] - `O_CREAT | O_EXCL` was specified and the
///   file already exists.
/// - [`SyscallError::InvalidArgument`] - The path is empty, contains null
///   bytes, or `flags` contains an invalid combination.
/// - [`SyscallError::IoError`] - An I/O error occurred accessing the underlying
///   storage.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_close, sys_open};
///
/// // Open an existing file for reading
/// let fd = sys_open("/etc/config.toml", 0x000).expect("open failed");
///
/// // Create a new file for writing (O_WRONLY | O_CREAT | O_TRUNC)
/// let fd_new = sys_open("/tmp/output.txt", 0x001 | 0x100 | 0x200).expect("create failed");
///
/// sys_close(fd).expect("close failed");
/// sys_close(fd_new).expect("close failed");
/// ```
pub fn sys_open(_path: &str, _flags: u32) -> SyscallResult<u64> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Read data from an open file descriptor.
///
/// Reads up to `buf.len()` bytes from the file descriptor into the
/// provided buffer, starting at the file's current offset. The file offset
/// is advanced by the number of bytes read. Partial reads are possible and
/// normal: the kernel may return fewer bytes than requested if fewer are
/// available (e.g., near end-of-file or for device files).
///
/// A return value of `Ok(0)` indicates end-of-file (EOF): the current offset
/// is at or past the end of the file and no more data can be read.
///
/// For special file descriptors: fd 0 (stdin) reads from the process's
/// standard input source (terminal or pipe).
///
/// # Arguments
///
/// * `fd` - File descriptor returned by a prior call to `sys_open`. Must be
///   opened with at least `O_RDONLY` or `O_RDWR` access.
/// * `buf` - Mutable buffer to read data into. The kernel reads at most
///   `buf.len()` bytes. Must be non-empty.
///
/// # Returns
///
/// - `Ok(bytes_read)` - The number of bytes actually read, which may be less
///   than `buf.len()`. Returns `Ok(0)` at end-of-file.
///
/// # Errors
///
/// - [`SyscallError::BadDescriptor`] - `fd` is not a valid open file
///   descriptor.
/// - [`SyscallError::PermissionDenied`] - The file descriptor was not opened
///   for reading.
/// - [`SyscallError::InvalidArgument`] - `buf` has zero length.
/// - [`SyscallError::IoError`] - An I/O error occurred reading from the
///   underlying storage or device.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_close, sys_open, sys_read};
///
/// let fd = sys_open("/etc/hostname", 0x000).expect("open failed");
/// let mut buf = [0u8; 256];
/// let bytes = sys_read(fd, &mut buf).expect("read failed");
/// if bytes == 0 {
///     // End of file reached
/// }
/// let data = &buf[..bytes];
/// sys_close(fd).expect("close failed");
/// ```
pub fn sys_read(_fd: u64, _buf: &mut [u8]) -> SyscallResult<usize> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Write data to an open file descriptor.
///
/// Writes up to `data.len()` bytes from the provided buffer to the file
/// descriptor, starting at the file's current offset (or at the end of
/// file if `O_APPEND` was set). The file offset is advanced by the number
/// of bytes written. Partial writes are possible: the kernel may write
/// fewer bytes than requested if storage is full or for device files.
///
/// For special file descriptors: fd 1 (stdout) and fd 2 (stderr) write to
/// the process's standard output, which is typically the serial console
/// in VeridianOS.
///
/// # Arguments
///
/// * `fd` - File descriptor returned by a prior call to `sys_open`. Must be
///   opened with at least `O_WRONLY` or `O_RDWR` access.
/// * `data` - Byte slice containing the data to write. The kernel writes at
///   most `data.len()` bytes. Must be non-empty.
///
/// # Returns
///
/// - `Ok(bytes_written)` - The number of bytes actually written, which may be
///   less than `data.len()` (partial write). The caller should retry with the
///   remaining data if a partial write occurs.
///
/// # Errors
///
/// - [`SyscallError::BadDescriptor`] - `fd` is not a valid open file
///   descriptor.
/// - [`SyscallError::PermissionDenied`] - The file descriptor was not opened
///   for writing.
/// - [`SyscallError::InvalidArgument`] - `data` has zero length.
/// - [`SyscallError::IoError`] - An I/O error occurred writing to the
///   underlying storage or device.
/// - [`SyscallError::OutOfMemory`] - The filesystem has no remaining free
///   space.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_close, sys_open, sys_write};
///
/// // Write to a file
/// let fd = sys_open("/tmp/log.txt", 0x001 | 0x100).expect("open failed");
/// let data = b"VeridianOS kernel log entry\n";
/// let written = sys_write(fd, data).expect("write failed");
/// sys_close(fd).expect("close failed");
///
/// // Write to stdout (serial console)
/// let stdout_fd = 1;
/// sys_write(stdout_fd, b"Hello, VeridianOS!\n").expect("write to stdout failed");
/// ```
pub fn sys_write(_fd: u64, _data: &[u8]) -> SyscallResult<usize> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Close an open file descriptor, releasing associated resources.
///
/// Releases the file descriptor so it can be reused by subsequent calls to
/// `sys_open` or other fd-allocating syscalls. If this is the last file
/// descriptor referring to the underlying file description, the file
/// description is also released and any pending writes are flushed.
///
/// Closing a file descriptor that has already been closed is an error.
/// Closing an fd does not guarantee data has been persisted to disk; use
/// a sync operation for durability guarantees.
///
/// # Arguments
///
/// * `fd` - File descriptor to close. Must be a valid, currently open
///   descriptor in the process's file descriptor table.
///
/// # Returns
///
/// - `Ok(())` on successful close.
///
/// # Errors
///
/// - [`SyscallError::BadDescriptor`] - `fd` is not a valid open file descriptor
///   (including the case where it was already closed).
/// - [`SyscallError::IoError`] - An I/O error occurred while flushing buffered
///   data during close.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_close, sys_open};
///
/// let fd = sys_open("/etc/motd", 0x000).expect("open failed");
/// // ... use the file descriptor ...
/// sys_close(fd).expect("close failed");
///
/// // Attempting to close again would return BadDescriptor
/// let result = sys_close(fd);
/// assert!(result.is_err());
/// ```
pub fn sys_close(_fd: u64) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Capability Syscalls
// ============================================================================

/// Create a new capability with the specified rights bitmask.
///
/// Allocates a new capability token and inserts it into the calling
/// process's capability space. The new capability is derived from the
/// caller's parent (root) capability; the caller can only create
/// capabilities with a subset of rights that it already holds.
///
/// Capability tokens are unforgeable 64-bit values with an embedded
/// generation counter used for efficient revocation. The token uniquely
/// identifies the capability across the entire system.
///
/// # Arguments
///
/// * `rights` - Bitmask of rights to assign to the new capability:
///   - `CAP_READ    (0x01)` - Permission to read or receive.
///   - `CAP_WRITE   (0x02)` - Permission to write or send.
///   - `CAP_EXECUTE (0x04)` - Permission to execute.
///   - `CAP_GRANT   (0x08)` - Permission to grant this capability to other
///     processes via `sys_cap_grant`.
///   - `CAP_REVOKE  (0x10)` - Permission to revoke derived capabilities.
///   - `CAP_MAP     (0x20)` - Permission to map associated memory regions.
///
///   Rights are subtractive: you cannot create a capability with rights
///   exceeding those of the parent capability.
///
/// # Returns
///
/// - `Ok(cap_token)` - The 64-bit capability token for the newly created
///   capability.
///
/// # Errors
///
/// - [`SyscallError::PermissionDenied`] - The caller does not hold a parent
///   capability with the requested rights, or lacks the ability to create new
///   capabilities.
/// - [`SyscallError::InvalidArgument`] - `rights` is zero or contains undefined
///   bits.
/// - [`SyscallError::OutOfMemory`] - The capability table is full and cannot
///   accommodate a new entry.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_cap_create;
///
/// // Create a read-only capability
/// let cap_ro = sys_cap_create(0x01).expect("cap_create failed");
///
/// // Create a read-write-grant capability
/// let cap_rwg = sys_cap_create(0x01 | 0x02 | 0x08).expect("cap_create failed");
/// ```
pub fn sys_cap_create(_rights: u64) -> SyscallResult<u64> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Grant a capability to another process, enabling cross-process resource
/// sharing.
///
/// Copies the specified capability into the target process's capability
/// space. The caller must hold the `GRANT` right (`0x08`) on the capability
/// being transferred. The granted capability may have equal or fewer rights
/// than the original; it can never have more rights (rights are monotonically
/// non-increasing through the delegation chain).
///
/// The granted capability becomes a derived child of the original. If the
/// original is later revoked via `sys_cap_revoke`, the derived capability
/// in the target process is also revoked (cascade revocation).
///
/// # Arguments
///
/// * `cap` - Capability token to grant. The caller must hold this capability
///   with the `GRANT` right.
/// * `target` - Process ID of the target process that will receive the
///   capability.
///
/// # Returns
///
/// - `Ok(())` on successful transfer.
///
/// # Errors
///
/// - [`SyscallError::PermissionDenied`] - The caller does not hold the `GRANT`
///   right on the specified capability.
/// - [`SyscallError::NotFound`] - The capability token is invalid or revoked,
///   or the target PID does not correspond to a running process.
/// - [`SyscallError::InvalidArgument`] - `target` is the caller's own PID
///   (self-grant is a no-op error) or an invalid PID value.
/// - [`SyscallError::OutOfMemory`] - The target process's capability table is
///   full.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_cap_create, sys_cap_grant};
///
/// // Create a grantable read-write capability
/// let cap = sys_cap_create(0x01 | 0x02 | 0x08).expect("cap_create failed");
///
/// // Grant it to process with PID 42
/// let target_pid: u64 = 42;
/// sys_cap_grant(cap, target_pid).expect("cap_grant failed");
/// ```
pub fn sys_cap_grant(_cap: u64, _target: u64) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Revoke a capability and all capabilities derived from it.
///
/// Invalidates the specified capability token and performs cascade
/// revocation: all capabilities that were derived from it (via
/// `sys_cap_grant` or `sys_cap_create`) are also revoked, recursively.
/// This ensures that once a capability is revoked, no process in the
/// system retains access through that delegation chain.
///
/// The generation counter embedded in the capability token is incremented
/// so that any cached references to the revoked token are immediately
/// detected as stale by the kernel's O(1) capability lookup.
///
/// After revocation, any attempt to use the revoked token (or its
/// derivatives) in a syscall will return `NotFound`.
///
/// # Arguments
///
/// * `cap` - Capability token to revoke. The caller must hold this capability
///   (or its parent with `REVOKE` rights).
///
/// # Returns
///
/// - `Ok(())` on successful revocation of the capability and all its
///   descendants.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - The capability token is invalid or was
///   already revoked.
/// - [`SyscallError::PermissionDenied`] - The caller does not own the
///   capability and does not hold a parent with `REVOKE` rights.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::{sys_cap_create, sys_cap_grant, sys_cap_revoke};
///
/// // Create and grant a capability
/// let cap = sys_cap_create(0x01 | 0x08).expect("cap_create failed");
/// sys_cap_grant(cap, 42).expect("cap_grant failed");
///
/// // Revoke the capability -- process 42's derived copy is also revoked
/// sys_cap_revoke(cap).expect("cap_revoke failed");
/// ```
pub fn sys_cap_revoke(_cap: u64) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Package Syscalls
// ============================================================================

/// Install a package by name from the configured repositories.
///
/// Initiates a transactional package installation. The package manager
/// resolves all dependencies using the DPLL-based SAT solver, downloads
/// the package and its dependencies from the configured repository mirrors,
/// verifies cryptographic signatures, and installs the files into the
/// system sysroot. The entire operation is atomic: if any step fails, all
/// changes are rolled back.
///
/// The installation process follows these steps:
/// 1. Query configured repositories for the package and its metadata.
/// 2. Resolve dependencies (transitive closure) using the SAT solver.
/// 3. Download package archives (with delta updates when available).
/// 4. Verify Ed25519/ML-DSA signatures on each archive.
/// 5. Extract files into the target locations within the VFS.
/// 6. Update the package database with installation records.
/// 7. Run post-install hooks if defined in the package manifest.
///
/// # Arguments
///
/// * `name` - Name of the package to install. Must match a package name in at
///   least one configured repository. Version constraints can be appended
///   (e.g., `"foo>=1.2.0"`) or the latest available version is selected.
///
/// # Returns
///
/// - `Ok(())` on successful installation of the package and all its
///   dependencies.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - The package name was not found in any
///   configured repository.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the package
///   management capability.
/// - [`SyscallError::InvalidArgument`] - The package name is empty or contains
///   invalid characters.
/// - [`SyscallError::AlreadyExists`] - The exact version of the package is
///   already installed.
/// - [`SyscallError::IoError`] - A network or disk I/O error occurred during
///   download or extraction.
/// - [`SyscallError::OutOfMemory`] - Insufficient disk space to install the
///   package.
/// - [`SyscallError::InternalError`] - Dependency resolution failed (e.g.,
///   unsatisfiable constraints) or signature verification failed.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_pkg_install;
///
/// // Install a package by name (latest version)
/// sys_pkg_install("libveridian-dev").expect("install failed");
///
/// // Install with version constraint
/// sys_pkg_install("openssl>=3.0.0").expect("install failed");
/// ```
pub fn sys_pkg_install(_name: &str) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Remove an installed package from the system.
///
/// Performs a transactional package removal. Configuration files installed
/// by the package are preserved by default (saved with a `.veridian-save`
/// suffix) so the user does not lose customizations. After removal, the
/// package manager checks for orphaned dependencies (packages that were
/// installed only as dependencies of the removed package and are no longer
/// required) and offers to remove them.
///
/// The removal process follows these steps:
/// 1. Verify the package is installed and not required by other packages.
/// 2. Run pre-removal hooks if defined in the package manifest.
/// 3. Remove installed files from the VFS (preserving config files).
/// 4. Update the package database to mark the package as removed.
/// 5. Detect and optionally clean up orphaned dependencies.
/// 6. Run post-removal hooks if defined.
///
/// # Arguments
///
/// * `name` - Name of the installed package to remove.
///
/// # Returns
///
/// - `Ok(())` on successful removal of the package.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - The package is not currently installed.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the package
///   management capability.
/// - [`SyscallError::InvalidArgument`] - The package name is empty or contains
///   invalid characters.
/// - [`SyscallError::NotSupported`] - The package is a critical system package
///   that cannot be removed, or other installed packages depend on it and
///   `--force` was not specified.
/// - [`SyscallError::IoError`] - A disk I/O error occurred during file removal.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_pkg_remove;
///
/// // Remove a package (config files are preserved)
/// sys_pkg_remove("libfoo").expect("remove failed");
/// ```
pub fn sys_pkg_remove(_name: &str) -> SyscallResult<()> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

/// Query information about a package by name.
///
/// Looks up the specified package in both the installed package database
/// and the configured repositories. Returns a [`PackageInfo`] struct
/// containing the package's name, version, and installation status.
///
/// This is a read-only operation that does not modify the system. It can
/// be used to check whether a package is installed, discover available
/// versions, or retrieve metadata before performing an install or remove
/// operation.
///
/// # Arguments
///
/// * `name` - Name of the package to query. Searches the installed package
///   database first, then falls back to configured repository indices.
///
/// # Returns
///
/// - `Ok(PackageInfo)` - Package metadata including:
///   - `name`: The canonical package name.
///   - `version`: The installed or latest available version string (semver).
///   - `installed`: `true` if the package is currently installed, `false` if it
///     is only available in repositories.
///
/// # Errors
///
/// - [`SyscallError::NotFound`] - The package name was not found in the
///   installed database or any configured repository.
/// - [`SyscallError::InvalidArgument`] - The package name is empty or contains
///   invalid characters.
/// - [`SyscallError::PermissionDenied`] - The caller lacks the capability to
///   query package information.
/// - [`SyscallError::IoError`] - An I/O error occurred reading the package
///   database or repository index.
///
/// # Examples
///
/// ```no_run
/// use veridian_kernel::pkg::sdk::syscall_api::sys_pkg_query;
///
/// let info = sys_pkg_query("libveridian").expect("query failed");
/// if info.installed {
///     // Package is installed, version info.version
/// } else {
///     // Package is available but not installed
/// }
/// ```
#[cfg(feature = "alloc")]
pub fn sys_pkg_query(_name: &str) -> SyscallResult<PackageInfo> {
    // TODO(user-space): Requires user-space process execution.
    // Implementation will use architecture-specific syscall instruction
    // (syscall on x86_64, svc on AArch64, ecall on RISC-V).
    Err(SyscallError::NotImplemented)
}

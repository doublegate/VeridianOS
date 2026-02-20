//! Comprehensive error types for VeridianOS kernel
//!
//! This module provides proper error types to replace string literals
//! throughout the kernel, as recommended in DEEP-RECOMMENDATIONS.md.

use core::fmt;

/// Main kernel error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "kernel errors must be handled, not silently discarded"]
pub enum KernelError {
    /// Memory-related errors
    OutOfMemory {
        requested: usize,
        available: usize,
    },
    InvalidAddress {
        addr: usize,
    },
    UnmappedMemory {
        addr: usize,
    },

    /// Capability-related errors
    InvalidCapability {
        cap_id: u64,
        reason: CapError,
    },
    InsufficientRights {
        required: u16,
        actual: u16,
    },
    CapabilityRevoked {
        cap_id: u64,
    },

    /// Process-related errors
    ProcessNotFound {
        pid: u64,
    },
    ThreadNotFound {
        tid: u64,
    },
    InvalidState {
        expected: &'static str,
        actual: &'static str,
    },

    /// IPC-related errors
    IpcError(IpcError),

    /// Scheduler-related errors
    SchedulerError(SchedError),

    /// System call errors
    SyscallError(SyscallError),

    /// Filesystem-related errors
    FsError(FsError),

    /// Hardware errors
    HardwareError {
        device: &'static str,
        code: u32,
    },

    /// Generic errors
    InvalidArgument {
        name: &'static str,
        value: &'static str,
    },
    OperationNotSupported {
        operation: &'static str,
    },
    ResourceExhausted {
        resource: &'static str,
    },
    PermissionDenied {
        operation: &'static str,
    },
    AlreadyExists {
        resource: &'static str,
        id: u64,
    },
    NotFound {
        resource: &'static str,
        id: u64,
    },
    Timeout {
        operation: &'static str,
        duration_ms: u64,
    },
    NotImplemented {
        feature: &'static str,
    },
    /// Operation would block
    WouldBlock,
    /// Broken pipe: write end closed or read end closed
    BrokenPipe,
    /// Subsystem not initialized (called before init())
    NotInitialized {
        subsystem: &'static str,
    },
    /// Legacy string error for gradual migration from &'static str patterns.
    /// New code should use specific error variants instead.
    LegacyError {
        message: &'static str,
    },
}

/// Capability-specific errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapError {
    InvalidCapability,
    InsufficientRights,
    CapabilityRevoked,
    InvalidObject,
    PermissionDenied,
    AlreadyExists,
    NotFound,
    IdExhausted,
}

/// IPC-specific errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidEndpoint { id: u64 },
    InvalidChannel { id: u64 },
    MessageTooLarge { size: usize, max: usize },
    QueueFull { capacity: usize },
    QueueEmpty,
    InvalidCapability,
    ProcessNotFound { pid: u64 },
    EndpointNotFound { id: u64 },
    PermissionDenied,
    WouldBlock,
    Timeout,
}

/// Scheduler-specific errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedError {
    InvalidPriority { priority: u8 },
    InvalidCpuId { cpu: usize },
    TaskNotFound { id: u64 },
    CpuOffline { cpu: usize },
    InvalidAffinity,
    QueueEmpty,
    AlreadyScheduled,
}

/// System call errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallError {
    InvalidSyscall { nr: usize },
    InvalidArgument { arg: usize },
    InvalidPointer { addr: usize },
    BufferTooSmall { required: usize, provided: usize },
    StringTooLong { max: usize },
    AccessDenied,
    NotImplemented,
}

/// Filesystem-specific errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// File or directory not found
    NotFound,
    /// Path already exists
    AlreadyExists,
    /// Permission denied
    PermissionDenied,
    /// Target is not a directory
    NotADirectory,
    /// Target is not a file
    NotAFile,
    /// Target is a directory (when file expected)
    IsADirectory,
    /// Filesystem is read-only
    ReadOnly,
    /// Invalid path format
    InvalidPath,
    /// Root filesystem not mounted
    NoRootFs,
    /// Path already has a mount point
    AlreadyMounted,
    /// Path is not a mount point
    NotMounted,
    /// Unknown filesystem type
    UnknownFsType,
    /// I/O error during operation
    IoError,
    /// Directory is not empty
    DirectoryNotEmpty,
    /// File descriptor table is full
    TooManyOpenFiles,
    /// Invalid file descriptor
    BadFileDescriptor,
    /// Operation not supported on this node type
    NotSupported,
    /// Target is not a symbolic link
    NotASymlink,
    /// File size exceeds maximum supported limit
    FileTooLarge,
}

/// Result type alias for kernel operations
pub type KernelResult<T> = Result<T, KernelError>;

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory {
                requested,
                available,
            } => {
                write!(
                    f,
                    "Out of memory: requested {} bytes, {} available",
                    requested, available
                )
            }
            Self::InvalidAddress { addr } => write!(f, "Invalid address: 0x{:x}", addr),
            Self::UnmappedMemory { addr } => write!(f, "Unmapped memory at 0x{:x}", addr),
            Self::InvalidCapability { cap_id, reason } => {
                write!(f, "Invalid capability {}: {:?}", cap_id, reason)
            }
            Self::InsufficientRights { required, actual } => {
                write!(
                    f,
                    "Insufficient rights: required 0x{:x}, have 0x{:x}",
                    required, actual
                )
            }
            Self::CapabilityRevoked { cap_id } => {
                write!(f, "Capability {} has been revoked", cap_id)
            }
            Self::ProcessNotFound { pid } => write!(f, "Process {} not found", pid),
            Self::ThreadNotFound { tid } => write!(f, "Thread {} not found", tid),
            Self::InvalidState { expected, actual } => {
                write!(f, "Invalid state: expected {}, got {}", expected, actual)
            }
            Self::IpcError(e) => write!(f, "IPC error: {:?}", e),
            Self::SchedulerError(e) => write!(f, "Scheduler error: {:?}", e),
            Self::SyscallError(e) => write!(f, "Syscall error: {:?}", e),
            Self::HardwareError { device, code } => {
                write!(f, "Hardware error on {}: code 0x{:x}", device, code)
            }
            Self::InvalidArgument { name, value } => {
                write!(f, "Invalid argument '{}': {}", name, value)
            }
            Self::OperationNotSupported { operation } => {
                write!(f, "Operation not supported: {}", operation)
            }
            Self::ResourceExhausted { resource } => write!(f, "Resource exhausted: {}", resource),
            Self::PermissionDenied { operation } => {
                write!(f, "Permission denied for operation: {}", operation)
            }
            Self::AlreadyExists { resource, id } => {
                write!(f, "{} with id {} already exists", resource, id)
            }
            Self::NotFound { resource, id } => write!(f, "{} with id {} not found", resource, id),
            Self::Timeout {
                operation,
                duration_ms,
            } => {
                write!(f, "Timeout during {}: {} ms", operation, duration_ms)
            }
            Self::NotImplemented { feature } => {
                write!(f, "Feature not implemented: {}", feature)
            }
            Self::WouldBlock => write!(f, "Operation would block"),
            Self::BrokenPipe => write!(f, "Broken pipe"),
            Self::FsError(e) => write!(f, "Filesystem error: {:?}", e),
            Self::NotInitialized { subsystem } => {
                write!(f, "Subsystem not initialized: {}", subsystem)
            }
            Self::LegacyError { message } => write!(f, "{}", message),
        }
    }
}

// Conversion implementations
impl From<CapError> for KernelError {
    fn from(err: CapError) -> Self {
        match err {
            CapError::InvalidCapability => Self::InvalidCapability {
                cap_id: 0,
                reason: err,
            },
            CapError::InsufficientRights => Self::InsufficientRights {
                required: 0,
                actual: 0,
            },
            CapError::CapabilityRevoked => Self::CapabilityRevoked { cap_id: 0 },
            CapError::IdExhausted => Self::ResourceExhausted {
                resource: "capability IDs",
            },
            _ => Self::InvalidCapability {
                cap_id: 0,
                reason: err,
            },
        }
    }
}

impl From<IpcError> for KernelError {
    fn from(err: IpcError) -> Self {
        Self::IpcError(err)
    }
}

impl From<SchedError> for KernelError {
    fn from(err: SchedError) -> Self {
        Self::SchedulerError(err)
    }
}

impl From<SyscallError> for KernelError {
    fn from(err: SyscallError) -> Self {
        Self::SyscallError(err)
    }
}

impl From<FsError> for KernelError {
    fn from(err: FsError) -> Self {
        Self::FsError(err)
    }
}

/// Conversion from legacy &'static str errors to KernelError.
///
/// This enables gradual migration: functions returning Result<T, &'static str>
/// can be called with `?` from functions returning Result<T, KernelError>.
/// New code should prefer specific error variants over this conversion.
impl From<&'static str> for KernelError {
    fn from(msg: &'static str) -> Self {
        Self::LegacyError { message: msg }
    }
}

// Helper macro for easy error creation
#[macro_export]
macro_rules! kernel_error {
    (OutOfMemory { requested: $req:expr, available: $avail:expr }) => {
        $crate::error::KernelError::OutOfMemory {
            requested: $req,
            available: $avail,
        }
    };
    (ProcessNotFound { pid: $pid:expr }) => {
        $crate::error::KernelError::ProcessNotFound { pid: $pid }
    };
    (InvalidArgument { $name:expr => $value:expr }) => {
        $crate::error::KernelError::InvalidArgument {
            name: $name,
            value: $value,
        }
    };
    ($variant:ident) => {
        $crate::error::KernelError::$variant
    };
}

//! IPC error types and result definitions

use core::fmt;

/// IPC operation result type
pub type Result<T> = core::result::Result<T, IpcError>;

/// IPC error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum IpcError {
    /// Invalid or revoked capability
    InvalidCapability,
    /// Target process not found
    ProcessNotFound,
    /// Target endpoint does not exist
    EndpointNotFound,
    /// Message size exceeds maximum allowed
    MessageTooLarge,
    /// No memory available for operation
    OutOfMemory,
    /// Operation would block but non-blocking mode requested
    WouldBlock,
    /// Rate limit exceeded for this endpoint
    RateLimitExceeded,
    /// Operation timed out
    Timeout,
    /// Permission denied for the requested operation
    PermissionDenied,
    /// Invalid message format or parameters
    InvalidMessage,
    /// Channel is full (for async channels)
    ChannelFull,
    /// Channel is empty (for async channels)
    ChannelEmpty,
    /// Endpoint is already bound to another process
    EndpointBusy,
    /// Invalid memory region specified
    InvalidMemoryRegion,
    /// Resource temporarily unavailable
    ResourceBusy,
}

impl IpcError {
    /// Get a static string description of the error
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidCapability => "Invalid or revoked capability",
            Self::ProcessNotFound => "Target process not found",
            Self::EndpointNotFound => "Endpoint not found",
            Self::MessageTooLarge => "Message too large",
            Self::OutOfMemory => "Out of memory",
            Self::WouldBlock => "Operation would block",
            Self::RateLimitExceeded => "Rate limit exceeded",
            Self::Timeout => "Operation timed out",
            Self::PermissionDenied => "Permission denied",
            Self::InvalidMessage => "Invalid message format",
            Self::ChannelFull => "Channel is full",
            Self::ChannelEmpty => "Channel is empty",
            Self::EndpointBusy => "Endpoint is busy",
            Self::InvalidMemoryRegion => "Invalid memory region",
            Self::ResourceBusy => "Resource temporarily unavailable",
        }
    }

    /// Convert error to a numeric code for system calls
    #[allow(dead_code)]
    pub fn to_errno(self) -> i32 {
        match self {
            Self::InvalidCapability => -1,
            Self::ProcessNotFound => -2,
            Self::EndpointNotFound => -3,
            Self::MessageTooLarge => -4,
            Self::OutOfMemory => -5,
            Self::WouldBlock => -6,
            Self::RateLimitExceeded => -7,
            Self::Timeout => -8,
            Self::PermissionDenied => -9,
            Self::InvalidMessage => -10,
            Self::ChannelFull => -11,
            Self::ChannelEmpty => -12,
            Self::EndpointBusy => -13,
            Self::InvalidMemoryRegion => -14,
            Self::ResourceBusy => -15,
        }
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

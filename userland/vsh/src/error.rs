//! Error types for vsh.
//!
//! Defines the central `VshError` enum and `Result` type alias used
//! throughout the shell.

use alloc::string::String;
use core::fmt;

/// Central error type for all vsh operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // All variants are part of the shell error API
pub enum VshError {
    /// Syntax error during lexing or parsing.
    Syntax(String),
    /// A command was not found.
    CommandNotFound(String),
    /// An I/O error occurred (with errno).
    Io(i32),
    /// A variable or parameter expansion error.
    Expansion(String),
    /// Redirection failed.
    Redirection(String),
    /// A signal was received.
    Signal(i32),
    /// Permission denied.
    PermissionDenied(String),
    /// A numeric argument was required.
    NotANumber(String),
    /// Division by zero in arithmetic.
    DivisionByZero,
    /// Assignment to a readonly variable.
    ReadOnly(String),
    /// Invalid option or argument.
    InvalidArgument(String),
    /// Fork failed.
    ForkFailed,
    /// Exec failed.
    ExecFailed(String),
    /// Pipe creation failed.
    PipeFailed,
    /// Out of memory.
    OutOfMemory,
    /// Exit requested with a status code.
    Exit(i32),
}

impl fmt::Display for VshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VshError::Syntax(msg) => write!(f, "syntax error: {}", msg),
            VshError::CommandNotFound(cmd) => write!(f, "{}: command not found", cmd),
            VshError::Io(errno) => write!(f, "I/O error (errno {})", errno),
            VshError::Expansion(msg) => write!(f, "expansion error: {}", msg),
            VshError::Redirection(msg) => write!(f, "redirection error: {}", msg),
            VshError::Signal(sig) => write!(f, "received signal {}", sig),
            VshError::PermissionDenied(path) => write!(f, "{}: Permission denied", path),
            VshError::NotANumber(s) => write!(f, "{}: not a valid number", s),
            VshError::DivisionByZero => write!(f, "division by zero"),
            VshError::ReadOnly(name) => write!(f, "{}: readonly variable", name),
            VshError::InvalidArgument(msg) => write!(f, "invalid argument: {}", msg),
            VshError::ForkFailed => write!(f, "fork: failed to create child process"),
            VshError::ExecFailed(cmd) => write!(f, "{}: exec failed", cmd),
            VshError::PipeFailed => write!(f, "pipe: failed to create pipe"),
            VshError::OutOfMemory => write!(f, "out of memory"),
            VshError::Exit(code) => write!(f, "exit {}", code),
        }
    }
}

/// Result type alias for vsh operations.
pub type Result<T> = core::result::Result<T, VshError>;

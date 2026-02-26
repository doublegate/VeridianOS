//! Structured kernel log service
//!
//! Provides a fixed-size, heap-free circular buffer of structured log entries.
//! Each entry carries a timestamp, severity level, subsystem tag, and a
//! fixed-length message. The service is stored as global state behind a
//! [`spin::Mutex`] and accessed through a small public API.
//!
//! # Usage
//!
//! ```ignore
//! log_service::log_init();
//! log_service::klog(LogLevel::Info, "sched", "scheduler initialized");
//! let n = log_service::log_count();
//! ```
//!
//! The buffer holds up to [`LOG_BUFFER_CAPACITY`] entries. Once full it wraps
//! around and silently overwrites the oldest entries.

// Log service module

use spin::Mutex;

use crate::sync::once_lock::GlobalState;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum number of log entries the circular buffer can hold.
const LOG_BUFFER_CAPACITY: usize = 256;

/// Maximum length (in bytes) of a log message stored in a [`LogEntry`].
const LOG_MESSAGE_MAX_LEN: usize = 128;

/// Maximum length (in bytes) of the subsystem tag in a [`LogEntry`].
const LOG_SUBSYSTEM_MAX_LEN: usize = 16;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Severity levels for kernel log messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    /// Unrecoverable or critical errors.
    Error = 0,
    /// Conditions that may indicate a problem.
    Warn = 1,
    /// Normal operational messages.
    Info = 2,
    /// Verbose diagnostic output.
    Debug = 3,
    /// Very detailed tracing information.
    Trace = 4,
}

/// A single structured log entry.
///
/// All fields are stored inline with fixed-size arrays so that the entry
/// can live in a static circular buffer without heap allocation.
#[derive(Clone)]
pub struct LogEntry {
    /// Milliseconds since boot (via `arch::timer::get_timestamp_ms`).
    pub timestamp_ms: u64,
    /// Severity of the message.
    pub level: LogLevel,
    /// Short subsystem identifier (e.g. `"sched"`, `"mm"`, `"ipc"`).
    /// Stored as a fixed-size byte array with the actual length tracked
    /// separately.
    subsystem_buf: [u8; LOG_SUBSYSTEM_MAX_LEN],
    subsystem_len: u8,
    /// The log message text, truncated to [`LOG_MESSAGE_MAX_LEN`] bytes.
    message_buf: [u8; LOG_MESSAGE_MAX_LEN],
    message_len: u8,
}

impl LogEntry {
    /// Create a zeroed, empty entry (used to initialize the buffer).
    const fn empty() -> Self {
        Self {
            timestamp_ms: 0,
            level: LogLevel::Trace,
            subsystem_buf: [0u8; LOG_SUBSYSTEM_MAX_LEN],
            subsystem_len: 0,
            message_buf: [0u8; LOG_MESSAGE_MAX_LEN],
            message_len: 0,
        }
    }

    /// Return the subsystem tag as a `&str`.
    pub fn subsystem(&self) -> &str {
        let len = self.subsystem_len as usize;
        // SAFETY/invariant: subsystem_len is always set from a valid UTF-8
        // source (an incoming &str) and capped at LOG_SUBSYSTEM_MAX_LEN.
        core::str::from_utf8(&self.subsystem_buf[..len]).unwrap_or("")
    }

    /// Return the message text as a `&str`.
    pub fn message(&self) -> &str {
        let len = self.message_len as usize;
        core::str::from_utf8(&self.message_buf[..len]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Circular buffer
// ---------------------------------------------------------------------------

/// Fixed-size circular buffer of [`LogEntry`] items.
///
/// Uses head/tail indices with a count to distinguish empty from full.
struct LogBuffer {
    entries: [LogEntry; LOG_BUFFER_CAPACITY],
    /// Index of the next slot to write.
    head: usize,
    /// Total number of valid entries (capped at `LOG_BUFFER_CAPACITY`).
    count: usize,
}

impl LogBuffer {
    /// Create a new empty buffer.
    fn new() -> Self {
        // Initialize with empty entries using array::from_fn to avoid Copy
        // requirement (LogEntry is Clone but not Copy due to large arrays).
        const EMPTY: LogEntry = LogEntry::empty();
        Self {
            entries: [EMPTY; LOG_BUFFER_CAPACITY],
            head: 0,
            count: 0,
        }
    }

    /// Append a log entry, overwriting the oldest if full.
    fn push(&mut self, entry: LogEntry) {
        self.entries[self.head] = entry;
        self.head = (self.head + 1) % LOG_BUFFER_CAPACITY;
        if self.count < LOG_BUFFER_CAPACITY {
            self.count += 1;
        }
    }

    /// Number of entries currently stored.
    fn len(&self) -> usize {
        self.count
    }

    /// Clear all entries.
    fn clear(&mut self) {
        self.head = 0;
        self.count = 0;
    }

    /// Return the tail index (oldest entry).
    fn tail(&self) -> usize {
        if self.count < LOG_BUFFER_CAPACITY {
            0
        } else {
            self.head // when full, head == tail (oldest)
        }
    }

    /// Get the entry at logical index `i` (0 = oldest).
    ///
    /// Returns `None` if `i >= count`.
    fn get(&self, i: usize) -> Option<&LogEntry> {
        if i >= self.count {
            return None;
        }
        let physical = (self.tail() + i) % LOG_BUFFER_CAPACITY;
        Some(&self.entries[physical])
    }
}

// ---------------------------------------------------------------------------
// LogService
// ---------------------------------------------------------------------------

/// The kernel log service wrapping a [`LogBuffer`].
struct LogService {
    buffer: LogBuffer,
}

impl LogService {
    fn new() -> Self {
        Self {
            buffer: LogBuffer::new(),
        }
    }

    /// Record a log entry.
    fn log(&mut self, level: LogLevel, subsystem: &str, message: &str) {
        let timestamp_ms = crate::arch::timer::get_timestamp_ms();

        let mut subsystem_buf = [0u8; LOG_SUBSYSTEM_MAX_LEN];
        let sub_len = subsystem.len().min(LOG_SUBSYSTEM_MAX_LEN);
        subsystem_buf[..sub_len].copy_from_slice(&subsystem.as_bytes()[..sub_len]);

        let mut message_buf = [0u8; LOG_MESSAGE_MAX_LEN];
        let msg_len = message.len().min(LOG_MESSAGE_MAX_LEN);
        message_buf[..msg_len].copy_from_slice(&message.as_bytes()[..msg_len]);

        let entry = LogEntry {
            timestamp_ms,
            level,
            subsystem_buf,
            subsystem_len: sub_len as u8,
            message_buf,
            message_len: msg_len as u8,
        };

        self.buffer.push(entry);
    }

    /// Number of entries in the buffer.
    fn count(&self) -> usize {
        self.buffer.len()
    }

    /// Clear all entries.
    fn clear(&mut self) {
        self.buffer.clear();
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static LOG_SERVICE: GlobalState<Mutex<LogService>> = GlobalState::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the kernel log service.
///
/// Must be called once during kernel boot, after the timer subsystem is
/// available. Subsequent calls are silently ignored (returns `Ok(())`).
pub fn log_init() {
    let _ = LOG_SERVICE.init(Mutex::new(LogService::new()));
}

/// Record a structured log entry.
///
/// If the log service has not been initialized yet (i.e., called before
/// `log_init()`), the message is silently dropped.
pub fn klog(level: LogLevel, subsystem: &str, message: &str) {
    LOG_SERVICE.with_mut(|lock| {
        lock.lock().log(level, subsystem, message);
    });
}

/// Iterate over all buffered log entries from oldest to newest, calling `f`
/// for each.
///
/// Returns the number of entries visited, or `None` if the service is not
/// initialized.
pub fn log_drain<F: FnMut(&LogEntry)>(mut f: F) -> Option<usize> {
    LOG_SERVICE.with(|lock| {
        let service = lock.lock();
        let n = service.buffer.len();
        for i in 0..n {
            if let Some(entry) = service.buffer.get(i) {
                f(entry);
            }
        }
        n
    })
}

/// Return the number of entries currently in the log buffer.
///
/// Returns `None` if the service is not initialized.
pub fn log_count() -> Option<usize> {
    LOG_SERVICE.with(|lock| lock.lock().count())
}

/// Clear all log entries.
///
/// Returns `None` if the service is not initialized.
pub fn log_clear() -> Option<()> {
    LOG_SERVICE.with_mut(|lock| lock.lock().clear())
}

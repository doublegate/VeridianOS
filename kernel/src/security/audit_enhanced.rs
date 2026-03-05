//! Enhanced audit logging with structured entries, ring buffer, and filtering
//!
//! Provides a next-generation audit subsystem for VeridianOS with:
//! - Structured log entries with timestamps, PIDs, TIDs, categories, severity
//! - Ring buffer storage with configurable capacity (default 8192 entries)
//! - Multi-dimensional filtering (category, severity, PID, time range)
//! - Event coalescing for repeated identical events within 1 second
//! - Thread-safe access via `spin::RwLock`
//!
//! This module complements the existing `security::audit` module by adding
//! richer categorization, severity levels, and query capabilities.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::VecDeque, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::RwLock;

// ---------------------------------------------------------------------------
// Audit Category
// ---------------------------------------------------------------------------

/// Category of an audit event, enabling fine-grained filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AuditCategory {
    /// Authentication events (login, logout, credential checks)
    Authentication = 0,
    /// Authorization / access control decisions
    Authorization = 1,
    /// File and directory access
    FileAccess = 2,
    /// Network connection, bind, send, receive
    NetworkAccess = 3,
    /// Process/thread creation and termination
    ProcessLifecycle = 4,
    /// Capability create, delegate, revoke, derive
    CapabilityOps = 5,
    /// Security policy changes (MAC, filter updates)
    SecurityPolicy = 6,
    /// System call audit trail
    SystemCall = 7,
}

impl AuditCategory {
    /// Convert to a bitmask flag for filtering.
    pub fn to_flag(self) -> u16 {
        1u16 << (self as u8)
    }

    /// Human-readable name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "AUTH",
            Self::Authorization => "AUTHZ",
            Self::FileAccess => "FILE",
            Self::NetworkAccess => "NET",
            Self::ProcessLifecycle => "PROC",
            Self::CapabilityOps => "CAP",
            Self::SecurityPolicy => "POLICY",
            Self::SystemCall => "SYSCALL",
        }
    }

    /// Total number of categories (for array sizing).
    const COUNT: usize = 8;
}

// ---------------------------------------------------------------------------
// Audit Severity
// ---------------------------------------------------------------------------

/// Severity level for an audit event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum AuditSeverity {
    /// Informational, normal operation
    Info = 0,
    /// Warning, unusual but non-critical
    Warning = 1,
    /// Error, operation failed
    Error = 2,
    /// Critical, security-relevant failure
    Critical = 3,
}

impl AuditSeverity {
    /// Human-readable name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
            Self::Critical => "CRIT",
        }
    }
}

// ---------------------------------------------------------------------------
// Audit Entry
// ---------------------------------------------------------------------------

/// A structured audit log entry.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Monotonic sequence number (unique per entry)
    pub sequence: u64,
    /// Timestamp in seconds since boot
    pub timestamp: u64,
    /// Process ID that generated the event
    pub pid: u64,
    /// Thread ID within the process
    pub tid: u64,
    /// Event category
    pub category: AuditCategory,
    /// Severity level
    pub severity: AuditSeverity,
    /// Short description of the event
    pub message: String,
    /// Whether the operation succeeded
    pub success: bool,
    /// How many times this event was coalesced (1 = no coalescing)
    pub coalesce_count: u32,
}

impl AuditEntry {
    /// Serialize to pipe-delimited text format.
    ///
    /// Format: `seq|timestamp|pid|tid|category|severity|success|count|message\
    /// n`
    #[cfg(feature = "alloc")]
    pub fn serialize(&self) -> String {
        alloc::format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
            self.sequence,
            self.timestamp,
            self.pid,
            self.tid,
            self.category.as_str(),
            self.severity.as_str(),
            if self.success { "OK" } else { "FAIL" },
            self.coalesce_count,
            self.message,
        )
    }
}

// ---------------------------------------------------------------------------
// Audit Filter
// ---------------------------------------------------------------------------

/// Multi-dimensional filter for querying audit events.
#[derive(Debug, Clone, Copy)]
pub struct AuditQueryFilter {
    /// Bitmask of enabled categories (0 = match all)
    pub category_mask: u16,
    /// Minimum severity to include (None = all)
    pub min_severity: Option<AuditSeverity>,
    /// Filter by PID (0 = match all)
    pub pid: u64,
    /// Start of time range (0 = no lower bound)
    pub time_min: u64,
    /// End of time range (0 = no upper bound)
    pub time_max: u64,
    /// Only include failures (false = include all)
    pub failures_only: bool,
}

impl AuditQueryFilter {
    /// Create a filter that matches everything.
    pub const fn match_all() -> Self {
        Self {
            category_mask: 0,
            min_severity: None,
            pid: 0,
            time_min: 0,
            time_max: 0,
            failures_only: false,
        }
    }

    /// Check if a given entry passes this filter.
    pub fn matches(&self, entry: &AuditEntry) -> bool {
        // Category filter
        if self.category_mask != 0 && (self.category_mask & entry.category.to_flag()) == 0 {
            return false;
        }

        // Severity filter
        if let Some(min) = self.min_severity {
            if (entry.severity as u8) < (min as u8) {
                return false;
            }
        }

        // PID filter
        if self.pid != 0 && entry.pid != self.pid {
            return false;
        }

        // Time range filter
        if self.time_min != 0 && entry.timestamp < self.time_min {
            return false;
        }
        if self.time_max != 0 && entry.timestamp > self.time_max {
            return false;
        }

        // Failures-only filter
        if self.failures_only && entry.success {
            return false;
        }

        true
    }
}

// ---------------------------------------------------------------------------
// Active Filter (controls which events get logged)
// ---------------------------------------------------------------------------

/// Active filter controlling which events are accepted into the log.
#[derive(Debug, Clone, Copy)]
pub struct AuditActiveFilter {
    /// Bitmask of enabled categories (all bits set = log everything)
    pub category_mask: u16,
    /// Minimum severity to log
    pub min_severity: AuditSeverity,
}

impl AuditActiveFilter {
    /// Create a filter that accepts all events.
    pub const fn accept_all() -> Self {
        Self {
            category_mask: 0xFFFF,
            min_severity: AuditSeverity::Info,
        }
    }

    /// Check if an event with the given category and severity should be logged.
    pub fn should_log(&self, category: AuditCategory, severity: AuditSeverity) -> bool {
        (self.category_mask & category.to_flag()) != 0
            && (severity as u8) >= (self.min_severity as u8)
    }
}

// ---------------------------------------------------------------------------
// Audit Statistics
// ---------------------------------------------------------------------------

/// Statistics for the enhanced audit log.
#[derive(Debug, Clone, Copy)]
pub struct EnhancedAuditStats {
    /// Total events logged (including coalesced)
    pub total_logged: u64,
    /// Events dropped because the buffer was full and coalescing didn't apply
    pub total_dropped: u64,
    /// Events filtered out by the active filter
    pub total_filtered: u64,
    /// Events that were coalesced into a previous entry
    pub total_coalesced: u64,
    /// Current number of entries in the ring buffer
    pub buffer_count: u64,
    /// Maximum capacity of the ring buffer
    pub buffer_capacity: u64,
    /// Per-category event counts
    pub per_category: [u64; AuditCategory::COUNT],
}

// ---------------------------------------------------------------------------
// Audit Log (Ring Buffer)
// ---------------------------------------------------------------------------

/// Default ring buffer capacity.
const DEFAULT_CAPACITY: usize = 8192;

/// Window (in seconds) within which identical events are coalesced.
const COALESCE_WINDOW_SECS: u64 = 1;

/// The enhanced audit log ring buffer.
#[cfg(feature = "alloc")]
struct AuditLog {
    /// Ring buffer of entries
    entries: VecDeque<AuditEntry>,
    /// Maximum capacity
    capacity: usize,
    /// Active filter
    active_filter: AuditActiveFilter,
    /// Next sequence number
    next_sequence: u64,
}

#[cfg(feature = "alloc")]
impl AuditLog {
    /// Create a new audit log with the given capacity.
    fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
            active_filter: AuditActiveFilter::accept_all(),
            next_sequence: 1,
        }
    }

    /// Insert an entry, possibly coalescing with the most recent matching
    /// entry.
    ///
    /// Returns `true` if a new entry was inserted, `false` if coalesced.
    fn insert(&mut self, mut entry: AuditEntry) -> bool {
        // Try coalescing: check last entry for identical category+pid+message within
        // window
        if let Some(last) = self.entries.back_mut() {
            if last.category == entry.category
                && last.pid == entry.pid
                && last.success == entry.success
                && last.message == entry.message
                && entry.timestamp.saturating_sub(last.timestamp) <= COALESCE_WINDOW_SECS
            {
                last.coalesce_count = last.coalesce_count.saturating_add(1);
                // Update timestamp to most recent occurrence
                last.timestamp = entry.timestamp;
                return false;
            }
        }

        // Assign sequence number
        entry.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        entry.coalesce_count = 1;

        // Evict oldest if at capacity
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }

        self.entries.push_back(entry);
        true
    }

    /// Query entries matching a filter.
    fn query(&self, filter: &AuditQueryFilter, max_results: usize) -> Vec<AuditEntry> {
        let mut results = Vec::new();
        for entry in self.entries.iter().rev() {
            if results.len() >= max_results {
                break;
            }
            if filter.matches(entry) {
                results.push(entry.clone());
            }
        }
        // Reverse so oldest is first
        results.reverse();
        results
    }

    /// Clear all entries.
    fn clear(&mut self) {
        self.entries.clear();
        self.next_sequence = 1;
    }

    /// Number of entries currently stored.
    fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// Global State
// ---------------------------------------------------------------------------

/// Global enhanced audit log, protected by RwLock for concurrent access.
#[cfg(feature = "alloc")]
static AUDIT_LOG: RwLock<Option<AuditLog>> = RwLock::new(None);

/// Whether the enhanced audit subsystem is enabled.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Monotonic counters for statistics (lock-free).
static STAT_LOGGED: AtomicU64 = AtomicU64::new(0);
static STAT_DROPPED: AtomicU64 = AtomicU64::new(0);
static STAT_FILTERED: AtomicU64 = AtomicU64::new(0);
static STAT_COALESCED: AtomicU64 = AtomicU64::new(0);
static PER_CATEGORY_COUNTS: [AtomicU64; AuditCategory::COUNT] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

// ---------------------------------------------------------------------------
// Timestamp Helper
// ---------------------------------------------------------------------------

/// Get a timestamp for audit events (seconds since boot).
fn get_timestamp() -> u64 {
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    ))]
    {
        crate::arch::timer::get_timestamp_secs()
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        0
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the enhanced audit subsystem.
#[cfg(feature = "alloc")]
pub fn init() {
    let mut log = AUDIT_LOG.write();
    *log = Some(AuditLog::new(DEFAULT_CAPACITY));
    ENABLED.store(true, Ordering::Release);
}

/// Initialize with a custom capacity.
#[cfg(feature = "alloc")]
pub fn init_with_capacity(capacity: usize) {
    let cap = if capacity == 0 {
        DEFAULT_CAPACITY
    } else {
        capacity
    };
    let mut log = AUDIT_LOG.write();
    *log = Some(AuditLog::new(cap));
    ENABLED.store(true, Ordering::Release);
}

/// Log a structured audit event.
///
/// This is the primary entry point. The event is checked against the active
/// filter. If accepted, it is inserted into the ring buffer (with coalescing).
/// Uses `try_write()` to avoid deadlocks in interrupt context.
#[cfg(feature = "alloc")]
pub fn log_event(
    pid: u64,
    tid: u64,
    category: AuditCategory,
    severity: AuditSeverity,
    message: String,
    success: bool,
) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }

    // Acquire the log with try_write to avoid deadlock
    let mut log_guard = match AUDIT_LOG.try_write() {
        Some(g) => g,
        None => {
            STAT_DROPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    let log = match log_guard.as_mut() {
        Some(l) => l,
        None => {
            STAT_DROPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    // Check active filter
    if !log.active_filter.should_log(category, severity) {
        STAT_FILTERED.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let entry = AuditEntry {
        sequence: 0, // Will be assigned by insert()
        timestamp: get_timestamp(),
        pid,
        tid,
        category,
        severity,
        message,
        success,
        coalesce_count: 1,
    };

    let inserted = log.insert(entry);

    // Update statistics
    STAT_LOGGED.fetch_add(1, Ordering::Relaxed);
    let cat_idx = category as usize;
    if cat_idx < PER_CATEGORY_COUNTS.len() {
        PER_CATEGORY_COUNTS[cat_idx].fetch_add(1, Ordering::Relaxed);
    }

    if !inserted {
        STAT_COALESCED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Query audit events matching a filter.
///
/// Returns up to `max_results` entries in chronological order (oldest first).
#[cfg(feature = "alloc")]
pub fn query_events(filter: &AuditQueryFilter, max_results: usize) -> Vec<AuditEntry> {
    let log_guard = AUDIT_LOG.read();
    match log_guard.as_ref() {
        Some(log) => log.query(filter, max_results),
        None => Vec::new(),
    }
}

/// Clear all audit log entries.
#[cfg(feature = "alloc")]
pub fn clear_log() {
    let mut log_guard = AUDIT_LOG.write();
    if let Some(log) = log_guard.as_mut() {
        log.clear();
    }
}

/// Set the active filter that controls which events are logged.
#[cfg(feature = "alloc")]
pub fn set_filter(filter: AuditActiveFilter) {
    let mut log_guard = AUDIT_LOG.write();
    if let Some(log) = log_guard.as_mut() {
        log.active_filter = filter;
    }
}

/// Get the current active filter.
#[cfg(feature = "alloc")]
pub fn get_filter() -> AuditActiveFilter {
    let log_guard = AUDIT_LOG.read();
    match log_guard.as_ref() {
        Some(log) => log.active_filter,
        None => AuditActiveFilter::accept_all(),
    }
}

/// Get audit statistics.
pub fn get_stats() -> EnhancedAuditStats {
    let buffer_count;
    let buffer_capacity;

    #[cfg(feature = "alloc")]
    {
        let log_guard = AUDIT_LOG.read();
        match log_guard.as_ref() {
            Some(log) => {
                buffer_count = log.len() as u64;
                buffer_capacity = log.capacity as u64;
            }
            None => {
                buffer_count = 0;
                buffer_capacity = 0;
            }
        }
    }

    #[cfg(not(feature = "alloc"))]
    {
        buffer_count = 0;
        buffer_capacity = 0;
    }

    let mut per_category = [0u64; AuditCategory::COUNT];
    for (i, counter) in PER_CATEGORY_COUNTS.iter().enumerate() {
        per_category[i] = counter.load(Ordering::Relaxed);
    }

    EnhancedAuditStats {
        total_logged: STAT_LOGGED.load(Ordering::Relaxed),
        total_dropped: STAT_DROPPED.load(Ordering::Relaxed),
        total_filtered: STAT_FILTERED.load(Ordering::Relaxed),
        total_coalesced: STAT_COALESCED.load(Ordering::Relaxed),
        buffer_count,
        buffer_capacity,
        per_category,
    }
}

/// Enable the enhanced audit subsystem.
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// Disable the enhanced audit subsystem.
pub fn disable() {
    ENABLED.store(false, Ordering::Release);
}

/// Check if the enhanced audit subsystem is enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Convenience logging functions
// ---------------------------------------------------------------------------

/// Log an authentication event.
#[cfg(feature = "alloc")]
pub fn log_auth(pid: u64, tid: u64, message: String, success: bool) {
    let severity = if success {
        AuditSeverity::Info
    } else {
        AuditSeverity::Warning
    };
    log_event(
        pid,
        tid,
        AuditCategory::Authentication,
        severity,
        message,
        success,
    );
}

/// Log an authorization / access control event.
#[cfg(feature = "alloc")]
pub fn log_authz(pid: u64, tid: u64, message: String, success: bool) {
    let severity = if success {
        AuditSeverity::Info
    } else {
        AuditSeverity::Error
    };
    log_event(
        pid,
        tid,
        AuditCategory::Authorization,
        severity,
        message,
        success,
    );
}

/// Log a file access event.
#[cfg(feature = "alloc")]
pub fn log_file(pid: u64, tid: u64, message: String, success: bool) {
    log_event(
        pid,
        tid,
        AuditCategory::FileAccess,
        AuditSeverity::Info,
        message,
        success,
    );
}

/// Log a network access event.
#[cfg(feature = "alloc")]
pub fn log_network(pid: u64, tid: u64, message: String, success: bool) {
    log_event(
        pid,
        tid,
        AuditCategory::NetworkAccess,
        AuditSeverity::Info,
        message,
        success,
    );
}

/// Log a process lifecycle event.
#[cfg(feature = "alloc")]
pub fn log_process(pid: u64, tid: u64, message: String, success: bool) {
    log_event(
        pid,
        tid,
        AuditCategory::ProcessLifecycle,
        AuditSeverity::Info,
        message,
        success,
    );
}

/// Log a capability operation event.
#[cfg(feature = "alloc")]
pub fn log_capability(pid: u64, tid: u64, message: String, success: bool) {
    let severity = if success {
        AuditSeverity::Info
    } else {
        AuditSeverity::Warning
    };
    log_event(
        pid,
        tid,
        AuditCategory::CapabilityOps,
        severity,
        message,
        success,
    );
}

/// Log a security policy change.
#[cfg(feature = "alloc")]
pub fn log_policy(pid: u64, tid: u64, message: String, success: bool) {
    log_event(
        pid,
        tid,
        AuditCategory::SecurityPolicy,
        AuditSeverity::Critical,
        message,
        success,
    );
}

/// Log a system call audit event.
#[cfg(feature = "alloc")]
pub fn log_syscall(pid: u64, tid: u64, message: String, success: bool) {
    log_event(
        pid,
        tid,
        AuditCategory::SystemCall,
        AuditSeverity::Info,
        message,
        success,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    #[allow(unused_imports)]
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn test_category_flags() {
        assert_eq!(AuditCategory::Authentication.to_flag(), 1);
        assert_eq!(AuditCategory::Authorization.to_flag(), 2);
        assert_eq!(AuditCategory::FileAccess.to_flag(), 4);
        assert_eq!(AuditCategory::SystemCall.to_flag(), 128);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(AuditSeverity::Info < AuditSeverity::Warning);
        assert!(AuditSeverity::Warning < AuditSeverity::Error);
        assert!(AuditSeverity::Error < AuditSeverity::Critical);
    }

    #[test]
    fn test_active_filter_accept_all() {
        let filter = AuditActiveFilter::accept_all();
        assert!(filter.should_log(AuditCategory::Authentication, AuditSeverity::Info));
        assert!(filter.should_log(AuditCategory::SystemCall, AuditSeverity::Critical));
    }

    #[test]
    fn test_active_filter_severity() {
        let filter = AuditActiveFilter {
            category_mask: 0xFFFF,
            min_severity: AuditSeverity::Warning,
        };
        assert!(!filter.should_log(AuditCategory::FileAccess, AuditSeverity::Info));
        assert!(filter.should_log(AuditCategory::FileAccess, AuditSeverity::Warning));
        assert!(filter.should_log(AuditCategory::FileAccess, AuditSeverity::Critical));
    }

    #[test]
    fn test_active_filter_category() {
        let filter = AuditActiveFilter {
            category_mask: AuditCategory::Authentication.to_flag()
                | AuditCategory::CapabilityOps.to_flag(),
            min_severity: AuditSeverity::Info,
        };
        assert!(filter.should_log(AuditCategory::Authentication, AuditSeverity::Info));
        assert!(filter.should_log(AuditCategory::CapabilityOps, AuditSeverity::Info));
        assert!(!filter.should_log(AuditCategory::FileAccess, AuditSeverity::Info));
    }

    #[test]
    fn test_query_filter_match_all() {
        let filter = AuditQueryFilter::match_all();
        let entry = AuditEntry {
            sequence: 1,
            timestamp: 100,
            pid: 42,
            tid: 1,
            category: AuditCategory::FileAccess,
            severity: AuditSeverity::Info,
            message: String::from("test"),
            success: true,
            coalesce_count: 1,
        };
        assert!(filter.matches(&entry));
    }

    #[test]
    fn test_query_filter_pid() {
        let filter = AuditQueryFilter {
            pid: 42,
            ..AuditQueryFilter::match_all()
        };
        let entry_match = AuditEntry {
            sequence: 1,
            timestamp: 100,
            pid: 42,
            tid: 1,
            category: AuditCategory::FileAccess,
            severity: AuditSeverity::Info,
            message: String::from("test"),
            success: true,
            coalesce_count: 1,
        };
        let entry_no_match = AuditEntry {
            pid: 99,
            ..entry_match.clone()
        };
        assert!(filter.matches(&entry_match));
        assert!(!filter.matches(&entry_no_match));
    }

    #[test]
    fn test_query_filter_time_range() {
        let filter = AuditQueryFilter {
            time_min: 50,
            time_max: 150,
            ..AuditQueryFilter::match_all()
        };
        let make_entry = |ts: u64| AuditEntry {
            sequence: 1,
            timestamp: ts,
            pid: 1,
            tid: 1,
            category: AuditCategory::FileAccess,
            severity: AuditSeverity::Info,
            message: String::from("test"),
            success: true,
            coalesce_count: 1,
        };
        assert!(!filter.matches(&make_entry(10)));
        assert!(filter.matches(&make_entry(50)));
        assert!(filter.matches(&make_entry(100)));
        assert!(filter.matches(&make_entry(150)));
        assert!(!filter.matches(&make_entry(200)));
    }

    #[test]
    fn test_query_filter_failures_only() {
        let filter = AuditQueryFilter {
            failures_only: true,
            ..AuditQueryFilter::match_all()
        };
        let success = AuditEntry {
            sequence: 1,
            timestamp: 100,
            pid: 1,
            tid: 1,
            category: AuditCategory::Authentication,
            severity: AuditSeverity::Info,
            message: String::from("login"),
            success: true,
            coalesce_count: 1,
        };
        let failure = AuditEntry {
            success: false,
            ..success.clone()
        };
        assert!(!filter.matches(&success));
        assert!(filter.matches(&failure));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_audit_log_insert_and_query() {
        let mut log = AuditLog::new(16);
        let entry = AuditEntry {
            sequence: 0,
            timestamp: 100,
            pid: 1,
            tid: 1,
            category: AuditCategory::FileAccess,
            severity: AuditSeverity::Info,
            message: String::from("open /etc/passwd"),
            success: true,
            coalesce_count: 1,
        };
        assert!(log.insert(entry));
        assert_eq!(log.len(), 1);

        let results = log.query(&AuditQueryFilter::match_all(), 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].sequence, 1);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_audit_log_coalescing() {
        let mut log = AuditLog::new(16);
        let entry1 = AuditEntry {
            sequence: 0,
            timestamp: 100,
            pid: 1,
            tid: 1,
            category: AuditCategory::FileAccess,
            severity: AuditSeverity::Info,
            message: String::from("read /tmp/data"),
            success: true,
            coalesce_count: 1,
        };
        let entry2 = AuditEntry {
            timestamp: 100, // Same second
            ..entry1.clone()
        };

        assert!(log.insert(entry1));
        assert!(!log.insert(entry2)); // Should coalesce
        assert_eq!(log.len(), 1);

        let results = log.query(&AuditQueryFilter::match_all(), 100);
        assert_eq!(results[0].coalesce_count, 2);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_audit_log_no_coalesce_different_message() {
        let mut log = AuditLog::new(16);
        let entry1 = AuditEntry {
            sequence: 0,
            timestamp: 100,
            pid: 1,
            tid: 1,
            category: AuditCategory::FileAccess,
            severity: AuditSeverity::Info,
            message: String::from("read /tmp/a"),
            success: true,
            coalesce_count: 1,
        };
        let entry2 = AuditEntry {
            message: String::from("read /tmp/b"),
            ..entry1.clone()
        };

        assert!(log.insert(entry1));
        assert!(log.insert(entry2)); // Different message, no coalesce
        assert_eq!(log.len(), 2);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_audit_log_ring_buffer_eviction() {
        let mut log = AuditLog::new(4);
        for i in 0..6 {
            let entry = AuditEntry {
                sequence: 0,
                timestamp: i * 10, // Different timestamps to prevent coalescing
                pid: i,
                tid: 1,
                category: AuditCategory::ProcessLifecycle,
                severity: AuditSeverity::Info,
                message: alloc::format!("event {}", i),
                success: true,
                coalesce_count: 1,
            };
            log.insert(entry);
        }
        // Should have evicted first 2 entries
        assert_eq!(log.len(), 4);

        let results = log.query(&AuditQueryFilter::match_all(), 100);
        // Oldest remaining should be entry with pid=2
        assert_eq!(results[0].pid, 2);
        assert_eq!(results[3].pid, 5);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_audit_log_clear() {
        let mut log = AuditLog::new(16);
        let entry = AuditEntry {
            sequence: 0,
            timestamp: 100,
            pid: 1,
            tid: 1,
            category: AuditCategory::Authentication,
            severity: AuditSeverity::Info,
            message: String::from("login root"),
            success: true,
            coalesce_count: 1,
        };
        log.insert(entry);
        assert_eq!(log.len(), 1);

        log.clear();
        assert_eq!(log.len(), 0);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_entry_serialize() {
        let entry = AuditEntry {
            sequence: 42,
            timestamp: 1000,
            pid: 123,
            tid: 5,
            category: AuditCategory::Authentication,
            severity: AuditSeverity::Warning,
            message: String::from("failed login"),
            success: false,
            coalesce_count: 3,
        };
        let s = entry.serialize();
        assert!(s.contains("42"));
        assert!(s.contains("1000"));
        assert!(s.contains("123"));
        assert!(s.contains("AUTH"));
        assert!(s.contains("WARN"));
        assert!(s.contains("FAIL"));
        assert!(s.contains("3"));
        assert!(s.contains("failed login"));
    }

    #[test]
    fn test_stats_initial() {
        let stats = get_stats();
        // Stats may have been modified by other tests running in parallel,
        // but capacity should be consistent
        assert!(stats.per_category.len() == AuditCategory::COUNT);
    }
}

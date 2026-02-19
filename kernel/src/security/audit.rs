//! Security audit framework
//!
//! Tracks and logs security-relevant events for compliance and forensics.
//!
//! # Features
//!
//! - Structured audit events with timestamps, PIDs, UIDs, and action details
//! - Configurable event filtering via bitmask
//! - Persistent storage to VFS-backed audit log (`/var/log/audit.log`)
//! - Serialization to pipe-delimited text format
//! - Convenience functions for syscall, capability, and MAC audit logging
//! - Real-time alert callbacks for critical security events
//! - Statistics tracking

use alloc::{format, string::String};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use spin::Mutex;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Audit Event Types
// ---------------------------------------------------------------------------

/// Audit event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuditEventType {
    /// Process creation
    ProcessCreate = 0,
    /// Process termination
    ProcessExit = 1,
    /// File access
    FileAccess = 2,
    /// Network connection
    NetworkConnect = 3,
    /// Authentication attempt
    AuthAttempt = 4,
    /// Permission denied
    PermissionDenied = 5,
    /// System call
    Syscall = 6,
    /// Capability operation
    CapabilityOp = 7,
    /// MAC policy decision
    MacDecision = 8,
    /// Privilege escalation attempt
    PrivilegeEscalation = 9,
    /// Security configuration change
    SecurityConfigChange = 10,
}

impl AuditEventType {
    /// Convert to bitmask flag for filtering.
    pub fn to_flag(self) -> u32 {
        1u32 << (self as u8)
    }

    /// Get a human-readable name for this event type.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcessCreate => "PROCESS_CREATE",
            Self::ProcessExit => "PROCESS_EXIT",
            Self::FileAccess => "FILE_ACCESS",
            Self::NetworkConnect => "NETWORK_CONNECT",
            Self::AuthAttempt => "AUTH_ATTEMPT",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::Syscall => "SYSCALL",
            Self::CapabilityOp => "CAPABILITY_OP",
            Self::MacDecision => "MAC_DECISION",
            Self::PrivilegeEscalation => "PRIVILEGE_ESCALATION",
            Self::SecurityConfigChange => "SECURITY_CONFIG_CHANGE",
        }
    }
}

// ---------------------------------------------------------------------------
// Audit Action (structured action descriptions)
// ---------------------------------------------------------------------------

/// Structured audit action for detailed event logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    /// Process created (with parent PID as data)
    Create,
    /// Process exited (with exit code as data)
    Exit,
    /// Read access
    Read,
    /// Write access
    Write,
    /// Execute access
    Execute,
    /// Delete operation
    Delete,
    /// Login attempt
    Login,
    /// Logout
    Logout,
    /// Capability created
    CapCreate,
    /// Capability revoked
    CapRevoke,
    /// Capability derived
    CapDerive,
    /// MAC allow decision
    MacAllow,
    /// MAC deny decision
    MacDeny,
    /// Privilege escalation
    Escalate,
    /// Configuration change
    ConfigChange,
    /// Generic operation
    Other,
}

impl AuditAction {
    /// Get a human-readable name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "CREATE",
            Self::Exit => "EXIT",
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::Execute => "EXECUTE",
            Self::Delete => "DELETE",
            Self::Login => "LOGIN",
            Self::Logout => "LOGOUT",
            Self::CapCreate => "CAP_CREATE",
            Self::CapRevoke => "CAP_REVOKE",
            Self::CapDerive => "CAP_DERIVE",
            Self::MacAllow => "MAC_ALLOW",
            Self::MacDeny => "MAC_DENY",
            Self::Escalate => "ESCALATE",
            Self::ConfigChange => "CONFIG_CHANGE",
            Self::Other => "OTHER",
        }
    }
}

// ---------------------------------------------------------------------------
// Audit Event (structured)
// ---------------------------------------------------------------------------

/// Structured audit event record.
///
/// Fields are serialized in pipe-delimited format for persistent storage.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// Event type
    pub event_type: AuditEventType,
    /// Timestamp (seconds since boot)
    pub timestamp: u64,
    /// Process ID
    pub pid: u64,
    /// User ID
    pub uid: u32,
    /// Structured action
    pub action: AuditAction,
    /// Target resource or object name
    pub target: String,
    /// Whether the operation succeeded
    pub result: bool,
    /// Extra data / details
    pub extra_data: String,
}

impl AuditEvent {
    /// Create a new structured audit event with automatic timestamp.
    pub fn new(
        event_type: AuditEventType,
        pid: u64,
        uid: u32,
        action: AuditAction,
        target: &str,
        result: bool,
        extra_data: &str,
    ) -> Self {
        Self {
            event_type,
            timestamp: get_audit_timestamp(),
            pid,
            uid,
            action,
            target: String::from(target),
            result,
            extra_data: String::from(extra_data),
        }
    }

    /// Create from legacy parameters (backward compat with old
    /// AuditEvent::new).
    pub fn from_legacy(
        event_type: AuditEventType,
        pid: u64,
        uid: u32,
        result_code: i32,
        data: u64,
    ) -> Self {
        let action = match event_type {
            AuditEventType::ProcessCreate => AuditAction::Create,
            AuditEventType::ProcessExit => AuditAction::Exit,
            AuditEventType::FileAccess => AuditAction::Read,
            AuditEventType::AuthAttempt => AuditAction::Login,
            AuditEventType::PermissionDenied => AuditAction::MacDeny,
            AuditEventType::CapabilityOp => AuditAction::CapCreate,
            _ => AuditAction::Other,
        };

        Self {
            event_type,
            timestamp: get_audit_timestamp(),
            pid,
            uid,
            action,
            target: format!("data:{:#x}", data),
            result: result_code == 0,
            extra_data: format!("result_code:{}", result_code),
        }
    }

    /// Serialize to pipe-delimited text format.
    ///
    /// Format: `timestamp|event_type|pid|uid|action|target|result|extra_data\n`
    pub fn serialize(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}\n",
            self.timestamp,
            self.event_type.as_str(),
            self.pid,
            self.uid,
            self.action.as_str(),
            self.target,
            if self.result { "OK" } else { "FAIL" },
            self.extra_data,
        )
    }
}

// ---------------------------------------------------------------------------
// Audit Filter
// ---------------------------------------------------------------------------

/// Configurable audit event filter using a bitmask.
///
/// Each event type corresponds to a bit. If the bit is set, events of
/// that type will be logged. If 0, all events are filtered out.
#[derive(Debug, Clone, Copy)]
pub struct AuditFilter {
    /// Bitmask of enabled event types
    pub enabled_types: u32,
}

impl AuditFilter {
    /// Create a filter that allows all event types.
    pub const fn allow_all() -> Self {
        Self {
            enabled_types: 0xFFFF_FFFF,
        }
    }

    /// Create a filter that blocks all event types.
    pub const fn deny_all() -> Self {
        Self { enabled_types: 0 }
    }

    /// Create a filter from an explicit bitmask.
    pub const fn from_mask(mask: u32) -> Self {
        Self {
            enabled_types: mask,
        }
    }

    /// Check if a given event type is enabled.
    pub fn is_enabled(&self, event_type: AuditEventType) -> bool {
        (self.enabled_types & event_type.to_flag()) != 0
    }

    /// Enable an event type.
    pub fn enable(&mut self, event_type: AuditEventType) {
        self.enabled_types |= event_type.to_flag();
    }

    /// Disable an event type.
    pub fn disable(&mut self, event_type: AuditEventType) {
        self.enabled_types &= !event_type.to_flag();
    }
}

// ---------------------------------------------------------------------------
// Alert Callback System
// ---------------------------------------------------------------------------

/// Trait for real-time audit alert handlers.
///
/// Implementations receive critical audit events as they occur. This
/// enables immediate response to security-relevant events such as
/// authentication failures and privilege escalation attempts.
pub trait AlertCallback: Send + Sync {
    /// Called when a critical audit event is generated.
    fn on_alert(&self, event: &AuditEvent);
}

/// Maximum number of registered alert callbacks.
const MAX_ALERT_CALLBACKS: usize = 8;

// ---------------------------------------------------------------------------
// Audit Statistics
// ---------------------------------------------------------------------------

/// Audit statistics tracking.
struct AuditStats {
    /// Total events logged
    total_events: AtomicU64,
    /// Events filtered out
    filtered_events: AtomicU64,
    /// Events written to persistent storage
    persisted_events: AtomicU64,
    /// Alerts triggered
    alerts_triggered: AtomicU64,
    /// Per-type event counts (indexed by AuditEventType discriminant)
    per_type_counts: [AtomicU64; 16],
}

impl AuditStats {
    const fn new() -> Self {
        Self {
            total_events: AtomicU64::new(0),
            filtered_events: AtomicU64::new(0),
            persisted_events: AtomicU64::new(0),
            alerts_triggered: AtomicU64::new(0),
            per_type_counts: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        }
    }

    fn record_event(&self, event_type: AuditEventType) {
        self.total_events.fetch_add(1, Ordering::Relaxed);
        let idx = event_type as usize;
        if idx < self.per_type_counts.len() {
            self.per_type_counts[idx].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_filtered(&self) {
        self.filtered_events.fetch_add(1, Ordering::Relaxed);
    }

    fn record_persisted(&self) {
        self.persisted_events.fetch_add(1, Ordering::Relaxed);
    }

    fn record_alert(&self) {
        self.alerts_triggered.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Global State
// ---------------------------------------------------------------------------

/// Maximum audit log size (circular buffer).
const MAX_AUDIT_LOG: usize = 4096;

/// Audit log buffer (circular), protected by a Mutex.
static AUDIT_LOG: Mutex<[Option<AuditEvent>; MAX_AUDIT_LOG]> =
    Mutex::new([const { None }; MAX_AUDIT_LOG]);
static AUDIT_HEAD: AtomicUsize = AtomicUsize::new(0);
static AUDIT_COUNT: AtomicUsize = AtomicUsize::new(0);
static AUDIT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Event filter (protected by Mutex since AuditFilter is not atomic).
static AUDIT_FILTER: Mutex<AuditFilter> = Mutex::new(AuditFilter::allow_all());

/// Alert callbacks (protected by Mutex).
static ALERT_CALLBACKS: Mutex<[Option<&'static dyn AlertCallback>; MAX_ALERT_CALLBACKS]> =
    Mutex::new([None; MAX_ALERT_CALLBACKS]);

/// Global statistics.
static AUDIT_STATS: AuditStats = AuditStats::new();

/// Path for the persistent audit log file.
const AUDIT_LOG_PATH: &str = "/var/log/audit.log";

// ---------------------------------------------------------------------------
// Timestamp Helper
// ---------------------------------------------------------------------------

/// Get a timestamp for audit events.
fn get_audit_timestamp() -> u64 {
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
// Core Logging
// ---------------------------------------------------------------------------

/// Log a structured audit event.
///
/// Events are checked against the active filter. If accepted, they are
/// stored in the circular in-memory buffer and optionally written to
/// persistent storage.
///
/// Uses try_lock() with graceful degradation to avoid deadlocks when called
/// from syscall return paths or interrupt contexts. If locks are held,
/// the event is skipped rather than blocking.
pub fn log_event(event: AuditEvent) {
    if !AUDIT_ENABLED.load(Ordering::Acquire) {
        return;
    }

    // Try to acquire filter lock without blocking. If lock contention occurs,
    // skip this event to avoid deadlock during syscall return path or
    // interrupt context. This is safe because audit logging is best-effort.
    let filter = match AUDIT_FILTER.try_lock() {
        Some(f) => f,
        None => {
            // Lock contention - skip event to avoid deadlock
            AUDIT_STATS.record_filtered();
            return;
        }
    };

    if !filter.is_enabled(event.event_type) {
        AUDIT_STATS.record_filtered();
        return;
    }
    drop(filter); // Release early

    // Record statistics
    AUDIT_STATS.record_event(event.event_type);

    // Check for critical events and fire alerts
    if is_critical_event(&event) {
        fire_alerts(&event);
    }

    // Write to persistent storage (best-effort, don't fail if VFS not ready)
    persist_event(&event);

    // Store in circular buffer with try_lock. If the buffer lock is held,
    // the event is dropped (acceptable for non-critical events).
    if let Some(mut log) = AUDIT_LOG.try_lock() {
        let head = AUDIT_HEAD.load(Ordering::Relaxed);
        log[head] = Some(event);
        AUDIT_HEAD.store((head + 1) % MAX_AUDIT_LOG, Ordering::Relaxed);

        let count = AUDIT_COUNT.load(Ordering::Relaxed);
        if count < MAX_AUDIT_LOG {
            AUDIT_COUNT.store(count + 1, Ordering::Relaxed);
        }
    }
    // If try_lock fails, event is dropped (graceful degradation)
}

/// Check if an event is critical enough to trigger alerts.
fn is_critical_event(event: &AuditEvent) -> bool {
    matches!(
        event.event_type,
        AuditEventType::PermissionDenied
            | AuditEventType::PrivilegeEscalation
            | AuditEventType::SecurityConfigChange
    ) || (event.event_type == AuditEventType::AuthAttempt && !event.result)
}

/// Fire all registered alert callbacks for a critical event.
fn fire_alerts(event: &AuditEvent) {
    let callbacks = ALERT_CALLBACKS.lock();
    for cb in callbacks.iter().flatten() {
        cb.on_alert(event);
        AUDIT_STATS.record_alert();
    }
}

/// Write an event to persistent VFS-backed audit log (best-effort).
fn persist_event(event: &AuditEvent) {
    let serialized = event.serialize();
    let bytes = serialized.as_bytes();

    // VFS is only available on bare-metal; skip persistence in host tests.
    #[cfg(not(test))]
    {
        // Try to append to the audit log file; ignore errors if VFS is not
        // mounted or the path does not exist yet.
        if crate::fs::append_file(AUDIT_LOG_PATH, bytes).is_ok() {
            AUDIT_STATS.record_persisted();
        }
    }
    #[cfg(test)]
    let _ = bytes;
}

// ---------------------------------------------------------------------------
// Convenience Logging Functions
// ---------------------------------------------------------------------------

/// Log a process creation event.
pub fn log_process_create(pid: u64, uid: u32, result: i32) {
    log_event(AuditEvent::new(
        AuditEventType::ProcessCreate,
        pid,
        uid,
        AuditAction::Create,
        "process",
        result == 0,
        "",
    ));
}

/// Log a process exit event.
pub fn log_process_exit(pid: u64, exit_code: i32) {
    log_event(AuditEvent::new(
        AuditEventType::ProcessExit,
        pid,
        0,
        AuditAction::Exit,
        "process",
        true,
        &format!("exit_code:{}", exit_code),
    ));
}

/// Log a file access event.
pub fn log_file_access(pid: u64, uid: u32, path_hash: u64, access_type: u32) {
    let action = match access_type {
        0 => AuditAction::Read,
        1 => AuditAction::Write,
        2 => AuditAction::Execute,
        _ => AuditAction::Other,
    };
    log_event(AuditEvent::new(
        AuditEventType::FileAccess,
        pid,
        uid,
        action,
        &format!("file:{:#x}", path_hash),
        true,
        "",
    ));
}

/// Log a permission denial event.
pub fn log_permission_denied(pid: u64, uid: u32, target: &str) {
    log_event(AuditEvent::new(
        AuditEventType::PermissionDenied,
        pid,
        uid,
        AuditAction::MacDeny,
        target,
        false,
        "denied",
    ));
}

/// Log a capability operation (create, revoke, derive).
pub fn log_capability_op(pid: u64, cap_id: u64, result: i32) {
    log_event(AuditEvent::new(
        AuditEventType::CapabilityOp,
        pid,
        0,
        AuditAction::CapCreate,
        &format!("cap:{:#x}", cap_id),
        result == 0,
        "",
    ));
}

/// Log a system call event.
///
/// Called from the syscall dispatch path.
pub fn log_syscall(pid: u64, uid: u32, syscall_nr: usize, result: bool) {
    log_event(AuditEvent::new(
        AuditEventType::Syscall,
        pid,
        uid,
        AuditAction::Other,
        &format!("syscall:{}", syscall_nr),
        result,
        "",
    ));
}

/// Log a capability operation with a specific action.
pub fn log_capability(pid: u64, cap_id: u64, action: AuditAction, result: bool) {
    log_event(AuditEvent::new(
        AuditEventType::CapabilityOp,
        pid,
        0,
        action,
        &format!("cap:{:#x}", cap_id),
        result,
        "",
    ));
}

/// Log a MAC policy decision.
pub fn log_mac_decision(
    pid: u64,
    uid: u32,
    source_type: &str,
    target_type: &str,
    access: &str,
    allowed: bool,
) {
    let action = if allowed {
        AuditAction::MacAllow
    } else {
        AuditAction::MacDeny
    };
    log_event(AuditEvent::new(
        AuditEventType::MacDecision,
        pid,
        uid,
        action,
        &format!("{}:{}", source_type, target_type),
        allowed,
        access,
    ));
}

/// Log an authentication attempt.
pub fn log_auth_attempt(pid: u64, uid: u32, username: &str, success: bool) {
    log_event(AuditEvent::new(
        AuditEventType::AuthAttempt,
        pid,
        uid,
        AuditAction::Login,
        username,
        success,
        if success {
            "login_success"
        } else {
            "login_failure"
        },
    ));
}

// ---------------------------------------------------------------------------
// Alert Callback Registration
// ---------------------------------------------------------------------------

/// Register a real-time alert callback.
///
/// The callback will be invoked for critical events such as authentication
/// failures, permission denials, and privilege escalation attempts.
///
/// Returns `Ok(())` if registered, or `Err` if all callback slots are full.
pub fn register_alert_callback(callback: &'static dyn AlertCallback) -> Result<(), KernelError> {
    let mut callbacks = ALERT_CALLBACKS.lock();
    for slot in callbacks.iter_mut() {
        if slot.is_none() {
            *slot = Some(callback);
            return Ok(());
        }
    }
    Err(KernelError::ResourceExhausted {
        resource: "audit alert callbacks",
    })
}

// ---------------------------------------------------------------------------
// Filter Management
// ---------------------------------------------------------------------------

/// Set the audit event filter.
pub fn set_filter(filter: AuditFilter) {
    let mut f = AUDIT_FILTER.lock();
    *f = filter;
}

/// Get the current audit event filter.
pub fn get_filter() -> AuditFilter {
    *AUDIT_FILTER.lock()
}

/// Enable a specific event type in the filter.
pub fn enable_event_type(event_type: AuditEventType) {
    let mut f = AUDIT_FILTER.lock();
    f.enable(event_type);
}

/// Disable a specific event type in the filter.
pub fn disable_event_type(event_type: AuditEventType) {
    let mut f = AUDIT_FILTER.lock();
    f.disable(event_type);
}

// ---------------------------------------------------------------------------
// Statistics and Query
// ---------------------------------------------------------------------------

/// Get audit log statistics: (current_count, max_capacity).
pub fn get_stats() -> (usize, usize) {
    (AUDIT_COUNT.load(Ordering::Relaxed), MAX_AUDIT_LOG)
}

/// Get detailed audit statistics.
pub fn get_detailed_stats() -> AuditStatistics {
    AuditStatistics {
        total_events: AUDIT_STATS.total_events.load(Ordering::Relaxed),
        filtered_events: AUDIT_STATS.filtered_events.load(Ordering::Relaxed),
        persisted_events: AUDIT_STATS.persisted_events.load(Ordering::Relaxed),
        alerts_triggered: AUDIT_STATS.alerts_triggered.load(Ordering::Relaxed),
        buffer_count: AUDIT_COUNT.load(Ordering::Relaxed) as u64,
        buffer_capacity: MAX_AUDIT_LOG as u64,
    }
}

/// Detailed audit statistics snapshot.
#[derive(Debug, Clone, Copy)]
pub struct AuditStatistics {
    /// Total events processed
    pub total_events: u64,
    /// Events dropped by filter
    pub filtered_events: u64,
    /// Events written to persistent storage
    pub persisted_events: u64,
    /// Alerts triggered
    pub alerts_triggered: u64,
    /// Events currently in buffer
    pub buffer_count: u64,
    /// Buffer capacity
    pub buffer_capacity: u64,
}

// ---------------------------------------------------------------------------
// Enable / Disable
// ---------------------------------------------------------------------------

/// Enable audit logging.
pub fn enable() {
    AUDIT_ENABLED.store(true, Ordering::Release);
    println!("[AUDIT] Audit logging enabled");
}

/// Disable audit logging.
pub fn disable() {
    AUDIT_ENABLED.store(false, Ordering::Release);
    println!("[AUDIT] Audit logging disabled");
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize audit system.
pub fn init() -> Result<(), KernelError> {
    println!("[AUDIT] Initializing audit framework...");

    // Clear audit log
    AUDIT_HEAD.store(0, Ordering::Relaxed);
    AUDIT_COUNT.store(0, Ordering::Relaxed);

    // Set default filter (all events enabled)
    set_filter(AuditFilter::allow_all());

    // Try to create the audit log directory (best-effort)
    // The VFS may not be mounted yet; persistent logging will start
    // once the filesystem is available.
    let _ = ensure_audit_log_dir();

    // Enable auditing
    enable();

    println!("[AUDIT] Audit framework initialized");
    Ok(())
}

/// Best-effort creation of `/var/log/` directory for audit log storage.
fn ensure_audit_log_dir() -> Result<(), KernelError> {
    // Try to create /var, /var/log directories if they don't exist.
    // This is best-effort; if VFS is not mounted we silently skip.
    if let Some(vfs_lock) = crate::fs::try_get_vfs() {
        let vfs = vfs_lock.read();
        let perms = crate::fs::Permissions::default();
        let _ = vfs.mkdir("/var", perms);
        let _ = vfs.mkdir("/var/log", perms);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Simple string hash function (kept for backward compatibility).
fn _simple_hash(s: &str) -> u64 {
    let mut hash = 0u64;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
    }
    hash
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event() {
        let event = AuditEvent::new(
            AuditEventType::ProcessCreate,
            123,
            1000,
            AuditAction::Create,
            "process",
            true,
            "",
        );
        assert_eq!(event.pid, 123);
        assert_eq!(event.uid, 1000);
        assert!(event.result);
    }

    #[test]
    fn test_log_event() {
        enable(); // Auditing is disabled by default; must enable before logging
        log_process_create(456, 1000, 0);
        let (count, _) = get_stats();
        assert!(count > 0);
    }

    #[test]
    fn test_audit_filter() {
        let mut filter = AuditFilter::deny_all();
        assert!(!filter.is_enabled(AuditEventType::ProcessCreate));

        filter.enable(AuditEventType::ProcessCreate);
        assert!(filter.is_enabled(AuditEventType::ProcessCreate));
        assert!(!filter.is_enabled(AuditEventType::FileAccess));

        filter.disable(AuditEventType::ProcessCreate);
        assert!(!filter.is_enabled(AuditEventType::ProcessCreate));
    }

    #[test]
    fn test_event_serialization() {
        let event = AuditEvent::new(
            AuditEventType::FileAccess,
            42,
            1000,
            AuditAction::Read,
            "/etc/passwd",
            true,
            "mode:0644",
        );
        let serialized = event.serialize();
        assert!(serialized.contains("FILE_ACCESS"));
        assert!(serialized.contains("42"));
        assert!(serialized.contains("READ"));
        assert!(serialized.contains("/etc/passwd"));
        assert!(serialized.contains("OK"));
    }
}

//! Security audit framework

//! Tracks and logs security-relevant events for compliance and forensics.

use crate::error::KernelError;

/// Audit event type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEventType {
    /// Process creation
    ProcessCreate,
    /// Process termination
    ProcessExit,
    /// File access
    FileAccess,
    /// Network connection
    NetworkConnect,
    /// Authentication attempt
    AuthAttempt,
    /// Permission denied
    PermissionDenied,
    /// System call
    Syscall,
    /// Capability operation
    CapabilityOp,
}

/// Audit event record
#[derive(Debug, Clone, Copy)]
pub struct AuditEvent {
    /// Event type
    pub event_type: AuditEventType,
    /// Timestamp (CPU cycles)
    pub timestamp: u64,
    /// Process ID
    pub pid: u64,
    /// User ID
    pub uid: u32,
    /// Result code (0 = success, other = error)
    pub result: i32,
    /// Additional data
    pub data: u64,
}

impl AuditEvent {
    /// Create a new audit event
    pub fn new(event_type: AuditEventType, pid: u64, uid: u32, result: i32, data: u64) -> Self {
        Self {
            event_type,
            timestamp: read_timestamp_fallback(),
            pid,
            uid,
            result,
            data,
        }
    }
}

/// Fallback timestamp function
fn read_timestamp_fallback() -> u64 {
    // Try to use the test framework function if available
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    ))]
    {
        crate::test_framework::read_timestamp()
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        0 // Fallback
    }
}

/// Maximum audit log size
const MAX_AUDIT_LOG: usize = 4096;

/// Audit log buffer (circular)
static mut AUDIT_LOG: [Option<AuditEvent>; MAX_AUDIT_LOG] = [None; MAX_AUDIT_LOG];
static mut AUDIT_HEAD: usize = 0;
static mut AUDIT_COUNT: usize = 0;
static mut AUDIT_ENABLED: bool = false;

/// Log an audit event
pub fn log_event(event: AuditEvent) {
    // SAFETY: AUDIT_ENABLED, AUDIT_LOG, AUDIT_HEAD, and AUDIT_COUNT are static mut
    // globals used as a circular buffer for audit events. Accessed during
    // single-threaded kernel operation. AUDIT_HEAD is always kept in range [0,
    // MAX_AUDIT_LOG) by the modulo operation.
    unsafe {
        if !AUDIT_ENABLED {
            return;
        }

        AUDIT_LOG[AUDIT_HEAD] = Some(event);
        AUDIT_HEAD = (AUDIT_HEAD + 1) % MAX_AUDIT_LOG;

        if AUDIT_COUNT < MAX_AUDIT_LOG {
            AUDIT_COUNT += 1;
        }
    }
}

/// Log a process creation
pub fn log_process_create(pid: u64, uid: u32, result: i32) {
    log_event(AuditEvent::new(
        AuditEventType::ProcessCreate,
        pid,
        uid,
        result,
        0,
    ));
}

/// Log a process exit
pub fn log_process_exit(pid: u64, exit_code: i32) {
    log_event(AuditEvent::new(
        AuditEventType::ProcessExit,
        pid,
        0,
        exit_code,
        0,
    ));
}

/// Log a file access
pub fn log_file_access(pid: u64, uid: u32, path_hash: u64, access_type: u32) {
    log_event(AuditEvent::new(
        AuditEventType::FileAccess,
        pid,
        uid,
        access_type as i32,
        path_hash,
    ));
}

/// Log a permission denial
pub fn log_permission_denied(pid: u64, uid: u32, target: &str) {
    let target_hash = simple_hash(target);
    log_event(AuditEvent::new(
        AuditEventType::PermissionDenied,
        pid,
        uid,
        -1,
        target_hash,
    ));
}

/// Simple string hash function
fn simple_hash(s: &str) -> u64 {
    let mut hash = 0u64;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
    }
    hash
}

/// Get audit log statistics
pub fn get_stats() -> (usize, usize) {
    // SAFETY: AUDIT_COUNT is a static mut counter read for diagnostic statistics.
    // Single-threaded access assumed during kernel operation.
    unsafe { (AUDIT_COUNT, MAX_AUDIT_LOG) }
}

/// Enable audit logging
pub fn enable() {
    // SAFETY: AUDIT_ENABLED is a static mut bool toggled during single-threaded
    // kernel init or controlled administrative operations.
    unsafe {
        AUDIT_ENABLED = true;
    }
    println!("[AUDIT] Audit logging enabled");
}

/// Disable audit logging
pub fn disable() {
    // SAFETY: AUDIT_ENABLED is a static mut bool toggled during controlled
    // administrative operations. Single-threaded access assumed.
    unsafe {
        AUDIT_ENABLED = false;
    }
    println!("[AUDIT] Audit logging disabled");
}

/// Initialize audit system
pub fn init() -> Result<(), KernelError> {
    println!("[AUDIT] Initializing audit framework...");

    // Clear audit log
    // SAFETY: AUDIT_HEAD and AUDIT_COUNT are static mut counters reset to zero
    // during single-threaded kernel init. No concurrent readers at this point
    // in bootstrap.
    unsafe {
        AUDIT_HEAD = 0;
        AUDIT_COUNT = 0;
    }

    // Enable auditing
    enable();

    println!("[AUDIT] Audit framework initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_audit_event() {
        let event = AuditEvent::new(AuditEventType::ProcessCreate, 123, 1000, 0, 0);
        assert_eq!(event.pid, 123);
        assert_eq!(event.uid, 1000);
        assert_eq!(event.result, 0);
    }

    #[test_case]
    fn test_log_event() {
        log_process_create(456, 1000, 0);
        let (count, _) = get_stats();
        assert!(count > 0);
    }
}

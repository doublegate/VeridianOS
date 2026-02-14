//! Mandatory Access Control (MAC) system
//!
//! Provides a policy-based access control system similar to SELinux.
//! Enforces security policies for all system operations.

#![allow(clippy::needless_range_loop)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use spin::Mutex;

use super::AccessType;
use crate::error::KernelError;

/// Maximum number of policy rules
const MAX_POLICY_RULES: usize = 1024;

/// Security policy rule
#[derive(Debug, Clone, Copy)]
pub struct PolicyRule {
    /// Source domain/label
    pub source: &'static str,
    /// Target domain/label
    pub target: &'static str,
    /// Allowed access types
    pub allowed: u8, // Bitmask: 0x1=Read, 0x2=Write, 0x4=Execute
    /// Rule enabled
    pub enabled: bool,
}

impl PolicyRule {
    /// Create a new policy rule
    pub const fn new(source: &'static str, target: &'static str, allowed: u8) -> Self {
        Self {
            source,
            target,
            allowed,
            enabled: true,
        }
    }

    /// Check if access is allowed by this rule
    pub fn allows(&self, access: AccessType) -> bool {
        if !self.enabled {
            return false;
        }

        let bit = match access {
            AccessType::Read => 0x1,
            AccessType::Write => 0x2,
            AccessType::Execute => 0x4,
        };

        (self.allowed & bit) != 0
    }
}

/// MAC policy database
static POLICY_RULES: Mutex<[Option<PolicyRule>; MAX_POLICY_RULES]> =
    Mutex::new([None; MAX_POLICY_RULES]);
static POLICY_COUNT: AtomicUsize = AtomicUsize::new(0);
static MAC_ENABLED: AtomicBool = AtomicBool::new(false);

/// Add a policy rule
pub fn add_rule(rule: PolicyRule) -> Result<(), KernelError> {
    let count = POLICY_COUNT.load(Ordering::Relaxed);
    if count >= MAX_POLICY_RULES {
        return Err(KernelError::OutOfMemory {
            requested: 0,
            available: 0,
        });
    }

    let mut rules = POLICY_RULES.lock();
    rules[count] = Some(rule);
    POLICY_COUNT.store(count + 1, Ordering::Relaxed);

    Ok(())
}

/// Check if access is allowed by MAC policy
pub fn check_access(source: &str, target: &str, access: AccessType) -> bool {
    if !MAC_ENABLED.load(Ordering::Acquire) {
        return true; // MAC disabled, allow all
    }

    let rules = POLICY_RULES.lock();
    let count = POLICY_COUNT.load(Ordering::Relaxed);

    // Check for matching rule
    for i in 0..count {
        if let Some(rule) = &rules[i] {
            if rule.source == source && rule.target == target {
                return rule.allows(access);
            }
        }
    }

    // No rule found - deny by default
    false
}

/// Enable MAC enforcement
pub fn enable() {
    MAC_ENABLED.store(true, Ordering::Release);
    println!("[MAC] Mandatory Access Control enabled");
}

/// Disable MAC enforcement (for debugging)
pub fn disable() {
    MAC_ENABLED.store(false, Ordering::Release);
    println!("[MAC] Mandatory Access Control disabled");
}

/// Load default policy
fn load_default_policy() -> Result<(), KernelError> {
    // System domain can access everything
    add_rule(PolicyRule::new("system_t", "system_t", 0x7))?;
    add_rule(PolicyRule::new("system_t", "user_t", 0x7))?;
    add_rule(PolicyRule::new("system_t", "file_t", 0x7))?;

    // User domain has limited access
    add_rule(PolicyRule::new("user_t", "user_t", 0x7))?;
    add_rule(PolicyRule::new("user_t", "file_t", 0x3))?; // Read/Write only

    // Driver domain
    add_rule(PolicyRule::new("driver_t", "system_t", 0x1))?; // Read only
    add_rule(PolicyRule::new("driver_t", "device_t", 0x7))?; // Full access to devices

    // Init process has special privileges
    add_rule(PolicyRule::new("init_t", "system_t", 0x7))?;
    add_rule(PolicyRule::new("init_t", "user_t", 0x7))?;
    add_rule(PolicyRule::new("init_t", "file_t", 0x7))?;

    println!(
        "[MAC] Loaded {} default policy rules",
        POLICY_COUNT.load(Ordering::Relaxed)
    );
    Ok(())
}

/// Initialize MAC system
pub fn init() -> Result<(), KernelError> {
    println!("[MAC] Initializing Mandatory Access Control...");

    // Load default policy
    load_default_policy()?;

    // Enable MAC enforcement
    enable();

    println!(
        "[MAC] MAC system initialized with {} rules",
        POLICY_COUNT.load(Ordering::Relaxed)
    );
    Ok(())
}

/// Check file access using MAC policy
///
/// Maps the calling process to a security domain and checks if that domain
/// can access file objects with the given access type.
pub fn check_file_access(_path: &str, access: AccessType, pid: u64) -> Result<(), &'static str> {
    // Determine source label based on PID
    // PID 0 = kernel (system_t), PID 1 = init (init_t), others = user_t
    let source = match pid {
        0 => "system_t",
        1 => "init_t",
        _ => "user_t",
    };

    // Files are in the file_t domain
    let target = "file_t";

    if check_access(source, target, access) {
        Ok(())
    } else {
        // Log the denial via audit if available
        crate::security::audit::log_permission_denied(pid, 0, "file_access");
        Err("MAC policy denied file access")
    }
}

/// Check IPC access using MAC policy
///
/// Validates that a process can perform IPC operations based on MAC policy.
pub fn check_ipc_access(access: AccessType, pid: u64) -> Result<(), &'static str> {
    let source = match pid {
        0 => "system_t",
        1 => "init_t",
        _ => "user_t",
    };

    // IPC targets are in the system_t domain
    let target = "system_t";

    if check_access(source, target, access) {
        Ok(())
    } else {
        crate::security::audit::log_permission_denied(pid, 0, "ipc_access");
        Err("MAC policy denied IPC access")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_policy_rule() {
        let rule = PolicyRule::new("user_t", "file_t", 0x3); // Read + Write
        assert!(rule.allows(AccessType::Read));
        assert!(rule.allows(AccessType::Write));
        assert!(!rule.allows(AccessType::Execute));
    }

    #[test_case]
    fn test_add_rule() {
        let rule = PolicyRule::new("test_t", "test_t", 0x7);
        assert!(add_rule(rule).is_ok());
    }
}

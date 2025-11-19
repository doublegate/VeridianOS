//! Mandatory Access Control (MAC) system

//!
//! Provides a policy-based access control system similar to SELinux.
//! Enforces security policies for all system operations.

use crate::error::KernelError;
use super::AccessType;

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
static mut POLICY_RULES: [Option<PolicyRule>; MAX_POLICY_RULES] = [None; MAX_POLICY_RULES];
static mut POLICY_COUNT: usize = 0;
static mut MAC_ENABLED: bool = false;

/// Add a policy rule
pub fn add_rule(rule: PolicyRule) -> Result<(), KernelError> {
    unsafe {
        if POLICY_COUNT >= MAX_POLICY_RULES {
            return Err(KernelError::OutOfMemory { requested: 0, available: 0 });
        }

        POLICY_RULES[POLICY_COUNT] = Some(rule);
        POLICY_COUNT += 1;
    }

    Ok(())
}

/// Check if access is allowed by MAC policy
pub fn check_access(source: &str, target: &str, access: AccessType) -> bool {
    unsafe {
        if !MAC_ENABLED {
            return true; // MAC disabled, allow all
        }

        // Check for matching rule
        for i in 0..POLICY_COUNT {
            if let Some(rule) = &POLICY_RULES[i] {
                if rule.source == source && rule.target == target {
                    return rule.allows(access);
                }
            }
        }

        // No rule found - deny by default
        false
    }
}

/// Enable MAC enforcement
pub fn enable() {
    unsafe {
        MAC_ENABLED = true;
    }
    println!("[MAC] Mandatory Access Control enabled");
}

/// Disable MAC enforcement (for debugging)
pub fn disable() {
    unsafe {
        MAC_ENABLED = false;
    }
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

    println!("[MAC] Loaded {} default policy rules", unsafe { POLICY_COUNT });
    Ok(())
}

/// Initialize MAC system
pub fn init() -> Result<(), KernelError> {
    println!("[MAC] Initializing Mandatory Access Control...");

    // Load default policy
    load_default_policy()?;

    // Enable MAC enforcement
    enable();

    println!("[MAC] MAC system initialized with {} rules", unsafe { POLICY_COUNT });
    Ok(())
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

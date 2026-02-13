//! Security infrastructure for VeridianOS

//! This module provides comprehensive security features including:
//! - Cryptographic primitives
//! - Mandatory Access Control (MAC)
//! - Security audit framework
//! - Secure boot verification

pub mod audit;
pub mod auth;
pub mod boot;
pub mod crypto;
pub mod mac;
pub mod memory_protection;
pub mod tpm;
pub mod tpm_commands;

use crate::error::KernelError;

/// Security context for processes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityContext {
    /// Security label/domain
    pub label: &'static str,
    /// User ID
    pub uid: u32,
    /// Group ID
    pub gid: u32,
    /// Security level
    pub level: SecurityLevel,
}

/// Security levels for multi-level security (MLS)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SecurityLevel {
    Unclassified = 0,
    Confidential = 1,
    Secret = 2,
    TopSecret = 3,
}

impl SecurityContext {
    /// Create a new security context
    pub const fn new(label: &'static str, uid: u32, gid: u32, level: SecurityLevel) -> Self {
        Self {
            label,
            uid,
            gid,
            level,
        }
    }

    /// Check if this context can access another context
    pub fn can_access(&self, target: &SecurityContext, access: AccessType) -> bool {
        // Check MAC policy
        if !mac::check_access(self.label, target.label, access) {
            return false;
        }

        // Check MLS: no read-up, no write-down
        match access {
            AccessType::Read => self.level >= target.level,
            AccessType::Write => self.level <= target.level,
            AccessType::Execute => self.level == target.level,
        }
    }
}

/// Access types for security checks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    Read,
    Write,
    Execute,
}

/// Initialize security subsystem
pub fn init() -> Result<(), KernelError> {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::aarch64::direct_uart::uart_write_str("[SECURITY] init start\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] Initializing security subsystem...");

    // Initialize memory protection (ASLR, stack canaries, etc.)
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] About to init memory_protection...");
    memory_protection::init()?;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::aarch64::direct_uart::uart_write_str("[SECURITY] mem_prot done\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] memory_protection done");

    // Initialize authentication framework
    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::aarch64::direct_uart::uart_write_str("[SECURITY] auth start\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] About to init auth...");
    auth::init()?;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::aarch64::direct_uart::uart_write_str("[SECURITY] auth done\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] auth done");

    // Initialize TPM support (if available)
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] About to init tpm...");
    tpm::init()?;
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] tpm done");

    // Initialize MAC system
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] About to init mac...");
    mac::init()?;
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] mac done");

    // Initialize audit system
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] About to init audit...");
    audit::init()?;
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] audit done");

    // Verify secure boot (if enabled)
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] About to verify boot...");
    boot::verify()?;
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] boot verify done");

    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::aarch64::direct_uart::uart_write_str("[SECURITY] init complete\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SECURITY] Security subsystem initialized successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_security_context() {
        let ctx1 = SecurityContext::new("user_t", 1000, 1000, SecurityLevel::Unclassified);
        let ctx2 = SecurityContext::new("system_t", 0, 0, SecurityLevel::Secret);

        // User can read unclassified
        assert!(ctx1.can_access(&ctx1, AccessType::Read));

        // User cannot read secret (no read-up)
        assert!(!ctx1.can_access(&ctx2, AccessType::Read));

        // System can read unclassified (read-down allowed)
        assert!(ctx2.can_access(&ctx1, AccessType::Read));
    }
}

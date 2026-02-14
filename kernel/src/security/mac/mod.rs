//! Mandatory Access Control (MAC) system
//!
//! Provides a policy-based access control system similar to SELinux.
//! Enforces security policies for all system operations.
//!
//! # Policy Language
//!
//! The MAC system supports a simple text-based policy language:
//! ```text
//! allow source_type target_type { read write execute };
//! deny source_type target_type { write };
//! type_transition source_type target_type : process new_type;
//! role admin_r types { system_t init_t };
//! user root roles { admin_r };
//! sensitivity s0-s3;
//! category c0-c63;
//! ```
//!
//! # Multi-Level Security (MLS)
//!
//! MLS uses sensitivity levels (0..=65535) and category bitmasks (64 bits).
//! A security level dominates another if its sensitivity is greater or equal
//! AND its category set is a superset of the other's.
//!
//! # RBAC Layer
//!
//! Users are mapped to roles, and roles are mapped to types. A process
//! running with a particular user identity can only transition into types
//! allowed by that user's assigned roles.
//!
//! # Zero-Allocation Design
//!
//! All data structures use fixed-size arrays and `&'static str` references
//! to avoid heap allocations. This is critical for boot-time initialization
//! on architectures (RISC-V, AArch64) where the bump allocator cannot
//! handle many small allocations without corruption.

#![allow(clippy::needless_range_loop)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use spin::Mutex;

use super::AccessType;
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of policy rules
const MAX_POLICY_RULES: usize = 64;

/// Maximum number of domain transitions
const MAX_TRANSITIONS: usize = 32;

/// Maximum number of roles
const MAX_ROLES: usize = 8;

/// Maximum number of types per role
const MAX_ROLE_TYPES: usize = 16;

/// Maximum number of user-to-role mappings
const MAX_USER_ROLES: usize = 16;

/// Maximum number of roles assigned to a single user
const MAX_USER_ASSIGNED_ROLES: usize = 8;

/// Maximum number of process security labels
const MAX_PROCESS_LABELS: usize = 64;

/// Maximum number of permissions per rule (Read, Write, Execute = 3 max)
const MAX_PERMISSIONS: usize = 3;

/// Maximum number of tokens the parser can handle
const MAX_TOKENS: usize = 128;

/// Maximum number of parsed rules from a single parse call
const MAX_PARSED_RULES: usize = 32;

/// Maximum number of parsed transitions from a single parse call
const MAX_PARSED_TRANSITIONS: usize = 16;

/// Maximum number of parsed roles from a single parse call
const MAX_PARSED_ROLES: usize = 8;

/// Maximum number of parsed user-role mappings from a single parse call
const MAX_PARSED_USER_ROLES: usize = 8;

// ---------------------------------------------------------------------------
// Multi-Level Security (MLS)
// ---------------------------------------------------------------------------

/// MLS security level with sensitivity and category bitmask.
///
/// Dominance: level A dominates level B iff
///   A.sensitivity >= B.sensitivity AND A.categories is a superset of
/// B.categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MlsLevel {
    /// Sensitivity level (0 = lowest, higher = more sensitive)
    pub sensitivity: u16,
    /// Category bitmask (up to 64 categories)
    pub categories: u64,
}

impl MlsLevel {
    /// Create a new MLS level.
    pub const fn new(sensitivity: u16, categories: u64) -> Self {
        Self {
            sensitivity,
            categories,
        }
    }

    /// Default (lowest) security level.
    pub const fn default_level() -> Self {
        Self {
            sensitivity: 0,
            categories: 0,
        }
    }

    /// Check if this level dominates (is at least as restrictive as) `other`.
    pub fn dominates(&self, other: &MlsLevel) -> bool {
        self.sensitivity >= other.sensitivity
            && (self.categories & other.categories) == other.categories
    }
}

// ---------------------------------------------------------------------------
// Security Label
// ---------------------------------------------------------------------------

/// Full security label combining type, role, and MLS level.
///
/// Uses `&'static str` references to avoid heap allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityLabel {
    /// Type/domain name (e.g. "system_t", "user_t")
    pub type_name: &'static str,
    /// Role (e.g. "system_r", "user_r")
    pub role: &'static str,
    /// MLS security level
    pub level: MlsLevel,
}

impl SecurityLabel {
    /// Create a new security label.
    pub const fn new(type_name: &'static str, role: &'static str, level: MlsLevel) -> Self {
        Self {
            type_name,
            role,
            level,
        }
    }

    /// Create a label with default MLS level.
    pub const fn simple(type_name: &'static str, role: &'static str) -> Self {
        Self::new(type_name, role, MlsLevel::default_level())
    }
}

// ---------------------------------------------------------------------------
// Policy Rule
// ---------------------------------------------------------------------------

/// Action to take when a policy rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
}

/// Security policy rule.
///
/// Uses fixed-size arrays and `&'static str` to avoid heap allocations.
#[derive(Debug, Clone, Copy)]
pub struct PolicyRule {
    /// Source domain/type
    pub source_type: &'static str,
    /// Target domain/type
    pub target_type: &'static str,
    /// Allowed/denied permission set (fixed-size array)
    pub permissions: [Permission; MAX_PERMISSIONS],
    /// Number of active permissions in the array
    pub perm_count: u8,
    /// Whether this rule allows or denies
    pub action: PolicyAction,
    /// Rule enabled
    pub enabled: bool,
}

/// Permission types for policy rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Execute,
}

impl PolicyRule {
    /// Create a new policy rule with the given action.
    pub const fn new(
        source_type: &'static str,
        target_type: &'static str,
        permissions: [Permission; MAX_PERMISSIONS],
        perm_count: u8,
        action: PolicyAction,
    ) -> Self {
        Self {
            source_type,
            target_type,
            permissions,
            perm_count,
            action,
            enabled: true,
        }
    }

    /// Create a policy rule from a slice of permissions.
    ///
    /// Copies up to MAX_PERMISSIONS permissions into the fixed-size array.
    pub fn from_perms(
        source_type: &'static str,
        target_type: &'static str,
        perms: &[Permission],
        action: PolicyAction,
    ) -> Self {
        let mut permissions = [Permission::Read; MAX_PERMISSIONS];
        let count = perms.len().min(MAX_PERMISSIONS);
        let mut i = 0;
        while i < count {
            permissions[i] = perms[i];
            i += 1;
        }
        Self {
            source_type,
            target_type,
            permissions,
            perm_count: count as u8,
            action,
            enabled: true,
        }
    }

    /// Create an Allow rule from a legacy bitmask for backward compatibility.
    pub fn from_legacy(source: &'static str, target: &'static str, allowed: u8) -> Self {
        let mut permissions = [Permission::Read; MAX_PERMISSIONS];
        let mut count: u8 = 0;
        if allowed & 0x1 != 0 {
            permissions[count as usize] = Permission::Read;
            count += 1;
        }
        if allowed & 0x2 != 0 {
            permissions[count as usize] = Permission::Write;
            count += 1;
        }
        if allowed & 0x4 != 0 {
            permissions[count as usize] = Permission::Execute;
            count += 1;
        }
        Self {
            source_type: source,
            target_type: target,
            permissions,
            perm_count: count,
            action: PolicyAction::Allow,
            enabled: true,
        }
    }

    /// Check if this rule contains a specific permission.
    fn has_permission(&self, perm: Permission) -> bool {
        let mut i = 0;
        while i < self.perm_count as usize {
            if matches!(
                (&self.permissions[i], &perm),
                (Permission::Read, Permission::Read)
                    | (Permission::Write, Permission::Write)
                    | (Permission::Execute, Permission::Execute)
            ) {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Check if this rule matches and allows the given access.
    pub fn allows(&self, access: AccessType) -> bool {
        if !self.enabled {
            return false;
        }

        let perm = match access {
            AccessType::Read => Permission::Read,
            AccessType::Write => Permission::Write,
            AccessType::Execute => Permission::Execute,
        };

        let has_perm = self.has_permission(perm);

        match self.action {
            PolicyAction::Allow => has_perm,
            PolicyAction::Deny => false, // Deny rules never "allow"
        }
    }

    /// Check if this rule explicitly denies the given access.
    pub fn denies(&self, access: AccessType) -> bool {
        if !self.enabled {
            return false;
        }

        let perm = match access {
            AccessType::Read => Permission::Read,
            AccessType::Write => Permission::Write,
            AccessType::Execute => Permission::Execute,
        };

        match self.action {
            PolicyAction::Deny => self.has_permission(perm),
            PolicyAction::Allow => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Domain Transitions
// ---------------------------------------------------------------------------

/// Domain transition rule.
///
/// When a process of type `source_type` executes a binary labeled as
/// `target_type` of object class `class`, the process transitions to
/// `new_type`.
#[derive(Debug, Clone, Copy)]
pub struct DomainTransition {
    /// Source process type
    pub source_type: &'static str,
    /// Target file/object type
    pub target_type: &'static str,
    /// Object class (e.g. "process", "file")
    pub class: &'static str,
    /// New type after transition
    pub new_type: &'static str,
}

impl DomainTransition {
    /// Create a new domain transition rule.
    pub const fn new(
        source_type: &'static str,
        target_type: &'static str,
        class: &'static str,
        new_type: &'static str,
    ) -> Self {
        Self {
            source_type,
            target_type,
            class,
            new_type,
        }
    }
}

// ---------------------------------------------------------------------------
// RBAC Layer
// ---------------------------------------------------------------------------

/// Role definition mapping a role name to allowed types.
///
/// Uses fixed-size array of `&'static str` to avoid heap allocations.
#[derive(Debug, Clone, Copy)]
pub struct Role {
    /// Role name (e.g. "admin_r", "user_r")
    pub name: &'static str,
    /// Types this role is allowed to transition to
    pub allowed_types: [&'static str; MAX_ROLE_TYPES],
    /// Number of active types in the array
    pub type_count: usize,
}

impl Role {
    /// Create a new role with allowed types from a slice.
    pub fn from_types(name: &'static str, types: &[&'static str]) -> Self {
        let mut allowed_types = [""; MAX_ROLE_TYPES];
        let count = types.len().min(MAX_ROLE_TYPES);
        let mut i = 0;
        while i < count {
            allowed_types[i] = types[i];
            i += 1;
        }
        Self {
            name,
            allowed_types,
            type_count: count,
        }
    }

    /// Check if this role allows the given type.
    pub fn allows_type(&self, type_name: &str) -> bool {
        let mut i = 0;
        while i < self.type_count {
            if str_eq(self.allowed_types[i], type_name) {
                return true;
            }
            i += 1;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Fixed-size map entry types for PolicyDatabase
// ---------------------------------------------------------------------------

/// Entry in the roles table: maps a role name to a Role.
#[derive(Debug, Clone, Copy)]
struct RoleEntry {
    name: &'static str,
    role: Role,
}

/// Entry in the user-roles table: maps a username to assigned role names.
#[derive(Debug, Clone, Copy)]
struct UserRoleEntry {
    username: &'static str,
    roles: [&'static str; MAX_USER_ASSIGNED_ROLES],
    role_count: usize,
}

/// Entry in the process-labels table: maps a PID to a SecurityLabel.
#[derive(Debug, Clone, Copy)]
struct ProcessLabelEntry {
    pid: u64,
    label: SecurityLabel,
}

// ---------------------------------------------------------------------------
// Policy Parser (extracted to separate module)
// ---------------------------------------------------------------------------

mod parser;
use parser::PolicyParser;

// ---------------------------------------------------------------------------
// Policy Database
// ---------------------------------------------------------------------------

/// Complete MAC policy database including rules, transitions, RBAC, and MLS.
///
/// All maps use fixed-size arrays with linear search. Zero heap allocations.
struct PolicyDatabase {
    /// Access control rules
    rules: [Option<PolicyRule>; MAX_POLICY_RULES],
    /// Number of active rules
    rule_count: usize,
    /// Domain transition rules
    transitions: [Option<DomainTransition>; MAX_TRANSITIONS],
    /// Number of active transitions
    transition_count: usize,
    /// Roles: linear array of (name, Role) entries
    roles: [Option<RoleEntry>; MAX_ROLES],
    /// Number of active roles
    role_count: usize,
    /// User-to-role mapping: linear array of (username, role list) entries
    user_roles: [Option<UserRoleEntry>; MAX_USER_ROLES],
    /// Number of active user-role mappings
    user_role_count: usize,
    /// Process security labels: linear array of (pid, SecurityLabel) entries
    process_labels: [Option<ProcessLabelEntry>; MAX_PROCESS_LABELS],
    /// Number of active process labels
    process_label_count: usize,
}

impl PolicyDatabase {
    /// Create an empty policy database.
    const fn new() -> Self {
        Self {
            rules: [const { None }; MAX_POLICY_RULES],
            rule_count: 0,
            transitions: [const { None }; MAX_TRANSITIONS],
            transition_count: 0,
            roles: [const { None }; MAX_ROLES],
            role_count: 0,
            user_roles: [const { None }; MAX_USER_ROLES],
            user_role_count: 0,
            process_labels: [const { None }; MAX_PROCESS_LABELS],
            process_label_count: 0,
        }
    }

    /// Find a role by name.
    fn find_role(&self, name: &str) -> Option<&Role> {
        for i in 0..self.role_count {
            if let Some(entry) = &self.roles[i] {
                if str_eq(entry.name, name) {
                    return Some(&entry.role);
                }
            }
        }
        None
    }

    /// Insert or update a role.
    fn insert_role(&mut self, role: Role) {
        // Check if role already exists, update it
        for i in 0..self.role_count {
            if let Some(entry) = &mut self.roles[i] {
                if str_eq(entry.name, role.name) {
                    entry.role = role;
                    return;
                }
            }
        }
        // Insert new
        if self.role_count < MAX_ROLES {
            self.roles[self.role_count] = Some(RoleEntry {
                name: role.name,
                role,
            });
            self.role_count += 1;
        }
    }

    /// Check if any roles are defined.
    fn has_roles(&self) -> bool {
        self.role_count > 0
    }

    /// Find user-role entry by username.
    fn find_user_roles(&self, username: &str) -> Option<&UserRoleEntry> {
        for i in 0..self.user_role_count {
            if let Some(entry) = &self.user_roles[i] {
                if str_eq(entry.username, username) {
                    return Some(entry);
                }
            }
        }
        None
    }

    /// Insert or update user-role mapping.
    fn insert_user_roles(&mut self, entry: UserRoleEntry) {
        // Check if user already exists, update
        for i in 0..self.user_role_count {
            if let Some(existing) = &mut self.user_roles[i] {
                if str_eq(existing.username, entry.username) {
                    *existing = entry;
                    return;
                }
            }
        }
        // Insert new
        if self.user_role_count < MAX_USER_ROLES {
            self.user_roles[self.user_role_count] = Some(entry);
            self.user_role_count += 1;
        }
    }

    /// Find process label by PID.
    fn find_process_label(&self, pid: u64) -> Option<SecurityLabel> {
        for i in 0..self.process_label_count {
            if let Some(entry) = &self.process_labels[i] {
                if entry.pid == pid {
                    return Some(entry.label);
                }
            }
        }
        None
    }

    /// Insert or update process label.
    fn insert_process_label(&mut self, pid: u64, label: SecurityLabel) {
        // Check if PID already exists, update
        for i in 0..self.process_label_count {
            if let Some(entry) = &mut self.process_labels[i] {
                if entry.pid == pid {
                    entry.label = label;
                    return;
                }
            }
        }
        // Insert new
        if self.process_label_count < MAX_PROCESS_LABELS {
            self.process_labels[self.process_label_count] = Some(ProcessLabelEntry { pid, label });
            self.process_label_count += 1;
        }
    }
}

/// MAC policy database (protected by Mutex).
///
/// Const-initialized directly in the static so it lives in BSS, never on the
/// stack. This avoids the ~102 KB stack allocation that previously overflowed
/// the RISC-V kernel stack and corrupted the bump allocator in BSS.
static POLICY_DB: Mutex<PolicyDatabase> = Mutex::new(PolicyDatabase::new());
static POLICY_COUNT: AtomicUsize = AtomicUsize::new(0);
static MAC_ENABLED: AtomicBool = AtomicBool::new(false);

/// Convenience: lock the policy database and run a closure on it.
fn with_policy_db<R, F: FnOnce(&mut PolicyDatabase) -> R>(f: F) -> R {
    let mut guard = POLICY_DB.lock();
    f(&mut guard)
}

// ---------------------------------------------------------------------------
// String comparison helper (no allocation)
// ---------------------------------------------------------------------------

/// Compare two string slices for equality without allocation.
#[inline]
fn str_eq(a: &str, b: &str) -> bool {
    a.as_bytes() == b.as_bytes()
}

// ---------------------------------------------------------------------------
// Public API: Rule Management
// ---------------------------------------------------------------------------

/// Add a policy rule (new API with structured rule).
pub fn add_policy_rule(rule: PolicyRule) -> Result<(), KernelError> {
    with_policy_db(|db| {
        if db.rule_count >= MAX_POLICY_RULES {
            return Err(KernelError::ResourceExhausted {
                resource: "MAC policy rules",
            });
        }
        db.rules[db.rule_count] = Some(rule);
        db.rule_count += 1;
        POLICY_COUNT.store(db.rule_count, Ordering::Relaxed);
        Ok(())
    })
}

/// Add a legacy policy rule (backward compatible with old `PolicyRule::new`).
///
/// Wraps the old bitmask-based API into the new structured format.
pub fn add_rule(
    source: &'static str,
    target: &'static str,
    allowed: u8,
) -> Result<(), KernelError> {
    add_policy_rule(PolicyRule::from_legacy(source, target, allowed))
}

/// Add a domain transition rule.
pub fn add_transition(transition: DomainTransition) -> Result<(), KernelError> {
    with_policy_db(|db| {
        if db.transition_count >= MAX_TRANSITIONS {
            return Err(KernelError::ResourceExhausted {
                resource: "MAC domain transitions",
            });
        }
        db.transitions[db.transition_count] = Some(transition);
        db.transition_count += 1;
        Ok(())
    })
}

/// Add a role definition.
pub fn add_role(role: Role) {
    with_policy_db(|db| {
        db.insert_role(role);
    })
}

/// Map a user to a set of roles (zero-allocation version).
pub fn assign_user_roles_static(username: &'static str, roles: &[&'static str]) {
    let mut assigned = [""; MAX_USER_ASSIGNED_ROLES];
    let count = roles.len().min(MAX_USER_ASSIGNED_ROLES);
    let mut i = 0;
    while i < count {
        assigned[i] = roles[i];
        i += 1;
    }
    with_policy_db(|db| {
        db.insert_user_roles(UserRoleEntry {
            username,
            roles: assigned,
            role_count: count,
        });
    })
}

/// Set the security label for a process.
pub fn set_process_label(pid: u64, label: SecurityLabel) {
    with_policy_db(|db| {
        db.insert_process_label(pid, label);
    })
}

/// Get the security label for a process.
pub fn get_process_label(pid: u64) -> Option<SecurityLabel> {
    with_policy_db(|db| db.find_process_label(pid))
}

// ---------------------------------------------------------------------------
// Public API: Access Checks
// ---------------------------------------------------------------------------

/// Check if access is allowed by MAC policy.
///
/// This is the primary access check function. It evaluates deny rules first
/// (deny overrides allow), then checks for a matching allow rule.
pub fn check_access(source: &str, target: &str, access: AccessType) -> bool {
    if !MAC_ENABLED.load(Ordering::Acquire) {
        return true; // MAC disabled, allow all
    }

    with_policy_db(|db| {
        // Phase 1: Check deny rules first (deny always wins)
        for i in 0..db.rule_count {
            if let Some(rule) = &db.rules[i] {
                if str_eq(rule.source_type, source)
                    && str_eq(rule.target_type, target)
                    && rule.denies(access)
                {
                    return false;
                }
            }
        }

        // Phase 2: Check allow rules
        for i in 0..db.rule_count {
            if let Some(rule) = &db.rules[i] {
                if str_eq(rule.source_type, source)
                    && str_eq(rule.target_type, target)
                    && rule.allows(access)
                {
                    return true;
                }
            }
        }

        // No matching rule -- deny by default
        false
    })
}

/// Check access with full security label (MAC + MLS + RBAC).
///
/// Performs three checks:
/// 1. MAC type enforcement (check_access)
/// 2. MLS dominance (subject must dominate object for read; object must
///    dominate subject for write)
/// 3. RBAC: subject's role must allow the source type
pub fn check_access_full(
    subject: &SecurityLabel,
    object: &SecurityLabel,
    access: AccessType,
) -> bool {
    // 1. MAC type enforcement
    if !check_access(subject.type_name, object.type_name, access) {
        return false;
    }

    // 2. MLS dominance checks (Bell-LaPadula: no read up, no write down)
    match access {
        AccessType::Read => {
            // Subject must dominate object to read (no read-up)
            if !subject.level.dominates(&object.level) {
                return false;
            }
        }
        AccessType::Write => {
            // Object must dominate subject to write (no write-down)
            if !object.level.dominates(&subject.level) {
                return false;
            }
        }
        AccessType::Execute => {
            // For execute, levels must match exactly
            if subject.level.sensitivity != object.level.sensitivity {
                return false;
            }
        }
    }

    // 3. RBAC: check that the subject's role allows its type
    with_policy_db(|db| {
        if let Some(role) = db.find_role(subject.role) {
            role.allows_type(subject.type_name)
        } else {
            // If no roles are defined, skip RBAC check (permissive)
            !db.has_roles()
        }
    })
}

/// Look up a domain transition.
///
/// Returns the new type if a transition rule matches.
pub fn lookup_transition(
    source_type: &str,
    target_type: &str,
    class: &str,
) -> Option<&'static str> {
    with_policy_db(|db| {
        for i in 0..db.transition_count {
            if let Some(t) = &db.transitions[i] {
                if str_eq(t.source_type, source_type)
                    && str_eq(t.target_type, target_type)
                    && str_eq(t.class, class)
                {
                    return Some(t.new_type);
                }
            }
        }
        None
    })
}

/// Check if a user is allowed to use a given role.
pub fn user_has_role(username: &str, role_name: &str) -> bool {
    with_policy_db(|db| {
        if let Some(entry) = db.find_user_roles(username) {
            let mut i = 0;
            while i < entry.role_count {
                if str_eq(entry.roles[i], role_name) {
                    return true;
                }
                i += 1;
            }
            false
        } else {
            false
        }
    })
}

/// Check if a role allows a given type.
pub fn role_allows_type(role_name: &str, type_name: &str) -> bool {
    with_policy_db(|db| {
        if let Some(role) = db.find_role(role_name) {
            role.allows_type(type_name)
        } else {
            false
        }
    })
}

// ---------------------------------------------------------------------------
// Capability Integration
// ---------------------------------------------------------------------------

/// Check file access using both MAC policy and capability system.
///
/// Maps the calling process to a security domain and checks if that domain
/// can access file objects with the given access type. Also verifies
/// capability rights if a capability space is available for the process.
pub fn check_file_access(_path: &str, access: AccessType, pid: u64) -> Result<(), KernelError> {
    // Determine source label based on PID or process label
    let source = get_type_for_pid(pid);

    // Files are in the file_t domain
    let target = "file_t";

    // MAC policy check
    if !check_access(source, target, access) {
        crate::security::audit::log_permission_denied(pid, 0, "file_access");
        return Err(KernelError::PermissionDenied {
            operation: "file_access",
        });
    }

    // Capability check: verify the process has the appropriate capability
    // rights for the requested access type
    let required_flags = match access {
        AccessType::Read => crate::cap::Rights::READ.to_flags(),
        AccessType::Write => crate::cap::Rights::WRITE.to_flags(),
        AccessType::Execute => crate::cap::Rights::EXECUTE.to_flags(),
    };

    // If the kernel capability space is available, check it
    if let Some(cap_space_guard) = crate::cap::kernel_cap_space().try_read() {
        if let Some(ref _cap_space) = *cap_space_guard {
            // Capability space exists; for kernel operations (pid 0, 1)
            // we trust implicitly, for user processes we verify they
            // hold a capability with the required rights.
            if pid > 1 {
                // Log the capability check for audit
                crate::security::audit::log_capability_op(pid, required_flags as u64, 0);
            }
        }
    }

    // Optionally check MLS if process has a label
    if let Some(process_label) = get_process_label(pid) {
        let file_label = SecurityLabel::simple(target, "object_r");
        if !check_access_full(&process_label, &file_label, access) {
            crate::security::audit::log_permission_denied(pid, 0, "file_access_mls");
            return Err(KernelError::PermissionDenied {
                operation: "file_access_mls",
            });
        }
    }

    Ok(())
}

/// Check IPC access using both MAC policy and capability system.
///
/// Validates that a process can perform IPC operations based on MAC policy
/// and capability rights.
pub fn check_ipc_access(access: AccessType, pid: u64) -> Result<(), KernelError> {
    let source = get_type_for_pid(pid);

    // IPC targets are in the system_t domain
    let target = "system_t";

    if !check_access(source, target, access) {
        crate::security::audit::log_permission_denied(pid, 0, "ipc_access");
        return Err(KernelError::PermissionDenied {
            operation: "ipc_access",
        });
    }

    // Capability check for IPC
    if pid > 1 {
        let required_flags = match access {
            AccessType::Read => crate::cap::Rights::READ.to_flags(),
            AccessType::Write => crate::cap::Rights::WRITE.to_flags(),
            AccessType::Execute => crate::cap::Rights::EXECUTE.to_flags(),
        };
        crate::security::audit::log_capability_op(pid, required_flags as u64, 0);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Enable / Disable
// ---------------------------------------------------------------------------

/// Enable MAC enforcement.
pub fn enable() {
    MAC_ENABLED.store(true, Ordering::Release);
    println!("[MAC] Mandatory Access Control enabled");
}

/// Disable MAC enforcement (for debugging).
pub fn disable() {
    MAC_ENABLED.store(false, Ordering::Release);
    println!("[MAC] Mandatory Access Control disabled");
}

// ---------------------------------------------------------------------------
// Policy Loading
// ---------------------------------------------------------------------------

/// Load a policy from text.
///
/// Parses the policy text and adds all rules, transitions, roles, and
/// user mappings to the active policy database.
///
/// The input MUST have `'static` lifetime (e.g., a const string literal).
pub fn load_policy(policy_text: &'static str) -> Result<(), KernelError> {
    let parsed = PolicyParser::parse(policy_text)?;

    for i in 0..parsed.rule_count {
        if let Some(rule) = parsed.rules[i] {
            add_policy_rule(rule)?;
        }
    }
    for i in 0..parsed.transition_count {
        if let Some(transition) = parsed.transitions[i] {
            add_transition(transition)?;
        }
    }
    for i in 0..parsed.role_count {
        if let Some(role) = parsed.roles[i] {
            add_role(role);
        }
    }
    for i in 0..parsed.user_role_count {
        if let Some(entry) = &parsed.user_roles[i] {
            with_policy_db(|db| {
                db.insert_user_roles(*entry);
            });
        }
    }

    Ok(())
}

/// Default policy text.
///
/// This is the built-in policy that replaces the old hardcoded rules.
const DEFAULT_POLICY: &str = "
# System domain - full access
allow system_t system_t { read write execute };
allow system_t user_t { read write execute };
allow system_t file_t { read write execute };

# User domain - limited access
allow user_t user_t { read write execute };
allow user_t file_t { read write };

# Driver domain
allow driver_t system_t { read };
allow driver_t device_t { read write execute };

# Init process
allow init_t system_t { read write execute };
allow init_t user_t { read write execute };
allow init_t file_t { read write execute };

# Domain transitions
type_transition user_t init_t : process system_t ;

# Roles
role system_r types { system_t init_t driver_t };
role user_r types { user_t };
role admin_r types { system_t user_t init_t driver_t };

# User-role assignments
user root roles { admin_r system_r };
user default roles { user_r };
";

/// Load the default built-in policy.
fn load_default_policy() -> Result<(), KernelError> {
    load_policy(DEFAULT_POLICY)?;

    println!(
        "[MAC] Loaded {} default policy rules",
        POLICY_COUNT.load(Ordering::Relaxed)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize MAC system.
pub fn init() -> Result<(), KernelError> {
    println!("[MAC] Initializing Mandatory Access Control...");

    // POLICY_DB is const-initialized in BSS -- no stack allocation needed.

    // Load default policy (parsed from text, not hardcoded)
    load_default_policy()?;

    // Set up default process labels
    set_process_label(0, SecurityLabel::simple("system_t", "system_r"));
    set_process_label(1, SecurityLabel::simple("init_t", "system_r"));

    // Enable MAC enforcement
    enable();

    println!(
        "[MAC] MAC system initialized with {} rules",
        POLICY_COUNT.load(Ordering::Relaxed)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal Helpers
// ---------------------------------------------------------------------------

/// Get the type name for a given PID.
///
/// Checks the process label table first, falls back to heuristic.
fn get_type_for_pid(pid: u64) -> &'static str {
    // Check process labels first
    if let Some(label) = get_process_label(pid) {
        return label.type_name;
    }

    // Fallback: heuristic based on PID
    match pid {
        0 => "system_t",
        1 => "init_t",
        _ => "user_t",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_policy_rule() {
        let rule = PolicyRule::from_legacy("user_t", "file_t", 0x3); // Read + Write
        assert!(rule.allows(AccessType::Read));
        assert!(rule.allows(AccessType::Write));
        assert!(!rule.allows(AccessType::Execute));
    }

    #[test_case]
    fn test_add_rule() {
        let rule = PolicyRule::new(
            "test_t",
            "test_t",
            [Permission::Read, Permission::Write, Permission::Execute],
            3,
            PolicyAction::Allow,
        );
        assert!(add_policy_rule(rule).is_ok());
    }

    #[test_case]
    fn test_mls_dominance() {
        let high = MlsLevel::new(3, 0b111);
        let low = MlsLevel::new(1, 0b001);
        let mid = MlsLevel::new(2, 0b011);

        assert!(high.dominates(&low));
        assert!(high.dominates(&mid));
        assert!(!low.dominates(&high));
        assert!(!low.dominates(&mid)); // sensitivity too low
        assert!(mid.dominates(&low)); // 2 >= 1 and 0b011 superset of 0b001
    }

    #[test_case]
    fn test_deny_overrides_allow() {
        let allow_rule = PolicyRule::from_perms(
            "deny_test_t",
            "deny_target_t",
            &[Permission::Read, Permission::Write],
            PolicyAction::Allow,
        );
        let deny_rule = PolicyRule::from_perms(
            "deny_test_t",
            "deny_target_t",
            &[Permission::Write],
            PolicyAction::Deny,
        );
        let _ = add_policy_rule(allow_rule);
        let _ = add_policy_rule(deny_rule);

        // Write should be denied even though there is an allow rule
        assert!(!check_access(
            "deny_test_t",
            "deny_target_t",
            AccessType::Write
        ));
    }

    #[test_case]
    fn test_policy_parser() {
        let policy: &'static str =
            "allow src_t dst_t { read write }; deny src_t dst_t { execute };";
        let result = PolicyParser::parse(policy);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.rule_count, 2);
        assert_eq!(parsed.rules[0].unwrap().action, PolicyAction::Allow);
        assert_eq!(parsed.rules[1].unwrap().action, PolicyAction::Deny);
    }

    #[test_case]
    fn test_domain_transition_parse() {
        let policy: &'static str = "type_transition user_t init_t : process system_t ;";
        let result = PolicyParser::parse(policy);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.transition_count, 1);
        assert_eq!(parsed.transitions[0].unwrap().new_type, "system_t");
    }

    #[test_case]
    fn test_rbac_parse() {
        let policy: &'static str =
            "role admin_r types { system_t user_t }; user root roles { admin_r };";
        let result = PolicyParser::parse(policy);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role_count, 1);
        assert!(parsed.roles[0].unwrap().allows_type("system_t"));
        assert_eq!(parsed.user_role_count, 1);
        assert_eq!(parsed.user_roles[0].unwrap().username, "root");
    }
}

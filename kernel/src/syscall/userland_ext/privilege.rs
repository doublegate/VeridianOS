//! sudo/su Privilege Elevation
//!
//! Implements sudoers-based privilege elevation, su user switching,
//! and session management for privilege escalation.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

// ============================================================================
// Constants
// ============================================================================

/// Root UID
const ROOT_UID: u32 = 0;
/// Default shell path
const DEFAULT_SHELL: &str = "/bin/vsh";

// ============================================================================
// Error Types
// ============================================================================

/// Privilege elevation errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeError {
    /// User not found
    UserNotFound,
    /// Permission denied by sudoers
    PermissionDenied,
    /// Authentication failed (bad password)
    AuthFailed,
    /// sudoers parse error
    ParseError,
    /// Session expired
    SessionExpired,
    /// Target user not found
    TargetUserNotFound,
    /// Operation not permitted
    NotPermitted,
    /// Internal error
    InternalError,
}

// ============================================================================
// Sudoers Rule
// ============================================================================

/// Sudoers rule specification
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoersRule {
    /// User or group specification (user name, or %group)
    pub user_spec: String,
    /// Host specification (ALL or hostname)
    pub host_spec: String,
    /// Runas specification (user to run as, ALL = any)
    pub runas_spec: String,
    /// Command specification (ALL or specific path)
    pub command_spec: String,
    /// Whether NOPASSWD is set
    pub nopasswd: bool,
    /// Whether SETENV is allowed
    pub setenv: bool,
}

impl SudoersRule {
    /// Create a standard rule
    pub fn new(user_spec: &str, host_spec: &str, runas_spec: &str, command_spec: &str) -> Self {
        Self {
            user_spec: String::from(user_spec),
            host_spec: String::from(host_spec),
            runas_spec: String::from(runas_spec),
            command_spec: String::from(command_spec),
            nopasswd: false,
            setenv: false,
        }
    }

    /// Create a NOPASSWD rule
    pub fn new_nopasswd(
        user_spec: &str,
        host_spec: &str,
        runas_spec: &str,
        command_spec: &str,
    ) -> Self {
        let mut rule = Self::new(user_spec, host_spec, runas_spec, command_spec);
        rule.nopasswd = true;
        rule
    }

    /// Check if this rule matches a user (direct or group membership)
    pub fn matches_user(&self, username: &str, groups: &[String]) -> bool {
        if self.user_spec == "ALL" || self.user_spec == username {
            return true;
        }
        // Check group match (%group)
        if let Some(group) = self.user_spec.strip_prefix('%') {
            return groups.iter().any(|g| g.as_str() == group);
        }
        false
    }

    /// Check if this rule matches a runas target
    pub fn matches_runas(&self, target_user: &str) -> bool {
        self.runas_spec == "ALL" || self.runas_spec == target_user
    }

    /// Check if this rule matches a command
    pub fn matches_command(&self, command: &str) -> bool {
        if self.command_spec == "ALL" {
            return true;
        }
        // Exact match or prefix match (for paths)
        command == self.command_spec || command.starts_with(&self.command_spec)
    }

    /// Serialize to sudoers format
    pub fn to_sudoers_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.user_spec);
        line.push(' ');
        line.push_str(&self.host_spec);
        line.push_str("=(");
        line.push_str(&self.runas_spec);
        line.push_str(") ");
        if self.nopasswd {
            line.push_str("NOPASSWD: ");
        }
        if self.setenv {
            line.push_str("SETENV: ");
        }
        line.push_str(&self.command_spec);
        line
    }
}

// ============================================================================
// Sudoers Parser
// ============================================================================

/// Sudoers parser and validator
#[derive(Debug)]
pub struct SudoersParser {
    /// Parsed rules
    pub rules: Vec<SudoersRule>,
    /// Host aliases (name -> hostnames)
    pub host_aliases: BTreeMap<String, Vec<String>>,
    /// User aliases (name -> usernames)
    pub user_aliases: BTreeMap<String, Vec<String>>,
    /// Command aliases (name -> commands)
    pub cmnd_aliases: BTreeMap<String, Vec<String>>,
    /// Defaults settings
    pub defaults: Vec<String>,
}

impl Default for SudoersParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SudoersParser {
    /// Create a new empty sudoers parser
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            host_aliases: BTreeMap::new(),
            user_aliases: BTreeMap::new(),
            cmnd_aliases: BTreeMap::new(),
            defaults: Vec::new(),
        }
    }

    /// Parse a sudoers file content
    pub fn parse(&mut self, content: &str) -> Result<usize, PrivilegeError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with("Defaults") {
                self.defaults.push(String::from(trimmed));
                count += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Host_Alias") {
                let mut aliases = self.host_aliases.clone();
                self.parse_alias(rest.trim(), &mut aliases)?;
                self.host_aliases = aliases;
                count += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("User_Alias") {
                let mut aliases = self.user_aliases.clone();
                self.parse_alias(rest.trim(), &mut aliases)?;
                self.user_aliases = aliases;
                count += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Cmnd_Alias") {
                let mut aliases = self.cmnd_aliases.clone();
                self.parse_alias(rest.trim(), &mut aliases)?;
                self.cmnd_aliases = aliases;
                count += 1;
                continue;
            }
            // Parse user rule
            if let Some(rule) = self.parse_rule(trimmed) {
                self.rules.push(rule);
                count += 1;
            }
        }
        Ok(count)
    }

    /// Parse an alias definition (NAME = val1, val2, ...)
    fn parse_alias(
        &self,
        spec: &str,
        aliases: &mut BTreeMap<String, Vec<String>>,
    ) -> Result<(), PrivilegeError> {
        let parts: Vec<&str> = spec.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err(PrivilegeError::ParseError);
        }
        let name = parts[0].trim();
        let values: Vec<String> = parts[1]
            .split(',')
            .map(|s| String::from(s.trim()))
            .collect();
        aliases.insert(String::from(name), values);
        Ok(())
    }

    /// Parse a single sudoers rule line
    fn parse_rule(&self, line: &str) -> Option<SudoersRule> {
        // Format: user host=(runas) [NOPASSWD:] command
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return None;
        }
        let user_spec = parts[0];
        let rest = parts[1];

        // Find host=(runas) pattern
        let eq_pos = rest.find('=')?;
        let host_spec = rest[..eq_pos].trim();

        let after_eq = rest[eq_pos + 1..].trim();
        let paren_close = after_eq.find(')')?;
        let runas_spec = after_eq[1..paren_close].trim(); // skip opening '('

        let mut cmd_part = after_eq[paren_close + 1..].trim();
        let mut nopasswd = false;
        let mut setenv = false;

        if let Some(rest_after) = cmd_part.strip_prefix("NOPASSWD:") {
            nopasswd = true;
            cmd_part = rest_after.trim();
        }
        if let Some(rest_after) = cmd_part.strip_prefix("SETENV:") {
            setenv = true;
            cmd_part = rest_after.trim();
        }

        let mut rule = SudoersRule::new(user_spec, host_spec, runas_spec, cmd_part);
        rule.nopasswd = nopasswd;
        rule.setenv = setenv;
        Some(rule)
    }

    /// Add a rule programmatically
    pub fn add_rule(&mut self, rule: SudoersRule) {
        self.rules.push(rule);
    }

    /// Get the number of rules
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

// ============================================================================
// Sudo Session
// ============================================================================

/// Sudo session tracking (timestamp-based)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SudoSession {
    /// User UID
    pub uid: u32,
    /// Terminal/TTY identifier
    pub tty: u64,
    /// Timestamp of last successful auth (monotonic, in seconds)
    pub auth_time: u64,
    /// Session timeout in seconds (default: 300 = 5 minutes)
    pub timeout_secs: u64,
}

impl SudoSession {
    /// Create a new session
    pub fn new(uid: u32, tty: u64, auth_time: u64) -> Self {
        Self {
            uid,
            tty,
            auth_time,
            timeout_secs: 300,
        }
    }

    /// Check if the session is still valid at the given time
    pub fn is_valid(&self, current_time: u64) -> bool {
        current_time.saturating_sub(self.auth_time) < self.timeout_secs
    }

    /// Refresh the session timestamp
    pub fn refresh(&mut self, current_time: u64) {
        self.auth_time = current_time;
    }
}

// ============================================================================
// Result Types
// ============================================================================

/// Result of a successful sudo execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoExecResult {
    /// Target UID
    pub target_uid: u32,
    /// Target GID
    pub target_gid: u32,
    /// Command to execute
    pub command: String,
    /// Sanitized environment
    pub environment: Vec<(String, String)>,
}

/// Result of a successful su switch
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuSwitchResult {
    /// Target user
    pub target_user: String,
    /// Sanitized environment
    pub environment: Vec<(String, String)>,
}

// ============================================================================
// Privilege Manager
// ============================================================================

/// Privilege manager for sudo/su operations
#[derive(Debug)]
pub struct PrivilegeManager {
    /// Sudoers configuration
    pub sudoers: SudoersParser,
    /// Active sudo sessions (uid -> session)
    sessions: BTreeMap<u32, SudoSession>,
    /// Sanitized environment variable names to keep
    env_keep: Vec<String>,
    /// Environment variables to reset
    env_reset: Vec<(String, String)>,
}

impl Default for PrivilegeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivilegeManager {
    /// Create a new privilege manager
    pub fn new() -> Self {
        let env_keep = vec![
            String::from("TERM"),
            String::from("LANG"),
            String::from("LC_ALL"),
        ];
        let env_reset = vec![
            (
                String::from("PATH"),
                String::from("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"),
            ),
            (String::from("HOME"), String::new()),
            (String::from("USER"), String::new()),
            (String::from("LOGNAME"), String::new()),
            (String::from("SHELL"), String::new()),
        ];
        Self {
            sudoers: SudoersParser::new(),
            sessions: BTreeMap::new(),
            env_keep,
            env_reset,
        }
    }

    /// Check if a user has sudo permission for a command
    pub fn check_sudo_permission(
        &self,
        username: &str,
        groups: &[String],
        target_user: &str,
        command: &str,
    ) -> Result<bool, PrivilegeError> {
        for rule in &self.sudoers.rules {
            if rule.matches_user(username, groups)
                && rule.matches_runas(target_user)
                && rule.matches_command(command)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check if NOPASSWD is set for a matching rule
    pub fn is_nopasswd(
        &self,
        username: &str,
        groups: &[String],
        target_user: &str,
        command: &str,
    ) -> bool {
        for rule in &self.sudoers.rules {
            if rule.matches_user(username, groups)
                && rule.matches_runas(target_user)
                && rule.matches_command(command)
            {
                return rule.nopasswd;
            }
        }
        false
    }

    /// Execute sudo: validate permissions, optionally authenticate, switch
    /// context
    #[allow(clippy::too_many_arguments)]
    pub fn sudo_exec(
        &mut self,
        uid: u32,
        username: &str,
        groups: &[String],
        target_user: &str,
        command: &str,
        password_hash: Option<&str>,
        current_time: u64,
        tty: u64,
    ) -> Result<SudoExecResult, PrivilegeError> {
        // Check permission
        let has_perm = self.check_sudo_permission(username, groups, target_user, command)?;
        if !has_perm {
            return Err(PrivilegeError::PermissionDenied);
        }

        // Check if NOPASSWD or session is still valid
        let nopasswd = self.is_nopasswd(username, groups, target_user, command);
        let session_valid = self
            .sessions
            .get(&uid)
            .map(|s| s.tty == tty && s.is_valid(current_time))
            .unwrap_or(false);

        if !nopasswd && !session_valid {
            // Need password authentication
            match password_hash {
                Some(_hash) => {
                    // Stub: in real implementation, verify against shadow
                    // database For now, accept any provided
                    // hash
                }
                None => {
                    return Err(PrivilegeError::AuthFailed);
                }
            }
            // Create/refresh session
            let session = SudoSession::new(uid, tty, current_time);
            self.sessions.insert(uid, session);
        } else if session_valid {
            // Refresh existing session
            if let Some(session) = self.sessions.get_mut(&uid) {
                session.refresh(current_time);
            }
        }

        // Build sanitized environment
        let env = self.build_sanitized_env(target_user);

        Ok(SudoExecResult {
            target_uid: 0, // caller resolves via user database
            target_gid: 0,
            command: String::from(command),
            environment: env,
        })
    }

    /// Switch user (su) - simpler than sudo, always requires auth for non-root
    pub fn su_switch(
        &self,
        caller_uid: u32,
        target_user: &str,
        password_hash: Option<&str>,
    ) -> Result<SuSwitchResult, PrivilegeError> {
        // Root can su without password
        if caller_uid != ROOT_UID {
            match password_hash {
                Some(_hash) => {
                    // Stub: verify against shadow database
                }
                None => {
                    return Err(PrivilegeError::AuthFailed);
                }
            }
        }

        let env = self.build_sanitized_env(target_user);

        Ok(SuSwitchResult {
            target_user: String::from(target_user),
            environment: env,
        })
    }

    /// Build sanitized environment for privilege elevation
    fn build_sanitized_env(&self, target_user: &str) -> Vec<(String, String)> {
        let mut env = Vec::new();
        for (key, value) in &self.env_reset {
            let val = if key == "USER" || key == "LOGNAME" {
                String::from(target_user)
            } else if key == "HOME" {
                let mut home = String::from("/home/");
                home.push_str(target_user);
                home
            } else if key == "SHELL" {
                String::from(DEFAULT_SHELL)
            } else {
                value.clone()
            };
            env.push((key.clone(), val));
        }
        env
    }

    /// Invalidate a sudo session
    pub fn invalidate_session(&mut self, uid: u32) {
        self.sessions.remove(&uid);
    }

    /// Invalidate all sessions
    pub fn invalidate_all_sessions(&mut self) {
        self.sessions.clear();
    }

    /// Get number of active sessions
    pub fn active_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// PBKDF2-like stub for password hashing (integer-only)
    ///
    /// In production, use a proper PBKDF2/argon2 implementation.
    /// This is a simplified hash for development purposes.
    pub fn hash_password_stub(password: &[u8], salt: &[u8], iterations: u32) -> u64 {
        let mut hash: u64 = 0x517cc1b727220a95;
        for _ in 0..iterations {
            for &b in password {
                hash = hash.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
            }
            for &b in salt {
                hash = hash.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
            }
            hash ^= hash >> 33;
            hash = hash.wrapping_mul(0xff51afd7ed558ccd);
            hash ^= hash >> 33;
        }
        hash
    }
}

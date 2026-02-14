//! Authentication Framework
//!
//! Provides user authentication, password hashing, and multi-factor
//! authentication.

#![allow(static_mut_refs)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use spin::RwLock;

use crate::{
    crypto::hash::{sha256, Hash256},
    error::KernelError,
};

/// User identifier
pub type UserId = u32;

/// Authentication result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResult {
    Success,
    InvalidCredentials,
    AccountLocked,
    PasswordExpired,
    MfaRequired,
    Denied,
}

/// User credential
#[derive(Debug, Clone)]
pub struct Credential {
    pub username: String,
    pub password_hash: Hash256,
    pub salt: [u8; 32],
}

/// User account information
#[derive(Debug, Clone)]
pub struct UserAccount {
    pub user_id: UserId,
    pub username: String,
    pub password_hash: Hash256,
    pub salt: [u8; 32],
    pub locked: bool,
    pub failed_attempts: u32,
    pub mfa_enabled: bool,
    pub mfa_secret: Option<[u8; 32]>,
}

impl UserAccount {
    /// Create new user account
    pub fn new(user_id: UserId, username: String, password: &str) -> Self {
        let (password_hash, salt) = Self::hash_password(password);

        Self {
            user_id,
            username,
            password_hash,
            salt,
            locked: false,
            failed_attempts: 0,
            mfa_enabled: false,
            mfa_secret: None,
        }
    }

    /// Hash password with salt using Argon2-like construction (simplified)
    fn hash_password(password: &str) -> (Hash256, [u8; 32]) {
        use crate::crypto::random::get_random;

        let rng = get_random();
        let mut salt = [0u8; 32];
        let _ = rng.fill_bytes(&mut salt);

        // Simplified password hashing (in production, use Argon2id)
        // For now, we'll do multiple rounds of SHA-256 with salt
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(password.as_bytes());
        hash_input.extend_from_slice(&salt);

        let mut current_hash = sha256(&hash_input);

        // Key stretching: use few iterations in debug builds (QEMU is slow),
        // full strength in release builds
        #[cfg(debug_assertions)]
        const ITERATIONS: u32 = 10;
        #[cfg(not(debug_assertions))]
        const ITERATIONS: u32 = 10_000;

        for _ in 0..ITERATIONS {
            let mut next_input = Vec::new();
            next_input.extend_from_slice(current_hash.as_bytes());
            next_input.extend_from_slice(&salt);
            current_hash = sha256(&next_input);
        }

        (current_hash, salt)
    }

    /// Verify password
    pub fn verify_password(&self, password: &str) -> bool {
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(password.as_bytes());
        hash_input.extend_from_slice(&self.salt);

        let mut current_hash = sha256(&hash_input);

        #[cfg(debug_assertions)]
        const ITERATIONS: u32 = 10;
        #[cfg(not(debug_assertions))]
        const ITERATIONS: u32 = 10_000;

        for _ in 0..ITERATIONS {
            let mut next_input = Vec::new();
            next_input.extend_from_slice(current_hash.as_bytes());
            next_input.extend_from_slice(&self.salt);
            current_hash = sha256(&next_input);
        }

        current_hash == self.password_hash
    }

    /// Enable MFA for this account
    pub fn enable_mfa(&mut self) -> [u8; 32] {
        use crate::crypto::random::get_random;

        let rng = get_random();
        let mut secret = [0u8; 32];
        let _ = rng.fill_bytes(&mut secret);

        self.mfa_secret = Some(secret);
        self.mfa_enabled = true;

        secret
    }

    /// Verify MFA token (TOTP-like)
    pub fn verify_mfa_token(&self, token: u32) -> bool {
        if !self.mfa_enabled {
            return true; // MFA not required
        }

        if let Some(secret) = self.mfa_secret {
            // Simplified TOTP verification
            // In production, use proper TOTP implementation
            let time_step = 30; // 30 second windows
            let current_time = 0u64; // TODO(phase3): Get actual system time from clock subsystem

            let time_counter = current_time / time_step;

            // Generate expected token from secret and time
            let mut input = Vec::new();
            input.extend_from_slice(&secret);
            input.extend_from_slice(&time_counter.to_le_bytes());

            let hash = sha256(&input);
            let expected_token = u32::from_le_bytes([
                hash.as_bytes()[0],
                hash.as_bytes()[1],
                hash.as_bytes()[2],
                hash.as_bytes()[3],
            ]) % 1000000; // 6-digit token

            token == expected_token
        } else {
            false
        }
    }
}

/// Authentication manager
pub struct AuthManager {
    accounts: RwLock<BTreeMap<String, UserAccount>>,
    next_user_id: RwLock<u32>,
    max_failed_attempts: u32,
}

impl AuthManager {
    /// Create new authentication manager
    pub fn new() -> Self {
        Self {
            accounts: RwLock::new(BTreeMap::new()),
            next_user_id: RwLock::new(1000), // Start UIDs at 1000
            max_failed_attempts: 5,
        }
    }

    /// Create new user account
    pub fn create_user(&self, username: String, password: &str) -> Result<UserId, KernelError> {
        let mut accounts = self.accounts.write();

        // Check if username already exists
        if accounts.contains_key(&username) {
            return Err(KernelError::AlreadyExists {
                resource: "user",
                id: 0, // Username lookup, no specific ID
            });
        }

        // Allocate new user ID
        let user_id = {
            let mut next_id = self.next_user_id.write();
            let id = *next_id;
            *next_id += 1;
            id
        };

        // Create account
        let account = UserAccount::new(user_id, username.clone(), password);

        accounts.insert(username, account);

        Ok(user_id)
    }

    /// Authenticate user
    pub fn authenticate(&self, username: &str, password: &str) -> AuthResult {
        let mut accounts = self.accounts.write();

        if let Some(account) = accounts.get_mut(username) {
            // Check if account is locked
            if account.locked {
                return AuthResult::AccountLocked;
            }

            // Verify password
            if account.verify_password(password) {
                // Reset failed attempts on successful login
                account.failed_attempts = 0;

                // Check if MFA is required
                if account.mfa_enabled {
                    return AuthResult::MfaRequired;
                }

                return AuthResult::Success;
            } else {
                // Increment failed attempts
                account.failed_attempts += 1;

                // Lock account if max attempts exceeded
                if account.failed_attempts >= self.max_failed_attempts {
                    account.locked = true;
                    return AuthResult::AccountLocked;
                }

                return AuthResult::InvalidCredentials;
            }
        }

        AuthResult::InvalidCredentials
    }

    /// Authenticate with MFA
    pub fn authenticate_mfa(&self, username: &str, password: &str, mfa_token: u32) -> AuthResult {
        // First verify password
        let result = self.authenticate(username, password);

        if result != AuthResult::MfaRequired {
            return result;
        }

        // Verify MFA token
        let accounts = self.accounts.read();
        if let Some(account) = accounts.get(username) {
            if account.verify_mfa_token(mfa_token) {
                return AuthResult::Success;
            }
        }

        AuthResult::InvalidCredentials
    }

    /// Enable MFA for user
    pub fn enable_mfa(&self, username: &str) -> Result<[u8; 32], KernelError> {
        let mut accounts = self.accounts.write();

        if let Some(account) = accounts.get_mut(username) {
            Ok(account.enable_mfa())
        } else {
            Err(KernelError::NotFound {
                resource: "user",
                id: 0, // Username lookup, no specific ID
            })
        }
    }

    /// Unlock user account
    pub fn unlock_account(&self, username: &str) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();

        if let Some(account) = accounts.get_mut(username) {
            account.locked = false;
            account.failed_attempts = 0;
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "user",
                id: 0, // Username lookup, no specific ID
            })
        }
    }

    /// Delete user account
    pub fn delete_user(&self, username: &str) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();

        accounts
            .remove(username)
            .map(|_| ())
            .ok_or(KernelError::NotFound {
                resource: "user",
                id: 0, // Username lookup, no specific ID
            })
    }

    /// List all usernames
    pub fn list_users(&self) -> Vec<String> {
        self.accounts.read().keys().cloned().collect()
    }

    /// Get user by ID
    pub fn get_user_by_id(&self, user_id: UserId) -> Option<String> {
        let accounts = self.accounts.read();

        for account in accounts.values() {
            if account.user_id == user_id {
                return Some(account.username.clone());
            }
        }

        None
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global authentication manager
static mut AUTH_MANAGER: Option<AuthManager> = None;

/// Initialize authentication framework
pub fn init() -> Result<(), KernelError> {
    // SAFETY: AUTH_MANAGER is a static mut Option written once during
    // single-threaded kernel init. No concurrent access is possible at this
    // point in kernel bootstrap.
    unsafe {
        AUTH_MANAGER = Some(AuthManager::new());
    }

    // Create default root account
    let auth_manager = get_auth_manager();
    let _ = auth_manager.create_user(String::from("root"), "veridian");

    crate::println!("[AUTH] Authentication framework initialized");
    crate::println!("[AUTH] Default root user created (password: veridian)");

    Ok(())
}

/// Get global authentication manager
pub fn get_auth_manager() -> &'static AuthManager {
    // SAFETY: AUTH_MANAGER is set once during init() and the returned reference has
    // 'static lifetime because the static mut is never moved or dropped. The
    // AuthManager uses internal RwLock synchronization for thread-safe access
    // to accounts.
    unsafe { AUTH_MANAGER.as_ref().expect("Auth manager not initialized") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_password_hashing() {
        let account = UserAccount::new(1000, String::from("test"), "password123");

        assert!(account.verify_password("password123"));
        assert!(!account.verify_password("wrongpassword"));
    }

    #[test_case]
    fn test_authentication() {
        let auth = AuthManager::new();

        let _ = auth.create_user(String::from("alice"), "secret");

        assert_eq!(auth.authenticate("alice", "secret"), AuthResult::Success);
        assert_eq!(
            auth.authenticate("alice", "wrong"),
            AuthResult::InvalidCredentials
        );
        assert_eq!(
            auth.authenticate("bob", "secret"),
            AuthResult::InvalidCredentials
        );
    }

    #[test_case]
    fn test_account_locking() {
        let auth = AuthManager::new();

        let _ = auth.create_user(String::from("bob"), "password");

        // Try wrong password multiple times
        for _ in 0..5 {
            let _ = auth.authenticate("bob", "wrong");
        }

        // Account should now be locked
        assert_eq!(
            auth.authenticate("bob", "password"),
            AuthResult::AccountLocked
        );
    }
}

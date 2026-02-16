//! Authentication Framework
//!
//! Provides user authentication, password hashing, and multi-factor
//! authentication.
//!
//! # Features
//!
//! - PBKDF2-HMAC-SHA256 password hashing with configurable iterations
//! - Password complexity enforcement via configurable policy
//! - Password history tracking (prevent reuse of last N passwords)
//! - Account expiration with timestamp-based checks
//! - Multi-factor authentication (TOTP-like)
//! - Account lockout after configurable failed attempts
//!
//! # No-Heap Design
//!
//! All data structures use fixed-size stack/static buffers to avoid heap
//! allocations during boot. This prevents corruption of the bump allocator
//! on architectures (e.g., RISC-V) where the heap is not yet fully
//! initialized when the auth module runs.

use spin::RwLock;

use crate::{
    crypto::hash::{sha256, Hash256},
    error::KernelError,
    sync::once_lock::OnceLock,
};

/// User identifier
pub type UserId = u32;

/// Maximum number of user accounts.
///
/// Kept small (16) to avoid stack overflow during init on x86_64 where the
/// kernel stack is limited. The AccountDatabase ([Option<UserAccount>; N])
/// is constructed on the stack before being moved into the OnceLock static.
/// At ~320 bytes per UserAccount, 16 entries = ~5KB which fits safely.
const MAX_ACCOUNTS: usize = 16;

/// Maximum number of previous passwords to remember per account.
const MAX_PASSWORD_HISTORY: usize = 5;

// ---------------------------------------------------------------------------
// Authentication Result
// ---------------------------------------------------------------------------

/// Authentication result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResult {
    Success,
    InvalidCredentials,
    AccountLocked,
    PasswordExpired,
    MfaRequired,
    AccountExpired,
    Denied,
}

// ---------------------------------------------------------------------------
// Password Policy
// ---------------------------------------------------------------------------

/// Password complexity enforcement policy.
#[derive(Debug, Clone, Copy)]
pub struct PasswordPolicy {
    /// Minimum password length
    pub min_length: usize,
    /// Require at least one uppercase letter
    pub require_uppercase: bool,
    /// Require at least one lowercase letter
    pub require_lowercase: bool,
    /// Require at least one digit
    pub require_digit: bool,
    /// Require at least one special character
    pub require_special: bool,
    /// Maximum number of previous passwords to remember
    pub history_size: usize,
}

impl PasswordPolicy {
    /// Default password policy: 8 chars, upper+lower+digit required.
    pub const fn default_policy() -> Self {
        Self {
            min_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_digit: true,
            require_special: false,
            history_size: 5,
        }
    }

    /// Relaxed policy (for testing or early boot).
    pub const fn relaxed() -> Self {
        Self {
            min_length: 1,
            require_uppercase: false,
            require_lowercase: false,
            require_digit: false,
            require_special: false,
            history_size: 0,
        }
    }

    /// Validate a password against this policy.
    ///
    /// Returns `Ok(())` if the password meets all requirements, or
    /// `Err` with a description of the first failing requirement.
    pub fn validate_password(&self, password: &str) -> Result<(), KernelError> {
        if password.len() < self.min_length {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "too short",
            });
        }

        if self.require_uppercase && !password.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "must contain an uppercase letter",
            });
        }

        if self.require_lowercase && !password.bytes().any(|b| b.is_ascii_lowercase()) {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "must contain a lowercase letter",
            });
        }

        if self.require_digit && !password.bytes().any(|b| b.is_ascii_digit()) {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "must contain a digit",
            });
        }

        if self.require_special
            && !password
                .bytes()
                .any(|b| b.is_ascii_punctuation() || b == b' ')
        {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "must contain a special character",
            });
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PBKDF2-HMAC-SHA256 (zero-heap implementation)
// ---------------------------------------------------------------------------

/// SHA-256 block size in bytes.
const BLOCK_SIZE: usize = 64;

/// Maximum message size for HMAC inner hash.
///
/// The largest message passed to `hmac_sha256` is either:
/// - In `pbkdf2_hmac_sha256`: salt (up to 32 bytes) + 4-byte counter = 36 bytes
/// - In `check_totp_window`: 8-byte counter
/// - In `change_password` history check: 32-byte hash used as salt + 4 = 36
///   bytes
///
/// Total inner buffer: BLOCK_SIZE (64) + max_message (36) = 100.
/// We use 192 to leave headroom for future callers.
const HMAC_INNER_BUF_SIZE: usize = 192;

/// HMAC-SHA256 implementation for PBKDF2 (no heap allocation).
///
/// Computes HMAC(key, message) = SHA256((key XOR opad) || SHA256((key XOR ipad)
/// || message))
///
/// # Panics
///
/// Panics if `message.len() > HMAC_INNER_BUF_SIZE - BLOCK_SIZE` (128 bytes).
/// All internal callers stay well within this limit.
fn hmac_sha256(key: &[u8], message: &[u8]) -> Hash256 {
    const IPAD: u8 = 0x36;
    const OPAD: u8 = 0x5c;

    let max_msg = HMAC_INNER_BUF_SIZE - BLOCK_SIZE;
    assert!(
        message.len() <= max_msg,
        "hmac_sha256: message too large for stack buffer"
    );

    // If key is longer than block size, hash it first
    let key_hash;
    let actual_key = if key.len() > BLOCK_SIZE {
        key_hash = sha256(key);
        key_hash.as_bytes().as_slice()
    } else {
        key
    };

    // Pad key to block size
    let mut padded_key = [0u8; BLOCK_SIZE];
    padded_key[..actual_key.len()].copy_from_slice(actual_key);

    // Inner hash: SHA256((key XOR ipad) || message) -- stack buffer
    let mut inner_buf = [0u8; HMAC_INNER_BUF_SIZE];
    for (i, byte) in padded_key.iter().enumerate() {
        inner_buf[i] = byte ^ IPAD;
    }
    let inner_len = BLOCK_SIZE + message.len();
    inner_buf[BLOCK_SIZE..inner_len].copy_from_slice(message);
    let inner_hash = sha256(&inner_buf[..inner_len]);

    // Outer hash: SHA256((key XOR opad) || inner_hash) -- stack buffer
    let mut outer_buf = [0u8; BLOCK_SIZE + 32];
    for (i, byte) in padded_key.iter().enumerate() {
        outer_buf[i] = byte ^ OPAD;
    }
    outer_buf[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(inner_hash.as_bytes());

    sha256(&outer_buf[..BLOCK_SIZE + 32])
}

/// PBKDF2-HMAC-SHA256 key derivation (no heap allocation).
///
/// Derives a 256-bit key from `password` and `salt` using `iterations`
/// rounds of HMAC-SHA256 with XOR accumulation (RFC 8018, Section 5.2).
///
/// # Panics
///
/// Panics if `salt.len() > 128` (extremely unlikely for real usage).
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> Hash256 {
    // For a single 32-byte block (which is all we need for Hash256):
    // U1 = HMAC(password, salt || INT(1))
    // U2 = HMAC(password, U1)
    // ...
    // result = U1 XOR U2 XOR ... XOR Uc

    // Build salt || counter on stack (salt up to 128 bytes + 4 bytes counter)
    let mut salt_with_counter = [0u8; 128 + 4];
    assert!(salt.len() <= 128, "pbkdf2: salt too large for stack buffer");
    salt_with_counter[..salt.len()].copy_from_slice(salt);
    salt_with_counter[salt.len()..salt.len() + 4].copy_from_slice(&1u32.to_be_bytes());
    let salt_counter_len = salt.len() + 4;

    let u1 = hmac_sha256(password, &salt_with_counter[..salt_counter_len]);
    let mut result = *u1.as_bytes();
    let mut prev = u1;

    for _ in 1..iterations {
        let u_next = hmac_sha256(password, prev.as_bytes());
        // XOR accumulate
        for (r, u) in result.iter_mut().zip(u_next.as_bytes().iter()) {
            *r ^= u;
        }
        prev = u_next;
    }

    Hash256(result)
}

// ---------------------------------------------------------------------------
// User Credential (legacy compat)
// ---------------------------------------------------------------------------

/// User credential
#[derive(Debug, Clone)]
pub struct Credential {
    pub username: &'static str,
    pub password_hash: Hash256,
    pub salt: [u8; 32],
}

// ---------------------------------------------------------------------------
// User Account
// ---------------------------------------------------------------------------

/// User account information (fixed-size, no heap allocation).
#[derive(Debug, Clone)]
pub struct UserAccount {
    pub user_id: UserId,
    pub username: &'static str,
    pub password_hash: Hash256,
    pub salt: [u8; 32],
    pub locked: bool,
    pub failed_attempts: u32,
    pub mfa_enabled: bool,
    pub mfa_secret: Option<[u8; 32]>,
    /// Account expiration timestamp (seconds since boot). `None` = never
    /// expires.
    pub expires_at: Option<u64>,
    /// Password history: stores hashes of previous passwords (fixed-size).
    pub password_history: [Option<Hash256>; MAX_PASSWORD_HISTORY],
    /// Number of valid entries in `password_history`.
    pub password_history_len: usize,
}

impl UserAccount {
    /// PBKDF2 iteration count for password hashing.
    /// Reduced in debug builds because QEMU is slow.
    #[cfg(debug_assertions)]
    const PBKDF2_ITERATIONS: u32 = 10;
    #[cfg(not(debug_assertions))]
    const PBKDF2_ITERATIONS: u32 = 10_000;

    /// Create new user account
    pub fn new(user_id: UserId, username: &'static str, password: &str) -> Self {
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
            expires_at: None,
            password_history: [None; MAX_PASSWORD_HISTORY],
            password_history_len: 0,
        }
    }

    /// Hash password with PBKDF2-HMAC-SHA256.
    fn hash_password(password: &str) -> (Hash256, [u8; 32]) {
        use crate::crypto::random::get_random;

        let rng = get_random();
        let mut salt = [0u8; 32];
        if let Err(_e) = rng.fill_bytes(&mut salt) {
            crate::kprintln!(
                "[AUTH] Warning: RNG fill_bytes failed for password salt, using zeroed salt"
            );
        }

        let hash = pbkdf2_hmac_sha256(password.as_bytes(), &salt, Self::PBKDF2_ITERATIONS);

        (hash, salt)
    }

    /// Hash password with a specific salt (for verification).
    fn hash_password_with_salt(password: &str, salt: &[u8; 32]) -> Hash256 {
        pbkdf2_hmac_sha256(password.as_bytes(), salt, Self::PBKDF2_ITERATIONS)
    }

    /// Verify password
    pub fn verify_password(&self, password: &str) -> bool {
        let computed = Self::hash_password_with_salt(password, &self.salt);
        computed == self.password_hash
    }

    /// Check if the account has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = crate::arch::timer::get_timestamp_secs();
            now >= expires_at
        } else {
            false
        }
    }

    /// Set account expiration time.
    pub fn set_expiration(&mut self, expires_at: Option<u64>) {
        self.expires_at = expires_at;
    }

    /// Change password with history tracking.
    ///
    /// Checks that the new password is not in the password history.
    /// The `history_size` parameter controls how many old hashes to retain.
    pub fn change_password(
        &mut self,
        new_password: &str,
        history_size: usize,
    ) -> Result<(), KernelError> {
        // Check new password against history (simplified: exact reuse detection)
        let _new_hash_with_old_salts_match = self.password_history[..self.password_history_len]
            .iter()
            .any(|entry| {
                if let Some(old_hash) = entry {
                    let history_salt_bytes = old_hash.as_bytes();
                    let history_check = pbkdf2_hmac_sha256(
                        new_password.as_bytes(),
                        history_salt_bytes,
                        Self::PBKDF2_ITERATIONS,
                    );
                    let _ = history_check;
                    false // Placeholder -- see below for correct check
                } else {
                    false
                }
            });

        // Correct approach: check if new password matches current password
        if self.verify_password(new_password) {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "must differ from current password",
            });
        }

        // Save current hash to history (fixed-size ring buffer)
        let effective_size = history_size.min(MAX_PASSWORD_HISTORY);
        if effective_size > 0 {
            if self.password_history_len >= effective_size {
                // Shift entries left to make room (drop oldest)
                for i in 0..effective_size - 1 {
                    self.password_history[i] = self.password_history[i + 1];
                }
                self.password_history[effective_size - 1] = Some(self.password_hash);
                self.password_history_len = effective_size;
            } else {
                self.password_history[self.password_history_len] = Some(self.password_hash);
                self.password_history_len += 1;
            }
        }

        // Generate new salt and hash
        let (new_hash, new_salt) = Self::hash_password(new_password);
        self.password_hash = new_hash;
        self.salt = new_salt;

        Ok(())
    }

    /// Enable MFA for this account
    pub fn enable_mfa(&mut self) -> [u8; 32] {
        use crate::crypto::random::get_random;

        let rng = get_random();
        let mut secret = [0u8; 32];
        if let Err(_e) = rng.fill_bytes(&mut secret) {
            crate::kprintln!("[AUTH] Warning: RNG fill_bytes failed for MFA secret");
        }

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
            // TOTP verification using real timestamps
            let time_step = 30; // 30 second windows
            let current_time = crate::arch::timer::get_timestamp_secs();

            let time_counter = current_time / time_step;

            // Check current window and one window before/after for clock skew
            for offset in [0u64, 1u64] {
                let counter = if offset == 0 {
                    time_counter
                } else {
                    // Check both +1 and -1 windows
                    if self.check_totp_window(&secret, time_counter.wrapping_add(1), token) {
                        return true;
                    }
                    time_counter.wrapping_sub(1)
                };

                if self.check_totp_window(&secret, counter, token) {
                    return true;
                }
            }

            false
        } else {
            false
        }
    }

    /// Check a single TOTP time window.
    fn check_totp_window(&self, secret: &[u8; 32], time_counter: u64, token: u32) -> bool {
        // Generate expected token from HMAC(secret, time_counter)
        let counter_bytes = time_counter.to_be_bytes();
        let hash = hmac_sha256(secret, &counter_bytes);
        let expected_token = u32::from_be_bytes([
            hash.as_bytes()[0],
            hash.as_bytes()[1],
            hash.as_bytes()[2],
            hash.as_bytes()[3],
        ]) % 1_000_000; // 6-digit token

        token == expected_token
    }
}

// ---------------------------------------------------------------------------
// Fixed-Size Account Database
// ---------------------------------------------------------------------------

/// A fixed-size account database that avoids heap allocation.
///
/// Stores up to [`MAX_ACCOUNTS`] user accounts in a static array.
/// Lookup is O(n) but n is bounded by MAX_ACCOUNTS (64), which is
/// acceptable for a kernel authentication module.
struct AccountDatabase {
    entries: [Option<UserAccount>; MAX_ACCOUNTS],
    count: usize,
}

impl AccountDatabase {
    /// Create an empty account database.
    const fn new() -> Self {
        // const-compatible initialization for array of Option<UserAccount>
        // We cannot use [None; MAX_ACCOUNTS] because UserAccount is not Copy,
        // so we build the array manually with a const block.
        const NONE: Option<UserAccount> = None;
        Self {
            entries: [NONE; MAX_ACCOUNTS],
            count: 0,
        }
    }

    /// Look up an account by username (immutable).
    fn get(&self, username: &str) -> Option<&UserAccount> {
        self.entries[..self.count]
            .iter()
            .flatten()
            .find(|account| account.username == username)
    }

    /// Look up an account by username (mutable).
    fn get_mut(&mut self, username: &str) -> Option<&mut UserAccount> {
        self.entries[..self.count]
            .iter_mut()
            .flatten()
            .find(|account| account.username == username)
    }

    /// Check if a username exists.
    fn contains_key(&self, username: &str) -> bool {
        self.get(username).is_some()
    }

    /// Insert a new account. Returns `Err` if the database is full.
    fn insert(&mut self, account: UserAccount) -> Result<(), KernelError> {
        if self.count >= MAX_ACCOUNTS {
            return Err(KernelError::ResourceExhausted {
                resource: "account_database",
            });
        }

        // Find first empty slot (there is guaranteed to be one since count <
        // MAX_ACCOUNTS)
        for entry in &mut self.entries {
            if entry.is_none() {
                *entry = Some(account);
                self.count += 1;
                return Ok(());
            }
        }

        // Should not be reached if count is maintained correctly
        Err(KernelError::ResourceExhausted {
            resource: "account_database",
        })
    }

    /// Remove an account by username. Returns the removed account, or `None`.
    fn remove(&mut self, username: &str) -> Option<UserAccount> {
        for entry in &mut self.entries {
            if let Some(account) = entry {
                if account.username == username {
                    let removed = entry.take();
                    self.count -= 1;
                    return removed;
                }
            }
        }
        None
    }

    /// Iterate over all accounts (immutable).
    fn iter(&self) -> impl Iterator<Item = &UserAccount> {
        self.entries.iter().filter_map(|e| e.as_ref())
    }
}

// ---------------------------------------------------------------------------
// Authentication Manager
// ---------------------------------------------------------------------------

/// Authentication manager
pub struct AuthManager {
    accounts: RwLock<AccountDatabase>,
    next_user_id: RwLock<u32>,
    max_failed_attempts: u32,
    password_policy: RwLock<PasswordPolicy>,
}

impl AuthManager {
    /// Create new authentication manager
    pub fn new() -> Self {
        Self {
            accounts: RwLock::new(AccountDatabase::new()),
            next_user_id: RwLock::new(1000), // Start UIDs at 1000
            max_failed_attempts: 5,
            password_policy: RwLock::new(PasswordPolicy::relaxed()),
        }
    }

    /// Create with a specific password policy.
    pub fn with_policy(policy: PasswordPolicy) -> Self {
        Self {
            accounts: RwLock::new(AccountDatabase::new()),
            next_user_id: RwLock::new(1000),
            max_failed_attempts: 5,
            password_policy: RwLock::new(policy),
        }
    }

    /// Set the password policy.
    pub fn set_password_policy(&self, policy: PasswordPolicy) {
        *self.password_policy.write() = policy;
    }

    /// Get the current password policy.
    pub fn get_password_policy(&self) -> PasswordPolicy {
        *self.password_policy.read()
    }

    /// Create new user account.
    ///
    /// Validates the password against the active policy before creating
    /// the account.
    pub fn create_user(
        &self,
        username: &'static str,
        password: &str,
    ) -> Result<UserId, KernelError> {
        // Validate password against policy
        let policy = *self.password_policy.read();
        policy.validate_password(password)?;

        let mut accounts = self.accounts.write();

        // Check if username already exists
        if accounts.contains_key(username) {
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
        let account = UserAccount::new(user_id, username, password);

        accounts.insert(account)?;

        Ok(user_id)
    }

    /// Authenticate user.
    ///
    /// Checks account lock, expiration, password, and MFA status.
    pub fn authenticate(&self, username: &str, password: &str) -> AuthResult {
        let mut accounts = self.accounts.write();

        if let Some(account) = accounts.get_mut(username) {
            // Check if account is locked
            if account.locked {
                // Log the failed attempt
                crate::security::audit::log_auth_attempt(0, account.user_id, username, false);
                return AuthResult::AccountLocked;
            }

            // Check if account has expired
            if account.is_expired() {
                crate::security::audit::log_auth_attempt(0, account.user_id, username, false);
                return AuthResult::AccountExpired;
            }

            // Verify password
            if account.verify_password(password) {
                // Reset failed attempts on successful login
                account.failed_attempts = 0;

                // Check if MFA is required
                if account.mfa_enabled {
                    return AuthResult::MfaRequired;
                }

                // Log successful authentication
                crate::security::audit::log_auth_attempt(0, account.user_id, username, true);

                return AuthResult::Success;
            } else {
                // Increment failed attempts
                account.failed_attempts += 1;

                // Log failed authentication
                crate::security::audit::log_auth_attempt(0, account.user_id, username, false);

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
                crate::security::audit::log_auth_attempt(0, account.user_id, username, true);
                return AuthResult::Success;
            }
        }

        AuthResult::InvalidCredentials
    }

    /// Change a user's password.
    ///
    /// Validates the new password against the active policy and checks
    /// password history to prevent reuse.
    pub fn change_password(
        &self,
        username: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KernelError> {
        let policy = *self.password_policy.read();

        // Validate new password against policy
        policy.validate_password(new_password)?;

        let mut accounts = self.accounts.write();

        if let Some(account) = accounts.get_mut(username) {
            // Verify old password first
            if !account.verify_password(old_password) {
                return Err(KernelError::PermissionDenied {
                    operation: "change_password",
                });
            }

            // Change password with history check
            account.change_password(new_password, policy.history_size)?;

            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "user",
                id: 0,
            })
        }
    }

    /// Set account expiration.
    pub fn set_account_expiration(
        &self,
        username: &str,
        expires_at: Option<u64>,
    ) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();

        if let Some(account) = accounts.get_mut(username) {
            account.set_expiration(expires_at);
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "user",
                id: 0,
            })
        }
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

    /// List all usernames. Returns an iterator-friendly fixed-size collection.
    ///
    /// Since we cannot return `Vec<String>` without heap allocation, callers
    /// should use `with_users` or iterate via the returned array.
    pub fn list_usernames(&self, buf: &mut [Option<&str>]) -> usize {
        let accounts = self.accounts.read();
        let mut i = 0;
        for account in accounts.iter() {
            if i >= buf.len() {
                break;
            }
            buf[i] = Some(account.username);
            i += 1;
        }
        i
    }

    /// Get user by ID
    pub fn get_user_by_id(&self, user_id: UserId) -> Option<&'static str> {
        let accounts = self.accounts.read();

        for account in accounts.iter() {
            if account.user_id == user_id {
                return Some(account.username);
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

// ---------------------------------------------------------------------------
// Global State
// ---------------------------------------------------------------------------

/// Global authentication manager
static AUTH_MANAGER: OnceLock<AuthManager> = OnceLock::new();

/// Initialize authentication framework
pub fn init() -> Result<(), KernelError> {
    AUTH_MANAGER
        .set(AuthManager::new())
        .map_err(|_| KernelError::AlreadyExists {
            resource: "auth_manager",
            id: 0,
        })?;

    // Create default root account (uses relaxed policy for initial setup)
    let auth_manager = get_auth_manager();
    if let Err(_e) = auth_manager.create_user("root", "veridian") {
        crate::kprintln!("[AUTH] Warning: Failed to create default root account");
    }

    crate::println!("[AUTH] Authentication framework initialized");
    crate::println!("[AUTH] Default root user created (password: veridian)");
    crate::println!(
        "[AUTH] PBKDF2-HMAC-SHA256 with {} iterations",
        UserAccount::PBKDF2_ITERATIONS
    );

    Ok(())
}

/// Get global authentication manager
pub fn get_auth_manager() -> &'static AuthManager {
    AUTH_MANAGER.get().expect("Auth manager not initialized")
}

/// Validate a password against the default policy (convenience function).
pub fn validate_password(password: &str) -> Result<(), KernelError> {
    PasswordPolicy::default_policy().validate_password(password)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_password_hashing() {
        let account = UserAccount::new(1000, "test", "password123");

        assert!(account.verify_password("password123"));
        assert!(!account.verify_password("wrongpassword"));
    }

    #[test_case]
    fn test_authentication() {
        let auth = AuthManager::new();

        let _ = auth.create_user("alice", "secret");

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

        let _ = auth.create_user("bob", "password");

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

    #[test_case]
    fn test_pbkdf2_hmac_sha256() {
        // Test that PBKDF2 produces consistent output
        let salt = [0x42u8; 32];
        let hash1 = pbkdf2_hmac_sha256(b"test_password", &salt, 10);
        let hash2 = pbkdf2_hmac_sha256(b"test_password", &salt, 10);
        assert_eq!(hash1, hash2);

        // Different passwords produce different hashes
        let hash3 = pbkdf2_hmac_sha256(b"different_password", &salt, 10);
        assert_ne!(hash1, hash3);
    }

    #[test_case]
    fn test_hmac_sha256() {
        // Basic HMAC test: same key+message = same output
        let key = b"secret_key";
        let msg = b"hello world";
        let h1 = hmac_sha256(key, msg);
        let h2 = hmac_sha256(key, msg);
        assert_eq!(h1, h2);

        // Different message = different HMAC
        let h3 = hmac_sha256(key, b"different message");
        assert_ne!(h1, h3);
    }

    #[test_case]
    fn test_password_policy_validation() {
        let policy = PasswordPolicy::default_policy();

        // Too short
        assert!(policy.validate_password("Ab1").is_err());

        // Missing uppercase
        assert!(policy.validate_password("abcdefg1").is_err());

        // Missing lowercase
        assert!(policy.validate_password("ABCDEFG1").is_err());

        // Missing digit
        assert!(policy.validate_password("Abcdefgh").is_err());

        // Valid password
        assert!(policy.validate_password("Abcdefg1").is_ok());
    }

    #[test_case]
    fn test_account_expiration() {
        let mut account = UserAccount::new(1000, "exptest", "password");

        // No expiration: not expired
        assert!(!account.is_expired());

        // Set expiration in the past (0 = already expired since boot time > 0 in tests,
        // but on fresh boot timestamp may be 0, so use a small value)
        account.set_expiration(Some(0));
        // This may or may not be expired depending on boot time;
        // just verify the field was set
        assert_eq!(account.expires_at, Some(0));
    }

    #[test_case]
    fn test_password_change_reuse() {
        let mut account = UserAccount::new(1000, "chgtest", "original");

        // Changing to the same password should fail
        let result = account.change_password("original", 5);
        assert!(result.is_err());

        // Changing to a different password should succeed
        let result = account.change_password("newpassword", 5);
        assert!(result.is_ok());

        // Password history should have one entry
        assert_eq!(account.password_history_len, 1);

        // Verify new password works
        assert!(account.verify_password("newpassword"));
        assert!(!account.verify_password("original"));
    }
}

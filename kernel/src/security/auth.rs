//! Authentication: the account store behind `veridian_auth`, pam_veridian
//! and the login screens (N-131).
//!
//! # Accounts
//!
//! An account is a line of `/etc/shadow` (shadow(5)): the password hash in
//! SHA-512-crypt form (`security::crypt`, the form musl's crypt(3) reads,
//! so BusyBox's `login` and `su` check the same passwords), an
//! administrator's lock as a `!` before it (`passwd -l`), and the aging
//! fields, in days since 1970, read as pam_unix reads them: an expiry date,
//! a forced change (last change 0), a maximum age after which the password
//! must be changed and an inactivity period after which the account is
//! expired, and a minimum age before a user may change it again. User IDs
//! and names come from `/etc/passwd` (the user database); previous hashes
//! from `/etc/security/opasswd` (pam_pwhistory's file); second-factor
//! secrets from `/etc/veridian/mfa`. `security::accounts` loads and saves
//! the files.
//!
//! There is no built-in password: an account whose line has none (`!`,
//! `*`, or no line at all) cannot be logged in to with a password until one
//! is set (`passwd` on the console, or the rootfs build's
//! `VERIDIAN_ROOT_PASSWORD`).
//!
//! Failed attempts lock an account for [`AuthManager::LOCKOUT_SECS`]
//! (pam_faillock's default); that count lives in memory only.

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use spin::RwLock;

use crate::{
    crypto::hash::{sha256, Hash256},
    error::KernelError,
    security::crypt,
    sync::once_lock::OnceLock,
};

/// User identifier
pub type UserId = u32;

/// Most previous passwords remembered per account.
const MAX_PASSWORD_HISTORY: usize = 5;

/// SHA-512-crypt rounds for new hashes: the standard 5000, or the
/// specification's minimum in unoptimized (dev) kernels, where 5000 rounds
/// take seconds. The count is stored with each hash, so either build
/// checks the other's.
#[cfg(debug_assertions)]
const HASH_ROUNDS: u32 = 1000;
#[cfg(not(debug_assertions))]
const HASH_ROUNDS: u32 = crypt::DEFAULT_ROUNDS;

/// shadow(5)'s "no maximum age" (pam_unix also treats -1 as none).
const NO_MAX_AGE: u64 = 99_999;

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
        // SHA-512-crypt hashes at most this much (musl's limit too).
        if password.len() > crate::security::crypt::MAX_KEY {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "too long",
            });
        }
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
// HMAC-SHA256 (TOTP)
// ---------------------------------------------------------------------------

/// SHA-256 block size in bytes.
const BLOCK_SIZE: usize = 64;

/// Maximum message size for HMAC inner hash: the only caller,
/// `check_totp_window`, passes an 8-byte counter.
const HMAC_INNER_BUF_SIZE: usize = 192;

/// HMAC-SHA256 for the TOTP check (no heap allocation).
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

// ---------------------------------------------------------------------------
// User Account
// ---------------------------------------------------------------------------

/// The aging fields of a shadow line, in days since 1970 (`None`: empty).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Aging {
    /// Day of the last password change; 0 forces a change at next login.
    pub last_change: Option<u64>,
    /// Days before a user may change the password again.
    pub min_days: Option<u64>,
    /// Days after which the password must be changed.
    pub max_days: Option<u64>,
    /// Days of warning before the password expires.
    pub warn_days: Option<u64>,
    /// Days after the password expires that it may still be changed at
    /// login; after them the account is expired.
    pub inactive_days: Option<u64>,
    /// Day on which the account expires.
    pub expire_day: Option<u64>,
}

/// What the aging fields say about an account on `today`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeState {
    Valid,
    /// The password must be changed (PAM's new-authtok-required).
    PasswordExpired,
    /// The account has expired, by date or by inactivity.
    AccountExpired,
}

impl Aging {
    /// pam_unix's reading of the fields (check_shadow_expiry) on `today`
    /// (`None`: the date is not known). Without the date, an account with
    /// an expiry date or a maximum password age cannot be shown to be
    /// valid, so it counts as expired (fail closed); a forced change
    /// needs no date.
    pub fn state(&self, today: Option<u64>) -> AgeState {
        let finite_max = self.max_days.is_some_and(|m| m < NO_MAX_AGE);
        if self.last_change == Some(0) {
            // An expiry date overrides; without the date it cannot be
            // checked, so it does too.
            let expired = self
                .expire_day
                .is_some_and(|e| today.is_none_or(|t| t >= e));
            return if expired {
                AgeState::AccountExpired
            } else {
                AgeState::PasswordExpired
            };
        }
        let Some(today) = today else {
            return if self.expire_day.is_some() || finite_max && self.last_change.is_some() {
                AgeState::AccountExpired
            } else {
                AgeState::Valid
            };
        };
        if self.expire_day.is_some_and(|e| today >= e) {
            return AgeState::AccountExpired;
        }
        let Some(last) = self.last_change else {
            return AgeState::Valid;
        };
        match self.max_days {
            Some(max) if max < NO_MAX_AGE && today > last.saturating_add(max) => {
                let inactive_end = self
                    .inactive_days
                    .map(|i| last.saturating_add(max).saturating_add(i));
                if inactive_end.is_some_and(|end| today > end) {
                    AgeState::AccountExpired
                } else {
                    AgeState::PasswordExpired
                }
            }
            _ => AgeState::Valid,
        }
    }

    /// Whether the minimum age forbids a user's change on `today` (refused
    /// when it cannot be checked: a minimum age and no date).
    pub fn too_soon_to_change(&self, today: Option<u64>) -> bool {
        match (self.last_change, self.min_days) {
            (Some(last), Some(min)) if last != 0 && min > 0 => {
                today.is_none_or(|t| t < last.saturating_add(min))
            }
            _ => false,
        }
    }
}

/// Today, in days since 1970, if the wall clock knows the date.
pub fn today() -> Option<u64> {
    crate::timer::realtime::is_known()
        .then(|| (crate::timer::realtime::now_ns().max(0) as u64) / 86_400_000_000_000)
}

/// A user account.
#[derive(Debug, Clone)]
pub struct UserAccount {
    pub user_id: UserId,
    pub username: String,
    /// The shadow password field: a `$6$` hash; `!` before it while an
    /// administrator has locked the password; anything else (`!`, `*`,
    /// empty) matches no password.
    pub(crate) password: String,
    /// Locked after too many failed attempts: until this time (seconds
    /// since boot), so mistyping cannot lock an account for good.
    pub locked_until: Option<u64>,
    pub failed_attempts: u32,
    pub mfa_enabled: bool,
    pub mfa_secret: Option<[u8; 32]>,
    pub aging: Aging,
    /// Previous password hashes, oldest first (pam_pwhistory's remember).
    pub(crate) password_history: Vec<String>,
}

impl UserAccount {
    /// A new account with no password (it cannot be logged in to until one
    /// is set), as useradd creates one.
    pub fn new(user_id: UserId, username: &str) -> Self {
        Self {
            user_id,
            username: username.to_string(),
            password: String::from("!"),
            locked_until: None,
            failed_attempts: 0,
            mfa_enabled: false,
            mfa_secret: None,
            aging: Aging::default(),
            password_history: Vec::new(),
        }
    }

    /// Whether a password is set (a hash, locked or not).
    pub fn has_password(&self) -> bool {
        self.password.trim_start_matches('!').starts_with("$6$")
    }

    /// Whether an administrator has locked the password.
    pub fn password_locked(&self) -> bool {
        self.password.starts_with('!') && self.has_password()
    }

    /// Verify password (in constant time: the comparison does not reveal
    /// how much of the hash matched). A locked or unset password matches
    /// nothing.
    pub fn verify_password(&self, password: &str) -> bool {
        crypt::verify_password(password, &self.password)
    }

    /// Whether the account is locked at `now` (seconds since boot): by an
    /// administrator, or by failed attempts whose lockout has not run out
    /// (one that has is cleared, with the failure count).
    fn lock_active(&mut self, now: u64) -> bool {
        match self.locked_until {
            Some(until) if now < until => return true,
            Some(_) => {
                self.locked_until = None;
                self.failed_attempts = 0;
            }
            None => {}
        }
        self.password_locked()
    }

    /// Set a new password: refused if it is the current one or among the
    /// last `history_size`; the current hash joins the history. The last
    /// change becomes `today` (if the date is known).
    pub fn change_password(
        &mut self,
        new_password: &str,
        history_size: usize,
        today: Option<u64>,
    ) -> Result<(), KernelError> {
        if self.verify_password(new_password) {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "must differ from current password",
            });
        }
        let keep = history_size.min(MAX_PASSWORD_HISTORY);
        let recent = self.password_history.len().saturating_sub(keep);
        if self.password_history[recent..]
            .iter()
            .any(|old| crypt::verify_password(new_password, old))
        {
            return Err(KernelError::InvalidArgument {
                name: "password",
                value: "matches a recent password in history",
            });
        }
        let hash = crypt::hash_password(new_password, HASH_ROUNDS).ok_or(
            KernelError::InvalidArgument {
                name: "password",
                value: "too long",
            },
        )?;
        if keep > 0 && self.has_password() {
            self.password_history
                .push(self.password.trim_start_matches('!').to_string());
            let excess = self.password_history.len().saturating_sub(keep);
            self.password_history.drain(..excess);
        }
        self.password = hash;
        // Without the date (no RTC read yet: AArch64 and RISC-V have no
        // driver) no change day is recorded; writing 0 would mean "change
        // it now" to shadow(5).
        self.aging.last_change = today.filter(|&t| t > 0);
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
// Authentication Manager
// ---------------------------------------------------------------------------

/// Authentication manager
pub struct AuthManager {
    accounts: RwLock<BTreeMap<String, UserAccount>>,
    max_failed_attempts: u32,
    password_policy: RwLock<PasswordPolicy>,
}

fn not_found() -> KernelError {
    KernelError::NotFound {
        resource: "user",
        id: 0,
    }
}

impl AuthManager {
    /// How long failed attempts lock an account (pam_faillock's default
    /// unlock_time).
    pub const LOCKOUT_SECS: u64 = 600;

    /// An empty store (accounts come from the files once the root
    /// filesystem is up).
    pub const fn new() -> Self {
        Self {
            accounts: RwLock::new(BTreeMap::new()),
            max_failed_attempts: 5,
            password_policy: RwLock::new(PasswordPolicy::relaxed()),
        }
    }

    /// Create with a specific password policy.
    pub const fn with_policy(policy: PasswordPolicy) -> Self {
        Self {
            accounts: RwLock::new(BTreeMap::new()),
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

    /// Add an account for user `user_id` named `username`, with no password
    /// (useradd). EEXIST for a name already present.
    pub fn add_account(&self, username: &str, user_id: UserId) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();
        if accounts.contains_key(username) {
            return Err(KernelError::AlreadyExists {
                resource: "user",
                id: user_id as u64,
            });
        }
        accounts.insert(username.to_string(), UserAccount::new(user_id, username));
        Ok(())
    }

    /// Whether an account named `username` exists.
    pub fn has_account(&self, username: &str) -> bool {
        self.accounts.read().contains_key(username)
    }

    /// Whether account `username` has a password set (locked or not).
    pub fn has_password(&self, username: &str) -> bool {
        self.accounts
            .read()
            .get(username)
            .is_some_and(UserAccount::has_password)
    }

    /// Authenticate user.
    ///
    /// Checks the lock, the expiry, the password, its age and MFA.
    pub fn authenticate(&self, username: &str, password: &str) -> AuthResult {
        self.authenticate_at(
            username,
            password,
            crate::arch::timer::get_timestamp_secs(),
            today(),
        )
    }

    /// [`authenticate`](Self::authenticate) at time `now` (seconds since
    /// boot) on day `today`.
    fn authenticate_at(
        &self,
        username: &str,
        password: &str,
        now: u64,
        today: Option<u64>,
    ) -> AuthResult {
        let mut accounts = self.accounts.write();
        let Some(account) = accounts.get_mut(username) else {
            return AuthResult::InvalidCredentials;
        };
        match self.check_password(account, password, now, today) {
            AuthResult::Success if account.aging.state(today) == AgeState::PasswordExpired => {
                AuthResult::PasswordExpired
            }
            AuthResult::Success if account.mfa_enabled => AuthResult::MfaRequired,
            AuthResult::Success => {
                crate::security::audit::log_auth_attempt(0, account.user_id, username, true);
                AuthResult::Success
            }
            other => other,
        }
    }

    /// Check `password` against `account` at time `now` on day `today`, as
    /// every password check must: refused while the account is locked or
    /// expired; a failure is counted and logged, and enough of them lock
    /// the account for [`LOCKOUT_SECS`](Self::LOCKOUT_SECS); a success
    /// clears the count. `Success` means only that the password is right.
    fn check_password(
        &self,
        account: &mut UserAccount,
        password: &str,
        now: u64,
        today: Option<u64>,
    ) -> AuthResult {
        let fail = |account: &UserAccount| {
            crate::security::audit::log_auth_attempt(0, account.user_id, &account.username, false);
        };
        if account.lock_active(now) {
            fail(account);
            return AuthResult::AccountLocked;
        }
        if account.aging.state(today) == AgeState::AccountExpired {
            fail(account);
            return AuthResult::AccountExpired;
        }
        if account.verify_password(password) {
            account.failed_attempts = 0;
            return AuthResult::Success;
        }
        account.failed_attempts += 1;
        fail(account);
        if account.failed_attempts >= self.max_failed_attempts {
            account.locked_until = Some(now.saturating_add(Self::LOCKOUT_SECS));
            return AuthResult::AccountLocked;
        }
        AuthResult::InvalidCredentials
    }

    /// The state of an account without checking a password (PAM's account
    /// management): `Success`, `AccountLocked`, `AccountExpired`,
    /// `PasswordExpired`, or `InvalidCredentials` for no such account.
    pub fn account_status(&self, username: &str) -> AuthResult {
        let now = crate::arch::timer::get_timestamp_secs();
        let mut accounts = self.accounts.write();
        let Some(account) = accounts.get_mut(username) else {
            return AuthResult::InvalidCredentials;
        };
        if account.lock_active(now) {
            return AuthResult::AccountLocked;
        }
        match account.aging.state(today()) {
            AgeState::AccountExpired => AuthResult::AccountExpired,
            AgeState::PasswordExpired => AuthResult::PasswordExpired,
            AgeState::Valid => AuthResult::Success,
        }
    }

    /// Set a user's password without the old one (the administrator's
    /// passwd): the policy and the reuse history still apply, the minimum
    /// age does not, and an administrator's lock is lifted.
    pub fn reset_password(&self, username: &str, new_password: &str) -> Result<(), KernelError> {
        let policy = *self.password_policy.read();
        policy.validate_password(new_password)?;
        let mut accounts = self.accounts.write();
        let account = accounts.get_mut(username).ok_or_else(not_found)?;
        account.change_password(new_password, policy.history_size, today())
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

    /// Change a user's password with the old one.
    ///
    /// The old password is checked as a login is
    /// ([`check_password`](Self::check_password): lockout, failure count,
    /// audit), or this would be a way to guess it without limit; an account
    /// that needs a second factor cannot change its password with the old
    /// one alone; the minimum age applies (pam_unix: too soon); an expired
    /// password may be changed (that is how it is renewed).
    pub fn change_password(
        &self,
        username: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KernelError> {
        self.change_password_at(
            username,
            old_password,
            new_password,
            crate::arch::timer::get_timestamp_secs(),
            today(),
        )
    }

    /// [`change_password`](Self::change_password) at time `now` on day
    /// `today`.
    fn change_password_at(
        &self,
        username: &str,
        old_password: &str,
        new_password: &str,
        now: u64,
        today: Option<u64>,
    ) -> Result<(), KernelError> {
        let policy = *self.password_policy.read();
        policy.validate_password(new_password)?;

        let mut accounts = self.accounts.write();
        let account = accounts.get_mut(username).ok_or_else(not_found)?;
        let denied = KernelError::PermissionDenied {
            operation: "change_password",
        };
        match self.check_password(account, old_password, now, today) {
            AuthResult::Success if !account.mfa_enabled => {
                if account.aging.too_soon_to_change(today) {
                    return Err(denied);
                }
                account.change_password(new_password, policy.history_size, today)
            }
            _ => Err(denied),
        }
    }

    /// Set the day an account expires (`None`: never).
    pub fn set_account_expiration(
        &self,
        username: &str,
        expire_day: Option<u64>,
    ) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();
        let account = accounts.get_mut(username).ok_or_else(not_found)?;
        account.aging.expire_day = expire_day;
        Ok(())
    }

    /// Lock (`passwd -l`) or unlock (`passwd -u`) a password; unlocking
    /// also clears a failure lockout. Locking an account with no password
    /// changes nothing.
    pub fn set_password_lock(&self, username: &str, locked: bool) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();
        let account = accounts.get_mut(username).ok_or_else(not_found)?;
        if account.has_password() {
            let hash = account.password.trim_start_matches('!').to_string();
            account.password = if locked {
                alloc::format!("!{}", hash)
            } else {
                hash
            };
        }
        if !locked {
            account.locked_until = None;
            account.failed_attempts = 0;
        }
        Ok(())
    }

    /// Enable MFA for user
    pub fn enable_mfa(&self, username: &str) -> Result<[u8; 32], KernelError> {
        let mut accounts = self.accounts.write();
        let account = accounts.get_mut(username).ok_or_else(not_found)?;
        Ok(account.enable_mfa())
    }

    /// Clear a failure lockout.
    pub fn unlock_account(&self, username: &str) -> Result<(), KernelError> {
        let mut accounts = self.accounts.write();
        let account = accounts.get_mut(username).ok_or_else(not_found)?;
        account.locked_until = None;
        account.failed_attempts = 0;
        Ok(())
    }

    /// Delete user account
    pub fn delete_user(&self, username: &str) -> Result<(), KernelError> {
        self.accounts
            .write()
            .remove(username)
            .map(|_| ())
            .ok_or_else(not_found)
    }

    /// The account names, in order.
    pub fn usernames(&self) -> Vec<String> {
        self.accounts.read().keys().cloned().collect()
    }

    /// The name of user `user_id`.
    pub fn get_user_by_id(&self, user_id: UserId) -> Option<String> {
        self.accounts
            .read()
            .values()
            .find(|a| a.user_id == user_id)
            .map(|a| a.username.clone())
    }

    // -- Files ------------------------------------------------------------

    /// Replace the accounts with those of the files: `shadow`
    /// (`/etc/shadow`), `opasswd` (`/etc/security/opasswd`) and `mfa`
    /// (`/etc/veridian/mfa`); `users` lists the user database's (name,
    /// uid) pairs, and every user gets an account -- with no password if
    /// `shadow` has no line for it. A shadow line for a name the user
    /// database does not know is dropped (it has no user ID). Malformed
    /// lines are skipped; how many were is returned.
    pub fn load(
        &self,
        users: &[(String, UserId)],
        shadow: &str,
        opasswd: &str,
        mfa: &str,
    ) -> usize {
        let mut bad = 0;
        let mut accounts = BTreeMap::new();
        for (name, uid) in users {
            accounts.insert(name.clone(), UserAccount::new(*uid, name));
        }
        for line in shadow.lines().filter(|l| !l.trim().is_empty()) {
            match parse_shadow_line(line) {
                Some((name, password, aging)) => {
                    if let Some(account) = accounts.get_mut(name) {
                        account.password = password.to_string();
                        account.aging = aging;
                    }
                }
                None => bad += 1,
            }
        }
        for line in opasswd.lines().filter(|l| !l.trim().is_empty()) {
            // pam_pwhistory: name:uid:count:hash,hash,...
            let fields: Vec<&str> = line.split(':').collect();
            match (fields.first(), fields.get(3)) {
                (Some(name), Some(hashes)) => {
                    if let Some(account) = accounts.get_mut(*name) {
                        account.password_history = hashes
                            .split(',')
                            .filter(|h| h.starts_with("$6$"))
                            .map(String::from)
                            .collect();
                        let excess = account
                            .password_history
                            .len()
                            .saturating_sub(MAX_PASSWORD_HISTORY);
                        account.password_history.drain(..excess);
                    }
                }
                _ => bad += 1,
            }
        }
        for line in mfa.lines().filter(|l| !l.trim().is_empty()) {
            let parsed = line
                .split_once(':')
                .and_then(|(name, hex)| Some((name, decode_hex32(hex.trim())?)));
            match parsed {
                Some((name, secret)) => {
                    if let Some(account) = accounts.get_mut(name) {
                        account.mfa_secret = Some(secret);
                        account.mfa_enabled = true;
                    }
                }
                None => bad += 1,
            }
        }
        *self.accounts.write() = accounts;
        bad
    }

    /// `/etc/shadow`, one line per account, in name order.
    pub fn shadow_file(&self) -> String {
        let mut out = String::new();
        for account in self.accounts.read().values() {
            out.push_str(&shadow_line(account));
            out.push('\n');
        }
        out
    }

    /// `/etc/security/opasswd`: the accounts with a password history.
    pub fn opasswd_file(&self) -> String {
        let mut out = String::new();
        for a in self.accounts.read().values() {
            if !a.password_history.is_empty() {
                out.push_str(&alloc::format!(
                    "{}:{}:{}:{}\n",
                    a.username,
                    a.user_id,
                    a.password_history.len(),
                    a.password_history.join(",")
                ));
            }
        }
        out
    }

    /// `/etc/veridian/mfa`: the accounts with a second factor.
    pub fn mfa_file(&self) -> String {
        let mut out = String::new();
        for a in self.accounts.read().values() {
            if let Some(secret) = a.mfa_secret {
                out.push_str(&a.username);
                out.push(':');
                for b in secret {
                    out.push_str(&alloc::format!("{:02x}", b));
                }
                out.push('\n');
            }
        }
        out
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// A day count field of a shadow line (`None` for empty or -1).
fn parse_day(field: &str) -> Result<Option<u64>, ()> {
    match field {
        "" | "-1" => Ok(None),
        f => f.parse().map(Some).map_err(|_| ()),
    }
}

/// `name:password:lastchg:min:max:warn:inactive:expire:reserved`, the last
/// fields optional; `None` for a malformed line.
fn parse_shadow_line(line: &str) -> Option<(&str, &str, Aging)> {
    let f: Vec<&str> = line.split(':').collect();
    if f.len() < 2 || f[0].is_empty() {
        return None;
    }
    let day = |i: usize| parse_day(f.get(i).copied().unwrap_or(""));
    Some((
        f[0],
        f[1],
        Aging {
            last_change: day(2).ok()?,
            min_days: day(3).ok()?,
            max_days: day(4).ok()?,
            warn_days: day(5).ok()?,
            inactive_days: day(6).ok()?,
            expire_day: day(7).ok()?,
        },
    ))
}

/// An account's shadow line.
fn shadow_line(a: &UserAccount) -> String {
    let day = |d: Option<u64>| d.map_or_else(String::new, |d| d.to_string());
    let g = &a.aging;
    alloc::format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:",
        a.username,
        a.password,
        day(g.last_change),
        day(g.min_days),
        day(g.max_days),
        day(g.warn_days),
        day(g.inactive_days),
        day(g.expire_day)
    )
}

fn decode_hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Global State
// ---------------------------------------------------------------------------

/// Global authentication manager
static AUTH_MANAGER: OnceLock<AuthManager> = OnceLock::new();

/// Initialize authentication framework: an empty store, filled from the
/// account files once the root filesystem is mounted
/// (`security::accounts::load`). There is no built-in account or password.
pub fn init() -> Result<(), KernelError> {
    AUTH_MANAGER
        .set(AuthManager::new())
        .map_err(|_| KernelError::AlreadyExists {
            resource: "auth_manager",
            id: 0,
        })?;
    crate::println!("[AUTH] Authentication framework initialized (SHA-512-crypt)");
    Ok(())
}

/// The global authentication manager, if initialized.
pub fn try_auth_manager() -> Option<&'static AuthManager> {
    AUTH_MANAGER.get()
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

    /// A store with `name` (uid 1000) whose password is `password`.
    fn with_user(name: &str, password: &str) -> AuthManager {
        let auth = AuthManager::new();
        auth.add_account(name, 1000).unwrap();
        auth.reset_password(name, password).unwrap();
        auth
    }

    const NOW: u64 = 100;
    const DAY: u64 = 20_000;
    const TODAY: Option<u64> = Some(DAY);

    #[test]
    fn test_password_hashing() {
        let mut account = UserAccount::new(1000, "test");
        assert!(!account.verify_password(""));
        account.change_password("password123", 0, TODAY).unwrap();
        assert!(account.password.starts_with("$6$"));
        assert!(account.verify_password("password123"));
        assert!(!account.verify_password("wrongpassword"));
    }

    #[test]
    fn test_authentication() {
        let auth = with_user("alice", "secret");
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

    /// No built-in password: an account without one cannot be logged in
    /// to with any password, the empty one included.
    #[test]
    fn accounts_without_a_password_admit_nobody() {
        let auth = AuthManager::new();
        auth.add_account("root", 0).unwrap();
        for guess in ["", "veridian", "root", "!"] {
            assert_eq!(
                auth.authenticate_at("root", guess, NOW, TODAY),
                AuthResult::InvalidCredentials
            );
        }
    }

    #[test]
    fn test_account_locking() {
        let auth = with_user("bob", "password");
        for _ in 0..5 {
            let _ = auth.authenticate("bob", "wrong");
        }
        assert_eq!(
            auth.authenticate("bob", "password"),
            AuthResult::AccountLocked
        );
    }

    /// Failed attempts lock an account for LOCKOUT_SECS, not for good; an
    /// administrator's lock stays until lifted.
    #[test]
    fn failure_lockout_runs_out() {
        let auth = with_user("carol", "password");
        for _ in 0..5 {
            let _ = auth.authenticate_at("carol", "wrong", NOW, TODAY);
        }
        assert_eq!(
            auth.authenticate_at(
                "carol",
                "password",
                NOW + AuthManager::LOCKOUT_SECS - 1,
                TODAY
            ),
            AuthResult::AccountLocked
        );
        assert_eq!(
            auth.authenticate_at("carol", "password", NOW + AuthManager::LOCKOUT_SECS, TODAY),
            AuthResult::Success
        );
        for _ in 0..4 {
            let _ = auth.authenticate_at("carol", "wrong", 1000, TODAY);
        }
        assert_eq!(
            auth.authenticate_at("carol", "password", 1000, TODAY),
            AuthResult::Success
        );
        auth.set_password_lock("carol", true).unwrap();
        assert_eq!(
            auth.authenticate_at("carol", "password", 5000, TODAY),
            AuthResult::AccountLocked
        );
        assert!(auth.shadow_file().starts_with("carol:!$6$"));
        auth.set_password_lock("carol", false).unwrap();
        assert_eq!(
            auth.authenticate_at("carol", "password", 5000, TODAY),
            AuthResult::Success
        );
    }

    /// Changing a password checks the old one as a login does: wrong
    /// guesses count and lock the account, and a locked account cannot
    /// change it even with the right one.
    #[test]
    fn password_change_cannot_guess_past_the_lockout() {
        let auth = with_user("erin", "password");
        for _ in 0..5 {
            assert!(auth
                .change_password_at("erin", "guess", "new-password", NOW, TODAY)
                .is_err());
        }
        assert!(auth
            .change_password_at("erin", "password", "new-password", NOW + 1, TODAY)
            .is_err());
        let later = NOW + AuthManager::LOCKOUT_SECS;
        auth.change_password_at("erin", "password", "new-password", later, TODAY)
            .unwrap();
        assert_eq!(
            auth.authenticate_at("erin", "new-password", later, TODAY),
            AuthResult::Success
        );
    }

    #[test]
    fn account_status_and_reset_password() {
        let auth = with_user("dave", "password");
        assert_eq!(auth.account_status("dave"), AuthResult::Success);
        assert_eq!(
            auth.account_status("nobody"),
            AuthResult::InvalidCredentials
        );
        auth.reset_password("dave", "another-password").unwrap();
        assert_eq!(
            auth.authenticate("dave", "another-password"),
            AuthResult::Success
        );
        // The new password must differ.
        assert!(auth.reset_password("dave", "another-password").is_err());
        assert!(auth.reset_password("nobody", "x-password").is_err());
    }

    /// Password history (pam_pwhistory's remember), with a policy that
    /// keeps some.
    #[test]
    fn history_refuses_recent_passwords() {
        let auth = AuthManager::with_policy(PasswordPolicy {
            history_size: 2,
            ..PasswordPolicy::relaxed()
        });
        auth.add_account("hal", 1000).unwrap();
        for p in ["one", "two", "three"] {
            auth.reset_password("hal", p).unwrap();
        }
        // The current one and the two before it are refused.
        for p in ["one", "two", "three"] {
            assert!(auth.reset_password("hal", p).is_err(), "{}", p);
        }
        // After another change "one" is no longer among the two kept.
        auth.reset_password("hal", "four").unwrap();
        auth.reset_password("hal", "one").unwrap();
        assert!(auth.opasswd_file().starts_with("hal:1000:2:$6$"));
    }

    /// The aging fields as pam_unix reads them.
    #[test]
    fn aging_follows_shadow() {
        let age = |last, max, inactive, expire| Aging {
            last_change: last,
            max_days: max,
            inactive_days: inactive,
            expire_day: expire,
            ..Aging::default()
        };
        assert_eq!(age(None, None, None, None).state(TODAY), AgeState::Valid);
        assert_eq!(
            age(Some(0), None, None, None).state(TODAY),
            AgeState::PasswordExpired
        );
        assert_eq!(
            age(None, None, None, Some(DAY)).state(TODAY),
            AgeState::AccountExpired
        );
        assert_eq!(
            age(None, None, None, Some(DAY + 1)).state(TODAY),
            AgeState::Valid
        );
        let old = DAY - 100;
        assert_eq!(
            age(Some(old), Some(99_999), None, None).state(TODAY),
            AgeState::Valid
        );
        assert_eq!(
            age(Some(old), Some(90), None, None).state(TODAY),
            AgeState::PasswordExpired
        );
        assert_eq!(
            age(Some(old), Some(90), Some(5), None).state(TODAY),
            AgeState::AccountExpired
        );
        assert_eq!(
            age(Some(old), Some(90), Some(20), None).state(TODAY),
            AgeState::PasswordExpired
        );
        let min = Aging {
            last_change: Some(DAY - 1),
            min_days: Some(7),
            ..Aging::default()
        };
        assert!(min.too_soon_to_change(TODAY));
        assert!(!min.too_soon_to_change(Some(DAY + 6)));
        // Without the date, aging that needs it fails closed.
        assert!(min.too_soon_to_change(None));
        assert_eq!(age(None, None, None, None).state(None), AgeState::Valid);
        assert_eq!(
            age(None, None, None, Some(DAY + 1000)).state(None),
            AgeState::AccountExpired
        );
        assert_eq!(
            age(Some(old), Some(90), None, None).state(None),
            AgeState::AccountExpired
        );
        assert_eq!(
            age(Some(old), Some(99_999), None, None).state(None),
            AgeState::Valid
        );
        assert_eq!(
            age(Some(0), None, None, None).state(None),
            AgeState::PasswordExpired
        );
    }

    /// A forced change: the right password reports PasswordExpired, and a
    /// change renews it.
    #[test]
    fn forced_change_is_reported_and_renewed() {
        let auth = with_user("ivy", "password");
        // The same line with a last change of 0 ("change it now").
        let line = auth.shadow_file();
        let mut fields: Vec<&str> = line.trim_end().split(':').collect();
        fields[2] = "0";
        auth.load(&[(String::from("ivy"), 1000)], &fields.join(":"), "", "");
        assert_eq!(
            auth.authenticate_at("ivy", "password", NOW, TODAY),
            AuthResult::PasswordExpired
        );
        auth.change_password_at("ivy", "password", "renewed", NOW, TODAY)
            .unwrap();
        assert_eq!(
            auth.authenticate_at("ivy", "renewed", NOW, TODAY),
            AuthResult::Success
        );
    }

    /// The files round-trip; every user of the database has an account,
    /// one with no shadow line has no password, and a shadow line for an
    /// unknown name is dropped.
    #[test]
    fn files_round_trip() {
        let auth = with_user("jo", "password");
        auth.add_account("ken", 1001).unwrap();
        auth.set_account_expiration("ken", Some(30_000)).unwrap();
        let secret = auth.enable_mfa("ken").unwrap();
        let (shadow, opasswd, mfa) = (auth.shadow_file(), auth.opasswd_file(), auth.mfa_file());
        let users = [
            (String::from("jo"), 1000),
            (String::from("ken"), 1001),
            (String::from("lee"), 1002),
        ];
        let loaded = AuthManager::new();
        let shadow = alloc::format!("{}ghost:$6$x$y:::::::\nbroken\n", shadow);
        assert_eq!(loaded.load(&users, &shadow, &opasswd, &mfa), 1);
        assert_eq!(
            loaded.authenticate_at("jo", "password", NOW, TODAY),
            AuthResult::Success
        );
        assert_eq!(
            loaded.authenticate_at("ken", "", NOW, TODAY),
            AuthResult::InvalidCredentials
        );
        assert!(loaded.has_account("lee") && !loaded.has_account("ghost"));
        assert_eq!(loaded.usernames(), ["jo", "ken", "lee"]);
        let ken = loaded.accounts.read().get("ken").cloned().unwrap();
        assert_eq!(ken.aging.expire_day, Some(30_000));
        assert_eq!(ken.mfa_secret, Some(secret));
        assert!(loaded.shadow_file().contains("ken:!::::::30000:"));
    }

    #[test]
    fn test_hmac_sha256() {
        let key = b"secret_key";
        let msg = b"hello world";
        assert_eq!(hmac_sha256(key, msg), hmac_sha256(key, msg));
        assert_ne!(
            hmac_sha256(key, msg),
            hmac_sha256(key, b"different message")
        );
    }

    #[test]
    fn test_password_policy_validation() {
        let policy = PasswordPolicy::default_policy();
        assert!(policy.validate_password("Ab1").is_err());
        assert!(policy.validate_password("abcdefg1").is_err());
        assert!(policy.validate_password("ABCDEFG1").is_err());
        assert!(policy.validate_password("Abcdefgh").is_err());
        assert!(policy.validate_password("Abcdefg1").is_ok());
        let long = alloc::format!("Ab1{}", "x".repeat(crypt::MAX_KEY));
        assert!(policy.validate_password(&long).is_err());
    }
}

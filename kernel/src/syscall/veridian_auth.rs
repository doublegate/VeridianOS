//! veridian_auth: password checks against the kernel's account store, for
//! the pam_veridian PAM module (userland/pam_veridian).
//!
//! `veridian_auth(op, name, secret, new_secret)`:
//!
//! - `AUTH_CHECK`: is `secret` the password of account `name`?
//! - `AUTH_ACCOUNT`: the account's state, without a password (PAM account
//!   management).
//! - `AUTH_CHANGE`: change the password from `secret` to `new_secret`; root may
//!   pass `secret` = NULL to set it without the old one.
//!
//! CHECK and ACCOUNT return an `AUTH_*` result code (failures are results,
//! not errors); CHANGE returns 0 or an error, and writes the account files
//! (`security::accounts`: /etc/shadow, mode 0600): EIO if they cannot be
//! written, though the new password is already in effect.
//!
//! Root may act on any account. Any other caller only on its own: the
//! account the user database names for the caller's real user ID, as
//! Linux's unix_chkpwd allows. So an unprivileged program can neither guess
//! another user's password nor lock that account by failing on purpose
//! (EPERM).

use super::{SyscallError, SyscallResult};
use crate::{process::creds::Credentials, security::auth::AuthResult};

/// Check a password.
pub const AUTH_CHECK: usize = 0;
/// Query an account's state.
pub const AUTH_ACCOUNT: usize = 1;
/// Change a password.
pub const AUTH_CHANGE: usize = 2;

/// The password is right and the account usable.
pub const AUTH_OK: usize = 0;
/// A wrong password, or no such account (not told apart).
pub const AUTH_DENIED: usize = 1;
/// Locked, by an administrator or for a while after failed attempts.
pub const AUTH_LOCKED: usize = 2;
/// The account has expired.
pub const AUTH_EXPIRED: usize = 3;
/// The password is right but has expired: it must be changed.
pub const AUTH_NEW_PASSWORD: usize = 4;
/// The password is right but a second factor is required.
pub const AUTH_MFA_REQUIRED: usize = 5;

/// Longest account name read (Linux's LOGIN_NAME_MAX, with its NUL).
const NAME_MAX: usize = 256;
/// Longest password read (PAM_MAX_RESP_SIZE, with its NUL).
const SECRET_MAX: usize = 512;

/// The result code for an authentication outcome.
fn result_code(result: AuthResult) -> usize {
    match result {
        AuthResult::Success => AUTH_OK,
        AuthResult::InvalidCredentials | AuthResult::Denied => AUTH_DENIED,
        AuthResult::AccountLocked => AUTH_LOCKED,
        AuthResult::AccountExpired => AUTH_EXPIRED,
        AuthResult::PasswordExpired => AUTH_NEW_PASSWORD,
        AuthResult::MfaRequired => AUTH_MFA_REQUIRED,
    }
}

/// Whether `creds` may act on an account whose user ID in the user
/// database is `account_uid` (`None`: the database does not know the name).
pub(crate) fn may_act_on(creds: &Credentials, account_uid: Option<u32>) -> bool {
    creds.euid == 0 || account_uid == Some(creds.ruid)
}

/// A NUL-terminated string from user memory into `buf`, at most
/// `buf.len() - 1` bytes (ENAMETOOLONG for the name, EINVAL for a
/// password, if longer). The bytes, without the NUL.
fn read_string(addr: usize, buf: &mut [u8], too_long: SyscallError) -> Result<usize, SyscallError> {
    let n = super::userspace::strncpy_from_user(addr, buf)?;
    if n == buf.len() {
        return Err(too_long);
    }
    Ok(n)
}

/// A password from user memory, wiped from the kernel stack when dropped.
struct Secret {
    buf: [u8; SECRET_MAX],
    len: usize,
}

impl Secret {
    fn read(addr: usize) -> Result<Self, SyscallError> {
        let mut secret = Self {
            buf: [0; SECRET_MAX],
            len: 0,
        };
        secret.len = read_string(addr, &mut secret.buf, SyscallError::InvalidArgument)?;
        Ok(secret)
    }

    /// The password as text; a password that is not UTF-8 cannot be any
    /// account's (they are set as text), so it is `None`.
    fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.buf[..self.len]).ok()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Volatile, so the wipe is not optimized away as a dead store.
        for byte in self.buf.iter_mut() {
            // SAFETY: `byte` is a valid, aligned reference into `self.buf`.
            unsafe { core::ptr::write_volatile(byte, 0) };
        }
    }
}

/// veridian_auth(op, name, secret, new_secret).
pub fn sys_veridian_auth(
    op: usize,
    name: usize,
    secret: usize,
    new_secret: usize,
) -> SyscallResult {
    if !matches!(op, AUTH_CHECK | AUTH_ACCOUNT | AUTH_CHANGE) {
        return Err(SyscallError::InvalidArgument);
    }
    let mut name_buf = [0u8; NAME_MAX];
    let name_len = read_string(name, &mut name_buf, SyscallError::NameTooLong)?;
    let name =
        core::str::from_utf8(&name_buf[..name_len]).map_err(|_| SyscallError::InvalidArgument)?;

    let creds = crate::process::current_process()
        .ok_or(SyscallError::InvalidState)?
        .credentials();
    let account_uid =
        super::userland_ext::with_user_db(|db| db.get_user_by_name(name).map(|u| u.uid)).flatten();
    if !may_act_on(&creds, account_uid) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let auth = crate::security::auth::try_auth_manager().ok_or(SyscallError::InvalidState)?;

    match op {
        AUTH_CHECK => {
            let secret = Secret::read(secret)?;
            Ok(match secret.as_str() {
                Some(password) => result_code(auth.authenticate(name, password)),
                None => AUTH_DENIED,
            })
        }
        AUTH_ACCOUNT => Ok(result_code(auth.account_status(name))),
        _ => {
            let new = Secret::read(new_secret)?;
            let new = new.as_str().ok_or(SyscallError::InvalidArgument)?;
            let changed = if secret == 0 {
                if creds.euid != 0 {
                    return Err(SyscallError::OperationNotPermitted);
                }
                auth.reset_password(name, new)
            } else {
                let old = Secret::read(secret)?;
                let old = old.as_str().ok_or(SyscallError::PermissionDenied)?;
                auth.change_password(name, old, new)
            };
            changed.map_err(super::map_kernel_error)?;
            crate::security::accounts::save().map_err(|_| SyscallError::IoError)?;
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Root acts on any account; a user only on its own (by real uid), and
    /// on no account the user database does not know.
    #[test]
    fn only_root_or_the_owner_may_act() {
        let root = Credentials::new(0, 0);
        assert!(may_act_on(&root, Some(1000)));
        assert!(may_act_on(&root, None));
        let mut user = Credentials::new(1000, 100);
        assert!(may_act_on(&user, Some(1000)));
        assert!(!may_act_on(&user, Some(0)));
        assert!(!may_act_on(&user, None));
        // A setuid program running for user 1000 (effective 1001, real
        // 1000) acts for the real user, as unix_chkpwd does.
        user.euid = 1001;
        assert!(may_act_on(&user, Some(1000)));
        assert!(!may_act_on(&user, Some(1001)));
    }

    #[test]
    fn outcomes_map_to_result_codes() {
        assert_eq!(result_code(AuthResult::Success), AUTH_OK);
        assert_eq!(result_code(AuthResult::InvalidCredentials), AUTH_DENIED);
        assert_eq!(result_code(AuthResult::Denied), AUTH_DENIED);
        assert_eq!(result_code(AuthResult::AccountLocked), AUTH_LOCKED);
        assert_eq!(result_code(AuthResult::AccountExpired), AUTH_EXPIRED);
        assert_eq!(result_code(AuthResult::PasswordExpired), AUTH_NEW_PASSWORD);
        assert_eq!(result_code(AuthResult::MfaRequired), AUTH_MFA_REQUIRED);
    }
}

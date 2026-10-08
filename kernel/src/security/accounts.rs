//! The account files (N-131): one user store, on disk.
//!
//! - `/etc/passwd` -- names, user IDs, homes and shells (the user database,
//!   `syscall::userland_ext::users`);
//! - `/etc/shadow` (mode 0600) -- password hashes and aging (`security::auth`);
//! - `/etc/security/opasswd` (0600) -- previous hashes, pam_pwhistory's file;
//! - `/etc/veridian/mfa` (0600) -- second-factor secrets.
//!
//! [`load`] reads them once the root filesystem is mounted; [`save`] writes
//! them after every change -- each file whole, to a temporary file renamed
//! over the old one, so a crash leaves the old or the new file and never
//! half of one -- and syncs, since BlockFS writes only at a sync.
//!
//! The kernel shell (`passwd`, `useradd`, `userdel`), `veridian_auth` (the
//! PAM module's password change) and the login screens all go through here,
//! so the store PAM checks and the files musl's getpwnam and getspnam read
//! (BusyBox's `login`, `su`) are the same.

extern crate alloc;

use alloc::{string::String, sync::Arc, vec::Vec};

use crate::{
    error::KernelError,
    fs::{Permissions, VfsNode},
};

pub const PASSWD: &str = "/etc/passwd";
pub const SHADOW: &str = "/etc/shadow";
pub const OPASSWD: &str = "/etc/security/opasswd";
pub const MFA: &str = "/etc/veridian/mfa";

/// A file's text, if it exists and is UTF-8.
fn read_text(path: &str) -> Option<String> {
    let bytes = crate::fs::read_file(path).ok()?;
    String::from_utf8(bytes).ok()
}

/// Load the account files into the user database and the account store.
/// Missing files leave the defaults: root (uid 0) alone in the user
/// database, and no password for anyone.
pub fn load() {
    let Some(auth) = super::auth::try_auth_manager() else {
        return;
    };
    if let Some(passwd) = read_text(PASSWD) {
        let loaded = crate::syscall::userland_ext::with_user_db_mut(|db| db.load_passwd(&passwd));
        if let Some(Err(e)) = loaded {
            crate::println!(
                "[AUTH] {}: malformed line ({:?}); kept what parsed",
                PASSWD,
                e
            );
        }
    }
    let users =
        crate::syscall::userland_ext::with_user_db(|db| db.name_uid_pairs()).unwrap_or_default();
    let shadow = read_text(SHADOW).unwrap_or_default();
    let opasswd = read_text(OPASSWD).unwrap_or_default();
    let mfa = read_text(MFA).unwrap_or_default();
    let bad = auth.load(&users, &shadow, &opasswd, &mfa);
    if bad > 0 {
        crate::println!("[AUTH] {} malformed account line(s) skipped", bad);
    }
    let without: Vec<String> = auth
        .usernames()
        .into_iter()
        .filter(|name| !auth.has_password(name))
        .collect();
    crate::println!(
        "[AUTH] {} account(s) loaded; no password set for: {}",
        users.len(),
        if without.is_empty() {
            String::from("(none)")
        } else {
            without.join(", ")
        }
    );
    if without.iter().any(|n| n == "root") {
        crate::println!(
            "[AUTH] root cannot log in with a password until one is set: run `passwd` on the \
             console"
        );
    }
}

/// Write the account files from the user database and the account store,
/// then sync the filesystem.
pub fn save() -> Result<(), KernelError> {
    let auth =
        super::auth::try_auth_manager().ok_or(KernelError::NotInitialized { subsystem: "auth" })?;
    let passwd = crate::syscall::userland_ext::with_user_db(|db| db.to_passwd_file()).ok_or(
        KernelError::NotInitialized {
            subsystem: "user database",
        },
    )?;
    write_atomic(PASSWD, passwd.as_bytes(), 0o644)?;
    write_atomic(SHADOW, auth.shadow_file().as_bytes(), 0o600)?;
    let opasswd = auth.opasswd_file();
    if !opasswd.is_empty() || crate::fs::file_exists(OPASSWD) {
        ensure_dir("/etc/security")?;
        write_atomic(OPASSWD, opasswd.as_bytes(), 0o600)?;
    }
    let mfa = auth.mfa_file();
    if !mfa.is_empty() || crate::fs::file_exists(MFA) {
        ensure_dir("/etc/veridian")?;
        write_atomic(MFA, mfa.as_bytes(), 0o600)?;
    }
    crate::fs::get_vfs().sync()
}

/// `path` as a directory owned by root, mode 0755, created if missing.
fn ensure_dir(path: &str) -> Result<(), KernelError> {
    let vfs = crate::fs::get_vfs();
    if vfs.resolve_path(path).is_ok() {
        return Ok(());
    }
    let (parent, name) = split(path)?;
    let dir = vfs
        .resolve_path(parent)?
        .mkdir(name, Permissions::from_mode(0o755))?;
    let _ = dir.chown(Some(0), Some(0));
    Ok(())
}

fn split(path: &str) -> Result<(&str, &str), KernelError> {
    let invalid = KernelError::FsError(crate::error::FsError::InvalidPath);
    let pos = path.rfind('/').ok_or(invalid)?;
    Ok((if pos == 0 { "/" } else { &path[..pos] }, &path[pos + 1..]))
}

/// Replace `path` with `data`, owned by root with `mode`: written to a
/// temporary file in the same directory and renamed over it.
fn write_atomic(path: &str, data: &[u8], mode: u32) -> Result<(), KernelError> {
    let vfs = crate::fs::get_vfs();
    let (parent_path, name) = split(path)?;
    let parent: Arc<dyn VfsNode> = vfs.resolve_path(parent_path)?;
    let tmp = alloc::format!(".{}.new", name);
    let _ = parent.unlink(&tmp);
    let node = parent.create(&tmp, Permissions::from_mode(mode))?;
    let written = node
        .chown(Some(0), Some(0))
        .and_then(|()| node.chmod(Permissions::from_mode(mode)))
        .and_then(|()| node.write(0, data));
    match written {
        Ok(n) if n == data.len() => {}
        Ok(_) => {
            let _ = parent.unlink(&tmp);
            return Err(KernelError::FsError(crate::error::FsError::NoSpace));
        }
        Err(e) => {
            let _ = parent.unlink(&tmp);
            return Err(e);
        }
    }
    let _rename = crate::fs::RENAME_LOCK.lock();
    parent.rename(&tmp, &parent, name).inspect_err(|_| {
        let _ = parent.unlink(&tmp);
    })
}

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
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    error::KernelError,
    fs::{Permissions, VfsNode},
    sync::sleep_mutex::SleepMutex,
};

/// Serializes saves: each takes the state and writes every file under it,
/// so two saves cannot interleave their temporary files, and the last one
/// written holds the newest state. It may sleep (the writes and the sync
/// wait for the disk), so it is not a spinlock.
static SAVE_LOCK: SleepMutex<()> = SleepMutex::new(());

/// Set when a file existed but was not fully read (unreadable, not UTF-8,
/// or malformed lines): saving would replace it with the part that was
/// read, losing the rest, so saves are refused until the files are fixed
/// and the system restarted.
static LOAD_INCOMPLETE: AtomicBool = AtomicBool::new(false);

pub const PASSWD: &str = "/etc/passwd";
pub const SHADOW: &str = "/etc/shadow";
pub const OPASSWD: &str = "/etc/security/opasswd";
pub const MFA: &str = "/etc/veridian/mfa";

/// A file's text: empty if it does not exist; `None` if it exists but
/// cannot be read as UTF-8 text.
fn read_text(path: &str) -> Option<String> {
    if !crate::fs::file_exists(path) {
        return Some(String::new());
    }
    let bytes = crate::fs::read_file(path).ok()?;
    String::from_utf8(bytes).ok()
}

/// [`read_text`], recording a file that exists but was not read.
fn read_or_mark(path: &str) -> String {
    read_text(path).unwrap_or_else(|| {
        crate::println!("[AUTH] {}: cannot be read", path);
        LOAD_INCOMPLETE.store(true, Ordering::Release);
        String::new()
    })
}

/// Load the account files into the user database and the account store.
/// Missing files leave the defaults: root (uid 0) alone in the user
/// database, and no password for anyone.
pub fn load() {
    let Some(auth) = super::auth::try_auth_manager() else {
        return;
    };
    let passwd = read_or_mark(PASSWD);
    let skipped =
        crate::syscall::userland_ext::with_user_db_mut(|db| db.load_passwd(&passwd).1).unwrap_or(0);
    let users =
        crate::syscall::userland_ext::with_user_db(|db| db.name_uid_pairs()).unwrap_or_default();
    let shadow = read_or_mark(SHADOW);
    let opasswd = read_or_mark(OPASSWD);
    let mfa = read_or_mark(MFA);
    let bad = skipped + auth.load(&users, &shadow, &opasswd, &mfa);
    if bad > 0 {
        crate::println!("[AUTH] {} malformed account line(s) skipped", bad);
        LOAD_INCOMPLETE.store(true, Ordering::Release);
    }
    if LOAD_INCOMPLETE.load(Ordering::Acquire) {
        crate::println!(
            "[AUTH] the account files were not fully read: changes will not be saved until they \
             are fixed"
        );
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
    if LOAD_INCOMPLETE.load(Ordering::Acquire) {
        return Err(KernelError::PermissionDenied {
            operation: "save accounts: the files were not fully read at boot",
        });
    }
    let _saving = SAVE_LOCK.lock();
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

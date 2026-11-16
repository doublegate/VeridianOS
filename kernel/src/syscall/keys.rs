//! add_key, request_key and keyctl: the Linux 7.3 key retention service
//! core (`security/keys/`; ADR 0027 decision 28). The keys, their checks
//! and every operation are `veridian-keys`'; here are the system calls'
//! decoding, each thread's keys and the `/proc` files.

use alloc::{format, string::String, vec::Vec};

use spin::Mutex;
use veridian_keys::{errno as e, KeyCred, KeyStore, RestrictArgs, Subject};

use super::{userspace, SyscallError, SyscallResult};
use crate::process::{creds::Caller, linux_caps};

/// Every key on the machine.
static KEYS: spin::Once<Mutex<KeyStore>> = spin::Once::new();

fn random() -> u32 {
    let b = crate::crypto::random::random_bytes(4);
    u32::from_ne_bytes([b[0], b[1], b[2], b[3]])
}

fn store() -> &'static Mutex<KeyStore> {
    KEYS.call_once(|| Mutex::new(KeyStore::new(random)))
}

/// The real-time clock in seconds, as the key timeouts count.
fn now() -> u64 {
    u64::try_from(crate::timer::realtime::now_ns() / 1_000_000_000).unwrap_or(0)
}

/// The calling thread as the key service sees it.
fn subject_of(c: &Caller) -> Subject {
    let cr = &c.creds;
    Subject {
        uid: cr.ruid,
        euid: cr.euid,
        suid: cr.suid,
        fsuid: cr.fsuid,
        gid: cr.rgid,
        egid: cr.egid,
        sgid: cr.sgid,
        fsgid: cr.fsgid,
        groups: cr.groups().to_vec(),
        ns: c.userns.ident.id,
        ns_uid: c.userns.uid_up(cr.ruid).unwrap_or(65534),
        ns_owner: c.userns.owner,
        // CHOWN and SETPERM's override is capable(CAP_SYS_ADMIN): the
        // initial namespace's, so a namespace's root cannot take keys it
        // does not own (commit security review).
        sys_admin: c.capable(linux_caps::CAP_SYS_ADMIN),
        // GET_PERSISTENT's is ns_capable in the caller's namespace.
        setuid: c.ns_capable(&c.userns, linux_caps::CAP_SETUID),
        now: now(),
    }
}

/// Run `f` on the store with the caller and its thread's keys. The
/// thread's keys are locked before the store, always.
fn with<R>(f: impl FnOnce(&mut KeyStore, &Subject, &mut KeyCred) -> R) -> Result<R, SyscallError> {
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    let subj = subject_of(&Caller::current());
    let mut kc = thread.keys.lock();
    let mut ks = store().lock();
    Ok(f(&mut ks, &subj, &mut kc))
}

/// A Linux errno from the key service as a system call error.
fn from_errno(n: i32) -> SyscallError {
    match n {
        e::EPERM => SyscallError::OperationNotPermitted,
        e::ENOENT => SyscallError::ResourceNotFound,
        e::EIO => SyscallError::IoError,
        e::EAGAIN => SyscallError::WouldBlock,
        e::EACCES => SyscallError::PermissionDenied,
        e::EBUSY => SyscallError::Busy,
        e::EEXIST => SyscallError::FileExists,
        e::ENODEV => SyscallError::NoDevice,
        e::ENOTDIR => SyscallError::NotADirectory,
        e::EDEADLK => SyscallError::Deadlock,
        e::ELOOP => SyscallError::SymlinkLoop,
        e::EOPNOTSUPP => SyscallError::NotSupported,
        e::EDQUOT => SyscallError::QuotaExceeded,
        e::ENOKEY => SyscallError::KeyNotAvailable,
        e::EKEYEXPIRED => SyscallError::KeyExpired,
        e::EKEYREVOKED => SyscallError::KeyRevoked,
        _ => SyscallError::InvalidArgument,
    }
}

fn run<T>(
    f: impl FnOnce(&mut KeyStore, &Subject, &mut KeyCred) -> Result<T, i32>,
) -> Result<T, SyscallError> {
    with(f)?.map_err(from_errno)
}

/// key_get_type_from_user: at most 31 bytes; empty or longer is EINVAL, a
/// name starting with '.' EPERM.
fn read_type(ptr: usize) -> Result<String, SyscallError> {
    let mut buf = [0u8; 32];
    let n = userspace::strncpy_from_user(ptr, &mut buf)?;
    if n == 0 || n >= 32 {
        return Err(SyscallError::InvalidArgument);
    }
    if buf[0] == b'.' {
        return Err(SyscallError::OperationNotPermitted);
    }
    String::from_utf8(buf[..n].to_vec()).map_err(|_| SyscallError::InvalidArgument)
}

/// strndup_user(ptr, 4096): EFAULT for a bad pointer, EINVAL without a NUL
/// within the limit.
fn read_desc(ptr: usize) -> Result<String, SyscallError> {
    userspace::read_user_cstr(ptr, 4095)
}

/// add_key(type, description, payload, plen, ringid).
pub fn sys_add_key(
    ty: usize,
    desc: usize,
    payload: usize,
    plen: usize,
    ringid: usize,
) -> SyscallResult {
    if plen > 1_048_575 {
        return Err(SyscallError::InvalidArgument);
    }
    let ty = read_type(ty)?;
    let desc = if desc != 0 {
        Some(read_desc(desc)?).filter(|d| !d.is_empty())
    } else {
        None
    };
    let data = if plen > 0 {
        userspace::read_user_vec(payload, plen, 1_048_575)?
    } else {
        Vec::new()
    };
    run(|ks, s, kc| ks.add_key(s, kc, &ty, desc.as_deref(), &data, ringid as i32))
        .map(|k| k as usize)
}

/// request_key(type, description, callout_info, destringid).
pub fn sys_request_key(ty: usize, desc: usize, callout: usize, dest: usize) -> SyscallResult {
    let ty = read_type(ty)?;
    let desc = read_desc(desc)?;
    let callout = if callout != 0 {
        Some(userspace::read_user_cstr(callout, 4095)?)
    } else {
        None
    };
    run(|ks, s, kc| {
        ks.request_key(
            s,
            kc,
            &ty,
            &desc,
            callout.as_deref().map(str::as_bytes),
            dest as i32,
        )
    })
    .map(|k| k as usize)
}

/// keyctl's options (KEYCTL_*).
mod op {
    pub const GET_KEYRING_ID: usize = 0;
    pub const JOIN_SESSION_KEYRING: usize = 1;
    pub const UPDATE: usize = 2;
    pub const REVOKE: usize = 3;
    pub const CHOWN: usize = 4;
    pub const SETPERM: usize = 5;
    pub const DESCRIBE: usize = 6;
    pub const CLEAR: usize = 7;
    pub const LINK: usize = 8;
    pub const UNLINK: usize = 9;
    pub const SEARCH: usize = 10;
    pub const READ: usize = 11;
    pub const INSTANTIATE: usize = 12;
    pub const NEGATE: usize = 13;
    pub const SET_REQKEY_KEYRING: usize = 14;
    pub const SET_TIMEOUT: usize = 15;
    pub const ASSUME_AUTHORITY: usize = 16;
    pub const GET_SECURITY: usize = 17;
    pub const SESSION_TO_PARENT: usize = 18;
    pub const REJECT: usize = 19;
    pub const INSTANTIATE_IOV: usize = 20;
    pub const INVALIDATE: usize = 21;
    pub const GET_PERSISTENT: usize = 22;
    pub const PKEY_QUERY: usize = 24;
    pub const RESTRICT_KEYRING: usize = 29;
    pub const MOVE: usize = 30;
    pub const CAPABILITIES: usize = 31;
}

/// Copy `data` out for a call that returns its full length and copies only
/// into a buffer that holds it all (DESCRIBE, READ).
fn copy_if_room(buf: usize, buflen: usize, data: &[u8]) -> SyscallResult {
    if buf != 0 && buflen >= data.len() {
        userspace::write_user_bytes(buf, data)?;
    }
    Ok(data.len())
}

/// keyctl(option, arg2, arg3, arg4, arg5).
pub fn sys_keyctl(option: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> SyscallResult {
    let id = a2 as i32;
    match option as u32 as usize {
        op::GET_KEYRING_ID => {
            run(|ks, s, kc| ks.get_keyring_id(s, kc, id, a3 != 0)).map(|k| k as usize)
        }
        op::JOIN_SESSION_KEYRING => {
            let name = if a2 != 0 { Some(read_desc(a2)?) } else { None };
            run(|ks, s, kc| ks.join_session(s, kc, name.as_deref())).map(|k| k as usize)
        }
        op::UPDATE => {
            if a4 > 4096 {
                return Err(SyscallError::InvalidArgument);
            }
            let data = if a4 > 0 {
                userspace::read_user_vec(a3, a4, 4096)?
            } else {
                Vec::new()
            };
            run(|ks, s, kc| ks.update(s, kc, id, &data)).map(|()| 0)
        }
        op::REVOKE => run(|ks, s, kc| ks.revoke(s, kc, id)).map(|()| 0),
        op::CHOWN => {
            let c = Caller::current();
            let uid = match a3 as u32 {
                u32::MAX => None,
                u => Some(c.userns.uid_arg(u).ok_or(SyscallError::InvalidArgument)?),
            };
            let gid = match a4 as u32 {
                u32::MAX => None,
                g => Some(c.userns.gid_arg(g).ok_or(SyscallError::InvalidArgument)?),
            };
            run(|ks, s, kc| ks.chown(s, kc, id, uid, gid)).map(|()| 0)
        }
        op::SETPERM => run(|ks, s, kc| ks.setperm(s, kc, id, a3 as u32)).map(|()| 0),
        op::DESCRIBE => {
            let d = run(|ks, s, kc| ks.describe(s, kc, id))?;
            let ns = Caller::current().userns;
            let text = format!(
                "{};{};{};{:08x};{}\0",
                d.ktype,
                ns.uid_munged(d.uid) as i32,
                d.gid.map_or(65534, |g| ns.gid_munged(g)) as i32,
                d.perm,
                d.desc
            );
            copy_if_room(a3, a4 as u32 as usize, text.as_bytes())
        }
        op::CLEAR => run(|ks, s, kc| ks.clear_ring(s, kc, id)).map(|()| 0),
        op::LINK => run(|ks, s, kc| ks.link_key(s, kc, id, a3 as i32)).map(|()| 0),
        op::UNLINK => run(|ks, s, kc| ks.unlink_key(s, kc, id, a3 as i32)).map(|()| 0),
        op::SEARCH => {
            let ty = read_type(a3)?;
            let desc = read_desc(a4)?;
            run(|ks, s, kc| ks.search_key(s, kc, id, &ty, &desc, a5 as i32)).map(|k| k as usize)
        }
        op::READ => {
            let out = run(|ks, s, kc| ks.read(s, kc, id))?;
            if a3 == 0 || a4 == 0 {
                return Ok(out.data.len());
            }
            if out.keyring && !a4.is_multiple_of(4) {
                return Err(SyscallError::InvalidArgument);
            }
            copy_if_room(a3, a4, &out.data)
        }
        op::INSTANTIATE => {
            let plen = if a3 != 0 { a4 } else { 0 };
            run(|ks, _, kc| ks.instantiate_key(kc, plen)).map(|()| 0)
        }
        op::INSTANTIATE_IOV => {
            // The vector's total, as import_iovec adds it up.
            let mut plen = 0usize;
            if a3 != 0 {
                if a4 > 1024 {
                    return Err(SyscallError::InvalidArgument);
                }
                for i in 0..a4 {
                    let [_, len]: [usize; 2] = userspace::read_user_index(a3, i)?;
                    plen = plen.checked_add(len).ok_or(SyscallError::InvalidArgument)?;
                }
            }
            run(|ks, _, kc| ks.instantiate_key(kc, plen)).map(|()| 0)
        }
        op::NEGATE => run(|ks, _, kc| ks.reject_key(kc, e::ENOKEY)).map(|()| 0),
        op::REJECT => run(|ks, _, kc| ks.reject_key(kc, a4 as u32 as i32)).map(|()| 0),
        op::SET_REQKEY_KEYRING => {
            run(|ks, s, kc| ks.set_reqkey(s, kc, a2 as i32)).map(|v| v as usize)
        }
        op::SET_TIMEOUT => run(|ks, s, kc| ks.set_timeout(s, kc, id, a3 as u32)).map(|()| 0),
        op::ASSUME_AUTHORITY => run(|ks, _, kc| ks.assume_authority(kc, id)).map(|k| k as usize),
        op::GET_SECURITY => {
            run(|ks, s, kc| ks.get_security(s, kc, id))?;
            // No LSM labels keys: an empty string.
            if a3 != 0 && a4 > 0 {
                userspace::write_user_bytes(a3, &[0])?;
            }
            Ok(1)
        }
        op::SESSION_TO_PARENT => session_to_parent(),
        op::INVALIDATE => run(|ks, s, kc| ks.invalidate(s, kc, id)).map(|()| 0),
        op::GET_PERSISTENT => {
            let c = Caller::current();
            let (uid, ns_uid) = match a2 as u32 {
                u32::MAX => (c.creds.ruid, c.userns.uid_up(c.creds.ruid).unwrap_or(65534)),
                u => (c.userns.uid_arg(u).ok_or(SyscallError::InvalidArgument)?, u),
            };
            run(|ks, s, kc| ks.get_persistent(s, kc, uid, ns_uid, a3 as i32)).map(|k| k as usize)
        }
        op::RESTRICT_KEYRING => {
            // The pointers' strings are copied only once the keyring is
            // found, as Linux looks it up first.
            let args = RestrictArgs {
                ktype: (a3 != 0).then(|| read_type(a3).map_err(to_errno)),
                restriction: (a4 != 0)
                    .then(|| userspace::read_user_cstr(a4, 4095).map_err(to_errno)),
            };
            run(|ks, s, kc| ks.restrict(s, kc, id, args)).map(|()| 0)
        }
        op::MOVE => {
            if a5 as u32 & !1 != 0 {
                return Err(SyscallError::InvalidArgument);
            }
            run(|ks, s, kc| ks.move_key(s, kc, id, a3 as i32, a4 as i32, a5 & 1 != 0)).map(|()| 0)
        }
        op::CAPABILITIES => {
            let n = a3.min(veridian_keys::CAPABILITIES.len());
            if a2 != 0 && a3 > 0 {
                userspace::write_user_bytes(a2, &veridian_keys::CAPABILITIES[..n])?;
                // The rest is cleared, as keyctl_capabilities' clear_user, in
                // place: never a kernel buffer of a user-chosen size (PR #49
                // review).
                if a3 > n {
                    userspace::clear_user(a2 + n, a3 - n)?;
                }
            }
            Ok(veridian_keys::CAPABILITIES.len())
        }
        // Diffie-Hellman, public keys and notifications are not built in.
        op::PKEY_QUERY if a3 != 0 => Err(SyscallError::InvalidArgument),
        _ => Err(SyscallError::NotSupported),
    }
}

/// A system call error back to the errno the key service compares with.
fn to_errno(err: SyscallError) -> i32 {
    match err {
        SyscallError::OperationNotPermitted => e::EPERM,
        SyscallError::InvalidPointer | SyscallError::UnmappedMemory => 14,
        _ => e::EINVAL,
    }
}

/// KEYCTL_SESSION_TO_PARENT: the caller's session keyring replaces its
/// parent's, which must be a single-threaded process of the same user
/// whose session keyring, if any, the caller's user owns.
fn session_to_parent() -> SyscallResult {
    let ses = run(|ks, s, kc| ks.session_for_parent(s, kc))?;
    let me = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let ppid = me.parent().ok_or(SyscallError::OperationNotPermitted)?;
    if ppid.0 <= 1 {
        return Err(SyscallError::OperationNotPermitted);
    }
    let parent = crate::process::get_process(ppid).ok_or(SyscallError::OperationNotPermitted)?;
    let threads = parent.threads.lock();
    if threads.len() != 1 {
        return Err(SyscallError::OperationNotPermitted);
    }
    let Some(pt) = threads.values().next().cloned() else {
        return Err(SyscallError::OperationNotPermitted);
    };
    drop(threads);
    let c = Caller::current().creds;
    let p = parent.credentials();
    let mut pkc = pt.keys.lock();
    let mut ks = store().lock();
    if pkc.session == Some(ses) {
        return Ok(0);
    }
    if [p.ruid, p.euid, p.suid].iter().any(|&u| u != c.euid)
        || [p.rgid, p.egid, p.sgid].iter().any(|&g| g != c.egid)
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    let owned = |s: Option<i32>| s.is_none_or(|s| ks.owner(s) == Some(c.euid));
    if !owned(pkc.session) || !owned(Some(ses)) {
        return Err(SyscallError::OperationNotPermitted);
    }
    ks.set_session(&mut pkc, ses);
    Ok(0)
}

// ---- threads -----------------------------------------------------------

/// A new process's keys (fork): no thread or process keyring.
pub fn fork_keys(parent: &KeyCred) -> KeyCred {
    store().lock().fork_cred(parent)
}

/// A new thread's keys (CLONE_THREAD): the process keyring shared, a thread
/// keyring of its own if the creator has one.
pub fn thread_keys(creator: &KeyCred) -> KeyCred {
    let subj = subject_of(&Caller::current());
    store().lock().thread_cred(&subj, creator)
}

/// exec drops the thread and process keyrings.
pub fn exec_keys(kc: &mut KeyCred) {
    store().lock().exec_cred(kc);
}

/// A thread's keys go with it.
pub fn release_keys(kc: &KeyCred) {
    store().lock().release_cred(kc);
}

/// setfsuid or setfsgid changed the thread's file-system ids: its thread
/// keyring follows.
pub fn fsid_changed() {
    if let Some(t) = crate::process::current_thread() {
        let subj = subject_of(&Caller::current());
        let kc = t.keys.lock();
        store().lock().fsid_changed(&subj, &kc);
    }
}

// ---- /proc ---------------------------------------------------------------

/// `/proc/keys` for `reader`.
pub fn proc_keys(reader: &Caller) -> String {
    let subj = subject_of(reader);
    let kc =
        crate::process::current_thread().map_or_else(KeyCred::default, |t| t.keys.lock().clone());
    let ns = reader.userns.clone();
    let visible = |kuid: u32| ns.uid_up(kuid).is_some();
    let lines = store().lock().proc_keys(&subj, &kc, &visible);
    let mut out = String::new();
    for l in lines {
        out.push_str(&format!(
            "{:08x} {} {:5} {:>4} {:08x} {:5} {:5} {:<9.9} {}\n",
            l.serial,
            core::str::from_utf8(&l.flags).unwrap_or("-------"),
            l.usage,
            l.timeout,
            l.perm,
            ns.uid_munged(l.uid) as i32,
            l.gid.map_or(65534, |g| ns.gid_munged(g)) as i32,
            l.ktype,
            l.describe
        ));
    }
    out
}

/// `/proc/key-users` for `reader`.
pub fn proc_key_users(reader: &Caller) -> String {
    let ns = reader.userns.clone();
    let ks = store().lock();
    let mut out = String::new();
    for (kuid, u) in ks.users() {
        let Some(uid) = ns.uid_up(kuid) else {
            continue;
        };
        let (maxkeys, maxbytes) = ks.limits.of(kuid);
        out.push_str(&format!(
            "{:5}: {:5} {}/{} {}/{} {}/{}\n",
            uid, u.nkeys, u.nkeys, u.nikeys, u.qnkeys, maxkeys, u.qnbytes, maxbytes
        ));
    }
    out
}

/// The `/proc/sys/kernel/keys` sysctls.
pub const SYSCTLS: &[&str] = &[
    "gc_delay",
    "maxbytes",
    "maxkeys",
    "persistent_keyring_expiry",
    "root_maxbytes",
    "root_maxkeys",
];

pub fn sysctl_text(name: &str) -> String {
    let l = store().lock().limits;
    let v: u64 = match name {
        "gc_delay" => l.gc_delay,
        "maxbytes" => l.maxbytes.into(),
        "maxkeys" => l.maxkeys.into(),
        "persistent_keyring_expiry" => l.persistent_expiry,
        "root_maxbytes" => l.root_maxbytes.into(),
        "root_maxkeys" => l.root_maxkeys.into(),
        _ => 0,
    };
    format!("{v}\n")
}

/// Write a keys sysctl: an integer in [1, INT_MAX] for the quotas, [0,
/// INT_MAX] for the delays (proc_dointvec_minmax). The files are root's,
/// 0644.
pub fn sysctl_write(name: &str, text: &[u8]) -> Result<(), SyscallError> {
    let s = core::str::from_utf8(text).map_err(|_| SyscallError::InvalidArgument)?;
    let v: i64 = s
        .trim()
        .parse()
        .map_err(|_| SyscallError::InvalidArgument)?;
    let min = match name {
        "gc_delay" | "persistent_keyring_expiry" => 0,
        _ => 1,
    };
    if v < min || v > i64::from(i32::MAX) {
        return Err(SyscallError::InvalidArgument);
    }
    let mut ks = store().lock();
    let l = &mut ks.limits;
    match name {
        "gc_delay" => l.gc_delay = v as u64,
        "maxbytes" => l.maxbytes = v as u32,
        "maxkeys" => l.maxkeys = v as u32,
        "persistent_keyring_expiry" => l.persistent_expiry = v as u64,
        "root_maxbytes" => l.root_maxbytes = v as u32,
        "root_maxkeys" => l.root_maxkeys = v as u32,
        _ => return Err(SyscallError::InvalidArgument),
    }
    Ok(())
}

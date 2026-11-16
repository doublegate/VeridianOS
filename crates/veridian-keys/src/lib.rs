//! The Linux 7.3 key retention service, its core (`security/keys/`): the
//! `user`, `logon` and `keyring` key types, the special keyrings, quotas,
//! permissions, searches and every `add_key`, `request_key` and `keyctl`
//! operation of the core, with Linux's checks in Linux's order.
//!
//! The kernel and the user-space Linux runtime (Track M) share this crate,
//! as `veridian-cbpf` and `veridian-landlock`. It is `no_std` (with
//! `alloc`) and has no `unsafe`. The caller decodes the system call (user
//! strings, buffers), says who is asking ([`Subject`]) and keeps each
//! thread's [`KeyCred`]; everything else is here.
//!
//! Errors are Linux errno values ([`errno`]). Times are seconds of the
//! real-time clock, as Linux's `ktime_get_real_seconds`.
//!
//! There is no usermode helper, so `request_key` behaves as Linux does with
//! no `/sbin/request-key`: the key it makes is negated for 60 s and the
//! caller gets ENOENT. No instantiation authority is ever assumed, so
//! INSTANTIATE, NEGATE, REJECT and ASSUME_AUTHORITY take their paths for a
//! caller that holds none.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

/// A key's serial number (`key_serial_t`).
pub type Serial = i32;

/// Linux errno values the core returns.
pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const EIO: i32 = 5;
    pub const EACCES: i32 = 13;
    /// A bad user pointer (the type or restriction string of
    /// KEYCTL_RESTRICT_KEYRING), passed through the crate's results.
    pub const EFAULT: i32 = 14;
    pub const EBUSY: i32 = 16;
    pub const EEXIST: i32 = 17;
    pub const ENODEV: i32 = 19;
    pub const ENOTDIR: i32 = 20;
    pub const EINVAL: i32 = 22;
    pub const EDEADLK: i32 = 35;
    pub const ELOOP: i32 = 40;
    pub const EAGAIN: i32 = 11;
    pub const EOPNOTSUPP: i32 = 95;
    pub const EDQUOT: i32 = 122;
    pub const ENOKEY: i32 = 126;
    pub const EKEYEXPIRED: i32 = 127;
    pub const EKEYREVOKED: i32 = 128;
}

use errno::*;

/// A result whose error is a Linux errno.
pub type KResult<T> = Result<T, i32>;

/// The special key ids (KEY_SPEC_*).
pub mod spec {
    pub const THREAD: i32 = -1;
    pub const PROCESS: i32 = -2;
    pub const SESSION: i32 = -3;
    pub const USER: i32 = -4;
    pub const USER_SESSION: i32 = -5;
    pub const GROUP: i32 = -6;
    pub const REQKEY_AUTH_KEY: i32 = -7;
    pub const REQUESTOR: i32 = -8;
}

/// request_key's default keyrings (KEY_REQKEY_DEFL_*).
pub mod defl {
    pub const NO_CHANGE: i32 = -1;
    pub const DEFAULT: i32 = 0;
    pub const THREAD: i32 = 1;
    pub const PROCESS: i32 = 2;
    pub const SESSION: i32 = 3;
    pub const USER: i32 = 4;
    pub const USER_SESSION: i32 = 5;
    pub const GROUP: i32 = 6;
    pub const REQUESTOR: i32 = 7;
}

/// Permission bits (KEY_POS_*, KEY_USR_*, KEY_GRP_*, KEY_OTH_*), one byte
/// each from the possessor's down to other's.
pub mod perm {
    pub const VIEW: u32 = 0x01;
    pub const READ: u32 = 0x02;
    pub const WRITE: u32 = 0x04;
    pub const SEARCH: u32 = 0x08;
    pub const LINK: u32 = 0x10;
    pub const SETATTR: u32 = 0x20;
    /// What SETPERM accepts.
    pub const VALID: u32 = 0x3f3f_3f3f;
    pub const GRP_ALL: u32 = 0x0000_3f00;
}

/// The longest user or logon payload.
pub const MAX_PAYLOAD: usize = 32767;
/// How deep keyrings nest below the one searched (KEYRING_SEARCH_MAX_DEPTH).
pub const MAX_DEPTH: usize = 6;
/// What a keyring link costs its owner's byte quota (KEYQUOTA_LINK_BYTES).
pub const LINK_BYTES: u32 = 4;
/// How long a key request_key could not construct stays negative.
pub const NEGATIVE_TIMEOUT: u64 = 60;

/// The core's key types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KType {
    User,
    Logon,
    Keyring,
}

impl KType {
    /// `key_type_lookup` of the core's types.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "user" => Some(Self::User),
            "logon" => Some(Self::Logon),
            "keyring" => Some(Self::Keyring),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Logon => "logon",
            Self::Keyring => "keyring",
        }
    }

    /// The default permissions (key_create_or_update with KEY_PERM_UNDEF).
    /// Possessor: View, Search, Link, Setattr, Read if the type can be
    /// read (not logon) and Write (all three update or are keyrings); the
    /// owner: View.
    fn default_perm(self) -> u32 {
        let mut pos = perm::VIEW | perm::SEARCH | perm::LINK | perm::SETATTR | perm::WRITE;
        if self != Self::Logon {
            pos |= perm::READ;
        }
        pos << 24 | perm::VIEW << 16
    }
}

/// Key flags (KEY_FLAG_*) the core uses.
pub mod flags {
    pub const DEAD: u32 = 1 << 0;
    pub const REVOKED: u32 = 1 << 1;
    pub const IN_QUOTA: u32 = 1 << 2;
    pub const USER_CONSTRUCT: u32 = 1 << 3;
    pub const INVALIDATED: u32 = 1 << 5;
    pub const UID_KEYRING: u32 = 1 << 9;
}

/// How a new key is charged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quota {
    /// Checked against the owner's limits and charged.
    In,
    /// Charged without the check (KEY_ALLOC_QUOTA_OVERRUN).
    Overrun,
    /// Neither (KEY_ALLOC_NOT_IN_QUOTA).
    Not,
}

#[derive(Debug, Clone)]
enum Payload {
    Data(Vec<u8>),
    /// Links by (type, description): at most one key per pair.
    Ring(BTreeMap<(KType, String), Serial>),
    /// Revoked user or logon key: the payload is gone.
    Gone,
}

/// One key.
#[derive(Debug, Clone)]
pub struct Key {
    pub serial: Serial,
    pub ktype: KType,
    pub desc: String,
    /// Owner (a kernel uid).
    pub uid: u32,
    /// The user charged for it (Linux's `key->user`): set when it is made
    /// and moved only by chown, so a thread keyring that `fsid_changed`
    /// gives a new owner is still refunded to the user it was charged to
    /// (PR #49 review).
    quota_uid: u32,
    /// Group, `None` for INVALID_GID.
    pub gid: Option<u32>,
    pub perm: u32,
    pub flags: u32,
    /// 0 under construction, 1 positive, a negative errno when negative.
    pub state: i32,
    /// When it expires; `None` is permanent.
    pub expiry: Option<u64>,
    /// References: links from keyrings and the threads that hold it.
    pub usage: u32,
    /// What it is charged (description, payload, links).
    quotalen: u32,
    /// The payload's size, or 4 bytes a link for a keyring.
    datalen: u32,
    payload: Payload,
    /// A keyring every link into is refused (reject-all restriction).
    restricted: bool,
    /// The user namespace its name is published in, for joining by name.
    published: Option<u64>,
    /// Creation order, for name lookup as Linux's list.
    seq: u64,
}

impl Key {
    fn is_keyring(&self) -> bool {
        self.ktype == KType::Keyring
    }

    fn expired(&self, now: u64) -> bool {
        self.expiry.is_some_and(|e| now >= e)
    }

    /// key_validate.
    fn validate(&self, now: u64) -> KResult<()> {
        if self.flags & flags::INVALIDATED != 0 {
            return Err(ENOKEY);
        }
        if self.flags & (flags::REVOKED | flags::DEAD) != 0 {
            return Err(EKEYREVOKED);
        }
        if self.expired(now) {
            return Err(EKEYEXPIRED);
        }
        Ok(())
    }

    /// The links of a keyring (none for another key).
    fn links(&self) -> impl Iterator<Item = Serial> + '_ {
        let ring = match &self.payload {
            Payload::Ring(r) => Some(r),
            _ => None,
        };
        ring.into_iter().flat_map(|r| r.values().copied())
    }

    /// How many links a keyring has.
    pub fn link_count(&self) -> usize {
        match &self.payload {
            Payload::Ring(r) => r.len(),
            _ => 0,
        }
    }

    /// The payload's length (user and logon keys).
    pub fn data_len(&self) -> usize {
        match &self.payload {
            Payload::Data(d) => d.len(),
            _ => 0,
        }
    }
}

/// Who asks: the caller's credentials, as kernel ids.
#[derive(Debug, Clone)]
pub struct Subject {
    pub uid: u32,
    pub euid: u32,
    pub suid: u32,
    pub fsuid: u32,
    pub gid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub fsgid: u32,
    pub groups: Vec<u32>,
    /// The caller's user namespace.
    pub ns: u64,
    /// The caller's real uid as its namespace numbers it ("_uid.N").
    pub ns_uid: u32,
    /// The owner of the caller's user namespace (owns its registers).
    pub ns_owner: u32,
    /// CAP_SYS_ADMIN in the initial user namespace (`capable`): CHOWN's
    /// and SETPERM's override of ownership.
    pub sys_admin: bool,
    /// CAP_SETUID in the caller's namespace.
    pub setuid: bool,
    /// Now, in seconds.
    pub now: u64,
}

impl Subject {
    fn in_group(&self, gid: u32) -> bool {
        gid == self.fsgid || self.groups.contains(&gid)
    }
}

/// A thread's keys (the key fields of Linux's `struct cred`). Every
/// serial in it holds a reference: [`KeyStore::hold_cred`] when one is
/// copied, [`KeyStore::release_cred`] when one goes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyCred {
    pub thread: Option<Serial>,
    pub process: Option<Serial>,
    pub session: Option<Serial>,
    /// The assumed instantiation authority (never set: no usermode helper).
    pub auth: Option<Serial>,
    /// request_key's default keyring (KEY_REQKEY_DEFL_*).
    pub jit: i32,
}

/// Per-uid accounting (`struct key_user`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyUser {
    pub nkeys: u32,
    pub nikeys: u32,
    pub qnkeys: u32,
    pub qnbytes: u32,
}

/// The quota and lifetime sysctls (`/proc/sys/kernel/keys`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub maxkeys: u32,
    pub maxbytes: u32,
    pub root_maxkeys: u32,
    pub root_maxbytes: u32,
    pub gc_delay: u64,
    pub persistent_expiry: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            maxkeys: 200,
            maxbytes: 20000,
            root_maxkeys: 1_000_000,
            root_maxbytes: 25_000_000,
            gc_delay: 300,
            persistent_expiry: 3 * 24 * 3600,
        }
    }
}

impl Limits {
    /// The limits of owner `uid` (the root ones for the initial namespace's
    /// root only).
    pub fn of(&self, uid: u32) -> (u32, u32) {
        if uid == 0 {
            (self.root_maxkeys, self.root_maxbytes)
        } else {
            (self.maxkeys, self.maxbytes)
        }
    }
}

/// What a lookup needs of the key (KEY_NEED_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Perm(u32),
    /// No validity or permission check.
    Unlink,
    /// The caller checks permission itself (KEYCTL_READ).
    Defer,
}

const CREATE: u8 = 1;
const PARTIAL: u8 = 2;

/// A key as KEYCTL_DESCRIBE and /proc/keys show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    pub ktype: &'static str,
    pub uid: u32,
    pub gid: Option<u32>,
    pub perm: u32,
    pub desc: String,
}

/// What KEYCTL_READ read: the bytes, and whether they are a keyring's
/// serials (whose buffer must be a multiple of 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOut {
    pub data: Vec<u8>,
    pub keyring: bool,
}

/// One /proc/keys line's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcKey {
    pub serial: Serial,
    pub flags: [u8; 7],
    pub usage: u32,
    /// `perm`, `expd`, or the time left.
    pub timeout: String,
    pub perm: u32,
    pub uid: u32,
    pub gid: Option<u32>,
    pub ktype: &'static str,
    /// The type's describe text.
    pub describe: String,
}

/// RESTRICT_KEYRING's two strings as the caller copied them in: `None` for
/// a NULL pointer, else the copy's result.
pub struct RestrictArgs {
    pub ktype: Option<KResult<String>>,
    pub restriction: Option<KResult<String>>,
}

/// Every key, and what goes with them.
pub struct KeyStore {
    keys: BTreeMap<Serial, Key>,
    users: BTreeMap<u32, KeyUser>,
    /// Per user namespace: the user-keyring register (".user_reg").
    user_reg: BTreeMap<u64, Serial>,
    /// Per user namespace: the persistent-keyring register.
    persistent_reg: BTreeMap<u64, Serial>,
    pub limits: Limits,
    random: fn() -> u32,
    seq: u64,
    /// When a key next becomes dead (expiry + gc_delay), for the sweep.
    next_gc: Option<u64>,
}

impl KeyStore {
    /// An empty store; `random` gives the serials' random bits.
    pub fn new(random: fn() -> u32) -> Self {
        Self {
            keys: BTreeMap::new(),
            users: BTreeMap::new(),
            user_reg: BTreeMap::new(),
            persistent_reg: BTreeMap::new(),
            limits: Limits::default(),
            random,
            seq: 0,
            next_gc: None,
        }
    }

    /// The key `serial`, if it is live.
    pub fn key(&self, serial: Serial) -> Option<&Key> {
        self.keys.get(&serial).filter(|k| k.usage > 0)
    }

    /// Every user's accounting, by uid (/proc/key-users).
    pub fn users(&self) -> impl Iterator<Item = (u32, KeyUser)> + '_ {
        self.users.iter().map(|(u, k)| (*u, *k))
    }

    fn key_mut(&mut self, serial: Serial) -> KResult<&mut Key> {
        self.keys.get_mut(&serial).ok_or(ENOKEY)
    }

    fn k(&self, serial: Serial) -> KResult<&Key> {
        self.keys.get(&serial).ok_or(ENOKEY)
    }

    // ---- references and lifetime -------------------------------------

    fn get(&mut self, serial: Serial) {
        if let Some(k) = self.keys.get_mut(&serial) {
            k.usage += 1;
        }
    }

    /// Drop a reference; the last one frees the key (key_put, then the
    /// garbage collector): its quota is refunded and a keyring's links go.
    fn put(&mut self, serial: Serial) {
        let mut work = Vec::from([serial]);
        while let Some(s) = work.pop() {
            let Some(k) = self.keys.get_mut(&s) else {
                continue;
            };
            k.usage = k.usage.saturating_sub(1);
            if k.usage > 0 {
                continue;
            }
            let Some(k) = self.keys.remove(&s) else {
                continue;
            };
            let user = self.users.entry(k.quota_uid).or_default();
            if k.flags & flags::IN_QUOTA != 0 {
                user.qnkeys = user.qnkeys.saturating_sub(1);
                user.qnbytes = user.qnbytes.saturating_sub(k.quotalen);
            }
            user.nkeys = user.nkeys.saturating_sub(1);
            if k.state != 0 {
                user.nikeys = user.nikeys.saturating_sub(1);
            }
            if *user == KeyUser::default() {
                self.users.remove(&k.quota_uid);
            }
            work.extend(k.links());
        }
    }

    /// The thread's references, once more (a copy of its keys).
    pub fn hold_cred(&mut self, kc: &KeyCred) {
        for s in [kc.thread, kc.process, kc.session, kc.auth]
            .into_iter()
            .flatten()
        {
            self.get(s);
        }
    }

    /// A thread's keys go (it exited, or its credentials were replaced).
    pub fn release_cred(&mut self, kc: &KeyCred) {
        for s in [kc.thread, kc.process, kc.session, kc.auth]
            .into_iter()
            .flatten()
        {
            self.put(s);
        }
    }

    /// Set `slot` to `serial` (one reference taken), dropping the old.
    fn install(&mut self, slot: &mut Option<Serial>, serial: Serial) {
        self.get(serial);
        if let Some(old) = slot.replace(serial) {
            self.put(old);
        }
    }

    fn new_serial(&mut self) -> Serial {
        let mut s = ((self.random)() >> 1) as i32;
        while s < 3 {
            s = ((self.random)() >> 1) as i32;
        }
        while self.keys.contains_key(&s) {
            s = if s == i32::MAX { 3 } else { s + 1 };
        }
        s
    }

    /// key_alloc: a new key with one reference, the caller's.
    #[allow(clippy::too_many_arguments)]
    fn alloc(
        &mut self,
        ktype: KType,
        desc: &str,
        uid: u32,
        gid: Option<u32>,
        perm: u32,
        quota: Quota,
        flags_: u32,
        ns: u64,
    ) -> KResult<Serial> {
        if desc.is_empty() {
            return Err(EINVAL);
        }
        // logon: "service:name", the service not empty.
        if ktype == KType::Logon && desc.find(':').is_none_or(|i| i == 0) {
            return Err(EINVAL);
        }
        let quotalen = desc.len() as u32 + 1;
        let (maxkeys, maxbytes) = self.limits.of(uid);
        let user = self.users.entry(uid).or_default();
        if quota == Quota::In
            && (user.qnkeys + 1 > maxkeys
                || user
                    .qnbytes
                    .checked_add(quotalen)
                    .is_none_or(|b| b > maxbytes))
        {
            return Err(EDQUOT);
        }
        let mut f = flags_;
        if quota != Quota::Not {
            user.qnkeys += 1;
            user.qnbytes += quotalen;
            f |= flags::IN_QUOTA;
        }
        user.nkeys += 1;
        let serial = self.new_serial();
        self.seq += 1;
        let published = (ktype == KType::Keyring && !desc.starts_with('.')).then_some(ns);
        self.keys.insert(
            serial,
            Key {
                serial,
                ktype,
                desc: desc.to_string(),
                uid,
                quota_uid: uid,
                gid,
                perm,
                flags: f,
                state: 0,
                expiry: None,
                usage: 1,
                quotalen: if quota != Quota::Not { quotalen } else { 0 },
                datalen: 0,
                payload: if ktype == KType::Keyring {
                    Payload::Ring(BTreeMap::new())
                } else {
                    Payload::Data(Vec::new())
                },
                restricted: false,
                published,
                seq: self.seq,
            },
        );
        Ok(serial)
    }

    /// key_payload_reserve: charge the key's owner for `len` bytes of data.
    fn reserve(&mut self, serial: Serial, len: u32) -> KResult<()> {
        let k = self.k(serial)?;
        let (uid, in_quota, old) = (k.quota_uid, k.flags & flags::IN_QUOTA != 0, k.datalen);
        if in_quota {
            let (_, maxbytes) = self.limits.of(uid);
            let user = self.users.entry(uid).or_default();
            if len > old {
                let more = len - old;
                if user.qnbytes.checked_add(more).is_none_or(|b| b > maxbytes) {
                    return Err(EDQUOT);
                }
                user.qnbytes += more;
            } else {
                user.qnbytes = user.qnbytes.saturating_sub(old - len);
            }
        }
        let k = self.key_mut(serial)?;
        if in_quota {
            k.quotalen = k.quotalen - old + len;
        }
        k.datalen = len;
        Ok(())
    }

    /// Mark a key positive (instantiated) or negative.
    fn instantiate(&mut self, serial: Serial, state: i32) {
        if let Some(k) = self.keys.get_mut(&serial) {
            let first = k.state == 0;
            k.state = state;
            k.flags &= !flags::USER_CONSTRUCT;
            if first {
                self.users.entry(k.quota_uid).or_default().nikeys += 1;
            }
        }
    }

    // ---- garbage collection ------------------------------------------

    fn dead(&self, k: &Key, now: u64) -> bool {
        k.flags & (flags::INVALIDATED | flags::DEAD) != 0
            || k.expiry
                .is_some_and(|e| e.saturating_add(self.limits.gc_delay) <= now)
    }

    /// The garbage collector's link sweep: unlink every dead key (expired
    /// for gc_delay, or invalidated) from every keyring, once one is due.
    pub fn gc(&mut self, now: u64) {
        if self.next_gc.is_none_or(|t| now < t) {
            return;
        }
        self.sweep(now);
    }

    fn sweep(&mut self, now: u64) {
        let dead: Vec<Serial> = self
            .keys
            .values()
            .filter(|k| self.dead(k, now))
            .map(|k| k.serial)
            .collect();
        // Per keyring, the dead keys it links.
        let mut unlinks: Vec<(Serial, Vec<(KType, String)>)> = Vec::new();
        for k in self.keys.values() {
            if k.flags & (flags::REVOKED | flags::INVALIDATED) != 0 {
                continue;
            }
            if let Payload::Ring(m) = &k.payload {
                let gone: Vec<(KType, String)> = m
                    .iter()
                    .filter(|(_, s)| dead.contains(s))
                    .map(|(i, _)| i.clone())
                    .collect();
                if !gone.is_empty() {
                    unlinks.push((k.serial, gone));
                }
            }
        }
        for (ring, gone) in unlinks {
            let mut drops = Vec::new();
            if let Some(Key {
                payload: Payload::Ring(m),
                ..
            }) = self.keys.get_mut(&ring)
            {
                for i in &gone {
                    if let Some(s) = m.remove(i) {
                        drops.push(s);
                    }
                }
            }
            let len = self.keys.get(&ring).map_or(0, |k| {
                k.datalen.saturating_sub(drops.len() as u32 * LINK_BYTES)
            });
            let _ = self.reserve(ring, len);
            for s in drops {
                self.put(s);
            }
        }
        self.next_gc = self
            .keys
            .values()
            .filter_map(|k| k.expiry)
            .map(|e| e.saturating_add(self.limits.gc_delay))
            .filter(|t| *t > now)
            .min();
    }

    fn schedule(&mut self, expiry: u64) {
        let t = expiry.saturating_add(self.limits.gc_delay);
        self.next_gc = Some(self.next_gc.map_or(t, |n| n.min(t)));
    }

    // ---- permission ----------------------------------------------------

    /// key_task_permission.
    fn permits(&self, k: &Key, subj: &Subject, possessed: bool, mask: u32) -> bool {
        let p = k.perm;
        let mut kperm = if k.uid == subj.fsuid {
            p >> 16
        } else if k
            .gid
            .is_some_and(|g| p & perm::GRP_ALL != 0 && subj.in_group(g))
        {
            p >> 8
        } else {
            p
        };
        if possessed {
            kperm |= p >> 24;
        }
        kperm & mask & 0x3f == mask
    }

    // ---- links -------------------------------------------------------

    /// Whether `key`'s index is already linked in `ring` (so linking it
    /// costs nothing).
    fn replaces(&self, ring: &Key, key: &Key) -> Option<Serial> {
        match &ring.payload {
            Payload::Ring(r) => r.get(&(key.ktype, key.desc.clone())).copied(),
            _ => None,
        }
    }

    /// The checks of key_link before anything changes: ENOTDIR, EKEYREVOKED,
    /// EDQUOT (the link's 4 bytes, unless it replaces one), EPERM for a
    /// restricted keyring, EDEADLK and ELOOP for a keyring cycle.
    fn link_check(&self, ring: Serial, key: Serial, excl: bool) -> KResult<bool> {
        self.link_check_as(ring, key, excl, true)
    }

    /// [`Self::link_check`], with the ring's restriction applied or not:
    /// request_key's construction links its new key without it
    /// (construct_alloc_key's __key_link_begin and __key_link; PR #49
    /// independent review).
    fn link_check_as(
        &self,
        ring: Serial,
        key: Serial,
        excl: bool,
        restrict: bool,
    ) -> KResult<bool> {
        let r = self.k(ring)?;
        if !r.is_keyring() {
            return Err(ENOTDIR);
        }
        let k = self.k(key)?;
        if r.flags & flags::REVOKED != 0 {
            return Err(EKEYREVOKED);
        }
        let replace = self.replaces(r, k).is_some();
        if !replace && r.flags & flags::IN_QUOTA != 0 {
            let (_, maxbytes) = self.limits.of(r.quota_uid);
            let used = self.users.get(&r.quota_uid).map_or(0, |u| u.qnbytes);
            if used.checked_add(LINK_BYTES).is_none_or(|b| b > maxbytes) {
                return Err(EDQUOT);
            }
        }
        if excl && replace {
            return Err(EEXIST);
        }
        if restrict && r.restricted {
            return Err(EPERM);
        }
        if k.is_keyring() {
            self.cycle_check(key, ring)?;
        }
        Ok(replace)
    }

    /// Whether `ring` is reachable from keyring `from` (or is it): EDEADLK;
    /// nesting past [`MAX_DEPTH`]: ELOOP.
    fn cycle_check(&self, from: Serial, ring: Serial) -> KResult<()> {
        let mut stack: Vec<(Serial, usize)> = Vec::from([(from, 0)]);
        while let Some((s, depth)) = stack.pop() {
            if s == ring {
                return Err(EDEADLK);
            }
            let Some(k) = self.keys.get(&s) else {
                continue;
            };
            for child in k.links() {
                if self.keys.get(&child).is_some_and(|c| c.is_keyring()) {
                    if child == ring {
                        return Err(EDEADLK);
                    }
                    if depth >= MAX_DEPTH {
                        return Err(ELOOP);
                    }
                    stack.push((child, depth + 1));
                }
            }
        }
        Ok(())
    }

    /// Link `key` into `ring` after [`Self::link_check`].
    fn link_commit(&mut self, ring: Serial, key: Serial, replace: bool) -> KResult<()> {
        if !replace {
            let len = self.k(ring)?.datalen + LINK_BYTES;
            self.reserve(ring, len)?;
        }
        let (kt, kd) = {
            let k = self.k(key)?;
            (k.ktype, k.desc.clone())
        };
        self.get(key);
        let old = match &mut self.key_mut(ring)?.payload {
            Payload::Ring(r) => r.insert((kt, kd), key),
            _ => None,
        };
        if let Some(old) = old {
            self.put(old);
        }
        Ok(())
    }

    /// key_link.
    fn link(&mut self, ring: Serial, key: Serial) -> KResult<()> {
        let replace = self.link_check(ring, key, false)?;
        self.link_commit(ring, key, replace)
    }

    /// key_unlink: ENOTDIR, ENOENT.
    fn unlink(&mut self, ring: Serial, key: Serial) -> KResult<()> {
        let r = self.k(ring)?;
        if !r.is_keyring() {
            return Err(ENOTDIR);
        }
        let k = self.k(key)?;
        let index = (k.ktype, k.desc.clone());
        let linked = match &r.payload {
            Payload::Ring(m) => m.get(&index) == Some(&key),
            _ => false,
        };
        if !linked {
            return Err(ENOENT);
        }
        if let Payload::Ring(m) = &mut self.key_mut(ring)?.payload {
            m.remove(&index);
        }
        let len = self.k(ring)?.datalen.saturating_sub(LINK_BYTES);
        let _ = self.reserve(ring, len);
        self.put(key);
        Ok(())
    }

    /// keyring_clear: every link goes.
    fn clear(&mut self, ring: Serial) -> KResult<()> {
        let k = self.key_mut(ring)?;
        if !k.is_keyring() {
            return Err(ENOTDIR);
        }
        let links: Vec<Serial> =
            match core::mem::replace(&mut k.payload, Payload::Ring(BTreeMap::new())) {
                Payload::Ring(r) => r.into_values().collect(),
                _ => Vec::new(),
            };
        let _ = self.reserve(ring, 0);
        for s in links {
            self.put(s);
        }
        Ok(())
    }

    // ---- searches ------------------------------------------------------

    /// keyring_search_rcu from `start`: the key of `ktype` and `desc` in it
    /// or in a keyring below it. `skip_expired` ignores expired keys
    /// (request_key); `check_state` makes the state checks.
    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        start: Serial,
        ktype: KType,
        desc: &str,
        subj: &Subject,
        possessed: bool,
        check_state: bool,
        skip_expired: bool,
        check_perm: bool,
    ) -> KResult<Serial> {
        let s = self.k(start)?;
        if !s.is_keyring() {
            return Err(ENOTDIR);
        }
        if check_perm && !self.permits(s, subj, possessed, perm::SEARCH) {
            return Err(EACCES);
        }
        // The keyring itself, when it is what is looked for.
        if ktype == KType::Keyring && s.desc == desc {
            return Ok(start);
        }
        let mut result = EAGAIN;
        let mut stack: Vec<(Serial, usize)> = Vec::from([(start, 0)]);
        while let Some((rs, depth)) = stack.pop() {
            let Some(r) = self.keys.get(&rs) else {
                continue;
            };
            let Payload::Ring(m) = &r.payload else {
                continue;
            };
            if let Some(&c) = m.get(&(ktype, desc.to_string())) {
                if let Some(k) = self.keys.get(&c) {
                    match self.candidate(k, subj, possessed, check_state, skip_expired, check_perm)
                    {
                        Ok(()) => return Ok(c),
                        Err(Some(e)) => result = e,
                        Err(None) => {}
                    }
                }
            }
            // Then the keyrings in it, depth first.
            let mut nested: Vec<Serial> = Vec::new();
            for c in m.values() {
                let Some(k) = self.keys.get(c) else {
                    continue;
                };
                if !k.is_keyring() || k.flags & (flags::REVOKED | flags::INVALIDATED) != 0 {
                    continue;
                }
                if depth >= MAX_DEPTH {
                    continue;
                }
                if check_perm && !self.permits(k, subj, possessed, perm::SEARCH) {
                    continue;
                }
                nested.push(*c);
            }
            for c in nested.into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
        Err(result)
    }

    /// One candidate of a search (keyring_search_iterator): `Ok` found,
    /// `Err(Some(e))` skipped with reason `e`, `Err(None)` ignored.
    fn candidate(
        &self,
        k: &Key,
        subj: &Subject,
        possessed: bool,
        check_state: bool,
        skip_expired: bool,
        check_perm: bool,
    ) -> Result<(), Option<i32>> {
        if check_state {
            if k.flags & (flags::INVALIDATED | flags::REVOKED) != 0 {
                return Err(Some(EKEYREVOKED));
            }
            if k.expired(subj.now) {
                return Err(if skip_expired {
                    None
                } else {
                    Some(EKEYEXPIRED)
                });
            }
        }
        if check_perm && !self.permits(k, subj, possessed, perm::SEARCH) {
            return Err(Some(EACCES));
        }
        if check_state && k.state < 0 {
            return Err(Some(-k.state));
        }
        Ok(())
    }

    /// Whether keyring `start` holds `target` (or is it), every keyring on
    /// the way and the key granting SEARCH to a possessor: possession.
    fn holds(&self, start: Serial, target: Serial, subj: &Subject) -> bool {
        let Some(s) = self.keys.get(&start) else {
            return false;
        };
        if !s.is_keyring() || !self.permits(s, subj, true, perm::SEARCH) {
            return false;
        }
        if start == target {
            return true;
        }
        let mut stack: Vec<(Serial, usize)> = Vec::from([(start, 0)]);
        while let Some((rs, depth)) = stack.pop() {
            let Some(r) = self.keys.get(&rs) else {
                continue;
            };
            for c in r.links() {
                let Some(k) = self.keys.get(&c) else {
                    continue;
                };
                if c == target && self.permits(k, subj, true, perm::SEARCH) {
                    return true;
                }
                if k.is_keyring()
                    && depth < MAX_DEPTH
                    && k.flags & (flags::REVOKED | flags::INVALIDATED) == 0
                    && self.permits(k, subj, true, perm::SEARCH)
                {
                    stack.push((c, depth + 1));
                }
            }
        }
        false
    }

    /// The keyrings a thread searches: thread, process, session, or (with
    /// no session keyring) its user-session keyring if it exists.
    fn cred_rings(&self, subj: &Subject, kc: &KeyCred) -> Vec<Serial> {
        let mut v: Vec<Serial> = [kc.thread, kc.process].into_iter().flatten().collect();
        match kc.session {
            Some(s) => v.push(s),
            None => {
                if let Some(us) = self.existing_user_ses(subj) {
                    v.push(us);
                }
            }
        }
        v
    }

    /// Whether the thread possesses `serial`.
    fn possessed(&self, subj: &Subject, kc: &KeyCred, serial: Serial) -> bool {
        self.cred_rings(subj, kc)
            .into_iter()
            .any(|r| self.holds(r, serial, subj))
    }

    /// search_cred_keyrings_rcu: success, then ENOKEY, then EAGAIN, then
    /// another error.
    fn search_cred(
        &self,
        subj: &Subject,
        kc: &KeyCred,
        ktype: KType,
        desc: &str,
        skip_expired: bool,
    ) -> KResult<Serial> {
        let mut ret: Option<i32> = None;
        let mut err = EAGAIN;
        for r in self.cred_rings(subj, kc) {
            match self.search(r, ktype, desc, subj, true, true, skip_expired, true) {
                Ok(s) => return Ok(s),
                Err(EAGAIN) => {
                    if ret.is_none() {
                        ret = Some(EAGAIN);
                    }
                }
                Err(ENOKEY) => ret = Some(ENOKEY),
                Err(e) => err = e,
            }
        }
        Err(ret.unwrap_or(err))
    }

    // ---- the special keyrings ------------------------------------------

    fn existing_user_ses(&self, subj: &Subject) -> Option<Serial> {
        let reg = *self.user_reg.get(&subj.ns)?;
        self.direct(reg, KType::Keyring, &format!("_uid_ses.{}", subj.ns_uid))
    }

    /// A non-recursive lookup in one keyring, skipping revoked and
    /// invalidated keys.
    fn direct(&self, ring: Serial, ktype: KType, desc: &str) -> Option<Serial> {
        let r = self.keys.get(&ring)?;
        let Payload::Ring(m) = &r.payload else {
            return None;
        };
        let s = *m.get(&(ktype, desc.to_string()))?;
        let k = self.keys.get(&s)?;
        (k.flags & (flags::REVOKED | flags::INVALIDATED) == 0).then_some(s)
    }

    /// A keyring as keyring_alloc makes one: instantiated (positive).
    #[allow(clippy::too_many_arguments)]
    fn new_ring(
        &mut self,
        desc: &str,
        uid: u32,
        gid: Option<u32>,
        perm_: u32,
        quota: Quota,
        flags_: u32,
        ns: u64,
    ) -> KResult<Serial> {
        let s = self.alloc(KType::Keyring, desc, uid, gid, perm_, quota, flags_, ns)?;
        self.instantiate(s, 1);
        Ok(s)
    }

    /// look_up_user_keyrings: the caller's user and user-session keyrings,
    /// made in its namespace's register when missing.
    fn user_rings(&mut self, subj: &Subject) -> KResult<(Serial, Serial)> {
        let reg = match self.user_reg.get(&subj.ns) {
            Some(&r) => r,
            None => {
                let r = self.new_ring(
                    ".user_reg",
                    subj.ns_owner,
                    None,
                    0x0c03_0000,
                    Quota::In,
                    0,
                    subj.ns,
                )?;
                self.user_reg.insert(subj.ns, r);
                r
            }
        };
        let uname = format!("_uid.{}", subj.ns_uid);
        let user = match self.direct(reg, KType::Keyring, &uname) {
            Some(s) => s,
            None => {
                let s = self.new_ring(
                    &uname,
                    subj.uid,
                    None,
                    0x1f3f_0000,
                    Quota::In,
                    flags::UID_KEYRING,
                    subj.ns,
                )?;
                let r = self.link(reg, s);
                self.put(s);
                r?;
                s
            }
        };
        let sname = format!("_uid_ses.{}", subj.ns_uid);
        let ses = match self.direct(reg, KType::Keyring, &sname) {
            Some(s) => s,
            None => {
                let s = self.new_ring(
                    &sname,
                    subj.uid,
                    None,
                    0x1f3f_0000,
                    Quota::In,
                    flags::UID_KEYRING,
                    subj.ns,
                )?;
                let r = self.link(s, user).and_then(|()| self.link(reg, s));
                self.put(s);
                r?;
                s
            }
        };
        Ok((user, ses))
    }

    fn make_thread_ring(&mut self, subj: &Subject, kc: &mut KeyCred) -> KResult<()> {
        let s = self.new_ring(
            "_tid",
            subj.uid,
            Some(subj.gid),
            0x3f01_0000,
            Quota::Overrun,
            0,
            subj.ns,
        )?;
        self.install(&mut kc.thread, s);
        self.put(s);
        Ok(())
    }

    fn make_process_ring(&mut self, subj: &Subject, kc: &mut KeyCred) -> KResult<()> {
        let s = self.new_ring(
            "_pid",
            subj.uid,
            Some(subj.gid),
            0x3f01_0000,
            Quota::Overrun,
            0,
            subj.ns,
        )?;
        self.install(&mut kc.process, s);
        self.put(s);
        Ok(())
    }

    /// join_session_keyring(NULL): a new anonymous session keyring.
    fn new_session(&mut self, subj: &Subject, kc: &mut KeyCred) -> KResult<Serial> {
        let quota = if kc.session.is_none() {
            Quota::Overrun
        } else {
            Quota::In
        };
        let s = self.new_ring(
            "_ses",
            subj.uid,
            Some(subj.gid),
            0x3f03_0000,
            quota,
            0,
            subj.ns,
        )?;
        self.install(&mut kc.session, s);
        self.put(s);
        Ok(s)
    }

    /// lookup_user_key.
    fn lookup(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        lflags: u8,
        need: Need,
    ) -> KResult<(Serial, bool)> {
        let create = lflags & CREATE != 0;
        let (serial, possessed) = match id {
            spec::THREAD => {
                if kc.thread.is_none() {
                    if !create {
                        return Err(ENOKEY);
                    }
                    self.make_thread_ring(subj, kc)?;
                }
                (kc.thread.ok_or(ENOKEY)?, true)
            }
            spec::PROCESS => {
                if kc.process.is_none() {
                    if !create {
                        return Err(ENOKEY);
                    }
                    self.make_process_ring(subj, kc)?;
                }
                (kc.process.ok_or(ENOKEY)?, true)
            }
            spec::SESSION => {
                match kc.session {
                    None => {
                        let (_, ses) = self.user_rings(subj)?;
                        if create {
                            self.new_session(subj, kc)?;
                        } else {
                            self.install(&mut kc.session, ses);
                        }
                    }
                    Some(s) => {
                        let uid_ring = self
                            .keys
                            .get(&s)
                            .is_some_and(|k| k.flags & flags::UID_KEYRING != 0);
                        if uid_ring && create {
                            self.new_session(subj, kc)?;
                        }
                    }
                }
                (kc.session.ok_or(ENOKEY)?, true)
            }
            spec::USER => (self.user_rings(subj)?.0, true),
            spec::USER_SESSION => (self.user_rings(subj)?.1, true),
            spec::GROUP => return Err(EINVAL),
            spec::REQKEY_AUTH_KEY => (kc.auth.ok_or(ENOKEY)?, true),
            spec::REQUESTOR => return Err(ENOKEY),
            id if id < 1 => return Err(EINVAL),
            id => {
                if self.key(id).is_none() {
                    return Err(ENOKEY);
                }
                (id, self.possessed(subj, kc, id))
            }
        };
        if need == Need::Unlink {
            return Ok((serial, possessed));
        }
        let k = self.k(serial)?;
        if need != Need::Defer {
            if lflags & PARTIAL == 0 {
                if k.state < 0 {
                    return Err(-k.state);
                }
                k.validate(subj.now)?;
                if k.state == 0 {
                    return Err(EIO);
                }
            } else {
                k.validate(subj.now)?;
            }
        }
        if let Need::Perm(mask) = need {
            if !self.permits(k, subj, possessed, mask) {
                return Err(EACCES);
            }
        }
        Ok((serial, possessed))
    }

    // ---- inheritance ---------------------------------------------------

    /// fork without CLONE_THREAD: no thread or process keyring; the session
    /// keyring, authority and default kept. The result holds its
    /// references.
    pub fn fork_cred(&mut self, kc: &KeyCred) -> KeyCred {
        let child = KeyCred {
            thread: None,
            process: None,
            ..kc.clone()
        };
        self.hold_cred(&child);
        child
    }

    /// clone with CLONE_THREAD: the process keyring shared; a new thread
    /// keyring if the creator has one (made without failing the clone).
    pub fn thread_cred(&mut self, subj: &Subject, kc: &KeyCred) -> KeyCred {
        let mut child = KeyCred {
            thread: None,
            ..kc.clone()
        };
        self.hold_cred(&child);
        if kc.thread.is_some() {
            let _ = self.make_thread_ring(subj, &mut child);
        }
        child
    }

    /// execve: the thread and process keyrings go.
    pub fn exec_cred(&mut self, kc: &mut KeyCred) {
        for s in [kc.thread.take(), kc.process.take()].into_iter().flatten() {
            self.put(s);
        }
    }

    /// The fsuid or fsgid changed: the thread keyring follows.
    pub fn fsid_changed(&mut self, subj: &Subject, kc: &KeyCred) {
        if let Some(k) = kc.thread.and_then(|t| self.keys.get_mut(&t)) {
            k.uid = subj.fsuid;
            k.gid = Some(subj.fsgid);
        }
    }

    // ---- add_key and request_key ---------------------------------------

    /// add_key(type, description, payload, ringid), after the caller copied
    /// the type (EFAULT, EINVAL, EPERM for a '.' type), the description
    /// (`None` for NULL or "") and the payload (EINVAL past 1 MiB, EFAULT).
    pub fn add_key(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        ktype: &str,
        desc: Option<&str>,
        payload: &[u8],
        ringid: i32,
    ) -> KResult<Serial> {
        self.gc(subj.now);
        if ktype.starts_with("keyring") && desc.is_some_and(|d| d.starts_with('.')) {
            return Err(EPERM);
        }
        let (ring, ring_pos) = self.lookup(subj, kc, ringid, CREATE, Need::Perm(perm::WRITE))?;
        let ktype = KType::named(ktype).ok_or(ENODEV)?;
        if !self.k(ring)?.is_keyring() {
            return Err(ENOTDIR);
        }
        match ktype {
            KType::Keyring if !payload.is_empty() => return Err(EINVAL),
            KType::User | KType::Logon if payload.is_empty() || payload.len() > MAX_PAYLOAD => {
                return Err(EINVAL)
            }
            _ => {}
        }
        let desc = desc.ok_or(EINVAL)?;
        // __key_link_begin and the restriction, as for any link.
        let r = self.k(ring)?;
        if r.flags & flags::REVOKED != 0 {
            return Err(EKEYREVOKED);
        }
        let existing = match &r.payload {
            Payload::Ring(m) => m.get(&(ktype, desc.to_string())).copied(),
            _ => None,
        };
        if existing.is_none() && r.flags & flags::IN_QUOTA != 0 {
            let (_, maxbytes) = self.limits.of(r.quota_uid);
            let used = self.users.get(&r.quota_uid).map_or(0, |u| u.qnbytes);
            if used.checked_add(LINK_BYTES).is_none_or(|b| b > maxbytes) {
                return Err(EDQUOT);
            }
        }
        if r.restricted {
            return Err(EPERM);
        }
        if !self.permits(r, subj, ring_pos, perm::WRITE) {
            return Err(EACCES);
        }
        // A user or logon key of the same description is updated.
        if ktype != KType::Keyring {
            if let Some(old) = self.direct(ring, ktype, desc) {
                let k = self.k(old)?;
                if !self.permits(k, subj, ring_pos, perm::WRITE) {
                    return Err(EACCES);
                }
                self.update_payload(old, payload)?;
                return Ok(old);
            }
        }
        let s = self.alloc(
            ktype,
            desc,
            subj.fsuid,
            Some(subj.fsgid),
            ktype.default_perm(),
            Quota::In,
            0,
            subj.ns,
        )?;
        let r = (|| {
            if ktype != KType::Keyring {
                self.reserve(s, payload.len() as u32)?;
                self.key_mut(s)?.payload = Payload::Data(payload.to_vec());
            }
            self.instantiate(s, 1);
            self.link(ring, s)
        })();
        self.put(s);
        r.map(|()| s)
    }

    fn update_payload(&mut self, serial: Serial, payload: &[u8]) -> KResult<()> {
        self.reserve(serial, payload.len() as u32)?;
        let k = self.key_mut(serial)?;
        k.payload = Payload::Data(payload.to_vec());
        k.expiry = None;
        if k.state == 0 {
            self.instantiate(serial, 1);
        } else {
            // A negative key becomes positive (it was counted already).
            k.state = 1;
        }
        Ok(())
    }

    /// request_key(type, description, callout, destringid), after the copies
    /// (the description may be empty; `callout` `None` for NULL).
    pub fn request_key(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        ktype: &str,
        desc: &str,
        callout: Option<&[u8]>,
        destringid: i32,
    ) -> KResult<Serial> {
        self.gc(subj.now);
        let dest = if destringid != 0 {
            Some(
                self.lookup(subj, kc, destringid, CREATE, Need::Perm(perm::WRITE))?
                    .0,
            )
        } else {
            None
        };
        let ktype = KType::named(ktype).ok_or(ENOKEY)?;
        match self.search_cred(subj, kc, ktype, desc, true) {
            Ok(found) => {
                if let Some(d) = dest {
                    let k = self.k(found)?;
                    if !self.permits(k, subj, true, perm::LINK) {
                        return Err(EACCES);
                    }
                    self.link(d, found)?;
                }
                Ok(found)
            }
            Err(EAGAIN) => {
                if callout.is_none() {
                    return Err(ENOKEY);
                }
                self.construct(subj, kc, ktype, desc, dest)
            }
            Err(e) => Err(e),
        }
    }

    /// construct_key_and_link with no usermode helper: the key is made and
    /// linked, then negated for 60 s, and the caller gets ENOENT.
    fn construct(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        ktype: KType,
        desc: &str,
        dest: Option<Serial>,
    ) -> KResult<Serial> {
        if ktype == KType::Keyring {
            return Err(EPERM);
        }
        let dest = match dest {
            Some(d) => d,
            None => {
                let d = match kc.jit {
                    defl::DEFAULT | defl::REQUESTOR | defl::THREAD => kc
                        .thread
                        .or(kc.process)
                        .or(kc.session)
                        .map_or_else(|| self.user_rings(subj).map(|r| r.1), Ok)?,
                    defl::PROCESS => kc
                        .process
                        .or(kc.session)
                        .map_or_else(|| self.user_rings(subj).map(|r| r.1), Ok)?,
                    defl::SESSION => kc
                        .session
                        .map_or_else(|| self.user_rings(subj).map(|r| r.1), Ok)?,
                    defl::USER_SESSION => self.user_rings(subj)?.1,
                    defl::USER => self.user_rings(subj)?.0,
                    _ => return Err(EINVAL),
                };
                if !self.permits(self.k(d)?, subj, true, perm::WRITE) {
                    return Err(EACCES);
                }
                d
            }
        };
        let s = self.alloc(
            ktype,
            desc,
            subj.fsuid,
            Some(subj.fsgid),
            ktype.default_perm(),
            Quota::In,
            flags::USER_CONSTRUCT,
            subj.ns,
        )?;
        let linked = self
            .link_check_as(dest, s, false, false)
            .and_then(|replace| self.link_commit(dest, s, replace));
        if let Err(e) = linked {
            self.put(s);
            return Err(e);
        }
        // The helper could not run (ENOENT): negated for 60 s.
        self.instantiate(s, -ENOKEY);
        let exp = subj.now + NEGATIVE_TIMEOUT;
        if let Some(k) = self.keys.get_mut(&s) {
            k.expiry = Some(exp);
        }
        self.schedule(exp);
        self.put(s);
        Err(ENOENT)
    }

    // ---- keyctl -------------------------------------------------------

    /// KEYCTL_GET_KEYRING_ID.
    pub fn get_keyring_id(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        create: bool,
    ) -> KResult<Serial> {
        self.gc(subj.now);
        let f = if create { CREATE } else { 0 };
        self.lookup(subj, kc, id, f, Need::Perm(perm::SEARCH))
            .map(|r| r.0)
    }

    /// KEYCTL_JOIN_SESSION_KEYRING, the name copied (`None` for NULL).
    /// The serial, or 0 when the named keyring already is the session.
    pub fn join_session(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        name: Option<&str>,
    ) -> KResult<Serial> {
        self.gc(subj.now);
        let Some(name) = name else {
            return self.new_session(subj, kc);
        };
        if name.starts_with('.') {
            return Err(EPERM);
        }
        let found = self
            .keys
            .values()
            .filter(|k| {
                k.is_keyring()
                    && k.published == Some(subj.ns)
                    && k.desc == name
                    && k.usage > 0
                    && k.flags & flags::REVOKED == 0
                    && self.permits(k, subj, false, perm::SEARCH)
            })
            .min_by_key(|k| k.seq)
            .map(|k| k.serial);
        match found {
            Some(s) if kc.session == Some(s) => Ok(0),
            Some(s) => {
                self.install(&mut kc.session, s);
                Ok(s)
            }
            None => {
                let s = self.new_ring(
                    name,
                    subj.uid,
                    Some(subj.gid),
                    0x3f13_0000,
                    Quota::In,
                    0,
                    subj.ns,
                )?;
                self.install(&mut kc.session, s);
                self.put(s);
                Ok(s)
            }
        }
    }

    /// KEYCTL_UPDATE (the caller refused a payload past a page).
    pub fn update(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        payload: &[u8],
    ) -> KResult<()> {
        self.gc(subj.now);
        let (s, pos) = self.lookup(subj, kc, id, 0, Need::Perm(perm::WRITE))?;
        let k = self.k(s)?;
        if !self.permits(k, subj, pos, perm::WRITE) {
            return Err(EACCES);
        }
        if k.is_keyring() {
            return Err(EOPNOTSUPP);
        }
        if payload.is_empty() || payload.len() > MAX_PAYLOAD {
            return Err(EINVAL);
        }
        self.update_payload(s, payload)
    }

    /// KEYCTL_REVOKE.
    pub fn revoke(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32) -> KResult<()> {
        self.gc(subj.now);
        let s = match self.lookup(subj, kc, id, 0, Need::Perm(perm::WRITE)) {
            Err(EACCES) => self.lookup(subj, kc, id, 0, Need::Perm(perm::SETATTR))?.0,
            r => r?.0,
        };
        let now = subj.now;
        let k = self.key_mut(s)?;
        if k.flags & flags::REVOKED != 0 {
            return Ok(());
        }
        // The expiry is left as it was: a revoked key is not dead
        // (key_is_dead looks at the expiry and DEAD/INVALIDATED only), so it
        // stays linked and shows as it was in /proc/keys; revoked_at only
        // schedules the collector (key_revoke; PR #49 independent review).
        k.flags |= flags::REVOKED;
        let ring = k.is_keyring();
        if ring {
            self.clear(s)?;
        } else {
            self.key_mut(s)?.payload = Payload::Gone;
            let _ = self.reserve(s, 0);
        }
        self.schedule(now);
        Ok(())
    }

    /// KEYCTL_CHOWN, the ids mapped into kernel ids by the caller (`None`
    /// for -1; EINVAL for an unmapped one there).
    pub fn chown(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> KResult<()> {
        if uid.is_none() && gid.is_none() {
            return Ok(());
        }
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, CREATE | PARTIAL, Need::Perm(perm::SETATTR))?;
        let k = self.k(s)?;
        let uid_change = uid.is_some_and(|u| u != k.uid);
        // The key's own group is no change (keyctl_chown_key: !gid_eq(gid,
        // key->gid) && !in_group_p(gid); PR #49 independent review).
        let gid_privileged = gid.is_some_and(|g| Some(g) != k.gid && !subj.in_group(g));
        if (uid_change || gid_privileged) && !subj.sys_admin {
            return Err(EACCES);
        }
        if let Some(new) = uid.filter(|_| uid_change) {
            // The charge moves from the user it is on (keyctl_chown_key:
            // key->user), which fsid_changed may have left apart from uid.
            let (old, in_quota, qlen, inst) = (
                k.quota_uid,
                k.flags & flags::IN_QUOTA != 0,
                k.quotalen,
                k.state != 0,
            );
            let (maxkeys, maxbytes) = self.limits.of(new);
            let nu = self.users.get(&new).copied().unwrap_or_default();
            if in_quota
                && (nu.qnkeys + 1 > maxkeys
                    || nu.qnbytes.checked_add(qlen).is_none_or(|b| b > maxbytes))
            {
                return Err(EDQUOT);
            }
            let o = self.users.entry(old).or_default();
            o.nkeys = o.nkeys.saturating_sub(1);
            if inst {
                o.nikeys = o.nikeys.saturating_sub(1);
            }
            if in_quota {
                o.qnkeys = o.qnkeys.saturating_sub(1);
                o.qnbytes = o.qnbytes.saturating_sub(qlen);
            }
            if *o == KeyUser::default() {
                self.users.remove(&old);
            }
            let n = self.users.entry(new).or_default();
            n.nkeys += 1;
            if inst {
                n.nikeys += 1;
            }
            if in_quota {
                n.qnkeys += 1;
                n.qnbytes += qlen;
            }
            let k = self.key_mut(s)?;
            k.uid = new;
            k.quota_uid = new;
        }
        if let Some(g) = gid {
            self.key_mut(s)?.gid = Some(g);
        }
        Ok(())
    }

    /// KEYCTL_SETPERM.
    pub fn setperm(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32, p: u32) -> KResult<()> {
        if p & !perm::VALID != 0 {
            return Err(EINVAL);
        }
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, CREATE | PARTIAL, Need::Perm(perm::SETATTR))?;
        let k = self.key_mut(s)?;
        if k.uid != subj.fsuid && !subj.sys_admin {
            return Err(EACCES);
        }
        k.perm = p;
        Ok(())
    }

    /// KEYCTL_DESCRIBE.
    pub fn describe(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32) -> KResult<Description> {
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, PARTIAL, Need::Perm(perm::VIEW))?;
        let k = self.k(s)?;
        Ok(Description {
            ktype: k.ktype.name(),
            uid: k.uid,
            gid: k.gid,
            perm: k.perm,
            desc: k.desc.clone(),
        })
    }

    /// KEYCTL_CLEAR.
    pub fn clear_ring(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32) -> KResult<()> {
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, CREATE, Need::Perm(perm::WRITE))?;
        self.clear(s)
    }

    /// KEYCTL_LINK.
    pub fn link_key(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        ringid: i32,
    ) -> KResult<()> {
        self.gc(subj.now);
        let (ring, _) = self.lookup(subj, kc, ringid, CREATE, Need::Perm(perm::WRITE))?;
        let (key, _) = self.lookup(subj, kc, id, CREATE, Need::Perm(perm::LINK))?;
        self.link(ring, key)
    }

    /// KEYCTL_UNLINK.
    pub fn unlink_key(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        ringid: i32,
    ) -> KResult<()> {
        self.gc(subj.now);
        let (ring, _) = self.lookup(subj, kc, ringid, 0, Need::Perm(perm::WRITE))?;
        let (key, _) = self.lookup(subj, kc, id, PARTIAL, Need::Unlink)?;
        self.unlink(ring, key)
    }

    /// KEYCTL_SEARCH, the type and description copied.
    pub fn search_key(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        ringid: i32,
        ktype: &str,
        desc: &str,
        destid: i32,
    ) -> KResult<Serial> {
        self.gc(subj.now);
        let (ring, pos) = self.lookup(subj, kc, ringid, 0, Need::Perm(perm::SEARCH))?;
        let dest = if destid != 0 {
            Some(
                self.lookup(subj, kc, destid, CREATE, Need::Perm(perm::WRITE))?
                    .0,
            )
        } else {
            None
        };
        let ktype = KType::named(ktype).ok_or(ENOKEY)?;
        let found = match self.search(ring, ktype, desc, subj, pos, true, false, true) {
            Err(EAGAIN) => return Err(ENOKEY),
            r => r?,
        };
        if let Some(d) = dest {
            if !self.permits(self.k(found)?, subj, pos, perm::LINK) {
                return Err(EACCES);
            }
            self.link(d, found)?;
        }
        Ok(found)
    }

    /// KEYCTL_READ: the whole payload (the caller copies it only into a
    /// buffer large enough).
    pub fn read(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32) -> KResult<ReadOut> {
        self.gc(subj.now);
        let (s, pos) = self
            .lookup(subj, kc, id, 0, Need::Defer)
            .map_err(|_| ENOKEY)?;
        let k = self.k(s)?;
        if k.state < 0 {
            return Err(-k.state);
        }
        if !self.permits(k, subj, pos, perm::READ) && !pos {
            return Err(EACCES);
        }
        if k.ktype == KType::Logon {
            return Err(EOPNOTSUPP);
        }
        k.validate(subj.now)?;
        let data = match &k.payload {
            Payload::Data(d) => d.clone(),
            Payload::Ring(m) => m.values().flat_map(|s| s.to_ne_bytes()).collect(),
            Payload::Gone => Vec::new(),
        };
        Ok(ReadOut {
            data,
            keyring: k.is_keyring(),
        })
    }

    /// KEYCTL_INSTANTIATE and INSTANTIATE_IOV (`plen` the payload's
    /// length): only an assumed authority may, and none is.
    pub fn instantiate_key(&mut self, kc: &KeyCred, plen: usize) -> KResult<()> {
        if plen > 1_048_575 {
            return Err(EINVAL);
        }
        match kc.auth {
            None => Err(EPERM),
            Some(_) => Err(EKEYREVOKED),
        }
    }

    /// KEYCTL_REJECT (and NEGATE with `error` ENOKEY): as INSTANTIATE.
    pub fn reject_key(&mut self, kc: &KeyCred, error: i32) -> KResult<()> {
        if error <= 0 || error >= 4095 || matches!(error, 512 | 513 | 514 | 516) {
            return Err(EINVAL);
        }
        match kc.auth {
            None => Err(EPERM),
            Some(_) => Err(EKEYREVOKED),
        }
    }

    /// KEYCTL_SET_REQKEY_KEYRING: the old default.
    pub fn set_reqkey(&mut self, subj: &Subject, kc: &mut KeyCred, v: i32) -> KResult<i32> {
        let old = kc.jit;
        match v {
            defl::NO_CHANGE => return Ok(old),
            defl::THREAD => {
                if kc.thread.is_none() {
                    self.make_thread_ring(subj, kc)?;
                }
            }
            defl::PROCESS => {
                if kc.process.is_none() {
                    self.make_process_ring(subj, kc)?;
                }
            }
            defl::DEFAULT | defl::SESSION | defl::USER | defl::USER_SESSION | defl::REQUESTOR => {}
            _ => return Err(EINVAL),
        }
        kc.jit = v;
        Ok(old)
    }

    /// KEYCTL_SET_TIMEOUT (seconds; 0 is permanent).
    pub fn set_timeout(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        timeout: u32,
    ) -> KResult<()> {
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, CREATE | PARTIAL, Need::Perm(perm::SETATTR))?;
        let exp = (timeout != 0).then(|| subj.now + u64::from(timeout));
        self.key_mut(s)?.expiry = exp;
        if let Some(e) = exp {
            self.schedule(e);
        }
        Ok(())
    }

    /// KEYCTL_ASSUME_AUTHORITY: 0 drops the authority; no other key is
    /// one, as no construction runs a helper that could be given one.
    pub fn assume_authority(&mut self, kc: &mut KeyCred, id: i32) -> KResult<Serial> {
        if id < 0 {
            return Err(EINVAL);
        }
        if id == 0 {
            if let Some(a) = kc.auth.take() {
                self.put(a);
            }
            return Ok(0);
        }
        Err(ENOKEY)
    }

    /// KEYCTL_GET_SECURITY: no LSM labels a key, so the label is empty (the
    /// caller returns 1 and copies one NUL). Without View, ENOKEY: the
    /// authority fallback finds none.
    pub fn get_security(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32) -> KResult<()> {
        self.gc(subj.now);
        match self.lookup(subj, kc, id, PARTIAL, Need::Perm(perm::VIEW)) {
            Ok(_) => Ok(()),
            Err(EACCES) => Err(ENOKEY),
            Err(e) => Err(e),
        }
    }

    /// The first step of KEYCTL_SESSION_TO_PARENT: the caller's session
    /// keyring (installing its user-session one if it has none), with Link
    /// permission. The kernel checks the parent and hands it over.
    pub fn session_for_parent(&mut self, subj: &Subject, kc: &mut KeyCred) -> KResult<Serial> {
        self.gc(subj.now);
        self.lookup(subj, kc, spec::SESSION, 0, Need::Perm(perm::LINK))
            .map(|r| r.0)
    }

    /// The owner of keyring `s` (SESSION_TO_PARENT's ownership check).
    pub fn owner(&self, s: Serial) -> Option<u32> {
        self.key(s).map(|k| k.uid)
    }

    /// Give `parent` the session keyring `session` (SESSION_TO_PARENT's
    /// replacement).
    pub fn set_session(&mut self, parent: &mut KeyCred, session: Serial) {
        self.install(&mut parent.session, session);
    }

    /// KEYCTL_INVALIDATE.
    pub fn invalidate(&mut self, subj: &Subject, kc: &mut KeyCred, id: i32) -> KResult<()> {
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, 0, Need::Perm(perm::SEARCH))?;
        self.key_mut(s)?.flags |= flags::INVALIDATED;
        self.sweep(subj.now);
        Ok(())
    }

    /// KEYCTL_GET_PERSISTENT for kernel uid `uid` (the caller resolved -1
    /// to its real uid and refused an unmapped one), `ns_uid` its number in
    /// the caller's namespace.
    pub fn get_persistent(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        uid: u32,
        ns_uid: u32,
        destid: i32,
    ) -> KResult<Serial> {
        if uid != subj.uid && uid != subj.euid && !subj.setuid {
            return Err(EPERM);
        }
        self.gc(subj.now);
        let (dest, _) = self.lookup(subj, kc, destid, CREATE, Need::Perm(perm::WRITE))?;
        if !self.k(dest)?.is_keyring() {
            return Err(ENOTDIR);
        }
        let reg = match self.persistent_reg.get(&subj.ns) {
            Some(&r) => r,
            None => {
                let r = self.new_ring(
                    ".persistent_register",
                    0,
                    Some(0),
                    0x1f03_0000,
                    Quota::Not,
                    0,
                    subj.ns,
                )?;
                self.persistent_reg.insert(subj.ns, r);
                r
            }
        };
        let name = format!("_persistent.{ns_uid}");
        let ring = match self.direct(reg, KType::Keyring, &name) {
            Some(s) => s,
            None => {
                let s = self.new_ring(&name, uid, None, 0x1f03_0000, Quota::Not, 0, subj.ns)?;
                let r = self.link(reg, s);
                self.put(s);
                r?;
                s
            }
        };
        if !self.permits(self.k(ring)?, subj, true, perm::LINK) {
            return Err(EACCES);
        }
        self.link(dest, ring)?;
        let exp = subj.now + self.limits.persistent_expiry;
        self.key_mut(ring)?.expiry = Some(exp);
        self.schedule(exp);
        Ok(ring)
    }

    /// KEYCTL_RESTRICT_KEYRING. Only reject-all exists in the core: a
    /// named restriction is ENOENT for a known type, ENOKEY for another.
    pub fn restrict(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        args: RestrictArgs,
    ) -> KResult<()> {
        self.gc(subj.now);
        let (s, _) = self.lookup(subj, kc, id, 0, Need::Perm(perm::SETATTR))?;
        match (&args.ktype, &args.restriction) {
            (Some(_), None) | (None, Some(_)) => return Err(EINVAL),
            _ => {}
        }
        let ktype = args.ktype.transpose()?;
        let _restriction = args.restriction.transpose()?;
        let k = self.k(s)?;
        if !k.is_keyring() {
            return Err(ENOTDIR);
        }
        if let Some(t) = ktype {
            return Err(if KType::named(&t).is_some() {
                ENOENT
            } else {
                ENOKEY
            });
        }
        if k.restricted {
            return Err(EEXIST);
        }
        self.key_mut(s)?.restricted = true;
        Ok(())
    }

    /// KEYCTL_MOVE (`excl`: KEYCTL_MOVE_EXCL; the caller refused other
    /// flags).
    pub fn move_key(
        &mut self,
        subj: &Subject,
        kc: &mut KeyCred,
        id: i32,
        from: i32,
        to: i32,
        excl: bool,
    ) -> KResult<()> {
        self.gc(subj.now);
        let (key, _) = self.lookup(subj, kc, id, CREATE, Need::Perm(perm::LINK))?;
        let (from, _) = self.lookup(subj, kc, from, 0, Need::Perm(perm::WRITE))?;
        let (to, _) = self.lookup(subj, kc, to, CREATE, Need::Perm(perm::WRITE))?;
        if from == to {
            return Ok(());
        }
        let (f, t, k) = (self.k(from)?, self.k(to)?, self.k(key)?);
        if !f.is_keyring() || !t.is_keyring() {
            return Err(ENOTDIR);
        }
        let index = (k.ktype, k.desc.clone());
        let in_from = matches!(&f.payload, Payload::Ring(m) if m.get(&index) == Some(&key));
        if !in_from {
            return Err(ENOENT);
        }
        let _ = t;
        let replace = self.link_check(to, key, excl)?;
        self.link_commit(to, key, replace)?;
        self.unlink(from, key)
    }

    // ---- /proc -------------------------------------------------------

    /// /proc/keys as the opener `subj` sees it: the keys whose owner
    /// `visible` maps into its namespace and that it may View.
    pub fn proc_keys(
        &mut self,
        subj: &Subject,
        kc: &KeyCred,
        visible: &dyn Fn(u32) -> bool,
    ) -> Vec<ProcKey> {
        self.gc(subj.now);
        let mut out = Vec::new();
        for k in self.keys.values().filter(|k| k.usage > 0 && visible(k.uid)) {
            let possessed = k.perm & (perm::VIEW << 24) != 0 && self.possessed(subj, kc, k.serial);
            if !self.permits(k, subj, possessed, perm::VIEW) {
                continue;
            }
            let f = |bit: bool, c: u8| if bit { c } else { b'-' };
            let flags_ = [
                f(k.state != 0, b'I'),
                f(k.flags & flags::REVOKED != 0, b'R'),
                f(k.flags & flags::DEAD != 0, b'D'),
                f(k.flags & flags::IN_QUOTA != 0, b'Q'),
                f(k.flags & flags::USER_CONSTRUCT != 0, b'U'),
                f(k.state < 0, b'N'),
                f(k.flags & flags::INVALIDATED != 0, b'i'),
            ];
            let timeout = match k.expiry {
                None => String::from("perm"),
                Some(e) if subj.now >= e => String::from("expd"),
                Some(e) => {
                    let t = e - subj.now;
                    if t < 60 {
                        format!("{t}s")
                    } else if t < 3600 {
                        format!("{}m", t / 60)
                    } else if t < 86400 {
                        format!("{}h", t / 3600)
                    } else if t < 604_800 {
                        format!("{}d", t / 86400)
                    } else {
                        format!("{}w", t / 604_800)
                    }
                }
            };
            let describe = match k.ktype {
                KType::Keyring => {
                    let mut d = k.desc.clone();
                    if k.state > 0 {
                        let n = k.link_count();
                        if n == 0 {
                            d.push_str(": empty");
                        } else {
                            d.push_str(&format!(": {n}"));
                        }
                    }
                    d
                }
                _ => {
                    let mut d = k.desc.clone();
                    if k.state > 0 {
                        d.push_str(&format!(": {}", k.data_len()));
                    }
                    d
                }
            };
            out.push(ProcKey {
                serial: k.serial,
                flags: flags_,
                usage: k.usage,
                timeout,
                perm: k.perm,
                uid: k.uid,
                gid: k.gid,
                ktype: k.ktype.name(),
                describe,
            });
        }
        out
    }
}

/// KEYCTL_CAPABILITIES' two bytes: CAPABILITIES, PERSISTENT_KEYRINGS,
/// INVALIDATE, RESTRICT_KEYRING, MOVE; NS_KEYRING_NAME, NS_KEY_TAG.
pub const CAPABILITIES: [u8; 2] = [0xe3, 0x03];

#[cfg(test)]
mod tests {
    use super::*;

    fn rand() -> u32 {
        use core::sync::atomic::{AtomicU32, Ordering};
        static X: AtomicU32 = AtomicU32::new(0x1234_5678);
        let mut x = X.load(Ordering::Relaxed);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        X.store(x, Ordering::Relaxed);
        x
    }

    fn who(uid: u32) -> Subject {
        Subject {
            uid,
            euid: uid,
            suid: uid,
            fsuid: uid,
            gid: uid,
            egid: uid,
            sgid: uid,
            fsgid: uid,
            groups: Vec::new(),
            ns: 1,
            ns_uid: uid,
            ns_owner: 0,
            sys_admin: uid == 0,
            setuid: uid == 0,
            now: 1000,
        }
    }

    /// A thread keyring stays charged to the user it was made for when an
    /// fsuid change gives it a new owner, and its release refunds that
    /// user (key->user, not key->uid; PR #49 review).
    #[test]
    fn a_thread_keyring_is_refunded_to_the_user_charged() {
        let mut ks = KeyStore::new(rand);
        let mut s = who(1000);
        s.fsuid = 0;
        let mut kc = KeyCred::default();
        ks.add_key(&s, &mut kc, "user", Some("a"), b"x", spec::THREAD)
            .unwrap();
        let charged = |ks: &KeyStore, uid| ks.users().find(|(u, _)| *u == uid).map(|(_, k)| k);
        // The thread keyring is the real uid's (install_thread_keyring_to_cred),
        // the key in it the fsuid's.
        assert!(charged(&ks, 1000).is_some_and(|k| k.nkeys == 1));
        assert!(charged(&ks, 0).is_some_and(|k| k.nkeys == 1));
        // setresuid: the thread keyring's owner follows the fsuid.
        ks.fsid_changed(&s, &kc);
        let ring = kc.thread.unwrap();
        assert_eq!(ks.k(ring).unwrap().uid, 0);
        ks.release_cred(&kc);
        // Both entries gone, as Linux's key_user_put frees an unused one:
        // an empty entry left under one uid while another's was removed is
        // how the refund went astray (PR #49 review).
        assert!(charged(&ks, 1000).is_none());
        assert!(charged(&ks, 0).is_none());
    }

    #[test]
    fn serials_and_default_perms_are_linuxs() {
        let mut ks = KeyStore::new(rand);
        let s = who(1000);
        let mut kc = KeyCred::default();
        let k = ks
            .add_key(&s, &mut kc, "user", Some("a"), b"xyz", spec::PROCESS)
            .unwrap();
        assert!(k >= 3);
        let d = ks.describe(&s, &mut kc, k).unwrap();
        assert_eq!(
            (d.ktype, d.perm, d.desc.as_str()),
            ("user", 0x3f01_0000, "a")
        );
        let l = ks
            .add_key(&s, &mut kc, "logon", Some("svc:a"), b"x", spec::PROCESS)
            .unwrap();
        assert_eq!(ks.describe(&s, &mut kc, l).unwrap().perm, 0x3d01_0000);
        let r = ks
            .add_key(&s, &mut kc, "keyring", Some("r"), b"", spec::PROCESS)
            .unwrap();
        assert_eq!(ks.describe(&s, &mut kc, r).unwrap().perm, 0x3f01_0000);
        // Logon needs "service:"; its payload cannot be read.
        assert_eq!(
            ks.add_key(&s, &mut kc, "logon", Some("x"), b"x", spec::PROCESS),
            Err(EINVAL)
        );
        assert_eq!(ks.read(&s, &mut kc, l), Err(EOPNOTSUPP));
        assert_eq!(
            ks.add_key(&s, &mut kc, "nope", Some("x"), b"x", spec::PROCESS),
            Err(ENODEV)
        );
        assert_eq!(
            ks.add_key(&s, &mut kc, "keyring", Some(".x"), b"", spec::PROCESS),
            Err(EPERM)
        );
        assert_eq!(
            ks.add_key(&s, &mut kc, "keyring", Some("r2"), b"1", spec::PROCESS),
            Err(EINVAL)
        );
        assert_eq!(
            ks.add_key(&s, &mut kc, "user", Some("big"), &[0; 32768], spec::PROCESS),
            Err(EINVAL)
        );
        // Past an ordinary user's 20000 bytes: EDQUOT; root may (add_key01
        // runs as root).
        assert_eq!(
            ks.add_key(&s, &mut kc, "user", Some("big"), &[0; 32767], spec::PROCESS),
            Err(EDQUOT)
        );
        let root = who(0);
        let mut rkc = KeyCred::default();
        assert!(ks
            .add_key(
                &root,
                &mut rkc,
                "user",
                Some("big"),
                &[0; 32767],
                spec::PROCESS
            )
            .is_ok());
    }

    #[test]
    fn add_key_updates_and_read_returns_the_payload() {
        let mut ks = KeyStore::new(rand);
        let s = who(1000);
        let mut kc = KeyCred::default();
        let k = ks
            .add_key(&s, &mut kc, "user", Some("a"), b"one", spec::THREAD)
            .unwrap();
        ks.set_timeout(&s, &mut kc, k, 50).unwrap();
        let k2 = ks
            .add_key(&s, &mut kc, "user", Some("a"), b"two!", spec::THREAD)
            .unwrap();
        assert_eq!(k, k2);
        assert_eq!(ks.read(&s, &mut kc, k).unwrap().data, b"two!");
        // The update cleared the timeout.
        assert_eq!(ks.key(k).unwrap().expiry, None);
        let ring = ks.read(&s, &mut kc, spec::THREAD).unwrap();
        assert!(ring.keyring);
        assert_eq!(ring.data, k.to_ne_bytes());
    }

    #[test]
    fn revoke_expiry_and_gc() {
        let mut ks = KeyStore::new(rand);
        let mut s = who(1000);
        let mut kc = KeyCred::default();
        let k = ks
            .add_key(&s, &mut kc, "user", Some("a"), b"x", spec::PROCESS)
            .unwrap();
        ks.revoke(&s, &mut kc, k).unwrap();
        assert_eq!(ks.read(&s, &mut kc, k), Err(EKEYREVOKED));
        assert_eq!(ks.revoke(&s, &mut kc, k), Err(EKEYREVOKED));
        let e = ks
            .add_key(&s, &mut kc, "user", Some("b"), b"x", spec::PROCESS)
            .unwrap();
        ks.set_timeout(&s, &mut kc, e, 10).unwrap();
        s.now += 10;
        assert_eq!(ks.read(&s, &mut kc, e), Err(EKEYEXPIRED));
        // The expired one is unlinked, and gone, at expiry + gc_delay; the
        // revoked one is not dead and stays linked, as on Linux (key_is_dead
        // does not look at revocation).
        s.now += 300;
        assert_eq!(ks.read(&s, &mut kc, e), Err(ENOKEY));
        assert_eq!(ks.read(&s, &mut kc, k), Err(EKEYREVOKED));
        assert_eq!(
            ks.read(&s, &mut kc, spec::PROCESS).unwrap().data,
            k.to_ne_bytes()
        );
    }

    #[test]
    fn permission_bytes_and_possession() {
        let mut ks = KeyStore::new(rand);
        let owner = who(1000);
        let other = who(2000);
        let mut kc = KeyCred::default();
        let k = ks
            .add_key(&owner, &mut kc, "user", Some("a"), b"x", spec::PROCESS)
            .unwrap();
        let mut kc2 = KeyCred::default();
        // Not possessed by another user, and other has no View.
        assert_eq!(ks.describe(&other, &mut kc2, k), Err(EACCES));
        // The owner, without possession, has View only (0x01 in the user byte).
        let mut none = KeyCred::default();
        assert!(ks.describe(&owner, &mut none, k).is_ok());
        assert_eq!(ks.read(&owner, &mut none, k), Err(EACCES));
        ks.setperm(&owner, &mut kc, k, 0x3f01_0002).unwrap();
        assert_eq!(ks.read(&other, &mut kc2, k).unwrap().data, b"x");
        assert_eq!(ks.setperm(&owner, &mut kc, k, 0x4000_0000), Err(EINVAL));
        assert_eq!(ks.setperm(&other, &mut kc2, k, 0x3f3f_3f3f), Err(EACCES));
    }

    #[test]
    fn links_cycles_and_restriction() {
        let mut ks = KeyStore::new(rand);
        let s = who(1000);
        let mut kc = KeyCred::default();
        let a = ks
            .add_key(&s, &mut kc, "keyring", Some("a"), b"", spec::PROCESS)
            .unwrap();
        let b = ks
            .add_key(&s, &mut kc, "keyring", Some("b"), b"", a)
            .unwrap();
        assert_eq!(ks.link_key(&s, &mut kc, a, b), Err(EDEADLK));
        assert_eq!(ks.link_key(&s, &mut kc, a, a), Err(EDEADLK));
        let u = ks
            .add_key(&s, &mut kc, "user", Some("u"), b"x", spec::PROCESS)
            .unwrap();
        assert_eq!(ks.link_key(&s, &mut kc, b, u), Err(ENOTDIR));
        ks.link_key(&s, &mut kc, u, b).unwrap();
        // Found below a, through b.
        assert_eq!(ks.search_key(&s, &mut kc, a, "user", "u", 0), Ok(u));
        assert_eq!(
            ks.search_key(&s, &mut kc, a, "user", "none", 0),
            Err(ENOKEY)
        );
        assert_eq!(ks.unlink_key(&s, &mut kc, u, a), Err(ENOENT));
        ks.unlink_key(&s, &mut kc, u, b).unwrap();
        let none = RestrictArgs {
            ktype: None,
            restriction: None,
        };
        ks.restrict(&s, &mut kc, b, none).unwrap();
        assert_eq!(ks.link_key(&s, &mut kc, u, b), Err(EPERM));
        assert_eq!(
            ks.add_key(&s, &mut kc, "user", Some("v"), b"x", b),
            Err(EPERM)
        );
        let again = RestrictArgs {
            ktype: None,
            restriction: None,
        };
        assert_eq!(ks.restrict(&s, &mut kc, b, again), Err(EEXIST));
        let named = RestrictArgs {
            ktype: Some(Ok(String::from("user"))),
            restriction: Some(Ok(String::from("x"))),
        };
        assert_eq!(ks.restrict(&s, &mut kc, a, named), Err(ENOENT));
        // Nested seven deep: the seventh link is ELOOP.
        let mut prev = ks
            .add_key(&s, &mut kc, "keyring", Some("n0"), b"", spec::PROCESS)
            .unwrap();
        let top = prev;
        let mut last = Ok(());
        // Each keyring is linked into the next, so the one linked has ever
        // more keyrings below it; the eighth would have seven.
        for i in 1..=8 {
            let n = ks
                .add_key(
                    &s,
                    &mut kc,
                    "keyring",
                    Some(&format!("n{i}")),
                    b"",
                    spec::PROCESS,
                )
                .unwrap();
            last = ks.link_key(&s, &mut kc, prev, n);
            if last.is_err() {
                break;
            }
            prev = n;
        }
        assert_eq!(last, Err(ELOOP));
        let _ = top;
    }

    #[test]
    fn quotas_are_exact_and_links_cost_four_bytes() {
        let mut ks = KeyStore::new(rand);
        ks.limits.maxkeys = 5;
        let s = who(1000);
        let mut kc = KeyCred::default();
        // The session keyring and the user keyrings count too.
        let ring = ks.get_keyring_id(&s, &mut kc, spec::SESSION, true).unwrap();
        let used = ks.users().find(|(u, _)| *u == 1000).unwrap().1;
        let room = 5 - used.qnkeys;
        for i in 0..room {
            ks.add_key(&s, &mut kc, "user", Some(&format!("k{i}")), b"x", ring)
                .unwrap();
        }
        assert_eq!(
            ks.add_key(&s, &mut kc, "user", Some("over"), b"x", ring),
            Err(EDQUOT)
        );
        let before = ks.users().find(|(u, _)| *u == 1000).unwrap().1.qnbytes;
        let k = ks.read(&s, &mut kc, ring).unwrap().data.len() as u32 / 4;
        assert!(k == room);
        assert!(before >= room * (LINK_BYTES + 3 + 1));
    }

    #[test]
    fn request_key_without_a_helper() {
        let mut ks = KeyStore::new(rand);
        let mut s = who(1000);
        let mut kc = KeyCred::default();
        assert_eq!(
            ks.request_key(&s, &mut kc, "user", "d", None, 0),
            Err(ENOKEY)
        );
        assert_eq!(
            ks.request_key(&s, &mut kc, "dead", "d", Some(b"x"), 0),
            Err(ENOKEY)
        );
        assert_eq!(
            ks.request_key(&s, &mut kc, "keyring", "d", Some(b"x"), spec::PROCESS),
            Err(EPERM)
        );
        assert_eq!(
            ks.request_key(&s, &mut kc, "user", "d", Some(b"x"), spec::PROCESS),
            Err(ENOENT)
        );
        // A negative key is left for 60 s.
        let ring = ks.read(&s, &mut kc, spec::PROCESS).unwrap().data;
        assert_eq!(ring.len(), 4);
        let neg = i32::from_ne_bytes([ring[0], ring[1], ring[2], ring[3]]);
        assert_eq!(ks.read(&s, &mut kc, neg), Err(ENOKEY));
        assert_eq!(
            ks.request_key(&s, &mut kc, "user", "d", Some(b"x"), 0),
            Err(ENOKEY)
        );
        s.now += 61;
        // Expired: skipped, and constructed again.
        assert_eq!(
            ks.request_key(&s, &mut kc, "user", "d", Some(b"x"), spec::PROCESS),
            Err(ENOENT)
        );
        // A real key is found.
        let k = ks
            .add_key(&s, &mut kc, "user", Some("e"), b"x", spec::PROCESS)
            .unwrap();
        assert_eq!(ks.request_key(&s, &mut kc, "user", "e", None, 0), Ok(k));
    }

    #[test]
    fn special_keyrings() {
        let mut ks = KeyStore::new(rand);
        let s = who(1000);
        let mut kc = KeyCred::default();
        assert_eq!(
            ks.get_keyring_id(&s, &mut kc, spec::THREAD, false),
            Err(ENOKEY)
        );
        let t = ks.get_keyring_id(&s, &mut kc, spec::THREAD, true).unwrap();
        assert_eq!(ks.get_keyring_id(&s, &mut kc, spec::THREAD, false), Ok(t));
        assert_eq!(
            ks.get_keyring_id(&s, &mut kc, spec::GROUP, false),
            Err(EINVAL)
        );
        assert_eq!(ks.get_keyring_id(&s, &mut kc, 0, false), Err(EINVAL));
        let user = ks.get_keyring_id(&s, &mut kc, spec::USER, false).unwrap();
        let ses = ks
            .get_keyring_id(&s, &mut kc, spec::USER_SESSION, false)
            .unwrap();
        assert_eq!(ks.describe(&s, &mut kc, user).unwrap().desc, "_uid.1000");
        assert_eq!(ks.describe(&s, &mut kc, ses).unwrap().desc, "_uid_ses.1000");
        // The session without create is the user-session keyring; with
        // create, a new anonymous one.
        assert_eq!(
            ks.get_keyring_id(&s, &mut kc, spec::SESSION, false),
            Ok(ses)
        );
        let anon = ks.get_keyring_id(&s, &mut kc, spec::SESSION, true).unwrap();
        assert_ne!(anon, ses);
        assert_eq!(ks.describe(&s, &mut kc, anon).unwrap().desc, "_ses");
        // Joining by name: a named session keyring (0x3f130000) does not
        // let its owner Search it without possession, so joining the name
        // again makes another, as on Linux; one that may be searched is
        // found, and is already the session (0).
        let first = ks.join_session(&s, &mut kc, Some("mine")).unwrap();
        let j = ks.join_session(&s, &mut kc, Some("mine")).unwrap();
        assert!(j != first && j != 0);
        ks.setperm(&s, &mut kc, j, 0x3f1b_0000).unwrap();
        assert_eq!(ks.join_session(&s, &mut kc, Some("mine")), Ok(0));
        assert_eq!(ks.join_session(&s, &mut kc, Some(".x")), Err(EPERM));
        assert_eq!(kc.session, Some(j));
        // fork: no thread or process keyring; exec drops them.
        let child = ks.fork_cred(&kc);
        assert_eq!(
            (child.thread, child.process, child.session),
            (None, None, Some(j))
        );
        ks.release_cred(&child);
        ks.exec_cred(&mut kc);
        assert_eq!(kc.thread, None);
        // The thread keyring went with its last holder.
        assert!(ks.key(t).is_none());
    }

    #[test]
    fn move_as_linux() {
        let mut ks = KeyStore::new(rand);
        let s = who(1000);
        let mut kc = KeyCred::default();
        let a = ks
            .add_key(&s, &mut kc, "keyring", Some("a"), b"", spec::PROCESS)
            .unwrap();
        let b = ks
            .add_key(&s, &mut kc, "keyring", Some("b"), b"", spec::PROCESS)
            .unwrap();
        let k = ks.add_key(&s, &mut kc, "user", Some("k"), b"1", a).unwrap();
        assert_eq!(ks.move_key(&s, &mut kc, k, a, a, false), Ok(()));
        ks.move_key(&s, &mut kc, k, a, b, false).unwrap();
        assert_eq!(ks.search_key(&s, &mut kc, a, "user", "k", 0), Err(ENOKEY));
        assert_eq!(ks.search_key(&s, &mut kc, b, "user", "k", 0), Ok(k));
        assert_eq!(ks.move_key(&s, &mut kc, k, a, b, false), Err(ENOENT));
        let k2 = ks.add_key(&s, &mut kc, "user", Some("k"), b"2", a).unwrap();
        assert_eq!(ks.move_key(&s, &mut kc, k2, a, b, true), Err(EEXIST));
        ks.move_key(&s, &mut kc, k2, a, b, false).unwrap();
        assert_eq!(ks.search_key(&s, &mut kc, b, "user", "k", 0), Ok(k2));
        assert_eq!(
            ks.move_key(&s, &mut kc, b, spec::PROCESS, b, false),
            Err(EDEADLK)
        );
    }

    #[test]
    fn proc_keys_lines() {
        let mut ks = KeyStore::new(rand);
        let s = who(1000);
        let mut kc = KeyCred::default();
        let k = ks
            .add_key(&s, &mut kc, "user", Some("foo"), b"12345", spec::PROCESS)
            .unwrap();
        let lines = ks.proc_keys(&s, &kc, &|_| true);
        let l = lines.iter().find(|l| l.serial == k).unwrap();
        assert_eq!(&l.flags, b"I--Q---");
        assert_eq!(
            (l.timeout.as_str(), l.perm, l.describe.as_str()),
            ("perm", 0x3f01_0000, "foo: 5")
        );
        let other = who(2000);
        let kc2 = KeyCred::default();
        assert!(ks
            .proc_keys(&other, &kc2, &|_| true)
            .iter()
            .all(|l| l.serial != k));
    }
}

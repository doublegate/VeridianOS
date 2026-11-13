//! System V IPC: shared memory, semaphores and message queues (v0.28 item
//! 5), each IPC namespace with its own (ADR 0022 H1), with Linux's
//! semantics (`ipc/util.c`, `shm.c`, `sem.c`, `msg.c`).
//!
//! The objects of one kind live in an [`Ids`] table, as Linux's `ipc_ids`:
//! an identifier is a slot index and a sequence number (`seq << 15 | idx`),
//! so a slot used again gives a new identifier and the old one is EINVAL.
//! Access is checked as `ipcperms` checks it (owner, group, other bits,
//! CAP_IPC_OWNER over the namespace's user namespace), and, as every Linux
//! check here (ADR 0024), also needs the caller's rights over the IPC
//! namespace, by intent (ADR 0027): READ to look, WRITE as well to change
//! an object's contents (send, receive, alter a semaphore, attach
//! writable), CREATE to make an object, MODIFY to control one (IPC_SET,
//! IPC_RMID, SHM_LOCK) or write the namespace's limits.

pub mod calls;
pub mod msg;
pub mod sem;
pub mod shm;

use alloc::{collections::BTreeMap, sync::Arc};
use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use spin::Mutex;

use super::SyscallError;
use crate::process::{creds::Caller, linux_caps, ns::PlainNs, userns::UserNs};

/// The key that always makes a new object.
pub const IPC_PRIVATE: i32 = 0;
/// get: create the object if the key has none.
pub const IPC_CREAT: i32 = 0o1000;
/// get with IPC_CREAT: fail (EEXIST) if the key has one.
pub const IPC_EXCL: i32 = 0o2000;
/// semop, msgsnd, msgrcv: fail instead of waiting.
pub const IPC_NOWAIT: i32 = 0o4000;

pub const IPC_RMID: i32 = 0;
pub const IPC_SET: i32 = 1;
pub const IPC_STAT: i32 = 2;
pub const IPC_INFO: i32 = 3;
/// The flag a 64-bit caller's library adds to every ctl command (the
/// 64-bit layouts are the only ones here).
pub const IPC_64: i32 = 0x100;

/// The mode bits a caller sets (S_IRWXUGO).
pub const MODE_MASK: u32 = 0o777;

/// Linux's IPCMNI_SHIFT: the slot index takes the low 15 bits.
const SEQ_SHIFT: u32 = 15;
/// How many slots one table has (IPCMNI).
pub const IPC_MNI: u32 = 1 << SEQ_SHIFT;
/// Slots are handed out in turn over at least this many (ipc_min_cycle
/// without IPCMNI_EXTEND: RADIX_TREE_MAP_SIZE).
const MIN_CYCLE: u32 = 64;
/// The sequence number wraps here (ipcid_seq_max: INT_MAX >> 15).
const SEQ_MAX: u32 = (i32::MAX as u32) >> SEQ_SHIFT;

/// An object's owner, creator and mode (Linux's `kern_ipc_perm`). The ids
/// are kernel ids; callers see them through their user namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perm {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    /// The access bits, and SHM_DEST and SHM_LOCKED for a segment.
    pub mode: u32,
    pub seq: u32,
    pub id: i32,
}

impl Perm {
    /// A new object's: owned and made by the caller, with the access bits
    /// of `flags`. The table fills in `seq` and `id`.
    pub fn new(key: i32, flags: i32, euid: u32, egid: u32) -> Self {
        Self {
            key,
            uid: euid,
            gid: egid,
            cuid: euid,
            cgid: egid,
            mode: flags as u32 & MODE_MASK,
            seq: 0,
            id: 0,
        }
    }
}

/// Who is asking: the credentials `ipcperms` reads, and what they may do
/// over the namespace. Taken once per call.
pub struct Asker {
    pub euid: u32,
    pub egid: u32,
    groups: alloc::vec::Vec<u32>,
    /// CAP_IPC_OWNER over the namespace's user namespace.
    pub ipc_owner: bool,
    /// CAP_SYS_ADMIN over it.
    pub sys_admin: bool,
    /// CAP_IPC_LOCK over it.
    pub ipc_lock: bool,
    /// CAP_SYS_RESOURCE over it.
    pub sys_resource: bool,
    /// The object capability's rights over the IPC namespace (ADR 0024,
    /// ADR 0027): READ, WRITE, CREATE and MODIFY.
    pub may_read: bool,
    pub may_write: bool,
    pub may_create: bool,
    pub may_modify: bool,
    /// The caller's user namespace, for the ids it sees and sets.
    pub userns: Arc<UserNs>,
}

impl Asker {
    /// The caller, for objects of the namespace owned by `owner`.
    pub fn from_caller(caller: &Caller, owner: &UserNs, rights: NsRights) -> Self {
        Self {
            euid: caller.creds.euid,
            egid: caller.creds.egid,
            groups: caller.creds.groups().to_vec(),
            ipc_owner: caller.ns_capable(owner, linux_caps::CAP_IPC_OWNER),
            sys_admin: caller.ns_capable(owner, linux_caps::CAP_SYS_ADMIN),
            ipc_lock: caller.ns_capable(owner, linux_caps::CAP_IPC_LOCK),
            sys_resource: caller.ns_capable(owner, linux_caps::CAP_SYS_RESOURCE),
            may_read: rights.read,
            may_write: rights.write,
            may_create: rights.create,
            may_modify: rights.modify,
            userns: caller.userns.clone(),
        }
    }

    /// The current thread, for the IPC namespace `ns`.
    pub fn current(ns: &Arc<PlainNs>) -> Self {
        let caller = Caller::current();
        Self::from_caller(&caller, &ns.owner, namespace_rights(ns))
    }

    fn in_group(&self, gid: u32) -> bool {
        self.egid == gid || self.groups.contains(&gid)
    }

    /// Linux's ipcperms: whether the bits of `flag` (rwx in any of the
    /// three positions) are granted to the caller by `perm`'s mode, or
    /// CAP_IPC_OWNER waives them. The namespace's READ is needed in any
    /// case, and WRITE for a write.
    pub fn permits(&self, perm: &Perm, flag: u32) -> bool {
        let writes = flag & 0o222 != 0;
        if !self.may_read || (writes && !self.may_write) {
            return false;
        }
        let requested = ((flag >> 6) | (flag >> 3) | flag) & 0o7;
        let granted = if self.euid == perm.cuid || self.euid == perm.uid {
            perm.mode >> 6
        } else if self.in_group(perm.cgid) || self.in_group(perm.gid) {
            perm.mode >> 3
        } else {
            perm.mode
        };
        requested & !granted & 0o7 == 0 || self.ipc_owner
    }

    /// IPC_SET and IPC_RMID (ipcctl_obtain_check): the owner, the creator
    /// or CAP_SYS_ADMIN over the namespace, which also needs MODIFY on it.
    pub fn may_control(&self, perm: &Perm) -> bool {
        self.may_modify && (self.euid == perm.cuid || self.euid == perm.uid || self.sys_admin)
    }

    /// Making a new object: CREATE on the namespace (EACCES without).
    pub fn may_make(&self) -> Result<(), SyscallError> {
        if self.may_read && self.may_create {
            Ok(())
        } else {
            Err(SyscallError::PermissionDenied)
        }
    }

    /// Changing an object's contents beyond what the Linux bits say (a
    /// message received, which is taken out): WRITE (EACCES without).
    pub fn may_change(&self) -> Result<(), SyscallError> {
        if self.may_read && self.may_write {
            Ok(())
        } else {
            Err(SyscallError::PermissionDenied)
        }
    }

    /// The ids of `perm` as the caller's user namespace shows them.
    pub fn shown(&self, perm: &Perm) -> Perm {
        Perm {
            uid: self.userns.uid_munged(perm.uid),
            gid: self.userns.gid_munged(perm.gid),
            cuid: self.userns.uid_munged(perm.cuid),
            cgid: self.userns.gid_munged(perm.cgid),
            ..*perm
        }
    }

    /// IPC_SET's new owner and mode (ipc_update_perm): ids from the
    /// caller's user namespace (an unmapped one is EINVAL), the access bits
    /// of `mode`.
    pub fn update(
        &self,
        perm: &mut Perm,
        uid: u32,
        gid: u32,
        mode: u32,
    ) -> Result<(), SyscallError> {
        let uid = self
            .userns
            .uid_down(uid)
            .ok_or(SyscallError::InvalidArgument)?;
        let gid = self
            .userns
            .gid_down(gid)
            .ok_or(SyscallError::InvalidArgument)?;
        perm.uid = uid;
        perm.gid = gid;
        perm.mode = (perm.mode & !MODE_MASK) | (mode & MODE_MASK);
        Ok(())
    }
}

/// The caller's rights over an IPC namespace (ADR 0024, ADR 0027).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NsRights {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub modify: bool,
}

impl NsRights {
    pub const ALL: Self = Self {
        read: true,
        write: true,
        create: true,
        modify: true,
    };
}

/// The caller's rights over IPC namespace `ns`, each right held (with
/// READ) on its own. The kernel's own context holds them all.
pub fn namespace_rights(ns: &Arc<PlainNs>) -> NsRights {
    use crate::cap::Rights;
    let r = crate::process::ns::NsRef::Plain(ns.clone());
    let held = |rights| crate::cap::authority::current_holds_namespace(&r, Rights::READ | rights);
    NsRights {
        read: held(Rights::READ),
        write: held(Rights::WRITE),
        create: held(Rights::CREATE),
        modify: held(Rights::MODIFY),
    }
}

/// What an [`Ids`] table holds.
pub trait IpcObject {
    fn perm(&self) -> &Perm;
    fn perm_mut(&mut self) -> &mut Perm;
}

/// One kind's objects in one namespace (Linux's `ipc_ids`).
pub struct Ids<T> {
    slots: BTreeMap<u32, T>,
    keys: BTreeMap<i32, u32>,
    seq: u32,
    /// The index handed out last (`last_idx`), for the sequence number.
    last_idx: i64,
    /// Where the next search for a free slot starts (the idr's cursor).
    next: u32,
}

impl<T: IpcObject> Default for Ids<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: IpcObject> Ids<T> {
    pub const fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
            keys: BTreeMap::new(),
            seq: 0,
            last_idx: -1,
            next: 0,
        }
    }

    /// How many objects there are (`in_use`).
    pub fn in_use(&self) -> usize {
        self.slots.len()
    }

    /// The highest index in use, 0 with none (what IPC_INFO and the
    /// *_INFO commands return).
    pub fn max_idx(&self) -> i32 {
        self.slots.keys().next_back().map_or(0, |&i| i as i32)
    }

    /// The object `key` names (ipc_findkey).
    pub fn find_key(&self, key: i32) -> Option<i32> {
        let idx = *self.keys.get(&key)?;
        self.slots.get(&idx).map(|o| o.perm().id)
    }

    /// Add `obj` (ipc_addid): ENOSPC with `limit` objects already (at most
    /// IPCMNI). Slots go round over max(in_use * 3 / 2, 64) of them, and a
    /// slot at or below the last one handed out takes a new sequence
    /// number, as idr_alloc_cyclic and ipc_idr_alloc do. Returns the new
    /// identifier.
    pub fn add(&mut self, mut obj: T, limit: usize) -> Result<i32, SyscallError> {
        let limit = limit.min(IPC_MNI as usize);
        if self.slots.len() >= limit {
            return Err(SyscallError::NoSpace);
        }
        let span = ((self.slots.len() * 3 / 2) as u32).clamp(MIN_CYCLE, IPC_MNI);
        let free = |from: u32, to: u32| (from..to).find(|i| !self.slots.contains_key(i));
        let start = if self.next >= span { 0 } else { self.next };
        let idx = free(start, span)
            .or_else(|| free(0, start))
            .ok_or(SyscallError::NoSpace)?;
        if i64::from(idx) <= self.last_idx {
            self.seq += 1;
            if self.seq >= SEQ_MAX {
                self.seq = 0;
            }
        }
        self.last_idx = i64::from(idx);
        self.next = idx + 1;
        let id = ((self.seq << SEQ_SHIFT) + idx) as i32;
        let perm = obj.perm_mut();
        perm.seq = self.seq;
        perm.id = id;
        if perm.key != IPC_PRIVATE {
            self.keys.insert(perm.key, idx);
        }
        self.slots.insert(idx, obj);
        Ok(id)
    }

    fn slot_of(id: i32) -> Option<u32> {
        (id >= 0).then_some(id as u32 & (IPC_MNI - 1))
    }

    /// The object `id` names (ipc_obtain_object_check): a negative id, an
    /// empty slot or a slot used again since is EINVAL.
    pub fn get(&self, id: i32) -> Result<&T, SyscallError> {
        let idx = Self::slot_of(id).ok_or(SyscallError::InvalidArgument)?;
        self.slots
            .get(&idx)
            .filter(|o| o.perm().id == id)
            .ok_or(SyscallError::InvalidArgument)
    }

    pub fn get_mut(&mut self, id: i32) -> Result<&mut T, SyscallError> {
        let idx = Self::slot_of(id).ok_or(SyscallError::InvalidArgument)?;
        self.slots
            .get_mut(&idx)
            .filter(|o| o.perm().id == id)
            .ok_or(SyscallError::InvalidArgument)
    }

    /// The object in slot `idx`, whatever its sequence number (SHM_STAT
    /// and the other *_STAT commands, which take an index).
    pub fn by_index(&self, idx: i32) -> Result<&T, SyscallError> {
        if !(0..IPC_MNI as i32).contains(&idx) {
            return Err(SyscallError::InvalidArgument);
        }
        self.slots
            .get(&(idx as u32))
            .ok_or(SyscallError::InvalidArgument)
    }

    /// Take `id` out of the table (ipc_rmid).
    pub fn remove(&mut self, id: i32) -> Option<T> {
        let idx = Self::slot_of(id)?;
        if self.slots.get(&idx)?.perm().id != id {
            return None;
        }
        let obj = self.slots.remove(&idx)?;
        let key = obj.perm().key;
        if key != IPC_PRIVATE && self.keys.get(&key) == Some(&idx) {
            self.keys.remove(&key);
        }
        Some(obj)
    }

    /// `id` keeps its slot but no longer answers to its key (a segment
    /// removed while attached: ipc_set_key_private).
    pub fn make_private(&mut self, id: i32) {
        let Ok(obj) = self.get_mut(id) else {
            return;
        };
        let key = obj.perm().key;
        obj.perm_mut().key = IPC_PRIVATE;
        let idx = id as u32 & (IPC_MNI - 1);
        if key != IPC_PRIVATE && self.keys.get(&key) == Some(&idx) {
            self.keys.remove(&key);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.slots.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.values_mut()
    }
}

/// What a get call asks for (`ipc_params`) and the kind's own checks.
pub enum GetOutcome {
    /// Make a new object.
    Create,
    /// The existing object's id, after the kind's checks.
    Existing(i32),
}

/// Linux's ipcget for `key` and `flags`, up to the point where the kind
/// takes over: IPC_PRIVATE or a key with nothing behind it and IPC_CREAT
/// make a new object; a key with nothing behind it otherwise is ENOENT;
/// IPC_CREAT | IPC_EXCL on a key in use is EEXIST; otherwise `more_checks`
/// (the kind's, before the access check, as Linux orders them) and the
/// access bits of `flags` (EACCES).
pub fn ipcget<T: IpcObject>(
    ids: &Ids<T>,
    asker: &Asker,
    key: i32,
    flags: i32,
    more_checks: impl FnOnce(&T) -> Result<(), SyscallError>,
) -> Result<GetOutcome, SyscallError> {
    if key == IPC_PRIVATE {
        return Ok(GetOutcome::Create);
    }
    let Some(id) = ids.find_key(key) else {
        return if flags & IPC_CREAT != 0 {
            Ok(GetOutcome::Create)
        } else {
            Err(SyscallError::ResourceNotFound)
        };
    };
    if flags & IPC_CREAT != 0 && flags & IPC_EXCL != 0 {
        return Err(SyscallError::FileExists);
    }
    let obj = ids.get(id)?;
    more_checks(obj)?;
    if !asker.permits(obj.perm(), flags as u32 & MODE_MASK) {
        return Err(SyscallError::PermissionDenied);
    }
    Ok(GetOutcome::Existing(id))
}

/// The limits `/proc/sys/kernel` shows for one namespace (Linux's
/// `ipc_namespace` fields; the defaults are Linux's).
pub struct Limits {
    pub shmmax: AtomicU64,
    pub shmall: AtomicU64,
    pub shmmni: AtomicI32,
    pub shm_rmid_forced: AtomicI32,
    /// SEMMSL, SEMMNS, SEMOPM, SEMMNI (`kernel.sem`).
    pub sem: [AtomicI32; 4],
    pub msgmax: AtomicI32,
    pub msgmnb: AtomicI32,
    pub msgmni: AtomicI32,
}

impl Limits {
    pub fn new() -> Self {
        Self {
            shmmax: AtomicU64::new(shm::SHMMAX),
            shmall: AtomicU64::new(shm::SHMALL),
            shmmni: AtomicI32::new(shm::SHMMNI),
            shm_rmid_forced: AtomicI32::new(0),
            sem: [
                AtomicI32::new(sem::SEMMSL),
                AtomicI32::new(sem::SEMMNS),
                AtomicI32::new(sem::SEMOPM),
                AtomicI32::new(sem::SEMMNI),
            ],
            msgmax: AtomicI32::new(msg::MSGMAX),
            msgmnb: AtomicI32::new(msg::MSGMNB),
            msgmni: AtomicI32::new(msg::MSGMNI),
        }
    }

    pub fn sem_limits(&self) -> [i32; 4] {
        [0, 1, 2, 3].map(|i| self.sem[i].load(Ordering::Relaxed))
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new()
    }
}

/// One IPC namespace's System V objects and limits.
pub struct SysvSpace {
    pub limits: Limits,
    pub shm: Mutex<shm::ShmTable>,
    pub sem: Mutex<Ids<sem::SemArray>>,
    pub msg: Mutex<Ids<msg::Queue>>,
    /// Woken on every change to a semaphore value or array.
    pub sem_waiters: crate::sched::dispatch::WaitQueue,
    /// Woken on every message sent, received or queue changed.
    pub msg_waiters: crate::sched::dispatch::WaitQueue,
}

impl SysvSpace {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            limits: Limits::new(),
            shm: Mutex::new(shm::ShmTable::new()),
            sem: Mutex::new(Ids::new()),
            msg: Mutex::new(Ids::new()),
            sem_waiters: crate::sched::dispatch::WaitQueue::new(),
            msg_waiters: crate::sched::dispatch::WaitQueue::new(),
        })
    }
}

/// The current thread's IPC namespace (the initial one in kernel
/// context), and its System V objects.
pub fn current_ns() -> Arc<PlainNs> {
    crate::process::current_thread()
        .map(|t| t.namespaces().ipc.clone())
        .unwrap_or_else(|| crate::process::ns::initial().ipc.clone())
}

/// The System V objects of IPC namespace `ns`.
pub fn space_of(ns: &PlainNs) -> &Arc<SysvSpace> {
    ns.sysv
        .as_ref()
        .expect("an IPC namespace has its System V space")
}

/// The wall clock in seconds, for the *_time fields.
pub fn now_secs() -> i64 {
    crate::timer::realtime::now_ns().div_euclid(1_000_000_000)
}

/// The caller's process id as the caller's PID namespace shows the process
/// `pid` (Linux's pid_vnr of a stored pid; 0 when it cannot see it).
pub fn pid_shown(pid: u64) -> i32 {
    if pid == 0 {
        return 0;
    }
    crate::process::current_process().map_or(pid as i32, |p| {
        crate::process::pidns::nr_in(pid, &p.pid_ns()) as i32
    })
}

/// The caller's process id, as stored (global).
pub fn current_pid() -> u64 {
    crate::process::current_process().map_or(0, |p| p.pid.0)
}

/// The `/proc/sys/kernel` files of the IPC namespace (`ipc_sysctl.c`).
pub const SYSCTLS: [&str; 8] = [
    "msgmax",
    "msgmnb",
    "msgmni",
    "sem",
    "shm_rmid_forced",
    "shmall",
    "shmmax",
    "shmmni",
];

/// The text of sysctl `name` (one of [`SYSCTLS`]) in the reader's IPC
/// namespace.
pub fn sysctl_text(name: &str) -> alloc::string::String {
    let ns = current_ns();
    let l = &space_of(&ns).limits;
    let r = Ordering::Relaxed;
    match name {
        "msgmax" => alloc::format!("{}\n", l.msgmax.load(r)),
        "msgmnb" => alloc::format!("{}\n", l.msgmnb.load(r)),
        "msgmni" => alloc::format!("{}\n", l.msgmni.load(r)),
        "sem" => {
            let [a, b, c, d] = l.sem_limits();
            alloc::format!("{}\t{}\t{}\t{}\n", a, b, c, d)
        }
        "shm_rmid_forced" => alloc::format!("{}\n", l.shm_rmid_forced.load(r)),
        "shmall" => alloc::format!("{}\n", l.shmall.load(r)),
        "shmmax" => alloc::format!("{}\n", l.shmmax.load(r)),
        "shmmni" => alloc::format!("{}\n", l.shmmni.load(r)),
        _ => alloc::string::String::new(),
    }
}

/// Whether `writer` may write the IPC sysctls of its namespace: the
/// files are root's of the namespace's user namespace, writable with
/// CAP_SYS_RESOURCE over it (ipc_set_ownership, ipc_permissions), and
/// with MODIFY on the IPC namespace (ADR 0027).
pub fn sysctl_writer_may(writer: &Caller) -> bool {
    let ns = current_ns();
    writer.creds.euid == ns.owner.root_uid()
        && writer.ns_capable(&ns.owner, linux_caps::CAP_SYS_RESOURCE)
        && namespace_rights(&ns).modify
}

/// Write sysctl `name` of the writer's IPC namespace: integers in their
/// ranges (EINVAL otherwise); `sem` takes up to four, in order; setting
/// shm_rmid_forced destroys the orphaned segments with nothing attached.
pub fn sysctl_write(name: &str, text: &[u8]) -> Result<(), SyscallError> {
    let ns = current_ns();
    let space = space_of(&ns);
    let l = &space.limits;
    let text = core::str::from_utf8(text).map_err(|_| SyscallError::InvalidArgument)?;
    let int = |lo: i64, hi: i64| -> Result<i64, SyscallError> {
        text.trim()
            .parse::<i64>()
            .ok()
            .filter(|v| (lo..=hi).contains(v))
            .ok_or(SyscallError::InvalidArgument)
    };
    let r = Ordering::Relaxed;
    match name {
        "msgmax" => l.msgmax.store(int(0, i64::from(i32::MAX))? as i32, r),
        "msgmnb" => l.msgmnb.store(int(0, i64::from(i32::MAX))? as i32, r),
        "msgmni" => l.msgmni.store(int(0, i64::from(IPC_MNI))? as i32, r),
        "shmmni" => l.shmmni.store(int(0, i64::from(IPC_MNI))? as i32, r),
        "shm_rmid_forced" => {
            let v = int(0, 1)? as i32;
            l.shm_rmid_forced.store(v, r);
            if v != 0 {
                shm::destroy_orphaned(space);
            }
        }
        "shmall" | "shmmax" => {
            let v = text
                .trim()
                .parse::<u64>()
                .map_err(|_| SyscallError::InvalidArgument)?;
            if name == "shmall" {
                l.shmall.store(v, r);
            } else {
                l.shmmax.store(v, r);
            }
        }
        "sem" => {
            let vals: Option<alloc::vec::Vec<i32>> =
                text.split_whitespace().map(|w| w.parse().ok()).collect();
            let vals = vals.ok_or(SyscallError::InvalidArgument)?;
            if vals.is_empty() || vals.len() > 4 {
                return Err(SyscallError::InvalidArgument);
            }
            // sem_check_semmni: SEMMNI within IPCMNI.
            if vals.len() == 4 && !(0..=IPC_MNI as i32).contains(&vals[3]) {
                return Err(SyscallError::InvalidArgument);
            }
            for (i, v) in vals.iter().enumerate() {
                l.sem[i].store(*v, r);
            }
        }
        _ => return Err(SyscallError::InvalidArgument),
    }
    Ok(())
}

/// `/proc/sysvipc/<kind>` for the reader's IPC namespace.
pub fn proc_text(kind: &str) -> alloc::string::String {
    let ns = current_ns();
    let asker = Asker::current(&ns);
    let space = space_of(&ns);
    match kind {
        "shm" => shm::proc_lines(space, &asker),
        "sem" => sem::proc_lines(space, &asker),
        "msg" => msg::proc_lines(space, &asker),
        _ => alloc::string::String::new(),
    }
}

/// `struct ipc64_perm` (the same on every 64-bit architecture): key, uid,
/// gid, cuid, cgid, mode, a pad, seq (u16), a pad, and two unused longs.
pub const IPC64_PERM_SIZE: usize = 48;

pub fn encode_perm(p: &Perm, out: &mut [u8]) {
    out[0..4].copy_from_slice(&p.key.to_ne_bytes());
    out[4..8].copy_from_slice(&p.uid.to_ne_bytes());
    out[8..12].copy_from_slice(&p.gid.to_ne_bytes());
    out[12..16].copy_from_slice(&p.cuid.to_ne_bytes());
    out[16..20].copy_from_slice(&p.cgid.to_ne_bytes());
    out[20..24].copy_from_slice(&p.mode.to_ne_bytes());
    out[24..26].copy_from_slice(&(p.seq as u16).to_ne_bytes());
    out[26..IPC64_PERM_SIZE].fill(0);
}

/// The uid, gid and mode IPC_SET reads from a `struct ipc64_perm`.
pub fn decode_set_perm(buf: &[u8]) -> (u32, u32, u32) {
    let word = |at: usize| u32::from_ne_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
    (word(4), word(8), word(20))
}

pub fn put_long(out: &mut [u8], at: usize, v: u64) {
    out[at..at + 8].copy_from_slice(&v.to_ne_bytes());
}

pub fn get_long(buf: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[at..at + 8]);
    u64::from_ne_bytes(b)
}

/// A ctl command without the IPC_64 flag (the caller's library adds it;
/// there are no other layouts).
pub fn ctl_cmd(cmd: i32) -> i32 {
    cmd & !IPC_64
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Obj(pub Perm);

    impl IpcObject for Obj {
        fn perm(&self) -> &Perm {
            &self.0
        }
        fn perm_mut(&mut self) -> &mut Perm {
            &mut self.0
        }
    }

    pub(crate) fn asker(euid: u32, egid: u32) -> Asker {
        Asker {
            euid,
            egid,
            groups: alloc::vec![],
            ipc_owner: false,
            sys_admin: false,
            ipc_lock: false,
            sys_resource: false,
            may_read: true,
            may_write: true,
            may_create: true,
            may_modify: true,
            userns: crate::process::userns::initial(),
        }
    }

    fn obj(key: i32, mode: i32) -> Obj {
        Obj(Perm::new(key, mode, 1000, 1000))
    }

    #[test]
    fn identifiers_carry_a_sequence_number_so_a_reused_slot_is_a_new_id() {
        let mut ids: Ids<Obj> = Ids::new();
        let a = ids.add(obj(IPC_PRIVATE, 0o600), 100).unwrap();
        assert_eq!(a, 0);
        let b = ids.add(obj(IPC_PRIVATE, 0o600), 100).unwrap();
        assert_eq!(b, 1, "slots are handed out in turn");
        assert!(ids.remove(a).is_some());
        // Slot 2 is next, not the freed 0.
        assert_eq!(ids.add(obj(IPC_PRIVATE, 0o600), 100).unwrap(), 2);
        // With few in use the cycle is 64 slots: make and remove objects
        // until the search passes slot 63 and wraps to the free slot 0.
        for _ in 3..64 {
            let t = ids.add(obj(IPC_PRIVATE, 0o600), 100).unwrap();
            assert!(ids.remove(t).is_some());
        }
        let again = ids.add(obj(IPC_PRIVATE, 0o600), 100).unwrap();
        assert_eq!(again & 0x7fff, 0);
        assert_eq!(
            again >> 15,
            1,
            "a slot at or below the last takes a new sequence"
        );
        assert_eq!(ids.get(a).err(), Some(SyscallError::InvalidArgument));
        assert!(ids.get(again).is_ok());
        assert_eq!(ids.by_index(0).unwrap().perm().id, again);
        assert_eq!(ids.max_idx(), 2);
        // Many in use widen the cycle (in_use * 3 / 2), so slot 64 comes
        // next rather than a wrap.
        for _ in 0..62 {
            ids.add(obj(IPC_PRIVATE, 0o600), 100).unwrap();
        }
        assert_eq!(ids.max_idx(), 64);
    }

    #[test]
    fn a_full_table_is_enospc() {
        let mut ids: Ids<Obj> = Ids::new();
        ids.add(obj(IPC_PRIVATE, 0), 1).unwrap();
        assert_eq!(ids.add(obj(IPC_PRIVATE, 0), 1), Err(SyscallError::NoSpace));
    }

    #[test]
    fn get_follows_linux_for_keys() {
        let mut ids: Ids<Obj> = Ids::new();
        let me = asker(1000, 1000);
        let ok = |_: &Obj| Ok(());
        assert!(matches!(
            ipcget(&ids, &me, IPC_PRIVATE, 0, ok),
            Ok(GetOutcome::Create)
        ));
        assert_eq!(
            ipcget(&ids, &me, 42, 0, ok).err(),
            Some(SyscallError::ResourceNotFound)
        );
        assert!(matches!(
            ipcget(&ids, &me, 42, IPC_CREAT, ok),
            Ok(GetOutcome::Create)
        ));
        let id = ids.add(obj(42, 0o600), 100).unwrap();
        assert!(matches!(ipcget(&ids, &me, 42, 0, ok), Ok(GetOutcome::Existing(i)) if i == id));
        assert_eq!(
            ipcget(&ids, &me, 42, IPC_CREAT | IPC_EXCL, ok).err(),
            Some(SyscallError::FileExists)
        );
        // Another user without the bits: EACCES; the kind's check first.
        let other = asker(2000, 2000);
        assert_eq!(
            ipcget(&ids, &other, 42, 0o400, ok).err(),
            Some(SyscallError::PermissionDenied)
        );
        assert_eq!(
            ipcget(&ids, &other, 42, 0o400, |_| Err(
                SyscallError::InvalidArgument
            ))
            .err(),
            Some(SyscallError::InvalidArgument)
        );
        // No bits asked: only the key matters.
        assert!(ipcget(&ids, &other, 42, 0, ok).is_ok());
        // Made private, the key no longer finds it.
        ids.make_private(id);
        assert_eq!(ids.find_key(42), None);
        assert!(ids.get(id).is_ok());
    }

    #[test]
    fn permits_takes_owner_group_or_other_bits_and_cap_ipc_owner() {
        let perm = Perm::new(1, 0o640, 1000, 50);
        assert!(asker(1000, 1000).permits(&perm, 0o600));
        let mut group = asker(2000, 50);
        assert!(group.permits(&perm, 0o400));
        assert!(!group.permits(&perm, 0o200));
        group.ipc_owner = true;
        assert!(group.permits(&perm, 0o200));
        let other = asker(3000, 3000);
        assert!(!other.permits(&perm, 0o004));
        // The namespace's rights come first.
        let mut owner = asker(1000, 1000);
        owner.may_write = false;
        assert!(owner.permits(&perm, 0o400));
        assert!(!owner.permits(&perm, 0o200));
        owner.may_read = false;
        assert!(!owner.permits(&perm, 0o400));
    }

    #[test]
    fn control_needs_owner_creator_or_cap_sys_admin_and_modify() {
        let mut perm = Perm::new(1, 0o600, 1000, 1000);
        perm.uid = 1500;
        assert!(asker(1000, 1).may_control(&perm), "the creator");
        assert!(asker(1500, 1).may_control(&perm), "the owner");
        let mut other = asker(2000, 1);
        assert!(!other.may_control(&perm));
        other.sys_admin = true;
        assert!(other.may_control(&perm));
        other.may_modify = false;
        assert!(!other.may_control(&perm));
        other.may_modify = true;
        other.may_write = false;
        assert!(other.may_control(&perm), "WRITE is not what control needs");
    }

    #[test]
    fn perm_encodes_as_ipc64_perm() {
        let mut p = Perm::new(0x1234, 0o640, 1000, 50);
        p.seq = 3;
        let mut out = [0xffu8; IPC64_PERM_SIZE];
        encode_perm(&p, &mut out);
        assert_eq!(&out[0..4], &0x1234i32.to_ne_bytes());
        assert_eq!(&out[20..24], &0o640u32.to_ne_bytes());
        assert_eq!(&out[24..26], &3u16.to_ne_bytes());
        assert!(out[26..].iter().all(|&b| b == 0));
        assert_eq!(decode_set_perm(&out), (1000, 50, 0o640));
    }
}

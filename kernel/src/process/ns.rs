//! Linux namespaces (ADR 0022): the namespace objects, their identities,
//! and the per-thread set (`Namespaces`, Linux's nsproxy).
//!
//! Each namespace has Linux's identity: an inode number on nsfs (the
//! initial namespaces Linux's fixed values, new ones from 0xF0000000) and a
//! 64-bit id (the initial ones 1 to 8, new ones from 9). UTS, IPC, cgroup
//! (ADR 0022 step H1), time (H2) and mount (H3) namespaces can be created;
//! the other types exist only as their initial namespace until their steps,
//! and asking for a new one is EINVAL, as on Linux built without that type.

use alloc::{string::String, sync::Arc};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use spin::Mutex;

use super::userns::UserNs;

/// CLONE_NEW* flags, as `include/uapi/linux/sched.h`.
pub const CLONE_NEWTIME: u64 = 0x0000_0080;
pub const CLONE_NEWNS: u64 = 0x0002_0000;
pub const CLONE_NEWCGROUP: u64 = 0x0200_0000;
pub const CLONE_NEWUTS: u64 = 0x0400_0000;
pub const CLONE_NEWIPC: u64 = 0x0800_0000;
pub const CLONE_NEWUSER: u64 = 0x1000_0000;
pub const CLONE_NEWPID: u64 = 0x2000_0000;
pub const CLONE_NEWNET: u64 = 0x4000_0000;
/// Every namespace flag (Linux's CLONE_NS_ALL).
pub const CLONE_NS_ALL: u64 = CLONE_NEWTIME
    | CLONE_NEWNS
    | CLONE_NEWCGROUP
    | CLONE_NEWUTS
    | CLONE_NEWIPC
    | CLONE_NEWUSER
    | CLONE_NEWPID
    | CLONE_NEWNET;
/// clone3's flag for a new mount namespace holding only a copy of the root
/// mount (Linux 7.0; it implies CLONE_NEWNS). unshare names it
/// UNSHARE_EMPTY_MNTNS and passes it on as this, as Linux's
/// unshare_nsproxy_namespaces does.
pub const CLONE_EMPTY_MNTNS: u64 = 1 << 37;
/// The types a new namespace can be made of so far (ADR 0022 steps H1 to
/// H3).
pub const CREATABLE: u64 =
    CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWCGROUP | CLONE_NEWTIME | CLONE_NEWNS;

/// The clock ids a time namespace offsets (`include/uapi/linux/time.h`).
pub const CLOCK_MONOTONIC: i32 = 1;
const CLOCK_MONOTONIC_RAW: i32 = 4;
const CLOCK_MONOTONIC_COARSE: i32 = 6;
pub const CLOCK_BOOTTIME: i32 = 7;
const CLOCK_BOOTTIME_ALARM: i32 = 9;

/// The longest host or domain name (Linux's __NEW_UTS_LEN).
pub const UTS_LEN: usize = 64;

/// A namespace type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NsKind {
    Mnt,
    Uts,
    Ipc,
    User,
    Pid,
    Net,
    Cgroup,
    Time,
}

impl NsKind {
    pub const ALL: [NsKind; 8] = [
        NsKind::Mnt,
        NsKind::Uts,
        NsKind::Ipc,
        NsKind::User,
        NsKind::Pid,
        NsKind::Net,
        NsKind::Cgroup,
        NsKind::Time,
    ];

    /// Its CLONE_NEW* flag (what NS_GET_NSTYPE returns).
    pub const fn clone_flag(self) -> u64 {
        match self {
            NsKind::Mnt => CLONE_NEWNS,
            NsKind::Uts => CLONE_NEWUTS,
            NsKind::Ipc => CLONE_NEWIPC,
            NsKind::User => CLONE_NEWUSER,
            NsKind::Pid => CLONE_NEWPID,
            NsKind::Net => CLONE_NEWNET,
            NsKind::Cgroup => CLONE_NEWCGROUP,
            NsKind::Time => CLONE_NEWTIME,
        }
    }

    /// The type a single CLONE_NEW* flag names.
    pub fn from_flag(flag: u64) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.clone_flag() == flag)
    }

    /// Its name in `/proc/<pid>/ns` links ("uts:[4026531838]").
    pub const fn name(self) -> &'static str {
        match self {
            NsKind::Mnt => "mnt",
            NsKind::Uts => "uts",
            NsKind::Ipc => "ipc",
            NsKind::User => "user",
            NsKind::Pid => "pid",
            NsKind::Net => "net",
            NsKind::Cgroup => "cgroup",
            NsKind::Time => "time",
        }
    }

    /// The initial namespace's inode number (Linux's `init_ns_ino`).
    const fn init_inum(self) -> u32 {
        match self {
            NsKind::Ipc => 0xEFFF_FFFF,
            NsKind::Uts => 0xEFFF_FFFE,
            NsKind::User => 0xEFFF_FFFD,
            NsKind::Pid => 0xEFFF_FFFC,
            NsKind::Cgroup => 0xEFFF_FFFB,
            NsKind::Time => 0xEFFF_FFFA,
            NsKind::Net => 0xEFFF_FFF9,
            NsKind::Mnt => 0xEFFF_FFF8,
        }
    }

    /// The initial namespace's id (Linux's `init_ns_id`).
    const fn init_id(self) -> u64 {
        match self {
            NsKind::Ipc => 1,
            NsKind::Uts => 2,
            NsKind::User => 3,
            NsKind::Pid => 4,
            NsKind::Cgroup => 5,
            NsKind::Time => 6,
            NsKind::Net => 7,
            NsKind::Mnt => 8,
        }
    }
}

/// A namespace's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NsIdent {
    pub kind: NsKind,
    /// Its inode number on nsfs.
    pub inum: u32,
    /// Its 64-bit id (NS_GET_ID).
    pub id: u64,
}

/// Inode numbers for new namespaces (Linux's proc_alloc_inum, from
/// PROC_DYNAMIC_FIRST).
static NEXT_INUM: AtomicU32 = AtomicU32::new(0xF000_0000);
/// Ids for new namespaces, after the initial ones'.
static NEXT_ID: AtomicU64 = AtomicU64::new(9);

impl NsIdent {
    pub(super) const fn initial(kind: NsKind) -> Self {
        Self {
            kind,
            inum: kind.init_inum(),
            id: kind.init_id(),
        }
    }

    pub(super) fn fresh(kind: NsKind) -> Self {
        Self {
            kind,
            inum: NEXT_INUM.fetch_add(1, Ordering::Relaxed),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// What a `/proc/<pid>/ns` link reads: "uts:[4026531838]".
    pub fn link_text(&self) -> String {
        alloc::format!("{}:[{}]", self.kind.name(), self.inum)
    }
}

/// A UTS namespace: the host and domain names uname reports.
pub struct UtsNs {
    pub ident: NsIdent,
    /// The user namespace it belongs to (its creator's, or the one made
    /// with it).
    pub owner: Arc<UserNs>,
    hostname: Mutex<String>,
    domainname: Mutex<String>,
}

impl UtsNs {
    pub fn hostname(&self) -> String {
        self.hostname.lock().clone()
    }

    pub fn domainname(&self) -> String {
        self.domainname.lock().clone()
    }

    /// Set the host name (the caller checked privilege and length).
    pub fn set_hostname(&self, name: &str) {
        *self.hostname.lock() = String::from(name);
    }

    pub fn set_domainname(&self, name: &str) {
        *self.domainname.lock() = String::from(name);
    }
}

/// A time namespace's offsets from the host's clocks, in nanoseconds
/// (Linux's `struct timens_offsets`), and whether they can still change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimeOffsets {
    pub monotonic: i64,
    pub boottime: i64,
    /// A process has entered the namespace: the offsets are fixed (Linux's
    /// frozen_offsets).
    frozen: bool,
}

/// A time namespace (Linux's `kernel/time/namespace.c`): CLOCK_MONOTONIC
/// and CLOCK_BOOTTIME read its offsets ahead of the host's, and absolute
/// times on them are taken back by them. A new one starts with the offsets
/// of the namespace it was made from, which `/proc/<pid>/timens_offsets`
/// can change until a process enters it.
pub struct TimeNs {
    pub ident: NsIdent,
    pub owner: Arc<UserNs>,
    offsets: Mutex<TimeOffsets>,
}

/// Why `/proc/<pid>/timens_offsets` refused new offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetError {
    /// A process is in the namespace (EACCES).
    Frozen,
}

impl TimeNs {
    pub fn offsets(&self) -> TimeOffsets {
        *self.offsets.lock()
    }

    /// Fix the offsets: a process enters the namespace (Linux's
    /// timens_commit).
    pub fn freeze(&self) {
        self.offsets.lock().frozen = true;
    }

    /// Set the offsets given (proc_timens_set_offset, whose caller checked
    /// privilege and range): EACCES once a process has entered.
    pub fn set_offsets(
        &self,
        monotonic: Option<i64>,
        boottime: Option<i64>,
    ) -> Result<(), OffsetError> {
        let mut offsets = self.offsets.lock();
        if offsets.frozen {
            return Err(OffsetError::Frozen);
        }
        if let Some(ns) = monotonic {
            offsets.monotonic = ns;
        }
        if let Some(ns) = boottime {
            offsets.boottime = ns;
        }
        Ok(())
    }

    /// What reading clock `clock_id` adds to the host's time: the
    /// monotonic offset for MONOTONIC and its raw and coarse forms, the
    /// boot-time one for BOOTTIME and its alarm form (Linux's
    /// timens_add_monotonic and timens_add_boottime), 0 for the others.
    pub fn read_offset(&self, clock_id: i32) -> i64 {
        let offsets = self.offsets();
        match clock_id {
            CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE => offsets.monotonic,
            CLOCK_BOOTTIME | CLOCK_BOOTTIME_ALARM => offsets.boottime,
            _ => 0,
        }
    }

    /// An absolute time `t` on clock `clock_id` as the host's clock reads
    /// it (Linux's do_timens_ktime_to_host): only MONOTONIC, BOOTTIME and
    /// BOOTTIME_ALARM move; a time before the offset is already past (0).
    pub fn to_host(&self, clock_id: i32, t: i64) -> i64 {
        let offsets = self.offsets();
        let offset = match clock_id {
            CLOCK_MONOTONIC => offsets.monotonic,
            CLOCK_BOOTTIME | CLOCK_BOOTTIME_ALARM => offsets.boottime,
            _ => return t,
        };
        if t < offset {
            0
        } else {
            t.saturating_sub(offset)
        }
    }
}

/// The time namespace the calling thread runs in (the initial one outside
/// a thread).
pub fn current_time() -> Arc<TimeNs> {
    match crate::process::current_thread() {
        Some(t) => t.namespaces().time.clone(),
        None => initial_all().set.time.clone(),
    }
}

/// The host's monotonic time `host_ns` as clock `clock_id` reads it in the
/// caller's time namespace (never below 0).
pub fn clock_from_host(clock_id: i32, host_ns: u64) -> u64 {
    let offset = current_time().read_offset(clock_id);
    (host_ns.min(i64::MAX as u64) as i64)
        .saturating_add(offset)
        .max(0) as u64
}

/// An absolute time `t` on clock `clock_id` in the caller's time namespace
/// as the host's monotonic time (see [`TimeNs::to_host`]).
pub fn clock_to_host(clock_id: i32, t: u64) -> u64 {
    current_time().to_host(clock_id, t.min(i64::MAX as u64) as i64) as u64
}

/// A mount namespace (Linux's `struct mnt_namespace`): its tree of
/// mounts, which path lookups in it walk (`fs::Vfs::caller_context`).
pub struct MntNs {
    pub ident: NsIdent,
    pub owner: Arc<UserNs>,
    pub mounts: Arc<crate::fs::mount::Mounts>,
}

/// A namespace that holds nothing yet besides its identity: IPC (no
/// System V IPC or message queues exist), cgroup (no hierarchy), and the
/// types whose own state comes with their later steps (ADR 0022).
pub struct PlainNs {
    pub ident: NsIdent,
    pub owner: Arc<UserNs>,
}

/// One namespace of any type, as an nsfs file and setns name it.
#[derive(Clone)]
pub enum NsRef {
    Uts(Arc<UtsNs>),
    Time(Arc<TimeNs>),
    Mnt(Arc<MntNs>),
    Plain(Arc<PlainNs>),
    User(Arc<UserNs>),
}

impl NsRef {
    pub fn ident(&self) -> NsIdent {
        match self {
            NsRef::Uts(ns) => ns.ident,
            NsRef::Time(ns) => ns.ident,
            NsRef::Mnt(ns) => ns.ident,
            NsRef::Plain(ns) => ns.ident,
            NsRef::User(ns) => ns.ident,
        }
    }

    /// The user namespace that owns it (Linux's ns->ops->owner): for a
    /// user namespace its parent, None for the initial one.
    pub fn owner(&self) -> Option<Arc<UserNs>> {
        match self {
            NsRef::Uts(ns) => Some(ns.owner.clone()),
            NsRef::Time(ns) => Some(ns.owner.clone()),
            NsRef::Mnt(ns) => Some(ns.owner.clone()),
            NsRef::Plain(ns) => Some(ns.owner.clone()),
            NsRef::User(ns) => ns.parent.clone(),
        }
    }

    pub fn kind(&self) -> NsKind {
        self.ident().kind
    }

    /// Whether `self` and `other` are one namespace.
    pub fn same(&self, other: &NsRef) -> bool {
        self.ident().id == other.ident().id
    }
}

/// A thread's namespaces (Linux's nsproxy; the user namespace belongs to
/// the credentials and the PID namespace a process is in to the process,
/// both the initial ones until ADR 0022's H4 and H5).
#[derive(Clone)]
pub struct Namespaces {
    pub uts: Arc<UtsNs>,
    pub ipc: Arc<PlainNs>,
    pub mnt: Arc<MntNs>,
    pub pid_for_children: Arc<PlainNs>,
    pub net: Arc<PlainNs>,
    pub cgroup: Arc<PlainNs>,
    pub time: Arc<TimeNs>,
    pub time_for_children: Arc<TimeNs>,
}

impl core::fmt::Debug for Namespaces {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Namespaces")
            .field("uts", &self.uts.ident.id)
            .field("ipc", &self.ipc.ident.id)
            .field("mnt", &self.mnt.ident.id)
            .field("pid_for_children", &self.pid_for_children.ident.id)
            .field("net", &self.net.ident.id)
            .field("cgroup", &self.cgroup.ident.id)
            .field("time", &self.time.ident.id)
            .field("time_for_children", &self.time_for_children.ident.id)
            .finish()
    }
}

/// Why a namespace set could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NsError {
    /// A type that cannot be created yet (EINVAL, as Linux without it).
    Unsupported,
}

struct Initial {
    set: Arc<Namespaces>,
    pid: Arc<PlainNs>,
}

static INITIAL: spin::Once<Initial> = spin::Once::new();

fn initial_all() -> &'static Initial {
    INITIAL.call_once(|| {
        let owner = super::userns::initial();
        let plain = |kind| {
            Arc::new(PlainNs {
                ident: NsIdent::initial(kind),
                owner: owner.clone(),
            })
        };
        let pid = plain(NsKind::Pid);
        // The initial time namespace's offsets are 0 and fixed.
        let time = Arc::new(TimeNs {
            ident: NsIdent::initial(NsKind::Time),
            owner: owner.clone(),
            offsets: Mutex::new(TimeOffsets {
                frozen: true,
                ..TimeOffsets::default()
            }),
        });
        Initial {
            set: Arc::new(Namespaces {
                uts: Arc::new(UtsNs {
                    ident: NsIdent::initial(NsKind::Uts),
                    owner: owner.clone(),
                    hostname: Mutex::new(String::from(crate::utils::uname::NODENAME)),
                    domainname: Mutex::new(String::from("(none)")),
                }),
                ipc: plain(NsKind::Ipc),
                mnt: Arc::new(MntNs {
                    ident: NsIdent::initial(NsKind::Mnt),
                    owner: owner.clone(),
                    mounts: crate::fs::initial_mounts(),
                }),
                pid_for_children: pid.clone(),
                net: plain(NsKind::Net),
                cgroup: plain(NsKind::Cgroup),
                time: time.clone(),
                time_for_children: time,
            }),
            pid,
        }
    })
}

/// The initial namespaces, which the first process and every kernel
/// thread have.
pub fn initial() -> Arc<Namespaces> {
    initial_all().set.clone()
}

/// The user namespace a process's credentials are in.
pub fn user_of(process: &super::Process) -> NsRef {
    NsRef::User(process.user_ns())
}

/// The PID namespace a process is in (only the initial one until ADR
/// 0022's H5).
pub fn pid_of(_process: &super::Process) -> NsRef {
    NsRef::Plain(initial_all().pid.clone())
}

impl Namespaces {
    /// The namespace of `kind` in this set, as `/proc/<pid>/ns` shows it
    /// (`pid` and `user` are the process's, not in the set, and `pid` here
    /// is `pid_for_children`).
    pub fn get(&self, kind: NsKind) -> NsRef {
        match kind {
            NsKind::Uts => NsRef::Uts(self.uts.clone()),
            NsKind::Ipc => NsRef::Plain(self.ipc.clone()),
            NsKind::Mnt => NsRef::Mnt(self.mnt.clone()),
            NsKind::Pid => NsRef::Plain(self.pid_for_children.clone()),
            NsKind::Net => NsRef::Plain(self.net.clone()),
            NsKind::Cgroup => NsRef::Plain(self.cgroup.clone()),
            NsKind::Time => NsRef::Time(self.time_for_children.clone()),
            // Not in the set: the credentials' ([`user_of`]).
            NsKind::User => NsRef::User(super::userns::initial()),
        }
    }

    /// Linux's create_new_namespaces: this set with a new namespace of
    /// each type in `flags` (a new UTS namespace starts with this one's
    /// names, as copy_utsname copies them; a new time namespace becomes the
    /// one for children, with the offsets of the current one for children,
    /// as copy_time_ns; a new mount namespace a copy of this one's tree, or
    /// with CLONE_EMPTY_MNTNS of its root mount only, as copy_mnt_ns). A type
    /// not yet creatable is EINVAL; CLONE_NEWUSER is the credentials', not
    /// looked at here. The new namespaces belong to user namespace `owner`.
    pub fn copy_with(&self, flags: u64, owner: &Arc<UserNs>) -> Result<Namespaces, NsError> {
        let wanted = flags & CLONE_NS_ALL & !CLONE_NEWUSER;
        if wanted & !CREATABLE != 0 {
            return Err(NsError::Unsupported);
        }
        let mut new = self.clone();
        let plain = |kind| {
            Arc::new(PlainNs {
                ident: NsIdent::fresh(kind),
                owner: owner.clone(),
            })
        };
        if flags & CLONE_NEWUTS != 0 {
            new.uts = Arc::new(UtsNs {
                ident: NsIdent::fresh(NsKind::Uts),
                owner: owner.clone(),
                hostname: Mutex::new(self.uts.hostname()),
                domainname: Mutex::new(self.uts.domainname()),
            });
        }
        if flags & CLONE_NEWIPC != 0 {
            new.ipc = plain(NsKind::Ipc);
        }
        if flags & CLONE_NEWCGROUP != 0 {
            new.cgroup = plain(NsKind::Cgroup);
        }
        if flags & CLONE_NEWNS != 0 {
            new.mnt = Arc::new(MntNs {
                ident: NsIdent::fresh(NsKind::Mnt),
                owner: owner.clone(),
                mounts: Arc::new(self.mnt.mounts.copy(flags & CLONE_EMPTY_MNTNS != 0)),
            });
        }
        if flags & CLONE_NEWTIME != 0 {
            let offsets = self.time_for_children.offsets();
            new.time_for_children = Arc::new(TimeNs {
                ident: NsIdent::fresh(NsKind::Time),
                owner: owner.clone(),
                offsets: Mutex::new(TimeOffsets {
                    frozen: false,
                    ..offsets
                }),
            });
        }
        Ok(new)
    }

    /// This set with `ns` in place of its namespace of the same type
    /// (setns's commit). The user namespace is not in the set.
    pub fn with(&self, ns: &NsRef) -> Namespaces {
        let mut new = self.clone();
        match (ns.kind(), ns) {
            (NsKind::Uts, NsRef::Uts(uts)) => new.uts = uts.clone(),
            (NsKind::Ipc, NsRef::Plain(p)) => new.ipc = p.clone(),
            (NsKind::Mnt, NsRef::Mnt(m)) => new.mnt = m.clone(),
            (NsKind::Pid, NsRef::Plain(p)) => new.pid_for_children = p.clone(),
            (NsKind::Net, NsRef::Plain(p)) => new.net = p.clone(),
            (NsKind::Cgroup, NsRef::Plain(p)) => new.cgroup = p.clone(),
            (NsKind::Time, NsRef::Time(t)) => {
                new.time = t.clone();
                new.time_for_children = t.clone();
            }
            _ => {}
        }
        new
    }

    /// The set a child gets at clone or fork, after any new namespaces:
    /// one that runs in its own address space (no CLONE_VM) enters the
    /// time namespace for children (Linux's timens_on_fork).
    pub fn for_child(mut self, share_vm: bool) -> Namespaces {
        if !share_vm {
            self.enter_time_for_children();
        }
        self
    }

    /// Whether the time namespace for children is not the one this set
    /// runs in (after unshare(CLONE_NEWTIME)).
    pub fn time_pending(&self) -> bool {
        self.time.ident != self.time_for_children.ident
    }

    /// Run in the time namespace for children, which then cannot change
    /// its offsets (Linux's timens_on_fork and timens_commit).
    fn enter_time_for_children(&mut self) {
        if self.time_pending() {
            self.time = self.time_for_children.clone();
            self.time.freeze();
        }
    }

    /// The set an exec leaves: the time namespace for children entered
    /// (Linux's exec_task_namespaces), or None when nothing changes.
    pub fn after_exec(&self) -> Option<Namespaces> {
        if !self.time_pending() {
            return None;
        }
        let mut new = self.clone();
        new.enter_time_for_children();
        Some(new)
    }
}

/// Linux's copy_namespaces, for clone: the child's set given the caller's
/// `current`, the clone `flags`, whether the caller has CAP_SYS_ADMIN, and
/// the user namespace new namespaces belong to (the child's).
/// No namespace flag shares the set (unless the child must enter the time
/// namespace for children); any but CLONE_NEWUSER needs CAP_SYS_ADMIN
/// (EPERM); CLONE_NEWIPC with CLONE_SYSVSEM is EINVAL (the undo list would
/// span two namespaces); a type not yet creatable is EINVAL.
pub fn for_clone(
    current: &Arc<Namespaces>,
    flags: u64,
    admin: bool,
    owner: &Arc<UserNs>,
) -> Result<Arc<Namespaces>, CloneNsError> {
    const CLONE_VM: u64 = 0x100;
    const CLONE_SYSVSEM: u64 = 0x0004_0000;
    let share_vm = flags & CLONE_VM != 0;
    if flags & CLONE_NS_ALL & !CLONE_NEWUSER == 0 {
        if share_vm || current.time_for_children.ident == current.time.ident {
            return Ok(current.clone());
        }
        return Ok(Arc::new((**current).clone().for_child(false)));
    }
    if !admin {
        return Err(CloneNsError::Permission);
    }
    if flags & (CLONE_NEWIPC | CLONE_SYSVSEM) == CLONE_NEWIPC | CLONE_SYSVSEM {
        return Err(CloneNsError::Invalid);
    }
    let new = current
        .copy_with(flags, owner)
        .map_err(|_| CloneNsError::Invalid)?;
    Ok(Arc::new(new.for_child(share_vm)))
}

/// Why clone or unshare refused its namespace flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneNsError {
    /// EPERM: no CAP_SYS_ADMIN.
    Permission,
    /// EINVAL.
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::userns;

    #[test]
    fn initial_identities_are_linuxs() {
        let set = initial();
        assert_eq!(set.uts.ident.inum, 4_026_531_838);
        assert_eq!(set.get(NsKind::Uts).ident().link_text(), "uts:[4026531838]");
        assert_eq!(set.get(NsKind::Mnt).ident().inum, 0xEFFF_FFF8);
        assert_eq!(set.get(NsKind::Ipc).ident().id, 1);
        assert_eq!(set.get(NsKind::Mnt).ident().id, 8);
        assert_eq!(NsKind::from_flag(CLONE_NEWNET), Some(NsKind::Net));
        assert_eq!(NsKind::from_flag(CLONE_NEWNET | CLONE_NEWUTS), None);
    }

    #[test]
    fn a_new_uts_namespace_copies_the_names_and_is_its_own() {
        let set = initial();
        let new = set.copy_with(CLONE_NEWUTS, &userns::initial()).unwrap();
        assert!(!new.get(NsKind::Uts).same(&set.get(NsKind::Uts)));
        assert!(new.get(NsKind::Ipc).same(&set.get(NsKind::Ipc)));
        assert_eq!(new.uts.hostname(), set.uts.hostname());
        assert!(new.uts.ident.inum >= 0xF000_0000);
        assert!(new.uts.ident.id > 8);
        new.uts.set_hostname("other");
        assert_ne!(set.uts.hostname(), "other");
    }

    #[test]
    fn types_not_yet_creatable_are_einval() {
        let set = initial();
        for flag in [CLONE_NEWPID, CLONE_NEWNET] {
            assert_eq!(
                set.copy_with(flag, &userns::initial()).err(),
                Some(NsError::Unsupported)
            );
        }
        assert!(set
            .copy_with(
                CLONE_NEWIPC | CLONE_NEWCGROUP | CLONE_NEWTIME,
                &userns::initial()
            )
            .is_ok());
    }

    #[test]
    fn a_new_mount_namespace_copies_the_tree_or_only_its_root() {
        let vfs = crate::fs::Vfs::new();
        vfs.mount_root(Arc::new(crate::fs::ramfs::RamFs::new()))
            .unwrap();
        vfs.mount("/tmp".into(), Arc::new(crate::fs::ramfs::RamFs::new()))
            .unwrap();
        let mut set = (*initial()).clone();
        set.mnt = Arc::new(MntNs {
            ident: NsIdent::fresh(NsKind::Mnt),
            mounts: vfs.mounts().clone(),
            owner: userns::initial(),
        });
        let full = set.copy_with(CLONE_NEWNS, &userns::initial()).unwrap();
        assert!(!full.get(NsKind::Mnt).same(&set.get(NsKind::Mnt)));
        assert!(full.get(NsKind::Uts).same(&set.get(NsKind::Uts)));
        assert!(full.mnt.mounts.render_mounts("/").contains(" /tmp "));
        assert!(!Arc::ptr_eq(&full.mnt.mounts, &set.mnt.mounts));
        // The empty one: the root mount only. CLONE_PARENT_SETTID, which
        // UNSHARE_EMPTY_MNTNS aliases, does not ask for it.
        let empty = set
            .copy_with(CLONE_NEWNS | CLONE_EMPTY_MNTNS, &userns::initial())
            .unwrap();
        assert_eq!(
            empty.mnt.mounts.render_mounts("/"),
            "ramfs / ramfs rw,relatime 0 0\n"
        );
        let settid = set
            .copy_with(CLONE_NEWNS | 0x0010_0000, &userns::initial())
            .unwrap();
        assert!(settid.mnt.mounts.render_mounts("/").contains(" /tmp "));
        // setns to it replaces the set's mount namespace.
        let joined = set.with(&full.get(NsKind::Mnt));
        assert!(joined.get(NsKind::Mnt).same(&full.get(NsKind::Mnt)));
    }

    #[test]
    fn clone_namespace_rules_are_copy_namespaces() {
        let set = initial();
        // No namespace flag: the same set.
        let same = for_clone(&set, 0x100, false, &userns::initial()).unwrap();
        assert!(Arc::ptr_eq(&same, &set));
        assert_eq!(
            for_clone(&set, CLONE_NEWUTS, false, &userns::initial()).err(),
            Some(CloneNsError::Permission)
        );
        assert_eq!(
            for_clone(&set, CLONE_NEWIPC | 0x0004_0000, true, &userns::initial()).err(),
            Some(CloneNsError::Invalid)
        );
        // EPERM comes before a type that does not exist yet.
        assert_eq!(
            for_clone(&set, CLONE_NEWPID, false, &userns::initial()).err(),
            Some(CloneNsError::Permission)
        );
        assert_eq!(
            for_clone(&set, CLONE_NEWPID, true, &userns::initial()).err(),
            Some(CloneNsError::Invalid)
        );
        let new = for_clone(&set, CLONE_NEWUTS, true, &userns::initial()).unwrap();
        assert!(!new.get(NsKind::Uts).same(&set.get(NsKind::Uts)));
    }

    #[test]
    fn a_new_time_namespace_is_for_children_until_entered() {
        let set = initial();
        assert_eq!(
            set.time.set_offsets(Some(1), None),
            Err(OffsetError::Frozen)
        );
        let new = set.copy_with(CLONE_NEWTIME, &userns::initial()).unwrap();
        assert!(new.time_pending());
        assert!(Arc::ptr_eq(&new.time, &set.time));
        // Offsets can change until a process enters.
        new.time_for_children
            .set_offsets(Some(1_000_000_000_000), Some(-5))
            .unwrap();
        assert_eq!(
            new.time_for_children.read_offset(CLOCK_MONOTONIC),
            1_000_000_000_000
        );
        assert_eq!(new.time_for_children.read_offset(4), 1_000_000_000_000);
        assert_eq!(new.time_for_children.read_offset(CLOCK_BOOTTIME), -5);
        assert_eq!(new.time_for_children.read_offset(0), 0);
        // A CLONE_VM child stays where it was; another enters.
        assert!(new.clone().for_child(true).time_pending());
        let child = new.clone().for_child(false);
        assert!(!child.time_pending());
        assert!(Arc::ptr_eq(&child.time, &new.time_for_children));
        assert_eq!(
            new.time_for_children.set_offsets(Some(0), None),
            Err(OffsetError::Frozen)
        );
        // Exec enters it too; a set with nothing pending stays.
        assert!(child.after_exec().is_none());
        let execed = new.after_exec().unwrap();
        assert!(Arc::ptr_eq(&execed.time, &new.time_for_children));
        // A namespace made from it starts with its offsets, unfrozen.
        let next = child.copy_with(CLONE_NEWTIME, &userns::initial()).unwrap();
        assert_eq!(
            next.time_for_children.offsets().monotonic,
            1_000_000_000_000
        );
        assert!(next.time_for_children.set_offsets(None, Some(0)).is_ok());
    }

    #[test]
    fn absolute_times_go_back_by_the_offset() {
        let set = initial()
            .copy_with(CLONE_NEWTIME, &userns::initial())
            .unwrap();
        let t = &set.time_for_children;
        t.set_offsets(Some(100), Some(1_000)).unwrap();
        assert_eq!(t.to_host(CLOCK_MONOTONIC, 150), 50);
        assert_eq!(t.to_host(CLOCK_MONOTONIC, 50), 0);
        assert_eq!(t.to_host(CLOCK_BOOTTIME, 1_500), 500);
        assert_eq!(t.to_host(9, 1_500), 500);
        // The raw and coarse clocks have no timers; others do not move.
        assert_eq!(t.to_host(4, 150), 150);
        assert_eq!(t.to_host(0, 150), 150);
    }

    #[test]
    fn setns_replaces_one_namespace() {
        let set = initial();
        let other = set.copy_with(CLONE_NEWUTS, &userns::initial()).unwrap();
        let joined = set.with(&other.get(NsKind::Uts));
        assert!(joined.get(NsKind::Uts).same(&other.get(NsKind::Uts)));
        assert!(joined.get(NsKind::Ipc).same(&set.get(NsKind::Ipc)));
    }
}

//! Linux namespaces (ADR 0022): the namespace objects, their identities,
//! and the per-thread set (`Namespaces`, Linux's nsproxy).
//!
//! Each namespace has Linux's identity: an inode number on nsfs (the
//! initial namespaces Linux's fixed values, new ones from 0xF0000000) and a
//! 64-bit id (the initial ones 1 to 8, new ones from 9). UTS, IPC and
//! cgroup namespaces can be created (ADR 0022 step H1); the other types
//! exist only as their initial namespace until their steps, and asking for
//! a new one is EINVAL, as on Linux built without that namespace type.

use alloc::{string::String, sync::Arc};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use spin::Mutex;

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
/// The types a new namespace can be made of so far (ADR 0022 step H1).
pub const CREATABLE: u64 = CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWCGROUP;

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
    const fn initial(kind: NsKind) -> Self {
        Self {
            kind,
            inum: kind.init_inum(),
            id: kind.init_id(),
        }
    }

    fn fresh(kind: NsKind) -> Self {
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

/// A namespace that holds nothing yet besides its identity: IPC (no
/// System V IPC or message queues exist), cgroup (no hierarchy), and the
/// types whose own state comes with their later steps (ADR 0022).
pub struct PlainNs {
    pub ident: NsIdent,
}

/// One namespace of any type, as an nsfs file and setns name it.
#[derive(Clone)]
pub enum NsRef {
    Uts(Arc<UtsNs>),
    Plain(Arc<PlainNs>),
}

impl NsRef {
    pub fn ident(&self) -> NsIdent {
        match self {
            NsRef::Uts(ns) => ns.ident,
            NsRef::Plain(ns) => ns.ident,
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
    pub mnt: Arc<PlainNs>,
    pub pid_for_children: Arc<PlainNs>,
    pub net: Arc<PlainNs>,
    pub cgroup: Arc<PlainNs>,
    pub time: Arc<PlainNs>,
    pub time_for_children: Arc<PlainNs>,
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
    user: Arc<PlainNs>,
    pid: Arc<PlainNs>,
}

static INITIAL: spin::Once<Initial> = spin::Once::new();

fn initial_all() -> &'static Initial {
    INITIAL.call_once(|| {
        let plain = |kind| {
            Arc::new(PlainNs {
                ident: NsIdent::initial(kind),
            })
        };
        let pid = plain(NsKind::Pid);
        let time = plain(NsKind::Time);
        Initial {
            set: Arc::new(Namespaces {
                uts: Arc::new(UtsNs {
                    ident: NsIdent::initial(NsKind::Uts),
                    hostname: Mutex::new(String::from(crate::utils::uname::NODENAME)),
                    domainname: Mutex::new(String::from("(none)")),
                }),
                ipc: plain(NsKind::Ipc),
                mnt: plain(NsKind::Mnt),
                pid_for_children: pid.clone(),
                net: plain(NsKind::Net),
                cgroup: plain(NsKind::Cgroup),
                time: time.clone(),
                time_for_children: time,
            }),
            user: plain(NsKind::User),
            pid,
        }
    })
}

/// The initial namespaces, which the first process and every kernel
/// thread have.
pub fn initial() -> Arc<Namespaces> {
    initial_all().set.clone()
}

/// The user namespace a process's credentials are in (only the initial one
/// until ADR 0022's H4).
pub fn user_of(_process: &super::Process) -> NsRef {
    NsRef::Plain(initial_all().user.clone())
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
            NsKind::Mnt => NsRef::Plain(self.mnt.clone()),
            NsKind::Pid => NsRef::Plain(self.pid_for_children.clone()),
            NsKind::Net => NsRef::Plain(self.net.clone()),
            NsKind::Cgroup => NsRef::Plain(self.cgroup.clone()),
            NsKind::Time => NsRef::Plain(self.time_for_children.clone()),
            NsKind::User => NsRef::Plain(initial_all().user.clone()),
        }
    }

    /// Linux's create_new_namespaces: this set with a new namespace of
    /// each type in `flags` (a new UTS namespace starts with this one's
    /// names, as copy_utsname copies them). A type not yet creatable is
    /// EINVAL; CLONE_NEWUSER is the credentials', not looked at here.
    pub fn copy_with(&self, flags: u64) -> Result<Namespaces, NsError> {
        let wanted = flags & CLONE_NS_ALL & !CLONE_NEWUSER;
        if wanted & !CREATABLE != 0 {
            return Err(NsError::Unsupported);
        }
        let mut new = self.clone();
        let plain = |kind| {
            Arc::new(PlainNs {
                ident: NsIdent::fresh(kind),
            })
        };
        if flags & CLONE_NEWUTS != 0 {
            new.uts = Arc::new(UtsNs {
                ident: NsIdent::fresh(NsKind::Uts),
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
        Ok(new)
    }

    /// This set with `ns` in place of its namespace of the same type
    /// (setns's commit). The user namespace is not in the set.
    pub fn with(&self, ns: &NsRef) -> Namespaces {
        let mut new = self.clone();
        match (ns.kind(), ns) {
            (NsKind::Uts, NsRef::Uts(uts)) => new.uts = uts.clone(),
            (NsKind::Ipc, NsRef::Plain(p)) => new.ipc = p.clone(),
            (NsKind::Mnt, NsRef::Plain(p)) => new.mnt = p.clone(),
            (NsKind::Pid, NsRef::Plain(p)) => new.pid_for_children = p.clone(),
            (NsKind::Net, NsRef::Plain(p)) => new.net = p.clone(),
            (NsKind::Cgroup, NsRef::Plain(p)) => new.cgroup = p.clone(),
            (NsKind::Time, NsRef::Plain(p)) => {
                new.time = p.clone();
                new.time_for_children = p.clone();
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
            self.time = self.time_for_children.clone();
        }
        self
    }
}

/// Linux's copy_namespaces, for clone: the child's set given the caller's
/// `current`, the clone `flags` and whether the caller has CAP_SYS_ADMIN.
/// No namespace flag shares the set (unless the child must enter the time
/// namespace for children); any but CLONE_NEWUSER needs CAP_SYS_ADMIN
/// (EPERM); CLONE_NEWIPC with CLONE_SYSVSEM is EINVAL (the undo list would
/// span two namespaces); a type not yet creatable is EINVAL.
pub fn for_clone(
    current: &Arc<Namespaces>,
    flags: u64,
    admin: bool,
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
        .copy_with(flags)
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
        let new = set.copy_with(CLONE_NEWUTS).unwrap();
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
        for flag in [CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWNET, CLONE_NEWTIME] {
            assert_eq!(set.copy_with(flag).err(), Some(NsError::Unsupported));
        }
        assert!(set.copy_with(CLONE_NEWIPC | CLONE_NEWCGROUP).is_ok());
    }

    #[test]
    fn clone_namespace_rules_are_copy_namespaces() {
        let set = initial();
        // No namespace flag: the same set.
        let same = for_clone(&set, 0x100, false).unwrap();
        assert!(Arc::ptr_eq(&same, &set));
        assert_eq!(
            for_clone(&set, CLONE_NEWUTS, false).err(),
            Some(CloneNsError::Permission)
        );
        assert_eq!(
            for_clone(&set, CLONE_NEWIPC | 0x0004_0000, true).err(),
            Some(CloneNsError::Invalid)
        );
        // EPERM comes before a type that does not exist yet.
        assert_eq!(
            for_clone(&set, CLONE_NEWPID, false).err(),
            Some(CloneNsError::Permission)
        );
        assert_eq!(
            for_clone(&set, CLONE_NEWPID, true).err(),
            Some(CloneNsError::Invalid)
        );
        let new = for_clone(&set, CLONE_NEWUTS, true).unwrap();
        assert!(!new.get(NsKind::Uts).same(&set.get(NsKind::Uts)));
    }

    #[test]
    fn setns_replaces_one_namespace() {
        let set = initial();
        let other = set.copy_with(CLONE_NEWUTS).unwrap();
        let joined = set.with(&other.get(NsKind::Uts));
        assert!(joined.get(NsKind::Uts).same(&other.get(NsKind::Uts)));
        assert!(joined.get(NsKind::Ipc).same(&set.get(NsKind::Ipc)));
    }
}

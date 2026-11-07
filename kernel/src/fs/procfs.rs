//! Process Filesystem (/proc)
//!
//! Provides information about running processes and system state.
//! Populated with real data from CPUID, frame allocator, timer, and
//! process table.

#![allow(clippy::useless_format)]

use alloc::{format, string::String, sync::Arc, vec::Vec};

use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// ProcFS node types
enum ProcNodeType {
    Root,
    ProcessDir(u64),
    ProcessFile(u64, String),
    SystemFile(String),
    /// Virtual subdirectory (e.g., /proc/sys, /proc/sys/kernel, /proc/self)
    SubDir(String),
    /// /proc/self/exe: symlink to the calling process's executable
    SelfExe,
    /// /proc/<pid>/ns (None: /proc/self/ns, the caller's).
    NsDir(Option<u64>),
    /// One of its links, by name (`NS_LINKS`).
    NsLink(Option<u64>, &'static str),
    /// A plain symlink in /proc, by name and target: `mounts`, which is
    /// `self/mounts` as on Linux.
    Link(&'static str, &'static str),
}

/// The links in `/proc/<pid>/ns`, as Linux lists them.
const NS_LINKS: [&str; 10] = [
    "cgroup",
    "ipc",
    "mnt",
    "net",
    "pid",
    "pid_for_children",
    "time",
    "time_for_children",
    "user",
    "uts",
];

/// The namespace the `/proc/<pid>/ns/<name>` link names (None: the
/// caller's): the process's first thread's set, its PID and user
/// namespaces, and the time namespace it runs in for "time". Reading or
/// following one needs ptrace read access to the process (EACCES), as
/// Linux's proc_ns_get_link.
#[cfg(feature = "alloc")]
/// The time namespace for children of process `pid`'s first thread, which
/// `/proc/<pid>/timens_offsets` is about (Linux's timens_for_children_get).
fn time_for_children(pid: u64) -> Option<alloc::sync::Arc<crate::process::ns::TimeNs>> {
    let process = crate::process::get_process(crate::process::ProcessId(pid))?;
    let thread = process
        .get_thread(crate::process::ThreadId(pid))
        .or_else(|| process.threads.lock().values().next().cloned())?;
    Some(thread.namespaces().time_for_children.clone())
}

/// What `/proc/<pid>/mounts` (or with `info` `mountinfo`) reads: the mounts
/// of the mount namespace of process `pid`'s first thread under that
/// thread's root, as Linux's mounts_open_common takes the task's
/// namespace and root.
#[cfg(feature = "alloc")]
fn mounts_text(pid: u64, info: bool) -> Option<String> {
    let process = crate::process::get_process(crate::process::ProcessId(pid))?;
    let thread = process
        .get_thread(crate::process::ThreadId(pid))
        .or_else(|| process.threads.lock().values().next().cloned())?;
    let root = thread.fs().root.lock().clone();
    let mounts = thread.namespaces().mnt.mounts.clone();
    Some(if info {
        mounts.render_mountinfo(&root)
    } else {
        mounts.render_mounts(&root)
    })
}

/// One offset line, as Linux's show_offset: "monotonic  <sec> <nsec>".
fn offset_line(clock: &str, ns: i64) -> String {
    format!(
        "{:<10} {:>10} {:>9}\n",
        clock,
        ns.div_euclid(1_000_000_000),
        ns.rem_euclid(1_000_000_000)
    )
}

/// What `/proc/<pid>/timens_offsets` reads: the offsets of the time
/// namespace its children get.
fn timens_offsets_text(pid: u64) -> Option<String> {
    let offsets = time_for_children(pid)?.offsets();
    Some(format!(
        "{}{}",
        offset_line("monotonic", offsets.monotonic),
        offset_line("boottime", offsets.boottime)
    ))
}

/// The most seconds an offset or the clock it moves may reach: Linux's
/// KTIME_SEC_MAX (KTIME_MAX in seconds).
const KTIME_SEC_MAX: i64 = i64::MAX / 1_000_000_000;

/// One "<clock> <sec> <nsec>" line of a timens_offsets write: the clock id
/// (named or by number), and the offset in nanoseconds; None if malformed
/// (sscanf's "%9s %lld %lu" failing, a nanosecond count of a second or
/// more, or an unknown clock).
fn parse_offset_line(line: &str) -> Option<(i32, i64, i64)> {
    use crate::process::ns::{CLOCK_BOOTTIME, CLOCK_MONOTONIC};
    let mut words = line.split_ascii_whitespace();
    let clock = words.next()?;
    let sec: i64 = words.next()?.parse().ok()?;
    let nsec: u64 = words.next()?.parse().ok()?;
    if nsec >= 1_000_000_000 {
        return None;
    }
    let clock = match clock {
        "monotonic" | "1" => CLOCK_MONOTONIC,
        "boottime" | "7" => CLOCK_BOOTTIME,
        _ => return None,
    };
    Some((clock, sec, nsec as i64))
}

/// A write to `/proc/<pid>/timens_offsets`, as Linux's
/// timens_offsets_write and proc_timens_set_offset: at the start of the
/// file and under a page (EINVAL), up to two well-formed lines (EINVAL),
/// the process there (ESRCH), the `opener`'s CAP_SYS_TIME over the user
/// namespace owning the time namespace and the writing process's MODIFY on
/// it (EPERM; ADR 0024, step 4), each
/// offset and the clock it moves within range (ERANGE), and a namespace no
/// process has entered (EACCES).
fn write_timens_offsets(
    pid: u64,
    opener: &crate::process::creds::Caller,
    offset: usize,
    data: &[u8],
) -> Result<usize, KernelError> {
    use crate::process::ns::CLOCK_MONOTONIC;
    let invalid = || KernelError::InvalidArgument {
        name: "timens_offsets",
        value: "malformed offset",
    };
    if offset != 0 || data.len() >= 4096 {
        return Err(invalid());
    }
    let text = core::str::from_utf8(data).map_err(|_| invalid())?;
    if text.is_empty() {
        return Err(invalid());
    }
    // Linux reads only the first two lines and reports what it consumed.
    let mut offsets: Vec<(i32, i64, i64)> = Vec::new();
    let mut consumed = data.len();
    let mut rest = text;
    while !rest.is_empty() {
        let (line, next) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        offsets.push(parse_offset_line(line).ok_or_else(invalid)?);
        rest = next;
        if offsets.len() == 2 {
            if !rest.is_empty() {
                consumed = text.len() - rest.len();
            }
            break;
        }
    }
    let ns = time_for_children(pid).ok_or(KernelError::FsError(FsError::NoProcess))?;
    // file_ns_capable: the opener's CAP_SYS_TIME over the user namespace
    // that owns the time namespace.
    if !opener.ns_capable(&ns.owner, crate::process::linux_caps::CAP_SYS_TIME) {
        return Err(KernelError::FsError(FsError::OperationNotPermitted));
    }
    // And the writing process's MODIFY on the time namespace, itself or
    // through a user namespace above it (ADR 0024, step 4).
    if !crate::cap::authority::current_holds_namespace(
        &crate::process::ns::NsRef::Time(ns.clone()),
        crate::cap::Rights::MODIFY,
    ) {
        return Err(KernelError::FsError(FsError::OperationNotPermitted));
    }
    // There is no suspend: the boot-time clock reads the monotonic one.
    let now = crate::timer::monotonic_ns().min(i64::MAX as u64) as i64;
    let mut monotonic = None;
    let mut boottime = None;
    for &(clock, sec, nsec) in &offsets {
        if !(-KTIME_SEC_MAX..=KTIME_SEC_MAX).contains(&sec) {
            return Err(KernelError::FsError(FsError::OutOfRange));
        }
        let moved = (now / 1_000_000_000).saturating_add(sec)
            + (now % 1_000_000_000 + nsec) / 1_000_000_000;
        if !(0..=KTIME_SEC_MAX / 2).contains(&moved) {
            return Err(KernelError::FsError(FsError::OutOfRange));
        }
        let ns_value = sec * 1_000_000_000 + nsec;
        if clock == CLOCK_MONOTONIC {
            monotonic = Some(ns_value);
        } else {
            boottime = Some(ns_value);
        }
    }
    ns.set_offsets(monotonic, boottime)
        .map_err(|_| KernelError::FsError(FsError::PermissionDenied))?;
    Ok(consumed)
}

fn ns_link_target(pid: Option<u64>, name: &str) -> Result<crate::process::ns::NsRef, KernelError> {
    use crate::process::ns::{self, NsKind, NsRef};
    let not_found = || KernelError::FsError(FsError::NotFound);
    let process = match pid {
        Some(pid) => crate::process::get_process(crate::process::ProcessId(pid)),
        None => crate::process::current_process(),
    }
    .ok_or_else(not_found)?;
    if !crate::syscall::may_read_process(&process) {
        return Err(KernelError::FsError(FsError::PermissionDenied));
    }
    let set = process
        .get_thread(crate::process::ThreadId(process.pid.0))
        .ok_or_else(not_found)?
        .namespaces();
    Ok(match name {
        "pid" => ns::pid_of(&process),
        "user" => ns::user_of(&process),
        // None until the namespace has its init (pidns_for_children_get).
        "pid_for_children" if set.pid_for_children.reaper().is_none() => return Err(not_found()),
        "pid_for_children" => set.get(NsKind::Pid),
        "time" => NsRef::Time(set.time.clone()),
        "time_for_children" => set.get(NsKind::Time),
        "cgroup" => set.get(NsKind::Cgroup),
        "ipc" => set.get(NsKind::Ipc),
        "mnt" => set.get(NsKind::Mnt),
        "net" => set.get(NsKind::Net),
        "uts" => set.get(NsKind::Uts),
        _ => return Err(not_found()),
    })
}

/// The pid part of an ns node's key.
fn ns_owner_key(pid: Option<u64>) -> String {
    pid.map_or(String::from("self"), |p| format!("{}", p))
}

/// ProcFS node
struct ProcNode {
    node_type: ProcNodeType,
    /// The PID namespace of the mount it is in (Linux's proc_pid_ns): its
    /// process directories, `self` and the pids in its files are as that
    /// namespace numbers them.
    ns: Arc<crate::process::pidns::PidNs>,
}

/// A writable sysctl.
#[derive(Clone, Copy)]
enum Sysctl {
    /// `fs/inotify/*`.
    Inotify(crate::fs::inotify::Limit),
    /// `kernel/unprivileged_userns_clone` (ADR 0022 H4a): whether a user
    /// without CAP_SYS_ADMIN may create a user namespace, 0 (the default)
    /// or 1, as Debian's sysctl of that name.
    UnprivilegedUsernsClone,
    /// `user/max_user_namespaces`: how many user namespaces may exist
    /// below the initial one.
    MaxUserNamespaces,
    /// `user/max_pid_namespaces` (ADR 0022 H5), likewise.
    MaxPidNamespaces,
    /// `user/max_net_namespaces` (ADR 0022 H6), likewise.
    MaxNetNamespaces,
}

impl Sysctl {
    /// The sysctl the system file `name` (`sys/...`) is.
    fn from_name(name: &str) -> Option<Self> {
        if let Some(limit) = name
            .strip_prefix("sys/fs/inotify/")
            .and_then(crate::fs::inotify::Limit::from_name)
        {
            return Some(Self::Inotify(limit));
        }
        match name {
            "sys/kernel/unprivileged_userns_clone" => Some(Self::UnprivilegedUsernsClone),
            "sys/user/max_user_namespaces" => Some(Self::MaxUserNamespaces),
            "sys/user/max_pid_namespaces" => Some(Self::MaxPidNamespaces),
            "sys/user/max_net_namespaces" => Some(Self::MaxNetNamespaces),
            _ => None,
        }
    }

    fn get(self) -> usize {
        use core::sync::atomic::Ordering;

        use crate::process::userns;
        match self {
            Self::Inotify(limit) => limit.get(),
            Self::UnprivilegedUsernsClone => {
                userns::UNPRIVILEGED_CLONE.load(Ordering::Acquire) as usize
            }
            Self::MaxUserNamespaces => userns::USER_NAMESPACES.max.load(Ordering::Acquire) as usize,
            Self::MaxPidNamespaces => crate::process::pidns::PID_NAMESPACES
                .max
                .load(Ordering::Acquire) as usize,
            Self::MaxNetNamespaces => crate::process::ns::NET_NAMESPACES
                .max
                .load(Ordering::Acquire) as usize,
        }
    }

    /// Set it from what was written to its file: a decimal integer in its
    /// range, surrounding white space allowed (EINVAL otherwise). The
    /// switch also needs CAP_SYS_ADMIN (EPERM), so root of a user namespace
    /// cannot turn it on.
    fn write(self, text: &[u8]) -> Result<(), KernelError> {
        use core::sync::atomic::Ordering;

        use crate::process::userns;
        let (store, max) = match self {
            Self::Inotify(limit) => return limit.write(text),
            Self::UnprivilegedUsernsClone => (&userns::UNPRIVILEGED_CLONE, 1),
            Self::MaxUserNamespaces => (&userns::USER_NAMESPACES.max, i32::MAX as u32),
            Self::MaxPidNamespaces => (&crate::process::pidns::PID_NAMESPACES.max, i32::MAX as u32),
            Self::MaxNetNamespaces => (&crate::process::ns::NET_NAMESPACES.max, i32::MAX as u32),
        };
        let value = core::str::from_utf8(text)
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
            .filter(|v| *v <= max)
            .ok_or(KernelError::InvalidArgument {
                name: "value",
                value: "out of the sysctl's range",
            })?;
        if matches!(self, Self::UnprivilegedUsernsClone)
            && !crate::process::creds::Caller::current()
                .capable(crate::process::linux_caps::CAP_SYS_ADMIN)
        {
            return Err(KernelError::FsError(FsError::OperationNotPermitted));
        }
        store.store(value, Ordering::Release);
        Ok(())
    }
}

/// The files of `/proc/<pid>` that name its user namespace's ID maps.
const ID_MAP_FILES: [&str; 3] = ["uid_map", "gid_map", "setgroups"];

/// The user namespace of process `pid`.
fn user_ns_of(pid: u64) -> Option<Arc<crate::process::userns::UserNs>> {
    crate::process::get_process(crate::process::ProcessId(pid)).map(|p| p.user_ns())
}

/// A write to `/proc/<pid>/uid_map`, `gid_map` or `setgroups` (ADR 0022
/// H4a) through a descriptor `opener` opened, by `writer`, by the rules of
/// [`UserNs::write_map`] and [`UserNs::write_setgroups`]; setgroups also
/// needs the opener's CAP_SYS_ADMIN over the namespace (EPERM), which
/// Linux's proc_setgroups_open checks. All three also need the writing
/// process to hold MODIFY on the namespace, itself or through a user
/// namespace above it (EPERM; ADR 0024, step 3).
///
/// [`UserNs::write_map`]: crate::process::userns::UserNs::write_map
/// [`UserNs::write_setgroups`]: crate::process::userns::UserNs::write_setgroups
fn write_id_map(
    pid: u64,
    name: &str,
    opener: &crate::process::creds::Caller,
    writer: &crate::process::creds::Caller,
    offset: usize,
    data: &[u8],
) -> Result<usize, KernelError> {
    use crate::process::userns::MapError;
    let ns = user_ns_of(pid).ok_or(KernelError::FsError(FsError::NoProcess))?;
    if !crate::cap::authority::current_holds_namespace(
        &crate::process::ns::NsRef::User(ns.clone()),
        crate::cap::Rights::MODIFY,
    ) {
        return Err(KernelError::FsError(FsError::OperationNotPermitted));
    }
    let done = if name == "setgroups" {
        if !opener.ns_capable(&ns, crate::process::linux_caps::CAP_SYS_ADMIN) {
            return Err(KernelError::FsError(FsError::OperationNotPermitted));
        }
        ns.write_setgroups(offset, data)
    } else {
        ns.write_map(name == "gid_map", opener, writer, offset, data)
    };
    done.map_err(|e| match e {
        MapError::Invalid => KernelError::InvalidArgument {
            name: "id map",
            value: "malformed",
        },
        MapError::Permission => KernelError::FsError(FsError::OperationNotPermitted),
    })
}

/// The owner of `/proc/<pid>` and what is in it (Linux's task_dump_owner):
/// the process's effective user and group while it is dumpable, otherwise
/// the root of the user namespace its memory belongs to, or the initial
/// root where that namespace maps no uid 0 (or gid 0).
fn dump_owner(pid: u64) -> (u32, u32) {
    let Some(p) = crate::process::get_process(crate::process::ProcessId(pid)) else {
        return (0, 0);
    };
    if p.dumpable.load(core::sync::atomic::Ordering::Acquire) {
        let creds = p.credentials();
        return (creds.euid, creds.egid);
    }
    let ns = p.mm_user_ns();
    (ns.uid_down(0).unwrap_or(0), ns.gid_down(0).unwrap_or(0))
}

impl ProcNode {
    /// The writable sysctl this node is, if it is one.
    fn sysctl(&self) -> Option<Sysctl> {
        match &self.node_type {
            ProcNodeType::SystemFile(name) => Sysctl::from_name(name),
            _ => None,
        }
    }

    /// Whether this is a writable file of a process directory.
    fn writable_process_file(&self) -> bool {
        matches!(&self.node_type, ProcNodeType::ProcessFile(_, n)
            if n == "timens_offsets" || ID_MAP_FILES.contains(&n.as_str()))
    }
}

impl ProcNode {
    /// A node in the initial PID namespace's view.
    #[cfg(test)]
    fn of(node_type: ProcNodeType) -> Self {
        Self {
            node_type,
            ns: crate::process::pidns::initial(),
        }
    }

    #[cfg(test)]
    fn new_root() -> Self {
        Self::of(ProcNodeType::Root)
    }

    #[cfg(test)]
    fn new_process_dir(pid: u64) -> Self {
        Self::of(ProcNodeType::ProcessDir(pid))
    }

    #[cfg(test)]
    fn new_process_file(pid: u64, name: String) -> Self {
        Self::of(ProcNodeType::ProcessFile(pid, name))
    }

    #[cfg(test)]
    fn new_system_file(name: String) -> Self {
        Self::of(ProcNodeType::SystemFile(name))
    }

    /// A node below this one, in the same mount's view.
    fn child(&self, node_type: ProcNodeType) -> Self {
        Self {
            node_type,
            ns: self.ns.clone(),
        }
    }

    fn process_dir(&self, pid: u64) -> Self {
        self.child(ProcNodeType::ProcessDir(pid))
    }

    fn process_file(&self, pid: u64, name: String) -> Self {
        self.child(ProcNodeType::ProcessFile(pid, name))
    }

    fn system_file(&self, name: String) -> Self {
        self.child(ProcNodeType::SystemFile(name))
    }

    /// Kernel id `kid` as this mount's PID namespace numbers it (0: not
    /// there).
    fn nr(&self, kid: u64) -> u64 {
        crate::process::pidns::nr_in(kid, &self.ns)
    }

    /// The calling process, if this mount's namespace numbers it (Linux's
    /// /proc/self, which names nothing for a caller above it).
    fn me(&self) -> Option<Arc<crate::process::Process>> {
        crate::process::current_process().filter(|p| self.nr(p.pid.0) != 0)
    }

    /// The node's path below /proc, which names it uniquely: "" for the
    /// root, "sys/kernel" or "123" for a directory, "123/status" for a
    /// process file. A system file's key is its internal name, unique too.
    fn key(&self) -> String {
        match &self.node_type {
            ProcNodeType::Root => String::new(),
            ProcNodeType::ProcessDir(pid) => format!("{}", pid),
            ProcNodeType::ProcessFile(pid, name) => format!("{}/{}", pid, name),
            ProcNodeType::SystemFile(name) => format!("file:{}", name),
            ProcNodeType::SubDir(dir) => dir.clone(),
            ProcNodeType::SelfExe => String::from("self/exe"),
            ProcNodeType::NsDir(pid) => format!("{}/ns", ns_owner_key(*pid)),
            ProcNodeType::NsLink(pid, name) => format!("{}/ns/{}", ns_owner_key(*pid), name),
            ProcNodeType::Link(name, _) => String::from(*name),
        }
    }

    /// One stable inode number per node, used by both stat and readdir:
    /// 1 for the root (Linux's PROC_ROOT_INO), otherwise a 64-bit FNV-1a
    /// hash of the node's key, kept above 1.
    fn inode(&self) -> u64 {
        if let ProcNodeType::Root = self.node_type {
            return 1;
        }
        let hash = self.key().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
        });
        hash.max(2)
    }

    /// The inode of the directory holding this one.
    fn parent_inode(&self) -> u64 {
        match &self.node_type {
            ProcNodeType::SubDir(dir) => match dir.rsplit_once('/') {
                Some((parent, _)) => self
                    .child(ProcNodeType::SubDir(String::from(parent)))
                    .inode(),
                None => 1,
            },
            // The root's parent is outside procfs; Linux reports the mount
            // point's, which procfs cannot see.
            _ => 1,
        }
    }
}

/// The process a `/proc/<pid>` directory names, which pidfd_send_signal
/// takes in place of a pidfd (Linux's tgid_pidfd_to_pid). `/proc/self`
/// names the caller: that node is not bound to the process that opened it.
pub fn process_dir_pid(node: &dyn VfsNode) -> Option<u64> {
    let node = node.as_any()?.downcast_ref::<ProcNode>()?;
    match &node.node_type {
        ProcNodeType::ProcessDir(pid) => Some(*pid),
        ProcNodeType::SubDir(dir) if dir == "self" => {
            crate::process::current_process().map(|p| p.pid.0)
        }
        _ => None,
    }
}

impl VfsNode for ProcNode {
    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    /// procfs: `simple_statfs`, no counts.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, crate::error::KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::PROC_SUPER_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        match &self.node_type {
            ProcNodeType::Root
            | ProcNodeType::ProcessDir(_)
            | ProcNodeType::SubDir(_)
            | ProcNodeType::NsDir(_) => NodeType::Directory,
            ProcNodeType::ProcessFile(_, _) | ProcNodeType::SystemFile(_) => NodeType::File,
            ProcNodeType::SelfExe | ProcNodeType::NsLink(..) | ProcNodeType::Link(..) => {
                NodeType::Symlink
            }
        }
    }

    fn readlink(&self) -> Result<String, KernelError> {
        match &self.node_type {
            // It used to be an empty regular file: exec("/proc/self/exe"),
            // which BusyBox uses to run its own applets, read zero bytes.
            ProcNodeType::SelfExe => crate::process::current_process()
                .map(|p| p.exe_path.lock().clone())
                .filter(|path| !path.is_empty())
                .ok_or(KernelError::FsError(FsError::NotFound)),
            // "uts:[4026531838]"
            #[cfg(feature = "alloc")]
            ProcNodeType::NsLink(pid, name) => {
                ns_link_target(*pid, name).map(|ns| ns.ident().link_text())
            }
            ProcNodeType::Link(_, target) => Ok(String::from(*target)),
            _ => Err(KernelError::FsError(FsError::NotASymlink)),
        }
    }

    /// /proc/self/exe names the executable, not a path; an ns link a
    /// namespace.
    fn is_magic_link(&self) -> bool {
        matches!(
            self.node_type,
            ProcNodeType::SelfExe | ProcNodeType::NsLink(..)
        )
    }

    /// An ns link opens the namespace's nsfs file.
    #[cfg(feature = "alloc")]
    fn magic_target(&self) -> Option<Result<Arc<dyn VfsNode>, KernelError>> {
        match &self.node_type {
            ProcNodeType::NsLink(pid, name) => Some(
                ns_link_target(*pid, name)
                    .map(|ns| Arc::new(crate::fs::nsfs::NsFileNode::new(ns)) as Arc<dyn VfsNode>),
            ),
            _ => None,
        }
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        self.read_as(offset, buffer, None)
    }

    /// The process files show IDs as the opener's user namespace sees them
    /// (seq_user_ns), the reader's when no opener is recorded.
    fn read_as(
        &self,
        offset: usize,
        buffer: &mut [u8],
        opener: Option<&crate::process::creds::Caller>,
    ) -> Result<usize, KernelError> {
        let viewer = opener.map_or_else(crate::process::userns::current, |o| o.userns.clone());
        let content = match &self.node_type {
            ProcNodeType::SystemFile(name) => {
                match name.as_str() {
                    // The system's identity (ADR 0021).
                    // The system name is the reader's (Linux uname mode).
                    "version" => {
                        crate::utils::uname::proc_version(crate::utils::uname::current_sysname())
                    }
                    "ostype" => format!("{}\n", crate::utils::uname::current_sysname()),
                    "osrelease" => format!("{}\n", crate::utils::uname::RELEASE),
                    "sys/kernel/version" => format!("{}\n", crate::utils::uname::VERSION),
                    "uptime" => {
                        // Boot time in the reader's time namespace (Linux's
                        // uptime_proc_show), then the idle time.
                        let up = crate::process::ns::clock_from_host(
                            crate::process::ns::CLOCK_BOOTTIME,
                            crate::timer::monotonic_ns(),
                        );
                        format!(
                            "{}.{:02} 0.00\n",
                            up / 1_000_000_000,
                            up % 1_000_000_000 / 10_000_000
                        )
                    }
                    "meminfo" => {
                        let stats = crate::mm::get_memory_stats();
                        let total_kb = stats.total_frames * 4; // 4KB per frame
                        let free_kb = stats.free_frames * 4;
                        let used_kb = total_kb.saturating_sub(free_kb);
                        // Cached: the file page cache (ADR 0010). Frames on
                        // the allocator's per-CPU lists are free memory too.
                        let cached_kb = crate::mm::page_cache::cached_pages() * 4;
                        let available_kb = free_kb + stats.cached_frames * 4;
                        let buffers_kb = 0usize;
                        let slab_kb = 0usize;

                        format!(
                            "MemTotal:       {:>8} kB\n\
                             MemFree:        {:>8} kB\n\
                             MemAvailable:   {:>8} kB\n\
                             Buffers:        {:>8} kB\n\
                             Cached:         {:>8} kB\n\
                             Slab:           {:>8} kB\n\
                             MemUsed:        {:>8} kB\n\
                             CowShared:      {:>8} kB\n",
                            total_kb,
                            free_kb,
                            available_kb,
                            buffers_kb,
                            cached_kb,
                            slab_kb,
                            used_kb,
                            // Frames with more than one owner (each counted
                            // once): shared copy-on-write after fork, or a
                            // cached file page that processes map.
                            crate::mm::frame_refs::shared_frames() * 4,
                        )
                    }
                    "cpuinfo" => generate_cpuinfo(),
                    "loadavg" => generate_loadavg(),
                    // The types mount(2) makes, none on a block device
                    // (Linux's filesystems_proc_show).
                    "filesystems" => crate::fs::mount::MOUNTABLE
                        .iter()
                        .map(|ty| format!("nodev\t{ty}\n"))
                        .collect(),
                    // Qt/KDE compatibility files
                    "core_pattern" => String::from("core\n"),
                    "pid_max" => format!("{}\n", crate::process::PID_MAX),
                    // No taint: no proprietary modules, no oops (LTP's
                    // tst_taint reads it).
                    "tainted" => String::from("0\n"),
                    "hostname" | "domainname" => {
                        let uts = crate::process::current_thread()
                            .map(|t| t.namespaces().uts.clone())
                            .unwrap_or_else(|| crate::process::ns::initial().uts.clone());
                        let name = if name == "hostname" {
                            uts.hostname()
                        } else {
                            uts.domainname()
                        };
                        format!("{}\n", name)
                    }
                    "cap_last_cap" => {
                        format!("{}\n", crate::process::linux_caps::CAP_LAST_CAP)
                    }
                    "boot_id" => String::from("00000000-0000-0000-0000-000000000001\n"),
                    // The reader's network namespace's interfaces.
                    "net/dev" => crate::net::device::proc_net_dev(
                        crate::process::ns::current_net() == crate::process::ns::INITIAL_NET,
                    ),
                    name if Sysctl::from_name(name).is_some() => {
                        format!("{}\n", Sysctl::from_name(name).map_or(0, |l| l.get()))
                    }
                    "self/maps" => crate::process::current_process()
                        .map(|p| generate_maps(p.pid.0))
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            }
            ProcNodeType::ProcessFile(pid, name) => {
                match name.as_str() {
                    "status" => {
                        // Get actual process information
                        if let Some(process) =
                            crate::process::get_process(crate::process::ProcessId(*pid))
                        {
                            let state = match process.get_state() {
                                crate::process::ProcessState::Creating => "N (new)",
                                crate::process::ProcessState::Ready => "R (running)",
                                crate::process::ProcessState::Running => "R (running)",
                                crate::process::ProcessState::Blocked => "S (sleeping)",
                                crate::process::ProcessState::Sleeping => "S (sleeping)",
                                crate::process::ProcessState::Zombie => "Z (zombie)",
                                crate::process::ProcessState::Dead => "X (dead)",
                            };

                            #[cfg(feature = "alloc")]
                            let name = &process.name;
                            #[cfg(not(feature = "alloc"))]
                            let name = "process";

                            // As this mount's namespace numbers them (0 for a
                            // parent above it).
                            let parent = process.parent().map_or(0, |p| self.nr(p.0));

                            // The sets are per thread; the process's are its
                            // main thread's, as on Linux.
                            let caps = process
                                .get_main_thread_id()
                                .and_then(|tid| process.get_thread(tid))
                                .map(|t| status_caps(&t.caps.lock()))
                                .unwrap_or_default();

                            format!(
                                "Name:\t{}\nPid:\t{}\nPPid:\t{}\nState:\t{}\n{}{}{}",
                                name,
                                self.nr(*pid),
                                parent,
                                state,
                                status_ids(&process.credentials(), &viewer),
                                status_nspids(&process, &self.ns),
                                caps
                            )
                        } else {
                            format!("Name:\tProcess\nPid:\t{}\nState:\tR (running)\n", pid)
                        }
                    }
                    "timens_offsets" => timens_offsets_text(*pid).unwrap_or_default(),
                    // In the opener's terms (proc_id_map_open's seq_ns).
                    "uid_map" | "gid_map" => user_ns_of(*pid)
                        .map(|ns| ns.map_text(name == "gid_map", &viewer))
                        .unwrap_or_default(),
                    "setgroups" => user_ns_of(*pid)
                        .map(|ns| String::from(ns.setgroups_text()))
                        .unwrap_or_default(),
                    #[cfg(feature = "alloc")]
                    "mounts" => mounts_text(*pid, false).unwrap_or_default(),
                    #[cfg(feature = "alloc")]
                    "mountinfo" => mounts_text(*pid, true).unwrap_or_default(),
                    "cmdline" => {
                        // Get actual command line
                        if let Some(process) =
                            crate::process::get_process(crate::process::ProcessId(*pid))
                        {
                            #[cfg(feature = "alloc")]
                            let name = &process.name;
                            #[cfg(not(feature = "alloc"))]
                            let name = "process";
                            format!("{}\0", name)
                        } else {
                            format!("init\0")
                        }
                    }
                    _ => String::new(),
                }
            }
            _ => return Err(KernelError::FsError(FsError::NotAFile)),
        };

        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return Ok(0);
        }

        let bytes_to_read = core::cmp::min(buffer.len(), bytes.len() - offset);
        buffer[..bytes_to_read].copy_from_slice(&bytes[offset..offset + bytes_to_read]);
        Ok(bytes_to_read)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        self.write_as(offset, data, None)
    }

    /// The process files check the descriptor's opener where Linux does
    /// (file_ns_capable, f_cred), the writer when no opener is recorded.
    fn write_as(
        &self,
        offset: usize,
        data: &[u8],
        opener: Option<&crate::process::creds::Caller>,
    ) -> Result<usize, KernelError> {
        if let ProcNodeType::ProcessFile(pid, name) = &self.node_type {
            let writer = crate::process::creds::Caller::current();
            let opener = opener.cloned().unwrap_or_else(|| writer.clone());
            if name == "timens_offsets" {
                return write_timens_offsets(*pid, &opener, offset, data);
            }
            if ID_MAP_FILES.contains(&name.as_str()) {
                return write_id_map(*pid, name, &opener, &writer, offset, data);
            }
        }
        // The inotify limits are sysctls root may set (the mode decides who
        // can open them for writing); the rest is EACCES, as Linux's
        // read-only /proc files give (EROFS would say the filesystem is
        // mounted read-only).
        let limit = self
            .sysctl()
            .ok_or(KernelError::FsError(FsError::PermissionDenied))?;
        limit.write(data)?;
        Ok(data.len())
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        let node_type = match &self.node_type {
            ProcNodeType::Root
            | ProcNodeType::ProcessDir(_)
            | ProcNodeType::SubDir(_)
            | ProcNodeType::NsDir(_) => NodeType::Directory,
            ProcNodeType::SelfExe | ProcNodeType::NsLink(..) | ProcNodeType::Link(..) => {
                NodeType::Symlink
            }
            _ => NodeType::File,
        };

        // As on Linux: everyone may walk the directories and read the
        // files; links (self/exe) are rwxrwxrwx.
        let permissions = Permissions::from_mode(match node_type {
            // dr-x--x--x, as Linux's ns directory.
            NodeType::Directory if matches!(self.node_type, ProcNodeType::NsDir(_)) => 0o511,
            NodeType::Directory => 0o555,
            NodeType::Symlink => 0o777,
            _ if self.sysctl().is_some() => 0o644,
            // rw-r--r--, as Linux's.
            _ if self.writable_process_file() => 0o644,
            _ => 0o444,
        });
        let owner = match &self.node_type {
            ProcNodeType::ProcessDir(pid) | ProcNodeType::ProcessFile(pid, _) => Some(*pid),
            ProcNodeType::NsDir(pid) | ProcNodeType::NsLink(pid, _) => {
                pid.or_else(|| crate::process::current_process().map(|p| p.pid.0))
            }
            _ => None,
        };
        let (uid, gid) = owner.map_or((0, 0), dump_owner);
        Ok(Metadata {
            node_type,
            size: 0,
            permissions,
            uid,
            gid,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: self.inode(),
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        let mut entries = Vec::new();

        entries.push(DirEntry {
            name: String::from("."),
            node_type: NodeType::Directory,
            inode: self.inode(),
        });

        entries.push(DirEntry {
            name: String::from(".."),
            node_type: NodeType::Directory,
            inode: self.parent_inode(),
        });

        match &self.node_type {
            ProcNodeType::Root => {
                // System files
                entries.push(DirEntry {
                    name: String::from("version"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("uptime"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("meminfo"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("cpuinfo"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("loadavg"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("filesystems"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("mounts"),
                    node_type: NodeType::Symlink,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("net"),
                    node_type: NodeType::Directory,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("sys"),
                    node_type: NodeType::Directory,
                    inode: 0,
                });

                // A directory for each process this mount's PID namespace
                // numbers, by that number.
                if let Some(process_list) = crate::process::get_process_list() {
                    for pid in process_list {
                        let nr = self.nr(pid);
                        if nr == 0 {
                            continue;
                        }
                        entries.push(DirEntry {
                            name: format!("{}", nr),
                            node_type: NodeType::Directory,
                            inode: self.process_dir(pid).inode(),
                        });
                    }
                } else {
                    // Fallback: just show init process
                    entries.push(DirEntry {
                        name: String::from("1"),
                        node_type: NodeType::Directory,
                        inode: self.process_dir(1).inode(),
                    });
                }
            }
            ProcNodeType::ProcessDir(_pid) => {
                entries.push(DirEntry {
                    name: String::from("status"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("cmdline"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("ns"),
                    node_type: NodeType::Directory,
                    inode: 0,
                });

                entries.push(DirEntry {
                    name: String::from("timens_offsets"),
                    node_type: NodeType::File,
                    inode: 0,
                });

                for name in ["mounts", "mountinfo"].into_iter().chain(ID_MAP_FILES) {
                    entries.push(DirEntry {
                        name: String::from(name),
                        node_type: NodeType::File,
                        inode: 0,
                    });
                }
            }
            ProcNodeType::NsDir(_) => {
                for name in NS_LINKS {
                    entries.push(DirEntry {
                        name: String::from(name),
                        node_type: NodeType::Symlink,
                        inode: 0,
                    });
                }
            }
            // What lookup finds in /proc/sys and below, and /proc/net.
            ProcNodeType::SubDir(dir) if dir.starts_with("sys") || dir == "net" => {
                let listed: &[(&str, NodeType)] = match dir.as_str() {
                    "net" => &[("dev", NodeType::File)],
                    "sys" => &[
                        ("fs", NodeType::Directory),
                        ("kernel", NodeType::Directory),
                        ("user", NodeType::Directory),
                    ],
                    "sys/fs" => &[("inotify", NodeType::Directory)],
                    "sys/fs/inotify" => &[
                        ("max_queued_events", NodeType::File),
                        ("max_user_instances", NodeType::File),
                        ("max_user_watches", NodeType::File),
                    ],
                    "sys/kernel" => &[
                        ("cap_last_cap", NodeType::File),
                        ("core_pattern", NodeType::File),
                        ("domainname", NodeType::File),
                        ("hostname", NodeType::File),
                        ("osrelease", NodeType::File),
                        ("ostype", NodeType::File),
                        ("pid_max", NodeType::File),
                        ("random", NodeType::Directory),
                        ("tainted", NodeType::File),
                        ("unprivileged_userns_clone", NodeType::File),
                        ("version", NodeType::File),
                    ],
                    "sys/kernel/random" => &[("boot_id", NodeType::File)],
                    "sys/user" => &[
                        ("max_net_namespaces", NodeType::File),
                        ("max_pid_namespaces", NodeType::File),
                        ("max_user_namespaces", NodeType::File),
                    ],
                    _ => return Err(KernelError::FsError(FsError::NotADirectory)),
                };
                for (name, node_type) in listed {
                    entries.push(DirEntry {
                        name: String::from(*name),
                        node_type: *node_type,
                        inode: 0,
                    });
                }
            }
            _ => return Err(KernelError::FsError(FsError::NotADirectory)),
        }

        // The rest are numbered as the node lookup returns, so a listing
        // and a stat of the same name agree. Every listed name is found;
        // should one not be, it still gets a stable nonzero number.
        for entry in entries.iter_mut().filter(|e| e.inode == 0) {
            entry.inode = self
                .lookup(&entry.name)
                .and_then(|node| node.metadata())
                .map(|m| m.inode)
                .unwrap_or_else(|_| {
                    self.child(ProcNodeType::SubDir(format!(
                        "{}/{}",
                        self.key(),
                        entry.name
                    )))
                    .inode()
                });
        }
        Ok(entries)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        match &self.node_type {
            ProcNodeType::Root => {
                // Check for system files
                match name {
                    "version" | "uptime" | "meminfo" | "cpuinfo" | "loadavg" | "filesystems" => {
                        Ok(Arc::new(self.system_file(String::from(name))) as Arc<dyn VfsNode>)
                    }
                    "mounts" => Ok(
                        Arc::new(self.child(ProcNodeType::Link("mounts", "self/mounts")))
                            as Arc<dyn VfsNode>,
                    ),
                    // The reader's network namespace's files (Linux's
                    // /proc/net, a link to self/net, here a directory).
                    "net" => Ok(
                        Arc::new(self.child(ProcNodeType::SubDir(String::from("net"))))
                            as Arc<dyn VfsNode>,
                    ),
                    // Virtual subdirectories for Qt/KDE compatibility
                    "sys" => Ok(
                        Arc::new(self.child(ProcNodeType::SubDir(String::from("sys"))))
                            as Arc<dyn VfsNode>,
                    ),
                    // Nothing for a caller this mount's namespace does not
                    // number.
                    "self" if self.me().is_some() => Ok(Arc::new(
                        self.child(ProcNodeType::SubDir(String::from("self"))),
                    ) as Arc<dyn VfsNode>),
                    _ => {
                        // A pid as this mount's PID namespace numbers it.
                        if let Some(pid) = name.parse::<u64>().ok().and_then(|nr| self.ns.find(nr))
                        {
                            // Validate PID exists in process table
                            if crate::process::get_process(crate::process::ProcessId(pid)).is_some()
                            {
                                Ok(Arc::new(self.process_dir(pid)) as Arc<dyn VfsNode>)
                            } else {
                                Err(KernelError::FsError(FsError::NotFound))
                            }
                        } else {
                            Err(KernelError::FsError(FsError::NotFound))
                        }
                    }
                }
            }
            ProcNodeType::ProcessDir(pid) => match name {
                "status" | "cmdline" | "timens_offsets" | "mounts" | "mountinfo" | "uid_map"
                | "gid_map" | "setgroups" => {
                    Ok(Arc::new(self.process_file(*pid, String::from(name))) as Arc<dyn VfsNode>)
                }
                "ns" => {
                    Ok(Arc::new(self.child(ProcNodeType::NsDir(Some(*pid)))) as Arc<dyn VfsNode>)
                }
                _ => Err(KernelError::FsError(FsError::NotFound)),
            },
            ProcNodeType::SubDir(dir_name) => {
                // Handle /proc/sys/*, /proc/self/*
                match dir_name.as_str() {
                    "net" => {
                        match name {
                            "dev" => Ok(Arc::new(self.system_file(String::from("net/dev")))
                                as Arc<dyn VfsNode>),
                            _ => Err(KernelError::FsError(FsError::NotFound)),
                        }
                    }
                    "sys" => match name {
                        "fs" | "kernel" | "user" => Ok(Arc::new(
                            self.child(ProcNodeType::SubDir(format!("sys/{}", name))),
                        )
                            as Arc<dyn VfsNode>),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/fs" => match name {
                        "inotify" => Ok(Arc::new(
                            self.child(ProcNodeType::SubDir(String::from("sys/fs/inotify"))),
                        ) as Arc<dyn VfsNode>),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/fs/inotify" => match name {
                        "max_queued_events" | "max_user_instances" | "max_user_watches" => Ok(
                            Arc::new(self.system_file(format!("sys/fs/inotify/{}", name)))
                                as Arc<dyn VfsNode>,
                        ),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/kernel" => match name {
                        "core_pattern" => {
                            Ok(Arc::new(self.system_file(String::from("core_pattern")))
                                as Arc<dyn VfsNode>)
                        }
                        "cap_last_cap" | "pid_max" | "ostype" | "osrelease" | "tainted"
                        | "hostname" | "domainname" => {
                            Ok(Arc::new(self.system_file(String::from(name))) as Arc<dyn VfsNode>)
                        }
                        // Not /proc/version, which has the same file name.
                        "version" => Ok(Arc::new(
                            self.system_file(String::from("sys/kernel/version")),
                        ) as Arc<dyn VfsNode>),
                        "random" => Ok(Arc::new(
                            self.child(ProcNodeType::SubDir(String::from("sys/kernel/random"))),
                        ) as Arc<dyn VfsNode>),
                        "unprivileged_userns_clone" => Ok(Arc::new(
                            self.system_file(String::from("sys/kernel/unprivileged_userns_clone")),
                        )
                            as Arc<dyn VfsNode>),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/user" => match name {
                        "max_user_namespaces" | "max_pid_namespaces" | "max_net_namespaces" => {
                            Ok(Arc::new(self.system_file(format!("sys/user/{}", name)))
                                as Arc<dyn VfsNode>)
                        }
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/kernel/random" => {
                        match name {
                            "boot_id" => Ok(Arc::new(self.system_file(String::from("boot_id")))
                                as Arc<dyn VfsNode>),
                            _ => Err(KernelError::FsError(FsError::NotFound)),
                        }
                    }
                    "self" => match name {
                        "exe" => {
                            Ok(Arc::new(self.child(ProcNodeType::SelfExe)) as Arc<dyn VfsNode>)
                        }
                        "ns" => {
                            Ok(Arc::new(self.child(ProcNodeType::NsDir(None))) as Arc<dyn VfsNode>)
                        }
                        // The caller's own files, as /proc/<its pid>/...
                        // (Linux's /proc/self is a link to that directory).
                        "status" | "cmdline" | "timens_offsets" | "mounts" | "mountinfo"
                        | "uid_map" | "gid_map" | "setgroups" => self
                            .me()
                            .map(|p| {
                                Arc::new(self.process_file(p.pid.0, String::from(name)))
                                    as Arc<dyn VfsNode>
                            })
                            .ok_or(KernelError::FsError(FsError::NotFound)),
                        "maps" => Ok(Arc::new(self.system_file(format!("self/{}", name)))
                            as Arc<dyn VfsNode>),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    _ => Err(KernelError::FsError(FsError::NotFound)),
                }
            }
            ProcNodeType::NsDir(pid) => NS_LINKS
                .iter()
                .find(|&&link| link == name)
                .map(|&link| {
                    Arc::new(self.child(ProcNodeType::NsLink(*pid, link))) as Arc<dyn VfsNode>
                })
                .ok_or(KernelError::FsError(FsError::NotFound)),
            _ => Err(KernelError::FsError(FsError::NotADirectory)),
        }
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    /// A sysctl has no size to change: opening one with O_TRUNC (fopen
    /// "w") succeeds, as on Linux.
    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        // The writable files ignore O_TRUNC, as Linux's: fopen("w") on
        // them must succeed.
        match self.sysctl() {
            Some(_) => Ok(()),
            None if self.writable_process_file() => Ok(()),
            None => Err(KernelError::FsError(FsError::PermissionDenied)),
        }
    }
}

/// Format one `/proc/<pid>/maps` line in the Linux layout:
/// `start-end perms offset dev inode [label]`.
///
/// Mappings are anonymous (offset 0, device 00:00, inode 0). Shared
/// mappings (`MAP_SHARED`, IPC shared regions, device memory) are marked `s`
/// and the rest `p`, as Linux does.
fn format_maps_line(
    start: u64,
    size: usize,
    flags: crate::mm::PageFlags,
    mapping_type: crate::mm::vas::MappingType,
) -> String {
    use crate::mm::{vas::MappingType, PageFlags};

    let end = start.saturating_add(size as u64);
    let write = if flags.contains(PageFlags::WRITABLE) {
        'w'
    } else {
        '-'
    };
    let exec = if flags.contains(PageFlags::NO_EXECUTE) {
        '-'
    } else {
        'x'
    };
    // Shared mappings used to be reported as private (review of the v0.26.0
    // stack, PR #7).
    let share = match mapping_type {
        MappingType::Shared | MappingType::SharedRegion | MappingType::Device => 's',
        _ => 'p',
    };
    let label = match mapping_type {
        MappingType::Stack => " [stack]",
        MappingType::Heap => " [heap]",
        _ => "",
    };
    format!(
        "{:08x}-{:08x} r{}{}{} 00000000 00:00 0{}\n",
        start, end, write, exec, share, label
    )
}

/// Generate `/proc/<pid>/maps` from the process's virtual address space.
///
/// Returns an empty string if the process does not exist or its address
/// space lock is contended (the lock is never waited on from procfs).
fn generate_maps(pid: u64) -> String {
    let Some(process) = crate::process::get_process(crate::process::ProcessId(pid)) else {
        return String::new();
    };
    let Some(space) = process.memory_space.try_lock() else {
        return String::new();
    };
    let Some(mappings) = space.mappings_ref().try_lock() else {
        return String::new();
    };
    let mut out = String::new();
    for mapping in mappings.values() {
        out.push_str(&format_maps_line(
            mapping.start.as_u64(),
            mapping.size,
            mapping.flags,
            mapping.mapping_type,
        ));
    }
    out
}

/// Generate /proc/cpuinfo content with real CPUID data on x86_64.
fn generate_cpuinfo() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        let brand = cpuid_brand_string();
        let freq_mhz = crate::arch::timer::hw_ticks_per_second() / 1_000_000;

        format!(
            "processor\t: 0\nvendor_id\t: {}\nmodel name\t: {}\ncpu MHz\t\t: {}\ncache size\t: 0 \
             KB\nbogomips\t: {}\n",
            cpuid_vendor_id(),
            brand,
            freq_mhz,
            freq_mhz * 2,
        )
    }

    #[cfg(target_arch = "aarch64")]
    {
        let freq_mhz = crate::arch::timer::hw_ticks_per_second() / 1_000_000;
        format!(
            "processor\t: 0\narchitecture\t: aarch64\nmodel name\t: ARMv8 Processor\nBogoMIPS\t: \
             {}\nFeatures\t: fp asimd\n",
            freq_mhz * 2,
        )
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        format!(
            "processor\t: 0\narchitecture\t: riscv64\nmodel name\t: RISC-V Processor\nmmu\t\t: \
             {}\n",
            riscv_mmu_name(crate::mm::page_table::paging_levels())
        )
    }
}

/// The `Uid:`, `Gid:` and `Groups:` lines of /proc/[pid]/status, as Linux
/// prints them: real, effective, saved and filesystem IDs, tab-separated,
/// then the supplementary groups separated by spaces, each as user
/// namespace `ns` sees it (the overflow ID if unmapped).
fn status_ids(
    c: &crate::process::creds::Credentials,
    ns: &crate::process::userns::UserNs,
) -> String {
    let mut groups = String::new();
    for (i, &g) in c.groups().iter().enumerate() {
        if i > 0 {
            groups.push(' ');
        }
        groups.push_str(&format!("{}", ns.gid_munged(g)));
    }
    let (u, g) = (|id| ns.uid_munged(id), |id| ns.gid_munged(id));
    format!(
        "Uid:\t{}\t{}\t{}\t{}\nGid:\t{}\t{}\t{}\t{}\nGroups:\t{}\n",
        u(c.ruid),
        u(c.euid),
        u(c.suid),
        u(c.fsuid),
        g(c.rgid),
        g(c.egid),
        g(c.sgid),
        g(c.fsgid),
        groups
    )
}

/// The `NStgid:`, `NSpid:`, `NSpgid:` and `NSsid:` lines of
/// /proc/[pid]/status, as Linux's proc_pid_status: from the mount's PID
/// namespace `ns` down to the process's own, its number at each level, and
/// its group's and session's in each of those namespaces (0 where one has
/// none).
fn status_nspids(
    process: &crate::process::Process,
    ns: &Arc<crate::process::pidns::PidNs>,
) -> String {
    use crate::process::pidns;
    let pid = process.pid.0;
    // Taken once each: the process may be reaped meanwhile.
    let levels = pidns::levels_from(pid, ns);
    let own_nrs = pidns::nrs_from(pid, ns);
    let line = |name: &str, kid: u64, own: bool| {
        let mut text = format!("{}:", name);
        for (i, level) in levels.iter().enumerate() {
            let nr = if own {
                own_nrs.get(i).copied().unwrap_or(0)
            } else {
                pidns::nr_in(kid, level)
            };
            text.push_str(&format!("\t{}", nr));
        }
        text.push('\n');
        text
    };
    let pgid = process.pgid.load(core::sync::atomic::Ordering::Acquire);
    let sid = process.sid.load(core::sync::atomic::Ordering::Acquire);
    format!(
        "{}{}{}{}",
        line("NStgid", pid, true),
        line("NSpid", pid, true),
        line("NSpgid", pgid, false),
        line("NSsid", sid, false)
    )
}

/// The capability lines of /proc/[pid]/status, as Linux prints them (ADR
/// 0020): each set as 16 hexadecimal digits.
fn status_caps(c: &crate::process::linux_caps::CapSets) -> String {
    format!(
        concat!(
            "CapInh:\t{:016x}\n",
            "CapPrm:\t{:016x}\n",
            "CapEff:\t{:016x}\n",
            "CapBnd:\t{:016x}\n",
            "CapAmb:\t{:016x}\n",
        ),
        c.inheritable.bits(),
        c.permitted.bits(),
        c.effective.bits(),
        c.bounding.bits(),
        c.ambient.bits()
    )
}

/// Linux's `mmu` field of a RISC-V /proc/cpuinfo: the paging mode in use
/// (ADR 0018), which user programs read to learn their address-space limit.
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), allow(dead_code))]
fn riscv_mmu_name(levels: u8) -> &'static str {
    if levels == 3 {
        "sv39"
    } else {
        "sv48"
    }
}

/// Generate /proc/loadavg content.
fn generate_loadavg() -> String {
    // Count running/total tasks from process table
    let (running, total) = if let Some(pids) = crate::process::get_process_list() {
        let total = pids.len();
        let mut running = 0usize;
        for pid in &pids {
            if let Some(p) = crate::process::get_process(crate::process::ProcessId(*pid)) {
                match p.get_state() {
                    crate::process::ProcessState::Running | crate::process::ProcessState::Ready => {
                        running += 1;
                    }
                    _ => {}
                }
            }
        }
        (running, total)
    } else {
        (1, 1)
    };

    // The last pid handed out in the reader's PID namespace (Linux's idr
    // cursor), the highest process id in the initial one.
    let last_pid = crate::process::pidns::current()
        .last_nr()
        .unwrap_or_else(|| {
            crate::process::get_process_list()
                .and_then(|pids| pids.iter().copied().max())
                .unwrap_or(1)
        });

    // Linux format: 1min 5min 15min running/total last_pid
    let [one, five, fifteen] =
        crate::sched::loadavg::averages().map(crate::sched::loadavg::format_parts);
    format!(
        "{}.{:02} {}.{:02} {}.{:02} {}/{} {}\n",
        one.0, one.1, five.0, five.1, fifteen.0, fifteen.1, running, total, last_pid
    )
}

/// Read CPUID vendor ID string (x86_64 only).
#[cfg(target_arch = "x86_64")]
fn cpuid_vendor_id() -> String {
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    // SAFETY: CPUID leaf 0 returns vendor string in EBX:EDX:ECX.
    // push/pop RBX required because CPUID clobbers it.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "xor eax, eax",
            "cpuid",
            "mov {0:e}, ebx",
            "mov {1:e}, ecx",
            "mov {2:e}, edx",
            "pop rbx",
            out(reg) ebx,
            out(reg) ecx,
            out(reg) edx,
            out("eax") _,
        );
    }
    let mut buf = [0u8; 12];
    buf[0..4].copy_from_slice(&ebx.to_le_bytes());
    buf[4..8].copy_from_slice(&edx.to_le_bytes());
    buf[8..12].copy_from_slice(&ecx.to_le_bytes());
    String::from_utf8_lossy(&buf).trim_end_matches('\0').into()
}

/// Read CPUID brand string (x86_64 only, leaves 0x80000002-0x80000004).
#[cfg(target_arch = "x86_64")]
fn cpuid_brand_string() -> String {
    // Check if extended CPUID is supported
    let max_ext: u32;
    // SAFETY: CPUID leaf 0x80000000 returns max extended leaf in EAX.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 0x80000000",
            "cpuid",
            "mov {0:e}, eax",
            "pop rbx",
            out(reg) max_ext,
            out("eax") _,
        );
    }

    if max_ext < 0x80000004 {
        return String::from("Unknown CPU");
    }

    let mut buf = [0u8; 48];

    for (i, leaf) in [0x80000002u32, 0x80000003, 0x80000004].iter().enumerate() {
        let eax: u32;
        let ebx: u32;
        let ecx: u32;
        let edx: u32;
        // SAFETY: CPUID leaves 0x80000002-4 return the CPU brand string.
        unsafe {
            core::arch::asm!(
                "push rbx",
                "mov eax, {leaf:e}",
                "cpuid",
                "mov {0:e}, eax",
                "mov {1:e}, ebx",
                "mov {2:e}, ecx",
                "mov {3:e}, edx",
                "pop rbx",
                out(reg) eax,
                out(reg) ebx,
                out(reg) ecx,
                out(reg) edx,
                leaf = in(reg) *leaf,
                out("eax") _,
            );
        }
        let off = i * 16;
        buf[off..off + 4].copy_from_slice(&eax.to_le_bytes());
        buf[off + 4..off + 8].copy_from_slice(&ebx.to_le_bytes());
        buf[off + 8..off + 12].copy_from_slice(&ecx.to_le_bytes());
        buf[off + 12..off + 16].copy_from_slice(&edx.to_le_bytes());
    }

    String::from_utf8_lossy(&buf)
        .trim_end_matches('\0')
        .trim()
        .into()
}

/// Process filesystem
pub struct ProcFs {
    root: Arc<ProcNode>,
}

impl ProcFs {
    /// A procfs for the caller's PID namespace, the one a mount of it shows
    /// (Linux's proc_fill_super takes the mounter's task_active_pid_ns).
    pub fn new() -> Self {
        Self::for_ns(crate::process::pidns::current())
    }

    /// A procfs showing PID namespace `ns`.
    pub fn for_ns(ns: Arc<crate::process::pidns::PidNs>) -> Self {
        Self {
            root: Arc::new(ProcNode {
                node_type: ProcNodeType::Root,
                ns,
            }),
        }
    }
}

impl Default for ProcFs {
    fn default() -> Self {
        Self::new()
    }
}

impl Filesystem for ProcFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }

    fn name(&self) -> &str {
        "procfs"
    }

    /// Mounted read-write, as Linux mounts proc: `/proc/sys` and
    /// `timens_offsets` take writes; the files that do not refuse them.
    fn is_readonly(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<(), KernelError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// fopen("w") opens with O_TRUNC, as LTP's SAFE_FILE_PRINTF does: the
    /// writable files take it (Linux ignores the size), the rest refuse it.
    #[test]
    fn writable_files_accept_o_trunc() {
        use super::ProcNode;
        use crate::fs::VfsNode;
        let offsets = ProcNode::new_process_file(1, "timens_offsets".into());
        assert!(offsets.truncate(0).is_ok());
        let status = ProcNode::new_process_file(1, "status".into());
        assert!(status.truncate(0).is_err());
    }

    #[test]
    fn timens_offset_lines_parse_as_linux_reads_them() {
        use crate::process::ns::{CLOCK_BOOTTIME, CLOCK_MONOTONIC};
        assert_eq!(
            super::parse_offset_line("monotonic 100 5"),
            Some((CLOCK_MONOTONIC, 100, 5))
        );
        assert_eq!(
            super::parse_offset_line("7 -3 0"),
            Some((CLOCK_BOOTTIME, -3, 0))
        );
        assert_eq!(super::parse_offset_line("boottime 1"), None);
        assert_eq!(super::parse_offset_line("boottime 1 1000000000"), None);
        assert_eq!(super::parse_offset_line("realtime 1 0"), None);
        assert_eq!(super::parse_offset_line("monotonic 1 -1"), None);
        assert_eq!(
            super::offset_line("monotonic", -1),
            "monotonic          -1 999999999\n"
        );
    }

    use super::*;

    /// Everyone may walk /proc and read its files, as on Linux: directories
    /// r-xr-xr-x, files r--r--r--. (Directories had no execute bit, so a
    /// non-root process could open nothing below /proc.)
    #[test]
    fn proc_nodes_are_world_readable() {
        let dir = ProcNode::new_root().metadata().unwrap();
        assert_eq!(dir.permissions, Permissions::from_mode(0o555));
        let pid_dir = ProcNode::new_process_dir(1).metadata().unwrap();
        assert_eq!(pid_dir.permissions, Permissions::from_mode(0o555));
        let file = ProcNode::new_system_file(String::from("uptime"))
            .metadata()
            .unwrap();
        assert_eq!(file.permissions, Permissions::from_mode(0o444));
    }

    /// Every /proc entry has one inode number, the same in a directory
    /// listing and in stat (find and `ls -i` compare the two), nonzero and
    /// distinct; the root is 1, Linux's PROC_ROOT_INO. They were all 0, and
    /// getdents numbered entries by their position instead.
    #[test]
    fn inode_numbers_agree_between_readdir_and_stat() {
        fn walk(dir: &dyn VfsNode, parent: u64, seen: &mut Vec<u64>) {
            let ino = dir.metadata().unwrap().inode;
            for e in dir.readdir().unwrap() {
                match e.name.as_str() {
                    "." => assert_eq!(e.inode, ino),
                    ".." => assert_eq!(e.inode, parent),
                    name => {
                        let Ok(child) = dir.lookup(name) else {
                            continue; // a process that is not in the table
                        };
                        let child_ino = child.metadata().unwrap().inode;
                        assert_eq!(e.inode, child_ino, "{name}");
                        assert!(child_ino > 1, "{name}");
                        assert!(!seen.contains(&child_ino), "{name}");
                        seen.push(child_ino);
                        if e.node_type == NodeType::Directory {
                            walk(child.as_ref(), ino, seen);
                        }
                    }
                }
            }
        }
        let root = ProcNode::new_root();
        assert_eq!(root.metadata().unwrap().inode, 1);
        let mut seen = Vec::new();
        walk(&root, 1, &mut seen);
        assert!(seen.len() >= 15, "{}", seen.len());
        // The same file looked up twice is the same inode.
        let a = root.lookup("uptime").unwrap().metadata().unwrap().inode;
        let b = root.lookup("uptime").unwrap().metadata().unwrap().inode;
        assert_eq!(a, b);
    }

    /// Status shows all four user and group IDs, filesystem ones last.
    #[test]
    fn status_shows_every_id() {
        let mut c = crate::process::creds::Credentials::new(0, 0);
        c.setgroups(&[10, 20], true).unwrap();
        c.setresgid(5, 6, 7, true).unwrap();
        c.setfsgid(8, true);
        c.setresuid(1, 2, 3, true).unwrap();
        assert_eq!(
            status_ids(&c, &crate::process::userns::initial()),
            "Uid:\t1\t2\t3\t2\nGid:\t5\t6\t7\t8\nGroups:\t10 20\n"
        );
        let mut c = crate::process::creds::Credentials::new(0, 0);
        c.setfsuid(1000, true);
        assert_eq!(
            status_ids(&c, &crate::process::userns::initial()),
            "Uid:\t0\t0\t0\t1000\nGid:\t0\t0\t0\t0\nGroups:\t\n"
        );
    }

    /// The NS lines go from the mount's PID namespace down to the
    /// process's own; a group led from above shows as 0 below.
    #[test]
    fn status_shows_the_pid_at_each_level() {
        use crate::process::{pidns, userns, Process, ProcessId, ProcessPriority};
        let init = pidns::initial();
        let ns = pidns::PidNs::create(&init, &userns::initial(), 0).unwrap();
        let p = Process::new(
            ProcessId(7_100_001),
            None,
            String::from("t"),
            ProcessPriority::Normal,
        );
        pidns::reserve(&ns, &[], |_| false).unwrap().commit(p.pid.0);
        p.set_pid_ns(ns.clone());
        p.pgid
            .store(7_100_000, core::sync::atomic::Ordering::Release);
        assert_eq!(
            status_nspids(&p, &init),
            "NStgid:\t7100001\t1\nNSpid:\t7100001\t1\nNSpgid:\t7100000\t0\nNSsid:\t7100001\t1\n"
        );
        assert_eq!(
            status_nspids(&p, &ns),
            "NStgid:\t1\nNSpid:\t1\nNSpgid:\t0\nNSsid:\t1\n"
        );
        // A mount of its namespace lists it as 1 and finds it by that.
        let root = ProcNode::of(ProcNodeType::Root);
        let mine = root.child(ProcNodeType::Root);
        assert_eq!(mine.nr(p.pid.0), 7_100_001);
        let theirs = ProcNode {
            node_type: ProcNodeType::Root,
            ns: ns.clone(),
        };
        assert_eq!(theirs.nr(p.pid.0), 1);
        assert_eq!(theirs.nr(1), 0, "the initial namespace's 1 is not there");
        pidns::release(p.pid.0);
    }

    /// Status shows the five capability sets as Linux does: 16 hex digits,
    /// in the order inheritable, permitted, effective, bounding, ambient.
    #[test]
    fn status_shows_the_capability_sets() {
        use crate::process::linux_caps::{CapSets, LinuxCaps, CAP_KILL, CAP_NET_BIND_SERVICE};
        assert_eq!(
            status_caps(&CapSets::ROOT),
            concat!(
                "CapInh:\t0000000000000000\n",
                "CapPrm:\t000001ffffffffff\n",
                "CapEff:\t000001ffffffffff\n",
                "CapBnd:\t000001ffffffffff\n",
                "CapAmb:\t0000000000000000\n",
            )
        );
        let mut caps = CapSets::NONE;
        caps.inheritable = LinuxCaps::of(CAP_KILL);
        caps.ambient = LinuxCaps::of(CAP_NET_BIND_SERVICE);
        assert_eq!(
            status_caps(&caps),
            concat!(
                "CapInh:\t0000000000000020\n",
                "CapPrm:\t0000000000000000\n",
                "CapEff:\t0000000000000000\n",
                "CapBnd:\t000001ffffffffff\n",
                "CapAmb:\t0000000000000400\n",
            )
        );
    }

    /// /proc/sys/kernel/cap_last_cap is the highest capability number, which
    /// libcap and LTP read to size their sets.
    #[test]
    fn cap_last_cap_names_the_highest_capability() {
        let dir = ProcNode::of(ProcNodeType::SubDir(String::from("sys/kernel")));
        let file = dir.lookup("cap_last_cap").unwrap();
        let mut buf = [0u8; 16];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"40\n");
    }

    /// /proc/sys/kernel/{ostype,osrelease,version} are uname's sysname,
    /// release and version, and /proc/version names all three (ADR 0021).
    #[test]
    fn kernel_identity_files_match_uname() {
        let dir = ProcNode::of(ProcNodeType::SubDir(String::from("sys/kernel")));
        let read = |node: Arc<dyn VfsNode>| {
            let mut buf = [0u8; 160];
            let n = node.read(0, &mut buf).unwrap();
            String::from(core::str::from_utf8(&buf[..n]).unwrap())
        };
        assert_eq!(read(dir.lookup("ostype").unwrap()), "VeridianOS\n");
        assert_eq!(read(dir.lookup("osrelease").unwrap()), "7.3.0-veridian\n");
        assert_eq!(
            read(dir.lookup("version").unwrap()),
            concat!("#1 SMP VeridianOS ", env!("CARGO_PKG_VERSION"), "\n")
        );
        let proc_version = read(Arc::new(ProcNode::new_system_file(String::from("version"))));
        assert!(proc_version.starts_with("VeridianOS version 7.3.0-veridian ("));
    }

    /// /proc/sys and the directories below it list what lookup finds there,
    /// so a program reading the directory (getdents64) sees the files; it
    /// was ENOTDIR (PR #34 review). /proc lists sys.
    #[test]
    /// The user namespace sysctls (ADR 0022 H4a) are found by their paths
    /// and refuse values outside their ranges without storing them.
    fn user_namespace_sysctls_check_their_ranges() {
        let switch = Sysctl::from_name("sys/kernel/unprivileged_userns_clone").unwrap();
        let max = Sysctl::from_name("sys/user/max_user_namespaces").unwrap();
        assert!(Sysctl::from_name("sys/kernel/unprivileged_userns").is_none());
        assert_eq!(switch.get(), 0, "off by default");
        for bad in [&b"2"[..], b"-1", b"on", b""] {
            assert!(switch.write(bad).is_err(), "{:?}", bad);
        }
        assert!(max.write(b"2147483648").is_err());
        assert!(max.write(b"x").is_err());
        assert_eq!(switch.get(), 0);
        assert_eq!(
            max.get(),
            crate::process::userns::DEFAULT_MAX_USER_NAMESPACES as usize
        );
        let pid_max = Sysctl::from_name("sys/user/max_pid_namespaces").unwrap();
        assert!(pid_max.write(b"-3").is_err());
        assert_eq!(
            pid_max.get(),
            crate::process::userns::DEFAULT_MAX_USER_NAMESPACES as usize
        );
        let net_max = Sysctl::from_name("sys/user/max_net_namespaces").unwrap();
        assert!(net_max.write(b"2147483648").is_err());
        assert_eq!(
            net_max.get(),
            crate::process::userns::DEFAULT_MAX_USER_NAMESPACES as usize
        );
    }

    #[test]
    fn sys_directories_list_their_entries() {
        let names = |n: &ProcNode| -> Vec<String> {
            n.readdir()
                .unwrap()
                .into_iter()
                .filter(|e| e.name != "." && e.name != "..")
                .map(|e| e.name)
                .collect()
        };
        assert!(names(&ProcNode::new_root()).iter().any(|n| n == "sys"));
        for (dir, want) in [
            ("sys", &["fs", "kernel", "user"][..]),
            ("sys/fs", &["inotify"][..]),
            (
                "sys/fs/inotify",
                &[
                    "max_queued_events",
                    "max_user_instances",
                    "max_user_watches",
                ][..],
            ),
            (
                "sys/kernel",
                &[
                    "cap_last_cap",
                    "core_pattern",
                    "domainname",
                    "hostname",
                    "osrelease",
                    "ostype",
                    "pid_max",
                    "random",
                    "tainted",
                    "unprivileged_userns_clone",
                    "version",
                ][..],
            ),
            ("sys/kernel/random", &["boot_id"][..]),
            (
                "sys/user",
                &[
                    "max_net_namespaces",
                    "max_pid_namespaces",
                    "max_user_namespaces",
                ][..],
            ),
            ("net", &["dev"][..]),
        ] {
            let node = ProcNode::of(ProcNodeType::SubDir(String::from(dir)));
            let mut got = names(&node);
            got.sort();
            assert_eq!(got, want, "/proc/{}", dir);
            for e in node
                .readdir()
                .unwrap()
                .iter()
                .filter(|e| !e.name.starts_with('.'))
            {
                let found = node.lookup(&e.name).unwrap();
                assert_eq!(found.node_type(), e.node_type, "/proc/{}/{}", dir, e.name);
            }
        }
    }

    /// /proc/sys/kernel/pid_max is Linux's 64-bit limit (PID_MAX_LIMIT);
    /// VeridianOS ids do not wrap at it (docs/KNOWN-LIMITATIONS.md).
    #[test]
    fn pid_max_is_the_linux_64_bit_limit() {
        let dir = ProcNode::of(ProcNodeType::SubDir(String::from("sys/kernel")));
        let file = dir.lookup("pid_max").unwrap();
        let mut buf = [0u8; 16];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"4194304\n");
    }

    /// The inotify limits are sysctls root may write, mode 0644: a decimal
    /// integer from 0 to INT_MAX, anything else EINVAL; O_TRUNC is fine.
    #[test]
    fn inotify_limits_can_be_written() {
        let file = ProcNode::new_system_file(String::from("sys/fs/inotify/max_queued_events"));
        assert_eq!(
            file.metadata().unwrap().permissions,
            Permissions::from_mode(0o644)
        );
        assert!(file.truncate(0).is_ok());
        // The value it has, so no other test sees a change.
        assert_eq!(file.write(0, b"16384\n"), Ok(6));
        for bad in [&b"-1"[..], b"x", b"2147483648", b""] {
            assert!(file.write(0, bad).is_err(), "{:?}", bad);
        }
        let mut buf = [0u8; 16];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"16384\n");
        // Other files stay read-only.
        let uptime = ProcNode::new_system_file(String::from("uptime"));
        assert!(uptime.write(0, b"1").is_err());
        assert!(uptime.truncate(0).is_err());
    }

    /// /proc/sys/fs/inotify has the inotify limits, Linux's defaults.
    #[test]
    fn inotify_limits_are_linux_defaults() {
        let dir = ProcNode::of(ProcNodeType::SubDir(String::from("sys/fs/inotify")));
        for (name, want) in [
            ("max_queued_events", &b"16384\n"[..]),
            ("max_user_instances", &b"128\n"[..]),
        ] {
            let mut buf = [0u8; 16];
            let n = dir.lookup(name).unwrap().read(0, &mut buf).unwrap();
            assert_eq!(&buf[..n], want, "{}", name);
        }
    }

    /// The RISC-V `mmu` field names the mode, as Linux prints it.
    #[test]
    fn riscv_mmu_field_names_the_paging_mode() {
        assert_eq!(riscv_mmu_name(3), "sv39");
        assert_eq!(riscv_mmu_name(4), "sv48");
    }
    use crate::mm::{vas::MappingType, PageFlags};

    #[test]
    fn maps_line_matches_linux_format() {
        let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::NO_EXECUTE;
        assert_eq!(
            format_maps_line(0x40_0000, 0x2000, flags, MappingType::Data),
            "00400000-00402000 r--p 00000000 00:00 0\n"
        );
    }

    #[test]
    fn maps_line_reports_write_exec_and_labels() {
        let rw = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITABLE | PageFlags::NO_EXECUTE;
        assert_eq!(
            format_maps_line(0x7fff_f000_0000, 0x1000, rw, MappingType::Stack),
            "7ffff0000000-7ffff0001000 rw-p 00000000 00:00 0 [stack]\n"
        );
        assert_eq!(
            format_maps_line(0x1000, 0x1000, rw, MappingType::Heap),
            "00001000-00002000 rw-p 00000000 00:00 0 [heap]\n"
        );
        let rx = PageFlags::PRESENT | PageFlags::USER;
        assert_eq!(
            format_maps_line(0x40_1000, 0x1000, rx, MappingType::Code),
            "00401000-00402000 r-xp 00000000 00:00 0\n"
        );
        // Shared mappings are `s`, not `p`.
        for ty in [MappingType::Shared, MappingType::SharedRegion] {
            assert_eq!(
                format_maps_line(0x4000_0000, 0x1000, rw, ty),
                "40000000-40001000 rw-s 00000000 00:00 0\n"
            );
        }
    }
}

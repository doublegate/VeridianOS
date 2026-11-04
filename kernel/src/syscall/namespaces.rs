//! unshare, setns, sethostname and setdomainname, and the nsfs ioctls, as
//! Linux 7.3's `kernel/fork.c` (ksys_unshare), `kernel/nsproxy.c`,
//! `kernel/sys.c` and `fs/nsfs.c` (ADR 0022). The namespaces themselves are
//! `process::ns`'s.

use alloc::sync::Arc;

use super::{SyscallError, SyscallResult};
use crate::{
    fs::nsfs::NsFileNode,
    process::{
        self,
        creds::Caller,
        linux_caps::{CapSets, CAP_SETFCAP, CAP_SYS_ADMIN, CAP_SYS_CHROOT},
        ns::{self, NsKind, NsRef, CLONE_NEWUSER, CLONE_NS_ALL},
        userns::{self, CreateError, UserNs},
    },
};

const CLONE_VM: u64 = 0x0000_0100;
const CLONE_FS: u64 = 0x0000_0200;
const CLONE_FILES: u64 = 0x0000_0400;
const CLONE_SIGHAND: u64 = 0x0000_0800;
const CLONE_THREAD: u64 = 0x0001_0000;
const CLONE_SYSVSEM: u64 = 0x0004_0000;
/// unshare's own name for CLONE_EMPTY_MNTNS (it aliases
/// CLONE_PARENT_SETTID, which unshare has no use for).
const UNSHARE_EMPTY_MNTNS: u64 = 0x0010_0000;

/// A new user namespace below `caller`'s, for clone or unshare with
/// CLONE_NEWUSER: without CAP_SYS_ADMIN in the initial namespace only while
/// `/proc/sys/kernel/unprivileged_userns_clone` is 1 (EPERM; ADR 0022),
/// then Linux's create_user_ns (ENOSPC too deep or too many, EPERM from a
/// chroot or for a creator the parent does not map).
pub(super) fn new_user_ns(caller: &Caller) -> Result<Arc<UserNs>, SyscallError> {
    let allowed = userns::UNPRIVILEGED_CLONE.load(core::sync::atomic::Ordering::Relaxed) != 0;
    if !allowed && !caller.capable(CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
    // current_chrooted: the root is not the mount namespace's.
    let chrooted = process::current_thread().is_some_and(|t| *t.fs().root.lock() != "/");
    UserNs::create(
        &caller.userns,
        caller.creds.euid,
        caller.creds.egid,
        chrooted,
        caller.caps.has(CAP_SETFCAP),
    )
    .map_err(|e| match e {
        CreateError::NoSpace => SyscallError::NoSpace,
        CreateError::Permission => SyscallError::OperationNotPermitted,
    })
}

/// Whether credentials in user namespace `new` hold no more than those in
/// `old` with effective uid `euid` did (Linux's cred_cap_issubset, for two
/// namespaces): only when `new` is below `old` and `euid` owns it or one
/// of its ancestors below `old`.
fn ns_change_is_subset(old: &UserNs, euid: u32, new: &UserNs) -> bool {
    old.ident.id == new.ident.id || new.capable(old, euid, false)
}

/// Put the calling process in user namespace `ns` with every capability
/// there (Linux's set_cred_user_ns). Joining one the old credentials had
/// no power over makes the process non-dumpable, as commit_creds does when
/// the new credentials are not a subset of the old: the namespace's root
/// must not trace memory a more privileged process filled.
fn enter_user_ns(ns: Arc<UserNs>) -> Result<(), SyscallError> {
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    if !ns_change_is_subset(&process.user_ns(), process.credentials().euid, &ns) {
        process
            .dumpable
            .store(false, core::sync::atomic::Ordering::Release);
    }
    process.set_user_ns(ns);
    *thread.caps.lock() = CapSets::ROOT;
    Ok(())
}

/// Whether the calling process has a thread besides the caller still
/// running (Linux's thread_group_empty is false).
fn other_threads() -> bool {
    let (Some(proc), Some(me)) = (process::current_process(), process::current_thread()) else {
        return false;
    };
    let others = proc.threads.lock().values().any(|t| {
        t.tid != me.tid
            && !matches!(
                t.get_state(),
                process::thread::ThreadState::Zombie | process::thread::ThreadState::Dead
            )
    });
    others
}

/// Whether another live process runs in the caller's address space (a
/// CLONE_VM child or parent): Linux's current_is_single_threaded is false.
fn vm_shared() -> bool {
    let Some(me) = process::current_process() else {
        return false;
    };
    let vm = me.vm_id.load(core::sync::atomic::Ordering::Acquire);
    let mut shared = false;
    process::table::PROCESS_TABLE.for_each(|p| {
        shared |= p.pid != me.pid
            && p.is_alive()
            && p.vm_id.load(core::sync::atomic::Ordering::Acquire) == vm;
    });
    shared
}

/// The flags unshare acts on, as ksys_unshare widens them: a new user
/// namespace unshares the thread group and filesystem state too, the
/// address space the signal handlers, the handlers the thread group, and
/// a mount namespace (empty or not) the filesystem state.
fn unshare_implied(mut flags: u64) -> u64 {
    if flags & CLONE_NEWUSER != 0 {
        flags |= CLONE_THREAD | CLONE_FS;
    }
    if flags & CLONE_VM != 0 {
        flags |= CLONE_SIGHAND;
    }
    if flags & CLONE_SIGHAND != 0 {
        flags |= CLONE_THREAD;
    }
    if flags & UNSHARE_EMPTY_MNTNS != 0 {
        flags |= ns::CLONE_NEWNS;
    }
    if flags & ns::CLONE_NEWNS != 0 {
        flags |= CLONE_FS;
    }
    flags
}

/// check_unshare_flags: unknown flags are EINVAL, and so is unsharing the
/// thread group, handlers or address space of a process that has other
/// threads (`others`), or the address space while another process shares
/// it (`vm_shared`) -- Linux pretends those work only when there is
/// nothing to unshare.
fn unshare_checks(flags: u64, others: bool, vm_shared: bool) -> Result<(), SyscallError> {
    let known = CLONE_THREAD
        | CLONE_FS
        | CLONE_SIGHAND
        | CLONE_VM
        | CLONE_FILES
        | CLONE_SYSVSEM
        | CLONE_NS_ALL
        | UNSHARE_EMPTY_MNTNS;
    if flags & !known != 0
        || (flags & (CLONE_THREAD | CLONE_SIGHAND | CLONE_VM) != 0 && others)
        || (flags & CLONE_VM != 0 && vm_shared)
    {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// unshare(flags), as Linux's ksys_unshare: the flags widened
/// ([`unshare_implied`]) and checked ([`unshare_checks`]); CLONE_FS gives
/// the thread its own copy of a filesystem state it shares; CLONE_FILES
/// is a no-op for a single-threaded process and EINVAL otherwise (the
/// descriptor table is the process's, shared by all its threads, here);
/// CLONE_NEWUSER makes a user namespace ([`new_user_ns`]), whose root may
/// then make the others, which belong to it; any other namespace needs
/// CAP_SYS_ADMIN over the user namespace that will own it, the new one or
/// the caller's (EPERM; unshare_nsproxy_namespaces).
/// Nothing changes unless everything succeeds.
pub fn sys_unshare(flags: usize) -> SyscallResult {
    let flags = unshare_implied(flags as u64);
    unshare_checks(flags, other_threads(), vm_shared())?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    if flags & CLONE_FILES != 0 && other_threads() {
        return Err(SyscallError::InvalidArgument);
    }
    let caller = Caller::current();
    let user_ns = if flags & CLONE_NEWUSER != 0 {
        Some(new_user_ns(&caller)?)
    } else {
        None
    };
    let owner = user_ns.clone().unwrap_or_else(|| caller.userns.clone());
    let new_ns = if flags & CLONE_NS_ALL & !CLONE_NEWUSER != 0 {
        // The root of a new user namespace holds it there.
        if !caller.ns_capable(&owner, CAP_SYS_ADMIN) {
            return Err(SyscallError::OperationNotPermitted);
        }
        let current = thread.namespaces();
        // The 32-bit UNSHARE_EMPTY_MNTNS (CLONE_PARENT_SETTID's bit) goes on
        // as the 64-bit CLONE_EMPTY_MNTNS, as unshare_nsproxy_namespaces.
        let flags = if flags & UNSHARE_EMPTY_MNTNS != 0 {
            (flags & !UNSHARE_EMPTY_MNTNS) | ns::CLONE_NEWNS | ns::CLONE_EMPTY_MNTNS
        } else {
            flags
        };
        let active = process::pidns::current();
        Some(Arc::new(
            current
                .copy_for(flags, &owner, &active, caller.creds.euid)
                .map_err(ns_error)?,
        ))
    } else {
        None
    };
    // The point of no return.
    if let Some(ns) = user_ns {
        enter_user_ns(ns)?;
    }
    if flags & CLONE_FS != 0 {
        let fs = thread.fs();
        if fs.users.load(core::sync::atomic::Ordering::Acquire) > 1 {
            thread.replace_fs(process::thread::ThreadFs::clone_copy(&fs));
        }
    }
    if let Some(set) = new_ns {
        thread.set_namespaces(set);
        // An empty mount namespace has only its root mount: the root and
        // working directory move to it (copy_mnt_ns).
        if flags & UNSHARE_EMPTY_MNTNS != 0 {
            thread.fs().move_to_root();
        }
    }
    Ok(0)
}

/// A namespace set that could not be made, as an errno: a second new PID
/// namespace before the first is entered EINVAL, a namespace limit ENOSPC, a
/// PID namespace whose user namespace is not below its parent's EPERM.
fn ns_error(e: ns::NsError) -> SyscallError {
    match e {
        ns::NsError::PidPending => SyscallError::InvalidArgument,
        ns::NsError::NoSpace => SyscallError::NoSpace,
        ns::NsError::Permission => SyscallError::OperationNotPermitted,
    }
}

/// The namespaces of the process `target` as setns from a pidfd takes
/// them: its first thread's set (the one `/proc/<pid>/ns` shows).
fn namespaces_of(target: &process::Process) -> Option<Arc<ns::Namespaces>> {
    target
        .get_thread(process::ThreadId(target.pid.0))
        .or_else(|| target.threads.lock().values().next().cloned())
        .map(|t| t.namespaces())
}

/// One namespace's install rule (each type's `install` operation in
/// Linux), checked before anything changes. UTS, IPC, cgroup, PID,
/// network and time namespaces need CAP_SYS_ADMIN over both the user
/// namespace that owns the target and the caller's own; a time namespace
/// first a single-threaded caller, sharing its address space with no
/// other process (EUSERS, as timens_install); a mount namespace
/// CAP_SYS_ADMIN over its owner and CAP_SYS_CHROOT and CAP_SYS_ADMIN in
/// the caller's own (mntns_install), and a filesystem state no other thread
/// shares (EINVAL), since joining it moves the root and working directory to
/// its root; a PID namespace (pidns_install) then also the caller's own or
/// one below it (EINVAL), which becomes the one for its children; a user
/// namespace (userns_install) not the caller's own (EINVAL:
/// re-entering it would give back dropped capabilities), from a
/// single-threaded caller with its own filesystem state (EINVAL), and
/// CAP_SYS_ADMIN over it (EPERM). The checks after a user namespace in one
/// pidfd setns use the caller's credentials from before, where Linux's use
/// those it will have in the new namespace.
fn install_check(ns: &NsRef) -> Result<(), SyscallError> {
    let caller = process::creds::Caller::current();
    // CAP_SYS_ADMIN over the namespace's owner and in the caller's own.
    let admin_over = |caller: &Caller| {
        ns.owner()
            .is_some_and(|owner| caller.ns_capable(&owner, CAP_SYS_ADMIN))
            && caller.ns_capable(&caller.userns, CAP_SYS_ADMIN)
    };
    match ns.kind() {
        NsKind::User => {
            let NsRef::User(target) = ns else {
                return Err(SyscallError::InvalidArgument);
            };
            if target.ident.id == caller.userns.ident.id || other_threads() {
                return Err(SyscallError::InvalidArgument);
            }
            let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
            if thread
                .fs()
                .users
                .load(core::sync::atomic::Ordering::Acquire)
                != 1
            {
                return Err(SyscallError::InvalidArgument);
            }
            if !caller.ns_capable(target, CAP_SYS_ADMIN) {
                return Err(SyscallError::OperationNotPermitted);
            }
            Ok(())
        }
        NsKind::Mnt => {
            if !admin_over(&caller) || !caller.ns_capable(&caller.userns, CAP_SYS_CHROOT) {
                return Err(SyscallError::OperationNotPermitted);
            }
            let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
            if thread
                .fs()
                .users
                .load(core::sync::atomic::Ordering::Acquire)
                != 1
            {
                return Err(SyscallError::InvalidArgument);
            }
            Ok(())
        }
        NsKind::Pid => {
            if !admin_over(&caller) {
                return Err(SyscallError::OperationNotPermitted);
            }
            let NsRef::Pid(target) = ns else {
                return Err(SyscallError::InvalidArgument);
            };
            if !process::pidns::current().contains(target) {
                return Err(SyscallError::InvalidArgument);
            }
            Ok(())
        }
        NsKind::Time => {
            if other_threads() || vm_shared() {
                return Err(SyscallError::TooManyUsers);
            }
            if !admin_over(&caller) {
                return Err(SyscallError::OperationNotPermitted);
            }
            Ok(())
        }
        _ if admin_over(&caller) => Ok(()),
        _ => Err(SyscallError::OperationNotPermitted),
    }
}

/// Make `namespaces` the caller's (setns's commit): the set replaced, for
/// a mount namespace the root and working directory moved to its root
/// (`mnt`), as Linux's mntns_install does, and the time namespace it runs
/// in fixed (timens_commit).
fn commit(namespaces: ns::Namespaces, mnt: bool) -> Result<(), SyscallError> {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    namespaces.time.freeze();
    if mnt {
        thread.fs().move_to_root();
    }
    thread.set_namespaces(Arc::new(namespaces));
    Ok(())
}

/// setns(fd, nstype), as Linux 7.3: `fd` is an nsfs file (from
/// `/proc/<pid>/ns`, or NS_GET_USERNS and NS_GET_PARENT), whose type
/// `nstype` must name if not 0, or a pidfd, whose process's namespaces of
/// every type in `nstype` (not 0, only CLONE_NEW* flags) are joined at
/// once; anything else is EINVAL, no file EBADF. For a pidfd the process
/// must exist (ESRCH) and be one the caller may ptrace-read (EPERM); then
/// each type's install rule ([`install_check`]) in Linux's order (user,
/// mnt, uts, ipc, pid, cgroup, net, time), and nothing changes unless all
/// pass.
pub fn sys_setns(fd: usize, nstype: usize) -> SyscallResult {
    let flags = nstype as u32 as u64;
    let file = super::file_calls::open_file(fd)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let current = thread.namespaces();
    if let Some(node) = file
        .node
        .as_any()
        .and_then(|any| any.downcast_ref::<NsFileNode>())
    {
        let target = node.ns().clone();
        if flags != 0 && target.kind().clone_flag() != flags {
            return Err(SyscallError::InvalidArgument);
        }
        install_check(&target)?;
        if let NsRef::User(user) = target {
            return enter_user_ns(user).map(|_| 0);
        }
        return commit(current.with(&target), target.kind() == NsKind::Mnt).map(|_| 0);
    }
    let (_, pid, _) = super::pidfd::target(fd).map_err(|_| SyscallError::InvalidArgument)?;
    if flags == 0 || flags & !CLONE_NS_ALL != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let proc = process::get_process(pid).ok_or(SyscallError::ProcessNotFound)?;
    {
        let caller = process::creds::Caller::current();
        let permitted = super::debug::current_permitted();
        let _guard = proc.cred_guard.lock();
        if !super::debug::may_access_process(&caller, permitted, &proc) {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    let theirs = namespaces_of(&proc).ok_or(SyscallError::ProcessNotFound)?;
    let order = [
        NsKind::User,
        NsKind::Mnt,
        NsKind::Uts,
        NsKind::Ipc,
        NsKind::Pid,
        NsKind::Cgroup,
        NsKind::Net,
        NsKind::Time,
    ];
    let mut new = (*current).clone();
    let mut user = None;
    for kind in order {
        if flags & kind.clone_flag() == 0 {
            continue;
        }
        let ns = match kind {
            NsKind::User => ns::user_of(&proc),
            NsKind::Pid => ns::pid_of(&proc),
            // The time namespace it runs in, not the one for children.
            NsKind::Time => NsRef::Time(theirs.time.clone()),
            _ => theirs.get(kind),
        };
        install_check(&ns)?;
        if let NsRef::User(target) = &ns {
            user = Some(target.clone());
        }
        new = new.with(&ns);
    }
    if let Some(target) = user {
        enter_user_ns(target)?;
    }
    commit(new, flags & ns::CLONE_NEWNS != 0).map(|_| 0)
}

/// sethostname(name, len) and setdomainname(name, len), as Linux's
/// kernel/sys.c: CAP_SYS_ADMIN over the user namespace that owns the
/// caller's UTS namespace (EPERM), a length past 64 (or negative, as an
/// int) EINVAL, an unreadable name EFAULT; the name is the caller's UTS
/// namespace's.
fn set_uts_name(name: usize, len: usize, host: bool) -> SyscallResult {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let uts = thread.namespaces().uts.clone();
    if !Caller::current().ns_capable(&uts.owner, CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let len = len as u32 as i32;
    if len < 0 || len as usize > ns::UTS_LEN {
        return Err(SyscallError::InvalidArgument);
    }
    let mut buf = [0u8; ns::UTS_LEN];
    super::userspace::read_user_bytes(name, &mut buf[..len as usize])?;
    // The bytes as given; Linux stores them as given too (a NUL ends what
    // uname shows).
    let end = buf[..len as usize]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(len as usize);
    let text = alloc::string::String::from_utf8_lossy(&buf[..end]);
    if host {
        uts.set_hostname(&text);
    } else {
        uts.set_domainname(&text);
    }
    Ok(0)
}

pub fn sys_sethostname(name: usize, len: usize) -> SyscallResult {
    set_uts_name(name, len, true)
}

pub fn sys_setdomainname(name: usize, len: usize) -> SyscallResult {
    set_uts_name(name, len, false)
}

/// The nsfs ioctls (`include/uapi/linux/nsfs.h`).
const NSIO: u32 = 0xb7;
const NS_GET_USERNS: u32 = NSIO << 8 | 0x1;
const NS_GET_PARENT: u32 = NSIO << 8 | 0x2;
const NS_GET_NSTYPE: u32 = NSIO << 8 | 0x3;
const NS_GET_OWNER_UID: u32 = NSIO << 8 | 0x4;
/// _IOR(NSIO, nr, int) and _IOR(NSIO, 13, __u64).
const fn ior(nr: u32, size: u32) -> u32 {
    2 << 30 | size << 16 | NSIO << 8 | nr
}
const NS_GET_PID_FROM_PIDNS: u32 = ior(6, 4);
const NS_GET_TGID_FROM_PIDNS: u32 = ior(7, 4);
const NS_GET_PID_IN_PIDNS: u32 = ior(8, 4);
const NS_GET_TGID_IN_PIDNS: u32 = ior(9, 4);
const NS_GET_ID: u32 = ior(13, 8);

/// Open an nsfs file for `ns` in the caller's table, read-only and
/// close-on-exec, as Linux's open_related_ns.
pub(super) fn open_ns(ns: NsRef) -> SyscallResult {
    use crate::fs::file::{File, OpenFlags};
    let file = File::new(Arc::new(NsFileNode::new(ns)), OpenFlags::read_only());
    let caller = process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = caller
        .file_table
        .lock()
        .open_with_flags(Arc::new(file), true)
        .map_err(super::map_kernel_error)?;
    Ok(fd)
}

/// The user namespace that owns `ns`, if the caller's is it or one of its
/// ancestors (Linux's ns_get_owner; EPERM otherwise, and for the initial
/// user namespace, which has no owner).
fn owner_for_caller(ns: &NsRef) -> Result<Arc<UserNs>, SyscallError> {
    let mine = Caller::current().userns;
    let owner = ns.owner().ok_or(SyscallError::OperationNotPermitted)?;
    let mut p = Some(owner.clone());
    while let Some(ns) = p {
        if ns.ident.id == mine.ident.id {
            return Ok(owner);
        }
        p = ns.parent.clone();
    }
    Err(SyscallError::OperationNotPermitted)
}

/// NS_GET_PARENT for a PID namespace (pidns_get_parent): its parent, if
/// that is the caller's PID namespace or below it (EPERM otherwise, and for
/// the initial namespace).
fn pid_parent_for_caller(
    ns: &Arc<process::pidns::PidNs>,
) -> Result<Arc<process::pidns::PidNs>, SyscallError> {
    let parent = ns
        .parent
        .clone()
        .ok_or(SyscallError::OperationNotPermitted)?;
    if process::pidns::current().contains(&parent) {
        Ok(parent)
    } else {
        Err(SyscallError::OperationNotPermitted)
    }
}

/// The NS_GET_*_PIDNS translations (ns_ioctl): FROM takes `id` as PID
/// namespace `ns` numbers it and gives the caller's number, IN the other
/// way; the TGID forms give the task's process. No such task, or none the
/// other side numbers, is ESRCH.
fn pid_translate(cmd: u32, ns: &Arc<process::pidns::PidNs>, id: i32) -> SyscallResult {
    use process::pidns;
    let from = matches!(cmd, NS_GET_PID_FROM_PIDNS | NS_GET_TGID_FROM_PIDNS);
    let tgid = matches!(cmd, NS_GET_TGID_FROM_PIDNS | NS_GET_TGID_IN_PIDNS);
    let mine = pidns::current();
    let (lookup, report) = if from { (ns, &mine) } else { (&mine, ns) };
    let kid = (id > 0)
        .then(|| pidns::find(id as u64, lookup))
        .flatten()
        .ok_or(SyscallError::ProcessNotFound)?;
    // The task, and for the TGID forms its process (task_tgid_nr_ns).
    let owner = if process::get_process(process::ProcessId(kid)).is_some() {
        kid
    } else {
        super::process::process_of_thread(kid as usize)
            .ok_or(SyscallError::ProcessNotFound)?
            .0
    };
    match pidns::nr_in(if tgid { owner } else { kid }, report) {
        0 => Err(SyscallError::ProcessNotFound),
        nr => Ok(nr as usize),
    }
}

/// An nsfs ioctl on `file`, or None if it is not an nsfs file:
/// NS_GET_NSTYPE gives the type's CLONE_NEW* flag; NS_GET_ID stores the
/// 64-bit id at `arg`; NS_GET_USERNS opens the owning user namespace and,
/// for a user namespace, NS_GET_PARENT its parent ([`owner_for_caller`]);
/// NS_GET_PARENT is EINVAL for types without a parent and, for a PID
/// namespace, its parent ([`pid_parent_for_caller`]); NS_GET_OWNER_UID stores a
/// user namespace's creator as the caller's namespace sees it (EINVAL for other
/// types); the NS_GET_*_PIDNS
/// translations map a pid between a PID namespace and the caller's
/// (EINVAL for other types, ESRCH for no such task); anything else ENOTTY.
pub(super) fn nsfs_ioctl(
    file: &crate::fs::file::File,
    cmd: usize,
    arg: usize,
) -> Option<SyscallResult> {
    let node = file.node.as_any()?.downcast_ref::<NsFileNode>()?;
    let ns = node.ns();
    let kind = ns.kind();
    Some(match cmd as u32 {
        NS_GET_NSTYPE => Ok(kind.clone_flag() as usize),
        NS_GET_ID => super::userspace::write_user::<u64>(arg, ns.ident().id).map(|_| 0),
        NS_GET_USERNS => owner_for_caller(ns).and_then(|o| open_ns(NsRef::User(o))),
        NS_GET_PARENT => match kind {
            NsKind::User => owner_for_caller(ns).and_then(|o| open_ns(NsRef::User(o))),
            NsKind::Pid => match &ns {
                NsRef::Pid(pid_ns) => {
                    pid_parent_for_caller(pid_ns).and_then(|parent| open_ns(NsRef::Pid(parent)))
                }
                _ => Err(SyscallError::InvalidArgument),
            },
            _ => Err(SyscallError::InvalidArgument),
        },
        NS_GET_OWNER_UID => match ns {
            NsRef::User(user) => {
                let uid = Caller::current().userns.uid_munged(user.owner);
                super::userspace::write_user::<u32>(arg, uid).map(|_| 0)
            }
            _ => Err(SyscallError::InvalidArgument),
        },
        NS_GET_PID_FROM_PIDNS
        | NS_GET_TGID_FROM_PIDNS
        | NS_GET_PID_IN_PIDNS
        | NS_GET_TGID_IN_PIDNS => match &ns {
            NsRef::Pid(pid_ns) => pid_translate(cmd as u32, pid_ns, arg as u32 as i32),
            _ => Err(SyscallError::InvalidArgument),
        },
        _ => Err(SyscallError::NotATerminal),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unshare_widens_flags_as_ksys_unshare() {
        assert_eq!(
            unshare_implied(CLONE_NEWUSER),
            CLONE_NEWUSER | CLONE_THREAD | CLONE_FS
        );
        assert_eq!(
            unshare_implied(CLONE_VM),
            CLONE_VM | CLONE_SIGHAND | CLONE_THREAD
        );
        assert_eq!(unshare_implied(ns::CLONE_NEWNS), ns::CLONE_NEWNS | CLONE_FS);
        assert_eq!(
            unshare_implied(UNSHARE_EMPTY_MNTNS),
            UNSHARE_EMPTY_MNTNS | ns::CLONE_NEWNS | CLONE_FS
        );
        assert_eq!(unshare_implied(ns::CLONE_NEWUTS), ns::CLONE_NEWUTS);
    }

    #[test]
    fn unshare_refuses_what_check_unshare_flags_refuses() {
        assert!(unshare_checks(ns::CLONE_NEWUTS | CLONE_FILES, false, false).is_ok());
        assert_eq!(
            unshare_checks(0x8000 /* CLONE_PARENT */, false, false),
            Err(SyscallError::InvalidArgument)
        );
        // Only with nothing to unshare.
        assert!(unshare_checks(CLONE_THREAD, false, false).is_ok());
        assert!(unshare_checks(CLONE_THREAD, true, false).is_err());
        assert!(unshare_checks(CLONE_VM | CLONE_SIGHAND | CLONE_THREAD, false, true).is_err());
        assert!(unshare_checks(CLONE_SIGHAND | CLONE_THREAD, false, true).is_ok());
    }

    #[test]
    fn nsfs_ioctl_numbers_are_linuxs() {
        assert_eq!(NS_GET_USERNS, 0xb701);
        assert_eq!(NS_GET_NSTYPE, 0xb703);
        assert_eq!(NS_GET_PID_FROM_PIDNS, 0x8004_b706);
        assert_eq!(NS_GET_ID, 0x8008_b70d);
    }

    /// commit_creds' cred_cap_issubset across namespaces: entering one's
    /// own namespace (or one below it) gives nothing new; root entering a
    /// namespace another user owns does, so it stops being dumpable.
    #[test]
    fn joining_anothers_user_namespace_is_not_a_subset() {
        let ns = crate::process::userns::test_child(b"0 1000 1", b"0 1000 1");
        let init = crate::process::userns::initial();
        assert!(ns_change_is_subset(&init, 1000, &ns), "its owner");
        assert!(!ns_change_is_subset(&init, 0, &ns), "root, not its owner");
        assert!(ns_change_is_subset(&init, 0, &init));
        assert!(!ns_change_is_subset(&ns, 0, &init), "an ancestor");
    }
}

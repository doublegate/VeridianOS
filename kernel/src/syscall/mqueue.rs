//! POSIX message queues (`ipc/mqueue.c`): mq_open, mq_unlink,
//! mq_timedsend, mq_timedreceive, mq_notify and mq_getsetattr, each IPC
//! namespace with its own queues and limits (`/proc/sys/fs/mqueue`).
//!
//! A queue is a file (Linux's mqueue inode): mq_open returns a descriptor
//! of it, reading it gives Linux's status line, poll reports a message to
//! receive (POLLIN) or room to send (POLLOUT), and closing the last
//! descriptor of an unlinked queue frees it. Messages come out highest
//! priority first, in order within a priority.

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::sync::atomic::{AtomicI32, Ordering};

use spin::{Mutex, RwLock};

use super::{SyscallError, SyscallResult};
use crate::{
    error::KernelError,
    fs::{Metadata, NodeType, Permissions, VfsNode},
    process::{creds::Caller, linux_caps, ns::PlainNs},
};

/// Priorities are below this (MQ_PRIO_MAX).
pub const MQ_PRIO_MAX: u32 = 32768;
/// Linux's limits (`include/linux/ipc_namespace.h`).
pub const DFLT_QUEUESMAX: i32 = 256;
pub const DFLT_MSG: i32 = 10;
pub const DFLT_MSGMAX: i32 = 10;
pub const DFLT_MSGSIZE: i32 = 8192;
pub const DFLT_MSGSIZEMAX: i32 = 8192;
pub const MIN_MSGMAX: i32 = 1;
pub const HARD_MSGMAX: i32 = 65536;
pub const MIN_MSGSIZEMAX: i32 = 128;
pub const HARD_MSGSIZEMAX: i32 = 16 * 1024 * 1024;
/// The mqueue filesystem's statfs type.
const MQUEUE_MAGIC: u64 = 0x1980_0202;
/// A queue's name, at most NAME_MAX bytes.
const NAME_MAX: usize = 255;
/// The kernel's bookkeeping per message and per priority, charged to
/// RLIMIT_MSGQUEUE with the messages (sizeof msg_msg and
/// posix_msg_tree_node on a 64-bit kernel).
const MSG_OVERHEAD: u64 = 48;
const TREE_NODE: u64 = 48;

/// sigev_notify.
pub const SIGEV_SIGNAL: i32 = 0;
pub const SIGEV_NONE: i32 = 1;
pub const SIGEV_THREAD: i32 = 2;
/// si_code of a message-queue signal.
pub const SI_MESGQ: i32 = -3;

/// One namespace's queues and its `/proc/sys/fs/mqueue` limits.
pub struct MqSpace {
    pub queues_max: AtomicI32,
    pub msg_max: AtomicI32,
    pub msgsize_max: AtomicI32,
    pub msg_default: AtomicI32,
    pub msgsize_default: AtomicI32,
    names: Mutex<BTreeMap<String, Arc<MqNode>>>,
    /// Queues that exist (named or not yet freed), for queues_max.
    count: AtomicI32,
}

impl MqSpace {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            queues_max: AtomicI32::new(DFLT_QUEUESMAX),
            msg_max: AtomicI32::new(DFLT_MSGMAX),
            msgsize_max: AtomicI32::new(DFLT_MSGSIZEMAX),
            msg_default: AtomicI32::new(DFLT_MSG),
            msgsize_default: AtomicI32::new(DFLT_MSGSIZE),
            names: Mutex::new(BTreeMap::new()),
            count: AtomicI32::new(0),
        })
    }
}

/// The bytes each real user's queues hold against RLIMIT_MSGQUEUE (Linux's
/// UCOUNT_RLIMIT_MSGQUEUE).
static CHARGED: Mutex<BTreeMap<u32, u64>> = Mutex::new(BTreeMap::new());

/// Who to tell when a message arrives in an empty queue.
#[derive(Clone)]
struct Notify {
    pid: u64,
    /// The registering process itself (Linux holds its `struct pid`): a
    /// later process given the same pid is not it.
    owner: Weak<crate::process::Process>,
    kind: i32,
    signo: i32,
    value: u64,
}

struct State {
    maxmsg: usize,
    msgsize: usize,
    /// By priority, each in arrival order (the highest priority last).
    msgs: BTreeMap<u32, alloc::collections::VecDeque<Vec<u8>>>,
    count: usize,
    qsize: usize,
    notify: Option<Notify>,
    /// Receivers waiting now (a notification is sent only with none).
    receivers: usize,
}

/// A queue (`struct mqueue_inode_info`).
pub struct MqNode {
    state: Mutex<State>,
    metadata: RwLock<Metadata>,
    waiters: crate::sched::dispatch::WaitQueue,
    /// The real user charged for it, and how much.
    charge: (u32, u64),
    space: Weak<MqSpace>,
    /// The IPC namespace it belongs to, whose rights every operation on
    /// it checks: a descriptor carried into another namespace (setns,
    /// unshare, SCM_RIGHTS) still answers to the queue's own (PR #40
    /// review).
    ns: Weak<PlainNs>,
}

/// The caller's rights over the namespace `node` belongs to; none if the
/// namespace is gone (no capability can name it any more).
fn queue_rights(node: &MqNode) -> super::sysvipc::NsRights {
    node.ns.upgrade().map_or(
        super::sysvipc::NsRights {
            read: false,
            write: false,
            create: false,
            modify: false,
        },
        |ns| rights(&ns),
    )
}

impl Drop for MqNode {
    fn drop(&mut self) {
        let (uid, bytes) = self.charge;
        let mut c = CHARGED.lock();
        if let Some(v) = c.get_mut(&uid) {
            *v = v.saturating_sub(bytes);
        }
        if let Some(space) = self.space.upgrade() {
            space.count.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl MqNode {
    fn status(&self) -> String {
        let s = self.state.lock();
        let (kind, signo, pid) = s.notify.as_ref().map_or((0, 0, 0), |n| {
            (
                n.kind,
                if n.kind == SIGEV_SIGNAL { n.signo } else { 0 },
                super::sysvipc::pid_shown(n.pid),
            )
        });
        alloc::format!(
            "QSIZE:{:<10} NOTIFY:{:<5} SIGNO:{:<5} NOTIFY_PID:{:<6}\n",
            s.qsize,
            kind,
            signo,
            pid
        )
    }

    fn changed(&self) {
        self.waiters.wake_all();
        crate::sched::dispatch::io_event();
    }
}

impl VfsNode for MqNode {
    fn node_type(&self) -> NodeType {
        NodeType::File
    }

    /// The status line (mqueue_read_file), from `offset`: READ on the
    /// queue's namespace (ADR 0027).
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if !queue_rights(self).read {
            return Err(KernelError::FsError(
                crate::error::FsError::PermissionDenied,
            ));
        }
        let text = self.status();
        let bytes = text.as_bytes();
        if offset >= bytes.len() {
            return Ok(0);
        }
        let n = buffer.len().min(bytes.len() - offset);
        buffer[..n].copy_from_slice(&bytes[offset..offset + n]);
        Ok(n)
    }

    /// No write operation: EINVAL, as for Linux's mqueue files.
    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a message queue is written with mq_timedsend",
        })
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        let mut m = self.metadata.read().clone();
        m.size = self.state.lock().qsize;
        Ok(m)
    }

    fn readdir(&self) -> Result<Vec<crate::fs::DirEntry>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a message queue cannot be truncated",
        })
    }

    /// POLLIN with a message, POLLOUT with room (mqueue_poll_file).
    fn poll_readiness(&self) -> u16 {
        let s = self.state.lock();
        let mut r = 0;
        if s.count > 0 {
            r |= 0x0001;
        }
        if s.count < s.maxmsg {
            r |= 0x0004;
        }
        r
    }

    fn wakes_io_waiters(&self) -> bool {
        true
    }

    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(MQUEUE_MAGIC))
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

/// The caller's rights over IPC namespace `ns` (ADR 0024, ADR 0027), as
/// for System V objects: READ to open a queue, WRITE to send, receive or
/// register for notification, CREATE to make a queue, MODIFY to unlink
/// one.
fn rights(ns: &Arc<PlainNs>) -> super::sysvipc::NsRights {
    super::sysvipc::namespace_rights(ns)
}

fn space_of(ns: &PlainNs) -> &Arc<MqSpace> {
    ns.mq
        .as_ref()
        .expect("an IPC namespace has its message queues")
}

/// A queue's name as the kernel gets it (the library drops the leading
/// '/'): empty ENOENT, a '/' in it or "." or ".." EACCES, longer than
/// NAME_MAX ENAMETOOLONG, as the lookup in the mqueue root answers.
pub fn check_name(name: &str) -> Result<(), SyscallError> {
    if name.len() > NAME_MAX {
        return Err(SyscallError::NameTooLong);
    }
    if name.is_empty() {
        return Err(SyscallError::ResourceNotFound);
    }
    if name.contains('/') || name == "." || name == ".." {
        return Err(SyscallError::PermissionDenied);
    }
    Ok(())
}

/// The attributes a new queue gets (mqueue_get_inode): the defaults
/// without `attr`; otherwise both positive (EINVAL), within the
/// namespace's maxima, or with CAP_SYS_RESOURCE within the hard limits
/// (EINVAL), and their product within an unsigned long (EOVERFLOW).
pub fn new_attr(
    space: &MqSpace,
    attr: Option<(i64, i64)>,
    sys_resource: bool,
) -> Result<(usize, usize), SyscallError> {
    let r = Ordering::Relaxed;
    let Some((maxmsg, msgsize)) = attr else {
        return Ok((
            space.msg_default.load(r).max(1) as usize,
            space.msgsize_default.load(r).max(1) as usize,
        ));
    };
    if maxmsg <= 0 || msgsize <= 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (lim_msg, lim_size) = if sys_resource {
        (i64::from(HARD_MSGMAX), i64::from(HARD_MSGSIZEMAX))
    } else {
        (
            i64::from(space.msg_max.load(r)),
            i64::from(space.msgsize_max.load(r)),
        )
    };
    if maxmsg > lim_msg || msgsize > lim_size {
        return Err(SyscallError::InvalidArgument);
    }
    (msgsize as u64)
        .checked_mul(maxmsg as u64)
        .ok_or(SyscallError::Overflow)?;
    Ok((maxmsg as usize, msgsize as usize))
}

/// The bytes a queue charges (mq_bytes and its tree).
pub fn charge_of(maxmsg: usize, msgsize: usize) -> u64 {
    let m = maxmsg as u64;
    m * msgsize as u64 + m * MSG_OVERHEAD + m.min(u64::from(MQ_PRIO_MAX)) * TREE_NODE
}

/// Make queue `name` in IPC namespace `ns` for the caller
/// (mqueue_create_attr and mqueue_get_inode): CREATE on the namespace
/// (EACCES), queues_max (ENOSPC unless CAP_SYS_RESOURCE), the attributes
/// ([`new_attr`]), RLIMIT_MSGQUEUE for the caller's real user (EMFILE);
/// mode `mode` less the umask, owned by the caller's fsuid and fsgid.
/// EEXIST if another caller made it meanwhile (the new queue is dropped,
/// with its charge): mq_open then opens theirs through its access checks.
pub(crate) fn create_queue(
    ns: &Arc<PlainNs>,
    name: &str,
    mode: u32,
    attr: Option<(i64, i64)>,
) -> Result<Arc<MqNode>, SyscallError> {
    let space = space_of(ns).clone();
    let caller = Caller::current();
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    if !rights(ns).create {
        return Err(SyscallError::PermissionDenied);
    }
    let sys_resource = caller.ns_capable(&ns.owner, linux_caps::CAP_SYS_RESOURCE);
    // queues_max, checked and counted in one step (Linux holds its lock
    // over both), so concurrent creators cannot all pass the check (PR #40
    // review). The queue's drop gives the slot back; so does a failure
    // before the queue exists.
    let unlimited = caller.capable(linux_caps::CAP_SYS_RESOURCE);
    let max = space.queues_max.load(Ordering::Relaxed);
    space
        .count
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |c| {
            (c < max || unlimited).then_some(c + 1)
        })
        .map_err(|_| SyscallError::NoSpace)?;
    let unreserve = || {
        space.count.fetch_sub(1, Ordering::AcqRel);
    };
    let (maxmsg, msgsize) = new_attr(&space, attr, sys_resource).inspect_err(|_| unreserve())?;
    let bytes = charge_of(maxmsg, msgsize);
    let ruid = caller.creds.ruid;
    let limit = proc.limits().cur(crate::process::rlimit::RLIMIT_MSGQUEUE);
    {
        let mut c = CHARGED.lock();
        let used = c.entry(ruid).or_insert(0);
        match used.checked_add(bytes) {
            Some(t) if t <= limit => *used = t,
            _ => {
                unreserve();
                return Err(SyscallError::ResourceLimitExceeded);
            }
        }
    }
    let umask =
        crate::process::current_thread().map_or(0o022, |t| t.fs().umask.load(Ordering::Acquire));
    let now = crate::arch::timer::get_timestamp_secs();
    let node = Arc::new(MqNode {
        state: Mutex::new(State {
            maxmsg,
            msgsize,
            msgs: BTreeMap::new(),
            count: 0,
            qsize: 0,
            notify: None,
            receivers: 0,
        }),
        metadata: RwLock::new(Metadata {
            node_type: NodeType::File,
            size: 0,
            permissions: Permissions::from_mode(mode & 0o777 & !umask),
            uid: caller.creds.fsuid,
            gid: caller.creds.fsgid,
            created: now,
            modified: now,
            accessed: now,
            inode: crate::fs::ramfs::next_inode(),
        }),
        waiters: crate::sched::dispatch::WaitQueue::new(),
        charge: (ruid, bytes),
        space: Arc::downgrade(&space),
        ns: Arc::downgrade(ns),
    });
    let mut names = space.names.lock();
    // Another creator first: theirs stands.
    if names.contains_key(name) {
        Err(SyscallError::FileExists)
    } else {
        names.insert(String::from(name), node.clone());
        Ok(node)
    }
}

/// O_ACCMODE and the open flags mq_open reads.
const O_ACCMODE: i32 = 3;
const O_WRONLY: i32 = 1;
const O_RDWR: i32 = 2;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const O_NONBLOCK: i32 = 0o4000;
const O_CLOEXEC: i32 = 0o2000000;

/// mq_open(name, oflag, mode, attr).
pub fn sys_mq_open(name_ptr: usize, oflag: usize, mode: usize, attr_ptr: usize) -> SyscallResult {
    let attr = if attr_ptr != 0 {
        let a: [i64; 8] = super::userspace::read_user(attr_ptr)?;
        Some((a[1], a[2]))
    } else {
        None
    };
    let name = super::userspace::read_user_cstr(name_ptr, 4096)?;
    let oflag = oflag as i32;
    check_name(&name)?;
    let ns = super::sysvipc::current_ns();
    let space = space_of(&ns).clone();
    let caller = Caller::current();
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    // Linux holds the mqueue root's lock across the lookup and the create;
    // here a create that loses the race (EEXIST) looks again, so the queue
    // another caller made is opened only through the access checks below.
    let mut created = false;
    let node = loop {
        let existing = space.names.lock().get(&name).cloned();
        let Some(node) = existing else {
            if oflag & O_CREAT == 0 {
                return Err(SyscallError::ResourceNotFound);
            }
            // A mode no descriptor can have, checked before anything is
            // made, so a failing call leaves no queue (PR #40 review).
            if oflag & O_ACCMODE == O_ACCMODE {
                return Err(SyscallError::InvalidArgument);
            }
            match create_queue(&ns, &name, mode as u32, attr) {
                Err(SyscallError::FileExists) if oflag & O_EXCL == 0 => continue,
                r => {
                    created = true;
                    break r?;
                }
            }
        };
        if oflag & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL {
            return Err(SyscallError::FileExists);
        }
        if oflag & O_ACCMODE == O_ACCMODE {
            return Err(SyscallError::InvalidArgument);
        }
        let meta = node.metadata.read().clone();
        let want = match oflag & O_ACCMODE {
            O_WRONLY => crate::fs::MAY_WRITE,
            O_RDWR => crate::fs::MAY_READ | crate::fs::MAY_WRITE,
            _ => crate::fs::MAY_READ,
        };
        if !rights(&ns).read || !crate::fs::permission(&caller, &meta, want) {
            return Err(SyscallError::PermissionDenied);
        }
        break node;
    };
    if oflag & O_ACCMODE == O_ACCMODE {
        return Err(SyscallError::InvalidArgument);
    }
    let mut flags = match oflag & O_ACCMODE {
        O_WRONLY => crate::fs::OpenFlags::write_only(),
        O_RDWR => crate::fs::OpenFlags::read_write(),
        _ => crate::fs::OpenFlags::read_only(),
    };
    flags.nonblock = oflag & O_NONBLOCK != 0;
    let file =
        crate::fs::file::File::new_with_path(node.clone(), flags, alloc::format!("/{}", name));
    let fd = proc
        .file_table
        .lock()
        .open_with_flags(Arc::new(file), oflag & O_CLOEXEC != 0)
        .map_err(|e| {
            // No descriptor (EMFILE): the queue this call made goes again,
            // if the name still names it.
            if created {
                let mut names = space.names.lock();
                if names.get(&name).is_some_and(|n| Arc::ptr_eq(n, &node)) {
                    names.remove(&name);
                }
            }
            super::map_kernel_error(e)
        })?;
    Ok(fd)
}

/// mq_unlink(name): the name goes at once; the queue when its last
/// descriptor closes. Needs write access to the mqueue root (01777, so
/// anyone) and, the root being sticky, ownership or CAP_FOWNER (EPERM).
pub fn sys_mq_unlink(name_ptr: usize) -> SyscallResult {
    let name = super::userspace::read_user_cstr(name_ptr, 4096)?;
    check_name(&name)?;
    let ns = super::sysvipc::current_ns();
    if !rights(&ns).modify {
        return Err(SyscallError::OperationNotPermitted);
    }
    let space = space_of(&ns);
    let caller = Caller::current();
    let mut names = space.names.lock();
    let node = names.get(&name).ok_or(SyscallError::ResourceNotFound)?;
    let owner = node.metadata.read().uid;
    if caller.creds.fsuid != owner && !caller.capable(linux_caps::CAP_FOWNER) {
        return Err(SyscallError::OperationNotPermitted);
    }
    names.remove(&name);
    Ok(0)
}

/// The open file `fd` names, if it is a queue: EBADF for a closed
/// descriptor or another file.
fn queue_file(fd: usize) -> Result<Arc<crate::fs::file::File>, SyscallError> {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    mq(&file)?;
    Ok(file)
}

/// The queue an open file shows.
fn mq(file: &crate::fs::file::File) -> Result<&MqNode, SyscallError> {
    file.node
        .as_any()
        .and_then(|a| a.downcast_ref::<MqNode>())
        .ok_or(SyscallError::BadFileDescriptor)
}

/// An absolute CLOCK_REALTIME timeout as a monotonic deadline: EFAULT,
/// and EINVAL for a nanosecond count out of range or negative seconds.
fn deadline(ptr: usize) -> Result<Option<u64>, SyscallError> {
    if ptr == 0 {
        return Ok(None);
    }
    let ts: [i64; 2] = super::userspace::read_user(ptr)?;
    if ts[0] < 0 || !(0..1_000_000_000).contains(&ts[1]) {
        return Err(SyscallError::InvalidArgument);
    }
    let wall = ts[0].saturating_mul(1_000_000_000).saturating_add(ts[1]);
    Ok(Some(crate::timer::realtime::monotonic_at(wall)))
}

/// mq_timedsend(fd, msg, len, prio, abs_timeout) (do_mq_timedsend).
pub fn sys_mq_timedsend(
    fd: usize,
    msg_ptr: usize,
    len: usize,
    prio: usize,
    timeout: usize,
) -> SyscallResult {
    let deadline = deadline(timeout)?;
    if prio as u64 >= u64::from(MQ_PRIO_MAX) {
        return Err(SyscallError::InvalidArgument);
    }
    let file = queue_file(fd)?;
    let node = mq(&file)?;
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let msgsize = node.state.lock().msgsize;
    if len > msgsize {
        return Err(SyscallError::MessageTooLong);
    }
    let text = super::userspace::read_user_vec(msg_ptr, len, len)?;
    if !queue_rights(node).write {
        return Err(SyscallError::PermissionDenied);
    }
    let mut text = Some(text);
    let mut try_send = || -> bool {
        let mut s = node.state.lock();
        if s.count >= s.maxmsg {
            return false;
        }
        let Some(t) = text.take() else {
            return true;
        };
        let was_empty = s.count == 0;
        s.qsize += t.len();
        s.count += 1;
        s.msgs.entry(prio as u32).or_default().push_back(t);
        // __do_notify: an empty queue got a message and nobody waits.
        if was_empty && s.receivers == 0 {
            if let Some(n) = s.notify.take() {
                drop(s);
                notify(n);
            }
        }
        true
    };
    if !try_send() {
        if file.nonblock.load(Ordering::Acquire) {
            return Err(SyscallError::WouldBlock);
        }
        let waited = crate::sched::dispatch::wait_event(&node.waiters, deadline, &mut try_send);
        match waited {
            Ok(()) => {}
            Err(crate::sched::dispatch::WaitError::TimedOut) => return Err(SyscallError::TimedOut),
            Err(_) => return Err(SyscallError::Interrupted),
        }
    }
    node.metadata.write().modified = crate::arch::timer::get_timestamp_secs();
    node.changed();
    Ok(0)
}

/// Send notification `n`: SIGEV_SIGNAL queues its signal with si_code
/// SI_MESGQ, the sender's pid and uid as the owner sees them, and the
/// value; SIGEV_NONE sends nothing.
fn notify(n: Notify) {
    if n.kind != SIGEV_SIGNAL {
        return;
    }
    // Only to the process that registered, while it lives.
    let Some(owner) = n.owner.upgrade() else {
        return;
    };
    let same = crate::process::table::get_process(crate::process::ProcessId(n.pid))
        .is_some_and(|p| Arc::ptr_eq(&p, &owner));
    if !same
        || matches!(
            owner.get_state(),
            crate::process::ProcessState::Zombie | crate::process::ProcessState::Dead
        )
    {
        return;
    }
    let mut info = crate::process::signals::SigInfo::from_current(n.signo as usize, SI_MESGQ);
    info.fields[1] = n.value;
    let _ =
        crate::process::exit::kill_process_info(crate::process::ProcessId(n.pid), n.signo, info);
}

/// mq_timedreceive(fd, msg, len, prio_ptr, abs_timeout): the highest
/// priority message, the oldest of that priority.
pub fn sys_mq_timedreceive(
    fd: usize,
    msg_ptr: usize,
    len: usize,
    prio_ptr: usize,
    timeout: usize,
) -> SyscallResult {
    let deadline = deadline(timeout)?;
    let file = queue_file(fd)?;
    let node = mq(&file)?;
    if !file.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    if len < node.state.lock().msgsize {
        return Err(SyscallError::MessageTooLong);
    }
    if !queue_rights(node).write {
        return Err(SyscallError::PermissionDenied);
    }
    let take = || -> Option<(u32, Vec<u8>)> {
        let mut s = node.state.lock();
        let (&prio, q) = s.msgs.iter_mut().next_back()?;
        let t = q.pop_front()?;
        if q.is_empty() {
            s.msgs.remove(&prio);
        }
        s.count -= 1;
        s.qsize -= t.len();
        Some((prio, t))
    };
    let got = match take() {
        Some(m) => m,
        None if file.nonblock.load(Ordering::Acquire) => return Err(SyscallError::WouldBlock),
        None => {
            node.state.lock().receivers += 1;
            let mut got = None;
            let waited = crate::sched::dispatch::wait_event(&node.waiters, deadline, || {
                got = take();
                got.is_some()
            });
            node.state.lock().receivers -= 1;
            match waited {
                Ok(()) => got.ok_or(SyscallError::Interrupted)?,
                Err(crate::sched::dispatch::WaitError::TimedOut) => {
                    return Err(SyscallError::TimedOut)
                }
                Err(_) => return Err(SyscallError::Interrupted),
            }
        }
    };
    node.metadata.write().accessed = crate::arch::timer::get_timestamp_secs();
    node.changed();
    super::userspace::write_user_bytes(msg_ptr, &got.1)?;
    if prio_ptr != 0 {
        super::userspace::write_user(prio_ptr, got.0)?;
    }
    Ok(got.1.len())
}

/// mq_notify(fd, sevp) (do_mq_notify): NULL removes the caller's
/// registration; otherwise SIGEV_NONE or SIGEV_SIGNAL (a valid signal,
/// EINVAL) registers the caller, EBUSY if another process is registered.
/// SIGEV_THREAD names a netlink socket, which do not exist here: a
/// descriptor that is no socket is ENOTSOCK, any other ECONNREFUSED, as
/// Linux answers for a socket that is not a netlink one.
pub fn sys_mq_notify(fd: usize, sev_ptr: usize) -> SyscallResult {
    let sev = if sev_ptr != 0 {
        // struct sigevent: sigev_value (8), sigev_signo (4), sigev_notify (4).
        let raw: [u8; 16] = super::userspace::read_user(sev_ptr)?;
        let value = u64::from_ne_bytes(raw[0..8].try_into().unwrap_or([0; 8]));
        let signo = i32::from_ne_bytes(raw[8..12].try_into().unwrap_or([0; 4]));
        let kind = i32::from_ne_bytes(raw[12..16].try_into().unwrap_or([0; 4]));
        match kind {
            SIGEV_NONE => {}
            SIGEV_SIGNAL => {
                if !(1..=crate::process::signals::NSIG as i32).contains(&signo) {
                    return Err(SyscallError::InvalidArgument);
                }
            }
            SIGEV_THREAD => {
                let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
                let sock = proc
                    .file_table
                    .lock()
                    .get(signo as u32 as usize)
                    .ok_or(SyscallError::BadFileDescriptor)?;
                return Err(if sock.node.node_type() == NodeType::Socket {
                    SyscallError::ConnectionRefused
                } else {
                    SyscallError::NotASocket
                });
            }
            _ => return Err(SyscallError::InvalidArgument),
        }
        Some((kind, signo, value))
    } else {
        None
    };
    let file = queue_file(fd)?;
    let node = mq(&file)?;
    // A registration changes the queue (ADR 0027): WRITE.
    if !queue_rights(node).write {
        return Err(SyscallError::PermissionDenied);
    }
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let me = proc.pid.0;
    let mut s = node.state.lock();
    match sev {
        None => {
            if s.notify.as_ref().is_some_and(|n| n.pid == me) {
                s.notify = None;
            }
        }
        Some((kind, signo, value)) => {
            if s.notify.is_some() {
                return Err(SyscallError::Busy);
            }
            s.notify = Some(Notify {
                pid: me,
                owner: Arc::downgrade(&proc),
                kind,
                signo,
                value,
            });
        }
    }
    Ok(0)
}

/// mqueue_flush_file: the registered process closing a descriptor of the
/// queue (any, as on Linux) drops its notification. Called for every
/// descriptor closed (`FileTable::closing`); other files are left alone.
pub(crate) fn flush_file(file: &crate::fs::file::File) {
    let Ok(node) = mq(file) else {
        return;
    };
    let me = super::sysvipc::current_pid();
    let mut s = node.state.lock();
    if s.notify.as_ref().is_some_and(|n| n.pid == me) {
        s.notify = None;
    }
}

/// mq_getsetattr(fd, new, old): `old` gets mq_flags (O_NONBLOCK), the
/// attributes and the message count; `new` may change only O_NONBLOCK
/// (other flags EINVAL).
pub fn sys_mq_getsetattr(fd: usize, new_ptr: usize, old_ptr: usize) -> SyscallResult {
    let new = if new_ptr != 0 {
        let a: [i64; 8] = super::userspace::read_user(new_ptr)?;
        if a[0] & !i64::from(O_NONBLOCK) != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        Some(a[0])
    } else {
        None
    };
    let file = queue_file(fd)?;
    let node = mq(&file)?;
    let old = {
        let s = node.state.lock();
        let flags = if file.nonblock.load(Ordering::Acquire) {
            i64::from(O_NONBLOCK)
        } else {
            0
        };
        [
            flags,
            s.maxmsg as i64,
            s.msgsize as i64,
            s.count as i64,
            0,
            0,
            0,
            0,
        ]
    };
    if let Some(f) = new {
        file.nonblock
            .store(f & i64::from(O_NONBLOCK) != 0, Ordering::Release);
    }
    if old_ptr != 0 {
        super::userspace::write_user(old_ptr, old)?;
    }
    Ok(0)
}

/// The `/proc/sys/fs/mqueue` files.
pub const SYSCTLS: [&str; 5] = [
    "msg_default",
    "msg_max",
    "msgsize_default",
    "msgsize_max",
    "queues_max",
];

fn sysctl_of<'a>(space: &'a MqSpace, name: &str) -> Option<(&'a AtomicI32, i32, i32)> {
    Some(match name {
        "queues_max" => (&space.queues_max, 0, i32::MAX),
        "msg_max" => (&space.msg_max, MIN_MSGMAX, HARD_MSGMAX),
        "msg_default" => (&space.msg_default, MIN_MSGMAX, HARD_MSGMAX),
        "msgsize_max" => (&space.msgsize_max, MIN_MSGSIZEMAX, HARD_MSGSIZEMAX),
        "msgsize_default" => (&space.msgsize_default, MIN_MSGSIZEMAX, HARD_MSGSIZEMAX),
        _ => return None,
    })
}

/// A `/proc/sys/fs/mqueue` file of the reader's namespace.
pub fn sysctl_text(name: &str) -> String {
    let ns = super::sysvipc::current_ns();
    sysctl_of(space_of(&ns), name).map_or_else(String::new, |(v, _, _)| {
        alloc::format!("{}\n", v.load(Ordering::Relaxed))
    })
}

/// Write one (EINVAL outside its range; queues_max has none but 0).
pub fn sysctl_write(name: &str, text: &[u8]) -> Result<(), SyscallError> {
    let ns = super::sysvipc::current_ns();
    let (v, lo, hi) = sysctl_of(space_of(&ns), name).ok_or(SyscallError::InvalidArgument)?;
    let n = core::str::from_utf8(text)
        .ok()
        .and_then(|t| t.trim().parse::<i32>().ok())
        .filter(|n| (lo..=hi).contains(n))
        .ok_or(SyscallError::InvalidArgument)?;
    v.store(n, Ordering::Relaxed);
    Ok(())
}

/// The mqueue filesystem (Linux's `mqueue` type, usually on /dev/mqueue):
/// its root lists the queues of the IPC namespace it was mounted in.
pub struct MqueueFs {
    root: Arc<MqRoot>,
}

impl MqueueFs {
    /// A filesystem showing the current thread's IPC namespace's queues,
    /// its root sticky and writable by all (S_ISVTX | 0777), owned by that
    /// namespace's root.
    pub fn new() -> Self {
        let ns = super::sysvipc::current_ns();
        let now = crate::arch::timer::get_timestamp_secs();
        let root_uid = ns.owner.root_uid();
        let root_gid = ns.owner.gid_down(0).unwrap_or(0);
        Self {
            root: Arc::new(MqRoot {
                ns,
                metadata: RwLock::new(Metadata {
                    node_type: NodeType::Directory,
                    size: 0,
                    permissions: Permissions::from_mode(0o1777),
                    uid: root_uid,
                    gid: root_gid,
                    created: now,
                    modified: now,
                    accessed: now,
                    inode: crate::fs::ramfs::next_inode(),
                }),
            }),
        }
    }
}

impl Default for MqueueFs {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::fs::Filesystem for MqueueFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone()
    }

    fn name(&self) -> &str {
        "mqueue"
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<(), KernelError> {
        Ok(())
    }
}

/// The mqueue filesystem's root directory.
struct MqRoot {
    ns: Arc<PlainNs>,
    metadata: RwLock<Metadata>,
}

impl VfsNode for MqRoot {
    fn node_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::IsADirectory))
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::IsADirectory))
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(self.metadata.read().clone())
    }

    fn readdir(&self) -> Result<Vec<crate::fs::DirEntry>, KernelError> {
        // Listing the queues looks at the namespace: READ (ADR 0027).
        if !rights(&self.ns).read {
            return Err(KernelError::FsError(
                crate::error::FsError::PermissionDenied,
            ));
        }
        Ok(space_of(&self.ns)
            .names
            .lock()
            .iter()
            .map(|(name, node)| crate::fs::DirEntry {
                name: name.clone(),
                node_type: NodeType::File,
                inode: node.metadata.read().inode,
            })
            .collect())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        // Opening a queue through the mount is opening it: READ, as
        // mq_open checks (PR #40 review).
        if !rights(&self.ns).read {
            return Err(KernelError::FsError(
                crate::error::FsError::PermissionDenied,
            ));
        }
        space_of(&self.ns)
            .names
            .lock()
            .get(name)
            .map(|n| n.clone() as Arc<dyn VfsNode>)
            .ok_or(KernelError::FsError(crate::error::FsError::NotFound))
    }

    /// open(O_CREAT) here makes a queue with the default attributes
    /// (mqueue_create).
    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        check_name(name)
            .map_err(|_| KernelError::FsError(crate::error::FsError::PermissionDenied))?;
        create_queue(&self.ns, name, permissions.to_mode() & 0o777, None)
            .map(|n| n as Arc<dyn VfsNode>)
            .map_err(|e| match e {
                SyscallError::FileExists => {
                    KernelError::FsError(crate::error::FsError::AlreadyExists)
                }
                SyscallError::NoSpace => KernelError::FsError(crate::error::FsError::NoSpace),
                // RLIMIT_MSGQUEUE: EMFILE, as mq_open reports it.
                SyscallError::ResourceLimitExceeded => {
                    KernelError::FsError(crate::error::FsError::TooManyOpenFiles)
                }
                _ => KernelError::FsError(crate::error::FsError::PermissionDenied),
            })
    }

    /// No directories in it (no mkdir operation: EPERM).
    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(
            crate::error::FsError::OperationNotPermitted,
        ))
    }

    /// rm of a queue is mq_unlink: MODIFY on the namespace, and the sticky
    /// root's rule (the owner or CAP_FOWNER, EPERM).
    fn unlink(&self, name: &str) -> Result<(), KernelError> {
        let eperm = KernelError::FsError(crate::error::FsError::OperationNotPermitted);
        if !rights(&self.ns).modify {
            return Err(eperm);
        }
        let caller = Caller::current();
        let mut names = space_of(&self.ns).names.lock();
        let node = names
            .get(name)
            .ok_or(KernelError::FsError(crate::error::FsError::NotFound))?;
        let owner = node.metadata.read().uid;
        if caller.creds.fsuid != owner && !caller.capable(linux_caps::CAP_FOWNER) {
            return Err(eperm);
        }
        names.remove(name);
        Ok(())
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::IsADirectory))
    }

    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(MQUEUE_MAGIC))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked_as_the_mqueue_root_looks_them_up() {
        assert_eq!(check_name("q"), Ok(()));
        assert_eq!(check_name(""), Err(SyscallError::ResourceNotFound));
        assert_eq!(check_name("a/b"), Err(SyscallError::PermissionDenied));
        assert_eq!(check_name(".."), Err(SyscallError::PermissionDenied));
        let long = "x".repeat(NAME_MAX + 1);
        assert_eq!(check_name(&long), Err(SyscallError::NameTooLong));
    }

    #[test]
    fn new_queues_take_the_defaults_or_checked_attributes() {
        let space = MqSpace::new();
        assert_eq!(new_attr(&space, None, false), Ok((10, 8192)));
        assert_eq!(
            new_attr(&space, Some((0, 10)), false),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            new_attr(&space, Some((11, 10)), false),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(new_attr(&space, Some((11, 10)), true), Ok((11, 10)));
        assert_eq!(
            new_attr(&space, Some(((HARD_MSGMAX + 1) as i64, 10)), true),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(new_attr(&space, Some((5, 100)), false), Ok((5, 100)));
        assert_eq!(charge_of(10, 8192), 10 * 8192 + 10 * 48 + 10 * 48);
    }

    #[test]
    fn sysctls_keep_linux_ranges() {
        let space = MqSpace::new();
        let (v, lo, hi) = sysctl_of(&space, "msgsize_max").unwrap();
        assert_eq!(
            (v.load(Ordering::Relaxed), lo, hi),
            (8192, 128, 16 * 1024 * 1024)
        );
        assert!(sysctl_of(&space, "nope").is_none());
    }
}

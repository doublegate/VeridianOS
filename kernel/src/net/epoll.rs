//! epoll I/O multiplexing (N-234, N-245).
//!
//! An epoll instance lives in its file ([`EpollNode`]), so every descriptor
//! for it -- a dup, a fork child's copy -- shares one interest list, which
//! goes away with the last of them.
//!
//! Registrations follow Linux: one is keyed by the descriptor number *and*
//! the open file it named, and holds that file weakly. It goes away when the
//! open file does (its last descriptor is closed), not when the number is
//! closed while a duplicate keeps the file open -- events then keep arriving,
//! and `EPOLL_CTL_DEL` through the closed number is EBADF, as on Linux.
//!
//! A level-triggered registration reports while its file is ready. An
//! edge-triggered one (EPOLLET) reports when readiness bits appear, and with
//! the same bits again only after the file may have changed: for files that
//! wake I/O waiters ([`VfsNode::wakes_io_waiters`]), after any I/O event
//! since its last report (the kernel has no per-file wake-ups yet, so this
//! can report once more than Linux would; edge-triggered users read until
//! EAGAIN and tolerate that), and for the others after the periodic re-check
//! interval. EPOLLONESHOT disables a registration once it reports, until
//! `EPOLL_CTL_MOD` re-arms it.
//!
//! An epoll file can watch another one (it is readable while the other has
//! events); a registration that would make a cycle, or nest more than
//! [`MAX_NESTS`] deep, is ELOOP.

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};

use spin::Mutex;

use crate::{
    error::KernelError,
    fs::{file::File, DirEntry, Metadata, NodeType, Permissions, VfsNode},
};

/// Registrations one epoll instance may hold (Linux limits watches per user
/// through `max_user_watches`; past it, ENOSPC).
const MAX_WATCHES: usize = 65_536;

/// Deepest chain of epoll files watching epoll files (Linux EP_MAX_NESTS).
pub const MAX_NESTS: usize = 4;

/// Re-check interval for files that do not wake waiters (the generic wait's).
#[cfg(feature = "alloc")]
const RECHECK_NS: u64 = crate::sched::dispatch::IO_RECHECK_NS;
#[cfg(not(feature = "alloc"))]
const RECHECK_NS: u64 = 10_000_000;

// ============================================================================
// Event flags and operations (Linux <sys/epoll.h>)
// ============================================================================

/// Available for read.
pub const EPOLLIN: u32 = 0x001;
/// Urgent data.
pub const EPOLLPRI: u32 = 0x002;
/// Available for write.
pub const EPOLLOUT: u32 = 0x004;
/// Error condition (always reported).
pub const EPOLLERR: u32 = 0x008;
/// Hang up (always reported).
pub const EPOLLHUP: u32 = 0x010;
/// Normal data readable.
pub const EPOLLRDNORM: u32 = 0x040;
/// Priority band data readable.
pub const EPOLLRDBAND: u32 = 0x080;
/// Normal data writable.
pub const EPOLLWRNORM: u32 = 0x100;
/// Priority band data writable.
pub const EPOLLWRBAND: u32 = 0x200;
/// Unused by Linux, accepted.
pub const EPOLLMSG: u32 = 0x400;
/// Peer closed its writing end.
pub const EPOLLRDHUP: u32 = 0x2000;
/// Wake one of several epoll instances waiting on one file.
pub const EPOLLEXCLUSIVE: u32 = 1 << 28;
/// Hold a wakeup source (needs CAP_BLOCK_SUSPEND; ignored).
pub const EPOLLWAKEUP: u32 = 1 << 29;
/// Disable the registration after one report.
pub const EPOLLONESHOT: u32 = 1 << 30;
/// Edge-triggered.
pub const EPOLLET: u32 = 1 << 31;

/// Readiness bits a registration can report (`poll_readiness` uses the
/// same values).
const READINESS: u32 = EPOLLIN
    | EPOLLPRI
    | EPOLLOUT
    | EPOLLERR
    | EPOLLHUP
    | EPOLLRDNORM
    | EPOLLRDBAND
    | EPOLLWRNORM
    | EPOLLWRBAND
    | EPOLLMSG
    | EPOLLRDHUP;

/// What an EPOLLEXCLUSIVE registration may also ask for.
const EXCLUSIVE_OK: u32 =
    EPOLLIN | EPOLLOUT | EPOLLERR | EPOLLHUP | EPOLLWAKEUP | EPOLLET | EPOLLEXCLUSIVE;

/// `epoll_create1` flag.
pub const EPOLL_CLOEXEC: u32 = 0x80000;

/// Add fd to interest list.
pub const EPOLL_CTL_ADD: u32 = 1;
/// Remove fd from interest list.
pub const EPOLL_CTL_DEL: u32 = 2;
/// Modify events for an fd.
pub const EPOLL_CTL_MOD: u32 = 3;

/// Event structure passed to/from user space (matches Linux struct
/// epoll_event).
///
/// Linux's `struct epoll_event` is `__attribute__((packed))` so it uses
/// 4-byte alignment (from the leading `u32 events` field) rather than the
/// natural 8-byte alignment that the `u64 data` field would impose. We must
/// match this layout to accept user-space pointers at 4-byte-aligned
/// addresses.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EpollEvent {
    /// Event flags (EPOLLIN, EPOLLOUT, etc.)
    pub events: u32,
    /// User data (typically the fd or a pointer)
    pub data: u64,
}

/// Why `epoll_ctl` refused (the syscall layer maps these to Linux errnos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtlError {
    /// EINVAL: unknown op, the epoll file itself, a bad EPOLLEXCLUSIVE use.
    Invalid,
    /// EEXIST: `EPOLL_CTL_ADD` of a registered (fd, file).
    Exists,
    /// ENOENT: `EPOLL_CTL_MOD`/`DEL` of an unregistered (fd, file).
    NotFound,
    /// EPERM: the file does not support readiness (a regular file or
    /// directory).
    NotPollable,
    /// ELOOP: the registration would make a cycle or nest too deep.
    Loop,
    /// ENOSPC: the instance holds [`MAX_WATCHES`] registrations.
    NoSpace,
}

// ============================================================================
// Registrations
// ============================================================================

/// One registration.
struct Interest {
    /// The open file the descriptor named when it was added.
    file: Weak<File>,
    /// Requested readiness bits, EPOLLERR and EPOLLHUP included.
    events: u32,
    /// EPOLLET, EPOLLONESHOT and EPOLLEXCLUSIVE as requested.
    mode: u32,
    /// Returned with each event.
    data: u64,
    /// A one-shot registration that has reported.
    disabled: bool,
    /// Edge-triggered state: the bits last reported, the I/O event count
    /// and the time at that report.
    last: u32,
    last_seq: u64,
    last_ns: u64,
}

impl Interest {
    fn new(file: &Arc<File>, event: &EpollEvent) -> Self {
        let mut interest = Self {
            file: Arc::downgrade(file),
            events: 0,
            mode: 0,
            data: 0,
            disabled: false,
            last: 0,
            last_seq: 0,
            last_ns: 0,
        };
        interest.set(event);
        interest
    }

    /// Apply `event` (ADD or MOD): re-armed, with no edge history.
    fn set(&mut self, event: &EpollEvent) {
        let requested = event.events;
        self.events = (requested & READINESS) | EPOLLERR | EPOLLHUP;
        self.mode = requested & (EPOLLET | EPOLLONESHOT | EPOLLEXCLUSIVE);
        self.data = event.data;
        self.disabled = false;
        self.last = 0;
    }
}

/// A registration's key: the descriptor number and the open file.
type Key = (i32, usize);

fn key(fd: i32, file: &Arc<File>) -> Key {
    (fd, Arc::as_ptr(file) as *const () as usize)
}

/// Whether an edge-triggered registration reports `ready` (non-empty) now:
/// bits it did not report last time appeared, or -- the same bits -- the
/// file may have changed since (an I/O event for files that announce
/// theirs, the re-check interval for the others).
fn edge_reports(
    last: u32,
    last_seq: u64,
    last_ns: u64,
    ready: u32,
    seq: u64,
    now_ns: u64,
    announces: bool,
) -> bool {
    if ready & !last != 0 {
        return true;
    }
    if announces {
        seq != last_seq
    } else {
        now_ns.saturating_sub(last_ns) >= RECHECK_NS
    }
}

/// Readiness of an open file, as epoll bits.
fn readiness(file: &File) -> u32 {
    file.node.poll_readiness() as u32 & READINESS
}

fn io_seq() -> u64 {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::io_seq()
    }
    #[cfg(not(feature = "alloc"))]
    {
        0
    }
}

fn now_ns() -> u64 {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::clock_ns()
    }
    #[cfg(not(feature = "alloc"))]
    {
        crate::timer::get_uptime_ms().saturating_mul(1_000_000)
    }
}

/// What a scan found: events written, whether every watched file announces
/// its changes, and the earliest time one becomes ready by itself (for the
/// wait, [`crate::sched::dispatch::wait_io`]).
struct Scan {
    count: usize,
    precise: bool,
    wake_at: Option<u64>,
}

// ============================================================================
// The epoll file
// ============================================================================

/// An epoll instance, as the file `epoll_create1` returns.
pub struct EpollNode {
    interest: Mutex<BTreeMap<Key, Interest>>,
    /// The epoll files that registered this one (weakly; pruned as they
    /// go or stop watching), for the nesting limit on the path above it.
    watchers: Mutex<Vec<Weak<File>>>,
}

/// Serialises adding epoll files to epoll files, so two concurrent adds
/// cannot build a cycle the checks each miss (Linux's epnested_mutex).
static NEST_LOCK: Mutex<()> = Mutex::new(());

impl Default for EpollNode {
    fn default() -> Self {
        Self::new()
    }
}

impl EpollNode {
    pub fn new() -> Self {
        Self {
            interest: Mutex::new(BTreeMap::new()),
            watchers: Mutex::new(Vec::new()),
        }
    }

    /// The epoll instance behind `file`, if it is one.
    pub fn of(file: &File) -> Option<&EpollNode> {
        file.node.as_any()?.downcast_ref::<EpollNode>()
    }

    /// `epoll_ctl(op, fd, event)` on the epoll file `this`, where `fd` names
    /// `target`. `event` is required for ADD and MOD (the caller has read it
    /// from user memory). EINVAL if `this` is not an epoll file.
    pub fn ctl(
        this: &Arc<File>,
        op: u32,
        fd: i32,
        target: &Arc<File>,
        event: Option<&EpollEvent>,
    ) -> Result<(), CtlError> {
        let ep = Self::of(this).ok_or(CtlError::Invalid)?;
        ep.ctl_on(this, op, fd, target, event)
    }

    fn ctl_on(
        &self,
        this: &Arc<File>,
        op: u32,
        fd: i32,
        target: &Arc<File>,
        event: Option<&EpollEvent>,
    ) -> Result<(), CtlError> {
        if matches!(
            target.node.node_type(),
            NodeType::File | NodeType::Directory
        ) {
            return Err(CtlError::NotPollable);
        }
        let nested = Self::of(target);
        if nested.is_some_and(|n| core::ptr::eq(n, self)) {
            return Err(CtlError::Invalid);
        }
        let mut event = match (op, event) {
            (EPOLL_CTL_ADD | EPOLL_CTL_MOD, Some(event)) => Some(*event),
            (EPOLL_CTL_ADD | EPOLL_CTL_MOD, None) => return Err(CtlError::Invalid),
            (EPOLL_CTL_DEL, _) => None,
            _ => return Err(CtlError::Invalid),
        };
        if let Some(event) = event.as_mut() {
            event.events &= !EPOLLWAKEUP;
            let requested = event.events;
            if requested & EPOLLEXCLUSIVE != 0
                && (op == EPOLL_CTL_MOD || nested.is_some() || requested & !EXCLUSIVE_OK != 0)
            {
                return Err(CtlError::Invalid);
            }
        }

        let key = key(fd, target);
        let _nesting = (op == EPOLL_CTL_ADD && nested.is_some()).then(|| NEST_LOCK.lock());
        if let (EPOLL_CTL_ADD, Some(inner)) = (op, nested) {
            // The new edge self -> inner: inner must not reach self, and the
            // longest path through it -- the epoll files above self, the
            // edge, the ones below inner -- must stay within MAX_NESTS
            // (Linux's ep_loop_check and reverse_path_check).
            let above = self.height_above(this, MAX_NESTS);
            let below = inner.depth_avoiding(self, MAX_NESTS);
            match (above, below) {
                (Some(a), Some(b)) if a + 1 + b <= MAX_NESTS => {}
                _ => return Err(CtlError::Loop),
            }
        }

        let mut interest = self.interest.lock();
        match (op, event) {
            (EPOLL_CTL_ADD, Some(event)) => {
                if interest.contains_key(&key) {
                    return Err(CtlError::Exists);
                }
                if interest.len() >= MAX_WATCHES {
                    interest.retain(|_, i| i.file.strong_count() > 0);
                    if interest.len() >= MAX_WATCHES {
                        return Err(CtlError::NoSpace);
                    }
                }
                interest.insert(key, Interest::new(target, &event));
            }
            (EPOLL_CTL_MOD, Some(event)) => {
                let entry = interest.get_mut(&key).ok_or(CtlError::NotFound)?;
                if entry.mode & EPOLLEXCLUSIVE != 0 {
                    return Err(CtlError::Invalid);
                }
                entry.set(&event);
            }
            _ => {
                interest.remove(&key).ok_or(CtlError::NotFound)?;
            }
        }
        drop(interest);
        if let (EPOLL_CTL_ADD, Some(inner)) = (op, nested) {
            inner.add_watcher(target, this);
        }
        // A waiter on this instance may now have something to report.
        #[cfg(feature = "alloc")]
        crate::sched::dispatch::io_event();
        Ok(())
    }

    /// How many epoll levels hang below this one (0: it watches no epoll
    /// file), or `None` if `avoid` is among them or the chain is deeper
    /// than `limit`.
    fn depth_avoiding(&self, avoid: &EpollNode, limit: usize) -> Option<usize> {
        if core::ptr::eq(self, avoid) {
            return None;
        }
        let inner: Vec<Arc<File>> = self
            .interest
            .lock()
            .values()
            .filter_map(|i| i.file.upgrade())
            .filter(|f| Self::of(f).is_some())
            .collect();
        let mut depth = 0;
        for file in &inner {
            let nested = Self::of(file)?;
            if limit == 0 {
                return None;
            }
            depth = depth.max(1 + nested.depth_avoiding(avoid, limit - 1)?);
        }
        Some(depth)
    }

    /// Whether a registration of this instance names the open file `file`.
    fn watches(&self, file: &File) -> bool {
        self.interest
            .lock()
            .values()
            .any(|i| core::ptr::eq(i.file.as_ptr(), file))
    }

    /// Record that the epoll file `watcher` registered this one (whose file
    /// is `me`), dropping watchers that went or no longer watch it, so the
    /// list stays bounded by the live ones.
    fn add_watcher(&self, me: &Arc<File>, watcher: &Arc<File>) {
        let mut watchers = self.watchers.lock();
        watchers.retain(|w| {
            w.upgrade()
                .is_some_and(|f| Self::of(&f).is_some_and(|parent| parent.watches(me)))
        });
        if !watchers
            .iter()
            .any(|w| core::ptr::eq(w.as_ptr(), Arc::as_ptr(watcher)))
        {
            watchers.push(Arc::downgrade(watcher));
        }
    }

    /// How many epoll levels sit above this one (whose file is `me`): 0 if
    /// no epoll file watches it, `None` past `limit`.
    fn height_above(&self, me: &Arc<File>, limit: usize) -> Option<usize> {
        let parents: Vec<Arc<File>> = self
            .watchers
            .lock()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let mut height = 0;
        for file in &parents {
            let Some(parent) = Self::of(file) else {
                continue;
            };
            if !parent.watches(me) {
                continue;
            }
            if limit == 0 {
                return None;
            }
            height = height.max(1 + parent.height_above(file, limit - 1)?);
        }
        Some(height)
    }

    /// Look at every registration: write the events due into `out` (all
    /// of them while `out` has room) and, if `collect`, record that they
    /// were reported (edge and one-shot state). Registrations whose file
    /// is gone are dropped.
    fn scan(&self, out: &mut [EpollEvent], collect: bool, seq: u64, now: u64) -> Scan {
        self.scan_at(out, collect, seq, now, 0)
    }

    /// [`Self::scan`] of an instance `depth` epoll levels down. A watched
    /// epoll file is looked at in one pass at the next depth, and nothing
    /// is looked at past [`MAX_NESTS`], so the recursion is bounded whatever
    /// the registrations (ctl refuses deeper nesting anyway).
    fn scan_at(
        &self,
        out: &mut [EpollEvent],
        collect: bool,
        seq: u64,
        now: u64,
        depth: usize,
    ) -> Scan {
        let mut scan = Scan {
            count: 0,
            precise: true,
            wake_at: None,
        };
        let mut interest = self.interest.lock();
        interest.retain(|_, entry| {
            let Some(file) = entry.file.upgrade() else {
                return false;
            };
            if entry.disabled {
                return true;
            }
            let (bits, announces, ready_at) = match Self::of(&file) {
                Some(_) if depth >= MAX_NESTS => (0, true, None),
                Some(nested) => {
                    let mut one = [EpollEvent { events: 0, data: 0 }];
                    let sub = nested.scan_at(&mut one, false, seq, now, depth + 1);
                    let bits = if sub.count > 0 { EPOLLIN } else { 0 };
                    (bits, sub.precise, sub.wake_at)
                }
                None => (
                    readiness(&file),
                    file.node.wakes_io_waiters(),
                    file.node.ready_at_ns(),
                ),
            };
            scan.precise &= announces;
            scan.wake_at = [scan.wake_at, ready_at].into_iter().flatten().min();
            if scan.count >= out.len() {
                return true;
            }
            let ready = bits & entry.events;
            let edge = entry.mode & EPOLLET != 0;
            if collect {
                // Bits that went away count as new when they come back.
                entry.last &= ready;
            }
            if ready == 0 {
                return true;
            }
            let report = !edge
                || edge_reports(
                    entry.last,
                    entry.last_seq,
                    entry.last_ns,
                    ready,
                    seq,
                    now,
                    announces,
                );
            if report {
                out[scan.count] = EpollEvent {
                    events: ready,
                    data: entry.data,
                };
                scan.count += 1;
                if collect {
                    entry.last = ready;
                    entry.last_seq = seq;
                    entry.last_ns = now;
                    entry.disabled = entry.mode & EPOLLONESHOT != 0;
                }
            }
            true
        });
        scan
    }

    /// `epoll_wait`: up to `events.len()` events, waiting up to `timeout_ns`
    /// (`None`: no limit, `Some(0)`: do not wait). An interrupting signal
    /// is `WouldBlock` (the caller reports EINTR).
    pub fn wait(
        &self,
        events: &mut [EpollEvent],
        timeout_ns: Option<u64>,
    ) -> Result<usize, KernelError> {
        // Boot-path cooperative dispatch: when a child thread is dispatched
        // from boot_futex_spin, the syscall handler yields back to the parent
        // after each syscall, so waiting here would block every other thread:
        // a single pass.
        #[cfg(target_arch = "x86_64")]
        let in_boot_coop = crate::arch::x86_64::usermode::BOOT_CLONE_YIELD_PENDING
            .load(core::sync::atomic::Ordering::Acquire);
        #[cfg(not(target_arch = "x86_64"))]
        let in_boot_coop = false;
        let timeout_ns = if in_boot_coop { Some(0) } else { timeout_ns };

        // A dispatched waiter sleeps between scans until a watched file
        // reports a change, becomes ready by itself (a timerfd's expiry),
        // or -- if one does not report its changes yet -- a periodic
        // re-check.
        #[cfg(feature = "alloc")]
        let dispatched = crate::sched::dispatch::current_owner().is_some();
        let start = now_ns();
        let deadline = timeout_ns.map(|t| start.saturating_add(t));

        loop {
            let seq = io_seq();
            let scan = self.scan(events, true, seq, now_ns());
            if scan.count > 0 || timeout_ns == Some(0) {
                return Ok(scan.count);
            }

            #[cfg(feature = "alloc")]
            if dispatched {
                use crate::sched::dispatch::{wait_io, WaitError};
                match wait_io(seq, deadline, scan.precise, scan.wake_at) {
                    Ok(()) => continue,
                    Err(WaitError::TimedOut) => return Ok(0),
                    Err(WaitError::Interrupted) => return Err(KernelError::WouldBlock),
                }
            }

            // The boot context (no dispatcher): an infinite wait is capped
            // at 30 s so a stuck program cannot hang boot.
            let limit = deadline.unwrap_or(start.saturating_add(30_000_000_000));
            if now_ns() >= limit {
                return Ok(0);
            }
            // `sti; hlt` lets the timer interrupt advance the clock (SFMASK
            // cleared IF on syscall entry), then interrupts are off again.
            if crate::sched::wait_for_interrupt_in_syscall() {
                return Err(KernelError::WouldBlock);
            }
        }
    }

    /// Number of registrations (whose files may since have gone).
    pub fn len(&self) -> usize {
        self.interest.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl VfsNode for EpollNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "an epoll file cannot be read",
        })
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "an epoll file cannot be written",
        })
    }

    /// Readable while a wait would return events.
    fn poll_readiness(&self) -> u16 {
        let mut one = [EpollEvent { events: 0, data: 0 }];
        if self.scan(&mut one, false, io_seq(), now_ns()).count > 0 {
            EPOLLIN as u16
        } else {
            0
        }
    }

    fn wakes_io_waiters(&self) -> bool {
        let mut none: [EpollEvent; 0] = [];
        self.scan(&mut none, false, io_seq(), now_ns()).precise
    }

    fn ready_at_ns(&self) -> Option<u64> {
        let mut none: [EpollEvent; 0] = [];
        self.scan(&mut none, false, io_seq(), now_ns()).wake_at
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
            permissions: Permissions::from_mode(0o600),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
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
            value: "an epoll file cannot be truncated",
        })
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

    use super::*;
    use crate::fs::OpenFlags;

    /// A file whose readiness the test sets.
    struct Probe {
        ready: AtomicU16,
        announces: AtomicBool,
        kind: NodeType,
    }

    impl Probe {
        fn file(kind: NodeType, announces: bool) -> (Arc<File>, Arc<Probe>) {
            let probe = Arc::new(Probe {
                ready: AtomicU16::new(0),
                announces: AtomicBool::new(announces),
                kind,
            });
            let file = Arc::new(File::new(probe.clone(), OpenFlags::read_write()));
            (file, probe)
        }

        fn set(&self, bits: u32) {
            self.ready.store(bits as u16, Ordering::SeqCst);
        }
    }

    impl VfsNode for Probe {
        fn node_type(&self) -> NodeType {
            self.kind
        }
        fn read(&self, _: usize, _: &mut [u8]) -> Result<usize, KernelError> {
            Ok(0)
        }
        fn write(&self, _: usize, d: &[u8]) -> Result<usize, KernelError> {
            Ok(d.len())
        }
        fn poll_readiness(&self) -> u16 {
            self.ready.load(Ordering::SeqCst)
        }
        fn wakes_io_waiters(&self) -> bool {
            self.announces.load(Ordering::SeqCst)
        }
        fn metadata(&self) -> Result<Metadata, KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
        fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
        fn lookup(&self, _: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
        fn create(&self, _: &str, _: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
        fn mkdir(&self, _: &str, _: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
        fn unlink(&self, _: &str) -> Result<(), KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
        fn truncate(&self, _: usize) -> Result<(), KernelError> {
            Err(KernelError::NotImplemented { feature: "probe" })
        }
    }

    fn ev(events: u32, data: u64) -> EpollEvent {
        EpollEvent { events, data }
    }

    fn epoll_file() -> Arc<File> {
        Arc::new(File::new(
            Arc::new(EpollNode::new()),
            OpenFlags::read_write(),
        ))
    }

    /// Collect what a non-blocking scan reports at I/O count `seq`.
    fn reported(ep: &EpollNode, seq: u64, now: u64) -> Vec<(u32, u64)> {
        let mut out = [ev(0, 0); 8];
        let n = ep.scan(&mut out, true, seq, now).count;
        out[..n].iter().map(|e| (e.events, e.data)).collect()
    }

    #[test]
    fn ctl_errors_follow_linux() {
        let epf = epoll_file();
        let ep = EpollNode::of(&epf).unwrap();
        let (sock, _) = Probe::file(NodeType::Socket, true);
        let (regular, _) = Probe::file(NodeType::File, true);
        let (dir, _) = Probe::file(NodeType::Directory, true);
        let add = Some(&ev(EPOLLIN, 1));
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_ADD, 3, &regular, add),
            Err(CtlError::NotPollable)
        );
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_ADD, 3, &dir, add),
            Err(CtlError::NotPollable)
        );
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_MOD, 4, &sock, add),
            Err(CtlError::NotFound)
        );
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_DEL, 4, &sock, None),
            Err(CtlError::NotFound)
        );
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_ADD, 4, &sock, None),
            Err(CtlError::Invalid)
        );
        assert_eq!(
            EpollNode::ctl(&epf, 9, 4, &sock, add),
            Err(CtlError::Invalid)
        );
        assert_eq!(EpollNode::ctl(&epf, EPOLL_CTL_ADD, 4, &sock, add), Ok(()));
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_ADD, 4, &sock, add),
            Err(CtlError::Exists)
        );
        // Another number for the same file is another registration.
        assert_eq!(EpollNode::ctl(&epf, EPOLL_CTL_ADD, 5, &sock, add), Ok(()));
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_MOD, 4, &sock, Some(&ev(EPOLLOUT, 2))),
            Ok(())
        );
        assert_eq!(EpollNode::ctl(&epf, EPOLL_CTL_DEL, 4, &sock, None), Ok(()));
        assert_eq!(ep.len(), 1);

        // EPOLLEXCLUSIVE: ADD only, only with in/out/err/hup/et bits.
        let (other, _) = Probe::file(NodeType::Pipe, true);
        let excl = ev(EPOLLIN | EPOLLEXCLUSIVE, 0);
        assert_eq!(
            EpollNode::ctl(
                &epf,
                EPOLL_CTL_ADD,
                6,
                &other,
                Some(&ev(EPOLLPRI | EPOLLEXCLUSIVE, 0))
            ),
            Err(CtlError::Invalid)
        );
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_ADD, 6, &other, Some(&excl)),
            Ok(())
        );
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_MOD, 6, &other, Some(&ev(EPOLLIN, 0))),
            Err(CtlError::Invalid)
        );
    }

    #[test]
    fn registration_lives_as_long_as_the_open_file() {
        let epf = epoll_file();
        let ep = EpollNode::of(&epf).unwrap();
        let (file, probe) = Probe::file(NodeType::Pipe, true);
        EpollNode::ctl(&epf, EPOLL_CTL_ADD, 7, &file, Some(&ev(EPOLLIN, 70))).unwrap();
        probe.set(EPOLLIN);
        // A duplicate keeps the open file: events still arrive.
        let dup = file.clone();
        drop(file);
        assert_eq!(reported(&ep, 0, 0), [(EPOLLIN, 70)]);
        // The last descriptor closed: the registration goes.
        drop(dup);
        assert_eq!(reported(&ep, 0, 0), []);
        assert!(ep.is_empty());
    }

    #[test]
    fn level_triggered_reports_requested_bits_and_err_hup() {
        let epf = epoll_file();
        let ep = EpollNode::of(&epf).unwrap();
        let (file, probe) = Probe::file(NodeType::Socket, true);
        EpollNode::ctl(&epf, EPOLL_CTL_ADD, 3, &file, Some(&ev(EPOLLIN, 1))).unwrap();
        probe.set(EPOLLOUT);
        assert_eq!(reported(&ep, 0, 0), []);
        probe.set(EPOLLIN | EPOLLOUT | EPOLLRDNORM);
        assert_eq!(reported(&ep, 0, 0), [(EPOLLIN, 1)]);
        assert_eq!(reported(&ep, 0, 0), [(EPOLLIN, 1)]);
        probe.set(EPOLLHUP | EPOLLERR | 0x20 /* POLLNVAL */);
        assert_eq!(reported(&ep, 0, 0), [(EPOLLERR | EPOLLHUP, 1)]);
    }

    #[test]
    fn edge_triggered_reports_changes_only() {
        let epf = epoll_file();
        let ep = EpollNode::of(&epf).unwrap();
        let (file, probe) = Probe::file(NodeType::Socket, true);
        EpollNode::ctl(
            &epf,
            EPOLL_CTL_ADD,
            3,
            &file,
            Some(&ev(EPOLLIN | EPOLLOUT | EPOLLET, 9)),
        )
        .unwrap();
        // Always writable: reported once, not again while nothing happens
        // (the mio/tokio registration that level-triggering spins on).
        probe.set(EPOLLOUT);
        assert_eq!(reported(&ep, 1, 0), [(EPOLLOUT, 9)]);
        assert_eq!(reported(&ep, 1, 0), []);
        // New bits: reported.
        probe.set(EPOLLIN | EPOLLOUT);
        assert_eq!(reported(&ep, 1, 0), [(EPOLLIN | EPOLLOUT, 9)]);
        // Same bits after an I/O event (more data may have come): again.
        assert_eq!(reported(&ep, 2, 0), [(EPOLLIN | EPOLLOUT, 9)]);
        assert_eq!(reported(&ep, 2, 0), []);
        // Drained, then readable again: reported.
        probe.set(EPOLLOUT);
        assert_eq!(reported(&ep, 2, 0), []);
        probe.set(EPOLLIN | EPOLLOUT);
        assert_eq!(reported(&ep, 2, 0), [(EPOLLIN | EPOLLOUT, 9)]);
    }

    #[test]
    fn edge_triggered_without_announcements_rechecks_periodically() {
        assert!(edge_reports(0, 0, 0, EPOLLIN, 0, 0, false));
        assert!(!edge_reports(
            EPOLLIN,
            0,
            100,
            EPOLLIN,
            5,
            100 + RECHECK_NS - 1,
            false
        ));
        assert!(edge_reports(
            EPOLLIN,
            0,
            100,
            EPOLLIN,
            0,
            100 + RECHECK_NS,
            false
        ));
        assert!(!edge_reports(EPOLLIN, 3, 0, EPOLLIN, 3, u64::MAX, true));
        assert!(edge_reports(EPOLLIN, 3, 0, EPOLLIN, 4, 0, true));
    }

    #[test]
    fn oneshot_reports_once_until_rearmed() {
        let epf = epoll_file();
        let ep = EpollNode::of(&epf).unwrap();
        let (file, probe) = Probe::file(NodeType::Pipe, true);
        EpollNode::ctl(
            &epf,
            EPOLL_CTL_ADD,
            3,
            &file,
            Some(&ev(EPOLLIN | EPOLLONESHOT, 4)),
        )
        .unwrap();
        probe.set(EPOLLIN);
        assert_eq!(reported(&ep, 0, 0), [(EPOLLIN, 4)]);
        assert_eq!(reported(&ep, 0, 0), []);
        // Still registered: ADD is EEXIST, MOD re-arms.
        assert_eq!(
            EpollNode::ctl(&epf, EPOLL_CTL_ADD, 3, &file, Some(&ev(EPOLLIN, 4))),
            Err(CtlError::Exists)
        );
        EpollNode::ctl(
            &epf,
            EPOLL_CTL_MOD,
            3,
            &file,
            Some(&ev(EPOLLIN | EPOLLONESHOT, 5)),
        )
        .unwrap();
        assert_eq!(reported(&ep, 0, 0), [(EPOLLIN, 5)]);
    }

    #[test]
    fn nested_epoll_is_readable_and_loops_are_refused() {
        let outer = epoll_file();
        let inner = epoll_file();
        let (pipe, probe) = Probe::file(NodeType::Pipe, true);
        let outer_ep = EpollNode::of(&outer).unwrap();
        let inner_ep = EpollNode::of(&inner).unwrap();
        // An epoll file cannot watch itself.
        assert_eq!(
            EpollNode::ctl(&outer, EPOLL_CTL_ADD, 3, &outer, Some(&ev(EPOLLIN, 0))),
            Err(CtlError::Invalid)
        );
        EpollNode::ctl(&inner, EPOLL_CTL_ADD, 5, &pipe, Some(&ev(EPOLLIN, 0))).unwrap();
        EpollNode::ctl(&outer, EPOLL_CTL_ADD, 4, &inner, Some(&ev(EPOLLIN, 44))).unwrap();
        assert_eq!(reported(outer_ep, 0, 0), []);
        probe.set(EPOLLIN);
        assert_eq!(inner_ep.poll_readiness() as u32, EPOLLIN);
        assert_eq!(reported(outer_ep, 0, 0), [(EPOLLIN, 44)]);
        // inner -> outer would close a cycle.
        assert_eq!(
            EpollNode::ctl(&inner, EPOLL_CTL_ADD, 6, &outer, Some(&ev(EPOLLIN, 0))),
            Err(CtlError::Loop)
        );
        // An epoll file cannot be added exclusively.
        let third = epoll_file();
        assert_eq!(
            EpollNode::ctl(
                &third,
                EPOLL_CTL_ADD,
                3,
                &outer,
                Some(&ev(EPOLLIN | EPOLLEXCLUSIVE, 0))
            ),
            Err(CtlError::Invalid)
        );
    }

    #[test]
    fn nesting_depth_is_limited() {
        // e0 <- e1 <- ... : each watches the previous one.
        let files: Vec<Arc<File>> = (0..=MAX_NESTS + 1).map(|_| epoll_file()).collect();
        let mut refused_at = None;
        for i in 1..files.len() {
            let r = EpollNode::ctl(
                &files[i],
                EPOLL_CTL_ADD,
                3,
                &files[i - 1],
                Some(&ev(EPOLLIN, 0)),
            );
            if r.is_err() {
                assert_eq!(r, Err(CtlError::Loop));
                refused_at = Some(i);
                break;
            }
        }
        assert_eq!(refused_at, Some(MAX_NESTS + 1));
    }

    /// The path above counts too: a chain built from the top down, where
    /// every new edge has nothing below it, stops at MAX_NESTS as well
    /// (otherwise scans would recurse without bound).
    #[test]
    fn nesting_depth_counts_the_path_above() {
        let files: Vec<Arc<File>> = (0..=MAX_NESTS + 1).map(|_| epoll_file()).collect();
        let mut refused_at = None;
        for i in 1..files.len() {
            let r = EpollNode::ctl(
                &files[i - 1],
                EPOLL_CTL_ADD,
                3,
                &files[i],
                Some(&ev(EPOLLIN, 0)),
            );
            if r.is_err() {
                assert_eq!(r, Err(CtlError::Loop));
                refused_at = Some(i);
                break;
            }
        }
        assert_eq!(refused_at, Some(MAX_NESTS + 1));
        // A watcher that deregistered no longer counts.
        let top = &files[0];
        EpollNode::ctl(top, EPOLL_CTL_DEL, 3, &files[1], None).unwrap();
        assert_eq!(
            EpollNode::ctl(
                &files[MAX_NESTS],
                EPOLL_CTL_ADD,
                3,
                &files[MAX_NESTS + 1],
                Some(&ev(EPOLLIN, 0))
            ),
            Ok(())
        );
    }

    #[test]
    fn scans_stop_at_the_nesting_limit() {
        // Even a deeper graph than ctl allows (built here directly) is
        // scanned only MAX_NESTS levels down.
        let files: Vec<Arc<File>> = (0..MAX_NESTS + 3).map(|_| epoll_file()).collect();
        let (pipe, probe) = Probe::file(NodeType::Pipe, true);
        probe.set(EPOLLIN);
        let last = files.last().unwrap();
        EpollNode::of(last)
            .unwrap()
            .interest
            .lock()
            .insert(key(0, &pipe), Interest::new(&pipe, &ev(EPOLLIN, 0)));
        for w in files.windows(2) {
            EpollNode::of(&w[0])
                .unwrap()
                .interest
                .lock()
                .insert(key(0, &w[1]), Interest::new(&w[1], &ev(EPOLLIN, 0)));
        }
        assert_eq!(EpollNode::of(&files[0]).unwrap().poll_readiness(), 0);
        assert_eq!(
            EpollNode::of(&files[2]).unwrap().poll_readiness(),
            EPOLLIN as u16
        );
    }

    #[test]
    fn wait_without_timeout_returns_at_once() {
        let epf = epoll_file();
        let ep = EpollNode::of(&epf).unwrap();
        let (file, probe) = Probe::file(NodeType::Pipe, false);
        EpollNode::ctl(&epf, EPOLL_CTL_ADD, 3, &file, Some(&ev(EPOLLIN, 8))).unwrap();
        let mut out = [ev(0, 0); 4];
        assert_eq!(ep.wait(&mut out, Some(0)).unwrap(), 0);
        probe.set(EPOLLIN);
        assert_eq!(ep.wait(&mut out, Some(0)).unwrap(), 1);
        let data = out[0].data;
        assert_eq!(data, 8);
        // A full buffer is a short count; the rest wait for the next call.
        let (second, p2) = Probe::file(NodeType::Pipe, false);
        EpollNode::ctl(&epf, EPOLL_CTL_ADD, 4, &second, Some(&ev(EPOLLIN, 9))).unwrap();
        p2.set(EPOLLIN);
        assert_eq!(ep.wait(&mut out[..1], Some(0)).unwrap(), 1);
        assert_eq!(ep.wait(&mut out, Some(0)).unwrap(), 2);
    }
}

//! inotify, as Linux 7.3's `fs/notify/inotify`.
//!
//! An inotify instance (`inotify_init1`) is a file whose reads return the
//! `struct inotify_event` records of the watches added to it
//! (`inotify_add_watch`). A watch is on an inode, found by
//! [`node_key`]; the file system calls report what they do through the
//! hooks below ([`created`], [`linked`], [`removed`], [`moved`],
//! [`on_file`], [`on_path`], [`on_node`]), which return at once while no
//! watch exists anywhere.
//!
//! As Linux: a watch on a directory also reports the events of its entries,
//! with their names (FS_EVENT_ON_CHILD); an event identical to the last one
//! queued merges into it; a full queue (`max_queued_events`) ends in one
//! IN_Q_OVERFLOW; ending a watch -- `inotify_rm_watch`, IN_ONESHOT after its
//! event, or the inode going away -- queues IN_IGNORED; MOVE_SELF and
//! DELETE_SELF never carry IN_ISDIR.
//!
//! Differences: an event on an open file is reported to the directory of
//! the path it was opened with, looked up again (Linux follows the dentry,
//! so a renamed file's events reach its new directory); a file's
//! DELETE_SELF comes when its last name goes, not at its last close; and
//! IN_UNMOUNT and IN_EXCL_UNLINK's effect are not reported.

use alloc::{
    collections::{BTreeMap, VecDeque},
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use spin::Mutex;

use super::{write_access::node_key, DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::{
    error::{FsError, KernelError},
    syscall::SyscallError,
};

/// File was read.
pub const IN_ACCESS: u32 = 0x0000_0001;
/// File was written or truncated.
pub const IN_MODIFY: u32 = 0x0000_0002;
/// Metadata changed: mode, owner, times, extended attributes, link count.
pub const IN_ATTRIB: u32 = 0x0000_0004;
/// A file opened for writing was closed.
pub const IN_CLOSE_WRITE: u32 = 0x0000_0008;
/// A file not opened for writing was closed.
pub const IN_CLOSE_NOWRITE: u32 = 0x0000_0010;
/// File was opened.
pub const IN_OPEN: u32 = 0x0000_0020;
/// An entry was renamed away from the watched directory.
pub const IN_MOVED_FROM: u32 = 0x0000_0040;
/// An entry was renamed into the watched directory.
pub const IN_MOVED_TO: u32 = 0x0000_0080;
/// An entry was created in the watched directory.
pub const IN_CREATE: u32 = 0x0000_0100;
/// An entry was removed from the watched directory.
pub const IN_DELETE: u32 = 0x0000_0200;
/// The watched inode was deleted.
pub const IN_DELETE_SELF: u32 = 0x0000_0400;
/// The watched inode was renamed.
pub const IN_MOVE_SELF: u32 = 0x0000_0800;
/// Every event a watch can ask for.
pub const IN_ALL_EVENTS: u32 = 0x0000_0fff;
/// The file system was unmounted.
pub const IN_UNMOUNT: u32 = 0x0000_2000;
/// Events were lost: the queue was full.
pub const IN_Q_OVERFLOW: u32 = 0x0000_4000;
/// The watch ended.
pub const IN_IGNORED: u32 = 0x0000_8000;
/// add_watch: the path must be a directory.
pub const IN_ONLYDIR: u32 = 0x0100_0000;
/// add_watch: do not follow a final symlink.
pub const IN_DONT_FOLLOW: u32 = 0x0200_0000;
/// add_watch: no events for entries once they are unlinked.
pub const IN_EXCL_UNLINK: u32 = 0x0400_0000;
/// add_watch: EEXIST if the inode is already watched.
pub const IN_MASK_CREATE: u32 = 0x1000_0000;
/// add_watch: add to an existing watch's mask instead of replacing it.
pub const IN_MASK_ADD: u32 = 0x2000_0000;
/// The event's subject is a directory.
pub const IN_ISDIR: u32 = 0x4000_0000;
/// add_watch: end the watch after its first event.
pub const IN_ONESHOT: u32 = 0x8000_0000;
/// Every bit inotify_add_watch accepts (`ALL_INOTIFY_BITS`).
pub const ALL_INOTIFY_BITS: u32 = IN_ALL_EVENTS
    | IN_UNMOUNT
    | IN_Q_OVERFLOW
    | IN_IGNORED
    | IN_ONLYDIR
    | IN_DONT_FOLLOW
    | IN_EXCL_UNLINK
    | IN_MASK_ADD
    | IN_MASK_CREATE
    | IN_ISDIR
    | IN_ONESHOT;
/// inotify_init1: O_NONBLOCK.
pub const IN_NONBLOCK: u32 = 0o4000;
/// inotify_init1: O_CLOEXEC.
pub const IN_CLOEXEC: u32 = 0o2_000_000;

/// The limits `/proc/sys/fs/inotify` shows, which root may change (any
/// value from 0 to `INT_MAX`, as Linux's `proc_dointvec_minmax`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Events an instance queues before IN_Q_OVERFLOW (16384, Linux's
    /// default), taken when the instance is made.
    QueuedEvents,
    /// Instances per user (128, Linux's default; EMFILE beyond).
    UserInstances,
    /// Watches per user (ENOSPC beyond): 65536, where Linux scales its
    /// default with memory between 8192 and 1048576.
    UserWatches,
}

static LIMITS: [AtomicUsize; 3] = [
    AtomicUsize::new(16384),
    AtomicUsize::new(128),
    AtomicUsize::new(65536),
];

impl Limit {
    /// The limit `/proc/sys/fs/inotify/<name>` is.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "max_queued_events" => Some(Self::QueuedEvents),
            "max_user_instances" => Some(Self::UserInstances),
            "max_user_watches" => Some(Self::UserWatches),
            _ => None,
        }
    }

    pub fn get(self) -> usize {
        LIMITS[self as usize].load(Ordering::Relaxed)
    }

    /// Set the limit from what was written to its file: a decimal integer
    /// from 0 to `i32::MAX`, surrounding white space allowed; anything else
    /// is EINVAL.
    pub fn write(self, text: &[u8]) -> Result<(), KernelError> {
        let value = core::str::from_utf8(text)
            .ok()
            .and_then(|t| t.trim().parse::<i32>().ok())
            .filter(|v| *v >= 0)
            .ok_or(KernelError::InvalidArgument {
                name: "value",
                value: "not an integer from 0 to INT_MAX",
            })?;
        LIMITS[self as usize].store(value as usize, Ordering::Relaxed);
        Ok(())
    }
}

/// `sizeof(struct inotify_event)`: the bytes before the name.
const EVENT_HEADER: usize = 16;

/// The watch flags kept with a watch's events.
const WATCH_FLAGS: u32 = IN_ONESHOT | IN_EXCL_UNLINK;

/// One queued event.
struct Event {
    wd: i32,
    mask: u32,
    cookie: u32,
    name: String,
}

impl Event {
    /// The name's bytes as read: with its NUL, padded to a multiple of the
    /// header size, or none.
    fn name_len(&self) -> usize {
        if self.name.is_empty() {
            0
        } else {
            (self.name.len() + 1).next_multiple_of(EVENT_HEADER)
        }
    }

    fn len(&self) -> usize {
        EVENT_HEADER + self.name_len()
    }

    /// Write the record into `out`, exactly [`Self::len`] bytes.
    fn write(&self, out: &mut [u8]) {
        let name_len = self.name_len();
        out[0..4].copy_from_slice(&self.wd.to_ne_bytes());
        out[4..8].copy_from_slice(&self.mask.to_ne_bytes());
        out[8..12].copy_from_slice(&self.cookie.to_ne_bytes());
        out[12..16].copy_from_slice(&(name_len as u32).to_ne_bytes());
        let name = &mut out[EVENT_HEADER..EVENT_HEADER + name_len];
        name.fill(0);
        name[..self.name.len()].copy_from_slice(self.name.as_bytes());
    }
}

/// A watch: the inode (kept alive, as Linux's mark keeps its inode), its
/// key and the events (and [`WATCH_FLAGS`]) it asks for.
struct Watch {
    _node: Arc<dyn VfsNode>,
    key: u64,
    mask: u32,
    /// The inode is a directory (counted in [`DIR_WATCHES`]).
    dir: bool,
}

struct State {
    events: VecDeque<Event>,
    /// [`Limit::QueuedEvents`] when the instance was made.
    max_events: usize,
    /// The bytes the queued events read as (FIONREAD).
    bytes: usize,
    /// An IN_Q_OVERFLOW is queued; no other is until it is read.
    overflowed: bool,
    watches: BTreeMap<i32, Watch>,
    /// The last watch descriptor handed out: the next is the first free one
    /// after it, as Linux's idr_alloc_cyclic.
    last_wd: i32,
}

impl State {
    /// Queue `event`, or merge it into the last one, or, the queue full,
    /// queue IN_Q_OVERFLOW once (`fsnotify_insert_event`).
    fn push(&mut self, event: Event) {
        if self.events.len() >= self.max_events {
            if !self.overflowed {
                self.overflowed = true;
                self.bytes += EVENT_HEADER;
                self.events.push_back(Event {
                    wd: -1,
                    mask: IN_Q_OVERFLOW,
                    cookie: 0,
                    name: String::new(),
                });
            }
            return;
        }
        if let Some(last) = self.events.back() {
            if last.mask & IN_IGNORED == 0
                && last.mask == event.mask
                && last.wd == event.wd
                && last.name == event.name
            {
                return;
            }
        }
        self.bytes += event.len();
        self.events.push_back(event);
    }
}

/// One inotify instance: its queue and watches.
pub struct Group {
    state: Mutex<State>,
    /// The user charged for the instance and its watches.
    euid: u32,
}

/// Which instances watch each inode, by [`node_key`].
static WATCHED: Mutex<BTreeMap<u64, Vec<(Weak<Group>, i32)>>> = Mutex::new(BTreeMap::new());
/// The number of watches: the hooks do nothing while it is 0.
static WATCH_COUNT: AtomicUsize = AtomicUsize::new(0);
/// The number of watches on directories: while it is 0, an event on an open
/// file or a path is not looked up in its directory (Linux likewise skips
/// a parent that nothing watches).
static DIR_WATCHES: AtomicUsize = AtomicUsize::new(0);
/// Instances and watches per user.
static PER_USER: Mutex<BTreeMap<u32, (usize, usize)>> = Mutex::new(BTreeMap::new());
/// The last rename cookie handed out.
static COOKIE: AtomicU32 = AtomicU32::new(0);

fn io_changed() {
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::io_event();
}

/// Charge (`step` 1) or refund (-1) an instance (`watch` false) or a watch
/// of user `euid`; a charge past the limit fails.
fn charge(euid: u32, watch: bool, step: isize) -> bool {
    let mut users = PER_USER.lock();
    let counts = users.entry(euid).or_insert((0, 0));
    let (count, limit) = if watch {
        (&mut counts.1, Limit::UserWatches.get())
    } else {
        (&mut counts.0, Limit::UserInstances.get())
    };
    if step > 0 && *count >= limit {
        return false;
    }
    *count = count.saturating_add_signed(step);
    if *counts == (0, 0) {
        users.remove(&euid);
    }
    true
}

impl Group {
    /// A new instance for user `euid`: EMFILE past its instance limit.
    pub fn new(euid: u32) -> Result<Arc<Self>, SyscallError> {
        if !charge(euid, false, 1) {
            return Err(SyscallError::ResourceLimitExceeded);
        }
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                events: VecDeque::new(),
                max_events: Limit::QueuedEvents.get(),
                bytes: 0,
                overflowed: false,
                watches: BTreeMap::new(),
                last_wd: 0,
            }),
            euid,
        }))
    }

    /// Watch `node` for the events in `mask` (inotify_add_watch's argument,
    /// checked by the caller): an existing watch on it gets the new mask
    /// (or, with IN_MASK_ADD, the union; with IN_MASK_CREATE it is
    /// EEXIST). Returns the watch descriptor; ENOSPC past the user's watch
    /// limit.
    pub fn add_watch(
        self: &Arc<Self>,
        node: &Arc<dyn VfsNode>,
        mask: u32,
    ) -> Result<i32, SyscallError> {
        let key = node_key(node);
        let wanted = mask & (IN_ALL_EVENTS | WATCH_FLAGS);
        let mut state = self.state.lock();
        if let Some((&wd, watch)) = state.watches.iter_mut().find(|(_, w)| w.key == key) {
            if mask & IN_MASK_CREATE != 0 {
                return Err(SyscallError::FileExists);
            }
            if mask & IN_MASK_ADD != 0 {
                watch.mask |= wanted;
            } else {
                watch.mask = wanted;
            }
            return Ok(wd);
        }
        if !charge(self.euid, true, 1) {
            return Err(SyscallError::NoSpace);
        }
        let mut wd = state.last_wd;
        loop {
            wd = if wd == i32::MAX { 1 } else { wd + 1 };
            if !state.watches.contains_key(&wd) {
                break;
            }
        }
        state.last_wd = wd;
        let dir = is_dir(node);
        state.watches.insert(
            wd,
            Watch {
                _node: node.clone(),
                key,
                mask: wanted,
                dir,
            },
        );
        WATCHED
            .lock()
            .entry(key)
            .or_default()
            .push((Arc::downgrade(self), wd));
        WATCH_COUNT.fetch_add(1, Ordering::Relaxed);
        if dir {
            DIR_WATCHES.fetch_add(1, Ordering::Relaxed);
        }
        Ok(wd)
    }

    /// End the watch `wd` (inotify_rm_watch): IN_IGNORED is queued. EINVAL
    /// if there is none.
    pub fn rm_watch(&self, wd: i32) -> Result<(), SyscallError> {
        let mut state = self.state.lock();
        if !self.end_watch(&mut state, wd) {
            return Err(SyscallError::InvalidArgument);
        }
        drop(state);
        io_changed();
        Ok(())
    }

    /// Remove the watch `wd`, queueing IN_IGNORED; false if there is none.
    fn end_watch(&self, state: &mut State, wd: i32) -> bool {
        let Some(watch) = state.watches.remove(&wd) else {
            return false;
        };
        self.forget(&watch, wd);
        state.push(Event {
            wd,
            mask: IN_IGNORED,
            cookie: 0,
            name: String::new(),
        });
        true
    }

    /// Take the watch `wd` out of [`WATCHED`] and refund it.
    fn forget(&self, watch: &Watch, wd: i32) {
        let mut watched = WATCHED.lock();
        if let Some(list) = watched.get_mut(&watch.key) {
            list.retain(|(group, w)| !(*w == wd && core::ptr::eq(group.as_ptr(), self)));
            if list.is_empty() {
                watched.remove(&watch.key);
            }
        }
        drop(watched);
        WATCH_COUNT.fetch_sub(1, Ordering::Relaxed);
        if watch.dir {
            DIR_WATCHES.fetch_sub(1, Ordering::Relaxed);
        }
        charge(self.euid, true, -1);
    }

    /// Queue `mask` for the watch `wd` if it asks for the event; an
    /// IN_ONESHOT watch then ends.
    fn event(&self, wd: i32, mask: u32, cookie: u32, name: &str) {
        let mut state = self.state.lock();
        let Some(watch) = state.watches.get(&wd) else {
            return;
        };
        if watch.mask & mask & IN_ALL_EVENTS == 0 {
            return;
        }
        let oneshot = watch.mask & IN_ONESHOT != 0;
        // inotify never reported IN_ISDIR with these (inotify_fsnotify.c).
        let mask = if mask & (IN_MOVE_SELF | IN_DELETE_SELF) != 0 {
            mask & !IN_ISDIR
        } else {
            mask
        };
        state.push(Event {
            wd,
            mask,
            cookie,
            name: String::from(name),
        });
        if oneshot {
            self.end_watch(&mut state, wd);
        }
        drop(state);
        io_changed();
    }

    /// End the watch `wd` because its inode is gone.
    fn inode_gone(&self, wd: i32) {
        let mut state = self.state.lock();
        if self.end_watch(&mut state, wd) {
            drop(state);
            io_changed();
        }
    }

    /// The instance's last close: its watches end, with no events (no one
    /// can read them).
    fn close(&self) {
        let mut state = self.state.lock();
        for (wd, watch) in core::mem::take(&mut state.watches) {
            self.forget(&watch, wd);
        }
        state.events.clear();
        state.bytes = 0;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        charge(self.euid, false, -1);
    }
}

/// The instances and watch descriptors watching `key`.
fn watchers(key: u64) -> Vec<(Arc<Group>, i32)> {
    WATCHED.lock().get(&key).map_or_else(Vec::new, |list| {
        list.iter()
            .filter_map(|(group, wd)| Some((group.upgrade()?, *wd)))
            .collect()
    })
}

/// Queue the event for every watch on `key` that asks for it. The watch
/// table is not held while the instances queue.
fn deliver(key: u64, mask: u32, cookie: u32, name: &str) {
    for (group, wd) in watchers(key) {
        group.event(wd, mask, cookie, name);
    }
}

/// Whether any watch exists: while none does, every hook returns at once.
#[inline]
pub fn active() -> bool {
    WATCH_COUNT.load(Ordering::Relaxed) != 0
}

fn is_dir(node: &Arc<dyn VfsNode>) -> bool {
    node.node_type() == NodeType::Directory
}

/// `mask` on `node` itself, for its own watches (`fsnotify_inode`).
pub fn on_node(node: &Arc<dyn VfsNode>, mask: u32) {
    if !active() {
        return;
    }
    let mask = if is_dir(node) { mask | IN_ISDIR } else { mask };
    deliver(node_key(node), mask, 0, "");
}

/// `mask` on the entry `name` of `dir`, for the directory's watches.
fn on_child(dir: &Arc<dyn VfsNode>, name: &str, child_dir: bool, mask: u32, cookie: u32) {
    let mask = if child_dir { mask | IN_ISDIR } else { mask };
    deliver(node_key(dir), mask, cookie, name);
}

/// `mask` on `node`, the entry `name` of `dir`: the directory's watches,
/// with the name, then the node's own (`fsnotify_dentry`).
pub fn on_entry(dir: &Arc<dyn VfsNode>, name: &str, node: &Arc<dyn VfsNode>, mask: u32) {
    if !active() {
        return;
    }
    on_child(dir, name, is_dir(node), mask, 0);
    on_node(node, mask);
}

/// [`on_entry`] for `node` at `path`, an absolute path as the caller sees
/// it (inside its root), its directory looked up (only the node's own
/// watches if that fails).
pub fn on_path(path: &str, node: &Arc<dyn VfsNode>, mask: u32) {
    if !active() {
        return;
    }
    if DIR_WATCHES.load(Ordering::Relaxed) == 0 {
        return on_node(node, mask);
    }
    let Some(vfs) = super::try_get_vfs() else {
        return on_node(node, mask);
    };
    let root = vfs.as_caller().get_root();
    let global = match (root.as_str(), path) {
        ("/", _) => String::from(path),
        (_, "/") => root,
        _ => alloc::format!("{}{}", root, path),
    };
    on_global_path(&global, node, mask);
}

/// [`on_path`] where the node's path is known, else [`on_node`].
pub fn on_target(path: Option<&str>, node: &Arc<dyn VfsNode>, mask: u32) {
    match path {
        Some(path) => on_path(path, node, mask),
        None => on_node(node, mask),
    }
}

/// [`on_entry`] for an open file, by the path it was opened with. An
/// O_PATH file reports nothing, as on Linux.
pub fn on_file(file: &super::file::File, mask: u32) {
    if !active() || file.is_path() {
        return;
    }
    match &file.path {
        Some(path) => on_global_path(path, &file.node, mask),
        None => on_node(&file.node, mask),
    }
}

/// [`on_entry`] for `node` at `path` in the whole tree (an open file's
/// recorded path), its directory looked up from the real root.
fn on_global_path(path: &str, node: &Arc<dyn VfsNode>, mask: u32) {
    if DIR_WATCHES.load(Ordering::Relaxed) == 0 {
        return on_node(node, mask);
    }
    match parent_of(path) {
        Some((dir, name)) => on_entry(&dir, &name, node, mask),
        None => on_node(node, mask),
    }
}

/// The directory holding `path` (absolute, in the whole tree), and the
/// entry's name.
fn parent_of(path: &str) -> Option<(Arc<dyn VfsNode>, String)> {
    let path = path.trim_end_matches('/');
    let pos = path.rfind('/')?;
    let (dir, name) = (if pos == 0 { "/" } else { &path[..pos] }, &path[pos + 1..]);
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let dir = super::try_get_vfs()?.resolve_path(dir).ok()?;
    Some((dir, String::from(name)))
}

/// `node` was made as the entry `name` of `dir` (`fsnotify_create`,
/// `fsnotify_mkdir`).
pub fn created(dir: &Arc<dyn VfsNode>, name: &str, node: &Arc<dyn VfsNode>) {
    if active() {
        on_child(dir, name, is_dir(node), IN_CREATE, 0);
    }
}

/// `node` got the new name `name` in `dir` (`fsnotify_link`): its link
/// count changed, then the entry was created.
pub fn linked(dir: &Arc<dyn VfsNode>, name: &str, node: &Arc<dyn VfsNode>) {
    if active() {
        on_node(node, IN_ATTRIB);
        on_child(dir, name, false, IN_CREATE, 0);
    }
}

/// `node`, the entry `name` of `dir`, was removed (unlink, rmdir): a file's
/// link count changed and, its last name gone, it is deleted; a directory
/// is deleted. Then the directory's IN_DELETE.
pub fn removed(dir: &Arc<dyn VfsNode>, name: &str, node: &Arc<dyn VfsNode>) {
    if !active() {
        return;
    }
    let node_dir = is_dir(node);
    if !node_dir {
        on_node(node, IN_ATTRIB);
    }
    if node_dir || node.link_count() == 0 {
        deleted(node);
    }
    on_child(dir, name, node_dir, IN_DELETE, 0);
}

/// `node` was renamed from `old_name` in `old_dir` to `new_name` in
/// `new_dir`, replacing `replaced` if there was one (`fsnotify_move`):
/// MOVED_FROM and MOVED_TO with one cookie, the replaced inode's link
/// count, MOVE_SELF, and the replaced inode's deletion if that was its last
/// name.
pub fn moved(
    old_dir: &Arc<dyn VfsNode>,
    old_name: &str,
    new_dir: &Arc<dyn VfsNode>,
    new_name: &str,
    node: &Arc<dyn VfsNode>,
    replaced: Option<&Arc<dyn VfsNode>>,
) {
    if !active() {
        return;
    }
    let cookie = loop {
        let c = COOKIE.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if c != 0 {
            break c;
        }
    };
    let node_dir = is_dir(node);
    on_child(old_dir, old_name, node_dir, IN_MOVED_FROM, cookie);
    on_child(new_dir, new_name, node_dir, IN_MOVED_TO, cookie);
    if let Some(target) = replaced {
        on_node(target, IN_ATTRIB);
    }
    on_node(node, IN_MOVE_SELF);
    if let Some(target) = replaced {
        if is_dir(target) || target.link_count() == 0 {
            deleted(target);
        }
    }
}

/// The inode `node` is gone: IN_DELETE_SELF, then every watch on it ends.
fn deleted(node: &Arc<dyn VfsNode>) {
    let key = node_key(node);
    deliver(key, IN_DELETE_SELF, 0, "");
    for (group, wd) in watchers(key) {
        group.inode_gone(wd);
    }
}

/// The file an inotify instance is: reads return its events.
pub struct InotifyNode {
    group: Arc<Group>,
}

impl InotifyNode {
    pub fn new(group: Arc<Group>) -> Self {
        Self { group }
    }

    pub fn group(&self) -> &Arc<Group> {
        &self.group
    }
}

impl Drop for InotifyNode {
    fn drop(&mut self) {
        self.group.close();
    }
}

impl VfsNode for InotifyNode {
    /// An anonymous inode, as on Linux.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::ANON_INODE_FS_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    /// As many whole events as fit; EINVAL if the first does not, EAGAIN
    /// (a wait in the read path) if there is none.
    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let mut state = self.group.state.lock();
        let Some(first) = state.events.front() else {
            return Err(KernelError::WouldBlock);
        };
        if first.len() > buffer.len() {
            return Err(KernelError::InvalidArgument {
                name: "count",
                value: "too small for the next inotify event",
            });
        }
        let mut done = 0;
        while let Some(event) = state.events.front() {
            let len = event.len();
            if done + len > buffer.len() {
                break;
            }
            event.write(&mut buffer[done..done + len]);
            if event.mask & IN_Q_OVERFLOW != 0 {
                state.overflowed = false;
            }
            done += len;
            state.bytes -= len;
            state.events.pop_front();
        }
        Ok(done)
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(FsError::BadFileDescriptor))
    }

    fn poll_readiness(&self) -> u16 {
        if self.group.state.lock().events.is_empty() {
            0
        } else {
            0x0001 // POLLIN
        }
    }

    fn wakes_io_waiters(&self) -> bool {
        true // every queued event calls io_changed
    }

    fn bytes_readable(&self) -> Option<usize> {
        Some(self.group.state.lock().bytes)
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
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::BadFileDescriptor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{ramfs::RamFs, Filesystem};

    /// The queued events as "wd:mask:name" words.
    fn drain(node: &InotifyNode) -> String {
        let mut buf = [0u8; 4096];
        let mut out = String::new();
        while let Ok(n) = node.read(0, &mut buf) {
            let mut at = 0;
            while at < n {
                let word =
                    |i: usize| u32::from_ne_bytes(buf[at + i..at + i + 4].try_into().unwrap());
                let len = word(12) as usize;
                let name = core::str::from_utf8(&buf[at + 16..at + 16 + len]).unwrap();
                out += &alloc::format!(
                    "{}:{:x}:{} ",
                    word(0) as i32,
                    word(4),
                    name.trim_end_matches('\0')
                );
                at += 16 + len;
            }
        }
        out
    }

    fn instance() -> InotifyNode {
        InotifyNode::new(Group::new(1000).unwrap())
    }

    #[test]
    fn directory_watches_report_entries_by_name() {
        let fs = RamFs::new();
        let dir = fs.root().mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let ino = instance();
        let wd = ino.group().add_watch(&dir, IN_ALL_EVENTS).unwrap();
        assert_eq!(ino.group().add_watch(&dir, IN_ALL_EVENTS), Ok(wd));
        assert_eq!(
            ino.group().add_watch(&dir, IN_CREATE | IN_MASK_CREATE),
            Err(SyscallError::FileExists)
        );

        let file = dir.create("a", Permissions::from_mode(0o644)).unwrap();
        created(&dir, "a", &file);
        on_entry(&dir, "a", &file, IN_MODIFY);
        on_entry(&dir, "a", &file, IN_MODIFY); // merges
        let sub = dir.mkdir("sub", Permissions::from_mode(0o755)).unwrap();
        created(&dir, "sub", &sub);
        assert_eq!(ino.bytes_readable(), Some(3 * 32));
        // Too small for the first event: EINVAL.
        assert!(matches!(
            ino.read(0, &mut [0u8; 8]),
            Err(KernelError::InvalidArgument { .. })
        ));
        assert_eq!(
            drain(&ino),
            alloc::format!("{wd}:100:a {wd}:2:a {wd}:40000100:sub ")
        );
        assert!(matches!(
            ino.read(0, &mut [0u8; 64]),
            Err(KernelError::WouldBlock)
        ));
    }

    #[test]
    fn file_watches_end_with_the_inode() {
        let fs = RamFs::new();
        let dir = fs.root().mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let file = dir.create("a", Permissions::from_mode(0o644)).unwrap();
        let ino = instance();
        let wd = ino.group().add_watch(&dir, IN_ALL_EVENTS).unwrap();
        let wf = ino
            .group()
            .add_watch(&file, IN_ATTRIB | IN_MOVE_SELF | IN_DELETE_SELF)
            .unwrap();
        assert!(wf > wd);

        on_entry(&dir, "a", &file, IN_ATTRIB);
        dir.rename("a", &dir, "b").unwrap();
        moved(&dir, "a", &dir, "b", &file, None);
        dir.unlink("b").unwrap();
        removed(&dir, "b", &file);
        assert_eq!(
            drain(&ino),
            alloc::format!(
                "{wd}:4:a {wf}:4: {wd}:40:a {wd}:80:b {wf}:800: {wf}:4: {wf}:400: {wf}:8000: \
                 {wd}:200:b "
            )
        );
        // The file's watch is gone; the directory's stays.
        assert_eq!(ino.group().rm_watch(wf), Err(SyscallError::InvalidArgument));
        assert_eq!(ino.group().rm_watch(wd), Ok(()));
        assert_eq!(drain(&ino), alloc::format!("{wd}:8000: "));
    }

    #[test]
    fn oneshot_watches_report_once() {
        let fs = RamFs::new();
        let dir = fs.root().mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let ino = instance();
        let wd = ino.group().add_watch(&dir, IN_CREATE | IN_ONESHOT).unwrap();
        for name in ["a", "b"] {
            let file = dir.create(name, Permissions::from_mode(0o644)).unwrap();
            created(&dir, name, &file);
        }
        assert_eq!(drain(&ino), alloc::format!("{wd}:100:a {wd}:8000: "));
    }

    #[test]
    fn a_full_queue_ends_in_one_overflow() {
        let fs = RamFs::new();
        let dir = fs.root().mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let ino = instance();
        ino.group().add_watch(&dir, IN_CREATE | IN_DELETE).unwrap();
        for i in 0..Limit::QueuedEvents.get() + 10 {
            // Alternating events, so none merges.
            let mask = if i % 2 == 0 { IN_CREATE } else { IN_DELETE };
            on_child(&dir, "x", false, mask, 0);
        }
        let state = ino.group().state.lock();
        assert_eq!(state.events.len(), Limit::QueuedEvents.get() + 1);
        assert_eq!(state.events.back().unwrap().mask, IN_Q_OVERFLOW);
        assert_eq!(state.events.back().unwrap().wd, -1);
    }

    #[test]
    fn closing_an_instance_ends_its_watches() {
        let fs = RamFs::new();
        let dir = fs.root().mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let key = node_key(&dir);
        let ino = instance();
        ino.group().add_watch(&dir, IN_CREATE).unwrap();
        assert_eq!(watchers(key).len(), 1);
        drop(ino);
        assert!(watchers(key).is_empty());
    }
}

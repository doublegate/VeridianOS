//! Mounts and mount trees (ADR 0022 step H3).
//!
//! A [`Mount`] is one filesystem attached at one place, as Linux's
//! `struct mount`; its [`Superblock`] is the filesystem instance, which
//! copies of the mount share. A [`Mounts`] is the tree of one mount
//! namespace: path resolution walks a snapshot of it, and a new namespace
//! starts with a copy of it whose mounts are new (fresh ids) and whose
//! filesystems are the same (Linux's copy_tree).
//!
//! Ids come from one pool per tree and the copies made from it, as Linux
//! takes every `mnt_id` from one IDA: the lowest free id, released when the
//! last reference to its mount goes (a walk still on an old snapshot keeps
//! it), and a unique id that is never reused. Device numbers (`st_dev`)
//! belong to the superblock, as Linux's anonymous devices, so a file
//! reports the same device in every namespace.

use alloc::{collections::BTreeSet, string::String, sync::Arc, vec::Vec};
use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use spin::{Mutex, RwLock};

use super::{
    normalize_path, path_is_under, Filesystem, MountIds, MountTable, NodeType, VfsNode,
    MNT_UNIQUE_ID_OFFSET, PSEUDO_MOUNT,
};

/// Per-mount flags, with Linux's MNT_* values (include/linux/mount.h); the
/// propagation type is kept here too.
pub mod flags {
    pub const NOSUID: u32 = 0x01;
    pub const NODEV: u32 = 0x02;
    pub const NOEXEC: u32 = 0x04;
    pub const NOATIME: u32 = 0x08;
    pub const NODIRATIME: u32 = 0x10;
    pub const RELATIME: u32 = 0x20;
    pub const READONLY: u32 = 0x40;
    pub const NOSYMFOLLOW: u32 = 0x80;
    /// The atime flags (Linux's MNT_ATIME_MASK).
    pub const ATIME: u32 = NOATIME | NODIRATIME | RELATIME;
    /// The flags mount(2) sets (Linux's MNT_USER_SETTABLE_MASK).
    pub const SETTABLE: u32 = NOSUID | NODEV | NOEXEC | ATIME | READONLY | NOSYMFOLLOW;
    /// MS_UNBINDABLE's propagation type: the mount cannot be bound.
    pub const UNBINDABLE: u32 = 0x1_0000;
}

/// Why a change to a tree was refused, as Linux's errno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountError {
    /// EINVAL.
    Invalid,
    /// EBUSY.
    Busy,
    /// ENOTDIR: a directory over a file or a file over a directory.
    NotDirectory,
    /// ELOOP: a mount moved under itself.
    Loop,
    /// EAGAIN: MNT_EXPIRE's first call marked the mount.
    Again,
}

/// A propagation type mount(2) asks for (one of MS_SHARED, MS_PRIVATE,
/// MS_SLAVE, MS_UNBINDABLE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Propagation {
    Shared,
    Private,
    Slave,
    Unbindable,
}

/// umount2's flags (MNT_FORCE, MNT_DETACH, MNT_EXPIRE).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Umount {
    pub force: bool,
    pub detach: bool,
    pub expire: bool,
}

/// The ids a tree and its copies hand out.
pub(super) struct IdPool {
    /// Mount ids in use.
    ids: Mutex<BTreeSet<u32>>,
    /// Anonymous device minors in use.
    devs: Mutex<BTreeSet<u32>>,
    /// The last unique id handed out.
    last_unique: AtomicU64,
}

impl IdPool {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            ids: Mutex::new(BTreeSet::new()),
            devs: Mutex::new(BTreeSet::new()),
            last_unique: AtomicU64::new(PSEUDO_MOUNT.unique),
        })
    }

    /// The lowest number from 1 not in `set`, now in it (Linux's
    /// ida_alloc_min(1)); 0 is the pseudo mount's id and device.
    fn take(set: &Mutex<BTreeSet<u32>>) -> u32 {
        let mut set = set.lock();
        let mut n = 1;
        for &used in set.iter() {
            if used != n {
                break;
            }
            n += 1;
        }
        set.insert(n);
        n
    }
}

/// A filesystem instance as mounts see it: the filesystem, its device
/// number and whether it is read-only (SB_RDONLY), freed when no mount of
/// it is left.
pub(super) struct Superblock {
    fs: Arc<dyn Filesystem>,
    dev: u32,
    readonly: AtomicBool,
    pool: Arc<IdPool>,
}

impl Drop for Superblock {
    fn drop(&mut self) {
        self.pool.devs.lock().remove(&self.dev);
    }
}

/// One mount: a filesystem, or (bind mount) a directory or file in one,
/// attached at a place in one tree.
pub struct Mount {
    sb: Arc<Superblock>,
    ids: MountIds,
    /// What `/proc/<pid>/mounts` names as its source (Linux's mnt_devname).
    source: String,
    /// What a walk entering the mount finds (Linux's mnt_root), and its
    /// path inside the filesystem ("/" for the whole of it).
    root: Arc<dyn VfsNode>,
    root_path: String,
    /// [`flags`] (Linux's mnt_flags).
    flags: AtomicU32,
    /// MNT_EXPIRE's mark.
    expiry: AtomicBool,
}

impl Drop for Mount {
    fn drop(&mut self) {
        self.sb.pool.ids.lock().remove(&self.ids.id);
    }
}

impl Mount {
    /// A mount of a filesystem not mounted before (a new superblock), as
    /// the kernel mounts at boot: relatime, read-only if the filesystem is.
    pub(super) fn new(fs: Arc<dyn Filesystem>, source: String, pool: &Arc<IdPool>) -> Arc<Self> {
        Self::with_flags(fs, source, pool, flags::RELATIME, false)
    }

    /// A new superblock's mount with the mount flags `mnt` and the
    /// superblock read-only if `sb_readonly` (or the filesystem is).
    pub(super) fn with_flags(
        fs: Arc<dyn Filesystem>,
        source: String,
        pool: &Arc<IdPool>,
        mnt: u32,
        sb_readonly: bool,
    ) -> Arc<Self> {
        let readonly = sb_readonly || fs.is_readonly();
        let root = fs.root();
        let sb = Arc::new(Superblock {
            fs,
            dev: IdPool::take(&pool.devs),
            readonly: AtomicBool::new(readonly),
            pool: pool.clone(),
        });
        let mnt = if readonly { mnt | flags::READONLY } else { mnt };
        Self::on(sb, source, root, String::from("/"), mnt)
    }

    fn on(
        sb: Arc<Superblock>,
        source: String,
        root: Arc<dyn VfsNode>,
        root_path: String,
        mnt: u32,
    ) -> Arc<Self> {
        let id = IdPool::take(&sb.pool.ids);
        let unique = sb.pool.last_unique.fetch_add(1, Ordering::Relaxed) + 1;
        let dev = sb.dev;
        Arc::new(Self {
            sb,
            ids: MountIds { id, unique, dev },
            source,
            root,
            root_path,
            flags: AtomicU32::new(mnt),
            expiry: AtomicBool::new(false),
        })
    }

    /// A new mount of the same filesystem and root, with its flags
    /// (Linux's clone_mnt).
    fn copy(&self) -> Arc<Self> {
        self.bound(self.root.clone(), self.root_path.clone())
    }

    /// A new mount of the same filesystem rooted at `root`, which is at
    /// `root_path` in it (a bind mount); private.
    fn bound(&self, root: Arc<dyn VfsNode>, root_path: String) -> Arc<Self> {
        Self::on(
            self.sb.clone(),
            self.source.clone(),
            root,
            root_path,
            self.flags() & flags::SETTABLE,
        )
    }

    pub fn fs(&self) -> &Arc<dyn Filesystem> {
        &self.sb.fs
    }

    /// The node a walk entering the mount finds.
    pub fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone()
    }

    pub fn ids(&self) -> MountIds {
        self.ids
    }

    /// Its [`flags`].
    pub fn flags(&self) -> u32 {
        self.flags.load(Ordering::Acquire)
    }

    /// Whether writes through it are refused: the mount or its superblock
    /// is read-only (Linux's __mnt_is_readonly).
    pub fn is_readonly(&self) -> bool {
        self.flags() & flags::READONLY != 0 || self.sb_readonly()
    }

    /// Whether its superblock is read-only (Linux's sb_rdonly), which
    /// refuses writes before the permission check does.
    pub fn sb_readonly(&self) -> bool {
        self.sb.readonly.load(Ordering::Acquire)
    }

    /// Whether mount flag `flag` ([`flags`]) is set.
    pub fn has(&self, flag: u32) -> bool {
        self.flags() & flag != 0
    }

    /// The mount-point flags become `mnt` (Linux's set_mount_attributes),
    /// keeping the propagation type.
    fn set_flags(&self, mnt: u32) {
        let keep = self.flags() & !flags::SETTABLE;
        self.flags
            .store(keep | (mnt & flags::SETTABLE), Ordering::Release);
    }

    fn set_unbindable(&self, on: bool) {
        if on {
            self.flags.fetch_or(flags::UNBINDABLE, Ordering::AcqRel);
        } else {
            self.flags.fetch_and(!flags::UNBINDABLE, Ordering::AcqRel);
        }
    }

    fn same_place(&self, other: &Mount) -> bool {
        Arc::ptr_eq(&self.sb, &other.sb) && self.root_path == other.root_path
    }
}

/// What a mount of `fs` is named after when nothing names its source: the
/// root of a disk filesystem is `/dev/root`, as Linux names the root it
/// mounts from `root=`; anything else its type.
pub(super) fn default_source(fs: &dyn Filesystem, at_root: bool) -> String {
    let ty = linux_fs_type(fs.name());
    let disk = matches!(ty, "blockfs" | "ext4" | "vfat");
    String::from(if at_root && disk { "/dev/root" } else { ty })
}

/// A filesystem's type as Linux names it (`/proc/filesystems`,
/// `/proc/<pid>/mounts`): procfs is `proc`, devfs `devtmpfs`, FAT `vfat`.
pub fn linux_fs_type(name: &str) -> &str {
    match name {
        "procfs" => "proc",
        "devfs" => "devtmpfs",
        "fat32" => "vfat",
        other => other,
    }
}

/// The filesystem types mount(2) can make, as `/proc/filesystems` lists
/// them (all `nodev`: none needs a block device).
pub const MOUNTABLE: [&str; 4] = ["tmpfs", "ramfs", "proc", "devtmpfs"];

/// Why mount(2) could not make a filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewFsError {
    /// ENODEV: no such filesystem type.
    UnknownType,
    /// EINVAL: an option the type does not take, or a bad value.
    BadOption,
}

/// A new filesystem of Linux type `ty` (Linux's do_new_mount with
/// fs_context_for_mount and parse_monolithic_mount_data), its root owned
/// by `owner` (the mounter's fsuid and fsgid, as shmem and ramfs make it).
/// `data` is the comma-separated option string: tmpfs takes `size=` (bytes
/// with a k, m, g or t suffix, or a percentage of RAM; 0 is unlimited),
/// `mode=` (octal), `uid=` and `gid=`; ramfs takes `mode=` and ignores the
/// rest, as Linux's does; proc and devtmpfs take none here.
pub fn new_filesystem(
    ty: &str,
    data: &str,
    owner: (u32, u32),
) -> Result<Arc<dyn Filesystem>, NewFsError> {
    use super::{devfs::DevFs, procfs::ProcFs, ramfs::RamFs, tmpfs::TmpFs, Permissions};
    let options = || data.split(',').filter(|o| !o.is_empty());
    let number =
        |v: &str, radix: u32| u64::from_str_radix(v, radix).map_err(|_| NewFsError::BadOption);
    let (mut size, mut mode, mut uid, mut gid) = (None, None, owner.0, owner.1);
    match ty {
        "tmpfs" => {
            for option in options() {
                let (key, value) = option.split_once('=').ok_or(NewFsError::BadOption)?;
                match key {
                    "size" => size = Some(tmpfs_size(value)?),
                    "mode" => mode = Some(number(value, 8)? as u32 & 0o7777),
                    "uid" => uid = number(value, 10)? as u32,
                    "gid" => gid = number(value, 10)? as u32,
                    _ => return Err(NewFsError::BadOption),
                }
            }
        }
        "ramfs" => {
            for option in options() {
                if let Some(value) = option.strip_prefix("mode=") {
                    mode = Some(number(value, 8)? as u32 & 0o7777);
                }
            }
        }
        "proc" | "devtmpfs" => {
            if options().next().is_some() {
                return Err(NewFsError::BadOption);
            }
        }
        _ => return Err(NewFsError::UnknownType),
    }
    let fs: Arc<dyn Filesystem> = match ty {
        "tmpfs" => Arc::new(match size {
            Some(bytes) => TmpFs::new(bytes),
            None => TmpFs::half_of_ram(),
        }),
        "ramfs" => Arc::new(RamFs::new()),
        "proc" => Arc::new(ProcFs::new()),
        _ => Arc::new(DevFs::new()),
    };
    if matches!(ty, "tmpfs" | "ramfs") {
        let root = fs.root();
        if let Some(mode) = mode {
            let _ = root.chmod(Permissions::from_mode(mode));
        }
        let _ = root.chown(Some(uid), Some(gid));
    }
    Ok(fs)
}

/// tmpfs's `size=`: bytes, with a k, m, g or t suffix (binary units), or
/// a percentage of RAM.
fn tmpfs_size(value: &str) -> Result<usize, NewFsError> {
    let bad = NewFsError::BadOption;
    if let Some(pct) = value.strip_suffix('%') {
        let pct: usize = pct.parse().map_err(|_| bad)?;
        let ram = crate::mm::get_memory_stats().total_frames * 4096;
        return Ok(ram / 100 * pct);
    }
    let (digits, shift) = match value.as_bytes().last() {
        Some(b'k' | b'K') => (&value[..value.len() - 1], 10),
        Some(b'm' | b'M') => (&value[..value.len() - 1], 20),
        Some(b'g' | b'G') => (&value[..value.len() - 1], 30),
        Some(b't' | b'T') => (&value[..value.len() - 1], 40),
        _ => (value, 0),
    };
    let n: usize = digits.parse().map_err(|_| bad)?;
    n.checked_mul(1usize << shift).ok_or(bad)
}

/// One mount namespace's tree.
pub struct Mounts {
    table: RwLock<Arc<MountTable>>,
    pool: Arc<IdPool>,
}

impl core::fmt::Debug for Mounts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mounts").finish_non_exhaustive()
    }
}

impl Default for Mounts {
    fn default() -> Self {
        Self::new()
    }
}

impl Mounts {
    /// An empty tree with ids of its own.
    pub fn new() -> Self {
        Self::with_table(MountTable::default(), IdPool::new())
    }

    fn with_table(table: MountTable, pool: Arc<IdPool>) -> Self {
        Self {
            table: RwLock::new(Arc::new(table)),
            pool,
        }
    }

    pub(super) fn pool(&self) -> &Arc<IdPool> {
        &self.pool
    }

    /// The tree as it is now; a walk on it is not disturbed by changes.
    pub(super) fn snapshot(&self) -> Arc<MountTable> {
        self.table.read().clone()
    }

    /// Apply `f` to a copy of the tree and publish the copy (RCU-style);
    /// changes are serialized by the write lock.
    pub(super) fn update<R>(&self, f: impl FnOnce(&mut MountTable) -> R) -> R {
        let mut table = self.table.write();
        let mut next = MountTable::clone(&table);
        let result = f(&mut next);
        *table = Arc::new(next);
        result
    }

    /// A new namespace's tree (Linux's copy_mnt_ns): every mount copied,
    /// root first and then in path order, so a parent's id comes before its
    /// children's; or with `empty` (CLONE_EMPTY_MNTNS) only the root.
    pub fn copy(&self, empty: bool) -> Mounts {
        let old = self.snapshot();
        let copy_stack = |point: &str| -> Vec<Arc<Mount>> {
            old.covered
                .get(point)
                .into_iter()
                .flatten()
                .map(|m| m.copy())
                .collect()
        };
        let mut table = MountTable::default();
        if empty {
            table.root = old.root.as_ref().map(|m| m.copy());
        } else {
            let below = copy_stack("/");
            if !below.is_empty() {
                table.covered.insert(String::from("/"), below);
            }
            table.root = old.root.as_ref().map(|m| m.copy());
            for (path, mount) in &old.mounts {
                let below = copy_stack(path);
                if !below.is_empty() {
                    table.covered.insert(path.clone(), below);
                }
                table.mounts.insert(path.clone(), mount.copy());
            }
        }
        Self::with_table(table, self.pool.clone())
    }

    /// The mounts a process whose root is `root` (a canonical path) can
    /// see, as `(mount point relative to root, mount, parent id)`, root
    /// first, each place's mounts in the order they cover one another:
    /// Linux's m_start walk with the mounts outside the root left out. The
    /// namespace root's parent is itself, as in Linux; a mount over another
    /// has that one as its parent.
    fn visible(&self, root: &str) -> Vec<(String, Arc<Mount>, u32)> {
        let table = self.snapshot();
        let root = normalize_path(root, "/");
        let Some(top) = table.root.clone() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let bottom_id = table
            .covered
            .get("/")
            .and_then(|s| s.first())
            .map_or(top.ids.id, |m| m.ids.id);
        let points = core::iter::once(String::from("/")).chain(table.mounts.keys().cloned());
        for path in points {
            if !path_is_under(&path, &root) {
                continue;
            }
            let mut parent = if path == "/" {
                bottom_id
            } else {
                let up = match path.rfind('/') {
                    Some(0) | None => "/",
                    Some(i) => &path[..i],
                };
                table
                    .mount_at(table.mount_of(up))
                    .map_or(bottom_id, |m| m.ids.id)
            };
            let shown = if root == "/" {
                path.clone()
            } else if path == root {
                String::from("/")
            } else {
                String::from(&path[root.len()..])
            };
            for mount in table.stack(&path) {
                let id = mount.ids.id;
                out.push((shown.clone(), mount, parent));
                parent = id;
            }
        }
        out
    }

    /// `/proc/<pid>/mounts` for a process whose root is `root` (Linux's
    /// show_vfsmnt): source, mount point, type, options, `0 0`.
    pub fn render_mounts(&self, root: &str) -> String {
        let mut out = String::new();
        for (point, mount, _) in self.visible(root) {
            mangle(&mut out, &mount.source);
            out.push(' ');
            escape(&mut out, &point);
            out.push(' ');
            mangle(&mut out, linux_fs_type(mount.fs().name()));
            out.push(' ');
            options(&mut out, mount.is_readonly(), mount.flags());
            out.push_str(" 0 0\n");
        }
        out
    }

    /// `/proc/<pid>/mountinfo` for a process whose root is `root` (Linux's
    /// show_mountinfo): id, parent id, device, the mount's root in its
    /// filesystem, mount point, mount options, the optional fields (only
    /// `unbindable`: there is no propagation), `-`, type, source,
    /// superblock options.
    pub fn render_mountinfo(&self, root: &str) -> String {
        let mut out = String::new();
        for (point, mount, parent) in self.visible(root) {
            let mnt = mount.flags();
            let _ = write!(out, "{} {} 0:{} ", mount.ids.id, parent, mount.ids.dev);
            escape(&mut out, &mount.root_path);
            out.push(' ');
            escape(&mut out, &point);
            out.push(' ');
            options(&mut out, mnt & flags::READONLY != 0, mnt);
            if mnt & flags::UNBINDABLE != 0 {
                out.push_str(" unbindable");
            }
            out.push_str(" - ");
            mangle(&mut out, linux_fs_type(mount.fs().name()));
            out.push(' ');
            mangle(&mut out, &mount.source);
            out.push_str(if mount.sb_readonly() {
                " ro\n"
            } else {
                " rw\n"
            });
        }
        out
    }

    /// The flags of the mount at mount point `point`, if it is one.
    pub fn flags_at(&self, point: &str) -> Option<u32> {
        self.snapshot().mount_at(point).map(|m| m.flags())
    }

    /// Mount a new filesystem at `point` (Linux's do_new_mount): a
    /// superblock read-only if `sb_readonly`, a mount with the flags `mnt`.
    pub fn attach_new(
        &self,
        point: &str,
        point_is_dir: bool,
        fs: Arc<dyn Filesystem>,
        source: String,
        mnt: u32,
        sb_readonly: bool,
    ) -> Result<(), MountError> {
        let mount = Mount::with_flags(fs, source, &self.pool, mnt, sb_readonly);
        self.update(|t| t.attach(point, point_is_dir, mount))
    }

    /// Mount `node`, which is at canonical path `source`, at `point` (Linux's
    /// do_loopback): the mount serving `source` again, rooted there, and
    /// with `recursive` (MS_REC) every mount under `source` too except the
    /// unbindable ones and what is under them.
    pub fn bind(
        &self,
        source: &str,
        node: Arc<dyn VfsNode>,
        point: &str,
        point_is_dir: bool,
        recursive: bool,
    ) -> Result<(), MountError> {
        self.update(|t| {
            let at = String::from(t.mount_of(source));
            let base = t.mount_at(&at).cloned().ok_or(MountError::Invalid)?;
            if base.flags() & flags::UNBINDABLE != 0 {
                return Err(MountError::Invalid);
            }
            let root_path = join_root(&base.root_path, relative(source, &at));
            let mount = base.bound(node, root_path);
            let mut copies = Vec::new();
            if recursive {
                let mut skipped: Vec<String> = Vec::new();
                for (path, top) in &t.mounts {
                    if path == source || !path_is_under(path, source) {
                        continue;
                    }
                    if skipped.iter().any(|s| path_is_under(path, s)) {
                        continue;
                    }
                    if top.flags() & flags::UNBINDABLE != 0 {
                        skipped.push(path.clone());
                        continue;
                    }
                    let stack: Vec<_> = t.stack(path).iter().map(|m| m.copy()).collect();
                    copies.push((rebase(path, source, point), stack));
                }
            }
            t.attach(point, point_is_dir, mount)?;
            for (path, stack) in copies {
                for m in stack {
                    t.push(&path, m);
                }
            }
            Ok(())
        })
    }

    /// Move the mount at `from`, with everything mounted under it, to `to`
    /// (Linux's do_move_mount): EINVAL unless `from` is a mount point other
    /// than the root or the two are of different kinds, ELOOP into itself.
    pub fn move_mount(
        &self,
        from: &str,
        from_is_dir: bool,
        to: &str,
        to_is_dir: bool,
    ) -> Result<(), MountError> {
        self.update(|t| {
            if from == "/" || !t.mounts.contains_key(from) || from_is_dir != to_is_dir {
                return Err(MountError::Invalid);
            }
            if path_is_under(to, from) {
                return Err(MountError::Loop);
            }
            let top = t.pop(from).ok_or(MountError::Invalid)?;
            let under: Vec<String> = t.under(from);
            let moved: Vec<_> = under
                .into_iter()
                .map(|path| {
                    let stack = t.take_stack(&path);
                    (rebase(&path, from, to), stack)
                })
                .collect();
            t.push(to, top);
            for (path, stack) in moved {
                for m in stack {
                    t.push(&path, m);
                }
            }
            Ok(())
        })
    }

    /// The mount at `point` gets the mount flags `mnt` and, unless only
    /// the mount changes (`sb_readonly` None: MS_REMOUNT|MS_BIND, Linux's
    /// do_reconfigure_mnt), a read-only or writable superblock (do_remount).
    /// Making the mount or its superblock read-only while `writers()` says
    /// files under it are open for writing is EBUSY (Linux's
    /// mnt_make_readonly and sb_prepare_remount_readonly).
    pub fn reconfigure(
        &self,
        point: &str,
        mnt: u32,
        sb_readonly: Option<bool>,
        writers: impl FnOnce() -> bool,
    ) -> Result<(), MountError> {
        let table = self.snapshot();
        let mount = table.mount_at(point).ok_or(MountError::Invalid)?;
        let mount_to_ro = mnt & flags::READONLY != 0 && !mount.has(flags::READONLY);
        let sb_to_ro = sb_readonly == Some(true) && !mount.sb_readonly();
        if (mount_to_ro || sb_to_ro) && writers() {
            return Err(MountError::Busy);
        }
        if let Some(ro) = sb_readonly {
            mount
                .sb
                .readonly
                .store(ro || mount.fs().is_readonly(), Ordering::Release);
        }
        mount.set_flags(mnt);
        Ok(())
    }

    /// The propagation type of the mount at `point`, and with `recursive`
    /// of every mount under it, becomes `ty` (Linux's do_change_type).
    /// Mounts do not propagate here, so a private and a slave mount are
    /// the same, and making one shared is refused (EINVAL; ADR 0022).
    pub fn change_type(
        &self,
        point: &str,
        ty: Propagation,
        recursive: bool,
    ) -> Result<(), MountError> {
        let table = self.snapshot();
        let top = table.mount_at(point).ok_or(MountError::Invalid)?;
        if ty == Propagation::Shared {
            return Err(MountError::Invalid);
        }
        let unbindable = ty == Propagation::Unbindable;
        top.set_unbindable(unbindable);
        if recursive {
            for path in table.under(point) {
                for m in table.stack(&path) {
                    m.set_unbindable(unbindable);
                }
            }
        }
        Ok(())
    }

    /// Unmount the mount at `point` (Linux's do_umount) for a caller whose
    /// root is canonical `root`; `busy` is whether something holds a file
    /// in it (open, working directory or root), checked by the caller.
    /// The caller's root mount without `detach` becomes read-only instead
    /// (do_umount_root); the namespace's own root cannot go (EINVAL).
    /// `detach` takes everything under the mount with it; otherwise a
    /// mount under it, or `busy`, is EBUSY. `expire` unmounts only on the
    /// second call (the first marks the mount, EAGAIN).
    pub fn umount(
        &self,
        point: &str,
        how: Umount,
        root: &str,
        busy: bool,
    ) -> Result<(), MountError> {
        self.update(|t| {
            let mount = t.mount_at(point).cloned().ok_or(MountError::Invalid)?;
            let holds_root = t.mount_of(&normalize_path(root, "/")) == point;
            let children = !t.under(point).is_empty();
            if how.expire {
                if holds_root || how.force || how.detach {
                    return Err(MountError::Invalid);
                }
                if children || busy {
                    return Err(MountError::Busy);
                }
                if !mount.expiry.swap(true, Ordering::AcqRel) {
                    return Err(MountError::Again);
                }
            }
            if holds_root && !how.detach {
                mount.sb.readonly.store(true, Ordering::Release);
                return Ok(());
            }
            if point == "/" && t.covered.get("/").is_none_or(|s| s.is_empty()) {
                return Err(MountError::Invalid);
            }
            if how.detach {
                for path in t.under(point) {
                    t.take_stack(&path);
                }
            } else if children || busy {
                return Err(MountError::Busy);
            }
            t.pop(point);
            Ok(())
        })
    }
}

impl MountTable {
    /// The mounts at `point`, the covered ones first and the top last.
    fn stack(&self, point: &str) -> Vec<Arc<Mount>> {
        self.covered
            .get(point)
            .into_iter()
            .flatten()
            .cloned()
            .chain(self.mount_at(point).cloned())
            .collect()
    }

    /// The mount points under `point`, not counting `point` itself.
    fn under(&self, point: &str) -> Vec<String> {
        self.mounts
            .keys()
            .filter(|p| p.as_str() != point && path_is_under(p, point))
            .cloned()
            .collect()
    }

    /// Put `mount` on top of whatever is at `point`.
    fn push(&mut self, point: &str, mount: Arc<Mount>) {
        let old = if point == "/" {
            self.root.replace(mount)
        } else {
            self.mounts.insert(String::from(point), mount)
        };
        if let Some(old) = old {
            self.covered
                .entry(String::from(point))
                .or_default()
                .push(old);
        }
    }

    /// Take the top mount at `point`; the one it covered, if any, is the
    /// top again.
    fn pop(&mut self, point: &str) -> Option<Arc<Mount>> {
        let below = self.covered.get_mut(point).and_then(|s| s.pop());
        if self.covered.get(point).is_some_and(|s| s.is_empty()) {
            self.covered.remove(point);
        }
        if point == "/" {
            let top = self.root.take();
            self.root = below;
            top
        } else {
            let top = self.mounts.remove(point);
            if let Some(below) = below {
                self.mounts.insert(String::from(point), below);
            }
            top
        }
    }

    /// Take every mount at `point` (not the root), bottom first.
    fn take_stack(&mut self, point: &str) -> Vec<Arc<Mount>> {
        let mut stack = self.covered.remove(point).unwrap_or_default();
        stack.extend(self.mounts.remove(point));
        stack
    }

    /// Linux's do_add_mount and graft_tree: the same filesystem and root
    /// as the mount already at `point` is EBUSY, a directory over a file
    /// or a file over a directory ENOTDIR.
    fn attach(
        &mut self,
        point: &str,
        point_is_dir: bool,
        mount: Arc<Mount>,
    ) -> Result<(), MountError> {
        if self.root.is_none() {
            return Err(MountError::Invalid);
        }
        if self
            .mount_at(point)
            .is_some_and(|top| top.same_place(&mount))
        {
            return Err(MountError::Busy);
        }
        if point_is_dir != (mount.root.node_type() == NodeType::Directory) {
            return Err(MountError::NotDirectory);
        }
        self.push(point, mount);
        Ok(())
    }
}

/// `path` relative to `at`, a mount point it is under: "" or "/a/b".
fn relative<'a>(path: &'a str, at: &str) -> &'a str {
    if at == "/" {
        if path == "/" {
            ""
        } else {
            path
        }
    } else {
        &path[at.len()..]
    }
}

/// `rel` ("" or "/a/b") under `root` ("/" or "/x").
fn join_root(root: &str, rel: &str) -> String {
    match (root, rel) {
        (r, "") => String::from(r),
        ("/", rel) => String::from(rel),
        (r, rel) => alloc::format!("{r}{rel}"),
    }
}

/// `path`, under `from`, moved to under `to`.
fn rebase(path: &str, from: &str, to: &str) -> String {
    join_root(to, relative(path, from))
}

/// Linux's mount options as /proc shows them: `ro` or `rw`, then the
/// mount-point flags (show_vfsmnt_opts).
fn options(out: &mut String, readonly: bool, mnt: u32) {
    out.push_str(if readonly { "ro" } else { "rw" });
    for (flag, name) in [
        (flags::NOSUID, ",nosuid"),
        (flags::NODEV, ",nodev"),
        (flags::NOEXEC, ",noexec"),
        (flags::NOATIME, ",noatime"),
        (flags::NODIRATIME, ",nodiratime"),
        (flags::RELATIME, ",relatime"),
        (flags::NOSYMFOLLOW, ",nosymfollow"),
    ] {
        if mnt & flag != 0 {
            out.push_str(name);
        }
    }
}

/// `s` with space, tab, newline and backslash as octal escapes, as Linux
/// writes paths (`seq_path_root`, `seq_dentry` with " \t\n\\").
fn escape(out: &mut String, s: &str) {
    escape_set(out, s, " \t\n\\");
}

/// As [`escape`] and `#` too, as Linux writes sources and types (mangle).
fn mangle(out: &mut String, s: &str) {
    escape_set(out, s, " \t\n\\#");
}

fn escape_set(out: &mut String, s: &str, set: &str) {
    for c in s.chars() {
        if set.contains(c) {
            let _ = write!(out, "\\{:03o}", c as u32);
        } else {
            out.push(c);
        }
    }
}

/// Unique ids start past Linux's MNT_UNIQUE_ID_OFFSET.
const _: () = assert!(PSEUDO_MOUNT.unique > MNT_UNIQUE_ID_OFFSET);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{ramfs::RamFs, tmpfs::TmpFs, Vfs};

    fn tree() -> Vfs {
        let vfs = Vfs::new();
        vfs.mount_root(Arc::new(RamFs::new())).unwrap();
        vfs.mount("/dev".into(), Arc::new(crate::fs::devfs::DevFs::new()))
            .unwrap();
        vfs.mount("/dev/shm".into(), Arc::new(TmpFs::new(1 << 20)))
            .unwrap();
        vfs.mount("/proc".into(), Arc::new(crate::fs::procfs::ProcFs::new()))
            .unwrap();
        vfs
    }

    #[test]
    fn a_copy_has_new_mounts_of_the_same_filesystems() {
        let vfs = tree();
        let old = vfs.mounts().snapshot();
        let copy = vfs.mounts().copy(false);
        let new = copy.snapshot();
        assert_eq!(
            new.mounts.keys().collect::<Vec<_>>(),
            ["/dev", "/dev/shm", "/proc"]
        );
        let (a, b) = (old.root.as_ref().unwrap(), new.root.as_ref().unwrap());
        assert!(Arc::ptr_eq(a.fs(), b.fs()));
        // New ids, after the old ones (in order: root, /dev, /dev/shm,
        // /proc), a new unique id; the same device.
        assert_eq!((a.ids.id, b.ids.id), (1, 5));
        assert_eq!(new.mounts["/proc"].ids.id, 8);
        assert!(b.ids.unique > old.mounts["/proc"].ids.unique);
        assert_eq!(a.ids.dev, b.ids.dev);
        assert_eq!(
            old.mounts["/dev/shm"].ids.dev,
            new.mounts["/dev/shm"].ids.dev
        );
        // A change in one tree is not seen in the other.
        copy.update(|t| t.mounts.remove("/proc"));
        assert!(vfs.mounted("/proc").is_some());
    }

    #[test]
    fn an_empty_copy_has_only_the_root() {
        let vfs = tree();
        let copy = vfs.mounts().copy(true);
        let new = copy.snapshot();
        assert!(new.mounts.is_empty());
        assert!(Arc::ptr_eq(
            new.root.as_ref().unwrap().fs(),
            vfs.root_fs().as_ref().unwrap()
        ));
    }

    #[test]
    fn ids_and_devices_are_freed_with_the_last_reference() {
        let vfs = tree();
        let copy = vfs.mounts().copy(false);
        let shm = copy.snapshot().mounts["/dev/shm"].ids;
        copy.update(|t| t.mounts.remove("/dev/shm"));
        // The id is free again; the device is not, the original mount
        // still has the filesystem.
        let again = Mount::new(Arc::new(RamFs::new()), "x".into(), copy.pool());
        assert_eq!(again.ids.id, shm.id);
        assert_ne!(again.ids.dev, shm.dev);
        drop(again);
        // A walk on an old snapshot keeps its mounts' ids.
        let before = copy.snapshot();
        let proc_id = before.mounts["/proc"].ids.id;
        copy.update(|t| t.mounts.remove("/proc"));
        let held = Mount::new(Arc::new(RamFs::new()), "x".into(), copy.pool());
        assert_ne!(held.ids.id, proc_id);
        drop(before);
        drop(held);
        let freed = Mount::new(Arc::new(RamFs::new()), "x".into(), copy.pool());
        assert_eq!(freed.ids.id, proc_id.min(shm.id));
    }

    #[test]
    fn mounts_and_mountinfo_read_as_linuxs() {
        let vfs = tree();
        assert_eq!(
            vfs.mounts().render_mounts("/"),
            "ramfs / ramfs rw,relatime 0 0\ndevtmpfs /dev devtmpfs rw,relatime 0 0\ntmpfs \
             /dev/shm tmpfs rw,relatime 0 0\nproc /proc proc rw,relatime 0 0\n"
        );
        assert_eq!(
            vfs.mounts().render_mountinfo("/"),
            "1 1 0:1 / / rw,relatime - ramfs ramfs rw\n2 1 0:2 / /dev rw,relatime - devtmpfs \
             devtmpfs rw\n3 2 0:3 / /dev/shm rw,relatime - tmpfs tmpfs rw\n4 1 0:4 / /proc \
             rw,relatime - proc proc rw\n"
        );
    }

    #[test]
    fn a_chrooted_reader_sees_only_the_mounts_under_its_root() {
        let vfs = tree();
        assert_eq!(
            vfs.mounts().render_mounts("/dev"),
            "devtmpfs / devtmpfs rw,relatime 0 0\ntmpfs /shm tmpfs rw,relatime 0 0\n"
        );
        assert_eq!(vfs.mounts().render_mountinfo("/nowhere"), "");
    }

    #[test]
    fn names_are_escaped_and_disk_roots_are_dev_root() {
        let vfs = Vfs::new();
        vfs.mount_root(Arc::new(RamFs::new())).unwrap();
        vfs.root_fs()
            .unwrap()
            .root()
            .mkdir("a b", crate::fs::Permissions::default())
            .unwrap();
        vfs.mount("/a b".into(), Arc::new(TmpFs::new(1 << 20)))
            .unwrap();
        assert!(vfs
            .mounts()
            .render_mounts("/")
            .contains("tmpfs /a\\040b tmpfs rw,relatime 0 0\n"));
        let blockfs = crate::fs::blockfs::BlockFs::new(64, 16);
        assert_eq!(default_source(&blockfs, true), "/dev/root");
        assert_eq!(default_source(&blockfs, false), "blockfs");
        assert_eq!(default_source(&RamFs::new(), true), "ramfs");
    }

    /// A tree with directories /a/sub (holding file f), /b, /c and file
    /// /file, and a tmpfs on /a/sub/m.
    fn work_tree() -> Vfs {
        let vfs = Vfs::new();
        vfs.mount_root(Arc::new(RamFs::new())).unwrap();
        let root = vfs.root_fs().unwrap().root();
        let p = crate::fs::Permissions::default();
        let a = root.mkdir("a", p).unwrap();
        let sub = a.mkdir("sub", p).unwrap();
        sub.create("f", p).unwrap();
        sub.mkdir("m", p).unwrap();
        root.mkdir("b", p).unwrap();
        root.mkdir("c", p).unwrap();
        root.create("file", p).unwrap();
        vfs.mount("/a/sub/m".into(), Arc::new(TmpFs::new(1 << 20)))
            .unwrap();
        vfs
    }

    fn tmpfs() -> Arc<dyn Filesystem> {
        Arc::new(TmpFs::new(1 << 20))
    }

    fn lines(vfs: &Vfs) -> Vec<String> {
        vfs.mounts()
            .render_mountinfo("/")
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn a_mount_over_a_mount_point_covers_it_until_unmounted() {
        let vfs = work_tree();
        let t = vfs.mounts();
        t.attach_new("/b", true, tmpfs(), "one".into(), flags::RELATIME, false)
            .unwrap();
        let first = vfs.mounted("/b").unwrap();
        t.attach_new("/b", true, tmpfs(), "two".into(), flags::RELATIME, false)
            .unwrap();
        let info = lines(&vfs);
        let one = info.iter().find(|l| l.ends_with("tmpfs one rw")).unwrap();
        let two = info.iter().find(|l| l.ends_with("tmpfs two rw")).unwrap();
        // The top's parent is the mount it covers, listed before it.
        let one_id = one.split(' ').next().unwrap();
        assert_eq!(two.split(' ').nth(1), Some(one_id));
        assert!(!Arc::ptr_eq(&vfs.mounted("/b").unwrap(), &first));
        t.umount("/b", Umount::default(), "/", false).unwrap();
        assert!(Arc::ptr_eq(&vfs.mounted("/b").unwrap(), &first));
        t.umount("/b", Umount::default(), "/", false).unwrap();
        assert!(vfs.mounted("/b").is_none());
        assert_eq!(
            t.umount("/b", Umount::default(), "/", false),
            Err(MountError::Invalid)
        );
    }

    #[test]
    fn a_bind_mount_shows_a_directory_again_with_or_without_its_submounts() {
        let vfs = work_tree();
        let t = vfs.mounts();
        let sub = vfs.resolve_path("/a/sub").unwrap();
        t.bind("/a/sub", sub.clone(), "/b", true, false).unwrap();
        let f = vfs.resolve_path("/b/f").unwrap();
        assert_eq!(
            f.metadata().unwrap().inode,
            vfs.resolve_path("/a/sub/f")
                .unwrap()
                .metadata()
                .unwrap()
                .inode
        );
        // Its root in the filesystem, and the same device as the original.
        let b = lines(&vfs)
            .into_iter()
            .find(|l| l.contains(" /b "))
            .unwrap();
        assert!(
            b.starts_with("3 1 0:1 /a/sub /b rw,relatime - ramfs"),
            "{b}"
        );
        // Without MS_REC the tmpfs on m is not there; with it, it is.
        assert!(vfs.mounted("/b/m").is_none());
        t.bind("/a/sub", sub, "/c", true, true).unwrap();
        assert!(Arc::ptr_eq(
            &vfs.mounted("/c/m").unwrap(),
            &vfs.mounted("/a/sub/m").unwrap()
        ));
        // Binding a mount onto itself is EBUSY; a directory onto a file
        // ENOTDIR; a file onto a file works.
        let c = vfs.resolve_path("/c").unwrap();
        assert_eq!(t.bind("/c", c, "/c", true, false), Err(MountError::Busy));
        let dir = vfs.resolve_path("/a").unwrap();
        assert_eq!(
            t.bind("/a", dir, "/file", false, false),
            Err(MountError::NotDirectory)
        );
        let f = vfs.resolve_path("/a/sub/f").unwrap();
        t.bind("/a/sub/f", f, "/file", false, false).unwrap();
    }

    #[test]
    fn unbindable_mounts_are_not_bound_and_not_copied_by_a_recursive_bind() {
        let vfs = work_tree();
        let t = vfs.mounts();
        t.change_type("/a/sub/m", Propagation::Unbindable, false)
            .unwrap();
        assert!(lines(&vfs)
            .iter()
            .any(|l| l.contains(" /a/sub/m rw,relatime unbindable - tmpfs")));
        let m = vfs.resolve_path("/a/sub/m").unwrap();
        assert_eq!(
            t.bind("/a/sub/m", m, "/b", true, false),
            Err(MountError::Invalid)
        );
        let sub = vfs.resolve_path("/a/sub").unwrap();
        t.bind("/a/sub", sub, "/b", true, true).unwrap();
        assert!(vfs.mounted("/b/m").is_none());
        // Private again; shared is refused (no propagation); only mount
        // points have a type.
        t.change_type("/a/sub/m", Propagation::Private, false)
            .unwrap();
        assert!(!lines(&vfs).iter().any(|l| l.contains("unbindable")));
        assert_eq!(
            t.change_type("/a/sub/m", Propagation::Shared, false),
            Err(MountError::Invalid)
        );
        assert_eq!(
            t.change_type("/a", Propagation::Private, false),
            Err(MountError::Invalid)
        );
        t.change_type("/", Propagation::Slave, true).unwrap();
    }

    #[test]
    fn moving_a_mount_takes_its_submounts() {
        let vfs = work_tree();
        let t = vfs.mounts();
        t.attach_new("/b", true, tmpfs(), "x".into(), 0, false)
            .unwrap();
        let b = vfs.resolve_path("/b").unwrap();
        b.mkdir("in", crate::fs::Permissions::default()).unwrap();
        t.attach_new("/b/in", true, tmpfs(), "y".into(), 0, false)
            .unwrap();
        let inner = vfs.mounted("/b/in").unwrap();
        t.move_mount("/b", true, "/c", true).unwrap();
        assert!(vfs.mounted("/b").is_none() && vfs.mounted("/b/in").is_none());
        assert!(Arc::ptr_eq(&vfs.mounted("/c/in").unwrap(), &inner));
        // Into itself ELOOP; not a mount point, the root, or a directory
        // onto a file EINVAL.
        assert_eq!(
            t.move_mount("/c", true, "/c/in", true),
            Err(MountError::Loop)
        );
        assert_eq!(
            t.move_mount("/a", true, "/b", true),
            Err(MountError::Invalid)
        );
        assert_eq!(
            t.move_mount("/", true, "/b", true),
            Err(MountError::Invalid)
        );
        assert_eq!(
            t.move_mount("/c", true, "/file", false),
            Err(MountError::Invalid)
        );
    }

    #[test]
    fn remount_changes_the_mount_and_maybe_the_superblock() {
        let vfs = work_tree();
        let t = vfs.mounts();
        let info = |vfs: &Vfs| {
            lines(vfs)
                .into_iter()
                .find(|l| l.contains(" /a/sub/m "))
                .unwrap()
        };
        // Not read-only while a file under it is open for writing.
        assert_eq!(
            t.reconfigure("/a/sub/m", flags::READONLY, None, || true),
            Err(MountError::Busy)
        );
        assert_eq!(
            t.reconfigure("/a/sub/m", flags::RELATIME, Some(true), || true),
            Err(MountError::Busy)
        );
        assert_eq!(t.flags_at("/a/sub/m"), Some(flags::RELATIME));
        // MS_REMOUNT|MS_BIND: the mount only.
        t.reconfigure("/a/sub/m", flags::READONLY | flags::NOSUID, None, || false)
            .unwrap();
        // Already read-only: writers do not matter.
        t.reconfigure("/a/sub/m", flags::READONLY | flags::NOSUID, None, || true)
            .unwrap();
        assert!(
            info(&vfs).contains(" ro,nosuid - tmpfs tmpfs rw"),
            "{}",
            info(&vfs)
        );
        assert!(t
            .render_mounts("/")
            .contains(" /a/sub/m tmpfs ro,nosuid 0 0"));
        // MS_REMOUNT: the superblock too.
        t.reconfigure("/a/sub/m", flags::RELATIME, Some(true), || false)
            .unwrap();
        assert!(info(&vfs).contains(" rw,relatime - tmpfs tmpfs ro"));
        // A read-only superblock makes /proc/mounts say ro.
        assert!(t
            .render_mounts("/")
            .contains(" /a/sub/m tmpfs ro,relatime 0 0"));
        assert_eq!(
            t.reconfigure("/a", 0, None, || false),
            Err(MountError::Invalid)
        );
        assert_eq!(t.flags_at("/a/sub/m"), Some(flags::RELATIME));
    }

    #[test]
    fn a_read_only_mount_makes_nothing_and_nosymfollow_follows_nothing() {
        use crate::error::{FsError, KernelError};
        let vfs = work_tree();
        let t = vfs.mounts();
        let p = crate::fs::Permissions::default();
        vfs.mkdir("/a/sub/m/d", p).unwrap();
        vfs.resolve_path("/a/sub/m")
            .unwrap()
            .symlink("l", "d")
            .unwrap();
        let fs_err = |e| Err::<(), _>(KernelError::FsError(e));
        // Read-only: an existing name is EEXIST, a new one EROFS; the
        // directory above the mount is unaffected.
        t.reconfigure("/a/sub/m", flags::READONLY, None, || false)
            .unwrap();
        assert_eq!(
            vfs.mkdir("/a/sub/m/d", p).map(|_| ()),
            fs_err(FsError::AlreadyExists)
        );
        assert_eq!(
            vfs.mkdir("/a/sub/m/e", p).map(|_| ()),
            fs_err(FsError::ReadOnly)
        );
        assert!(vfs.mkdir("/a/sub/e", p).is_ok());
        // A read-only superblock refuses as well.
        t.reconfigure("/a/sub/m", flags::RELATIME, Some(true), || false)
            .unwrap();
        assert_eq!(
            vfs.mkdir("/a/sub/m/e", p).map(|_| ()),
            fs_err(FsError::ReadOnly)
        );
        t.reconfigure("/a/sub/m", flags::RELATIME, Some(false), || false)
            .unwrap();
        assert!(vfs.mkdir("/a/sub/m/e", p).is_ok());
        // nosymfollow: the link is not followed, last or in the middle, but
        // is found itself.
        assert!(vfs.resolve_path("/a/sub/m/l").is_ok());
        t.reconfigure("/a/sub/m", flags::NOSYMFOLLOW, None, || false)
            .unwrap();
        for path in ["/a/sub/m/l", "/a/sub/m/l/x"] {
            assert_eq!(
                vfs.resolve_path(path).map(|_| ()),
                fs_err(FsError::SymlinkLoop),
                "{path}"
            );
        }
        assert!(vfs.resolve_path_no_follow("/a/sub/m/l").is_ok());
    }

    #[test]
    fn umount_refuses_busy_mounts_unless_detached() {
        let vfs = work_tree();
        let t = vfs.mounts();
        let plain = Umount::default();
        let detach = Umount {
            detach: true,
            ..plain
        };
        let sub = vfs.resolve_path("/a/sub").unwrap();
        t.bind("/a/sub", sub, "/b", true, true).unwrap();
        // A mount under it, or a file in use: EBUSY.
        assert_eq!(t.umount("/b", plain, "/", false), Err(MountError::Busy));
        assert_eq!(t.umount("/b/m", plain, "/", true), Err(MountError::Busy));
        // Detached, it goes with what is under it.
        t.umount("/b", detach, "/", false).unwrap();
        assert!(vfs.mounted("/b/m").is_none());
        // MNT_EXPIRE: marked, EAGAIN, then unmounted.
        let expire = Umount {
            expire: true,
            ..plain
        };
        assert_eq!(
            t.umount("/a/sub/m", expire, "/", false),
            Err(MountError::Again)
        );
        t.umount("/a/sub/m", expire, "/", false).unwrap();
        assert_eq!(t.umount("/a", expire, "/", false), Err(MountError::Invalid));
        // The caller's root mount turns read-only instead; the namespace
        // root cannot be detached.
        t.umount("/", plain, "/", false).unwrap();
        assert!(t
            .render_mounts("/")
            .starts_with("ramfs / ramfs ro,relatime"));
        assert_eq!(t.umount("/", detach, "/", false), Err(MountError::Invalid));
    }

    #[test]
    fn mount_makes_the_filesystems_proc_filesystems_lists() {
        let owner = (1000, 100);
        for ty in MOUNTABLE {
            assert!(new_filesystem(ty, "", owner).is_ok(), "{ty}");
        }
        assert_eq!(
            new_filesystem("ext9", "", owner).err(),
            Some(NewFsError::UnknownType)
        );
        let fs = new_filesystem("tmpfs", "size=1m,mode=700,uid=5,gid=6", owner).unwrap();
        let meta = fs.root().metadata().unwrap();
        assert_eq!((meta.uid, meta.gid), (5, 6));
        assert_eq!(meta.permissions.to_mode() & 0o7777, 0o700);
        let owned = new_filesystem("tmpfs", "", owner).unwrap();
        let meta = owned.root().metadata().unwrap();
        assert_eq!((meta.uid, meta.gid), owner);
        assert_eq!(meta.permissions.to_mode() & 0o7777, 0o1777);
        for bad in ["size=1x", "nosuch", "mode=9", "size"] {
            assert_eq!(
                new_filesystem("tmpfs", bad, owner).err(),
                Some(NewFsError::BadOption),
                "{bad}"
            );
        }
        assert!(new_filesystem("ramfs", "mode=755,whatever", owner).is_ok());
        assert_eq!(
            new_filesystem("proc", "hidepid=2", owner).err(),
            Some(NewFsError::BadOption)
        );
        assert_eq!(tmpfs_size("2k"), Ok(2048));
        assert_eq!(tmpfs_size("3G"), Ok(3 << 30));
    }
}

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
    sync::atomic::{AtomicU64, Ordering},
};

use spin::{Mutex, RwLock};

use super::{
    normalize_path, path_is_under, Filesystem, MountIds, MountTable, MNT_UNIQUE_ID_OFFSET,
    PSEUDO_MOUNT,
};

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

/// A filesystem instance as mounts see it: the filesystem and its device
/// number, freed when no mount of it is left.
pub(super) struct Superblock {
    fs: Arc<dyn Filesystem>,
    dev: u32,
    pool: Arc<IdPool>,
}

impl Drop for Superblock {
    fn drop(&mut self) {
        self.pool.devs.lock().remove(&self.dev);
    }
}

/// One mount: a filesystem attached at a place in one tree.
pub struct Mount {
    sb: Arc<Superblock>,
    ids: MountIds,
    /// What `/proc/<pid>/mounts` names as its source (Linux's mnt_devname).
    source: String,
}

impl Drop for Mount {
    fn drop(&mut self) {
        self.sb.pool.ids.lock().remove(&self.ids.id);
    }
}

impl Mount {
    /// A mount of a filesystem not mounted before (a new superblock).
    pub(super) fn new(fs: Arc<dyn Filesystem>, source: String, pool: &Arc<IdPool>) -> Arc<Self> {
        let sb = Arc::new(Superblock {
            fs,
            dev: IdPool::take(&pool.devs),
            pool: pool.clone(),
        });
        Self::on(sb, source)
    }

    fn on(sb: Arc<Superblock>, source: String) -> Arc<Self> {
        let id = IdPool::take(&sb.pool.ids);
        let unique = sb.pool.last_unique.fetch_add(1, Ordering::Relaxed) + 1;
        let dev = sb.dev;
        Arc::new(Self {
            sb,
            ids: MountIds { id, unique, dev },
            source,
        })
    }

    /// A new mount of the same filesystem (Linux's clone_mnt).
    fn copy(&self) -> Arc<Self> {
        Self::on(self.sb.clone(), self.source.clone())
    }

    pub fn fs(&self) -> &Arc<dyn Filesystem> {
        &self.sb.fs
    }

    pub fn ids(&self) -> MountIds {
        self.ids
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
        let mut table = MountTable {
            root: old.root.as_ref().map(|m| m.copy()),
            ..MountTable::default()
        };
        if !empty {
            for (path, mount) in &old.mounts {
                table.mounts.insert(path.clone(), mount.copy());
            }
        }
        Self::with_table(table, self.pool.clone())
    }

    /// The mounts a process whose root is `root` (a canonical path) can
    /// see, as `(mount point relative to root, mount, parent id)`, root
    /// first: Linux's m_start walk with the mounts outside the root left
    /// out. The namespace root's parent is itself, as in Linux.
    fn visible(&self, root: &str) -> Vec<(String, Arc<Mount>, u32)> {
        let table = self.snapshot();
        let root = normalize_path(root, "/");
        let Some(top) = table.root.clone() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let top_id = top.ids.id;
        let all = core::iter::once((String::from("/"), top)).chain(
            table
                .mounts
                .iter()
                .map(|(path, mount)| (path.clone(), mount.clone())),
        );
        for (path, mount) in all {
            if !path_is_under(&path, &root) {
                continue;
            }
            let parent = if path == "/" {
                top_id
            } else {
                let up = match path.rfind('/') {
                    Some(0) | None => "/",
                    Some(i) => &path[..i],
                };
                let at = table.mount_of(up);
                if at == "/" {
                    top_id
                } else {
                    table.mounts.get(at).map_or(top_id, |m| m.ids.id)
                }
            };
            let shown = if root == "/" {
                path
            } else if path == root {
                String::from("/")
            } else {
                String::from(&path[root.len()..])
            };
            out.push((shown, mount, parent));
        }
        out
    }

    /// `/proc/<pid>/mounts` for a process whose root is `root` (Linux's
    /// show_vfsmnt): source, mount point, type, options, `0 0`.
    pub fn render_mounts(&self, root: &str) -> String {
        let mut out = String::new();
        for (point, mount, _) in self.visible(root) {
            escape(&mut out, &mount.source);
            out.push(' ');
            escape(&mut out, &point);
            let _ = writeln!(
                out,
                " {} {} 0 0",
                linux_fs_type(mount.fs().name()),
                rw(mount.fs().as_ref())
            );
        }
        out
    }

    /// `/proc/<pid>/mountinfo` for a process whose root is `root` (Linux's
    /// show_mountinfo): id, parent id, device, root of the mount (all of
    /// it: there are no bind mounts yet), mount point, mount options, no
    /// propagation fields, `-`, type, source, superblock options.
    pub fn render_mountinfo(&self, root: &str) -> String {
        let mut out = String::new();
        for (point, mount, parent) in self.visible(root) {
            let flags = rw(mount.fs().as_ref());
            let _ = write!(out, "{} {} 0:{} / ", mount.ids.id, parent, mount.ids.dev);
            escape(&mut out, &point);
            let _ = write!(out, " {} - {} ", flags, linux_fs_type(mount.fs().name()));
            escape(&mut out, &mount.source);
            let _ = writeln!(out, " {}", flags);
        }
        out
    }
}

/// `ro` or `rw`, as the options start.
fn rw(fs: &dyn Filesystem) -> &'static str {
    if fs.is_readonly() {
        "ro"
    } else {
        "rw"
    }
}

/// `s` with space, tab, newline and backslash as octal escapes (Linux's
/// mangle, `seq_escape(m, s, " \t\n\\")`).
fn escape(out: &mut String, s: &str) {
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\\') {
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
    use crate::fs::{ramfs::RamFs, tmpfs::TmpFs, Vfs, VfsNode};

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
            "ramfs / ramfs rw 0 0\ndevtmpfs /dev devtmpfs rw 0 0\ntmpfs /dev/shm tmpfs rw 0 \
             0\nproc /proc proc rw 0 0\n"
        );
        assert_eq!(
            vfs.mounts().render_mountinfo("/"),
            "1 1 0:1 / / rw - ramfs ramfs rw\n2 1 0:2 / /dev rw - devtmpfs devtmpfs rw\n3 2 0:3 / \
             /dev/shm rw - tmpfs tmpfs rw\n4 1 0:4 / /proc rw - proc proc rw\n"
        );
    }

    #[test]
    fn a_chrooted_reader_sees_only_the_mounts_under_its_root() {
        let vfs = tree();
        assert_eq!(
            vfs.mounts().render_mounts("/dev"),
            "devtmpfs / devtmpfs rw 0 0\ntmpfs /shm tmpfs rw 0 0\n"
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
            .contains("tmpfs /a\\040b tmpfs rw 0 0\n"));
        let blockfs = crate::fs::blockfs::BlockFs::new(64, 16);
        assert_eq!(default_source(&blockfs, true), "/dev/root");
        assert_eq!(default_source(&blockfs, false), "blockfs");
        assert_eq!(default_source(&RamFs::new(), true), "ramfs");
    }
}

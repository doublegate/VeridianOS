//! ID-mapped mounts (Linux's `mnt_idmap`, `fs/mnt_idmapping.c`;
//! mount_setattr with MOUNT_ATTR_IDMAP; ADR 0027 decision 24).
//!
//! An idmapped mount shows its filesystem's owners through a user
//! namespace's ID maps: an owner stored as `k` reads as `map.down(k)`
//! (make_vfsuid), and an owner given through the mount is stored as
//! `map.up(id)` (from_vfsuid), EOVERFLOW when the map has no such ID. All
//! of the filesystems here keep owners in the initial user namespace, so
//! the filesystem's own mapping is the identity (initial_idmapping).
//!
//! The mount's root is an [`IdmappedNode`], a view of the real node, and
//! every node reached through it is one too: lookups, creations and the
//! like return views, so stat, the permission checks, exec's set-ID bits
//! and chown all see and give mapped owners without knowing about the
//! mount. A view is never stored in a filesystem: links, renames and
//! comparisons use the real node ([`super::base_node`]). This is the
//! translating view a file server can offer per mount handle once the
//! filesystems move out of the kernel (Track M).

use alloc::{string::String, sync::Arc, vec::Vec};

use super::{falloc, statfs, xattr, CacheStat, DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::{
    error::{FsError, KernelError},
    process::userns::{IdMap, UserNs},
};

/// An owner the mount's map has no ID for (INVALID_VFSUID): it matches
/// no caller and reads as the overflow ID.
pub const NO_ID: u32 = u32::MAX;

/// A mount's ID maps: copies of a user namespace's, taken when the mount
/// was made idmapped (alloc_mnt_idmap copies them), and that namespace.
pub struct MntIdmap {
    uid: IdMap,
    gid: IdMap,
    /// The user namespace the maps were taken from (statmount's maps are
    /// shown as it sees them).
    pub userns: Arc<UserNs>,
}

impl MntIdmap {
    /// The maps of `ns`.
    pub fn of(ns: &Arc<UserNs>) -> Arc<Self> {
        Arc::new(Self {
            uid: ns.uid_map(),
            gid: ns.gid_map(),
            userns: ns.clone(),
        })
    }

    /// How a stored owner reads through the mount (make_vfsuid).
    pub fn seen_uid(&self, kuid: u32) -> u32 {
        self.uid.down(kuid).unwrap_or(NO_ID)
    }

    /// How a stored group reads through the mount (make_vfsgid).
    pub fn seen_gid(&self, kgid: u32) -> u32 {
        self.gid.down(kgid).unwrap_or(NO_ID)
    }

    /// The owner stored for one given through the mount (from_vfsuid);
    /// None when the map has no such ID.
    pub fn stored_uid(&self, uid: u32) -> Option<u32> {
        self.uid.up(uid)
    }

    /// The group stored for one given through the mount.
    pub fn stored_gid(&self, gid: u32) -> Option<u32> {
        self.gid.up(gid)
    }

    /// The maps as statmount's MNT_UIDMAP and MNT_GIDMAP show them: one
    /// "inside outside count" line per extent.
    pub fn extents(&self, gid: bool) -> Vec<(u32, u32, u32)> {
        let map = if gid { &self.gid } else { &self.uid };
        map.extents()
            .iter()
            .map(|e| (e.first, e.lower, e.count))
            .collect()
    }
}

/// EOVERFLOW: an owner the mount cannot store (chown_ok, may_create's
/// fsuidgid_has_mapping).
fn overflow() -> KernelError {
    KernelError::FsError(FsError::Overflow)
}

/// The view of `node` through `map`.
pub fn view(node: Arc<dyn VfsNode>, map: &Arc<MntIdmap>) -> Arc<dyn VfsNode> {
    // A view of a view is the view of the real node.
    let inner = super::base_node(&node);
    Arc::new(IdmappedNode {
        inner,
        map: map.clone(),
    })
}

/// A node seen through an idmapped mount (module docs).
pub struct IdmappedNode {
    inner: Arc<dyn VfsNode>,
    map: Arc<MntIdmap>,
}

impl IdmappedNode {
    fn wrap(
        &self,
        r: Result<Arc<dyn VfsNode>, KernelError>,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        r.map(|n| view(n, &self.map))
    }

    fn stored(
        &self,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<(Option<u32>, Option<u32>), KernelError> {
        let uid = match uid {
            Some(u) => Some(self.map.stored_uid(u).ok_or_else(overflow)?),
            None => None,
        };
        let gid = match gid {
            Some(g) => Some(self.map.stored_gid(g).ok_or_else(overflow)?),
            None => None,
        };
        Ok((uid, gid))
    }

    /// A v3 `security.capability`'s root id as `to` maps it (rootid at
    /// byte 20); other values unchanged. EOVERFLOW when it does not map.
    fn map_caps(
        set: &mut xattr::XattrSet,
        to: impl Fn(u32) -> Option<u32>,
    ) -> Result<(), KernelError> {
        let Some(v) = set.get(xattr::XATTR_NAME_CAPS) else {
            return Ok(());
        };
        if v.len() != 24 || v[3] != 0x03 {
            return Ok(());
        }
        let root = u32::from_le_bytes([v[20], v[21], v[22], v[23]]);
        let mapped = to(root).ok_or_else(overflow)?;
        let mut new = Vec::from(v);
        new[20..24].copy_from_slice(&mapped.to_le_bytes());
        set.set(xattr::XATTR_NAME_CAPS, &new, 0)
    }
}

impl VfsNode for IdmappedNode {
    fn node_type(&self) -> NodeType {
        self.inner.node_type()
    }
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        self.inner.read(offset, buffer)
    }
    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        self.inner.write(offset, data)
    }
    fn read_as(
        &self,
        offset: usize,
        buffer: &mut [u8],
        opener: Option<&crate::process::creds::Caller>,
    ) -> Result<usize, KernelError> {
        self.inner.read_as(offset, buffer, opener)
    }
    fn write_as(
        &self,
        offset: usize,
        data: &[u8],
        opener: Option<&crate::process::creds::Caller>,
    ) -> Result<usize, KernelError> {
        self.inner.write_as(offset, data, opener)
    }
    fn metadata(&self) -> Result<Metadata, KernelError> {
        let mut m = self.inner.metadata()?;
        m.uid = self.map.seen_uid(m.uid);
        m.gid = self.map.seen_gid(m.gid);
        Ok(m)
    }
    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        self.inner.readdir()
    }
    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.wrap(self.inner.lookup(name))
    }
    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.wrap(self.inner.create(name, permissions))
    }
    fn mkdir(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.wrap(self.inner.mkdir(name, permissions))
    }
    fn unlink(&self, name: &str) -> Result<(), KernelError> {
        self.inner.unlink(name)
    }
    fn unlink_kind(&self, name: &str, want_dir: bool) -> Result<(), KernelError> {
        self.inner.unlink_kind(name, want_dir)
    }
    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        self.inner.truncate(size)
    }
    fn rename(
        &self,
        old_name: &str,
        new_parent: &Arc<dyn VfsNode>,
        new_name: &str,
    ) -> Result<(), KernelError> {
        self.inner
            .rename(old_name, &super::base_node(new_parent), new_name)
    }
    fn link(&self, name: &str, target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        self.inner.link(name, super::base_node(&target))
    }
    fn tmpfile(
        &self,
        permissions: Permissions,
        linkable: bool,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.wrap(self.inner.tmpfile(permissions, linkable))
    }
    fn tmpfile_linkable(&self) -> bool {
        self.inner.tmpfile_linkable()
    }
    fn file_identity(&self) -> Option<(u64, u64)> {
        self.inner.file_identity()
    }
    fn symlink(&self, name: &str, target: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.wrap(self.inner.symlink(name, target))
    }
    fn readlink(&self) -> Result<String, KernelError> {
        self.inner.readlink()
    }
    fn is_magic_link(&self) -> bool {
        self.inner.is_magic_link()
    }
    fn magic_target(&self) -> Option<Result<Arc<dyn VfsNode>, KernelError>> {
        self.inner.magic_target()
    }
    fn chmod(&self, permissions: Permissions) -> Result<(), KernelError> {
        self.inner.chmod(permissions)
    }
    fn chown(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), KernelError> {
        let (uid, gid) = self.stored(uid, gid)?;
        self.inner.chown(uid, gid)
    }
    fn check_owner(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), KernelError> {
        let (uid, gid) = self.stored(uid, gid)?;
        self.inner.check_owner(uid, gid)
    }
    fn cache_stat(&self, first: u64, last: u64) -> CacheStat {
        self.inner.cache_stat(first, last)
    }
    fn attr_flags(&self) -> Option<u32> {
        self.inner.attr_flags()
    }
    fn set_attr_flags(&self, flags: u32) -> Result<(), KernelError> {
        self.inner.set_attr_flags(flags)
    }
    fn set_times(&self, atime: Option<u64>, mtime: Option<u64>) -> Result<(), KernelError> {
        self.inner.set_times(atime, mtime)
    }
    fn poll_readiness(&self) -> u16 {
        self.inner.poll_readiness()
    }
    fn bytes_readable(&self) -> Option<usize> {
        self.inner.bytes_readable()
    }
    fn is_stream(&self) -> bool {
        self.inner.is_stream()
    }
    fn pipe_id(&self) -> Option<usize> {
        self.inner.pipe_id()
    }
    fn pipe_room(&self) -> Result<usize, KernelError> {
        self.inner.pipe_room()
    }
    fn pipe_splice(
        &self,
        max: usize,
        consume: bool,
        sink: &mut dyn FnMut(&[u8]) -> Result<usize, KernelError>,
    ) -> Result<usize, KernelError> {
        self.inner.pipe_splice(max, consume, sink)
    }
    fn seals(&self) -> Option<u32> {
        self.inner.seals()
    }
    fn add_seals(&self, seals: u32) -> Result<(), KernelError> {
        self.inner.add_seals(seals)
    }
    #[cfg(feature = "alloc")]
    fn page_cache(&self) -> Option<alloc::sync::Arc<crate::mm::page_cache::FileCache>> {
        self.inner.page_cache()
    }
    #[cfg(feature = "alloc")]
    fn share_pages(
        &self,
        first_page: usize,
        count: usize,
        writable: bool,
    ) -> Option<Result<Vec<crate::mm::FrameNumber>, KernelError>> {
        self.inner.share_pages(first_page, count, writable)
    }
    fn maps_own_pages(&self) -> bool {
        self.inner.maps_own_pages()
    }
    fn io_generation(&self) -> Option<u64> {
        self.inner.io_generation()
    }
    fn wakes_io_waiters(&self) -> bool {
        self.inner.wakes_io_waiters()
    }
    fn ready_at_ns(&self) -> Option<u64> {
        self.inner.ready_at_ns()
    }
    fn as_any(&self) -> Option<&dyn core::any::Any> {
        self.inner.as_any()
    }
    fn held_by_open(&self) -> Option<Arc<dyn core::any::Any + Send + Sync>> {
        self.inner.held_by_open()
    }
    fn device_id(&self) -> Option<(u32, u32)> {
        self.inner.device_id()
    }
    fn mknod(
        &self,
        name: &str,
        kind: NodeType,
        permissions: Permissions,
        rdev: u32,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.wrap(self.inner.mknod(name, kind, permissions, rdev))
    }
    fn is_special_inode(&self) -> bool {
        self.inner.is_special_inode()
    }
    fn xattrs(&self) -> Result<xattr::XattrSet, KernelError> {
        let mut set = self.inner.xattrs()?;
        // A file capability's root reads as the mount maps it; one it does
        // not map reads as no root at all (the capability is then not for
        // the reader's namespace).
        let map = self.map.clone();
        let _ = Self::map_caps(&mut set, |k| Some(map.seen_uid(k)));
        Ok(set)
    }
    fn update_xattrs(
        &self,
        f: &mut dyn FnMut(&mut xattr::XattrSet) -> Result<(), KernelError>,
    ) -> Result<(), KernelError> {
        let map = self.map.clone();
        self.inner.update_xattrs(&mut |set| {
            Self::map_caps(set, |k| Some(map.seen_uid(k)))?;
            f(set)?;
            Self::map_caps(set, |u| map.stored_uid(u))
        })
    }
    fn statfs(&self) -> Result<statfs::FsStats, KernelError> {
        self.inner.statfs()
    }
    fn fallocate(&self, request: falloc::Request) -> Result<(), KernelError> {
        self.inner.fallocate(request)
    }
    fn link_count(&self) -> u32 {
        self.inner.link_count()
    }
    fn view_of(&self) -> Option<Arc<dyn VfsNode>> {
        Some(self.inner.clone())
    }
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeSet, string::String};

    use super::*;

    /// The method names declared in the block that starts at `start` (up
    /// to its closing brace at column 0).
    fn methods(src: &str, start: &str) -> BTreeSet<String> {
        let at = src.find(start).expect("block");
        let body = &src[at..];
        let end = body.find("\n}\n").expect("end of block");
        body[..end]
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("fn "))
            .map(|l| String::from(&l[..l.find(['(', '<']).unwrap_or(l.len())]))
            .collect()
    }

    /// Every VfsNode method is passed on by the view: one added to the
    /// trait with a default would otherwise run that default on idmapped
    /// mounts only, silently (side review). Add the forwarding here when
    /// the trait grows.
    #[test]
    fn the_view_forwards_every_vfsnode_method() {
        let trait_methods = methods(include_str!("mod.rs"), "pub trait VfsNode");
        let view_methods = methods(include_str!("idmap.rs"), "impl VfsNode for IdmappedNode");
        let missing: Vec<_> = trait_methods.difference(&view_methods).collect();
        assert!(
            missing.is_empty(),
            "IdmappedNode does not forward {missing:?}"
        );
        assert!(
            trait_methods.len() > 40,
            "trait parse found {}",
            trait_methods.len()
        );
    }

    fn ns(map: &[u8]) -> Arc<UserNs> {
        crate::process::userns::test_child(map, map)
    }

    /// Stored owners read through the map; owners given are stored back;
    /// an ID the map lacks reads as none and cannot be stored.
    #[test]
    fn owners_map_both_ways() {
        let map = MntIdmap::of(&ns(b"0 100000 1000\n"));
        assert_eq!(map.seen_uid(0), 100000);
        assert_eq!(map.seen_uid(999), 100999);
        assert_eq!(map.seen_uid(5000), NO_ID);
        assert_eq!(map.stored_uid(100005), Some(5));
        assert_eq!(map.stored_uid(5), None);
        assert_eq!(map.extents(false), alloc::vec![(0, 100000, 1000)]);
    }

    /// A view shows mapped owners, chowns through the map (EOVERFLOW off
    /// it), hands out views from lookups and creations, and is the real
    /// node for identity.
    #[test]
    fn a_view_maps_owners_and_stays_one_file() {
        use crate::fs::{ramfs::RamFs, Filesystem};
        let fs = RamFs::new();
        let real = fs
            .root()
            .create("f", Permissions::from_mode(0o644))
            .unwrap();
        real.chown(Some(0), Some(0)).unwrap();
        let map = MntIdmap::of(&ns(b"0 100000 1000\n"));
        let root = view(fs.root(), &map);
        let f = root.lookup("f").unwrap();
        let m = f.metadata().unwrap();
        assert_eq!((m.uid, m.gid), (100000, 100000));
        f.chown(Some(100007), None).unwrap();
        assert_eq!(real.metadata().unwrap().uid, 7);
        assert_eq!(
            f.chown(Some(7), None),
            Err(KernelError::FsError(FsError::Overflow))
        );
        let made = root.create("g", Permissions::from_mode(0o600)).unwrap();
        assert!(made.view_of().is_some());
        assert_eq!(super::super::inode_key(&f), super::super::inode_key(&real));
        // A view of a view is one view of the real node.
        let twice = view(f.clone(), &map);
        assert!(twice.view_of().unwrap().view_of().is_none());
    }

    /// A mount over a namespace whose user and group maps differ: owners
    /// and groups read and are stored each through its own map, and one
    /// the map has no ID for is EOVERFLOW for chown and for the owner
    /// check (chown_ok), changing nothing.
    #[test]
    fn users_and_groups_map_apart() {
        use crate::fs::{ramfs::RamFs, Filesystem};
        let fs = RamFs::new();
        let real = fs
            .root()
            .create("f", Permissions::from_mode(0o644))
            .unwrap();
        real.chown(Some(5), Some(6)).unwrap();
        let map = MntIdmap::of(&crate::process::userns::test_child(
            b"0 100000 1000\n",
            b"0 200000 1000\n",
        ));
        assert_eq!(map.seen_gid(6), 200006);
        assert_eq!(map.stored_gid(200009), Some(9));
        assert_eq!(map.stored_gid(100009), None);
        assert_eq!(map.extents(true), alloc::vec![(0, 200000, 1000)]);
        let f = view(real.clone(), &map);
        let m = f.metadata().unwrap();
        assert_eq!((m.uid, m.gid), (100005, 200006));
        f.chown(None, Some(200042)).unwrap();
        assert_eq!(real.metadata().unwrap().gid, 42);
        let overflow = Err(KernelError::FsError(FsError::Overflow));
        assert_eq!(f.chown(Some(100001), Some(100001)), overflow);
        assert_eq!(f.check_owner(None, Some(7)), overflow);
        let m = real.metadata().unwrap();
        assert_eq!((m.uid, m.gid), (5, 42), "a refused chown changes nothing");
        // An owner the map does not reach reads as no ID at all.
        real.chown(Some(5000), None).unwrap();
        assert_eq!(f.metadata().unwrap().uid, NO_ID);
    }

    /// A v3 file capability's root id reads through the map and is stored
    /// back through it; a root the map does not reach cannot be written
    /// (EOVERFLOW); other values, a v2 capability among them, pass as they
    /// are.
    #[test]
    fn file_capability_roots_map() {
        use crate::fs::{ramfs::RamFs, Filesystem};
        let fs = RamFs::new();
        let real = fs
            .root()
            .create("f", Permissions::from_mode(0o755))
            .unwrap();
        let v3 = |root: u32| {
            let mut v = alloc::vec![0u8; 24];
            v[0..4].copy_from_slice(&0x0300_0000u32.to_le_bytes());
            v[20..24].copy_from_slice(&root.to_le_bytes());
            v
        };
        real.update_xattrs(&mut |set| set.set(xattr::XATTR_NAME_CAPS, &v3(3), 0))
            .unwrap();
        let map = MntIdmap::of(&ns(b"0 100000 1000\n"));
        let f = view(real.clone(), &map);
        let seen = f.xattrs().unwrap();
        assert_eq!(seen.get(xattr::XATTR_NAME_CAPS), Some(&v3(100003)[..]));
        // Written through the view: stored unmapped.
        f.update_xattrs(&mut |set| set.set(xattr::XATTR_NAME_CAPS, &v3(100009), 0))
            .unwrap();
        assert_eq!(
            real.xattrs().unwrap().get(xattr::XATTR_NAME_CAPS),
            Some(&v3(9)[..])
        );
        assert_eq!(
            f.update_xattrs(&mut |set| set.set(xattr::XATTR_NAME_CAPS, &v3(7), 0)),
            Err(KernelError::FsError(FsError::Overflow))
        );
        // A v2 capability (20 bytes, no root id) and other names pass.
        let v2 = [
            0x00, 0x00, 0x00, 0x02, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        f.update_xattrs(&mut |set| {
            set.set(xattr::XATTR_NAME_CAPS, &v2, 0)?;
            set.set("user.note", b"x", 0)
        })
        .unwrap();
        let stored = real.xattrs().unwrap();
        assert_eq!(stored.get(xattr::XATTR_NAME_CAPS), Some(&v2[..]));
        assert_eq!(stored.get("user.note"), Some(&b"x"[..]));
    }

    /// Everything reached or made through a view is a view (mapped owners)
    /// of the real node; rename and link through views act on the real
    /// nodes; data, modes, times and removals pass through.
    #[test]
    fn a_view_tree_is_the_real_tree() {
        use crate::fs::{ramfs::RamFs, Filesystem};
        let fs = RamFs::new();
        let map = MntIdmap::of(&ns(b"0 100000 1000\n"));
        let root = view(fs.root(), &map);
        let p = Permissions::from_mode(0o755);
        let d = root.mkdir("d", p).unwrap();
        let s = root.symlink("s", "d").unwrap();
        let n = root.mknod("n", NodeType::Pipe, p, 0).unwrap();
        let f = d.create("f", Permissions::from_mode(0o644)).unwrap();
        for node in [&d, &s, &n, &f] {
            assert!(node.view_of().is_some(), "a view");
            assert_eq!(node.metadata().unwrap().uid, map.seen_uid(0));
        }
        assert_eq!(s.readlink().unwrap(), "d");
        assert_eq!(root.lookup("d").unwrap().node_type(), NodeType::Directory);
        // Data written through the view is the real file's.
        assert_eq!(f.write(0, b"abc").unwrap(), 3);
        let real_f = fs.root().lookup("d").unwrap().lookup("f").unwrap();
        let mut buf = [0u8; 3];
        assert_eq!(real_f.read(0, &mut buf).unwrap(), 3);
        assert_eq!(&buf, b"abc");
        f.truncate(1).unwrap();
        assert_eq!(real_f.metadata().unwrap().size, 1);
        f.chmod(Permissions::from_mode(0o600)).unwrap();
        assert!(!real_f.metadata().unwrap().permissions.other_read);
        f.set_times(Some(11), Some(22)).unwrap();
        let m = real_f.metadata().unwrap();
        assert_eq!((m.accessed, m.modified), (11, 22));
        // Rename into a view directory, and a hard link of a view, land on
        // the real tree (no view is ever stored in a filesystem).
        root.mkdir("e", p).unwrap();
        let e = root.lookup("e").unwrap();
        d.rename("f", &e, "g").unwrap();
        let moved = fs.root().lookup("e").unwrap().lookup("g").unwrap();
        assert_eq!(
            super::super::inode_key(&moved),
            super::super::inode_key(&real_f)
        );
        let g = e.lookup("g").unwrap();
        root.link("h", g.clone()).unwrap();
        let linked = fs.root().lookup("h").unwrap();
        assert!(linked.view_of().is_none(), "the link names the real node");
        assert_eq!(linked.link_count(), 2);
        let names: Vec<String> = root
            .readdir()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(names.iter().any(|n| n == "h") && names.iter().any(|n| n == "e"));
        root.unlink("h").unwrap();
        assert!(fs.root().lookup("h").is_err());
        assert_eq!(g.link_count(), 1);
    }
}

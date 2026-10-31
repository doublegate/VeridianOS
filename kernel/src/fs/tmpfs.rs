//! tmpfs -- Memory-Backed Filesystem
//!
//! A temporary in-memory filesystem with configurable size limits, lost on
//! unmount. A regular file's bytes are pages it owns (`fs::shmem`), so a
//! MAP_SHARED mapping maps the file itself, as Linux's tmpfs; this is what
//! `/dev/shm` is. Supports all standard VFS operations including symlinks.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use spin::RwLock;

use super::{shmem::ShmemPages, DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// Global inode counter for tmpfs (separate from RamFS)
static TMPFS_NEXT_INODE: AtomicU64 = AtomicU64::new(1);

/// tmpfs node
struct TmpNode {
    node_type: NodeType,
    /// A regular file's contents.
    pages: RwLock<ShmemPages>,
    /// A symlink's target.
    data: RwLock<Vec<u8>>,
    children: RwLock<BTreeMap<String, Arc<TmpNode>>>,

    /// Extended attributes (`fs::xattr`), kept with the node.
    xattrs: RwLock<super::xattr::XattrSet>,
    metadata: RwLock<Metadata>,
    inode: u64,
    /// Device number of a device node (Linux's encoding, `fs::make_dev`);
    /// 0 for anything else.
    rdev: u32,
    /// Parent directory inode (".."); changes when a rename moves this
    /// directory.
    parent_inode: core::sync::atomic::AtomicU64,
    /// Shared reference to the filesystem's total bytes used counter
    bytes_used: Arc<AtomicUsize>,
    /// Shared reference to the filesystem's size limit
    size_limit: usize,
}

impl TmpNode {
    /// [`VfsNode::unlink`], and with `kind` [`VfsNode::unlink_kind`]: the
    /// entry's type is checked under the children lock the removal holds.
    fn unlink_checked(&self, name: &str, kind: Option<bool>) -> Result<(), KernelError> {
        let mut children = self.children.write();

        if let Some(node) = children.get(name) {
            if let Some(want_dir) = kind {
                super::check_unlink_kind(node.node_type, want_dir)?;
            }
            if node.node_type == NodeType::Directory {
                let dir_children = node.children.read();
                if !dir_children.is_empty() {
                    return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
                }
            }

            // The bytes go back to the size budget when the node itself is
            // dropped (see `Drop for TmpNode`), not here: another hard link
            // or an open file can still hold the data.
            children.remove(name);
            Ok(())
        } else {
            Err(KernelError::FsError(FsError::NotFound))
        }
    }

    fn new_file(
        inode: u64,
        parent_inode: u64,
        permissions: Permissions,
        bytes_used: Arc<AtomicUsize>,
        size_limit: usize,
    ) -> Self {
        Self {
            node_type: NodeType::File,
            pages: RwLock::new(ShmemPages::new()),
            data: RwLock::new(Vec::new()),
            children: RwLock::new(BTreeMap::new()),
            xattrs: RwLock::new(super::xattr::XattrSet::new()),
            metadata: RwLock::new(Metadata {
                node_type: NodeType::File,
                size: 0,
                permissions,
                uid: 0,
                gid: 0,
                created: crate::arch::timer::get_timestamp_secs(),
                modified: crate::arch::timer::get_timestamp_secs(),
                accessed: crate::arch::timer::get_timestamp_secs(),
                inode,
            }),
            inode,
            rdev: 0,
            parent_inode: core::sync::atomic::AtomicU64::new(parent_inode),
            bytes_used,
            size_limit,
        }
    }

    fn new_directory(
        inode: u64,
        parent_inode: u64,
        permissions: Permissions,
        bytes_used: Arc<AtomicUsize>,
        size_limit: usize,
    ) -> Self {
        Self {
            node_type: NodeType::Directory,
            pages: RwLock::new(ShmemPages::new()),
            data: RwLock::new(Vec::new()),
            children: RwLock::new(BTreeMap::new()),
            xattrs: RwLock::new(super::xattr::XattrSet::new()),
            metadata: RwLock::new(Metadata {
                node_type: NodeType::Directory,
                size: 0,
                permissions,
                uid: 0,
                gid: 0,
                created: crate::arch::timer::get_timestamp_secs(),
                modified: crate::arch::timer::get_timestamp_secs(),
                accessed: crate::arch::timer::get_timestamp_secs(),
                inode,
            }),
            inode,
            rdev: 0,
            parent_inode: core::sync::atomic::AtomicU64::new(parent_inode),
            bytes_used,
            size_limit,
        }
    }

    fn new_symlink(
        inode: u64,
        parent_inode: u64,
        target: &str,
        bytes_used: Arc<AtomicUsize>,
        size_limit: usize,
    ) -> Self {
        let target_bytes = Vec::from(target.as_bytes());
        let size = target_bytes.len();
        Self {
            node_type: NodeType::Symlink,
            pages: RwLock::new(ShmemPages::new()),
            data: RwLock::new(target_bytes),
            children: RwLock::new(BTreeMap::new()),
            xattrs: RwLock::new(super::xattr::XattrSet::new()),
            metadata: RwLock::new(Metadata {
                node_type: NodeType::Symlink,
                size,
                permissions: Permissions::from_mode(0o777),
                uid: 0,
                gid: 0,
                created: crate::arch::timer::get_timestamp_secs(),
                modified: crate::arch::timer::get_timestamp_secs(),
                accessed: crate::arch::timer::get_timestamp_secs(),
                inode,
            }),
            inode,
            rdev: 0,
            parent_inode: core::sync::atomic::AtomicU64::new(parent_inode),
            bytes_used,
            size_limit,
        }
    }

    /// Check if writing `additional` bytes would exceed the size limit.
    /// Returns Ok(()) if within limits, Err if exceeded.
    /// Charge `additional` bytes to the filesystem, atomically: a separate
    /// check and add let concurrent writers exceed the limit (N-122).
    fn reserve_space(&self, additional: usize) -> Result<(), KernelError> {
        if additional == 0 {
            return Ok(());
        }
        let limit = self.size_limit;
        self.bytes_used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                let new = cur.checked_add(additional)?;
                (limit == 0 || new <= limit).then_some(new)
            })
            .map(|_| ())
            .map_err(|_| KernelError::FsError(FsError::NoSpace))
    }

    /// What a node's extended attributes are charged: their encoded size,
    /// nothing for none (an empty set's size is only its header).
    fn xattr_charge(set: &super::xattr::XattrSet) -> usize {
        if set.is_empty() {
            0
        } else {
            set.encoded_size()
        }
    }

    fn release_space(&self, bytes: usize) {
        let _ = self
            .bytes_used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                Some(cur.saturating_sub(bytes))
            });
    }
}

impl Drop for TmpNode {
    /// A file's bytes are charged to its tmpfs while the node exists, so
    /// they are released when the last name and the last open file are
    /// gone. Releasing at unlink let a hard link keep data alive uncounted.
    fn drop(&mut self) {
        let freed = self.data.get_mut().len()
            + self.pages.get_mut().allocated_bytes()
            + Self::xattr_charge(self.xattrs.get_mut());
        if freed > 0 {
            let _ = self
                .bytes_used
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    Some(used.saturating_sub(freed))
                });
        }
    }
}

/// Recover the concrete `Arc<TmpNode>` behind a VFS node, if it is one.
fn as_tmp_arc(node: Arc<dyn VfsNode>) -> Option<Arc<TmpNode>> {
    node.as_any()?.downcast_ref::<TmpNode>()?;
    // SAFETY: the downcast above proved the value behind this `Arc<dyn
    // VfsNode>` is a `TmpNode`, so the allocation is an `ArcInner<TmpNode>` and the
    // data pointer, stripped of its vtable, is the pointer `Arc<TmpNode>` would
    // hold. This is the reverse of the unsizing coercion that made the
    // trait object, and is how `Arc::downcast` is implemented. The strong
    // count moves from `node` to the result unchanged.
    Some(unsafe { Arc::from_raw(Arc::into_raw(node) as *const TmpNode) })
}

impl VfsNode for TmpNode {
    /// Linux's `shmem_statfs`: page-sized blocks, the size limit as the
    /// block count and what is not charged as free; no counts without a
    /// limit. Inodes are not limited, so their counts are 0.
    fn statfs(&self) -> Result<super::statfs::FsStats, KernelError> {
        let mut st = super::statfs::FsStats::simple(super::statfs::TMPFS_MAGIC);
        if self.size_limit != 0 {
            let page = st.block_size as usize;
            st.blocks = (self.size_limit / page) as u64;
            let used = self.bytes_used.load(Ordering::Acquire).div_ceil(page) as u64;
            st.blocks_free = st.blocks.saturating_sub(used);
            st.blocks_avail = st.blocks_free;
        }
        Ok(st)
    }

    fn xattrs(&self) -> Result<super::xattr::XattrSet, KernelError> {
        Ok(self.xattrs.read().clone())
    }

    fn update_xattrs(
        &self,
        f: &mut dyn FnMut(&mut super::xattr::XattrSet) -> Result<(), KernelError>,
    ) -> Result<(), KernelError> {
        // Charged to the filesystem like file data (PR #33 review): user.*
        // attributes are writable by the owner of any file, so without it
        // they would let a user fill memory past the size limit.
        let mut set = self.xattrs.write();
        let mut new = set.clone();
        super::xattr::update_in_memory(&mut new, f)?;
        let (old_charge, new_charge) = (Self::xattr_charge(&set), Self::xattr_charge(&new));
        if new_charge > old_charge {
            self.reserve_space(new_charge - old_charge)?;
        } else {
            self.release_space(old_charge - new_charge);
        }
        *set = new;
        // The status-change time, which stat reports from `created`.
        self.metadata.write().created = crate::arch::timer::get_timestamp_secs();
        Ok(())
    }

    fn node_type(&self) -> NodeType {
        self.node_type
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let bytes_to_read = self.pages.read().read(offset, buffer);

        self.metadata.write().accessed = crate::arch::timer::get_timestamp_secs();

        Ok(bytes_to_read)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let end = super::ram_write_end(offset, data.len())?;
        let mut pages = self.pages.write();
        // Charged in whole pages, the memory a file holds (PR #28 review).
        let growth =
            ShmemPages::bytes_for(end.max(pages.size())).saturating_sub(pages.allocated_bytes());

        self.reserve_space(growth)?;
        if let Err(e) = pages.write(offset, data) {
            self.release_space(growth);
            return Err(e);
        }

        let mut metadata = self.metadata.write();
        metadata.size = pages.size();
        metadata.modified = crate::arch::timer::get_timestamp_secs();

        Ok(data.len())
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(self.metadata.read().clone())
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let children = self.children.read();
        let mut entries = Vec::new();

        entries.push(DirEntry {
            name: String::from("."),
            node_type: NodeType::Directory,
            inode: self.inode,
        });
        entries.push(DirEntry {
            name: String::from(".."),
            node_type: NodeType::Directory,
            inode: self
                .parent_inode
                .load(core::sync::atomic::Ordering::Relaxed),
        });

        for (name, child) in children.iter() {
            entries.push(DirEntry {
                name: name.clone(),
                node_type: child.node_type,
                inode: child.inode,
            });
        }

        Ok(entries)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let children = self.children.read();
        children
            .get(name)
            .map(|node| node.clone() as Arc<dyn VfsNode>)
            .ok_or(KernelError::FsError(FsError::NotFound))
    }

    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode = TMPFS_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let new_file = Arc::new(TmpNode::new_file(
            inode,
            self.inode,
            permissions,
            self.bytes_used.clone(),
            self.size_limit,
        ));
        children.insert(String::from(name), new_file.clone());

        Ok(new_file as Arc<dyn VfsNode>)
    }

    fn mkdir(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode = TMPFS_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let new_dir = Arc::new(TmpNode::new_directory(
            inode,
            self.inode,
            permissions,
            self.bytes_used.clone(),
            self.size_limit,
        ));
        children.insert(String::from(name), new_dir.clone());

        Ok(new_dir as Arc<dyn VfsNode>)
    }

    fn unlink(&self, name: &str) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        self.unlink_checked(name, None)
    }

    fn unlink_kind(&self, name: &str, want_dir: bool) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }
        self.unlink_checked(name, Some(want_dir))
    }

    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let mut pages = self.pages.write();
        // Charged in whole pages, as in `write`.
        let held = pages.allocated_bytes();
        let need = ShmemPages::bytes_for(size);

        if need > held {
            // Growing: check space
            self.reserve_space(need - held)?;
            if let Err(e) = pages.resize(size) {
                self.release_space(need - held);
                return Err(e);
            }
        } else {
            // Same pages or fewer: reclaim what goes (EBUSY while a
            // dropped page is still mapped, `ShmemPages::resize`)
            pages.resize(size)?;
            self.release_space(held - need);
        }

        let mut metadata = self.metadata.write();
        metadata.size = size;
        metadata.modified = crate::arch::timer::get_timestamp_secs();

        Ok(())
    }

    fn rename(
        &self,
        old_name: &str,
        new_parent: &Arc<dyn VfsNode>,
        new_name: &str,
    ) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }
        if super::is_special_name(old_name) || super::is_special_name(new_name) {
            return Err(KernelError::FsError(FsError::InvalidPath));
        }
        let np = new_parent
            .as_any()
            .and_then(|a| a.downcast_ref::<TmpNode>())
            .ok_or(KernelError::FsError(FsError::CrossDevice))?;
        if np.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }
        // Same tmpfs instance only: moving a file between two tmpfs mounts
        // would carry its bytes past the destination's size limit.
        if !Arc::ptr_eq(&self.bytes_used, &np.bytes_used) {
            return Err(KernelError::FsError(FsError::CrossDevice));
        }

        // Both parent maps stay locked for the whole operation (one lock if
        // they are the same directory), so the entry checked is the entry
        // moved. Lock order: with renames serialized by RENAME_LOCK, every
        // other path locks parent before child, so if one directory is the
        // other's parent it is locked first; otherwise by address.
        let same_dir = core::ptr::eq(self, np);
        let self_first =
            if np.parent_inode.load(core::sync::atomic::Ordering::Relaxed) == self.inode {
                true
            } else if self
                .parent_inode
                .load(core::sync::atomic::Ordering::Relaxed)
                == np.inode
            {
                false
            } else {
                (self as *const TmpNode) < (np as *const TmpNode)
            };
        let (mut src, mut dst) = if same_dir {
            (self.children.write(), None)
        } else if self_first {
            let a = self.children.write();
            (a, Some(np.children.write()))
        } else {
            let b = np.children.write();
            (self.children.write(), Some(b))
        };

        let node = src
            .get(old_name)
            .cloned()
            .ok_or(KernelError::FsError(FsError::NotFound))?;
        if core::ptr::eq(&*node, np) {
            // A directory cannot become its own parent.
            return Err(KernelError::FsError(FsError::InvalidPath));
        }
        let target = match dst.as_ref() {
            Some(d) => d.get(new_name).cloned(),
            None => src.get(new_name).cloned(),
        };
        if let Some(existing) = target {
            if Arc::ptr_eq(&existing, &node) {
                return Ok(()); // same node: POSIX says do nothing
            }
            let moving_dir = node.node_type == NodeType::Directory;
            let existing_dir = existing.node_type == NodeType::Directory;
            match (moving_dir, existing_dir) {
                (true, false) => return Err(KernelError::FsError(FsError::NotADirectory)),
                (false, true) => return Err(KernelError::FsError(FsError::IsADirectory)),
                (true, true) => {
                    // The source directory contains `old_name`, so it is not
                    // empty (and is locked here, so it must not be re-read).
                    if core::ptr::eq(&*existing, self) || !existing.children.read().is_empty() {
                        return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
                    }
                }
                (false, false) => {}
            }
            // The replaced file's bytes go back when its node is dropped
            // (`Drop for TmpNode`), as for unlink: refunded here as well,
            // they were counted back twice, or while another link or an
            // open file still held them (PR #24 review).
        }

        src.remove(old_name);
        match dst.as_mut() {
            Some(d) => d.insert(String::from(new_name), node.clone()),
            None => src.insert(String::from(new_name), node.clone()),
        };
        if node.node_type == NodeType::Directory {
            node.parent_inode
                .store(np.inode, core::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn mknod(
        &self,
        name: &str,
        kind: NodeType,
        permissions: Permissions,
        rdev: u32,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }
        if !matches!(
            kind,
            NodeType::Pipe | NodeType::CharDevice | NodeType::BlockDevice | NodeType::Socket
        ) {
            return Err(KernelError::FsError(FsError::NotSupported));
        }
        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }
        let inode = TMPFS_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let mut node = TmpNode::new_file(
            inode,
            self.inode,
            permissions,
            self.bytes_used.clone(),
            self.size_limit,
        );
        node.node_type = kind;
        node.metadata.get_mut().node_type = kind;
        if matches!(kind, NodeType::CharDevice | NodeType::BlockDevice) {
            node.rdev = rdev;
        }
        let node = Arc::new(node);
        children.insert(String::from(name), node.clone());
        Ok(node as Arc<dyn VfsNode>)
    }

    fn is_special_inode(&self) -> bool {
        matches!(
            self.node_type,
            NodeType::Pipe | NodeType::CharDevice | NodeType::BlockDevice | NodeType::Socket
        )
    }

    fn device_id(&self) -> Option<(u32, u32)> {
        matches!(self.node_type, NodeType::CharDevice | NodeType::BlockDevice)
            .then(|| (super::dev_major(self.rdev), super::dev_minor(self.rdev)))
    }

    #[cfg(feature = "alloc")]
    fn share_pages(
        &self,
        first_page: usize,
        count: usize,
        _writable: bool,
    ) -> Option<Result<Vec<crate::mm::FrameNumber>, KernelError>> {
        (self.node_type == NodeType::File).then(|| self.pages.read().share(first_page, count))
    }

    fn link(&self, name: &str, target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        if target.node_type() == NodeType::Directory {
            return Err(KernelError::FsError(FsError::IsADirectory));
        }

        // Same node, same tmpfs instance only (N-45): a copy drifted apart
        // from the original and was charged twice against the size limit.
        let target = as_tmp_arc(target)
            .filter(|t| Arc::ptr_eq(&t.bytes_used, &self.bytes_used))
            .ok_or(KernelError::FsError(FsError::CrossDevice))?;

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }
        children.insert(String::from(name), target);
        Ok(())
    }

    fn symlink(&self, name: &str, target: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        // The target's bytes are charged, as a file's are: Drop gives every
        // node's data back, so an uncharged symlink, once removed, lowered
        // the count below what live files use and the limit could be
        // exceeded (PR #24 review; Linux's shmem charges long symlinks).
        self.reserve_space(target.len())?;
        let inode = TMPFS_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let new_symlink = Arc::new(TmpNode::new_symlink(
            inode,
            self.inode,
            target,
            self.bytes_used.clone(),
            self.size_limit,
        ));
        children.insert(String::from(name), new_symlink.clone());

        Ok(new_symlink as Arc<dyn VfsNode>)
    }

    fn readlink(&self) -> Result<String, KernelError> {
        if self.node_type != NodeType::Symlink {
            return Err(KernelError::FsError(FsError::NotASymlink));
        }

        let data = self.data.read();
        let s =
            core::str::from_utf8(&data).map_err(|_| KernelError::FsError(FsError::InvalidPath))?;
        Ok(String::from(s))
    }

    fn chmod(&self, permissions: Permissions) -> Result<(), KernelError> {
        let mut metadata = self.metadata.write();
        metadata.permissions = permissions;
        metadata.modified = crate::arch::timer::get_timestamp_secs();
        Ok(())
    }

    fn set_times(&self, atime: Option<u64>, mtime: Option<u64>) -> Result<(), KernelError> {
        let mut metadata = self.metadata.write();
        if let Some(atime) = atime {
            metadata.accessed = atime;
        }
        if let Some(mtime) = mtime {
            metadata.modified = mtime;
        }
        Ok(())
    }

    fn chown(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), KernelError> {
        let mut metadata = self.metadata.write();
        if let Some(uid) = uid {
            metadata.uid = uid;
        }
        if let Some(gid) = gid {
            metadata.gid = gid;
        }
        metadata.modified = crate::arch::timer::get_timestamp_secs();
        Ok(())
    }
}

/// tmpfs filesystem instance
pub struct TmpFs {
    root: Arc<TmpNode>,
    /// Total bytes used across all files
    bytes_used: Arc<AtomicUsize>,
    /// Maximum bytes allowed (0 = unlimited)
    size_limit: usize,
}

impl TmpFs {
    /// Create a new tmpfs with the given size limit in bytes, rounded up to
    /// whole pages as Linux's `size=` is (files are charged in pages).
    /// Pass 0 for unlimited. The root is mode 1777, Linux's tmpfs default.
    pub fn new(size_limit: usize) -> Self {
        let size_limit = ShmemPages::bytes_for(size_limit);
        let bytes_used = Arc::new(AtomicUsize::new(0));
        let root_inode = TMPFS_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let root = Arc::new(TmpNode::new_directory(
            root_inode,
            root_inode,
            Permissions::from_mode(0o1777),
            bytes_used.clone(),
            size_limit,
        ));

        Self {
            root,
            bytes_used,
            size_limit,
        }
    }

    /// A tmpfs limited to half of RAM, Linux's default size. At least one
    /// page: a limit of 0 means unlimited, which an unknown RAM size must
    /// not become (PR #28 review).
    pub fn half_of_ram() -> Self {
        Self::new((crate::mm::get_memory_stats().total_frames * 4096 / 2).max(4096))
    }

    /// Get current bytes used
    pub fn bytes_used(&self) -> usize {
        self.bytes_used.load(Ordering::Relaxed)
    }

    /// Get size limit (0 = unlimited)
    pub fn size_limit(&self) -> usize {
        self.size_limit
    }
}

impl Default for TmpFs {
    fn default() -> Self {
        // Default: 128MB limit
        Self::new(128 * 1024 * 1024)
    }
}

impl Filesystem for TmpFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }

    fn name(&self) -> &str {
        "tmpfs"
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<(), KernelError> {
        // tmpfs is memory-backed, nothing to sync
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    /// tmpfs charges whole pages, as Linux's does.
    const P: usize = 4096;

    #[test]
    fn statfs_counts_pages_against_the_size_limit() {
        let fs = TmpFs::new(4 * P);
        let root = fs.root();
        let a = root.create("a", Permissions::default()).unwrap();
        a.write(0, b"x").unwrap();
        let st = a.statfs().unwrap();
        assert_eq!(st.fs_type, super::super::statfs::TMPFS_MAGIC);
        assert_eq!((st.block_size, st.blocks), (P as u64, 4));
        // One page charged; every node of the filesystem says the same.
        assert_eq!((st.blocks_free, st.blocks_avail), (3, 3));
        assert_eq!(root.statfs().unwrap(), st);
        // No limit: no counts, as Linux's tmpfs mounted with size=0.
        let st = TmpFs::new(0).root().statfs().unwrap();
        assert_eq!((st.blocks, st.blocks_free), (0, 0));
    }

    #[test]
    fn hard_link_shares_the_file_and_its_bytes() {
        let fs = TmpFs::new(1024 * 1024);
        let root = fs.root();
        let a = root.create("a", Permissions::default()).unwrap();
        a.write(0, b"one").unwrap();
        root.link("b", a.clone()).unwrap();
        // One file, counted once against the size limit (N-45).
        assert_eq!(fs.bytes_used(), P);
        root.lookup("b").unwrap().write(0, b"two").unwrap();
        let mut buf = [0u8; 3];
        a.read(0, &mut buf).unwrap();
        assert_eq!(&buf, b"two");
    }

    #[test]
    fn bytes_are_released_when_the_last_link_goes() {
        let fs = TmpFs::new(P);
        let root = fs.root();
        let a = root.create("a", Permissions::default()).unwrap();
        a.write(0, b"12345678").unwrap();
        root.link("b", a.clone()).unwrap();
        drop(a);
        // Unlinking one name must not return bytes still reachable through
        // the other, or link + unlink would grow tmpfs past its limit.
        root.unlink("a").unwrap();
        assert_eq!(fs.bytes_used(), P);
        let c = root.create("c", Permissions::default()).unwrap();
        assert!(c.write(0, b"x").is_err());
        drop(c);
        root.unlink("b").unwrap();
        assert_eq!(fs.bytes_used(), 0);
    }

    #[test]
    fn hard_link_across_tmpfs_instances_is_exdev() {
        let fs1 = TmpFs::new(1024 * 1024);
        let fs2 = TmpFs::new(1024 * 1024);
        let a = fs1.root().create("a", Permissions::default()).unwrap();
        assert_eq!(
            fs2.root().link("b", a).err(),
            Some(KernelError::FsError(FsError::CrossDevice))
        );
        let ram = super::super::ramfs::RamFs::new();
        let r = ram.root().create("r", Permissions::default()).unwrap();
        assert_eq!(
            fs1.root().link("r", r).err(),
            Some(KernelError::FsError(FsError::CrossDevice))
        );
    }

    #[test]
    fn test_tmpfs_new() {
        let fs = TmpFs::new(1024 * 1024);
        assert_eq!(fs.name(), "tmpfs");
        assert!(!fs.is_readonly());
        assert_eq!(fs.size_limit(), 1024 * 1024);
        assert_eq!(fs.bytes_used(), 0);
    }

    #[test]
    fn test_tmpfs_default() {
        let fs = TmpFs::default();
        assert_eq!(fs.size_limit(), 128 * 1024 * 1024);
    }

    #[test]
    fn test_tmpfs_root_is_directory() {
        let fs = TmpFs::new(0);
        let root = fs.root();
        assert_eq!(root.node_type(), NodeType::Directory);
    }

    #[test]
    fn test_tmpfs_sync() {
        let fs = TmpFs::new(0);
        assert!(fs.sync().is_ok());
    }

    #[test]
    fn test_create_and_read_file() {
        let fs = TmpFs::new(0);
        let root = fs.root();

        let file = root.create("test.txt", Permissions::default()).unwrap();
        assert_eq!(file.node_type(), NodeType::File);

        file.write(0, b"Hello, tmpfs!").unwrap();
        assert_eq!(fs.bytes_used(), P);

        let mut buf = vec![0u8; 20];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 13);
        assert_eq!(&buf[..13], b"Hello, tmpfs!");
    }

    /// The size budget counts what is stored once: a symlink's target is
    /// charged (it was refunded on removal without ever being charged), and
    /// a file replaced by rename is refunded once, when its node goes (it
    /// was refunded at the rename too) (PR #24 review).
    #[test]
    fn the_size_budget_counts_symlinks_and_renames_once() {
        let fs = TmpFs::new(4 * P);
        let root = fs.root();
        let a = root.create("a", Permissions::default()).unwrap();
        a.write(0, &[1u8; 30]).unwrap();
        root.symlink("l", "12345678").unwrap();
        assert_eq!(fs.bytes_used(), P + 8);
        root.unlink("l").unwrap();
        assert_eq!(fs.bytes_used(), P);

        let b = root.create("b", Permissions::default()).unwrap();
        b.write(0, &[2u8; 20]).unwrap();
        drop((a, b));
        root.rename("b", &root, "a").unwrap();
        assert_eq!(fs.bytes_used(), P, "the replaced file refunded once");
    }

    #[test]
    fn test_size_limit_enforcement() {
        let fs = TmpFs::new(10); // rounded up to one page, as on Linux
        assert_eq!(fs.size_limit(), P);
        let root = fs.root();

        let file = root.create("big.txt", Permissions::default()).unwrap();

        // Filling the one page succeeds
        assert!(file.write(0, &[1u8; P - 2]).is_ok());
        assert_eq!(fs.bytes_used(), P);

        // Running into a second page fails
        let result = file.write(P - 2, b"abcde");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NoSpace));

        // Still one page used
        assert_eq!(fs.bytes_used(), P);
    }

    /// A committed attribute change moves the status-change time (stat's
    /// st_ctime, the node's `created`), as Linux's shmem_xattr_handler_set
    /// does; a refused one leaves it (PR #33 review).
    #[test]
    fn xattr_changes_move_the_status_change_time() {
        let node = TmpNode::new_file(
            2,
            1,
            Permissions::default(),
            Arc::new(AtomicUsize::new(0)),
            P,
        );
        node.metadata.write().created = u64::MAX;
        assert!(node
            .update_xattrs(&mut |_| Err(KernelError::FsError(FsError::NoSpace)))
            .is_err());
        assert_eq!(node.metadata.read().created, u64::MAX);
        node.update_xattrs(&mut |a| a.set("user.a", b"v", 0))
            .unwrap();
        assert_ne!(node.metadata.read().created, u64::MAX);
    }

    /// Extended attributes count against the size limit, and are refunded
    /// when removed and when the node goes.
    #[test]
    fn xattrs_are_charged_to_the_size_limit() {
        let fs = TmpFs::new(P);
        let root = fs.root();
        let file = root.create("f", Permissions::default()).unwrap();
        file.update_xattrs(&mut |a| a.set("user.a", &[7; 1000], 0))
            .unwrap();
        let one = fs.bytes_used();
        assert!(one >= 1000);
        // Five more would pass the one-page limit: the set is unchanged.
        let mut grow = |a: &mut crate::fs::xattr::XattrSet| {
            for name in ["user.b", "user.c", "user.d", "user.e"] {
                a.set(name, &[7; 1000], 0)?;
            }
            Ok(())
        };
        assert_eq!(
            file.update_xattrs(&mut grow),
            Err(KernelError::FsError(FsError::NoSpace))
        );
        assert_eq!(fs.bytes_used(), one);
        let kept = file.xattrs().unwrap();
        assert!(kept.get("user.a").is_some() && kept.get("user.b").is_none());
        file.update_xattrs(&mut |a| a.remove("user.a")).unwrap();
        assert_eq!(fs.bytes_used(), 0, "an empty set costs nothing");
        file.update_xattrs(&mut |a| a.set("user.a", &[7; 1000], 0))
            .unwrap();
        drop(file);
        root.unlink("f").unwrap();
        assert_eq!(fs.bytes_used(), 0, "refunded with the node");
    }

    #[test]
    fn test_truncate_reclaims_space() {
        let fs = TmpFs::new(4 * P);
        let root = fs.root();

        let file = root.create("shrink.txt", Permissions::default()).unwrap();
        file.write(0, &[7u8; 2 * P + 10]).unwrap();
        assert_eq!(fs.bytes_used(), 3 * P);

        file.truncate(3).unwrap();
        assert_eq!(fs.bytes_used(), P);

        let meta = file.metadata().unwrap();
        assert_eq!(meta.size, 3);
    }

    #[test]
    fn test_truncate_grow_checks_space() {
        let fs = TmpFs::new(2 * P);
        let root = fs.root();

        let file = root.create("grow.txt", Permissions::default()).unwrap();
        file.write(0, b"12345").unwrap();
        assert_eq!(fs.bytes_used(), P);

        // Growing to two pages should work
        assert!(file.truncate(2 * P).is_ok());
        assert_eq!(fs.bytes_used(), 2 * P);

        // A third page should fail
        let result = file.truncate(2 * P + 1);
        assert!(result.is_err());
        assert_eq!(fs.bytes_used(), 2 * P);
    }

    #[test]
    fn test_unlink_reclaims_space() {
        let fs = TmpFs::new(100);
        let root = fs.root();

        let file = root.create("temp.txt", Permissions::default()).unwrap();
        file.write(0, b"some data").unwrap();
        assert_eq!(fs.bytes_used(), P);

        root.unlink("temp.txt").unwrap();
        // Still open: the data exists, so it stays charged (as on Linux,
        // where an unlinked open file keeps its blocks until the last close).
        assert_eq!(fs.bytes_used(), P);
        drop(file);
        assert_eq!(fs.bytes_used(), 0);
    }

    #[test]
    fn test_mkdir_and_readdir() {
        let fs = TmpFs::new(0);
        let root = fs.root();

        root.mkdir("subdir", Permissions::default()).unwrap();
        root.create("file.txt", Permissions::default()).unwrap();

        let entries = root.readdir().unwrap();
        assert_eq!(entries.len(), 4); // . .. subdir file.txt

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"."));
        assert!(names.contains(&".."));
        assert!(names.contains(&"subdir"));
        assert!(names.contains(&"file.txt"));
    }

    #[test]
    fn test_nested_directories() {
        let fs = TmpFs::new(0);
        let root = fs.root();

        let sub = root.mkdir("a", Permissions::default()).unwrap();
        let subsub = sub.mkdir("b", Permissions::default()).unwrap();
        let file = subsub.create("c.txt", Permissions::default()).unwrap();
        file.write(0, b"nested").unwrap();

        // Lookup chain
        let a = root.lookup("a").unwrap();
        let b = a.lookup("b").unwrap();
        let c = b.lookup("c.txt").unwrap();

        let mut buf = vec![0u8; 10];
        let n = c.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"nested");
    }

    #[test]
    fn test_symlink() {
        let fs = TmpFs::new(0);
        let root = fs.root();

        let link = root.symlink("mylink", "/tmp/target").unwrap();
        assert_eq!(link.node_type(), NodeType::Symlink);

        let target = link.readlink().unwrap();
        assert_eq!(target, "/tmp/target");
    }

    #[test]
    fn test_unlink_nonempty_dir_fails() {
        let fs = TmpFs::new(0);
        let root = fs.root();

        let dir = root.mkdir("occupied", Permissions::default()).unwrap();
        dir.create("child", Permissions::default()).unwrap();

        let result = root.unlink("occupied");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(FsError::DirectoryNotEmpty)
        );
    }

    #[test]
    fn test_chmod() {
        let fs = TmpFs::new(0);
        let root = fs.root();

        let file = root.create("perm.txt", Permissions::default()).unwrap();
        let ro = Permissions::read_only();
        assert!(file.chmod(ro).is_ok());

        let meta = file.metadata().unwrap();
        assert!(meta.permissions.owner_read);
        assert!(!meta.permissions.owner_write);
    }

    #[test]
    fn test_multiple_files_share_limit() {
        let fs = TmpFs::new(2 * P);
        let root = fs.root();

        let f1 = root.create("a.txt", Permissions::default()).unwrap();
        let f2 = root.create("b.txt", Permissions::default()).unwrap();

        // Ten bytes each, but each file holds a whole page: the limit is
        // reached (PR #28 review: one-byte files must not exhaust RAM
        // under a nearly empty budget).
        f1.write(0, b"1234567890").unwrap();
        f2.write(0, b"1234567890").unwrap();
        assert_eq!(fs.bytes_used(), 2 * P);

        let f3 = root.create("c.txt", Permissions::default()).unwrap();
        assert!(f3.write(0, b"x").is_err());
        // Growing within a file's last page needs no new page.
        assert!(f1.write(10, b"x").is_ok());
        assert!(f1.write(P, b"x").is_err());
    }

    /// A file's pages are the file (N-230 for tmpfs, so /dev/shm works as
    /// on Linux): a MAP_SHARED mapping gets the frames themselves, and
    /// what is stored through one is what read() returns.
    #[test]
    fn a_shared_mapping_maps_the_file_s_own_pages() {
        let fs = TmpFs::new(1 << 20);
        let f = fs.root().create("ltp_ipc", Permissions::default()).unwrap();
        f.truncate(4096).unwrap();
        let frames = f.share_pages(0, 1, true).expect("tmpfs pages").unwrap();
        assert_eq!(frames.len(), 1);
        // SAFETY: on the host a shmem frame number is its heap page's
        // address >> 12 (fs::shmem), and the file keeps the page alive.
        unsafe { ((frames[0].as_u64() << 12) as *mut u8).add(5).write(42) };
        let mut b = [0u8; 1];
        f.read(5, &mut b).unwrap();
        assert_eq!(b, [42]);
        // A mapped page cannot be truncated away (EBUSY, as for a memfd).
        assert!(f.truncate(0).is_err());
        assert!(!crate::mm::frame_refs::release(frames[0]));
        f.truncate(0).unwrap();
        assert_eq!(fs.bytes_used(), 0);
    }
}

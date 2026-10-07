//! RAM Filesystem Implementation
//!
//! A simple in-memory filesystem for testing and temporary storage.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// RAM filesystem node
struct RamNode {
    /// Node type
    node_type: NodeType,

    /// File data (for files)
    data: RwLock<Vec<u8>>,

    /// Children (for directories)
    children: RwLock<BTreeMap<String, Arc<RamNode>>>,

    /// Metadata
    metadata: RwLock<Metadata>,

    /// Inode number
    inode: u64,

    /// Which `RamFs` instance this node belongs to. Every instance's nodes
    /// downcast to `RamNode`, so this is what tells two mounts apart.
    fs_id: u64,

    /// Parent directory inode (for ".." entries)
    /// Parent directory inode (".."); changes when a rename moves this
    /// directory.
    parent_inode: core::sync::atomic::AtomicU64,
}

impl RamNode {
    /// Create a new file node
    fn new_file(inode: u64, parent_inode: u64, fs_id: u64, permissions: Permissions) -> Self {
        Self {
            node_type: NodeType::File,
            data: RwLock::new(Vec::new()),
            children: RwLock::new(BTreeMap::new()),
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
            fs_id,
            parent_inode: core::sync::atomic::AtomicU64::new(parent_inode),
        }
    }

    /// Create a new directory node
    fn new_directory(inode: u64, parent_inode: u64, fs_id: u64, permissions: Permissions) -> Self {
        Self {
            node_type: NodeType::Directory,
            data: RwLock::new(Vec::new()),
            children: RwLock::new(BTreeMap::new()),
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
            fs_id,
            parent_inode: core::sync::atomic::AtomicU64::new(parent_inode),
        }
    }

    /// Create a new symbolic link node.
    ///
    /// The target path is stored in the node's `data` field.
    fn new_symlink(inode: u64, parent_inode: u64, fs_id: u64, target: &str) -> Self {
        let target_bytes = Vec::from(target.as_bytes());
        let size = target_bytes.len();
        Self {
            node_type: NodeType::Symlink,
            data: RwLock::new(target_bytes),
            children: RwLock::new(BTreeMap::new()),
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
            fs_id,
            parent_inode: core::sync::atomic::AtomicU64::new(parent_inode),
        }
    }
}

/// Recover the concrete `Arc<RamNode>` behind a VFS node, if it is one.
fn as_ram_arc(node: Arc<dyn VfsNode>) -> Option<Arc<RamNode>> {
    node.as_any()?.downcast_ref::<RamNode>()?;
    // SAFETY: the downcast above proved the value behind this `Arc<dyn
    // VfsNode>` is a `RamNode`, so the allocation is an `ArcInner<RamNode>` and the
    // data pointer, stripped of its vtable, is the pointer `Arc<RamNode>` would
    // hold. This is the reverse of the unsizing coercion that made the
    // trait object, and is how `Arc::downcast` is implemented. The strong
    // count moves from `node` to the result unchanged.
    Some(unsafe { Arc::from_raw(Arc::into_raw(node) as *const RamNode) })
}

impl VfsNode for RamNode {
    fn node_type(&self) -> NodeType {
        self.node_type
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, crate::error::KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let data = self.data.read();
        if offset >= data.len() {
            return Ok(0);
        }

        let bytes_to_read = core::cmp::min(buffer.len(), data.len() - offset);
        buffer[..bytes_to_read].copy_from_slice(&data[offset..offset + bytes_to_read]);

        // Update accessed time
        self.metadata.write().accessed = crate::arch::timer::get_timestamp_secs();

        Ok(bytes_to_read)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, crate::error::KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let end = super::ram_write_end(offset, data.len())?;
        let mut file_data = self.data.write();
        super::grow_ram_file(&mut file_data, end)?;
        file_data[offset..end].copy_from_slice(data);

        // Update metadata
        let mut metadata = self.metadata.write();
        metadata.size = file_data.len();
        metadata.modified = crate::arch::timer::get_timestamp_secs();

        Ok(data.len())
    }

    fn metadata(&self) -> Result<Metadata, crate::error::KernelError> {
        Ok(self.metadata.read().clone())
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, crate::error::KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let children = self.children.read();
        let mut entries = Vec::new();

        // Add . and .. entries
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

        // Add children
        for (name, child) in children.iter() {
            entries.push(DirEntry {
                name: name.clone(),
                node_type: child.node_type,
                inode: child.inode,
            });
        }

        Ok(entries)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, crate::error::KernelError> {
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
    ) -> Result<Arc<dyn VfsNode>, crate::error::KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut children = self.children.write();

        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode = NEXT_INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let new_file = Arc::new(RamNode::new_file(
            inode,
            self.inode,
            self.fs_id,
            permissions,
        ));
        children.insert(String::from(name), new_file.clone());

        Ok(new_file as Arc<dyn VfsNode>)
    }

    fn mkdir(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, crate::error::KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut children = self.children.write();

        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode = NEXT_INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let new_dir = Arc::new(RamNode::new_directory(
            inode,
            self.inode,
            self.fs_id,
            permissions,
        ));
        children.insert(String::from(name), new_dir.clone());

        Ok(new_dir as Arc<dyn VfsNode>)
    }

    fn unlink(&self, name: &str) -> Result<(), crate::error::KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut children = self.children.write();

        if let Some(node) = children.get(name) {
            if node.node_type == NodeType::Directory {
                // Check if directory is empty
                let dir_children = node.children.read();
                if !dir_children.is_empty() {
                    return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
                }
            }

            children.remove(name);
            Ok(())
        } else {
            Err(KernelError::FsError(FsError::NotFound))
        }
    }

    fn truncate(&self, size: usize) -> Result<(), crate::error::KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let mut data = self.data.write();
        if size > data.len() {
            super::grow_ram_file(&mut data, size)?;
        } else {
            data.truncate(size);
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
            .and_then(|a| a.downcast_ref::<RamNode>())
            .filter(|np| np.fs_id == self.fs_id)
            .ok_or(KernelError::FsError(FsError::CrossDevice))?;
        if np.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
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
                (self as *const RamNode) < (np as *const RamNode)
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

    fn link(&self, name: &str, target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        // Hard links to directories are not allowed (POSIX)
        if target.node_type() == NodeType::Directory {
            return Err(KernelError::FsError(FsError::IsADirectory));
        }

        // A hard link names the same node, and only within one RamFs
        // instance; anything else is EXDEV. The old code copied the data
        // into a new node, so the two names drifted apart on the next write
        // and a file from another filesystem was silently duplicated (N-45).
        let target = as_ram_arc(target)
            .filter(|t| t.fs_id == self.fs_id)
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

        let inode = NEXT_INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let new_symlink = Arc::new(RamNode::new_symlink(inode, self.inode, self.fs_id, target));
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

/// Global inode counter
static NEXT_INODE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

/// Source of `RamNode::fs_id`, one per `RamFs` instance.
static NEXT_FS_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

/// RAM filesystem
pub struct RamFs {
    root: Arc<RamNode>,
}

impl RamFs {
    /// Create a new RAM filesystem
    pub fn new() -> Self {
        let root_inode = NEXT_INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let fs_id = NEXT_FS_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // Root directory's parent is itself (standard POSIX behavior)
        let root = Arc::new(RamNode::new_directory(
            root_inode,
            root_inode,
            fs_id,
            Permissions::default(),
        ));

        Self { root }
    }
}

impl Default for RamFs {
    fn default() -> Self {
        Self::new()
    }
}

impl Filesystem for RamFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }

    fn name(&self) -> &str {
        "ramfs"
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<(), crate::error::KernelError> {
        // RAM filesystem doesn't need syncing
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    // --- Offsets and sizes (N-122) ---

    #[test]
    fn huge_offsets_fail_instead_of_allocating() {
        let fs = RamFs::new();
        let f = fs.root().create("f", Permissions::default()).unwrap();
        assert!(f.write(1 << 40, b"x").is_err(), "write past the size limit");
        assert!(f.write(usize::MAX, b"xy").is_err(), "offset + len overflow");
        assert!(f.truncate(usize::MAX).is_err(), "truncate past the limit");
        assert_eq!(f.write(0, b"ok").unwrap(), 2);
        assert_eq!(f.metadata().unwrap().size, 2);
    }

    // --- Hard links (N-45) ---

    #[test]
    fn hard_link_shares_the_file() {
        let fs = RamFs::new();
        let root = fs.root();
        let a = root.create("a", Permissions::default()).unwrap();
        a.write(0, b"one").unwrap();
        root.link("b", a.clone()).unwrap();
        // A write through either name is visible through the other.
        root.lookup("b").unwrap().write(0, b"two").unwrap();
        let mut buf = [0u8; 3];
        a.read(0, &mut buf).unwrap();
        assert_eq!(&buf, b"two");
        // Removing one name leaves the file reachable through the other.
        root.unlink("a").unwrap();
        root.lookup("b").unwrap().read(0, &mut buf).unwrap();
        assert_eq!(&buf, b"two");
    }

    #[test]
    fn hard_link_across_ramfs_instances_is_exdev() {
        let fs1 = RamFs::new();
        let fs2 = RamFs::new();
        let a = fs1.root().create("a", Permissions::default()).unwrap();
        assert_eq!(
            fs2.root().link("b", a).err(),
            Some(KernelError::FsError(FsError::CrossDevice))
        );
        assert!(fs2.root().lookup("b").is_err());
    }

    // --- RamFs construction tests ---

    #[test]
    fn test_ramfs_new() {
        let fs = RamFs::new();
        assert_eq!(fs.name(), "ramfs");
        assert!(!fs.is_readonly());
    }

    #[test]
    fn test_ramfs_default() {
        let fs = RamFs::default();
        assert_eq!(fs.name(), "ramfs");
    }

    #[test]
    fn test_ramfs_root_is_directory() {
        let fs = RamFs::new();
        let root = fs.root();
        assert_eq!(root.node_type(), NodeType::Directory);
    }

    #[test]
    fn test_ramfs_sync() {
        let fs = RamFs::new();
        assert!(fs.sync().is_ok());
    }

    // --- File creation and I/O tests ---

    #[test]
    fn test_create_file() {
        let fs = RamFs::new();
        let root = fs.root();

        let file = root.create("hello.txt", Permissions::default());
        assert!(file.is_ok());
        assert_eq!(file.unwrap().node_type(), NodeType::File);
    }

    #[test]
    fn test_create_duplicate_file_fails() {
        let fs = RamFs::new();
        let root = fs.root();

        root.create("dup.txt", Permissions::default()).unwrap();
        let result = root.create("dup.txt", Permissions::default());
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::AlreadyExists)
        );
    }

    #[test]
    fn test_write_and_read_file() {
        let fs = RamFs::new();
        let root = fs.root();

        let file = root.create("data.txt", Permissions::default()).unwrap();

        // Write data
        let written = file.write(0, b"Hello, World!");
        assert!(written.is_ok());
        assert_eq!(written.unwrap(), 13);

        // Read data back
        let mut buf = vec![0u8; 20];
        let read = file.read(0, &mut buf);
        assert!(read.is_ok());
        assert_eq!(read.unwrap(), 13);
        assert_eq!(&buf[..13], b"Hello, World!");
    }

    #[test]
    fn test_write_at_offset() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("offset.txt", Permissions::default()).unwrap();

        // Write at offset 0
        file.write(0, b"AAAA").unwrap();
        // Overwrite middle bytes
        file.write(1, b"BB").unwrap();

        let mut buf = vec![0u8; 4];
        file.read(0, &mut buf).unwrap();
        assert_eq!(&buf, b"ABBA");
    }

    #[test]
    fn test_write_extends_file() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("extend.txt", Permissions::default()).unwrap();

        // Write at offset beyond current size -- should zero-fill gap
        file.write(5, b"end").unwrap();

        let mut buf = vec![0u8; 8];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 8);
        assert_eq!(&buf[..5], &[0, 0, 0, 0, 0]);
        assert_eq!(&buf[5..8], b"end");
    }

    #[test]
    fn test_read_at_offset_beyond_eof() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("eof.txt", Permissions::default()).unwrap();
        file.write(0, b"short").unwrap();

        let mut buf = vec![0u8; 10];
        let n = file.read(100, &mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_read_partial() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("partial.txt", Permissions::default()).unwrap();
        file.write(0, b"Hello, World!").unwrap();

        // Read only 5 bytes
        let mut buf = vec![0u8; 5];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"Hello");
    }

    #[test]
    fn test_read_from_directory_fails() {
        let fs = RamFs::new();
        let root = fs.root();

        let mut buf = vec![0u8; 10];
        let result = root.read(0, &mut buf);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotAFile));
    }

    #[test]
    fn test_write_to_directory_fails() {
        let fs = RamFs::new();
        let root = fs.root();

        let result = root.write(0, b"data");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotAFile));
    }

    // --- File metadata tests ---

    #[test]
    fn test_file_metadata() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("meta.txt", Permissions::default()).unwrap();
        file.write(0, b"content").unwrap();

        let meta = file.metadata().unwrap();
        assert_eq!(meta.node_type, NodeType::File);
        assert_eq!(meta.size, 7);
    }

    #[test]
    fn test_directory_metadata() {
        let fs = RamFs::new();
        let root = fs.root();
        let meta = root.metadata().unwrap();
        assert_eq!(meta.node_type, NodeType::Directory);
    }

    // --- Truncate tests ---

    #[test]
    fn test_truncate_file() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("trunc.txt", Permissions::default()).unwrap();
        file.write(0, b"Hello, World!").unwrap();

        // Truncate to 5 bytes
        file.truncate(5).unwrap();

        let meta = file.metadata().unwrap();
        assert_eq!(meta.size, 5);

        let mut buf = vec![0u8; 10];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"Hello");
    }

    #[test]
    fn test_truncate_to_zero() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("empty.txt", Permissions::default()).unwrap();
        file.write(0, b"data").unwrap();

        file.truncate(0).unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!(meta.size, 0);
    }

    #[test]
    fn test_truncate_directory_fails() {
        let fs = RamFs::new();
        let root = fs.root();
        let result = root.truncate(0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotAFile));
    }

    // --- Directory operations tests ---

    #[test]
    fn test_mkdir() {
        let fs = RamFs::new();
        let root = fs.root();

        let dir = root.mkdir("subdir", Permissions::default());
        assert!(dir.is_ok());
        assert_eq!(dir.unwrap().node_type(), NodeType::Directory);
    }

    #[test]
    fn test_mkdir_duplicate_fails() {
        let fs = RamFs::new();
        let root = fs.root();

        root.mkdir("dup", Permissions::default()).unwrap();
        let result = root.mkdir("dup", Permissions::default());
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::AlreadyExists)
        );
    }

    #[test]
    fn test_mkdir_on_file_fails() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("file", Permissions::default()).unwrap();

        let result = file.mkdir("subdir", Permissions::default());
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::NotADirectory)
        );
    }

    #[test]
    fn test_lookup() {
        let fs = RamFs::new();
        let root = fs.root();

        root.create("myfile", Permissions::default()).unwrap();

        let found = root.lookup("myfile");
        assert!(found.is_ok());
        assert_eq!(found.unwrap().node_type(), NodeType::File);
    }

    #[test]
    fn test_lookup_not_found() {
        let fs = RamFs::new();
        let root = fs.root();

        let result = root.lookup("missing");
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::NotFound)
        );
    }

    #[test]
    fn test_lookup_on_file_fails() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("f", Permissions::default()).unwrap();

        let result = file.lookup("anything");
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::NotADirectory)
        );
    }

    #[test]
    fn test_readdir() {
        let fs = RamFs::new();
        let root = fs.root();

        root.create("file1", Permissions::default()).unwrap();
        root.mkdir("dir1", Permissions::default()).unwrap();

        let entries = root.readdir().unwrap();
        // Should have ".", "..", "file1", "dir1"
        assert_eq!(entries.len(), 4);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"."));
        assert!(names.contains(&".."));
        assert!(names.contains(&"file1"));
        assert!(names.contains(&"dir1"));
    }

    #[test]
    fn test_readdir_on_file_fails() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("f", Permissions::default()).unwrap();

        let result = file.readdir();
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::NotADirectory)
        );
    }

    // --- Unlink tests ---

    #[test]
    fn test_unlink_file() {
        let fs = RamFs::new();
        let root = fs.root();

        root.create("victim", Permissions::default()).unwrap();
        let result = root.unlink("victim");
        assert!(result.is_ok());

        // Should no longer be found
        assert!(root.lookup("victim").is_err());
    }

    #[test]
    fn test_unlink_empty_directory() {
        let fs = RamFs::new();
        let root = fs.root();

        root.mkdir("emptydir", Permissions::default()).unwrap();
        let result = root.unlink("emptydir");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unlink_nonempty_directory_fails() {
        let fs = RamFs::new();
        let root = fs.root();

        let dir = root.mkdir("notempty", Permissions::default()).unwrap();
        dir.create("child", Permissions::default()).unwrap();

        let result = root.unlink("notempty");
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::DirectoryNotEmpty)
        );
    }

    #[test]
    fn test_unlink_not_found() {
        let fs = RamFs::new();
        let root = fs.root();

        let result = root.unlink("phantom");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotFound));
    }

    #[test]
    fn test_unlink_on_file_fails() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("f", Permissions::default()).unwrap();

        let result = file.unlink("anything");
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::NotADirectory)
        );
    }

    #[test]
    fn test_create_on_file_fails() {
        let fs = RamFs::new();
        let root = fs.root();
        let file = root.create("f", Permissions::default()).unwrap();

        let result = file.create("sub", Permissions::default());
        assert!(result.is_err());
        assert_eq!(
            result.err().expect("expected Err"),
            KernelError::FsError(FsError::NotADirectory)
        );
    }

    /// Two ramfs instances are two devices even though both downcast to
    /// `RamNode` (review of the v0.26.0 stack, PR #11).
    #[test]
    fn test_rename_across_ramfs_instances_is_cross_device() {
        let a = RamFs::new();
        let b = RamFs::new();
        let root_a = a.root();
        let root_b = b.root();
        root_a.create("f", Permissions::default()).unwrap();
        assert_eq!(
            root_a.rename("f", &root_b, "g").err(),
            Some(KernelError::FsError(FsError::CrossDevice))
        );
        assert!(root_a.lookup("f").is_ok());
        assert!(root_b.lookup("g").is_err());

        // Within one instance, subdirectories are the same device.
        let sub = root_a.mkdir("d", Permissions::default()).unwrap();
        root_a.rename("f", &sub, "g").unwrap();
        assert!(sub.lookup("g").is_ok());
    }
}

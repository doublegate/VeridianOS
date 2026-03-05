//! tmpfs -- Memory-Backed Filesystem
//!
//! A temporary in-memory filesystem with configurable size limits.
//! Data is stored in heap-backed buffers and is lost on unmount.
//! Supports all standard VFS operations including symlinks.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// Global inode counter for tmpfs (separate from RamFS)
static TMPFS_NEXT_INODE: AtomicU64 = AtomicU64::new(1);

/// tmpfs node
struct TmpNode {
    node_type: NodeType,
    data: RwLock<Vec<u8>>,
    children: RwLock<BTreeMap<String, Arc<TmpNode>>>,
    metadata: RwLock<Metadata>,
    inode: u64,
    parent_inode: u64,
    /// Shared reference to the filesystem's total bytes used counter
    bytes_used: Arc<AtomicUsize>,
    /// Shared reference to the filesystem's size limit
    size_limit: usize,
}

impl TmpNode {
    fn new_file(
        inode: u64,
        parent_inode: u64,
        permissions: Permissions,
        bytes_used: Arc<AtomicUsize>,
        size_limit: usize,
    ) -> Self {
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
            parent_inode,
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
            parent_inode,
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
            parent_inode,
            bytes_used,
            size_limit,
        }
    }

    /// Check if writing `additional` bytes would exceed the size limit.
    /// Returns Ok(()) if within limits, Err if exceeded.
    fn check_space(&self, additional: usize) -> Result<(), KernelError> {
        if self.size_limit == 0 {
            return Ok(()); // No limit
        }
        let current = self.bytes_used.load(Ordering::Relaxed);
        if current.saturating_add(additional) > self.size_limit {
            return Err(KernelError::FsError(FsError::NoSpace));
        }
        Ok(())
    }
}

impl VfsNode for TmpNode {
    fn node_type(&self) -> NodeType {
        self.node_type
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let data = self.data.read();
        if offset >= data.len() {
            return Ok(0);
        }

        let bytes_to_read = core::cmp::min(buffer.len(), data.len() - offset);
        buffer[..bytes_to_read].copy_from_slice(&data[offset..offset + bytes_to_read]);

        self.metadata.write().accessed = crate::arch::timer::get_timestamp_secs();

        Ok(bytes_to_read)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let mut file_data = self.data.write();
        let old_len = file_data.len();

        // Calculate new size after write
        let new_len = core::cmp::max(old_len, offset + data.len());
        let growth = new_len.saturating_sub(old_len);

        // Check space before expanding
        if growth > 0 {
            self.check_space(growth)?;
        }

        // Extend file if necessary
        if offset > file_data.len() {
            file_data.resize(offset, 0);
        }
        if offset + data.len() > file_data.len() {
            file_data.resize(offset + data.len(), 0);
        }
        file_data[offset..offset + data.len()].copy_from_slice(data);

        // Update bytes used counter
        if growth > 0 {
            self.bytes_used.fetch_add(growth, Ordering::Relaxed);
        }

        let mut metadata = self.metadata.write();
        metadata.size = file_data.len();
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
            inode: self.parent_inode,
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

        let mut children = self.children.write();

        if let Some(node) = children.get(name) {
            if node.node_type == NodeType::Directory {
                let dir_children = node.children.read();
                if !dir_children.is_empty() {
                    return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
                }
            }

            // Reclaim bytes if it's a file
            if node.node_type == NodeType::File {
                let data = node.data.read();
                let freed = data.len();
                if freed > 0 {
                    // Saturating sub to avoid underflow on race
                    let prev = self.bytes_used.fetch_sub(freed, Ordering::Relaxed);
                    // Guard against underflow (should not happen but be safe)
                    if prev < freed {
                        self.bytes_used.store(0, Ordering::Relaxed);
                    }
                }
            }

            children.remove(name);
            Ok(())
        } else {
            Err(KernelError::FsError(FsError::NotFound))
        }
    }

    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let mut data = self.data.write();
        let old_len = data.len();

        if size > old_len {
            // Growing: check space
            let growth = size - old_len;
            self.check_space(growth)?;
            data.resize(size, 0);
            self.bytes_used.fetch_add(growth, Ordering::Relaxed);
        } else if size < old_len {
            // Shrinking: reclaim space
            let freed = old_len - size;
            data.truncate(size);
            let prev = self.bytes_used.fetch_sub(freed, Ordering::Relaxed);
            if prev < freed {
                self.bytes_used.store(0, Ordering::Relaxed);
            }
        }

        let mut metadata = self.metadata.write();
        metadata.size = size;
        metadata.modified = crate::arch::timer::get_timestamp_secs();

        Ok(())
    }

    fn link(&self, name: &str, target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        if target.node_type() == NodeType::Directory {
            return Err(KernelError::FsError(FsError::IsADirectory));
        }

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        // Copy data for hard link (same pattern as RamFS)
        let target_meta = target.metadata()?;
        let inode = target_meta.inode;

        let new_node = Arc::new(TmpNode::new_file(
            inode,
            self.inode,
            target_meta.permissions,
            self.bytes_used.clone(),
            self.size_limit,
        ));

        let mut buf = vec![0u8; target_meta.size];
        if !buf.is_empty() {
            let bytes_read = target.read(0, &mut buf)?;
            buf.truncate(bytes_read);
        }
        if !buf.is_empty() {
            new_node.write(0, &buf)?;
        }

        children.insert(String::from(name), new_node);
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
    /// Create a new tmpfs with the given size limit in bytes.
    /// Pass 0 for unlimited.
    pub fn new(size_limit: usize) -> Self {
        let bytes_used = Arc::new(AtomicUsize::new(0));
        let root_inode = TMPFS_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let root = Arc::new(TmpNode::new_directory(
            root_inode,
            root_inode,
            Permissions::default(),
            bytes_used.clone(),
            size_limit,
        ));

        Self {
            root,
            bytes_used,
            size_limit,
        }
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
        assert_eq!(fs.bytes_used(), 13);

        let mut buf = vec![0u8; 20];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 13);
        assert_eq!(&buf[..13], b"Hello, tmpfs!");
    }

    #[test]
    fn test_size_limit_enforcement() {
        let fs = TmpFs::new(10); // 10 byte limit
        let root = fs.root();

        let file = root.create("big.txt", Permissions::default()).unwrap();

        // Write 8 bytes -- should succeed
        assert!(file.write(0, b"12345678").is_ok());
        assert_eq!(fs.bytes_used(), 8);

        // Write 5 more bytes (would total 13) -- should fail
        let result = file.write(8, b"abcde");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NoSpace));

        // Bytes used should still be 8
        assert_eq!(fs.bytes_used(), 8);
    }

    #[test]
    fn test_truncate_reclaims_space() {
        let fs = TmpFs::new(100);
        let root = fs.root();

        let file = root.create("shrink.txt", Permissions::default()).unwrap();
        file.write(0, b"0123456789").unwrap();
        assert_eq!(fs.bytes_used(), 10);

        file.truncate(3).unwrap();
        assert_eq!(fs.bytes_used(), 3);

        let meta = file.metadata().unwrap();
        assert_eq!(meta.size, 3);
    }

    #[test]
    fn test_truncate_grow_checks_space() {
        let fs = TmpFs::new(10);
        let root = fs.root();

        let file = root.create("grow.txt", Permissions::default()).unwrap();
        file.write(0, b"12345").unwrap();
        assert_eq!(fs.bytes_used(), 5);

        // Growing to 10 should work
        assert!(file.truncate(10).is_ok());
        assert_eq!(fs.bytes_used(), 10);

        // Growing to 11 should fail
        let result = file.truncate(11);
        assert!(result.is_err());
    }

    #[test]
    fn test_unlink_reclaims_space() {
        let fs = TmpFs::new(100);
        let root = fs.root();

        let file = root.create("temp.txt", Permissions::default()).unwrap();
        file.write(0, b"some data").unwrap();
        assert_eq!(fs.bytes_used(), 9);

        root.unlink("temp.txt").unwrap();
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
        let fs = TmpFs::new(20);
        let root = fs.root();

        let f1 = root.create("a.txt", Permissions::default()).unwrap();
        let f2 = root.create("b.txt", Permissions::default()).unwrap();

        f1.write(0, b"1234567890").unwrap(); // 10 bytes
        f2.write(0, b"1234567890").unwrap(); // 10 bytes, total 20

        // One more byte should fail
        let result = f1.write(10, b"x");
        assert!(result.is_err());
    }
}

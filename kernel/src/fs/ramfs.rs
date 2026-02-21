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
}

impl RamNode {
    /// Create a new file node
    fn new_file(inode: u64, permissions: Permissions) -> Self {
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
            }),
            inode,
        }
    }

    /// Create a new directory node
    fn new_directory(inode: u64, permissions: Permissions) -> Self {
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
            }),
            inode,
        }
    }
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

        let mut file_data = self.data.write();

        // Extend file if necessary
        if offset > file_data.len() {
            file_data.resize(offset, 0);
        }

        // Write data
        if offset + data.len() > file_data.len() {
            file_data.resize(offset + data.len(), 0);
        }
        file_data[offset..offset + data.len()].copy_from_slice(data);

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
            inode: self.inode, // TODO(phase5): Track parent inode for proper ".." entries
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
        let new_file = Arc::new(RamNode::new_file(inode, permissions));
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
        let new_dir = Arc::new(RamNode::new_directory(inode, permissions));
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
        data.resize(size, 0);

        let mut metadata = self.metadata.write();
        metadata.size = size;
        metadata.modified = crate::arch::timer::get_timestamp_secs();

        Ok(())
    }
}

/// Global inode counter
static NEXT_INODE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

/// RAM filesystem
pub struct RamFs {
    root: Arc<RamNode>,
}

impl RamFs {
    /// Create a new RAM filesystem
    pub fn new() -> Self {
        let root = Arc::new(RamNode::new_directory(
            NEXT_INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
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
}

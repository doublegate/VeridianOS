//! RAM Filesystem Implementation
//!
//! A simple in-memory filesystem for testing and temporary storage.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use spin::RwLock;

use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};

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
                created: 0, // TODO: Add timestamp
                modified: 0,
                accessed: 0,
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
                created: 0,
                modified: 0,
                accessed: 0,
            }),
            inode,
        }
    }
}

impl VfsNode for RamNode {
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, &'static str> {
        if self.node_type != NodeType::File {
            return Err("Not a file");
        }

        let data = self.data.read();
        if offset >= data.len() {
            return Ok(0);
        }

        let bytes_to_read = core::cmp::min(buffer.len(), data.len() - offset);
        buffer[..bytes_to_read].copy_from_slice(&data[offset..offset + bytes_to_read]);

        // Update accessed time
        self.metadata.write().accessed = 0; // TODO: Get current time

        Ok(bytes_to_read)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, &'static str> {
        if self.node_type != NodeType::File {
            return Err("Not a file");
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
        metadata.modified = 0; // TODO: Get current time

        Ok(data.len())
    }

    fn metadata(&self) -> Result<Metadata, &'static str> {
        Ok(self.metadata.read().clone())
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str> {
        if self.node_type != NodeType::Directory {
            return Err("Not a directory");
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
            inode: self.inode, // TODO: Track parent
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

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        if self.node_type != NodeType::Directory {
            return Err("Not a directory");
        }

        let children = self.children.read();
        children
            .get(name)
            .map(|node| node.clone() as Arc<dyn VfsNode>)
            .ok_or("File not found")
    }

    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        if self.node_type != NodeType::Directory {
            return Err("Not a directory");
        }

        let mut children = self.children.write();

        if children.contains_key(name) {
            return Err("File already exists");
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
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        if self.node_type != NodeType::Directory {
            return Err("Not a directory");
        }

        let mut children = self.children.write();

        if children.contains_key(name) {
            return Err("Directory already exists");
        }

        let inode = NEXT_INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let new_dir = Arc::new(RamNode::new_directory(inode, permissions));
        children.insert(String::from(name), new_dir.clone());

        Ok(new_dir as Arc<dyn VfsNode>)
    }

    fn unlink(&self, name: &str) -> Result<(), &'static str> {
        if self.node_type != NodeType::Directory {
            return Err("Not a directory");
        }

        let mut children = self.children.write();

        if let Some(node) = children.get(name) {
            if node.node_type == NodeType::Directory {
                // Check if directory is empty
                let dir_children = node.children.read();
                if !dir_children.is_empty() {
                    return Err("Directory not empty");
                }
            }

            children.remove(name);
            Ok(())
        } else {
            Err("File not found")
        }
    }

    fn truncate(&self, size: usize) -> Result<(), &'static str> {
        if self.node_type != NodeType::File {
            return Err("Not a file");
        }

        let mut data = self.data.write();
        data.resize(size, 0);

        let mut metadata = self.metadata.write();
        metadata.size = size;
        metadata.modified = 0; // TODO: Get current time

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

    fn sync(&self) -> Result<(), &'static str> {
        // RAM filesystem doesn't need syncing
        Ok(())
    }
}

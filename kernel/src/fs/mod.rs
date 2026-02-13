//! Virtual Filesystem (VFS) Layer
//!
//! Provides a unified interface for different filesystem implementations.

#![allow(clippy::should_implement_trait)]

use alloc::{collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use spin::RwLock;

#[cfg(target_arch = "aarch64")]
pub mod bare_lock;
pub mod blockdev;
pub mod blockfs;
pub mod devfs;
pub mod file;
pub mod procfs;
pub mod pty;
pub mod ramfs;

pub use file::{File, FileDescriptor, FileTable, OpenFlags, SeekFrom};

/// Maximum path length
pub const PATH_MAX: usize = 4096;

/// Maximum filename length
pub const NAME_MAX: usize = 255;

/// Filesystem node types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    File,
    Directory,
    CharDevice,
    BlockDevice,
    Pipe,
    Socket,
    Symlink,
}

/// File permissions (Unix-style)
#[derive(Debug, Clone, Copy)]
pub struct Permissions {
    pub owner_read: bool,
    pub owner_write: bool,
    pub owner_exec: bool,
    pub group_read: bool,
    pub group_write: bool,
    pub group_exec: bool,
    pub other_read: bool,
    pub other_write: bool,
    pub other_exec: bool,
}

impl Permissions {
    /// Create default permissions (rwxr-xr-x)
    pub fn default() -> Self {
        Self {
            owner_read: true,
            owner_write: true,
            owner_exec: true,
            group_read: true,
            group_write: false,
            group_exec: true,
            other_read: true,
            other_write: false,
            other_exec: true,
        }
    }

    /// Create read-only permissions
    pub fn read_only() -> Self {
        Self {
            owner_read: true,
            owner_write: false,
            owner_exec: false,
            group_read: true,
            group_write: false,
            group_exec: false,
            other_read: true,
            other_write: false,
            other_exec: false,
        }
    }

    /// Create permissions from Unix mode bits
    pub fn from_mode(mode: u32) -> Self {
        Self {
            owner_read: (mode & 0o400) != 0,
            owner_write: (mode & 0o200) != 0,
            owner_exec: (mode & 0o100) != 0,
            group_read: (mode & 0o040) != 0,
            group_write: (mode & 0o020) != 0,
            group_exec: (mode & 0o010) != 0,
            other_read: (mode & 0o004) != 0,
            other_write: (mode & 0o002) != 0,
            other_exec: (mode & 0o001) != 0,
        }
    }
}

/// File metadata
#[derive(Debug, Clone)]
pub struct Metadata {
    pub node_type: NodeType,
    pub size: usize,
    pub permissions: Permissions,
    pub uid: u32,
    pub gid: u32,
    pub created: u64,
    pub modified: u64,
    pub accessed: u64,
}

/// Directory entry
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub node_type: NodeType,
    pub inode: u64,
}

/// VFS node operations trait
pub trait VfsNode: Send + Sync {
    /// Node type query (also serves as vtable slot padding for AArch64)
    fn node_type(&self) -> NodeType;

    /// Read data from the node
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, &'static str>;

    /// Write data to the node
    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, &'static str>;

    /// Get metadata for the node
    fn metadata(&self) -> Result<Metadata, &'static str>;

    /// List directory entries (if this is a directory)
    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str>;

    /// Look up a child node by name (if this is a directory)
    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, &'static str>;

    /// Create a new file in this directory
    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str>;

    /// Create a new directory in this directory
    fn mkdir(&self, name: &str, permissions: Permissions)
        -> Result<Arc<dyn VfsNode>, &'static str>;

    /// Remove a file or empty directory
    fn unlink(&self, name: &str) -> Result<(), &'static str>;

    /// Truncate the file to the specified size
    fn truncate(&self, size: usize) -> Result<(), &'static str>;
}

/// Filesystem trait
pub trait Filesystem: Send + Sync {
    /// Get the root node of the filesystem
    fn root(&self) -> Arc<dyn VfsNode>;

    /// Get filesystem name
    fn name(&self) -> &str;

    /// Check if filesystem is read-only
    fn is_readonly(&self) -> bool;

    /// Sync filesystem to disk
    fn sync(&self) -> Result<(), &'static str>;
}

/// Mount point information
pub struct MountPoint {
    pub path: String,
    pub filesystem: Arc<dyn Filesystem>,
}

/// Virtual Filesystem Manager
pub struct Vfs {
    /// Root filesystem
    root_fs: Option<Arc<dyn Filesystem>>,

    /// Mount points
    mounts: BTreeMap<String, Arc<dyn Filesystem>>,

    /// Current working directory for processes
    /// TODO: Move this to per-process data
    cwd: String,
}

impl Vfs {
    /// Create a new VFS instance
    pub fn new() -> Self {
        Self {
            root_fs: None,
            mounts: BTreeMap::new(),
            cwd: String::from("/"),
        }
    }
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs {
    /// Mount the root filesystem
    pub fn mount_root(&mut self, fs: Arc<dyn Filesystem>) -> Result<(), &'static str> {
        if self.root_fs.is_some() {
            return Err("Root filesystem already mounted");
        }
        self.root_fs = Some(fs);
        Ok(())
    }

    /// Mount a filesystem at the specified path
    pub fn mount(&mut self, path: String, fs: Arc<dyn Filesystem>) -> Result<(), &'static str> {
        if self.root_fs.is_none() {
            return Err("Root filesystem not mounted");
        }

        if self.mounts.contains_key(&path) {
            return Err("Path already mounted");
        }

        self.mounts.insert(path, fs);
        Ok(())
    }

    /// Mount a filesystem by type at the specified path
    pub fn mount_by_type(
        &mut self,
        path: &str,
        fs_type: &str,
        _flags: u32,
    ) -> Result<(), &'static str> {
        let fs: Arc<dyn Filesystem> = match fs_type {
            "ramfs" => Arc::new(ramfs::RamFs::new()),
            "devfs" => Arc::new(devfs::DevFs::new()),
            "procfs" => Arc::new(procfs::ProcFs::new()),
            "blockfs" => Arc::new(blockfs::BlockFs::new(10000, 1000)),
            _ => return Err("Unknown filesystem type"),
        };

        if path == "/" {
            self.mount_root(fs)
        } else {
            self.mount(path.into(), fs)
        }
    }

    /// Unmount a filesystem at the specified path
    pub fn unmount(&mut self, path: &str) -> Result<(), &'static str> {
        self.mounts
            .remove(path)
            .ok_or("Path not mounted")
            .map(|_| ())
    }

    /// Resolve a path to a VFS node
    pub fn resolve_path(&self, path: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        let root_fs = self.root_fs.as_ref().ok_or("Root filesystem not mounted")?;

        // Normalize path
        let path = if path.starts_with('/') {
            path.into()
        } else {
            // Relative path - prepend CWD
            format!("{}/{}", self.cwd, path)
        };

        // Check if path is under a mount point
        for (mount_path, fs) in self.mounts.iter().rev() {
            if path.starts_with(mount_path) {
                let relative_path = &path[mount_path.len()..];
                return self.traverse_path(fs.root(), relative_path);
            }
        }

        // Use root filesystem
        self.traverse_path(root_fs.root(), &path)
    }

    /// Traverse a path from a starting node
    fn traverse_path(
        &self,
        mut node: Arc<dyn VfsNode>,
        path: &str,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        // Keep track of path components for parent traversal
        let mut path_stack: Vec<Arc<dyn VfsNode>> = Vec::new();
        path_stack.push(node.clone());

        let components: Vec<&str> = path
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();

        for component in components {
            if component == ".." {
                // Go back to parent directory
                if path_stack.len() > 1 {
                    path_stack.pop();
                    node = path_stack.last().unwrap().clone();
                }
                // If at root, stay at root
            } else {
                // Move forward to child
                node = node.lookup(component)?;
                path_stack.push(node.clone());
            }
        }

        Ok(node)
    }

    /// Get current working directory
    pub fn get_cwd(&self) -> &str {
        &self.cwd
    }

    /// Set current working directory
    pub fn set_cwd(&mut self, path: String) -> Result<(), &'static str> {
        // Verify the path exists and is a directory
        let node = self.resolve_path(&path)?;
        let metadata = node.metadata()?;

        if metadata.node_type != NodeType::Directory {
            return Err("Not a directory");
        }

        self.cwd = path;
        Ok(())
    }

    /// Open a file
    pub fn open(&self, path: &str, _flags: OpenFlags) -> Result<Arc<dyn VfsNode>, &'static str> {
        self.resolve_path(path)
    }

    /// Create a directory
    pub fn mkdir(&self, path: &str, permissions: Permissions) -> Result<(), &'static str> {
        // Split path into parent and name
        let (parent_path, name) = if let Some(pos) = path.rfind('/') {
            if pos == 0 {
                ("/", &path[1..])
            } else {
                (&path[..pos], &path[pos + 1..])
            }
        } else {
            return Err("Invalid path");
        };

        // Get parent directory
        let parent = self.resolve_path(parent_path)?;

        // Create directory in parent
        parent.mkdir(name, permissions)?;
        Ok(())
    }

    /// Remove a file or directory
    pub fn unlink(&self, path: &str) -> Result<(), &'static str> {
        // Split path into parent and name
        let (parent_path, name) = if let Some(pos) = path.rfind('/') {
            if pos == 0 {
                ("/", &path[1..])
            } else {
                (&path[..pos], &path[pos + 1..])
            }
        } else {
            return Err("Invalid path");
        };

        // Get parent directory
        let parent = self.resolve_path(parent_path)?;

        // Remove from parent
        parent.unlink(name)
    }

    /// Sync all filesystems
    pub fn sync(&self) -> Result<(), &'static str> {
        // Sync root filesystem
        if let Some(ref root) = self.root_fs {
            root.sync()?;
        }

        // Sync all mounted filesystems
        for fs in self.mounts.values() {
            fs.sync()?;
        }

        Ok(())
    }
}

/// Global VFS instance - using pointer pattern for all architectures
/// This avoids static mut Option issues and provides consistent behavior
static mut VFS_PTR: *mut RwLock<Vfs> = core::ptr::null_mut();

/// Get the VFS instance (unified for all architectures)
pub fn get_vfs() -> &'static RwLock<Vfs> {
    unsafe {
        let ptr = core::ptr::read_volatile(&raw const VFS_PTR);
        if ptr.is_null() {
            panic!("VFS not initialized");
        }
        &*ptr
    }
}

/// Try to get the VFS instance without panicking
pub fn try_get_vfs() -> Option<&'static RwLock<Vfs>> {
    unsafe {
        let ptr = core::ptr::read_volatile(&raw const VFS_PTR);
        if ptr.is_null() {
            None
        } else {
            Some(&*ptr)
        }
    }
}

/// Initialize the VFS with a RAM filesystem as root
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[VFS] Initializing Virtual Filesystem...");

    unsafe {
        if !core::ptr::read_volatile(&raw const VFS_PTR).is_null() {
            println!("[VFS] WARNING: VFS already initialized! Skipping re-initialization.");
            return;
        }

        println!("[VFS] Creating VFS structure...");

        let mounts = BTreeMap::new();
        let cwd = String::from("/");
        let vfs = Vfs {
            root_fs: None,
            mounts,
            cwd,
        };
        let vfs_lock = RwLock::new(vfs);
        let vfs_box = alloc::boxed::Box::new(vfs_lock);
        let ptr = alloc::boxed::Box::leak(vfs_box) as *mut RwLock<Vfs>;

        // Memory barriers before assignment
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("dsb sy", "isb", options(nostack, nomem, preserves_flags));
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("fence rw, rw", options(nostack, nomem, preserves_flags));

        core::ptr::write_volatile(&raw mut VFS_PTR, ptr);

        // Memory barriers after assignment
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("dsb sy", "isb", options(nostack, nomem, preserves_flags));
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("fence rw, rw", options(nostack, nomem, preserves_flags));

        println!("[VFS] VFS initialized successfully");
    }

    // Create and mount filesystems
    #[cfg(feature = "alloc")]
    {
        println!("[VFS] Creating RAM filesystem...");

        // Create a RAM filesystem as the root
        let ramfs = ramfs::RamFs::new();

        // Mount as root
        {
            let vfs = get_vfs();
            let mut vfs_guard = vfs.write();
            vfs_guard.mount_root(Arc::new(ramfs)).ok();
        }

        println!("[VFS] RAM filesystem mounted as root");

        // Create standard directories in root
        {
            let vfs = get_vfs();
            let vfs_guard = vfs.read();
            if let Some(ref root_fs) = vfs_guard.root_fs {
                let root = root_fs.root();
                root.mkdir("bin", Permissions::default()).ok();
                root.mkdir("boot", Permissions::default()).ok();
                root.mkdir("dev", Permissions::default()).ok();
                root.mkdir("etc", Permissions::default()).ok();
                root.mkdir("home", Permissions::default()).ok();
                root.mkdir("lib", Permissions::default()).ok();
                root.mkdir("mnt", Permissions::default()).ok();
                root.mkdir("opt", Permissions::default()).ok();
                root.mkdir("proc", Permissions::default()).ok();
                root.mkdir("root", Permissions::default()).ok();
                root.mkdir("sbin", Permissions::default()).ok();
                root.mkdir("sys", Permissions::default()).ok();
                root.mkdir("tmp", Permissions::default()).ok();
                root.mkdir("usr", Permissions::default()).ok();
                root.mkdir("var", Permissions::default()).ok();
            }
        }

        println!("[VFS] Created standard directories");

        // Create DevFS and mount at /dev
        println!("[VFS] Creating device filesystem...");
        let devfs = devfs::DevFs::new();

        {
            let vfs = get_vfs();
            let mut vfs_guard = vfs.write();
            vfs_guard.mount("/dev".into(), Arc::new(devfs)).ok();
        }

        println!("[VFS] Device filesystem mounted at /dev");

        // Create ProcFS and mount at /proc
        println!("[VFS] Creating process filesystem...");
        let procfs = procfs::ProcFs::new();

        {
            let vfs = get_vfs();
            let mut vfs_guard = vfs.write();
            vfs_guard.mount("/proc".into(), Arc::new(procfs)).ok();
        }

        println!("[VFS] Process filesystem mounted at /proc");

        println!("[VFS] Virtual Filesystem initialization complete");
    }

    #[cfg(not(feature = "alloc"))]
    {
        println!("[VFS] Skipping VFS initialization (no alloc)");
    }
}

/// Read the entire contents of a file into a Vec<u8>
///
/// This is a convenience function that opens a file, reads its entire
/// contents into memory, and returns the data as a byte vector.
///
/// # Arguments
/// * `path` - The filesystem path to the file
///
/// # Returns
/// * `Ok(Vec<u8>)` - The file contents on success
/// * `Err(&'static str)` - An error message on failure
pub fn read_file(path: &str) -> Result<Vec<u8>, &'static str> {
    let vfs = get_vfs().read();

    // Resolve the path to a VFS node
    let node = vfs.resolve_path(path)?;

    // Get file metadata to determine size
    let metadata = node.metadata()?;

    // Ensure it's a file, not a directory
    if metadata.node_type != NodeType::File {
        return Err("Not a file");
    }

    // Allocate buffer for file contents
    let size = metadata.size;
    let mut buffer = vec![0u8; size];

    // Read the entire file
    let bytes_read = node.read(0, &mut buffer)?;

    // Truncate to actual bytes read (in case file changed)
    buffer.truncate(bytes_read);

    Ok(buffer)
}

/// Write data to a file, creating it if it doesn't exist
///
/// # Arguments
/// * `path` - The filesystem path to the file
/// * `data` - The data to write
///
/// # Returns
/// * `Ok(usize)` - The number of bytes written on success
/// * `Err(&'static str)` - An error message on failure
pub fn write_file(path: &str, data: &[u8]) -> Result<usize, &'static str> {
    let vfs = get_vfs().read();

    // Try to resolve the path first
    let node = match vfs.resolve_path(path) {
        Ok(node) => node,
        Err(_) => {
            // File doesn't exist, try to create it
            // Split path into parent directory and filename
            let (parent_path, filename) = if let Some(pos) = path.rfind('/') {
                if pos == 0 {
                    ("/", &path[1..])
                } else {
                    (&path[..pos], &path[pos + 1..])
                }
            } else {
                return Err("Invalid path");
            };

            // Get parent directory
            let parent = vfs.resolve_path(parent_path)?;

            // Create the file
            parent.create(filename, Permissions::default())?
        }
    };

    // Truncate the file first
    node.truncate(0)?;

    // Write the data
    node.write(0, data)
}

/// Check if a file exists
pub fn file_exists(path: &str) -> bool {
    let vfs = get_vfs().read();
    vfs.resolve_path(path).is_ok()
}

/// Get file size without reading contents
pub fn file_size(path: &str) -> Result<usize, &'static str> {
    let vfs = get_vfs().read();
    let node = vfs.resolve_path(path)?;
    let metadata = node.metadata()?;
    Ok(metadata.size)
}

/// Copy a file from one location to another
pub fn copy_file(src_path: &str, dst_path: &str) -> Result<usize, &'static str> {
    let data = read_file(src_path)?;
    write_file(dst_path, &data)
}

/// Append data to a file
pub fn append_file(path: &str, data: &[u8]) -> Result<usize, &'static str> {
    let vfs = get_vfs().read();
    let node = vfs.resolve_path(path)?;
    let metadata = node.metadata()?;
    let current_size = metadata.size;

    // Write at the end of the file
    node.write(current_size, data)
}

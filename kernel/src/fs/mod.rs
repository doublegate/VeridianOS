//! Virtual Filesystem (VFS) Layer
//!
//! Provides a unified interface for different filesystem implementations.

#![allow(clippy::should_implement_trait)]

use alloc::{collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use spin::RwLock;

use crate::error::KernelError;

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
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError>;

    /// Write data to the node
    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError>;

    /// Get metadata for the node
    fn metadata(&self) -> Result<Metadata, KernelError>;

    /// List directory entries (if this is a directory)
    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError>;

    /// Look up a child node by name (if this is a directory)
    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError>;

    /// Create a new file in this directory
    fn create(&self, name: &str, permissions: Permissions)
        -> Result<Arc<dyn VfsNode>, KernelError>;

    /// Create a new directory in this directory
    fn mkdir(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, KernelError>;

    /// Remove a file or empty directory
    fn unlink(&self, name: &str) -> Result<(), KernelError>;

    /// Truncate the file to the specified size
    fn truncate(&self, size: usize) -> Result<(), KernelError>;
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
    fn sync(&self) -> Result<(), KernelError>;
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
    /// TODO(phase3): Move this to per-process data
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
    pub fn mount_root(&mut self, fs: Arc<dyn Filesystem>) -> Result<(), KernelError> {
        if self.root_fs.is_some() {
            return Err(KernelError::FsError(crate::error::FsError::AlreadyMounted));
        }
        self.root_fs = Some(fs);
        Ok(())
    }

    /// Mount a filesystem at the specified path
    pub fn mount(&mut self, path: String, fs: Arc<dyn Filesystem>) -> Result<(), KernelError> {
        if self.root_fs.is_none() {
            return Err(KernelError::FsError(crate::error::FsError::NoRootFs));
        }

        if self.mounts.contains_key(&path) {
            return Err(KernelError::FsError(crate::error::FsError::AlreadyMounted));
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
    ) -> Result<(), KernelError> {
        let fs: Arc<dyn Filesystem> = match fs_type {
            "ramfs" => Arc::new(ramfs::RamFs::new()),
            "devfs" => Arc::new(devfs::DevFs::new()),
            "procfs" => Arc::new(procfs::ProcFs::new()),
            "blockfs" => Arc::new(blockfs::BlockFs::new(10000, 1000)),
            _ => return Err(KernelError::FsError(crate::error::FsError::UnknownFsType)),
        };

        if path == "/" {
            self.mount_root(fs)
        } else {
            self.mount(path.into(), fs)
        }
    }

    /// Unmount a filesystem at the specified path
    pub fn unmount(&mut self, path: &str) -> Result<(), KernelError> {
        self.mounts
            .remove(path)
            .ok_or(KernelError::FsError(crate::error::FsError::NotMounted))
            .map(|_| ())
    }

    /// Resolve a path to a VFS node
    pub fn resolve_path(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        let root_fs = self
            .root_fs
            .as_ref()
            .ok_or(KernelError::FsError(crate::error::FsError::NoRootFs))?;

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
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
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
                    // path_stack.len() > 1 was checked above, so last() always succeeds.
                    // Use ok_or to return an error instead of panicking on the
                    // impossible case.
                    node = path_stack
                        .last()
                        .ok_or(KernelError::LegacyError {
                            message: "internal error: path stack unexpectedly empty",
                        })?
                        .clone();
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
    pub fn set_cwd(&mut self, path: String) -> Result<(), KernelError> {
        // Verify the path exists and is a directory
        let node = self.resolve_path(&path)?;
        let metadata = node.metadata()?;

        if metadata.node_type != NodeType::Directory {
            return Err(KernelError::FsError(crate::error::FsError::NotADirectory));
        }

        self.cwd = path;
        Ok(())
    }

    /// Open a file
    ///
    /// Checks MAC policy before allowing access.
    pub fn open(&self, path: &str, flags: OpenFlags) -> Result<Arc<dyn VfsNode>, KernelError> {
        // Determine access type from flags
        let access = if flags.write {
            crate::security::AccessType::Write
        } else {
            crate::security::AccessType::Read
        };

        // Get current process PID for MAC check (0 = kernel context)
        let pid = crate::process::current_process()
            .map(|p| p.pid.0)
            .unwrap_or(0);

        crate::security::mac::check_file_access(path, access, pid)?;

        self.resolve_path(path)
    }

    /// Create a directory
    ///
    /// Checks MAC policy (Write access to file domain) before creating.
    pub fn mkdir(&self, path: &str, permissions: Permissions) -> Result<(), KernelError> {
        // MAC check: creating a directory requires Write access
        let pid = crate::process::current_process()
            .map(|p| p.pid.0)
            .unwrap_or(0);
        crate::security::mac::check_file_access(path, crate::security::AccessType::Write, pid)?;

        // Split path into parent and name
        let (parent_path, name) = if let Some(pos) = path.rfind('/') {
            if pos == 0 {
                ("/", &path[1..])
            } else {
                (&path[..pos], &path[pos + 1..])
            }
        } else {
            return Err(KernelError::FsError(crate::error::FsError::InvalidPath));
        };

        // Get parent directory
        let parent = self.resolve_path(parent_path)?;

        // Create directory in parent
        parent.mkdir(name, permissions)?;
        Ok(())
    }

    /// Remove a file or directory
    pub fn unlink(&self, path: &str) -> Result<(), KernelError> {
        // Split path into parent and name
        let (parent_path, name) = if let Some(pos) = path.rfind('/') {
            if pos == 0 {
                ("/", &path[1..])
            } else {
                (&path[..pos], &path[pos + 1..])
            }
        } else {
            return Err(KernelError::FsError(crate::error::FsError::InvalidPath));
        };

        // Get parent directory
        let parent = self.resolve_path(parent_path)?;

        // Remove from parent
        parent.unlink(name)
    }

    /// Sync all filesystems
    pub fn sync(&self) -> Result<(), KernelError> {
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

/// Global VFS instance using OnceLock for safe initialization.
static VFS_LOCK: crate::sync::once_lock::OnceLock<RwLock<Vfs>> =
    crate::sync::once_lock::OnceLock::new();

/// Get the VFS instance (unified for all architectures).
///
/// Panics if the VFS has not been initialized via [`init`].
/// Prefer [`try_get_vfs`] in contexts where a panic is unacceptable.
pub fn get_vfs() -> &'static RwLock<Vfs> {
    VFS_LOCK
        .get()
        .expect("VFS not initialized: init() was not called")
}

/// Try to get the VFS instance without panicking
pub fn try_get_vfs() -> Option<&'static RwLock<Vfs>> {
    VFS_LOCK.get()
}

/// Initialize the VFS with a RAM filesystem as root
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[VFS] Initializing Virtual Filesystem...");

    println!("[VFS] Creating VFS structure...");
    let vfs = Vfs::new();
    let vfs_lock = RwLock::new(vfs);

    match VFS_LOCK.set(vfs_lock) {
        Ok(()) => println!("[VFS] VFS initialized successfully"),
        Err(_) => {
            println!("[VFS] WARNING: VFS already initialized! Skipping re-initialization.");
            return;
        }
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
pub fn read_file(path: &str) -> Result<Vec<u8>, KernelError> {
    let vfs = get_vfs().read();

    // Resolve the path to a VFS node
    let node = vfs.resolve_path(path)?;

    // Get file metadata to determine size
    let metadata = node.metadata()?;

    // Ensure it's a file, not a directory
    if metadata.node_type != NodeType::File {
        return Err(KernelError::FsError(crate::error::FsError::NotAFile));
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
pub fn write_file(path: &str, data: &[u8]) -> Result<usize, KernelError> {
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
                return Err(KernelError::FsError(crate::error::FsError::InvalidPath));
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
pub fn file_size(path: &str) -> Result<usize, KernelError> {
    let vfs = get_vfs().read();
    let node = vfs.resolve_path(path)?;
    let metadata = node.metadata()?;
    Ok(metadata.size)
}

/// Copy a file from one location to another
pub fn copy_file(src_path: &str, dst_path: &str) -> Result<usize, KernelError> {
    let data = read_file(src_path)?;
    write_file(dst_path, &data)
}

/// Append data to a file
pub fn append_file(path: &str, data: &[u8]) -> Result<usize, KernelError> {
    let vfs = get_vfs().read();
    let node = vfs.resolve_path(path)?;
    let metadata = node.metadata()?;
    let current_size = metadata.size;

    // Write at the end of the file
    node.write(current_size, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a Vfs with a ramfs root filesystem already mounted.
    fn make_vfs_with_root() -> Vfs {
        let mut vfs = Vfs::new();
        let ramfs = Arc::new(ramfs::RamFs::new());
        vfs.mount_root(ramfs).expect("mount_root should succeed");
        vfs
    }

    // --- Permissions tests ---

    #[test]
    fn test_permissions_default() {
        let perm = Permissions::default();
        assert!(perm.owner_read);
        assert!(perm.owner_write);
        assert!(perm.owner_exec);
        assert!(perm.group_read);
        assert!(!perm.group_write);
        assert!(perm.group_exec);
        assert!(perm.other_read);
        assert!(!perm.other_write);
        assert!(perm.other_exec);
    }

    #[test]
    fn test_permissions_read_only() {
        let perm = Permissions::read_only();
        assert!(perm.owner_read);
        assert!(!perm.owner_write);
        assert!(!perm.owner_exec);
        assert!(perm.group_read);
        assert!(!perm.group_write);
    }

    #[test]
    fn test_permissions_from_mode_755() {
        let perm = Permissions::from_mode(0o755);
        assert!(perm.owner_read);
        assert!(perm.owner_write);
        assert!(perm.owner_exec);
        assert!(perm.group_read);
        assert!(!perm.group_write);
        assert!(perm.group_exec);
        assert!(perm.other_read);
        assert!(!perm.other_write);
        assert!(perm.other_exec);
    }

    #[test]
    fn test_permissions_from_mode_644() {
        let perm = Permissions::from_mode(0o644);
        assert!(perm.owner_read);
        assert!(perm.owner_write);
        assert!(!perm.owner_exec);
        assert!(perm.group_read);
        assert!(!perm.group_write);
        assert!(!perm.group_exec);
        assert!(perm.other_read);
        assert!(!perm.other_write);
        assert!(!perm.other_exec);
    }

    #[test]
    fn test_permissions_from_mode_000() {
        let perm = Permissions::from_mode(0o000);
        assert!(!perm.owner_read);
        assert!(!perm.owner_write);
        assert!(!perm.owner_exec);
        assert!(!perm.group_read);
        assert!(!perm.other_read);
    }

    // --- Vfs construction tests ---

    #[test]
    fn test_vfs_new() {
        let vfs = Vfs::new();
        assert_eq!(vfs.get_cwd(), "/");
    }

    #[test]
    fn test_vfs_default() {
        let vfs = Vfs::default();
        assert_eq!(vfs.get_cwd(), "/");
    }

    // --- Mount tests ---

    #[test]
    fn test_mount_root() {
        let mut vfs = Vfs::new();
        let ramfs = Arc::new(ramfs::RamFs::new());
        let result = vfs.mount_root(ramfs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_root_twice_fails() {
        let mut vfs = Vfs::new();
        let ramfs1 = Arc::new(ramfs::RamFs::new());
        let ramfs2 = Arc::new(ramfs::RamFs::new());

        vfs.mount_root(ramfs1).unwrap();
        let result = vfs.mount_root(ramfs2);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::AlreadyMounted)
        );
    }

    #[test]
    fn test_mount_without_root_fails() {
        let mut vfs = Vfs::new();
        let ramfs = Arc::new(ramfs::RamFs::new());
        let result = vfs.mount("/dev".into(), ramfs);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::NoRootFs)
        );
    }

    #[test]
    fn test_mount_at_path() {
        let mut vfs = make_vfs_with_root();
        let devfs = Arc::new(devfs::DevFs::new());
        let result = vfs.mount("/dev".into(), devfs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_duplicate_path_fails() {
        let mut vfs = make_vfs_with_root();
        let fs1 = Arc::new(ramfs::RamFs::new());
        let fs2 = Arc::new(ramfs::RamFs::new());

        vfs.mount("/mnt".into(), fs1).unwrap();
        let result = vfs.mount("/mnt".into(), fs2);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::AlreadyMounted)
        );
    }

    // --- Unmount tests ---

    #[test]
    fn test_unmount() {
        let mut vfs = make_vfs_with_root();
        let fs = Arc::new(ramfs::RamFs::new());
        vfs.mount("/mnt".into(), fs).unwrap();

        let result = vfs.unmount("/mnt");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unmount_nonexistent_fails() {
        let mut vfs = make_vfs_with_root();
        let result = vfs.unmount("/nonexistent");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::NotMounted)
        );
    }

    // --- mount_by_type tests ---

    #[test]
    fn test_mount_by_type_ramfs() {
        let mut vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/tmp", "ramfs", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_by_type_devfs() {
        let mut vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/dev", "devfs", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_by_type_procfs() {
        let mut vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/proc", "procfs", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_by_type_unknown_fails() {
        let mut vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/foo", "unknownfs", 0);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::UnknownFsType)
        );
    }

    #[test]
    fn test_mount_by_type_root() {
        let mut vfs = Vfs::new();
        let result = vfs.mount_by_type("/", "ramfs", 0);
        assert!(result.is_ok());
    }

    // --- Path resolution tests ---

    #[test]
    fn test_resolve_root_path() {
        let vfs = make_vfs_with_root();
        let result = vfs.resolve_path("/");
        assert!(result.is_ok());
        let node = result.unwrap();
        assert_eq!(node.node_type(), NodeType::Directory);
    }

    #[test]
    fn test_resolve_no_root_fails() {
        let vfs = Vfs::new();
        let result = vfs.resolve_path("/anything");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::NoRootFs)
        );
    }

    #[test]
    fn test_resolve_nonexistent_path() {
        let vfs = make_vfs_with_root();
        let result = vfs.resolve_path("/nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_created_directory() {
        let vfs = make_vfs_with_root();

        // Create directory via the root node
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.mkdir("testdir", Permissions::default()).unwrap();

        let result = vfs.resolve_path("/testdir");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().node_type(), NodeType::Directory);
    }

    #[test]
    fn test_resolve_nested_path() {
        let vfs = make_vfs_with_root();

        let root = vfs.root_fs.as_ref().unwrap().root();
        let sub = root.mkdir("a", Permissions::default()).unwrap();
        sub.mkdir("b", Permissions::default()).unwrap();

        let result = vfs.resolve_path("/a/b");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().node_type(), NodeType::Directory);
    }

    #[test]
    fn test_resolve_path_with_dot() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.mkdir("mydir", Permissions::default()).unwrap();

        // "." should be ignored in path traversal
        let result = vfs.resolve_path("/./mydir/.");
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_path_with_dotdot() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        let sub = root.mkdir("parent", Permissions::default()).unwrap();
        sub.mkdir("child", Permissions::default()).unwrap();

        // /parent/child/.. should resolve to /parent
        let result = vfs.resolve_path("/parent/child/..");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().node_type(), NodeType::Directory);
    }

    #[test]
    fn test_resolve_dotdot_at_root() {
        let vfs = make_vfs_with_root();

        // Going up from root should stay at root
        let result = vfs.resolve_path("/../..");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().node_type(), NodeType::Directory);
    }

    // --- mkdir and unlink tests ---

    #[test]
    fn test_mkdir_via_vfs() {
        let vfs = make_vfs_with_root();
        let result = vfs.mkdir("/newdir", Permissions::default());
        assert!(result.is_ok());

        // Verify it exists
        let node = vfs.resolve_path("/newdir").unwrap();
        assert_eq!(node.node_type(), NodeType::Directory);
    }

    #[test]
    fn test_mkdir_invalid_path() {
        let vfs = make_vfs_with_root();
        let result = vfs.mkdir("no_slash", Permissions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_unlink_file() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.create("testfile", Permissions::default()).unwrap();

        let result = vfs.unlink("/testfile");
        assert!(result.is_ok());

        // Should no longer exist
        assert!(vfs.resolve_path("/testfile").is_err());
    }

    #[test]
    fn test_unlink_nonexistent() {
        let vfs = make_vfs_with_root();
        let result = vfs.unlink("/ghost");
        assert!(result.is_err());
    }

    // --- set_cwd tests ---

    #[test]
    fn test_set_cwd() {
        let mut vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.mkdir("home", Permissions::default()).unwrap();

        let result = vfs.set_cwd(String::from("/home"));
        assert!(result.is_ok());
        assert_eq!(vfs.get_cwd(), "/home");
    }

    #[test]
    fn test_set_cwd_not_directory_fails() {
        let mut vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.create("afile", Permissions::default()).unwrap();

        let result = vfs.set_cwd(String::from("/afile"));
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::NotADirectory)
        );
    }

    // --- sync tests ---

    #[test]
    fn test_sync_with_root() {
        let vfs = make_vfs_with_root();
        let result = vfs.sync();
        assert!(result.is_ok());
    }

    #[test]
    fn test_sync_without_root() {
        let vfs = Vfs::new();
        let result = vfs.sync();
        assert!(result.is_ok()); // No root, but should not error
    }

    // --- NodeType tests ---

    #[test]
    fn test_node_type_equality() {
        assert_eq!(NodeType::File, NodeType::File);
        assert_eq!(NodeType::Directory, NodeType::Directory);
        assert_ne!(NodeType::File, NodeType::Directory);
        assert_ne!(NodeType::CharDevice, NodeType::BlockDevice);
    }
}

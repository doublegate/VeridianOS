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
pub mod eventfd;
pub mod ext4;
pub mod fat32;
pub mod file;
pub mod flock;
pub mod inotify;
pub mod pipe;
pub mod procfs;
pub mod pty;
pub mod ramfs;
pub mod signalfd;
pub mod tar;
pub mod timerfd;
pub mod tmpfs;
pub mod xattr;

// Phase 8 Wave 3: Enterprise Storage
pub mod nfs;
pub mod smb;

pub use file::{File, FileDescriptor, FileTable, OpenFlags, SeekFrom};

/// Maximum path length
pub const PATH_MAX: usize = 4096;

/// Maximum filename length
pub const NAME_MAX: usize = 255;

/// Maximum number of symbolic link traversals before returning ELOOP.
///
/// POSIX recommends at least `SYMLOOP_MAX` (typically 8-40). We use 40
/// to be generous while still detecting infinite cycles.
pub const SYMLINK_MAX_DEPTH: usize = 40;

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
    /// Sticky bit (S_ISVTX): in a directory, only an entry's owner, the
    /// directory's owner or root may remove or rename the entry.
    pub sticky: bool,
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
            sticky: false,
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
            sticky: false,
        }
    }

    /// Create permissions from Unix mode bits
    /// The permission bits as a POSIX mode (`0o777` mask), the inverse of
    /// [`from_mode`](Self::from_mode).
    pub fn to_mode(&self) -> u32 {
        [
            (self.owner_read, 0o400),
            (self.owner_write, 0o200),
            (self.owner_exec, 0o100),
            (self.group_read, 0o040),
            (self.group_write, 0o020),
            (self.group_exec, 0o010),
            (self.other_read, 0o004),
            (self.other_write, 0o002),
            (self.other_exec, 0o001),
            (self.sticky, 0o1000),
        ]
        .iter()
        .filter(|(set, _)| *set)
        .map(|(_, bit)| bit)
        .sum()
    }

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
            sticky: (mode & 0o1000) != 0,
        }
    }

    /// Check if the given uid/gid has read access.
    pub fn can_read(&self, uid: u32, gid: u32, file_uid: u32, file_gid: u32) -> bool {
        if uid == 0 {
            return true; // root bypasses permission checks
        }
        if uid == file_uid {
            self.owner_read
        } else if gid == file_gid {
            self.group_read
        } else {
            self.other_read
        }
    }

    /// Check if the given uid/gid has write access.
    pub fn can_write(&self, uid: u32, gid: u32, file_uid: u32, file_gid: u32) -> bool {
        if uid == 0 {
            return true;
        }
        if uid == file_uid {
            self.owner_write
        } else if gid == file_gid {
            self.group_write
        } else {
            self.other_write
        }
    }

    /// Check if the given uid/gid has run access.
    pub fn can_run(&self, uid: u32, gid: u32, file_uid: u32, file_gid: u32) -> bool {
        if uid == 0 {
            return true;
        }
        if uid == file_uid {
            self.owner_exec
        } else if gid == file_gid {
            self.group_exec
        } else {
            self.other_exec
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
    pub inode: u64,
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

    /// Create a hard link to this node
    fn link(&self, _name: &str, _target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        Err(KernelError::NotImplemented {
            feature: "hard links",
        })
    }

    /// Create a symbolic link in this directory.
    ///
    /// Creates a new symlink entry named `name` in this directory node
    /// that points to `target`. The target path is stored as the symlink's
    /// data content and is not validated (it may be relative or absolute,
    /// and the target need not exist).
    ///
    /// # Default Implementation
    ///
    /// Returns `NotImplemented` for filesystems that do not support
    /// symbolic links (e.g., DevFS, ProcFS). Filesystems that support
    /// symlinks (e.g., RamFS, BlockFS) override this method.
    ///
    /// # Arguments
    /// - `name`: The name of the symlink entry to create in this directory.
    /// - `target`: The target path that the symlink points to.
    ///
    /// # Returns
    /// An `Arc<dyn VfsNode>` representing the newly created symlink node.
    fn symlink(&self, _name: &str, _target: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::NotImplemented {
            feature: "symbolic links",
        })
    }

    /// Read the target of a symbolic link.
    ///
    /// If this node is a symbolic link, returns the target path as a
    /// `String`. The target is the raw path stored when the symlink was
    /// created and is not resolved or canonicalized.
    ///
    /// # Default Implementation
    ///
    /// Returns `NotImplemented` for filesystem nodes that are not symbolic
    /// links or for filesystems that do not support symlinks. Callers
    /// should check `node_type() == NodeType::Symlink` before calling, or
    /// handle the error.
    ///
    /// # Returns
    /// - `Ok(String)`: The symlink target path.
    /// - `Err(NotImplemented)`: This node is not a symlink or the filesystem
    ///   does not support readlink.
    /// - `Err(FsError(NotASymlink))`: The node exists but is not a symlink
    ///   (used by BlockFS for type-checked readlink).
    fn readlink(&self) -> Result<String, KernelError> {
        Err(KernelError::NotImplemented {
            feature: "readlink",
        })
    }

    /// Change permissions on this node
    fn chmod(&self, _permissions: Permissions) -> Result<(), KernelError> {
        Err(KernelError::NotImplemented { feature: "chmod" })
    }

    /// Change the owner and/or group of this node; `None` leaves that id
    /// unchanged. Permission checks are the caller's job.
    fn chown(&self, _uid: Option<u32>, _gid: Option<u32>) -> Result<(), KernelError> {
        Err(KernelError::NotImplemented { feature: "chown" })
    }

    /// Poll readiness for I/O multiplexing (poll/epoll).
    ///
    /// Returns a bitmask of ready events using POLL* constants:
    /// - bit 0 (POLLIN=1): readable without blocking
    /// - bit 2 (POLLOUT=4): writable without blocking
    /// - bit 3 (POLLERR=8): error condition
    /// - bit 4 (POLLHUP=16): hangup (peer closed)
    ///
    /// Default: regular files are always readable and writable.
    /// Pipe nodes override this to check actual buffer state.
    fn poll_readiness(&self) -> u16 {
        // Regular files/dirs: always ready for read+write
        0x0001 | 0x0004 // POLLIN | POLLOUT
    }

    /// Downcast to `&dyn core::any::Any` for type-specific operations.
    ///
    /// Used by syscall handlers that need to extract implementation-specific
    /// state from a VfsNode (e.g., timerfd_settime needs the internal timer
    /// ID from a TimerFdNode). Default returns `None`; only nodes that need
    /// downcasting override this.
    fn as_any(&self) -> Option<&dyn core::any::Any> {
        None
    }

    /// `(major, minor)` for device nodes, `None` for everything else.
    ///
    /// Use this, never the path a file was opened with, to decide whether a
    /// file is a particular device: a path such as `/tmp/dri/x` says nothing
    /// about what the node is (W-7).
    fn device_id(&self) -> Option<(u32, u32)> {
        None
    }
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

/// Make `path` absolute (relative to `cwd`) and remove `.`, `..` and empty
/// components lexically. `..` at the root stays at the root. The result
/// always starts with `/` and never ends with one (except `/` itself).
pub(crate) fn normalize_path(path: &str, cwd: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let joined = if path.starts_with('/') {
        None
    } else {
        Some(cwd)
    };
    for component in joined
        .into_iter()
        .chain(core::iter::once(path))
        .flat_map(|p| p.split('/'))
    {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let mut out = String::with_capacity(path.len() + 1);
    for part in &parts {
        out.push('/');
        out.push_str(part);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// Whether normalized `path` is `mount` itself or lies below it, on a
/// whole-component boundary.
fn path_is_under(path: &str, mount: &str) -> bool {
    mount == "/"
        || path == mount
        || (path.starts_with(mount) && path.as_bytes().get(mount.len()) == Some(&b'/'))
}

/// The part of normalized `path` below `mount` (empty for the mount root).
fn path_relative_to_mount<'a>(path: &'a str, mount: &str) -> &'a str {
    if mount == "/" {
        path
    } else {
        &path[mount.len().min(path.len())..]
    }
}

/// Virtual Filesystem Manager
pub struct Vfs {
    /// Root filesystem
    root_fs: Option<Arc<dyn Filesystem>>,

    /// Mount points
    mounts: BTreeMap<String, Arc<dyn Filesystem>>,

    /// Legacy global working directory (fallback only).
    /// Per-process CWD is tracked in `process::cwd::ProcessCwd` and
    /// `process::thread::ThreadFs`.  This field is retained for
    /// kernel-context operations where no process is running.
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

        let path = normalize_path(&path, "/");
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

    /// Replace the root filesystem (used for persistent BlockFS mount at boot).
    ///
    /// The previous root filesystem (if any) is dropped. Mount points under
    /// `/dev` and `/proc` should be re-mounted after calling this.
    pub fn swap_root(&mut self, fs: Arc<dyn Filesystem>) {
        self.root_fs = Some(fs);
    }

    /// Unmount a filesystem at the specified path
    pub fn unmount(&mut self, path: &str) -> Result<(), KernelError> {
        self.mounts
            .remove(&normalize_path(path, "/"))
            .ok_or(KernelError::FsError(crate::error::FsError::NotMounted))
            .map(|_| ())
    }

    /// Resolve a path to a VFS node, following symlinks (including the
    /// final component).
    pub fn resolve_path(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.resolve_path_inner(path, &self.cwd, true, 0)
    }

    /// Resolve a path to a VFS node without following the final symlink
    /// component. Intermediate symlinks are still followed. Used by
    /// `lstat()` and `readlink()`.
    pub fn resolve_path_no_follow(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.resolve_path_inner(path, &self.cwd, false, 0)
    }

    /// Resolve a path to a VFS node using an explicit cwd (per-thread FS
    /// state).
    pub fn resolve_from(&self, path: &str, cwd: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.resolve_path_inner(path, cwd, true, 0)
    }

    /// Resolve a path to a VFS node using an explicit cwd, without
    /// following the final symlink component.
    pub fn resolve_from_no_follow(
        &self,
        path: &str,
        cwd: &str,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.resolve_path_inner(path, cwd, false, 0)
    }

    /// The mount point (normalized path) that serves `path`.
    pub fn mount_point_of(&self, path: &str) -> String {
        let path = normalize_path(path, &self.cwd);
        self.mounts
            .keys()
            .filter(|m| path_is_under(&path, m))
            .max_by_key(|m| m.len())
            .cloned()
            .unwrap_or_else(|| String::from("/"))
    }

    /// Resolve a path, returning the node together with its canonical
    /// absolute path (all `.`, `..` and followed symlinks removed). Access
    /// checks must use this path: checking the path as written lets a
    /// symlink or a `..` reach an object the policy meant to protect.
    pub fn resolve_canonical(
        &self,
        path: &str,
        cwd: &str,
        follow_last: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.resolve_inner(path, cwd, follow_last, 0)
    }

    /// Inner path resolution with configurable symlink behavior.
    ///
    /// - `follow_last`: if `true`, a symlink at the final component is
    ///   resolved. If `false`, the symlink node itself is returned.
    /// - `symlink_depth`: current nesting depth for loop detection. Returns
    ///   `FsError::SymlinkLoop` when it exceeds `SYMLINK_MAX_DEPTH`.
    fn resolve_path_inner(
        &self,
        path: &str,
        cwd: &str,
        follow_last: bool,
        symlink_depth: usize,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.resolve_inner(path, cwd, follow_last, symlink_depth)
            .map(|(node, _)| node)
    }

    /// Resolution core (FS-SEC-01). The path is made absolute and its `.`
    /// and `..` components are removed lexically *before* the mount table is
    /// consulted, so `..` cannot stay pinned inside a mount, and the mount
    /// is chosen by whole path components (longest match), so `/devices`
    /// is not served by a filesystem mounted at `/dev`.
    fn resolve_inner(
        &self,
        path: &str,
        cwd: &str,
        follow_last: bool,
        symlink_depth: usize,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        if symlink_depth > SYMLINK_MAX_DEPTH {
            return Err(KernelError::FsError(crate::error::FsError::SymlinkLoop));
        }

        let root_fs = self
            .root_fs
            .as_ref()
            .ok_or(KernelError::FsError(crate::error::FsError::NoRootFs))?;

        let path = normalize_path(path, cwd);

        let mount = self
            .mounts
            .iter()
            .filter(|(mount_path, _)| path_is_under(&path, mount_path))
            .max_by_key(|(mount_path, _)| mount_path.len());
        let (mount_path, start) = match mount {
            Some((mount_path, fs)) => (mount_path.as_str(), fs.root()),
            None => ("/", root_fs.root()),
        };

        self.traverse_path(start, mount_path, &path, follow_last, symlink_depth)
    }

    /// Walk the normalized absolute `path` from `node`, the root of the
    /// filesystem mounted at `mount_path`.
    ///
    /// When a component is a symlink that must be followed, its target --
    /// taken relative to the symlink's own directory unless absolute -- is
    /// spliced in front of the remaining components and the result is
    /// resolved from scratch, so mounts and `..` inside the target are
    /// handled exactly like a path the caller wrote.
    fn traverse_path(
        &self,
        mut node: Arc<dyn VfsNode>,
        mount_path: &str,
        path: &str,
        follow_last: bool,
        symlink_depth: usize,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        let relative = path_relative_to_mount(path, mount_path);
        let components: Vec<&str> = relative.split('/').filter(|s| !s.is_empty()).collect();
        let last_idx = components.len().saturating_sub(1);

        for (idx, component) in components.iter().enumerate() {
            node = node.lookup(component)?;

            let is_last = idx == last_idx;
            if node.node_type() == NodeType::Symlink && (!is_last || follow_last) {
                let target = node.readlink()?;

                // Absolute path of the directory holding the symlink.
                let mut link_dir = String::from(mount_path);
                for c in &components[..idx] {
                    if !link_dir.ends_with('/') {
                        link_dir.push('/');
                    }
                    link_dir.push_str(c);
                }

                let mut spliced = if target.starts_with('/') {
                    target
                } else {
                    format!("{}/{}", link_dir, target)
                };
                for c in &components[idx + 1..] {
                    spliced.push('/');
                    spliced.push_str(c);
                }
                return self.resolve_inner(&spliced, "/", follow_last, symlink_depth + 1);
            }
        }

        Ok((node, String::from(path)))
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

        // MAC is checked on the canonical path, after `..` and symlinks are
        // resolved, never on the path as written (FS-SEC-01).
        let (node, canonical) = self.resolve_canonical(path, &self.cwd, true)?;
        crate::security::mac::check_file_access(&canonical, access, pid)?;
        Ok(node)
    }

    /// Create a directory
    ///
    /// Checks MAC policy (Write access to file domain) before creating.
    pub fn mkdir(&self, path: &str, permissions: Permissions) -> Result<(), KernelError> {
        // Parse the path once and use that single result both for the MAC
        // check and for the creation, so they cannot disagree: the parent
        // is resolved canonically (following symlinks) and the policy sees
        // the path the directory is really created at. Normalizing also
        // removes trailing slashes, `.` and `..`, so the final name is
        // never one of those.
        let path = normalize_path(path, &self.cwd);
        if path == "/" {
            return Err(KernelError::FsError(crate::error::FsError::AlreadyExists));
        }
        let pos = path.rfind('/').unwrap_or(0);
        let (parent_path, name) = (&path[..pos.max(1)], &path[pos + 1..]);

        let (parent, canonical_parent) = self.resolve_canonical(parent_path, "/", true)?;
        let target = if canonical_parent == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", canonical_parent, name)
        };

        // MAC check: creating a directory requires Write access
        let pid = crate::process::current_process()
            .map(|p| p.pid.0)
            .unwrap_or(0);
        crate::security::mac::check_file_access(&target, crate::security::AccessType::Write, pid)?;

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

    /// List all mount points and their filesystem types.
    ///
    /// Returns a vector of `(path, fs_name, readonly)` tuples.
    pub fn list_mounts(&self) -> Vec<(String, String, bool)> {
        let mut result = Vec::new();

        // Root filesystem
        if let Some(ref root) = self.root_fs {
            result.push((
                String::from("/"),
                String::from(root.name()),
                root.is_readonly(),
            ));
        }

        // Mounted filesystems
        for (path, fs) in &self.mounts {
            result.push((path.clone(), String::from(fs.name()), fs.is_readonly()));
        }

        result
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
                root.mkdir("run", Permissions::default()).ok();
                root.mkdir("tmp", Permissions::from_mode(0o1777)).ok();
                root.mkdir("usr", Permissions::default()).ok();
                root.mkdir("var", Permissions::default()).ok();
            }
        }

        println!("[VFS] Created standard directories");

        // Create standard subdirectories
        {
            let vfs = get_vfs();
            let vfs_guard = vfs.read();
            if let Some(ref root_fs) = vfs_guard.root_fs {
                let root = root_fs.root();

                // /usr subdirectories
                if let Ok(usr) = root.lookup("usr") {
                    usr.mkdir("bin", Permissions::default()).ok();
                    usr.mkdir("sbin", Permissions::default()).ok();
                    usr.mkdir("lib", Permissions::default()).ok();
                    usr.mkdir("share", Permissions::default()).ok();
                    usr.mkdir("local", Permissions::default()).ok();
                }

                // /var subdirectories
                if let Ok(var) = root.lookup("var") {
                    var.mkdir("log", Permissions::default()).ok();
                    var.mkdir("tmp", Permissions::default()).ok();
                    var.mkdir("run", Permissions::default()).ok();
                    var.mkdir("cache", Permissions::default()).ok();
                    // /var/lib/dbus/machine-id (D-Bus machine identifier)
                    // Some libraries check this path before /etc/machine-id.
                    if let Ok(lib) = var.mkdir("lib", Permissions::default()) {
                        if let Ok(dbus) = lib.mkdir("dbus", Permissions::default()) {
                            if let Ok(f) = dbus.create("machine-id", Permissions::read_only()) {
                                f.write(0, b"a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6\n").ok();
                            }
                        }
                    }
                }

                // /run subdirectories (XDG_RUNTIME_DIR, D-Bus sockets)
                if let Ok(run) = root.lookup("run") {
                    // /run/user/0 -- XDG_RUNTIME_DIR for root user (KWin, Wayland)
                    if let Ok(user) = run.mkdir("user", Permissions::default()) {
                        user.mkdir("0", Permissions::from_mode(0o700)).ok();
                    }
                    // /run/dbus -- D-Bus system bus socket directory
                    run.mkdir("dbus", Permissions::default()).ok();
                }

                // /home/root (root user home directory)
                if let Ok(home) = root.lookup("home") {
                    home.mkdir("root", Permissions::default()).ok();
                }
            }
        }

        // Populate /etc with basic configuration files
        {
            let vfs = get_vfs();
            let vfs_guard = vfs.read();
            if let Some(ref root_fs) = vfs_guard.root_fs {
                let root = root_fs.root();
                if let Ok(etc) = root.lookup("etc") {
                    // /etc/hostname
                    if let Ok(f) = etc.create("hostname", Permissions::default()) {
                        f.write(0, b"veridian\n").ok();
                    }

                    // /etc/os-release
                    if let Ok(f) = etc.create("os-release", Permissions::default()) {
                        f.write(
                            0,
                            b"NAME=\"VeridianOS\"\nVERSION=\"0.25.2\"\nID=veridian\nPRETTY_NAME=\"VeridianOS v0.25.2\"\n",
                        )
                        .ok();
                    }

                    // /etc/passwd (minimal)
                    if let Ok(f) = etc.create("passwd", Permissions::read_only()) {
                        f.write(0, b"root:x:0:0:root:/root:/bin/vsh\n").ok();
                    }

                    // /etc/group (minimal)
                    if let Ok(f) = etc.create("group", Permissions::read_only()) {
                        f.write(0, b"root:x:0:root\n").ok();
                    }

                    // /etc/shells
                    if let Ok(f) = etc.create("shells", Permissions::read_only()) {
                        f.write(0, b"/bin/vsh\n").ok();
                    }

                    // /etc/motd (message of the day)
                    if let Ok(f) = etc.create("motd", Permissions::read_only()) {
                        f.write(
                            0,
                            b"Welcome to VeridianOS - a capability-based microkernel OS\n",
                        )
                        .ok();
                    }

                    // /etc/machine-id (D-Bus/systemd machine identifier, 32 hex + newline)
                    // Required by Qt/KDE/D-Bus for session tracking. Without this,
                    // kwin_wayland crashes with a page fault when reading machine-id.
                    if let Ok(f) = etc.create("machine-id", Permissions::read_only()) {
                        f.write(0, b"a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6\n").ok();
                    }

                    // /etc/veridian/session.conf (default desktop session config)
                    if let Ok(veridian_dir) = etc.mkdir("veridian", Permissions::default()) {
                        if let Ok(f) = veridian_dir.create("session.conf", Permissions::default()) {
                            f.write(
                                0,
                                b"# VeridianOS session configuration\n# session_type: plasma (KDE Plasma 6) or builtin (built-in DE)\nsession_type=plasma\n",
                            )
                            .ok();
                        }
                    }
                }
            }
        }

        println!("[VFS] Populated /etc and subdirectories");

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

        // Create /proc/sys/kernel/ hierarchy for KDE/Qt compatibility.
        // Qt's KCrash module reads /proc/sys/kernel/core_pattern to decide
        // whether to install a crash handler. Without it, KCrash enters an
        // infinite retry loop that prevents kwin from starting.
        {
            let vfs = get_vfs();
            let vfs_guard = vfs.read();
            // ProcFS is mounted at /proc. Access its root node through
            // the mounts table.
            if let Some(proc_fs) = vfs_guard.mounts.get("/proc") {
                let proc_root = proc_fs.root();
                // /proc/sys/kernel/core_pattern
                if let Ok(sys_dir) = proc_root.mkdir("sys", Permissions::default()) {
                    if let Ok(kernel_dir) = sys_dir.mkdir("kernel", Permissions::default()) {
                        if let Ok(f) = kernel_dir.create("core_pattern", Permissions::default()) {
                            f.write(0, b"core\n").ok();
                        }
                        // /proc/sys/kernel/random/boot_id (UUID for Qt sessions)
                        if let Ok(random_dir) = kernel_dir.mkdir("random", Permissions::default()) {
                            if let Ok(f) = random_dir.create("boot_id", Permissions::default()) {
                                f.write(0, b"00000000-0000-0000-0000-000000000001\n").ok();
                            }
                        }
                    }
                }
                // /proc/self/exe (Qt's applicationFilePath())
                if let Ok(self_dir) = proc_root.mkdir("self", Permissions::default()) {
                    if let Ok(f) = self_dir.create("exe", Permissions::default()) {
                        f.write(0, b"").ok();
                    }
                    // /proc/self/maps (Qt crash handler)
                    if let Ok(f) = self_dir.create("maps", Permissions::default()) {
                        f.write(0, b"").ok();
                    }
                }
            }
        }
        println!("[VFS] Created /proc/sys/kernel/ stubs for Qt/KDE");

        println!("[VFS] Virtual Filesystem initialization complete");
    }

    #[cfg(not(feature = "alloc"))]
    {
        println!("[VFS] Skipping VFS initialization (no alloc)");
    }
}

/// Re-create /proc/sys/kernel/ stubs after a root filesystem swap.
///
/// When BlockFS replaces the initial RamFS root, the old ProcFS mount is
/// destroyed and a fresh one is created. This function re-populates the
/// Qt/KDE-required files in /proc that were originally created in [`init`].
#[cfg(feature = "alloc")]
pub fn recreate_proc_stubs() {
    let vfs = get_vfs();
    let vfs_guard = vfs.read();
    if let Some(proc_fs) = vfs_guard.mounts.get("/proc") {
        let proc_root = proc_fs.root();
        // /proc/sys/kernel/core_pattern
        if let Ok(sys_dir) = proc_root
            .lookup("sys")
            .or_else(|_| proc_root.mkdir("sys", Permissions::default()))
        {
            if let Ok(kernel_dir) = sys_dir
                .lookup("kernel")
                .or_else(|_| sys_dir.mkdir("kernel", Permissions::default()))
            {
                if kernel_dir.lookup("core_pattern").is_err() {
                    if let Ok(f) = kernel_dir.create("core_pattern", Permissions::default()) {
                        f.write(0, b"core\n").ok();
                    }
                }
                // /proc/sys/kernel/random/boot_id
                if let Ok(random_dir) = kernel_dir
                    .lookup("random")
                    .or_else(|_| kernel_dir.mkdir("random", Permissions::default()))
                {
                    if random_dir.lookup("boot_id").is_err() {
                        if let Ok(f) = random_dir.create("boot_id", Permissions::default()) {
                            f.write(0, b"00000000-0000-0000-0000-000000000001\n").ok();
                        }
                    }
                }
            }
        }
        // /proc/self/exe and /proc/self/maps
        if let Ok(self_dir) = proc_root
            .lookup("self")
            .or_else(|_| proc_root.mkdir("self", Permissions::default()))
        {
            if self_dir.lookup("exe").is_err() {
                if let Ok(f) = self_dir.create("exe", Permissions::default()) {
                    f.write(0, b"").ok();
                }
            }
            if self_dir.lookup("maps").is_err() {
                if let Ok(f) = self_dir.create("maps", Permissions::default()) {
                    f.write(0, b"").ok();
                }
            }
        }
    }
    println!("[VFS] Re-created /proc/sys/kernel/ stubs for Qt/KDE");
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

    #[test]
    fn permissions_mode_round_trip_includes_sticky() {
        for mode in [0o1777, 0o755, 0o1700, 0o644, 0o0] {
            assert_eq!(Permissions::from_mode(mode).to_mode(), mode);
        }
    }

    /// Helper: create a Vfs with a ramfs root filesystem already mounted.
    fn make_vfs_with_root() -> Vfs {
        let mut vfs = Vfs::new();
        let ramfs = Arc::new(ramfs::RamFs::new());
        vfs.mount_root(ramfs).expect("mount_root should succeed");
        vfs
    }

    // --- Path resolution (FS-SEC-01) ---

    #[test]
    fn normalize_path_removes_dots() {
        assert_eq!(normalize_path("/a/./b/../c/", "/"), "/a/c");
        assert_eq!(normalize_path("../../x", "/a"), "/x");
        assert_eq!(normalize_path("..", "/"), "/");
        assert_eq!(normalize_path("b", "/a/"), "/a/b");
        assert_eq!(normalize_path("", "/"), "/");
    }

    #[test]
    fn mount_match_respects_component_boundary() {
        assert!(path_is_under("/dev", "/dev"));
        assert!(path_is_under("/dev/null", "/dev"));
        assert!(!path_is_under("/devices", "/dev"));
        assert!(path_is_under("/anything", "/"));
    }

    #[test]
    fn sibling_with_mount_prefix_is_not_hijacked() {
        // Before the fix "/devices" matched the "/dev" mount by prefix.
        let mut vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.mkdir("dev", Permissions::default()).unwrap();
        let devices = root.mkdir("devices", Permissions::default()).unwrap();
        devices.create("marker", Permissions::default()).unwrap();
        vfs.mount(String::from("/dev"), Arc::new(ramfs::RamFs::new()))
            .unwrap();
        assert!(vfs.resolve_path("/devices/marker").is_ok());
        assert!(vfs.resolve_path("/dev/marker").is_err());
    }

    #[test]
    fn dotdot_leaves_a_mount() {
        // Before the fix ".." at a mount root stayed inside the mount.
        let mut vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.mkdir("mnt", Permissions::default()).unwrap();
        root.create("top", Permissions::default()).unwrap();
        vfs.mount(String::from("/mnt"), Arc::new(ramfs::RamFs::new()))
            .unwrap();
        assert!(vfs.resolve_path("/mnt/../top").is_ok());
    }

    #[test]
    fn relative_symlink_resolves_from_its_directory() {
        // Before the fix a relative target was resolved from "/".
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        a.create("target", Permissions::default()).unwrap();
        a.symlink("link", "target").unwrap();
        let (_, canonical) = vfs.resolve_canonical("/a/link", "/", true).unwrap();
        assert_eq!(canonical, "/a/target");
    }

    #[test]
    fn canonical_path_follows_intermediate_symlink() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        let real = root.mkdir("real", Permissions::default()).unwrap();
        real.create("f", Permissions::default()).unwrap();
        root.symlink("alias", "/real").unwrap();
        let (_, canonical) = vfs.resolve_canonical("/alias/f", "/", true).unwrap();
        assert_eq!(canonical, "/real/f");
        // Not following the last component returns the link itself.
        let (node, canonical) = vfs.resolve_canonical("/alias", "/", false).unwrap();
        assert_eq!(node.node_type(), NodeType::Symlink);
        assert_eq!(canonical, "/alias");
    }

    #[test]
    fn mkdir_parses_path_once() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.mkdir("real", Permissions::default()).unwrap();
        root.symlink("alias", "/real").unwrap();
        // Created where the parent really resolves, which is the path the
        // MAC check now sees.
        vfs.mkdir("/alias/sub/", Permissions::default()).unwrap();
        assert!(vfs.resolve_path("/real/sub").is_ok());
        // ".." is resolved, never created as an entry name.
        assert!(vfs.mkdir("/real/..", Permissions::default()).is_err());
    }

    #[test]
    fn symlink_loop_is_reported() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs.as_ref().unwrap().root();
        root.symlink("loop", "/loop").unwrap();
        assert!(vfs.resolve_path("/loop").is_err());
    }

    // --- chown (FS-SEC-02) ---

    #[test]
    fn ramfs_chown_updates_owner_and_keeps_unset_ids() {
        let fs = ramfs::RamFs::new();
        let file = fs.root().create("f", Permissions::default()).unwrap();
        file.chown(Some(1000), None).unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!((meta.uid, meta.gid), (1000, 0));
        file.chown(None, Some(50)).unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!((meta.uid, meta.gid), (1000, 50));
    }

    #[test]
    fn blockfs_chown_rejects_ids_wider_than_disk_format() {
        let fs = blockfs::BlockFs::format(1000, 100).unwrap();
        let file = fs.root().create("f", Permissions::default()).unwrap();
        assert!(file.chown(Some(70_000), None).is_err());
        file.chown(Some(1000), Some(100)).unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!((meta.uid, meta.gid), (1000, 100));
    }

    #[test]
    fn permissions_mode_round_trip() {
        for mode in [0o000, 0o600, 0o640, 0o755, 0o777, 0o421] {
            assert_eq!(Permissions::from_mode(mode).to_mode(), mode);
        }
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
        assert_eq!(
            result.err().expect("expected error"),
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
    fn test_mkdir_relative_path_uses_cwd() {
        // A relative path is resolved against the cwd like everywhere else
        // in the VFS (it used to be rejected as invalid).
        let vfs = make_vfs_with_root();
        vfs.mkdir("no_slash", Permissions::default()).unwrap();
        assert!(vfs.resolve_path("/no_slash").is_ok());
        assert!(vfs.mkdir("/", Permissions::default()).is_err());
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

//! Virtual Filesystem (VFS) Layer
//!
//! Provides a unified interface for different filesystem implementations.

#![allow(clippy::should_implement_trait)]

use alloc::{collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use spin::RwLock;

use crate::error::{FsError, KernelError};

pub mod blockdev;
pub mod blockfs;
pub mod devfs;
pub mod eventfd;
pub mod ext4;
pub mod falloc;
pub mod fat32;
pub mod fifo;
pub mod file;
pub mod flock;
pub mod inotify;
#[cfg(feature = "alloc")]
pub mod memfd;
pub mod mount;
#[cfg(feature = "alloc")]
pub mod nsfs;
#[cfg(feature = "alloc")]
pub mod pidfd;
pub mod pipe;
pub mod procfs;
pub mod pty;
pub mod ramfs;
#[cfg(feature = "alloc")]
pub mod shmem;
pub mod signalfd;
pub mod statfs;
pub mod tar;
pub mod timerfd;
#[cfg(feature = "alloc")]
pub mod tmpfs;
pub mod write_access;
pub mod xattr;

// Phase 8 Wave 3: Enterprise Storage
pub mod nfs;
pub mod smb;

pub use file::{File, FileDescriptor, FileTable, OpenFlags, SeekFrom};

/// Largest file a RAM-backed filesystem (ramfs, tmpfs) will hold. Without a
/// bound, one write at a huge offset made the kernel allocate the whole
/// range and abort (N-122).
pub const MAX_RAM_FILE_SIZE: usize = 1 << 30;

/// The end of a write of `len` bytes at `offset`, if the file may grow to it.
pub(crate) fn ram_write_end(offset: usize, len: usize) -> Result<usize, KernelError> {
    offset
        .checked_add(len)
        .filter(|&end| end <= MAX_RAM_FILE_SIZE)
        .ok_or(KernelError::FsError(FsError::FileTooLarge))
}

/// Grow `data` to `new_len` zero-filled bytes, failing (ENOSPC) instead of
/// aborting when the heap cannot provide them.
pub(crate) fn grow_ram_file(data: &mut Vec<u8>, new_len: usize) -> Result<(), KernelError> {
    if new_len > MAX_RAM_FILE_SIZE {
        return Err(KernelError::FsError(FsError::FileTooLarge));
    }
    if new_len > data.len() {
        data.try_reserve_exact(new_len - data.len())
            .map_err(|_| KernelError::FsError(FsError::NoSpace))?;
        data.resize(new_len, 0);
    }
    Ok(())
}

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

/// Serializes renames, as Linux's per-filesystem rename mutex does. Rename
/// is the only operation that changes which directory contains which, so
/// while this is held (a) the "not into its own subtree" check stays true
/// until the move is done, and (b) no two renames can hold directory locks
/// in conflicting orders. Every other operation takes directory locks only
/// parent-before-child. Callers of `VfsNode::rename` must hold it.
pub(crate) static RENAME_LOCK: spin::Mutex<()> = spin::Mutex::new(());

/// How much of a `len`-byte write at `pos` a file-size limit of `limit`
/// bytes allows (Linux's generic_write_check_limits): `None` if the write
/// starts at or past the limit (EFBIG), otherwise the bytes up to it.
pub fn fsize_allowed(limit: u64, pos: usize, len: usize) -> Option<usize> {
    let room = limit.checked_sub(pos as u64).filter(|&r| r > 0)?;
    Some(len.min(room.min(usize::MAX as u64) as usize))
}

/// RLIMIT_FSIZE (N-224) for the calling process writing `len` bytes at
/// `pos` of `node`: the bytes it may write. Only regular files are
/// limited. A write that would start at or past the limit sends the caller
/// SIGXFSZ and fails with EFBIG; one that crosses it is shortened. A
/// zero-length write is never refused.
pub fn check_fsize(node: &dyn VfsNode, pos: usize, len: usize) -> Result<usize, KernelError> {
    if len == 0 || node.node_type() != NodeType::File {
        return Ok(len);
    }
    let Some(process) = crate::process::current_process() else {
        return Ok(len);
    };
    let limit = process.limits().cur(crate::process::rlimit::RLIMIT_FSIZE);
    fsize_allowed(limit, pos, len).ok_or_else(|| {
        crate::process::signals::send_to_current(crate::process::exit::signals::SIGXFSZ as usize);
        KernelError::FsError(FsError::FileTooLarge)
    })
}

/// RLIMIT_FSIZE for setting the size of `node` to `size` (truncate,
/// ftruncate; Linux's inode_newsize_ok): growing a regular file past the
/// limit sends SIGXFSZ and fails with EFBIG.
pub fn check_fsize_truncate(node: &dyn VfsNode, size: usize) -> Result<(), KernelError> {
    if node.node_type() != NodeType::File || node.metadata().is_ok_and(|m| size <= m.size) {
        return Ok(());
    }
    let Some(process) = crate::process::current_process() else {
        return Ok(());
    };
    if size as u64 > process.limits().cur(crate::process::rlimit::RLIMIT_FSIZE) {
        crate::process::signals::send_to_current(crate::process::exit::signals::SIGXFSZ as usize);
        return Err(KernelError::FsError(FsError::FileTooLarge));
    }
    Ok(())
}

/// Before a write or truncate of `node` by the calling process (Linux's
/// file_remove_privs): drop its file capabilities ([`kill_file_caps`]) and
/// the set-user-ID and set-group-ID bits as [`Permissions::after_write`]
/// says. Regular files only. As on Linux it runs before the data changes
/// and its error is the write's, so a file that keeps its privileges is
/// never changed (PR #33 review).
pub fn remove_privs(node: &dyn VfsNode) -> Result<(), KernelError> {
    if node.node_type() != NodeType::File {
        return Ok(());
    }
    kill_file_caps(node)?;
    drop_set_id(node, &crate::process::creds::Caller::current())
}

/// Set the size of `node` to `size` for the calling process, removing its
/// privileges as Linux's notify_change does: the file capabilities go
/// first (setattr_prepare), and the set-ID bits only once the size has
/// changed (setattr_copy), so a refused truncate, such as one a memfd seal
/// forbids, leaves the mode as it was (PR #33 review).
pub fn truncate_removing_privs(node: &dyn VfsNode, size: usize) -> Result<(), KernelError> {
    truncate_removing_privs_as(node, size, &crate::process::creds::Caller::current())
}

fn truncate_removing_privs_as(
    node: &dyn VfsNode,
    size: usize,
    caller: &crate::process::creds::Caller,
) -> Result<(), KernelError> {
    if node.node_type() != NodeType::File {
        return node.truncate(size);
    }
    kill_file_caps(node)?;
    node.truncate(size)?;
    drop_set_id(node, caller)
}

/// The set-user-ID and set-group-ID bits a write or truncate by `caller`
/// removes ([`Permissions::after_write`]). The kernel itself (no process)
/// holds CAP_FSETID. Keeping either bit takes CAP_FSETID in the initial
/// user namespace; a set-group-ID bit without group execute also stays
/// for a caller in the file's group or holding CAP_FSETID over the file
/// (setattr_should_drop_sgid).
fn drop_set_id(
    node: &dyn VfsNode,
    caller: &crate::process::creds::Caller,
) -> Result<(), KernelError> {
    let meta = node.metadata()?;
    let fsetid = caller.capable(crate::process::linux_caps::CAP_FSETID);
    if let Some(kept) = meta
        .permissions
        .after_write(fsetid, caller.in_group_or_capable(meta.uid, meta.gid))
    {
        node.chmod(kept)?;
    }
    Ok(())
}

/// Remove a file's capabilities (Linux's cap_inode_killpriv), as a write,
/// truncate or chown does whoever makes it: a changed program must not
/// keep the privilege granted to the old one. Only read when present; a
/// filesystem without attributes has none to remove. Any other read error
/// is the caller's, since the capability may still be there (PR #33 review).
pub fn kill_file_caps(node: &dyn VfsNode) -> Result<(), KernelError> {
    let has = match node.xattrs() {
        Ok(a) => a.get(xattr::XATTR_NAME_CAPS).is_some(),
        Err(KernelError::FsError(FsError::NotSupported)) => false,
        Err(e) => return Err(e),
    };
    if has {
        // Another writer may remove it between the check and the update:
        // gone is the outcome wanted, not an error.
        node.update_xattrs(&mut |a| match a.remove(xattr::XATTR_NAME_CAPS) {
            Err(KernelError::FsError(FsError::NoData)) => Ok(()),
            r => r,
        })?;
    }
    Ok(())
}

/// Names that can never be a rename source or target.
pub(crate) fn is_special_name(name: &str) -> bool {
    name.is_empty() || name == "." || name == ".."
}

/// File permissions (Unix-style)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Set-user-ID bit (S_ISUID): exec runs the program with its owner as
    /// the effective user.
    pub set_uid: bool,
    /// Set-group-ID bit (S_ISGID): exec runs the program with its group as
    /// the effective group (with group execute permission; without it the
    /// bit marks mandatory locking, which Linux no longer honours).
    pub set_gid: bool,
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
            set_uid: false,
            set_gid: false,
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
            set_uid: false,
            set_gid: false,
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
            (self.set_gid, 0o2000),
            (self.set_uid, 0o4000),
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
            set_gid: (mode & 0o2000) != 0,
            set_uid: (mode & 0o4000) != 0,
        }
    }

    /// Linux's file_remove_privs: the permissions a file keeps after a
    /// write or truncate (`fsetid`: the writer holds CAP_FSETID;
    /// `in_group`: it belongs to the file's group). With CAP_FSETID the
    /// bits stay; otherwise set-user-ID goes, and set-group-ID where it
    /// means set-group-ID (group execute set) or the writer is outside the
    /// group. `None` when nothing changes.
    pub fn after_write(&self, fsetid: bool, in_group: bool) -> Option<Self> {
        if fsetid {
            return None;
        }
        let mut kept = *self;
        kept.set_uid = false;
        if self.group_exec || !in_group {
            kept.set_gid = false;
        }
        (kept != *self).then_some(kept)
    }

    /// The permissions a non-directory keeps when its owner or group
    /// changes (Linux's chown, whoever calls it): no set-user-ID, and no
    /// set-group-ID with group execute (without it the bit marks mandatory
    /// locking and stays). `None` when nothing changes.
    pub fn after_chown(&self) -> Option<Self> {
        let mut kept = *self;
        kept.set_uid = false;
        if self.group_exec {
            kept.set_gid = false;
        }
        (kept != *self).then_some(kept)
    }

    /// The access bits (MAY_READ, MAY_WRITE, MAY_EXEC) of one class: the
    /// owner's, the group's, or everyone else's.
    fn class_bits(&self, owner: bool, group: bool) -> u32 {
        let (r, w, x) = if owner {
            (self.owner_read, self.owner_write, self.owner_exec)
        } else if group {
            (self.group_read, self.group_write, self.group_exec)
        } else {
            (self.other_read, self.other_write, self.other_exec)
        };
        ((r as u32) * MAY_READ) | ((w as u32) * MAY_WRITE) | ((x as u32) * MAY_EXEC)
    }

    /// Whether any class may execute.
    pub fn any_exec(&self) -> bool {
        self.owner_exec || self.group_exec || self.other_exec
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
    /// The status-change time (`st_ctime`): every filesystem keeps its
    /// ctime here; none records a birth time.
    pub created: u64,
    pub modified: u64,
    pub accessed: u64,
    pub inode: u64,
}

/// Access asked of a node, as Linux's MAY_* bits (the same values as
/// access(2)'s R_OK, W_OK and X_OK).
pub const MAY_EXEC: u32 = 1;
pub const MAY_WRITE: u32 = 2;
pub const MAY_READ: u32 = 4;

/// Whether `caller` may access a node with `meta` as `mask` asks (MAY_*
/// bits), as Linux's generic_permission (ADR 0020): the mode bits of the
/// first class that applies -- owner, then group (any of the caller's
/// groups), then other -- and then the capabilities that override them. On
/// a directory CAP_DAC_READ_SEARCH grants anything but writing and
/// CAP_DAC_OVERRIDE everything; on anything else CAP_DAC_READ_SEARCH grants
/// reading alone, and CAP_DAC_OVERRIDE reading and writing, and executing
/// only where some execute bit is set. The capabilities count in the
/// caller's own user namespace, for a node whose owner and group it maps
/// (capable_wrt_inode_uidgid).
pub fn permission(caller: &crate::process::creds::Caller, meta: &Metadata, mask: u32) -> bool {
    use crate::process::linux_caps::{CAP_DAC_OVERRIDE, CAP_DAC_READ_SEARCH};
    let c = &caller.creds;
    let p = &meta.permissions;
    let owner = c.fsuid == meta.uid;
    let granted = p.class_bits(owner, !owner && c.in_group(meta.gid));
    if mask & !granted == 0 {
        return true;
    }
    let capable = |cap| caller.capable_wrt_inode(meta.uid, meta.gid, cap);
    if meta.node_type == NodeType::Directory {
        return (mask & MAY_WRITE == 0 && capable(CAP_DAC_READ_SEARCH))
            || capable(CAP_DAC_OVERRIDE);
    }
    if mask == MAY_READ && capable(CAP_DAC_READ_SEARCH) {
        return true;
    }
    (mask & MAY_EXEC == 0 || p.any_exec()) && capable(CAP_DAC_OVERRIDE)
}

/// Directory entry
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub node_type: NodeType,
    pub inode: u64,
}

/// Readiness bits as Linux reports them: POLLIN (1) comes with POLLRDNORM
/// (0x40) and POLLOUT (4) with POLLWRNORM (0x100), as Linux's pipes, sockets
/// and regular files set them, whichever of a pair a node reports.
pub fn normalize_poll_bits(bits: u16) -> u16 {
    const IN: u16 = 0x001 | 0x040;
    const OUT: u16 = 0x004 | 0x100;
    let mut bits = bits;
    if bits & IN != 0 {
        bits |= IN;
    }
    if bits & OUT != 0 {
        bits |= OUT;
    }
    bits
}

/// A node's readiness for poll, select and epoll ([`normalize_poll_bits`]
/// of [`VfsNode::poll_readiness`]).
pub fn poll_bits(node: &dyn VfsNode) -> u16 {
    normalize_poll_bits(node.poll_readiness())
}

/// VFS node operations trait
pub trait VfsNode: Send + Sync {
    /// Node type query (also serves as vtable slot padding for AArch64)
    fn node_type(&self) -> NodeType;

    /// Read data from the node
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError>;

    /// Write data to the node
    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError>;

    /// [`Self::read`] through a descriptor opened with `opener`'s
    /// credentials (None if the file does not record them), for the files
    /// that show IDs as the opener's user namespace sees them (Linux's
    /// seq_user_ns): procfs's status and ID maps.
    fn read_as(
        &self,
        offset: usize,
        buffer: &mut [u8],
        opener: Option<&crate::process::creds::Caller>,
    ) -> Result<usize, KernelError> {
        let _ = opener;
        self.read(offset, buffer)
    }

    /// [`Self::write`] through a descriptor opened with `opener`'s
    /// credentials (Linux's file->f_cred; None if the file does not record
    /// them), for the files whose checks are the opener's: procfs's ID
    /// maps and timens_offsets.
    fn write_as(
        &self,
        offset: usize,
        data: &[u8],
        opener: Option<&crate::process::creds::Caller>,
    ) -> Result<usize, KernelError> {
        let _ = opener;
        self.write(offset, data)
    }

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

    /// Remove `name` only if it is a directory (`want_dir`, rmdir) or only
    /// if it is not (unlink): EISDIR or ENOTDIR otherwise. Filesystems that
    /// can check the entry's type under the lock the removal takes do so;
    /// checked apart, another thread could replace the entry in between,
    /// and unlink(2) removed a directory or rmdir(2) a file (PR #23
    /// review; Linux checks under the parent's lock). This default does
    /// the two steps apart.
    fn unlink_kind(&self, name: &str, want_dir: bool) -> Result<(), KernelError> {
        check_unlink_kind(self.lookup(name)?.node_type(), want_dir)?;
        self.unlink(name)
    }

    /// Truncate the file to the specified size
    fn truncate(&self, size: usize) -> Result<(), KernelError>;

    /// Move entry `old_name` of this directory to `new_name` in
    /// `new_parent` (same filesystem), replacing an existing entry there
    /// under POSIX rules. The node itself moves: nothing is copied, its
    /// owner, mode and inode are kept (FS-PERF-03). The caller must hold
    /// [`RENAME_LOCK`].
    fn rename(
        &self,
        _old_name: &str,
        _new_parent: &Arc<dyn VfsNode>,
        _new_name: &str,
    ) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotSupported))
    }

    /// Create a hard link to this node
    fn link(&self, _name: &str, _target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        Err(KernelError::NotImplemented {
            feature: "hard links",
        })
    }

    /// O_TMPFILE: a new regular file of this directory's filesystem with
    /// no name (link count 0), which [`Self::link`] may name later only if
    /// `linkable` ([`Self::tmpfile_linkable`]); it is freed when the last
    /// reference goes. EOPNOTSUPP by default, as Linux answers for a
    /// filesystem without `tmpfile` (BlockFS here).
    fn tmpfile(
        &self,
        _permissions: Permissions,
        _linkable: bool,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotSupported))
    }

    /// Whether this node, an O_TMPFILE file not yet named, may be linked
    /// although it has no name (Linux's I_LINKABLE): made without O_EXCL
    /// and not linked since.
    fn tmpfile_linkable(&self) -> bool {
        false
    }

    /// A stable identity for the file this node names, `(filesystem,
    /// inode)`, for filesystems that build a new node object on every
    /// lookup (BlockFS). `None` (the default) means node objects are shared,
    /// so the node's address identifies the file.
    fn file_identity(&self) -> Option<(u64, u64)> {
        None
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

    /// Whether this symlink is a procfs "magic link" (`/proc/self/exe`),
    /// which names an object rather than a path: openat2's
    /// RESOLVE_NO_MAGICLINKS refuses to follow one.
    fn is_magic_link(&self) -> bool {
        false
    }

    /// For a magic link that names an object rather than a path
    /// (`/proc/<pid>/ns/*`), that object: the walk continues from it, as
    /// Linux's nd_jump_link. None for any other node.
    fn magic_target(&self) -> Option<Result<Arc<dyn VfsNode>, KernelError>> {
        None
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

    /// Whether this node can store `uid` and `gid` (`None` unchanged),
    /// checked before a chown removes any privileges so a chown the
    /// filesystem would refuse changes nothing (as Linux's notify_change
    /// rejects an unmappable id before the inode changes).
    fn check_owner(&self, _uid: Option<u32>, _gid: Option<u32>) -> Result<(), KernelError> {
        Ok(())
    }

    /// Set the access and/or modification time (seconds); `None` leaves
    /// that time unchanged. Permission checks are the caller's job
    /// (utimensat, N-249).
    fn set_times(&self, _atime: Option<u64>, _mtime: Option<u64>) -> Result<(), KernelError> {
        Err(KernelError::NotImplemented {
            feature: "set_times",
        })
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

    /// FIONREAD on a node that is not a regular file: how many bytes are
    /// queued for reading now (a pipe's contents, a terminal's input as
    /// its line discipline would hand it out). `None`: the node keeps no
    /// such count, and FIONREAD is ENOTTY, as on Linux for a file whose
    /// driver has no FIONREAD.
    fn bytes_readable(&self) -> Option<usize> {
        None
    }

    /// Whether the node is a stream with no file position -- a pipe,
    /// socket, terminal or event object -- so lseek, pread and pwrite on
    /// it are ESPIPE, as for Linux's nonseekable files.
    fn is_stream(&self) -> bool {
        matches!(self.node_type(), NodeType::Pipe | NodeType::Socket)
    }

    /// Which pipe this node is an end of (`None`: it is not a pipe or FIFO
    /// end), so that splice and tee can tell two ends of one pipe from two
    /// pipes.
    fn pipe_id(&self) -> Option<usize> {
        None
    }

    /// Free bytes in the pipe this end writes to, for splice, tee and
    /// vmsplice: `BrokenPipe` once it has no reader.
    fn pipe_room(&self) -> Result<usize, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "pipe_room",
        })
    }

    /// Splice's read of the pipe this end reads: hand up to `max` queued
    /// bytes to `sink` and, with `consume`, remove from the pipe the
    /// number it returns (tee leaves them). No other read of the pipe runs
    /// meanwhile, so what `sink` was given is what goes. An empty pipe is
    /// `Ok(0)` without a writer (end of file) and `WouldBlock` with one;
    /// `sink` is never given nothing.
    fn pipe_splice(
        &self,
        _max: usize,
        _consume: bool,
        _sink: &mut dyn FnMut(&[u8]) -> Result<usize, KernelError>,
    ) -> Result<usize, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "pipe_splice",
        })
    }

    /// The file's seals (fcntl F_GET_SEALS, bits from [`seals`]); `None`
    /// for a file that cannot be sealed -- only memfds can -- which fcntl
    /// reports as EINVAL.
    fn seals(&self) -> Option<u32> {
        None
    }

    /// Add seals (F_ADD_SEALS): EPERM once `seals::SEAL` is set, EINVAL for
    /// a file that cannot be sealed.
    fn add_seals(&self, _seals: u32) -> Result<(), KernelError> {
        Err(KernelError::InvalidArgument {
            name: "seals",
            value: "file cannot be sealed",
        })
    }

    /// The page cache of the file (`mm::page_cache`, ADR 0010): its pages,
    /// shared by every private mapping of the file and every program
    /// segment loaded from it (see `mm::page_cache::file_pages`). `None`:
    /// the filesystem has no page cache, and the pages are copied.
    #[cfg(feature = "alloc")]
    fn page_cache(&self) -> Option<alloc::sync::Arc<crate::mm::page_cache::FileCache>> {
        None
    }

    /// The frames holding pages `[first_page, first_page + count)` of the
    /// file, for a MAP_SHARED mapping that maps the file's own pages, so it
    /// and every other mapping, read and write see one copy (N-230). Each
    /// frame returned has gained an owner (`mm::frame_refs`) that the
    /// mapping releases when unmapped. `writable`: the mapping may write
    /// (refused under F_SEAL_WRITE). `None`: the node has no such pages,
    /// and a MAP_SHARED mapping of it is a private copy (a page cache is
    /// v0.29 work).
    #[cfg(feature = "alloc")]
    fn share_pages(
        &self,
        _first_page: usize,
        _count: usize,
        _writable: bool,
    ) -> Option<Result<Vec<crate::mm::FrameNumber>, KernelError>> {
        None
    }

    /// Whether every readiness change of this node calls
    /// [`crate::sched::dispatch::io_event`]. Waiters on such a node sleep
    /// until woken; on any other they also re-check periodically.
    /// A count of this file's own readiness changes, if it keeps one:
    /// edge-triggered epoll reports a still-ready file again only when its
    /// own count moved (Linux's per-file wakeup callback). Without one, any
    /// I/O event in the system counts as a change (PR #24 review: that
    /// re-fired every edge-triggered registration of every file).
    fn io_generation(&self) -> Option<u64> {
        None
    }

    fn wakes_io_waiters(&self) -> bool {
        false
    }

    /// When this node becomes ready by itself, with no event to wake
    /// waiters (a timerfd's next expiry), as `dispatch::clock_ns`: read,
    /// poll and epoll sleep no longer than that. `None`: only events change
    /// its readiness.
    fn ready_at_ns(&self) -> Option<u64> {
        None
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

    /// Create special file `name` in this directory (mknod): a FIFO
    /// (`NodeType::Pipe`), a character or block device numbered `rdev`
    /// (Linux's encoding, [`dev_major`] and [`dev_minor`]) or a socket.
    /// Default: this filesystem cannot hold one, which mknod reports as
    /// EPERM, as Linux does for a filesystem without `mknod`.
    fn mknod(
        &self,
        _name: &str,
        _kind: NodeType,
        _permissions: Permissions,
        _rdev: u32,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotSupported))
    }

    /// Whether this is a special file kept by a filesystem (made by
    /// mknod), which open replaces with what it names: a FIFO's pipe
    /// (`fifo`), a device (`devfs::open_device`). A pipe, device or socket
    /// object itself is not one.
    fn is_special_inode(&self) -> bool {
        false
    }

    /// The node's extended attributes (`fs::xattr`; ADR 0020). EOPNOTSUPP
    /// on a filesystem that keeps none.
    fn xattrs(&self) -> Result<xattr::XattrSet, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotSupported))
    }

    /// Change the node's extended attributes with `f`, as one step: when
    /// `f` or the filesystem fails (ENOSPC when they no longer fit),
    /// nothing changes. EOPNOTSUPP on a filesystem that keeps none.
    fn update_xattrs(
        &self,
        _f: &mut dyn FnMut(&mut xattr::XattrSet) -> Result<(), KernelError>,
    ) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotSupported))
    }

    /// What statfs and fstatfs report about the filesystem holding this
    /// node. ENOSYS by default, as Linux answers for a filesystem without
    /// `statfs` in its superblock operations.
    fn statfs(&self) -> Result<statfs::FsStats, KernelError> {
        Err(KernelError::NotImplemented { feature: "statfs" })
    }

    /// fallocate(2) on this regular file, after the VFS checks
    /// (`falloc::Request::parse`). EOPNOTSUPP by default, as Linux answers
    /// for a filesystem without `fallocate` in its file operations (ramfs);
    /// a filesystem that has one answers EOPNOTSUPP for the operations it
    /// does not do.
    fn fallocate(&self, _request: falloc::Request) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotSupported))
    }

    /// How many names the node has (`st_nlink`, Linux's i_nlink): for a
    /// directory 2 and one per subdirectory, 0 once it is removed. The
    /// default, for a node outside a tree that counts them, is 1, or 2 for
    /// a directory.
    fn link_count(&self) -> u32 {
        if self.node_type() == NodeType::Directory {
            2
        } else {
            1
        }
    }
}

/// Record a change of the node with metadata `meta` now: the status-change
/// time always, and with `contents` the modification time too (a write, a
/// truncate, an entry added to or removed from a directory), as Linux's
/// inode_set_ctime_current and inode_set_mtime_to_ts.
pub(crate) fn note_change(meta: &mut Metadata, contents: bool) {
    let now = crate::arch::timer::get_timestamp_secs();
    if contents {
        meta.modified = now;
    }
    meta.created = now;
}

/// The device number stat reports for files on the mount with `ids`
/// (`st_dev`): an anonymous device, major 0, numbered by the filesystem
/// instance, as Linux numbers the superblocks of ramfs, tmpfs and procfs,
/// so a file has the same device in every mount namespace. The first root
/// is 0:1, and files outside the tree (pipes, sockets) are 0:0.
pub const fn mount_dev(ids: MountIds) -> u32 {
    make_dev(0, ids.dev)
}

/// The major number of a device number in Linux's 32-bit encoding (as
/// `mknod` takes it and `stat` reports it: `new_decode_dev`).
pub const fn dev_major(dev: u32) -> u32 {
    (dev & 0xf_ff00) >> 8
}

/// The minor number of a device number in Linux's 32-bit encoding.
pub const fn dev_minor(dev: u32) -> u32 {
    (dev & 0xff) | ((dev >> 12) & 0xf_ff00)
}

/// A device number in Linux's 32-bit encoding (`new_encode_dev`): 12 bits
/// of major, 20 of minor.
pub const fn make_dev(major: u32, minor: u32) -> u32 {
    (minor & 0xff) | ((major & 0xfff) << 8) | ((minor & !0xff) << 12)
}

/// Which file a node names, for tables that find something by its inode (a
/// FIFO's pipe, the socket bound to a socket file): `file_identity` where
/// the filesystem has one (BlockFS), otherwise the node's address, which
/// stays unique while the table holds the node.
pub(crate) fn inode_key(node: &Arc<dyn VfsNode>) -> (u64, u64) {
    node.file_identity()
        .unwrap_or((0, Arc::as_ptr(node) as *const () as u64))
}

/// A special inode found by number (BlockFS's `file_identity`) lost its
/// last link and was freed. Forget what its FIFO table and socket bindings
/// hold under that key, so a new file that is given the same inode number
/// starts fresh; open FIFO ends and the bound socket keep working, as on
/// Linux, where an unlinked inode lives until its last reference goes.
pub(crate) fn special_inode_freed(key: (u64, u64)) {
    fifo::forget(key);
    crate::net::unix_socket::forget_file(key);
}

/// The kind of file a mknod `mode` asks for (Linux's `may_mknod`): a type
/// of 0 is a regular file; a directory is EPERM (`Err(true)`) and any
/// other unknown type EINVAL (`Err(false)`).
pub fn mknod_kind(mode: u32) -> Result<NodeType, bool> {
    match mode & 0o170000 {
        0 | 0o100000 => Ok(NodeType::File),
        0o020000 => Ok(NodeType::CharDevice),
        0o060000 => Ok(NodeType::BlockDevice),
        0o010000 => Ok(NodeType::Pipe),
        0o140000 => Ok(NodeType::Socket),
        0o040000 => Err(true),
        _ => Err(false),
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

    /// Whether a file handle can name this filesystem's files (Linux's
    /// export operations): its inode numbers are unique among its files,
    /// so open_by_handle_at can find one again by number
    /// ([`Vfs::find_by_inode`]). Filesystems that make inode numbers up
    /// (procfs, devfs) say no, and name_to_handle_at is EOPNOTSUPP there
    /// unless AT_HANDLE_FID asks for a handle that is never opened.
    fn exportable(&self) -> bool {
        false
    }
}

/// Mount point information
pub struct MountPoint {
    pub path: String,
    pub filesystem: Arc<dyn Filesystem>,
}

/// A mount's identifiers, as statx reports them (Linux's `mnt_id` and
/// `mnt_id_unique`): `id` is the lowest one free when it was mounted and
/// is reused after an unmount; `unique` never is. `dev` is the minor of
/// its filesystem's anonymous device, the same for every mount of that
/// filesystem ([`mount_dev`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountIds {
    pub id: u32,
    pub unique: u64,
    pub dev: u32,
}

/// Linux's MNT_UNIQUE_ID_OFFSET: unique ids start above it, so that one
/// is never mistaken for a 31-bit `mnt_id`.
const MNT_UNIQUE_ID_OFFSET: u64 = 1 << 31;

/// The mount of files that have no path (pipes, sockets, anonymous
/// inodes), Linux's internal pipefs and sockfs mounts: the first ids,
/// taken before any other mount.
pub const PSEUDO_MOUNT: MountIds = MountIds {
    id: 0,
    unique: MNT_UNIQUE_ID_OFFSET + 1,
    dev: 0,
};

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
pub(crate) fn path_is_under(path: &str, mount: &str) -> bool {
    mount == "/"
        || path == mount
        || (path.starts_with(mount) && path.as_bytes().get(mount.len()) == Some(&b'/'))
}

/// An immutable view of the mounted filesystems (FS-PERF-02).
///
/// Path resolution works on a snapshot: it clones the `Arc` under a short
/// read lock and walks without holding any VFS lock, so a lookup that reads
/// a disk does not hold up mount changes, and mount changes never observe a
/// half-finished walk. Mount changes copy the table, edit the copy and swap
/// it in (RCU-style); a walk already in progress finishes on the table it
/// started with.
#[derive(Clone, Default)]
struct MountTable {
    /// The root mount.
    root: Option<Arc<mount::Mount>>,
    /// The other mounts, keyed by normalized mount point: the top one at
    /// each place.
    mounts: BTreeMap<String, Arc<mount::Mount>>,
    /// The mounts a mount at the same place covers ("/" too), bottom
    /// first: mounting on a mount point stacks, as in Linux.
    covered: BTreeMap<String, Vec<Arc<mount::Mount>>>,
    /// Old roots `pivot_root(".", ".")` left on top of the new root at the
    /// same place. Linux mounts the old root on the new root's root, where
    /// no path walk crosses into it: these serve no path but are listed in
    /// /proc, copied with the namespace and taken off by umount2 at that
    /// place. By place, each a tree of its own (paths relative to its
    /// root), the last on top.
    parked: BTreeMap<String, Vec<MountTable>>,
}

impl MountTable {
    /// The mount point serving normalized `path`: the longest one it lies
    /// under, or the root.
    fn mount_of(&self, path: &str) -> &str {
        self.mounts
            .keys()
            .filter(|m| path_is_under(path, m))
            .max_by_key(|m| m.len())
            .map_or("/", |m| m.as_str())
    }

    /// The mount at mount point `point` ("/" for the root).
    fn mount_at(&self, point: &str) -> Option<&Arc<mount::Mount>> {
        match point {
            "/" => self.root.as_ref(),
            m => self.mounts.get(m),
        }
    }

    /// The mount serving canonical `path` (normalized here).
    fn serving(&self, path: &str) -> Option<&Arc<mount::Mount>> {
        self.mount_at(self.mount_of(&normalize_path(path, "/")))
    }

    /// The filesystem mounted at mount point `point`.
    fn fs_at(&self, point: &str) -> Option<Arc<dyn Filesystem>> {
        self.mount_at(point).map(|m| m.fs().clone())
    }

    /// The ids of the mount serving canonical `path`, and whether `path` is
    /// that mount's root (statx's STATX_ATTR_MOUNT_ROOT).
    fn ids_of(&self, path: &str) -> (MountIds, bool) {
        let path = normalize_path(path, "/");
        let mount = self.mount_of(&path);
        let ids = self.mount_at(mount).map_or(PSEUDO_MOUNT, |m| m.ids());
        (ids, path == mount)
    }

    /// The filesystem whose files stat reports with device number `dev`.
    fn fs_with_dev(&self, dev: u32) -> Option<Arc<dyn Filesystem>> {
        self.root
            .iter()
            .chain(self.mounts.values())
            .find(|m| mount_dev(m.ids()) == dev)
            .map(|m| m.fs().clone())
    }

    /// The filesystem serving canonical `path`, with its mount point.
    fn fs_of(&self, path: &str) -> Option<(String, Arc<dyn Filesystem>)> {
        let path = normalize_path(path, "/");
        let mount = self.mount_of(&path);
        let fs = self.fs_at(mount)?;
        Some((String::from(mount), fs))
    }
}

/// The tree of the initial mount namespace: the global VFS's, which the
/// initial namespace set (`process::ns`) names.
pub fn initial_mounts() -> Arc<mount::Mounts> {
    static INITIAL: spin::Once<Arc<mount::Mounts>> = spin::Once::new();
    INITIAL.call_once(|| Arc::new(mount::Mounts::new())).clone()
}

/// Virtual Filesystem Manager
pub struct Vfs {
    /// The tree its own methods (the kernel's view) work on: for the
    /// global VFS the initial mount namespace's. A caller in another mount
    /// namespace sees that one's ([`Vfs::caller_context`]).
    mounts: Arc<mount::Mounts>,

    /// Legacy global working directory (fallback only).
    /// Per-process CWD is tracked in `process::cwd::ProcessCwd` and
    /// `process::thread::ThreadFs`.  This field is retained for
    /// kernel-context operations where no process is running.
    cwd: RwLock<String>,
}

impl Vfs {
    /// Create a new VFS instance
    pub fn new() -> Self {
        Self::with_mounts(Arc::new(mount::Mounts::new()))
    }

    /// A VFS working on the tree `mounts`.
    fn with_mounts(mounts: Arc<mount::Mounts>) -> Self {
        Self {
            mounts,
            cwd: RwLock::new(String::from("/")),
        }
    }

    /// Its tree (for the global VFS the initial mount namespace's).
    #[cfg(test)]
    pub(crate) fn mounts(&self) -> &Arc<mount::Mounts> {
        &self.mounts
    }

    /// The root filesystem, if one is mounted.
    pub(crate) fn root_fs(&self) -> Option<Arc<dyn Filesystem>> {
        self.snapshot().fs_at("/")
    }

    /// The filesystem mounted exactly at `path`, if any.
    pub(crate) fn mounted(&self, path: &str) -> Option<Arc<dyn Filesystem>> {
        self.snapshot()
            .mounts
            .get(&normalize_path(path, "/"))
            .map(|m| m.fs().clone())
    }

    fn snapshot(&self) -> Arc<MountTable> {
        self.mounts.snapshot()
    }

    /// Apply `f` to a copy of the mount table and publish the copy.
    /// Mount changes are serialized by the table's write lock.
    fn update_table<R>(&self, f: impl FnOnce(&mut MountTable) -> R) -> R {
        self.mounts.update(f)
    }
}

impl MountTable {
    /// Resolution core (FS-SEC-01), component by component as POSIX
    /// specifies.
    ///
    /// `..` is applied in order, after the components before it, so it
    /// removes the directory a symlink *resolved to*: with `link -> b/c`,
    /// `/a/link/../x` is `/a/b/x`. The earlier implementation removed `..`
    /// lexically before looking at any component and returned `/a/x`, so
    /// checks made on the canonical path could concern a different object
    /// (review of the v0.26.0 stack, PR #9).
    ///
    /// The canonical path is kept as components with no `.`, `..` or
    /// followed symlink. A mount is entered when that path equals its
    /// mount point (whole components, so `/devices` is never served by a
    /// filesystem at `/dev`), and `..` leaves a mount the same way, by
    /// re-walking the shortened canonical path from the root. Every
    /// component that is not last must be a directory.
    ///
    /// With `creds = Some(credentials)` every directory searched -- for a
    /// name or for `..` -- must grant that caller search (execute)
    /// permission, as POSIX requires; otherwise the walk fails with
    /// `PermissionDenied`. `None` is the kernel's own lookups, and root.
    #[cfg(test)]
    fn resolve(
        &self,
        path: &str,
        cwd: &str,
        follow_last: bool,
        symlink_depth: usize,
        creds: Option<crate::process::creds::Caller>,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.resolve_in(path, cwd, "/", follow_last, symlink_depth, creds, 0)
    }

    /// `resolve` under a root directory (chroot). `root` is a canonical
    /// path in the whole tree; `path` when absolute, and `cwd`, are
    /// relative to it. Absolute paths and absolute symlink targets start at
    /// the root and `..` stops there, so no lookup leaves it. The canonical
    /// path returned is in the whole tree (for MAC and mount checks); see
    /// `root_relative` for the process's view.
    ///
    /// `resolve` holds openat2's RESOLVE_* flags ([`resolve_flags`]), as
    /// Linux's path walk applies them: the walk starts at `cwd` itself, so
    /// the mounts on the way there are not crossings; RESOLVE_NO_XDEV
    /// refuses entering or leaving (`..`) a mount and an absolute symlink
    /// read on another mount than the root's (EXDEV); RESOLVE_BENEATH an
    /// absolute path or symlink and `..` above the start (EXDEV);
    /// RESOLVE_NO_SYMLINKS every symlink and RESOLVE_NO_MAGICLINKS a magic
    /// link (ELOOP), which a scoped lookup cannot follow either (EXDEV).
    /// RESOLVE_IN_ROOT is `root` itself, as a chroot.
    #[allow(clippy::too_many_arguments)]
    fn resolve_in(
        &self,
        path: &str,
        cwd: &str,
        root: &str,
        follow_last: bool,
        symlink_depth: usize,
        creds: Option<crate::process::creds::Caller>,
        resolve: u32,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        use alloc::collections::VecDeque;

        let split = |p: &str| -> VecDeque<String> {
            p.split('/')
                .filter(|c| !c.is_empty() && *c != ".")
                .map(String::from)
                .collect()
        };

        let xdev = || KernelError::FsError(crate::error::FsError::CrossDevice);
        let symlink_loop = || KernelError::FsError(crate::error::FsError::SymlinkLoop);
        // A trailing slash names a directory (N-203): the last component is
        // followed even without `follow_last`, and must be one (ENOTDIR).
        let trailing_slash = path.ends_with('/');
        let follow_last = follow_last || trailing_slash;
        let root_canon: Vec<String> = split(root).into_iter().collect();
        let mut canon: Vec<String> = root_canon.clone();
        let mut pending = if path.starts_with('/') {
            if resolve & resolve_flags::BENEATH != 0 {
                return Err(xdev());
            }
            split(path)
        } else if resolve != 0 {
            // The working directory is canonical: start in it, as Linux's
            // path_init starts at the directory's dentry.
            canon.extend(split(cwd));
            split(path)
        } else {
            let mut q = split(cwd);
            q.extend(split(path));
            q
        };
        let mut node = self.walk_canonical(&canon)?;
        let mut links = symlink_depth;

        while let Some(component) = pending.pop_front() {
            // Whatever is being searched -- for a name or for ".." -- must
            // be a directory: "/a/file/.." is ENOTDIR, as in POSIX.
            if node.node_type() != NodeType::Directory {
                return Err(KernelError::FsError(crate::error::FsError::NotADirectory));
            }
            if let Some(c) = &creds {
                if !permission(c, &node.metadata()?, MAY_EXEC) {
                    return Err(KernelError::FsError(
                        crate::error::FsError::PermissionDenied,
                    ));
                }
            }
            if component == ".." {
                // ".." of the root is the root itself.
                if canon.len() > root_canon.len() {
                    if resolve & resolve_flags::NO_XDEV != 0
                        && self.mounts.contains_key(&join_components(&canon))
                    {
                        return Err(xdev());
                    }
                    canon.pop();
                } else if resolve & resolve_flags::BENEATH != 0 {
                    return Err(xdev());
                }
                node = self.walk_canonical(&canon)?;
                continue;
            }
            canon.push(component);
            let canon_path = join_components(&canon);
            node = match self.mounts.get(&canon_path) {
                Some(_) if resolve & resolve_flags::NO_XDEV != 0 => return Err(xdev()),
                Some(mount) => mount.root(),
                None => node.lookup(canon.last().map(String::as_str).unwrap_or(""))?,
            };

            let is_last = pending.is_empty();
            if node.node_type() == NodeType::Symlink && (!is_last || follow_last) {
                links += 1;
                if links > SYMLINK_MAX_DEPTH || resolve & resolve_flags::NO_SYMLINKS != 0 {
                    return Err(symlink_loop());
                }
                // A link on a nosymfollow mount is not followed (Linux's
                // pick_link, magic links too; ADR 0022 H3c).
                if self
                    .serving(&canon_path)
                    .is_some_and(|m| m.has(mount::flags::NOSYMFOLLOW))
                {
                    return Err(symlink_loop());
                }
                // A link to an object: the walk goes on from the object
                // (anything after it finds no directory, ENOTDIR). It is a
                // magic link, and leaves the mount (Linux's nd_jump_link).
                if let Some(target) = node.magic_target() {
                    if resolve & resolve_flags::NO_MAGICLINKS != 0 {
                        return Err(symlink_loop());
                    }
                    if resolve & (resolve_flags::SCOPED | resolve_flags::NO_XDEV) != 0 {
                        return Err(xdev());
                    }
                    node = target?;
                    continue;
                }
                let target = node.readlink()?;
                // The target replaces the link: relative to the link's own
                // directory, or to the root if absolute. Its components go
                // in front of whatever was still pending.
                canon.pop();
                if resolve != 0 {
                    let here = self.mount_of(&join_components(&canon));
                    if node.is_magic_link() {
                        // Linux's nd_jump_link.
                        if resolve & resolve_flags::NO_MAGICLINKS != 0 {
                            return Err(symlink_loop());
                        }
                        if resolve & resolve_flags::NO_XDEV != 0
                            && here != self.mount_of(&normalize_path(&target, "/"))
                        {
                            return Err(xdev());
                        }
                        if resolve & resolve_flags::SCOPED != 0 {
                            return Err(xdev());
                        }
                    } else if target.starts_with('/') {
                        // Linux's nd_jump_root.
                        if resolve & resolve_flags::BENEATH != 0
                            || (resolve & resolve_flags::NO_XDEV != 0
                                && here != self.mount_of(&join_components(&root_canon)))
                        {
                            return Err(xdev());
                        }
                    }
                }
                if target.starts_with('/') {
                    canon = root_canon.clone();
                }
                let mut spliced = split(&target);
                spliced.extend(pending.drain(..));
                pending = spliced;
                node = self.walk_canonical(&canon)?;
            }
        }

        if trailing_slash && node.node_type() != NodeType::Directory {
            return Err(KernelError::FsError(crate::error::FsError::NotADirectory));
        }
        Ok((node, join_components(&canon)))
    }

    /// The node at canonical components `canon` (no `.`, `..` or symlinks),
    /// entering mounts by whole-path match.
    fn walk_canonical(&self, canon: &[String]) -> Result<Arc<dyn VfsNode>, KernelError> {
        let root = self
            .root
            .as_ref()
            .ok_or(KernelError::FsError(crate::error::FsError::NoRootFs))?;
        let mut node = root.root();
        let mut path = String::new();
        for c in canon {
            path.push('/');
            path.push_str(c);
            node = match self.mounts.get(&path) {
                Some(mount) => mount.root(),
                None => node.lookup(c)?,
            };
        }
        Ok(node)
    }
}

/// File seals (fcntl F_ADD_SEALS / F_GET_SEALS), with Linux's values. A
/// sealed memfd refuses the sealed operations with EPERM.
pub mod seals {
    /// No further seals may be added.
    pub const SEAL: u32 = 0x1;
    /// The size may not decrease.
    pub const SHRINK: u32 = 0x2;
    /// The size may not increase.
    pub const GROW: u32 = 0x4;
    /// The contents may not change.
    pub const WRITE: u32 = 0x8;
    /// As `WRITE` for write(2) and new writable mappings; existing writable
    /// mappings keep working.
    pub const FUTURE_WRITE: u32 = 0x10;
    /// The execute bits may not change.
    pub const EXEC: u32 = 0x20;
    /// Every seal.
    pub const ALL: u32 = SEAL | SHRINK | GROW | WRITE | FUTURE_WRITE | EXEC;
}

/// Whose view of the tree a path lookup uses (N-251). A kernel subsystem
/// (audit, logging, the package manager) must see the real tree whichever
/// thread is current -- with the caller's root a chrooted process could
/// make the audit log land in a file it owns, and with its permissions
/// make logging fail -- while a system call must see exactly the caller's.
/// `Vfs` methods use the kernel's view; `Vfs::as_caller` gives the
/// caller's.
#[derive(Debug, Clone)]
pub struct PathContext {
    /// Where relative paths start, relative to `root`.
    pub cwd: String,
    /// The root directory, as a canonical path in the whole tree.
    pub root: String,
    /// Whose search permission a walk checks; `None` checks none.
    pub creds: Option<crate::process::creds::Caller>,
    /// The process MAC policy is checked for (0: the kernel).
    pub pid: u64,
    /// openat2's RESOLVE_* flags ([`resolve_flags`]); 0 for every other
    /// lookup.
    pub resolve: u32,
    /// The mount tree the walk uses: the caller's mount namespace's.
    pub mounts: Arc<mount::Mounts>,
}

/// openat2's `how.resolve` flags, with Linux's values.
pub mod resolve_flags {
    /// No mount crossing, bind mounts included.
    pub const NO_XDEV: u32 = 0x01;
    /// No procfs magic links.
    pub const NO_MAGICLINKS: u32 = 0x02;
    /// No symlinks at all.
    pub const NO_SYMLINKS: u32 = 0x04;
    /// Nothing outside the starting directory: no `..` above it, no
    /// absolute path or symlink.
    pub const BENEATH: u32 = 0x08;
    /// The starting directory is the root, as with chroot.
    pub const IN_ROOT: u32 = 0x10;
    /// Only a lookup the dentry cache could finish (there is none, so the
    /// walk runs; only the open flags it refuses are checked).
    pub const CACHED: u32 = 0x20;
    /// The lookups scoped to the starting directory.
    pub const SCOPED: u32 = BENEATH | IN_ROOT;
    /// Every flag.
    pub const ALL: u32 = 0x3f;
}

/// The VFS as the calling thread sees it (see [`PathContext`]). It does
/// not dereference to `Vfs`: an operation it lacks is a compile error in
/// the system call that wants it, never a silent lookup in the kernel's
/// view, which would escape a chroot and skip permission checks.
pub struct CallerView<'a> {
    vfs: &'a Vfs,
    ctx: PathContext,
}

impl<'a> CallerView<'a> {
    /// The caller's context.
    pub fn context(&self) -> &PathContext {
        &self.ctx
    }

    /// The VFS itself, for operations on the mount table rather than on
    /// paths (mount, unmount, sync).
    pub fn global(&self) -> &'a Vfs {
        self.vfs
    }

    /// The caller's working directory, relative to its root.
    pub fn get_cwd(&self) -> String {
        self.ctx.cwd.clone()
    }

    /// The caller's root, as a canonical path in the whole tree.
    pub fn get_root(&self) -> String {
        self.ctx.root.clone()
    }

    /// This view with openat2's `resolve` flags ([`resolve_flags`]),
    /// relative paths starting at `start` (canonical, relative to the
    /// caller's root). A scoped lookup (RESOLVE_BENEATH, RESOLVE_IN_ROOT)
    /// has `start` as its root.
    pub fn resolving(mut self, resolve: u32, start: &str) -> Self {
        self.ctx.resolve = resolve;
        if resolve & resolve_flags::SCOPED != 0 {
            self.ctx.root = if self.ctx.root == "/" {
                String::from(start)
            } else if start == "/" {
                self.ctx.root.clone()
            } else {
                format!("{}{}", self.ctx.root, start)
            };
            self.ctx.cwd = String::from("/");
        } else {
            self.ctx.cwd = String::from(start);
        }
        self
    }

    pub fn resolve_path(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.vfs
            .lookup(&self.ctx, path, &self.ctx.cwd, true)
            .map(|(n, _)| n)
    }

    /// [`Self::resolve_path`] (relative to the caller's working directory)
    /// with the node's canonical path from the same lookup.
    pub fn resolve_path_canonical(
        &self,
        path: &str,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.vfs.lookup(&self.ctx, path, &self.ctx.cwd, true)
    }

    pub fn resolve_path_no_follow(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.vfs
            .lookup(&self.ctx, path, &self.ctx.cwd, false)
            .map(|(n, _)| n)
    }

    /// [`Vfs::resolve_canonical`] in the caller's view; `cwd` is relative
    /// to the caller's root.
    pub fn resolve_canonical(
        &self,
        path: &str,
        cwd: &str,
        follow_last: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.vfs.lookup(&self.ctx, path, cwd, follow_last)
    }

    pub fn open_follow_canonical(
        &self,
        path: &str,
        flags: OpenFlags,
        follow_last: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.vfs.open_in(&self.ctx, path, flags, follow_last)
    }

    pub fn entry_mount_point(&self, parent: &str, name: &str) -> Result<String, KernelError> {
        self.vfs.entry_mount_point_in(&self.ctx, parent, name)
    }

    /// The mount point of entry `name` of the directory whose canonical path
    /// is `canonical_parent` (from [`Self::resolve_canonical`]), without
    /// resolving anything again.
    pub fn canonical_entry_mount_point(&self, canonical_parent: &str, name: &str) -> String {
        let entry = if canonical_parent == "/" {
            alloc::format!("/{}", name)
        } else {
            alloc::format!("{}/{}", canonical_parent, name)
        };
        self.mount_point_of(&entry)
    }

    /// Whether something is mounted strictly under canonical `path` in the
    /// caller's mount namespace (a mount, or an old root pivot_root parked
    /// there): renaming `path` would leave those mounts at the old place,
    /// as mounts are kept by path (PR #35 review).
    pub fn mounts_under(&self, path: &str) -> bool {
        let path = normalize_path(path, "/");
        let table = self.ctx.mounts.snapshot();
        let under = |m: &String| m.as_str() != path && path_is_under(m, &path);
        table.mounts.keys().any(under) || table.parked.keys().any(|p| path_is_under(p, &path))
    }

    /// [`Vfs::mount_point_of`] in the caller's mount namespace.
    pub fn mount_point_of(&self, path: &str) -> String {
        String::from(
            self.ctx
                .mounts
                .snapshot()
                .mount_of(&normalize_path(path, "/")),
        )
    }

    /// [`Vfs::mount_ids_of`] in the caller's mount namespace.
    pub fn mount_ids_of(&self, path: &str) -> (MountIds, bool) {
        self.ctx.mounts.snapshot().ids_of(path)
    }

    /// The mount serving canonical `path` in the caller's mount namespace,
    /// whose flags say what may be done there (ADR 0022 H3c); `None`
    /// before a root is mounted.
    pub fn mount_serving(&self, path: &str) -> Option<Arc<mount::Mount>> {
        self.ctx.mounts.snapshot().serving(path).cloned()
    }

    /// [`Vfs::filesystem_of`] in the caller's mount namespace.
    pub fn filesystem_of(&self, path: &str) -> Option<(String, Arc<dyn Filesystem>)> {
        self.ctx.mounts.snapshot().fs_of(path)
    }

    /// [`Vfs::filesystem_with_dev`] in the caller's mount namespace.
    pub fn filesystem_with_dev(&self, dev: u32) -> Option<Arc<dyn Filesystem>> {
        self.ctx.mounts.snapshot().fs_with_dev(dev)
    }

    /// [`Vfs::find_by_inode`] in the caller's mount namespace.
    pub fn find_by_inode(&self, mount: &str, ino: u64) -> Option<(Arc<dyn VfsNode>, String)> {
        self.ctx.mounts.snapshot().find_by_inode(mount, ino)
    }

    pub fn mkdir(
        &self,
        path: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.vfs.mkdir_in(&self.ctx, path, permissions, |_| Ok(()))
    }

    /// [`Self::mkdir`], with `check` (the caller's permission check) run on
    /// the resolved parent directory before the entry is created in that
    /// same directory, so the path cannot change in between (N-51).
    pub fn mkdir_checked(
        &self,
        path: &str,
        permissions: Permissions,
        check: impl FnOnce(&dyn VfsNode) -> Result<(), KernelError>,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.vfs.mkdir_in(&self.ctx, path, permissions, check)
    }

    /// Create `path` for mknod -- a regular file (`NodeType::File`), FIFO,
    /// device numbered `rdev`, or socket -- as [`Self::mkdir_checked`]
    /// creates a directory: one resolution of the parent for the MAC check,
    /// `check` and the creation.
    pub fn mknod_checked(
        &self,
        path: &str,
        kind: NodeType,
        permissions: Permissions,
        rdev: u32,
        check: impl FnOnce(&dyn VfsNode) -> Result<(), KernelError>,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.vfs.create_in(&self.ctx, path, check, |parent, name| {
            if kind == NodeType::File {
                parent.create(name, permissions)
            } else {
                parent.mknod(name, kind, permissions, rdev)
            }
        })
    }

    pub fn unlink(&self, path: &str) -> Result<(), KernelError> {
        self.vfs.unlink_in(&self.ctx, path)
    }

    /// The canonical path in the whole tree of the directory `path` names:
    /// what the mount table is keyed by. Without this a chrooted
    /// `mount("/")` would name the real root.
    fn mount_target(&self, path: &str) -> Result<String, KernelError> {
        let (node, canonical) = self.vfs.lookup(&self.ctx, path, &self.ctx.cwd, true)?;
        if node.node_type() != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }
        Ok(canonical)
    }

    /// [`Vfs::mount_by_type`] at the directory `path` names for the caller.
    pub fn mount_by_type(&self, path: &str, fs_type: &str, _flags: u32) -> Result<(), KernelError> {
        let target = self.mount_target(path)?;
        mount_on(&self.ctx.mounts, &target, fs_by_type(fs_type)?)
    }

    /// [`Vfs::unmount`] of the mount at the directory `path` names for the
    /// caller.
    pub fn unmount(&self, path: &str) -> Result<(), KernelError> {
        let target = self.mount_target(path)?;
        unmount_on(&self.ctx.mounts, &target)
    }

    /// The node `path` names for the caller (relative paths from its
    /// working directory; a last symlink followed if `follow`) and its
    /// canonical path in the whole tree.
    pub fn locate(
        &self,
        path: &str,
        follow: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.vfs.lookup(&self.ctx, path, &self.ctx.cwd, follow)
    }

    /// The caller's mount namespace's tree (mount(2), umount2).
    pub fn mount_tree(&self) -> &Arc<mount::Mounts> {
        &self.ctx.mounts
    }

    /// The caller's root, a canonical path in the whole tree.
    pub fn root_path(&self) -> &str {
        &self.ctx.root
    }

    /// Flush every filesystem; not path-dependent.
    pub fn sync(&self) -> Result<(), KernelError> {
        self.vfs.sync()
    }
}

/// `path`, a canonical path in the whole tree, as seen from inside `root`
/// (the inverse of `resolve_in`'s mapping): "/jail/a" is "/a" under root
/// "/jail". A path outside the root (which a lookup never produces) is
/// returned whole.
pub(crate) fn root_relative(path: &str, root: &str) -> String {
    if root == "/" {
        return String::from(path);
    }
    if path == root {
        return String::from("/");
    }
    match path.strip_prefix(root) {
        Some(rest) if rest.starts_with('/') => String::from(rest),
        _ => String::from(path),
    }
}

/// `/` followed by the components joined with `/`; `/` for none.
fn join_components(canon: &[String]) -> String {
    if canon.is_empty() {
        return String::from("/");
    }
    let mut path = String::new();
    for c in canon {
        path.push('/');
        path.push_str(c);
    }
    path
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs {
    /// Mount the root filesystem
    pub fn mount_root(&self, fs: Arc<dyn Filesystem>) -> Result<(), KernelError> {
        let pool = self.mounts.pool().clone();
        self.update_table(|t| {
            if t.root.is_some() {
                return Err(KernelError::FsError(crate::error::FsError::AlreadyMounted));
            }
            let source = mount::default_source(fs.as_ref(), true);
            t.root = Some(mount::Mount::new(fs, source, &pool));
            Ok(())
        })
    }

    /// Mount a filesystem at the specified path
    pub fn mount(&self, path: String, fs: Arc<dyn Filesystem>) -> Result<(), KernelError> {
        mount_on(&self.mounts, &path, fs)
    }

    /// Mount a filesystem by type at the specified path
    pub fn mount_by_type(&self, path: &str, fs_type: &str, _flags: u32) -> Result<(), KernelError> {
        let fs = fs_by_type(fs_type)?;
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
    pub fn swap_root(&self, fs: Arc<dyn Filesystem>) {
        let pool = self.mounts.pool().clone();
        self.update_table(|t| {
            let source = mount::default_source(fs.as_ref(), true);
            t.root = Some(mount::Mount::new(fs, source, &pool));
        });
    }

    /// Unmount a filesystem at the specified path
    pub fn unmount(&self, path: &str) -> Result<(), KernelError> {
        unmount_on(&self.mounts, path)
    }

    /// The kernel's own view of the tree: the whole tree, the kernel
    /// shell's working directory, no search-permission checks, MAC as the
    /// kernel.
    pub fn kernel_context(&self) -> PathContext {
        PathContext {
            cwd: self.cwd.read().clone(),
            root: String::from("/"),
            creds: None,
            pid: 0,
            resolve: 0,
            mounts: self.mounts.clone(),
        }
    }

    /// The calling thread's view: its working directory and root
    /// (chroot), who it is for search permission (`Caller`: CAP_DAC_OVERRIDE
    /// and CAP_DAC_READ_SEARCH pass it) and its pid for MAC. Without a
    /// current thread or process
    /// (the kernel shell) that part is the kernel's.
    pub fn caller_context(&self) -> PathContext {
        let mut ctx = self.kernel_context();
        #[cfg(feature = "alloc")]
        if let Some(thread) = crate::process::current_thread() {
            let fs = thread.fs();
            ctx.cwd = fs.cwd.lock().clone();
            ctx.root = fs.root.lock().clone();
            // The caller's mount namespace, for the VFS whose tree is the
            // initial namespace's (a VFS of its own, as in tests, keeps it).
            if Arc::ptr_eq(&self.mounts, &initial_mounts()) {
                ctx.mounts = thread.namespaces().mnt.mounts.clone();
            }
        }
        if let Some(process) = crate::process::current_process() {
            ctx.pid = process.pid.0;
            ctx.creds = Some(crate::process::creds::Caller::current());
        }
        ctx
    }

    /// Lookups as the calling thread: what system calls, exec and the
    /// program loader use for paths a program supplied.
    pub fn as_caller(&self) -> CallerView<'_> {
        CallerView {
            vfs: self,
            ctx: self.caller_context(),
        }
    }

    /// Lookups as `caller` instead of the calling thread's credentials,
    /// with the thread's root and working directory: access(2) resolves the
    /// whole path with the real IDs, as Linux's do_faccessat overrides the
    /// credentials before the lookup (PR #33 review).
    pub fn as_caller_with(&self, caller: crate::process::creds::Caller) -> CallerView<'_> {
        let mut ctx = self.caller_context();
        ctx.creds = Some(caller);
        CallerView { vfs: self, ctx }
    }

    /// Resolve `path` (relative paths from `cwd`) in `ctx`'s view: the
    /// node and its canonical path in the whole tree.
    fn lookup(
        &self,
        ctx: &PathContext,
        path: &str,
        cwd: &str,
        follow_last: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        ctx.mounts.snapshot().resolve_in(
            path,
            cwd,
            &ctx.root,
            follow_last,
            0,
            ctx.creds.clone(),
            ctx.resolve,
        )
    }

    /// Resolve a path to a VFS node, following symlinks (including the
    /// final component).
    pub fn resolve_path(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        let ctx = self.kernel_context();
        self.lookup(&ctx, path, &ctx.cwd, true).map(|(n, _)| n)
    }

    /// Resolve a path to a VFS node without following the final symlink
    /// component. Intermediate symlinks are still followed. Used by
    /// `lstat()` and `readlink()`.
    pub fn resolve_path_no_follow(&self, path: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        let ctx = self.kernel_context();
        self.lookup(&ctx, path, &ctx.cwd, false).map(|(n, _)| n)
    }

    /// Resolve a path to a VFS node using an explicit cwd.
    pub fn resolve_from(&self, path: &str, cwd: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.lookup(&self.kernel_context(), path, cwd, true)
            .map(|(n, _)| n)
    }

    /// Resolve a path to a VFS node using an explicit cwd, without
    /// following the final symlink component.
    pub fn resolve_from_no_follow(
        &self,
        path: &str,
        cwd: &str,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.lookup(&self.kernel_context(), path, cwd, false)
            .map(|(n, _)| n)
    }

    /// The mount point that serves `path`, a canonical path in the whole
    /// tree.
    pub fn mount_point_of(&self, path: &str) -> String {
        let path = normalize_path(path, "/");
        String::from(self.snapshot().mount_of(&path))
    }

    /// The ids of the mount serving `path`, a canonical path in the whole
    /// tree, and whether `path` is that mount's root (statx's
    /// STATX_ATTR_MOUNT_ROOT).
    pub fn mount_ids_of(&self, path: &str) -> (MountIds, bool) {
        self.snapshot().ids_of(path)
    }

    /// The filesystem whose files stat reports with device number `dev`
    /// ([`mount_dev`]), as ustat looks one up.
    pub fn filesystem_with_dev(&self, dev: u32) -> Option<Arc<dyn Filesystem>> {
        self.snapshot().fs_with_dev(dev)
    }

    /// The filesystem serving `path`, a canonical path in the whole tree,
    /// with its mount point; `None` before a root is mounted.
    pub fn filesystem_of(&self, path: &str) -> Option<(String, Arc<dyn Filesystem>)> {
        self.snapshot().fs_of(path)
    }

    /// The file with inode number `ino` in the filesystem mounted at
    /// `mount`, and a canonical path to it, for open_by_handle_at (Linux's
    /// exportfs_decode_fh). The search walks that filesystem's directories
    /// from its root, breadth first, and does not enter a directory another
    /// mount covers, whose files cannot be reached by a path. A file that
    /// is open but has no name left is not found (Linux finds it while its
    /// inode is cached), and on a large filesystem the walk is slow; Linux
    /// filesystems find an inode by number directly.
    pub fn find_by_inode(&self, mount: &str, ino: u64) -> Option<(Arc<dyn VfsNode>, String)> {
        self.snapshot().find_by_inode(mount, ino)
    }
}

impl MountTable {
    /// [`Vfs::find_by_inode`] in this tree.
    fn find_by_inode(&self, mount: &str, ino: u64) -> Option<(Arc<dyn VfsNode>, String)> {
        let table = self;
        let fs = table.fs_at(mount)?;
        let root = fs.root();
        if root.metadata().ok()?.inode == ino {
            return Some((root, String::from(mount)));
        }
        let join = |dir: &str, name: &str| {
            if dir == "/" {
                alloc::format!("/{}", name)
            } else {
                alloc::format!("{}/{}", dir, name)
            }
        };
        let mut queue = alloc::collections::VecDeque::new();
        queue.push_back((root, String::from(mount)));
        while let Some((dir, dir_path)) = queue.pop_front() {
            let Ok(entries) = dir.readdir() else {
                continue;
            };
            for entry in entries {
                if entry.name == "." || entry.name == ".." {
                    continue;
                }
                let is_dir = entry.node_type == NodeType::Directory;
                // Directory entries carry the inode number on every
                // exportable filesystem; only a match or a directory is
                // looked up.
                if !is_dir && entry.inode != ino {
                    continue;
                }
                let path = join(&dir_path, &entry.name);
                if is_dir && table.mounts.contains_key(&path) {
                    continue;
                }
                let Ok(node) = dir.lookup(&entry.name) else {
                    continue;
                };
                if node.metadata().is_ok_and(|m| m.inode == ino) {
                    return Some((node, path));
                }
                if is_dir {
                    queue.push_back((node, path));
                }
            }
        }
        None
    }
}

impl Vfs {
    /// The mount point holding the directory entry `name` in `parent`, with
    /// `parent` resolved canonically (symlinks, `.` and `..` followed; the
    /// entry itself is not). A rename must compare these rather than the
    /// paths as written: a symlink in the parent path can point into another
    /// mount (review of the v0.26.0 stack, PR #11).
    pub fn entry_mount_point(&self, parent: &str, name: &str) -> Result<String, KernelError> {
        let ctx = self.kernel_context();
        self.entry_mount_point_in(&ctx, parent, name)
    }

    fn entry_mount_point_in(
        &self,
        ctx: &PathContext,
        parent: &str,
        name: &str,
    ) -> Result<String, KernelError> {
        let (_, canon) = self.lookup(ctx, parent, &ctx.cwd, true)?;
        let entry = if canon == "/" {
            alloc::format!("/{}", name)
        } else {
            alloc::format!("{}/{}", canon, name)
        };
        Ok(String::from(ctx.mounts.snapshot().mount_of(&entry)))
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
        self.lookup(&self.kernel_context(), path, cwd, follow_last)
    }

    /// The kernel's working directory (the kernel shell's `cd`). A user
    /// thread's own is in `ThreadFs` and reached through `as_caller`
    /// (N-115).
    pub fn get_cwd(&self) -> String {
        self.cwd.read().clone()
    }

    /// Set current working directory
    pub fn set_cwd(&self, path: String) -> Result<(), KernelError> {
        // Verify the path exists and is a directory
        let node = self.resolve_path(&path)?;
        let metadata = node.metadata()?;

        if metadata.node_type != NodeType::Directory {
            return Err(KernelError::FsError(crate::error::FsError::NotADirectory));
        }

        *self.cwd.write() = path;
        Ok(())
    }

    /// Open a file
    ///
    /// Checks MAC policy before allowing access.
    pub fn open(&self, path: &str, flags: OpenFlags) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.open_follow(path, flags, true)
    }

    /// Like [`Self::open`]; `follow_last = false` returns a symlink at the
    /// final component itself (O_NOFOLLOW).
    pub fn open_follow(
        &self,
        path: &str,
        flags: OpenFlags,
        follow_last: bool,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.open_follow_canonical(path, flags, follow_last)
            .map(|(node, _)| node)
    }

    /// [`Self::open_follow`], also returning the node's canonical path in
    /// the whole tree (what an open file records for `fchdir` and `*at`).
    pub fn open_follow_canonical(
        &self,
        path: &str,
        flags: OpenFlags,
        follow_last: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        self.open_in(&self.kernel_context(), path, flags, follow_last)
    }

    fn open_in(
        &self,
        ctx: &PathContext,
        path: &str,
        flags: OpenFlags,
        follow_last: bool,
    ) -> Result<(Arc<dyn VfsNode>, String), KernelError> {
        let access = if flags.write {
            crate::security::AccessType::Write
        } else {
            crate::security::AccessType::Read
        };
        // MAC is checked on the canonical path, after `..` and symlinks are
        // resolved, never on the path as written (FS-SEC-01).
        let (node, canonical) = self.lookup(ctx, path, &ctx.cwd, follow_last)?;
        crate::security::mac::check_file_access(&canonical, access, ctx.pid)?;
        Ok((node, canonical))
    }

    /// Create a directory
    ///
    /// Checks MAC policy (Write access to file domain) before creating.
    /// Create a directory and return its node. Callers that adjust the new
    /// directory (ownership) must use this node, not look the path up again:
    /// the name can be replaced in between.
    pub fn mkdir(
        &self,
        path: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.mkdir_in(&self.kernel_context(), path, permissions, |_| Ok(()))
    }

    /// Make a special file (FIFO, device or socket) in the kernel's context,
    /// as [`Self::mkdir`] makes a directory.
    pub fn mknod(
        &self,
        path: &str,
        kind: NodeType,
        permissions: Permissions,
        rdev: u32,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.create_in(
            &self.kernel_context(),
            path,
            |_| Ok(()),
            |parent, name| parent.mknod(name, kind, permissions, rdev),
        )
    }

    fn mkdir_in(
        &self,
        ctx: &PathContext,
        path: &str,
        permissions: Permissions,
        check: impl FnOnce(&dyn VfsNode) -> Result<(), KernelError>,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        self.create_in(ctx, path, check, |parent, name| {
            parent.mkdir(name, permissions)
        })
    }

    /// Create the entry `path` names with `make(parent, name)`, after the
    /// MAC check (Write) and `check` on the parent.
    fn create_in(
        &self,
        ctx: &PathContext,
        path: &str,
        check: impl FnOnce(&dyn VfsNode) -> Result<(), KernelError>,
        make: impl FnOnce(&dyn VfsNode, &str) -> Result<Arc<dyn VfsNode>, KernelError>,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        // Parse the path once and use that single result both for the MAC
        // check and for the creation, so they cannot disagree: the parent
        // is resolved canonically (following symlinks) and the policy sees
        // the path the directory is really created at. Normalizing also
        // removes trailing slashes, `.` and `..`, so the final name is
        // never one of those.
        //
        // openat2's walk to the parent applies its RESOLVE_* flags, so the
        // path is split as written and `..` in it is walked, not removed.
        let (parent, canonical_parent, name) = if ctx.resolve != 0 {
            let path = path.trim_end_matches('/');
            let (parent_path, name) = match path.rfind('/') {
                Some(0) => ("/", &path[1..]),
                Some(pos) => (&path[..pos], &path[pos + 1..]),
                None => (".", path),
            };
            if name.is_empty() || name == "." || name == ".." {
                return Err(KernelError::FsError(crate::error::FsError::AlreadyExists));
            }
            let (parent, canonical) = self.lookup(ctx, parent_path, &ctx.cwd, true)?;
            (parent, canonical, String::from(name))
        } else {
            let path = normalize_path(path, &ctx.cwd);
            if path == "/" {
                return Err(KernelError::FsError(crate::error::FsError::AlreadyExists));
            }
            let pos = path.rfind('/').unwrap_or(0);
            let (parent_path, name) = (&path[..pos.max(1)], &path[pos + 1..]);
            let (parent, canonical) = self.lookup(ctx, parent_path, "/", true)?;
            (parent, canonical, String::from(name))
        };
        let name = name.as_str();
        let target = if canonical_parent == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", canonical_parent, name)
        };

        // A read-only mount makes nothing: EEXIST for a name that exists,
        // else EROFS, before any permission check (Linux's filename_create
        // and lookup_open).
        let snapshot = ctx.mounts.snapshot();
        if snapshot
            .serving(&canonical_parent)
            .is_some_and(|m| m.is_readonly())
        {
            if parent.lookup(name).is_ok() {
                return Err(KernelError::FsError(crate::error::FsError::AlreadyExists));
            }
            return Err(KernelError::FsError(crate::error::FsError::ReadOnly));
        }
        drop(snapshot);

        // MAC check: creating an entry requires Write access
        crate::security::mac::check_file_access(
            &target,
            crate::security::AccessType::Write,
            ctx.pid,
        )?;
        check(&*parent)?;

        let node = make(&*parent, name)?;
        inotify::created(&parent, name, &node);
        Ok(node)
    }

    /// Remove a file or directory
    pub fn unlink(&self, path: &str) -> Result<(), KernelError> {
        self.unlink_in(&self.kernel_context(), path)
    }

    fn unlink_in(&self, ctx: &PathContext, path: &str) -> Result<(), KernelError> {
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

        let (parent, _) = self.lookup(ctx, parent_path, &ctx.cwd, true)?;
        parent.unlink(name)
    }

    /// List all mount points and their filesystem types.
    ///
    /// Returns a vector of `(path, fs_name, readonly)` tuples.
    pub fn list_mounts(&self) -> Vec<(String, String, bool)> {
        let mut result = Vec::new();
        let table = self.snapshot();

        // Root filesystem
        if let Some(ref root) = table.root {
            let fs = root.fs();
            result.push((String::from("/"), String::from(fs.name()), fs.is_readonly()));
        }

        // Mounted filesystems
        for (path, mount) in &table.mounts {
            let fs = mount.fs();
            result.push((path.clone(), String::from(fs.name()), fs.is_readonly()));
        }

        result
    }

    /// Sync all filesystems
    pub fn sync(&self) -> Result<(), KernelError> {
        let table = self.snapshot();

        // Sync root filesystem
        if let Some(ref root) = table.root {
            root.fs().sync()?;
        }

        // Sync all mounted filesystems
        for mount in table.mounts.values() {
            mount.fs().sync()?;
        }

        Ok(())
    }
}

/// A new filesystem of the type `mount_by_type` names.
fn fs_by_type(fs_type: &str) -> Result<Arc<dyn Filesystem>, KernelError> {
    Ok(match fs_type {
        "ramfs" => Arc::new(ramfs::RamFs::new()),
        "devfs" => Arc::new(devfs::DevFs::new()),
        "procfs" => Arc::new(procfs::ProcFs::new()),
        "tmpfs" => Arc::new(tmpfs::TmpFs::half_of_ram()),
        "blockfs" => Arc::new(blockfs::BlockFs::new(10000, 1000)),
        _ => return Err(KernelError::FsError(FsError::UnknownFsType)),
    })
}

/// Mount `fs` at `path` in the tree `mounts`: EBUSY-like AlreadyMounted
/// when something is mounted there, NoRootFs before a root.
fn mount_on(
    mounts: &mount::Mounts,
    path: &str,
    fs: Arc<dyn Filesystem>,
) -> Result<(), KernelError> {
    let path = normalize_path(path, "/");
    let pool = mounts.pool().clone();
    mounts.update(|t| {
        if t.root.is_none() {
            return Err(KernelError::FsError(FsError::NoRootFs));
        }
        if path == "/" || t.mounts.contains_key(&path) {
            return Err(KernelError::FsError(FsError::AlreadyMounted));
        }
        let source = mount::default_source(fs.as_ref(), false);
        t.mounts.insert(path, mount::Mount::new(fs, source, &pool));
        Ok(())
    })
}

/// Remove the mount at `path` from the tree `mounts`.
fn unmount_on(mounts: &mount::Mounts, path: &str) -> Result<(), KernelError> {
    let path = normalize_path(path, "/");
    mounts.update(|t| {
        if t.mounts.remove(&path).is_none() {
            return Err(KernelError::FsError(FsError::NotMounted));
        }
        Ok(())
    })
}

/// EISDIR for unlink of a directory, ENOTDIR for rmdir of anything else.
pub fn check_unlink_kind(node_type: NodeType, want_dir: bool) -> Result<(), KernelError> {
    match (node_type == NodeType::Directory, want_dir) {
        (true, false) => Err(KernelError::FsError(FsError::IsADirectory)),
        (false, true) => Err(KernelError::FsError(FsError::NotADirectory)),
        _ => Ok(()),
    }
}

/// Global VFS instance using OnceLock for safe initialization.
static VFS: crate::sync::once_lock::OnceLock<Vfs> = crate::sync::once_lock::OnceLock::new();

/// Get the VFS instance (unified for all architectures).
///
/// Panics if the VFS has not been initialized via [`init`].
/// Prefer [`try_get_vfs`] in contexts where a panic is unacceptable.
/// Mount a tmpfs on /dev/shm, as Linux systems do: shm_open's objects and
/// any file there mapped MAP_SHARED share the file's pages (`fs::tmpfs`).
/// Called after every mount of /dev.
#[cfg(feature = "alloc")]
pub fn mount_dev_shm(vfs: &Vfs) {
    vfs.mount("/dev/shm".into(), Arc::new(tmpfs::TmpFs::half_of_ram()))
        .ok();
}

pub fn get_vfs() -> &'static Vfs {
    VFS.get()
        .expect("VFS not initialized: init() was not called")
}

/// Try to get the VFS instance without panicking
pub fn try_get_vfs() -> Option<&'static Vfs> {
    VFS.get()
}

/// Initialize the VFS with a RAM filesystem as root
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[VFS] Initializing Virtual Filesystem...");

    println!("[VFS] Creating VFS structure...");
    match VFS.set(Vfs::with_mounts(initial_mounts())) {
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
            vfs.mount_root(Arc::new(ramfs)).ok();
        }

        println!("[VFS] RAM filesystem mounted as root");

        // Create standard directories in root
        {
            let vfs = get_vfs();
            if let Some(root_fs) = vfs.root_fs() {
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
            if let Some(root_fs) = vfs.root_fs() {
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
            if let Some(root_fs) = vfs.root_fs() {
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
                            concat!(
                                "NAME=\"VeridianOS\"\nVERSION=\"",
                                env!("CARGO_PKG_VERSION"),
                                "\"\nID=veridian\nPRETTY_NAME=\"VeridianOS v",
                                env!("CARGO_PKG_VERSION"),
                                "\"\n"
                            )
                            .as_bytes(),
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
            vfs.mount("/dev".into(), Arc::new(devfs)).ok();
            mount_dev_shm(vfs);
        }

        println!("[VFS] Device filesystem mounted at /dev, tmpfs at /dev/shm");

        // Create ProcFS and mount at /proc
        println!("[VFS] Creating process filesystem...");
        let procfs = procfs::ProcFs::new();

        {
            let vfs = get_vfs();
            vfs.mount("/proc".into(), Arc::new(procfs)).ok();
        }

        println!("[VFS] Process filesystem mounted at /proc");

        // Create /proc/sys/kernel/ hierarchy for KDE/Qt compatibility.
        // Qt's KCrash module reads /proc/sys/kernel/core_pattern to decide
        // whether to install a crash handler. Without it, KCrash enters an
        // infinite retry loop that prevents kwin from starting.
        {
            let vfs = get_vfs();
            // ProcFS is mounted at /proc. Access its root node through
            // the mounts table.
            if let Some(proc_fs) = vfs.mounted("/proc") {
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
    if let Some(proc_fs) = vfs.mounted("/proc") {
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
    let vfs = get_vfs();

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
    let vfs = get_vfs();

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
    let vfs = get_vfs();
    vfs.resolve_path(path).is_ok()
}

/// Get file size without reading contents
pub fn file_size(path: &str) -> Result<usize, KernelError> {
    let vfs = get_vfs();
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
    let vfs = get_vfs();
    let node = vfs.resolve_path(path)?;
    let metadata = node.metadata()?;
    let current_size = metadata.size;

    // Write at the end of the file
    node.write(current_size, data)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Linux's device-number encoding (`new_encode_dev`): 12 bits of
    /// major, 20 of minor, both round-trip.
    #[test]
    fn device_numbers_use_linux_encoding() {
        assert_eq!(make_dev(1, 3), 0x103);
        assert_eq!(make_dev(8, 0x12345), 0x1230_0845);
        for (major, minor) in [(0, 0), (1, 3), (226, 128), (0xfff, 0xf_ffff), (8, 300)] {
            let dev = make_dev(major, minor);
            assert_eq!((dev_major(dev), dev_minor(dev)), (major, minor));
        }
    }

    /// mknod's file types, as Linux's `may_mknod`.
    #[test]
    fn mknod_types_follow_linux() {
        assert_eq!(mknod_kind(0o644), Ok(NodeType::File));
        assert_eq!(mknod_kind(0o100644), Ok(NodeType::File));
        assert_eq!(mknod_kind(0o010644), Ok(NodeType::Pipe));
        assert_eq!(mknod_kind(0o020600), Ok(NodeType::CharDevice));
        assert_eq!(mknod_kind(0o060600), Ok(NodeType::BlockDevice));
        assert_eq!(mknod_kind(0o140755), Ok(NodeType::Socket));
        assert_eq!(mknod_kind(0o040755), Err(true), "a directory: EPERM");
        assert_eq!(mknod_kind(0o120777), Err(false), "a symlink: EINVAL");
        assert_eq!(mknod_kind(0o030000), Err(false));
    }

    /// RamFS and tmpfs hold special files: the type, the device number of
    /// a device only, EEXIST for a taken name, and no data.
    #[test]
    fn ram_filesystems_hold_special_files() {
        let roots = [
            ramfs::RamFs::new().root(),
            tmpfs::TmpFs::new(1 << 20).root(),
        ];
        for root in roots {
            let perms = Permissions::from_mode(0o620);
            let dev = root
                .mknod("tty", NodeType::CharDevice, perms, make_dev(4, 1))
                .unwrap();
            assert_eq!(dev.device_id(), Some((4, 1)));
            assert!(dev.is_special_inode());
            assert_eq!(dev.metadata().unwrap().node_type, NodeType::CharDevice);
            let fifo = root
                .mknod("p", NodeType::Pipe, perms, make_dev(4, 1))
                .unwrap();
            assert_eq!(fifo.device_id(), None, "only a device has a number");
            assert!(fifo.write(0, b"x").is_err());
            assert!(matches!(
                root.mknod("p", NodeType::Socket, perms, 0),
                Err(KernelError::FsError(crate::error::FsError::AlreadyExists))
            ));
            assert!(root.mknod("d", NodeType::Directory, perms, 0).is_err());
            let types: Vec<_> = root
                .readdir()
                .unwrap()
                .into_iter()
                .map(|e| e.node_type)
                .collect();
            assert!(types.contains(&NodeType::Pipe) && types.contains(&NodeType::CharDevice));
            assert!(root.lookup("p").unwrap().is_special_inode());
        }
    }

    /// RLIMIT_FSIZE arithmetic (Linux's generic_write_check_limits).
    #[test]
    fn fsize_limit_shortens_or_refuses_writes() {
        assert_eq!(fsize_allowed(u64::MAX, 1 << 40, 4096), Some(4096));
        assert_eq!(fsize_allowed(1000, 0, 4096), Some(1000));
        assert_eq!(fsize_allowed(1000, 990, 4096), Some(10));
        assert_eq!(fsize_allowed(1000, 1000, 1), None);
        assert_eq!(fsize_allowed(1000, 2000, 1), None);
        assert_eq!(fsize_allowed(0, 0, 1), None);
    }

    /// FS-PERF-03: the same rename behaviour on every filesystem that
    /// implements it.
    fn rename_contract(fs: &dyn Filesystem) {
        let root = fs.root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        let b = root.mkdir("b", Permissions::default()).unwrap();
        let f = a.create("f", Permissions::from_mode(0o640)).unwrap();
        f.write(0, b"payload").unwrap();
        let _ = f.chown(Some(1000), Some(100));
        let inode = f.metadata().unwrap().inode;

        // File across directories: same node, data, mode and owner.
        a.rename("f", &b, "g").unwrap();
        assert!(a.lookup("f").is_err());
        let g = b.lookup("g").unwrap();
        let meta = g.metadata().unwrap();
        assert_eq!(meta.inode, inode);
        assert_eq!(meta.permissions.to_mode(), 0o640);
        let mut buf = [0u8; 7];
        assert_eq!(g.read(0, &mut buf).unwrap(), 7);
        assert_eq!(&buf, b"payload");

        // Same-node rename is a no-op; replacing follows POSIX.
        b.rename("g", &b, "g").unwrap();
        let h = b.create("h", Permissions::default()).unwrap();
        h.write(0, b"old").unwrap();
        b.rename("g", &b, "h").unwrap(); // file replaces file
        assert!(b.lookup("g").is_err());
        let mut buf = [0u8; 7];
        b.lookup("h").unwrap().read(0, &mut buf).unwrap();
        assert_eq!(&buf, b"payload");
        let d = b.mkdir("d", Permissions::default()).unwrap();
        assert!(b.rename("h", &b, "d").is_err(), "file over directory");
        assert!(b.rename("d", &b, "h").is_err(), "directory over file");
        d.create("x", Permissions::default()).unwrap();
        b.mkdir("e", Permissions::default()).unwrap();
        assert!(b.rename("e", &b, "d").is_err(), "over non-empty directory");
        assert!(b.rename(".", &b, "z").is_err());

        // Directory across parents: contents follow, ".." points at the
        // new parent.
        b.rename("d", &a, "moved").unwrap();
        let moved = a.lookup("moved").unwrap();
        assert!(moved.lookup("x").is_ok());
        let parent = moved
            .readdir()
            .unwrap()
            .into_iter()
            .find(|e| e.name == "..")
            .unwrap();
        assert_eq!(parent.inode, a.metadata().unwrap().inode);
        // An empty directory may replace an empty directory.
        a.mkdir("empty", Permissions::default()).unwrap();
        b.rename("e", &a, "empty").unwrap();
        assert!(b.lookup("e").is_err());
    }

    #[test]
    fn rename_on_ramfs_tmpfs_blockfs() {
        rename_contract(&ramfs::RamFs::new());
        rename_contract(&tmpfs::TmpFs::new(1 << 20));
        let bfs = blockfs::BlockFs::format(4096, 256).unwrap();
        rename_contract(&bfs);
    }

    #[test]
    fn blockfs_rename_refuses_own_subtree() {
        // Checked under the filesystem lock by walking "..", not only at
        // the syscall layer.
        let fs = blockfs::BlockFs::format(4096, 256).unwrap();
        let root = fs.root();
        let p = root.mkdir("p", Permissions::default()).unwrap();
        let q = p.mkdir("q", Permissions::default()).unwrap();
        let r = q.mkdir("r", Permissions::default()).unwrap();
        assert!(root.rename("p", &r, "loop").is_err());
        assert!(root.rename("p", &q, "loop").is_err());
        assert!(root.lookup("p").is_ok(), "nothing moved");
        // A legitimate move upwards still works.
        q.rename("r", &root, "r").unwrap();
    }

    #[test]
    fn blockfs_rename_onto_own_hard_link_keeps_both() {
        // Each BlockFS lookup returns a fresh node, so identity must be the
        // inode number, not the Arc (review of the v0.26.0 stack, PR #9).
        // POSIX: renaming one link onto another link of the same file does
        // nothing.
        let fs = blockfs::BlockFs::format(4096, 256).unwrap();
        let root = fs.root();
        let f = root.create("f", Permissions::default()).unwrap();
        f.write(0, b"data").unwrap();
        root.link("alias", root.lookup("f").unwrap()).unwrap();
        root.rename("alias", &root, "f").unwrap();
        root.rename("f", &root, "f").unwrap();
        let mut buf = [0u8; 4];
        root.lookup("f").unwrap().read(0, &mut buf).unwrap();
        assert_eq!(&buf, b"data");
        assert!(root.lookup("alias").is_ok());
        assert_eq!(
            root.lookup("f").unwrap().metadata().unwrap().inode,
            root.lookup("alias").unwrap().metadata().unwrap().inode
        );
    }

    #[test]
    fn rename_refuses_another_tmpfs_instance() {
        // Moving data between tmpfs mounts would bypass the destination's
        // size limit.
        let (x, y) = (tmpfs::TmpFs::new(1 << 20), tmpfs::TmpFs::new(16));
        x.root().create("big", Permissions::default()).unwrap();
        assert!(x.root().rename("big", &y.root(), "big").is_err());
        let d = x.root().mkdir("d", Permissions::default()).unwrap();
        // A directory cannot become its own parent.
        assert!(x.root().rename("d", &d, "d2").is_err());
    }

    #[test]
    fn rename_refuses_other_filesystems() {
        let (x, y) = (
            ramfs::RamFs::new(),
            blockfs::BlockFs::format(1024, 64).unwrap(),
        );
        x.root().create("f", Permissions::default()).unwrap();
        assert!(x.root().rename("f", &y.root(), "f").is_err());
    }

    #[test]
    fn permissions_mode_round_trip_includes_sticky() {
        for mode in [
            0o1777, 0o755, 0o1700, 0o644, 0o0, 0o4755, 0o2755, 0o6711, 0o7777,
        ] {
            assert_eq!(Permissions::from_mode(mode).to_mode(), mode);
        }
        let p = Permissions::from_mode(0o6755);
        assert!(p.set_uid && p.set_gid && !p.sticky);
    }

    /// fs::permission is Linux's generic_permission: the first class that
    /// applies decides from the mode bits, then the DAC capabilities
    /// override -- not uid 0 (ADR 0020).
    #[test]
    fn permission_is_mode_bits_then_capabilities() {
        use crate::process::{
            creds::{Caller, Credentials},
            linux_caps::{LinuxCaps, CAP_DAC_OVERRIDE, CAP_DAC_READ_SEARCH},
        };
        let meta = |node_type, mode, uid, gid| Metadata {
            node_type,
            size: 0,
            permissions: Permissions::from_mode(mode),
            uid,
            gid,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 1,
        };
        let who = |uid, caps: &[u32]| Caller {
            creds: Credentials::new(uid, uid),
            caps: caps.iter().fold(LinuxCaps::EMPTY, |s, &c| s.with(c)),
            userns: crate::process::userns::initial(),
        };
        let file = NodeType::File;
        let dir = NodeType::Directory;
        let user = who(1000, &[]);
        // The owner class alone decides for the owner, even if others may.
        assert!(!permission(&user, &meta(file, 0o077, 1000, 1000), MAY_READ));
        assert!(permission(&user, &meta(file, 0o640, 0, 1000), MAY_READ));
        assert!(!permission(&user, &meta(file, 0o640, 0, 1000), MAY_WRITE));
        assert!(permission(&user, &meta(file, 0o604, 0, 0), MAY_READ));
        // Root without capabilities is an ordinary user.
        let bare_root = who(0, &[]);
        assert!(!permission(
            &bare_root,
            &meta(file, 0o600, 1000, 1000),
            MAY_READ
        ));
        assert!(permission(
            &Caller::kernel(),
            &meta(file, 0o600, 1000, 1000),
            MAY_READ
        ));
        // CAP_DAC_OVERRIDE: read and write anything, execute only with an
        // execute bit; any directory.
        let dac = who(1000, &[CAP_DAC_OVERRIDE]);
        let secret = meta(file, 0o000, 0, 0);
        assert!(permission(&dac, &secret, MAY_READ | MAY_WRITE));
        assert!(!permission(&dac, &secret, MAY_EXEC));
        assert!(permission(&dac, &meta(file, 0o001, 0, 0), MAY_EXEC));
        assert!(permission(
            &dac,
            &meta(dir, 0o000, 0, 0),
            MAY_WRITE | MAY_EXEC
        ));
        // CAP_DAC_READ_SEARCH: read files, read and search directories.
        let rs = who(1000, &[CAP_DAC_READ_SEARCH]);
        assert!(permission(&rs, &secret, MAY_READ));
        assert!(!permission(&rs, &secret, MAY_READ | MAY_WRITE));
        assert!(permission(
            &rs,
            &meta(dir, 0o000, 0, 0),
            MAY_READ | MAY_EXEC
        ));
        assert!(!permission(&rs, &meta(dir, 0o000, 0, 0), MAY_WRITE));
        // In a user namespace they count for nodes whose owner and group it
        // maps (capable_wrt_inode_uidgid; ADR 0022 H4c).
        let ns = crate::process::userns::test_child(b"0 1000 1\n1 2000 1", b"0 1000 1");
        let ns_root = Caller {
            creds: Credentials::new(1000, 1000),
            caps: LinuxCaps::FULL,
            userns: ns,
        };
        assert!(permission(
            &ns_root,
            &meta(file, 0o000, 2000, 1000),
            MAY_READ | MAY_WRITE
        ));
        assert!(!permission(&ns_root, &meta(file, 0o600, 0, 0), MAY_READ));
        assert!(!permission(&ns_root, &meta(dir, 0o700, 0, 1000), MAY_EXEC));
    }

    /// A write, as a truncate or chown, removes a file's capabilities
    /// (cap_inode_killpriv), even the kernel's own write; other
    /// attributes stay.
    #[test]
    fn writes_remove_file_capabilities() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let f = root.create("prog", Permissions::from_mode(0o755)).unwrap();
        f.update_xattrs(&mut |a| {
            a.set(xattr::XATTR_NAME_CAPS, &[0; 20], 0)?;
            a.set("user.keep", b"1", 0)
        })
        .unwrap();
        remove_privs(&*f).unwrap();
        let attrs = f.xattrs().unwrap();
        assert!(attrs.get(xattr::XATTR_NAME_CAPS).is_none());
        assert_eq!(attrs.get("user.keep"), Some(&b"1"[..]));
        // Nothing to remove: no error, nothing changes.
        kill_file_caps(&*f).unwrap();
        assert_eq!(f.xattrs().unwrap().names().count(), 1);
    }

    /// A write(2) split into pieces removes the privileges once, before its
    /// first piece, as Linux's __generic_file_write_iter does; later pieces
    /// do not read the attributes again (PR #33 review).
    #[test]
    fn a_split_write_removes_privileges_with_its_first_piece() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let f = root.create("prog", Permissions::from_mode(0o755)).unwrap();
        let caps = |f: &Arc<dyn VfsNode>| f.xattrs().unwrap().get(xattr::XATTR_NAME_CAPS).is_some();
        let set_caps = |f: &Arc<dyn VfsNode>| {
            f.update_xattrs(&mut |a| a.set(xattr::XATTR_NAME_CAPS, &[0; 20], 0))
                .unwrap()
        };
        let file = file::File::new(f.clone(), file::OpenFlags::read_write());
        set_caps(&f);
        file.write_part(b"later", false).unwrap();
        assert!(caps(&f), "a later piece leaves the attribute alone");
        file.write_part(b"first", true).unwrap();
        assert!(!caps(&f), "the first piece removes it");
        set_caps(&f);
        file.write(b"whole").unwrap();
        assert!(!caps(&f), "a whole write removes it");
    }

    /// A file whose capabilities cannot be removed is not written: as
    /// Linux's file_remove_privs, the removal comes first and its error is
    /// the write's (PR #33 review). Delegates to a ramfs file; with
    /// `unreadable` the attribute read fails too.
    pub(crate) struct StuckCaps(pub(crate) Arc<dyn VfsNode>, pub(crate) bool);

    impl VfsNode for StuckCaps {
        fn node_type(&self) -> NodeType {
            self.0.node_type()
        }
        fn read(&self, o: usize, b: &mut [u8]) -> Result<usize, KernelError> {
            self.0.read(o, b)
        }
        fn write(&self, o: usize, d: &[u8]) -> Result<usize, KernelError> {
            self.0.write(o, d)
        }
        fn metadata(&self) -> Result<Metadata, KernelError> {
            self.0.metadata()
        }
        fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
            self.0.readdir()
        }
        fn lookup(&self, n: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.lookup(n)
        }
        fn create(&self, n: &str, p: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.create(n, p)
        }
        fn mkdir(&self, n: &str, p: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.mkdir(n, p)
        }
        fn unlink(&self, n: &str) -> Result<(), KernelError> {
            self.0.unlink(n)
        }
        fn truncate(&self, s: usize) -> Result<(), KernelError> {
            self.0.truncate(s)
        }
        fn chmod(&self, p: Permissions) -> Result<(), KernelError> {
            self.0.chmod(p)
        }
        fn chown(&self, u: Option<u32>, g: Option<u32>) -> Result<(), KernelError> {
            self.0.chown(u, g)
        }
        fn check_owner(&self, u: Option<u32>, g: Option<u32>) -> Result<(), KernelError> {
            self.0.check_owner(u, g)
        }
        fn xattrs(&self) -> Result<xattr::XattrSet, KernelError> {
            if self.1 {
                return Err(KernelError::FsError(FsError::IoError));
            }
            self.0.xattrs()
        }
        fn update_xattrs(
            &self,
            _f: &mut dyn FnMut(&mut xattr::XattrSet) -> Result<(), KernelError>,
        ) -> Result<(), KernelError> {
            Err(KernelError::FsError(FsError::IoError))
        }
    }

    #[test]
    fn a_write_that_cannot_drop_file_caps_fails_unwritten() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let inner = root.create("prog", Permissions::from_mode(0o755)).unwrap();
        inner
            .update_xattrs(&mut |a| a.set(xattr::XATTR_NAME_CAPS, &[0; 20], 0))
            .unwrap();
        let node: Arc<dyn VfsNode> = Arc::new(StuckCaps(inner.clone(), false));
        assert!(remove_privs(&*node).is_err());
        let file = file::File::new(node, OpenFlags::read_write());
        assert!(file.write(b"new").is_err());
        assert_eq!(inner.metadata().unwrap().size, 0);
        assert!(inner
            .xattrs()
            .unwrap()
            .get(xattr::XATTR_NAME_CAPS)
            .is_some());
        // Without file capabilities nothing needs removing: the write runs.
        let plain = root.create("data", Permissions::from_mode(0o644)).unwrap();
        let file = file::File::new(
            Arc::new(StuckCaps(plain.clone(), false)),
            OpenFlags::read_write(),
        );
        assert_eq!(file.write(b"new").unwrap(), 3);
    }

    /// A file whose capability another writer removes between
    /// kill_file_caps's check and its update (PR #33 review): the update
    /// finds the attribute already gone.
    struct RacedCaps(Arc<dyn VfsNode>);

    impl VfsNode for RacedCaps {
        fn node_type(&self) -> NodeType {
            self.0.node_type()
        }
        fn read(&self, o: usize, b: &mut [u8]) -> Result<usize, KernelError> {
            self.0.read(o, b)
        }
        fn write(&self, o: usize, d: &[u8]) -> Result<usize, KernelError> {
            self.0.write(o, d)
        }
        fn metadata(&self) -> Result<Metadata, KernelError> {
            self.0.metadata()
        }
        fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
            self.0.readdir()
        }
        fn lookup(&self, n: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.lookup(n)
        }
        fn create(&self, n: &str, p: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.create(n, p)
        }
        fn mkdir(&self, n: &str, p: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.mkdir(n, p)
        }
        fn unlink(&self, n: &str) -> Result<(), KernelError> {
            self.0.unlink(n)
        }
        fn truncate(&self, s: usize) -> Result<(), KernelError> {
            self.0.truncate(s)
        }
        fn xattrs(&self) -> Result<xattr::XattrSet, KernelError> {
            self.0.xattrs()
        }
        fn update_xattrs(
            &self,
            f: &mut dyn FnMut(&mut xattr::XattrSet) -> Result<(), KernelError>,
        ) -> Result<(), KernelError> {
            self.0
                .update_xattrs(&mut |a| a.remove(xattr::XATTR_NAME_CAPS))?;
            self.0.update_xattrs(f)
        }
    }

    #[test]
    fn a_capability_removed_meanwhile_does_not_fail_the_write() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let inner = root.create("prog", Permissions::from_mode(0o755)).unwrap();
        inner
            .update_xattrs(&mut |a| a.set(xattr::XATTR_NAME_CAPS, &[0; 20], 0))
            .unwrap();
        let node = RacedCaps(inner.clone());
        assert!(kill_file_caps(&node).is_ok());
        assert!(inner
            .xattrs()
            .unwrap()
            .get(xattr::XATTR_NAME_CAPS)
            .is_none());
    }

    /// An attribute read that fails is not taken for "no capability": the
    /// write stops, since the capability may still be there (PR #33 review).
    /// Only a filesystem without attributes has none to remove.
    #[test]
    fn a_write_whose_caps_cannot_be_read_fails_unwritten() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let inner = root.create("prog", Permissions::from_mode(0o755)).unwrap();
        let file = file::File::new(
            Arc::new(StuckCaps(inner.clone(), true)),
            OpenFlags::read_write(),
        );
        assert!(file.write(b"new").is_err());
        assert_eq!(inner.metadata().unwrap().size, 0);
    }

    /// Linux's file_remove_privs: a write without CAP_FSETID drops
    /// set-user-ID; set-group-ID goes with group execute or when the writer
    /// is outside the group, and stays as a locking mark otherwise.
    #[test]
    fn writes_drop_set_id_bits_without_cap_fsetid() {
        let suid = Permissions::from_mode(0o4755);
        assert_eq!(suid.after_write(true, false), None);
        assert_eq!(suid.after_write(false, true).unwrap().to_mode(), 0o755);
        let sgid = Permissions::from_mode(0o2755);
        assert_eq!(sgid.after_write(false, true).unwrap().to_mode(), 0o755);
        let lock_mark = Permissions::from_mode(0o2644);
        assert_eq!(lock_mark.after_write(false, true), None);
        assert_eq!(
            lock_mark.after_write(false, false).unwrap().to_mode(),
            0o644
        );
        assert_eq!(
            Permissions::from_mode(0o644).after_write(false, false),
            None
        );
    }

    /// A truncate removes the set-ID bits only once the size has changed,
    /// as Linux's shmem_setattr applies the mode after its seal check: a
    /// memfd sealed against shrinking keeps them (PR #33 review).
    #[test]
    fn a_refused_truncate_keeps_the_set_id_bits() {
        let user = crate::process::creds::Caller {
            creds: crate::process::creds::Credentials::new(1000, 1000),
            caps: crate::process::linux_caps::LinuxCaps::EMPTY,
            userns: crate::process::userns::initial(),
        };
        let m = memfd::MemfdNode::new(Permissions::from_mode(0o4755), 1000, 1000, true, 0);
        m.truncate(8192).unwrap();
        m.add_seals(seals::SHRINK).unwrap();
        assert!(truncate_removing_privs_as(&*m, 0, &user).is_err());
        assert_eq!(m.metadata().unwrap().permissions.to_mode(), 0o4755);
        assert_eq!(m.metadata().unwrap().size, 8192);
        truncate_removing_privs_as(&*m, 16384, &user).unwrap();
        assert_eq!(m.metadata().unwrap().permissions.to_mode(), 0o755);
    }

    /// chown drops set-user-ID always and set-group-ID with group execute.
    #[test]
    fn chown_drops_set_id_bits() {
        assert_eq!(
            Permissions::from_mode(0o6755)
                .after_chown()
                .unwrap()
                .to_mode(),
            0o755
        );
        assert_eq!(
            Permissions::from_mode(0o6744)
                .after_chown()
                .unwrap()
                .to_mode(),
            0o2744
        );
        assert_eq!(Permissions::from_mode(0o1777).after_chown(), None);
    }

    /// N-205: st_nlink counts names, as Linux's i_nlink: one per hard link
    /// of a file, 2 plus the subdirectories of a directory, 0 once removed.
    fn check_link_counts(fs: &dyn Filesystem) {
        let root = fs.root();
        let base = root.link_count();
        let f = root.create("f", Permissions::default()).unwrap();
        assert_eq!(f.link_count(), 1);
        root.link("g", f.clone()).unwrap();
        assert_eq!(f.link_count(), 2);
        let d = root.mkdir("d", Permissions::default()).unwrap();
        assert_eq!(d.link_count(), 2);
        assert_eq!(root.link_count(), base + 1);
        let e = d.mkdir("e", Permissions::default()).unwrap();
        assert_eq!(d.link_count(), 3);
        // A rename over a name of `f` takes that name away.
        let h = root.create("h", Permissions::default()).unwrap();
        root.rename("h", &root, "g").unwrap();
        assert_eq!((f.link_count(), h.link_count()), (1, 1));
        root.unlink("f").unwrap();
        assert_eq!(f.link_count(), 0);
        d.unlink("e").unwrap();
        assert_eq!((e.link_count(), d.link_count()), (0, 2));
    }

    #[test]
    fn link_counts_follow_names_on_every_tree_filesystem() {
        check_link_counts(&ramfs::RamFs::new());
        check_link_counts(&tmpfs::TmpFs::new(0));
        check_link_counts(&blockfs::BlockFs::format(1000, 100).unwrap());
    }

    /// O_TMPFILE's file (N-206): ramfs and tmpfs make one with no name,
    /// which a link names only if it was made linkable, and then never
    /// again once unnamed; BlockFS has none (EOPNOTSUPP).
    fn check_tmpfile(fs: &dyn Filesystem) {
        let root = fs.root();
        let t = root.tmpfile(Permissions::from_mode(0o600), true).unwrap();
        assert_eq!(t.node_type(), NodeType::File);
        assert_eq!(t.link_count(), 0);
        assert!(t.tmpfile_linkable());
        assert!(root.lookup("t").is_err());
        t.write(0, b"data").unwrap();
        root.link("t", t.clone()).unwrap();
        assert_eq!(t.link_count(), 1);
        assert!(!t.tmpfile_linkable());
        let mut buf = [0u8; 4];
        assert_eq!(root.lookup("t").unwrap().read(0, &mut buf).unwrap(), 4);
        assert_eq!(&buf, b"data");
        root.unlink("t").unwrap();
        assert!(t.link_count() == 0 && !t.tmpfile_linkable());
        let excl = root.tmpfile(Permissions::from_mode(0o600), false).unwrap();
        assert!(excl.link_count() == 0 && !excl.tmpfile_linkable());
        // Only a directory makes them.
        assert!(t.tmpfile(Permissions::from_mode(0o600), true).is_err());
    }

    #[test]
    fn tmpfiles_are_unnamed_until_linked() {
        check_tmpfile(&ramfs::RamFs::new());
        check_tmpfile(&tmpfs::TmpFs::new(0));
        let blockfs = blockfs::BlockFs::format(1000, 100).unwrap();
        assert!(matches!(
            blockfs.root().tmpfile(Permissions::from_mode(0o600), true),
            Err(KernelError::FsError(crate::error::FsError::NotSupported))
        ));
    }

    /// Helper: create a Vfs with a ramfs root filesystem already mounted.
    pub(crate) fn make_vfs_with_root() -> Vfs {
        let vfs = Vfs::new();
        let ramfs = Arc::new(ramfs::RamFs::new());
        vfs.mount_root(ramfs).expect("mount_root should succeed");
        vfs
    }

    // --- Path resolution (FS-SEC-01) ---

    #[test]
    fn a_trailing_slash_names_a_directory() {
        // N-203: "name/" is a directory, a symlink to one followed even
        // where the last component is not; anything else is ENOTDIR.
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let d = root.mkdir("d", Permissions::default()).unwrap();
        root.create("f", Permissions::default()).unwrap();
        root.symlink("dl", "d").unwrap();
        root.symlink("fl", "f").unwrap();
        let enotdir = Err(KernelError::FsError(FsError::NotADirectory));
        let ino_of =
            |r: Result<Arc<dyn VfsNode>, KernelError>| r.map(|n| n.metadata().unwrap().inode);
        assert_eq!(ino_of(vfs.resolve_path("/f/")), enotdir);
        assert_eq!(ino_of(vfs.resolve_path_no_follow("/f//")), enotdir);
        assert_eq!(ino_of(vfs.resolve_path_no_follow("/fl/")), enotdir);
        let dir = Ok(d.metadata().unwrap().inode);
        assert_eq!(ino_of(vfs.resolve_path_no_follow("/dl/")), dir);
        assert_eq!(ino_of(vfs.resolve_from_no_follow("dl/", "/")), dir);
        assert_eq!(ino_of(vfs.resolve_path("/d/")), dir);
        // Without the slash nothing changes.
        assert!(vfs.resolve_path_no_follow("/fl").is_ok());
        assert_eq!(
            vfs.resolve_path_no_follow("/dl").unwrap().node_type(),
            NodeType::Symlink
        );
        assert!(vfs.resolve_path("/").is_ok());
    }

    #[test]
    fn dotdot_applies_after_symlink_expansion() {
        // POSIX order (review of the v0.26.0 stack, PR #9): with
        // /a/link -> b/c, "/a/link/../x" is /a/b/x, not /a/x.
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        let b = a.mkdir("b", Permissions::default()).unwrap();
        b.mkdir("c", Permissions::default()).unwrap();
        b.create("x", Permissions::default()).unwrap();
        a.create("x", Permissions::default()).unwrap();
        a.symlink("link", "b/c").unwrap();
        let (_, canonical) = vfs.resolve_canonical("/a/link/../x", "/", true).unwrap();
        assert_eq!(canonical, "/a/b/x");
        // An absolute target restarts at the root.
        a.symlink("abs", "/a/b/c").unwrap();
        let (_, canonical) = vfs.resolve_canonical("/a/abs/../x", "/", true).unwrap();
        assert_eq!(canonical, "/a/b/x");
    }

    /// chroot (N-250): under root /jail, absolute paths, `..` and
    /// absolute symlink targets stay inside it.
    #[test]
    fn lookups_under_a_root_never_leave_it() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.create("secret", Permissions::default()).unwrap();
        let jail = root.mkdir("jail", Permissions::default()).unwrap();
        let etc = jail.mkdir("etc", Permissions::default()).unwrap();
        etc.create("passwd", Permissions::default()).unwrap();
        jail.symlink("out", "/secret").unwrap();
        jail.symlink("up", "../../secret").unwrap();
        let table = vfs.snapshot();
        let r = |path: &str, cwd: &str| {
            table
                .resolve_in(path, cwd, "/jail", true, 0, None, 0)
                .map(|(_, canon)| canon)
        };
        assert_eq!(r("/etc/passwd", "/").unwrap(), "/jail/etc/passwd");
        assert_eq!(r("/../../etc/passwd", "/").unwrap(), "/jail/etc/passwd");
        assert_eq!(r("passwd", "/etc").unwrap(), "/jail/etc/passwd");
        assert_eq!(r("..", "/").unwrap(), "/jail");
        // /secret exists outside the jail only.
        assert!(r("/secret", "/").is_err());
        assert!(r("/out", "/").is_err());
        assert!(r("/up", "/").is_err());
        assert_eq!(root_relative("/jail/etc/passwd", "/jail"), "/etc/passwd");
        assert_eq!(root_relative("/jail", "/jail"), "/");
        assert_eq!(root_relative("/jailbreak", "/jail"), "/jailbreak");
        assert_eq!(root_relative("/a/b", "/"), "/a/b");
    }

    #[test]
    fn dotdot_through_missing_or_file_component_fails() {
        // Each component before ".." must exist and be a directory; a
        // lexical "/a/missing/.." used to resolve to "/a".
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        a.create("file", Permissions::default()).unwrap();
        assert!(vfs.resolve_path("/a/missing/..").is_err());
        assert!(vfs.resolve_path("/a/file/..").is_err());
        assert!(vfs.resolve_path("/a/./../a").is_ok());
        // ".." above the root stays at the root.
        let (_, canonical) = vfs.resolve_canonical("/../../a", "/", true).unwrap();
        assert_eq!(canonical, "/a");
    }

    #[test]
    fn walk_requires_search_permission_on_every_directory() {
        // Review of the v0.26.0 stack, PR #9: the walk never checked the
        // execute bit, so a mode-0700 directory hid nothing below it.
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let private = root
            .mkdir("private", Permissions::from_mode(0o700))
            .unwrap();
        let open = private
            .mkdir("open", Permissions::from_mode(0o755))
            .unwrap();
        open.create("f", Permissions::from_mode(0o644)).unwrap();
        let table = vfs.snapshot();
        let other = Some(crate::process::creds::Caller {
            creds: crate::process::creds::Credentials::new(1000, 1000),
            caps: crate::process::linux_caps::LinuxCaps::EMPTY,
            userns: crate::process::userns::initial(),
        });
        // Root-owned 0700: another user cannot pass through it, even to a
        // world-readable file, and not by ".." either.
        assert!(matches!(
            table.resolve("/private/open/f", "/", true, 0, other.clone()),
            Err(KernelError::FsError(
                crate::error::FsError::PermissionDenied
            ))
        ));
        assert!(table
            .resolve("/private/..", "/", true, 0, other.clone())
            .is_err());
        // The directory itself can still be named (its parent is searchable).
        assert!(table
            .resolve("/private", "/", true, 0, other.clone())
            .is_ok());
        // Kernel context and root are unaffected.
        assert!(table.resolve("/private/open/f", "/", true, 0, None).is_ok());
        // Execute without read is enough to pass through a directory.
        let x_only = root.mkdir("xonly", Permissions::from_mode(0o711)).unwrap();
        x_only.create("g", Permissions::from_mode(0o644)).unwrap();
        assert!(table
            .resolve("/xonly/g", "/", true, 0, other.clone())
            .is_ok());
    }

    #[test]
    fn symlink_loop_is_detected() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.symlink("p", "q").unwrap();
        root.symlink("q", "p").unwrap();
        assert_eq!(
            vfs.resolve_path("/p").err(),
            Some(KernelError::FsError(crate::error::FsError::SymlinkLoop))
        );
    }

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
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
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
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.mkdir("mnt", Permissions::default()).unwrap();
        root.create("top", Permissions::default()).unwrap();
        vfs.mount(String::from("/mnt"), Arc::new(ramfs::RamFs::new()))
            .unwrap();
        assert!(vfs.resolve_path("/mnt/../top").is_ok());
    }

    #[test]
    fn entry_mount_point_follows_symlinked_parent() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.mkdir("mnt", Permissions::default()).unwrap();
        vfs.mount(String::from("/mnt"), Arc::new(ramfs::RamFs::new()))
            .unwrap();
        root.symlink("link", "/mnt").unwrap();
        // The path as written is fooled by the symlink...
        assert_eq!(vfs.mount_point_of("/link/g"), "/");
        // ...the canonical parent is not.
        assert_eq!(vfs.entry_mount_point("/link", "g").unwrap(), "/mnt");
        assert_eq!(vfs.entry_mount_point("/", "f").unwrap(), "/");
        // A mount point belongs to its own mount, so renaming it stays EXDEV.
        assert_eq!(vfs.entry_mount_point("/", "mnt").unwrap(), "/mnt");
    }

    /// A view as a thread chrooted to "/jail" would have it.
    fn jailed(vfs: &Vfs) -> CallerView<'_> {
        CallerView {
            vfs,
            ctx: PathContext {
                cwd: String::from("/"),
                root: String::from("/jail"),
                creds: None,
                pid: 0,
                resolve: 0,
                mounts: vfs.mounts.clone(),
            },
        }
    }

    #[test]
    fn kernel_lookups_ignore_the_callers_root() {
        // N-251: a kernel subsystem (the audit log) resolving "/var/log/x"
        // while a chrooted thread was current resolved it inside the
        // chroot, into a file the process controls.
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.create("log", Permissions::default()).unwrap();
        let jail = root.mkdir("jail", Permissions::default()).unwrap();
        jail.create("log", Permissions::default()).unwrap();
        let real = root.lookup("log").unwrap().metadata().unwrap().inode;
        let fake = jail.lookup("log").unwrap().metadata().unwrap().inode;

        let kernel = vfs.resolve_path("/log").unwrap();
        assert_eq!(kernel.metadata().unwrap().inode, real);
        let (_, canonical) = vfs
            .open_follow_canonical("/log", OpenFlags::read_only(), true)
            .unwrap();
        assert_eq!(canonical, "/log");

        let caller = jailed(&vfs).resolve_path("/log").unwrap();
        assert_eq!(caller.metadata().unwrap().inode, fake);
        let (_, canonical) = jailed(&vfs)
            .open_follow_canonical("/log", OpenFlags::read_only(), true)
            .unwrap();
        assert_eq!(canonical, "/jail/log");
        // ".." stops at the caller's root.
        assert_eq!(
            jailed(&vfs)
                .resolve_path("/../../log")
                .unwrap()
                .metadata()
                .unwrap()
                .inode,
            fake
        );
    }

    #[test]
    fn kernel_context_has_no_caller_identity() {
        let vfs = make_vfs_with_root();
        let ctx = vfs.kernel_context();
        assert_eq!(ctx.root, "/");
        assert!(ctx.creds.is_none());
        assert_eq!(ctx.pid, 0);
    }

    #[test]
    fn caller_mounts_are_placed_inside_its_root() {
        // A chrooted mount("/") named the real root (and failed as
        // already mounted); it must mount over the chroot's "/".
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let jail = root.mkdir("jail", Permissions::default()).unwrap();
        jail.mkdir("mnt", Permissions::default()).unwrap();
        jail.create("file", Permissions::default()).unwrap();

        jailed(&vfs).mount_by_type("/mnt", "ramfs", 0).unwrap();
        assert_eq!(vfs.mount_point_of("/jail/mnt/x"), "/jail/mnt");
        assert_eq!(vfs.mount_point_of("/mnt"), "/");
        // Not a directory, and not there at all.
        assert!(jailed(&vfs).mount_by_type("/file", "ramfs", 0).is_err());
        assert!(jailed(&vfs).mount_by_type("/none", "ramfs", 0).is_err());

        jailed(&vfs).unmount("/mnt").unwrap();
        assert_eq!(vfs.mount_point_of("/jail/mnt/x"), "/");
    }

    #[test]
    fn relative_symlink_resolves_from_its_directory() {
        // Before the fix a relative target was resolved from "/".
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        a.create("target", Permissions::default()).unwrap();
        a.symlink("link", "target").unwrap();
        let (_, canonical) = vfs.resolve_canonical("/a/link", "/", true).unwrap();
        assert_eq!(canonical, "/a/target");
    }

    #[test]
    fn canonical_path_follows_intermediate_symlink() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
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
        let root = vfs.root_fs().unwrap().root();
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
        let root = vfs.root_fs().unwrap().root();
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

    // --- Mount table snapshots (FS-PERF-02) ---

    /// A filesystem whose `sync` mounts another filesystem on the VFS that
    /// is syncing it, i.e. VFS work running inside a VFS operation.
    struct MountsOnSync {
        vfs: &'static Vfs,
        inner: ramfs::RamFs,
    }

    impl Filesystem for MountsOnSync {
        fn root(&self) -> Arc<dyn VfsNode> {
            self.inner.root()
        }
        fn name(&self) -> &str {
            "mounts-on-sync"
        }
        fn is_readonly(&self) -> bool {
            false
        }
        fn sync(&self) -> Result<(), KernelError> {
            self.vfs
                .mount("/late".into(), Arc::new(ramfs::RamFs::new()))
        }
    }

    #[test]
    fn vfs_operations_hold_no_lock_across_filesystem_calls() {
        // With the old global RwLock<Vfs>, `sync` ran under the read lock
        // and this mount needed the write lock: a self-deadlock.
        let vfs: &'static Vfs = alloc::boxed::Box::leak(alloc::boxed::Box::new(Vfs::new()));
        vfs.mount_root(Arc::new(MountsOnSync {
            vfs,
            inner: ramfs::RamFs::new(),
        }))
        .unwrap();
        vfs.sync().unwrap();
        assert!(vfs.mounted("/late").is_some());
    }

    #[test]
    fn walk_in_progress_keeps_its_mount_table() {
        let vfs = Vfs::new();
        vfs.mount_root(Arc::new(ramfs::RamFs::new())).unwrap();
        let tmp = Arc::new(ramfs::RamFs::new());
        tmp.root().create("f", Permissions::default()).unwrap();
        vfs.mount("/tmp".into(), tmp).unwrap();

        let before = vfs.snapshot();
        vfs.unmount("/tmp").unwrap();
        // The old snapshot still sees the mount; new walks do not.
        assert!(before.resolve("/tmp/f", "/", true, 0, None).is_ok());
        assert!(vfs.resolve_path("/tmp/f").is_err());
        assert!(vfs.mounted("/tmp").is_none());
    }

    // --- Mount tests ---

    #[test]
    fn mounts_take_the_lowest_free_id_and_a_new_unique_id() {
        let vfs = make_vfs_with_root();
        let (root, at_root) = vfs.mount_ids_of("/");
        assert!(at_root);
        assert_eq!(root.id, 1);
        assert!(root.unique > PSEUDO_MOUNT.unique);
        vfs.mount("/a".into(), Arc::new(ramfs::RamFs::new()))
            .unwrap();
        let (a, a_root) = vfs.mount_ids_of("/a");
        assert!(a_root);
        assert_eq!(a.id, 2);
        assert!(a.unique > root.unique);
        // Below a mount point, and beside one with the same prefix.
        assert_eq!(vfs.mount_ids_of("/a/x/y"), (a, false));
        assert_eq!(vfs.mount_ids_of("/ab"), (root, false));
        // An unmounted id is reused; a unique id never is.
        vfs.unmount("/a").unwrap();
        vfs.mount("/b".into(), Arc::new(ramfs::RamFs::new()))
            .unwrap();
        let (b, _) = vfs.mount_ids_of("/b");
        assert_eq!(b.id, 2);
        assert!(b.unique > a.unique);
        // A new root is a new mount.
        vfs.swap_root(Arc::new(ramfs::RamFs::new()));
        let (new_root, _) = vfs.mount_ids_of("/");
        assert_eq!(new_root.id, 3);
        assert!(new_root.unique > b.unique);
    }

    #[test]
    fn without_a_root_everything_is_the_pseudo_mount() {
        assert_eq!(Vfs::new().mount_ids_of("/x"), (PSEUDO_MOUNT, false));
    }

    // --- File handles (group 3 step D4) ---

    #[test]
    fn find_by_inode_walks_one_filesystem() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let b = root
            .mkdir("a", Permissions::default())
            .unwrap()
            .mkdir("b", Permissions::default())
            .unwrap();
        let f = b.create("f", Permissions::default()).unwrap();
        let f_ino = f.metadata().unwrap().inode;
        // A file under a mount point is hidden by the mount.
        let hidden = root
            .mkdir("m", Permissions::default())
            .unwrap()
            .create("hidden", Permissions::default())
            .unwrap();
        let hidden_ino = hidden.metadata().unwrap().inode;
        let tmp = Arc::new(tmpfs::TmpFs::new(1 << 20));
        let g = tmp.root().create("g", Permissions::default()).unwrap();
        let g_ino = g.metadata().unwrap().inode;
        vfs.mount("/m".into(), tmp).unwrap();

        let (node, path) = vfs.find_by_inode("/", f_ino).unwrap();
        assert!(Arc::ptr_eq(&node, &f));
        assert_eq!(path, "/a/b/f");
        let (_, path) = vfs
            .find_by_inode("/", root.metadata().unwrap().inode)
            .unwrap();
        assert_eq!(path, "/");
        let (node, path) = vfs.find_by_inode("/m", g_ino).unwrap();
        assert!(Arc::ptr_eq(&node, &g));
        assert_eq!(path, "/m/g");
        assert!(vfs
            .find_by_inode("/", hidden_ino)
            .is_none_or(|(n, _)| !Arc::ptr_eq(&n, &hidden)));
        // Unlinked, the file has no name to be found by.
        b.unlink("f").unwrap();
        assert!(vfs.find_by_inode("/", f_ino).is_none());
    }

    #[test]
    fn filesystem_of_names_the_serving_mount() {
        let vfs = make_vfs_with_root();
        vfs.mount("/m".into(), Arc::new(tmpfs::TmpFs::new(1 << 20)))
            .unwrap();
        let (mount, fs) = vfs.filesystem_of("/m/x/y").unwrap();
        assert_eq!((mount.as_str(), fs.name()), ("/m", "tmpfs"));
        let (mount, fs) = vfs.filesystem_of("/mx").unwrap();
        assert_eq!((mount.as_str(), fs.name()), ("/", "ramfs"));
        assert!(fs.exportable());
        assert!(Vfs::new().filesystem_of("/").is_none());
    }

    // --- openat2 RESOLVE_* flags (group 3 step D3b) ---

    /// A procfs stand-in: delegates to a ramfs node, and its child `exe`
    /// is a magic link, as /proc/self/exe.
    struct Magic(Arc<dyn VfsNode>);

    impl VfsNode for Magic {
        fn node_type(&self) -> NodeType {
            self.0.node_type()
        }
        fn read(&self, o: usize, b: &mut [u8]) -> Result<usize, KernelError> {
            self.0.read(o, b)
        }
        fn write(&self, o: usize, d: &[u8]) -> Result<usize, KernelError> {
            self.0.write(o, d)
        }
        fn metadata(&self) -> Result<Metadata, KernelError> {
            self.0.metadata()
        }
        fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
            self.0.readdir()
        }
        fn lookup(&self, n: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0
                .lookup(n)
                .map(|c| Arc::new(Magic(c)) as Arc<dyn VfsNode>)
        }
        fn create(&self, n: &str, p: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.create(n, p)
        }
        fn mkdir(&self, n: &str, p: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
            self.0.mkdir(n, p)
        }
        fn unlink(&self, n: &str) -> Result<(), KernelError> {
            self.0.unlink(n)
        }
        fn truncate(&self, s: usize) -> Result<(), KernelError> {
            self.0.truncate(s)
        }
        fn readlink(&self) -> Result<String, KernelError> {
            self.0.readlink()
        }
        fn is_magic_link(&self) -> bool {
            self.0.node_type() == NodeType::Symlink
        }
    }

    struct MagicFs(ramfs::RamFs);

    impl Filesystem for MagicFs {
        fn root(&self) -> Arc<dyn VfsNode> {
            Arc::new(Magic(self.0.root()))
        }
        fn name(&self) -> &str {
            "magic"
        }
        fn is_readonly(&self) -> bool {
            false
        }
        fn sync(&self) -> Result<(), KernelError> {
            Ok(())
        }
    }

    /// / with /top, /d/{f, sub/, rel -> f, abs -> /d/f, up -> ../top,
    /// inroot -> /f, tomnt -> /mnt/m}, a ramfs on /mnt with
    /// /mnt/{m, back -> /top}, and a magic filesystem on /proc with
    /// /proc/exe -> /top.
    fn resolve_tree() -> Vfs {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let p = Permissions::default();
        root.create("top", p).unwrap();
        root.mkdir("mnt", p).unwrap();
        root.mkdir("proc", p).unwrap();
        let d = root.mkdir("d", p).unwrap();
        d.create("f", p).unwrap();
        d.mkdir("sub", p).unwrap();
        d.symlink("rel", "f").unwrap();
        d.symlink("abs", "/d/f").unwrap();
        d.symlink("up", "../top").unwrap();
        d.symlink("inroot", "/f").unwrap();
        d.symlink("tomnt", "/mnt/m").unwrap();
        let mnt = Arc::new(ramfs::RamFs::new());
        mnt.root().create("m", p).unwrap();
        mnt.root().symlink("back", "/top").unwrap();
        vfs.mount("/mnt".into(), mnt).unwrap();
        let proc_fs = MagicFs(ramfs::RamFs::new());
        proc_fs.0.root().symlink("exe", "/top").unwrap();
        vfs.mount("/proc".into(), Arc::new(proc_fs)).unwrap();
        vfs
    }

    /// The caller's view (root "/") with `resolve` from `start`.
    fn resolving<'a>(vfs: &'a Vfs, resolve: u32, start: &str) -> CallerView<'a> {
        CallerView {
            vfs,
            ctx: PathContext {
                cwd: String::from("/"),
                root: String::from("/"),
                creds: None,
                pid: 0,
                resolve: 0,
                mounts: vfs.mounts.clone(),
            },
        }
        .resolving(resolve, start)
    }

    fn ino(vfs: &Vfs, path: &str) -> u64 {
        vfs.resolve_path(path).unwrap().metadata().unwrap().inode
    }

    fn ino_of(r: Result<Arc<dyn VfsNode>, KernelError>) -> Result<u64, KernelError> {
        r.map(|n| n.metadata().unwrap().inode)
    }

    const EXDEV: KernelError = KernelError::FsError(FsError::CrossDevice);
    const ELOOP: KernelError = KernelError::FsError(FsError::SymlinkLoop);

    #[test]
    fn resolve_beneath_stays_below_the_start() {
        use resolve_flags::BENEATH;
        let vfs = resolve_tree();
        let f = ino(&vfs, "/d/f");
        let view = resolving(&vfs, BENEATH, "/d");
        assert_eq!(ino_of(view.resolve_path("f")), Ok(f));
        assert_eq!(ino_of(view.resolve_path("sub/../f")), Ok(f));
        assert_eq!(ino_of(view.resolve_path("rel")), Ok(f));
        // Out by "..", by an absolute path, by an absolute symlink and by
        // a relative symlink that climbs.
        assert_eq!(ino_of(view.resolve_path("../top")), Err(EXDEV));
        assert_eq!(ino_of(view.resolve_path("/d/f")), Err(EXDEV));
        assert_eq!(ino_of(view.resolve_path("abs")), Err(EXDEV));
        assert_eq!(ino_of(view.resolve_path("up")), Err(EXDEV));
        // Without the flag the same walks leave /d.
        let plain = resolving(&vfs, 0, "/d");
        assert_eq!(ino_of(plain.resolve_path("../top")), Ok(ino(&vfs, "/top")));
        assert_eq!(ino_of(plain.resolve_path("abs")), Ok(f));
    }

    #[test]
    fn resolve_in_root_treats_the_start_as_root() {
        let vfs = resolve_tree();
        let f = ino(&vfs, "/d/f");
        let view = resolving(&vfs, resolve_flags::IN_ROOT, "/d");
        assert_eq!(ino_of(view.resolve_path("/f")), Ok(f));
        assert_eq!(ino_of(view.resolve_path("../../f")), Ok(f));
        assert_eq!(ino_of(view.resolve_path("inroot")), Ok(f));
        // "/d/f" inside the new root is /d/d/f.
        assert_eq!(
            ino_of(view.resolve_path("abs")),
            Err(KernelError::FsError(FsError::NotFound))
        );
    }

    #[test]
    fn resolve_no_xdev_refuses_every_mount_crossing() {
        use resolve_flags::NO_XDEV;
        let vfs = resolve_tree();
        // Entering a mount, by name or through a symlink.
        assert_eq!(
            ino_of(resolving(&vfs, NO_XDEV, "/").resolve_path("mnt/m")),
            Err(EXDEV)
        );
        assert_eq!(
            ino_of(resolving(&vfs, NO_XDEV, "/d").resolve_path("tomnt")),
            Err(EXDEV)
        );
        // Inside one mount; leaving it by ".."; an absolute symlink read on
        // another mount than the root's.
        let mnt = resolving(&vfs, NO_XDEV, "/mnt");
        assert_eq!(ino_of(mnt.resolve_path("m")), Ok(ino(&vfs, "/mnt/m")));
        assert_eq!(ino_of(mnt.resolve_path("../top")), Err(EXDEV));
        assert_eq!(ino_of(mnt.resolve_path("back")), Err(EXDEV));
        // An absolute path starts at the root, whatever the start.
        assert_eq!(ino_of(mnt.resolve_path("/d/f")), Ok(ino(&vfs, "/d/f")));
        // An absolute symlink on the root's mount to the root's mount.
        assert_eq!(
            ino_of(resolving(&vfs, NO_XDEV, "/d").resolve_path("abs")),
            Ok(ino(&vfs, "/d/f"))
        );
    }

    #[test]
    fn resolve_no_symlinks_refuses_following_any() {
        let vfs = resolve_tree();
        let view = resolving(&vfs, resolve_flags::NO_SYMLINKS, "/d");
        assert_eq!(ino_of(view.resolve_path("f")), Ok(ino(&vfs, "/d/f")));
        assert_eq!(ino_of(view.resolve_path("rel")), Err(ELOOP));
        assert_eq!(ino_of(view.resolve_path("/proc/exe")), Err(ELOOP));
        // A final symlink that is not followed is not refused.
        assert!(view.resolve_path_no_follow("rel").is_ok());
    }

    #[test]
    fn resolve_magic_links() {
        use resolve_flags::{BENEATH, IN_ROOT, NO_MAGICLINKS, NO_XDEV};
        let vfs = resolve_tree();
        let top = ino(&vfs, "/top");
        assert_eq!(
            ino_of(resolving(&vfs, 0, "/").resolve_path("/proc/exe")),
            Ok(top)
        );
        assert_eq!(
            ino_of(resolving(&vfs, NO_MAGICLINKS, "/").resolve_path("/proc/exe")),
            Err(ELOOP)
        );
        // An ordinary symlink is still followed.
        assert_eq!(
            ino_of(resolving(&vfs, NO_MAGICLINKS, "/d").resolve_path("rel")),
            Ok(ino(&vfs, "/d/f"))
        );
        // A scoped lookup cannot follow one; NO_XDEV cannot when it leaves
        // the link's mount.
        for flags in [BENEATH, IN_ROOT, NO_XDEV] {
            assert_eq!(
                ino_of(resolving(&vfs, flags, "/proc").resolve_path("exe")),
                Err(EXDEV),
                "flags {:#x}",
                flags
            );
        }
    }

    #[test]
    fn resolve_flags_apply_to_the_parent_of_a_new_entry() {
        let vfs = resolve_tree();
        let p = Permissions::default();
        let beneath = resolving(&vfs, resolve_flags::BENEATH, "/d");
        assert!(beneath.mkdir("sub/new", p).is_ok());
        assert!(vfs.resolve_path("/d/sub/new").is_ok());
        // ".." is walked, not normalized away.
        assert_eq!(beneath.mkdir("../escape", p).err(), Some(EXDEV));
        assert_eq!(beneath.mkdir("/d/abs_new", p).err(), Some(EXDEV));
        assert!(vfs.resolve_path("/escape").is_err());
        // A name of its own: a relative one starts at the start.
        assert!(beneath.mkdir("plain", p).is_ok());
        assert!(vfs.resolve_path("/d/plain").is_ok());
        assert_eq!(
            resolving(&vfs, resolve_flags::NO_XDEV, "/")
                .mkdir("mnt/x", p)
                .err(),
            Some(EXDEV)
        );
    }

    #[test]
    fn a_scoped_view_inside_a_chroot_nests_the_roots() {
        let vfs = resolve_tree();
        let mut view = jailed(&vfs);
        view = view.resolving(resolve_flags::BENEATH, "/sub");
        assert_eq!(view.get_root(), "/jail/sub");
        assert_eq!(view.get_cwd(), "/");
        let view = resolving(&vfs, resolve_flags::IN_ROOT, "/");
        assert_eq!(view.get_root(), "/");
        let view = resolving(&vfs, resolve_flags::NO_XDEV, "/d");
        assert_eq!((view.get_root(), view.get_cwd()), ("/".into(), "/d".into()));
    }

    #[test]
    fn test_mount_root() {
        let vfs = Vfs::new();
        let ramfs = Arc::new(ramfs::RamFs::new());
        let result = vfs.mount_root(ramfs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_root_twice_fails() {
        let vfs = Vfs::new();
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
        let vfs = Vfs::new();
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
        let vfs = make_vfs_with_root();
        let devfs = Arc::new(devfs::DevFs::new());
        let result = vfs.mount("/dev".into(), devfs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_duplicate_path_fails() {
        let vfs = make_vfs_with_root();
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
        let vfs = make_vfs_with_root();
        let fs = Arc::new(ramfs::RamFs::new());
        vfs.mount("/mnt".into(), fs).unwrap();

        let result = vfs.unmount("/mnt");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unmount_nonexistent_fails() {
        let vfs = make_vfs_with_root();
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
        let vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/tmp", "ramfs", 0);
        assert!(result.is_ok());
    }

    /// /dev/shm is a tmpfs mounted on devfs's `shm` directory, and a file
    /// there maps its own pages (`fs::tmpfs`). A caller can unmount it and
    /// mount it again, which needs the directory (`mount_target`; PR #28
    /// review).
    #[test]
    fn test_mount_tmpfs_on_dev_shm() {
        let vfs = make_vfs_with_root();
        vfs.mount_by_type("/dev", "devfs", 0).unwrap();
        vfs.mount_by_type("/dev/shm", "tmpfs", 0).unwrap();
        let shm = vfs.resolve_path("/dev/shm").unwrap();
        assert_eq!(
            shm.metadata().unwrap().permissions.to_mode() & 0o7777,
            0o1777
        );
        let f = shm.create("seg", Permissions::from_mode(0o600)).unwrap();
        f.truncate(4096).unwrap();
        let frames = f.share_pages(0, 1, true).expect("tmpfs pages").unwrap();
        // The mapping's owner goes back; the file keeps the page.
        assert!(!crate::mm::frame_refs::release(frames[0]));

        let caller = CallerView {
            vfs: &vfs,
            ctx: PathContext {
                cwd: String::from("/"),
                root: String::from("/"),
                creds: None,
                pid: 0,
                resolve: 0,
                mounts: vfs.mounts.clone(),
            },
        };
        caller.unmount("/dev/shm").unwrap();
        caller.mount_by_type("/dev/shm", "tmpfs", 0).unwrap();
        assert!(vfs.resolve_path("/dev/shm/seg").is_err(), "a fresh tmpfs");
    }

    #[test]
    fn test_mount_by_type_devfs() {
        let vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/dev", "devfs", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_by_type_procfs() {
        let vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/proc", "procfs", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mount_by_type_unknown_fails() {
        let vfs = make_vfs_with_root();
        let result = vfs.mount_by_type("/foo", "unknownfs", 0);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(crate::error::FsError::UnknownFsType)
        );
    }

    #[test]
    fn test_mount_by_type_root() {
        let vfs = Vfs::new();
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
        let root = vfs.root_fs().unwrap().root();
        root.mkdir("testdir", Permissions::default()).unwrap();

        let result = vfs.resolve_path("/testdir");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().node_type(), NodeType::Directory);
    }

    #[test]
    fn test_resolve_nested_path() {
        let vfs = make_vfs_with_root();

        let root = vfs.root_fs().unwrap().root();
        let sub = root.mkdir("a", Permissions::default()).unwrap();
        sub.mkdir("b", Permissions::default()).unwrap();

        let result = vfs.resolve_path("/a/b");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().node_type(), NodeType::Directory);
    }

    #[test]
    fn test_resolve_path_with_dot() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.mkdir("mydir", Permissions::default()).unwrap();

        // "." should be ignored in path traversal
        let result = vfs.resolve_path("/./mydir/.");
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_path_with_dotdot() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
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
        let root = vfs.root_fs().unwrap().root();
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
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        root.mkdir("home", Permissions::default()).unwrap();

        let result = vfs.set_cwd(String::from("/home"));
        assert!(result.is_ok());
        assert_eq!(vfs.get_cwd(), "/home");
    }

    #[test]
    fn test_set_cwd_not_directory_fails() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
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

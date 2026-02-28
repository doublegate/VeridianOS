//! File system operations for VeridianOS.
//!
//! Provides both low-level syscall wrappers (matching the existing API) and
//! higher-level types that mirror `std::fs`:
//!
//! - `File` -- an owned file descriptor with read/write/seek/metadata
//! - `OpenOptions` -- builder pattern for file opening
//! - `Metadata` / `FileType` / `Permissions` -- stat-derived information
//! - `ReadDir` / `DirEntry` -- directory iteration
//! - Free functions: `create_dir`, `remove_dir`, `remove_file`, `rename`,
//!   `symlink`, `readlink`, `hard_link`, `metadata`, `set_permissions`
//!
//! Syscall mappings:
//! - `open`    -> SYS_FILE_OPEN (50)
//! - `close`   -> SYS_FILE_CLOSE (51)
//! - `read`    -> SYS_FILE_READ (52)
//! - `write`   -> SYS_FILE_WRITE (53)
//! - `seek`    -> SYS_FILE_SEEK (54)
//! - `stat`    -> SYS_FILE_STAT (55)
//! - `unlink`  -> SYS_FILE_UNLINK (157)
//! - `rename`  -> SYS_FILE_RENAME (154)
//! - `link`    -> SYS_FILE_LINK (155)
//! - `symlink` -> SYS_FILE_SYMLINK (156)
//! - `readlink`-> SYS_FILE_READLINK (152)
//! - `mkdir`   -> SYS_DIR_MKDIR (60)
//! - `rmdir`   -> SYS_DIR_RMDIR (61)
//! - `opendir` -> SYS_DIR_OPENDIR (62)
//! - `readdir` -> SYS_DIR_READDIR (63)
//! - `closedir`-> SYS_DIR_CLOSEDIR (64)
//! - `fsync`   -> SYS_FS_FSYNC (73)

extern crate alloc;
use alloc::vec::Vec;

use super::{
    fd::SharedFd,
    path::{OsStr, OsString, Path, PathBuf},
    syscall1, syscall2, syscall3, syscall_result, SyscallError, SYS_DIR_CLOSEDIR, SYS_DIR_MKDIR,
    SYS_DIR_OPENDIR, SYS_DIR_READDIR, SYS_DIR_RMDIR, SYS_FILE_CLOSE, SYS_FILE_DUP, SYS_FILE_DUP2,
    SYS_FILE_LINK, SYS_FILE_OPEN, SYS_FILE_PIPE, SYS_FILE_READ, SYS_FILE_READLINK, SYS_FILE_RENAME,
    SYS_FILE_SEEK, SYS_FILE_STAT, SYS_FILE_STAT_PATH, SYS_FILE_SYMLINK, SYS_FILE_TRUNCATE,
    SYS_FILE_UNLINK, SYS_FILE_WRITE, SYS_FS_FSYNC,
};

// ============================================================================
// Open flags (must match kernel/toolchain definitions)
// ============================================================================

/// Open for reading only.
pub const O_RDONLY: usize = 0;
/// Open for writing only.
pub const O_WRONLY: usize = 1;
/// Open for reading and writing.
pub const O_RDWR: usize = 2;
/// Access mode mask (low 2 bits).
pub const O_ACCMODE: usize = 3;
/// Create file if it does not exist.
pub const O_CREAT: usize = 0x40;
/// Error if O_CREAT and file already exists.
pub const O_EXCL: usize = 0x80;
/// Do not set the file as controlling terminal.
pub const O_NOCTTY: usize = 0x100;
/// Truncate file to zero length.
pub const O_TRUNC: usize = 0x200;
/// Append on each write.
pub const O_APPEND: usize = 0x400;
/// Non-blocking mode.
pub const O_NONBLOCK: usize = 0x800;
/// Open as directory (error if not a directory).
pub const O_DIRECTORY: usize = 0x10000;
/// Do not follow symbolic links.
pub const O_NOFOLLOW: usize = 0x20000;
/// Close-on-exec flag.
pub const O_CLOEXEC: usize = 0x80000;

// ============================================================================
// Seek whence values
// ============================================================================

/// Seek from beginning of file.
pub const SEEK_SET: usize = 0;
/// Seek from current position.
pub const SEEK_CUR: usize = 1;
/// Seek from end of file.
pub const SEEK_END: usize = 2;

// ============================================================================
// Mode bits
// ============================================================================

/// File type mask.
pub const S_IFMT: u32 = 0o170000;
/// Regular file.
pub const S_IFREG: u32 = 0o100000;
/// Directory.
pub const S_IFDIR: u32 = 0o040000;
/// Symbolic link.
pub const S_IFLNK: u32 = 0o120000;
/// Character device.
pub const S_IFCHR: u32 = 0o020000;
/// Block device.
pub const S_IFBLK: u32 = 0o060000;
/// FIFO (named pipe).
pub const S_IFIFO: u32 = 0o010000;
/// Socket.
pub const S_IFSOCK: u32 = 0o140000;

/// Owner read.
pub const S_IRUSR: u32 = 0o400;
/// Owner write.
pub const S_IWUSR: u32 = 0o200;
/// Owner execute.
pub const S_IXUSR: u32 = 0o100;
/// Group read.
pub const S_IRGRP: u32 = 0o040;
/// Group write.
pub const S_IWGRP: u32 = 0o020;
/// Group execute.
pub const S_IXGRP: u32 = 0o010;
/// Others read.
pub const S_IROTH: u32 = 0o004;
/// Others write.
pub const S_IWOTH: u32 = 0o002;
/// Others execute.
pub const S_IXOTH: u32 = 0o001;

// ============================================================================
// Stat structure (matches kernel layout)
// ============================================================================

/// File status information.
///
/// Layout must match the kernel's stat structure passed via syscall.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_nsec: i64,
    pub st_mtime: i64,
    pub st_mtime_nsec: i64,
    pub st_ctime: i64,
    pub st_ctime_nsec: i64,
}

// ============================================================================
// DirEntry structure for readdir (matches kernel layout)
// ============================================================================

/// Kernel directory entry structure for SYS_DIR_READDIR.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KernelDirent {
    /// Inode number.
    pub d_ino: u64,
    /// File type (DT_REG, DT_DIR, etc.).
    pub d_type: u8,
    /// Length of name (excluding NUL).
    pub d_namlen: u8,
    /// Padding for alignment.
    pub _pad: [u8; 6],
    /// Null-terminated file name (up to 255 bytes + NUL).
    pub d_name: [u8; 256],
}

impl Default for KernelDirent {
    fn default() -> Self {
        KernelDirent {
            d_ino: 0,
            d_type: 0,
            d_namlen: 0,
            _pad: [0; 6],
            d_name: [0; 256],
        }
    }
}

/// Directory entry file type constants.
pub const DT_UNKNOWN: u8 = 0;
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_BLK: u8 = 6;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;
pub const DT_SOCK: u8 = 12;

// ============================================================================
// Low-level syscall wrappers (original API preserved)
// ============================================================================

/// Open a file.
///
/// # Arguments
/// - `path`: Null-terminated path string
/// - `flags`: Open flags (O_RDONLY, O_WRONLY, O_RDWR, O_CREAT, etc.)
/// - `mode`: File permissions (used with O_CREAT)
///
/// # Returns
/// File descriptor on success, error on failure.
pub fn open(path: *const u8, flags: usize, mode: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_FILE_OPEN, path as usize, flags, mode) };
    syscall_result(ret)
}

/// Close a file descriptor.
pub fn close(fd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_FILE_CLOSE, fd) };
    syscall_result(ret)
}

/// Read from a file descriptor.
pub fn read(fd: usize, buf: *mut u8, count: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_FILE_READ, fd, buf as usize, count) };
    syscall_result(ret)
}

/// Write to a file descriptor.
pub fn write(fd: usize, buf: *const u8, count: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_FILE_WRITE, fd, buf as usize, count) };
    syscall_result(ret)
}

/// Convenience: write a byte slice to a file descriptor.
pub fn write_bytes(fd: usize, data: &[u8]) -> Result<usize, SyscallError> {
    write(fd, data.as_ptr(), data.len())
}

/// Seek within a file.
pub fn seek(fd: usize, offset: isize, whence: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_FILE_SEEK, fd, offset as usize, whence) };
    syscall_result(ret)
}

/// Get file status by file descriptor.
pub fn fstat(fd: usize, stat_buf: *mut Stat) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_STAT, fd, stat_buf as usize) };
    syscall_result(ret)
}

/// Get file status by path.
pub fn stat(path: *const u8, stat_buf: *mut Stat) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_STAT_PATH, path as usize, stat_buf as usize) };
    syscall_result(ret)
}

/// Truncate a file to a specified length.
pub fn ftruncate(fd: usize, length: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_TRUNCATE, fd, length) };
    syscall_result(ret)
}

/// Duplicate a file descriptor.
pub fn dup(oldfd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_FILE_DUP, oldfd) };
    syscall_result(ret)
}

/// Duplicate a file descriptor to a specific fd number.
pub fn dup2(oldfd: usize, newfd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_DUP2, oldfd, newfd) };
    syscall_result(ret)
}

/// Create a pipe.
pub fn pipe(pipefd: *mut [i32; 2]) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_FILE_PIPE, pipefd as usize) };
    syscall_result(ret)
}

/// Unlink (delete) a file.
pub fn unlink(path: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_FILE_UNLINK, path as usize) };
    syscall_result(ret)
}

/// Rename a file.
pub fn rename(oldpath: *const u8, newpath: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_RENAME, oldpath as usize, newpath as usize) };
    syscall_result(ret)
}

/// Create a hard link.
pub fn link(oldpath: *const u8, newpath: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_LINK, oldpath as usize, newpath as usize) };
    syscall_result(ret)
}

/// Create a symbolic link.
pub fn symlink(target: *const u8, linkpath: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_FILE_SYMLINK, target as usize, linkpath as usize) };
    syscall_result(ret)
}

/// Read the target of a symbolic link.
pub fn readlink(path: *const u8, buf: *mut u8, bufsiz: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_FILE_READLINK, path as usize, buf as usize, bufsiz) };
    syscall_result(ret)
}

/// Sync a file descriptor to disk.
pub fn fsync(fd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_FS_FSYNC, fd) };
    syscall_result(ret)
}

// ============================================================================
// Directory Operations
// ============================================================================

/// Create a directory.
pub fn mkdir(path: *const u8, mode: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_DIR_MKDIR, path as usize, mode) };
    syscall_result(ret)
}

/// Remove a directory.
pub fn rmdir(path: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_DIR_RMDIR, path as usize) };
    syscall_result(ret)
}

/// Open a directory for reading.
pub fn opendir(path: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_DIR_OPENDIR, path as usize) };
    syscall_result(ret)
}

/// Read one entry from an open directory.
///
/// Returns the number of entries read (0 or 1).
pub fn readdir_raw(
    dirfd: usize,
    entry: *mut KernelDirent,
    count: usize,
) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_DIR_READDIR, dirfd, entry as usize, count) };
    syscall_result(ret)
}

/// Close an open directory.
pub fn closedir(dirfd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_DIR_CLOSEDIR, dirfd) };
    syscall_result(ret)
}

// ============================================================================
// FileType
// ============================================================================

/// The type of a file (regular, directory, symlink, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link.
    Symlink,
    /// Character device.
    CharDevice,
    /// Block device.
    BlockDevice,
    /// FIFO (named pipe).
    Fifo,
    /// Unix domain socket.
    Socket,
    /// Unknown or unrecognised type.
    Unknown,
}

impl FileType {
    /// Determine file type from the `st_mode` field of a `Stat`.
    pub fn from_mode(mode: u32) -> Self {
        match mode & S_IFMT {
            S_IFREG => FileType::File,
            S_IFDIR => FileType::Dir,
            S_IFLNK => FileType::Symlink,
            S_IFCHR => FileType::CharDevice,
            S_IFBLK => FileType::BlockDevice,
            S_IFIFO => FileType::Fifo,
            S_IFSOCK => FileType::Socket,
            _ => FileType::Unknown,
        }
    }

    /// Determine file type from a `d_type` byte (readdir).
    pub fn from_dirent_type(d_type: u8) -> Self {
        match d_type {
            DT_REG => FileType::File,
            DT_DIR => FileType::Dir,
            DT_LNK => FileType::Symlink,
            DT_CHR => FileType::CharDevice,
            DT_BLK => FileType::BlockDevice,
            DT_FIFO => FileType::Fifo,
            DT_SOCK => FileType::Socket,
            _ => FileType::Unknown,
        }
    }

    pub fn is_file(self) -> bool {
        self == FileType::File
    }
    pub fn is_dir(self) -> bool {
        self == FileType::Dir
    }
    pub fn is_symlink(self) -> bool {
        self == FileType::Symlink
    }
}

// ============================================================================
// Permissions
// ============================================================================

/// File permissions (Unix mode bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    mode: u32,
}

impl Permissions {
    /// Create from raw mode bits (only the permission bits are retained).
    pub fn from_mode(mode: u32) -> Self {
        Permissions {
            mode: mode & 0o7777,
        }
    }

    /// Return the raw permission mode bits.
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Is the file read-only (no write bits set)?
    pub fn readonly(&self) -> bool {
        (self.mode & (S_IWUSR | S_IWGRP | S_IWOTH)) == 0
    }

    /// Set or clear the read-only flag.
    ///
    /// When `readonly` is `true`, all write bits are cleared.
    /// When `false`, owner write is added.
    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            self.mode &= !(S_IWUSR | S_IWGRP | S_IWOTH);
        } else {
            self.mode |= S_IWUSR;
        }
    }
}

// ============================================================================
// Metadata
// ============================================================================

/// File metadata obtained from `stat` / `fstat`.
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    stat: Stat,
}

impl Metadata {
    /// Create from a raw `Stat` structure.
    pub fn from_stat(stat: Stat) -> Self {
        Metadata { stat }
    }

    /// File type.
    pub fn file_type(&self) -> FileType {
        FileType::from_mode(self.stat.st_mode)
    }

    /// Is this a regular file?
    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }

    /// Is this a directory?
    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }

    /// Is this a symbolic link?
    pub fn is_symlink(&self) -> bool {
        self.file_type().is_symlink()
    }

    /// File size in bytes.
    pub fn len(&self) -> u64 {
        self.stat.st_size as u64
    }

    /// Is the file empty?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// File permissions.
    pub fn permissions(&self) -> Permissions {
        Permissions::from_mode(self.stat.st_mode)
    }

    /// Last modification time (seconds since UNIX epoch).
    pub fn modified_secs(&self) -> i64 {
        self.stat.st_mtime
    }

    /// Last modification time nanosecond component.
    pub fn modified_nsec(&self) -> i64 {
        self.stat.st_mtime_nsec
    }

    /// Last access time (seconds since UNIX epoch).
    pub fn accessed_secs(&self) -> i64 {
        self.stat.st_atime
    }

    /// Creation / status-change time (seconds since UNIX epoch).
    pub fn created_secs(&self) -> i64 {
        self.stat.st_ctime
    }

    /// Inode number.
    pub fn ino(&self) -> u64 {
        self.stat.st_ino
    }

    /// Device ID.
    pub fn dev(&self) -> u64 {
        self.stat.st_dev
    }

    /// Number of hard links.
    pub fn nlink(&self) -> u32 {
        self.stat.st_nlink
    }

    /// Owner UID.
    pub fn uid(&self) -> u32 {
        self.stat.st_uid
    }

    /// Owner GID.
    pub fn gid(&self) -> u32 {
        self.stat.st_gid
    }

    /// Block size for I/O.
    pub fn blksize(&self) -> i64 {
        self.stat.st_blksize
    }

    /// Number of 512-byte blocks allocated.
    pub fn blocks(&self) -> i64 {
        self.stat.st_blocks
    }

    /// Access the raw `Stat` structure.
    pub fn raw_stat(&self) -> &Stat {
        &self.stat
    }
}

// ============================================================================
// OpenOptions
// ============================================================================

/// Builder for configuring how a file is opened.
///
/// Mirrors `std::fs::OpenOptions`.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    mode: u32,
}

impl OpenOptions {
    /// Create a new set of options with everything turned off.
    pub fn new() -> Self {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
            mode: 0o666,
        }
    }

    /// Open for reading.
    pub fn read(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    /// Open for writing.
    pub fn write(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    /// Append to the file.
    pub fn append(&mut self, append: bool) -> &mut Self {
        self.append = append;
        self
    }

    /// Truncate the file to zero length on open.
    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    /// Create the file if it does not exist.
    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    /// Create the file, failing if it already exists.
    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    /// Set the file permission mode (used with create).
    pub fn mode(&mut self, mode: u32) -> &mut Self {
        self.mode = mode;
        self
    }

    /// Compute the flags integer for `SYS_FILE_OPEN`.
    fn flags(&self) -> usize {
        let mut flags = if self.read && self.write {
            O_RDWR
        } else if self.write || self.append {
            O_WRONLY
        } else {
            O_RDONLY
        };
        if self.append {
            flags |= O_APPEND;
        }
        if self.truncate {
            flags |= O_TRUNC;
        }
        if self.create_new {
            flags |= O_CREAT | O_EXCL;
        } else if self.create {
            flags |= O_CREAT;
        }
        flags
    }

    /// Open the file at the given path.
    pub fn open(&self, path: &Path) -> Result<File, SyscallError> {
        let c_path = path.to_cstring();
        let fd = open(c_path.as_ptr(), self.flags(), self.mode as usize)?;
        // SAFETY: `open` returned a valid new fd.
        Ok(File {
            fd: unsafe { SharedFd::from_raw(fd) },
        })
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// File
// ============================================================================

/// An open file on the filesystem.
///
/// The file is automatically closed when dropped (via `SharedFd`).
pub struct File {
    fd: SharedFd,
}

impl File {
    /// Open a file at the given path for reading.
    pub fn open(path: &Path) -> Result<File, SyscallError> {
        OpenOptions::new().read(true).open(path)
    }

    /// Create (or truncate) a file at the given path for writing.
    pub fn create(path: &Path) -> Result<File, SyscallError> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
    }

    /// Wrap an existing raw file descriptor.
    ///
    /// # Safety
    /// The caller must own the fd.
    pub unsafe fn from_raw_fd(fd: usize) -> File {
        File {
            fd: unsafe { SharedFd::from_raw(fd) },
        }
    }

    /// Return the raw file descriptor number.
    pub fn raw_fd(&self) -> usize {
        self.fd.raw()
    }

    /// Read bytes into `buf`.  Returns the number of bytes read.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        self.fd.read(buf)
    }

    /// Read all available bytes until EOF.
    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize, SyscallError> {
        let mut total = 0;
        let mut tmp = [0u8; 4096];
        loop {
            let n = self.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            total += n;
        }
        Ok(total)
    }

    /// Write bytes from `data`.  Returns the number of bytes written.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        self.fd.write(data)
    }

    /// Write all bytes, retrying on partial writes.
    pub fn write_all(&self, data: &[u8]) -> Result<(), SyscallError> {
        let mut written = 0;
        while written < data.len() {
            let n = self.write(&data[written..])?;
            if n == 0 {
                return Err(SyscallError::InvalidState);
            }
            written += n;
        }
        Ok(())
    }

    /// Seek to a position.
    pub fn seek(&self, offset: isize, whence: usize) -> Result<usize, SyscallError> {
        seek(self.fd.raw(), offset, whence)
    }

    /// Seek to an absolute position from the start.
    pub fn seek_start(&self, pos: u64) -> Result<u64, SyscallError> {
        seek(self.fd.raw(), pos as isize, SEEK_SET).map(|v| v as u64)
    }

    /// Seek relative to current position.
    pub fn seek_current(&self, offset: i64) -> Result<u64, SyscallError> {
        seek(self.fd.raw(), offset as isize, SEEK_CUR).map(|v| v as u64)
    }

    /// Seek relative to end of file.
    pub fn seek_end(&self, offset: i64) -> Result<u64, SyscallError> {
        seek(self.fd.raw(), offset as isize, SEEK_END).map(|v| v as u64)
    }

    /// Get file metadata via `fstat`.
    pub fn metadata(&self) -> Result<Metadata, SyscallError> {
        let mut st = Stat::default();
        fstat(self.fd.raw(), &mut st)?;
        Ok(Metadata::from_stat(st))
    }

    /// Truncate (or extend) the file to `size` bytes.
    pub fn set_len(&self, size: u64) -> Result<(), SyscallError> {
        ftruncate(self.fd.raw(), size as usize)?;
        Ok(())
    }

    /// Flush all data and metadata to the underlying storage.
    pub fn sync_all(&self) -> Result<(), SyscallError> {
        fsync(self.fd.raw())?;
        Ok(())
    }

    /// Duplicate the file descriptor.
    pub fn try_clone(&self) -> Result<File, SyscallError> {
        Ok(File {
            fd: self.fd.try_clone()?,
        })
    }
}

impl core::fmt::Debug for File {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File").field("fd", &self.fd).finish()
    }
}

// ============================================================================
// DirEntry
// ============================================================================

/// An entry in a directory.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Inode number.
    ino: u64,
    /// File type.
    file_type: FileType,
    /// File name (not full path).
    name: OsString,
    /// Parent directory path for constructing full path.
    dir_path: PathBuf,
}

impl DirEntry {
    /// The file name of this entry (no path prefix).
    pub fn file_name(&self) -> &OsStr {
        self.name.as_os_str()
    }

    /// The file type, if available.
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// The full path to this entry.
    pub fn path(&self) -> PathBuf {
        self.dir_path
            .as_path()
            .join(Path::from_os_str(self.file_name()))
    }

    /// Get metadata for this entry (follows symlinks).
    pub fn metadata(&self) -> Result<Metadata, SyscallError> {
        metadata(&self.path())
    }

    /// Inode number.
    pub fn ino(&self) -> u64 {
        self.ino
    }
}

// ============================================================================
// ReadDir
// ============================================================================

/// Iterator over the entries in a directory.
pub struct ReadDir {
    /// Kernel directory handle from `opendir`.
    dirfd: usize,
    /// Directory path (for constructing DirEntry paths).
    path: PathBuf,
    /// Has the iterator reached the end?
    done: bool,
}

impl ReadDir {
    /// Open a directory for iteration.
    pub fn new(path: &Path) -> Result<Self, SyscallError> {
        let c_path = path.to_cstring();
        let dirfd = opendir(c_path.as_ptr())?;
        Ok(ReadDir {
            dirfd,
            path: path.to_path_buf(),
            done: false,
        })
    }
}

impl Iterator for ReadDir {
    type Item = Result<DirEntry, SyscallError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let mut dirent = KernelDirent::default();
        match readdir_raw(self.dirfd, &mut dirent, 1) {
            Ok(0) => {
                self.done = true;
                None
            }
            Ok(_) => {
                let name_len = dirent.d_namlen as usize;
                let name = OsString::from_vec(dirent.d_name[..name_len].to_vec());
                Some(Ok(DirEntry {
                    ino: dirent.d_ino,
                    file_type: FileType::from_dirent_type(dirent.d_type),
                    name,
                    dir_path: self.path.clone(),
                }))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        let _ = closedir(self.dirfd);
    }
}

// ============================================================================
// Free functions (high-level API)
// ============================================================================

/// Get metadata for a path (follows symlinks).
pub fn metadata(path: &Path) -> Result<Metadata, SyscallError> {
    let c_path = path.to_cstring();
    let mut st = Stat::default();
    stat(c_path.as_ptr(), &mut st)?;
    Ok(Metadata::from_stat(st))
}

/// Read the contents of a directory.
pub fn read_dir(path: &Path) -> Result<ReadDir, SyscallError> {
    ReadDir::new(path)
}

/// Create a directory at the given path.
pub fn create_dir(path: &Path) -> Result<(), SyscallError> {
    let c_path = path.to_cstring();
    mkdir(c_path.as_ptr(), 0o755)?;
    Ok(())
}

/// Create a directory and all parent directories as needed.
pub fn create_dir_all(path: &Path) -> Result<(), SyscallError> {
    if path.as_bytes().is_empty() {
        return Ok(());
    }
    // Try creating directly first -- if it works or already exists, done.
    match create_dir(path) {
        Ok(()) => return Ok(()),
        Err(SyscallError::FileExists) => return Ok(()),
        Err(SyscallError::ResourceNotFound) => {
            // Parent missing -- recursively create it.
        }
        Err(e) => return Err(e),
    }
    if let Some(parent) = path.parent() {
        if !parent.as_bytes().is_empty() {
            create_dir_all(parent)?;
        }
    }
    match create_dir(path) {
        Ok(()) | Err(SyscallError::FileExists) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove a directory (must be empty).
pub fn remove_dir(path: &Path) -> Result<(), SyscallError> {
    let c_path = path.to_cstring();
    rmdir(c_path.as_ptr())?;
    Ok(())
}

/// Remove a file.
pub fn remove_file(path: &Path) -> Result<(), SyscallError> {
    let c_path = path.to_cstring();
    unlink(c_path.as_ptr())?;
    Ok(())
}

/// Rename a file or directory.
pub fn rename_path(from: &Path, to: &Path) -> Result<(), SyscallError> {
    let c_from = from.to_cstring();
    let c_to = to.to_cstring();
    rename(c_from.as_ptr(), c_to.as_ptr())?;
    Ok(())
}

/// Create a hard link.
pub fn hard_link(src: &Path, dst: &Path) -> Result<(), SyscallError> {
    let c_src = src.to_cstring();
    let c_dst = dst.to_cstring();
    link(c_src.as_ptr(), c_dst.as_ptr())?;
    Ok(())
}

/// Create a symbolic link.
pub fn symlink_path(target: &Path, link_path: &Path) -> Result<(), SyscallError> {
    let c_target = target.to_cstring();
    let c_link = link_path.to_cstring();
    symlink(c_target.as_ptr(), c_link.as_ptr())?;
    Ok(())
}

/// Read the target of a symbolic link.
pub fn read_link(path: &Path) -> Result<PathBuf, SyscallError> {
    let c_path = path.to_cstring();
    let mut buf = [0u8; 4096];
    let len = readlink(c_path.as_ptr(), buf.as_mut_ptr(), buf.len())?;
    Ok(PathBuf::from_vec(buf[..len].to_vec()))
}

/// Read the entire contents of a file into a byte vector.
pub fn read_file(path: &Path) -> Result<Vec<u8>, SyscallError> {
    let file = File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Write a byte slice to a file (create or truncate).
pub fn write_file(path: &Path, data: &[u8]) -> Result<(), SyscallError> {
    let file = File::create(path)?;
    file.write_all(data)
}

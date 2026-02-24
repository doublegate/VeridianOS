//! File descriptors and file operations

use alloc::{string::String, sync::Arc, vec::Vec};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::VfsNode;
use crate::error::{FsError, KernelError};

/// File descriptor number
pub type FileDescriptor = usize;

/// Standard file descriptors
pub const STDIN: FileDescriptor = 0;
pub const STDOUT: FileDescriptor = 1;
pub const STDERR: FileDescriptor = 2;

/// File open flags
#[derive(Debug, Clone, Copy)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub create: bool,
    pub truncate: bool,
    pub exclusive: bool,
}

impl OpenFlags {
    /// Read-only mode
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            append: false,
            create: false,
            truncate: false,
            exclusive: false,
        }
    }

    /// Write-only mode
    pub fn write_only() -> Self {
        Self {
            read: false,
            write: true,
            append: false,
            create: true,
            truncate: true,
            exclusive: false,
        }
    }

    /// Read-write mode
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            append: false,
            create: true,
            truncate: false,
            exclusive: false,
        }
    }

    /// Append mode
    pub fn append() -> Self {
        Self {
            read: false,
            write: true,
            append: true,
            create: true,
            truncate: false,
            exclusive: false,
        }
    }

    /// Create from bits (for syscall interface)
    ///
    /// Flag values MUST match `<veridian/fcntl.h>` in the sysroot -- that is
    /// the ABI contract user-space programs (including GCC) are compiled
    /// against.
    pub fn from_bits(bits: u32) -> Option<Self> {
        // VeridianOS ABI flags (from veridian/fcntl.h in sysroot)
        const O_RDONLY: u32 = 0x0001;
        const O_WRONLY: u32 = 0x0002;
        const O_RDWR: u32 = 0x0003;
        const O_ACCMODE: u32 = 0x0003;
        const O_CREAT: u32 = 0x0100;
        const O_TRUNC: u32 = 0x0200;
        const O_APPEND: u32 = 0x0400;
        const O_EXCL: u32 = 0x0800;

        let access_mode = bits & O_ACCMODE;

        Some(Self {
            read: access_mode == O_RDONLY || access_mode == O_RDWR,
            write: access_mode == O_WRONLY || access_mode == O_RDWR,
            append: (bits & O_APPEND) != 0,
            create: (bits & O_CREAT) != 0,
            truncate: (bits & O_TRUNC) != 0,
            exclusive: (bits & O_EXCL) != 0,
        })
    }
}

/// Seek position
#[derive(Debug, Clone, Copy)]
pub enum SeekFrom {
    Start(usize),
    Current(isize),
    End(isize),
}

/// Open file structure
pub struct File {
    /// VFS node this file refers to
    pub node: Arc<dyn VfsNode>,

    /// Open flags
    pub flags: OpenFlags,

    /// Current position in file
    pub position: RwLock<usize>,

    /// Reference count
    pub refcount: RwLock<usize>,

    /// Absolute path this file was opened with (for dirfd resolution in *at
    /// syscalls)
    pub path: Option<String>,
}

impl File {
    /// Create a new file structure
    pub fn new(node: Arc<dyn VfsNode>, flags: OpenFlags) -> Self {
        Self {
            node,
            flags,
            position: RwLock::new(0),
            refcount: RwLock::new(1),
            path: None,
        }
    }

    /// Create a new file structure with a known path
    pub fn new_with_path(node: Arc<dyn VfsNode>, flags: OpenFlags, path: String) -> Self {
        Self {
            node,
            flags,
            position: RwLock::new(0),
            refcount: RwLock::new(1),
            path: Some(path),
        }
    }

    /// Read from the file
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if !self.flags.read {
            return Err(KernelError::PermissionDenied {
                operation: "read file not opened for reading",
            });
        }

        let mut pos = self.position.write();
        let bytes_read = self.node.read(*pos, buffer)?;
        *pos += bytes_read;
        Ok(bytes_read)
    }

    /// Write to the file
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        if !self.flags.write {
            return Err(KernelError::PermissionDenied {
                operation: "write file not opened for writing",
            });
        }

        let mut pos = self.position.write();

        if self.flags.append {
            // For append mode, always write at end
            let metadata = self.node.metadata()?;
            *pos = metadata.size;
        }

        let bytes_written = self.node.write(*pos, data)?;
        *pos += bytes_written;
        Ok(bytes_written)
    }

    /// Seek to a position in the file
    pub fn seek(&self, from: SeekFrom) -> Result<usize, KernelError> {
        let mut pos = self.position.write();

        let new_pos = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset < 0 {
                    pos.checked_sub((-offset) as usize)
                        .ok_or(KernelError::InvalidArgument {
                            name: "offset",
                            value: "seek before start of file",
                        })?
                } else {
                    pos.checked_add(offset as usize)
                        .ok_or(KernelError::InvalidArgument {
                            name: "offset",
                            value: "seek overflow",
                        })?
                }
            }
            SeekFrom::End(offset) => {
                let metadata = self.node.metadata()?;
                if offset < 0 {
                    metadata.size.checked_sub((-offset) as usize).ok_or(
                        KernelError::InvalidArgument {
                            name: "offset",
                            value: "seek before start of file",
                        },
                    )?
                } else {
                    metadata.size.checked_add(offset as usize).ok_or(
                        KernelError::InvalidArgument {
                            name: "offset",
                            value: "seek overflow",
                        },
                    )?
                }
            }
        };

        *pos = new_pos;
        Ok(new_pos)
    }

    /// Get current position
    pub fn tell(&self) -> usize {
        *self.position.read()
    }

    /// Increment reference count
    pub fn inc_ref(&self) {
        *self.refcount.write() += 1;
    }

    /// Decrement reference count
    pub fn dec_ref(&self) -> usize {
        let mut count = self.refcount.write();
        *count = count.saturating_sub(1);
        *count
    }
}

/// File descriptor entry with flags
pub struct FileEntry {
    /// The file itself
    pub file: Arc<File>,
    /// Close-on-exec flag
    pub cloexec: bool,
}

/// File descriptor table for a process
pub struct FileTable {
    /// File descriptors
    files: RwLock<Vec<Option<FileEntry>>>,

    /// Next available file descriptor
    next_fd: RwLock<FileDescriptor>,
}

impl FileTable {
    /// Create a new file table
    pub fn new() -> Self {
        let mut files = Vec::with_capacity(256);

        // Reserve standard file descriptors
        files.push(None); // stdin
        files.push(None); // stdout
        files.push(None); // stderr

        Self {
            files: RwLock::new(files),
            next_fd: RwLock::new(3),
        }
    }
}

impl Default for FileTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTable {
    /// Open a file and return a file descriptor
    pub fn open(&self, file: Arc<File>) -> Result<FileDescriptor, KernelError> {
        self.open_with_flags(file, false)
    }

    /// Open a file with close-on-exec flag and return a file descriptor
    pub fn open_with_flags(
        &self,
        file: Arc<File>,
        cloexec: bool,
    ) -> Result<FileDescriptor, KernelError> {
        let mut files = self.files.write();
        let mut next_fd = self.next_fd.write();

        let entry = FileEntry { file, cloexec };

        // Find an empty slot
        for (fd, slot) in files.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(entry);
                return Ok(fd);
            }
        }

        // No empty slot, append new one
        let fd = *next_fd;
        if fd >= 1024 {
            return Err(KernelError::FsError(FsError::TooManyOpenFiles));
        }

        files.push(Some(entry));
        *next_fd += 1;
        Ok(fd)
    }

    /// Get a file by descriptor
    pub fn get(&self, fd: FileDescriptor) -> Option<Arc<File>> {
        let files = self.files.read();
        files.get(fd)?.as_ref().map(|entry| entry.file.clone())
    }

    /// Get a file entry by descriptor (includes flags)
    pub fn get_entry(&self, fd: FileDescriptor) -> Option<(Arc<File>, bool)> {
        let files = self.files.read();
        files
            .get(fd)?
            .as_ref()
            .map(|entry| (entry.file.clone(), entry.cloexec))
    }

    /// Close a file descriptor
    pub fn close(&self, fd: FileDescriptor) -> Result<(), KernelError> {
        let mut files = self.files.write();

        if fd >= files.len() {
            return Err(KernelError::FsError(FsError::BadFileDescriptor));
        }

        if let Some(entry) = files[fd].take() {
            // Decrement reference count
            if entry.file.dec_ref() == 0 {
                // Last reference, file will be dropped
            }
            Ok(())
        } else {
            Err(KernelError::FsError(FsError::BadFileDescriptor))
        }
    }

    /// Duplicate a file descriptor
    pub fn dup(&self, fd: FileDescriptor) -> Result<FileDescriptor, KernelError> {
        let file = self
            .get(fd)
            .ok_or(KernelError::FsError(FsError::BadFileDescriptor))?;
        file.inc_ref();
        // Duplicated FDs don't inherit close-on-exec
        self.open(file)
    }

    /// Duplicate a file descriptor with close-on-exec flag
    pub fn dup_cloexec(&self, fd: FileDescriptor) -> Result<FileDescriptor, KernelError> {
        let file = self
            .get(fd)
            .ok_or(KernelError::FsError(FsError::BadFileDescriptor))?;
        file.inc_ref();
        self.open_with_flags(file, true)
    }

    /// Duplicate fd to the lowest available fd >= min_fd (for F_DUPFD)
    pub fn dup_at_least(
        &self,
        fd: FileDescriptor,
        min_fd: FileDescriptor,
        cloexec: bool,
    ) -> Result<FileDescriptor, KernelError> {
        let file = self
            .get(fd)
            .ok_or(KernelError::FsError(FsError::BadFileDescriptor))?;
        file.inc_ref();

        let mut files = self.files.write();
        let mut next_fd = self.next_fd.write();

        let entry = FileEntry { file, cloexec };

        // Ensure the vector is large enough to scan from min_fd
        while files.len() <= min_fd {
            files.push(None);
        }
        if *next_fd <= min_fd {
            *next_fd = min_fd;
        }

        // Find the lowest empty slot >= min_fd
        for slot_fd in min_fd..files.len() {
            if files[slot_fd].is_none() {
                files[slot_fd] = Some(entry);
                return Ok(slot_fd);
            }
        }

        // No empty slot found in existing range; append new one
        let new_fd = *next_fd;
        if new_fd >= 1024 {
            return Err(KernelError::FsError(FsError::TooManyOpenFiles));
        }

        // Ensure vector has capacity up to new_fd
        while files.len() <= new_fd {
            files.push(None);
        }
        files[new_fd] = Some(entry);
        *next_fd = new_fd + 1;
        Ok(new_fd)
    }

    /// Replace a file descriptor with another
    pub fn dup2(&self, old_fd: FileDescriptor, new_fd: FileDescriptor) -> Result<(), KernelError> {
        // If old_fd == new_fd, just return success without doing anything
        if old_fd == new_fd {
            // Verify old_fd is valid
            if self.get(old_fd).is_none() {
                return Err(KernelError::FsError(FsError::BadFileDescriptor));
            }
            return Ok(());
        }

        let file = self
            .get(old_fd)
            .ok_or(KernelError::FsError(FsError::BadFileDescriptor))?;
        file.inc_ref();

        let mut files = self.files.write();

        // Ensure files vector is large enough
        while files.len() <= new_fd {
            files.push(None);
        }

        // Close existing file at new_fd if any
        if let Some(existing) = files[new_fd].take() {
            existing.file.dec_ref();
        }

        // Set new file (dup2 doesn't preserve close-on-exec)
        files[new_fd] = Some(FileEntry {
            file,
            cloexec: false,
        });
        Ok(())
    }

    /// Replace a file descriptor with another, setting close-on-exec flag
    pub fn dup3(
        &self,
        old_fd: FileDescriptor,
        new_fd: FileDescriptor,
        cloexec: bool,
    ) -> Result<(), KernelError> {
        // dup3 with same fds is an error (unlike dup2)
        if old_fd == new_fd {
            return Err(KernelError::InvalidArgument {
                name: "new_fd",
                value: "cannot be same as old_fd in dup3",
            });
        }

        let file = self
            .get(old_fd)
            .ok_or(KernelError::FsError(FsError::BadFileDescriptor))?;
        file.inc_ref();

        let mut files = self.files.write();

        // Ensure files vector is large enough
        while files.len() <= new_fd {
            files.push(None);
        }

        // Close existing file at new_fd if any
        if let Some(existing) = files[new_fd].take() {
            existing.file.dec_ref();
        }

        // Set new file with specified close-on-exec flag
        files[new_fd] = Some(FileEntry { file, cloexec });
        Ok(())
    }

    /// Set close-on-exec flag for a file descriptor
    pub fn set_cloexec(&self, fd: FileDescriptor, cloexec: bool) -> Result<(), KernelError> {
        let mut files = self.files.write();

        if fd >= files.len() {
            return Err(KernelError::FsError(FsError::BadFileDescriptor));
        }

        if let Some(entry) = files[fd].as_mut() {
            entry.cloexec = cloexec;
            Ok(())
        } else {
            Err(KernelError::FsError(FsError::BadFileDescriptor))
        }
    }

    /// Get close-on-exec flag for a file descriptor
    pub fn get_cloexec(&self, fd: FileDescriptor) -> Result<bool, KernelError> {
        let files = self.files.read();

        if fd >= files.len() {
            return Err(KernelError::FsError(FsError::BadFileDescriptor));
        }

        if let Some(entry) = files[fd].as_ref() {
            Ok(entry.cloexec)
        } else {
            Err(KernelError::FsError(FsError::BadFileDescriptor))
        }
    }

    /// Close all file descriptors marked with close-on-exec
    /// Called during exec() system call
    pub fn close_on_exec(&self) {
        let mut files = self.files.write();

        for slot in files.iter_mut() {
            if let Some(entry) = slot.as_ref() {
                if entry.cloexec {
                    // Close this descriptor
                    if let Some(entry) = slot.take() {
                        entry.file.dec_ref();
                    }
                }
            }
        }
    }

    /// Get the number of open file descriptors
    pub fn count_open(&self) -> usize {
        let files = self.files.read();
        files.iter().filter(|slot| slot.is_some()).count()
    }

    /// Clone file table for fork()
    /// All file descriptors are duplicated with same flags
    pub fn clone_for_fork(&self) -> Self {
        let files = self.files.read();
        let next_fd = *self.next_fd.read();

        let mut new_files = Vec::with_capacity(files.len());
        for slot in files.iter() {
            if let Some(entry) = slot {
                entry.file.inc_ref();
                new_files.push(Some(FileEntry {
                    file: entry.file.clone(),
                    cloexec: entry.cloexec,
                }));
            } else {
                new_files.push(None);
            }
        }

        Self {
            files: RwLock::new(new_files),
            next_fd: RwLock::new(next_fd),
        }
    }

    /// Close all open file descriptors
    pub fn close_all(&self) {
        let mut files = self.files.write();

        for slot in files.iter_mut() {
            if let Some(entry) = slot.take() {
                entry.file.dec_ref();
            }
        }
    }
}

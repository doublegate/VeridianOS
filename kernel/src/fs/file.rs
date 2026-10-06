//! File descriptors and file operations

use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

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
    pub nonblock: bool,
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
            nonblock: false,
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
            nonblock: false,
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
            nonblock: false,
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
            nonblock: false,
        }
    }

    /// Create from bits (for syscall interface).
    ///
    /// Flag values match the Linux x86_64 ABI (`<fcntl.h>`) so musl-compiled
    /// programs work correctly. O_RDONLY=0, O_WRONLY=1, O_RDWR=2.
    pub fn from_bits(bits: u32) -> Option<Self> {
        // Linux x86_64 ABI flags
        const O_WRONLY: u32 = 0x0001;
        const O_RDWR: u32 = 0x0002;
        const O_ACCMODE: u32 = 0x0003;
        const O_CREAT: u32 = 0x0040;
        const O_EXCL: u32 = 0x0080;
        const O_TRUNC: u32 = 0x0200;
        const O_APPEND: u32 = 0x0400;
        const O_NONBLOCK: u32 = 0x0800;

        let access_mode = bits & O_ACCMODE;

        Some(Self {
            // O_RDONLY = 0: read when access mode is 0 (rdonly) or 2 (rdwr)
            read: access_mode != O_WRONLY,
            write: access_mode == O_WRONLY || access_mode == O_RDWR,
            append: (bits & O_APPEND) != 0,
            create: (bits & O_CREAT) != 0,
            truncate: (bits & O_TRUNC) != 0,
            exclusive: (bits & O_EXCL) != 0,
            nonblock: (bits & O_NONBLOCK) != 0,
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

    /// Non-blocking I/O flag (can be toggled via fcntl F_SETFL after open).
    /// Stored as AtomicBool because File is behind Arc and F_SETFL needs to
    /// toggle this without mutable access.
    pub nonblock: AtomicBool,

    /// Current position in file
    pub position: RwLock<usize>,

    /// Reference count
    pub refcount: RwLock<usize>,

    /// Absolute path this file was opened with (for dirfd resolution in *at
    /// syscalls)
    pub path: Option<String>,
}

impl File {
    /// Whether this file is a DRM device (a devfs node with the DRM major),
    /// decided by the node itself rather than the path it was opened with.
    pub fn is_drm_device(&self) -> bool {
        matches!(self.node.device_id(), Some((major, _)) if major == crate::fs::devfs::DRM_MAJOR)
    }

    /// Create a new file structure
    pub fn new(node: Arc<dyn VfsNode>, flags: OpenFlags) -> Self {
        let nb = flags.nonblock;
        Self {
            node,
            flags,
            nonblock: AtomicBool::new(nb),
            position: RwLock::new(0),
            refcount: RwLock::new(1),
            path: None,
        }
    }

    /// Create a new file structure with a known path
    pub fn new_with_path(node: Arc<dyn VfsNode>, flags: OpenFlags, path: String) -> Self {
        let nb = flags.nonblock;
        Self {
            node,
            flags,
            nonblock: AtomicBool::new(nb),
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

    /// Bit `n` (n < 3) set: fd `n` has no table entry but is still the
    /// implicit serial console that `sys_read`/`sys_write` fall back to.
    /// Allocation skips such fds, so the first `open` cannot shadow stdin
    /// or stdout (N-06); closing or `dup2`-ing over one clears its bit.
    console_fds: AtomicU8,

    /// No allocatable fd lies below this (FS-ARCH-01): `open` scans from
    /// here instead of from 0. Updated with the `files` write lock held;
    /// every path that frees a slot lowers it, allocation raises it.
    free_hint: AtomicUsize,
}

/// All three standard descriptors are the implicit console.
const CONSOLE_FDS_ALL: u8 = 0b111;

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
            console_fds: AtomicU8::new(CONSOLE_FDS_ALL),
            free_hint: AtomicUsize::new(3),
        }
    }
}

impl Default for FileTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTable {
    /// Whether `fd` is free for allocation: no entry, and not an fd that is
    /// still the implicit console.
    fn is_allocatable(&self, files: &[Option<FileEntry>], fd: FileDescriptor) -> bool {
        files[fd].is_none()
            && (fd >= 3 || self.console_fds.load(Ordering::Acquire) & (1 << fd) == 0)
    }

    /// Slot `fd` became free: the lowest free fd is now at most `fd`.
    fn note_freed(&self, fd: FileDescriptor) {
        self.free_hint.fetch_min(fd, Ordering::AcqRel);
    }

    /// Slot `fd` was just filled: if it was the hint, nothing below the
    /// next slot is free.
    fn note_filled(&self, fd: FileDescriptor) {
        let _ = self
            .free_hint
            .compare_exchange(fd, fd + 1, Ordering::AcqRel, Ordering::Relaxed);
    }

    /// Whether `fd` (0-2) is still the implicit serial console: it has no
    /// table entry and has not been closed or replaced. A standard
    /// descriptor that was closed is NOT the console; I/O on it must fail
    /// with EBADF instead of falling back to serial.
    pub fn is_implicit_console(&self, fd: FileDescriptor) -> bool {
        fd < 3 && self.console_fds.load(Ordering::Acquire) & (1u8 << fd) != 0
    }

    /// `fd` now refers to something other than the implicit console.
    fn release_console_fd(&self, fd: FileDescriptor) {
        if fd < 3 {
            self.console_fds.fetch_and(!(1u8 << fd), Ordering::AcqRel);
        }
    }

    /// Install `file` at exactly descriptor `fd`, replacing whatever was
    /// there (including the implicit console). Used to set up a new
    /// process's standard descriptors, which `open` would never hand out
    /// while they are still the implicit console.
    pub fn install(&self, fd: FileDescriptor, file: Arc<File>) -> Result<(), KernelError> {
        if fd >= 1024 {
            return Err(KernelError::FsError(FsError::TooManyOpenFiles));
        }
        let mut files = self.files.write();
        while files.len() <= fd {
            files.push(None);
        }
        if let Some(existing) = files[fd].take() {
            existing.file.dec_ref();
        }
        self.release_console_fd(fd);
        files[fd] = Some(FileEntry {
            file,
            cloexec: false,
        });
        Ok(())
    }

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

        let entry = FileEntry { file, cloexec };

        // Find the lowest free slot, starting at the hint
        let start = self.free_hint.load(Ordering::Acquire).min(files.len());
        if let Some(fd) = (start..files.len()).find(|&fd| self.is_allocatable(&files, fd)) {
            files[fd] = Some(entry);
            self.free_hint.store(fd + 1, Ordering::Release);
            return Ok(fd);
        }

        // No empty slot: append. The new fd is the index it is stored at; a
        // separate next-fd counter drifted from the table length once dup2
        // grew the table, so open returned one fd and stored the file at
        // another.
        let fd = files.len();
        if fd >= 1024 {
            return Err(KernelError::FsError(FsError::TooManyOpenFiles));
        }

        files.push(Some(entry));
        self.free_hint.store(fd + 1, Ordering::Release);
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

        if fd < 3 && files.get(fd).is_none_or(|slot| slot.is_none()) {
            // Closing the implicit console: valid, and frees the fd.
            let bit = 1u8 << fd;
            if self.console_fds.fetch_and(!bit, Ordering::AcqRel) & bit != 0 {
                self.note_freed(fd);
                return Ok(());
            }
        }

        if fd >= files.len() {
            return Err(KernelError::FsError(FsError::BadFileDescriptor));
        }

        if let Some(entry) = files[fd].take() {
            self.note_freed(fd);
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

        let entry = FileEntry { file, cloexec };

        // Ensure the vector is large enough to scan from min_fd
        while files.len() <= min_fd {
            files.push(None);
        }

        // Find the lowest free slot >= min_fd
        if let Some(slot_fd) = (min_fd..files.len()).find(|&fd| self.is_allocatable(&files, fd)) {
            files[slot_fd] = Some(entry);
            self.note_filled(slot_fd);
            return Ok(slot_fd);
        }

        // No free slot >= min_fd: append. (This used a separate counter
        // that could point at an occupied slot, which was then overwritten.)
        let new_fd = files.len();
        if new_fd >= 1024 {
            return Err(KernelError::FsError(FsError::TooManyOpenFiles));
        }
        files.push(Some(entry));
        self.note_filled(new_fd);
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
        self.release_console_fd(new_fd);
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
        self.release_console_fd(new_fd);
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

        for (fd, slot) in files.iter_mut().enumerate() {
            if let Some(entry) = slot.as_ref() {
                if entry.cloexec {
                    // Close this descriptor
                    if let Some(entry) = slot.take() {
                        self.note_freed(fd);
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
            console_fds: AtomicU8::new(self.console_fds.load(Ordering::Acquire)),
            free_hint: AtomicUsize::new(self.free_hint.load(Ordering::Acquire)),
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
        self.free_hint.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{ramfs::RamFs, Filesystem};

    fn some_file() -> Arc<File> {
        Arc::new(File::new(RamFs::new().root(), OpenFlags::read_only()))
    }

    /// Reference: the lowest fd that is neither open nor still the console.
    fn lowest_free(table: &FileTable) -> usize {
        let files = table.files.read();
        (0..)
            .find(|&fd| fd >= files.len() || table.is_allocatable(&files, fd))
            .unwrap()
    }

    #[test]
    fn open_always_returns_lowest_free_fd() {
        // FS-ARCH-01: the hint must never skip a free slot, whatever mix of
        // close / dup2 / close-on-exec freed it.
        let table = FileTable::new();
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        for step in 0..4000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let fd = (x >> 8) as usize % 40;
            match x % 7 {
                0 | 1 | 2 => {
                    let want = lowest_free(&table);
                    let got = table.open_with_flags(some_file(), x & 8 != 0).unwrap();
                    assert_eq!(got, want, "step {}", step);
                }
                3 | 4 => {
                    let _ = table.close(fd);
                }
                5 => {
                    if table.get(fd % 8).is_some() {
                        let _ = table.dup2(fd % 8, fd);
                    }
                }
                6 if step % 3 == 0 => {
                    // F_DUPFD: lowest free fd >= min, never an open one.
                    if let Some(src) = table.get(fd % 8) {
                        let before = table.count_open();
                        let min = fd;
                        let want = {
                            let files = table.files.read();
                            (min..)
                                .find(|&f| f >= files.len() || table.is_allocatable(&files, f))
                                .unwrap()
                        };
                        let got = table.dup_at_least(fd % 8, min, false).unwrap();
                        assert_eq!(got, want, "dup step {}", step);
                        assert_eq!(table.count_open(), before + 1, "overwrote a slot");
                        assert!(Arc::ptr_eq(&table.get(got).unwrap(), &src));
                    }
                }
                _ => {
                    if step % 50 == 0 {
                        table.close_on_exec();
                    }
                }
            }
        }
    }

    #[test]
    fn closed_standard_fd_is_no_longer_the_console() {
        // After close(1), I/O on fd 1 must fail (EBADF) rather than fall
        // back to the serial console (review of the v0.26.0 stack, PR #9).
        let table = FileTable::new();
        assert!((0..3).all(|fd| table.is_implicit_console(fd)));
        table.close(1).unwrap();
        assert!(!table.is_implicit_console(1));
        assert!(table.is_implicit_console(0) && table.is_implicit_console(2));
        assert!(table.close(1).is_err(), "closing it again is EBADF");
        assert!(!table.is_implicit_console(3));
    }

    #[test]
    fn first_open_does_not_shadow_console_fds() {
        // N-06: fds 0-2 are the implicit console, so the first open is 3.
        let table = FileTable::new();
        assert_eq!(table.open(some_file()).unwrap(), 3);
        assert_eq!(table.open(some_file()).unwrap(), 4);
    }

    #[test]
    fn closing_console_fd_frees_it_for_open() {
        // close(0); open() must return 0, as daemons and shells rely on.
        let table = FileTable::new();
        table.close(0).unwrap();
        assert_eq!(table.open(some_file()).unwrap(), 0);
        assert_eq!(table.open(some_file()).unwrap(), 3);
        // A second close of the now-real fd 0 closes the file, then EBADF.
        table.close(0).unwrap();
        assert!(table.close(0).is_err());
    }

    #[test]
    fn dup2_over_console_fd_replaces_it() {
        let table = FileTable::new();
        let fd = table.open(some_file()).unwrap();
        table.dup2(fd, 1).unwrap();
        assert!(table.get(1).is_some());
        table.close(1).unwrap();
        // fd 1 is no longer the console: it is free for the next open.
        assert_eq!(table.open(some_file()).unwrap(), 1);
    }

    #[test]
    fn install_sets_standard_fds() {
        let table = FileTable::new();
        for fd in 0..3 {
            table.install(fd, some_file()).unwrap();
        }
        assert!(table.get(0).is_some() && table.get(2).is_some());
        assert_eq!(table.open(some_file()).unwrap(), 3);
    }

    #[test]
    fn fork_inherits_console_state() {
        let table = FileTable::new();
        table.close(2).unwrap();
        let child = table.clone_for_fork();
        assert_eq!(child.open(some_file()).unwrap(), 2);
        assert_eq!(table.clone_for_fork().open(some_file()).unwrap(), 2);
    }
}

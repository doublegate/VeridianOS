//! File descriptors and file operations

use alloc::{sync::Arc, vec::Vec};

use spin::RwLock;

use super::VfsNode;

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
    pub fn from_bits(bits: u32) -> Option<Self> {
        // Standard POSIX O_* flags
        const O_RDONLY: u32 = 0x0000;
        const O_WRONLY: u32 = 0x0001;
        const O_RDWR: u32 = 0x0002;
        const O_CREAT: u32 = 0x0040;
        const O_EXCL: u32 = 0x0080;
        const O_TRUNC: u32 = 0x0200;
        const O_APPEND: u32 = 0x0400;

        let access_mode = bits & 0x3;

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
}

impl File {
    /// Create a new file structure
    pub fn new(node: Arc<dyn VfsNode>, flags: OpenFlags) -> Self {
        Self {
            node,
            flags,
            position: RwLock::new(0),
            refcount: RwLock::new(1),
        }
    }

    /// Read from the file
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, &'static str> {
        if !self.flags.read {
            return Err("File not opened for reading");
        }

        let mut pos = self.position.write();
        let bytes_read = self.node.read(*pos, buffer)?;
        *pos += bytes_read;
        Ok(bytes_read)
    }

    /// Write to the file
    pub fn write(&self, data: &[u8]) -> Result<usize, &'static str> {
        if !self.flags.write {
            return Err("File not opened for writing");
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
    pub fn seek(&self, from: SeekFrom) -> Result<usize, &'static str> {
        let mut pos = self.position.write();

        let new_pos = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset < 0 {
                    pos.checked_sub((-offset) as usize)
                        .ok_or("Seek before start of file")?
                } else {
                    pos.checked_add(offset as usize).ok_or("Seek overflow")?
                }
            }
            SeekFrom::End(offset) => {
                let metadata = self.node.metadata()?;
                if offset < 0 {
                    metadata
                        .size
                        .checked_sub((-offset) as usize)
                        .ok_or("Seek before start of file")?
                } else {
                    metadata
                        .size
                        .checked_add(offset as usize)
                        .ok_or("Seek overflow")?
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

/// File descriptor table for a process
pub struct FileTable {
    /// File descriptors
    files: RwLock<Vec<Option<Arc<File>>>>,

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

    /// Open a file and return a file descriptor
    pub fn open(&self, file: Arc<File>) -> Result<FileDescriptor, &'static str> {
        let mut files = self.files.write();
        let mut next_fd = self.next_fd.write();

        // Find an empty slot
        for (fd, slot) in files.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(file);
                return Ok(fd);
            }
        }

        // No empty slot, append new one
        let fd = *next_fd;
        if fd >= 1024 {
            return Err("Too many open files");
        }

        files.push(Some(file));
        *next_fd += 1;
        Ok(fd)
    }

    /// Get a file by descriptor
    pub fn get(&self, fd: FileDescriptor) -> Option<Arc<File>> {
        let files = self.files.read();
        files.get(fd)?.as_ref().cloned()
    }

    /// Close a file descriptor
    pub fn close(&self, fd: FileDescriptor) -> Result<(), &'static str> {
        let mut files = self.files.write();

        if fd >= files.len() {
            return Err("Invalid file descriptor");
        }

        if let Some(file) = files[fd].take() {
            // Decrement reference count
            if file.dec_ref() == 0 {
                // Last reference, file will be dropped
            }
            Ok(())
        } else {
            Err("File descriptor not open")
        }
    }

    /// Duplicate a file descriptor
    pub fn dup(&self, fd: FileDescriptor) -> Result<FileDescriptor, &'static str> {
        let file = self.get(fd).ok_or("Invalid file descriptor")?;
        file.inc_ref();
        self.open(file)
    }

    /// Replace a file descriptor with another
    pub fn dup2(&self, old_fd: FileDescriptor, new_fd: FileDescriptor) -> Result<(), &'static str> {
        let file = self.get(old_fd).ok_or("Invalid file descriptor")?;
        file.inc_ref();

        let mut files = self.files.write();

        // Ensure files vector is large enough
        while files.len() <= new_fd {
            files.push(None);
        }

        // Close existing file at new_fd if any
        if let Some(existing) = files[new_fd].take() {
            existing.dec_ref();
        }

        // Set new file
        files[new_fd] = Some(file);
        Ok(())
    }
}

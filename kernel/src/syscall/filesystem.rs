//! Filesystem system calls implementation
//!
//! Provides kernel-side implementation of filesystem operations including
//! file I/O, directory management, and filesystem management.

#![allow(clippy::unnecessary_cast)]

use super::{SyscallError, SyscallResult};
use crate::{
    fs::{try_get_vfs, OpenFlags, Permissions, SeekFrom},
    process,
};

/// Helper to get the VFS instance, returning a syscall error instead of
/// panicking if the VFS subsystem has not been initialized yet.
fn vfs() -> Result<&'static spin::RwLock<crate::fs::Vfs>, SyscallError> {
    try_get_vfs().ok_or(SyscallError::InvalidState)
}

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Open a file
///
/// # Arguments
/// - path: Pointer to null-terminated path string
/// - flags: Open flags (read/write/create/etc)
/// - mode: File permissions (if creating)
///
/// # Returns
/// File descriptor on success
pub fn sys_open(path: usize, flags: usize, _mode: usize) -> SyscallResult {
    // Validate path pointer
    if path == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Get path string from user space
    // SAFETY: path was validated as non-zero above. We read bytes one at a
    // time from the user-space pointer until we find a null terminator or
    // reach the 4096-byte limit. The caller must provide a valid, null-
    // terminated string in mapped user memory.
    let path_bytes = unsafe {
        let mut bytes = Vec::new();
        let mut ptr = path as *const u8;

        // Read until null terminator (max 4096 bytes)
        for _ in 0..4096 {
            let byte = *ptr;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
            ptr = ptr.add(1);
        }
        bytes
    };

    let path_str = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };

    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Convert flags
    let open_flags = OpenFlags::from_bits(flags as u32).ok_or(SyscallError::InvalidArgument)?;

    // Open the file through VFS
    match vfs()?.read().open(path_str, open_flags) {
        Ok(node) => {
            // Create file
            let file = crate::fs::file::File::new(node, open_flags);

            // Add to process file table
            let file_table = process.file_table.lock();
            match file_table.open(alloc::sync::Arc::new(file)) {
                Ok(fd_num) => Ok(fd_num),
                Err(_) => Err(SyscallError::OutOfMemory),
            }
        }
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// Close a file descriptor
///
/// # Arguments
/// - fd: File descriptor to close
pub fn sys_close(fd: usize) -> SyscallResult {
    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Remove from file table
    let file_table = process.file_table.lock();
    match file_table.close(fd) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// Read from a file
///
/// # Arguments
/// - fd: File descriptor
/// - buffer: Buffer to read into
/// - count: Number of bytes to read
///
/// # Returns
/// Number of bytes actually read
pub fn sys_read(fd: usize, buffer: usize, count: usize) -> SyscallResult {
    // Validate buffer
    if buffer == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Get file descriptor
    let file_table = process.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Create buffer slice
    // SAFETY: buffer was validated as non-zero above. The caller must
    // provide a valid, writable user-space buffer of at least `count`
    // bytes. from_raw_parts_mut creates a mutable slice for the read.
    let buffer_slice = unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, count) };

    // Read from file
    match file_desc.read(buffer_slice) {
        Ok(bytes_read) => Ok(bytes_read),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Write to a file
///
/// # Arguments
/// - fd: File descriptor
/// - buffer: Buffer to write from
/// - count: Number of bytes to write
///
/// # Returns
/// Number of bytes actually written
pub fn sys_write(fd: usize, buffer: usize, count: usize) -> SyscallResult {
    // Validate buffer
    if buffer == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Get file descriptor
    let file_table = process.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Create buffer slice
    // SAFETY: buffer was validated as non-zero above. The caller must
    // provide a valid, readable user-space buffer of at least `count`
    // bytes. from_raw_parts creates an immutable slice for the write.
    let buffer_slice = unsafe { core::slice::from_raw_parts(buffer as *const u8, count) };

    // Write to file
    match file_desc.write(buffer_slice) {
        Ok(bytes_written) => Ok(bytes_written),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Seek within a file
///
/// # Arguments
/// - fd: File descriptor
/// - offset: Offset to seek
/// - whence: Seek origin (0=start, 1=current, 2=end)
///
/// # Returns
/// New file position
pub fn sys_seek(fd: usize, offset: isize, whence: usize) -> SyscallResult {
    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Get file descriptor
    let file_table = process.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Convert whence to SeekFrom
    let seek_from = match whence {
        0 => SeekFrom::Start(offset as usize),
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return Err(SyscallError::InvalidArgument),
    };

    // Perform seek
    match file_desc.seek(seek_from) {
        Ok(new_pos) => Ok(new_pos as usize),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Get file status
///
/// # Arguments
/// - fd: File descriptor
/// - stat_buf: Buffer to write stat structure
pub fn sys_stat(fd: usize, stat_buf: usize) -> SyscallResult {
    // Validate buffer
    if stat_buf == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Get file descriptor
    let file_table = process.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Get metadata
    match file_desc.node.metadata() {
        Ok(metadata) => {
            // Write metadata to user buffer
            // SAFETY: stat_buf was validated as non-zero above. The caller
            // must provide a valid, writable pointer to a FileStat struct.
            // We write individual fields through the pointer. FileStat is
            // repr(C) for stable layout.
            unsafe {
                let buf = stat_buf as *mut FileStat;
                (*buf).size = metadata.size;
                (*buf).mode = match metadata.node_type {
                    crate::fs::NodeType::File => 0o100644,
                    crate::fs::NodeType::Directory => 0o040755,
                    crate::fs::NodeType::CharDevice => 0o020666,
                    crate::fs::NodeType::BlockDevice => 0o060666,
                    _ => 0,
                };
                (*buf).uid = metadata.uid;
                (*buf).gid = metadata.gid;
                (*buf).created = metadata.created;
                (*buf).modified = metadata.modified;
                (*buf).accessed = metadata.accessed;
            }
            Ok(0)
        }
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Truncate a file
///
/// # Arguments
/// - fd: File descriptor
/// - size: New file size
pub fn sys_truncate(fd: usize, size: usize) -> SyscallResult {
    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Get file descriptor
    let file_table = process.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Truncate file
    match file_desc.node.truncate(size) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Create a directory
///
/// # Arguments
/// - path: Path to new directory
/// - mode: Directory permissions
pub fn sys_mkdir(path: usize, mode: usize) -> SyscallResult {
    // Validate path
    if path == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Get path string
    // SAFETY: path was validated as non-zero above. We read bytes from the
    // user-space pointer until null terminator or 4096-byte limit. The
    // caller must provide a valid null-terminated string.
    let path_bytes = unsafe {
        let mut bytes = Vec::new();
        let mut ptr = path as *const u8;

        for _ in 0..4096 {
            let byte = *ptr;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
            ptr = ptr.add(1);
        }
        bytes
    };

    let path_str = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };

    // Create directory through VFS
    let permissions = Permissions::from_mode(mode as u32);
    match vfs()?.read().mkdir(path_str, permissions) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Remove a directory
///
/// # Arguments
/// - path: Path to directory to remove
pub fn sys_rmdir(path: usize) -> SyscallResult {
    // Validate path
    if path == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Get path string
    // SAFETY: path was validated as non-zero above. We read bytes from the
    // user-space pointer until null terminator or 4096-byte limit. The
    // caller must provide a valid null-terminated string.
    let path_bytes = unsafe {
        let mut bytes = Vec::new();
        let mut ptr = path as *const u8;

        for _ in 0..4096 {
            let byte = *ptr;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
            ptr = ptr.add(1);
        }
        bytes
    };

    let path_str = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };

    // Remove directory through VFS
    match vfs()?.read().unlink(path_str) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Mount a filesystem
///
/// # Arguments
/// - device: Device path (or filesystem type for virtual filesystems)
/// - mount_point: Where to mount the filesystem
/// - fs_type: Filesystem type string
/// - flags: Mount flags
///
/// This is a privileged operation requiring a kernel-level capability.
pub fn sys_mount(
    _device: usize,
    mount_point: usize,
    fs_type: usize,
    flags: usize,
) -> SyscallResult {
    // Validate pointers
    if mount_point == 0 || fs_type == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Mount is a privileged operation - verify the calling process has
    // a Memory capability with WRITE rights (needed to modify the VFS tree)
    let current = process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current.capability_space.lock();
    let has_mount_perm = {
        let mut found = false;
        #[cfg(feature = "alloc")]
        {
            let _ = cap_space.iter_capabilities(|entry| {
                if matches!(entry.object, crate::cap::ObjectRef::Memory { .. })
                    && entry.rights.contains(crate::cap::Rights::WRITE)
                    && entry.rights.contains(crate::cap::Rights::CREATE)
                {
                    found = true;
                    return false;
                }
                true
            });
        }
        found
    };
    if !has_mount_perm {
        return Err(SyscallError::PermissionDenied);
    }

    // Get mount point path
    // SAFETY: mount_point was validated as non-zero above. We read bytes
    // from the user-space pointer until null terminator or 4096-byte limit.
    let mount_path_bytes = unsafe {
        let mut bytes = Vec::new();
        let mut ptr = mount_point as *const u8;

        for _ in 0..4096 {
            let byte = *ptr;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
            ptr = ptr.add(1);
        }
        bytes
    };

    let mount_path = match core::str::from_utf8(&mount_path_bytes) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };

    // Get filesystem type
    // SAFETY: fs_type was validated as non-zero above. We read bytes from
    // the user-space pointer until null terminator or 256-byte limit.
    let fs_type_bytes = unsafe {
        let mut bytes = Vec::new();
        let mut ptr = fs_type as *const u8;

        for _ in 0..256 {
            let byte = *ptr;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
            ptr = ptr.add(1);
        }
        bytes
    };

    let fs_type_str = match core::str::from_utf8(&fs_type_bytes) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };

    // Mount filesystem
    match vfs()?
        .write()
        .mount_by_type(mount_path, fs_type_str, flags as u32)
    {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Unmount a filesystem
///
/// # Arguments
/// - mount_point: Mount point to unmount
///
/// This is a privileged operation requiring a kernel-level capability.
pub fn sys_unmount(mount_point: usize) -> SyscallResult {
    // Validate pointer
    if mount_point == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Unmount is a privileged operation - verify the calling process has
    // a Memory capability with WRITE rights (needed to modify the VFS tree)
    let current = process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current.capability_space.lock();
    let has_unmount_perm = {
        let mut found = false;
        #[cfg(feature = "alloc")]
        {
            let _ = cap_space.iter_capabilities(|entry| {
                if matches!(entry.object, crate::cap::ObjectRef::Memory { .. })
                    && entry.rights.contains(crate::cap::Rights::WRITE)
                    && entry.rights.contains(crate::cap::Rights::CREATE)
                {
                    found = true;
                    return false;
                }
                true
            });
        }
        found
    };
    if !has_unmount_perm {
        return Err(SyscallError::PermissionDenied);
    }

    // Get mount point path
    // SAFETY: mount_point was validated as non-zero above. We read bytes
    // from the user-space pointer until null terminator or 4096-byte limit.
    let mount_path_bytes = unsafe {
        let mut bytes = Vec::new();
        let mut ptr = mount_point as *const u8;

        for _ in 0..4096 {
            let byte = *ptr;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
            ptr = ptr.add(1);
        }
        bytes
    };

    let mount_path = match core::str::from_utf8(&mount_path_bytes) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };

    // Unmount filesystem
    match vfs()?.write().unmount(mount_path) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Sync filesystem
///
/// Flushes all pending writes to disk
pub fn sys_sync() -> SyscallResult {
    match vfs()?.read().sync() {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

// File stat structure for userspace
#[repr(C)]
struct FileStat {
    size: usize,
    mode: u32,
    uid: u32,
    gid: u32,
    created: u64,
    modified: u64,
    accessed: u64,
}

//! Filesystem system calls implementation
//!
//! Provides kernel-side implementation of filesystem operations including
//! file I/O, directory management, and filesystem management.
//!
//! For fd 0 (stdin), fd 1 (stdout), and fd 2 (stderr), the read/write
//! syscalls fall back to serial UART I/O when the process does not yet
//! have a file descriptor table entry for those descriptors. This enables
//! early user-space binaries (e.g., the embedded init) to produce output
//! and read input before a full VFS-backed console is available.

#![allow(clippy::unnecessary_cast)]

#[allow(unused_imports)]
use super::{
    validate_user_buffer, validate_user_ptr_typed, validate_user_string_ptr, SyscallError,
    SyscallResult,
};
use crate::{
    fs::{try_get_vfs, OpenFlags, Permissions, SeekFrom},
    process,
};

// ---------------------------------------------------------------------------
// Architecture-specific serial I/O helpers for syscall fallback
// ---------------------------------------------------------------------------

/// Write a single byte to the serial UART.
///
/// Used as a fallback when stdout/stderr file descriptors are not yet set up
/// in the process's file table.
fn serial_write_byte(byte: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        use core::fmt::Write;
        // Use the kernel's initialized serial port (COM1 at 0x3F8).
        // This goes through the uart_16550 driver with proper FIFO handling.
        x86_64::instructions::interrupts::without_interrupts(|| {
            crate::arch::x86_64::serial::SERIAL1
                .lock()
                .write_char(byte as char)
                .ok();
        });
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Direct MMIO write to PL011 UART data register (QEMU virt machine).
        const UART_DR: usize = 0x0900_0000;
        // SAFETY: The PL011 UART data register at 0x09000000 is memory-mapped
        // I/O on the QEMU virt machine. Writing a byte transmits a character.
        // volatile_write ensures the compiler does not elide the store.
        unsafe {
            core::ptr::write_volatile(UART_DR as *mut u8, byte);
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SBI legacy console putchar (function 0x01).
        // SAFETY: The ecall instruction invokes the SBI console putchar
        // interface. a0 holds the character, a7 holds the function ID.
        // This is the standard mechanism for RISC-V console output.
        unsafe {
            core::arch::asm!(
                "ecall",
                in("a0") byte as usize,
                in("a7") 0x01usize,
                options(nostack, nomem)
            );
        }
    }
}

/// Try to read a single byte from the serial UART (non-blocking).
///
/// Returns `Some(byte)` if data is available, `None` otherwise.
/// Used as a fallback when the stdin file descriptor is not yet set up.
fn serial_try_read_byte() -> Option<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        // Check Line Status Register (base + 5) bit 0 for data ready,
        // then read from data register (base + 0) at COM1 (0x3F8).
        let status: u8;
        // SAFETY: Reading the Line Status Register at I/O port 0x3FD.
        // This is a well-defined 16550 UART register read.
        unsafe {
            core::arch::asm!(
                "in al, dx",
                out("al") status,
                in("dx") 0x3FDu16,
                options(nomem, nostack)
            );
        }
        if (status & 1) != 0 {
            let data: u8;
            // SAFETY: Reading the data register at I/O port 0x3F8.
            // The LSR check above confirmed data is available.
            unsafe {
                core::arch::asm!(
                    "in al, dx",
                    out("al") data,
                    in("dx") 0x3F8u16,
                    options(nomem, nostack)
                );
            }
            Some(data)
        } else {
            None
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        const UART_BASE: usize = 0x0900_0000;
        const UART_FR: usize = UART_BASE + 0x18; // Flag register
        const UART_DR: usize = UART_BASE; // Data register

        // SAFETY: Reading PL011 UART MMIO registers. The QEMU virt machine
        // maps the UART at this address. volatile_read prevents reordering.
        unsafe {
            let flags = core::ptr::read_volatile(UART_FR as *const u32);
            if (flags & (1 << 4)) == 0 {
                // RXFE bit clear = data available
                let data = core::ptr::read_volatile(UART_DR as *const u32);
                Some((data & 0xFF) as u8)
            } else {
                None
            }
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SBI legacy console getchar (function 0x02).
        let result: isize;
        // SAFETY: The ecall invokes SBI console_getchar. Returns the
        // character in a0, or -1 if no data is available.
        unsafe {
            core::arch::asm!(
                "li a7, 0x02",
                "ecall",
                out("a0") result,
                out("a7") _,
                options(nomem)
            );
        }
        if result >= 0 {
            Some(result as u8)
        } else {
            None
        }
    }
}

/// Maximum buffer size for serial I/O fallback (64 KB).
/// Prevents unbounded kernel-side loops for large writes.
const SERIAL_IO_MAX_SIZE: usize = 64 * 1024;

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
    // Validate path pointer is in user space
    validate_user_string_ptr(path)?;

    // Get path string from user space
    // SAFETY: path was validated as non-null and in user-space above. We read
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
///
/// For fd 0 (stdin), if the process does not have a file descriptor table
/// entry, falls back to polling-mode serial UART input. This allows the
/// embedded shell to accept keyboard input before a full console subsystem
/// is initialized.
pub fn sys_read(fd: usize, buffer: usize, count: usize) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    // Validate buffer is in user space
    validate_user_buffer(buffer, count)?;

    // For stdin (fd 0), try file table first, then fall back to serial
    if fd == 0 {
        // Try the file table first if we have a process context
        if let Some(proc) = process::current_process() {
            let file_table = proc.file_table.lock();
            if let Some(file_desc) = file_table.get(fd) {
                // SAFETY: buffer is non-zero (checked above). The caller
                // must provide a valid, writable buffer of at least `count`
                // bytes. from_raw_parts_mut creates a mutable slice.
                let buffer_slice =
                    unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, count) };
                return match file_desc.read(buffer_slice) {
                    Ok(bytes_read) => Ok(bytes_read),
                    Err(_) => Err(SyscallError::InvalidState),
                };
            }
        }

        // Fallback: read from serial UART (polling mode, line-buffered)
        let read_count = count.min(SERIAL_IO_MAX_SIZE);
        // SAFETY: buffer is non-zero (checked above). We limit the size
        // via SERIAL_IO_MAX_SIZE. The caller must provide a valid writable
        // buffer of at least `count` bytes. During early bring-up this
        // may be a kernel-space address from the embedded init binary.
        let buffer_slice =
            unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, read_count) };

        let mut bytes_read = 0;
        for slot in buffer_slice.iter_mut() {
            // Spin-wait for a byte to become available
            let byte = loop {
                if let Some(b) = serial_try_read_byte() {
                    break b;
                }
                core::hint::spin_loop();
            };

            *slot = byte;
            bytes_read += 1;

            // Line-buffered: stop after newline or carriage return
            if byte == b'\n' || byte == b'\r' {
                break;
            }
        }

        return Ok(bytes_read);
    }

    // Non-stdin: use file table normally
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // SAFETY: buffer was validated as non-zero above. The caller must
    // provide a valid, writable user-space buffer of at least `count`
    // bytes. from_raw_parts_mut creates a mutable slice for the read.
    let buffer_slice = unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, count) };

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
///
/// For fd 1 (stdout) and fd 2 (stderr), if the process does not have a
/// file descriptor table entry, falls back to writing directly to the
/// serial UART. This is the critical path for the embedded init binary
/// which calls `syscall(53, 1, buf_ptr, len)` before a full VFS-backed
/// console is available.
pub fn sys_write(fd: usize, buffer: usize, count: usize) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    // Validate buffer is in user space
    validate_user_buffer(buffer, count)?;

    // For stdout (fd 1) and stderr (fd 2), try file table first, then
    // fall back to serial output
    if fd == 1 || fd == 2 {
        // Try the file table first if we have a process context
        if let Some(proc) = process::current_process() {
            let file_table = proc.file_table.lock();
            if let Some(file_desc) = file_table.get(fd) {
                // SAFETY: buffer is non-zero (checked above). The caller
                // must provide a valid, readable buffer of at least `count`
                // bytes. from_raw_parts creates an immutable slice.
                let buffer_slice =
                    unsafe { core::slice::from_raw_parts(buffer as *const u8, count) };
                return match file_desc.write(buffer_slice) {
                    Ok(bytes_written) => Ok(bytes_written),
                    Err(_) => Err(SyscallError::InvalidState),
                };
            }
        }

        // Fallback: write directly to serial UART
        let write_count = count.min(SERIAL_IO_MAX_SIZE);
        // SAFETY: buffer is non-zero (checked above). We limit the size
        // via SERIAL_IO_MAX_SIZE. The caller must provide a valid readable
        // buffer of at least `count` bytes. During early bring-up this
        // may be a kernel-space address from the embedded init binary.
        let buffer_slice = unsafe { core::slice::from_raw_parts(buffer as *const u8, write_count) };

        for &byte in buffer_slice {
            serial_write_byte(byte);
        }

        return Ok(write_count);
    }

    // Non-stdout/stderr: use file table normally
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // SAFETY: buffer was validated as non-zero above. The caller must
    // provide a valid, readable user-space buffer of at least `count`
    // bytes. from_raw_parts creates an immutable slice for the write.
    let buffer_slice = unsafe { core::slice::from_raw_parts(buffer as *const u8, count) };

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
    // Validate stat buffer pointer is in user space and aligned for FileStat
    validate_user_ptr_typed::<FileStat>(stat_buf)?;

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
    // Validate path pointer is in user space
    validate_user_string_ptr(path)?;

    // Get path string
    // SAFETY: path was validated as non-null and in user-space above. We read
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
    // Validate path pointer is in user space
    validate_user_string_ptr(path)?;

    // Get path string
    // SAFETY: path was validated as non-null and in user-space above. We read
    // bytes from the user-space pointer until null terminator or 4096-byte
    // limit. The caller must provide a valid null-terminated string.
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
    // Validate mount_point and fs_type string pointers are in user space
    validate_user_string_ptr(mount_point)?;
    validate_user_string_ptr(fs_type)?;

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
    // Validate mount_point string pointer is in user space
    validate_user_string_ptr(mount_point)?;

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

// ============================================================================
// Extended filesystem syscalls (Sprint 2C)
// ============================================================================

/// Duplicate a file descriptor
///
/// # Arguments
/// - fd: File descriptor to duplicate
///
/// # Returns
/// The new file descriptor number
pub fn sys_dup(fd: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    match file_table.dup(fd) {
        Ok(new_fd) => Ok(new_fd),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// Duplicate a file descriptor to a specific number
///
/// # Arguments
/// - old_fd: File descriptor to duplicate
/// - new_fd: Target file descriptor number
///
/// # Returns
/// The new file descriptor number
pub fn sys_dup2(old_fd: usize, new_fd: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    match file_table.dup2(old_fd, new_fd) {
        Ok(()) => Ok(new_fd),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// Create a pipe
///
/// Creates a pipe and allocates real file descriptors for both ends.
///
/// # Arguments
/// - pipe_fds_ptr: Pointer to a [usize; 2] array to receive [read_fd, write_fd]
///
/// # Returns
/// 0 on success
pub fn sys_pipe(pipe_fds_ptr: usize) -> SyscallResult {
    // Delegate to pipe2 with no flags
    sys_pipe2(pipe_fds_ptr, 0)
}

/// Get current working directory
///
/// # Arguments
/// - buf: Buffer to write the CWD path
/// - size: Buffer size
///
/// # Returns
/// Length of the CWD path
pub fn sys_getcwd(buf: usize, size: usize) -> SyscallResult {
    if size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(buf, size)?;

    let cwd = if let Some(vfs) = try_get_vfs() {
        let vfs_guard = vfs.read();
        alloc::string::String::from(vfs_guard.get_cwd())
    } else {
        alloc::string::String::from("/")
    };

    let cwd_bytes = cwd.as_bytes();
    if cwd_bytes.len() + 1 > size {
        return Err(SyscallError::InvalidArgument); // Buffer too small
    }

    // SAFETY: buf was validated above as non-null and in user space with
    // sufficient size. We write cwd_bytes.len() + 1 bytes (including NUL).
    unsafe {
        let dst = buf as *mut u8;
        core::ptr::copy_nonoverlapping(cwd_bytes.as_ptr(), dst, cwd_bytes.len());
        *dst.add(cwd_bytes.len()) = 0; // NUL terminator
    }

    Ok(cwd_bytes.len())
}

/// Change current working directory
///
/// # Arguments
/// - path_ptr: Pointer to the new directory path (NUL-terminated)
///
/// # Returns
/// 0 on success
pub fn sys_chdir(path_ptr: usize) -> SyscallResult {
    validate_user_string_ptr(path_ptr)?;

    // SAFETY: path_ptr was validated as non-null and in user-space above. We read
    // bytes until null terminator or 4096-byte limit.
    let path_bytes = unsafe {
        let mut bytes = alloc::vec::Vec::new();
        let mut ptr = path_ptr as *const u8;
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

    let path_str = core::str::from_utf8(&path_bytes).map_err(|_| SyscallError::InvalidArgument)?;

    match vfs()?
        .write()
        .set_cwd(alloc::string::String::from(path_str))
    {
        Ok(()) => Ok(0),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// I/O control operations on a file descriptor
///
/// # Arguments
/// - fd: File descriptor
/// - cmd: I/O control command
/// - arg: Command-specific argument
///
/// # Returns
/// Command-specific return value
pub fn sys_ioctl(_fd: usize, cmd: usize, arg: usize) -> SyscallResult {
    // Terminal ioctl constants (matching Linux values)
    const TCGETS: usize = 0x5401;
    const TCSETSW: usize = 0x5403;
    const TIOCGWINSZ: usize = 0x5413;
    const TIOCSWINSZ: usize = 0x5414;
    const TIOCGPGRP: usize = 0x540F;
    const TIOCSPGRP: usize = 0x5410;

    match cmd {
        TIOCGWINSZ => {
            // Return terminal window size (default 80x24)
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<Winsize>(arg)?;

            let ws = Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: arg was validated as aligned, non-null, and in user space.
            unsafe {
                core::ptr::write(arg as *mut Winsize, ws);
            }
            Ok(0)
        }
        TIOCSWINSZ => {
            // Set window size -- accept silently (no real terminal to resize)
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<Winsize>(arg)?;
            Ok(0)
        }
        TCGETS => {
            // Get terminal attributes -- return a default termios
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<Termios>(arg)?;

            let termios = Termios::default_console();
            // SAFETY: arg was validated above.
            unsafe {
                core::ptr::write(arg as *mut Termios, termios);
            }
            Ok(0)
        }
        TCSETSW => {
            // Set terminal attributes (drain first) -- accept silently
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<Termios>(arg)?;
            Ok(0)
        }
        TIOCGPGRP => {
            // Get foreground process group
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<i32>(arg)?;
            let pgid = if let Some(proc) = process::current_process() {
                proc.pgid.load(core::sync::atomic::Ordering::Acquire) as i32
            } else {
                1
            };
            // SAFETY: arg was validated above.
            unsafe {
                core::ptr::write(arg as *mut i32, pgid);
            }
            Ok(0)
        }
        TIOCSPGRP => {
            // Set foreground process group -- accept silently
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<i32>(arg)?;
            Ok(0)
        }
        _ => {
            // ENOTTY -- not a terminal or unsupported ioctl
            Err(SyscallError::InvalidArgument)
        }
    }
}

/// Terminal window size structure (matches C struct winsize).
#[repr(C)]
#[derive(Clone, Copy)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

/// Terminal attributes structure (matches C struct termios, simplified).
#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

impl Termios {
    /// Return default console termios (cooked mode, echo on).
    fn default_console() -> Self {
        // Common flag values matching Linux defaults
        const ICRNL: u32 = 0o0000400;
        const OPOST: u32 = 0o0000001;
        const ONLCR: u32 = 0o0000004;
        const CS8: u32 = 0o0000060;
        const CREAD: u32 = 0o0000200;
        const HUPCL: u32 = 0o0002000;
        const ECHO: u32 = 0o0000010;
        const ECHOE: u32 = 0o0000020;
        const ECHOK: u32 = 0o0000040;
        const ICANON: u32 = 0o0000002;
        const ISIG: u32 = 0o0000001;
        const IEXTEN: u32 = 0o0100000;

        let mut cc = [0u8; 32];
        cc[0] = 3; // VINTR = Ctrl-C
        cc[1] = 28; // VQUIT = Ctrl-backslash
        cc[2] = 127; // VERASE = DEL
        cc[3] = 21; // VKILL = Ctrl-U
        cc[4] = 4; // VEOF = Ctrl-D
        cc[5] = 0; // VTIME
        cc[6] = 1; // VMIN

        Self {
            c_iflag: ICRNL,
            c_oflag: OPOST | ONLCR,
            c_cflag: CS8 | CREAD | HUPCL,
            c_lflag: ECHO | ECHOE | ECHOK | ICANON | ISIG | IEXTEN,
            c_cc: cc,
            c_ispeed: 38400,
            c_ospeed: 38400,
        }
    }
}

/// Send a signal to a process
///
/// # Arguments
/// - pid: Process ID to signal
/// - signal: Signal number
///
/// # Returns
/// 0 on success
pub fn sys_kill(pid: usize, signal: usize) -> SyscallResult {
    let process_server = crate::services::process_server::get_process_server();
    match process_server.send_signal(crate::process::ProcessId(pid as u64), signal as i32) {
        Ok(()) => Ok(0),
        Err(_) => Err(SyscallError::ProcessNotFound),
    }
}

// ============================================================================
// Extended filesystem syscalls (Phase 4B)
// ============================================================================

/// Helper: read a NUL-terminated path from user space into an alloc::String.
#[cfg(feature = "alloc")]
fn read_user_path(ptr: usize) -> Result<alloc::string::String, SyscallError> {
    validate_user_string_ptr(ptr)?;

    // SAFETY: ptr was validated as non-null and in user-space above. We read
    // bytes until null terminator or 4096-byte limit. The caller must provide
    // a valid null-terminated string in mapped user memory.
    let bytes = unsafe {
        let mut v = Vec::new();
        let mut p = ptr as *const u8;
        for _ in 0..4096 {
            let b = *p;
            if b == 0 {
                break;
            }
            v.push(b);
            p = p.add(1);
        }
        v
    };

    core::str::from_utf8(&bytes)
        .map(alloc::string::String::from)
        .map_err(|_| SyscallError::InvalidArgument)
}

/// Stat a file by path (syscall 150).
///
/// Like `sys_stat` but takes a path instead of an fd.
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated path string.
/// - `stat_buf`: Pointer to `FileStat` output buffer.
///
/// # Returns
/// 0 on success.
pub fn sys_stat_path(path_ptr: usize, stat_buf: usize) -> SyscallResult {
    validate_user_ptr_typed::<FileStat>(stat_buf)?;
    let path = read_user_path(path_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();
    let node = vfs_guard
        .resolve_path(&path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    match node.metadata() {
        Ok(metadata) => {
            // SAFETY: stat_buf was validated as non-null, in user-space, and
            // aligned for FileStat above. We write metadata fields.
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

/// Stat a file by path without following symlinks (syscall 151).
///
/// Currently identical to `sys_stat_path` since VeridianOS does not yet
/// support symbolic links.
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated path string.
/// - `stat_buf`: Pointer to `FileStat` output buffer.
///
/// # Returns
/// 0 on success.
pub fn sys_lstat(path_ptr: usize, stat_buf: usize) -> SyscallResult {
    // No symlinks yet -- lstat behaves identically to stat
    sys_stat_path(path_ptr, stat_buf)
}

/// Read the target of a symbolic link (syscall 152).
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated symlink path.
/// - `buf`: Buffer to receive the link target.
/// - `bufsiz`: Size of the buffer.
///
/// # Returns
/// Number of bytes written to `buf`, or error.
pub fn sys_readlink(path_ptr: usize, buf: usize, bufsiz: usize) -> SyscallResult {
    let _path = read_user_path(path_ptr)?;
    if bufsiz == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(buf, bufsiz)?;

    // TODO(phase5): Implement symlink resolution in VFS. For now,
    // all paths are concrete -- no symlinks exist.
    Err(SyscallError::InvalidSyscall)
}

/// Check file accessibility (syscall 153).
///
/// Tests whether the calling process can access the file at `path_ptr`
/// with the requested mode bits (R=4, W=2, X=1, F_OK=0).
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated path.
/// - `mode`: Access mode to check (bitmask of R_OK|W_OK|X_OK or F_OK=0).
///
/// # Returns
/// 0 if accessible, error otherwise.
pub fn sys_access(path_ptr: usize, mode: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();

    // Check if the file exists (F_OK = 0)
    let node = vfs_guard
        .resolve_path(&path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    // For non-zero mode, check permissions against metadata
    if mode != 0 {
        let _metadata = node.metadata().map_err(|_| SyscallError::InvalidState)?;
        // TODO(phase5): Check actual file permissions against the calling
        // process's uid/gid. For now, all access checks pass if the file
        // exists.
    }

    Ok(0)
}

/// Rename a file or directory (syscall 154).
///
/// # Arguments
/// - `old_ptr`: Pointer to NUL-terminated old path.
/// - `new_ptr`: Pointer to NUL-terminated new path.
///
/// # Returns
/// 0 on success.
pub fn sys_rename(old_ptr: usize, new_ptr: usize) -> SyscallResult {
    let old_path = read_user_path(old_ptr)?;
    let new_path = read_user_path(new_ptr)?;

    // Rename as copy + delete (VFS has no native rename)
    // Use the free-standing fs helpers which handle locking internally.
    let data = crate::fs::read_file(&old_path).map_err(|_| SyscallError::ResourceNotFound)?;
    crate::fs::write_file(&new_path, &data).map_err(|_| SyscallError::InvalidState)?;

    let vfs_lock = vfs()?;
    vfs_lock
        .read()
        .unlink(&old_path)
        .map_err(|_| SyscallError::InvalidState)?;

    Ok(0)
}

/// Remove a file (not a directory) (syscall 157).
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated path.
///
/// # Returns
/// 0 on success.
pub fn sys_unlink(path_ptr: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();

    match vfs_guard.unlink(&path) {
        Ok(()) => Ok(0),
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// File descriptor control (syscall 158).
///
/// Implements POSIX fcntl operations using the FileTable's cloexec API.
///
/// # Arguments
/// - `fd`: File descriptor.
/// - `cmd`: Command (F_DUPFD=0, F_GETFD=1, F_SETFD=2, F_GETFL=3, F_SETFL=4).
/// - `arg`: Command-specific argument.
///
/// # Returns
/// Command-specific value on success.
pub fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> SyscallResult {
    const F_DUPFD: usize = 0;
    const F_GETFD: usize = 1;
    const F_SETFD: usize = 2;
    const F_GETFL: usize = 3;
    const F_SETFL: usize = 4;
    const FD_CLOEXEC: usize = 1;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();

    match cmd {
        F_DUPFD => {
            // Duplicate fd to lowest available >= arg
            match file_table.dup(fd) {
                Ok(new_fd) => Ok(new_fd),
                Err(_) => Err(SyscallError::InvalidArgument),
            }
        }
        F_GETFD => {
            // Get close-on-exec flag via FileTable API
            match file_table.get_cloexec(fd) {
                Ok(cloexec) => Ok(if cloexec { FD_CLOEXEC } else { 0 }),
                Err(_) => Err(SyscallError::InvalidArgument),
            }
        }
        F_SETFD => {
            // Set close-on-exec flag via FileTable API
            let cloexec = arg & FD_CLOEXEC != 0;
            file_table
                .set_cloexec(fd, cloexec)
                .map_err(|_| SyscallError::InvalidArgument)?;
            Ok(0)
        }
        F_GETFL => {
            // Get file status flags from the File struct
            let file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;
            let mut flags: usize = 0;
            if file.flags.read && file.flags.write {
                flags |= 0x0002; // O_RDWR
            } else if file.flags.write {
                flags |= 0x0001; // O_WRONLY
            }
            // else O_RDONLY = 0
            if file.flags.append {
                flags |= 0x0400; // O_APPEND
            }
            Ok(flags)
        }
        F_SETFL => {
            // Set file status flags -- only O_APPEND and O_NONBLOCK can be
            // changed after open. We validate the fd exists but the actual
            // flag mutation requires mutable access to the File struct which
            // is behind an Arc. This is a no-op for now (flags are set at
            // open time and not typically changed).
            let _file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// Create a pipe with flags (syscall 65).
///
/// Creates a pipe, wraps both ends as VfsNode-backed File objects,
/// allocates file descriptors in the calling process's file table,
/// and writes [read_fd, write_fd] to the user buffer.
///
/// # Arguments
/// - `pipe_fds_ptr`: Pointer to `[usize; 2]` to receive [read_fd, write_fd].
/// - `flags`: O_CLOEXEC (0x2000) | O_NONBLOCK (0x1000).
///
/// # Returns
/// 0 on success.
pub fn sys_pipe2(pipe_fds_ptr: usize, flags: usize) -> SyscallResult {
    validate_user_buffer(pipe_fds_ptr, 2 * core::mem::size_of::<usize>())?;

    let cloexec = flags & 0x2000 != 0;

    // Create the pipe
    let (reader, writer) = crate::fs::pipe::create_pipe().map_err(|_| SyscallError::OutOfMemory)?;

    // Wrap pipe ends as VfsNode objects
    let read_node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
        alloc::sync::Arc::new(crate::fs::pipe::PipeReadNode::new(reader));
    let write_node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
        alloc::sync::Arc::new(crate::fs::pipe::PipeWriteNode::new(writer));

    // Create File objects
    let read_file = crate::fs::file::File::new(read_node, OpenFlags::read_only());
    let write_file = crate::fs::file::File::new(write_node, OpenFlags::write_only());

    // Allocate file descriptors in the calling process's file table
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();

    let read_fd = file_table
        .open_with_flags(alloc::sync::Arc::new(read_file), cloexec)
        .map_err(|_| SyscallError::OutOfMemory)?;

    let write_fd = file_table
        .open_with_flags(alloc::sync::Arc::new(write_file), cloexec)
        .map_err(|_| {
            // Clean up read fd on failure
            let _ = file_table.close(read_fd);
            SyscallError::OutOfMemory
        })?;

    // Write [read_fd, write_fd] to user buffer
    // SAFETY: pipe_fds_ptr was validated above as non-null and in user
    // space with sufficient size for two usize values.
    unsafe {
        let fds = pipe_fds_ptr as *mut usize;
        *fds = read_fd;
        *fds.add(1) = write_fd;
    }

    Ok(0)
}

/// Duplicate a file descriptor with flags (syscall 66).
///
/// Uses FileTable::dup3() which atomically sets the close-on-exec flag
/// on the new descriptor.
///
/// # Arguments
/// - `old_fd`: Source file descriptor.
/// - `new_fd`: Target file descriptor number.
/// - `flags`: O_CLOEXEC (0x2000) only.
///
/// # Returns
/// The new file descriptor number on success.
pub fn sys_dup3(old_fd: usize, new_fd: usize, flags: usize) -> SyscallResult {
    // old_fd and new_fd must differ
    if old_fd == new_fd {
        return Err(SyscallError::InvalidArgument);
    }

    // Only O_CLOEXEC is valid
    if flags & !0x2000 != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let cloexec = flags & 0x2000 != 0;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();

    file_table
        .dup3(old_fd, new_fd, cloexec)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(new_fd)
}

/// Open a directory for reading (syscall 62).
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated directory path.
///
/// # Returns
/// Directory handle (fd) on success.
pub fn sys_opendir(path_ptr: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();

    // Verify path exists and is a directory
    let node = vfs_guard
        .resolve_path(&path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    let metadata = node.metadata().map_err(|_| SyscallError::InvalidState)?;
    if metadata.node_type != crate::fs::NodeType::Directory {
        return Err(SyscallError::InvalidArgument);
    }

    // Open as a file descriptor using read-only flags
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = crate::fs::file::File::new(node, OpenFlags::read_only());
    let file_table = proc.file_table.lock();
    match file_table.open(alloc::sync::Arc::new(file)) {
        Ok(fd) => Ok(fd),
        Err(_) => Err(SyscallError::OutOfMemory),
    }
}

/// Read a directory entry (syscall 63).
///
/// Reads entries from the VFS node via VfsNode::readdir(). Uses the
/// file's position (via seek) as an index into the directory entry list.
/// Returns one entry per call; returns 0 when all entries have been read.
///
/// The entry is written as a NUL-terminated name string followed by a
/// single byte indicating the node type (0=file, 1=dir, 2=chardev,
/// 3=blockdev, 4=symlink, 5=pipe).
///
/// # Arguments
/// - `fd`: Directory file descriptor (from opendir).
/// - `entry_buf`: Buffer to receive directory entry name + type byte.
/// - `buf_size`: Size of the buffer.
///
/// # Returns
/// Length of entry name (not including NUL or type byte), or 0 if no more
/// entries.
pub fn sys_readdir(fd: usize, entry_buf: usize, buf_size: usize) -> SyscallResult {
    if buf_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(entry_buf, buf_size)?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Read all directory entries from the VFS node
    let entries = file_desc
        .node
        .readdir()
        .map_err(|_| SyscallError::InvalidArgument)?;

    // Use the file position as the current entry index
    let pos = file_desc.tell();
    if pos >= entries.len() {
        return Ok(0); // No more entries
    }

    let entry = &entries[pos];
    let name_bytes = entry.name.as_bytes();

    // Need space for name + NUL + type byte
    if name_bytes.len() + 2 > buf_size {
        return Err(SyscallError::InvalidArgument);
    }

    // Write entry name + NUL terminator + node type byte to user buffer
    // SAFETY: entry_buf was validated above as non-null and in user space
    // with sufficient size. We write name_bytes.len() + 2 bytes total.
    unsafe {
        let dst = entry_buf as *mut u8;
        core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), dst, name_bytes.len());
        *dst.add(name_bytes.len()) = 0; // NUL terminator
        *dst.add(name_bytes.len() + 1) = match entry.node_type {
            crate::fs::NodeType::File => 0,
            crate::fs::NodeType::Directory => 1,
            crate::fs::NodeType::CharDevice => 2,
            crate::fs::NodeType::BlockDevice => 3,
            crate::fs::NodeType::Symlink => 4,
            crate::fs::NodeType::Pipe => 5,
            _ => 0,
        };
    }

    // Advance the file position to the next entry
    let _ = file_desc.seek(crate::fs::SeekFrom::Start(pos + 1));

    Ok(name_bytes.len())
}

/// Close a directory handle (syscall 64).
///
/// # Arguments
/// - `fd`: Directory file descriptor to close.
///
/// # Returns
/// 0 on success.
pub fn sys_closedir(fd: usize) -> SyscallResult {
    // Closing a directory fd is the same as closing any fd
    sys_close(fd)
}

// ============================================================================
// Scatter/gather I/O syscalls (183-184)
// ============================================================================

/// POSIX iovec structure layout (matches C struct iovec).
#[repr(C)]
#[derive(Clone, Copy)]
struct Iovec {
    iov_base: usize,
    iov_len: usize,
}

/// Maximum number of iovec entries per readv/writev call.
const IOV_MAX: usize = 1024;

/// Read from a file descriptor into multiple buffers (SYS_READV = 183).
///
/// # Arguments
/// - `fd`: File descriptor to read from.
/// - `iov_ptr`: Pointer to an array of `struct iovec`.
/// - `iovcnt`: Number of iovec entries.
///
/// # Returns
/// Total number of bytes read across all buffers.
pub fn sys_readv(fd: usize, iov_ptr: usize, iovcnt: usize) -> SyscallResult {
    if iovcnt == 0 {
        return Ok(0);
    }
    if iovcnt > IOV_MAX {
        return Err(SyscallError::InvalidArgument);
    }

    // Validate the iovec array itself
    let iov_size = iovcnt * core::mem::size_of::<Iovec>();
    validate_user_buffer(iov_ptr, iov_size)?;

    let mut total_read = 0usize;

    for i in 0..iovcnt {
        // SAFETY: iov_ptr was validated above. Each Iovec is repr(C) with
        // known size. We read iov entries one at a time within bounds.
        let iov = unsafe {
            let ptr = (iov_ptr as *const Iovec).add(i);
            core::ptr::read(ptr)
        };

        if iov.iov_len == 0 {
            continue;
        }

        // Delegate to existing sys_read for each segment
        match sys_read(fd, iov.iov_base, iov.iov_len) {
            Ok(n) => {
                total_read += n;
                // Short read means EOF or no more data available
                if n < iov.iov_len {
                    break;
                }
            }
            Err(e) => {
                // If we already read some data, return what we have
                if total_read > 0 {
                    break;
                }
                return Err(e);
            }
        }
    }

    Ok(total_read)
}

/// Write to a file descriptor from multiple buffers (SYS_WRITEV = 184).
///
/// # Arguments
/// - `fd`: File descriptor to write to.
/// - `iov_ptr`: Pointer to an array of `struct iovec`.
/// - `iovcnt`: Number of iovec entries.
///
/// # Returns
/// Total number of bytes written across all buffers.
pub fn sys_writev(fd: usize, iov_ptr: usize, iovcnt: usize) -> SyscallResult {
    if iovcnt == 0 {
        return Ok(0);
    }
    if iovcnt > IOV_MAX {
        return Err(SyscallError::InvalidArgument);
    }

    // Validate the iovec array itself
    let iov_size = iovcnt * core::mem::size_of::<Iovec>();
    validate_user_buffer(iov_ptr, iov_size)?;

    let mut total_written = 0usize;

    for i in 0..iovcnt {
        // SAFETY: iov_ptr was validated above. Each Iovec is repr(C) with
        // known size. We read iov entries one at a time within bounds.
        let iov = unsafe {
            let ptr = (iov_ptr as *const Iovec).add(i);
            core::ptr::read(ptr)
        };

        if iov.iov_len == 0 {
            continue;
        }

        // Delegate to existing sys_write for each segment
        match sys_write(fd, iov.iov_base, iov.iov_len) {
            Ok(n) => {
                total_written += n;
                // Short write means buffer full or error
                if n < iov.iov_len {
                    break;
                }
            }
            Err(e) => {
                // If we already wrote some data, return what we have
                if total_written > 0 {
                    break;
                }
                return Err(e);
            }
        }
    }

    Ok(total_written)
}

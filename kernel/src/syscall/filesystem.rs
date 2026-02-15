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

use super::{SyscallError, SyscallResult};
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
///
/// For fd 0 (stdin), if the process does not have a file descriptor table
/// entry, falls back to polling-mode serial UART input. This allows the
/// embedded shell to accept keyboard input before a full console subsystem
/// is initialized.
pub fn sys_read(fd: usize, buffer: usize, count: usize) -> SyscallResult {
    // Validate buffer
    if buffer == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    if count == 0 {
        return Ok(0);
    }

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
    // Validate buffer
    if buffer == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    if count == 0 {
        return Ok(0);
    }

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

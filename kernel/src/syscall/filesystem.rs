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
pub fn sys_open(path: usize, flags: usize, mode: usize) -> SyscallResult {
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

    // Diagnostic: print open attempts for debugging file access issues
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[OPEN] ");
        crate::arch::x86_64::idt::raw_serial_str(&path_bytes[..path_bytes.len().min(80)]);
        crate::arch::x86_64::idt::raw_serial_str(b" flags=0x");
        crate::arch::x86_64::idt::raw_serial_hex(flags as u64);
        crate::arch::x86_64::idt::raw_serial_str(b"\n");
    }

    // Open the file through VFS
    match vfs()?.read().open(path_str, open_flags) {
        Ok(node) => {
            // Create file with path stored for dirfd resolution
            let file = crate::fs::file::File::new_with_path(
                node,
                open_flags,
                alloc::string::String::from(path_str),
            );

            // Add to process file table
            let file_table = process.file_table.lock();
            let arc_file = alloc::sync::Arc::new(file);
            match file_table.open(arc_file.clone()) {
                Ok(fd_num) => {
                    // Trace fd number and file size for debugging
                    #[cfg(target_arch = "x86_64")]
                    if fd_num >= 3 {
                        let sz = arc_file.node.metadata().map(|m| m.size).unwrap_or(0);
                        unsafe {
                            crate::arch::x86_64::idt::raw_serial_str(b"  -> fd=");
                            crate::arch::x86_64::idt::raw_serial_hex(fd_num as u64);
                            crate::arch::x86_64::idt::raw_serial_str(b" sz=0x");
                            crate::arch::x86_64::idt::raw_serial_hex(sz as u64);
                            crate::arch::x86_64::idt::raw_serial_str(b"\n");
                        }
                    }
                    Ok(fd_num)
                }
                Err(_) => Err(SyscallError::OutOfMemory),
            }
        }
        Err(_e) => {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                crate::arch::x86_64::idt::raw_serial_str(b"[OPEN] FAIL: ");
                crate::arch::x86_64::idt::raw_serial_str(&path_bytes[..path_bytes.len().min(60)]);
                crate::arch::x86_64::idt::raw_serial_str(b"\n");
            }
            // If O_CREAT is set, create the file in its parent directory
            if open_flags.create {
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"[CREAT] flags=create\n");
                }
                let perms = Permissions::from_mode(mode as u32);
                let (parent_path, name) = split_path(path_str)?;
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"[CREAT] parent=");
                    crate::arch::x86_64::idt::raw_serial_str(parent_path.as_bytes());
                    crate::arch::x86_64::idt::raw_serial_str(b" name=");
                    crate::arch::x86_64::idt::raw_serial_str(name.as_bytes());
                    crate::arch::x86_64::idt::raw_serial_str(b"\n");
                }
                let vfs_guard = vfs()?.read();
                let parent = match vfs_guard.resolve_path(&parent_path) {
                    Ok(p) => p,
                    Err(_e) => {
                        #[cfg(target_arch = "x86_64")]
                        unsafe {
                            crate::arch::x86_64::idt::raw_serial_str(b"[CREAT] resolve_path FAIL\n");
                        }
                        return Err(SyscallError::ResourceNotFound);
                    }
                };
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"[CREAT] parent resolved OK\n");
                }
                match parent.create(&name, perms) {
                    Ok(node) => {
                        #[cfg(target_arch = "x86_64")]
                        unsafe {
                            crate::arch::x86_64::idt::raw_serial_str(b"[CREAT] file created OK\n");
                        }
                        let file = crate::fs::file::File::new_with_path(
                            node,
                            open_flags,
                            alloc::string::String::from(path_str),
                        );
                        let file_table = process.file_table.lock();
                        match file_table.open(alloc::sync::Arc::new(file)) {
                            Ok(fd_num) => Ok(fd_num),
                            Err(_) => Err(SyscallError::OutOfMemory),
                        }
                    }
                    Err(_e) => {
                        #[cfg(target_arch = "x86_64")]
                        unsafe {
                            crate::arch::x86_64::idt::raw_serial_str(b"[CREAT] create() FAIL\n");
                        }
                        Err(SyscallError::ResourceNotFound)
                    }
                }
            } else {
                Err(SyscallError::ResourceNotFound)
            }
        }
    }
}

/// Close a file descriptor
///
/// # Arguments
/// - fd: File descriptor to close
pub fn sys_close(fd: usize) -> SyscallResult {
    // DEBUG: trace close for high fds
    #[cfg(target_arch = "x86_64")]
    if fd >= 3 {
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[CL] fd=");
            crate::arch::x86_64::idt::raw_serial_hex(fd as u64);
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

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

        // Fallback: read from serial UART, respecting terminal state.
        let read_count = count.min(SERIAL_IO_MAX_SIZE);
        // SAFETY: buffer is non-zero (checked above). We limit the size
        // via SERIAL_IO_MAX_SIZE. The caller must provide a valid writable
        // buffer of at least `count` bytes. During early bring-up this
        // may be a kernel-space address from the embedded init binary.
        let buffer_slice =
            unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, read_count) };

        let canonical = crate::drivers::terminal::is_canonical_mode();
        let echo = crate::drivers::terminal::is_echo_enabled();

        let mut bytes_read = 0;
        for slot in buffer_slice.iter_mut() {
            // Spin-wait for a byte to become available
            let byte = loop {
                if let Some(b) = serial_try_read_byte() {
                    break b;
                }
                core::hint::spin_loop();
            };

            // Echo if enabled
            if echo {
                serial_write_byte(byte);
            }

            *slot = byte;
            bytes_read += 1;

            // In canonical mode, stop after newline or carriage return.
            // In raw mode, return immediately after each character (VMIN=1).
            if canonical {
                if byte == b'\n' || byte == b'\r' {
                    break;
                }
            } else {
                // Raw mode: return after first character
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

    // DEBUG: dump position + first 4 bytes for ld archive reads (fd >= 3)
    let pre_pos = file_desc.tell();

    match file_desc.read(buffer_slice) {
        Ok(bytes_read) => {
            if fd >= 3 && bytes_read >= 4 {
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"[RD] ");
                    crate::arch::x86_64::idt::raw_serial_hex(fd as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" @");
                    crate::arch::x86_64::idt::raw_serial_hex(pre_pos as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" n=");
                    crate::arch::x86_64::idt::raw_serial_hex(bytes_read as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" [");
                    for i in 0..4.min(bytes_read) {
                        crate::arch::x86_64::idt::raw_serial_hex(buffer_slice[i] as u64);
                        if i < 3 {
                            crate::arch::x86_64::idt::raw_serial_str(b" ");
                        }
                    }
                    crate::arch::x86_64::idt::raw_serial_str(b"]\n");
                }
            }
            Ok(bytes_read)
        }
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
        Ok(new_pos) => {
            // DEBUG: trace seeks on high fds (ld archive processing)
            #[cfg(target_arch = "x86_64")]
            if fd >= 3 {
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"[SK] ");
                    crate::arch::x86_64::idt::raw_serial_hex(fd as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" w=");
                    crate::arch::x86_64::idt::raw_serial_hex(whence as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" off=");
                    crate::arch::x86_64::idt::raw_serial_hex(offset as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" ->pos=");
                    crate::arch::x86_64::idt::raw_serial_hex(new_pos as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b"\n");
                }
            }
            Ok(new_pos as usize)
        }
        Err(_) => {
            // Trace failed seeks too (for ld debugging)
            #[cfg(target_arch = "x86_64")]
            if fd >= 3 {
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"[SK-ERR] ");
                    crate::arch::x86_64::idt::raw_serial_hex(fd as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" w=");
                    crate::arch::x86_64::idt::raw_serial_hex(whence as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b" off=");
                    crate::arch::x86_64::idt::raw_serial_hex(offset as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b"\n");
                }
            }
            Err(SyscallError::InvalidState)
        }
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

    // Get metadata and write to user buffer
    let metadata = file_desc
        .node
        .metadata()
        .map_err(|_| SyscallError::InvalidState)?;
    let stat = fill_stat(&metadata);

    // DEBUG: trace fstat for high fds (ld archive processing)
    #[cfg(target_arch = "x86_64")]
    if fd >= 3 {
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[STAT] fd=");
            crate::arch::x86_64::idt::raw_serial_hex(fd as u64);
            crate::arch::x86_64::idt::raw_serial_str(b" sz=");
            crate::arch::x86_64::idt::raw_serial_hex(stat.st_size as u64);
            crate::arch::x86_64::idt::raw_serial_str(b" mode=");
            crate::arch::x86_64::idt::raw_serial_hex(stat.st_mode as u64);
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    // SAFETY: stat_buf was validated as non-null, in user-space, and aligned.
    unsafe {
        core::ptr::write(stat_buf as *mut FileStat, stat);
    }
    Ok(0)
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

/// File stat structure for userspace.
///
/// Layout matches the C `struct stat` in `veridian/stat.h` exactly.
/// All fields use fixed-size types matching the C typedefs in
/// `veridian/types.h`.
#[repr(C)]
struct FileStat {
    st_dev: u64,     // dev_t
    st_ino: u64,     // ino_t
    st_mode: u32,    // mode_t
    st_nlink: u64,   // nlink_t
    st_uid: u32,     // uid_t
    st_gid: u32,     // gid_t
    st_rdev: u64,    // dev_t
    st_size: i64,    // off_t
    st_blksize: i64, // blksize_t
    st_blocks: i64,  // blkcnt_t
    st_atime: i64,   // time_t
    st_mtime: i64,   // time_t
    st_ctime: i64,   // time_t
}

/// Helper: populate a FileStat from VFS metadata.
fn fill_stat(metadata: &crate::fs::Metadata) -> FileStat {
    let mode = match metadata.node_type {
        crate::fs::NodeType::File => 0o100644,
        crate::fs::NodeType::Directory => 0o040755,
        crate::fs::NodeType::CharDevice => 0o020666,
        crate::fs::NodeType::BlockDevice => 0o060666,
        _ => 0,
    };
    let size = metadata.size as i64;
    FileStat {
        st_dev: 0,
        st_ino: 0,
        st_mode: mode,
        st_nlink: 1,
        st_uid: metadata.uid,
        st_gid: metadata.gid,
        st_rdev: 0,
        st_size: size,
        st_blksize: 4096,
        st_blocks: (size + 511) / 512,
        st_atime: metadata.accessed as i64,
        st_mtime: metadata.modified as i64,
        st_ctime: metadata.created as i64,
    }
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

    let cwd = {
        let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
        #[cfg(feature = "alloc")]
        {
            thread.fs().cwd.lock().clone()
        }
        #[cfg(not(feature = "alloc"))]
        {
            alloc::string::String::from("/")
        }
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

    let path = read_user_path(path_ptr)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    #[cfg(not(feature = "alloc"))]
    {
        let _ = (path, thread);
        return Err(SyscallError::InvalidState);
    }
    #[cfg(feature = "alloc")]
    {
        let cwd = thread.fs().cwd.lock().clone();
        let vfs_lock = vfs()?;
        let vfs_guard = vfs_lock.read();
        let node = vfs_guard
            .resolve_from(&path, &cwd)
            .map_err(|_| SyscallError::ResourceNotFound)?;
        if node.node_type() != crate::fs::NodeType::Directory {
            return Err(SyscallError::InvalidArgument);
        }
        // Update per-thread cwd
        let normalized = crate::process::cwd::resolve_path(&path, &cwd);
        *thread.fs().cwd.lock() = normalized;
        Ok(0)
    }
}

/// I/O control operations on a file descriptor
///
/// Handles terminal ioctls (TCGETS, TCSETS, TCSETSW, TCSETSF, TIOCGWINSZ,
/// etc.) using the real terminal state from `drivers::terminal`. Changes
/// to terminal attributes (e.g., clearing ICANON for raw mode) take effect
/// immediately and are visible to subsequent console reads.
///
/// # Arguments
/// - fd: File descriptor
/// - cmd: I/O control command
/// - arg: Command-specific argument
///
/// # Returns
/// Command-specific return value
pub fn sys_ioctl(fd: usize, cmd: usize, arg: usize) -> SyscallResult {
    use crate::drivers::terminal::{
        self, KernelTermios, KernelWinsize, TCGETS, TCSETS, TCSETSF, TCSETSW, TIOCGPGRP,
        TIOCGWINSZ, TIOCSPGRP, TIOCSWINSZ,
    };

    // Terminal ioctls are only valid on terminal fds (0=stdin, 1=stdout,
    // 2=stderr which are connected to the serial console). Regular files
    // opened via open() must return ENOTTY so that isatty() returns false
    // and BFD/stdio treat them as seekable files, not terminal streams.
    let is_terminal_cmd = matches!(
        cmd,
        TIOCGWINSZ | TIOCSWINSZ | TCGETS | TCSETS | TCSETSW | TCSETSF | TIOCGPGRP | TIOCSPGRP
    );
    if is_terminal_cmd && fd > 2 {
        return Err(SyscallError::InvalidArgument);
    }

    match cmd {
        TIOCGWINSZ => {
            // Return terminal window size from real terminal state
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelWinsize>(arg)?;

            let ws = terminal::get_winsize_snapshot();
            // SAFETY: arg was validated as aligned, non-null, and in user space.
            unsafe {
                core::ptr::write(arg as *mut KernelWinsize, ws);
            }
            Ok(0)
        }
        TIOCSWINSZ => {
            // Set window size in terminal state
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelWinsize>(arg)?;

            // SAFETY: arg was validated as aligned, non-null, and in user space.
            let ws = unsafe { core::ptr::read(arg as *const KernelWinsize) };
            terminal::set_winsize(&ws);
            Ok(0)
        }
        TCGETS => {
            // Get terminal attributes from real terminal state
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelTermios>(arg)?;

            let termios = terminal::get_termios_snapshot();
            // SAFETY: arg was validated above.
            unsafe {
                core::ptr::write(arg as *mut KernelTermios, termios);
            }
            Ok(0)
        }
        TCSETS => {
            // Set terminal attributes immediately
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelTermios>(arg)?;

            // SAFETY: arg was validated as aligned, non-null, and in user space.
            let new_termios = unsafe { core::ptr::read(arg as *const KernelTermios) };
            terminal::set_termios(&new_termios);
            Ok(0)
        }
        TCSETSW => {
            // Set terminal attributes after draining output.
            // For serial console, output is always drained (synchronous),
            // so this is equivalent to TCSETS.
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelTermios>(arg)?;

            // SAFETY: arg was validated above.
            let new_termios = unsafe { core::ptr::read(arg as *const KernelTermios) };
            terminal::set_termios(&new_termios);
            Ok(0)
        }
        TCSETSF => {
            // Set terminal attributes after draining output and flushing input.
            // For serial console, no input buffer to flush, so equivalent to TCSETS.
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelTermios>(arg)?;

            // SAFETY: arg was validated above.
            let new_termios = unsafe { core::ptr::read(arg as *const KernelTermios) };
            terminal::set_termios(&new_termios);
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
            let b = core::ptr::read_volatile(p);
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

    let metadata = node.metadata().map_err(|_| SyscallError::InvalidState)?;
    let stat = fill_stat(&metadata);
    // SAFETY: stat_buf was validated as non-null, in user-space, and aligned.
    unsafe {
        core::ptr::write(stat_buf as *mut FileStat, stat);
    }
    Ok(0)
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
/// Reads the target path that a symbolic link points to, without following
/// the link. The target string is written to the user-space buffer `buf`
/// and is NOT null-terminated (matching POSIX readlink(2) semantics).
///
/// If the target string is longer than `bufsiz`, it is silently truncated
/// to `bufsiz` bytes. The caller should allocate a buffer of at least
/// `PATH_MAX` bytes to avoid truncation.
///
/// # Arguments
/// - `path_ptr`: Pointer to a NUL-terminated path in user space that names the
///   symbolic link to read.
/// - `buf`: Pointer to a user-space buffer to receive the link target.
/// - `bufsiz`: Size of the buffer in bytes. Must be > 0.
///
/// # Returns
/// - `Ok(n)`: Number of bytes written to `buf` (not null-terminated). This is
///   `min(target.len(), bufsiz)`.
/// - `Err(InvalidArgument)`: `bufsiz` is 0, or the node is not a symlink.
/// - `Err(ResourceNotFound)`: The path does not exist.
///
/// # Errors
/// - If the VFS node at `path` does not support `readlink()` (i.e., is not a
///   symbolic link), the VfsNode default implementation returns
///   `NotImplemented`, which is mapped to `InvalidArgument` here.
pub fn sys_readlink(path_ptr: usize, buf: usize, bufsiz: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;
    if bufsiz == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(buf, bufsiz)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();

    let node = vfs_guard
        .resolve_path(&path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    // readlink operates on the link node itself; if the node is not a
    // symlink, readlink() returns NotImplemented or NotASymlink.
    let target = node.readlink().map_err(|_| SyscallError::InvalidArgument)?;

    let bytes = target.as_bytes();
    let to_copy = core::cmp::min(bytes.len(), bufsiz);

    // SAFETY: buf was validated above as non-null and in user space with
    // at least bufsiz bytes. We copy at most bufsiz bytes of the target
    // string. copy_slice_to_user handles the raw pointer write.
    unsafe {
        crate::syscall::userspace::copy_slice_to_user(buf, &bytes[..to_copy])
            .map_err(|_| SyscallError::InvalidArgument)?;
    }

    Ok(to_copy)
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
    let result = vfs_guard.resolve_path(&path);

    // Diagnostic: log access() calls to trace GCC's tool discovery
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[ACCESS] ");
        for &b in path.as_bytes() {
            crate::arch::x86_64::idt::raw_serial_str(&[b]);
        }
        if result.is_ok() {
            crate::arch::x86_64::idt::raw_serial_str(b" OK\n");
        } else {
            crate::arch::x86_64::idt::raw_serial_str(b" ENOENT\n");
        }
    }

    let node = result.map_err(|_| SyscallError::ResourceNotFound)?;

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

// ============================================================================
// Self-hosting syscalls (Phase 4A: Tiers 1)
// ============================================================================

/// AT_FDCWD sentinel: use process current working directory.
/// Must match the C-side `#define AT_FDCWD (-100)` in syscall.h.
const AT_FDCWD: usize = (-100isize) as usize;

/// Create a hard link (syscall 155).
///
/// Creates a new directory entry `new_path` pointing to the same file as
/// `old_path`. Both paths must be on the same filesystem.
pub fn sys_link(old_ptr: usize, new_ptr: usize) -> SyscallResult {
    let old_path = read_user_path(old_ptr)?;
    let new_path = read_user_path(new_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();

    // Resolve the old path to get the target node
    let target = vfs_guard
        .resolve_path(&old_path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    // Split new_path into parent dir + name
    let (parent_path, link_name) = split_path(&new_path)?;

    let parent = vfs_guard
        .resolve_path(&parent_path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    parent
        .link(&link_name, target)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Create a symbolic link (syscall 156).
///
/// Creates a symlink at `link_path` pointing to `target`.
pub fn sys_symlink(target_ptr: usize, link_ptr: usize) -> SyscallResult {
    let target = read_user_path(target_ptr)?;
    let link_path = read_user_path(link_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();

    let (parent_path, link_name) = split_path(&link_path)?;

    let parent = vfs_guard
        .resolve_path(&parent_path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    parent
        .symlink(&link_name, &target)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Change file permissions by path (syscall 185).
pub fn sys_chmod(path_ptr: usize, mode: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();
    let node = vfs_guard
        .resolve_path(&path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    let perms = Permissions::from_mode(mode as u32);
    node.chmod(perms)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Change file permissions by fd (syscall 186).
pub fn sys_fchmod(fd: usize, mode: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    let perms = Permissions::from_mode(mode as u32);
    file.node
        .chmod(perms)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Set file creation mask (syscall 187).
///
/// Returns the previous umask value.
pub fn sys_umask(mask: usize) -> SyscallResult {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    #[cfg(feature = "alloc")]
    {
        let old = thread
            .fs()
            .umask
            .swap(mask as u32 & 0o777, core::sync::atomic::Ordering::AcqRel);
        Ok(old as usize)
    }
    #[cfg(not(feature = "alloc"))]
    {
        let _ = (mask, thread);
        Err(SyscallError::InvalidState)
    }
}

/// Truncate a file by path (syscall 188).
pub fn sys_truncate_path(path_ptr: usize, size: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();
    let node = vfs_guard
        .resolve_path(&path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    node.truncate(size)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Poll file descriptors for readiness (syscall 189).
///
/// Simplified implementation: checks each fd for readability/writability
/// and returns immediately (no blocking).
///
/// # Arguments
/// - `fds_ptr`: Pointer to array of PollFd structs.
/// - `nfds`: Number of entries.
/// - `_timeout_ms`: Timeout in milliseconds (ignored — always returns
///   immediately).
pub fn sys_poll(fds_ptr: usize, nfds: usize, _timeout_ms: usize) -> SyscallResult {
    if nfds == 0 {
        return Ok(0);
    }
    if nfds > 256 {
        return Err(SyscallError::InvalidArgument);
    }

    validate_user_buffer(fds_ptr, nfds * core::mem::size_of::<PollFd>())?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let mut ready_count = 0usize;

    for i in 0..nfds {
        // SAFETY: fds_ptr was validated above and PollFd is repr(C).
        let pollfd = unsafe { &mut *((fds_ptr as *mut PollFd).add(i)) };
        pollfd.revents = 0;

        if pollfd.fd < 0 {
            continue;
        }

        if let Some(_file) = file_table.get(pollfd.fd as usize) {
            // Simplified: files/pipes are always readable and writable for now.
            // A proper implementation would check pipe buffer state.
            if pollfd.events & POLLIN != 0 {
                pollfd.revents |= POLLIN;
            }
            if pollfd.events & POLLOUT != 0 {
                pollfd.revents |= POLLOUT;
            }
            if pollfd.revents != 0 {
                ready_count += 1;
            }
        } else {
            pollfd.revents = POLLNVAL;
            ready_count += 1;
        }
    }

    Ok(ready_count)
}

/// Poll event flags
const POLLIN: i16 = 0x001;
const POLLOUT: i16 = 0x004;
const POLLNVAL: i16 = 0x020;

/// Poll file descriptor structure (matches C struct pollfd).
#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

/// Resolve a path relative to a directory fd.
///
/// If `dirfd == AT_FDCWD`, uses the process CWD. Otherwise resolves the
/// path relative to the directory referred to by dirfd.
fn resolve_at_path(dirfd: usize, path: &str) -> Result<alloc::string::String, SyscallError> {
    use alloc::string::String;

    if path.starts_with('/') {
        // Absolute path — dirfd is irrelevant
        return Ok(String::from(path));
    }

    if dirfd == AT_FDCWD {
        // Relative to CWD
        let cwd = if let Some(vfs_lock) = try_get_vfs() {
            let vfs_guard = vfs_lock.read();
            String::from(vfs_guard.get_cwd())
        } else {
            String::from("/")
        };
        if cwd.ends_with('/') {
            Ok(alloc::format!("{}{}", cwd, path))
        } else {
            Ok(alloc::format!("{}/{}", cwd, path))
        }
    } else {
        // Relative to directory fd — look up the fd in the file table
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let file_table = proc.file_table.lock();
        let file = file_table.get(dirfd).ok_or(SyscallError::InvalidArgument)?;

        // Use the stored path if available, otherwise fall back to "/"
        let dir_path = if let Some(ref p) = file.path {
            String::from(p.as_str())
        } else {
            // No stored path — fall back to root (best effort)
            String::from("/")
        };

        if dir_path.ends_with('/') {
            Ok(alloc::format!("{}{}", dir_path, path))
        } else {
            Ok(alloc::format!("{}/{}", dir_path, path))
        }
    }
}

/// Open a file relative to a directory fd (syscall 190).
pub fn sys_openat(dirfd: usize, path_ptr: usize, flags: usize, mode: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;

    // Delegate to sys_open using the resolved absolute path.
    // We write the path to a temporary kernel buffer, then call the existing
    // sys_open logic. Since sys_open reads from a user pointer, we use the
    // VFS directly instead.
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let open_flags = OpenFlags::from_bits(flags as u32).ok_or(SyscallError::InvalidArgument)?;

    match vfs()?.read().open(&abs_path, open_flags) {
        Ok(node) => {
            let file = crate::fs::file::File::new(node, open_flags);
            let file_table = proc.file_table.lock();
            match file_table.open(alloc::sync::Arc::new(file)) {
                Ok(fd_num) => Ok(fd_num),
                Err(_) => Err(SyscallError::OutOfMemory),
            }
        }
        Err(_) => {
            // If O_CREAT, create the file
            if open_flags.create {
                let perms = Permissions::from_mode(mode as u32);
                let (parent_path, name) = split_path(&abs_path)?;
                let vfs_guard = vfs()?.read();
                let parent = vfs_guard
                    .resolve_path(&parent_path)
                    .map_err(|_| SyscallError::ResourceNotFound)?;
                match parent.create(&name, perms) {
                    Ok(node) => {
                        let file = crate::fs::file::File::new(node, open_flags);
                        let file_table = proc.file_table.lock();
                        match file_table.open(alloc::sync::Arc::new(file)) {
                            Ok(fd_num) => Ok(fd_num),
                            Err(_) => Err(SyscallError::OutOfMemory),
                        }
                    }
                    Err(_) => Err(SyscallError::ResourceNotFound),
                }
            } else {
                Err(SyscallError::ResourceNotFound)
            }
        }
    }
}

/// Stat a file relative to a directory fd (syscall 191).
pub fn sys_fstatat(dirfd: usize, path_ptr: usize, stat_buf: usize, _flags: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;

    validate_user_ptr_typed::<FileStat>(stat_buf)?;

    let vfs_lock = vfs()?;
    let vfs_guard = vfs_lock.read();
    let node = vfs_guard
        .resolve_path(&abs_path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    let metadata = node.metadata().map_err(|_| SyscallError::InvalidState)?;
    let stat = fill_stat(&metadata);
    // SAFETY: stat_buf was validated above.
    unsafe {
        core::ptr::write(stat_buf as *mut FileStat, stat);
    }
    Ok(0)
}

/// Unlink a file relative to a directory fd (syscall 192).
///
/// If flags contains AT_REMOVEDIR (0x200), acts like rmdir.
pub fn sys_unlinkat(dirfd: usize, path_ptr: usize, _flags: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;

    let vfs_lock = vfs()?;
    vfs_lock
        .read()
        .unlink(&abs_path)
        .map_err(|_| SyscallError::ResourceNotFound)?;

    Ok(0)
}

/// Create a directory relative to a directory fd (syscall 193).
pub fn sys_mkdirat(dirfd: usize, path_ptr: usize, mode: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;

    let permissions = Permissions::from_mode(mode as u32);
    vfs()?
        .read()
        .mkdir(&abs_path, permissions)
        .map_err(|_| SyscallError::InvalidState)?;

    Ok(0)
}

/// Rename a file relative to directory fds (syscall 194).
pub fn sys_renameat(
    olddirfd: usize,
    old_ptr: usize,
    newdirfd: usize,
    new_ptr: usize,
) -> SyscallResult {
    let old_rel = read_user_path(old_ptr)?;
    let new_rel = read_user_path(new_ptr)?;
    let old_abs = resolve_at_path(olddirfd, &old_rel)?;
    let new_abs = resolve_at_path(newdirfd, &new_rel)?;

    // Rename as copy + delete
    let data = crate::fs::read_file(&old_abs).map_err(|_| SyscallError::ResourceNotFound)?;
    crate::fs::write_file(&new_abs, &data).map_err(|_| SyscallError::InvalidState)?;
    vfs()?
        .read()
        .unlink(&old_abs)
        .map_err(|_| SyscallError::InvalidState)?;

    Ok(0)
}

/// Read from a file descriptor at a given offset without changing position
/// (syscall 195).
pub fn sys_pread(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Read directly at offset through the VfsNode, bypassing File position
    // SAFETY: buf was validated above.
    let buffer_slice = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, count) };
    match file.node.read(offset, buffer_slice) {
        Ok(n) => Ok(n),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Write to a file descriptor at a given offset without changing position
/// (syscall 196).
pub fn sys_pwrite(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;

    // Write directly at offset through the VfsNode, bypassing File position
    // SAFETY: buf was validated above.
    let buffer_slice = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
    match file.node.write(offset, buffer_slice) {
        Ok(n) => Ok(n),
        Err(_) => Err(SyscallError::InvalidState),
    }
}

/// Helper: split a path into (parent_dir, basename).
fn split_path(path: &str) -> Result<(alloc::string::String, alloc::string::String), SyscallError> {
    use alloc::string::String;

    if let Some(pos) = path.rfind('/') {
        let parent = if pos == 0 {
            String::from("/")
        } else {
            String::from(&path[..pos])
        };
        let name = String::from(&path[pos + 1..]);
        if name.is_empty() {
            return Err(SyscallError::InvalidArgument);
        }
        Ok((parent, name))
    } else {
        // No slash — parent is CWD
        let cwd = if let Some(vfs_lock) = try_get_vfs() {
            let vfs_guard = vfs_lock.read();
            String::from(vfs_guard.get_cwd())
        } else {
            String::from("/")
        };
        Ok((cwd, String::from(path)))
    }
}

// =========================================================================
// Ownership and device node syscalls (197-200)
// =========================================================================

/// Change ownership of a file by path (syscall 197).
///
/// Stub: accepts but ignores — no real UID/GID enforcement yet.
pub fn sys_chown(path_ptr: usize, uid: usize, gid: usize) -> SyscallResult {
    let _path = read_user_path(path_ptr)?;
    let _uid = uid as u32;
    let _gid = gid as u32;
    // No-op: accept but don't enforce ownership changes
    Ok(0)
}

/// Change ownership of a file by file descriptor (syscall 198).
///
/// Stub: accepts but ignores — no real UID/GID enforcement yet.
pub fn sys_fchown(fd: usize, uid: usize, gid: usize) -> SyscallResult {
    let _fd = fd;
    let _uid = uid as u32;
    let _gid = gid as u32;
    // Verify fd is valid
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let _file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;
    // No-op: accept but don't enforce ownership changes
    Ok(0)
}

/// Create a special or ordinary file (syscall 199).
///
/// Stub: returns EPERM — device file creation not supported.
pub fn sys_mknod(path_ptr: usize, _mode: usize, _dev: usize) -> SyscallResult {
    let _path = read_user_path(path_ptr)?;
    Err(SyscallError::PermissionDenied)
}

/// Synchronous I/O multiplexing (syscall 200).
///
/// Thin wrapper that converts select-style arguments to poll().
pub fn sys_select(
    nfds: usize,
    readfds_ptr: usize,
    writefds_ptr: usize,
    exceptfds_ptr: usize,
    timeout_ptr: usize,
) -> SyscallResult {
    // Delegate to poll with a simplified implementation.
    // The user-space select() in libc converts fd_sets to pollfd
    // arrays and calls poll() directly. This kernel-side select
    // is provided for completeness but may not be called if libc
    // does the conversion itself.
    let _ = (nfds, readfds_ptr, writefds_ptr, exceptfds_ptr, timeout_ptr);

    // For now, return 0 (no fds ready, immediate timeout).
    // The libc select() implementation uses poll() directly,
    // so this kernel path is not critical.
    Ok(0)
}

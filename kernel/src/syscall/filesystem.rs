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

/// Whether a read, write or terminal ioctl on standard descriptor `fd`
/// (0-2) with no file-table entry may fall back to the serial console.
/// Only while it is still the implicit console: after `close(fd)` the
/// descriptor is closed and must fail with EBADF (review of the v0.26.0
/// stack, PR #9). Without a process (kernel boot context) the console is
/// always available.
fn console_fallback_allowed(fd: usize) -> bool {
    match process::current_process() {
        Some(proc) => proc.file_table.lock().is_implicit_console(fd),
        None => fd < 3,
    }
}

/// Maximum buffer size for serial I/O fallback (64 KB).
/// Prevents unbounded kernel-side loops for large writes.
const SERIAL_IO_MAX_SIZE: usize = 64 * 1024;

/// The VFS as the calling thread sees it (its cwd, root and permissions;
/// N-251), or a syscall error instead of a panic if the VFS subsystem has
/// not been initialized yet. Every path a program supplies is resolved
/// through this.
pub(crate) fn vfs() -> Result<crate::fs::CallerView<'static>, SyscallError> {
    try_get_vfs()
        .map(|v| v.as_caller())
        .ok_or(SyscallError::InvalidState)
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
    let path_owned = read_user_path(path)?;
    let path_str = path_owned.as_str();

    // Trace ALL open calls during kwin bringup
    #[cfg(all(target_arch = "x86_64", feature = "trace"))]
    {
        // SAFETY: Writing to COM1 for diagnostic output.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[OPEN] ");
            let print_len = path_str.len().min(80);
            for &b in &path_str.as_bytes()[..print_len] {
                crate::arch::x86_64::idt::raw_serial_str(&[b]);
            }
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    open_path(path_str, flags, mode)
}

/// Open `path` with Linux `open(2)` semantics (shared by open and openat).
///
/// - O_CREAT|O_EXCL fails with EEXIST if the name exists, and a lost create
///   race is EEXIST or a plain open, never ENOENT;
/// - O_TRUNC truncates (openat used to skip it, and musl routes every open()
///   through openat) and reports a failure;
/// - O_NOFOLLOW refuses a final symlink (ELOOP), O_DIRECTORY anything but a
///   directory (ENOTDIR), and a directory cannot be opened for writing (EISDIR)
///   (N-116).
fn open_path(path: &str, flags: usize, mode: usize) -> SyscallResult {
    const O_DIRECTORY: usize = 0x1_0000;
    const O_NOFOLLOW: usize = 0x2_0000;
    const O_CLOEXEC: usize = 0x8_0000;
    use crate::fs::NodeType;

    let process = process::current_process().ok_or(SyscallError::InvalidState)?;
    let open_flags = OpenFlags::from_bits(flags as u32).ok_or(SyscallError::InvalidArgument)?;
    let cloexec = flags & O_CLOEXEC != 0;
    let follow = flags & O_NOFOLLOW == 0;

    let (node, created, canonical) = match vfs()?.open_follow_canonical(path, open_flags, follow) {
        Ok((node, canonical)) => {
            if open_flags.create && open_flags.exclusive {
                return Err(SyscallError::FileExists);
            }
            (node, false, Some(canonical))
        }
        // Only a missing name may be created; any other failure (EACCES
        // from a directory without search permission, ENOTDIR, ELOOP) is
        // reported as itself.
        Err(e) if open_flags.create && is_not_found(&e) => {
            let perms = creation_perms(mode);
            let (parent_path, name) = split_path(path)?;
            require_dir_write(path)?;
            let parent = vfs()?.resolve_path(&parent_path).map_err(map_resolve_err)?;
            match parent.create(&name, perms) {
                Ok(node) => {
                    own_new_node(&node);
                    (node, true, None)
                }
                // Another opener created it between the lookup and here.
                Err(crate::error::KernelError::FsError(crate::error::FsError::AlreadyExists))
                    if !open_flags.exclusive =>
                {
                    let (node, canonical) = vfs()?
                        .open_follow_canonical(path, open_flags, follow)
                        .map_err(map_resolve_err)?;
                    (node, false, Some(canonical))
                }
                Err(crate::error::KernelError::FsError(crate::error::FsError::AlreadyExists)) => {
                    return Err(SyscallError::FileExists);
                }
                Err(e) => return Err(map_resolve_err(e)),
            }
        }
        Err(e) => return Err(map_resolve_err(e)),
    };

    let node_type = node.node_type();
    if !follow && node_type == NodeType::Symlink {
        return Err(SyscallError::SymlinkLoop);
    }
    if flags & O_DIRECTORY != 0 && node_type != NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    if open_flags.write && node_type == NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    if !created {
        require_open_access(&node, &open_flags)?;
    }
    if open_flags.truncate && open_flags.write && !created && node_type == NodeType::File {
        node.truncate(0).map_err(map_resolve_err)?;
    }

    // The file records the canonical path of what it opened (in the whole
    // tree) for fchdir and dirfd resolution: the path as written could be
    // relative, and would be re-resolved against a later cwd (N-204).
    let canonical = match canonical {
        Some(c) => c,
        None => vfs()?
            .resolve_canonical(path, &vfs()?.get_cwd(), false)
            .map(|(_, c)| c)
            .unwrap_or_else(|_| alloc::string::String::from(path)),
    };
    let file = crate::fs::file::File::new_with_path(node, open_flags, canonical);
    let file_table = process.file_table.lock();
    file_table
        .open_with_flags(alloc::sync::Arc::new(file), cloexec)
        .map_err(|_| SyscallError::OutOfMemory)
}

/// Close a file descriptor
///
/// # Arguments
/// - fd: File descriptor to close
pub fn sys_close(fd: usize) -> SyscallResult {
    // Get current process
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // A closed PRIME export can no longer be imported (W-11).
    {
        let file_table = process.file_table.lock();
        if file_table.get(fd).is_some_and(|f| f.is_drm_device()) {
            crate::graphics::drm_ioctl::prime_fd_closed(process.pid.0, fd as i32);
        }
    }

    // Remove from file table
    let file_table = process.file_table.lock();
    match file_table.close(fd) {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
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

    // For stdin (fd 0): an open file (a pipe, a tty) is read like any other
    // fd below -- including waiting for a pipe writer, which returning here
    // skipped (`head` on a pipe got an error instead of the data). Only a
    // process without one falls back to the serial console.
    let stdin_open =
        fd == 0 && process::current_process().is_some_and(|p| p.file_table.lock().get(0).is_some());
    if fd == 0 && !stdin_open {
        if !console_fallback_allowed(fd) {
            return Err(SyscallError::BadFileDescriptor);
        }
        // Fallback: read from serial UART, respecting terminal state.
        let read_count = count.min(SERIAL_IO_MAX_SIZE);
        // Bytes are gathered in a kernel buffer and copied out once (N-43).
        let mut line = alloc::vec![0u8; read_count];
        let buffer_slice = &mut line[..];

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

        super::userspace::write_user_bytes(buffer, &line[..bytes_read])?;
        return Ok(bytes_read);
    }

    // Non-stdin: use file table normally.
    //
    // In boot execution mode (no preemptive scheduler), a pipe read may
    // return WouldBlock because the child process that writes to the pipe
    // hasn't been dispatched yet. When this happens, we dispatch any Ready
    // children inline (the same pattern used by waitpid), then retry the
    // read. This enables shell command substitution `$(cmd)` where the
    // parent reads from a pipe that the child writes to.
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // First attempt: try reading directly. The file table lock covers only
    // the lookup: a read can sleep (an empty eventfd), and a lock held over
    // it stopped every other thread's file system calls (N-118).
    {
        let file_desc = proc
            .file_table
            .lock()
            .get(fd)
            .ok_or(SyscallError::BadFileDescriptor)?;
        match file_read_to_user(&file_desc, buffer, count) {
            Err(SyscallError::WouldBlock) => {
                // Pipe empty with write end open -- fall through to try
                // dispatching children in boot context.
            }
            other => return other,
        }
    } // Drop file_table lock before dispatching children.

    // A dispatched reader (stage D2) runs alongside its writer: a blocking
    // read of an empty pipe or socket waits until data arrives or the
    // writers close (EOF), as POSIX requires. It used to return EAGAIN,
    // which the nested model hid by running the writer inline and which
    // made `$(sort f | head -n 1)` read nothing once both ran at once
    // (N-119). O_NONBLOCK still gets EAGAIN.
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        let mut retry_now = false;
        loop {
            // The file table lock is held only for the lookup: never across
            // the user copy or the wait (N-118).
            let file_desc = proc
                .file_table
                .lock()
                .get(fd)
                .ok_or(SyscallError::BadFileDescriptor)?;
            if file_desc
                .nonblock
                .load(core::sync::atomic::Ordering::Acquire)
            {
                return Err(SyscallError::WouldBlock);
            }
            match file_read_to_user(&file_desc, buffer, count) {
                Err(SyscallError::WouldBlock) => {}
                other => return other,
            }
            // Sleep until readable (data, EOF or error); a signal ends the
            // wait (EINTR).
            wait_ready(
                &file_desc,
                (POLLIN | POLLHUP | POLLERR) as u16,
                &mut retry_now,
            )?;
        }
    }

    // Boot execution mode: dispatch any Ready children so they can write
    // to the pipe, then retry the read. This loop runs at most
    // MAX_CHILD_DISPATCH iterations to prevent infinite loops.
    #[cfg(target_arch = "x86_64")]
    {
        use crate::process::{pcb::ProcessState, table, thread::ThreadId};

        if crate::arch::x86_64::usermode::has_boot_return_context() {
            let current_pid = proc.pid;
            const MAX_CHILD_DISPATCH: usize = 16;

            for _ in 0..MAX_CHILD_DISPATCH {
                // Find a Ready child to dispatch.
                let children = table::PROCESS_TABLE.find_children(current_pid);
                let mut dispatched = false;

                for child_pid in &children {
                    if let Some(child) = table::get_process(*child_pid) {
                        if child.get_state() == ProcessState::Ready {
                            let parent_tid = proc
                                .threads
                                .lock()
                                .values()
                                .next()
                                .map(|t| t.tid)
                                .unwrap_or(ThreadId(current_pid.0));

                            dispatched = crate::bootstrap::boot_run_forked_child(
                                *child_pid,
                                current_pid,
                                parent_tid,
                            );
                            break;
                        }
                    }
                }

                // Retry the read after dispatching the child.
                let file_table = proc.file_table.lock();
                if let Some(file_desc) = file_table.get(fd) {
                    match file_read_to_user(&file_desc, buffer, count) {
                        Err(SyscallError::WouldBlock) => {
                            // Still empty -- continue dispatching if we ran a child
                            drop(file_table);
                            if !dispatched {
                                // No more children to dispatch, return EAGAIN
                                return Err(SyscallError::WouldBlock);
                            }
                        }
                        other => return other,
                    }
                } else {
                    return Err(SyscallError::InvalidArgument);
                }
            }
        }
    }

    // Non-boot context or exhausted dispatch attempts: return EAGAIN so
    // the C library can retry via poll()/select().
    Err(SyscallError::WouldBlock)
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
            if let Some(result) = write_fd(&proc, fd, buffer, count) {
                return result;
            }
        }

        if !console_fallback_allowed(fd) {
            return Err(SyscallError::BadFileDescriptor);
        }
        // Fallback: write directly to serial UART
        let write_count = count.min(SERIAL_IO_MAX_SIZE);
        // Copied in once through the fault-tolerant reader (N-43).
        let mut out = alloc::vec![0u8; write_count];
        super::userspace::read_user_bytes(buffer, &mut out)?;
        for &byte in &out {
            serial_write_byte(byte);
        }

        return Ok(write_count);
    }

    // Non-stdout/stderr: use file table normally
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    write_fd(&proc, fd, buffer, count).unwrap_or(Err(SyscallError::InvalidArgument))
}

/// Write `count` bytes from `buffer` to `fd` of `proc` (`None`: no such
/// fd). A dispatched writer (stage D2) to a full pipe or socket waits for
/// room, as POSIX requires of a blocking descriptor, instead of getting
/// EAGAIN; O_NONBLOCK still gets it (N-119). No lock is held while waiting,
/// and a signal ends the wait (EINTR).
fn write_fd(
    proc: &crate::process::Process,
    fd: usize,
    buffer: usize,
    count: usize,
) -> Option<SyscallResult> {
    #[cfg(feature = "alloc")]
    let mut retry_now = false;
    // A blocking write to a stream (pipe, socket, terminal) waits until all
    // of it is written, as POSIX requires; a short count used to come back
    // as soon as a pipe filled. A signal or a closed reader after part was
    // written returns the count so far.
    let mut done = 0usize;
    loop {
        // Looked up under the file table lock, written without it (N-118).
        let file_desc = proc.file_table.lock().get(fd)?;
        let blocking = !file_desc
            .nonblock
            .load(core::sync::atomic::Ordering::Acquire);
        let stream = !matches!(
            file_desc.node.node_type(),
            crate::fs::NodeType::File | crate::fs::NodeType::Directory
        );
        let result = file_write_from_user(&file_desc, buffer + done, count - done);
        #[cfg(feature = "alloc")]
        let dispatched = crate::sched::dispatch::current_owner().is_some();
        #[cfg(not(feature = "alloc"))]
        let dispatched = false;
        match result {
            Ok(n) if blocking && stream && dispatched && done + n < count => done += n,
            Ok(n) => return Some(Ok(done + n)),
            Err(SyscallError::WouldBlock) if blocking && dispatched => {}
            Err(_) if done > 0 => return Some(Ok(done)),
            Err(e) => return Some(Err(e)),
        }
        #[cfg(feature = "alloc")]
        if let Err(e) = wait_ready(&file_desc, (POLLOUT | POLLERR) as u16, &mut retry_now) {
            return Some(if done > 0 { Ok(done) } else { Err(e) });
        }
    }
}

/// Sleep until `file` reports one of `mask` (poll bits), a signal must be
/// acted on (EINTR), or -- for a file type that does not wake waiters
/// itself -- a short re-check interval passes. `retry_now` carries state
/// between calls: when the last wait ended because the file looked ready
/// but the operation still blocked, this one sleeps for the interval
/// instead, so a node whose readiness and I/O disagree cannot spin.
#[cfg(feature = "alloc")]
fn wait_ready(
    file: &crate::fs::file::File,
    mask: u16,
    retry_now: &mut bool,
) -> Result<(), SyscallError> {
    use crate::sched::dispatch::{self, WaitError};
    const RECHECK_NS: u64 = 10_000_000;
    let rearm = core::mem::take(retry_now);
    let deadline = (rearm || !file.node.wakes_io_waiters())
        .then(|| dispatch::clock_ns().saturating_add(RECHECK_NS));
    match dispatch::wait_event(&dispatch::IO_EVENTS, deadline, || {
        !rearm && file.node.poll_readiness() & mask != 0
    }) {
        Ok(()) => {
            *retry_now = true;
            Ok(())
        }
        Err(WaitError::TimedOut) => Ok(()),
        Err(WaitError::Interrupted) => Err(SyscallError::Interrupted),
    }
}

/// Read up to `count` bytes from an open file into user memory at `buf`.
/// A regular file is read in chunks until `count` or end of file; anything
/// else (pipe, tty, socket, device) gets exactly one underlying read, so its
/// blocking and short-read behaviour is unchanged. EOF on a pipe is 0.
fn file_read_to_user(file: &crate::fs::file::File, buf: usize, count: usize) -> SyscallResult {
    let regular = file.node.node_type() == crate::fs::NodeType::File;
    super::userspace::produce_to_user(buf, count, regular, |kbuf| match file.read(kbuf) {
        Ok(n) => Ok(n),
        Err(crate::error::KernelError::BrokenPipe) => Ok(0),
        Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
        Err(e) => Err(super::map_kernel_error(e)),
    })
}

/// Write up to `count` bytes of user memory at `buf` to an open file, in
/// chunks; a short write (a full pipe) ends the transfer with the count so
/// far.
fn file_write_from_user(file: &crate::fs::file::File, buf: usize, count: usize) -> SyscallResult {
    super::userspace::consume_from_user(buf, count, true, |kbuf| match file.write(kbuf) {
        Ok(n) => Ok(n),
        Err(crate::error::KernelError::BrokenPipe) => Err(SyscallError::BrokenPipe),
        Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
        Err(e) => Err(super::map_kernel_error(e)),
    })
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
    // Lookup only; the lock is not held over the operation (N-118).
    let file_desc = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Convert whence to SeekFrom
    let seek_from = match whence {
        // A negative absolute offset is EINVAL, not a huge position (N-122).
        0 if offset < 0 => return Err(SyscallError::InvalidArgument),
        0 => SeekFrom::Start(offset as usize),
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return Err(SyscallError::InvalidArgument),
    };

    // Perform seek
    match file_desc.seek(seek_from) {
        Ok(new_pos) => Ok(new_pos as usize),
        Err(e) => Err(super::map_kernel_error(e)),
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
    // Lookup only; the lock is not held over the operation (N-118).
    let file_desc = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Get metadata and write to user buffer
    let metadata = file_desc.node.metadata().map_err(super::map_kernel_error)?;
    let stat = fill_stat(&metadata);

    super::userspace::write_user(stat_buf, stat)?;
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
    // Lookup only; the lock is not held over the operation (N-118).
    let file_desc = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Truncate file
    match file_desc.node.truncate(size) {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
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

    // Copied in through the fault-tolerant reader (N-43).
    let path_owned = read_user_path(path)?;
    let path_str = path_owned.as_str();

    // Create directory through VFS
    let permissions = creation_perms(mode);
    let vfs_guard = vfs()?;
    match vfs_guard.mkdir(path_str, permissions) {
        Ok(node) => {
            own_new_node(&node);
            Ok(0)
        }
        Err(e) => Err(super::map_kernel_error(e)),
    }
}

/// Remove a directory
///
/// # Arguments
/// - path: Path to directory to remove
pub fn sys_rmdir(path: usize) -> SyscallResult {
    let path_str = read_user_path(path)?;
    require_may_remove(&path_str)?;

    // Remove directory through VFS
    match vfs()?.unlink(&path_str) {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
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

    // Copied in through the fault-tolerant reader (N-43).
    let mount_path_owned = read_user_path(mount_point)?;
    let mount_path = mount_path_owned.as_str();

    // Copied in through the fault-tolerant reader (N-43).
    let fs_type_owned = crate::syscall::userspace::read_user_cstr(fs_type, 255)?;
    let fs_type_str = fs_type_owned.as_str();

    // Mount filesystem
    match vfs()?.mount_by_type(mount_path, fs_type_str, flags as u32) {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
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

    // Copied in through the fault-tolerant reader (N-43).
    let mount_path_owned = read_user_path(mount_point)?;
    let mount_path = mount_path_owned.as_str();

    // Unmount filesystem
    match vfs()?.unmount(mount_path) {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
    }
}

/// Sync filesystem
///
/// Flushes all pending writes to disk
pub fn sys_sync() -> SyscallResult {
    match vfs()?.sync() {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
    }
}

/// Sync a single file descriptor to disk (fsync)
///
/// Validates the fd exists, then triggers a full filesystem sync.
/// Since BlockFS syncs all dirty blocks at once, per-fd sync is
/// equivalent to a full sync.
pub fn sys_fsync(fd: usize) -> SyscallResult {
    // Validate the fd exists
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    if file_table.get(fd).is_none() {
        return Err(SyscallError::BadFileDescriptor);
    }
    drop(file_table);

    // Sync all filesystems (BlockFS syncs dirty blocks to disk)
    match vfs()?.sync() {
        Ok(_) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
    }
}

/// File stat structure for userspace.
///
/// Layout matches the C `struct stat` in `veridian/stat.h` exactly.
/// All fields use fixed-size types matching the C typedefs in
/// `veridian/types.h`.
/// Linux x86_64 `struct stat` layout (144 bytes).
///
/// Field order and padding MUST match the kernel ABI that musl expects.
/// Key difference from naive layout: nlink comes before mode, and there
/// is a 4-byte pad after gid, plus nanosecond fields and trailing padding.
#[repr(C)]
#[derive(Clone, Copy)]
struct FileStat {
    st_dev: u64,        // offset 0
    st_ino: u64,        // offset 8
    st_nlink: u64,      // offset 16 (note: before st_mode in Linux ABI)
    st_mode: u32,       // offset 24
    st_uid: u32,        // offset 28
    st_gid: u32,        // offset 32
    __pad0: u32,        // offset 36
    st_rdev: u64,       // offset 40
    st_size: i64,       // offset 48
    st_blksize: i64,    // offset 56
    st_blocks: i64,     // offset 64
    st_atime: i64,      // offset 72
    st_atime_nsec: i64, // offset 80
    st_mtime: i64,      // offset 88
    st_mtime_nsec: i64, // offset 96
    st_ctime: i64,      // offset 104
    st_ctime_nsec: i64, // offset 112
    __unused: [i64; 3], // offset 120 (padding to 144 bytes)
}

// Compile-time assertion: Linux x86_64 struct stat is exactly 144 bytes.
const _: () = assert!(core::mem::size_of::<FileStat>() == 144);
// Every field sits where the next one ends (Linux x86_64 offsets), so there
// is no implicit padding and a copy to user memory exposes no
// uninitialized kernel bytes.
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(FileStat, st_mode) == 24);
    assert!(offset_of!(FileStat, __pad0) == 36);
    assert!(offset_of!(FileStat, st_rdev) == 40);
    assert!(offset_of!(FileStat, st_atime) == 72);
    assert!(offset_of!(FileStat, __unused) == 120);
};

/// Helper: populate a FileStat from VFS metadata.
fn fill_stat(metadata: &crate::fs::Metadata) -> FileStat {
    // File type bits plus the node's real permission bits; this used to
    // report a fixed 0644/0755 for every file and directory.
    let type_bits = match metadata.node_type {
        crate::fs::NodeType::File => 0o100000,
        crate::fs::NodeType::Directory => 0o040000,
        crate::fs::NodeType::CharDevice => 0o020000,
        crate::fs::NodeType::BlockDevice => 0o060000,
        crate::fs::NodeType::Symlink => 0o120000,
        crate::fs::NodeType::Pipe => 0o010000,
        crate::fs::NodeType::Socket => 0o140000,
    };
    let mode = type_bits | metadata.permissions.to_mode();
    let size = metadata.size as i64;
    FileStat {
        st_dev: 1,
        st_ino: metadata.inode,
        st_nlink: 1,
        st_mode: mode,
        st_uid: metadata.uid,
        st_gid: metadata.gid,
        __pad0: 0,
        st_rdev: 0,
        st_size: size,
        st_blksize: 4096,
        st_blocks: (size + 511) / 512,
        st_atime: metadata.accessed as i64,
        st_atime_nsec: 0,
        st_mtime: metadata.modified as i64,
        st_mtime_nsec: 0,
        st_ctime: metadata.created as i64,
        st_ctime_nsec: 0,
        __unused: [0; 3],
    }
}

/// Map a `KernelError` from VFS path resolution to the most appropriate
/// `SyscallError`, preserving important distinctions like ELOOP and
/// ENOENT.
/// Whether a lookup failed because the final name does not exist, the only
/// failure O_CREAT may turn into a create.
fn is_not_found(e: &crate::error::KernelError) -> bool {
    matches!(
        e,
        crate::error::KernelError::FsError(crate::error::FsError::NotFound)
    )
}

pub(crate) fn map_resolve_err(e: crate::error::KernelError) -> SyscallError {
    match e {
        crate::error::KernelError::FsError(crate::error::FsError::SymlinkLoop) => {
            SyscallError::SymlinkLoop
        }
        crate::error::KernelError::FsError(crate::error::FsError::NotFound) => {
            SyscallError::ResourceNotFound
        }
        crate::error::KernelError::FsError(crate::error::FsError::NotADirectory) => {
            SyscallError::NotADirectory
        }
        crate::error::KernelError::FsError(crate::error::FsError::IsADirectory) => {
            SyscallError::IsADirectory
        }
        crate::error::KernelError::FsError(crate::error::FsError::AlreadyExists) => {
            SyscallError::FileExists
        }
        crate::error::KernelError::FsError(crate::error::FsError::NoSpace) => {
            SyscallError::OutOfMemory
        }
        crate::error::KernelError::FsError(crate::error::FsError::PermissionDenied) => {
            SyscallError::PermissionDenied
        }
        _ => SyscallError::ResourceNotFound,
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
        Err(e) => Err(super::map_kernel_error(e)),
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
        Err(e) => Err(super::map_kernel_error(e)),
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

    // The path and its NUL go out through the fault-tolerant writer (N-43).
    let mut out = Vec::with_capacity(cwd_bytes.len() + 1);
    out.extend_from_slice(cwd_bytes);
    out.push(0);
    super::userspace::write_user_bytes(buf, &out)?;

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
        let (node, canonical) = vfs()?
            .resolve_canonical(&path, &cwd, true)
            .map_err(map_resolve_err)?;
        enter_directory(&thread, &node, &canonical)
    }
}

/// Make `node` (at canonical path `canonical` in the whole tree) the
/// thread's working directory: it must be a directory the caller may
/// search. The working directory is kept as the canonical path relative to
/// the root, so a symlink or ".." in the argument is resolved once and
/// getcwd reports the real directory.
#[cfg(feature = "alloc")]
fn enter_directory(
    thread: &process::Thread,
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    canonical: &str,
) -> SyscallResult {
    if node.node_type() != crate::fs::NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    let creds = caller_creds();
    if creds.euid != 0 {
        let meta = node.metadata().map_err(super::map_kernel_error)?;
        if !meta
            .permissions
            .can_run(creds.euid, creds.gid_for(meta.gid), meta.uid, meta.gid)
        {
            return Err(SyscallError::PermissionDenied);
        }
    }
    let fs = thread.fs();
    let root = fs.root.lock().clone();
    *fs.cwd.lock() = crate::fs::root_relative(canonical, &root);
    Ok(0)
}

/// utimensat's special `tv_nsec` values.
const UTIME_NOW: i64 = (1 << 30) - 1;
const UTIME_OMIT: i64 = (1 << 30) - 2;

/// One utimensat time ([tv_sec, tv_nsec]) as the seconds to store, or
/// `None` to leave it (UTIME_OMIT). Times before 1970 are stored as 0, the
/// earliest the filesystems represent.
fn utime_spec(spec: [i64; 2], now: u64) -> Result<Option<u64>, SyscallError> {
    match spec[1] {
        UTIME_NOW => Ok(Some(now)),
        UTIME_OMIT => Ok(None),
        0..=999_999_999 => Ok(Some(spec[0].max(0) as u64)),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// utimensat(dirfd, path, times, flags) (N-249); with a NULL path it acts
/// on `dirfd` itself (futimens). NULL `times` sets both to now. Setting
/// explicit times needs the owner (or root, EPERM otherwise); setting the
/// current time also allows a caller with write permission (EACCES).
pub fn sys_utimensat(
    dirfd: usize,
    path_ptr: usize,
    times_ptr: usize,
    flags: usize,
) -> SyscallResult {
    if flags & !AT_SYMLINK_NOFOLLOW_FLAG != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let node = if path_ptr == 0 {
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let table = proc.file_table.lock();
        table
            .get(dirfd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?
            .node
            .clone()
    } else {
        let path = resolve_at_path(dirfd, &read_user_path(path_ptr)?)?;
        let vfs = vfs()?;
        if flags & AT_SYMLINK_NOFOLLOW_FLAG != 0 {
            vfs.resolve_path_no_follow(&path)
        } else {
            vfs.resolve_path(&path)
        }
        .map_err(map_resolve_err)?
    };
    let specs = if times_ptr == 0 {
        None
    } else {
        Some(super::userspace::read_user::<[[i64; 2]; 2]>(times_ptr)?)
    };
    let now = crate::arch::timer::get_timestamp_secs();
    let (atime, mtime) = match specs {
        None => (Some(now), Some(now)),
        Some([a, m]) => (utime_spec(a, now)?, utime_spec(m, now)?),
    };
    let only_now = specs.is_none_or(|[a, m]| a[1] == UTIME_NOW && m[1] == UTIME_NOW);
    let creds = caller_creds();
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if creds.euid != 0 && meta.uid != creds.euid {
        if !only_now {
            return Err(SyscallError::OperationNotPermitted);
        }
        if !meta
            .permissions
            .can_write(creds.euid, creds.gid_for(meta.gid), meta.uid, meta.gid)
        {
            return Err(SyscallError::PermissionDenied);
        }
    }
    if atime.is_none() && mtime.is_none() {
        return Ok(0);
    }
    node.set_times(atime, mtime)
        .map_err(super::map_kernel_error)?;
    Ok(0)
}

/// The fd's recorded path, as seen from the caller's root, provided it
/// still names the object the fd refers to. Files are reached by path
/// strings (N-115), so a directory renamed after it was opened, with
/// another put at its old name, would otherwise let `fchdir` or an `*at`
/// call check one directory and act on the other; that case is ENOENT.
fn fd_directory_path(file: &crate::fs::file::File) -> Result<alloc::string::String, SyscallError> {
    if file.node.node_type() != crate::fs::NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    let global = file.path.as_deref().ok_or(SyscallError::NotADirectory)?;
    let vfs = vfs()?;
    let root = vfs.get_root();
    let path = crate::fs::root_relative(global, &root);
    let (now, canonical) = vfs
        .resolve_canonical(&path, "/", true)
        .map_err(|_| SyscallError::ResourceNotFound)?;
    let same = canonical == global
        && now.node_type() == crate::fs::NodeType::Directory
        && match (now.metadata(), file.node.metadata()) {
            (Ok(a), Ok(b)) => a.inode == b.inode,
            _ => false,
        };
    if same {
        Ok(path)
    } else {
        Err(SyscallError::ResourceNotFound)
    }
}

/// fchdir: change to the directory an open fd refers to (N-250).
pub fn sys_fchdir(fd: usize) -> SyscallResult {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc
        .file_table
        .lock()
        .get(fd as u32 as usize)
        .ok_or(SyscallError::BadFileDescriptor)?;
    let (node, path) = (file.node.clone(), fd_directory_path(&file)?);
    // `path` is relative to the root; enter_directory wants the whole-tree
    // path it maps back.
    let root = vfs()?.get_root();
    let path = if root == "/" {
        path
    } else if path == "/" {
        root
    } else {
        alloc::format!("{}{}", root, path)
    };
    #[cfg(not(feature = "alloc"))]
    {
        let _ = (thread, node, path);
        return Err(SyscallError::InvalidState);
    }
    #[cfg(feature = "alloc")]
    enter_directory(&thread, &node, &path)
}

/// chroot (N-250): make `path` the root for every later lookup of this
/// process's threads that share its filesystem state (and of children,
/// which inherit it). Root only (EPERM), as without CAP_SYS_CHROOT on
/// Linux. The working directory stays where it is if it is under the new
/// root; otherwise it becomes the new root, where Linux would leave it
/// outside (the classic way out of a chroot).
pub fn sys_chroot(path_ptr: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;
    if caller_creds().euid != 0 {
        return Err(SyscallError::OperationNotPermitted);
    }
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let fs = thread.fs();
    let cwd = fs.cwd.lock().clone();
    let (node, new_root) = vfs()?
        .resolve_canonical(&path, &cwd, true)
        .map_err(map_resolve_err)?;
    if node.node_type() != crate::fs::NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    let old_root = fs.root.lock().clone();
    // The working directory in the whole tree, then seen from the new root.
    let cwd_global = if old_root == "/" {
        cwd
    } else if cwd == "/" {
        old_root
    } else {
        alloc::format!("{}{}", old_root, cwd)
    };
    let new_cwd = if new_root == "/" {
        cwd_global
    } else if cwd_global == new_root {
        alloc::string::String::from("/")
    } else {
        match cwd_global.strip_prefix(new_root.as_str()) {
            Some(rest) if rest.starts_with('/') => alloc::string::String::from(rest),
            _ => alloc::string::String::from("/"),
        }
    };
    *fs.root.lock() = new_root;
    *fs.cwd.lock() = new_cwd;
    Ok(0)
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
/// Major number of evdev input devices (`/dev/input/event*`).
const EVDEV_MAJOR: u32 = 13;

pub fn sys_ioctl(fd: usize, cmd: usize, arg: usize) -> SyscallResult {
    use crate::drivers::terminal::{
        self, KernelTermios, KernelWinsize, TCGETS, TCSETS, TCSETSF, TCSETSW, TIOCGPGRP,
        TIOCGWINSZ, TIOCSPGRP, TIOCSWINSZ,
    };

    // Give PTY fds priority for TIOCGWINSZ / TIOCSWINSZ before the generic
    // terminal-only guard below rejects them.  For fd > 2, check whether the
    // fd refers to a PTY master or slave node.  If so, the PTY handler
    // returns Some(result) and we propagate it directly.  If the fd is not a
    // PTY node, it returns None and we fall through to the standard path.
    if fd > 2 {
        if let Some(result) = crate::syscall::pty::handle_pty_ioctl(fd, cmd, arg) {
            return result;
        }
    }

    // Device ioctls (DRM, evdev). The device is identified by the node's
    // (major, minor), never by the path it was opened with (W-7). Both run
    // against a kernel copy of the argument (W-4, W-19): the handlers never
    // see the user pointer.
    if fd > 2 {
        if let Some(proc) = process::current_process() {
            let file_table = proc.file_table.lock();
            let device = file_table
                .get(fd as crate::fs::file::FileDescriptor)
                .and_then(|file| file.node.device_id());
            drop(file_table);
            match device {
                Some((crate::fs::devfs::DRM_MAJOR, _)) => {
                    let result =
                        crate::syscall::userspace::ioctl_bounce(cmd as u64, arg, |kernel_arg| {
                            crate::graphics::drm_ioctl::drm_ioctl_dispatch(
                                fd as i32, cmd as u64, kernel_arg,
                            )
                        })?;
                    return match result {
                        Ok(v) => Ok(v as usize),
                        Err(crate::error::KernelError::PermissionDenied { .. }) => {
                            Err(SyscallError::PermissionDenied)
                        }
                        Err(_) => Err(SyscallError::InvalidArgument),
                    };
                }
                Some((EVDEV_MAJOR, minor)) => {
                    let result =
                        crate::syscall::userspace::ioctl_bounce(cmd as u64, arg, |kernel_arg| {
                            crate::drivers::evdev::handle_ioctl(minor, cmd as u32, kernel_arg)
                        })?;
                    return match result {
                        Ok(v) => Ok(v as usize),
                        Err(_) => Err(SyscallError::InvalidArgument),
                    };
                }
                _ => {}
            }
        }
    }

    // Terminal ioctls are only valid on terminal fds (0=stdin, 1=stdout,
    // 2=stderr which are connected to the serial console). Regular files
    // opened via open() must return ENOTTY so that isatty() returns false
    // and BFD/stdio treat them as seekable files, not terminal streams.
    let is_terminal_cmd = matches!(
        cmd,
        TIOCGWINSZ | TIOCSWINSZ | TCGETS | TCSETS | TCSETSW | TCSETSF | TIOCGPGRP | TIOCSPGRP
    );
    if is_terminal_cmd && fd > 2 {
        return Err(SyscallError::NotATerminal);
    }
    // A closed standard descriptor is not a terminal; it is not open at all.
    if is_terminal_cmd {
        let has_entry =
            process::current_process().is_some_and(|p| p.file_table.lock().get(fd).is_some());
        if !has_entry && !console_fallback_allowed(fd) {
            return Err(SyscallError::BadFileDescriptor);
        }
    }

    match cmd {
        TIOCGWINSZ => {
            // Return terminal window size from real terminal state
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelWinsize>(arg)?;

            let ws = terminal::get_winsize_snapshot();
            super::userspace::write_user(arg, ws)?;
            Ok(0)
        }
        TIOCSWINSZ => {
            // Set window size in terminal state
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelWinsize>(arg)?;

            let ws: KernelWinsize = super::userspace::read_user(arg)?;
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
            super::userspace::write_user(arg, termios)?;
            Ok(0)
        }
        TCSETS => {
            // Set terminal attributes immediately
            if arg == 0 {
                return Err(SyscallError::InvalidPointer);
            }
            validate_user_ptr_typed::<KernelTermios>(arg)?;

            let new_termios: KernelTermios = super::userspace::read_user(arg)?;
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

            let new_termios: KernelTermios = super::userspace::read_user(arg)?;
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

            let new_termios: KernelTermios = super::userspace::read_user(arg)?;
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
            super::userspace::write_user(arg, pgid)?;
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
    // kill(2) used to update a separate bookkeeping table (the process
    // server's) and never reach a process, with no permission check and
    // process groups scanned only up to pid 1024 (N-92).
    let sig = i32::try_from(signal)
        .ok()
        .filter(|s| (0..=31).contains(s))
        .ok_or(SyscallError::InvalidArgument)?;
    let caller = crate::process::current_process().ok_or(SyscallError::InvalidState)?;

    match pid as isize {
        p if p > 0 => kill_one(&caller, crate::process::ProcessId(p as u64), sig),
        0 => kill_matching(&caller, sig, |p| {
            p.pgid.load(core::sync::atomic::Ordering::Relaxed)
                == caller.pgid.load(core::sync::atomic::Ordering::Relaxed)
        }),
        // Every process the caller may signal except init and itself.
        -1 => kill_matching(&caller, sig, |p| p.pid.0 != 1 && p.pid != caller.pid),
        p => {
            let pgid = p.unsigned_abs() as u64;
            kill_matching(&caller, sig, |p| {
                p.pgid.load(core::sync::atomic::Ordering::Relaxed) == pgid
            })
        }
    }
}

/// Whether `caller` may signal `target` (Linux kill(2)): root, or the
/// sender's real or effective uid is the target's real or saved uid.
fn may_signal(caller: &crate::process::Process, target: &crate::process::Process) -> bool {
    let (c, t) = (caller.credentials(), target.credentials());
    c.euid == 0
        || [c.ruid, c.euid]
            .iter()
            .any(|&id| id == t.ruid || id == t.suid)
}

/// Signal one process; `sig` 0 only checks that it exists and may be
/// signalled.
pub(crate) fn kill_one(
    caller: &crate::process::Process,
    pid: crate::process::ProcessId,
    sig: i32,
) -> SyscallResult {
    let target = crate::process::table::get_process(pid).ok_or(SyscallError::ProcessNotFound)?;
    if !target.is_alive() {
        return Err(SyscallError::ProcessNotFound);
    }
    if !may_signal(caller, &target) {
        return Err(SyscallError::OperationNotPermitted);
    }
    if sig == 0 {
        return Ok(0);
    }
    // A signal the caller sends itself whose action ends the process must
    // leave through the exit path: tearing the process down here would
    // return into the address space just freed.
    // (A dispatched process queues it instead and acts on it at the
    // system-call exit, honouring its mask; sprint D3.)
    if target.pid == caller.pid
        && terminates(caller, sig)
        && !target
            .dispatched
            .load(core::sync::atomic::Ordering::Acquire)
    {
        return super::process::exit_current(0, sig as u32);
    }
    crate::process::exit::kill_process(pid, sig)
        .map(|_| 0)
        .map_err(|_| SyscallError::ProcessNotFound)
}

/// Whether delivering `sig` to `process` would end it (default action
/// terminate or core dump, or SIGKILL).
fn terminates(process: &crate::process::Process, sig: i32) -> bool {
    use crate::process::exit::{default_signal_action, signals, SignalAction};
    if sig == signals::SIGKILL {
        return true;
    }
    let handler = process.get_signal_handler(sig as usize).unwrap_or(0);
    handler == 0
        && matches!(
            default_signal_action(sig),
            SignalAction::Terminate | SignalAction::CoreDump | SignalAction::Default
        )
}

/// Signal every process selected by `pred` that the caller may signal.
/// ESRCH if none matched, EPERM if all matches were refused.
fn kill_matching(
    caller: &crate::process::Process,
    sig: i32,
    pred: impl Fn(&crate::process::Process) -> bool,
) -> SyscallResult {
    let mut targets = alloc::vec::Vec::new();
    crate::process::table::PROCESS_TABLE.for_each(|p| {
        if p.is_alive() && pred(p) {
            targets.push(p.pid);
        }
    });
    if targets.is_empty() {
        return Err(SyscallError::ProcessNotFound);
    }
    // The caller last, so it is still alive to signal the others.
    targets.sort_by_key(|&p| p == caller.pid);
    let mut sent = false;
    for pid in targets {
        if kill_one(caller, pid, sig).is_ok() {
            sent = true;
        }
    }
    if sent {
        Ok(0)
    } else {
        Err(SyscallError::OperationNotPermitted)
    }
}

// ============================================================================
// Extended filesystem syscalls (Phase 4B)
// ============================================================================

/// Helper: read a NUL-terminated path from user space into an alloc::String.
#[cfg(feature = "alloc")]
pub(crate) fn read_user_path(ptr: usize) -> Result<alloc::string::String, SyscallError> {
    validate_user_string_ptr(ptr)?;

    // Copy through the fault-tolerant accessor in chunks that never cross a
    // page boundary, so a path ending just before an unmapped page is read
    // without touching that page, and an unmapped pointer is EFAULT rather
    // than a kernel fault.
    const PATH_MAX: usize = 4096;
    const PAGE: usize = 4096;
    let mut bytes = Vec::new();
    let mut addr = ptr;
    'copy: while bytes.len() < PATH_MAX {
        let in_page = PAGE - (addr % PAGE);
        let mut chunk = [0u8; 64];
        let n = in_page.min(chunk.len()).min(PATH_MAX - bytes.len());
        crate::syscall::userspace::read_user_bytes(addr, &mut chunk[..n])?;
        for &b in &chunk[..n] {
            if b == 0 {
                break 'copy;
            }
            bytes.push(b);
        }
        addr += n;
    }

    core::str::from_utf8(&bytes)
        .map(alloc::string::String::from)
        .map_err(|_| SyscallError::InvalidArgument)
}

/// Credentials of the calling process. Kernel context, with no process,
/// acts as root. Checks use the effective IDs, and a file's group grants
/// access to any process in that group (`Credentials::gid_for`).
fn caller_creds() -> crate::process::creds::Credentials {
    process::current_process().map_or(crate::process::creds::Credentials::new(0, 0), |p| {
        p.credentials()
    })
}

/// Permissions for a new node: the requested `mode` minus the calling
/// thread's umask (POSIX). The umask never removes the sticky bit.
pub(crate) fn creation_perms(mode: usize) -> Permissions {
    let umask = process::current_thread().map_or(0o022, |t| {
        t.fs().umask.load(core::sync::atomic::Ordering::Acquire)
    });
    Permissions::from_mode(mode as u32 & 0o7777 & !(umask & 0o777))
}

/// A node created by a syscall belongs to the caller. Without this every
/// new file was owned by root, so a user could not chmod, or remove from a
/// sticky directory, what it had just created.
///
/// `node` must be the node the creating call returned. Looking the path up
/// again would chown whatever is at that name by then -- a user could swap
/// in a hard link to a root-owned file and take ownership of it.
pub(crate) fn own_new_node(node: &alloc::sync::Arc<dyn crate::fs::VfsNode>) {
    let creds = caller_creds();
    let (uid, gid) = (creds.euid, creds.egid);
    if uid != 0 || gid != 0 {
        let _ = node.chown(Some(uid), Some(gid));
    }
}

/// chmod-style operations: only the owner or root may change a node's
/// mode (FS-SEC-02).
pub(crate) fn require_owner_or_root(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
) -> Result<(), SyscallError> {
    let uid = caller_creds().euid;
    if uid == 0 {
        return Ok(());
    }
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if meta.uid == uid {
        Ok(())
    } else {
        Err(SyscallError::PermissionDenied)
    }
}

/// Check that the caller may open `node` with `flags`, using the POSIX
/// owner/group/other precedence (the owner class uses only the owner bits).
/// Fails closed when the metadata cannot be read.
pub(crate) fn require_open_access(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    flags: &OpenFlags,
) -> Result<(), SyscallError> {
    let creds = caller_creds();
    let uid = creds.euid;
    if uid == 0 {
        return Ok(());
    }
    let meta = node
        .metadata()
        .map_err(|_| SyscallError::PermissionDenied)?;
    let gid = creds.gid_for(meta.gid);
    let p = meta.permissions;
    if flags.read && !p.can_read(uid, gid, meta.uid, meta.gid) {
        return Err(SyscallError::PermissionDenied);
    }
    if (flags.write || flags.append || flags.truncate) && !p.can_write(uid, gid, meta.uid, meta.gid)
    {
        return Err(SyscallError::PermissionDenied);
    }
    Ok(())
}

/// Rename `old` to `new` (absolute paths) by linking the existing node
/// under the new name and removing the old entry. The node -- its owner,
/// mode and contents -- is moved, never copied, and a symlink at `new` is
/// replaced rather than followed.
fn rename_entry(old: &str, new: &str) -> SyscallResult {
    // Held across the ancestry check and the move, so no concurrent rename
    // can make the check stale.
    let _rename = crate::fs::RENAME_LOCK.lock();
    require_may_remove(old)?;
    require_dir_write(new)?;
    // Replacing an existing `new` removes it, so the sticky rule applies.
    let new_exists = vfs()?.resolve_path_no_follow(new).is_ok();
    if new_exists {
        require_may_remove(new)?;
    }

    let vfs = vfs()?;
    let (old_parent_path, old_name) = split_path(old)?;
    let (new_parent_path, new_name) = split_path(new)?;
    // Compared on canonical parents: the parents are resolved with symlinks
    // followed below, so a string comparison let a symlink into another
    // mount through (review of the v0.26.0 stack, PR #11).
    let old_mount = vfs
        .entry_mount_point(&old_parent_path, &old_name)
        .map_err(map_resolve_err)?;
    let new_mount = vfs
        .entry_mount_point(&new_parent_path, &new_name)
        .map_err(map_resolve_err)?;
    if old_mount != new_mount {
        return Err(SyscallError::CrossDevice);
    }
    let src = vfs.resolve_path_no_follow(old).map_err(map_resolve_err)?;

    if src.node_type() == crate::fs::NodeType::Directory {
        // A directory cannot move into its own subtree (POSIX: EINVAL).
        // Compared on canonical paths (symlinks, ".", ".." resolved): a
        // string prefix test on the raw paths was bypassed by "/a/./b" or
        // a symlink to the directory, which orphaned it as its own child.
        let (_, old_parent_canon) = vfs
            .resolve_canonical(&old_parent_path, "/", true)
            .map_err(map_resolve_err)?;
        let src_canon = if old_parent_canon == "/" {
            alloc::format!("/{}", old_name)
        } else {
            alloc::format!("{}/{}", old_parent_canon, old_name)
        };
        let (_, new_parent_canon) = vfs
            .resolve_canonical(&new_parent_path, "/", true)
            .map_err(map_resolve_err)?;
        if crate::fs::path_is_under(&new_parent_canon, &src_canon) {
            return Err(SyscallError::InvalidArgument);
        }
        // Moving it to another parent rewrites its "..", which needs write
        // permission on the directory itself.
        if old_parent_path != new_parent_path {
            let creds = caller_creds();
            let meta = src.metadata().map_err(super::map_kernel_error)?;
            let (uid, gid) = (creds.euid, creds.gid_for(meta.gid));
            if uid != 0 && !meta.permissions.can_write(uid, gid, meta.uid, meta.gid) {
                return Err(SyscallError::PermissionDenied);
            }
        }
    }

    let old_parent = vfs
        .resolve_path(&old_parent_path)
        .map_err(map_resolve_err)?;
    let new_parent = vfs
        .resolve_path(&new_parent_path)
        .map_err(map_resolve_err)?;
    // The node moves; nothing is copied, and a symlink at `new` is replaced
    // rather than followed (FS-PERF-03).
    old_parent
        .rename(&old_name, &new_parent, &new_name)
        .map_err(super::map_kernel_error)?;
    Ok(0)
}

/// Creating or removing a directory entry needs write and search
/// permission on the directory holding it (FS-SEC-02).
pub(crate) fn require_dir_write(path: &str) -> Result<(), SyscallError> {
    let creds = caller_creds();
    let uid = creds.euid;
    if uid == 0 {
        return Ok(());
    }
    let (parent, _) = split_path(path)?;
    let dir = vfs()?.resolve_path(&parent).map_err(map_resolve_err)?;
    let meta = dir.metadata().map_err(super::map_kernel_error)?;
    let gid = creds.gid_for(meta.gid);
    let p = meta.permissions;
    if p.can_write(uid, gid, meta.uid, meta.gid) && p.can_run(uid, gid, meta.uid, meta.gid) {
        Ok(())
    } else {
        Err(SyscallError::PermissionDenied)
    }
}

/// Check that the caller may remove or rename the entry at `path`: it needs
/// write and search permission on the parent directory and, if that
/// directory is sticky, must own the entry or the directory (or be root).
pub(crate) fn require_may_remove(path: &str) -> Result<(), SyscallError> {
    require_dir_write(path)?;
    let uid = caller_creds().euid;
    if uid == 0 {
        return Ok(());
    }
    let (parent, _) = split_path(path)?;
    let vfs_guard = vfs()?;
    let dir_meta = vfs_guard
        .resolve_path(&parent)
        .map_err(map_resolve_err)?
        .metadata()
        .map_err(super::map_kernel_error)?;
    if !dir_meta.permissions.sticky || dir_meta.uid == uid {
        return Ok(());
    }
    let entry_meta = vfs_guard
        .resolve_path_no_follow(path)
        .map_err(map_resolve_err)?
        .metadata()
        .map_err(super::map_kernel_error)?;
    if entry_meta.uid == uid {
        Ok(())
    } else {
        Err(SyscallError::PermissionDenied)
    }
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

    let vfs = vfs()?;
    let node = vfs.resolve_path(&path).map_err(map_resolve_err)?;

    let metadata = node.metadata().map_err(super::map_kernel_error)?;
    let stat = fill_stat(&metadata);

    super::userspace::write_user(stat_buf, stat)?;
    Ok(0)
}

/// Stat a file by path without following the final symlink (syscall 151).
///
/// Like `stat`, but if the final component of the path is a symbolic
/// link, returns information about the link itself rather than the file
/// it points to. Intermediate symlinks in the path are still followed.
///
/// # Arguments
/// - `path_ptr`: Pointer to NUL-terminated path string.
/// - `stat_buf`: Pointer to `FileStat` output buffer.
///
/// # Returns
/// 0 on success.
pub fn sys_lstat(path_ptr: usize, stat_buf: usize) -> SyscallResult {
    validate_user_ptr_typed::<FileStat>(stat_buf)?;
    let path = read_user_path(path_ptr)?;

    // Trace lstat calls during kwin bringup
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Writing to COM1 for diagnostic output.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[LSTAT] ");
            let show_len = path.len().min(80);
            for &b in &path.as_bytes()[..show_len] {
                crate::arch::x86_64::idt::raw_serial_str(&[b]);
            }
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    let vfs = vfs()?;
    match vfs.resolve_path_no_follow(&path) {
        Ok(node) => {
            let metadata = node.metadata().map_err(super::map_kernel_error)?;
            let stat = fill_stat(&metadata);
            super::userspace::write_user(stat_buf, stat)?;
            Ok(0)
        }
        Err(_) => {
            // Workaround for wayland display socket creation:
            // kwin's musl wrapper has a bug where stat() returning ENOENT
            // is misinterpreted as a non-ENOENT error (likely due to double
            // __syscall_ret application in the musl remap wrapper). This
            // prevents kwin from finding a free wayland display number.
            //
            // For paths matching /run/user/0/wayland-N (without .lock),
            // return a fake stat indicating S_IFSOCK. This tells wayland
            // "stale socket from previous run" which triggers the correct
            // unlink+rebind path instead of the broken ENOENT path.
            if path.starts_with("/run/user/")
                && path.contains("wayland-")
                && !path.ends_with(".lock")
            {
                // S_IFSOCK (0xC000) | 0o755
                let fake_stat = FileStat {
                    st_dev: 0,
                    st_ino: 0xFFFF,
                    st_nlink: 1,
                    st_mode: 0xC1ED, // S_IFSOCK | 0755
                    st_uid: 0,
                    st_gid: 0,
                    __pad0: 0,
                    st_rdev: 0,
                    st_size: 0,
                    st_blksize: 4096,
                    st_blocks: 0,
                    st_atime: 0,
                    st_atime_nsec: 0,
                    st_mtime: 0,
                    st_mtime_nsec: 0,
                    st_ctime: 0,
                    st_ctime_nsec: 0,
                    __unused: [0; 3],
                };
                super::userspace::write_user(stat_buf, fake_stat)?;
                return Ok(0);
            }
            Err(map_resolve_err(crate::error::KernelError::FsError(
                crate::error::FsError::NotFound,
            )))
        }
    }
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

    let vfs = vfs()?;

    // readlink must NOT follow the final symlink -- we want the link
    // node itself so we can read its target.
    let node = vfs.resolve_path_no_follow(&path).map_err(map_resolve_err)?;

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
    access_path(&path, mode, 0)
}

/// faccessat (3 arguments, no flags) and faccessat2 (with `flags`):
/// `access` relative to a directory fd.
pub fn sys_faccessat(dirfd: usize, path_ptr: usize, mode: usize, flags: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;
    let path = resolve_at_path(dirfd, &path)?;
    access_path(&path, mode, flags)
}

const AT_EACCESS: usize = 0x200;
const AT_SYMLINK_NOFOLLOW_FLAG: usize = 0x100;

/// Whether access mode `mode` (R_OK 4, W_OK 2, X_OK 1) is granted to a
/// process with IDs (uid, gid as `gid_for` chose it) on a file with `meta`.
/// Root may read and write anything and execute anything with an execute
/// bit set, or any directory, as on Linux.
fn access_granted(
    uid: u32,
    gid: u32,
    meta: &crate::fs::Metadata,
    is_dir: bool,
    mode: usize,
) -> bool {
    let p = &meta.permissions;
    if uid == 0 {
        let any_x = p.owner_exec || p.group_exec || p.other_exec;
        return mode & 1 == 0 || any_x || is_dir;
    }
    (mode & 4 == 0 || p.can_read(uid, gid, meta.uid, meta.gid))
        && (mode & 2 == 0 || p.can_write(uid, gid, meta.uid, meta.gid))
        && (mode & 1 == 0 || p.can_run(uid, gid, meta.uid, meta.gid))
}

/// access(2): checked with the real IDs (the effective ones with
/// AT_EACCESS), owner, then group (any of the process's groups), then
/// other bits. It checked only the "other" bits (N-193).
fn access_path(path: &str, mode: usize, flags: usize) -> SyscallResult {
    if mode & !7 != 0 || flags & !(AT_EACCESS | AT_SYMLINK_NOFOLLOW_FLAG) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let vfs = vfs()?;
    let node = if flags & AT_SYMLINK_NOFOLLOW_FLAG != 0 {
        vfs.resolve_path_no_follow(path)
    } else {
        vfs.resolve_path(path)
    }
    .map_err(map_resolve_err)?;
    if mode == 0 {
        return Ok(0); // F_OK: it exists
    }
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    let creds = caller_creds();
    let (uid, gid) = if flags & AT_EACCESS != 0 {
        (creds.euid, creds.gid_for(meta.gid))
    } else {
        (creds.ruid, creds.real_gid_for(meta.gid))
    };
    let is_dir = node.node_type() == crate::fs::NodeType::Directory;
    if access_granted(uid, gid, &meta, is_dir, mode) {
        Ok(0)
    } else {
        Err(SyscallError::PermissionDenied)
    }
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
    let old_path = resolve_at_path(AT_FDCWD, &read_user_path(old_ptr)?)?;
    let new_path = resolve_at_path(AT_FDCWD, &read_user_path(new_ptr)?)?;
    rename_entry(&old_path, &new_path)
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
    require_may_remove(&path)?;

    let vfs = vfs()?;

    match vfs.unlink(&path) {
        Ok(()) => Ok(0),
        Err(e) => Err(super::map_kernel_error(e)),
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
    const F_DUPFD_CLOEXEC: usize = 1030;
    const FD_CLOEXEC: usize = 1;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();

    match cmd {
        F_DUPFD => {
            // Duplicate fd to lowest available >= arg
            match file_table.dup_at_least(fd, arg, false) {
                Ok(new_fd) => Ok(new_fd),
                Err(e) => Err(super::map_kernel_error(e)),
            }
        }
        F_DUPFD_CLOEXEC => {
            // Duplicate fd to lowest available >= arg, with close-on-exec
            match file_table.dup_at_least(fd, arg, true) {
                Ok(new_fd) => Ok(new_fd),
                Err(e) => Err(super::map_kernel_error(e)),
            }
        }
        F_GETFD => {
            // Get close-on-exec flag via FileTable API
            match file_table.get_cloexec(fd) {
                Ok(cloexec) => Ok(if cloexec { FD_CLOEXEC } else { 0 }),
                Err(e) => Err(super::map_kernel_error(e)),
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
            // Get file status flags. Linux ABI: O_RDONLY=0, O_WRONLY=1, O_RDWR=2.
            let file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;
            let mut flags: usize = if file.flags.read && file.flags.write {
                2 // O_RDWR
            } else if file.flags.write {
                1 // O_WRONLY
            } else {
                0 // O_RDONLY
            };
            if file.flags.append {
                flags |= 0x0400; // O_APPEND
            }
            if file.nonblock.load(core::sync::atomic::Ordering::Relaxed) {
                flags |= 0x0800; // O_NONBLOCK
            }
            Ok(flags)
        }
        F_SETFL => {
            // Set file status flags. Only O_APPEND and O_NONBLOCK can be
            // changed after open (per POSIX). O_NONBLOCK is stored as an
            // AtomicBool on the File for lock-free toggling.
            let file = file_table.get(fd).ok_or(SyscallError::InvalidArgument)?;
            let nonblock = (arg & 0x0800) != 0; // O_NONBLOCK
            file.nonblock
                .store(nonblock, core::sync::atomic::Ordering::Relaxed);
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
/// - `pipe_fds_ptr`: Pointer to `[i32; 2]` to receive [read_fd, write_fd].
/// - `flags`: O_CLOEXEC (0x2000) | O_NONBLOCK (0x1000).
///
/// # Returns
/// 0 on success.
/// Linux O_CLOEXEC and O_NONBLOCK, the only flags pipe2 and dup3 accept.
const FD_FLAG_CLOEXEC: usize = 0x8_0000;
const FD_FLAG_NONBLOCK: usize = 0x800;

/// pipe2's flags as (cloexec, nonblock); anything else is EINVAL, as on
/// Linux.
fn pipe2_flags(flags: usize) -> Result<(bool, bool), SyscallError> {
    if flags & !(FD_FLAG_CLOEXEC | FD_FLAG_NONBLOCK) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    Ok((flags & FD_FLAG_CLOEXEC != 0, flags & FD_FLAG_NONBLOCK != 0))
}

pub fn sys_pipe2(pipe_fds_ptr: usize, flags: usize) -> SyscallResult {
    let (cloexec, nonblock) = pipe2_flags(flags)?;
    validate_user_buffer(pipe_fds_ptr, 2 * core::mem::size_of::<i32>())?;

    // Create the pipe
    let (reader, writer) = crate::fs::pipe::create_pipe().map_err(|_| SyscallError::OutOfMemory)?;

    // Wrap pipe ends as VfsNode objects
    let read_node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
        alloc::sync::Arc::new(crate::fs::pipe::PipeReadNode::new(reader));
    let write_node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
        alloc::sync::Arc::new(crate::fs::pipe::PipeWriteNode::new(writer));

    // Create File objects
    let read_file = crate::fs::file::File::new(
        read_node,
        OpenFlags {
            nonblock,
            ..OpenFlags::read_only()
        },
    );
    let write_file = crate::fs::file::File::new(
        write_node,
        OpenFlags {
            nonblock,
            ..OpenFlags::write_only()
        },
    );

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
            file_table.close_on_rollback(read_fd, "pipe");
            SyscallError::OutOfMemory
        })?;

    // Write [read_fd, write_fd] to user buffer as i32 (C int), through the
    // fault-tolerant writer (N-43). An unwritable buffer leaves the caller
    // without the fds, so they are closed again.
    if let Err(e) =
        crate::syscall::userspace::write_user(pipe_fds_ptr, [read_fd as i32, write_fd as i32])
    {
        file_table.close_on_rollback(write_fd, "pipe");
        file_table.close_on_rollback(read_fd, "pipe");
        return Err(e);
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
    if flags & !FD_FLAG_CLOEXEC != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let cloexec = flags & FD_FLAG_CLOEXEC != 0;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();

    file_table
        .dup3(old_fd, new_fd, cloexec)
        .map_err(super::map_kernel_error)?;

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

    let vfs = vfs()?;

    // Verify path exists and is a directory
    let node = vfs.resolve_path(&path).map_err(map_resolve_err)?;

    let metadata = node.metadata().map_err(super::map_kernel_error)?;
    if metadata.node_type != crate::fs::NodeType::Directory {
        return Err(SyscallError::InvalidArgument);
    }

    // Open as a file descriptor using read-only flags
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = crate::fs::file::File::new(node, OpenFlags::read_only());
    let file_table = proc.file_table.lock();
    match file_table.open(alloc::sync::Arc::new(file)) {
        Ok(fd) => Ok(fd),
        Err(e) => Err(super::map_kernel_error(e)),
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
    // Lookup only; the lock is not held over the operation (N-118).
    let file_desc = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

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

    // Entry name + NUL terminator + node type byte, written to the user
    // buffer in one fault-tolerant copy (N-43).
    let mut out = Vec::with_capacity(name_bytes.len() + 2);
    out.extend_from_slice(name_bytes);
    out.push(0);
    out.push(match entry.node_type {
        crate::fs::NodeType::File => 0,
        crate::fs::NodeType::Directory => 1,
        crate::fs::NodeType::CharDevice => 2,
        crate::fs::NodeType::BlockDevice => 3,
        crate::fs::NodeType::Symlink => 4,
        crate::fs::NodeType::Pipe => 5,
        _ => 0,
    });
    super::userspace::write_user_bytes(entry_buf, &out)?;

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

// SAFETY: repr(C) with two usize fields and no padding, so every bit
// pattern is a valid value.
unsafe impl super::userspace::UserPod for Iovec {}

/// Maximum number of iovec entries per readv/writev call.
const IOV_MAX: usize = 1024;

/// Read from a file descriptor into multiple buffers (SYS_readv = 19).
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
        let iov: Iovec = super::userspace::read_user_index(iov_ptr, i)?;

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

/// Write to a file descriptor from multiple buffers (SYS_writev = 20).
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
        let iov: Iovec = super::userspace::read_user_index(iov_ptr, i)?;

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
    require_dir_write(&new_path)?;

    let vfs = vfs()?;

    // Resolve the old path to get the target node
    let target = vfs.resolve_path(&old_path).map_err(map_resolve_err)?;

    // Split new_path into parent dir + name
    let (parent_path, link_name) = split_path(&new_path)?;

    let parent = vfs.resolve_path(&parent_path).map_err(map_resolve_err)?;

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
    require_dir_write(&link_path)?;

    let vfs = vfs()?;

    let (parent_path, link_name) = split_path(&link_path)?;

    let parent = vfs.resolve_path(&parent_path).map_err(map_resolve_err)?;

    let node = parent
        .symlink(&link_name, &target)
        .map_err(|_| SyscallError::InvalidArgument)?;
    own_new_node(&node);

    Ok(0)
}

/// Change file permissions by path (syscall 185).
pub fn sys_chmod(path_ptr: usize, mode: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let vfs = vfs()?;
    let node = vfs.resolve_path(&path).map_err(map_resolve_err)?;
    require_owner_or_root(&node)?;

    let perms = Permissions::from_mode(mode as u32);
    node.chmod(perms)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Change file permissions by fd (syscall 186).
pub fn sys_fchmod(fd: usize, mode: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    require_owner_or_root(&file.node)?;

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

    let vfs = vfs()?;
    let node = vfs.resolve_path(&path).map_err(map_resolve_err)?;

    node.truncate(size)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Poll file descriptors for readiness (syscall 189).
///
/// Poll file descriptors for I/O readiness using VfsNode::poll_readiness().
///
/// Checks each fd's actual buffer state (pipe occupancy, write-end closed,
/// etc.) rather than always reporting ready. Supports timeout via busy-wait
/// with scheduler yield (same approach as nanosleep).
///
/// # Arguments
/// - `fds_ptr`: Pointer to array of PollFd structs.
/// - `nfds`: Number of entries.
/// - `timeout_ms`: Timeout in milliseconds. 0 = non-blocking poll, negative (as
///   i32) = infinite wait, positive = wait up to N ms.
pub fn sys_poll(fds_ptr: usize, nfds: usize, timeout_ms: usize) -> SyscallResult {
    // Boot-path cooperative dispatch: when a child thread is dispatched from
    // boot_futex_spin, the syscall handler yields back to the parent after
    // each syscall. If we spin-loop here for the full timeout, the
    // cooperative scheduler is blocked and no other threads can run. Treat
    // the call as non-blocking (single poll pass) so the child yields
    // promptly and the event loop makes progress across multiple dispatches.
    #[cfg(target_arch = "x86_64")]
    let in_boot_coop = crate::arch::x86_64::usermode::BOOT_CLONE_YIELD_PENDING
        .load(core::sync::atomic::Ordering::Acquire);
    #[cfg(not(target_arch = "x86_64"))]
    let in_boot_coop = false;

    if nfds == 0 {
        // timeout_ms > 0 means sleep for that duration (like usleep via poll)
        #[cfg(feature = "alloc")]
        if (timeout_ms as i32) > 0 && crate::sched::dispatch::current_owner().is_some() {
            let deadline = crate::sched::dispatch::clock_ns()
                .saturating_add((timeout_ms as i32 as u64).saturating_mul(1_000_000));
            return super::time::sleep_until(deadline, 0);
        }
        if (timeout_ms as i32) > 0 && !in_boot_coop {
            let start = crate::timer::get_uptime_ms();
            while crate::timer::get_uptime_ms() - start < timeout_ms as u64 {
                // Enable interrupts briefly to let APIC timer advance
                // UPTIME_MS (see epoll::epoll_wait for full rationale).
                if crate::sched::wait_for_interrupt_in_syscall() {
                    return Err(SyscallError::Interrupted);
                }
            }
        }
        return Ok(0);
    }
    if nfds > 256 {
        return Err(SyscallError::InvalidArgument);
    }

    validate_user_buffer(fds_ptr, nfds * core::mem::size_of::<PollFd>())?;
    // The pollfd array is copied in once and written back on return, never
    // accessed in place (N-43).
    let mut pollfds = Vec::with_capacity(nfds);
    for i in 0..nfds {
        pollfds.push(crate::syscall::userspace::read_user_index::<PollFd>(
            fds_ptr, i,
        )?);
    }
    let write_back = |pollfds: &[PollFd]| -> Result<(), SyscallError> {
        for (i, pfd) in pollfds.iter().enumerate() {
            crate::syscall::userspace::write_user(
                fds_ptr + i * core::mem::size_of::<PollFd>(),
                *pfd,
            )?;
        }
        Ok(())
    };

    let timeout_i32 = if in_boot_coop {
        0i32
    } else {
        timeout_ms as i32
    };
    let start = crate::timer::get_uptime_ms();
    // Cap infinite wait to 30 seconds to prevent permanent hangs
    let max_wait_ms: u64 = if timeout_i32 < 0 {
        30_000
    } else {
        timeout_i32 as u64
    };

    // A dispatched poller sleeps between scans (no cap on an infinite
    // wait): woken when a file object reports a change, at the deadline,
    // or by a signal.
    #[cfg(feature = "alloc")]
    let dispatched = crate::sched::dispatch::current_owner().is_some();
    #[cfg(feature = "alloc")]
    let deadline = (timeout_i32 > 0)
        .then(|| crate::sched::dispatch::clock_ns().saturating_add(timeout_i32 as u64 * 1_000_000));

    loop {
        #[cfg(feature = "alloc")]
        let seq = crate::sched::dispatch::io_seq();
        #[cfg(feature = "alloc")]
        let mut precise = true;
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let file_table = proc.file_table.lock();
        let mut ready_count = 0usize;

        for pollfd in pollfds.iter_mut() {
            pollfd.revents = 0;

            if pollfd.fd < 0 {
                continue;
            }

            if let Some(file) = file_table.get(pollfd.fd as usize) {
                #[cfg(feature = "alloc")]
                {
                    precise &= file.node.wakes_io_waiters();
                }
                let readiness = file.node.poll_readiness();
                if pollfd.events & POLLIN != 0 && readiness & 0x0001 != 0 {
                    pollfd.revents |= POLLIN;
                }
                if pollfd.events & POLLOUT != 0 && readiness & 0x0004 != 0 {
                    pollfd.revents |= POLLOUT;
                }
                // POLLERR and POLLHUP always delivered regardless of events mask
                if readiness & 0x0008 != 0 {
                    pollfd.revents |= POLLERR;
                }
                if readiness & 0x0010 != 0 {
                    pollfd.revents |= POLLHUP;
                }
                if pollfd.revents != 0 {
                    ready_count += 1;
                }
            } else {
                // Not an open fd in this process. Sockets are fds now, so
                // there is no global-socket-id fallback (W-14).
                pollfd.revents = POLLNVAL;
                ready_count += 1;
            }
        }
        // Drop file_table lock before yielding
        drop(file_table);

        if ready_count > 0 || timeout_i32 == 0 {
            write_back(&pollfds)?;
            return Ok(ready_count);
        }

        #[cfg(feature = "alloc")]
        if dispatched {
            use crate::sched::dispatch::{wait_io, WaitError};
            match wait_io(seq, deadline, precise) {
                Ok(()) => continue,
                Err(WaitError::TimedOut) => {
                    write_back(&pollfds)?;
                    return Ok(0);
                }
                Err(WaitError::Interrupted) => return Err(SyscallError::Interrupted),
            }
        }

        // Timeout expired?
        if crate::timer::get_uptime_ms() - start >= max_wait_ms {
            write_back(&pollfds)?;
            return Ok(0);
        }

        // Enable interrupts briefly so the APIC timer ISR can fire and
        // advance UPTIME_MS.  Without this, the monotonic clock is frozen
        // (SFMASK clears IF on syscall entry) and time-based fds such as
        // timerfd never become readable.  See epoll::epoll_wait for the
        // detailed rationale.
        if crate::sched::wait_for_interrupt_in_syscall() {
            return Err(SyscallError::Interrupted);
        }
    }
}

/// Poll event flags
const POLLIN: i16 = 0x001;
const POLLOUT: i16 = 0x004;
const POLLERR: i16 = 0x008;
const POLLHUP: i16 = 0x010;
const POLLNVAL: i16 = 0x020;

/// Poll file descriptor structure (matches C struct pollfd).
#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

// SAFETY: i32 + two i16, no padding (size 8); every bit pattern is valid.
unsafe impl crate::syscall::userspace::UserPod for PollFd {}

/// Resolve a path relative to a directory fd.
///
/// If `dirfd == AT_FDCWD`, uses the process CWD. Otherwise resolves the
/// path relative to the directory referred to by dirfd.
pub(crate) fn resolve_at_path(
    dirfd: usize,
    path: &str,
) -> Result<alloc::string::String, SyscallError> {
    use alloc::string::String;

    if path.starts_with('/') {
        // Absolute path — dirfd is irrelevant
        return Ok(String::from(path));
    }

    // dirfd is an int: compare its low 32 bits, sign-extended or not.
    if dirfd as u32 as i32 == AT_FDCWD as u32 as i32 {
        // Relative to CWD
        let cwd = if let Ok(vfs) = vfs() {
            vfs.get_cwd()
        } else {
            String::from("/")
        };
        if cwd.ends_with('/') {
            Ok(alloc::format!("{}{}", cwd, path))
        } else {
            Ok(alloc::format!("{}/{}", cwd, path))
        }
    } else {
        // Relative to the directory the fd refers to: EBADF if it is not
        // open, ENOTDIR if it is not a directory (N-204).
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        // The table lock is not held across the path lookup.
        let file = proc
            .file_table
            .lock()
            .get(dirfd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?;
        let dir_path = fd_directory_path(&file)?;

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

    // Trace openat calls (kwin bring-up aid; `trace` feature only).
    #[cfg(all(target_arch = "x86_64", feature = "trace"))]
    {
        // SAFETY: Writing to COM1 for diagnostic output.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[OA] ");
            let show_len = abs_path.len().min(80);
            for &b in &abs_path.as_bytes()[..show_len] {
                crate::arch::x86_64::idt::raw_serial_str(&[b]);
            }
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    open_path(&abs_path, flags, mode)
}

/// Stat a file relative to a directory fd (syscall 191).
pub fn sys_fstatat(dirfd: usize, path_ptr: usize, stat_buf: usize, _flags: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;

    // Trace fstatat calls during kwin bringup
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Writing to COM1 for diagnostic output.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[STAT] ");
            let show_len = abs_path.len().min(80);
            for &b in &abs_path.as_bytes()[..show_len] {
                crate::arch::x86_64::idt::raw_serial_str(&[b]);
            }
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    validate_user_ptr_typed::<FileStat>(stat_buf)?;

    let vfs = vfs()?;
    let node = vfs.resolve_path(&abs_path).map_err(map_resolve_err)?;

    let metadata = node.metadata().map_err(super::map_kernel_error)?;
    let stat = fill_stat(&metadata);
    super::userspace::write_user(stat_buf, stat)?;
    Ok(0)
}

/// Unlink a file relative to a directory fd (syscall 192).
///
/// If flags contains AT_REMOVEDIR (0x200), acts like rmdir.
pub fn sys_unlinkat(dirfd: usize, path_ptr: usize, _flags: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;
    require_may_remove(&abs_path)?;

    let vfs = vfs()?;
    vfs.unlink(&abs_path).map_err(super::map_kernel_error)?;

    Ok(0)
}

/// Create a directory relative to a directory fd (syscall 193).
pub fn sys_mkdirat(dirfd: usize, path_ptr: usize, mode: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(dirfd, &rel_path)?;
    require_dir_write(&abs_path)?;

    let permissions = creation_perms(mode);
    let vfs_guard = vfs()?;
    let node = vfs_guard
        .mkdir(&abs_path, permissions)
        .map_err(super::map_kernel_error)?;
    own_new_node(&node);

    Ok(0)
}

/// Rename a file relative to directory fds (syscall 194).
pub fn sys_renameat(
    olddirfd: usize,
    old_ptr: usize,
    newdirfd: usize,
    new_ptr: usize,
) -> SyscallResult {
    let old_abs = resolve_at_path(olddirfd, &read_user_path(old_ptr)?)?;
    let new_abs = resolve_at_path(newdirfd, &read_user_path(new_ptr)?)?;
    rename_entry(&old_abs, &new_abs)
}

/// Read from a file descriptor at a given offset without changing position
/// (syscall 195).
pub fn sys_pread(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Read at `offset` through the VfsNode, bypassing the File position,
    // a chunk at a time through a kernel buffer (N-43).
    let mut at = offset;
    super::userspace::produce_to_user(buf, count, true, |kbuf| {
        let n = file.node.read(at, kbuf).map_err(super::map_kernel_error)?;
        at += n;
        Ok(n)
    })
}

/// Write to a file descriptor at a given offset without changing position
/// (syscall 196).
pub fn sys_pwrite(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Write at `offset` through the VfsNode, a chunk at a time (N-43).
    let mut at = offset;
    super::userspace::consume_from_user(buf, count, true, |kbuf| {
        let n = file.node.write(at, kbuf).map_err(super::map_kernel_error)?;
        at += n;
        Ok(n)
    })
}

/// Helper: split a path into (parent_dir, basename).
pub(crate) fn split_path(
    path: &str,
) -> Result<(alloc::string::String, alloc::string::String), SyscallError> {
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
        let cwd = if let Ok(vfs) = vfs() {
            vfs.get_cwd()
        } else {
            String::from("/")
        };
        Ok((cwd, String::from(path)))
    }
}

// =========================================================================
// Ownership and device node syscalls (197-200)
// =========================================================================

/// Map a chown id argument: `-1` (as u32) means "leave unchanged".
fn chown_id(id: usize) -> Option<u32> {
    let id = id as u32;
    (id != u32::MAX).then_some(id)
}

/// Apply a chown to `node`. Only root may change ownership (FS-SEC-02);
/// this used to be a no-op that reported success.
pub(crate) fn chown_node(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    uid: usize,
    gid: usize,
) -> SyscallResult {
    let (uid, gid) = (chown_id(uid), chown_id(gid));
    if uid.is_none() && gid.is_none() {
        return Ok(0);
    }
    if caller_creds().euid != 0 {
        return Err(SyscallError::PermissionDenied);
    }
    node.chown(uid, gid).map_err(super::map_kernel_error)?;
    Ok(0)
}

/// Change ownership of a file by path (syscall 197).
pub fn sys_chown(path_ptr: usize, uid: usize, gid: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;
    let node = vfs()?.resolve_path(&path).map_err(map_resolve_err)?;
    chown_node(&node, uid, gid)
}

/// Change ownership of a file by file descriptor (syscall 198).
pub fn sys_fchown(fd: usize, uid: usize, gid: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::InvalidArgument)?;
    chown_node(&file.node, uid, gid)
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
/// Scans fd_set bitmaps for set bits and checks whether the corresponding
/// file descriptors exist in the current process's file table. Files and
/// pipes are always considered ready (same simplification as `sys_poll`).
pub fn sys_select(
    nfds: usize,
    readfds_ptr: usize,
    writefds_ptr: usize,
    _exceptfds_ptr: usize,
    _timeout_ptr: usize,
) -> SyscallResult {
    // fd_set is a bitmap: 1 bit per fd, packed into usize-width words.
    // FD_SETSIZE is typically 1024, but we cap at nfds.
    let nfds = nfds.min(1024);
    if nfds == 0 {
        return Ok(0);
    }

    // Number of bytes needed for the bitmap
    let bytes_needed = nfds.div_ceil(8);
    let mut ready_count: usize = 0;

    // Helper: scan an fd_set bitmap and count ready fds.
    // All existing fds are considered ready (files/pipes always ready).
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = process.file_table.lock();

    for fdset_ptr in [readfds_ptr, writefds_ptr] {
        if fdset_ptr == 0 {
            continue;
        }
        // Copied in, updated, and copied back out (N-43).
        let mut set = alloc::vec![0u8; bytes_needed];
        super::userspace::read_user_bytes(fdset_ptr, &mut set)?;
        for fd in 0..nfds {
            let (byte_idx, bit) = (fd / 8, 1u8 << (fd % 8));
            if set[byte_idx] & bit != 0 {
                if file_table.get(fd).is_some() {
                    ready_count += 1;
                } else {
                    // Clear the bit for fds that don't exist
                    set[byte_idx] &= !bit;
                }
            }
        }
        super::userspace::write_user_bytes(fdset_ptr, &set)?;
    }

    Ok(ready_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// pipe2 takes Linux O_CLOEXEC (0x80000) and O_NONBLOCK (0x800); it
    /// tested 0x2000, which no C library sends, and accepted anything.
    #[test]
    fn pipe2_decodes_linux_flags() {
        assert_eq!(pipe2_flags(0), Ok((false, false)));
        assert_eq!(pipe2_flags(0x8_0000), Ok((true, false)));
        assert_eq!(pipe2_flags(0x800), Ok((false, true)));
        assert_eq!(pipe2_flags(0x8_0800), Ok((true, true)));
        assert_eq!(pipe2_flags(0x2000), Err(SyscallError::InvalidArgument));
        assert_eq!(pipe2_flags(1), Err(SyscallError::InvalidArgument));
    }

    fn meta(mode: u32, uid: u32, gid: u32) -> crate::fs::Metadata {
        crate::fs::Metadata {
            node_type: crate::fs::NodeType::File,
            size: 0,
            permissions: Permissions::from_mode(mode),
            uid,
            gid,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        }
    }

    /// access(2) uses owner, then group, then other bits (it checked only
    /// "other", so a 0700 file failed for its owner), and root needs an
    /// execute bit to execute (N-193).
    #[test]
    fn access_uses_owner_group_other_precedence() {
        let m = meta(0o750, 1000, 100);
        assert!(access_granted(1000, 100, &m, false, 7));
        assert!(access_granted(2000, 100, &m, false, 4 | 1));
        assert!(!access_granted(2000, 100, &m, false, 2));
        assert!(!access_granted(2000, 200, &m, false, 4));
        assert!(access_granted(0, 0, &m, false, 7));
        let noexec = meta(0o644, 1000, 100);
        assert!(!access_granted(0, 0, &noexec, false, 1));
        assert!(access_granted(0, 0, &noexec, true, 1));
    }

    /// utimensat times: UTIME_NOW, UTIME_OMIT, a valid time, and EINVAL
    /// for an out-of-range tv_nsec.
    #[test]
    fn utime_spec_follows_utimensat() {
        assert_eq!(utime_spec([5, UTIME_NOW], 100), Ok(Some(100)));
        assert_eq!(utime_spec([5, UTIME_OMIT], 100), Ok(None));
        assert_eq!(utime_spec([1234, 500], 100), Ok(Some(1234)));
        assert_eq!(utime_spec([-10, 0], 100), Ok(Some(0)));
        assert_eq!(
            utime_spec([1, 1_000_000_000], 100),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(utime_spec([1, -1], 100), Err(SyscallError::InvalidArgument));
    }
}

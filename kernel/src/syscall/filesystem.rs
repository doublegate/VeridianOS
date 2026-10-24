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

use super::multiplex::{POLLERR, POLLHUP, POLLIN, POLLOUT};
#[allow(unused_imports)]
use super::{
    validate_user_buffer, validate_user_ptr_typed, validate_user_string_ptr, SyscallError,
    SyscallResult,
};
use crate::{
    fs::{try_get_vfs, OpenFlags, Permissions, SeekFrom},
    process::{self, creds::Caller},
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
        const UART_DR: usize = crate::arch::aarch64::platform::PL011;
        // SAFETY: The PL011 UART data register is memory-mapped I/O on the
        // QEMU virt machine, reached through the physical-memory map. Writing a byte
        // transmits a character. volatile_write ensures the compiler does not
        // elide the store.
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

/// Read the console for a process without a stdin descriptor, sleeping
/// (not spinning, N-118) until the line discipline has something to return;
/// a signal ends the wait with EINTR.
fn read_console_waiting(out: &mut [u8]) -> Result<usize, SyscallError> {
    use crate::drivers::console_input;
    loop {
        match console_input::read_tty(out) {
            Ok(n) => return Ok(n),
            Err(crate::error::KernelError::WouldBlock) => {}
            Err(e) => return Err(super::map_kernel_error(e)),
        }
        #[cfg(feature = "alloc")]
        {
            use crate::sched::dispatch::{self, WaitError};
            // Without a tick hook (not x86_64) nothing wakes the queue, so
            // re-check every 10 ms; a raw-mode VTIME read ends at its
            // deadline.
            let recheck = (!cfg!(target_arch = "x86_64"))
                .then(|| dispatch::clock_ns().saturating_add(10_000_000));
            let deadline = [recheck, console_input::tty_ready_at()]
                .into_iter()
                .flatten()
                .min();
            // Until the read would return: poll readiness was "a byte is
            // waiting", true while a raw read waited for VMIN bytes, so the
            // wait returned at once and the loop spun (PR #24 review).
            let want = out.len();
            match dispatch::wait_event(&console_input::CONSOLE_INPUT, deadline, || {
                console_input::read_would_return(want)
            }) {
                Ok(()) | Err(WaitError::TimedOut) => {}
                Err(WaitError::Interrupted) => return Err(SyscallError::Interrupted),
            }
        }
        #[cfg(not(feature = "alloc"))]
        core::hint::spin_loop();
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
    // arm64 has its own values (its O_LARGEFILE, 0x2_0000, which musl adds
    // to every open, is x86_64's O_NOFOLLOW); riscv64 has x86_64's.
    #[cfg(target_arch = "aarch64")]
    const O_DIRECTORY: usize = 0x4000;
    #[cfg(target_arch = "aarch64")]
    const O_NOFOLLOW: usize = 0x8000;
    #[cfg(not(target_arch = "aarch64"))]
    const O_DIRECTORY: usize = 0x1_0000;
    #[cfg(not(target_arch = "aarch64"))]
    const O_NOFOLLOW: usize = 0x2_0000;
    const O_CLOEXEC: usize = 0x8_0000;
    use crate::fs::NodeType;

    let process = process::current_process().ok_or(SyscallError::InvalidState)?;
    let open_flags = OpenFlags::from_bits(flags as u32).ok_or(SyscallError::InvalidArgument)?;
    let cloexec = flags & O_CLOEXEC != 0;
    let follow = flags & O_NOFOLLOW == 0;
    // O_CREAT with O_DIRECTORY is EINVAL whether the name exists or not,
    // as on Linux since 6.4: it made a regular file, then failed with
    // ENOTDIR and left the file behind (PR #23 review).
    if open_flags.create && flags & O_DIRECTORY != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let (node, created, canonical) = match vfs()?.open_follow_canonical(path, open_flags, follow) {
        Ok((node, canonical)) => {
            if open_flags.create && open_flags.exclusive {
                return Err(SyscallError::FileExists);
            }
            (node, false, Some(canonical))
        }
        // Only a missing name may be created; any other failure (EACCES
        // from a directory without search permission, ENOTDIR, ELOOP) is
        // reported as itself. A name ending in '/' can only be a directory,
        // which O_CREAT does not make (EISDIR, as Linux's open_last_lookups).
        Err(e) if open_flags.create && is_not_found(&e) => {
            if path.ends_with('/') {
                return Err(SyscallError::IsADirectory);
            }
            let perms = creation_perms(mode);
            // One resolution of the parent for the MAC check, the permission
            // check and the creation, as mkdir and mknod do.
            let made = vfs()?.mknod_checked(path, NodeType::File, perms, 0, |dir| {
                check_dir_write(dir).map_err(|_| crate::error::KernelError::PermissionDenied {
                    operation: "create directory entry",
                })
            });
            match made {
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
                // map_kernel_error: a MAC or permission denial is EACCES.
                Err(e) => return Err(super::map_kernel_error(e)),
            }
        }
        Err(e) => return Err(map_resolve_err(e)),
    };

    let node_type = node.node_type();
    // O_CREAT names a file: an existing directory is EISDIR even read-only.
    if open_flags.create && !created && node_type == NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
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
        crate::fs::remove_privs_after_write(&*node);
    }
    // A special file made by mknod opens what it names; a socket file
    // cannot be opened (ENXIO, as Linux's `sock_no_open`).
    let node = if node.is_special_inode() {
        match node_type {
            NodeType::Pipe => open_fifo(node, &open_flags)?,
            NodeType::CharDevice | NodeType::BlockDevice => {
                crate::fs::devfs::open_device(node).map_err(super::map_kernel_error)?
            }
            _ => return Err(SyscallError::NoDeviceOrAddress),
        }
    } else {
        node
    };

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
        .map_err(super::map_kernel_error)
}

/// Open FIFO `inode` (`fs::fifo`): a blocking open of one end sleeps until
/// the other end is opened, or a signal interrupts it (EINTR, restarted
/// under SA_RESTART as Linux's ERESTARTSYS).
fn open_fifo(
    inode: alloc::sync::Arc<dyn crate::fs::VfsNode>,
    flags: &OpenFlags,
) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, SyscallError> {
    let end = crate::fs::fifo::FifoEnd::open(inode, flags.read, flags.write, flags.nonblock)
        .map_err(super::map_kernel_error)?;
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        use crate::sched::dispatch::{self, WaitError};
        while end.needs_partner() {
            match dispatch::wait_event(&dispatch::IO_EVENTS, None, || !end.needs_partner()) {
                Ok(()) | Err(WaitError::TimedOut) => {}
                // Dropping the end undoes the open.
                Err(WaitError::Interrupted) => return Err(SyscallError::Interrupted),
            }
        }
    }
    Ok(alloc::sync::Arc::new(end))
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
    let result = read_fd(fd, buffer, count);
    // However the read ended (data, EINTR, EAGAIN), a console read's VTIME
    // timer ends with it.
    crate::drivers::console_input::end_read();
    result
}

fn read_fd(fd: usize, buffer: usize, count: usize) -> SyscallResult {
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
        // Fallback: the console, through its shared line discipline.
        let read_count = count.min(SERIAL_IO_MAX_SIZE);
        // Bytes are gathered in a kernel buffer and copied out once (N-43).
        let mut line = alloc::vec![0u8; read_count];
        let n = read_console_waiting(&mut line)?;
        super::userspace::write_user_bytes(buffer, &line[..n])?;
        return Ok(n);
    }

    // Non-stdin: use file table normally.
    //
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
            wait_ready(&file_desc, POLLIN | POLLHUP | POLLERR, &mut retry_now)?;
        }
    }

    // Not a dispatched thread (no user thread is, without the dispatcher):
    // EAGAIN.
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
    write_fd(&proc, fd, buffer, count).unwrap_or(Err(SyscallError::BadFileDescriptor))
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
            Err(e) if done > 0 => {
                sigpipe_after_partial_write(&file_desc, e);
                return Some(Ok(done));
            }
            Err(e) => return Some(Err(e)),
        }
        #[cfg(feature = "alloc")]
        // POLLHUP too: a stream socket whose peer closed reports only
        // that, and a writer waiting for POLLOUT or POLLERR slept forever
        // (PR #23 review); the retried write then gets EPIPE.
        if let Err(e) = wait_ready(&file_desc, POLLOUT | POLLERR | POLLHUP, &mut retry_now) {
            return Some(if done > 0 { Ok(done) } else { Err(e) });
        }
    }
}

/// A write to a pipe that loses its last reader after part of the data went
/// in still raises SIGPIPE, as Linux's pipe_write does; the caller gets the
/// count written. (A socket raises it only when nothing was sent, and an
/// EPIPE with nothing written is signalled by the system call dispatcher.)
fn sigpipe_after_partial_write(file: &crate::fs::file::File, error: SyscallError) {
    if error == SyscallError::BrokenPipe && file.node.node_type() == crate::fs::NodeType::Pipe {
        crate::process::signals::send_to_current(crate::process::exit::signals::SIGPIPE as usize);
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
    let recheck = (rearm || !file.node.wakes_io_waiters())
        .then(|| dispatch::clock_ns().saturating_add(RECHECK_NS));
    // A node that becomes ready by itself (a timerfd's expiry) is woken for
    // then.
    let deadline = [recheck, file.node.ready_at_ns()]
        .into_iter()
        .flatten()
        .min();
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
    // Not open for reading is EBADF (it was EACCES), a directory EISDIR
    // (N-197).
    if !file.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    let regular = match file.node.node_type() {
        crate::fs::NodeType::File => true,
        crate::fs::NodeType::Directory => return Err(SyscallError::IsADirectory),
        _ => false,
    };
    // A regular file is read as one step (see `File::io_lock`).
    let _io = regular.then(|| file.io_lock.lock());
    super::userspace::produce_to_user(
        buf,
        count,
        regular,
        |kbuf| match file.read(kbuf) {
            Ok(n) => Ok(n),
            Err(crate::error::KernelError::BrokenPipe) => Ok(0),
            Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
            Err(e) => Err(super::map_kernel_error(e)),
        },
        // What did not reach the caller is read again next time.
        |unread| {
            if regular {
                let _ = file.seek(crate::fs::file::SeekFrom::Current(-(unread as isize)));
            }
        },
    )
}

/// Write up to `count` bytes of user memory at `buf` to an open file, in
/// chunks; a short write (a full pipe) ends the transfer with the count so
/// far.
fn file_write_from_user(file: &crate::fs::file::File, buf: usize, count: usize) -> SyscallResult {
    // Not open for writing is EBADF, as on Linux (N-197).
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let count = file.fsize_room(count).map_err(super::map_kernel_error)?;
    // A regular file is written as one step (see `File::io_lock`).
    let _io = (file.node.node_type() == crate::fs::NodeType::File).then(|| file.io_lock.lock());
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
    // A pipe, socket or terminal has no position (N-190).
    if file_desc.node.is_stream() {
        return Err(SyscallError::IllegalSeek);
    }

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
    let stat = fill_stat(&metadata, &*file_desc.node);

    super::userspace::write_user(stat_buf, stat)?;
    Ok(0)
}

/// An `loff_t` argument (an offset or a length): negative is EINVAL.
fn file_offset(value: usize) -> Result<usize, SyscallError> {
    if (value as i64) < 0 {
        Err(SyscallError::InvalidArgument)
    } else {
        Ok(value)
    }
}

/// ftruncate: the fd must be a regular file open for writing, else EINVAL;
/// a negative length is EINVAL before the fd is looked at (N-191).
pub fn sys_ftruncate(fd: usize, length: usize) -> SyscallResult {
    let length = file_offset(length)?;
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    if file.node.node_type() != crate::fs::NodeType::File || !file.flags.write {
        return Err(SyscallError::InvalidArgument);
    }
    crate::fs::check_fsize_truncate(&*file.node, length).map_err(super::map_kernel_error)?;
    file.node
        .truncate(length)
        .map_err(super::map_kernel_error)?;
    crate::fs::remove_privs_after_write(&*file.node);
    Ok(0)
}

/// Create a directory
///
/// # Arguments
/// - path: Path to new directory
/// - mode: Directory permissions
pub fn sys_mkdir(path: usize, mode: usize) -> SyscallResult {
    // Copied in through the fault-tolerant reader (N-43).
    make_directory(&read_user_path(path)?, mode)
}

/// mkdir and mkdirat, in Linux's order: an existing name (a symlink is not
/// followed) is EEXIST, which `mkdir -p` relies on, before write and search
/// permission on the parent is required (N-192: mkdir checked none).
fn make_directory(path: &str, mode: usize) -> SyscallResult {
    match vfs()?.resolve_path_no_follow(path) {
        Ok(_) => return Err(SyscallError::FileExists),
        Err(e) if is_not_found(&e) => {}
        Err(e) => return Err(map_resolve_err(e)),
    }
    let node = make_directory_node(path, creation_perms(mode))?;
    own_new_node(&node);
    Ok(0)
}

/// Create directory `path`: the VFS resolves its parent once and checks
/// the MAC policy and this permission callback on that same node before
/// creating the entry there (N-51).
fn make_directory_node(
    path: &str,
    perms: Permissions,
) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, SyscallError> {
    vfs()?
        .mkdir_checked(path, perms, |dir| {
            check_dir_write(dir).map_err(|_| crate::error::KernelError::PermissionDenied {
                operation: "create directory entry",
            })
        })
        .map_err(super::map_kernel_error)
}

/// Remove a directory
///
/// # Arguments
/// - path: Path to directory to remove
pub fn sys_rmdir(path: usize) -> SyscallResult {
    sys_unlinkat(AT_FDCWD, path, AT_REMOVEDIR)
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
/// `fill_stat` builds this on every architecture; arm64 and RISC-V copy it
/// out as [`GenericStat`].
#[repr(C)]
#[derive(Clone, Copy)]
struct X86Stat {
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
const _: () = assert!(core::mem::size_of::<X86Stat>() == 144);
// Every field sits where the next one ends (Linux x86_64 offsets), so there
// is no implicit padding and a copy to user memory exposes no
// uninitialized kernel bytes.
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(X86Stat, st_mode) == 24);
    assert!(offset_of!(X86Stat, __pad0) == 36);
    assert!(offset_of!(X86Stat, st_rdev) == 40);
    assert!(offset_of!(X86Stat, st_atime) == 72);
    assert!(offset_of!(X86Stat, __unused) == 120);
};

/// Linux's asm-generic `struct stat` (128 bytes), which arm64 and RISC-V
/// use: mode before a 32-bit nlink, a pad after rdev, a 32-bit blksize.
/// Writing the 144-byte x86_64 layout there ran 16 bytes past the caller's
/// buffer (BusyBox ash lost its saved frame and returned to address 0).
#[repr(C)]
#[derive(Clone, Copy)]
#[cfg_attr(
    not(any(target_arch = "aarch64", target_arch = "riscv64")),
    allow(dead_code)
)]
struct GenericStat {
    st_dev: u64,        // offset 0
    st_ino: u64,        // offset 8
    st_mode: u32,       // offset 16
    st_nlink: u32,      // offset 20
    st_uid: u32,        // offset 24
    st_gid: u32,        // offset 28
    st_rdev: u64,       // offset 32
    __pad1: u64,        // offset 40
    st_size: i64,       // offset 48
    st_blksize: i32,    // offset 56
    __pad2: i32,        // offset 60
    st_blocks: i64,     // offset 64
    st_atime: i64,      // offset 72
    st_atime_nsec: i64, // offset 80
    st_mtime: i64,      // offset 88
    st_mtime_nsec: i64, // offset 96
    st_ctime: i64,      // offset 104
    st_ctime_nsec: i64, // offset 112
    __unused: [u32; 2], // offset 120
}

const _: () = assert!(core::mem::size_of::<GenericStat>() == 128);
// No implicit padding (no uninitialised kernel bytes reach user memory).
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(GenericStat, st_mode) == 16);
    assert!(offset_of!(GenericStat, st_rdev) == 32);
    assert!(offset_of!(GenericStat, st_size) == 48);
    assert!(offset_of!(GenericStat, st_blocks) == 64);
    assert!(offset_of!(GenericStat, __unused) == 120);
};

impl From<X86Stat> for GenericStat {
    fn from(s: X86Stat) -> Self {
        Self {
            st_dev: s.st_dev,
            st_ino: s.st_ino,
            st_mode: s.st_mode,
            st_nlink: s.st_nlink as u32,
            st_uid: s.st_uid,
            st_gid: s.st_gid,
            st_rdev: s.st_rdev,
            __pad1: 0,
            st_size: s.st_size,
            st_blksize: s.st_blksize as i32,
            __pad2: 0,
            st_blocks: s.st_blocks,
            st_atime: s.st_atime,
            st_atime_nsec: s.st_atime_nsec,
            st_mtime: s.st_mtime,
            st_mtime_nsec: s.st_mtime_nsec,
            st_ctime: s.st_ctime,
            st_ctime_nsec: s.st_ctime_nsec,
            __unused: [0; 2],
        }
    }
}

/// The `struct stat` this architecture's programs read.
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
type FileStat = X86Stat;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
type FileStat = GenericStat;

/// Helper: populate a FileStat from the VFS metadata of `node`, and its
/// device number for a device.
fn fill_stat(metadata: &crate::fs::Metadata, node: &dyn crate::fs::VfsNode) -> FileStat {
    let rdev = node
        .device_id()
        .map_or(0, |(major, minor)| crate::fs::make_dev(major, minor));
    #[allow(clippy::useless_conversion)] // the identity on x86_64
    let stat = FileStat::from(fill_x86_stat(metadata, rdev));
    stat
}

/// [`fill_stat`] in the x86_64 layout; `rdev` in Linux's encoding.
fn fill_x86_stat(metadata: &crate::fs::Metadata, rdev: u32) -> X86Stat {
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
    X86Stat {
        st_dev: 1,
        st_ino: metadata.inode,
        st_nlink: 1,
        st_mode: mode,
        st_uid: metadata.uid,
        st_gid: metadata.gid,
        __pad0: 0,
        st_rdev: rdev as u64,
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

    // As Linux: ERANGE when the path and its NUL do not fit (size 0
    // included), only the bytes written must be user memory, and the
    // result counts the NUL (N-214; it was strlen, and EINVAL).
    let cwd_bytes = cwd.as_bytes();
    if cwd_bytes.len() + 1 > size {
        return Err(SyscallError::RangeError);
    }
    validate_user_buffer(buf, cwd_bytes.len() + 1)?;

    // The path and its NUL go out through the fault-tolerant writer (N-43).
    let mut out = Vec::with_capacity(cwd_bytes.len() + 1);
    out.extend_from_slice(cwd_bytes);
    out.push(0);
    super::userspace::write_user_bytes(buf, &out)?;

    Ok(out.len())
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
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if !crate::fs::permission(&Caller::current(), &meta, crate::fs::MAY_EXEC) {
        return Err(SyscallError::PermissionDenied);
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
    let caller = Caller::current();
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if !caller.owner_or_capable(meta.uid) {
        if !only_now {
            return Err(SyscallError::OperationNotPermitted);
        }
        if !crate::fs::permission(&caller, &meta, crate::fs::MAY_WRITE) {
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
/// which inherit it). CAP_SYS_CHROOT only (EPERM), as on Linux. The
/// working directory stays where it is if it is under the new
/// root; otherwise it becomes the new root, where Linux would leave it
/// outside (the classic way out of a chroot).
pub fn sys_chroot(path_ptr: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;
    if !Caller::current().capable(crate::process::linux_caps::CAP_SYS_CHROOT) {
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
    // An int: its low 32 bits. Signals 1..=64 (0 tests for existence).
    let sig = Some(signal as u32 as i32)
        .filter(|s| (0..=crate::process::signals::NSIG as i32).contains(s))
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

/// Whether the calling thread of `caller` may send `sig` to `target`
/// (Linux's check_kill_permission).
fn may_signal(
    caller: &crate::process::Process,
    target: &crate::process::Process,
    sig: i32,
) -> bool {
    use core::sync::atomic::Ordering;
    let same_session = caller.sid.load(Ordering::Acquire) == target.sid.load(Ordering::Acquire);
    kill_ok(
        &Caller::current(),
        &target.credentials(),
        caller.pid == target.pid,
        sig,
        same_session,
    )
}

/// Linux's kill rule: a process may always signal itself; otherwise the
/// sender's real or effective uid must be the target's real or saved uid,
/// or the sender must hold CAP_KILL -- except that SIGCONT may go to any
/// process in the sender's session.
fn kill_ok(
    caller: &Caller,
    target: &crate::process::creds::Credentials,
    same_process: bool,
    sig: i32,
    same_session: bool,
) -> bool {
    let c = &caller.creds;
    same_process
        || [c.ruid, c.euid]
            .iter()
            .any(|&id| id == target.ruid || id == target.suid)
        || caller.capable(crate::process::linux_caps::CAP_KILL)
        || (sig == crate::process::exit::signals::SIGCONT && same_session)
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
    if !may_signal(caller, &target, sig) {
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
    // The target sees the sender: SI_USER, the caller's pid and real user.
    let info = crate::process::signals::SigInfo::from_process(
        sig as usize,
        crate::process::signals::SI_USER,
        caller.pid.0,
        caller.credentials().ruid,
    );
    send_info(pid, sig, info)
}

/// Make `sig` pending for process `pid` with `info` (the caller checked
/// existence and permission): EAGAIN when a queued real-time signal would
/// exceed RLIMIT_SIGPENDING.
pub(crate) fn send_info(
    pid: crate::process::ProcessId,
    sig: i32,
    info: crate::process::signals::SigInfo,
) -> SyscallResult {
    crate::process::exit::kill_process_info(pid, sig, info)
        .map(|_| 0)
        .map_err(|e| match e {
            crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
            _ => SyscallError::ProcessNotFound,
        })
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
    // An empty path names nothing (ENOENT), as Linux's getname (N-203).
    let path = read_user_path_or_empty(ptr)?;
    if path.is_empty() {
        return Err(SyscallError::ResourceNotFound);
    }
    Ok(path)
}

/// A path argument that may be empty (calls taking AT_EMPTY_PATH). A path
/// with no terminator within PATH_MAX (4096 bytes, the NUL included) is
/// ENAMETOOLONG; it used to be cut off and used (N-203).
pub(crate) fn read_user_path_or_empty(ptr: usize) -> Result<alloc::string::String, SyscallError> {
    validate_user_string_ptr(ptr)?;

    // Copy through the fault-tolerant accessor in chunks that never cross a
    // page boundary, so a path ending just before an unmapped page is read
    // without touching that page, and an unmapped pointer is EFAULT rather
    // than a kernel fault.
    const PATH_MAX: usize = 4096;
    const PAGE: usize = 4096;
    let mut bytes = Vec::new();
    let mut addr = ptr;
    let mut terminated = false;
    'copy: while bytes.len() < PATH_MAX {
        let in_page = PAGE - (addr % PAGE);
        let mut chunk = [0u8; 64];
        let n = in_page.min(chunk.len()).min(PATH_MAX - bytes.len());
        crate::syscall::userspace::read_user_bytes(addr, &mut chunk[..n])?;
        for &b in &chunk[..n] {
            if b == 0 {
                terminated = true;
                break 'copy;
            }
            bytes.push(b);
        }
        addr += n;
    }
    if !terminated {
        return Err(SyscallError::NameTooLong);
    }

    core::str::from_utf8(&bytes)
        .map(alloc::string::String::from)
        .map_err(|_| SyscallError::InvalidArgument)
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
    let creds = Caller::current().creds;
    let (uid, gid) = (creds.fsuid, creds.fsgid);
    if uid != 0 || gid != 0 {
        let _ = node.chown(Some(uid), Some(gid));
    }
}

/// chmod-style operations: only the owner or a caller with CAP_FOWNER may
/// change a node's mode (FS-SEC-02).
pub(crate) fn chmod_node(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    mode: usize,
) -> SyscallResult {
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    let perms = chmod_perms(&Caller::current(), meta.uid, meta.gid, mode)?;
    node.chmod(perms).map_err(super::map_kernel_error)?;
    Ok(0)
}

/// The mode a chmod by `caller` stores on a file owned by `uid`:`gid`.
/// EPERM unless the caller owns the file or holds CAP_FOWNER; and as
/// Linux's setattr_copy, a caller outside the file's group without
/// CAP_FSETID cannot set the set-group-ID bit (it is dropped, not refused).
pub(crate) fn chmod_perms(
    caller: &Caller,
    uid: u32,
    gid: u32,
    mode: usize,
) -> Result<Permissions, SyscallError> {
    if !caller.owner_or_capable(uid) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let mut perms = Permissions::from_mode(mode as u32 & 0o7777);
    if !caller.creds.in_group(gid) && !caller.capable(crate::process::linux_caps::CAP_FSETID) {
        perms.set_gid = false;
    }
    Ok(perms)
}

/// Check that the caller may open `node` with `flags`, using the POSIX
/// owner/group/other precedence (the owner class uses only the owner bits).
/// Fails closed when the metadata cannot be read.
pub(crate) fn require_open_access(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    flags: &OpenFlags,
) -> Result<(), SyscallError> {
    let meta = node
        .metadata()
        .map_err(|_| SyscallError::PermissionDenied)?;
    let mut mask = 0;
    if flags.read {
        mask |= crate::fs::MAY_READ;
    }
    if flags.write || flags.append || flags.truncate {
        mask |= crate::fs::MAY_WRITE;
    }
    if mask != 0 && !crate::fs::permission(&Caller::current(), &meta, mask) {
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
    // Both directories are resolved once; the checks and the move below
    // use these nodes (N-51).
    let (old_parent, old_name, old_parent_canon) = parent_dir_canonical(old)?;
    let (new_parent, new_name, new_parent_canon) = parent_dir_canonical(new)?;
    check_may_remove(&*old_parent, &old_name)?;
    check_dir_write(&*new_parent)?;
    // Replacing an existing `new` removes it, so the sticky rule applies.
    if new_parent.lookup(&new_name).is_ok() {
        check_may_remove(&*new_parent, &new_name)?;
    }

    let vfs = vfs()?;
    // Compared on the canonical parents of the resolution that produced the
    // nodes above: a string comparison let a symlink into another mount
    // through (review of PR #11), and resolving the paths again let them
    // describe other directories than the nodes (N-51).
    let old_mount = vfs.canonical_entry_mount_point(&old_parent_canon, &old_name);
    let new_mount = vfs.canonical_entry_mount_point(&new_parent_canon, &new_name);
    if old_mount != new_mount {
        return Err(SyscallError::CrossDevice);
    }
    let src = old_parent.lookup(&old_name).map_err(map_resolve_err)?;

    if src.node_type() == crate::fs::NodeType::Directory {
        // A directory cannot move into its own subtree (POSIX: EINVAL).
        // Compared on canonical paths (symlinks, ".", ".." resolved): a
        // string prefix test on the raw paths was bypassed by "/a/./b" or
        // a symlink to the directory, which orphaned it as its own child.
        let src_canon = if old_parent_canon == "/" {
            alloc::format!("/{}", old_name)
        } else {
            alloc::format!("{}/{}", old_parent_canon, old_name)
        };
        if crate::fs::path_is_under(&new_parent_canon, &src_canon) {
            return Err(SyscallError::InvalidArgument);
        }
        // Moving it to another parent rewrites its "..", which needs write
        // permission on the directory itself.
        if old_parent_canon != new_parent_canon {
            let meta = src.metadata().map_err(super::map_kernel_error)?;
            if !crate::fs::permission(&Caller::current(), &meta, crate::fs::MAY_WRITE) {
                return Err(SyscallError::PermissionDenied);
            }
        }
    }

    // The node moves; nothing is copied, and a symlink at `new` is replaced
    // rather than followed (FS-PERF-03).
    old_parent
        .rename(&old_name, &new_parent, &new_name)
        .map_err(super::map_kernel_error)?;
    Ok(0)
}

/// The directory holding `path`, resolved once for the caller, and the
/// entry's name. The permission checks below and the operation that
/// follows all use this one node: checking a path and then operating on it
/// by path again let the path change in between -- a symlink swapped in, a
/// directory renamed -- so the check applied to another directory (N-51).
pub(crate) fn parent_dir(
    path: &str,
) -> Result<
    (
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        alloc::string::String,
    ),
    SyscallError,
> {
    let (dir, name, _) = parent_dir_canonical(path)?;
    Ok((dir, name))
}

/// [`parent_dir`] with the directory's canonical path from the same
/// resolution, for checks that need a path (mounts, ancestry): taking it
/// from a second resolution would let it describe another directory.
pub(crate) fn parent_dir_canonical(
    path: &str,
) -> Result<
    (
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        alloc::string::String,
        alloc::string::String,
    ),
    SyscallError,
> {
    let (parent, name) = split_path(path)?;
    // Relative to the caller's working directory, as `resolve_path` does: a
    // relative "a/b" names "a" there, not under "/" (review of 40695cdb).
    let (dir, canonical) = vfs()?
        .resolve_path_canonical(&parent)
        .map_err(map_resolve_err)?;
    Ok((dir, name, canonical))
}

/// Creating or removing an entry in `dir` needs write and search permission
/// on it (FS-SEC-02).
pub(crate) fn check_dir_write(dir: &dyn crate::fs::VfsNode) -> Result<(), SyscallError> {
    let meta = dir.metadata().map_err(super::map_kernel_error)?;
    let mask = crate::fs::MAY_WRITE | crate::fs::MAY_EXEC;
    if crate::fs::permission(&Caller::current(), &meta, mask) {
        Ok(())
    } else {
        Err(SyscallError::PermissionDenied)
    }
}

/// Check that the caller may remove or rename entry `name` of `dir`: write
/// and search permission on `dir` and, if it is sticky, ownership of the
/// entry or the directory, or CAP_FOWNER (Linux's check_sticky).
pub(crate) fn check_may_remove(
    dir: &dyn crate::fs::VfsNode,
    name: &str,
) -> Result<(), SyscallError> {
    check_dir_write(dir)?;
    let caller = Caller::current();
    let uid = caller.creds.fsuid;
    let dir_meta = dir.metadata().map_err(super::map_kernel_error)?;
    if !dir_meta.permissions.sticky || dir_meta.uid == uid {
        return Ok(());
    }
    let entry_meta = dir
        .lookup(name)
        .map_err(map_resolve_err)?
        .metadata()
        .map_err(super::map_kernel_error)?;
    if caller.owner_or_capable(entry_meta.uid) {
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
    sys_fstatat(AT_FDCWD, path_ptr, stat_buf, 0)
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
    // A missing Wayland socket used to stat as a socket here, a workaround
    // for a musl remapping bug that ADR 0009 removed.
    sys_fstatat(AT_FDCWD, path_ptr, stat_buf, AT_SYMLINK_NOFOLLOW)
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

/// Who access(2) checks for (Linux's do_faccessat): the real IDs as the
/// filesystem IDs, and unless SECBIT_NO_SETUID_FIXUP, no capabilities for
/// a real uid other than 0 and the permitted set for real uid 0 -- what
/// the caller would hold as its real self.
fn access_caller() -> Caller {
    let mut caller = Caller::current();
    let c = &mut caller.creds;
    c.fsuid = c.ruid;
    c.fsgid = c.rgid;
    if let Some(thread) = process::current_thread() {
        caller.caps = thread.caps.lock().access_effective(caller.creds.ruid);
    }
    caller
}

/// access(2): checked as [`access_caller`] (as the caller itself with
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
    let caller = if flags & AT_EACCESS != 0 {
        Caller::current()
    } else {
        access_caller()
    };
    if crate::fs::permission(&caller, &meta, mode as u32) {
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
    rename_paths(&read_user_path(old_ptr)?, &read_user_path(new_ptr)?)
}

/// `rename(old, new)` for kernel callers (the shell's `mv`): paths relative
/// to the calling process's working directory, the same checks and atomic
/// replace as the system call.
pub(crate) fn rename_paths(old: &str, new: &str) -> SyscallResult {
    let old_path = resolve_at_path(AT_FDCWD, old)?;
    let new_path = resolve_at_path(AT_FDCWD, new)?;
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
    sys_unlinkat(AT_FDCWD, path_ptr, 0)
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
    const F_ADD_SEALS: usize = 1033;
    const F_GET_SEALS: usize = 1034;
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
        // Seals (N-229): only a memfd has them (EINVAL otherwise); adding
        // needs the fd open for writing (EPERM), as on Linux.
        F_GET_SEALS => {
            let file = file_table.get(fd).ok_or(SyscallError::BadFileDescriptor)?;
            file.node
                .seals()
                .map(|s| s as usize)
                .ok_or(SyscallError::InvalidArgument)
        }
        F_ADD_SEALS => {
            let file = file_table.get(fd).ok_or(SyscallError::BadFileDescriptor)?;
            drop(file_table);
            if file.node.seals().is_none() {
                return Err(SyscallError::InvalidArgument);
            }
            if !file.flags.write {
                return Err(SyscallError::OperationNotPermitted);
            }
            file.node
                .add_seals(arg as u32)
                .map(|_| 0)
                .map_err(super::map_kernel_error)
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
        .map_err(super::map_kernel_error)?;

    let write_fd = file_table
        .open_with_flags(alloc::sync::Arc::new(write_file), cloexec)
        .map_err(|e| {
            // Clean up read fd on failure
            file_table.close_on_rollback(read_fd, "pipe");
            super::map_kernel_error(e)
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
    let iovs = (0..iovcnt)
        .map(|i| super::userspace::read_user_index::<Iovec>(iov_ptr, i))
        .collect::<Result<alloc::vec::Vec<_>, _>>()?;

    // RLIMIT_FSIZE applies to the vector as one write: only a write that
    // starts at the limit is refused (EFBIG, SIGXFSZ), and the segments
    // stop where it is reached.
    let total = iovs
        .iter()
        .fold(0usize, |t, iov| t.saturating_add(iov.iov_len));
    let file = process::current_process().and_then(|p| p.file_table.lock().get(fd));
    let mut room = match file {
        Some(file) if file.flags.write => {
            file.fsize_room(total).map_err(super::map_kernel_error)?
        }
        _ => total,
    };

    for iov in iovs {
        let len = iov.iov_len.min(room);
        if len == 0 {
            if room == 0 {
                break;
            }
            continue;
        }

        // Delegate to existing sys_write for each segment
        match sys_write(fd, iov.iov_base, len) {
            Ok(n) => {
                total_written += n;
                room -= n.min(room);
                // Short write means buffer full or error
                if n < iov.iov_len {
                    break;
                }
            }
            Err(e) => {
                // If we already wrote some data, return what we have
                if total_written > 0 {
                    if let Some(file) =
                        process::current_process().and_then(|p| p.file_table.lock().get(fd))
                    {
                        sigpipe_after_partial_write(&file, e);
                    }
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

/// unlinkat: remove a directory (rmdir).
const AT_REMOVEDIR: usize = 0x200;
/// Do not follow a final symlink.
const AT_SYMLINK_NOFOLLOW: usize = 0x100;
/// An empty path names the dirfd itself.
const AT_EMPTY_PATH: usize = 0x1000;

/// Create a hard link (syscall 155).
///
/// Creates a new directory entry `new_path` pointing to the same file as
/// `old_path`. Both paths must be on the same filesystem.
pub fn sys_link(old_ptr: usize, new_ptr: usize) -> SyscallResult {
    let old_path = read_user_path(old_ptr)?;
    let new_path = read_user_path(new_ptr)?;
    let (parent, link_name) = parent_dir(&new_path)?;
    check_dir_write(&*parent)?;

    let vfs = vfs()?;

    // Resolve the old path to get the target node
    let target = vfs.resolve_path(&old_path).map_err(map_resolve_err)?;

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
    let (parent, link_name) = parent_dir(&link_path)?;
    check_dir_write(&*parent)?;

    let node = parent
        .symlink(&link_name, &target)
        .map_err(|_| SyscallError::InvalidArgument)?;
    own_new_node(&node);

    Ok(0)
}

/// Change file permissions by path (syscall 185).
pub fn sys_chmod(path_ptr: usize, mode: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;

    let node = vfs()?.resolve_path(&path).map_err(map_resolve_err)?;
    chmod_node(&node, mode)
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
    chmod_node(&file.node, mode)
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

/// truncate: set the length of the regular file at `path` (EISDIR for a
/// directory, EINVAL for anything else), which needs write permission on
/// it; a negative length is EINVAL before the lookup (N-191).
pub fn sys_truncate(path_ptr: usize, length: usize) -> SyscallResult {
    let length = file_offset(length)?;
    let path = read_user_path(path_ptr)?;
    let node = vfs()?.resolve_path(&path).map_err(map_resolve_err)?;
    match node.node_type() {
        crate::fs::NodeType::File => {}
        crate::fs::NodeType::Directory => return Err(SyscallError::IsADirectory),
        _ => return Err(SyscallError::InvalidArgument),
    }
    // As opening it for writing would need (N-191).
    require_open_access(&node, &OpenFlags::write_only())?;
    crate::fs::check_fsize_truncate(&*node, length).map_err(super::map_kernel_error)?;
    node.truncate(length).map_err(super::map_kernel_error)?;
    crate::fs::remove_privs_after_write(&*node);
    Ok(0)
}

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

/// newfstatat(dirfd, path, buf, flags) (N-196), and stat and lstat
/// through it, as Linux's vfs_fstatat: AT_SYMLINK_NOFOLLOW stats a final
/// symlink itself (it was always followed), AT_EMPTY_PATH with an empty
/// path stats `dirfd` itself -- any kind of file -- or the cwd,
/// AT_NO_AUTOMOUNT is accepted (there is no automounting); other flags are
/// EINVAL. A trailing slash follows a final symlink and needs a directory
/// (ENOTDIR).
pub fn sys_fstatat(dirfd: usize, path_ptr: usize, stat_buf: usize, flags: usize) -> SyscallResult {
    const AT_NO_AUTOMOUNT: usize = 0x800;
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT | AT_EMPTY_PATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let rel_path = if flags & AT_EMPTY_PATH != 0 {
        read_user_path_or_empty(path_ptr)?
    } else {
        read_user_path(path_ptr)?
    };
    validate_user_ptr_typed::<FileStat>(stat_buf)?;

    let vfs = vfs()?;
    let node = if rel_path.is_empty() {
        if dirfd as u32 as i32 == AT_FDCWD as u32 as i32 {
            vfs.resolve_path(&vfs.get_cwd()).map_err(map_resolve_err)?
        } else {
            let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
            let file = proc
                .file_table
                .lock()
                .get(dirfd as u32 as usize)
                .ok_or(SyscallError::BadFileDescriptor)?;
            file.node.clone()
        }
    } else {
        let abs_path = resolve_at_path(dirfd, &rel_path)?;
        let trailing_slash = rel_path.ends_with('/');
        let node = if flags & AT_SYMLINK_NOFOLLOW != 0 && !trailing_slash {
            vfs.resolve_path_no_follow(&abs_path)
        } else {
            vfs.resolve_path(&abs_path)
        }
        .map_err(map_resolve_err)?;
        if trailing_slash && node.node_type() != crate::fs::NodeType::Directory {
            return Err(SyscallError::NotADirectory);
        }
        node
    };

    let metadata = node.metadata().map_err(super::map_kernel_error)?;
    super::userspace::write_user(stat_buf, fill_stat(&metadata, &*node))?;
    Ok(0)
}

/// unlinkat(dirfd, path, flags) (N-195), and unlink and rmdir through it,
/// as Linux's do_unlinkat and do_rmdir: a relative path is resolved
/// against the cwd or `dirfd` (unlink used to refuse a path without `/`).
/// AT_REMOVEDIR removes a directory -- ENOTDIR for anything else, EINVAL
/// for `.`, ENOTEMPTY for `..`, EBUSY for `/` -- and without it a directory
/// is EISDIR, as is `.`, `..` or `/`, and a trailing slash on anything but
/// a directory is ENOTDIR. The last component is not followed: a symlink
/// is removed, not its target. Other flags are EINVAL.
pub fn sys_unlinkat(dirfd: usize, path_ptr: usize, flags: usize) -> SyscallResult {
    if flags & !AT_REMOVEDIR != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let remove_dir = flags & AT_REMOVEDIR != 0;
    let rel_path = read_user_path(path_ptr)?;
    let trimmed = rel_path.trim_end_matches('/');
    let trailing_slash = trimmed.len() != rel_path.len();
    let last = trimmed.rsplit('/').next().unwrap_or("");
    if trimmed.is_empty() {
        // "/" (or "//"): the root.
        return Err(if remove_dir {
            SyscallError::Busy
        } else {
            SyscallError::IsADirectory
        });
    }
    match (remove_dir, last) {
        (true, ".") => return Err(SyscallError::InvalidArgument),
        (true, "..") => return Err(SyscallError::DirectoryNotEmpty),
        (false, "." | "..") => return Err(SyscallError::IsADirectory),
        _ => {}
    }
    let abs_path = resolve_at_path(dirfd, trimmed)?;
    let node = vfs()?
        .resolve_path_no_follow(&abs_path)
        .map_err(map_resolve_err)?;
    let is_dir = node.node_type() == crate::fs::NodeType::Directory;
    match (remove_dir, is_dir) {
        (true, false) => return Err(SyscallError::NotADirectory),
        (false, true) => return Err(SyscallError::IsADirectory),
        (false, false) if trailing_slash => return Err(SyscallError::NotADirectory),
        _ => {}
    }
    let (dir, name) = parent_dir(&abs_path)?;
    check_may_remove(&*dir, &name)?;
    // The type checked above is checked again where the removal happens.
    dir.unlink_kind(&name, remove_dir)
        .map_err(super::map_kernel_error)?;
    Ok(0)
}

/// Create a directory relative to a directory fd (syscall 193).
pub fn sys_mkdirat(dirfd: usize, path_ptr: usize, mode: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    make_directory(&resolve_at_path(dirfd, &rel_path)?, mode)
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

/// renameat2(olddirfd, old, newdirfd, new, flags) (syscall 316; 276 in the
/// generic table, where it is riscv64's only rename). Flags 0 is renameat.
/// RENAME_NOREPLACE, RENAME_EXCHANGE and RENAME_WHITEOUT are EINVAL, as Linux
/// answers on a filesystem without them: none of the filesystems here can
/// do them atomically.
pub fn sys_renameat2(
    olddirfd: usize,
    old_ptr: usize,
    newdirfd: usize,
    new_ptr: usize,
    flags: usize,
) -> SyscallResult {
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    sys_renameat(olddirfd, old_ptr, newdirfd, new_ptr)
}

/// The file behind `fd` for pread/pwrite, in Linux's order: a negative
/// offset is EINVAL, then an unknown fd EBADF, then a stream (pipe, socket,
/// terminal, event fd), which has no offset, ESPIPE (N-190). The access
/// mode is the caller's check.
fn positioned_file(
    fd: usize,
    offset: usize,
) -> Result<alloc::sync::Arc<crate::fs::file::File>, SyscallError> {
    file_offset(offset)?;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    if file.node.is_stream() {
        return Err(SyscallError::IllegalSeek);
    }
    Ok(file)
}

/// Read from a file descriptor at a given offset without changing position
/// (syscall 195).
pub fn sys_pread(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    let file = positioned_file(fd, offset)?;
    if !file.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    if file.node.node_type() == crate::fs::NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;

    // Read at `offset` through the VfsNode, bypassing the File position,
    // a chunk at a time through a kernel buffer (N-43).
    let mut at = offset;
    super::userspace::produce_to_user(
        buf,
        count,
        true,
        |kbuf| {
            let n = file.node.read(at, kbuf).map_err(super::map_kernel_error)?;
            at += n;
            Ok(n)
        },
        |_| {},
    ) // no file position to give bytes back to
}

/// Write to a file descriptor at a given offset without changing position
/// (syscall 196).
pub fn sys_pwrite(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    let file = positioned_file(fd, offset)?;
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;

    // RLIMIT_FSIZE for the whole write, then at `offset` through the
    // VfsNode, a chunk at a time (N-43).
    let count =
        crate::fs::check_fsize(&*file.node, offset, count).map_err(super::map_kernel_error)?;
    let mut at = offset;
    let written = super::userspace::consume_from_user(buf, count, true, |kbuf| {
        let n = file.node.write(at, kbuf).map_err(super::map_kernel_error)?;
        at += n;
        Ok(n)
    })?;
    crate::fs::remove_privs_after_write(&*file.node);
    Ok(written)
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

/// Whether `creds` may give a file owned by `file_uid`:`file_gid` the
/// owner `uid` and group `gid` (`None`: unchanged), as Linux's chown_ok and
/// chgrp_ok with _POSIX_CHOWN_RESTRICTED: CAP_CHOWN may do anything; the
/// owner may keep the owner and set the group to one it belongs to.
pub(crate) fn may_chown(
    caller: &Caller,
    file_uid: u32,
    file_gid: u32,
    uid: Option<u32>,
    gid: Option<u32>,
) -> bool {
    if caller.capable(crate::process::linux_caps::CAP_CHOWN) {
        return true;
    }
    let creds = &caller.creds;
    let owner = creds.fsuid == file_uid;
    let uid_ok = uid.is_none_or(|u| owner && u == file_uid);
    let gid_ok = gid.is_none_or(|g| owner && (g == file_gid || creds.in_group(g)));
    uid_ok && gid_ok
}

/// Apply a chown to `node` ([`may_chown`], EPERM otherwise; FS-SEC-02).
/// This used to be a no-op that reported success.
pub(crate) fn chown_node(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    uid: usize,
    gid: usize,
) -> SyscallResult {
    let (uid, gid) = (chown_id(uid), chown_id(gid));
    if uid.is_none() && gid.is_none() {
        return Ok(0);
    }
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if !may_chown(&Caller::current(), meta.uid, meta.gid, uid, gid) {
        return Err(SyscallError::OperationNotPermitted);
    }
    node.chown(uid, gid).map_err(super::map_kernel_error)?;
    // A non-directory that changes hands loses set-user-ID and
    // set-group-ID (`Permissions::after_chown`).
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if meta.node_type != crate::fs::NodeType::Directory {
        if let Some(kept) = meta.permissions.after_chown() {
            node.chmod(kept).map_err(super::map_kernel_error)?;
        }
    }
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

/// mknod(2): mknodat relative to the working directory.
pub fn sys_mknod(path_ptr: usize, mode: usize, dev: usize) -> SyscallResult {
    sys_mknodat(AT_FDCWD, path_ptr, mode, dev)
}

/// mknodat(2), as Linux's `filename_mknodat`: a regular file (type 0 or
/// S_IFREG), FIFO, character or block device numbered `dev` (32 bits,
/// Linux's encoding) or socket file named by `path_ptr` relative to
/// `dirfd`. A directory type is EPERM, any other type EINVAL; an existing
/// name (not followed) is EEXIST; the parent needs write and search
/// permission (EACCES); a device needs privilege (EPERM, CAP_MKNOD: root
/// here); the mode is reduced by the umask. A filesystem that cannot hold
/// special files (devfs, procfs) is EPERM.
pub fn sys_mknodat(dirfd: usize, path_ptr: usize, mode: usize, dev: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let kind = crate::fs::mknod_kind(mode as u32).map_err(|dir| {
        if dir {
            SyscallError::OperationNotPermitted
        } else {
            SyscallError::InvalidArgument
        }
    })?;
    let path = resolve_at_path(dirfd, &rel_path)?;
    let perms = creation_perms(mode);
    make_node(&path, rel_path.ends_with('/'), kind, perms, dev as u32)?;
    Ok(0)
}

/// Make the special file (or, for `NodeType::File`, regular file) `kind` at
/// the absolute `path`, owned by the caller, as mknodat does: EEXIST for an
/// existing name (not followed), ENOENT for a name that ended in '/'
/// (`trailing_slash`), the parent's MAC, write and search checks, and
/// privilege for a device. `rdev` is used only for a device. bind() of a
/// Unix socket makes its socket file through this too.
pub(crate) fn make_node(
    path: &str,
    trailing_slash: bool,
    kind: crate::fs::NodeType,
    perms: Permissions,
    rdev: u32,
) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, SyscallError> {
    match vfs()?.resolve_path_no_follow(path) {
        Ok(_) => return Err(SyscallError::FileExists),
        Err(e) if is_not_found(&e) => {}
        Err(e) => return Err(map_resolve_err(e)),
    }
    // "name/" can only be a directory, which mknod does not make.
    if trailing_slash {
        return Err(SyscallError::ResourceNotFound);
    }
    let device = matches!(
        kind,
        crate::fs::NodeType::CharDevice | crate::fs::NodeType::BlockDevice
    );
    let rdev = if device { rdev } else { 0 };
    // Every type, regular files included, goes through one resolution of
    // the parent for the MAC check, the permission checks and the creation.
    let node = vfs()?.mknod_checked(path, kind, perms, rdev, |dir| {
        let denied = crate::error::KernelError::PermissionDenied {
            operation: "create directory entry",
        };
        check_dir_write(dir).map_err(|_| denied)?;
        if device && !Caller::current().capable(crate::process::linux_caps::CAP_MKNOD) {
            return Err(crate::error::KernelError::FsError(
                crate::error::FsError::OperationNotPermitted,
            ));
        }
        Ok(())
    });
    let node = node.map_err(|e| match e {
        crate::error::KernelError::FsError(crate::error::FsError::AlreadyExists) => {
            SyscallError::FileExists
        }
        crate::error::KernelError::FsError(crate::error::FsError::NotSupported) => {
            SyscallError::OperationNotPermitted
        }
        e => super::map_kernel_error(e),
    })?;
    own_new_node(&node);
    Ok(node)
}

/// The socket file bind() of a Unix socket makes for the name `name`
/// (relative to the working directory), as Linux's `unix_bind_bsd`: a
/// socket file with mode 0777 less the umask, through [`make_node`], so an
/// existing name is EADDRINUSE.
pub(crate) fn make_socket_file(
    name: &str,
) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, SyscallError> {
    let path = resolve_at_path(AT_FDCWD, name)?;
    make_node(
        &path,
        name.ends_with('/'),
        crate::fs::NodeType::Socket,
        creation_perms(0o777),
        0,
    )
    .map_err(|e| match e {
        SyscallError::FileExists => SyscallError::AddressInUse,
        e => e,
    })
}

/// The socket file a connect (or an addressed send) to the Unix socket name
/// `name` reaches, as Linux's `unix_find_bsd`: the name is followed like any
/// path, the caller needs write permission on the file (EACCES), and
/// anything but a socket file is ECONNREFUSED.
pub(crate) fn find_socket_file(
    name: &str,
) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, SyscallError> {
    let path = resolve_at_path(AT_FDCWD, name)?;
    let file = vfs()?.resolve_path(&path).map_err(map_resolve_err)?;
    require_open_access(&file, &OpenFlags::write_only())?;
    if file.node_type() != crate::fs::NodeType::Socket {
        return Err(SyscallError::ConnectionRefused);
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    #[test]
    fn generic_stat_takes_every_field() {
        let x = super::X86Stat {
            st_dev: 1,
            st_ino: 2,
            st_nlink: 3,
            st_mode: 0o100644,
            st_uid: 5,
            st_gid: 6,
            __pad0: 0,
            st_rdev: 7,
            st_size: 8,
            st_blksize: 4096,
            st_blocks: 9,
            st_atime: 10,
            st_atime_nsec: 11,
            st_mtime: 12,
            st_mtime_nsec: 13,
            st_ctime: 14,
            st_ctime_nsec: 15,
            __unused: [0; 3],
        };
        let g = super::GenericStat::from(x);
        assert_eq!(
            (g.st_dev, g.st_ino, g.st_mode, g.st_nlink),
            (1, 2, 0o100644, 3)
        );
        assert_eq!((g.st_uid, g.st_gid, g.st_rdev, g.st_size), (5, 6, 7, 8));
        assert_eq!((g.st_blksize, g.st_blocks), (4096, 9));
        assert_eq!((g.st_atime, g.st_atime_nsec, g.st_mtime), (10, 11, 12));
        assert_eq!((g.st_mtime_nsec, g.st_ctime, g.st_ctime_nsec), (13, 14, 15));
    }

    use super::*;
    use crate::process::{
        creds::Credentials,
        linux_caps::{LinuxCaps, CAP_CHOWN, CAP_FOWNER, CAP_FSETID},
    };

    /// A user 1000 in group 100 and supplementary group 20, holding `caps`.
    fn user_with(caps: &[u32]) -> Caller {
        let mut c = Credentials::new(0, 0);
        c.setgroups(&[20], true).unwrap();
        c.setresgid(100, 100, 100, true).unwrap();
        c.setresuid(1000, 1000, 1000, true).unwrap();
        Caller {
            creds: c,
            caps: caps.iter().fold(LinuxCaps::EMPTY, |s, &cap| s.with(cap)),
        }
    }

    fn user() -> Caller {
        user_with(&[])
    }

    /// chmod: the owner or CAP_FOWNER (EPERM for others); set-group-ID is
    /// dropped for a caller outside the file's group without CAP_FSETID.
    #[test]
    fn chmod_follows_linux_rules() {
        let u = user();
        assert_eq!(
            chmod_perms(&u, 0, 0, 0o644),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            chmod_perms(&u, 1000, 100, 0o6755).unwrap().to_mode(),
            0o6755
        );
        assert_eq!(chmod_perms(&u, 1000, 20, 0o2755).unwrap().to_mode(), 0o2755);
        assert_eq!(chmod_perms(&u, 1000, 7, 0o6755).unwrap().to_mode(), 0o4755);
        let root = Caller::KERNEL;
        assert_eq!(
            chmod_perms(&root, 1000, 7, 0o2755).unwrap().to_mode(),
            0o2755
        );
        // The capabilities decide, not uid 0.
        let fowner = user_with(&[CAP_FOWNER]);
        assert_eq!(chmod_perms(&fowner, 0, 7, 0o2755).unwrap().to_mode(), 0o755);
        let both = user_with(&[CAP_FOWNER, CAP_FSETID]);
        assert_eq!(chmod_perms(&both, 0, 7, 0o2755).unwrap().to_mode(), 0o2755);
        let bare_root = Caller {
            creds: Credentials::new(0, 0),
            caps: LinuxCaps::EMPTY,
        };
        assert_eq!(
            chmod_perms(&bare_root, 1000, 7, 0o644),
            Err(SyscallError::OperationNotPermitted)
        );
        // File type bits in the argument are ignored.
        assert_eq!(
            chmod_perms(&u, 1000, 100, 0o100644).unwrap().to_mode(),
            0o644
        );
    }

    /// chown: CAP_CHOWN anything; the owner only its own groups, never the
    /// user.
    #[test]
    fn chown_is_restricted_to_root_and_owner_chgrp() {
        let u = user();
        assert!(may_chown(&u, 1000, 100, None, Some(20)));
        assert!(may_chown(&u, 1000, 100, Some(1000), Some(100)));
        assert!(!may_chown(&u, 1000, 100, None, Some(7)));
        assert!(!may_chown(&u, 1000, 100, Some(1001), None));
        assert!(!may_chown(&u, 1001, 100, None, Some(100)));
        assert!(may_chown(&Caller::KERNEL, 1001, 7, Some(5), Some(9)));
        assert!(may_chown(
            &user_with(&[CAP_CHOWN]),
            1001,
            7,
            Some(5),
            Some(9)
        ));
        let bare_root = Caller {
            creds: Credentials::new(0, 0),
            caps: LinuxCaps::EMPTY,
        };
        assert!(!may_chown(&bare_root, 1001, 7, Some(5), None));
    }

    /// kill(2): uid matches or CAP_KILL, not uid 0; oneself always; SIGCONT
    /// within the session (check_kill_permission).
    #[test]
    fn kill_needs_a_uid_match_or_cap_kill() {
        use crate::process::{exit::signals::SIGCONT, linux_caps::CAP_KILL};
        let target = Credentials::new(2000, 2000);
        let mut saved = Credentials::new(3000, 3000);
        saved.suid = 1000;
        let u = user();
        assert!(!kill_ok(&u, &target, false, 15, false));
        assert!(kill_ok(&u, &saved, false, 15, false), "target's saved uid");
        assert!(kill_ok(&u, &target, true, 15, false), "itself");
        assert!(kill_ok(&user_with(&[CAP_KILL]), &target, false, 15, false));
        assert!(kill_ok(&u, &target, false, SIGCONT, true));
        assert!(!kill_ok(&u, &target, false, SIGCONT, false));
        let bare_root = Caller {
            creds: Credentials::new(0, 0),
            caps: LinuxCaps::EMPTY,
        };
        assert!(!kill_ok(&bare_root, &target, false, 15, false));
        assert!(kill_ok(&Caller::KERNEL, &target, false, 15, false));
    }

    /// loff_t is signed: a negative offset or length (as a register value)
    /// is EINVAL, not a huge unsigned one (N-190, N-191).
    #[test]
    fn negative_file_offsets_are_invalid() {
        assert_eq!(file_offset(0), Ok(0));
        assert_eq!(file_offset(i64::MAX as usize), Ok(i64::MAX as usize));
        assert_eq!(
            file_offset(-1i64 as usize),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            file_offset(i64::MIN as usize),
            Err(SyscallError::InvalidArgument)
        );
    }

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
    /// execute bit to execute (N-193). Its mode argument is the MAY_* mask.
    #[test]
    fn access_uses_owner_group_other_precedence() {
        let granted = |uid: u32, gid: u32, m: &crate::fs::Metadata, mode: u32| {
            let caller = Caller {
                creds: Credentials::new(uid, gid),
                caps: crate::process::linux_caps::LinuxCaps::EMPTY,
            };
            crate::fs::permission(&caller, m, mode)
        };
        let m = meta(0o750, 1000, 100);
        assert!(granted(1000, 100, &m, 7));
        assert!(granted(2000, 100, &m, 4 | 1));
        assert!(!granted(2000, 100, &m, 2));
        assert!(!granted(2000, 200, &m, 4));
        assert!(crate::fs::permission(&Caller::KERNEL, &m, 7));
        let noexec = meta(0o644, 1000, 100);
        assert!(!crate::fs::permission(&Caller::KERNEL, &noexec, 1));
        let dir = crate::fs::Metadata {
            node_type: crate::fs::NodeType::Directory,
            ..noexec
        };
        assert!(crate::fs::permission(&Caller::KERNEL, &dir, 1));
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

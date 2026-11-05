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

use super::{
    multiplex::{POLLERR, POLLHUP, POLLIN, POLLOUT},
    userspace::UserSegs,
};
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
    let path_owned = read_open_path(path, flags)?;
    if path_owned.is_empty() {
        return open_empty_path(AT_FDCWD, flags, mode, false);
    }
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

    open_path(&vfs()?, path_str, flags, mode, false)
}

/// Open `path` with Linux `open(2)` semantics (shared by open, openat and
/// openat2), looked up in `view`. `regular` is openat2's OPENAT2_REGULAR:
/// anything but a regular file is EFTYPE.
///
/// - O_CREAT|O_EXCL fails with EEXIST if the name exists, and a lost create
///   race is EEXIST or a plain open, never ENOENT;
/// - O_TRUNC truncates (openat used to skip it, and musl routes every open()
///   through openat) and reports a failure;
/// - O_NOFOLLOW refuses a final symlink (ELOOP), O_DIRECTORY anything but a
///   directory (ENOTDIR), and a directory cannot be opened for writing (EISDIR)
///   (N-116);
/// - O_PATH keeps only O_DIRECTORY, O_NOFOLLOW and O_CLOEXEC (and O_EMPTYPATH),
///   as Linux's build_open_how, and O_TMPFILE makes an unnamed file in the
///   directory `path` names (N-206).
fn open_path(
    view: &crate::fs::CallerView<'_>,
    path: &str,
    flags: usize,
    mode: usize,
    regular: bool,
) -> SyscallResult {
    let flags = open_how_flags(flags);
    check_open_flags(flags)?;
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
    use crate::fs::NodeType;

    let open_flags = OpenFlags::from_bits(flags as u32).ok_or(SyscallError::InvalidArgument)?;
    let follow = flags & O_NOFOLLOW == 0;
    if flags & O_TMPFILE_BIT as usize != 0 {
        // The directory is looked up as an open for writing is, so the
        // MAC check is a write to it.
        let dir_flags = OpenFlags {
            read: false,
            write: true,
            ..open_flags
        };
        let (dir, canonical) = view
            .open_follow_canonical(path, dir_flags, true)
            .map_err(map_resolve_err)?;
        return open_tmpfile(dir, &canonical, flags, mode);
    }

    let (node, created, canonical) = match view.open_follow_canonical(path, open_flags, follow) {
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
            let made = view.mknod_checked(path, NodeType::File, perms, 0, |dir| {
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
                    let (node, canonical) = view
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
    // The file records the canonical path of what it opened (in the whole
    // tree) for fchdir and dirfd resolution: the path as written could be
    // relative, and would be re-resolved against a later cwd (N-204).
    let canonical = match canonical {
        Some(c) => c,
        None => view
            .resolve_canonical(path, &view.get_cwd(), false)
            .map(|(_, c)| c)
            .unwrap_or_else(|_| alloc::string::String::from(path)),
    };
    open_node(node, created, canonical, flags, regular, follow)
}

/// The second half of an open, once the name is resolved: the checks on
/// the node itself (Linux's do_open), then the new descriptor. `created`
/// says this open made the file; `follow` is false when a final symlink was
/// not followed, and then opening it is ELOOP. open_by_handle_at opens a
/// decoded node here with `follow` false, as Linux opens it through
/// file_open_root.
pub(super) fn open_node(
    node: alloc::sync::Arc<dyn crate::fs::VfsNode>,
    created: bool,
    canonical: alloc::string::String,
    flags: usize,
    regular: bool,
    follow: bool,
) -> SyscallResult {
    #[cfg(target_arch = "aarch64")]
    const O_DIRECTORY: usize = 0x4000;
    #[cfg(not(target_arch = "aarch64"))]
    const O_DIRECTORY: usize = 0x1_0000;
    const O_CLOEXEC: usize = 0x8_0000;
    use crate::fs::NodeType;

    let process = process::current_process().ok_or(SyscallError::InvalidState)?;
    let cloexec = flags & O_CLOEXEC != 0;
    let node_type = node.node_type();
    if flags & O_PATH as usize != 0 {
        // An O_PATH open only names the file (Linux's do_o_path): no
        // permission check, truncation or device or FIFO open, a final
        // symlink kept when not followed; only O_DIRECTORY is checked.
        if flags & O_DIRECTORY != 0 && node_type != NodeType::Directory {
            return Err(SyscallError::NotADirectory);
        }
        let mut file =
            crate::fs::file::File::new_with_path(node, OpenFlags::path_only(), canonical);
        file.opener = Some(alloc::boxed::Box::new(Caller::current()));
        file.status.fetch_or(
            crate::fs::file::status::PATH,
            core::sync::atomic::Ordering::AcqRel,
        );
        return process
            .file_table
            .lock()
            .open_with_flags(alloc::sync::Arc::new(file), cloexec)
            .map_err(super::map_kernel_error);
    }
    let open_flags = OpenFlags::from_bits(flags as u32).ok_or(SyscallError::InvalidArgument)?;
    // O_CREAT names a file: an existing directory is EISDIR even read-only.
    if open_flags.create && !created && node_type == NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    if !follow && node_type == NodeType::Symlink {
        return Err(SyscallError::SymlinkLoop);
    }
    // After EEXIST and EISDIR, before ENOTDIR, as Linux's do_open.
    if regular && node_type != NodeType::File {
        return Err(SyscallError::WrongFileType);
    }
    if flags & O_DIRECTORY != 0 && node_type != NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    if open_flags.write && node_type == NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    // The mount's flags (ADR 0022 H3c), in the order of Linux's do_open,
    // may_open and do_dentry_open: a device on a nodev mount is EACCES;
    // truncating a regular file, or writing to anything but a device, FIFO
    // or socket, on a read-only filesystem is EROFS before the permission
    // check, and on a read-only mount after it. A file this open created
    // is on a writable mount.
    use crate::fs::mount::flags as mnt;
    let mount = vfs()?.mount_serving(&canonical);
    let device = matches!(node_type, NodeType::CharDevice | NodeType::BlockDevice);
    let special = device || matches!(node_type, NodeType::Pipe | NodeType::Socket);
    if device && mount.as_ref().is_some_and(|m| m.has(mnt::NODEV)) {
        return Err(SyscallError::PermissionDenied);
    }
    let writes = !created && !special && open_flags.write;
    let truncates = !created && open_flags.truncate && node_type == NodeType::File;
    let mount_ro = mount.as_ref().is_some_and(|m| m.is_readonly());
    let sb_ro = mount.as_ref().is_some_and(|m| m.sb_readonly());
    if (truncates && mount_ro) || (writes && sb_ro) {
        return Err(SyscallError::ReadOnlyFilesystem);
    }
    if !created {
        require_open_access(&node, &open_flags)?;
    }
    if writes && mount_ro {
        return Err(SyscallError::ReadOnlyFilesystem);
    }
    check_open_status(&*node, flags)?;
    // A writable open of a running program is ETXTBSY, before O_TRUNC
    // truncates it (Linux's do_dentry_open, then handle_truncate).
    let write_access = if open_flags.write {
        Some(alloc::sync::Arc::new(
            crate::fs::write_access::WriteAccess::get(&node).map_err(super::map_kernel_error)?,
        ))
    } else {
        None
    };
    let truncated =
        open_flags.truncate && open_flags.write && !created && node_type == NodeType::File;
    if truncated {
        open_truncate(&*node)?;
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

    let mut file = crate::fs::file::File::new_with_path(node, open_flags, canonical);
    file.opener = Some(alloc::boxed::Box::new(Caller::current()));
    file.write_access = write_access;
    file.status
        .fetch_or(open_status(flags), core::sync::atomic::Ordering::AcqRel);
    // IN_OPEN, then O_TRUNC's IN_MODIFY, as Linux's do_open.
    crate::fs::inotify::on_file(&file, crate::fs::inotify::IN_OPEN);
    if truncated {
        crate::fs::inotify::on_file(&file, crate::fs::inotify::IN_MODIFY);
    }
    let file_table = process.file_table.lock();
    file_table
        .open_with_flags(alloc::sync::Arc::new(file), cloexec)
        .map_err(super::map_kernel_error)
}

/// The flags open(2) and openat(2) act on, as Linux's build_open_how:
/// with O_PATH only O_DIRECTORY, O_NOFOLLOW, O_PATH, O_CLOEXEC and
/// O_EMPTYPATH remain (openat2 refuses the rest in [`check_open_how`]).
fn open_how_flags(flags: usize) -> usize {
    if flags as u64 & O_PATH != 0 {
        flags & (O_DIRECTORY_64 | O_NOFOLLOW_64 | O_PATH | O_CLOEXEC | O_EMPTYPATH) as usize
    } else {
        flags
    }
}

/// The flag combinations open refuses (EINVAL), in Linux's
/// build_open_flags order: O_CREAT with O_DIRECTORY, whether the name
/// exists or not, as on Linux since 6.4 (it made a regular file, then
/// failed with ENOTDIR and left the file behind; PR #23 review); then
/// O_TMPFILE (which includes O_DIRECTORY) without both its bits or without
/// a write access mode.
fn check_open_flags(flags: usize) -> Result<(), SyscallError> {
    let flags = flags as u64;
    if flags & (O_DIRECTORY_64 | O_CREAT) == O_DIRECTORY_64 | O_CREAT {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & O_TMPFILE_BIT != 0 && (flags & O_DIRECTORY_64 == 0 || flags & 3 == 0) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// The file status flags an open records from `flags` (F_GETFL reports
/// them): O_ASYNC, O_DIRECT, O_NOATIME, O_DSYNC and O_SYNC, which implies
/// O_DSYNC as on Linux. O_APPEND comes with the open flags.
fn open_status(flags: usize) -> u32 {
    use crate::fs::file::status;
    let mut bits = flags as u32 & (status::OPEN & !status::PATH);
    if bits & status::SYNC != 0 {
        bits |= status::DSYNC;
    }
    bits
}

/// The status flags' own checks at open, after the permission check as in
/// Linux's may_open and do_dentry_open: O_NOATIME needs the file's owner
/// or CAP_FOWNER (EPERM), and O_DIRECT a regular file (EINVAL, as for
/// F_SETFL).
fn check_open_status(node: &dyn crate::fs::VfsNode, flags: usize) -> Result<(), SyscallError> {
    use crate::fs::file::status;
    if flags as u32 & status::NOATIME != 0 {
        let meta = node.metadata().map_err(super::map_kernel_error)?;
        if !Caller::current().owner_or_capable(meta.uid) {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    if flags as u32 & status::DIRECT != 0 && node.node_type() != crate::fs::NodeType::File {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// O_TMPFILE (N-206): an unnamed regular file in directory `dir` (whose
/// canonical path is `canonical`), as Linux's vfs_tmpfile: write and search
/// permission on the directory (EACCES), a filesystem that makes such files
/// (EOPNOTSUPP: ramfs and tmpfs do, BlockFS does not), the mode less the
/// umask. It has no name until linkat with AT_EMPTY_PATH gives it one,
/// which O_EXCL forbids; its last close frees it. The descriptor records
/// `<dir>/#<inode>` as its path, as Linux shows such a file, so the mount
/// is known and nothing finds the file by that name.
fn open_tmpfile(
    dir: alloc::sync::Arc<dyn crate::fs::VfsNode>,
    canonical: &str,
    flags: usize,
    mode: usize,
) -> SyscallResult {
    const O_EXCL: usize = 0x80;
    if dir.node_type() != crate::fs::NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    // Linux's do_o_tmpfile: EROFS before the permission check.
    want_write(&vfs()?, canonical)?;
    check_dir_write(&*dir)?;
    let node = dir
        .tmpfile(creation_perms(mode), flags & O_EXCL == 0)
        .map_err(super::map_kernel_error)?;
    own_new_node(&node);
    let inode = node.metadata().map_err(super::map_kernel_error)?.inode;
    let path = alloc::format!("{}/#{}", canonical.trim_end_matches('/'), inode);
    // O_DIRECTORY is part of O_TMPFILE, not a request for a directory.
    let flags = flags & !(O_TMPFILE_BIT | O_DIRECTORY_64) as usize;
    open_node(node, true, path, flags, false, true)
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
    if count != 0 {
        // Validate buffer is in user space
        if let Err(e) = validate_user_buffer(buffer, count) {
            crate::drivers::console_input::end_read();
            return Err(e);
        }
    }
    read_segs(fd, UserSegs::new(&[(buffer, count)]))
}

/// read(2) and readv(2) of `fd` into `dst`, as one read.
fn read_segs(fd: usize, dst: UserSegs<'_>) -> SyscallResult {
    let result = read_fd(fd, dst);
    // However the read ended (data, EINTR, EAGAIN), a console read's VTIME
    // timer ends with it.
    crate::drivers::console_input::end_read();
    result
}

fn read_fd(fd: usize, dst: UserSegs<'_>) -> SyscallResult {
    if dst.is_empty() {
        return Ok(0);
    }
    dst.validate()?;

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
        let read_count = dst.len().min(SERIAL_IO_MAX_SIZE);
        // Bytes are gathered in a kernel buffer and copied out once (N-43).
        let mut line = alloc::vec![0u8; read_count];
        let n = read_console_waiting(&mut line)?;
        dst.copy_out(0, &line[..n]).map_err(|(_, e)| e)?;
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
        match file_read_to_user(&file_desc, dst) {
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
            match file_read_to_user(&file_desc, dst) {
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
    write_segs(fd, UserSegs::new(&[(buffer, count)]), false)
}

/// write(2) and writev(2) of `src` to `fd`, as one write; `nosignal`
/// (pwritev2's RWF_NOSIGNAL) keeps a pipe that loses its reader part way
/// from raising SIGPIPE.
fn write_segs(fd: usize, src: UserSegs<'_>, nosignal: bool) -> SyscallResult {
    if src.is_empty() {
        return Ok(0);
    }
    // For stdout (fd 1) and stderr (fd 2), try file table first, then
    // fall back to serial output
    if fd == 1 || fd == 2 {
        // Try the file table first if we have a process context
        if let Some(proc) = process::current_process() {
            if let Some(result) = write_fd(&proc, fd, src, nosignal) {
                return result;
            }
        }

        if !console_fallback_allowed(fd) {
            return Err(SyscallError::BadFileDescriptor);
        }
        // Fallback: write directly to serial UART
        let write_count = src.len().min(SERIAL_IO_MAX_SIZE);
        src.validate()?;
        // Copied in once through the fault-tolerant reader (N-43).
        let mut out = alloc::vec![0u8; write_count];
        src.copy_in(0, &mut out).map_err(|(_, e)| e)?;
        for &byte in &out {
            serial_write_byte(byte);
        }

        return Ok(write_count);
    }

    // Non-stdout/stderr: use file table normally
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    write_fd(&proc, fd, src, nosignal).unwrap_or(Err(SyscallError::BadFileDescriptor))
}

/// Write `src` to `fd` of `proc` (`None`: no such fd). A dispatched writer
/// (stage D2) to a full pipe or socket waits for room, as POSIX requires of a
/// blocking descriptor, instead of getting EAGAIN; O_NONBLOCK still gets it
/// (N-119). No lock is held while waiting, and a signal ends the wait (EINTR).
fn write_fd(
    proc: &crate::process::Process,
    fd: usize,
    src: UserSegs<'_>,
    nosignal: bool,
) -> Option<SyscallResult> {
    let count = src.len();
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
        let result = file_write_from_user(&file_desc, src.advance(done));
        #[cfg(feature = "alloc")]
        let dispatched = crate::sched::dispatch::current_owner().is_some();
        #[cfg(not(feature = "alloc"))]
        let dispatched = false;
        match result {
            Ok(n) if blocking && stream && dispatched && done + n < count => done += n,
            Ok(n) => return Some(Ok(done + n)),
            Err(SyscallError::WouldBlock) if blocking && dispatched => {}
            Err(e) if done > 0 => {
                if !nosignal {
                    sigpipe_after_partial_write(&file_desc, e);
                }
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
pub(super) fn wait_ready(
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

/// Read up to `dst.len()` bytes from an open file into user memory `dst`.
/// A regular file is read in chunks until `count` or end of file; anything
/// else (pipe, tty, socket, device) gets exactly one underlying read, so its
/// blocking and short-read behaviour is unchanged. EOF on a pipe is 0.
fn file_read_to_user(file: &crate::fs::file::File, dst: UserSegs<'_>) -> SyscallResult {
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
    let read = super::userspace::produce_to_segs(
        dst,
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
    )?;
    if read > 0 {
        crate::fs::inotify::on_file(file, crate::fs::inotify::IN_ACCESS);
    }
    Ok(read)
}

/// Write up to `src.len()` bytes of user memory `src` to an open file, in
/// chunks (the first gathers every segment of a vector, so a writev of up
/// to PIPE_BUF bytes is one atomic pipe write); a short write (a full
/// pipe) ends the transfer with the count so far.
fn file_write_from_user(file: &crate::fs::file::File, src: UserSegs<'_>) -> SyscallResult {
    // Not open for writing is EBADF, as on Linux (N-197).
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let count = file
        .fsize_room(src.len())
        .map_err(super::map_kernel_error)?;
    // A regular file is written as one step (see `File::io_lock`).
    let _io = (file.node.node_type() == crate::fs::NodeType::File).then(|| file.io_lock.lock());
    let mut first = true;
    let written =
        super::userspace::consume_from_segs(src.truncate(count), true, |kbuf| {
            match file.write_part(kbuf, core::mem::take(&mut first)) {
                Ok(n) => Ok(n),
                Err(crate::error::KernelError::BrokenPipe) => Err(SyscallError::BrokenPipe),
                Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
                Err(e) => Err(super::map_kernel_error(e)),
            }
        })?;
    if written > 0 {
        crate::fs::inotify::on_file(file, crate::fs::inotify::IN_MODIFY);
    }
    Ok(written)
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
        .get_path(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Get metadata and write to user buffer
    let metadata = file_desc.node.metadata().map_err(super::map_kernel_error)?;
    let (mount, _) = mount_of(file_desc.path.as_deref())?;
    let stat = fill_stat(&metadata, &*file_desc.node, mount);

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
    crate::fs::truncate_removing_privs(&*file.node, length).map_err(super::map_kernel_error)?;
    crate::fs::inotify::on_file(&file, crate::fs::inotify::IN_MODIFY);
    Ok(0)
}

/// fallocate (N-208), with Linux's `vfs_fallocate` checks in its order:
/// the range (EINVAL) and the mode (EOPNOTSUPP), then the descriptor
/// (EBADF unless open for writing), the file type (ESPIPE for a pipe,
/// EISDIR, ENODEV for anything else but a regular file) and the range's
/// end (EFBIG). The filesystem does the rest (`VfsNode::fallocate`,
/// starting with `falloc::begin` once it accepts the operation).
pub fn sys_fallocate(fd: usize, mode: usize, offset: usize, len: usize) -> SyscallResult {
    use crate::fs::{falloc, NodeType};
    let fs_err = |e| super::map_kernel_error(crate::error::KernelError::FsError(e));
    let request = falloc::Request::parse(mode as u32, offset as i64, len as i64).map_err(fs_err)?;
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    match file.node.node_type() {
        NodeType::File => {}
        NodeType::Pipe => return Err(SyscallError::IllegalSeek),
        NodeType::Directory => return Err(SyscallError::IsADirectory),
        _ => return Err(SyscallError::NoDevice),
    }
    request.check_range().map_err(fs_err)?;
    file.node
        .fallocate(request)
        .map_err(super::map_kernel_error)?;
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

    // Mount needs CAP_SYS_ADMIN, as on Linux (EPERM), and, as a native
    // authority over the VFS tree, a Memory object capability with WRITE
    // and CREATE rights: both must hold (ADR 0020, decision 9).
    let current = process::current_process().ok_or(SyscallError::InvalidState)?;
    if !Caller::current().capable(crate::process::linux_caps::CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
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

/// Helper: populate a FileStat from the VFS metadata of `node`, on the
/// mount with `mount`: its link count, and its device number for a device.
fn fill_stat(
    metadata: &crate::fs::Metadata,
    node: &dyn crate::fs::VfsNode,
    mount: crate::fs::MountIds,
) -> FileStat {
    let rdev = node
        .device_id()
        .map_or(0, |(major, minor)| crate::fs::make_dev(major, minor));
    let dev = crate::fs::mount_dev(mount);
    #[allow(clippy::useless_conversion)] // the identity on x86_64
    let stat = FileStat::from(fill_x86_stat(metadata, rdev, dev, node.link_count()));
    stat
}

/// The mount serving the file at canonical path `path` (in the whole
/// tree), and whether it is that mount's root; a file without a path
/// (a pipe, a socket) is on the pseudo mount (N-205).
fn mount_of(path: Option<&str>) -> Result<(crate::fs::MountIds, bool), SyscallError> {
    Ok(match path {
        Some(path) => vfs()?.mount_ids_of(path),
        None => (crate::fs::PSEUDO_MOUNT, false),
    })
}

/// [`fill_stat`] in the x86_64 layout; `rdev` and `dev` in Linux's
/// encoding, `nlink` the node's link count.
fn fill_x86_stat(metadata: &crate::fs::Metadata, rdev: u32, dev: u32, nlink: u32) -> X86Stat {
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
    // The owner as the caller's user namespace sees it (cp_new_stat's
    // from_kuid_munged): one it does not map reads as the overflow ID.
    let ns = crate::process::userns::current();
    X86Stat {
        st_dev: dev as u64,
        st_ino: metadata.inode,
        st_nlink: nlink as u64,
        st_mode: mode,
        st_uid: ns.uid_munged(metadata.uid),
        st_gid: ns.gid_munged(metadata.gid),
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

/// Linux's `struct statx_timestamp`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct StatxTimestamp {
    tv_sec: i64,
    tv_nsec: u32,
    __reserved: i32,
}

/// Linux's `struct statx`: 256 bytes, the same on every architecture.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Statx {
    stx_mask: u32,
    stx_blksize: u32,
    stx_attributes: u64,
    stx_nlink: u32,
    stx_uid: u32,
    stx_gid: u32,
    stx_mode: u16,
    __spare0: u16,
    stx_ino: u64,
    stx_size: u64,
    stx_blocks: u64,
    stx_attributes_mask: u64,
    stx_atime: StatxTimestamp,
    stx_btime: StatxTimestamp,
    stx_ctime: StatxTimestamp,
    stx_mtime: StatxTimestamp,
    stx_rdev_major: u32,
    stx_rdev_minor: u32,
    stx_dev_major: u32,
    stx_dev_minor: u32,
    stx_mnt_id: u64,
    stx_dio_mem_align: u32,
    stx_dio_offset_align: u32,
    stx_subvol: u64,
    stx_atomic_write_unit_min: u32,
    stx_atomic_write_unit_max: u32,
    stx_atomic_write_segments_max: u32,
    stx_dio_read_offset_align: u32,
    stx_atomic_write_unit_max_opt: u32,
    __spare2: u32,
    __spare3: [u64; 8],
}

const _: () = assert!(core::mem::size_of::<Statx>() == 256);
// The offsets of include/uapi/linux/stat.h, with no implicit padding.
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(Statx, stx_nlink) == 0x10);
    assert!(offset_of!(Statx, stx_ino) == 0x20);
    assert!(offset_of!(Statx, stx_atime) == 0x40);
    assert!(offset_of!(Statx, stx_rdev_major) == 0x80);
    assert!(offset_of!(Statx, stx_mnt_id) == 0x90);
    assert!(offset_of!(Statx, stx_subvol) == 0xa0);
    assert!(offset_of!(Statx, stx_atomic_write_segments_max) == 0xb0);
    assert!(offset_of!(Statx, __spare3) == 0xc0);
};

const STATX_BASIC_STATS: u32 = 0x7ff;
const STATX_MNT_ID: u32 = 0x1000;
const STATX_MNT_ID_UNIQUE: u32 = 0x4000;
const STATX_RESERVED: u32 = 0x8000_0000;
const STATX_ATTR_AUTOMOUNT: u64 = 0x1000;
const STATX_ATTR_MOUNT_ROOT: u64 = 0x2000;
const STATX_ATTR_DAX: u64 = 0x20_0000;
/// AT_STATX_FORCE_SYNC | AT_STATX_DONT_SYNC; both at once is EINVAL.
const AT_STATX_SYNC_TYPE: usize = 0x6000;

/// What statx reports for a node (Linux's vfs_statx_path and cp_statx):
/// the numbers stat reports, with `nlink` links, its device's major and
/// minor in `rdev`, the id of its mount (the unique one when `mask` asks
/// for it) and whether it is that mount's root. No filesystem records a
/// birth time, so STATX_BTIME is never in the result.
fn fill_statx(
    metadata: &crate::fs::Metadata,
    rdev: (u32, u32),
    mount: crate::fs::MountIds,
    mount_root: bool,
    mask: u32,
    nlink: u32,
) -> Statx {
    let st = fill_x86_stat(metadata, 0, crate::fs::mount_dev(mount), nlink);
    let time = |sec: i64, nsec: i64| StatxTimestamp {
        tv_sec: sec,
        tv_nsec: nsec as u32,
        __reserved: 0,
    };
    let (mnt_bit, mnt_id) = if mask & STATX_MNT_ID_UNIQUE != 0 {
        (STATX_MNT_ID_UNIQUE, mount.unique)
    } else {
        (STATX_MNT_ID, u64::from(mount.id))
    };
    let dev = st.st_dev as u32;
    Statx {
        stx_mask: STATX_BASIC_STATS | mnt_bit,
        stx_blksize: st.st_blksize as u32,
        stx_attributes: if mount_root { STATX_ATTR_MOUNT_ROOT } else { 0 },
        stx_nlink: st.st_nlink as u32,
        stx_uid: st.st_uid,
        stx_gid: st.st_gid,
        stx_mode: st.st_mode as u16,
        stx_ino: st.st_ino,
        stx_size: st.st_size as u64,
        stx_blocks: st.st_blocks as u64,
        stx_attributes_mask: STATX_ATTR_AUTOMOUNT | STATX_ATTR_DAX | STATX_ATTR_MOUNT_ROOT,
        stx_atime: time(st.st_atime, st.st_atime_nsec),
        stx_ctime: time(st.st_ctime, st.st_ctime_nsec),
        stx_mtime: time(st.st_mtime, st.st_mtime_nsec),
        stx_rdev_major: rdev.0,
        stx_rdev_minor: rdev.1,
        stx_dev_major: crate::fs::dev_major(dev),
        stx_dev_minor: crate::fs::dev_minor(dev),
        stx_mnt_id: mnt_id,
        ..Statx::default()
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

/// open(O_TRUNC) of an existing file: truncate it to zero, removing its
/// privileges as a write does. The file was found, so a failure is reported
/// as its own errno.
fn open_truncate(node: &dyn crate::fs::VfsNode) -> Result<(), SyscallError> {
    crate::fs::truncate_removing_privs(node, 0).map_err(super::map_kernel_error)
}

/// The errno of a failed path lookup: the error's own (N-206). It used to
/// map ENOSPC to ENOMEM and anything unlisted to ENOENT.
pub(crate) fn map_resolve_err(e: crate::error::KernelError) -> SyscallError {
    super::map_kernel_error(e)
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

/// utimes/futimesat's two timevals ([tv_sec, tv_usec]) as utimensat times,
/// as Linux's do_futimesat converts them. The range check comes before the
/// multiplication so that it also refuses UTIME_NOW and UTIME_OMIT.
fn timeval_times(tv: [[i64; 2]; 2]) -> Result<[[i64; 2]; 2], SyscallError> {
    if tv.iter().any(|t| !(0..1_000_000).contains(&t[1])) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(tv.map(|t| [t[0], t[1] * 1_000]))
}

/// utime's `struct utimbuf` ([actime, modtime], whole seconds) as utimensat
/// times.
fn utimbuf_times(buf: [i64; 2]) -> [[i64; 2]; 2] {
    [[buf[0], 0], [buf[1], 0]]
}

/// utimensat(dirfd, path, times, flags) (N-249); with a NULL path it acts
/// on `dirfd` itself (futimens). NULL `times` sets both to now; two
/// UTIME_OMIT times return 0 without looking at the path, as Linux does.
pub fn sys_utimensat(
    dirfd: usize,
    path_ptr: usize,
    times_ptr: usize,
    flags: usize,
) -> SyscallResult {
    let specs = if times_ptr == 0 {
        None
    } else {
        Some(super::userspace::read_user::<[[i64; 2]; 2]>(times_ptr)?)
    };
    if specs.is_some_and(|[a, m]| a[1] == UTIME_OMIT && m[1] == UTIME_OMIT) {
        return Ok(0);
    }
    do_utimes(dirfd, path_ptr, specs, flags)
}

/// utimes(path, times): utimensat at the cwd with microsecond times.
pub fn sys_utimes(path_ptr: usize, times_ptr: usize) -> SyscallResult {
    sys_futimesat(AT_FDCWD, path_ptr, times_ptr)
}

/// futimesat(dirfd, path, times): utimensat with microsecond times and no
/// flags; a NULL path acts on `dirfd` (futimes).
pub fn sys_futimesat(dirfd: usize, path_ptr: usize, times_ptr: usize) -> SyscallResult {
    let specs = if times_ptr == 0 {
        None
    } else {
        Some(timeval_times(
            super::userspace::read_user::<[[i64; 2]; 2]>(times_ptr)?,
        )?)
    };
    do_utimes(dirfd, path_ptr, specs, 0)
}

/// utime(path, times): utimensat at the cwd with whole-second times.
pub fn sys_utime(path_ptr: usize, times_ptr: usize) -> SyscallResult {
    let specs = if times_ptr == 0 {
        None
    } else {
        Some(utimbuf_times(super::userspace::read_user::<[i64; 2]>(
            times_ptr,
        )?))
    };
    do_utimes(AT_FDCWD, path_ptr, specs, 0)
}

/// The common part of the four time-setting calls, Linux's do_utimes:
/// `specs` are utimensat times, or `None` for now. A NULL path acts on
/// `dirfd` and takes no flags. Setting explicit times needs the owner (or
/// root, EPERM otherwise); setting the current time also allows a caller
/// with write permission (EACCES).
fn do_utimes(
    dirfd: usize,
    path_ptr: usize,
    specs: Option<[[i64; 2]; 2]>,
    flags: usize,
) -> SyscallResult {
    let allowed = if path_ptr == 0 {
        0
    } else {
        AT_SYMLINK_NOFOLLOW
    };
    if flags & !allowed != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // The file, by descriptor or by path (which inotify reports to).
    let (node, file, path) = if path_ptr == 0 {
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let file = proc
            .file_table
            .lock()
            .get(dirfd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?;
        (file.node.clone(), Some(file), None)
    } else {
        let path = resolve_at_path(dirfd, &read_user_path(path_ptr)?)?;
        let vfs = vfs()?;
        let (node, canonical) = vfs
            .locate(&path, flags & AT_SYMLINK_NOFOLLOW == 0)
            .map_err(map_resolve_err)?;
        want_write(&vfs, &canonical)?;
        (node, None, Some(path))
    };
    // Linux's utimes_common: EROFS before the permission checks.
    if let Some(file) = &file {
        want_write_file(file)?;
    }
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
    // Both times are a utime (IN_ATTRIB), one alone an access or a
    // modification, as Linux's fsnotify_change.
    use crate::fs::inotify::{self, IN_ACCESS, IN_ATTRIB, IN_MODIFY};
    let mask = match (atime, mtime) {
        (Some(_), Some(_)) => IN_ATTRIB,
        (Some(_), None) => IN_ACCESS,
        _ => IN_MODIFY,
    };
    match file {
        Some(file) => inotify::on_file(&file, mask),
        None => inotify::on_target(path.as_deref(), &node, mask),
    }
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
        .get_path(fd as u32 as usize)
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
/// which inherit it). CAP_SYS_CHROOT in the caller's own user namespace
/// only (EPERM), as on Linux. The
/// working directory stays where it is if it is under the new
/// root; otherwise it becomes the new root, where Linux would leave it
/// outside (the classic way out of a chroot).
pub fn sys_chroot(path_ptr: usize) -> SyscallResult {
    let path = read_user_path(path_ptr)?;
    let caller = Caller::current();
    if !caller.ns_capable(&caller.userns, crate::process::linux_caps::CAP_SYS_CHROOT) {
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
    const FIONREAD: usize = 0x541B;
    const FIONBIO: usize = 0x5421;
    const FIONCLEX: usize = 0x5450;
    const FIOCLEX: usize = 0x5451;
    const FIOASYNC: usize = 0x5452;

    // The open file, or the implicit console of a standard descriptor
    // never replaced. Anything else is EBADF whatever the command, as
    // Linux's fdget comes first (N-198).
    let file = process::current_process().and_then(|p| p.file_table.lock().get(fd));
    if file.is_none() && !console_fallback_allowed(fd) {
        return Err(SyscallError::BadFileDescriptor);
    }

    // What every file answers, before its driver sees the command
    // (Linux's do_vfs_ioctl).
    match cmd {
        FIOCLEX | FIONCLEX => {
            if let Some(proc) = process::current_process() {
                // The implicit console has no entry to mark; Linux's
                // set_close_on_exec has no failure either.
                let _ = proc.file_table.lock().set_cloexec(fd, cmd == FIOCLEX);
            }
            return Ok(0);
        }
        FIONBIO => {
            let on: i32 = super::userspace::read_user(arg)?;
            if let Some(file) = &file {
                file.nonblock
                    .store(on != 0, core::sync::atomic::Ordering::Relaxed);
            }
            return Ok(0);
        }
        FIOASYNC => {
            let on: i32 = super::userspace::read_user(arg)?;
            // No file sends SIGIO yet, so none can turn it on: Linux's
            // ENOTTY for a file without fasync. Turning it off is a no-op.
            return if on != 0 {
                Err(SyscallError::NotATerminal)
            } else {
                Ok(0)
            };
        }
        FIONREAD => {
            // A regular file: size minus position, truncated to an int as
            // Linux's put_user does. Anything else: what its node queues.
            let count = match &file {
                Some(file) if file.node.node_type() == crate::fs::NodeType::File => {
                    let size = file.node.metadata().map_err(super::map_kernel_error)?.size;
                    (size as i64).wrapping_sub(*file.position.read() as i64) as i32
                }
                Some(file) => file
                    .node
                    .bytes_readable()
                    .ok_or(SyscallError::NotATerminal)? as i32,
                None => crate::drivers::console_input::bytes_readable() as i32,
            };
            super::userspace::write_user(arg, count)?;
            return Ok(0);
        }
        _ => {}
    }

    // PTY ends answer the window-size calls themselves (and ENOTTY for
    // the rest); None means the fd is not a PTY.
    if file.is_some() {
        if let Some(result) = crate::syscall::pty::handle_pty_ioctl(fd, cmd, arg) {
            return result;
        }
    }

    // A namespace file (ADR 0022).
    #[cfg(feature = "alloc")]
    if let Some(result) = file
        .as_ref()
        .and_then(|f| super::namespaces::nsfs_ioctl(f, cmd, arg))
    {
        return result;
    }

    // Device ioctls (DRM, evdev). The device is identified by the node's
    // (major, minor), never by the path it was opened with (W-7). Both run
    // against a kernel copy of the argument (W-4, W-19): the handlers never
    // see the user pointer.
    let device = file.as_ref().and_then(|file| file.node.device_id());
    match device {
        Some((crate::fs::devfs::DRM_MAJOR, _)) => {
            let result = crate::syscall::userspace::ioctl_bounce(cmd as u64, arg, |kernel_arg| {
                crate::graphics::drm_ioctl::drm_ioctl_dispatch(fd as i32, cmd as u64, kernel_arg)
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
            let result = crate::syscall::userspace::ioctl_bounce(cmd as u64, arg, |kernel_arg| {
                crate::drivers::evdev::handle_ioctl(minor, cmd as u32, kernel_arg)
            })?;
            return match result {
                Ok(v) => Ok(v as usize),
                Err(_) => Err(SyscallError::InvalidArgument),
            };
        }
        _ => {}
    }

    // Terminal ioctls go to the console only: the implicit console, or a
    // file whose node is /dev/console or /dev/tty0, whatever its fd number.
    // Deciding by number made fds 0-2 a terminal even when redirected to a
    // file, and an open of /dev/console above 2 not one (N-198). Everything
    // else, and any command the console does not know, is ENOTTY.
    if !is_console_terminal(device, file.is_none()) {
        return Err(SyscallError::NotATerminal);
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
            // In the caller's PID namespace (pid_vnr).
            let pgid = if let Some(proc) = process::current_process() {
                crate::process::pidns::vnr(proc.pgid.load(core::sync::atomic::Ordering::Acquire))
                    as i32
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
        _ => Err(SyscallError::NotATerminal),
    }
}

/// Whether an ioctl target is the console terminal: the implicit console
/// (`implicit`, a standard descriptor with no file), or a character device
/// numbered as /dev/console (5, 1) or /dev/tty0 (4, 0).
fn is_console_terminal(device: Option<(u32, u32)>, implicit: bool) -> bool {
    implicit || matches!(device, Some((5, 1) | (4, 0)))
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

    // A pid_t: its low 32 bits; -INT_MIN names no group (Linux's
    // kill_something_info). A pid or group as the caller's PID namespace
    // numbers it; one it does not name is ESRCH.
    let ns = caller.pid_ns();
    let kid = |nr: i32| crate::process::pidns::find(nr.unsigned_abs() as u64, &ns).unwrap_or(0);
    match pid as u32 as i32 {
        p if p > 0 => kill_one(&caller, crate::process::ProcessId(kid(p)), sig),
        i32::MIN => Err(SyscallError::ProcessNotFound),
        0 => kill_matching(&caller, sig, |p| {
            p.pgid.load(core::sync::atomic::Ordering::Relaxed)
                == caller.pgid.load(core::sync::atomic::Ordering::Relaxed)
        }),
        // Every process the caller's PID namespace numbers that it may
        // signal, except that namespace's init (1 there) and itself.
        -1 => kill_matching(&caller, sig, |p| {
            crate::process::pidns::nr_in(p.pid.0, &ns) > 1 && p.pid != caller.pid
        }),
        p => {
            let pgid = kid(p);
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
        &target.user_ns(),
        caller.pid == target.pid,
        sig,
        same_session,
    )
}

/// Linux's kill rule: a process may always signal itself; otherwise the
/// sender's real or effective uid must be the target's real or saved uid,
/// or the sender must hold CAP_KILL over the target's user namespace
/// `target_ns` (kill_ok_by_cred) -- except that SIGCONT may go to any
/// process in the sender's session.
fn kill_ok(
    caller: &Caller,
    target: &crate::process::creds::Credentials,
    target_ns: &crate::process::userns::UserNs,
    same_process: bool,
    sig: i32,
    same_session: bool,
) -> bool {
    let c = &caller.creds;
    same_process
        || [c.ruid, c.euid]
            .iter()
            .any(|&id| id == target.ruid || id == target.suid)
        || caller.ns_capable(target_ns, crate::process::linux_caps::CAP_KILL)
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
    if !may_signal(caller, &target, sig) {
        return Err(SyscallError::OperationNotPermitted);
    }
    // A zombie (exited, not yet reaped) takes the signal and discards it,
    // as on Linux, where it is still there to be signalled; only a reaped
    // process is ESRCH.
    if sig == 0 || !target.is_alive() {
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
    let info = crate::process::signals::SigInfo::from_sender(
        sig as usize,
        crate::process::signals::SI_USER,
        caller,
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
pub(super) fn terminates(process: &crate::process::Process, sig: i32) -> bool {
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
    signal_matching(caller, sig, None, pred)
}

/// As [`kill_matching`], each target seeing `info` when given (the caller
/// built or checked it) rather than kill's SI_USER from the caller.
pub(crate) fn signal_matching(
    caller: &crate::process::Process,
    sig: i32,
    info: Option<crate::process::signals::SigInfo>,
    pred: impl Fn(&crate::process::Process) -> bool,
) -> SyscallResult {
    let mut targets = alloc::vec::Vec::new();
    // Zombies too: they are still in their group (kill_one discards the
    // signal for them).
    crate::process::table::PROCESS_TABLE.for_each(|p| {
        if pred(p) {
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
        let done = match info {
            Some(info) if sig != 0 => kill_one(caller, pid, 0).and_then(|_| {
                let target =
                    crate::process::table::get_process(pid).ok_or(SyscallError::ProcessNotFound)?;
                super::signal::deliver(&target, None, info)
            }),
            _ => kill_one(caller, pid, sig),
        };
        if done.is_ok() {
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
    // A symlink's mode cannot change (Linux's notify_change, which
    // fchmodat2 with AT_SYMLINK_NOFOLLOW reaches), before any permission
    // check.
    if node.node_type() == crate::fs::NodeType::Symlink {
        return Err(SyscallError::NotSupported);
    }
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    let perms = chmod_perms(&Caller::current(), meta.uid, meta.gid, mode)?;
    node.chmod(perms).map_err(super::map_kernel_error)?;
    Ok(0)
}

/// The mode a chmod by `caller` stores on a file owned by `uid`:`gid`.
/// EPERM unless the caller owns the file or holds CAP_FOWNER over it; and
/// as Linux's setattr_copy, a caller outside the file's group without
/// CAP_FSETID over it cannot set the set-group-ID bit (it is dropped, not
/// refused).
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
    if !caller.in_group_or_capable(uid, gid) {
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

    let vfs = vfs()?;
    // Compared on the canonical parents of the resolution that produced the
    // nodes above: a string comparison let a symlink into another mount
    // through (review of PR #11), and resolving the paths again let them
    // describe other directories than the nodes (N-51). EXDEV, then EROFS,
    // come before the permission checks, as in Linux's do_renameat2.
    let old_mount = vfs.canonical_entry_mount_point(&old_parent_canon, &old_name);
    let new_mount = vfs.canonical_entry_mount_point(&new_parent_canon, &new_name);
    if old_mount != new_mount {
        return Err(SyscallError::CrossDevice);
    }
    want_write(&vfs, &old_parent_canon)?;
    check_may_remove(&*old_parent, &old_name)?;
    check_dir_write(&*new_parent)?;
    // Replacing an existing `new` removes it, so the sticky rule applies.
    if new_parent.lookup(&new_name).is_ok() {
        check_may_remove(&*new_parent, &new_name)?;
    }
    let src = old_parent.lookup(&old_name).map_err(map_resolve_err)?;
    // What `new` names now, which the rename replaces (for inotify).
    let replaced = crate::fs::inotify::active()
        .then(|| new_parent.lookup(&new_name).ok())
        .flatten();

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
        // Mounts are kept by path: moving a directory with mounts under it
        // would leave them at the old place, uncovering what they covered
        // and dropping their flags (read-only, noexec) from the moved tree.
        // Linux moves them with it; this refuses (EBUSY, as Linux does for
        // a mount point; docs/KNOWN-LIMITATIONS.md).
        if vfs.mounts_under(&src_canon) {
            return Err(SyscallError::Busy);
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
    // Two names of one file: Linux's vfs_rename does nothing, and reports
    // nothing.
    use crate::fs::write_access::node_key;
    if replaced
        .as_ref()
        .is_none_or(|node| node_key(node) != node_key(&src))
    {
        crate::fs::inotify::moved(
            &old_parent,
            &old_name,
            &new_parent,
            &new_name,
            &src,
            replaced.as_ref(),
        );
    }
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

/// Linux's mnt_want_write: EROFS when the mount serving canonical `path`
/// (in the whole tree) is read-only, by its own flag or its filesystem's
/// (ADR 0022 H3c).
pub(crate) fn want_write(
    view: &crate::fs::CallerView<'_>,
    canonical: &str,
) -> Result<(), SyscallError> {
    match view.mount_serving(canonical) {
        Some(mount) if mount.is_readonly() => Err(SyscallError::ReadOnlyFilesystem),
        _ => Ok(()),
    }
}

/// mnt_want_write_file: [`want_write`] for an open file, at the canonical
/// path it was opened at (pipes and sockets have none, and no mount).
pub(crate) fn want_write_file(file: &crate::fs::file::File) -> Result<(), SyscallError> {
    match file.path.as_deref() {
        Some(path) => want_write(&vfs()?, path),
        None => Ok(()),
    }
}

/// Whether the mount serving canonical `path` has mount flag `flag`
/// (`fs::mount::flags`).
pub(crate) fn mount_has(view: &crate::fs::CallerView<'_>, canonical: &str, flag: u32) -> bool {
    view.mount_serving(canonical).is_some_and(|m| m.has(flag))
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

/// Why reading a link failed, as an errno: EINVAL for a node that is not a
/// symlink, otherwise the link's own error (a `/proc/<pid>/ns` link with no
/// namespace behind it is ENOENT, one the caller may not read EACCES, as
/// Linux's proc_ns_readlink).
pub(crate) fn readlink_error(e: crate::error::KernelError) -> SyscallError {
    use crate::error::{FsError, KernelError};
    match e {
        KernelError::FsError(FsError::NotFound) => SyscallError::ResourceNotFound,
        KernelError::FsError(FsError::PermissionDenied) => SyscallError::PermissionDenied,
        _ => SyscallError::InvalidArgument,
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
///   `NotImplemented`, which is mapped to `InvalidArgument` here; a link's own
///   errors go through [`readlink_error`].
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
    let target = node.readlink().map_err(readlink_error)?;

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
        let root = caller.userns.root_uid();
        caller.caps = thread.caps.lock().access_effective(caller.creds.ruid, root);
    }
    caller
}

/// access(2): checked as [`access_caller`] (as the caller itself with
/// AT_EACCESS), owner, then group (any of the process's groups), then
/// other bits. It checked only the "other" bits (N-193).
fn access_path(path: &str, mode: usize, flags: usize) -> SyscallResult {
    if mode & !7 != 0 || flags & !(AT_EACCESS | AT_SYMLINK_NOFOLLOW) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let caller = if flags & AT_EACCESS != 0 {
        Caller::current()
    } else {
        access_caller()
    };
    // The whole lookup runs as that caller, not only the last component:
    // a directory the real user may not search is not searched for it.
    let vfs = try_get_vfs()
        .ok_or(SyscallError::InvalidState)?
        .as_caller_with(caller.clone());
    let (node, canonical) = vfs
        .locate(path, flags & AT_SYMLINK_NOFOLLOW == 0)
        .map_err(map_resolve_err)?;
    if mode == 0 {
        return Ok(0); // F_OK: it exists
    }
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    // The mount's flags, as Linux's do_faccessat: X_OK on a regular file
    // of a noexec mount is EACCES; W_OK on anything but a device, FIFO or
    // socket is EROFS on a read-only filesystem before the permission
    // check (sb_permission), on a read-only mount after it.
    use crate::fs::{mount::flags as mnt, NodeType};
    let mount = vfs.mount_serving(&canonical);
    let regular = meta.node_type == NodeType::File;
    if mode & 1 != 0 && regular && mount.as_ref().is_some_and(|m| m.has(mnt::NOEXEC)) {
        return Err(SyscallError::PermissionDenied);
    }
    let writes = mode & 2 != 0
        && matches!(
            meta.node_type,
            NodeType::File | NodeType::Directory | NodeType::Symlink
        );
    if writes && mount.as_ref().is_some_and(|m| m.sb_readonly()) {
        return Err(SyscallError::ReadOnlyFilesystem);
    }
    if !crate::fs::permission(&caller, &meta, mode as u32) {
        return Err(SyscallError::PermissionDenied);
    }
    if writes && mount.as_ref().is_some_and(|m| m.is_readonly()) {
        return Err(SyscallError::ReadOnlyFilesystem);
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

/// fcntl's record-lock commands (Linux's numbers on every architecture,
/// the 64-bit `struct flock` being the only one).
const F_GETLK: usize = 5;
const F_SETLK: usize = 6;
const F_SETLKW: usize = 7;
const F_OFD_GETLK: usize = 36;
const F_OFD_SETLK: usize = 37;
const F_OFD_SETLKW: usize = 38;

/// Linux's `struct flock` (the same on x86_64, AArch64 and RISC-V).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct UserFlock {
    l_type: i16,
    l_whence: i16,
    _pad0: u32,
    l_start: i64,
    l_len: i64,
    l_pid: i32,
    _pad1: u32,
}

// SAFETY: repr(C) integers only; every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for UserFlock {}

/// Linux's flock64_to_posix_lock: the inclusive byte range a `struct
/// flock` names, measured from the start, the file position or the end of
/// `file`. EINVAL for an unknown whence, a range starting before byte 0 or
/// an unknown type; EOVERFLOW for one ending past OFFSET_MAX.
fn flock_range(
    file: &crate::fs::file::File,
    fl: &UserFlock,
) -> Result<(u16, u64, u64), SyscallError> {
    use crate::fs::flock::{F_RDLCK, F_UNLCK, F_WRLCK, OFFSET_MAX};
    let max = OFFSET_MAX as i64;
    let base: usize = match fl.l_whence {
        0 => 0,
        1 => *file.position.read(),
        2 => file.node.metadata().map_err(super::map_kernel_error)?.size,
        _ => return Err(SyscallError::InvalidArgument),
    };
    // A base past OFFSET_MAX cannot name a range (lseek refuses one, but
    // a file's size is the filesystem's to report).
    let base = i64::try_from(base).map_err(|_| SyscallError::Overflow)?;
    let mut start = base
        .checked_add(fl.l_start)
        .filter(|&start| start <= max)
        .ok_or(SyscallError::Overflow)?;
    if start < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let end = if fl.l_len > 0 {
        if fl.l_len - 1 > max - start {
            return Err(SyscallError::Overflow);
        }
        start + (fl.l_len - 1)
    } else if fl.l_len < 0 {
        if start + fl.l_len < 0 {
            return Err(SyscallError::InvalidArgument);
        }
        let end = start - 1;
        start += fl.l_len;
        end
    } else {
        max
    };
    let kind = match fl.l_type as u16 {
        k @ (F_RDLCK | F_WRLCK | F_UNLCK) => k,
        _ => return Err(SyscallError::InvalidArgument),
    };
    Ok((kind, start as u64, end as u64))
}

/// fcntl's record-lock commands (N-199), as Linux's fcntl_getlk and
/// fcntl_setlk. A POSIX lock belongs to the caller's descriptor table and
/// goes when any of its descriptors of the file closes; an OFD lock
/// belongs to the open file and goes with its last close. F_SETLKW and
/// F_OFD_SETLKW sleep until the lock is granted (EINTR on a signal), and
/// a POSIX waiter that would close a cycle of waiters is EDEADLK.
fn fcntl_record_lock(fd: usize, cmd: usize, arg: usize) -> SyscallResult {
    use crate::fs::flock::{self, RangeError, RangeLock, F_RDLCK, F_UNLCK, F_WRLCK};
    let (file, owner, pid) = {
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let table = proc.file_table.lock();
        let file = table.get(fd).ok_or(SyscallError::BadFileDescriptor)?;
        (file, table.lock_owner(), proc.pid.0 as i32)
    };
    let mut fl: UserFlock = super::userspace::read_user(arg)?;
    let ofd = matches!(cmd, F_OFD_GETLK | F_OFD_SETLK | F_OFD_SETLKW);
    let key = file.flock_key();

    if matches!(cmd, F_GETLK | F_OFD_GETLK) {
        if !ofd && !matches!(fl.l_type as u16, F_RDLCK | F_WRLCK) {
            return Err(SyscallError::InvalidArgument);
        }
        let (kind, start, end) = flock_range(&file, &fl)?;
        if ofd && fl.l_pid != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        let query = RangeLock {
            kind,
            start,
            end,
            owner: if ofd { ofd_owner(&file) } else { owner },
            pid: 0,
            ofd,
        };
        match flock::test_lock(key, &query) {
            Some(held) => {
                fl.l_type = held.kind as i16;
                fl.l_whence = 0;
                fl.l_start = held.start as i64;
                fl.l_len = if held.end == flock::OFFSET_MAX {
                    0
                } else {
                    (held.end - held.start + 1) as i64
                };
                // As the caller's PID namespace numbers the holder, 0 if
                // it does not (locks_translate_pid); an OFD lock's -1 stays.
                fl.l_pid = if held.pid > 0 {
                    crate::process::pidns::vnr(held.pid as u64) as i32
                } else {
                    held.pid
                };
            }
            None => fl.l_type = F_UNLCK as i16,
        }
        super::userspace::write_user(arg, fl)?;
        return Ok(0);
    }

    let (kind, start, end) = flock_range(&file, &fl)?;
    // Linux's check_fmode_for_setlk.
    if (kind == F_RDLCK && !file.flags.read) || (kind == F_WRLCK && !file.flags.write) {
        return Err(SyscallError::BadFileDescriptor);
    }
    if ofd && fl.l_pid != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let req = RangeLock {
        kind,
        start,
        end,
        owner: if ofd { ofd_owner(&file) } else { owner },
        pid: if ofd { -1 } else { pid },
        ofd,
    };
    // Noted before an OFD lock is held, and never cleared, so the open
    // file's last close always looks (as for flock). A POSIX lock needs no
    // mark: every close asks the lock table.
    if kind != F_UNLCK && ofd {
        file.ofd_locked
            .store(true, core::sync::atomic::Ordering::Release);
    }
    let wait = matches!(cmd, F_SETLKW | F_OFD_SETLKW);
    let result = match flock::set_lock(key, &req, wait) {
        #[cfg(feature = "alloc")]
        Err(RangeError::Conflict) if wait && crate::sched::dispatch::current_owner().is_some() => {
            let mut outcome = Err(RangeError::Conflict);
            let waited = crate::sched::dispatch::wait_event(&flock::RANGE_WAITERS, None, || {
                outcome = flock::set_lock(key, &req, true);
                outcome != Err(RangeError::Conflict)
            });
            if waited.is_err() {
                flock::stop_waiting(req.owner);
                return Err(SyscallError::Interrupted);
            }
            outcome
        }
        other => other,
    };
    match result {
        Ok(()) => {}
        Err(RangeError::Conflict) => return Err(SyscallError::WouldBlock),
        Err(RangeError::Deadlock) => return Err(SyscallError::Deadlock),
    }
    // A close of the descriptor while the lock was being placed: the
    // close found nothing to release, so give it up now (Linux's
    // close/fcntl race check). OFD locks go with the open file instead.
    if kind != F_UNLCK && !ofd {
        let still = process::current_process().is_some_and(|proc| {
            proc.file_table
                .lock()
                .get(fd)
                .is_some_and(|f| alloc::sync::Arc::ptr_eq(&f, &file))
        });
        if !still {
            flock::release_owner(key, owner);
            return Err(SyscallError::BadFileDescriptor);
        }
    }
    Ok(0)
}

/// The owner of an open file's OFD locks: the open file itself.
fn ofd_owner(file: &alloc::sync::Arc<crate::fs::file::File>) -> u64 {
    alloc::sync::Arc::as_ptr(file) as u64
}

/// fcntl's O_LARGEFILE, which every open has on a 64-bit Linux kernel
/// (force_o_largefile), so F_GETFL reports it.
#[cfg(target_arch = "aarch64")]
const O_LARGEFILE: u32 = 0x2_0000;
#[cfg(not(target_arch = "aarch64"))]
const O_LARGEFILE: u32 = 0x8000;
/// Linux's O_NONBLOCK (and O_NDELAY), the same on every architecture.
const O_NONBLOCK_FL: u32 = 0x800;

/// F_GETFL: the access mode and status flags of an open file.
fn file_status_flags(file: &crate::fs::file::File) -> u32 {
    // An O_PATH open keeps none of the other flags, O_LARGEFILE included.
    if file.is_path() {
        return file.status.load(core::sync::atomic::Ordering::Acquire);
    }
    let access = match (file.flags.read, file.flags.write) {
        (true, true) => 2,
        (false, true) => 1,
        _ => 0,
    };
    let nonblock = if file.nonblock.load(core::sync::atomic::Ordering::Acquire) {
        O_NONBLOCK_FL
    } else {
        0
    };
    access | O_LARGEFILE | nonblock | file.status.load(core::sync::atomic::Ordering::Acquire)
}

/// F_SETFL, as Linux's setfl: O_APPEND, O_ASYNC, O_DIRECT, O_NOATIME and
/// O_NONBLOCK take `arg`'s value and every other bit of it is ignored.
/// Turning O_NOATIME on needs the file's owner or CAP_FOWNER
/// (`owner_ok`, asked with the file's uid; EPERM); O_DIRECT is EINVAL but
/// on a regular file (a pipe's packet mode does not exist).
fn set_file_status_flags(
    file: &crate::fs::file::File,
    arg: u32,
    owner_ok: impl FnOnce(u32) -> bool,
) -> Result<(), SyscallError> {
    use core::sync::atomic::Ordering;

    use crate::fs::file::status;
    let old = file.status.load(Ordering::Acquire);
    if arg & status::NOATIME != 0 && old & status::NOATIME == 0 {
        let meta = file.node.metadata().map_err(super::map_kernel_error)?;
        if !owner_ok(meta.uid) {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    if arg & status::DIRECT != 0 && file.node.node_type() != crate::fs::NodeType::File {
        return Err(SyscallError::InvalidArgument);
    }
    file.nonblock
        .store(arg & O_NONBLOCK_FL != 0, Ordering::Release);
    let _ = file
        .status
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            Some((arg & status::SETFL) | (old & !status::SETFL))
        });
    Ok(())
}

/// File descriptor control (fcntl).
///
/// The descriptor is looked up first, so any command on one that is not
/// open is EBADF, as Linux's do_fcntl; an unknown command on an open one
/// is EINVAL. A standard descriptor that is still the implicit console
/// answers the descriptor and status flag commands as a terminal open for
/// reading and writing would. An O_PATH descriptor takes only the
/// duplication, descriptor flag and F_GETFL commands (EBADF otherwise, as
/// Linux's check_fcntl_cmd).
pub fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> SyscallResult {
    const F_DUPFD: usize = 0;
    const F_GETFD: usize = 1;
    const F_SETFD: usize = 2;
    const F_GETFL: usize = 3;
    const F_SETFL: usize = 4;
    const F_DUPFD_QUERY: usize = 1027;
    const F_DUPFD_CLOEXEC: usize = 1030;
    const F_ADD_SEALS: usize = 1033;
    const F_GET_SEALS: usize = 1034;
    const FD_CLOEXEC: usize = 1;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let Some((file, cloexec)) = file_table.get_entry(fd) else {
        if !file_table.is_implicit_console(fd) {
            return Err(SyscallError::BadFileDescriptor);
        }
        return match cmd {
            F_GETFD => Ok(0),
            F_GETFL => Ok((2 | O_LARGEFILE) as usize),
            F_SETFD | F_SETFL => Ok(0),
            _ => Err(SyscallError::InvalidArgument),
        };
    };

    if file.is_path()
        && !matches!(
            cmd,
            F_DUPFD | F_DUPFD_CLOEXEC | F_DUPFD_QUERY | F_GETFD | F_SETFD | F_GETFL
        )
    {
        return Err(SyscallError::BadFileDescriptor);
    }

    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => file_table
            .dup_at_least(fd, arg, cmd == F_DUPFD_CLOEXEC)
            .map_err(super::map_kernel_error),
        F_GETFD => Ok(if cloexec { FD_CLOEXEC } else { 0 }),
        F_SETFD => file_table
            .set_cloexec(fd, arg & FD_CLOEXEC != 0)
            .map(|()| 0)
            .map_err(super::map_kernel_error),
        F_GETFL => Ok(file_status_flags(&file) as usize),
        F_SETFL => {
            drop(file_table);
            set_file_status_flags(&file, arg as u32, |uid| {
                Caller::current().owner_or_capable(uid)
            })
            .map(|()| 0)
        }
        // Whether `arg` is a descriptor of the same open file (Linux 6.10).
        F_DUPFD_QUERY => {
            let other = file_table
                .get_path(arg)
                .ok_or(SyscallError::BadFileDescriptor)?;
            Ok(usize::from(alloc::sync::Arc::ptr_eq(&other, &file)))
        }
        // Seals (N-229): only a memfd has them (EINVAL otherwise); adding
        // needs the fd open for writing (EPERM), as on Linux.
        F_GET_SEALS => file
            .node
            .seals()
            .map(|s| s as usize)
            .ok_or(SyscallError::InvalidArgument),
        F_ADD_SEALS => {
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
        F_GETLK | F_SETLK | F_SETLKW | F_OFD_GETLK | F_OFD_SETLK | F_OFD_SETLKW => {
            drop(file_table);
            fcntl_record_lock(fd, cmd, arg)
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

/// The most one read or write moves (Linux's MAX_RW_COUNT, INT_MAX rounded
/// down to a page).
const MAX_RW_COUNT: usize = 0x7fff_f000;

/// The `iovcnt` entries of the iovec array at `iov_ptr`, as (base, length),
/// checked as Linux's import_iovec: more than IOV_MAX entries, or a length
/// that is negative as an ssize_t, is EINVAL; an unreadable array is
/// EFAULT; and the lengths are cut so the total is at most MAX_RW_COUNT.
/// The buffers themselves are checked when they are copied.
pub(super) fn read_iovecs(
    iov_ptr: usize,
    iovcnt: usize,
) -> Result<alloc::vec::Vec<(usize, usize)>, SyscallError> {
    if iovcnt > IOV_MAX {
        return Err(SyscallError::InvalidArgument);
    }
    let mut iovs = alloc::vec::Vec::with_capacity(iovcnt);
    let mut total = 0usize;
    for i in 0..iovcnt {
        let iov: Iovec = super::userspace::read_user_index(iov_ptr, i)?;
        if (iov.iov_len as isize) < 0 {
            return Err(SyscallError::InvalidArgument);
        }
        let len = iov.iov_len.min(MAX_RW_COUNT - total);
        total += len;
        iovs.push((iov.iov_base, len));
    }
    Ok(iovs)
}

/// readv(fd, iov, iovcnt): read into the `iovcnt` buffers at `iov` as
/// one read (N-200): a pipe, socket or terminal is read once, so a readv
/// never waits again for a later buffer, as Linux's do_readv. The
/// descriptor is checked before the array (EBADF before EINVAL or
/// EFAULT), and every buffer's range before anything is read.
pub fn sys_readv(fd: usize, iov_ptr: usize, iovcnt: usize) -> SyscallResult {
    vectored_fd(fd, false)?;
    let iovs = read_iovecs(iov_ptr, iovcnt)?;
    read_segs(fd, UserSegs::new(&iovs))
}

/// writev(fd, iov, iovcnt): write the `iovcnt` buffers at `iov` as one
/// write (N-200), so one of at most PIPE_BUF bytes to a pipe goes in
/// whole or not at all; checked in readv's order.
pub fn sys_writev(fd: usize, iov_ptr: usize, iovcnt: usize) -> SyscallResult {
    writev(fd, iov_ptr, iovcnt, false)
}

/// writev, and pwritev2 at the file position: `nosignal` (RWF_NOSIGNAL)
/// keeps a pipe that loses its reader part way from raising SIGPIPE.
pub(super) fn writev(fd: usize, iov_ptr: usize, iovcnt: usize, nosignal: bool) -> SyscallResult {
    vectored_fd(fd, true)?;
    let iovs = read_iovecs(iov_ptr, iovcnt)?;
    write_segs(fd, UserSegs::new(&iovs), nosignal)
}

/// readv's and writev's descriptor check, made before the iovec array is
/// read (Linux's vfs_readv): no such descriptor, or one not open for
/// reading (`write`: writing), is EBADF. The implicit console of a process
/// without descriptors 0-2 passes, as it does for read and write.
fn vectored_fd(fd: usize, write: bool) -> Result<(), SyscallError> {
    match process::current_process().and_then(|p| p.file_table.lock().get(fd)) {
        Some(file)
            if (if write {
                file.flags.write
            } else {
                file.flags.read
            }) =>
        {
            Ok(())
        }
        Some(_) => Err(SyscallError::BadFileDescriptor),
        None if (if write { fd == 1 || fd == 2 } else { fd == 0 })
            && console_fallback_allowed(fd) =>
        {
            Ok(())
        }
        None => Err(SyscallError::BadFileDescriptor),
    }
}

// ============================================================================
// Self-hosting syscalls (Phase 4A: Tiers 1)
// ============================================================================

/// AT_FDCWD sentinel: use process current working directory.
/// Must match the C-side `#define AT_FDCWD (-100)` in syscall.h.
pub(super) const AT_FDCWD: usize = (-100isize) as usize;

/// unlinkat: remove a directory (rmdir).
const AT_REMOVEDIR: usize = 0x200;
/// Do not follow a final symlink.
pub(super) const AT_SYMLINK_NOFOLLOW: usize = 0x100;
/// An empty path names the dirfd itself.
pub(super) const AT_EMPTY_PATH: usize = 0x1000;

/// linkat: follow a final symlink of the old name.
pub(super) const AT_SYMLINK_FOLLOW: usize = 0x400;

/// link (syscall 155): linkat from the working directory, with no flags.
pub fn sys_link(old_ptr: usize, new_ptr: usize) -> SyscallResult {
    link_at(AT_FDCWD, old_ptr, AT_FDCWD, new_ptr, 0)
}

/// link and linkat (N-202), in the order of Linux's filename_linkat: flags
/// other than AT_SYMLINK_FOLLOW and AT_EMPTY_PATH are EINVAL; the old name
/// is looked up, its final symlink followed only with AT_SYMLINK_FOLLOW,
/// and with AT_EMPTY_PATH an empty one names `olddirfd` itself (the
/// working directory for AT_FDCWD), which needs CAP_DAC_READ_SEARCH or
/// the credentials the descriptor was opened with (`File::opener`; ENOENT
/// otherwise), as Linux 6.10's LOOKUP_LINKAT_EMPTY. Then the new name is
/// made as [`new_entry`] does; then a file with no name left (unless an
/// O_TMPFILE file made to be named) is ENOENT, a link into another
/// filesystem EXDEV, and one to a directory, or on a filesystem without
/// hard links, EPERM.
pub(crate) fn link_at(
    olddirfd: usize,
    old_ptr: usize,
    newdirfd: usize,
    new_ptr: usize,
    flags: usize,
) -> SyscallResult {
    if flags & !(AT_SYMLINK_FOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let old = if flags & AT_EMPTY_PATH != 0 {
        read_user_path_or_empty(old_ptr)?
    } else {
        read_user_path(old_ptr)?
    };
    let new = read_user_path(new_ptr)?;
    let vfs = vfs()?;
    // Where the old name is (canonical), for the mount check below; `None`
    // for a descriptor without a path.
    let mut old_canonical: Option<alloc::string::String> = None;
    let target = if old.is_empty() {
        if olddirfd as u32 as i32 == AT_FDCWD as u32 as i32 {
            let (node, canonical) = vfs
                .resolve_canonical(&vfs.get_cwd(), "/", true)
                .map_err(map_resolve_err)?;
            old_canonical = Some(canonical);
            node
        } else {
            let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
            let file = proc
                .file_table
                .lock()
                .get_path(olddirfd as u32 as usize)
                .ok_or(SyscallError::BadFileDescriptor)?;
            // Another's descriptor needs CAP_DAC_READ_SEARCH over the
            // opener's user namespace (filename_lookup's LINKAT_EMPTY).
            let caller = Caller::current();
            let opener_ns = file
                .opener
                .as_deref()
                .map_or_else(crate::process::userns::initial, |o| o.userns.clone());
            if file.opener.as_deref() != Some(&caller)
                && !caller.ns_capable(&opener_ns, crate::process::linux_caps::CAP_DAC_READ_SEARCH)
            {
                return Err(SyscallError::ResourceNotFound);
            }
            old_canonical = file.path.clone();
            file.node.clone()
        }
    } else {
        let abs = resolve_at_path(olddirfd, &old)?;
        let (node, canonical) = vfs
            .resolve_canonical(&abs, "/", flags & AT_SYMLINK_FOLLOW != 0)
            .map_err(map_resolve_err)?;
        old_canonical = Some(canonical);
        node
    };
    let (parent, name, parent_canonical) = new_entry_in(newdirfd, &new)?;
    // Both names on one mount, as Linux's do_linkat (old_path.mnt !=
    // new_path.mnt): two bind mounts of one filesystem are two mounts, and
    // a link across them would carry a file out of a read-only, noexec or
    // nosuid mount (PR #35 review). After the new name's checks, as there.
    // A descriptor with no path is on no mount here (a pipe, a socket: in
    // Linux an internal one such as pipefs), so it is EXDEV too, not let
    // through unchecked.
    let same_mount = old_canonical
        .as_deref()
        .is_some_and(|old| vfs.mount_point_of(old) == vfs.mount_point_of(&parent_canonical));
    if !same_mount {
        return Err(SyscallError::CrossDevice);
    }
    // A file whose last name is gone (open, through AT_EMPTY_PATH) gets
    // no new one, as Linux's vfs_link, unless it is an O_TMPFILE file made
    // to be named.
    if target.node_type() != crate::fs::NodeType::Directory
        && target.link_count() == 0
        && !target.tmpfile_linkable()
    {
        return Err(SyscallError::ResourceNotFound);
    }
    parent.link(&name, target.clone()).map_err(|e| match e {
        crate::error::KernelError::FsError(crate::error::FsError::IsADirectory)
        | crate::error::KernelError::NotImplemented { .. } => SyscallError::OperationNotPermitted,
        e => super::map_kernel_error(e),
    })?;
    crate::fs::inotify::linked(&parent, &name, &target);
    Ok(0)
}

/// symlink (syscall 156): symlinkat from the working directory.
pub fn sys_symlink(target_ptr: usize, link_ptr: usize) -> SyscallResult {
    symlink_at(target_ptr, AT_FDCWD, link_ptr)
}

/// symlink and symlinkat (N-202): an empty target is ENOENT, and the new
/// name is made as [`new_entry`] does; the filesystem's errors are its own
/// (they all used to be EINVAL).
pub(crate) fn symlink_at(target_ptr: usize, newdirfd: usize, link_ptr: usize) -> SyscallResult {
    let target = read_user_path(target_ptr)?;
    let link = read_user_path(link_ptr)?;
    let (parent, name) = new_entry(newdirfd, &link)?;
    let node = parent
        .symlink(&name, &target)
        .map_err(super::map_kernel_error)?;
    own_new_node(&node);
    crate::fs::inotify::created(&parent, &name, &node);
    Ok(0)
}

/// The directory and name of the entry link or symlink makes at `rel` from
/// `dirfd`, as Linux's filename_create: EEXIST if the name exists, even
/// with a trailing slash; ENOENT for a missing one with a trailing slash,
/// which only mkdir makes; then write and search permission on the
/// directory (EACCES).
fn new_entry(
    dirfd: usize,
    rel: &str,
) -> Result<
    (
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        alloc::string::String,
    ),
    SyscallError,
> {
    new_entry_in(dirfd, rel).map(|(parent, name, _)| (parent, name))
}

/// [`new_entry`], also returning the parent directory's canonical path.
fn new_entry_in(
    dirfd: usize,
    rel: &str,
) -> Result<
    (
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        alloc::string::String,
        alloc::string::String,
    ),
    SyscallError,
> {
    let abs = resolve_at_path(dirfd, rel)?;
    let name = abs.trim_end_matches('/');
    if name.is_empty() {
        // "/": the root exists.
        return Err(SyscallError::FileExists);
    }
    match vfs()?.resolve_path_no_follow(name) {
        Ok(_) => return Err(SyscallError::FileExists),
        Err(e) if is_not_found(&e) => {}
        Err(e) => return Err(map_resolve_err(e)),
    }
    if name.len() != abs.len() {
        return Err(SyscallError::ResourceNotFound);
    }
    let (parent, name, canonical) = parent_dir_canonical(name)?;
    want_write(&vfs()?, &canonical)?;
    check_dir_write(&*parent)?;
    Ok((parent, name, canonical))
}

/// Change file permissions by path (syscall 185).
pub fn sys_chmod(path_ptr: usize, mode: usize) -> SyscallResult {
    let path = resolve_at_path(AT_FDCWD, &read_user_path(path_ptr)?)?;
    let vfs = vfs()?;
    let (node, canonical) = vfs.locate(&path, true).map_err(map_resolve_err)?;
    want_write(&vfs, &canonical)?;
    chmod_node(&node, mode)?;
    crate::fs::inotify::on_path(&path, &node, crate::fs::inotify::IN_ATTRIB);
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
    want_write_file(&file)?;
    chmod_node(&file.node, mode)?;
    crate::fs::inotify::on_file(&file, crate::fs::inotify::IN_ATTRIB);
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

/// truncate: set the length of the regular file at `path` (EISDIR for a
/// directory, EINVAL for anything else), which needs write permission on
/// it; a negative length is EINVAL before the lookup (N-191).
pub fn sys_truncate(path_ptr: usize, length: usize) -> SyscallResult {
    let length = file_offset(length)?;
    let path = resolve_at_path(AT_FDCWD, &read_user_path(path_ptr)?)?;
    let vfs = vfs()?;
    let (node, canonical) = vfs.locate(&path, true).map_err(map_resolve_err)?;
    match node.node_type() {
        crate::fs::NodeType::File => {}
        crate::fs::NodeType::Directory => return Err(SyscallError::IsADirectory),
        _ => return Err(SyscallError::InvalidArgument),
    }
    // Linux's vfs_truncate: EROFS before the permission check.
    want_write(&vfs, &canonical)?;
    // As opening it for writing would need (N-191).
    require_open_access(&node, &OpenFlags::write_only())?;
    // Write access for the truncate: a running program is ETXTBSY (Linux's
    // vfs_truncate).
    let _writing =
        crate::fs::write_access::WriteAccess::get(&node).map_err(super::map_kernel_error)?;
    crate::fs::check_fsize_truncate(&*node, length).map_err(super::map_kernel_error)?;
    crate::fs::truncate_removing_privs(&*node, length).map_err(super::map_kernel_error)?;
    crate::fs::inotify::on_path(&path, &node, crate::fs::inotify::IN_MODIFY);
    Ok(0)
}

/// The file execveat runs and its names (Linux's do_open_execat and
/// alloc_bprm): `name` from `dirfd` (EBADF, ENOTDIR), the final symlink
/// not followed with `nofollow` (ELOOP), or with an empty name the
/// descriptor itself (AT_EMPTY_PATH; the caller checked the flag) or, for
/// AT_FDCWD, the working directory. A name relative to a descriptor is
/// `/dev/fd/N/name`, an empty one `/dev/fd/N`; such a name is inaccessible
/// after the exec when the descriptor is close-on-exec.
pub(crate) fn exec_file_at(
    dirfd: usize,
    name: &str,
    nofollow: bool,
) -> Result<crate::process::creation::ExecFile, SyscallError> {
    use alloc::{format, string::String};
    let dfd = dirfd as u32 as i32;
    let at_cwd = dfd == AT_FDCWD as u32 as i32;
    let vfs = vfs()?;
    let root = vfs.get_root();
    let entry = || -> Result<(alloc::sync::Arc<crate::fs::file::File>, bool), SyscallError> {
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let entry = proc.file_table.lock().get_entry(dfd as u32 as usize);
        entry.ok_or(SyscallError::BadFileDescriptor)
    };
    let base = |p: &str| String::from(p.rsplit('/').next().unwrap_or(""));

    if name.is_empty() && !at_cwd {
        let (file, cloexec) = entry()?;
        let exe_path = file
            .path
            .as_deref()
            .map(|p| crate::fs::root_relative(p, &root))
            .unwrap_or_default();
        let mnt_flags = file
            .path
            .as_deref()
            .map_or(0, |p| crate::process::creation::mount_flags(&vfs, p));
        return Ok(crate::process::creation::ExecFile {
            node: file.node.clone(),
            filename: format!("/dev/fd/{}", dfd),
            comm: base(&exe_path),
            exe_path,
            path_inaccessible: cloexec,
            mnt_flags,
        });
    }
    let lookup = if name.is_empty() {
        String::from(".")
    } else {
        resolve_at_path(dirfd, name)?
    };
    let (node, canonical) = vfs
        .resolve_canonical(&lookup, &vfs.get_cwd(), !nofollow || name.ends_with('/'))
        .map_err(map_resolve_err)?;
    if name.ends_with('/') && node.node_type() != crate::fs::NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    if node.node_type() == crate::fs::NodeType::Symlink {
        return Err(SyscallError::SymlinkLoop);
    }
    let from_fd = !name.starts_with('/') && !at_cwd;
    let filename = if from_fd {
        format!("/dev/fd/{}/{}", dfd, name)
    } else {
        String::from(name)
    };
    Ok(crate::process::creation::ExecFile {
        node,
        comm: base(&filename),
        exe_path: crate::fs::root_relative(&canonical, &root),
        filename,
        path_inaccessible: from_fd && entry().is_ok_and(|(_, cloexec)| cloexec),
        mnt_flags: crate::process::creation::mount_flags(&vfs, &canonical),
    })
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
            .get_path(dirfd as u32 as usize)
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
    let rel_path = read_open_path(path_ptr, flags)?;
    if rel_path.is_empty() {
        return open_empty_path(dirfd, flags, mode, false);
    }
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

    open_path(&vfs()?, &abs_path, flags, mode, false)
}

/// An open's path argument: empty only with O_EMPTYPATH (ENOENT otherwise).
fn read_open_path(ptr: usize, flags: usize) -> Result<alloc::string::String, SyscallError> {
    if flags as u64 & O_EMPTYPATH != 0 {
        read_user_path_or_empty(ptr)
    } else {
        read_user_path(ptr)
    }
}

/// open, openat and openat2 with O_EMPTYPATH and an empty path (Linux's
/// LOOKUP_EMPTY): the file `dirfd` names, O_PATH descriptors included (the
/// working directory for AT_FDCWD), is opened again with the checks of an
/// open by name, a symlink not followed; with O_TMPFILE the file is made in
/// it. A descriptor without a recorded path (a pipe or socket) cannot be
/// reopened (ENXIO, as Linux for a socket).
fn open_empty_path(dirfd: usize, flags: usize, mode: usize, regular: bool) -> SyscallResult {
    let flags = open_how_flags(flags);
    check_open_flags(flags)?;
    let (node, canonical) = if dirfd as u32 as i32 == AT_FDCWD as u32 as i32 {
        vfs()?
            .resolve_path_canonical(".")
            .map_err(map_resolve_err)?
    } else {
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let file = proc
            .file_table
            .lock()
            .get_path(dirfd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?;
        let canonical = file.path.clone().ok_or(SyscallError::NoDeviceOrAddress)?;
        (file.node.clone(), canonical)
    };
    // The MAC check of an open by name (`Vfs::open_in`), for the access now
    // asked: an O_PATH descriptor was checked for reading only, and must
    // not reopen for writing past it. O_TMPFILE writes the directory.
    let write = OpenFlags::from_bits(flags as u32)
        .ok_or(SyscallError::InvalidArgument)?
        .write
        || flags as u64 & O_TMPFILE_BIT != 0;
    let access = if write {
        crate::security::AccessType::Write
    } else {
        crate::security::AccessType::Read
    };
    let pid = process::current_process().map_or(0, |p| p.pid.0);
    crate::security::mac::check_file_access(&canonical, access, pid)
        .map_err(super::map_kernel_error)?;
    if flags as u64 & O_TMPFILE_BIT != 0 {
        return open_tmpfile(node, &canonical, flags, mode);
    }
    open_node(node, false, canonical, flags, regular, false)
}

/// openat2's `struct open_how` (its first version, the only one).
const OPEN_HOW_SIZE_VER0: usize = 24;
/// openat2's own flag, outside open(2)'s 32 bits.
const OPENAT2_REGULAR: u64 = 1 << 32;

/// The open flags open(2) knows (Linux's VALID_OPEN_FLAGS, O_EMPTYPATH
/// included). arm64 numbers four of them differently, the same four bits.
const VALID_OPEN_FLAGS: u64 = 0x007f_ffc3 | O_EMPTYPATH;
const O_EMPTYPATH: u64 = 1 << 26;
const O_CREAT: u64 = 0x40;
const O_TRUNC: u64 = 0x200;
const O_CLOEXEC: u64 = 0x8_0000;
const O_PATH: u64 = 0x20_0000;
const O_TMPFILE_BIT: u64 = 0x40_0000;
#[cfg(target_arch = "aarch64")]
const O_DIRECTORY_64: u64 = 0x4000;
#[cfg(target_arch = "aarch64")]
const O_NOFOLLOW_64: u64 = 0x8000;
#[cfg(not(target_arch = "aarch64"))]
const O_DIRECTORY_64: u64 = 0x1_0000;
#[cfg(not(target_arch = "aarch64"))]
const O_NOFOLLOW_64: u64 = 0x2_0000;

/// openat2's checks of `how`, in Linux's order (build_open_flags):
/// unknown flags or resolve bits, both scopes, a mode without a create
/// (or a wider one with it), O_DIRECTORY with O_CREAT, O_TMPFILE without
/// O_DIRECTORY or write access, OPENAT2_REGULAR with O_DIRECTORY, O_PATH
/// with other flags (EINVAL), RESOLVE_CACHED with a create or truncate
/// (EAGAIN).
fn check_open_how(flags: u64, mode: u64, resolve: u64) -> Result<(), SyscallError> {
    use crate::fs::resolve_flags as r;
    let flags = flags & !O_CLOEXEC;
    if flags & !(VALID_OPEN_FLAGS | OPENAT2_REGULAR) != 0 || resolve & !u64::from(r::ALL) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if resolve & u64::from(r::SCOPED) == u64::from(r::SCOPED) {
        return Err(SyscallError::InvalidArgument);
    }
    let will_create = flags & (O_CREAT | O_TMPFILE_BIT) != 0;
    if (will_create && mode & !0o7777 != 0) || (!will_create && mode != 0) {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & (O_DIRECTORY_64 | O_CREAT) == O_DIRECTORY_64 | O_CREAT {
        return Err(SyscallError::InvalidArgument);
    }
    // ACC_MODE: access modes 1, 2 and 3 all ask for write.
    if flags & O_TMPFILE_BIT != 0 && (flags & O_DIRECTORY_64 == 0 || flags & 3 == 0) {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & (O_DIRECTORY_64 | OPENAT2_REGULAR) == O_DIRECTORY_64 | OPENAT2_REGULAR {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & O_PATH != 0 && flags & !(O_DIRECTORY_64 | O_NOFOLLOW_64 | O_PATH | O_EMPTYPATH) != 0
    {
        return Err(SyscallError::InvalidArgument);
    }
    if resolve & u64::from(r::CACHED) != 0 && flags & (O_TRUNC | O_CREAT | O_TMPFILE_BIT) != 0 {
        return Err(SyscallError::WouldBlock);
    }
    Ok(())
}

/// openat2(dirfd, path, how, size) (Linux 7.3's fs/open.c): `how` is read
/// as copy_struct_from_user reads an extensible struct (EINVAL below its
/// first version, E2BIG above a page or for nonzero bytes past it,
/// EFAULT), checked (`check_open_how`), then the name (EFAULT, ENOENT for
/// an empty one) and the directory (EBADF, ENOTDIR). The open is openat's,
/// with `how.resolve` applied to the walk (`fs::resolve_flags`).
pub fn sys_openat2(dirfd: usize, path_ptr: usize, how: usize, size: usize) -> SyscallResult {
    use crate::fs::resolve_flags as r;
    if size < OPEN_HOW_SIZE_VER0 {
        return Err(SyscallError::InvalidArgument);
    }
    if size > 4096 {
        return Err(SyscallError::ArgumentListTooLong);
    }
    // The bytes past the struct first, as check_zeroed_user.
    if size > OPEN_HOW_SIZE_VER0 {
        let mut tail = alloc::vec![0u8; size - OPEN_HOW_SIZE_VER0];
        let past = how
            .checked_add(OPEN_HOW_SIZE_VER0)
            .ok_or(SyscallError::InvalidPointer)?;
        super::userspace::read_user_bytes(past, &mut tail)?;
        if tail.iter().any(|&b| b != 0) {
            return Err(SyscallError::ArgumentListTooLong);
        }
    }
    let mut raw = [0u8; OPEN_HOW_SIZE_VER0];
    super::userspace::read_user_bytes(how, &mut raw)?;
    let word = |at: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&raw[at..at + 8]);
        u64::from_ne_bytes(b)
    };
    let (flags, mode, resolve) = (word(0), word(8), word(16));
    check_open_how(flags, mode, resolve)?;
    // The checks passed: every resolve bit is known.
    let resolve = resolve as u32 & r::ALL;
    let regular = flags & OPENAT2_REGULAR != 0;
    let flags = (flags & VALID_OPEN_FLAGS) as usize;

    let path = read_open_path(path_ptr, flags)?;
    if path.is_empty() {
        return open_empty_path(dirfd, flags, mode as usize, regular);
    }
    if resolve == 0 {
        let abs_path = resolve_at_path(dirfd, &path)?;
        return open_path(&vfs()?, &abs_path, flags, mode as usize, regular);
    }
    // The walk starts at the directory; an absolute path that is not
    // RESOLVE_IN_ROOT's starts at the root without looking at it.
    let view = vfs()?;
    let start = if (path.starts_with('/') && resolve & r::IN_ROOT == 0)
        || dirfd as u32 as i32 == AT_FDCWD as u32 as i32
    {
        view.get_cwd()
    } else {
        let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
        let file = proc
            .file_table
            .lock()
            .get_path(dirfd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?;
        fd_directory_path(&file)?
    };
    let view = view.resolving(resolve, &start);
    open_path(&view, &path, flags, mode as usize, regular)
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
    // The node and its canonical path, for the mount (st_dev).
    let (node, canonical) = if rel_path.is_empty() {
        if dirfd as u32 as i32 == AT_FDCWD as u32 as i32 {
            let (node, canonical) = vfs.resolve_path_canonical(".").map_err(map_resolve_err)?;
            (node, Some(canonical))
        } else {
            let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
            let file = proc
                .file_table
                .lock()
                .get_path(dirfd as u32 as usize)
                .ok_or(SyscallError::BadFileDescriptor)?;
            (file.node.clone(), file.path.clone())
        }
    } else {
        // A trailing slash is followed and needs a directory (resolve_in).
        let abs_path = resolve_at_path(dirfd, &rel_path)?;
        let follow = flags & AT_SYMLINK_NOFOLLOW == 0;
        let (node, canonical) = vfs
            .resolve_canonical(&abs_path, &vfs.get_cwd(), follow)
            .map_err(map_resolve_err)?;
        (node, Some(canonical))
    };

    let metadata = node.metadata().map_err(super::map_kernel_error)?;
    let (mount, _) = mount_of(canonical.as_deref())?;
    super::userspace::write_user(stat_buf, fill_stat(&metadata, &*node, mount))?;
    Ok(0)
}

/// statx(dirfd, path, flags, mask, buf), as Linux's (fs/stat.c). With
/// AT_EMPTY_PATH a NULL or empty path names `dirfd` itself -- an open
/// descriptor of any kind, without looking at the other flags, as
/// do_statx_fd -- or with AT_FDCWD the working directory. A reserved mask
/// bit or both AT_STATX_SYNC_TYPE bits is EINVAL, as is an unknown flag
/// for a lookup; either sync type alone is accepted, every filesystem
/// being local. The path is read first, so a bad one is EFAULT before any
/// of those.
pub fn sys_statx(
    dirfd: usize,
    path_ptr: usize,
    flags: usize,
    mask: usize,
    buf: usize,
) -> SyscallResult {
    const AT_NO_AUTOMOUNT: usize = 0x800;
    let flags = flags as u32 as usize;
    let mask = mask as u32;
    let name = if flags & AT_EMPTY_PATH == 0 {
        Some(read_user_path(path_ptr)?)
    } else if path_ptr == 0 {
        None
    } else {
        Some(read_user_path_or_empty(path_ptr)?).filter(|p| !p.is_empty())
    };
    let dfd = dirfd as u32 as i32;
    let bad_request =
        mask & STATX_RESERVED != 0 || flags & AT_STATX_SYNC_TYPE == AT_STATX_SYNC_TYPE;
    let vfs = vfs()?;
    let (node, canonical) = match name {
        None if dfd >= 0 => {
            if bad_request {
                return Err(SyscallError::InvalidArgument);
            }
            let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
            let file = proc
                .file_table
                .lock()
                .get_path(dfd as usize)
                .ok_or(SyscallError::BadFileDescriptor)?;
            (file.node.clone(), file.path.clone())
        }
        name => {
            if bad_request
                || flags
                    & !(AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT | AT_EMPTY_PATH | AT_STATX_SYNC_TYPE)
                    != 0
            {
                return Err(SyscallError::InvalidArgument);
            }
            let (node, canonical) = match name {
                None if dfd != AT_FDCWD as u32 as i32 => {
                    return Err(SyscallError::BadFileDescriptor)
                }
                None => vfs.resolve_path_canonical(".").map_err(map_resolve_err)?,
                Some(rel_path) => {
                    let abs_path = resolve_at_path(dirfd, &rel_path)?;
                    let trailing_slash = rel_path.ends_with('/');
                    let follow = flags & AT_SYMLINK_NOFOLLOW == 0 || trailing_slash;
                    let found = vfs
                        .resolve_canonical(&abs_path, &vfs.get_cwd(), follow)
                        .map_err(map_resolve_err)?;
                    if trailing_slash && found.0.node_type() != crate::fs::NodeType::Directory {
                        return Err(SyscallError::NotADirectory);
                    }
                    found
                }
            };
            (node, Some(canonical))
        }
    };

    let metadata = node.metadata().map_err(super::map_kernel_error)?;
    let (mount, mount_root) = mount_of(canonical.as_deref())?;
    let rdev = node.device_id().unwrap_or((0, 0));
    let statx = fill_statx(&metadata, rdev, mount, mount_root, mask, node.link_count());
    super::userspace::write_user(buf, statx)?;
    Ok(0)
}

/// Copy what statfs reports about `node`'s filesystem to `buf` (`struct
/// statfs`, `fs::statfs::LINUX_STATFS_SIZE` bytes). A filesystem that
/// cannot say is ENOSYS, as on Linux.
fn write_statfs(node: &dyn crate::fs::VfsNode, at: Option<&str>, buf: usize) -> SyscallResult {
    let mut stats = node.statfs().map_err(|e| match e {
        crate::error::KernelError::NotImplemented { .. } => SyscallError::NotImplemented,
        e => super::map_kernel_error(e),
    })?;
    // The flags of the mount `at` (a canonical path) is on, and its
    // superblock's read-only state, as Linux's calculate_f_flags.
    if let Some(mount) = at.and_then(|path| vfs().ok()?.mount_serving(path)) {
        stats.flags |= crate::fs::statfs::mount_flags(mount.flags(), mount.sb_readonly());
    }
    super::userspace::write_user_bytes(buf, &stats.to_linux())?;
    Ok(0)
}

/// statfs(path, buf): the filesystem holding `path`, following symlinks,
/// as Linux's `user_statfs` (`fs/statfs.c`). A trailing slash needs a
/// directory (ENOTDIR), as for stat.
pub fn sys_statfs(path_ptr: usize, buf: usize) -> SyscallResult {
    let rel_path = read_user_path(path_ptr)?;
    let abs_path = resolve_at_path(AT_FDCWD, &rel_path)?;
    let (node, canonical) = vfs()?.locate(&abs_path, true).map_err(map_resolve_err)?;
    if rel_path.ends_with('/') && node.node_type() != crate::fs::NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    write_statfs(&*node, Some(&canonical), buf)
}

/// fstatfs(fd, buf): the filesystem holding the open file `fd`; for a
/// pipe, socket or anonymous-inode file, its pseudo filesystem.
pub fn sys_fstatfs(fd: usize, buf: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = proc
        .file_table
        .lock()
        .get_path(fd as u32 as usize)
        .ok_or(SyscallError::BadFileDescriptor)?;
    write_statfs(&*file.node, file.path.as_deref(), buf)
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
    // The directory first: a read-only mount is EROFS whether or not the
    // name exists (Linux's do_unlinkat and do_rmdir).
    let (dir, name, dir_canonical) = parent_dir_canonical(&abs_path)?;
    let vfs = vfs()?;
    want_write(&vfs, &dir_canonical)?;
    let node = vfs
        .resolve_path_no_follow(&abs_path)
        .map_err(map_resolve_err)?;
    let is_dir = node.node_type() == crate::fs::NodeType::Directory;
    match (remove_dir, is_dir) {
        (true, false) => return Err(SyscallError::NotADirectory),
        (false, true) => return Err(SyscallError::IsADirectory),
        (false, false) if trailing_slash => return Err(SyscallError::NotADirectory),
        _ => {}
    }
    check_may_remove(&*dir, &name)?;
    let victim = crate::fs::inotify::active()
        .then(|| dir.lookup(&name).ok())
        .flatten();
    // The type checked above is checked again where the removal happens.
    dir.unlink_kind(&name, remove_dir)
        .map_err(super::map_kernel_error)?;
    if let Some(victim) = victim {
        crate::fs::inotify::removed(&dir, &name, &victim);
    }
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
pub(super) fn positioned_file(
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
    pread_file(&file, buf, count, offset)
}

/// Read `count` bytes at `offset` of `file` (open for reading, with an
/// offset) into `buf`, leaving its position alone (pread).
fn pread_file(
    file: &crate::fs::file::File,
    buf: usize,
    count: usize,
    offset: usize,
) -> SyscallResult {
    if file.node.node_type() == crate::fs::NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;
    pread_segs(file, UserSegs::new(&[(buf, count)]), offset)
}

/// Read at `offset` of `file` into `dst`, leaving its position alone:
/// pread, and preadv as one read.
pub(super) fn pread_segs(
    file: &crate::fs::file::File,
    dst: UserSegs<'_>,
    offset: usize,
) -> SyscallResult {
    if file.node.node_type() == crate::fs::NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    // Read at `offset` through the VfsNode, bypassing the File position,
    // a chunk at a time through a kernel buffer (N-43).
    let mut at = offset;
    let read = super::userspace::produce_to_segs(
        dst,
        true,
        |kbuf| {
            let n = file.read_node(at, kbuf).map_err(super::map_kernel_error)?;
            at += n;
            Ok(n)
        },
        |_| {},
    )?; // no file position to give bytes back to
    if read > 0 {
        crate::fs::inotify::on_file(file, crate::fs::inotify::IN_ACCESS);
    }
    Ok(read)
}

/// Write to a file descriptor at a given offset without changing position
/// (syscall 196). As on Linux, a file opened with O_APPEND is appended to
/// whatever the offset (musl's pwrite asks for POSIX's behaviour through
/// pwritev2 with RWF_NOAPPEND instead).
pub fn sys_pwrite(fd: usize, buf: usize, count: usize, offset: usize) -> SyscallResult {
    let file = positioned_file(fd, offset)?;
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let offset = if file.is_append() {
        file_size(&file)?
    } else {
        offset
    };
    pwrite_file(&file, buf, count, offset)
}

/// The size of `file`'s node: where an appending write starts.
pub(super) fn file_size(file: &crate::fs::file::File) -> Result<usize, SyscallError> {
    Ok(file.node.metadata().map_err(super::map_kernel_error)?.size)
}

/// Write `count` bytes from `buf` at `offset` of `file` (open for writing,
/// with an offset), leaving its position alone (pwrite).
fn pwrite_file(
    file: &crate::fs::file::File,
    buf: usize,
    count: usize,
    offset: usize,
) -> SyscallResult {
    if count == 0 {
        return Ok(0);
    }
    validate_user_buffer(buf, count)?;
    pwrite_segs(file, UserSegs::new(&[(buf, count)]), offset)
}

/// Write `src` at `offset` of `file`, leaving its position alone: pwrite,
/// and pwritev as one write.
pub(super) fn pwrite_segs(
    file: &crate::fs::file::File,
    src: UserSegs<'_>,
    offset: usize,
) -> SyscallResult {
    if src.is_empty() {
        return Ok(0);
    }
    // RLIMIT_FSIZE for the whole write (only one starting at the limit is
    // refused, EFBIG and SIGXFSZ), then at `offset` through the VfsNode, a
    // chunk at a time (N-43).
    let count =
        crate::fs::check_fsize(&*file.node, offset, src.len()).map_err(super::map_kernel_error)?;
    if count != 0 {
        crate::fs::remove_privs(&*file.node).map_err(super::map_kernel_error)?;
    }
    let mut at = offset;
    let written = super::userspace::consume_from_segs(src.truncate(count), true, |kbuf| {
        let n = file.write_node(at, kbuf).map_err(super::map_kernel_error)?;
        at += n;
        Ok(n)
    })?;
    if written > 0 {
        crate::fs::inotify::on_file(file, crate::fs::inotify::IN_MODIFY);
    }
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
/// chgrp_ok with _POSIX_CHOWN_RESTRICTED: CAP_CHOWN over the file
/// (capable_wrt_inode_uidgid), or in the initial user namespace, may do
/// anything; the owner may keep the owner and set the group to one it
/// belongs to.
pub(crate) fn may_chown(
    caller: &Caller,
    file_uid: u32,
    file_gid: u32,
    uid: Option<u32>,
    gid: Option<u32>,
) -> bool {
    use crate::process::linux_caps::CAP_CHOWN;
    if caller.capable_wrt_inode(file_uid, file_gid, CAP_CHOWN) || caller.capable(CAP_CHOWN) {
        return true;
    }
    let creds = &caller.creds;
    let owner = creds.fsuid == file_uid;
    let uid_ok = uid.is_none_or(|u| owner && u == file_uid);
    let gid_ok = gid.is_none_or(|g| owner && (g == file_gid || creds.in_group(g)));
    uid_ok && gid_ok
}

/// Apply a chown to `node` ([`may_chown`], EPERM otherwise; FS-SEC-02).
/// This used to be a no-op that reported success. The IDs are the
/// caller's user namespace's: one it does not map EINVAL (chown_common's
/// make_kuid).
pub(crate) fn chown_node(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    uid: usize,
    gid: usize,
) -> SyscallResult {
    let ns = crate::process::userns::current();
    let uid = chown_id(uid)
        .map(|u| ns.uid_down(u).ok_or(SyscallError::InvalidArgument))
        .transpose()?;
    let gid = chown_id(gid)
        .map(|g| ns.gid_down(g).ok_or(SyscallError::InvalidArgument))
        .transpose()?;
    if uid.is_none() && gid.is_none() {
        return Ok(0);
    }
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if !may_chown(&Caller::current(), meta.uid, meta.gid, uid, gid) {
        return Err(SyscallError::OperationNotPermitted);
    }
    // A non-directory that changes hands loses set-user-ID and
    // set-group-ID (`Permissions::after_chown`) and its file capabilities.
    // As Linux's setattr_prepare, they go first: when that fails the file
    // keeps its owner, never a new owner with the old privileges (PR #33
    // review). An owner the filesystem cannot store is refused before
    // anything changes.
    node.check_owner(uid, gid)
        .map_err(super::map_kernel_error)?;
    if meta.node_type != crate::fs::NodeType::Directory {
        crate::fs::kill_file_caps(&**node).map_err(super::map_kernel_error)?;
        if let Some(kept) = meta.permissions.after_chown() {
            node.chmod(kept).map_err(super::map_kernel_error)?;
        }
    }
    node.chown(uid, gid).map_err(super::map_kernel_error)?;
    Ok(0)
}

/// Change ownership of a file by path (syscall 197).
pub fn sys_chown(path_ptr: usize, uid: usize, gid: usize) -> SyscallResult {
    let path = resolve_at_path(AT_FDCWD, &read_user_path(path_ptr)?)?;
    let vfs = vfs()?;
    let (node, canonical) = vfs.locate(&path, true).map_err(map_resolve_err)?;
    want_write(&vfs, &canonical)?;
    chown_node(&node, uid, gid)?;
    crate::fs::inotify::on_path(&path, &node, crate::fs::inotify::IN_ATTRIB);
    Ok(0)
}

/// Change ownership of a file by file descriptor (syscall 198).
pub fn sys_fchown(fd: usize, uid: usize, gid: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    want_write_file(&file)?;
    chown_node(&file.node, uid, gid)?;
    crate::fs::inotify::on_file(&file, crate::fs::inotify::IN_ATTRIB);
    Ok(0)
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
/// permission (EACCES); a device needs CAP_MKNOD (EPERM without it); the
/// mode is reduced by the umask. A filesystem that cannot hold
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
    use alloc::sync::Arc;

    use crate::fs::{
        tests::{make_vfs_with_root, StuckCaps},
        xattr, Permissions, VfsNode,
    };

    /// The console is a terminal by what the fd is, not by its number
    /// (N-198): /dev/console and /dev/tty0 are, the implicit console is,
    /// a file, a pipe or /dev/null is not.
    #[test]
    fn terminal_ioctls_are_decided_by_the_node() {
        use super::is_console_terminal as tty;
        assert!(tty(Some((5, 1)), false));
        assert!(tty(Some((4, 0)), false));
        assert!(tty(None, true));
        assert!(!tty(None, false), "a regular file or a pipe");
        assert!(!tty(Some((1, 3)), false), "/dev/null");
        assert!(!tty(Some((226, 0)), false), "a DRM card");
    }

    /// FIONREAD counts on the nodes that keep one: a pipe's contents from
    /// either end, a FIFO's; none on a plain file node (whose count comes
    /// from size and position instead).
    #[test]
    fn fionread_counts_queued_bytes() {
        use crate::fs::pipe::{create_pipe, PipeReadNode, PipeWriteNode};
        let (r, w) = create_pipe().unwrap();
        let (r, w) = (PipeReadNode::new(r), PipeWriteNode::new(w));
        assert_eq!(r.bytes_readable(), Some(0));
        w.write(0, b"hello").unwrap();
        assert_eq!(r.bytes_readable(), Some(5));
        assert_eq!(w.bytes_readable(), Some(5));
        let mut buf = [0u8; 2];
        r.read(0, &mut buf).unwrap();
        assert_eq!(r.bytes_readable(), Some(3));
        let vfs = make_vfs_with_root();
        let root = vfs.resolve_path("/").unwrap();
        let file = root.create("f", Permissions::from_mode(0o644)).unwrap();
        assert_eq!(file.bytes_readable(), None);
    }

    /// F_SETFL changes O_APPEND, O_ASYNC, O_DIRECT, O_NOATIME and
    /// O_NONBLOCK and nothing else, and F_GETFL reports them with the
    /// access mode and O_LARGEFILE, as Linux's setfl.
    #[test]
    fn status_flags_are_set_and_reported_as_linux_setfl() {
        use super::{
            file_status_flags as get, set_file_status_flags as set, SyscallError, O_LARGEFILE,
            O_NONBLOCK_FL,
        };
        use crate::fs::file::{status, File, OpenFlags};
        let vfs = make_vfs_with_root();
        let root = vfs.resolve_path("/").unwrap();
        let node = root.create("f", Permissions::from_mode(0o644)).unwrap();
        let file = File::new(node, OpenFlags::read_write());
        assert_eq!(get(&file), 2 | O_LARGEFILE);
        assert!(!file.is_append());
        // Access mode, O_CREAT and O_TRUNC bits are ignored.
        set(&file, status::APPEND | O_NONBLOCK_FL | 0x241, |_| true).unwrap();
        assert!(file.is_append());
        assert_eq!(get(&file), 2 | O_LARGEFILE | status::APPEND | O_NONBLOCK_FL);
        set(&file, status::ASYNC | status::DIRECT, |_| true).unwrap();
        assert!(!file.is_append());
        assert_eq!(get(&file), 2 | O_LARGEFILE | status::ASYNC | status::DIRECT);
        // O_NOATIME needs the owner; keeping it once set asks no one.
        assert_eq!(
            set(&file, status::NOATIME, |_| false),
            Err(SyscallError::OperationNotPermitted)
        );
        set(&file, status::NOATIME, |uid| uid == 0).unwrap();
        set(&file, status::NOATIME | status::APPEND, |_| false).unwrap();
        // O_DIRECT only on a regular file.
        let dir = File::new(root, OpenFlags::read_only());
        assert_eq!(
            set(&dir, status::DIRECT, |_| true),
            Err(SyscallError::InvalidArgument)
        );
        // An O_APPEND open starts with it.
        let mut flags = OpenFlags::read_write();
        flags.append = true;
        let node = vfs.resolve_path("/f").unwrap();
        assert!(File::new(node, flags).is_append());
    }

    /// A `struct flock` names an inclusive range from the start, the file
    /// position or the end, as Linux's flock64_to_posix_lock.
    #[test]
    fn flock_ranges_follow_whence_and_length() {
        use super::{flock_range, SyscallError, UserFlock};
        use crate::fs::flock::{F_RDLCK, F_UNLCK, F_WRLCK, OFFSET_MAX};
        let vfs = make_vfs_with_root();
        let root = vfs.resolve_path("/").unwrap();
        let node = root.create("f", Permissions::from_mode(0o644)).unwrap();
        node.write(0, &[0u8; 100]).unwrap();
        let file = crate::fs::file::File::new(node, crate::fs::file::OpenFlags::read_write());
        *file.position.write() = 10;
        let fl = |l_type: u16, l_whence: i16, l_start: i64, l_len: i64| UserFlock {
            l_type: l_type as i16,
            l_whence,
            l_start,
            l_len,
            ..UserFlock::default()
        };
        let range = |f| flock_range(&file, &f);
        assert_eq!(range(fl(F_RDLCK, 0, 5, 10)), Ok((F_RDLCK, 5, 14)));
        assert_eq!(range(fl(F_WRLCK, 1, 0, 0)), Ok((F_WRLCK, 10, OFFSET_MAX)));
        assert_eq!(range(fl(F_UNLCK, 2, -20, 5)), Ok((F_UNLCK, 80, 84)));
        // A negative length runs back from the start, which is excluded.
        assert_eq!(range(fl(F_WRLCK, 0, 10, -4)), Ok((F_WRLCK, 6, 9)));
        let inval = Err(SyscallError::InvalidArgument);
        assert_eq!(range(fl(F_WRLCK, 0, 3, -4)), inval);
        assert_eq!(range(fl(F_WRLCK, 1, -11, 1)), inval);
        assert_eq!(range(fl(F_WRLCK, 3, 0, 1)), inval);
        assert_eq!(range(fl(7, 0, 0, 1)), inval);
        let over = Err(SyscallError::Overflow);
        assert_eq!(range(fl(F_WRLCK, 2, i64::MAX, 1)), over);
        assert_eq!(range(fl(F_WRLCK, 0, i64::MAX, 2)), over);
        assert_eq!(
            range(fl(F_WRLCK, 0, i64::MAX, 1)),
            Ok((F_WRLCK, OFFSET_MAX, OFFSET_MAX))
        );
        // A position past OFFSET_MAX is an overflow, never a wrap.
        *file.position.write() = usize::MAX;
        assert_eq!(range(fl(F_WRLCK, 1, 0, 1)), over);
        assert_eq!(range(fl(F_WRLCK, 1, i64::MIN, 1)), over);
    }

    /// openat2's `how` checks, in build_open_flags' order.
    #[test]
    fn open_how_is_checked_as_linux_checks_it() {
        use super::{
            check_open_how as check, SyscallError, OPENAT2_REGULAR as REGULAR, O_CLOEXEC, O_CREAT,
            O_DIRECTORY_64 as DIR, O_EMPTYPATH, O_NOFOLLOW_64 as NOFOLLOW, O_PATH,
            O_TMPFILE_BIT as TMPFILE, O_TRUNC,
        };
        use crate::fs::resolve_flags as r;
        let inval = Err(SyscallError::InvalidArgument);
        assert_eq!(check(0, 0, 0), Ok(()));
        assert_eq!(
            check(O_CLOEXEC | 2, 0, u64::from(r::ALL & !r::IN_ROOT)),
            Ok(())
        );
        // Unknown open and resolve bits; O_CLOEXEC is always allowed.
        assert_eq!(check(1 << 31, 0, 0), inval);
        assert_eq!(check(1 << 33, 0, 0), inval);
        assert_eq!(check(0, 0, 0x40), inval);
        assert_eq!(check(0, 0, u64::from(r::BENEATH | r::IN_ROOT)), inval);
        // A mode only with a creation, and only permission bits.
        assert_eq!(check(O_CREAT, 0o7777, 0), Ok(()));
        assert_eq!(check(O_CREAT, 0o10000, 0), inval);
        assert_eq!(check(0, 0o644, 0), inval);
        assert_eq!(check(DIR | O_CREAT, 0, 0), inval);
        // O_TMPFILE needs O_DIRECTORY (it includes it) and a write mode.
        assert_eq!(check(TMPFILE | DIR | 2, 0o600, 0), Ok(()));
        assert_eq!(check(TMPFILE | 2, 0, 0), inval);
        assert_eq!(check(TMPFILE | DIR, 0, 0), inval);
        assert_eq!(check(REGULAR, 0, 0), Ok(()));
        assert_eq!(check(REGULAR | DIR, 0, 0), inval);
        // O_PATH takes only O_DIRECTORY, O_NOFOLLOW, O_EMPTYPATH, O_CLOEXEC.
        assert_eq!(
            check(O_PATH | DIR | NOFOLLOW | O_EMPTYPATH | O_CLOEXEC, 0, 0),
            Ok(())
        );
        assert_eq!(check(O_PATH | 2, 0, 0), inval);
        // RESOLVE_CACHED cannot create or truncate: EAGAIN.
        let cached = u64::from(r::CACHED);
        assert_eq!(check(0, 0, cached), Ok(()));
        for f in [O_TRUNC, O_CREAT, TMPFILE | DIR | 2] {
            assert_eq!(check(f, 0, cached), Err(SyscallError::WouldBlock));
        }
    }

    /// open(O_TRUNC) reports why a truncate of an existing file failed: a
    /// memfd sealed against shrinking is EPERM, as Linux's shmem_setattr
    /// returns, not ENOENT for a file that exists.
    #[test]
    fn a_refused_open_truncate_keeps_its_errno() {
        let m = crate::fs::memfd::MemfdNode::new(Permissions::from_mode(0o644), 0, 0, true, 0);
        m.truncate(4096).unwrap();
        m.add_seals(crate::fs::seals::SHRINK).unwrap();
        assert_eq!(
            super::open_truncate(&*m),
            Err(super::SyscallError::OperationNotPermitted)
        );
        assert_eq!(m.metadata().unwrap().size, 4096);
    }

    /// As Linux's setattr_prepare, chown removes the file's privileges
    /// before the owner changes: when that fails, the file keeps its owner
    /// (PR #33 review).
    #[test]
    fn a_chown_that_cannot_drop_file_caps_keeps_the_owner() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let inner = root.create("prog", Permissions::from_mode(0o4755)).unwrap();
        inner
            .update_xattrs(&mut |a| a.set(xattr::XATTR_NAME_CAPS, &[0; 20], 0))
            .unwrap();
        let node: Arc<dyn VfsNode> = Arc::new(StuckCaps(inner.clone(), false));
        assert!(super::chown_node(&node, 1000, 1000).is_err());
        let meta = inner.metadata().unwrap();
        assert_eq!((meta.uid, meta.gid), (0, 0));
        assert!(meta.permissions.set_uid);
    }

    /// A chown the filesystem cannot store (BlockFS keeps 16-bit IDs) fails
    /// before the file loses its privileges (PR #33 review round 7).
    #[test]
    fn a_chown_the_filesystem_cannot_store_keeps_the_privileges() {
        use crate::fs::Filesystem;
        let fs = crate::fs::blockfs::BlockFs::format(1000, 100).unwrap();
        let prog = fs
            .root()
            .create("prog", Permissions::from_mode(0o4755))
            .unwrap();
        assert!(super::chown_node(&prog, 70000, 0).is_err());
        let meta = prog.metadata().unwrap();
        assert_eq!(meta.uid, 0);
        assert!(meta.permissions.set_uid);
    }

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
            userns: crate::process::userns::initial(),
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
        let root = Caller::kernel();
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
            userns: crate::process::userns::initial(),
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
        assert!(may_chown(&Caller::kernel(), 1001, 7, Some(5), Some(9)));
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
            userns: crate::process::userns::initial(),
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
        let init = crate::process::userns::initial();
        assert!(!kill_ok(&u, &target, &init, false, 15, false));
        assert!(
            kill_ok(&u, &saved, &init, false, 15, false),
            "target's saved uid"
        );
        assert!(kill_ok(&u, &target, &init, true, 15, false), "itself");
        assert!(kill_ok(
            &user_with(&[CAP_KILL]),
            &target,
            &init,
            false,
            15,
            false
        ));
        assert!(kill_ok(&u, &target, &init, false, SIGCONT, true));
        assert!(!kill_ok(&u, &target, &init, false, SIGCONT, false));
        let bare_root = Caller {
            creds: Credentials::new(0, 0),
            caps: LinuxCaps::EMPTY,
            userns: crate::process::userns::initial(),
        };
        assert!(!kill_ok(&bare_root, &target, &init, false, 15, false));
        assert!(kill_ok(&Caller::kernel(), &target, &init, false, 15, false));
        // A namespace's root holds CAP_KILL over the processes in it, and
        // over nothing outside (ADR 0022 H4c).
        let ns = crate::process::userns::test_child(b"0 1000 1\n1 2000 1", b"0 1000 1");
        let ns_root = Caller {
            creds: Credentials::new(1000, 1000),
            caps: LinuxCaps::FULL,
            userns: ns.clone(),
        };
        assert!(kill_ok(&ns_root, &target, &ns, false, 15, false));
        assert!(!kill_ok(&ns_root, &target, &init, false, 15, false));
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
                userns: crate::process::userns::initial(),
            };
            crate::fs::permission(&caller, m, mode)
        };
        let m = meta(0o750, 1000, 100);
        assert!(granted(1000, 100, &m, 7));
        assert!(granted(2000, 100, &m, 4 | 1));
        assert!(!granted(2000, 100, &m, 2));
        assert!(!granted(2000, 200, &m, 4));
        assert!(crate::fs::permission(&Caller::kernel(), &m, 7));
        let noexec = meta(0o644, 1000, 100);
        assert!(!crate::fs::permission(&Caller::kernel(), &noexec, 1));
        let dir = crate::fs::Metadata {
            node_type: crate::fs::NodeType::Directory,
            ..noexec
        };
        assert!(crate::fs::permission(&Caller::kernel(), &dir, 1));
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

    /// utimes/futimesat times: microseconds become nanoseconds, and a
    /// tv_usec outside 0..1000000 is EINVAL, which also refuses the
    /// UTIME_NOW and UTIME_OMIT values utimensat alone accepts.
    #[test]
    fn timeval_times_follow_do_futimesat() {
        assert_eq!(
            timeval_times([[7, 1], [9, 999_999]]),
            Ok([[7, 1_000], [9, 999_999_000]])
        );
        for usec in [1_000_000, -1, UTIME_NOW, UTIME_OMIT] {
            assert_eq!(
                timeval_times([[1, usec], [2, 0]]),
                Err(SyscallError::InvalidArgument)
            );
            assert_eq!(
                timeval_times([[1, 0], [2, usec]]),
                Err(SyscallError::InvalidArgument)
            );
        }
    }

    /// utime's utimbuf holds whole seconds: actime, then modtime.
    #[test]
    fn utimbuf_times_are_whole_seconds() {
        assert_eq!(utimbuf_times([11, 22]), [[11, 0], [22, 0]]);
    }

    /// statx reports what stat does, plus the mount: its 31-bit id, or the
    /// unique one when asked, never both, and MOUNT_ROOT at its root; the
    /// attributes it can report are in the mask either way.
    #[test]
    fn statx_reports_stat_numbers_and_the_mount() {
        let mut m = meta(0o4750, 1000, 100);
        m.size = 1000;
        m.inode = 77;
        m.accessed = 5;
        m.modified = 6;
        m.created = 7;
        let mount = crate::fs::MountIds {
            id: 3,
            unique: (1 << 31) + 9,
            dev: 3,
        };
        let s = fill_statx(&m, (4, 64), mount, false, STATX_BASIC_STATS, 1);
        assert_eq!(s.stx_mask, STATX_BASIC_STATS | STATX_MNT_ID);
        assert_eq!(s.stx_mode, 0o104750);
        assert_eq!((s.stx_uid, s.stx_gid, s.stx_nlink), (1000, 100, 1));
        assert_eq!((s.stx_ino, s.stx_size, s.stx_blocks), (77, 1000, 2));
        assert_eq!(s.stx_blksize, 4096);
        assert_eq!(
            (s.stx_atime.tv_sec, s.stx_mtime.tv_sec, s.stx_ctime.tv_sec),
            (5, 6, 7)
        );
        assert_eq!(s.stx_btime, StatxTimestamp::default());
        assert_eq!((s.stx_rdev_major, s.stx_rdev_minor), (4, 64));
        // The mount's anonymous device, 0:3.
        assert_eq!((s.stx_dev_major, s.stx_dev_minor), (0, 3));
        assert_eq!(s.stx_mnt_id, 3);
        assert_eq!(s.stx_attributes, 0);
        assert_eq!(
            s.stx_attributes_mask,
            STATX_ATTR_AUTOMOUNT | STATX_ATTR_DAX | STATX_ATTR_MOUNT_ROOT
        );
        assert_eq!(s.__spare3, [0; 8]);

        let u = fill_statx(&m, (0, 0), mount, true, STATX_MNT_ID_UNIQUE, 2);
        assert_eq!(u.stx_nlink, 2);
        assert_eq!(u.stx_mask, STATX_BASIC_STATS | STATX_MNT_ID_UNIQUE);
        assert_eq!(u.stx_mnt_id, (1 << 31) + 9);
        assert_eq!(u.stx_attributes, STATX_ATTR_MOUNT_ROOT);
    }
}

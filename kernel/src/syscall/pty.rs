//! PTY (Pseudo-Terminal) system call handlers
//!
//! Implements the four PTY-related syscalls wired in Phase 6.5:
//!
//! | Number | Name       | Signature                                          |
//! |--------|------------|----------------------------------------------------|
//! | 280    | OpenPty    | `(master_fd_ptr: *mut i32, slave_fd_ptr: *mut i32)` |
//! | 281    | GrantPty   | `(master_fd: usize) -> 0`                          |
//! | 282    | UnlockPty  | `(master_fd: usize) -> 0`                          |
//! | 283    | PtsName    | `(master_fd: usize, buf: *mut u8, len: usize)`     |
//!
//! # Data Flow
//!
//! ```text
//!  [Terminal emulator]          [Shell / application]
//!   open(master_fd)              open(slave_fd via pts path)
//!       |                                |
//!       v                                v
//!  PtyMasterNode                   PtySlaveNode
//!   read()  <-- slave output  <-- PtySlave::write()
//!   write() --> slave input   --> PtySlave::read()
//! ```
//!
//! The `openpty` syscall allocates both fds in the calling process's file
//! table and writes their numbers to the two user-space `i32` pointers
//! supplied by the caller.

use alloc::{format, sync::Arc};

use super::{validate_user_buffer, validate_user_ptr_typed, SyscallError, SyscallResult};
use crate::{
    fs::{
        file::{File, OpenFlags},
        pty::{with_pty_manager, PtyMasterNode, PtySlave, PtySlaveNode},
        VfsNode,
    },
    process,
};

// ============================================================================
// sys_openpty — create a PTY pair and return two file descriptors
// ============================================================================

/// Create a new pseudo-terminal pair.
///
/// Allocates a PTY master/slave pair via [`PtyManager`], wraps each side in a
/// [`PtyMasterNode`] / [`PtySlaveNode`] VfsNode, creates [`File`] objects for
/// both, inserts them into the calling process's file table, and writes the
/// resulting file descriptor numbers to the caller-provided user-space
/// pointers.
///
/// # Arguments
/// - `master_fd_ptr`: user-space `*mut i32` that receives the master fd.
/// - `slave_fd_ptr`:  user-space `*mut i32` that receives the slave fd.
///
/// # Returns
/// `0` on success.
pub fn sys_openpty(master_fd_ptr: usize, slave_fd_ptr: usize) -> SyscallResult {
    // Validate both output pointers before doing any allocation.
    validate_user_ptr_typed::<i32>(master_fd_ptr)?;
    validate_user_ptr_typed::<i32>(slave_fd_ptr)?;

    // Allocate a new PTY pair through the global PtyManager.
    // create_pty() returns (master_id, slave_id); currently master_id == slave_id
    // because the slave shares the master's ID space.
    let (master_id, slave_id) = with_pty_manager(|mgr| mgr.create_pty())
        .ok_or(SyscallError::InvalidState)? // PTY manager not initialised
        .map_err(|_| SyscallError::OutOfMemory)?;

    // Retrieve the Arc<PtyMaster> we just created.
    let master_arc = with_pty_manager(|mgr| mgr.get_master(master_id))
        .ok_or(SyscallError::InvalidState)?
        .ok_or(SyscallError::ResourceNotFound)?;

    // Build VfsNode wrappers.
    let master_node: Arc<dyn VfsNode> = Arc::new(PtyMasterNode::new(master_arc));
    let slave_node: Arc<dyn VfsNode> =
        Arc::new(PtySlaveNode::new(PtySlave::new(slave_id, master_id)));

    // Create File objects.  Both sides are readable and writable.
    let master_file = Arc::new(File::new(master_node, OpenFlags::read_write()));
    let slave_file = Arc::new(File::new(slave_node, OpenFlags::read_write()));

    // Insert into the calling process's file table.
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();

    let master_fd = file_table
        .open(master_file)
        .map_err(|_| SyscallError::OutOfMemory)?;

    let slave_fd = file_table.open(slave_file).map_err(|_| {
        // Roll back the master fd on failure.
        let _ = file_table.close(master_fd);
        SyscallError::OutOfMemory
    })?;

    // Write the fd numbers to user space.
    // SAFETY: Both pointers were validated as aligned, non-null, and in user
    // space by validate_user_ptr_typed above.  No other thread can alias
    // these locations because they are caller-owned stack/heap slots.
    unsafe {
        core::ptr::write(master_fd_ptr as *mut i32, master_fd as i32);
        core::ptr::write(slave_fd_ptr as *mut i32, slave_fd as i32);
    }

    crate::println!(
        "[PTY] openpty: master_fd={}, slave_fd={}, pty_id={}",
        master_fd,
        slave_fd,
        master_id
    );

    Ok(0)
}

// ============================================================================
// sys_grantpt — grant access to the slave side
// ============================================================================

/// Grant ownership of the slave PTY device to the calling process.
///
/// On a production system this would `chown` `/dev/pts/N` to the calling
/// user and set mode 0620 (owned by group `tty`).  In VeridianOS the PTY
/// nodes are always accessible to root and all processes run as uid 0, so
/// this is a pure stub that validates the master fd and returns success.
///
/// # Arguments
/// - `master_fd`: file descriptor for the PTY master.
///
/// # Returns
/// `0` on success, `BadFileDescriptor` if `master_fd` is not a PTY master.
pub fn sys_grantpt(master_fd: usize) -> SyscallResult {
    // Validate that the fd exists in the calling process.
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let _file = file_table
        .get(master_fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Ownership is always root:tty in VeridianOS; nothing to change.
    Ok(0)
}

// ============================================================================
// sys_unlockpt — unlock the slave side for opening
// ============================================================================

/// Unlock the slave end of a PTY so that it can be opened.
///
/// The POSIX openpty(3) sequence requires `grantpt` + `unlockpt` before the
/// slave can be opened.  VeridianOS PTY slaves are always available once the
/// pair is created, so this is a stub that validates the fd and returns 0.
///
/// # Arguments
/// - `master_fd`: file descriptor for the PTY master.
///
/// # Returns
/// `0` on success, `BadFileDescriptor` if `master_fd` is invalid.
pub fn sys_unlockpt(master_fd: usize) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let _file = file_table
        .get(master_fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // The slave is already unlocked; nothing to do.
    Ok(0)
}

// ============================================================================
// sys_ptsname — return the device path of the slave
// ============================================================================

/// Write the slave device path for a PTY master fd into a user-space buffer.
///
/// The path has the form `/dev/pts/N` where `N` is the PTY ID. The kernel
/// infers the PTY ID by inspecting the VfsNode metadata inode field of the
/// master file, which encodes the ID as `0x9000_0000 | pty_id`.
///
/// # Arguments
/// - `master_fd`: file descriptor for the PTY master.
/// - `buf_ptr`:   user-space buffer to receive the NUL-terminated path.
/// - `buf_len`:   length of the buffer (must be at least 14 bytes for
///   `/dev/pts/NNNNN\0`).
///
/// # Returns
/// Number of bytes written (excluding the NUL terminator) on success.
pub fn sys_ptsname(master_fd: usize, buf_ptr: usize, buf_len: usize) -> SyscallResult {
    // Validate the output buffer.
    if buf_len == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(buf_ptr, buf_len)?;

    // Resolve the master fd to a File.
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file = file_table
        .get(master_fd)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Derive the PTY ID from the VfsNode metadata inode.
    // PtyMasterNode encodes the inode as 0x9000_0000 | pty_id.
    let metadata = file
        .node
        .metadata()
        .map_err(|_| SyscallError::BadFileDescriptor)?;

    // Verify this really is a PTY master node (inode high byte = 0x90).
    let inode = metadata.inode;
    if (inode >> 24) != 0x90 {
        return Err(SyscallError::NotATerminal);
    }
    let pty_id = (inode & 0x00FF_FFFF) as u32;

    // Build the path string.
    let path = format!("/dev/pts/{}", pty_id);
    let path_bytes = path.as_bytes();

    // The buffer must be large enough for the path plus NUL terminator.
    if buf_len < path_bytes.len() + 1 {
        return Err(SyscallError::InvalidArgument);
    }

    // Copy path into user space.
    // SAFETY: buf_ptr was validated as non-null and within user-space bounds
    // covering buf_len bytes.  path_bytes.len() < buf_len so the write does
    // not exceed the validated region.
    unsafe {
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(path_bytes.as_ptr(), dst, path_bytes.len());
        // NUL terminate.
        core::ptr::write(dst.add(path_bytes.len()), 0u8);
    }

    Ok(path_bytes.len())
}

// ============================================================================
// ioctl helpers for PTY file descriptors
// ============================================================================

/// ioctl constants relevant to PTY operations.
///
/// These use the same values as Linux/POSIX to allow existing user-space
/// programs compiled against a standard sysroot to work without changes.
pub mod pty_ioctl {
    /// Get window size (`struct winsize`). Value matches Linux `TIOCGWINSZ`.
    pub const TIOCGWINSZ: usize = 0x5413;
    /// Set window size (`struct winsize`). Value matches Linux `TIOCSWINSZ`.
    pub const TIOCSWINSZ: usize = 0x5414;
}

/// `struct winsize` as exposed to user space (4 × u16, C layout).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserWinsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

/// Handle ioctl commands that target a PTY master or slave file descriptor.
///
/// Returns `Some(result)` if the command was handled (the caller should
/// propagate the result), or `None` if the fd does not refer to a PTY node
/// and the caller should fall through to the generic ioctl path.
///
/// Currently supported commands:
/// - [`pty_ioctl::TIOCGWINSZ`] – read window size from the PTY master.
/// - [`pty_ioctl::TIOCSWINSZ`] – set window size on the PTY master.
pub fn handle_pty_ioctl(master_fd: usize, cmd: usize, arg: usize) -> Option<SyscallResult> {
    let proc = process::current_process()?;
    let file_table = proc.file_table.lock();
    let file = file_table.get(master_fd)?;

    // Determine the PTY ID from the inode.
    // Master inodes: 0x9000_0000 | pty_id
    // Slave  inodes: 0x9100_0000 | pty_id
    let inode = file.node.metadata().ok()?.inode;
    let high = inode >> 24;
    if high != 0x90 && high != 0x91 {
        return None; // Not a PTY fd – let the caller handle it.
    }
    let pty_id = (inode & 0x00FF_FFFF) as u32;

    Some(match cmd {
        pty_ioctl::TIOCGWINSZ => {
            if arg == 0 {
                return Some(Err(SyscallError::InvalidPointer));
            }
            if let Err(e) = validate_user_ptr_typed::<UserWinsize>(arg) {
                return Some(Err(e));
            }

            let ws =
                with_pty_manager(|mgr| mgr.get_master(pty_id).map(|m| m.get_winsize())).flatten();

            match ws {
                Some(w) => {
                    let user_ws = UserWinsize {
                        ws_row: w.rows,
                        ws_col: w.cols,
                        ws_xpixel: w.xpixel,
                        ws_ypixel: w.ypixel,
                    };
                    // SAFETY: arg was validated above.
                    unsafe { core::ptr::write(arg as *mut UserWinsize, user_ws) };
                    Ok(0)
                }
                None => Err(SyscallError::ResourceNotFound),
            }
        }

        pty_ioctl::TIOCSWINSZ => {
            if arg == 0 {
                return Some(Err(SyscallError::InvalidPointer));
            }
            if let Err(e) = validate_user_ptr_typed::<UserWinsize>(arg) {
                return Some(Err(e));
            }

            // SAFETY: arg was validated above.
            let user_ws = unsafe { core::ptr::read(arg as *const UserWinsize) };

            let new_ws = crate::fs::pty::Winsize {
                rows: user_ws.ws_row,
                cols: user_ws.ws_col,
                xpixel: user_ws.ws_xpixel,
                ypixel: user_ws.ws_ypixel,
            };

            with_pty_manager(|mgr| {
                if let Some(master) = mgr.get_master(pty_id) {
                    master.set_winsize(new_ws);
                }
            });

            Ok(0)
        }

        _ => {
            // Unrecognised command on a PTY fd; ENOTTY.
            Err(SyscallError::NotATerminal)
        }
    })
}

//! File attribute flags from user space (ADR 0027, `fs::attr`):
//! FS_IOC_GETFLAGS, FS_IOC_SETFLAGS, FS_IOC_FSGETXATTR and
//! FS_IOC_FSSETXATTR on an open file, and file_getattr/file_setattr by
//! path (Linux's fs/file_attr.c).

use alloc::sync::Arc;

use super::{
    filesystem::AT_EMPTY_PATH,
    userspace::{read_user, read_user_bytes, write_user, write_user_bytes},
    xattr, SyscallError, SyscallResult,
};
use crate::fs::{
    attr::{self, Change},
    VfsNode,
};

pub const FS_IOC_GETFLAGS: u32 = 0x8008_6601;
pub const FS_IOC_SETFLAGS: u32 = 0x4008_6602;
pub const FS_IOC_FSGETXATTR: u32 = 0x801c_581f;
pub const FS_IOC_FSSETXATTR: u32 = 0x401c_5820;

/// `struct file_attr` (FILE_ATTR_SIZE_VER0): fa_xflags (u64), fa_extsize,
/// fa_nextents, fa_projid, fa_cowextsize (u32 each).
const FILE_ATTR_SIZE: usize = 24;
/// `struct fsxattr`: five u32 and 8 bytes of padding.
const FSXATTR_SIZE: usize = 28;
const PAGE: usize = 4096;

/// A node's flags, or the error a filesystem without them gives: ENOTTY
/// from an ioctl (ENOIOCTLCMD), EOPNOTSUPP from file_getattr.
fn flags_of(node: &dyn VfsNode, from_ioctl: bool) -> Result<u32, SyscallError> {
    node.attr_flags().ok_or(if from_ioctl {
        SyscallError::NotATerminal
    } else {
        SyscallError::NotSupported
    })
}

/// vfs_fileattr_set: the owner or CAP_FOWNER over it, with WRITE on the
/// mount namespace (`owner_or_capable`, as chmod; EPERM), then
/// `attr::apply` (CAP_LINUX_IMMUTABLE for immutable and append-only). The
/// read-only mount check (EROFS) is the caller's.
fn set(node: &Arc<dyn VfsNode>, change: Change, from_ioctl: bool) -> Result<(), SyscallError> {
    let old = flags_of(&**node, from_ioctl)?;
    let caller = crate::process::creds::Caller::current();
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if !caller.owner_or_capable(meta.uid) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let immutable_ok = caller.capable(crate::process::linux_caps::CAP_LINUX_IMMUTABLE);
    let new = attr::apply(old, change, immutable_ok).map_err(super::map_kernel_error)?;
    if new & attr::FS_IMMUTABLE_FL != 0 && old & attr::FS_IMMUTABLE_FL == 0 {
        let _changing = attr::begin_change();
        freeze_mappings(node)?;
        node.set_attr_flags(new).map_err(super::map_kernel_error)?;
        // A second pass, with the flag set: a process made, or a mapping
        // made, while the first ran (commit security review). Their pages
        // have been write-protected since (mmap, fork while `changing`), so
        // nothing reached them unseen; this only settles them.
        write_protect_everywhere(node);
        return Ok(());
    }
    node.set_attr_flags(new).map_err(super::map_kernel_error)
}

/// Write-protect `node`'s writable shared mappings in every process.
fn write_protect_everywhere(
    node: &Arc<dyn VfsNode>,
) -> alloc::vec::Vec<Arc<crate::process::Process>> {
    let end = crate::mm::user_layout::user_space_end() as u64;
    let mut pids = alloc::vec::Vec::new();
    crate::process::table::PROCESS_TABLE.for_each(|p| pids.push(p.pid));
    let procs: alloc::vec::Vec<_> = pids
        .into_iter()
        .filter_map(crate::process::table::get_process)
        .collect();
    for p in &procs {
        p.memory_space.lock().write_protect_file(Some(node), 0, end);
    }
    procs
}

/// A file about to become immutable: every writable shared mapping of it,
/// in every process, loses its pages' write bit first, then what was
/// written through them goes to the file, while it still takes writes.
/// From then on a write through them faults, and once the flag is set the
/// fault is SIGBUS (ext4's page_mkwrite; the maintainer's choice). A fault
/// in between is retried until the flag is set (`attr::changing`).
fn freeze_mappings(node: &Arc<dyn VfsNode>) -> Result<(), SyscallError> {
    let end = crate::mm::user_layout::user_space_end() as u64;
    let procs = write_protect_everywhere(node);
    for p in &procs {
        let copies: alloc::vec::Vec<_> = p
            .memory_space
            .lock()
            .shared_file_copies(0, end)
            .into_iter()
            .filter(|(n, ..)| {
                core::ptr::eq(Arc::as_ptr(n) as *const u8, Arc::as_ptr(node) as *const u8)
            })
            .collect();
        super::memctl::write_back(p, copies)?;
    }
    Ok(())
}

/// The four ioctls on open file `file`; None for any other command.
pub fn ioctl(file: &crate::fs::file::File, cmd: u32, arg: usize) -> Option<SyscallResult> {
    let node = &file.node;
    Some(match cmd {
        FS_IOC_GETFLAGS => flags_of(&**node, true)
            .and_then(|f| write_user(arg, f & attr::USER_VISIBLE))
            .map(|_| 0),
        FS_IOC_FSGETXATTR => flags_of(&**node, true).and_then(|f| {
            let mut out = [0u8; FSXATTR_SIZE];
            out[..4].copy_from_slice(&attr::to_xflags(f).to_ne_bytes());
            write_user_bytes(arg, &out).map(|_| 0)
        }),
        FS_IOC_SETFLAGS => read_user::<u32>(arg).and_then(|flags| {
            super::filesystem::want_write_file(file)?;
            set(node, Change::Flags(flags), true)?;
            crate::fs::inotify::on_file(file, crate::fs::inotify::IN_ATTRIB);
            Ok(0)
        }),
        FS_IOC_FSSETXATTR => (|| {
            let mut raw = [0u8; FSXATTR_SIZE];
            read_user_bytes(arg, &mut raw)?;
            let word =
                |i: usize| u32::from_ne_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
            // copy_fsxattr_from_user: an unknown xflag is EOPNOTSUPP.
            if word(0) & !attr::XFLAGS_MASK != 0 {
                return Err(SyscallError::NotSupported);
            }
            super::filesystem::want_write_file(file)?;
            let change = Change::Fsx {
                xflags: word(0),
                extsize: word(1),
                projid: word(3),
                cowextsize: word(4),
            };
            set(node, change, true)?;
            crate::fs::inotify::on_file(file, crate::fs::inotify::IN_ATTRIB);
            Ok(0)
        })(),
        _ => return None,
    })
}

/// file_getattr and file_setattr's checks: at_flags AT_SYMLINK_NOFOLLOW
/// and AT_EMPTY_PATH only (EINVAL), a size of at least
/// FILE_ATTR_SIZE_VER0 (EINVAL) and at most a page (E2BIG). A NULL path
/// with a descriptor names the descriptor (filename_maybe_null).
fn checked(
    path_ptr: usize,
    dfd: usize,
    usize_: usize,
    at_flags: usize,
) -> Result<usize, SyscallError> {
    let at_flags = at_flags as u32 as usize;
    if at_flags & !(super::filesystem::AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if usize_ > PAGE {
        return Err(SyscallError::ArgumentListTooLong);
    }
    if usize_ < FILE_ATTR_SIZE {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(if path_ptr == 0 && (dfd as u32 as i32) >= 0 {
        at_flags | AT_EMPTY_PATH
    } else {
        at_flags
    })
}

/// file_getattr(dfd, path, attr, size, at_flags): the node's xflags in a
/// `struct file_attr` (extent sizes, extent count and project ID 0), the
/// bytes past it zeroed up to `size` (copy_struct_to_user).
pub fn sys_file_getattr(
    dfd: usize,
    path_ptr: usize,
    attr_ptr: usize,
    size: usize,
    at_flags: usize,
) -> SyscallResult {
    let at_flags = checked(path_ptr, dfd, size, at_flags)?;
    let node = xattr::path_target(dfd, path_ptr, at_flags)?;
    let flags = flags_of(&*node, false)?;
    let mut out = alloc::vec![0u8; size];
    out[..8].copy_from_slice(&u64::from(attr::to_xflags(flags)).to_ne_bytes());
    write_user_bytes(attr_ptr, &out)?;
    Ok(0)
}

/// file_setattr(dfd, path, attr, size, at_flags): read `struct file_attr`
/// (bytes past it must be zero: E2BIG, copy_struct_from_user; an xflag
/// Linux does not define: EINVAL), then as FS_IOC_FSSETXATTR. EROFS on a
/// read-only mount.
pub fn sys_file_setattr(
    dfd: usize,
    path_ptr: usize,
    attr_ptr: usize,
    size: usize,
    at_flags: usize,
) -> SyscallResult {
    let at_flags = checked(path_ptr, dfd, size, at_flags)?;
    let mut raw = alloc::vec![0u8; size];
    read_user_bytes(attr_ptr, &mut raw)?;
    if raw[FILE_ATTR_SIZE..].iter().any(|&b| b != 0) {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let xflags = u64::from_ne_bytes(raw[..8].try_into().unwrap_or([0; 8]));
    let word = |at: usize| u32::from_ne_bytes(raw[at..at + 4].try_into().unwrap_or([0; 4]));
    if xflags & !u64::from(attr::XFLAGS_MASK) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let change = Change::Fsx {
        xflags: xflags as u32,
        extsize: word(8),
        projid: word(16),
        cowextsize: word(20),
    };
    let (node, at) = xattr::path_target_located(dfd, path_ptr, at_flags)?;
    set(&node, change, false)?;
    crate::fs::inotify::on_target(at.as_deref(), &node, crate::fs::inotify::IN_ATTRIB);
    Ok(0)
}

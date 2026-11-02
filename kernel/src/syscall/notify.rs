//! inotify_init, inotify_init1, inotify_add_watch and inotify_rm_watch, as
//! Linux 7.3's `fs/notify/inotify/inotify_user.c`. The instances and their
//! events are [`crate::fs::inotify`]'s.

use alloc::sync::Arc;

use super::{
    filesystem::{map_resolve_err, read_user_path, resolve_at_path, vfs, AT_FDCWD},
    SyscallError, SyscallResult,
};
use crate::{
    fs::{
        file::{File, OpenFlags},
        inotify::{
            Group, InotifyNode, ALL_INOTIFY_BITS, IN_CLOEXEC, IN_DONT_FOLLOW, IN_MASK_ADD,
            IN_MASK_CREATE, IN_NONBLOCK, IN_ONLYDIR,
        },
        NodeType, VfsNode,
    },
    process::creds::Caller,
};

/// inotify_init1(flags): a new instance, read-only, with IN_NONBLOCK and
/// IN_CLOEXEC as asked (anything else is EINVAL); EMFILE past the user's
/// instance limit.
pub fn sys_inotify_init1(flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags & !(IN_CLOEXEC | IN_NONBLOCK) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let group = Group::new(Caller::current().creds.euid)?;
    let mut open = OpenFlags::read_only();
    open.nonblock = flags & IN_NONBLOCK != 0;
    let file = File::new(Arc::new(InotifyNode::new(group)), open);
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = proc
        .file_table
        .lock()
        .open_with_flags(Arc::new(file), flags & IN_CLOEXEC != 0)
        .map_err(super::map_kernel_error)?;
    Ok(fd)
}

/// The instance `file` is: EINVAL for another kind of file.
fn group_of(file: &File) -> Result<Arc<Group>, SyscallError> {
    file.node
        .as_any()
        .and_then(|any| any.downcast_ref::<InotifyNode>())
        .map(|node| node.group().clone())
        .ok_or(SyscallError::InvalidArgument)
}

/// inotify_add_watch(fd, path, mask): watch the inode `path` names (its
/// final symlink followed unless IN_DONT_FOLLOW; a directory with
/// IN_ONLYDIR), which the caller must be able to read, for the events in
/// `mask`. The checks are Linux's, in its order: bits outside
/// ALL_INOTIFY_BITS or none of them are EINVAL, a bad descriptor EBADF,
/// IN_MASK_ADD with IN_MASK_CREATE and a descriptor that is not an inotify
/// instance EINVAL, then the lookup.
pub fn sys_inotify_add_watch(fd: usize, path_ptr: usize, mask: usize) -> SyscallResult {
    let mask = mask as u32;
    if mask & !ALL_INOTIFY_BITS != 0 || mask & ALL_INOTIFY_BITS == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let file = super::file_calls::open_file(fd)?;
    if mask & IN_MASK_ADD != 0 && mask & IN_MASK_CREATE != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let group = group_of(&file)?;
    let path = resolve_at_path(AT_FDCWD, &read_user_path(path_ptr)?)?;
    let view = vfs()?;
    let cwd = view.get_cwd();
    let (node, canonical): (Arc<dyn VfsNode>, _) = view
        .resolve_canonical(&path, &cwd, mask & IN_DONT_FOLLOW == 0)
        .map_err(map_resolve_err)?;
    if mask & IN_ONLYDIR != 0 && node.node_type() != NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    // Watching needs read permission (inotify_find_inode), and the MAC
    // policy's read access, as an open for reading by name.
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    if !crate::fs::permission(&Caller::current(), &meta, crate::fs::MAY_READ) {
        return Err(SyscallError::PermissionDenied);
    }
    let pid = crate::process::current_process().map_or(0, |p| p.pid.0);
    crate::security::mac::check_file_access(&canonical, crate::security::AccessType::Read, pid)
        .map_err(super::map_kernel_error)?;
    Ok(group.add_watch(&node, mask)? as usize)
}

/// inotify_rm_watch(fd, wd): end the watch `wd`, which queues IN_IGNORED.
/// EBADF for a bad descriptor, EINVAL for one that is not an instance or a
/// `wd` it does not have.
pub fn sys_inotify_rm_watch(fd: usize, wd: usize) -> SyscallResult {
    group_of(&*super::file_calls::open_file(fd)?)?.rm_watch(wd as u32 as i32)?;
    Ok(0)
}

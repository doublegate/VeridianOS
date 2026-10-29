//! The extended attribute system calls (ADR 0020 step 8), as Linux's
//! `fs/xattr.c`: setxattr, getxattr, listxattr and removexattr, each on a
//! path, on a path without following a final symlink (`l*`), on a file
//! descriptor (`f*`), and relative to a directory with flags (`*xattrat`,
//! Linux 6.13). The namespace rules are `fs::xattr::check_access`; the
//! attributes are the node's (`VfsNode::xattrs`, `VfsNode::update_xattrs`).

use alloc::{string::String, sync::Arc, vec::Vec};

use super::{
    filesystem::{map_resolve_err, read_user_path, read_user_path_or_empty, resolve_at_path, vfs},
    userspace, SyscallError, SyscallResult,
};
use crate::{
    fs::{
        xattr::{
            self, XATTR_CREATE, XATTR_LIST_MAX, XATTR_NAME_MAX, XATTR_REPLACE, XATTR_SIZE_MAX,
        },
        VfsNode,
    },
    process::{creds::Caller, linux_caps::FileCaps},
};

const AT_FDCWD: i32 = -100;
const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
const AT_EMPTY_PATH: u32 = 0x1000;

/// `struct xattr_args` (value pointer, size, flags) as the `*xattrat` calls
/// take it: 16 bytes in its first version, the only one.
const XATTR_ARGS_SIZE_VER0: usize = 16;

/// The node a call acts on: a path (`at_flags` of the `*xattrat` form),
/// or with AT_EMPTY_PATH and an empty or NULL path, `dfd` itself. There
/// AT_FDCWD names the working directory only when `cwd_ok` (get and set:
/// Linux's path_getxattrat and path_setxattrat look the empty name up for
/// a negative fd); list and remove always take the fd, so it is EBADF.
fn target(
    dfd: usize,
    path_ptr: usize,
    at_flags: u32,
    cwd_ok: bool,
) -> Result<Arc<dyn VfsNode>, SyscallError> {
    if at_flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let path = if at_flags & AT_EMPTY_PATH != 0 {
        if path_ptr == 0 {
            String::new()
        } else {
            read_user_path_or_empty(path_ptr)?
        }
    } else {
        read_user_path(path_ptr)?
    };
    let vfs = vfs()?;
    if path.is_empty() {
        if cwd_ok && dfd as u32 as i32 == AT_FDCWD {
            return vfs.resolve_path(&vfs.get_cwd()).map_err(map_resolve_err);
        }
        let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
        let file = proc
            .file_table
            .lock()
            .get(dfd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?;
        return Ok(file.node.clone());
    }
    let abs = resolve_at_path(dfd, &path)?;
    if at_flags & AT_SYMLINK_NOFOLLOW != 0 {
        vfs.resolve_path_no_follow(&abs)
    } else {
        vfs.resolve_path(&abs)
    }
    .map_err(map_resolve_err)
}

/// An attribute name from user memory (Linux's import_xattr_name): empty,
/// or no NUL within XATTR_NAME_MAX + 1 bytes, is ERANGE.
fn read_name(ptr: usize) -> Result<String, SyscallError> {
    let mut buf = [0u8; XATTR_NAME_MAX + 1];
    let n = userspace::strncpy_from_user(ptr, &mut buf)?;
    if n == 0 || n == buf.len() {
        return Err(SyscallError::RangeError);
    }
    core::str::from_utf8(&buf[..n])
        .map(String::from)
        // Linux takes any bytes; no namespace prefix matches these.
        .map_err(|_| SyscallError::NotSupported)
}

/// Linux's xattr_permission and the capability checks, for `node`.
fn check(node: &Arc<dyn VfsNode>, name: &str, write: bool) -> Result<(), SyscallError> {
    let meta = node.metadata().map_err(super::map_kernel_error)?;
    xattr::check_access(&Caller::current(), &meta, name, write).map_err(super::map_kernel_error)
}

fn set(
    dfd: usize,
    path: usize,
    at_flags: u32,
    name: usize,
    value: usize,
    size: usize,
    flags: u32,
) -> SyscallResult {
    if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let name = read_name(name)?;
    if size > XATTR_SIZE_MAX {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let mut bytes = alloc::vec![0u8; size];
    userspace::read_user_bytes(value, &mut bytes)?;
    let node = target(dfd, path, at_flags, true)?;
    // A file capability must be a v2 or v3 value (cap_convert_nscap,
    // before its CAP_SETFCAP check).
    xattr::check_value(&name, &bytes).map_err(|_| SyscallError::InvalidArgument)?;
    check(&node, &name, true)?;
    node.update_xattrs(&mut |a| a.set(&name, &bytes, flags))
        .map_err(super::map_kernel_error)?;
    Ok(0)
}

fn get(
    dfd: usize,
    path: usize,
    at_flags: u32,
    name: usize,
    value: usize,
    size: usize,
) -> SyscallResult {
    let name = read_name(name)?;
    let node = target(dfd, path, at_flags, true)?;
    check(&node, &name, false)?;
    let attrs = node.xattrs().map_err(super::map_kernel_error)?;
    let found = attrs.get(&name).ok_or(SyscallError::NoData)?;
    // A file capability is shown as the reader's namespace sees it
    // (cap_inode_getsecurity).
    let converted;
    let found = if name == xattr::XATTR_NAME_CAPS {
        converted = FileCaps::xattr_for_reader(found)?;
        &converted[..]
    } else {
        found
    };
    if size == 0 {
        return Ok(found.len());
    }
    if size.min(XATTR_SIZE_MAX) < found.len() {
        return Err(SyscallError::RangeError);
    }
    userspace::write_user_bytes(value, found)?;
    Ok(found.len())
}

fn list(dfd: usize, path: usize, at_flags: u32, out: usize, size: usize) -> SyscallResult {
    let node = target(dfd, path, at_flags, false)?;
    let attrs = node.xattrs().map_err(super::map_kernel_error)?;
    let caller = Caller::current();
    let mut names = Vec::new();
    for name in attrs.names().filter(|n| xattr::listed(&caller, n)) {
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    if size == 0 {
        return Ok(names.len());
    }
    if names.len() > size.min(XATTR_LIST_MAX) {
        return Err(if size >= XATTR_LIST_MAX {
            SyscallError::ArgumentListTooLong
        } else {
            SyscallError::RangeError
        });
    }
    userspace::write_user_bytes(out, &names)?;
    Ok(names.len())
}

fn remove(dfd: usize, path: usize, at_flags: u32, name: usize) -> SyscallResult {
    let name = read_name(name)?;
    let node = target(dfd, path, at_flags, false)?;
    check(&node, &name, true)?;
    node.update_xattrs(&mut |a| a.remove(&name))
        .map_err(super::map_kernel_error)?;
    Ok(0)
}

const CWD: usize = AT_FDCWD as isize as usize;

pub fn sys_setxattr(
    path: usize,
    name: usize,
    value: usize,
    size: usize,
    flags: usize,
) -> SyscallResult {
    set(CWD, path, 0, name, value, size, flags as u32)
}

pub fn sys_lsetxattr(
    path: usize,
    name: usize,
    value: usize,
    size: usize,
    flags: usize,
) -> SyscallResult {
    set(
        CWD,
        path,
        AT_SYMLINK_NOFOLLOW,
        name,
        value,
        size,
        flags as u32,
    )
}

pub fn sys_fsetxattr(
    fd: usize,
    name: usize,
    value: usize,
    size: usize,
    flags: usize,
) -> SyscallResult {
    set(fd, 0, AT_EMPTY_PATH, name, value, size, flags as u32)
}

pub fn sys_getxattr(path: usize, name: usize, value: usize, size: usize) -> SyscallResult {
    get(CWD, path, 0, name, value, size)
}

pub fn sys_lgetxattr(path: usize, name: usize, value: usize, size: usize) -> SyscallResult {
    get(CWD, path, AT_SYMLINK_NOFOLLOW, name, value, size)
}

pub fn sys_fgetxattr(fd: usize, name: usize, value: usize, size: usize) -> SyscallResult {
    get(fd, 0, AT_EMPTY_PATH, name, value, size)
}

pub fn sys_listxattr(path: usize, list_ptr: usize, size: usize) -> SyscallResult {
    list(CWD, path, 0, list_ptr, size)
}

pub fn sys_llistxattr(path: usize, list_ptr: usize, size: usize) -> SyscallResult {
    list(CWD, path, AT_SYMLINK_NOFOLLOW, list_ptr, size)
}

pub fn sys_flistxattr(fd: usize, list_ptr: usize, size: usize) -> SyscallResult {
    list(fd, 0, AT_EMPTY_PATH, list_ptr, size)
}

pub fn sys_removexattr(path: usize, name: usize) -> SyscallResult {
    remove(CWD, path, 0, name)
}

pub fn sys_lremovexattr(path: usize, name: usize) -> SyscallResult {
    remove(CWD, path, AT_SYMLINK_NOFOLLOW, name)
}

pub fn sys_fremovexattr(fd: usize, name: usize) -> SyscallResult {
    remove(fd, 0, AT_EMPTY_PATH, name)
}

/// `struct xattr_args` from user memory (Linux's copy_struct_from_user):
/// EINVAL below its first version's size, E2BIG above a page or for
/// nonzero bytes past the fields this kernel knows.
fn read_args(ptr: usize, args_size: usize) -> Result<(usize, usize, u32), SyscallError> {
    if args_size < XATTR_ARGS_SIZE_VER0 {
        return Err(SyscallError::InvalidArgument);
    }
    if args_size > 4096 {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let mut raw = alloc::vec![0u8; args_size];
    userspace::read_user_bytes(ptr, &mut raw)?;
    if raw[XATTR_ARGS_SIZE_VER0..].iter().any(|&b| b != 0) {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let word = |at: usize| u32::from_ne_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
    let value = u64::from_ne_bytes(
        raw[0..8]
            .try_into()
            .map_err(|_| SyscallError::InvalidArgument)?,
    );
    Ok((value as usize, word(8) as usize, word(12)))
}

/// setxattrat(dfd, path, at_flags, name, args, size).
pub fn sys_setxattrat(
    dfd: usize,
    path: usize,
    at_flags: usize,
    name: usize,
    args: usize,
    args_size: usize,
) -> SyscallResult {
    let (value, size, flags) = read_args(args, args_size)?;
    set(dfd, path, at_flags as u32, name, value, size, flags)
}

/// getxattrat(dfd, path, at_flags, name, args, size): `args.flags` must be 0.
pub fn sys_getxattrat(
    dfd: usize,
    path: usize,
    at_flags: usize,
    name: usize,
    args: usize,
    args_size: usize,
) -> SyscallResult {
    let (value, size, flags) = read_args(args, args_size)?;
    if flags != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    get(dfd, path, at_flags as u32, name, value, size)
}

/// listxattrat(dfd, path, at_flags, list, size).
pub fn sys_listxattrat(
    dfd: usize,
    path: usize,
    at_flags: usize,
    list_ptr: usize,
    size: usize,
) -> SyscallResult {
    list(dfd, path, at_flags as u32, list_ptr, size)
}

/// removexattrat(dfd, path, at_flags, name).
pub fn sys_removexattrat(dfd: usize, path: usize, at_flags: usize, name: usize) -> SyscallResult {
    remove(dfd, path, at_flags as u32, name)
}

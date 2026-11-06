//! File handles (group 3, step D4), as Linux 7.3's `fs/fhandle.c`:
//! name_to_handle_at encodes a file as its inode number, and
//! open_by_handle_at finds that number again in the filesystem of a mount
//! ([`crate::fs::Vfs::find_by_inode`]) and opens the file. Handles that can
//! be opened name files on exportable filesystems (ramfs, tmpfs, BlockFS);
//! AT_HANDLE_FID asks for one that only identifies a file, which every
//! filesystem gives.

use alloc::{string::String, sync::Arc};

use super::{
    file_calls::path_file,
    filesystem::{
        map_resolve_err, open_node, read_user_path, read_user_path_or_empty, resolve_at_path, vfs,
        AT_EMPTY_PATH, AT_FDCWD,
    },
    userspace, SyscallError, SyscallResult,
};
use crate::fs::{CallerView, NodeType, VfsNode};

const AT_SYMLINK_FOLLOW: usize = 0x400;
const AT_HANDLE_FID: usize = 0x200;
const AT_HANDLE_MNT_ID_UNIQUE: usize = 0x1;
const AT_HANDLE_CONNECTABLE: usize = 0x2;

/// The largest handle body a caller may pass (MAX_HANDLE_SZ).
const MAX_HANDLE_SZ: u32 = 128;
/// `struct file_handle`'s header: `handle_bytes` (u32), `handle_type` (int).
const HEADER_LEN: usize = 8;
/// Our handles: the inode number (u64), then a generation (u32) that is
/// always 0, as Linux's FILEID_INO64_GEN.
const FILEID_INO64_GEN: i32 = 0x81;
const HANDLE_LEN: u32 = 12;
/// The type name_to_handle_at reports with EOVERFLOW.
const FILEID_INVALID: i32 = 0xff;
/// Flags in the upper 16 bits of a handle's type, which only the VFS sees.
const FILEID_USER_FLAGS_MASK: i32 = 0xffff_0000_u32 as i32;
const FILEID_IS_CONNECTABLE: i32 = 0x1_0000;
const FILEID_IS_DIR: i32 = 0x2_0000;

/// The handle of a file with inode number `ino`, for a buffer with room
/// for `room` bytes: its length, its type, and its body, or `None` when it
/// does not fit (EOVERFLOW: the length is still reported, the body is not
/// written). A connectable handle records that, and whether it names a
/// directory, in its type, as Linux's do_sys_name_to_handle.
fn encode(
    ino: u64,
    room: u32,
    connectable: bool,
    is_dir: bool,
) -> (u32, i32, Option<[u8; HANDLE_LEN as usize]>) {
    if room < HANDLE_LEN {
        return (HANDLE_LEN, FILEID_INVALID, None);
    }
    let mut ty = FILEID_INO64_GEN;
    if connectable {
        ty |= FILEID_IS_CONNECTABLE;
        if is_dir {
            ty |= FILEID_IS_DIR;
        }
    }
    let mut body = [0u8; HANDLE_LEN as usize];
    body[..8].copy_from_slice(&ino.to_le_bytes());
    (HANDLE_LEN, ty, Some(body))
}

/// The file a handle of type `ty` with `body` names, decoded within the
/// filesystem serving `anchor` (a canonical path; `None` for a descriptor
/// without one), and its canonical path. `None` is ESTALE: the anchor's
/// filesystem cannot decode handles, the handle is not one of ours, the
/// file is gone, a directory handle names something else, or a
/// connectable one names a file outside the anchor.
fn decode(
    vfs: &CallerView<'_>,
    anchor: Option<&str>,
    ty: i32,
    body: &[u8],
) -> Option<(Arc<dyn VfsNode>, String)> {
    let anchor = anchor?;
    let (mount, fs) = vfs.filesystem_of(anchor)?;
    if !fs.exportable() || ty & !FILEID_USER_FLAGS_MASK != FILEID_INO64_GEN {
        return None;
    }
    let ino = u64::from_le_bytes(body.get(..8)?.try_into().ok()?);
    if body.get(8..12)? != [0; 4] {
        return None;
    }
    let (node, path) = vfs.find_by_inode(&mount, ino)?;
    if ty & FILEID_IS_DIR != 0 && node.node_type() != NodeType::Directory {
        return None;
    }
    if ty & FILEID_IS_CONNECTABLE != 0 && !crate::fs::path_is_under(&path, anchor) {
        return None;
    }
    Some((node, path))
}

/// The file name_to_handle_at encodes, and its canonical path (`None` for
/// a descriptor without one, a pipe or socket): `name` at `dirfd`, a final
/// symlink followed only with AT_SYMLINK_FOLLOW, or with AT_EMPTY_PATH and
/// an empty or NULL name, `dirfd` itself (the cwd for AT_FDCWD).
fn lookup(
    dirfd: usize,
    path_ptr: usize,
    flags: usize,
) -> Result<(Arc<dyn VfsNode>, Option<String>), SyscallError> {
    let name = if flags & AT_EMPTY_PATH == 0 {
        read_user_path(path_ptr)?
    } else if path_ptr == 0 {
        String::new()
    } else {
        read_user_path_or_empty(path_ptr)?
    };
    let vfs = vfs()?;
    if name.is_empty() {
        if dirfd as u32 as i32 == AT_FDCWD as u32 as i32 {
            let (node, canonical) = vfs.resolve_path_canonical(".").map_err(map_resolve_err)?;
            return Ok((node, Some(canonical)));
        }
        let file = path_file(dirfd)?;
        return Ok((file.node.clone(), file.path.clone()));
    }
    let abs = resolve_at_path(dirfd, &name)?;
    let trailing_slash = name.ends_with('/');
    let follow = flags & AT_SYMLINK_FOLLOW != 0 || trailing_slash;
    let (node, canonical) = vfs
        .resolve_canonical(&abs, &vfs.get_cwd(), follow)
        .map_err(map_resolve_err)?;
    if trailing_slash && node.node_type() != NodeType::Directory {
        return Err(SyscallError::NotADirectory);
    }
    Ok((node, Some(canonical)))
}

/// name_to_handle_at(dirfd, name, handle, mount_id, flags): the handle of
/// a file, and the id of its mount (an int, or with AT_HANDLE_MNT_ID_UNIQUE
/// the u64 unique id), in Linux's order: unknown flags, and CONNECTABLE
/// with FID or EMPTY_PATH, are EINVAL; then the lookup; then a filesystem
/// that cannot decode handles is EOPNOTSUPP (unless AT_HANDLE_FID); then
/// the header (EFAULT) and a `handle_bytes` over MAX_HANDLE_SZ (EINVAL).
/// Too small a buffer is EOVERFLOW, with the needed size written back; the
/// mount id is written either way, and when that faults (EFAULT) the
/// handle is not.
pub fn sys_name_to_handle_at(
    dirfd: usize,
    path_ptr: usize,
    handle_ptr: usize,
    mnt_id_ptr: usize,
    flags: usize,
) -> SyscallResult {
    let flags = flags as u32 as usize;
    if flags
        & !(AT_SYMLINK_FOLLOW
            | AT_EMPTY_PATH
            | AT_HANDLE_FID
            | AT_HANDLE_MNT_ID_UNIQUE
            | AT_HANDLE_CONNECTABLE)
        != 0
    {
        return Err(SyscallError::InvalidArgument);
    }
    let connectable = flags & AT_HANDLE_CONNECTABLE != 0;
    if connectable && flags & (AT_HANDLE_FID | AT_EMPTY_PATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (node, canonical) = lookup(dirfd, path_ptr, flags)?;
    let vfs = vfs()?;
    let exportable = canonical
        .as_deref()
        .and_then(|path| vfs.filesystem_of(path))
        .is_some_and(|(_, fs)| fs.exportable());
    if flags & AT_HANDLE_FID == 0 && !exportable {
        return Err(SyscallError::NotSupported);
    }
    let [room, _] = userspace::read_user::<[u32; 2]>(handle_ptr)?;
    if room > MAX_HANDLE_SZ {
        return Err(SyscallError::InvalidArgument);
    }
    let ino = node.metadata().map_err(super::map_kernel_error)?.inode;
    let is_dir = node.node_type() == NodeType::Directory;
    let (len, ty, body) = encode(ino, room, connectable, is_dir);

    let (mount, _) = canonical
        .as_deref()
        .map_or((crate::fs::PSEUDO_MOUNT, false), |path| {
            vfs.mount_ids_of(path)
        });
    let wrote_mount = if flags & AT_HANDLE_MNT_ID_UNIQUE != 0 {
        userspace::write_user(mnt_id_ptr, mount.unique)
    } else {
        userspace::write_user(mnt_id_ptr, mount.id as i32)
    };
    wrote_mount?;
    userspace::write_user(handle_ptr, [len, ty as u32])?;
    match body {
        Some(body) => {
            userspace::write_user_bytes(handle_ptr + HEADER_LEN, &body)?;
            Ok(0)
        }
        None => Err(SyscallError::Overflow),
    }
}

/// open_by_handle_at(mount_fd, handle, flags): open the file a handle
/// names, looked for in the filesystem of `mount_fd`'s file (the cwd's for
/// AT_FDCWD), with open(2)'s `flags`. In Linux's order: the header
/// (EFAULT), a `handle_bytes` of 0 or over MAX_HANDLE_SZ and a negative
/// type or unknown type flags (EINVAL), the descriptor (EBADF), then
/// CAP_DAC_READ_SEARCH (EPERM: Linux's relaxed rule for a caller who could
/// mount the filesystem needs user namespaces), then the body (EFAULT),
/// then the decode (ESTALE), then the open as open(2) does it on the file
/// itself: a symlink is ELOOP.
pub fn sys_open_by_handle_at(mount_fd: usize, handle_ptr: usize, flags: usize) -> SyscallResult {
    #[cfg(target_arch = "aarch64")]
    const O_DIRECTORY: usize = 0x4000;
    #[cfg(not(target_arch = "aarch64"))]
    const O_DIRECTORY: usize = 0x1_0000;
    const O_CREAT: usize = 0x40;
    const O_EXCL: usize = 0x80;

    let [len, ty] = userspace::read_user::<[u32; 2]>(handle_ptr)?;
    let ty = ty as i32;
    if len == 0 || len > MAX_HANDLE_SZ {
        return Err(SyscallError::InvalidArgument);
    }
    if ty < 0 || ty & FILEID_USER_FLAGS_MASK & !(FILEID_IS_CONNECTABLE | FILEID_IS_DIR) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let fd = mount_fd as u32 as i32;
    let anchor = if fd >= 0 {
        path_file(fd as usize)?.path.clone()
    } else if fd == AT_FDCWD as u32 as i32 {
        vfs()?
            .resolve_path_canonical(".")
            .map(|(_, canonical)| canonical)
            .ok()
    } else {
        return Err(SyscallError::BadFileDescriptor);
    };
    if !crate::process::creds::Caller::current()
        .capable(crate::process::linux_caps::CAP_DAC_READ_SEARCH)
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    let mut body = [0u8; MAX_HANDLE_SZ as usize];
    let body = &mut body[..len as usize];
    userspace::read_user_bytes(handle_ptr + HEADER_LEN, body)?;
    let vfs = vfs()?;
    let (node, canonical) = decode(&vfs, anchor.as_deref(), ty, body).ok_or(SyscallError::Stale)?;
    // open_by_handle_at opens what it found; O_CREAT makes nothing, and
    // the flags are checked as open's (Linux's build_open_flags).
    let flags = flags as u32 as usize;
    if flags & O_CREAT != 0 {
        if flags & O_DIRECTORY != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        if flags & O_EXCL != 0 {
            return Err(SyscallError::FileExists);
        }
    }
    open_node(node, false, canonical, flags, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{tests::make_vfs_with_root, Permissions};

    #[test]
    fn encode_reports_the_size_when_the_buffer_is_too_small() {
        assert_eq!(encode(7, 0, false, false), (12, FILEID_INVALID, None));
        assert_eq!(encode(7, 11, false, false).2, None);
        let (len, ty, body) = encode(0x0102_0304_0506_0708, 12, false, false);
        assert_eq!((len, ty), (12, FILEID_INO64_GEN));
        assert_eq!(body.unwrap(), [8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 0, 0]);
        assert_eq!(
            encode(7, 128, true, false).1,
            FILEID_INO64_GEN | FILEID_IS_CONNECTABLE
        );
        assert_eq!(
            encode(7, 128, true, true).1,
            FILEID_INO64_GEN | FILEID_IS_CONNECTABLE | FILEID_IS_DIR
        );
    }

    #[test]
    fn decode_finds_the_encoded_file_until_it_is_gone() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let d = root.mkdir("d", Permissions::default()).unwrap();
        let f = d.create("f", Permissions::default()).unwrap();
        let (_, ty, body) = encode(f.metadata().unwrap().inode, 128, false, false);
        let body = body.unwrap();

        let (node, path) = decode(&vfs.as_caller(), Some("/d"), ty, &body).unwrap();
        assert!(Arc::ptr_eq(&node, &f));
        assert_eq!(path, "/d/f");
        // Not ours, a generation, a short body, no anchor path: stale.
        assert!(decode(&vfs.as_caller(), Some("/"), 1, &body).is_none());
        let mut gen = body;
        gen[8] = 1;
        assert!(decode(&vfs.as_caller(), Some("/"), ty, &gen).is_none());
        assert!(decode(&vfs.as_caller(), Some("/"), ty, &body[..8]).is_none());
        assert!(decode(&vfs.as_caller(), None, ty, &body).is_none());
        // A directory handle must name a directory.
        assert!(decode(&vfs.as_caller(), Some("/"), ty | FILEID_IS_DIR, &body).is_none());
        d.unlink("f").unwrap();
        assert!(decode(&vfs.as_caller(), Some("/"), ty, &body).is_none());
    }

    #[test]
    fn a_connectable_handle_decodes_only_below_its_anchor() {
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        root.mkdir("b", Permissions::default()).unwrap();
        let f = a.create("f", Permissions::default()).unwrap();
        let (_, ty, body) = encode(f.metadata().unwrap().inode, 128, true, false);
        let body = body.unwrap();
        assert!(decode(&vfs.as_caller(), Some("/a"), ty, &body).is_some());
        assert!(decode(&vfs.as_caller(), Some("/"), ty, &body).is_some());
        assert!(decode(&vfs.as_caller(), Some("/b"), ty, &body).is_none());
    }

    #[test]
    fn decode_needs_an_exportable_filesystem() {
        use crate::fs::Filesystem;
        struct Opaque(crate::fs::ramfs::RamFs);
        impl Filesystem for Opaque {
            fn root(&self) -> Arc<dyn VfsNode> {
                self.0.root()
            }
            fn name(&self) -> &str {
                "opaque"
            }
            fn is_readonly(&self) -> bool {
                false
            }
            fn sync(&self) -> Result<(), crate::error::KernelError> {
                Ok(())
            }
        }
        let vfs = make_vfs_with_root();
        let fs = Opaque(crate::fs::ramfs::RamFs::new());
        let f = fs.0.root().create("f", Permissions::default()).unwrap();
        vfs.mount("/o".into(), Arc::new(fs)).unwrap();
        let (_, ty, body) = encode(f.metadata().unwrap().inode, 128, false, false);
        assert!(decode(&vfs.as_caller(), Some("/o"), ty, &body.unwrap()).is_none());
    }
}

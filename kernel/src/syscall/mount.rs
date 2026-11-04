//! mount(2) and umount2, as Linux 7.3 (fs/namespace.c: path_mount and
//! ksys_umount; ADR 0022 H3b), on the caller's mount namespace.
//!
//! The tree operations are `fs::mount`'s; this module copies the arguments
//! in, resolves the paths in the caller's view, checks privilege and maps
//! the flags, in Linux's order: the strings, the target, MS_NOUSER, then
//! CAP_SYS_ADMIN.

use alloc::{string::String, sync::Arc};

use super::{
    filesystem::{read_user_path, vfs},
    userspace::{read_user_cstr, strncpy_from_user},
    SyscallError, SyscallResult,
};
use crate::{
    fs::{
        mount::{self, flags as mnt, MountError, NewFsError, Propagation, Umount},
        NodeType,
    },
    process::{self, creds::Caller, linux_caps::CAP_SYS_ADMIN},
};

/// mount(2)'s flags (include/uapi/linux/mount.h).
mod ms {
    pub const RDONLY: u64 = 1;
    pub const NOSUID: u64 = 2;
    pub const NODEV: u64 = 4;
    pub const NOEXEC: u64 = 8;
    pub const REMOUNT: u64 = 32;
    pub const NOSYMFOLLOW: u64 = 256;
    pub const NOATIME: u64 = 1024;
    pub const NODIRATIME: u64 = 2048;
    pub const BIND: u64 = 4096;
    pub const MOVE: u64 = 8192;
    pub const REC: u64 = 16384;
    pub const SILENT: u64 = 32768;
    pub const UNBINDABLE: u64 = 1 << 17;
    pub const PRIVATE: u64 = 1 << 18;
    pub const SLAVE: u64 = 1 << 19;
    pub const SHARED: u64 = 1 << 20;
    pub const RELATIME: u64 = 1 << 21;
    pub const STRICTATIME: u64 = 1 << 24;
    pub const NOUSER: u64 = 1 << 31;
    pub const MGC_VAL: u64 = 0xC0ED_0000;
    pub const MGC_MSK: u64 = 0xffff_0000;
    pub const PROPAGATION: u64 = SHARED | PRIVATE | SLAVE | UNBINDABLE;
}

/// umount2's flags (include/linux/fs.h).
const MNT_FORCE: usize = 1;
const MNT_DETACH: usize = 2;
const MNT_EXPIRE: usize = 4;
const UMOUNT_NOFOLLOW: usize = 8;

/// Linux's copy_mount_string: none for a null pointer, else the string
/// (PATH_MAX at most).
fn mount_string(ptr: usize) -> Result<Option<String>, SyscallError> {
    if ptr == 0 {
        return Ok(None);
    }
    read_user_cstr(ptr, 4095).map(Some)
}

/// Linux's copy_mount_options: up to a page, here the option string
/// these filesystems take.
fn mount_data(ptr: usize) -> Result<String, SyscallError> {
    if ptr == 0 {
        return Ok(String::new());
    }
    let mut buf = alloc::vec![0u8; 4095];
    let n = strncpy_from_user(ptr, &mut buf)?;
    buf.truncate(n);
    String::from_utf8(buf).map_err(|_| SyscallError::InvalidArgument)
}

fn mount_error(e: MountError) -> SyscallError {
    match e {
        MountError::Invalid => SyscallError::InvalidArgument,
        MountError::Busy => SyscallError::Busy,
        MountError::NotDirectory => SyscallError::NotADirectory,
        MountError::Loop => SyscallError::SymlinkLoop,
        MountError::Again => SyscallError::WouldBlock,
    }
}

/// The mount-point flags path_mount separates from `flags`: relatime
/// unless MS_NOATIME, strictatime clearing both.
fn mount_flags(flags: u64) -> u32 {
    let mut out = 0;
    if flags & ms::NOATIME == 0 {
        out |= mnt::RELATIME;
    }
    for (ms_flag, mnt_flag) in [
        (ms::NOSUID, mnt::NOSUID),
        (ms::NODEV, mnt::NODEV),
        (ms::NOEXEC, mnt::NOEXEC),
        (ms::NOATIME, mnt::NOATIME),
        (ms::NODIRATIME, mnt::NODIRATIME),
        (ms::RDONLY, mnt::READONLY),
        (ms::NOSYMFOLLOW, mnt::NOSYMFOLLOW),
    ] {
        if flags & ms_flag != 0 {
            out |= mnt_flag;
        }
    }
    if flags & ms::STRICTATIME != 0 {
        out &= !(mnt::RELATIME | mnt::NOATIME);
    }
    out
}

/// flags_to_propagation_type: exactly one propagation flag and nothing
/// else but MS_REC and MS_SILENT.
fn propagation(flags: u64) -> Option<Propagation> {
    match flags & !(ms::REC | ms::SILENT) {
        ms::SHARED => Some(Propagation::Shared),
        ms::PRIVATE => Some(Propagation::Private),
        ms::SLAVE => Some(Propagation::Slave),
        ms::UNBINDABLE => Some(Propagation::Unbindable),
        _ => None,
    }
}

/// mount(source, target, fstype, flags, data).
pub fn sys_mount(
    source: usize,
    target: usize,
    fstype: usize,
    flags: usize,
    data: usize,
) -> SyscallResult {
    let fstype = mount_string(fstype)?;
    let source = mount_string(source)?;
    let data = mount_data(data)?;
    let view = vfs()?;
    let target = read_user_path(target)?;
    let (node, point) = view
        .locate(&target, true)
        .map_err(super::map_kernel_error)?;
    let point_is_dir = node.node_type() == NodeType::Directory;

    let mut flags = flags as u64;
    if flags & ms::MGC_MSK == ms::MGC_VAL {
        flags &= !ms::MGC_MSK;
    }
    if flags & ms::NOUSER != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let caller = Caller::current();
    if !caller.capable(CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let tree = view.mount_tree().clone();
    let mut mnt_flags = mount_flags(flags);
    // A remount keeps the atime flags unless it names one.
    if flags & ms::REMOUNT != 0
        && flags & (ms::NOATIME | ms::NODIRATIME | ms::RELATIME | ms::STRICTATIME) == 0
    {
        let old = tree.flags_at(&point).unwrap_or(0);
        mnt_flags = (mnt_flags & !mnt::ATIME) | (old & mnt::ATIME);
    }
    let writers = || writing_under(&point, &tree);
    let result = if flags & (ms::REMOUNT | ms::BIND) == ms::REMOUNT | ms::BIND {
        tree.reconfigure(&point, mnt_flags, None, writers)
    } else if flags & ms::REMOUNT != 0 {
        // Options a remount names are not applied (KNOWN-LIMITATIONS).
        tree.reconfigure(&point, mnt_flags, Some(flags & ms::RDONLY != 0), writers)
    } else if flags & ms::BIND != 0 {
        let from = source
            .filter(|s| !s.is_empty())
            .ok_or(SyscallError::InvalidArgument)?;
        let (from_node, from) = view.locate(&from, true).map_err(super::map_kernel_error)?;
        tree.bind(&from, from_node, &point, point_is_dir, flags & ms::REC != 0)
    } else if flags & ms::PROPAGATION != 0 {
        let ty = propagation(flags).ok_or(SyscallError::InvalidArgument)?;
        tree.change_type(&point, ty, flags & ms::REC != 0)
    } else if flags & ms::MOVE != 0 {
        let from = source
            .filter(|s| !s.is_empty())
            .ok_or(SyscallError::InvalidArgument)?;
        let (from_node, from) = view.locate(&from, true).map_err(super::map_kernel_error)?;
        let from_is_dir = from_node.node_type() == NodeType::Directory;
        tree.move_mount(&from, from_is_dir, &point, point_is_dir)
    } else {
        let ty = fstype.ok_or(SyscallError::InvalidArgument)?;
        let owner = (caller.creds.fsuid, caller.creds.fsgid);
        let fs = mount::new_filesystem(&ty, &data, owner).map_err(|e| match e {
            NewFsError::UnknownType => SyscallError::NoDevice,
            NewFsError::BadOption => SyscallError::InvalidArgument,
        })?;
        let source = source.unwrap_or_else(|| String::from("none"));
        tree.attach_new(
            &point,
            point_is_dir,
            fs,
            source,
            mnt_flags,
            flags & ms::RDONLY != 0,
        )
    };
    result.map(|()| 0).map_err(mount_error)
}

/// umount2(target, flags).
pub fn sys_umount2(target: usize, flags: usize) -> SyscallResult {
    if flags & !(MNT_FORCE | MNT_DETACH | MNT_EXPIRE | UMOUNT_NOFOLLOW) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let view = vfs()?;
    let target = read_user_path(target)?;
    let (_, point) = view
        .locate(&target, flags & UMOUNT_NOFOLLOW == 0)
        .map_err(super::map_kernel_error)?;
    if !Caller::current().capable(CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let how = Umount {
        force: flags & MNT_FORCE != 0,
        detach: flags & MNT_DETACH != 0,
        expire: flags & MNT_EXPIRE != 0,
    };
    let tree = view.mount_tree().clone();
    let busy = !how.detach && in_use(&point, &tree);
    tree.umount(&point, how, view.root_path(), busy)
        .map(|()| 0)
        .map_err(mount_error)
}

/// Whether a process of the namespace whose tree is `tree` has a file
/// under `point` (canonical) open for writing, which keeps the mount from
/// becoming read-only. Linux counts the writers of the mount, or for a
/// read-only superblock of every mount of it; here files are found by the
/// path they were opened at.
fn writing_under(point: &str, tree: &Arc<mount::Mounts>) -> bool {
    scan(point, tree, true)
}

/// Whether a process of the namespace whose tree is `tree` has a working
/// directory, root or open file under `point` (canonical): what pins a
/// mount in Linux, except memory mappings, which are not counted here.
fn in_use(point: &str, tree: &Arc<mount::Mounts>) -> bool {
    scan(point, tree, false)
}

/// [`in_use`], or with `writers` [`writing_under`].
fn scan(point: &str, tree: &Arc<mount::Mounts>, writers: bool) -> bool {
    let under = |path: &str| crate::fs::path_is_under(path, point);
    let mut busy = false;
    process::table::PROCESS_TABLE.for_each(|p| {
        if busy {
            return;
        }
        let mut member = false;
        for thread in p.threads.lock().values() {
            if !Arc::ptr_eq(&thread.namespaces().mnt.mounts, tree) {
                continue;
            }
            member = true;
            if writers {
                break;
            }
            let fs = thread.fs();
            let root = fs.root.lock().clone();
            let cwd = crate::fs::normalize_path(&fs.cwd.lock(), "/");
            let cwd = if root == "/" {
                cwd
            } else if cwd == "/" {
                root.clone()
            } else {
                alloc::format!("{root}{cwd}")
            };
            if under(&root) || under(&cwd) {
                busy = true;
                return;
            }
        }
        if member {
            let table = p.file_table.lock();
            let paths = if writers {
                table.writable_paths()
            } else {
                table.open_paths()
            };
            busy = paths.iter().any(|f| under(f));
        }
    });
    busy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_mount_separates_the_mount_point_flags() {
        assert_eq!(mount_flags(0), mnt::RELATIME);
        assert_eq!(
            mount_flags(ms::RDONLY | ms::NOSUID | ms::NODEV | ms::NOEXEC),
            mnt::RELATIME | mnt::READONLY | mnt::NOSUID | mnt::NODEV | mnt::NOEXEC
        );
        assert_eq!(mount_flags(ms::NOATIME), mnt::NOATIME);
        assert_eq!(mount_flags(ms::STRICTATIME), 0);
        assert_eq!(
            mount_flags(ms::NOSYMFOLLOW),
            mnt::RELATIME | mnt::NOSYMFOLLOW
        );
    }

    #[test]
    fn one_propagation_flag_and_only_rec_or_silent_besides() {
        assert_eq!(
            propagation(ms::PRIVATE | ms::REC),
            Some(Propagation::Private)
        );
        assert_eq!(
            propagation(ms::UNBINDABLE | ms::SILENT),
            Some(Propagation::Unbindable)
        );
        assert_eq!(propagation(ms::PRIVATE | ms::SLAVE), None);
        assert_eq!(propagation(ms::PRIVATE | ms::NOSUID), None);
    }
}

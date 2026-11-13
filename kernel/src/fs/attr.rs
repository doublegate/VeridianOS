//! Inode attribute flags (chattr's flags; FS_IOC_GETFLAGS/SETFLAGS,
//! FS_IOC_FSGETXATTR/FSSETXATTR, file_getattr/file_setattr; ADR 0027).
//!
//! The set is Linux tmpfs's, the maintainer's choice: immutable, append-only,
//! nodump and noatime, kept by BlockFS (the on-disk inode's `flags`), tmpfs,
//! ramfs and memfd, and reported by ext4. Immutable and append-only are
//! enforced where Linux's VFS enforces them (the checks below); nodump is
//! only recorded; noatime stops atime updates. A new file or directory takes
//! nodump and noatime from its directory (SHMEM_FL_INHERITED).

use super::VfsNode;
use crate::error::{FsError, KernelError};

pub const FS_IMMUTABLE_FL: u32 = 0x10;
pub const FS_APPEND_FL: u32 = 0x20;
pub const FS_NODUMP_FL: u32 = 0x40;
pub const FS_NOATIME_FL: u32 = 0x80;
const FS_SYNC_FL: u32 = 0x8;
const FS_DAX_FL: u32 = 0x0200_0000;
const FS_PROJINHERIT_FL: u32 = 0x2000_0000;
const FS_VERITY_FL: u32 = 0x0010_0000;

/// What may be set (SHMEM_FL_USER_MODIFIABLE without casefold).
pub const MODIFIABLE: u32 = FS_IMMUTABLE_FL | FS_APPEND_FL | FS_NODUMP_FL | FS_NOATIME_FL;
/// What a new node takes from its directory.
pub const INHERITED: u32 = FS_NODUMP_FL | FS_NOATIME_FL;
/// What FS_IOC_GETFLAGS may show (FS_FL_USER_VISIBLE).
pub const USER_VISIBLE: u32 = 0x0003_DFFF;

const FS_XFLAG_SYNC: u32 = 0x20;
const FS_XFLAG_IMMUTABLE: u32 = 0x8;
const FS_XFLAG_APPEND: u32 = 0x10;
const FS_XFLAG_NODUMP: u32 = 0x80;
const FS_XFLAG_NOATIME: u32 = 0x40;
const FS_XFLAG_DAX: u32 = 0x8000;
const FS_XFLAG_PROJINHERIT: u32 = 0x200;
const FS_XFLAG_VERITY: u32 = 0x20000;
/// The xflags that are also flags (FS_XFLAG_COMMON).
const XFLAG_COMMON: u32 = FS_XFLAG_SYNC
    | FS_XFLAG_IMMUTABLE
    | FS_XFLAG_APPEND
    | FS_XFLAG_NODUMP
    | FS_XFLAG_NOATIME
    | FS_XFLAG_DAX
    | FS_XFLAG_PROJINHERIT
    | FS_XFLAG_VERITY;
/// The xflags only a filesystem reports, dropped from a request before it
/// is checked (FS_XFLAG_RDONLY_MASK: PREALLOC, VERITY, CASEFOLD,
/// CASENONPRESERVING, HASATTR).
const XFLAG_RDONLY_MASK: u32 = 0x2 | 0x2_0000 | 0x4_0000 | 0x8_0000 | 0x8000_0000;
/// Every xflag Linux defines (FS_XFLAGS_MASK): past it EINVAL
/// (file_setattr) or EOPNOTSUPP (FSSETXATTR).
pub const XFLAGS_MASK: u32 = 0x8007_FFFF;

/// The flag <-> xflag pairs (fileattr_fill_flags, fileattr_fill_xflags).
const PAIRS: [(u32, u32); 8] = [
    (FS_SYNC_FL, FS_XFLAG_SYNC),
    (FS_IMMUTABLE_FL, FS_XFLAG_IMMUTABLE),
    (FS_APPEND_FL, FS_XFLAG_APPEND),
    (FS_NODUMP_FL, FS_XFLAG_NODUMP),
    (FS_NOATIME_FL, FS_XFLAG_NOATIME),
    (FS_DAX_FL, FS_XFLAG_DAX),
    (FS_PROJINHERIT_FL, FS_XFLAG_PROJINHERIT),
    (FS_VERITY_FL, FS_XFLAG_VERITY),
];

pub fn to_xflags(flags: u32) -> u32 {
    PAIRS
        .iter()
        .filter(|(f, _)| flags & f != 0)
        .fold(0, |x, (_, xf)| x | xf)
}

fn from_xflags(xflags: u32) -> u32 {
    PAIRS
        .iter()
        .filter(|(_, xf)| xflags & xf != 0)
        .fold(0, |f, (fl, _)| f | fl)
}

/// A change asked for: FS_IOC_SETFLAGS's flags, or an fsxattr's fields
/// (FSSETXATTR, file_setattr).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Flags(u32),
    Fsx {
        xflags: u32,
        extsize: u32,
        projid: u32,
        cowextsize: u32,
    },
}

/// The flags a node ends with after `change`, from `old` (vfs_fileattr_set
/// keeping the other interface's bits, then shmem_fileattr_set): an fsxattr
/// with anything past the common xflags, or flags past [`MODIFIABLE`], is
/// EOPNOTSUPP, as tmpfs answers. `may_change_immutable` answers
/// CAP_LINUX_IMMUTABLE, which changing immutable or append-only needs
/// (EPERM). The owner check is the caller's.
pub fn apply(old: u32, change: Change, may_change_immutable: bool) -> Result<u32, KernelError> {
    let unsupported = KernelError::FsError(FsError::NotSupported);
    let asked = match change {
        Change::Flags(flags) => flags,
        Change::Fsx {
            xflags,
            extsize,
            projid,
            cowextsize,
        } => {
            let xflags = xflags & !XFLAG_RDONLY_MASK;
            if xflags & !XFLAG_COMMON != 0 || extsize != 0 || projid != 0 || cowextsize != 0 {
                return Err(unsupported);
            }
            // The flags no xflag stands for are kept.
            from_xflags(xflags) | (old & !from_xflags(XFLAG_COMMON))
        }
    };
    if (asked ^ old) & (FS_APPEND_FL | FS_IMMUTABLE_FL) != 0 && !may_change_immutable {
        return Err(KernelError::FsError(FsError::OperationNotPermitted));
    }
    if asked & !MODIFIABLE != 0 {
        return Err(unsupported);
    }
    Ok((old & !MODIFIABLE) | (asked & MODIFIABLE))
}

/// How many attribute changes that make a file immutable are under way:
/// while any is, a write fault on a write-protected shared file page is
/// retried rather than let through (`VirtualAddressSpace::immutable_fault`).
static CHANGING: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

pub fn changing() -> bool {
    CHANGING.load(core::sync::atomic::Ordering::SeqCst) != 0
}

/// Bumped as each change begins: a fork that copied its parent's mappings
/// before a change could have been missed by the change's passes over the
/// process table, and checks this once its child is in it.
static GENERATION: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(core::sync::atomic::Ordering::SeqCst)
}

/// Mark a change under way for as long as the guard lives.
pub fn begin_change() -> ChangeGuard {
    CHANGING.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    GENERATION.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    ChangeGuard
}

pub struct ChangeGuard;

impl Drop for ChangeGuard {
    fn drop(&mut self) {
        CHANGING.fetch_sub(1, core::sync::atomic::Ordering::SeqCst);
    }
}

/// A node's flags; 0 for a filesystem that keeps none.
pub fn of(node: &dyn VfsNode) -> u32 {
    node.attr_flags().unwrap_or(0)
}

fn eperm() -> KernelError {
    KernelError::FsError(FsError::OperationNotPermitted)
}

/// inode_permission's rule for MAY_WRITE: nobody writes an immutable node
/// (EPERM) -- its data, or, for a directory, its entries.
pub fn may_write(node: &dyn VfsNode) -> Result<(), KernelError> {
    if of(node) & FS_IMMUTABLE_FL != 0 {
        Err(eperm())
    } else {
        Ok(())
    }
}

/// may_open's rule: an append-only file opens for writing only with
/// O_APPEND, and never with O_TRUNC.
pub fn may_open(
    node: &dyn VfsNode,
    write: bool,
    append: bool,
    trunc: bool,
) -> Result<(), KernelError> {
    if of(node) & FS_APPEND_FL != 0 && (write && !append || trunc) {
        Err(eperm())
    } else {
        Ok(())
    }
}

/// may_delete's rule (unlink, rmdir, rename's source and replaced target):
/// not out of an append-only directory, and not an append-only or
/// immutable node. Also vfs_link's for the node linked.
pub fn may_remove(dir: Option<&dyn VfsNode>, node: &dyn VfsNode) -> Result<(), KernelError> {
    if dir.is_some_and(|d| of(d) & FS_APPEND_FL != 0)
        || of(node) & (FS_APPEND_FL | FS_IMMUTABLE_FL) != 0
    {
        Err(eperm())
    } else {
        Ok(())
    }
}

/// may_setattr's rule for a mode, owner or explicit times, and truncate's
/// (do_truncate, do_sys_ftruncate), fallocate's and the fcntl clearing of
/// O_APPEND: not on an append-only or immutable node.
pub fn may_change(node: &dyn VfsNode) -> Result<(), KernelError> {
    if of(node) & (FS_APPEND_FL | FS_IMMUTABLE_FL) != 0 {
        Err(eperm())
    } else {
        Ok(())
    }
}

/// Whether reading `node` leaves its atime alone (noatime).
pub fn no_atime(node: &dyn VfsNode) -> bool {
    of(node) & FS_NOATIME_FL != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_and_xflags_translate_both_ways() {
        let f = FS_IMMUTABLE_FL | FS_NODUMP_FL;
        assert_eq!(to_xflags(f), FS_XFLAG_IMMUTABLE | FS_XFLAG_NODUMP);
        assert_eq!(from_xflags(to_xflags(f)), f);
    }

    #[test]
    fn immutable_and_append_need_the_capability() {
        assert_eq!(
            apply(0, Change::Flags(FS_IMMUTABLE_FL), false),
            Err(KernelError::FsError(FsError::OperationNotPermitted))
        );
        assert_eq!(
            apply(0, Change::Flags(FS_IMMUTABLE_FL), true),
            Ok(FS_IMMUTABLE_FL)
        );
        // Clearing them needs it too; nodump and noatime do not.
        assert!(apply(FS_APPEND_FL, Change::Flags(0), false).is_err());
        assert_eq!(
            apply(0, Change::Flags(FS_NODUMP_FL | FS_NOATIME_FL), false),
            Ok(0xC0)
        );
        // Leaving them as they are needs nothing.
        assert_eq!(
            apply(
                FS_APPEND_FL,
                Change::Flags(FS_APPEND_FL | FS_NODUMP_FL),
                false
            ),
            Ok(FS_APPEND_FL | FS_NODUMP_FL)
        );
    }

    #[test]
    fn what_tmpfs_does_not_keep_is_eopnotsupp() {
        let unsupported = Err(KernelError::FsError(FsError::NotSupported));
        assert_eq!(apply(0, Change::Flags(FS_SYNC_FL), true), unsupported);
        let fsx = |xflags, projid| Change::Fsx {
            xflags,
            extsize: 0,
            projid,
            cowextsize: 0,
        };
        assert_eq!(
            apply(0, fsx(0x1, 0), true),
            unsupported,
            "REALTIME, not common"
        );
        assert_eq!(apply(0, fsx(0, 7), true), unsupported, "a project ID");
        assert_eq!(
            apply(0, fsx(0x8000_0002, 0), true),
            Ok(0),
            "read-only xflags dropped"
        );
        assert_eq!(
            apply(0, fsx(FS_XFLAG_NOATIME | FS_XFLAG_APPEND, 0), true),
            Ok(FS_NOATIME_FL | FS_APPEND_FL)
        );
    }
}

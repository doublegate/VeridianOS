//! Simple file calls (group 3, step D1), each as Linux 7.3 has it:
//! creat, lchown and fchmodat2 (`fs/open.c`); getdents (`fs/readdir.c`);
//! preadv, pwritev, preadv2 and pwritev2 (`fs/read_write.c`); close_range
//! (`fs/file.c`); syncfs and sync_file_range (`fs/sync.c`); readahead
//! (`mm/readahead.c`); fadvise64 (`mm/fadvise.c`); ustat (`fs/statfs.c`);
//! and sysfs (`fs/filesystems.c`). There is no page cache, so the calls
//! that advise it check their arguments and do nothing else, and the sync
//! calls sync everything, as fsync does.

use alloc::{sync::Arc, vec::Vec};

use super::{
    filesystem::{
        self, file_size, pread_segs, pwrite_segs, read_iovecs, read_user_path_or_empty,
        AT_EMPTY_PATH, AT_FDCWD, AT_SYMLINK_NOFOLLOW,
    },
    userspace, xattr, SyscallError, SyscallResult,
};
use crate::fs::{file::File, NodeType};

/// The open file `fd` (an `int`), EBADF if there is none.
pub(super) fn open_file(fd: usize) -> Result<Arc<File>, SyscallError> {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    // Lookup only; the lock is not held over the operation (N-118).
    let file = proc.file_table.lock().get(fd as u32 as usize);
    file.ok_or(SyscallError::BadFileDescriptor)
}

/// The file `fd` names, O_PATH descriptors included
/// ([`crate::fs::file::FileTable::get_path`]), EBADF if there is none.
pub(super) fn path_file(fd: usize) -> Result<Arc<File>, SyscallError> {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc.file_table.lock().get_path(fd as u32 as usize);
    file.ok_or(SyscallError::BadFileDescriptor)
}

/// A 64-bit file offset from a register (`loff_t`).
fn loff(value: usize) -> i64 {
    value as i64
}

// ---------------------------------------------------------------------------
// creat, lchown, fchmodat2
// ---------------------------------------------------------------------------

/// creat(path, mode): open(path, O_CREAT | O_WRONLY | O_TRUNC, mode).
pub fn sys_creat(path_ptr: usize, mode: usize) -> SyscallResult {
    const O_WRONLY: usize = 0x1;
    const O_CREAT: usize = 0x40;
    const O_TRUNC: usize = 0x200;
    filesystem::sys_open(path_ptr, O_CREAT | O_WRONLY | O_TRUNC, mode)
}

/// lchown(path, uid, gid): fchownat(AT_FDCWD, path, uid, gid,
/// AT_SYMLINK_NOFOLLOW), so a final symlink is changed itself.
pub fn sys_lchown(path_ptr: usize, uid: usize, gid: usize) -> SyscallResult {
    super::sys_fchownat(AT_FDCWD, path_ptr, uid, gid, AT_SYMLINK_NOFOLLOW)
}

/// fchmodat2(dirfd, path, mode, flags): fchmodat with AT_SYMLINK_NOFOLLOW
/// (a symlink's mode cannot change: EOPNOTSUPP) and AT_EMPTY_PATH (the
/// mode of `dirfd` itself, or of the cwd); other flags are EINVAL.
pub fn sys_fchmodat2(dirfd: usize, path_ptr: usize, mode: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32 as usize;
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (node, at) = xattr::path_target_located(dirfd, path_ptr, flags)?;
    filesystem::chmod_node(&node, mode)?;
    crate::fs::inotify::on_target(at.as_deref(), &node, crate::fs::inotify::IN_ATTRIB);
    Ok(0)
}

// ---------------------------------------------------------------------------
// getdents
// ---------------------------------------------------------------------------

/// getdents(fd, dirp, count): getdents64 with the old `struct
/// linux_dirent` records (x86_64 only).
pub fn sys_getdents(fd: usize, buf_ptr: usize, buf_size: usize) -> SyscallResult {
    super::getdents(fd, buf_ptr, buf_size, super::DirentForm::Dirent)
}

// ---------------------------------------------------------------------------
// preadv, pwritev, preadv2, pwritev2
// ---------------------------------------------------------------------------

const RWF_HIPRI: u32 = 0x01;
const RWF_DSYNC: u32 = 0x02;
const RWF_SYNC: u32 = 0x04;
const RWF_NOWAIT: u32 = 0x08;
const RWF_APPEND: u32 = 0x10;
const RWF_NOAPPEND: u32 = 0x20;
const RWF_ATOMIC: u32 = 0x40;
const RWF_DONTCACHE: u32 = 0x80;
const RWF_NOSIGNAL: u32 = 0x100;
const RWF_SUPPORTED: u32 = RWF_HIPRI
    | RWF_DSYNC
    | RWF_SYNC
    | RWF_NOWAIT
    | RWF_APPEND
    | RWF_NOAPPEND
    | RWF_ATOMIC
    | RWF_DONTCACHE
    | RWF_NOSIGNAL;

/// What the RWF_* flags of a preadv2 or pwritev2 ask for, once checked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Rwf {
    /// RWF_DSYNC or RWF_SYNC: sync once the data is written.
    sync: bool,
    /// RWF_APPEND: write at the end of the file.
    append: bool,
    /// RWF_NOAPPEND: write where asked even on an O_APPEND file.
    noappend: bool,
    /// RWF_NOSIGNAL: EPIPE without SIGPIPE.
    nosignal: bool,
}

impl Rwf {
    /// Check `flags` for a read or a write on a file that is a stream or
    /// not, as Linux's kiocb_set_rw_flags: an unknown flag is EOPNOTSUPP,
    /// RWF_APPEND with RWF_NOAPPEND EINVAL. RWF_ATOMIC and RWF_DONTCACHE
    /// need support no filesystem here has (EOPNOTSUPP). RWF_NOWAIT is
    /// kept on files, whose I/O never waits; on a stream it is EOPNOTSUPP
    /// (Linux waits for nothing there instead; see KNOWN-LIMITATIONS).
    /// RWF_HIPRI is a hint.
    fn parse(flags: u32, write: bool, stream: bool) -> Result<Self, SyscallError> {
        if flags == 0 {
            return Ok(Self::default());
        }
        if flags & !RWF_SUPPORTED != 0 {
            return Err(SyscallError::NotSupported);
        }
        if flags & RWF_APPEND != 0 && flags & RWF_NOAPPEND != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        if flags & RWF_NOWAIT != 0 && stream {
            return Err(SyscallError::NotSupported);
        }
        if flags & RWF_ATOMIC != 0 || flags & RWF_DONTCACHE != 0 {
            return Err(SyscallError::NotSupported);
        }
        Ok(Self {
            sync: write && flags & (RWF_DSYNC | RWF_SYNC) != 0,
            append: flags & RWF_APPEND != 0,
            noappend: flags & RWF_NOAPPEND != 0,
            nosignal: flags & RWF_NOSIGNAL != 0,
        })
    }
}

/// preadv(fd, iov, iovcnt, pos_l, pos_h): readv at offset `pos_l` (on a
/// 64-bit kernel `pos_h` is shifted out entirely, as Linux's
/// pos_from_hilo), leaving the file position alone.
pub fn sys_preadv(fd: usize, iov_ptr: usize, iovcnt: usize, pos_l: usize) -> SyscallResult {
    preadv_at(fd, iov_ptr, iovcnt, loff(pos_l), 0)
}

/// preadv2(fd, iov, iovcnt, pos_l, pos_h, flags): preadv with RWF_*
/// flags; an offset of -1 reads at the file position, as readv.
pub fn sys_preadv2(
    fd: usize,
    iov_ptr: usize,
    iovcnt: usize,
    pos_l: usize,
    flags: usize,
) -> SyscallResult {
    let flags = flags as u32;
    if loff(pos_l) != -1 {
        return preadv_at(fd, iov_ptr, iovcnt, loff(pos_l), flags);
    }
    let file = open_file(fd)?;
    if !file.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    Rwf::parse(flags, false, file.node.is_stream())?;
    filesystem::sys_readv(fd, iov_ptr, iovcnt)
}

/// The vectored read at `pos` of preadv and preadv2, in Linux's order:
/// a negative offset is EINVAL; no fd EBADF; a stream ESPIPE; a file not
/// open for reading EBADF; then the iovec array (EINVAL, EFAULT) and the
/// flags. The buffers are filled as one read.
fn preadv_at(fd: usize, iov_ptr: usize, iovcnt: usize, pos: i64, flags: u32) -> SyscallResult {
    if pos < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let file = filesystem::positioned_file(fd as u32 as usize, pos as usize)?;
    if !file.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    let iovs = read_iovecs(iov_ptr, iovcnt)?;
    if iovs.iter().all(|&(_, len)| len == 0) {
        return Ok(0);
    }
    Rwf::parse(flags, false, false)?;
    pread_segs(&file, userspace::UserSegs::new(&iovs), pos as usize)
}

/// pwritev(fd, iov, iovcnt, pos_l, pos_h): writev at offset `pos_l`,
/// leaving the file position alone; an O_APPEND file is appended to, as
/// on Linux.
pub fn sys_pwritev(fd: usize, iov_ptr: usize, iovcnt: usize, pos_l: usize) -> SyscallResult {
    pwritev_at(fd, iov_ptr, iovcnt, loff(pos_l), 0)
}

/// pwritev2(fd, iov, iovcnt, pos_l, pos_h, flags): pwritev with RWF_*
/// flags (musl's pwrite and pwritev pass RWF_NOAPPEND, to write where
/// they are asked as POSIX says); an offset of -1 writes at the file
/// position, as writev.
pub fn sys_pwritev2(
    fd: usize,
    iov_ptr: usize,
    iovcnt: usize,
    pos_l: usize,
    flags: usize,
) -> SyscallResult {
    let flags = flags as u32;
    if loff(pos_l) != -1 {
        return pwritev_at(fd, iov_ptr, iovcnt, loff(pos_l), flags);
    }
    let file = open_file(fd)?;
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let stream = file.node.is_stream();
    let rwf = Rwf::parse(flags, true, stream)?;
    // RWF_APPEND, or RWF_NOAPPEND on an O_APPEND file, moves a file's
    // write away from where writev puts it; the position then follows the
    // data, as Linux's do_writev.
    let at = if stream {
        None
    } else if rwf.append {
        Some(file_size(&file)?)
    } else if rwf.noappend && file.is_append() {
        Some(file.tell())
    } else {
        None
    };
    let written = match at {
        Some(at) => {
            let n = pwritev_file(&file, iov_ptr, iovcnt, at)?;
            let _ = file.seek(crate::fs::SeekFrom::Start(at + n));
            n
        }
        None => filesystem::writev(fd, iov_ptr, iovcnt, rwf.nosignal)?,
    };
    if rwf.sync {
        filesystem::sys_sync()?;
    }
    Ok(written)
}

/// The vectored write at `pos` of pwritev and pwritev2, checked in the
/// same order as preadv_at; RWF_APPEND, or an O_APPEND file without
/// RWF_NOAPPEND, writes at the end instead.
fn pwritev_at(fd: usize, iov_ptr: usize, iovcnt: usize, pos: i64, flags: u32) -> SyscallResult {
    if pos < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let file = filesystem::positioned_file(fd as u32 as usize, pos as usize)?;
    if !file.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let iovs = read_iovecs(iov_ptr, iovcnt)?;
    if iovs.iter().all(|&(_, len)| len == 0) {
        return Ok(0);
    }
    let rwf = Rwf::parse(flags, true, false)?;
    let at = if rwf.append || (file.is_append() && !rwf.noappend) {
        file_size(&file)?
    } else {
        pos as usize
    };
    let written = write_iovs(&file, &iovs, at)?;
    if rwf.sync {
        filesystem::sys_sync()?;
    }
    Ok(written)
}

/// Write the iovec array at `iov_ptr` at `at` of `file`.
fn pwritev_file(file: &File, iov_ptr: usize, iovcnt: usize, at: usize) -> SyscallResult {
    let iovs = read_iovecs(iov_ptr, iovcnt)?;
    write_iovs(file, &iovs, at)
}

/// Write `iovs` at `at` of `file` as one write: RLIMIT_FSIZE applies to
/// them together, as for writev (only a write that starts at the limit is
/// refused, EFBIG and SIGXFSZ, and the data stops where it is reached),
/// and the file loses its set-ID bits once.
fn write_iovs(file: &File, iovs: &[(usize, usize)], at: usize) -> SyscallResult {
    pwrite_segs(file, userspace::UserSegs::new(iovs), at)
}

// ---------------------------------------------------------------------------
// close_range
// ---------------------------------------------------------------------------

const CLOSE_RANGE_UNSHARE: u32 = 1 << 1;
const CLOSE_RANGE_CLOEXEC: u32 = 1 << 2;

/// close_range(fd, max_fd, flags): close the open descriptors from `fd`
/// to `max_fd`, or with CLOSE_RANGE_CLOEXEC mark them close-on-exec.
/// Unknown flags, or `fd` past `max_fd`, are EINVAL. CLOSE_RANGE_UNSHARE
/// first gives the caller a table of its own if it shares one; the threads
/// of a process share one table here, so a process with more than one
/// thread is refused (EINVAL; see KNOWN-LIMITATIONS), and a process with
/// one has nothing to unshare. Errors closing a descriptor are ignored,
/// as on Linux.
pub fn sys_close_range(fd: usize, max_fd: usize, flags: usize) -> SyscallResult {
    let (first, last, flags) = (fd as u32, max_fd as u32, flags as u32);
    if flags & !(CLOSE_RANGE_UNSHARE | CLOSE_RANGE_CLOEXEC) != 0 || first > last {
        return Err(SyscallError::InvalidArgument);
    }
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    #[cfg(feature = "alloc")]
    if flags & CLOSE_RANGE_UNSHARE != 0 && proc.thread_count() > 1 {
        return Err(SyscallError::InvalidArgument);
    }
    let fds = proc
        .file_table
        .lock()
        .open_in(first as usize, last as usize);
    for fd in fds {
        if flags & CLOSE_RANGE_CLOEXEC != 0 {
            let _ = proc.file_table.lock().set_cloexec(fd, true);
        } else {
            let _ = filesystem::sys_close(fd);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// syncfs, sync_file_range, readahead, fadvise64
// ---------------------------------------------------------------------------

/// syncfs(fd): sync the filesystem holding `fd`. Every filesystem is
/// synced, as fsync does here.
pub fn sys_syncfs(fd: usize) -> SyscallResult {
    open_file(fd)?;
    filesystem::sys_sync()
}

const SYNC_FILE_RANGE_WAIT_BEFORE: u32 = 1;
const SYNC_FILE_RANGE_WRITE: u32 = 2;
const SYNC_FILE_RANGE_WAIT_AFTER: u32 = 4;

/// The range check of sync_file_range: unknown flags, a negative offset,
/// or a range that ends before it starts or past the largest offset, are
/// EINVAL.
fn check_sync_range(offset: i64, nbytes: i64, flags: u32) -> Result<(), SyscallError> {
    let valid = SYNC_FILE_RANGE_WAIT_BEFORE | SYNC_FILE_RANGE_WRITE | SYNC_FILE_RANGE_WAIT_AFTER;
    let end = (offset as u64).wrapping_add(nbytes as u64) as i64;
    if flags & !valid != 0 || offset < 0 || end < 0 || end < offset {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// sync_file_range(fd, offset, nbytes, flags): start writing back, or
/// wait for, a range of a file. A pipe, socket or other stream is ESPIPE.
/// SYNC_FILE_RANGE_WRITE syncs everything; waiting has nothing to wait
/// for, since a sync here is complete when it returns.
pub fn sys_sync_file_range(fd: usize, offset: usize, nbytes: usize, flags: usize) -> SyscallResult {
    let file = open_file(fd)?;
    let flags = flags as u32;
    check_sync_range(loff(offset), loff(nbytes), flags)?;
    if !matches!(
        file.node.node_type(),
        NodeType::File | NodeType::BlockDevice | NodeType::Directory
    ) {
        return Err(SyscallError::IllegalSeek);
    }
    if flags & SYNC_FILE_RANGE_WRITE != 0 {
        filesystem::sys_sync()?;
    }
    Ok(0)
}

/// The POSIX_FADV_* advice values (NORMAL, RANDOM, SEQUENTIAL, WILLNEED,
/// DONTNEED, NOREUSE).
const POSIX_FADV_MAX: u32 = 5;
const POSIX_FADV_WILLNEED: u32 = 3;

/// fadvise for an open file of `node_type`, as Linux's generic_fadvise on
/// a filesystem without readahead state to keep: a pipe or FIFO is
/// ESPIPE; a negative offset or length, or unknown advice, EINVAL; any
/// other advice is taken and changes nothing.
fn fadvise(node_type: NodeType, offset: i64, len: i64, advice: u32) -> SyscallResult {
    if node_type == NodeType::Pipe {
        return Err(SyscallError::IllegalSeek);
    }
    if offset < 0 || len < 0 || advice > POSIX_FADV_MAX {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(0)
}

/// fadvise64(fd, offset, len, advice) (posix_fadvise).
pub fn sys_fadvise64(fd: usize, offset: usize, len: usize, advice: usize) -> SyscallResult {
    let file = open_file(fd)?;
    fadvise(
        file.node.node_type(),
        loff(offset),
        loff(len),
        advice as u32,
    )
}

/// readahead(fd, offset, count): fadvise WILLNEED on a regular file or a
/// block device open for reading: no fd, or one not open for reading, is
/// EBADF; any other kind of file EINVAL.
pub fn sys_readahead(fd: usize, offset: usize, count: usize) -> SyscallResult {
    let file = open_file(fd)?;
    if !file.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    let node_type = file.node.node_type();
    if !matches!(node_type, NodeType::File | NodeType::BlockDevice) {
        return Err(SyscallError::InvalidArgument);
    }
    fadvise(node_type, loff(offset), loff(count), POSIX_FADV_WILLNEED)
}

// ---------------------------------------------------------------------------
// ustat, sysfs
// ---------------------------------------------------------------------------

/// Size of `struct ustat` on x86_64: f_tfree (int), f_tinode (unsigned
/// long), f_fname[6], f_fpack[6], padded to 8 bytes.
const USTAT_SIZE: usize = 32;

/// `struct ustat` for a filesystem with `stats`: its free blocks (cut to
/// an int, as Linux's assignment) and free inodes; the names are empty.
fn ustat_bytes(stats: &crate::fs::statfs::FsStats) -> [u8; USTAT_SIZE] {
    let mut out = [0u8; USTAT_SIZE];
    out[0..4].copy_from_slice(&(stats.blocks_free as i32).to_ne_bytes());
    out[8..16].copy_from_slice(&stats.files_free.to_ne_bytes());
    out
}

/// ustat(dev, ubuf) (x86_64 only): free space on the filesystem whose
/// files stat reports as on device `dev` (each filesystem's own,
/// `fs::mount_dev`; the root's is 0:1); a device with no filesystem is EINVAL.
pub fn sys_ustat(dev: usize, ubuf: usize) -> SyscallResult {
    // The filesystem whose files stat reports as on `dev` (N-205): EINVAL
    // for none, as Linux's user_get_super finding no superblock.
    let fs = filesystem::vfs()?
        .filesystem_with_dev(dev as u32)
        .ok_or(SyscallError::InvalidArgument)?;
    let stats = fs.root().statfs().map_err(super::map_kernel_error)?;
    userspace::write_user_bytes(ubuf, &ustat_bytes(&stats))?;
    Ok(0)
}

/// The filesystem types this kernel has, by their Linux names, in the
/// order sysfs numbers them (blockfs is VeridianOS's own).
const FILESYSTEM_TYPES: &[&str] = &[
    "ramfs", "tmpfs", "proc", "devtmpfs", "ext4", "vfat", "blockfs",
];

/// sysfs(option, arg1, arg2) (x86_64 only), the System V call: option 1
/// is the index of the type named at `arg1` (EINVAL for none), option 2
/// copies the name of type `arg1` to `arg2` (EINVAL past the last), and
/// option 3 is the number of types; other options are EINVAL.
pub fn sys_sysfs(option: usize, arg1: usize, arg2: usize) -> SyscallResult {
    match option as i32 {
        1 => {
            let name = read_user_path_or_empty(arg1)?;
            FILESYSTEM_TYPES
                .iter()
                .position(|&t| t == name)
                .ok_or(SyscallError::InvalidArgument)
        }
        2 => {
            let name = FILESYSTEM_TYPES
                .get(arg1 as u32 as usize)
                .ok_or(SyscallError::InvalidArgument)?;
            let mut bytes: Vec<u8> = name.as_bytes().to_vec();
            bytes.push(0);
            userspace::write_user_bytes(arg2, &bytes)?;
            Ok(0)
        }
        3 => Ok(FILESYSTEM_TYPES.len()),
        _ => Err(SyscallError::InvalidArgument),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RWF flags are checked as Linux's kiocb_set_rw_flags.
    #[test]
    fn rwf_flags_follow_linux() {
        assert_eq!(Rwf::parse(0, true, false), Ok(Rwf::default()));
        assert_eq!(
            Rwf::parse(0x200, false, false),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            Rwf::parse(u32::MAX, false, false),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            Rwf::parse(RWF_APPEND | RWF_NOAPPEND, true, false),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            Rwf::parse(RWF_ATOMIC, true, false),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            Rwf::parse(RWF_DONTCACHE, false, false),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            Rwf::parse(RWF_NOWAIT, false, true),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            Rwf::parse(RWF_NOWAIT | RWF_HIPRI, false, false),
            Ok(Rwf::default())
        );
        let w = Rwf::parse(RWF_DSYNC | RWF_NOAPPEND | RWF_NOSIGNAL, true, false).unwrap();
        assert!(w.sync && w.noappend && w.nosignal && !w.append);
        // A read does not sync.
        assert!(!Rwf::parse(RWF_SYNC, false, false).unwrap().sync);
        assert!(Rwf::parse(RWF_APPEND, true, false).unwrap().append);
    }

    /// sync_file_range's range and flag checks.
    #[test]
    fn sync_range_checks() {
        assert_eq!(check_sync_range(0, 0, 0), Ok(()));
        assert_eq!(check_sync_range(4096, 4096, 7), Ok(()));
        assert_eq!(
            check_sync_range(0, 0, 8),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            check_sync_range(-1, 0, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            check_sync_range(0, -2, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            check_sync_range(i64::MAX, 1, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            check_sync_range(10, -5, 0),
            Err(SyscallError::InvalidArgument)
        );
    }

    /// fadvise: ESPIPE for a pipe before anything else, EINVAL for a
    /// negative range or unknown advice, else nothing to do.
    #[test]
    fn fadvise_follows_generic_fadvise() {
        assert_eq!(fadvise(NodeType::File, 0, 0, 0), Ok(0));
        assert_eq!(fadvise(NodeType::File, 0, 0, 5), Ok(0));
        assert_eq!(fadvise(NodeType::Socket, 0, 0, 4), Ok(0));
        assert_eq!(
            fadvise(NodeType::File, 0, 0, 6),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            fadvise(NodeType::File, -1, 0, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            fadvise(NodeType::File, 0, -1, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            fadvise(NodeType::Pipe, -1, 0, 9),
            Err(SyscallError::IllegalSeek)
        );
    }

    /// struct ustat: f_tfree at 0 (an int), f_tinode at 8.
    #[test]
    fn ustat_layout() {
        let stats = crate::fs::statfs::FsStats {
            blocks_free: 0x1_0000_0005,
            files_free: 77,
            ..Default::default()
        };
        let b = ustat_bytes(&stats);
        assert_eq!(i32::from_ne_bytes(b[0..4].try_into().unwrap()), 5);
        assert_eq!(u64::from_ne_bytes(b[8..16].try_into().unwrap()), 77);
        assert!(b[16..].iter().all(|&x| x == 0));
    }

    /// sysfs knows "proc" and has no "ext0".
    #[test]
    fn sysfs_types() {
        assert!(FILESYSTEM_TYPES.contains(&"proc"));
        assert!(!FILESYSTEM_TYPES.contains(&"ext0"));
        assert!(FILESYSTEM_TYPES.iter().all(|t| t.len() < 16));
    }
}

//! fallocate(2): the mode a caller asks for (`include/uapi/linux/falloc.h`)
//! and the checks Linux's `vfs_fallocate` makes on it before a filesystem
//! sees the request (`VfsNode::fallocate`).
//!
//! A filesystem implements the operations its Linux counterpart does and
//! answers EOPNOTSUPP for the rest: tmpfs and memfd allocate and punch holes
//! (Linux's `shmem_fallocate`), BlockFS also zeroes ranges, and ramfs has no
//! fallocate at all.

use crate::error::{FsError, KernelError};

/// Allocate without changing the file size.
pub const KEEP_SIZE: u32 = 0x01;
/// Deallocate a range (with KEEP_SIZE only).
pub const PUNCH_HOLE: u32 = 0x02;
/// Reserved by Linux; never accepted.
pub const NO_HIDE_STALE: u32 = 0x04;
pub const COLLAPSE_RANGE: u32 = 0x08;
pub const ZERO_RANGE: u32 = 0x10;
pub const INSERT_RANGE: u32 = 0x20;
pub const UNSHARE_RANGE: u32 = 0x40;
pub const WRITE_ZEROES: u32 = 0x80;

/// The operation bits; at most one may be set (none: allocate).
const MODE_MASK: u32 =
    PUNCH_HOLE | COLLAPSE_RANGE | ZERO_RANGE | INSERT_RANGE | UNSHARE_RANGE | WRITE_ZEROES;

/// What fallocate is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Allocate the range; bytes not yet written read as zero.
    Allocate,
    /// Deallocate the range: it reads as zero afterwards.
    PunchHole,
    /// Make the range read as zero, allocated.
    ZeroRange,
    /// Collapse, insert, unshare or write zeroes: valid requests that no
    /// VeridianOS filesystem implements (EOPNOTSUPP from the filesystem).
    Other(u32),
}

/// One fallocate request on `offset..offset + len`, both already checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub op: Op,
    /// FALLOC_FL_KEEP_SIZE: the file size does not change.
    pub keep_size: bool,
    pub offset: usize,
    pub len: usize,
}

impl Request {
    /// The end of the range (no overflow: [`Request::parse`] checked it).
    pub fn end(&self) -> usize {
        self.offset + self.len
    }

    /// The size the file has afterwards, given its size now.
    pub fn new_size(&self, size: usize) -> usize {
        match self.op {
            Op::Allocate | Op::ZeroRange if !self.keep_size => size.max(self.end()),
            _ => size,
        }
    }

    /// The checks `vfs_fallocate` makes on the arguments alone, in its
    /// order: a negative offset or a length that is not positive is EINVAL;
    /// unknown bits, two operations, PUNCH_HOLE without KEEP_SIZE or
    /// KEEP_SIZE with collapse, insert or write-zeroes are EOPNOTSUPP; a
    /// range past the largest file offset is EFBIG. The descriptor and the
    /// file are checked between the mode and the range
    /// ([`Request::check_range`]).
    pub fn parse(mode: u32, offset: i64, len: i64) -> Result<Self, FsError> {
        if offset < 0 || len <= 0 {
            // EINVAL (FsError::InvalidPath is the EINVAL of fs errors).
            return Err(FsError::InvalidPath);
        }
        if mode & !(MODE_MASK | KEEP_SIZE) != 0 {
            return Err(FsError::NotSupported);
        }
        let keep_size = mode & KEEP_SIZE != 0;
        let op = match mode & MODE_MASK {
            0 => Op::Allocate,
            UNSHARE_RANGE => Op::Other(UNSHARE_RANGE),
            ZERO_RANGE => Op::ZeroRange,
            PUNCH_HOLE if keep_size => Op::PunchHole,
            m @ (COLLAPSE_RANGE | INSERT_RANGE | WRITE_ZEROES) if !keep_size => Op::Other(m),
            _ => return Err(FsError::NotSupported),
        };
        Ok(Self {
            op,
            keep_size,
            offset: offset as usize,
            len: len as usize,
        })
    }

    /// EFBIG for a range that wraps or ends past `i64::MAX` (Linux's
    /// `s_maxbytes` check; a filesystem with a smaller limit refuses it
    /// with EFBIG itself).
    pub fn check_range(&self) -> Result<(), FsError> {
        match self.offset.checked_add(self.len) {
            Some(end) if end <= i64::MAX as usize => Ok(()),
            _ => Err(FsError::FileTooLarge),
        }
    }
}

/// What a filesystem does once it accepts `request` on `node`, before it
/// changes anything: an allocation that may end past RLIMIT_FSIZE is EFBIG
/// with SIGXFSZ, even with KEEP_SIZE (`shmem_fallocate`'s
/// `inode_newsize_ok`), and, as for a write, the file loses its set-ID bits
/// and capabilities (`file_modified`). Not under the filesystem's lock: it
/// may change the file's mode.
pub fn begin(node: &dyn super::VfsNode, request: &Request) -> Result<(), KernelError> {
    if matches!(request.op, Op::Allocate | Op::ZeroRange) {
        super::check_fsize_truncate(node, request.end())?;
    }
    super::remove_privs(node)
}

/// EOPNOTSUPP, for an operation a filesystem does not do.
pub fn unsupported() -> KernelError {
    KernelError::FsError(FsError::NotSupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_checked_as_vfs_fallocate_does() {
        let op = |mode| Request::parse(mode, 0, 1).map(|r| (r.op, r.keep_size));
        assert_eq!(op(0), Ok((Op::Allocate, false)));
        assert_eq!(op(KEEP_SIZE), Ok((Op::Allocate, true)));
        assert_eq!(op(PUNCH_HOLE | KEEP_SIZE), Ok((Op::PunchHole, true)));
        assert_eq!(op(ZERO_RANGE), Ok((Op::ZeroRange, false)));
        assert_eq!(op(ZERO_RANGE | KEEP_SIZE), Ok((Op::ZeroRange, true)));
        assert_eq!(op(COLLAPSE_RANGE), Ok((Op::Other(COLLAPSE_RANGE), false)));
        assert_eq!(
            op(UNSHARE_RANGE | KEEP_SIZE),
            Ok((Op::Other(UNSHARE_RANGE), true))
        );
        for bad in [
            PUNCH_HOLE,
            NO_HIDE_STALE,
            PUNCH_HOLE | ZERO_RANGE | KEEP_SIZE,
            COLLAPSE_RANGE | KEEP_SIZE,
            INSERT_RANGE | KEEP_SIZE,
            WRITE_ZEROES | KEEP_SIZE,
            0x100,
        ] {
            assert_eq!(op(bad), Err(FsError::NotSupported), "mode {bad:#x}");
        }
    }

    #[test]
    fn a_bad_range_is_einval_before_the_mode_and_efbig_after() {
        assert_eq!(Request::parse(0x100, -1, 1), Err(FsError::InvalidPath));
        assert_eq!(Request::parse(0, 0, 0), Err(FsError::InvalidPath));
        assert_eq!(Request::parse(0, 0, -5), Err(FsError::InvalidPath));
        let r = Request::parse(0, i64::MAX, 1).unwrap();
        assert_eq!(r.check_range(), Err(FsError::FileTooLarge));
        let r = Request::parse(0, i64::MAX - 1, 1).unwrap();
        assert_eq!(r.check_range(), Ok(()));
    }

    #[test]
    fn only_allocation_without_keep_size_grows_the_file() {
        let r = |mode| Request::parse(mode, 100, 50).unwrap();
        assert_eq!(r(0).new_size(10), 150);
        assert_eq!(r(0).new_size(400), 400);
        assert_eq!(r(KEEP_SIZE).new_size(10), 10);
        assert_eq!(r(ZERO_RANGE).new_size(10), 150);
        assert_eq!(r(PUNCH_HOLE | KEEP_SIZE).new_size(10), 10);
    }
}

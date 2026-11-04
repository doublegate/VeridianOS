//! Moving data between open files without a user buffer (group 3, step
//! D2), as Linux 7.3 has it: sendfile and copy_file_range
//! (`fs/read_write.c`), splice, tee and vmsplice (`fs/splice.c`).
//!
//! Pipes here hold bytes, not page references, so the data goes through a
//! kernel buffer: splice copies where Linux moves pages, and tee copies
//! where Linux shares them. What a caller sees -- the checks and their
//! order, the counts, when a call waits and what ends the wait -- is
//! Linux's. A pipe is read through [`VfsNode::pipe_splice`], which removes
//! only the bytes the destination took, so a failed write loses nothing.
//!
//! [`VfsNode::pipe_splice`]: crate::fs::VfsNode::pipe_splice

use alloc::{vec, vec::Vec};

use super::{
    file_calls::open_file,
    filesystem::{self, read_iovecs},
    userspace, SyscallError, SyscallResult,
};
use crate::{
    error::KernelError,
    fs::{file::File, NodeType},
};

const SPLICE_F_NONBLOCK: u32 = 0x02;
/// SPLICE_F_MOVE, SPLICE_F_NONBLOCK, SPLICE_F_MORE and SPLICE_F_GIFT; MOVE,
/// MORE and GIFT are hints with nothing to do here.
const SPLICE_F_ALL: u32 = 0x0f;

/// The most one call moves (Linux's MAX_RW_COUNT).
const MAX_RW_COUNT: usize = 0x7fff_f000;
/// The kernel buffer one step copies through.
const CHUNK: usize = 64 * 1024;
/// The largest file offset (Linux's MAX_LFS_FILESIZE, every filesystem's
/// s_maxbytes here).
const MAX_FILE_SIZE: i64 = i64::MAX;

const POLLIN: u16 = 0x001;
const POLLOUT: u16 = 0x004;
const POLLERR: u16 = 0x008;
const POLLHUP: u16 = 0x010;

fn map(e: KernelError) -> SyscallError {
    super::map_kernel_error(e)
}

/// Whether `file` is an end of a pipe or FIFO.
fn is_pipe(file: &File) -> bool {
    file.node.pipe_id().is_some()
}

/// Whether `file` has a position (and so takes an offset): not a pipe,
/// socket or other stream.
fn seekable(file: &File) -> bool {
    !file.node.is_stream()
}

fn position(file: &File) -> i64 {
    *file.position.read() as i64
}

fn set_position(file: &File, pos: i64) {
    *file.position.write() = pos as usize;
}

/// Sleep until `file` reports one of `mask`, or a signal (EINTR). Outside
/// a dispatched thread nothing can wake it: EAGAIN.
fn wait(file: &File, mask: u16, retry: &mut bool) -> Result<(), SyscallError> {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return filesystem::wait_ready(file, mask, retry);
    }
    let _ = (file, mask, retry);
    Err(SyscallError::WouldBlock)
}

/// Linux's rw_verify_area for `count` bytes at `pos`: a negative offset,
/// or one whose end does not fit a file offset, is EINVAL.
fn verify_area(pos: i64, count: usize) -> Result<(), SyscallError> {
    if pos < 0 || pos.checked_add(count as i64).is_none() || count > i64::MAX as usize {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// A user `loff_t` (EFAULT).
fn read_loff(ptr: usize) -> Result<i64, SyscallError> {
    userspace::read_user::<i64>(ptr)
}

/// Free room in the pipe `out` writes to, waiting while it is full (Linux's
/// wait_for_space and opipe_prep): no reader is EPIPE (the dispatcher
/// raises SIGPIPE), full with `nonblock` EAGAIN.
fn wait_for_room(out: &File, nonblock: bool) -> Result<usize, SyscallError> {
    let mut retry = false;
    loop {
        let room = out.node.pipe_room().map_err(map)?;
        if room > 0 {
            return Ok(room);
        }
        if nonblock {
            return Err(SyscallError::WouldBlock);
        }
        wait(out, POLLOUT | POLLERR | POLLHUP, &mut retry)?;
    }
}

/// Wait until the pipe `inp` has data or no writer (Linux's ipipe_prep):
/// empty with `nonblock` is EAGAIN.
fn wait_for_data(inp: &File, nonblock: bool) -> Result<(), SyscallError> {
    let mut retry = false;
    // A look at one byte, taking nothing: data or end of file goes on.
    while let Err(e) = inp.node.pipe_splice(1, false, &mut |_| Ok(0)) {
        if e != KernelError::WouldBlock {
            return Err(map(e));
        }
        if nonblock {
            return Err(SyscallError::WouldBlock);
        }
        wait(inp, POLLIN | POLLHUP | POLLERR, &mut retry)?;
    }
    Ok(())
}

/// Write `data` to the pipe `out` without waiting: the count it took (0 if
/// another writer filled it first).
fn into_pipe(out: &File, data: &[u8]) -> Result<usize, KernelError> {
    match out.write_node(0, data) {
        Err(KernelError::WouldBlock) => Ok(0),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// The three directions of a splice
// ---------------------------------------------------------------------------

/// Pipe to pipe (Linux's splice_pipe_to_pipe and, with `consume` false,
/// link_pipe for tee): wait for data, then for room, then copy what both
/// allow.
fn pipe_to_pipe(
    inp: &File,
    out: &File,
    len: usize,
    nonblock: bool,
    consume: bool,
) -> SyscallResult {
    loop {
        wait_for_data(inp, nonblock)?;
        let room = wait_for_room(out, nonblock)?;
        let mut tried = false;
        let r = inp.node.pipe_splice(len.min(room), consume, &mut |data| {
            tried = true;
            into_pipe(out, data)
        });
        match r {
            Ok(0) if tried => {} // another writer took the room: again
            Ok(n) => return Ok(n),
            // Emptied by another reader since the readiness check.
            Err(KernelError::WouldBlock) => {}
            Err(e) => return Err(map(e)),
        }
    }
}

/// Pipe to a file (Linux's iter_file_splice_write): wait for data, write
/// it at `*offset` (a regular file or block device) or to the stream, and
/// keep going until `len` or the pipe runs dry after something was
/// written. Only what the file took leaves the pipe.
fn pipe_to_file(
    inp: &File,
    out: &File,
    offset: &mut i64,
    len: usize,
    nonblock: bool,
) -> SyscallResult {
    let positioned = seekable(out);
    let mut done = 0usize;
    let mut retry_in = false;
    let mut retry_out = false;
    while done < len {
        let mut out_full = false;
        let mut first = done == 0;
        let at = *offset;
        let r = inp
            .node
            .pipe_splice((len - done).min(CHUNK), true, &mut |data| {
                if !positioned {
                    return match out.write_node(0, data) {
                        Err(KernelError::WouldBlock) => {
                            out_full = true;
                            Ok(0)
                        }
                        other => other,
                    };
                }
                let count = crate::fs::check_fsize(&*out.node, at as usize, data.len())?;
                if count != 0 && core::mem::take(&mut first) {
                    crate::fs::remove_privs(&*out.node)?;
                }
                out.write_node(at as usize, &data[..count])
            });
        match r {
            Ok(0) if out_full => {
                if done > 0 {
                    break;
                }
                if out.nonblock.load(core::sync::atomic::Ordering::Acquire) {
                    return Err(SyscallError::WouldBlock);
                }
                if let Err(e) = wait(out, POLLOUT | POLLERR | POLLHUP, &mut retry_out) {
                    return if done > 0 { Ok(done) } else { Err(e) };
                }
            }
            Ok(0) => break, // end of file: no writer and nothing queued
            Ok(n) => {
                done += n;
                if positioned {
                    *offset += n as i64;
                }
            }
            Err(KernelError::WouldBlock) => {
                // Empty with a writer: done once something was written.
                if done > 0 {
                    break;
                }
                if nonblock {
                    return Err(SyscallError::WouldBlock);
                }
                wait(inp, POLLIN | POLLHUP | POLLERR, &mut retry_in)?;
            }
            Err(e) if done > 0 => {
                let _ = e;
                break;
            }
            Err(e) => return Err(map(e)),
        }
    }
    Ok(done)
}

/// Read up to `buf.len()` bytes of `inp` at `*offset` (positioned) or from
/// the stream, waiting for a blocking stream; what has no splice_read on
/// Linux (a pipe here is handled by the callers, a directory) is EINVAL.
fn read_source(inp: &File, offset: i64, buf: &mut [u8]) -> Result<usize, SyscallError> {
    match inp.node.node_type() {
        NodeType::Directory | NodeType::Symlink => return Err(SyscallError::InvalidArgument),
        _ if is_pipe(inp) => return Err(SyscallError::InvalidArgument),
        _ => {}
    }
    if seekable(inp) {
        return inp.read_node(offset as usize, buf).map_err(map);
    }
    let mut retry = false;
    loop {
        match inp.read_node(0, buf) {
            Err(KernelError::WouldBlock)
                if !inp.nonblock.load(core::sync::atomic::Ordering::Acquire) =>
            {
                wait(inp, POLLIN | POLLHUP | POLLERR, &mut retry)?
            }
            Err(KernelError::BrokenPipe) => return Ok(0),
            other => return other.map_err(map),
        }
    }
}

/// A file to a pipe (Linux's splice_file_to_pipe): wait for room, then one
/// read of at most that much, all of which goes into the pipe.
fn file_to_pipe(
    inp: &File,
    out: &File,
    offset: &mut i64,
    len: usize,
    nonblock: bool,
) -> SyscallResult {
    let room = wait_for_room(out, nonblock)?;
    let mut buf = vec![0u8; len.min(room).min(CHUNK)];
    let n = read_source(inp, *offset, &mut buf)?;
    let mut put = 0;
    let mut retry = false;
    // `n` fits the room seen; only another writer can take it meanwhile.
    while put < n {
        match into_pipe(out, &buf[put..n]) {
            Ok(0) => {
                if seekable(inp) && put > 0 {
                    break; // the rest stays in the file
                }
                wait(out, POLLOUT | POLLERR | POLLHUP, &mut retry)?;
            }
            Ok(w) => put += w,
            Err(e) if put > 0 => {
                let _ = e;
                break;
            }
            Err(e) => return Err(map(e)),
        }
    }
    if seekable(inp) {
        *offset += put as i64;
    }
    Ok(put)
}

/// Write `data` to `out` at its position (or to the stream), waiting for a
/// blocking stream: the count written. `first` says whether this is the
/// first piece (the one that removes the file's privileges).
fn write_out(out: &File, data: &[u8], first: bool) -> Result<usize, SyscallError> {
    let mut retry = false;
    loop {
        match out.write_part(data, first) {
            Err(KernelError::WouldBlock)
                if !out.nonblock.load(core::sync::atomic::Ordering::Acquire) =>
            {
                wait(out, POLLOUT | POLLERR | POLLHUP, &mut retry)?
            }
            other => return other.map_err(map),
        }
    }
}

/// A file to a file through a kernel buffer (Linux's do_splice_direct, for
/// sendfile to anything but a pipe): from `*pos` of `inp`, at the position
/// of `out`, until `count`, end of file or a short write.
fn file_to_file(inp: &File, pos: &mut i64, out: &File, count: usize) -> SyscallResult {
    if out.is_append() {
        return Err(SyscallError::InvalidArgument);
    }
    let _io = seekable(out).then(|| out.io_lock.lock());
    let mut buf = vec![0u8; count.min(CHUNK)];
    let mut done = 0usize;
    while done < count {
        let want = (count - done).min(CHUNK);
        let step = read_source(inp, *pos, &mut buf[..want]).and_then(|n| {
            let w = if n == 0 {
                0
            } else {
                write_out(out, &buf[..n], done == 0)?
            };
            Ok((n, w))
        });
        match step {
            Ok((0, _)) => break,
            Ok((n, w)) => {
                done += w;
                if seekable(inp) {
                    *pos += w as i64;
                }
                if w < n {
                    break;
                }
            }
            Err(_) if done > 0 => break,
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

// ---------------------------------------------------------------------------
// The system calls
// ---------------------------------------------------------------------------

/// sendfile(out_fd, in_fd, offset, count) (Linux's sendfile64; x86_64's
/// sendfile is it). With `offset`, the input is read from `*offset`, which
/// is updated, and its position is left alone; without, from and past its
/// position.
pub fn sys_sendfile(out_fd: usize, in_fd: usize, offset_ptr: usize, count: usize) -> SyscallResult {
    let given = if offset_ptr != 0 {
        Some(read_loff(offset_ptr)?)
    } else {
        None
    };
    let ret = sendfile(out_fd, in_fd, given, count);
    // The offset goes back whatever happened, as Linux's put_user.
    if let Some((pos, _)) = ret.as_ref().ok().filter(|_| offset_ptr != 0) {
        userspace::write_user::<i64>(offset_ptr, *pos)?;
    } else if let Some(pos) = given {
        userspace::write_user::<i64>(offset_ptr, pos)?;
    }
    ret.map(|(_, n)| n)
}

/// Linux's do_sendfile: the new input offset and the count.
fn sendfile(
    out_fd: usize,
    in_fd: usize,
    given: Option<i64>,
    count: usize,
) -> Result<(i64, usize), SyscallError> {
    let inp = open_file(in_fd)?;
    if !inp.flags.read {
        return Err(SyscallError::BadFileDescriptor);
    }
    let mut pos = match given {
        Some(_) if !seekable(&inp) => return Err(SyscallError::IllegalSeek),
        Some(p) => p,
        None if seekable(&inp) => position(&inp),
        None => 0,
    };
    verify_area(pos, count)?;
    let mut count = count.min(MAX_RW_COUNT);
    let out = open_file(out_fd)?;
    if !out.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let room = MAX_FILE_SIZE.saturating_sub(pos);
    if count as i64 > room {
        if room <= 0 {
            return Err(SyscallError::Overflow);
        }
        count = room as usize;
    }
    let n = if is_pipe(&out) {
        let nonblock = out.nonblock.load(core::sync::atomic::Ordering::Acquire);
        file_to_pipe(&inp, &out, &mut pos, count, nonblock)?
    } else {
        if seekable(&out) {
            verify_area(position(&out), count)?;
        }
        file_to_file(&inp, &mut pos, &out, count)?
    };
    if n > 0 && given.is_none() && seekable(&inp) {
        set_position(&inp, pos);
    }
    Ok((pos, n))
}

/// copy_file_range(fd_in, off_in, fd_out, off_out, len, flags) between two
/// regular files of one filesystem.
pub fn sys_copy_file_range(
    fd_in: usize,
    off_in: usize,
    fd_out: usize,
    off_out: usize,
    len: usize,
    flags: usize,
) -> SyscallResult {
    let inp = open_file(fd_in)?;
    let out = open_file(fd_out)?;
    let mut pos_in = if off_in != 0 {
        read_loff(off_in)?
    } else {
        position(&inp)
    };
    let mut pos_out = if off_out != 0 {
        read_loff(off_out)?
    } else {
        position(&out)
    };
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let count = copy_checks(&inp, pos_in, &out, pos_out, len as u64)?;
    verify_area(pos_in, count)?;
    verify_area(pos_out, count)?;
    if count == 0 {
        return Ok(0);
    }
    let count = count.min(MAX_RW_COUNT);
    let n = copy_range(&inp, &mut pos_in, &out, &mut pos_out, count)?;
    if n > 0 {
        if off_in != 0 {
            userspace::write_user::<i64>(off_in, pos_in)?;
        } else {
            set_position(&inp, pos_in);
        }
        if off_out != 0 {
            userspace::write_user::<i64>(off_out, pos_out)?;
        } else {
            set_position(&out, pos_out);
        }
    }
    Ok(n)
}

/// Linux's generic_file_rw_checks and generic_copy_file_checks: the count
/// to copy.
fn copy_checks(
    inp: &File,
    pos_in: i64,
    out: &File,
    pos_out: i64,
    len: u64,
) -> Result<usize, SyscallError> {
    let (tin, tout) = (inp.node.node_type(), out.node.node_type());
    if tin == NodeType::Directory || tout == NodeType::Directory {
        return Err(SyscallError::IsADirectory);
    }
    if tin != NodeType::File || tout != NodeType::File || is_pipe(inp) || is_pipe(out) {
        return Err(SyscallError::InvalidArgument);
    }
    if !inp.flags.read || !out.flags.write || out.is_append() {
        return Err(SyscallError::BadFileDescriptor);
    }
    if !same_filesystem(inp, out) {
        return Err(SyscallError::CrossDevice);
    }
    let size_in = filesystem::file_size(inp)? as u64;
    let count = shorten(pos_in, pos_out, len, size_in)?;
    // RLIMIT_FSIZE (EFBIG with SIGXFSZ), then the largest file.
    let count = if pos_out >= 0 {
        crate::fs::check_fsize(&*out.node, pos_out as usize, count as usize).map_err(map)? as u64
    } else {
        count
    };
    let count = max_bytes(pos_out, count)?;
    let same = crate::fs::inode_key(&inp.node) == crate::fs::inode_key(&out.node);
    if same && overlaps(pos_in, pos_out, count) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(count as usize)
}

/// Offsets that wrap are EOVERFLOW; the count stops at the end of the input
/// (`size_in` bytes).
fn shorten(pos_in: i64, pos_out: i64, len: u64, size_in: u64) -> Result<u64, SyscallError> {
    if (pos_in as u64).checked_add(len).is_none() || (pos_out as u64).checked_add(len).is_none() {
        return Err(SyscallError::Overflow);
    }
    Ok(if pos_in >= size_in as i64 {
        0
    } else {
        len.min(size_in - pos_in as u64)
    })
}

/// A write at `pos` past the largest file is EFBIG; one reaching it is cut
/// (generic_write_check_limits' s_maxbytes part).
fn max_bytes(pos: i64, count: u64) -> Result<u64, SyscallError> {
    let room = MAX_FILE_SIZE.saturating_sub(pos);
    if room <= 0 {
        return Err(SyscallError::FileTooLarge);
    }
    Ok(count.min(room as u64))
}

/// Whether `count` bytes at `pos_out` overlap those at `pos_in`.
fn overlaps(pos_in: i64, pos_out: i64, count: u64) -> bool {
    let (a, b, n) = (pos_in as i128, pos_out as i128, count as i128);
    b + n > a && b < a + n
}

/// Whether two open files are on one filesystem: by the filesystem of
/// their inode identities where both have one, else by the mounts their
/// paths are under.
fn same_filesystem(a: &File, b: &File) -> bool {
    match (a.node.file_identity(), b.node.file_identity()) {
        (Some((fa, _)), Some((fb, _))) => return fa == fb,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    let (Some(pa), Some(pb)) = (a.path.as_deref(), b.path.as_deref()) else {
        return true;
    };
    match filesystem::vfs() {
        Ok(vfs) => vfs.mount_point_of(pa) == vfs.mount_point_of(pb),
        Err(_) => true,
    }
}

/// Copy `count` bytes from `*pos_in` of `inp` to `*pos_out` of `out`, a
/// chunk at a time, advancing both by what was written.
fn copy_range(
    inp: &File,
    pos_in: &mut i64,
    out: &File,
    pos_out: &mut i64,
    count: usize,
) -> SyscallResult {
    let mut buf = vec![0u8; count.min(CHUNK)];
    let mut done = 0usize;
    while done < count {
        let want = (count - done).min(CHUNK);
        let step = (|| {
            let n = inp
                .node
                .read(*pos_in as usize, &mut buf[..want])
                .map_err(map)?;
            if n == 0 {
                return Ok((0, 0));
            }
            if done == 0 {
                crate::fs::remove_privs(&*out.node).map_err(map)?;
            }
            let w = out.write_node(*pos_out as usize, &buf[..n]).map_err(map)?;
            Ok((n, w))
        })();
        match step {
            Ok((0, _)) => break,
            Ok((n, w)) => {
                done += w;
                *pos_in += w as i64;
                *pos_out += w as i64;
                if w < n {
                    break;
                }
            }
            Err(_) if done > 0 => break,
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// splice(fd_in, off_in, fd_out, off_out, len, flags): from a pipe, to a
/// pipe, or both.
pub fn sys_splice(
    fd_in: usize,
    off_in: usize,
    fd_out: usize,
    off_out: usize,
    len: usize,
    flags: usize,
) -> SyscallResult {
    let flags = flags as u32;
    if len == 0 {
        return Ok(0);
    }
    if flags & !SPLICE_F_ALL != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let inp = open_file(fd_in)?;
    let out = open_file(fd_out)?;
    let (ipipe, opipe) = (is_pipe(&inp), is_pipe(&out));
    if (ipipe && off_in != 0) || (opipe && off_out != 0) {
        return Err(SyscallError::IllegalSeek);
    }
    // Linux reads the output offset first.
    let given_out = if off_out != 0 {
        Some(read_loff(off_out)?)
    } else {
        None
    };
    let given_in = if off_in != 0 {
        Some(read_loff(off_in)?)
    } else {
        None
    };
    if !inp.flags.read || !out.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    let nonblock_of = |f: &File| f.nonblock.load(core::sync::atomic::Ordering::Acquire);
    let mut nonblock = flags & SPLICE_F_NONBLOCK != 0;
    let n = match (ipipe, opipe) {
        (true, true) => {
            if inp.node.pipe_id() == out.node.pipe_id() {
                return Err(SyscallError::InvalidArgument);
            }
            nonblock |= nonblock_of(&inp) || nonblock_of(&out);
            pipe_to_pipe(&inp, &out, len, nonblock, true)?
        }
        (true, false) => {
            let mut offset = match given_out {
                Some(_) if !seekable(&out) => return Err(SyscallError::InvalidArgument),
                Some(o) => o,
                None if seekable(&out) => position(&out),
                None => 0,
            };
            if out.is_append() {
                return Err(SyscallError::InvalidArgument);
            }
            if seekable(&out) {
                verify_area(offset, len)?;
            }
            nonblock |= nonblock_of(&inp);
            let n = pipe_to_file(&inp, &out, &mut offset, len, nonblock)?;
            match given_out {
                Some(_) => userspace::write_user::<i64>(off_out, offset)?,
                None if seekable(&out) => set_position(&out, offset),
                None => {}
            }
            n
        }
        (false, true) => {
            let mut offset = match given_in {
                Some(_) if !seekable(&inp) => return Err(SyscallError::InvalidArgument),
                Some(o) => o,
                None if seekable(&inp) => position(&inp),
                None => 0,
            };
            if seekable(&inp) {
                verify_area(offset, len)?;
            }
            nonblock |= nonblock_of(&out);
            let n = file_to_pipe(&inp, &out, &mut offset, len.min(MAX_RW_COUNT), nonblock)?;
            match given_in {
                Some(_) => userspace::write_user::<i64>(off_in, offset)?,
                None if seekable(&inp) => set_position(&inp, offset),
                None => {}
            }
            n
        }
        (false, false) => return Err(SyscallError::InvalidArgument),
    };
    Ok(n)
}

/// tee(fd_in, fd_out, len, flags): copy what the pipe `fd_in` holds into
/// the pipe `fd_out`, leaving it in `fd_in`.
pub fn sys_tee(fd_in: usize, fd_out: usize, len: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags & !SPLICE_F_ALL != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if len == 0 {
        return Ok(0);
    }
    let inp = open_file(fd_in)?;
    let out = open_file(fd_out)?;
    if !inp.flags.read || !out.flags.write {
        return Err(SyscallError::BadFileDescriptor);
    }
    if !is_pipe(&inp) || !is_pipe(&out) || inp.node.pipe_id() == out.node.pipe_id() {
        return Err(SyscallError::InvalidArgument);
    }
    let nonblock_of = |f: &File| f.nonblock.load(core::sync::atomic::Ordering::Acquire);
    let nonblock = flags & SPLICE_F_NONBLOCK != 0 || nonblock_of(&inp) || nonblock_of(&out);
    pipe_to_pipe(&inp, &out, len, nonblock, false)
}

/// vmsplice(fd, iov, nr_segs, flags): user memory into the pipe `fd`
/// (open for writing), or out of it (open for reading only). Only
/// SPLICE_F_NONBLOCK makes it not wait; the pipe's O_NONBLOCK does not.
pub fn sys_vmsplice(fd: usize, iov_ptr: usize, nr_segs: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags & !SPLICE_F_ALL != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let file = open_file(fd)?;
    let to_pipe = if file.flags.write {
        true
    } else if file.flags.read {
        false
    } else {
        return Err(SyscallError::BadFileDescriptor);
    };
    let iovs = read_iovecs(iov_ptr, nr_segs)?;
    let total: usize = iovs.iter().map(|&(_, len)| len).sum();
    if total == 0 {
        return Ok(0);
    }
    if !is_pipe(&file) {
        return Err(SyscallError::BadFileDescriptor);
    }
    let nonblock = flags & SPLICE_F_NONBLOCK != 0;
    if to_pipe {
        vmsplice_to_pipe(&file, &iovs, total, nonblock)
    } else {
        vmsplice_to_user(&file, &iovs, total, nonblock)
    }
}

/// Wait for room, then put as much of the user buffers as fits; a fault
/// after some bytes stops there.
fn vmsplice_to_pipe(
    file: &File,
    iovs: &[(usize, usize)],
    total: usize,
    nonblock: bool,
) -> SyscallResult {
    let room = wait_for_room(file, nonblock)?;
    let want = total.min(room);
    let mut buf: Vec<u8> = Vec::with_capacity(want);
    for &(base, len) in iovs {
        let take = len.min(want - buf.len());
        if take == 0 {
            break;
        }
        let start = buf.len();
        buf.resize(start + take, 0);
        if let Err(e) = userspace::read_user_bytes(base, &mut buf[start..]) {
            buf.truncate(start);
            if buf.is_empty() {
                return Err(e);
            }
            break;
        }
    }
    into_pipe(file, &buf).map_err(map)
}

/// Copy what the pipe holds, up to `total` bytes, into the user buffers,
/// waiting while it is empty and has a writer.
fn vmsplice_to_user(
    file: &File,
    iovs: &[(usize, usize)],
    total: usize,
    nonblock: bool,
) -> SyscallResult {
    let mut retry = false;
    loop {
        let r = file
            .node
            .pipe_splice(total.min(MAX_RW_COUNT), true, &mut |data| {
                let mut done = 0;
                for &(base, len) in iovs {
                    if done == data.len() {
                        break;
                    }
                    let n = len.min(data.len() - done);
                    if userspace::write_user_bytes(base, &data[done..done + n]).is_err() {
                        if done == 0 {
                            return Err(KernelError::InvalidAddress { addr: base });
                        }
                        break;
                    }
                    done += n;
                }
                Ok(done)
            });
        match r {
            Ok(n) => return Ok(n),
            Err(KernelError::WouldBlock) if !nonblock => {
                wait(file, POLLIN | POLLHUP | POLLERR, &mut retry)?
            }
            Err(e) => return Err(map(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_area_follows_rw_verify_area() {
        assert_eq!(verify_area(0, 10), Ok(()));
        assert_eq!(verify_area(-1, 1), Err(SyscallError::InvalidArgument));
        assert_eq!(verify_area(i64::MAX, 1), Err(SyscallError::InvalidArgument));
        assert_eq!(verify_area(i64::MAX, 0), Ok(()));
    }

    #[test]
    fn copy_checks_follow_generic_copy_file_checks() {
        // Wrapping offsets, then the end of the input.
        assert_eq!(
            shorten(0, i64::MAX, u64::MAX, 10),
            Err(SyscallError::Overflow)
        );
        assert_eq!(shorten(-1, 0, 1, 10), Err(SyscallError::Overflow));
        assert_eq!(shorten(0, 0, 100, 10), Ok(10));
        assert_eq!(shorten(4, 0, 100, 10), Ok(6));
        assert_eq!(shorten(10, 0, 100, 10), Ok(0));
        // The largest file: at it EFBIG, short of it cut.
        assert_eq!(max_bytes(i64::MAX, 1), Err(SyscallError::FileTooLarge));
        assert_eq!(max_bytes(i64::MAX - 4, 10), Ok(4));
        // Overlap within one file.
        assert!(overlaps(0, 5, 10));
        assert!(overlaps(5, 0, 10));
        assert!(!overlaps(0, 10, 10));
        assert!(!overlaps(10, 0, 10));
    }
}

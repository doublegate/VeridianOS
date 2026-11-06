//! process_vm_readv(2) and process_vm_writev(2), as Linux 7.3's
//! `mm/process_vm_access.c`: copy between the caller's memory and another
//! process's, which the caller must be allowed to ptrace.

use alloc::sync::Arc;

use super::{SyscallError, SyscallResult};
use crate::process::{self, Process};

const PAGE: usize = 4096;

/// A position in a list of (base, length) segments.
struct Cursor<'a> {
    segs: &'a [(usize, usize)],
    index: usize,
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(segs: &'a [(usize, usize)]) -> Self {
        Self {
            segs,
            index: 0,
            offset: 0,
        }
    }

    /// The next stretch of at most `max` bytes (empty segments skipped),
    /// or None at the end.
    fn peek(&mut self, max: usize) -> Option<(usize, usize)> {
        while let Some(&(base, len)) = self.segs.get(self.index) {
            if self.offset < len {
                return Some((base.wrapping_add(self.offset), (len - self.offset).min(max)));
            }
            self.index += 1;
            self.offset = 0;
        }
        None
    }

    fn advance(&mut self, n: usize) {
        self.offset += n;
    }
}

/// The pieces a copy is made in: one remote page at a time, and no more
/// than the local segment at the cursor holds. Each is (remote address,
/// local address, length); the walk stops when the local segments run out.
struct Pieces<'a> {
    local: Cursor<'a>,
    remote: &'a [(usize, usize)],
    index: usize,
    offset: usize,
}

fn pieces<'a>(local: &'a [(usize, usize)], remote: &'a [(usize, usize)]) -> Pieces<'a> {
    Pieces {
        local: Cursor::new(local),
        remote,
        index: 0,
        offset: 0,
    }
}

impl Iterator for Pieces<'_> {
    type Item = (usize, usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(base, len) = self.remote.get(self.index)?;
            if self.offset >= len {
                self.index += 1;
                self.offset = 0;
                continue;
            }
            let addr = base.wrapping_add(self.offset);
            let in_page = PAGE - (addr % PAGE);
            let (laddr, n) = self.local.peek(in_page.min(len - self.offset))?;
            self.local.advance(n);
            self.offset += n;
            return Some((addr, laddr, n));
        }
    }
}

/// Copy `buf.len()` bytes (within one page) between `target`'s memory at
/// `addr` and `buf`, faulting the page in first if it is not present, as
/// Linux's pin_user_pages_remote does. A write goes only where the target
/// may write itself (a copy-on-write page is copied first). Checked again
/// for each page under the target's credential guard: a set-ID exec that
/// began since, or finished with credentials the caller may not access,
/// ends the copy, so the caller never reaches the privileged image.
fn remote_page(
    target: &Process,
    allowed: &dyn Fn(&Process) -> bool,
    addr: usize,
    buf: &mut [u8],
    write: bool,
) -> bool {
    let _guard = target.cred_guard.lock();
    if target
        .exec_in_progress
        .load(core::sync::atomic::Ordering::Acquire)
        || !target.is_alive()
        || !allowed(target)
    {
        return false;
    }
    for attempt in 0..2 {
        let done = {
            let vas = target.memory_space.lock();
            if write {
                vas.write_bytes_private(addr as u64, buf, false).is_ok()
            } else {
                vas.read_user_bytes(addr as u64, buf).is_ok()
            }
        };
        if done {
            return true;
        }
        if attempt == 0 && !crate::mm::page_fault::fault_in_remote(target, addr as u64, write) {
            return false;
        }
    }
    false
}

/// process_vm_readv and process_vm_writev, in Linux's order: flags other
/// than 0 are EINVAL; the local iovecs as readv's (EINVAL, EFAULT), and
/// nothing to copy returns 0; the remote iovecs likewise, and only empty
/// ones return 0; no such process is ESRCH, and one the caller may not
/// ptrace (Linux's PTRACE_MODE_ATTACH_REALCREDS) EPERM. Then the bytes
/// are copied, local segment by local segment against remote page by
/// remote page; a fault on either side ends the copy, which returns the
/// bytes moved, or EFAULT if there were none.
fn process_vm_rw(
    pid: usize,
    lvec: usize,
    liovcnt: usize,
    rvec: usize,
    riovcnt: usize,
    flags: usize,
    write: bool,
) -> SyscallResult {
    if flags != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let local = super::filesystem::read_iovecs(lvec, liovcnt)?;
    if local.iter().all(|&(_, len)| len == 0) {
        return Ok(0);
    }
    let remote = super::filesystem::read_iovecs(rvec, riovcnt)?;
    if remote.iter().all(|&(_, len)| len == 0) {
        return Ok(0);
    }
    let target = target(pid)?;
    let caller = process::creds::Caller::current();
    let permitted = super::debug::current_permitted();
    let allowed = |p: &Process| super::debug::may_access_process(&caller, permitted, p);
    {
        let _guard = target.cred_guard.lock();
        if !allowed(&target) {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    let mut buf = alloc::vec![0u8; PAGE];
    let mut done = 0usize;
    for (raddr, laddr, n) in pieces(&local, &remote) {
        let chunk = &mut buf[..n];
        let ok = if write {
            super::userspace::read_user_bytes(laddr, chunk).is_ok()
                && remote_page(&target, &allowed, raddr, chunk, true)
        } else {
            remote_page(&target, &allowed, raddr, chunk, false)
                && super::userspace::write_user_bytes(laddr, chunk).is_ok()
        };
        if !ok {
            break;
        }
        done += n;
    }
    buf.fill(0);
    if done > 0 {
        Ok(done)
    } else {
        Err(SyscallError::InvalidPointer)
    }
}

/// The process `pid` (a process's or thread's ID, a pid_t in the caller's
/// PID namespace) names: ESRCH for none.
fn target(pid: usize) -> Result<Arc<Process>, SyscallError> {
    let id =
        process::pidns::from_vnr(pid as u32 as i32 as i64).ok_or(SyscallError::ProcessNotFound)?;
    process::get_process(process::ProcessId(id))
        .or_else(|| super::process::process_of_thread(id as usize).and_then(process::get_process))
        .ok_or(SyscallError::ProcessNotFound)
}

/// process_vm_readv(pid, lvec, liovcnt, rvec, riovcnt, flags): copy from
/// process `pid`'s memory at `rvec` into the caller's at `lvec`.
pub fn sys_process_vm_readv(
    pid: usize,
    lvec: usize,
    liovcnt: usize,
    rvec: usize,
    riovcnt: usize,
    flags: usize,
) -> SyscallResult {
    process_vm_rw(pid, lvec, liovcnt, rvec, riovcnt, flags, false)
}

/// process_vm_writev(pid, lvec, liovcnt, rvec, riovcnt, flags): copy from
/// the caller's memory at `lvec` into process `pid`'s at `rvec`.
pub fn sys_process_vm_writev(
    pid: usize,
    lvec: usize,
    liovcnt: usize,
    rvec: usize,
    riovcnt: usize,
    flags: usize,
) -> SyscallResult {
    process_vm_rw(pid, lvec, liovcnt, rvec, riovcnt, flags, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_split_at_remote_pages_and_local_segments() {
        // One local buffer, a remote range across a page boundary.
        let p: alloc::vec::Vec<_> = pieces(&[(0x10_0000, 0x3000)], &[(0x2_0ff0, 0x20)]).collect();
        assert_eq!(
            p,
            [(0x2_0ff0, 0x10_0000, 0x10), (0x2_1000, 0x10_0010, 0x10)]
        );
        // Two local buffers, one remote page.
        let p: alloc::vec::Vec<_> =
            pieces(&[(0x10_0000, 8), (0, 0), (0x20_0000, 8)], &[(0x3_0000, 16)]).collect();
        assert_eq!(p, [(0x3_0000, 0x10_0000, 8), (0x3_0008, 0x20_0000, 8)]);
    }

    #[test]
    fn pieces_stop_when_the_local_side_is_full() {
        let p: alloc::vec::Vec<_> =
            pieces(&[(0x10_0000, 4)], &[(0x3_0000, 16), (0x4_0000, 16)]).collect();
        assert_eq!(p, [(0x3_0000, 0x10_0000, 4)]);
        // Empty remote segments are skipped.
        let p: alloc::vec::Vec<_> =
            pieces(&[(0x10_0000, 4)], &[(0x3_0000, 0), (0x4_0000, 2)]).collect();
        assert_eq!(p, [(0x4_0000, 0x10_0000, 2)]);
    }
}

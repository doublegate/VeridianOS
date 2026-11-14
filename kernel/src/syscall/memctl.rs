//! Memory locking, sealing and residency (v0.28 item 5, ADR 0027): mlock,
//! mlock2, munlock, mlockall, munlockall (Linux's `mm/mlock.c`), mseal
//! (`mm/mseal.c`), mincore (`mm/mincore.c`) and msync (`mm/msync.c`), with
//! Linux 7.3's argument checks in Linux's order.
//!
//! Pages here are never paged out, so a lock changes what is reported and
//! counted (VmLck, smaps' Locked, RLIMIT_MEMLOCK) and what msync's
//! MS_INVALIDATE accepts. Locking past RLIMIT_MEMLOCK, or at all when it is
//! 0, needs CAP_IPC_LOCK and, as ADR 0024's RLIMIT_NPROC override, the
//! `System` object with MODIFY (the maintainer's choice).

use super::{SyscallError, SyscallResult};
use crate::{
    mm::vas::Mlock,
    process::{self, rlimit},
};

const PAGE: u64 = 4096;

/// mlock2's flag.
pub const MLOCK_ONFAULT: usize = 1;
/// mlockall's flags.
pub const MCL_CURRENT: usize = 1;
pub const MCL_FUTURE: usize = 2;
pub const MCL_ONFAULT: usize = 4;
/// msync's flags.
pub const MS_ASYNC: usize = 1;
pub const MS_INVALIDATE: usize = 2;
pub const MS_SYNC: usize = 4;

/// CAP_IPC_LOCK, honoured with `System` MODIFY (ADR 0024).
fn ipc_lock_exempt() -> bool {
    crate::process::creds::Caller::current().capable(crate::process::linux_caps::CAP_IPC_LOCK)
        && crate::cap::authority::current_may_change_system()
}

/// RLIMIT_MEMLOCK in pages (`rlimit(RLIMIT_MEMLOCK) >> PAGE_SHIFT`).
fn limit_pages(proc: &process::Process) -> u64 {
    let lim = proc.limits().cur(rlimit::RLIMIT_MEMLOCK);
    if lim == rlimit::RLIM_INFINITY {
        u64::MAX
    } else {
        lim / PAGE
    }
}

/// can_do_mlock: a non-zero RLIMIT_MEMLOCK, or the exemption.
fn can_do_mlock(proc: &process::Process) -> bool {
    proc.limits().cur(rlimit::RLIMIT_MEMLOCK) != 0 || ipc_lock_exempt()
}

/// do_mlock's range: the start rounded down to a page, the length grown by
/// what that added and rounded up; `None` where that wraps (EINVAL).
pub fn lock_range(start: usize, len: usize) -> Option<(u64, u64)> {
    let start = start as u64;
    let off = start & (PAGE - 1);
    let len = (len as u64)
        .checked_add(off)?
        .checked_next_multiple_of(PAGE)?;
    let start = start & !(PAGE - 1);
    Some((start, start.checked_add(len)?))
}

/// The error a lock or seal of a range gives: EINVAL for one that ends
/// inside a huge page, ENOMEM for a hole.
fn range_error(e: crate::error::KernelError) -> SyscallError {
    match e {
        crate::error::KernelError::InvalidArgument { .. } => SyscallError::InvalidArgument,
        _ => SyscallError::OutOfMemory,
    }
}

/// Whether a range lies in user space: beyond it nothing is mapped (ENOMEM
/// from the walk on Linux; here the kernel half must not be touched).
fn in_user_space(end: u64) -> bool {
    end <= crate::mm::user_layout::user_space_end() as u64
}

/// do_mlock for mlock (`Locked`) and mlock2 (`OnFault` with MLOCK_ONFAULT).
fn do_mlock(start: usize, len: usize, lock: Mlock) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    if !can_do_mlock(&proc) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let (start, end) = lock_range(start, len).ok_or(SyscallError::InvalidArgument)?;
    if end == start {
        return Ok(0);
    }
    let vas = proc.memory_space.lock();
    let pages = (end - start) / PAGE;
    let limit = limit_pages(&proc);
    let mut locked = pages.saturating_add(vas.locked_pages() as u64);
    if locked > limit && !ipc_lock_exempt() {
        // What is already locked in the range does not count twice.
        locked = locked.saturating_sub(vas.locked_pages_in(start, end) as u64);
        if locked > limit {
            return Err(SyscallError::OutOfMemory);
        }
    }
    if !in_user_space(end) {
        return Err(SyscallError::OutOfMemory);
    }
    vas.set_mlock(start, end, lock).map_err(range_error)?;
    Ok(0)
}

/// mlock(addr, len).
pub fn sys_mlock(start: usize, len: usize) -> SyscallResult {
    do_mlock(start, len, Mlock::Locked)
}

/// mlock2(addr, len, flags): MLOCK_ONFAULT is the only flag (EINVAL first).
pub fn sys_mlock2(start: usize, len: usize, flags: usize) -> SyscallResult {
    if flags & !MLOCK_ONFAULT != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let lock = if flags & MLOCK_ONFAULT != 0 {
        Mlock::OnFault
    } else {
        Mlock::Locked
    };
    do_mlock(start, len, lock)
}

/// munlock(addr, len): no permission or limit check; a hole is ENOMEM.
pub fn sys_munlock(start: usize, len: usize) -> SyscallResult {
    let (start, end) = lock_range(start, len).ok_or(SyscallError::InvalidArgument)?;
    if end == start {
        return Ok(0);
    }
    if !in_user_space(end) {
        return Err(SyscallError::OutOfMemory);
    }
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    proc.memory_space
        .lock()
        .set_mlock(start, end, Mlock::Unlocked)
        .map_err(range_error)?;
    Ok(0)
}

/// mlockall's flags, checked as Linux: none, an unknown bit, or
/// MCL_ONFAULT alone is EINVAL.
pub fn mlockall_flags_valid(flags: usize) -> bool {
    flags != 0 && flags & !(MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT) == 0 && flags != MCL_ONFAULT
}

/// What mlockall's flags make the default for new mappings (MCL_FUTURE)
/// and of the present ones (MCL_CURRENT).
pub fn mlockall_states(flags: usize) -> (Mlock, Option<Mlock>) {
    let lock = if flags & MCL_ONFAULT != 0 {
        Mlock::OnFault
    } else {
        Mlock::Locked
    };
    let future = if flags & MCL_FUTURE != 0 {
        lock
    } else {
        Mlock::Unlocked
    };
    (future, (flags & MCL_CURRENT != 0).then_some(lock))
}

/// mlockall(flags): with MCL_CURRENT, every mapping (ENOMEM if the whole
/// address space, not just what is locked, is over the limit); with
/// MCL_FUTURE, every mapping made later (mmap, brk, mremap growth). A call
/// without MCL_FUTURE cancels an earlier one's.
pub fn sys_mlockall(flags: usize) -> SyscallResult {
    if !mlockall_flags_valid(flags) {
        return Err(SyscallError::InvalidArgument);
    }
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    if !can_do_mlock(&proc) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let vas = proc.memory_space.lock();
    if flags & MCL_CURRENT != 0
        && vas.total_pages() as u64 > limit_pages(&proc)
        && !ipc_lock_exempt()
    {
        return Err(SyscallError::OutOfMemory);
    }
    let (future, current) = mlockall_states(flags);
    vas.update_mm_state(|m| m.def_mlock = future);
    if let Some(lock) = current {
        vas.set_mlock_all(lock);
    }
    Ok(0)
}

/// munlockall(): nothing locked, and nothing to be.
pub fn sys_munlockall() -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let vas = proc.memory_space.lock();
    vas.update_mm_state(|m| m.def_mlock = Mlock::Unlocked);
    vas.set_mlock_all(Mlock::Unlocked);
    Ok(0)
}

/// mlock_future_ok: may `bytes` more of locked memory be added (a new
/// mapping under MCL_FUTURE or MAP_LOCKED)? EAGAIN from mmap otherwise.
pub fn future_lock_allowed(
    proc: &process::Process,
    vas: &crate::mm::VirtualAddressSpace,
    bytes: u64,
) -> bool {
    let pages = bytes.div_ceil(PAGE);
    (vas.locked_pages() as u64).saturating_add(pages) <= limit_pages(proc) || ipc_lock_exempt()
}

/// mmap's MAP_LOCKED without the right to lock: EPERM (do_mmap).
pub fn may_map_locked(proc: &process::Process) -> bool {
    can_do_mlock(proc)
}

/// mseal's range: an aligned start (EINVAL), the length rounded up (EINVAL
/// if that wraps), `None` for EINVAL.
pub fn seal_range(start: usize, len: usize) -> Option<(u64, u64)> {
    let start = start as u64;
    if start & (PAGE - 1) != 0 {
        return None;
    }
    let len = (len as u64).checked_next_multiple_of(PAGE)?;
    Some((start, start.checked_add(len)?))
}

/// mseal(addr, len, flags): flags must be 0; any unmapped part of the range
/// is ENOMEM before anything changes; then the range is sealed for good.
pub fn sys_mseal(start: usize, len: usize, flags: usize) -> SyscallResult {
    if flags != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (start, end) = seal_range(start, len).ok_or(SyscallError::InvalidArgument)?;
    if end == start {
        return Ok(0);
    }
    if !in_user_space(end) {
        return Err(SyscallError::OutOfMemory);
    }
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    proc.memory_space
        .lock()
        .seal(start, end)
        .map_err(range_error)?;
    Ok(0)
}

/// Pages mincore answers for at a time, so a huge reservation costs no
/// huge buffer (Linux copies a page of answers at a time).
const MINCORE_CHUNK: u64 = 4096;

/// mincore(addr, len, vec): an aligned start (EINVAL), the range in user
/// space (ENOMEM), `vec` writable for one byte per page (EFAULT), the whole
/// range mapped (ENOMEM). Bit 0 of each byte: the page is resident.
pub fn sys_mincore(start: usize, len: usize, vec: usize) -> SyscallResult {
    let start = start as u64;
    if start & (PAGE - 1) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let end = start
        .checked_add(len as u64)
        .filter(|&e| in_user_space(e))
        .ok_or(SyscallError::OutOfMemory)?;
    let pages = (len as u64).div_ceil(PAGE);
    super::userspace::validate_user_ptr(vec as *const u8, pages as usize)?;
    if pages == 0 {
        return Ok(0);
    }
    let end = end.div_ceil(PAGE) * PAGE;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let mut at = start;
    while at < end {
        let to = end.min(at + MINCORE_CHUNK * PAGE);
        let bytes = proc
            .memory_space
            .lock()
            .residency(at, to)
            .ok_or(SyscallError::OutOfMemory)?;
        super::userspace::write_user_bytes(vec + ((at - start) / PAGE) as usize, &bytes)?;
        at = to;
    }
    Ok(0)
}

/// msync's checks before the walk: known flags, an aligned start, not both
/// MS_ASYNC and MS_SYNC (EINVAL); then the length rounded up, and a range
/// that wraps is ENOMEM. `Ok(None)`: an empty range (0).
pub fn msync_range(
    start: usize,
    len: usize,
    flags: usize,
) -> Result<Option<(u64, u64)>, SyscallError> {
    if flags & !(MS_ASYNC | MS_INVALIDATE | MS_SYNC) != 0
        || start as u64 & (PAGE - 1) != 0
        || flags & MS_ASYNC != 0 && flags & MS_SYNC != 0
    {
        return Err(SyscallError::InvalidArgument);
    }
    let start = start as u64;
    let len = (len as u64)
        .checked_next_multiple_of(PAGE)
        .ok_or(SyscallError::OutOfMemory)?;
    let end = start.checked_add(len).ok_or(SyscallError::OutOfMemory)?;
    // Nothing a user mapping can be past user space: ENOMEM, as the other
    // calls answer (PR #41 review).
    if end != start && !in_user_space(end) {
        return Err(SyscallError::OutOfMemory);
    }
    Ok((end != start).then_some((start, end)))
}

/// msync(addr, len, flags). A MAP_SHARED mapping of a regular file holds a
/// copy of it here (a page cache is v0.29), so both MS_SYNC and MS_ASYNC
/// write that copy back to the file, as the maintainer chose (on Linux the
/// page cache makes the data the file's at once, and MS_ASYNC does nothing):
/// within the file's size, as a mapping beyond it is SIGBUS. A memfd's or
/// tmpfs file's mapping is the file already. MS_INVALIDATE over a locked
/// mapping is EBUSY. A hole: ENOMEM at once for exactly MS_ASYNC, otherwise
/// after the rest is done.
pub fn sys_msync(start: usize, len: usize, flags: usize) -> SyscallResult {
    let Some((start, end)) = msync_range(start, len, flags)? else {
        return Ok(0);
    };
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let (hole, locked, copies) = {
        let vas = proc.memory_space.lock();
        (
            vas.hole_in(start, end),
            vas.locked_in(start, end),
            vas.shared_file_copies(start, end),
        )
    };
    if hole.is_some() && flags == MS_ASYNC {
        return Err(SyscallError::OutOfMemory);
    }
    if flags & MS_INVALIDATE != 0 && locked {
        return Err(SyscallError::Busy);
    }
    if flags & (MS_SYNC | MS_ASYNC) != 0 {
        write_back(&proc, copies)?;
    }
    if hole.is_some() {
        return Err(SyscallError::OutOfMemory);
    }
    Ok(0)
}

/// Write `copies` (`shared_file_copies` of `proc`'s space) back to their
/// files. Each page is copied out under the address space's lock, checking
/// that it is still mapped to the same frame: another thread may unmap it
/// (and the frame be reused) once the lock is dropped, and only the copy
/// goes to the file. A page unmapped meanwhile is skipped, as Linux's msync
/// would no longer find it. An immutable file is skipped: nothing can have
/// written its mappings since it became one (they fault, SIGBUS).
pub(crate) fn write_back(
    proc: &process::Process,
    copies: alloc::vec::Vec<(
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        usize,
        u64,
        alloc::vec::Vec<crate::mm::FrameNumber>,
    )>,
) -> Result<(), SyscallError> {
    let mut buf = alloc::vec![0u8; PAGE as usize];
    for (node, offset, addr, frames) in copies {
        if crate::fs::attr::of(&*node) & crate::fs::attr::FS_IMMUTABLE_FL != 0 {
            continue;
        }
        let size = node.metadata().map_or(0, |m| m.size);
        for (i, frame) in frames.iter().enumerate() {
            let at = offset + i * PAGE as usize;
            if at >= size {
                break;
            }
            let n = (size - at).min(PAGE as usize);
            let page = addr + i as u64 * PAGE;
            if !proc
                .memory_space
                .lock()
                .copy_mapped_page(page, *frame, &mut buf[..n])
            {
                continue;
            }
            node.write(at, &buf[..n]).map_err(super::map_kernel_error)?;
        }
    }
    Ok(())
}

/// madvise advice another process's memory may be given (Linux's
/// process_madvise_remote_valid: COLD, PAGEOUT, WILLNEED, COLLAPSE). They
/// are hints here, and MADV_COLLAPSE does not exist (EINVAL, as madvise).
const MADV_WILLNEED: usize = 3;
const MADV_COLD: usize = 20;
const MADV_PAGEOUT: usize = 21;
/// UIO_MAXIOV.
const UIO_MAXIOV: usize = 1024;

/// The process a pidfd names: EBADF for a descriptor that is not one,
/// ESRCH for a process that has gone.
fn pidfd_process(pidfd: usize) -> Result<alloc::sync::Arc<process::Process>, SyscallError> {
    let (_, pid, _) = super::pidfd::target(pidfd)?;
    process::get_process(pid).ok_or(SyscallError::ProcessNotFound)
}

/// process_madvise(pidfd, iovec, vlen, advice, flags), checked in Linux's
/// order: flags (EINVAL), at most UIO_MAXIOV segments (EINVAL), the iovec
/// readable (EFAULT), the pidfd (EBADF, ESRCH), ptrace read access to the
/// target with ADR 0024's MODIFY over it (EPERM, `debug::may_read_process`),
/// the advice; for another process only the advice it may be given
/// (EINVAL), with CAP_SYS_NICE (EPERM). Each segment is advised in turn;
/// the bytes advised before a failure are returned, else the failure.
pub fn sys_process_madvise(
    pidfd: usize,
    iov: usize,
    vlen: usize,
    advice: usize,
    flags: usize,
) -> SyscallResult {
    if flags as u32 != 0 || vlen > UIO_MAXIOV {
        return Err(SyscallError::InvalidArgument);
    }
    let raw = super::userspace::read_user_vec(iov, vlen * 16, UIO_MAXIOV * 16)?;
    // import_iovec: a negative length is EINVAL, and the total stops at
    // MAX_RW_COUNT (the segments past it are cut short), so the count
    // returned is never a negative ssize_t (PR #41 review).
    const MAX_RW_COUNT: usize = 0x7fff_f000;
    let mut total = 0usize;
    let mut segs: alloc::vec::Vec<(usize, usize)> = alloc::vec::Vec::new();
    for c in raw.chunks_exact(16) {
        let base = u64::from_ne_bytes(c[0..8].try_into().unwrap_or([0; 8]));
        let len = i64::from_ne_bytes(c[8..16].try_into().unwrap_or([0; 8]));
        if len < 0 {
            return Err(SyscallError::InvalidArgument);
        }
        let len = (len as usize).min(MAX_RW_COUNT - total);
        total += len;
        segs.push((base as usize, len));
    }
    // Unknown advice is EINVAL however many segments there are, as Linux
    // checks madvise_behavior_valid first (PR #41 review).
    if !super::memory::madvise_valid(advice) {
        return Err(SyscallError::InvalidArgument);
    }
    let target = pidfd_process(pidfd)?;
    let me = process::current_process().ok_or(SyscallError::InvalidState)?;
    let remote = target.pid != me.pid;
    if remote && !super::debug::may_read_process(&target) {
        return Err(SyscallError::OperationNotPermitted);
    }
    if remote {
        if !matches!(advice, MADV_COLD | MADV_PAGEOUT | MADV_WILLNEED) {
            return Err(SyscallError::InvalidArgument);
        }
        if !crate::process::creds::Caller::current()
            .capable(crate::process::linux_caps::CAP_SYS_NICE)
        {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    let mut done = 0usize;
    for (base, len) in segs {
        let r = if remote {
            remote_hint(&target, base, len)
        } else {
            super::memory::sys_madvise(base, len, advice).map(|_| ())
        };
        match r {
            Ok(()) => done = done.saturating_add(len),
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done)
}

/// A hint for another process's memory: an aligned start (EINVAL), the
/// range mapped (ENOMEM); nothing to do, as pages here are never paged out
/// or read ahead.
fn remote_hint(target: &process::Process, base: usize, len: usize) -> Result<(), SyscallError> {
    if base as u64 & (PAGE - 1) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let len = (len as u64)
        .checked_next_multiple_of(PAGE)
        .ok_or(SyscallError::InvalidArgument)?;
    if len == 0 {
        return Ok(());
    }
    let end = (base as u64)
        .checked_add(len)
        .ok_or(SyscallError::InvalidArgument)?;
    // The target's user space only (PR #41 review).
    if !in_user_space(end) {
        return Err(SyscallError::OutOfMemory);
    }
    if target
        .memory_space
        .lock()
        .hole_in(base as u64, end)
        .is_some()
    {
        return Err(SyscallError::OutOfMemory);
    }
    Ok(())
}

/// process_mrelease(pidfd, flags): flags 0 (EINVAL); the pidfd (EBADF,
/// ESRCH); the target must be dying -- killed by a fatal signal or already
/// exited (EINVAL otherwise). Its memory goes as it exits here, so there is
/// nothing to reap ahead of that: 0.
pub fn sys_process_mrelease(pidfd: usize, flags: usize) -> SyscallResult {
    use core::sync::atomic::Ordering;
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let target = pidfd_process(pidfd)?;
    let kill = target.kill_pending.load(Ordering::Acquire);
    let dying = kill != 0 && kill != process::exit::EXEC_PENDING
        || matches!(
            target.get_state(),
            process::ProcessState::Zombie | process::ProcessState::Dead
        );
    if !dying {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(0)
}

/// pkey_alloc's access-rights bits (PKEY_DISABLE_ACCESS, _WRITE; arm64
/// also has _EXECUTE and _READ).
#[cfg(target_arch = "aarch64")]
const PKEY_ACCESS_MASK: usize = 0xf;
#[cfg(not(target_arch = "aarch64"))]
const PKEY_ACCESS_MASK: usize = 0x3;

/// pkey_alloc(flags, init_val) as Linux answers without protection-key
/// hardware (x86 without OSPKE, arm64 without POE; the maintainer's choice
/// for every architecture until real keys come): flags 0 and known
/// access bits (EINVAL), then no key to give (ENOSPC).
pub fn sys_pkey_alloc(flags: usize, init_val: usize) -> SyscallResult {
    if flags != 0 || init_val & !PKEY_ACCESS_MASK != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    Err(SyscallError::NoSpace)
}

/// pkey_free(pkey): only key 0 exists, and it is never allocated by
/// pkey_alloc, so every key is EINVAL (the generic mm_pkey_free).
pub fn sys_pkey_free(_pkey: usize) -> SyscallResult {
    Err(SyscallError::InvalidArgument)
}

/// pkey_mprotect(addr, len, prot, pkey): mprotect's argument checks
/// (an unaligned start EINVAL, a zero length 0, a range that wraps
/// ENOMEM), then a key other than -1 (keep) or 0 (the default, the one key
/// there is) EINVAL; then mprotect.
pub fn sys_pkey_mprotect(addr: usize, len: usize, prot: usize, pkey: usize) -> SyscallResult {
    if addr as u64 & (PAGE - 1) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if len == 0 {
        return Ok(0);
    }
    (len as u64)
        .checked_next_multiple_of(PAGE)
        .and_then(|l| (addr as u64).checked_add(l))
        .ok_or(SyscallError::OutOfMemory)?;
    if !matches!(pkey as i32, -1 | 0) {
        return Err(SyscallError::InvalidArgument);
    }
    super::memory::sys_mprotect(addr, len, prot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mlock_rounds_the_start_down_and_the_length_up() {
        assert_eq!(lock_range(0x1234, 1), Some((0x1000, 0x2000)));
        assert_eq!(lock_range(0x1000, 0x1001), Some((0x1000, 0x3000)));
        assert_eq!(lock_range(0x1000, 0), Some((0x1000, 0x1000)));
        assert_eq!(lock_range(0x1fff, usize::MAX), None);
    }

    #[test]
    fn pkeys_answer_as_without_hardware() {
        assert_eq!(sys_pkey_alloc(1, 0), Err(SyscallError::InvalidArgument));
        assert_eq!(sys_pkey_alloc(0, 0x100), Err(SyscallError::InvalidArgument));
        assert_eq!(sys_pkey_alloc(0, 1), Err(SyscallError::NoSpace));
        assert_eq!(sys_pkey_free(0), Err(SyscallError::InvalidArgument));
        assert_eq!(
            sys_pkey_mprotect(0x1001, 1, 0, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(sys_pkey_mprotect(0x1000, 0, 0, 5), Ok(0));
        assert_eq!(
            sys_pkey_mprotect(0x1000, 1, 0, 5),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn mlockall_flags_as_linux() {
        assert!(!mlockall_flags_valid(0));
        assert!(!mlockall_flags_valid(MCL_ONFAULT));
        assert!(!mlockall_flags_valid(8));
        assert!(mlockall_flags_valid(MCL_CURRENT | MCL_ONFAULT));
        assert_eq!(
            mlockall_states(MCL_CURRENT),
            (Mlock::Unlocked, Some(Mlock::Locked))
        );
        assert_eq!(
            mlockall_states(MCL_FUTURE | MCL_ONFAULT),
            (Mlock::OnFault, None)
        );
    }

    #[test]
    fn mseal_and_msync_check_their_arguments() {
        assert_eq!(seal_range(0x1001, 1), None);
        assert_eq!(seal_range(0x1000, 1), Some((0x1000, 0x2000)));
        assert_eq!(seal_range(0x1000, usize::MAX), None);
        assert_eq!(
            msync_range(0x1000, 1, 8),
            Err(SyscallError::InvalidArgument),
            "unknown flag"
        );
        assert_eq!(
            msync_range(0x1001, 1, MS_SYNC),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            msync_range(0x1000, 1, MS_SYNC | MS_ASYNC),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(msync_range(0x1000, 0, MS_SYNC), Ok(None));
        let kernel = crate::mm::user_layout::user_space_end() & !0xfff;
        assert_eq!(
            msync_range(kernel, 0x1000, MS_SYNC),
            Err(SyscallError::OutOfMemory),
            "past user space"
        );
        assert_eq!(
            msync_range(usize::MAX & !0xfff, 0x2000, MS_SYNC),
            Err(SyscallError::OutOfMemory)
        );
    }
}

/// cachestat(fd, range, cstat, flags) (mm/filemap.c): how many pages of
/// `range` (`struct cachestat_range`: off, len; len 0 to the end of the
/// file) are cached and dirty, in `struct cachestat` (nr_cache, nr_dirty,
/// nr_writeback, nr_evicted, nr_recently_evicted; the last three are 0
/// here, ADR 0027). Checked as Linux: EBADF, EFAULT for the range, EPERM
/// unless the file is open for writing or the caller owns it or may write
/// it (can_do_cachestat), then EINVAL for flags.
pub fn sys_cachestat(fd: usize, range_ptr: usize, out_ptr: usize, flags: usize) -> SyscallResult {
    let file = super::file_calls::open_file(fd)?;
    let [off, len]: [u64; 2] = super::userspace::read_user(range_ptr)?;
    if !file.flags.write {
        let caller = crate::process::creds::Caller::current();
        let meta = file.node.metadata().map_err(super::map_kernel_error)?;
        if !caller.owner_or_capable(meta.uid)
            && !crate::fs::permission(&caller, &meta, crate::fs::MAY_WRITE)
        {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (first, last) = cachestat_pages(off, len);
    let stat = file.node.cache_stat(first, last);
    super::userspace::write_user(out_ptr, [stat.cached, stat.dirty, 0u64, 0, 0])?;
    Ok(0)
}

/// The pages `[off, off + len)` covers (len 0: to the end), as Linux
/// computes them in 64 bits.
fn cachestat_pages(off: u64, len: u64) -> (u64, u64) {
    let last = if len == 0 {
        u64::MAX
    } else {
        off.wrapping_add(len).wrapping_sub(1) >> 12
    };
    (off >> 12, last)
}

#[cfg(test)]
mod cachestat_tests {
    use super::*;

    #[test]
    fn the_range_is_counted_in_whole_pages() {
        assert_eq!(cachestat_pages(0, 0), (0, u64::MAX));
        assert_eq!(cachestat_pages(0, 1), (0, 0));
        assert_eq!(cachestat_pages(4095, 2), (0, 1));
        assert_eq!(cachestat_pages(8192, 4096), (2, 2));
    }

    #[test]
    fn nothing_past_the_end_of_the_file_counts() {
        assert_eq!(crate::fs::page_span(0, 0, u64::MAX), None);
        assert_eq!(crate::fs::page_span(1, 0, u64::MAX), Some((0, 0)));
        assert_eq!(crate::fs::page_span(8193, 1, u64::MAX), Some((1, 2)));
        assert_eq!(crate::fs::page_span(4096, 1, 5), None);
    }

    #[test]
    fn a_wrapped_range_counts_nothing() {
        let (first, last) = cachestat_pages(8192, u64::MAX - 4095);
        assert_eq!((first, last), (2, 0));
        assert_eq!(crate::fs::page_span(16384, first, last), None);
    }
}

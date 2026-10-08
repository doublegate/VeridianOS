//! poll, ppoll, select and pselect6 (N-194, N-206, N-258).
//!
//! All four look at descriptors' readiness ([`readiness`]) and sleep
//! between looks in [`wait_ready`]: until a watched file reports a change,
//! one becomes ready by itself (a timerfd's expiry), the timeout passes or
//! a signal interrupts (EINTR). ppoll and pselect6 apply a signal mask for
//! the wait ([`super::signal::begin_wait_sigmask`]); select, ppoll and
//! pselect6 write the time left back into their timeout, as Linux does.

use alloc::vec::Vec;

use super::{userspace, SyscallError, SyscallResult};
use crate::{fs::VfsNode, process};

pub(crate) const POLLIN: u16 = 0x001;
pub(crate) const POLLPRI: u16 = 0x002;
pub(crate) const POLLOUT: u16 = 0x004;
pub(crate) const POLLERR: u16 = 0x008;
pub(crate) const POLLHUP: u16 = 0x010;
pub(crate) const POLLNVAL: u16 = 0x020;
pub(crate) const POLLRDNORM: u16 = 0x040;
pub(crate) const POLLRDBAND: u16 = 0x080;
pub(crate) const POLLWRNORM: u16 = 0x100;
pub(crate) const POLLWRBAND: u16 = 0x200;

/// Descriptors one call may name (Linux: RLIMIT_NOFILE; select clamps to
/// the table size).
const MAX_FDS: usize = crate::fs::file::MAX_FDS;

/// A node's readiness, as Linux reports it ([`crate::fs::poll_bits`]).
pub(crate) fn readiness(node: &dyn VfsNode) -> u16 {
    crate::fs::poll_bits(node)
}

/// What one look at a set of descriptors found.
pub(crate) struct Scan {
    /// Descriptors (poll) or set bits (select) reported ready.
    pub ready: usize,
    /// Every file looked at reports its changes (no periodic re-check).
    pub precise: bool,
    /// The earliest time a file becomes ready by itself.
    pub wake_at: Option<u64>,
}

impl Scan {
    fn new() -> Self {
        Self {
            ready: 0,
            precise: true,
            wake_at: None,
        }
    }

    /// Account for watching `node` (how the wait must sleep).
    fn watch(&mut self, node: &dyn VfsNode) {
        self.precise &= node.wakes_io_waiters();
        self.wake_at = [self.wake_at, node.ready_at_ns()]
            .into_iter()
            .flatten()
            .min();
    }
}

/// The monotonic clock the waits use, in nanoseconds.
pub(crate) fn now_ns() -> u64 {
    crate::arch::timer::monotonic_ns()
}

/// Look with `scan` until it finds something ready or the timeout passes
/// (`None`: no limit, `Some(0)`: one look); returns the last look's count.
/// EINTR if a signal must be acted on first.
pub(crate) fn wait_ready(
    timeout_ns: Option<u64>,
    mut scan: impl FnMut() -> Result<Scan, SyscallError>,
) -> SyscallResult {
    // Boot-path cooperative dispatch: a child thread dispatched from
    // boot_futex_spin yields back to its parent after each system call, so
    // waiting here would block every other thread: a single look.
    #[cfg(target_arch = "x86_64")]
    let in_boot_coop = crate::arch::x86_64::usermode::BOOT_CLONE_YIELD_PENDING
        .load(core::sync::atomic::Ordering::Acquire);
    #[cfg(not(target_arch = "x86_64"))]
    let in_boot_coop = false;
    let timeout_ns = if in_boot_coop { Some(0) } else { timeout_ns };

    #[cfg(feature = "alloc")]
    let dispatched = crate::sched::dispatch::current_owner().is_some();
    let start = now_ns();
    let deadline = timeout_ns.map(|t| start.saturating_add(t));

    loop {
        #[cfg(feature = "alloc")]
        let seq = crate::sched::dispatch::io_seq();
        let found = scan()?;
        if found.ready > 0 || timeout_ns == Some(0) {
            return Ok(found.ready);
        }

        #[cfg(feature = "alloc")]
        if dispatched {
            use crate::sched::dispatch::{wait_io, WaitError};
            match wait_io(seq, deadline, found.precise, found.wake_at) {
                Ok(()) => continue,
                // One last look, as Linux takes after its timeout.
                Err(WaitError::TimedOut) => return scan().map(|s| s.ready),
                Err(WaitError::Interrupted) => return Err(SyscallError::Interrupted),
            }
        }

        // The boot context (no dispatcher): an infinite wait is capped at
        // 30 s so a stuck program cannot hang boot.
        if now_ns() >= deadline.unwrap_or(start.saturating_add(30_000_000_000)) {
            return scan().map(|s| s.ready);
        }
        // `sti; hlt` lets the timer interrupt advance the clock (SFMASK
        // cleared IF on syscall entry), then interrupts are off again.
        if crate::sched::wait_for_interrupt_in_syscall() {
            return Err(SyscallError::Interrupted);
        }
    }
}

/// A timespec as nanoseconds: EINVAL for a negative `tv_sec` or a `tv_nsec`
/// outside 0..1e9, as Linux's ppoll and pselect6.
pub(crate) fn timespec_ns(sec: i64, nsec: i64) -> Result<u64, SyscallError> {
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok((sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(nsec as u64))
}

/// A timeval as nanoseconds: EINVAL for a negative `tv_sec` or a `tv_usec`
/// outside 0..1e6, as Linux's select.
pub(crate) fn timeval_ns(sec: i64, usec: i64) -> Result<u64, SyscallError> {
    if sec < 0 || !(0..1_000_000).contains(&usec) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok((sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(usec as u64 * 1000))
}

/// The time left of a `timeout` wait that started at `start`.
fn remaining(timeout: u64, start: u64) -> u64 {
    timeout.saturating_sub(now_ns().saturating_sub(start))
}

/// Write the time left back into a user timespec (`usec` false) or timeval
/// (`usec` true), as Linux does for a non-zero timeout. A timeout in
/// read-only memory is not an error (Linux ignores the failed copy).
fn write_back_timeout(ptr: usize, timeout: u64, start: u64, usec: bool) {
    if ptr == 0 || timeout == 0 {
        return;
    }
    let left = remaining(timeout, start);
    let sec = (left / 1_000_000_000) as i64;
    let frac = (left % 1_000_000_000) as i64;
    let frac = if usec { frac / 1000 } else { frac };
    let _ = userspace::write_user::<[i64; 2]>(ptr, [sec, frac]);
}

// ============================================================================
// poll and ppoll
// ============================================================================

/// `struct pollfd`.
#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

// SAFETY: i32 + two i16, no padding (size 8); every bit pattern is valid.
unsafe impl userspace::UserPod for PollFd {}

/// What poll reports for a descriptor with readiness `ready` (or `None`
/// if it is not open) that asked for `events`: the requested bits, and
/// POLLERR and POLLHUP whether asked for or not; POLLNVAL for a closed
/// descriptor.
fn poll_revents(ready: Option<u16>, events: u16) -> u16 {
    match ready {
        None => POLLNVAL,
        Some(bits) => bits & (events | POLLERR | POLLHUP),
    }
}

/// poll(fds, nfds, timeout): a negative timeout waits without a limit.
pub fn sys_poll(fds_ptr: usize, nfds: usize, timeout_ms: usize) -> SyscallResult {
    let ms = timeout_ms as i32;
    poll_for(fds_ptr, nfds, (ms >= 0).then(|| ms as u64 * 1_000_000))
}

/// poll with a timeout in nanoseconds (`None`: no limit). The pollfd array
/// is copied in once and its `revents` written back on return, never used
/// in place (N-43).
pub(crate) fn poll_for(fds_ptr: usize, nfds: usize, timeout_ns: Option<u64>) -> SyscallResult {
    if nfds > MAX_FDS {
        return Err(SyscallError::InvalidArgument);
    }
    let mut pollfds = Vec::with_capacity(nfds);
    for i in 0..nfds {
        pollfds.push(userspace::read_user_index::<PollFd>(fds_ptr, i)?);
    }
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let result = wait_ready(timeout_ns, || {
        let mut scan = Scan::new();
        let table = proc.file_table.lock();
        for pfd in pollfds.iter_mut() {
            pfd.revents = 0;
            if pfd.fd < 0 {
                continue;
            }
            let ready = table.get(pfd.fd as usize).map(|file| {
                scan.watch(&*file.node);
                readiness(&*file.node)
            });
            pfd.revents = poll_revents(ready, pfd.events as u16) as i16;
            if pfd.revents != 0 {
                scan.ready += 1;
            }
        }
        Ok(scan)
    });
    if result.is_ok() {
        for (i, pfd) in pollfds.iter().enumerate() {
            userspace::write_user(fds_ptr + i * core::mem::size_of::<PollFd>(), *pfd)?;
        }
    }
    result
}

/// ppoll(fds, nfds, timeout, sigmask, sigsetsize): poll with a timespec
/// timeout (NULL: none) and, for the wait, the given signal mask (N-258).
pub(crate) fn sys_ppoll(
    fds_ptr: usize,
    nfds: usize,
    timespec_ptr: usize,
    sigmask_ptr: usize,
    sigsetsize: usize,
) -> SyscallResult {
    // Fault-tolerant read: a bad pointer is EFAULT, not a raw dereference
    // and not a silent zero timeout (review of the v0.26.0 stack, PR #14).
    let timeout = if timespec_ptr == 0 {
        None
    } else {
        let [sec, nsec] = userspace::read_user::<[i64; 2]>(timespec_ptr)?;
        Some(timespec_ns(sec, nsec)?)
    };
    let mask = super::signal::begin_wait_sigmask(sigmask_ptr, sigsetsize)?;
    let start = now_ns();
    let result = poll_for(fds_ptr, nfds, timeout);
    mask.end(&result);
    if let Some(timeout) = timeout {
        write_back_timeout(timespec_ptr, timeout, start, false);
    }
    result
}

// ============================================================================
// select and pselect6
// ============================================================================

/// What select reports for readiness `bits`: (readable, writable,
/// exceptional), with Linux's POLLIN_SET, POLLOUT_SET and POLLEX_SET.
fn select_classes(bits: u16) -> (bool, bool, bool) {
    let read = bits & (POLLIN | POLLRDNORM | POLLRDBAND | POLLHUP | POLLERR) != 0;
    let write = bits & (POLLOUT | POLLWRNORM | POLLWRBAND | POLLERR) != 0;
    let except = bits & POLLPRI != 0;
    (read, write, except)
}

/// An `fd_set` of `n` descriptors, as the `u64` words select copies.
struct FdSet {
    ptr: usize,
    words: Vec<u64>,
}

impl FdSet {
    /// Copy the set at `ptr` in (NULL: no set). Whole words are copied, as
    /// Linux copies whole longs.
    fn read(ptr: usize, n: usize) -> Result<Option<Self>, SyscallError> {
        if ptr == 0 {
            return Ok(None);
        }
        let mut words = alloc::vec![0u64; n.div_ceil(64)];
        for (i, word) in words.iter_mut().enumerate() {
            *word = userspace::read_user_index::<u64>(ptr, i)?;
        }
        // Bits past n are not looked at.
        if !n.is_multiple_of(64) {
            if let Some(last) = words.last_mut() {
                *last &= (1u64 << (n % 64)) - 1;
            }
        }
        Ok(Some(Self { ptr, words }))
    }

    fn has(&self, fd: usize) -> bool {
        self.words[fd / 64] & (1 << (fd % 64)) != 0
    }

    fn set(&mut self, fd: usize) {
        self.words[fd / 64] |= 1 << (fd % 64);
    }

    fn cleared(&self) -> Self {
        Self {
            ptr: self.ptr,
            words: alloc::vec![0; self.words.len()],
        }
    }

    fn write(&self) -> Result<(), SyscallError> {
        for (i, word) in self.words.iter().enumerate() {
            userspace::write_user(self.ptr + i * 8, *word)?;
        }
        Ok(())
    }
}

/// select's core: wait until a descriptor in `sets` (read, write,
/// exceptional) is ready, and leave only the ready ones in them. Every
/// descriptor named must be open (EBADF). On EINTR the sets are left as
/// they were.
fn select_for(nfds: usize, ptrs: [usize; 3], timeout_ns: Option<u64>) -> SyscallResult {
    if nfds as i32 <= -1 {
        return Err(SyscallError::InvalidArgument);
    }
    let n = nfds.min(MAX_FDS);
    let mut sets = [
        FdSet::read(ptrs[0], n)?,
        FdSet::read(ptrs[1], n)?,
        FdSet::read(ptrs[2], n)?,
    ];
    let named = |fd: usize| sets.iter().flatten().any(|s| s.has(fd));
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    {
        let table = proc.file_table.lock();
        if (0..n).any(|fd| named(fd) && table.get(fd).is_none()) {
            return Err(SyscallError::BadFileDescriptor);
        }
    }
    let wanted: Vec<usize> = (0..n).filter(|&fd| named(fd)).collect();
    let mut out: [Option<FdSet>; 3] = [
        sets[0].as_ref().map(FdSet::cleared),
        sets[1].as_ref().map(FdSet::cleared),
        sets[2].as_ref().map(FdSet::cleared),
    ];
    let result = wait_ready(timeout_ns, || {
        let mut scan = Scan::new();
        out = [
            sets[0].as_ref().map(FdSet::cleared),
            sets[1].as_ref().map(FdSet::cleared),
            sets[2].as_ref().map(FdSet::cleared),
        ];
        let table = proc.file_table.lock();
        for &fd in &wanted {
            // A descriptor closed during the wait is simply not ready.
            let Some(file) = table.get(fd) else {
                continue;
            };
            scan.watch(&*file.node);
            let (read, write, except) = select_classes(readiness(&*file.node));
            for (class, ready) in [read, write, except].into_iter().enumerate() {
                if let (Some(set), Some(res)) = (&sets[class], &mut out[class]) {
                    if ready && set.has(fd) {
                        res.set(fd);
                        scan.ready += 1;
                    }
                }
            }
        }
        Ok(scan)
    })?;
    for set in out.iter().flatten() {
        set.write()?;
    }
    sets = [None, None, None];
    let _ = sets;
    Ok(result)
}

/// select(nfds, readfds, writefds, exceptfds, timeout): a timeval timeout
/// (NULL: none), the time left written back into it.
pub fn sys_select(
    nfds: usize,
    readfds: usize,
    writefds: usize,
    exceptfds: usize,
    timeout_ptr: usize,
) -> SyscallResult {
    let timeout = if timeout_ptr == 0 {
        None
    } else {
        let [sec, usec] = userspace::read_user::<[i64; 2]>(timeout_ptr)?;
        Some(timeval_ns(sec, usec)?)
    };
    let start = now_ns();
    let result = select_for(nfds, [readfds, writefds, exceptfds], timeout);
    if let Some(timeout) = timeout {
        write_back_timeout(timeout_ptr, timeout, start, true);
    }
    result
}

/// pselect6(nfds, readfds, writefds, exceptfds, timeout, sig): a timespec
/// timeout, and `sig` points to `{ const sigset_t *ss; size_t ss_len; }`,
/// the signal mask for the wait (either pointer may be NULL).
pub fn sys_pselect6(
    nfds: usize,
    readfds: usize,
    writefds: usize,
    exceptfds: usize,
    timeout_ptr: usize,
    sig_ptr: usize,
) -> SyscallResult {
    let timeout = if timeout_ptr == 0 {
        None
    } else {
        let [sec, nsec] = userspace::read_user::<[i64; 2]>(timeout_ptr)?;
        Some(timespec_ns(sec, nsec)?)
    };
    let (mask_ptr, mask_len) = if sig_ptr == 0 {
        (0, 0)
    } else {
        let [ss, len] = userspace::read_user::<[u64; 2]>(sig_ptr)?;
        (ss as usize, len as usize)
    };
    let mask = super::signal::begin_wait_sigmask(mask_ptr, mask_len)?;
    let start = now_ns();
    let result = select_for(nfds, [readfds, writefds, exceptfds], timeout);
    mask.end(&result);
    if let Some(timeout) = timeout {
        write_back_timeout(timeout_ptr, timeout, start, false);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::normalize_poll_bits as normalize;

    #[test]
    fn readiness_pairs_normal_data_bits() {
        assert_eq!(normalize(POLLIN), POLLIN | POLLRDNORM);
        assert_eq!(normalize(POLLRDNORM), POLLIN | POLLRDNORM);
        assert_eq!(normalize(POLLOUT), POLLOUT | POLLWRNORM);
        assert_eq!(normalize(POLLHUP), POLLHUP);
        assert_eq!(
            normalize(POLLIN | POLLOUT),
            POLLIN | POLLRDNORM | POLLOUT | POLLWRNORM
        );
    }

    #[test]
    fn poll_reports_requested_bits_and_errors() {
        let both = normalize(POLLIN | POLLOUT);
        assert_eq!(poll_revents(Some(both), POLLIN as u16), POLLIN);
        // A program asking for POLLRDNORM alone gets it (N-206).
        assert_eq!(poll_revents(Some(both), POLLRDNORM), POLLRDNORM);
        assert_eq!(poll_revents(Some(POLLHUP | POLLERR), 0), POLLHUP | POLLERR);
        assert_eq!(poll_revents(Some(POLLOUT), POLLIN), 0);
        assert_eq!(poll_revents(None, POLLIN), POLLNVAL);
    }

    #[test]
    fn select_classes_follow_linux_sets() {
        assert_eq!(select_classes(normalize(POLLIN)), (true, false, false));
        assert_eq!(select_classes(normalize(POLLOUT)), (false, true, false));
        // Hang-up is readable (EOF); an error is readable and writable.
        assert_eq!(select_classes(POLLHUP), (true, false, false));
        assert_eq!(select_classes(POLLERR), (true, true, false));
        assert_eq!(select_classes(POLLPRI), (false, false, true));
        assert_eq!(select_classes(0), (false, false, false));
    }

    #[test]
    fn timeouts_are_validated_like_linux() {
        assert_eq!(timespec_ns(1, 500_000_000), Ok(1_500_000_000));
        assert_eq!(timespec_ns(0, 999_999_999), Ok(999_999_999));
        assert_eq!(timespec_ns(-1, 0), Err(SyscallError::InvalidArgument));
        assert_eq!(timespec_ns(0, -1), Err(SyscallError::InvalidArgument));
        assert_eq!(
            timespec_ns(0, 1_000_000_000),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(timespec_ns(i64::MAX, 0), Ok(u64::MAX));
        assert_eq!(timeval_ns(2, 250_000), Ok(2_250_000_000));
        assert_eq!(timeval_ns(0, 1_000_000), Err(SyscallError::InvalidArgument));
        assert_eq!(timeval_ns(-1, 0), Err(SyscallError::InvalidArgument));
    }

    #[test]
    fn fd_set_words() {
        let mut set = FdSet {
            ptr: 0,
            words: alloc::vec![0; 2],
        };
        set.set(0);
        set.set(65);
        assert!(set.has(0) && set.has(65) && !set.has(64));
        let cleared = set.cleared();
        assert!(!cleared.has(0) && !cleared.has(65));
        assert_eq!(cleared.words.len(), 2);
    }
}

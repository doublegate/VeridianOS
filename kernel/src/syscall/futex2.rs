//! The futex2 calls (v0.28 item 5; `kernel/futex/syscalls.c`):
//! futex_waitv, futex_wake, futex_wait and futex_requeue, on the futex
//! table the futex(2) operations use (`syscall::futex`).
//!
//! Only 32-bit futexes exist, as on Linux. FUTEX2_PRIVATE is a hint here
//! as there: the key comes from the mapping (`futex::futex_key`).
//! FUTEX2_NUMA doubles the word: its second half is the node, which on
//! this one-node system must be FUTEX_NO_NODE (-1, then set to 0) or 0.

use super::{futex, SyscallError, SyscallResult};

/// The futex2 flags (`include/uapi/linux/futex.h`).
pub const FUTEX2_SIZE_U8: u32 = 0x00;
pub const FUTEX2_SIZE_U32: u32 = 0x02;
pub const FUTEX2_SIZE_MASK: u32 = 0x03;
pub const FUTEX2_NUMA: u32 = 0x04;
pub const FUTEX2_MPOL: u32 = 0x08;
pub const FUTEX2_PRIVATE: u32 = 0x80;
const FUTEX2_VALID_MASK: u32 = FUTEX2_SIZE_MASK | FUTEX2_NUMA | FUTEX2_MPOL | FUTEX2_PRIVATE;
/// The most futexes one futex_waitv waits on.
pub const FUTEX_WAITV_MAX: usize = 128;
/// A NUMA futex's node word before a node is chosen.
const FUTEX_NO_NODE: u32 = u32::MAX;
const CLOCK_REALTIME: usize = 0;
const CLOCK_MONOTONIC: usize = 1;

/// futex2_to_flags and futex_flags_valid: bits outside the valid mask are
/// EINVAL, and so is any size but 32 bits (the only one implemented).
pub fn check_flags(flags: u32) -> Result<(), SyscallError> {
    if flags & !FUTEX2_VALID_MASK != 0 || flags & FUTEX2_SIZE_MASK != FUTEX2_SIZE_U32 {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// futex_validate_input for a 32-bit futex: a value or mask must fit.
pub fn fits(v: u64) -> bool {
    v >> 32 == 0
}

/// get_futex_key's checks of the address: aligned to the futex's size
/// (doubled with FUTEX2_NUMA, EINVAL), and for FUTEX2_NUMA the node word:
/// FUTEX_NO_NODE becomes node 0, the only node; any other node than 0 is
/// EINVAL.
pub fn check_address(uaddr: usize, flags: u32) -> Result<(), SyscallError> {
    let size = if flags & FUTEX2_NUMA != 0 { 8 } else { 4 };
    if uaddr == 0 || !uaddr.is_multiple_of(size) {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & FUTEX2_NUMA != 0 {
        let naddr = uaddr + 4;
        let node: u32 = super::userspace::read_user(naddr)?;
        match node {
            0 => {}
            FUTEX_NO_NODE => super::userspace::write_user(naddr, 0u32)?,
            _ => return Err(SyscallError::InvalidArgument),
        }
    }
    Ok(())
}

/// futex2_setup_timeout: an absolute `struct timespec` on CLOCK_REALTIME
/// or CLOCK_MONOTONIC (another clock EINVAL), as a monotonic deadline.
fn deadline(timeout: usize, clockid: usize) -> Result<Option<u64>, SyscallError> {
    if timeout == 0 {
        return Ok(None);
    }
    let kind = match clockid as u32 as usize {
        CLOCK_REALTIME => futex::FutexTimeout::Realtime,
        CLOCK_MONOTONIC => futex::FutexTimeout::Monotonic,
        _ => return Err(SyscallError::InvalidArgument),
    };
    futex::linux_timeout(timeout, kind)
}

/// futex_wake(uaddr, mask, nr, flags): the number woken. A mask of 0 is
/// EINVAL; `nr` 0 or below wakes none (FLAGS_STRICT).
pub fn sys_futex_wake(uaddr: usize, mask: usize, nr: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    check_flags(flags)?;
    if !fits(mask as u64) || mask == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    check_address(uaddr, flags)?;
    let nr = nr as i32;
    if nr <= 0 {
        return Ok(0);
    }
    futex::sys_futex_wake(uaddr, nr as usize, mask).map(|n| n as usize)
}

/// futex_wait(uaddr, val, mask, flags, timeout, clockid): 0 once woken,
/// EAGAIN if the word is not `val`, ETIMEDOUT at the absolute timeout.
pub fn sys_futex_wait(
    uaddr: usize,
    val: usize,
    mask: usize,
    flags: usize,
    timeout: usize,
) -> SyscallResult {
    let clockid = super::syscall_arg6()?;
    let flags = flags as u32;
    check_flags(flags)?;
    if !fits(val as u64) || !fits(mask as u64) {
        return Err(SyscallError::InvalidArgument);
    }
    let deadline = deadline(timeout, clockid)?;
    if mask == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    check_address(uaddr, flags)?;
    futex::futex_wait_until(uaddr, val as u32, deadline, mask as u32).map(|_| 0)
}

/// One `struct futex_waitv`: val (u64), uaddr (u64), flags (u32), a
/// reserved u32 that must be 0.
pub fn parse_waitv(raw: &[u8]) -> Result<(usize, u32, u32), SyscallError> {
    let val = u64::from_ne_bytes(raw[0..8].try_into().unwrap_or([0; 8]));
    let uaddr = u64::from_ne_bytes(raw[8..16].try_into().unwrap_or([0; 8]));
    let flags = u32::from_ne_bytes(raw[16..20].try_into().unwrap_or([0; 4]));
    let reserved = u32::from_ne_bytes(raw[20..24].try_into().unwrap_or([0; 4]));
    if flags & !FUTEX2_VALID_MASK != 0 || reserved != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    check_flags(flags)?;
    if !fits(val) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok((uaddr as usize, val as u32, flags))
}

fn read_waitv(ptr: usize, n: usize) -> Result<alloc::vec::Vec<(usize, u32, u32)>, SyscallError> {
    let raw = super::userspace::read_user_vec(ptr, n * 24, n * 24)?;
    raw.chunks_exact(24).map(parse_waitv).collect()
}

/// futex_waitv(waiters, nr_futexes, flags, timeout, clockid): the index of
/// the futex that woke the caller. `flags` must be 0, and 1 to 128
/// futexes, `waiters` not NULL (EINVAL); the timeout is set up before the
/// vector is read, as Linux orders it.
pub fn sys_futex_waitv(
    waiters: usize,
    nr: usize,
    flags: usize,
    timeout: usize,
    clockid: usize,
) -> SyscallResult {
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let nr = nr as u32 as usize;
    if nr == 0 || nr > FUTEX_WAITV_MAX || waiters == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let deadline = deadline(timeout, clockid)?;
    let v = read_waitv(waiters, nr)?;
    let mut entries = alloc::vec::Vec::with_capacity(nr);
    for &(uaddr, val, f) in &v {
        check_address(uaddr, f)?;
        entries.push((uaddr, val));
    }
    if crate::sched::dispatch::current_owner().is_none() {
        return Err(SyscallError::InvalidState);
    }
    futex::dwait_multiple(&entries, deadline)
}

/// futex_requeue(waiters, flags, nr_wake, nr_requeue): wake up to
/// `nr_wake` on the first futex and move up to `nr_requeue` more to the
/// second, if the first still holds its `val` (EAGAIN otherwise). `flags`
/// must be 0 and the two futexes' flags equal (EINVAL).
pub fn sys_futex_requeue(
    waiters: usize,
    flags: usize,
    nr_wake: usize,
    nr_requeue: usize,
) -> SyscallResult {
    if flags as u32 != 0 || waiters == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let v = read_waitv(waiters, 2)?;
    if v[0].2 != v[1].2 {
        return Err(SyscallError::InvalidArgument);
    }
    let (nw, nq) = (nr_wake as i32, nr_requeue as i32);
    if nw < 0 || nq < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    check_address(v[0].0, v[0].2)?;
    check_address(v[1].0, v[1].2)?;
    futex::sys_futex_cmp_requeue(v[0].0, nw as usize, v[1].0, nq as usize, v[0].1)
        .map(|n| n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_32_bit_futexes_with_known_flags() {
        assert_eq!(check_flags(FUTEX2_SIZE_U32), Ok(()));
        assert_eq!(
            check_flags(FUTEX2_SIZE_U32 | FUTEX2_PRIVATE | FUTEX2_NUMA),
            Ok(())
        );
        assert_eq!(
            check_flags(FUTEX2_SIZE_U8),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(check_flags(0x03), Err(SyscallError::InvalidArgument));
        assert_eq!(
            check_flags(FUTEX2_SIZE_U32 | 0x10),
            Err(SyscallError::InvalidArgument)
        );
        assert!(fits(u32::MAX as u64));
        assert!(!fits(1 << 32));
    }

    #[test]
    fn waitv_entries_are_checked_as_linux_parses_them() {
        let mut raw = [0u8; 24];
        raw[0..8].copy_from_slice(&7u64.to_ne_bytes());
        raw[8..16].copy_from_slice(&0x1000u64.to_ne_bytes());
        raw[16..20].copy_from_slice(&FUTEX2_SIZE_U32.to_ne_bytes());
        assert_eq!(parse_waitv(&raw), Ok((0x1000, 7, FUTEX2_SIZE_U32)));
        let mut bad = raw;
        bad[20] = 1;
        assert_eq!(
            parse_waitv(&bad),
            Err(SyscallError::InvalidArgument),
            "reserved"
        );
        let mut big = raw;
        big[0..8].copy_from_slice(&(1u64 << 32).to_ne_bytes());
        assert_eq!(
            parse_waitv(&big),
            Err(SyscallError::InvalidArgument),
            "value too wide"
        );
    }

    #[test]
    fn addresses_must_be_aligned_to_the_futex_size() {
        assert_eq!(
            check_address(0, FUTEX2_SIZE_U32),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            check_address(0x1002, FUTEX2_SIZE_U32),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(check_address(0x1004, FUTEX2_SIZE_U32), Ok(()));
        assert_eq!(
            check_address(0x1004, FUTEX2_SIZE_U32 | FUTEX2_NUMA),
            Err(SyscallError::InvalidArgument),
            "a NUMA futex is a doubled word"
        );
    }
}

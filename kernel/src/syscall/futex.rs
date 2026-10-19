//! Futex syscall handlers (FUTEX_WAIT, FUTEX_WAKE, FUTEX_REQUEUE,
//! FUTEX_WAKE_OP).
//!
//! Implements Linux-compatible futex (fast userspace mutex) operations keyed by
//! per-process user virtual address.  The implementation enforces:
//! - 32-bit aligned futex words
//! - Per-process isolation (same address in different processes does not alias)
//! - Atomic re-check of expected value before sleeping
//! - Bitset-aware wake filtering for `FUTEX_WAIT_BITSET` / `FUTEX_WAKE` callers

use alloc::vec::Vec;

use spin::Mutex;

use crate::{
    process, sched,
    syscall::{userspace::validate_user_ptr, SyscallError},
};

// FUTEX_WAKE_OP operation encoding, exactly as Linux's FUTEX_OP():
//   (op & 0xf) << 28 | (cmp & 0xf) << 24 | (oparg & 0xfff) << 12 | (cmparg &
// 0xfff) oparg and cmparg are sign-extended 12-bit values; comparisons are
// signed.
/// `op` flag: the operand is `1 << oparg` rather than `oparg`.
const FUTEX_OP_OPARG_SHIFT: u32 = 8;

// Supported operations (subset)
const FUTEX_OP_SET: u32 = 0; // *(int *)uaddr2 = oparg
const FUTEX_OP_ADD: u32 = 1; // *(int *)uaddr2 += oparg
const FUTEX_OP_OR: u32 = 2;
const FUTEX_OP_ANDN: u32 = 3;
const FUTEX_OP_XOR: u32 = 4;

// Supported compares (subset)
const FUTEX_CMP_EQ: u32 = 0;
const FUTEX_CMP_NE: u32 = 1;
const FUTEX_CMP_LT: u32 = 2;
const FUTEX_CMP_LE: u32 = 3;
const FUTEX_CMP_GT: u32 = 4;
const FUTEX_CMP_GE: u32 = 5;

// Futex operations (subset)
const FUTEX_WAIT: u32 = 0;
const FUTEX_WAKE: u32 = 1;
const FUTEX_REQUEUE: u32 = 3;
const FUTEX_WAIT_BITSET: u32 = 9;
const FUTEX_WAKE_OP: u32 = 5;

/// Special bitset value that matches any waiter, equivalent to plain
/// `FUTEX_WAKE`.  Linux defines this as `FUTEX_BITSET_MATCH_ANY`.
pub(crate) const FUTEX_WAIT_BITSET_MATCH_ANY: u32 = 0xFFFF_FFFF;

// Futex wait queue keyed by (pid, uaddr)
pub(super) type FutexKey = (u64, usize);

/// Number of futex hash buckets (power of two).
const FUTEX_BUCKETS: usize = 256;

/// Bucket index for a key: multiplicative hash of the word index and pid
/// (SYS-PERF-02: waits and wakes on unrelated futexes do not share a lock).
fn bucket_index(key: FutexKey) -> usize {
    let (pid, uaddr) = key;
    let h = ((uaddr as u64) >> 2) ^ pid.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as usize % FUTEX_BUCKETS
}

/// Perform a `FUTEX_WAIT` or `FUTEX_WAIT_BITSET` operation.
///
/// Atomically checks that `*uaddr == expected` and, if so, suspends the
/// calling thread on the per-process wait queue keyed by `uaddr`.  The thread
/// is woken by a corresponding `FUTEX_WAKE` (or `FUTEX_WAKE_OP` /
/// `FUTEX_REQUEUE`) call, by a timeout, or by an incoming signal.
///
/// # Arguments
///
/// * `uaddr`       - User-space address of a 32-bit aligned futex word.
/// * `expected`    - Value that `*uaddr` must equal for the wait to proceed.
/// * `timeout_ptr` - Optional pointer to a `u64` timeout value (ticks). Zero
///   means no timeout.
/// * `aux`         - For `FUTEX_WAIT_BITSET`: the 32-bit bitset mask (must be
///   non-zero).  For plain `FUTEX_WAIT`: must be `sizeof(u64)` when a timeout
///   is supplied.
/// * `op`          - Raw futex operation field (includes command + flags).
///
/// # Returns
///
/// `Ok(0)` on successful wake, or an error:
/// - `InvalidArgument` if alignment / argument validation fails.
/// - `WouldBlock` if `*uaddr != expected` or if the timeout expired.
/// - `Interrupted` if a signal was pending when the thread woke.
pub fn sys_futex_wait(
    uaddr: usize,
    expected: u32,
    timeout_ptr: usize,
    aux: usize,
    op: usize,
) -> Result<isize, SyscallError> {
    // Validate alignment and address
    if uaddr == 0 || uaddr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // Must reside in user space (single validation -- no duplicate call)
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;

    // Fast path: the word already differs. The authoritative check is
    // repeated under the bucket lock below.
    if crate::syscall::userspace::read_user::<u32>(uaddr)? != expected {
        return Err(SyscallError::WouldBlock);
    }

    // Determine whether this is FUTEX_WAIT or FUTEX_WAIT_BITSET
    let op_base = (op as u32) & 0xF;
    let bitset_mask = if op_base == FUTEX_WAIT_BITSET {
        let mask = aux as u32;
        if mask == 0 {
            return Err(SyscallError::InvalidArgument);
        }
        mask
    } else {
        FUTEX_WAIT_BITSET_MATCH_ANY
    };

    // Optional timeout: the native ABI passes a u64 of milliseconds,
    // relative, or absolute since boot with bit 8 of `op`.
    let deadline = if timeout_ptr != 0 {
        if aux != 0 && op_base != FUTEX_WAIT_BITSET && aux != core::mem::size_of::<u64>() {
            // For plain WAIT the caller must supply sizeof(u64) to document
            // the layout; for WAIT_BITSET we treat `aux` as the mask instead.
            return Err(SyscallError::InvalidArgument);
        }
        validate_user_ptr(timeout_ptr as *const u64, core::mem::size_of::<u64>())?;
        let ms: u64 = super::userspace::read_user(timeout_ptr)?;
        let ns = ms.saturating_mul(1_000_000);
        if (op & 0x100) != 0 {
            Some(ns)
        } else {
            Some(crate::timer::monotonic_ns().saturating_add(ns))
        }
    } else {
        None
    };
    futex_wait_until(uaddr, expected, deadline, bitset_mask)
}

/// How a Linux futex timeout is meant.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FutexTimeout {
    /// Relative to now (FUTEX_WAIT, whatever clock flag it has).
    Relative,
    /// An absolute time on CLOCK_MONOTONIC (FUTEX_WAIT_BITSET).
    Monotonic,
    /// An absolute time on CLOCK_REALTIME (FUTEX_WAIT_BITSET with
    /// FUTEX_CLOCK_REALTIME: musl's timed mutex, condition and semaphore
    /// waits).
    Realtime,
}

/// A Linux futex timeout: the `struct timespec` at `ptr` (none if null),
/// read as `kind` says (Linux's futex_init_timeout). Returns the
/// `monotonic_ns` deadline (N-104: these used to be read as raw ticks).
pub(crate) fn linux_timeout(ptr: usize, kind: FutexTimeout) -> Result<Option<u64>, SyscallError> {
    if ptr == 0 {
        return Ok(None);
    }
    let [sec, nsec]: [i64; 2] = super::userspace::read_user(ptr)?;
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return Err(SyscallError::InvalidArgument);
    }
    let ns = (sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(nsec as u64);
    Ok(Some(match kind {
        FutexTimeout::Relative => crate::timer::monotonic_ns().saturating_add(ns),
        FutexTimeout::Monotonic => ns,
        FutexTimeout::Realtime => {
            crate::timer::realtime::monotonic_at(ns.min(i64::MAX as u64) as i64)
        }
    }))
}

/// Wait on the futex word at `uaddr` while it holds `expected`, until
/// woken (by a wake whose bitset meets `bitset`), until `deadline`
/// (`monotonic_ns`; ETIMEDOUT), or until a signal must be acted on (EINTR).
pub(crate) fn futex_wait_until(
    uaddr: usize,
    expected: u32,
    deadline: Option<u64>,
    bitset_mask: u32,
) -> Result<isize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 || bitset_mask == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    if crate::syscall::userspace::read_user::<u32>(uaddr)? != expected {
        return Err(SyscallError::WouldBlock);
    }

    // A thread running as its own dispatcher task (stage D2) sleeps on a
    // keyed wait (see `dwait`). The old paths below would either block on
    // the old scheduler, which never runs, or run sibling threads nested
    // from their saved context.
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return dwait(uaddr, expected, deadline, bitset_mask);
    }

    // Not a dispatched thread: no user thread runs otherwise (the old
    // scheduler's futex table and the nested boot dispatch went in sprint
    // D4, N-279).
    Err(SyscallError::InvalidState)
}

/// Perform a `FUTEX_WAKE` operation, waking up to `num_wake` threads
/// waiting on the futex word at `uaddr`.
///
/// Only waiters whose bitset overlaps with `wake_bitset` are eligible.
/// A `wake_bitset` of `FUTEX_WAIT_BITSET_MATCH_ANY` (0xFFFF_FFFF) wakes
/// any waiter regardless of its individual bitset.
///
/// # Arguments
///
/// * `uaddr`        - User-space address of the 32-bit aligned futex word.
/// * `num_wake`     - Maximum number of waiters to wake.
/// * `wake_bitset`  - Bitset mask for selective waking.  Pass
///   `FUTEX_WAIT_BITSET_MATCH_ANY` for unconditional wake.
///
/// # Returns
///
/// `Ok(n)` where `n` is the number of waiters actually woken, or an error
/// if argument validation fails.
pub fn sys_futex_wake(
    uaddr: usize,
    num_wake: usize,
    wake_bitset: usize,
) -> Result<isize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;

    // Interpret the wake bitset: 0 means the caller did not supply one
    // (plain FUTEX_WAKE), so default to match-any.
    let wake_bits: u32 = if wake_bitset == 0 {
        FUTEX_WAIT_BITSET_MATCH_ANY
    } else {
        wake_bitset as u32
    };

    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return dwake(uaddr, num_wake, wake_bits);
    }

    // Not a dispatched thread: no user thread runs otherwise (the old
    // scheduler's futex table and the nested boot dispatch went in sprint
    // D4, N-279).
    Err(SyscallError::InvalidState)
}

/// Top-level futex dispatcher matching the Linux `futex(2)` parameter order.
///
/// Decodes the operation from the `op` field and delegates to the appropriate
/// handler.  Validates alignment and user-space addresses up front.
///
/// # Arguments
///
/// * `uaddr`  - Primary futex word address (must be 4-byte aligned, in user
///   space).
/// * `val`    - Interpretation depends on operation: expected value (WAIT),
///   wake count (WAKE/REQUEUE), etc.
/// * `uaddr2` - Secondary futex address for `FUTEX_REQUEUE` / `FUTEX_WAKE_OP`.
/// * `val3`   - Auxiliary value: requeue count, timeout size, or bitset.
/// * `op`     - Futex operation code plus optional flags (e.g.
///   `FUTEX_CLOCK_REALTIME`).
///
/// # Returns
///
/// Depends on the sub-operation; see individual handler documentation.
pub fn sys_futex_dispatch(
    uaddr: usize,
    val: usize,
    uaddr2: usize,
    val3: usize,
    op: usize,
) -> Result<isize, SyscallError> {
    // Enforce user-space alignment up front
    if uaddr == 0 || uaddr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;

    if (op as u32 & FUTEX_REQUEUE) != 0 || (op as u32 & FUTEX_WAKE_OP) != 0 {
        if uaddr2 == 0 || uaddr2 & 0x3 != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        validate_user_ptr(uaddr2 as *const u32, core::mem::size_of::<u32>())?;
    }

    match (op as u32) & 0xF {
        FUTEX_WAIT => sys_futex_wait(uaddr, val as u32, uaddr2, val3, op),
        FUTEX_WAIT_BITSET => sys_futex_wait(uaddr, val as u32, uaddr2, val3, op),
        FUTEX_WAKE => sys_futex_wake(uaddr, val, uaddr2),
        FUTEX_REQUEUE => sys_futex_requeue(uaddr, val, uaddr2, val3),
        // Native ABI has no separate val2: wake up to `val` on both words.
        FUTEX_WAKE_OP => sys_futex_wake_op(uaddr, val, uaddr2, val, val3),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// Perform a `FUTEX_WAKE_OP` operation, with Linux semantics: atomically
/// apply the encoded operation to `*uaddr2`, wake up to `val` waiters on
/// `uaddr`, and -- if the encoded comparison holds for the *old* value of
/// `*uaddr2` -- also wake up to `val2` waiters on `uaddr2`.
///
/// `encoded_op` is Linux's `FUTEX_OP(op, oparg, cmp, cmparg)`; see
/// [`wake_op_eval`].
///
/// # Returns
///
/// The total number of waiters woken.
pub fn sys_futex_wake_op(
    uaddr: usize,
    val: usize,
    uaddr2: usize,
    val2: usize,
    encoded_op: usize,
) -> Result<isize, SyscallError> {
    // For safety, require alignment and same-process addresses.
    if uaddr == 0 || uaddr & 0x3 != 0 || uaddr2 == 0 || uaddr2 & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    validate_user_ptr(uaddr2 as *const u32, core::mem::size_of::<u32>())?;

    let encoded = encoded_op as u32;
    // Reject a malformed encoding before touching user memory.
    wake_op_eval(encoded, 0)?;

    // Atomic read-modify-write of *uaddr2: other threads update it
    // concurrently; a volatile read + write lost updates (SYS-CONC-01).
    // Both accesses go through the fault-tolerant accessors, so a
    // read-only page, or one unmapped by another thread between the two,
    // fails with EFAULT rather than faulting in the kernel.
    let mut old = crate::syscall::userspace::read_user::<u32>(uaddr2)?;
    let cmp_ok = loop {
        let (new, cmp_ok) = wake_op_eval(encoded, old)?;
        let found = crate::syscall::userspace::cmpxchg_user_u32(uaddr2, old, new)?;
        if found == old {
            break cmp_ok;
        }
        old = found;
    };

    // Linux semantics: always wake up to `val` waiters on uaddr; if the
    // comparison held, also wake up to `val2` waiters on uaddr2.
    let mut woken = sys_futex_wake(uaddr, val, FUTEX_WAIT_BITSET_MATCH_ANY as usize)?;
    if cmp_ok {
        woken += sys_futex_wake(uaddr2, val2, FUTEX_WAIT_BITSET_MATCH_ANY as usize)?;
    }
    Ok(woken)
}

/// Perform a `FUTEX_REQUEUE` operation: wake up to `wake_count` threads on
/// `uaddr`, then move up to `requeue_count` remaining waiters from `uaddr`
/// to `uaddr2`.
///
/// This is used by `pthread_cond_broadcast` and similar primitives to
/// efficiently transfer waiters from a condition variable's futex to a
/// mutex's futex without thundering-herd wakeups.
///
/// # Arguments
///
/// * `uaddr`         - Source futex word address.
/// * `wake_count`    - Maximum number of waiters to wake immediately.
/// * `uaddr2`        - Destination futex word address for requeued waiters.
/// * `requeue_count` - Maximum number of waiters to move to `uaddr2`.
///
/// # Returns
///
/// `Ok(n)` where `n` is the total number of waiters woken plus requeued.
pub fn sys_futex_requeue(
    uaddr: usize,
    wake_count: usize,
    uaddr2: usize,
    requeue_count: usize,
) -> Result<isize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 || uaddr2 == 0 || uaddr2 & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if uaddr == uaddr2 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    validate_user_ptr(uaddr2 as *const u32, core::mem::size_of::<u32>())?;

    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return drequeue(uaddr, wake_count, uaddr2, requeue_count, None);
    }

    // Not a dispatched thread: no user thread runs otherwise (the old
    // scheduler's futex table and the nested boot dispatch went in sprint
    // D4, N-279).
    Err(SyscallError::InvalidState)
}

// ============================================================================
// Boot-path futex spin for processes launched outside the scheduler
// ============================================================================

/// A dispatched thread waiting on a futex. It sleeps on its own wait
/// queue until a waker sets `woken`; FUTEX_REQUEUE moves the entry to
/// another key without touching the sleeper.
#[cfg(feature = "alloc")]
struct DWaiter {
    key: FutexKey,
    bitset: u32,
    woken: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
    wq: alloc::sync::Arc<sched::dispatch::WaitQueue>,
    /// The `DFUTEX` bucket holding the entry, changed only by a requeue
    /// holding both buckets' locks: a waiter leaving by timeout or signal
    /// finds its entry there (PR #23 review: a scan of the buckets one by
    /// one could miss an entry a requeue moved behind it).
    bucket: alloc::sync::Arc<core::sync::atomic::AtomicUsize>,
}

/// Move up to `nmove` waiters on `k1` in `lo` to `k2`: into `hi` (its
/// bucket `to`), or, when both keys hash to the bucket of `lo` (`hi` is
/// `None`), retagged where they are -- pushing them back onto `lo` would
/// meet them again and loop until `nmove`, which the caller chooses
/// (PR #23 review). Returns how many moved.
#[cfg(feature = "alloc")]
fn move_waiters(
    lo: &mut Vec<DWaiter>,
    hi: Option<&mut Vec<DWaiter>>,
    to: usize,
    k1: FutexKey,
    k2: FutexKey,
    nmove: usize,
) -> usize {
    use core::sync::atomic::Ordering;
    let mut moved = 0usize;
    match hi {
        Some(h) => {
            let mut i = 0;
            while i < lo.len() && moved < nmove {
                if lo[i].key == k1 {
                    let mut w = lo.remove(i);
                    w.key = k2;
                    w.bucket.store(to, Ordering::Release);
                    h.push(w);
                    moved += 1;
                } else {
                    i += 1;
                }
            }
        }
        None => {
            for w in lo.iter_mut().filter(|w| w.key == k1).take(nmove) {
                w.key = k2;
                moved += 1;
            }
        }
    }
    moved
}

/// Dispatched futex waiters, hashed by key like `FUTEX_TABLE`.
#[cfg(feature = "alloc")]
static DFUTEX: [crate::mm::cache_aligned::CacheAligned<Mutex<Vec<DWaiter>>>; FUTEX_BUCKETS] =
    [const { crate::mm::cache_aligned::CacheAligned::new(Mutex::new(Vec::new())) }; FUTEX_BUCKETS];

/// Key of the futex word at `uaddr` for the calling process: the physical
/// address when the word is in `MAP_SHARED` memory, so waiters in every
/// process mapping it meet (N-114), otherwise (pid, address).
#[cfg(feature = "alloc")]
pub(super) fn futex_key(uaddr: usize) -> Result<FutexKey, SyscallError> {
    /// The "pid" of keys for shared memory.
    const SHARED: u64 = u64::MAX;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let shared = proc
        .memory_space
        .lock()
        .shared_phys_addr(crate::mm::VirtualAddress(uaddr as u64));
    Ok(match shared {
        Some(phys) => (SHARED, phys as usize),
        None => (proc.pid.0, uaddr),
    })
}

/// Take up to `n` waiters on `key` whose bitset meets `bitset` out of
/// `list`, marking them woken; returns their queues.
#[cfg(feature = "alloc")]
fn take_waiters(
    list: &mut Vec<DWaiter>,
    key: FutexKey,
    bitset: u32,
    n: usize,
) -> Vec<alloc::sync::Arc<sched::dispatch::WaitQueue>> {
    let mut woken = Vec::new();
    let mut i = 0;
    while i < list.len() && woken.len() < n {
        if list[i].key == key && list[i].bitset & bitset != 0 {
            let w = list.remove(i);
            w.woken.store(true, core::sync::atomic::Ordering::Release);
            woken.push(w.wq);
        } else {
            i += 1;
        }
    }
    woken
}

/// FUTEX_WAIT for a dispatched thread: queued under the bucket lock after
/// re-checking the word (a wake that changed it either came first, and we
/// return EAGAIN, or finds us queued), then asleep until woken, the
/// deadline (ETIMEDOUT) or a signal (EINTR). It used to poll the word.
#[cfg(feature = "alloc")]
fn dwait(
    uaddr: usize,
    expected: u32,
    deadline: Option<u64>,
    bitset: u32,
) -> Result<isize, SyscallError> {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};

    use sched::dispatch::{self, WaitError};

    let key = futex_key(uaddr)?;
    let woken = Arc::new(AtomicBool::new(false));
    let wq = Arc::new(dispatch::WaitQueue::new());
    let bucket = Arc::new(core::sync::atomic::AtomicUsize::new(bucket_index(key)));
    {
        let mut list = DFUTEX[bucket_index(key)].lock();
        if super::userspace::read_user::<u32>(uaddr)? != expected {
            return Err(SyscallError::WouldBlock);
        }
        list.push(DWaiter {
            key,
            bitset,
            woken: woken.clone(),
            wq: wq.clone(),
            bucket: bucket.clone(),
        });
    }
    let result = dispatch::wait_event(&wq, deadline, || woken.load(Ordering::Acquire));
    if result.is_ok() {
        return Ok(0);
    }
    // Timed out or interrupted: leave the queue -- wherever a requeue put
    // us -- unless a waker took us first, which counts as woken. With the
    // bucket's lock held, an entry not there and a bucket index unchanged
    // mean a waker removed it; a changed index means a requeue moved it.
    let found = loop {
        let at = bucket.load(Ordering::Acquire);
        let mut list = DFUTEX[at].lock();
        if let Some(i) = list.iter().position(|w| Arc::ptr_eq(&w.woken, &woken)) {
            list.remove(i);
            break true;
        }
        if bucket.load(Ordering::Acquire) == at {
            break false;
        }
    };
    if !found && woken.load(Ordering::Acquire) {
        return Ok(0);
    }
    Err(match result {
        Err(WaitError::TimedOut) => SyscallError::TimedOut,
        _ => SyscallError::Interrupted,
    })
}

/// FUTEX_WAKE for dispatched waiters.
#[cfg(feature = "alloc")]
fn dwake(uaddr: usize, n: usize, bitset: u32) -> Result<isize, SyscallError> {
    let key = futex_key(uaddr)?;
    let queues = take_waiters(&mut DFUTEX[bucket_index(key)].lock(), key, bitset, n);
    for q in &queues {
        q.wake_all();
    }
    Ok(queues.len() as isize)
}

/// FUTEX_REQUEUE / FUTEX_CMP_REQUEUE for dispatched waiters: wake up to
/// `nwake` on `uaddr` and move up to `nmove` more to `uaddr2`. With `cmp`,
/// nothing happens (EAGAIN) unless `*uaddr` still equals it.
#[cfg(feature = "alloc")]
fn drequeue(
    uaddr: usize,
    nwake: usize,
    uaddr2: usize,
    nmove: usize,
    cmp: Option<u32>,
) -> Result<isize, SyscallError> {
    let (k1, k2) = (futex_key(uaddr)?, futex_key(uaddr2)?);
    let (i1, i2) = (bucket_index(k1), bucket_index(k2));
    // Both buckets in index order, so opposite requeues cannot deadlock.
    let (mut lo, mut hi) = match i1.cmp(&i2) {
        core::cmp::Ordering::Less => (DFUTEX[i1].lock(), Some(DFUTEX[i2].lock())),
        core::cmp::Ordering::Greater => {
            let hi = DFUTEX[i2].lock();
            (DFUTEX[i1].lock(), Some(hi))
        }
        core::cmp::Ordering::Equal => (DFUTEX[i1].lock(), None),
    };
    if let Some(v) = cmp {
        if super::userspace::read_user::<u32>(uaddr)? != v {
            return Err(SyscallError::WouldBlock);
        }
    }
    let woken = take_waiters(&mut lo, k1, FUTEX_WAIT_BITSET_MATCH_ANY, nwake);
    let moved = move_waiters(&mut lo, hi.as_deref_mut(), i2, k1, k2, nmove);
    drop(hi);
    drop(lo);
    for q in &woken {
        q.wake_all();
    }
    Ok((woken.len() + moved) as isize)
}

/// FUTEX_CMP_REQUEUE: requeue only if `*uaddr == cmp` (EAGAIN otherwise).
pub fn sys_futex_cmp_requeue(
    uaddr: usize,
    nwake: usize,
    uaddr2: usize,
    nmove: usize,
    cmp: u32,
) -> Result<isize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 || uaddr2 == 0 || uaddr2 & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    validate_user_ptr(uaddr2 as *const u32, core::mem::size_of::<u32>())?;
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return drequeue(uaddr, nwake, uaddr2, nmove, Some(cmp));
    }
    let _ = (nwake, nmove, cmp);
    Err(SyscallError::InvalidState)
}

/// Evaluate a FUTEX_WAKE_OP encoding against the old value of `*uaddr2`:
/// returns the value to store and whether the comparison holds.
fn wake_op_eval(encoded: u32, old: u32) -> Result<(u32, bool), SyscallError> {
    let op = encoded >> 28;
    let cmp = (encoded >> 24) & 0xF;
    let mut oparg = ((encoded << 8) as i32) >> 20;
    let cmparg = ((encoded << 20) as i32) >> 20;
    if op & FUTEX_OP_OPARG_SHIFT != 0 {
        if !(0..32).contains(&oparg) {
            return Err(SyscallError::InvalidArgument);
        }
        oparg = (1u32 << oparg) as i32;
    }
    let old_s = old as i32;
    let new = match op & !FUTEX_OP_OPARG_SHIFT {
        FUTEX_OP_SET => oparg,
        FUTEX_OP_ADD => old_s.wrapping_add(oparg),
        FUTEX_OP_OR => old_s | oparg,
        FUTEX_OP_ANDN => old_s & !oparg,
        FUTEX_OP_XOR => old_s ^ oparg,
        _ => return Err(SyscallError::InvalidArgument),
    };
    let cmp_ok = match cmp {
        FUTEX_CMP_EQ => old_s == cmparg,
        FUTEX_CMP_NE => old_s != cmparg,
        FUTEX_CMP_LT => old_s < cmparg,
        FUTEX_CMP_LE => old_s <= cmparg,
        FUTEX_CMP_GT => old_s > cmparg,
        FUTEX_CMP_GE => old_s >= cmparg,
        _ => return Err(SyscallError::InvalidArgument),
    };
    Ok((new as u32, cmp_ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "alloc")]
    fn waiter(key: FutexKey) -> DWaiter {
        use alloc::sync::Arc;
        DWaiter {
            key,
            bitset: FUTEX_WAIT_BITSET_MATCH_ANY,
            woken: Arc::new(core::sync::atomic::AtomicBool::new(false)),
            wq: Arc::new(sched::dispatch::WaitQueue::new()),
            bucket: Arc::new(core::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// A requeue between two keys of one bucket ends, moving what it may:
    /// it pushed each waiter back onto the same list and met it again,
    /// looping under the bucket lock until `nmove` (from the user) moved
    /// (PR #23 review).
    #[cfg(feature = "alloc")]
    #[test]
    fn requeue_within_one_bucket_terminates() {
        let (k1, k2, other) = ((1, 0x1000), (1, 0x2000), (1, 0x3000));
        let mut lo = alloc::vec![waiter(k1), waiter(other), waiter(k1)];
        assert_eq!(move_waiters(&mut lo, None, 0, k1, k2, usize::MAX), 2);
        assert_eq!(lo.iter().filter(|w| w.key == k2).count(), 2);
        assert_eq!(lo.iter().filter(|w| w.key == other).count(), 1);
        // A limit is honoured, and between buckets the moved ones say where
        // they went.
        let mut lo = alloc::vec![waiter(k1), waiter(k1), waiter(k1)];
        let mut hi = alloc::vec::Vec::new();
        assert_eq!(move_waiters(&mut lo, Some(&mut hi), 9, k1, k2, 2), 2);
        assert_eq!((lo.len(), hi.len()), (1, 2));
        assert!(hi
            .iter()
            .all(|w| w.key == k2 && w.bucket.load(core::sync::atomic::Ordering::Acquire) == 9));
    }

    #[test]
    fn futex_keys_spread_over_buckets() {
        // Adjacent futex words of one process (a typical mutex array) and the
        // same address in different processes land in many buckets.
        let mut used = [false; FUTEX_BUCKETS];
        for i in 0..1024usize {
            used[bucket_index((7, 0x7000_0000 + i * 4))] = true;
        }
        assert!(used.iter().filter(|&&u| u).count() > FUTEX_BUCKETS * 3 / 4);
        let mut used = [false; FUTEX_BUCKETS];
        for pid in 0..1024u64 {
            used[bucket_index((pid, 0x7000_0000))] = true;
        }
        assert!(used.iter().filter(|&&u| u).count() > FUTEX_BUCKETS * 3 / 4);
        // Stable: the same key always maps to the same bucket.
        assert_eq!(bucket_index((3, 0x1234)), bucket_index((3, 0x1234)));
    }

    /// Linux's FUTEX_OP() macro.
    const fn futex_op(op: u32, oparg: u32, cmp: u32, cmparg: u32) -> u32 {
        ((op & 0xF) << 28) | ((cmp & 0xF) << 24) | ((oparg & 0xFFF) << 12) | (cmparg & 0xFFF)
    }

    /// SYS-CONC-01: the fields used to be read from the wrong bits (op and
    /// cmp swapped, oparg and cmparg swapped).
    #[test]
    fn wake_op_decodes_linux_layout() {
        // *uaddr2 += 5; wake uaddr2 if old > 1.
        let enc = futex_op(FUTEX_OP_ADD, 5, FUTEX_CMP_GT, 1);
        assert_eq!(wake_op_eval(enc, 3), Ok((8, true)));
        assert_eq!(wake_op_eval(enc, 1), Ok((6, false)));
        // *uaddr2 = 0; wake if old == 1 (glibc's pthread_cond_signal shape).
        let enc = futex_op(FUTEX_OP_SET, 0, FUTEX_CMP_EQ, 1);
        assert_eq!(wake_op_eval(enc, 1), Ok((0, true)));
        assert_eq!(wake_op_eval(enc, 2), Ok((0, false)));
        assert_eq!(
            wake_op_eval(futex_op(FUTEX_OP_ANDN, 0b110, FUTEX_CMP_NE, 0), 0b111),
            Ok((0b001, true))
        );
        assert_eq!(
            wake_op_eval(futex_op(FUTEX_OP_XOR, 0xFF, FUTEX_CMP_LE, 0), 0x0F),
            Ok((0xF0, false))
        );
    }

    #[test]
    fn wake_op_sign_extends_and_shifts() {
        // oparg 0xFFF is -1: ADD -1 decrements.
        assert_eq!(
            wake_op_eval(futex_op(FUTEX_OP_ADD, 0xFFF, FUTEX_CMP_EQ, 0), 5),
            Ok((4, false))
        );
        // cmparg 0xFFF is -1, compared signed.
        assert_eq!(
            wake_op_eval(futex_op(FUTEX_OP_SET, 0, FUTEX_CMP_LT, 0xFFF), u32::MAX - 1),
            Ok((0, true))
        );
        // OPARG_SHIFT: OR (1 << 4).
        let enc = futex_op(FUTEX_OP_OR | FUTEX_OP_OPARG_SHIFT, 4, FUTEX_CMP_GE, 0);
        assert_eq!(wake_op_eval(enc, 1), Ok((17, true)));
        let enc = futex_op(FUTEX_OP_SET | FUTEX_OP_OPARG_SHIFT, 31, FUTEX_CMP_EQ, 0);
        assert_eq!(wake_op_eval(enc, 0), Ok((0x8000_0000, true)));
    }

    #[test]
    fn wake_op_rejects_bad_encodings() {
        assert!(wake_op_eval(futex_op(5, 0, FUTEX_CMP_EQ, 0), 0).is_err());
        assert!(wake_op_eval(futex_op(FUTEX_OP_SET, 0, 6, 0), 0).is_err());
        // Shift counts outside 0..32 (oparg is signed: 0xFFF = -1).
        assert!(wake_op_eval(futex_op(FUTEX_OP_SET | FUTEX_OP_OPARG_SHIFT, 32, 0, 0), 0).is_err());
        assert!(wake_op_eval(
            futex_op(FUTEX_OP_SET | FUTEX_OP_OPARG_SHIFT, 0xFFF, 0, 0),
            0
        )
        .is_err());
    }
}

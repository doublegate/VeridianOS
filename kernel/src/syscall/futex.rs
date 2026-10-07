//! Futex syscall handlers (FUTEX_WAIT, FUTEX_WAKE, FUTEX_REQUEUE,
//! FUTEX_WAKE_OP).
//!
//! Implements Linux-compatible futex (fast userspace mutex) operations keyed by
//! per-process user virtual address.  The implementation enforces:
//! - 32-bit aligned futex words
//! - Per-process isolation (same address in different processes does not alias)
//! - Atomic re-check of expected value before sleeping
//! - Bitset-aware wake filtering for `FUTEX_WAIT_BITSET` / `FUTEX_WAKE` callers

use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use crate::{
    arch::timer::get_ticks,
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
const FUTEX_WAIT_BITSET_MATCH_ANY: u32 = 0xFFFF_FFFF;

// Futex wait queue keyed by (pid, uaddr)
type FutexKey = (u64, usize);

struct FutexWaiter {
    task: core::ptr::NonNull<sched::task::Task>,
    priority: u8,
    /// Bitset supplied by the waiting thread.  A waker only unblocks this
    /// waiter when `wake_bitset & waiter.bitset != 0`.
    bitset: u32,
}

// SAFETY: FutexWaiter holds a NonNull<Task> that is only accessed while the
// futex bucket lock is held or by the scheduler after the waiter has been
// dequeued.  Send/Sync are required so the BTreeMap can live in a static
// Mutex, which is safe because all accesses are serialised by the spinlock.
unsafe impl Send for FutexWaiter {}
// SAFETY: as for Send above -- every access to the task pointer is
// serialised by the bucket spinlock or happens after dequeue.
unsafe impl Sync for FutexWaiter {}

/// Waiters for one hash bucket.
type FutexBucket = Mutex<BTreeMap<FutexKey, Vec<FutexWaiter>>>;

/// Number of futex hash buckets (power of two).
const FUTEX_BUCKETS: usize = 256;

/// Futex waiters, hashed by (pid, uaddr) into independently locked,
/// cache-line-aligned buckets (SYS-PERF-02): waits and wakes on unrelated
/// futexes no longer serialize on one global lock.
static FUTEX_TABLE: [crate::mm::cache_aligned::CacheAligned<FutexBucket>; FUTEX_BUCKETS] =
    [const { crate::mm::cache_aligned::CacheAligned::new(Mutex::new(BTreeMap::new())) };
        FUTEX_BUCKETS];

/// Bucket index for a key: multiplicative hash of the word index and pid.
fn bucket_index(key: FutexKey) -> usize {
    let (pid, uaddr) = key;
    let h = ((uaddr as u64) >> 2) ^ pid.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as usize % FUTEX_BUCKETS
}

/// The bucket holding `key`'s waiters.
fn bucket(key: FutexKey) -> &'static FutexBucket {
    &FUTEX_TABLE[bucket_index(key)]
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

    // Optional timeout: expect a u64 ticks value
    let deadline = if timeout_ptr != 0 {
        if aux != 0 && op_base != FUTEX_WAIT_BITSET && aux != core::mem::size_of::<u64>() {
            // For plain WAIT the caller must supply sizeof(u64) to document
            // the layout; for WAIT_BITSET we treat `aux` as the mask instead.
            return Err(SyscallError::InvalidArgument);
        }
        validate_user_ptr(timeout_ptr as *const u64, core::mem::size_of::<u64>())?;
        let rel: u64 = super::userspace::read_user(timeout_ptr)?;
        // If op uses absolute time (FUTEX_CLOCK_REALTIME bit), treat rel as absolute
        // ticks
        if (op & 0x100) != 0 {
            Some(rel)
        } else {
            Some(get_ticks().saturating_add(rel))
        }
    } else {
        None
    };

    // A thread running as its own dispatcher task (stage D2) waits through
    // the dispatcher. The old paths below would either block on the old
    // scheduler, which never runs, or run sibling threads nested from their
    // saved context -- a second copy of a thread that is already running.
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return dispatched_futex_wait(uaddr, expected, deadline);
    }

    let pid = process::current_process()
        .ok_or(SyscallError::InvalidState)?
        .pid
        .0;
    let key = (pid, uaddr);

    // Check if we are in boot-launched mode (no scheduler task for this thread).
    // In the boot path, user processes run directly via iretq from
    // enter_usermode_returnable, not through the scheduler.  The scheduler has
    // no "current task" for us, so we cannot block via the normal
    // task-state-based path.  Instead, cooperatively dispatch child threads
    // from the scheduler's ready queue and spin-poll the futex word.
    let has_sched_task = {
        let sched = crate::sched::scheduler::current_scheduler();
        let slock = sched.lock();
        slock.current().is_some()
    };

    if !has_sched_task {
        return boot_futex_spin(uaddr, expected, deadline);
    }

    let task_ptr = {
        // The value check and the enqueue happen under the bucket lock, which
        // every waker of this futex also takes: a wake that changes the word
        // either happens before the check (we return WouldBlock) or finds us
        // queued. Checking before taking the lock lost wakeups in between.
        let mut table = bucket(key).lock();
        if crate::syscall::userspace::read_user::<u32>(uaddr)? != expected {
            return Err(SyscallError::WouldBlock);
        }
        let sched = crate::sched::scheduler::current_scheduler();
        let slock = sched.lock();
        let task = slock.current().ok_or(SyscallError::InvalidState)?;
        // SAFETY: We hold the scheduler lock, which guarantees exclusive
        // access to the current task's state field.  The pointer is valid
        // because it was obtained from the scheduler's active task list.
        let prio = unsafe {
            (*task.as_ptr()).state = process::ProcessState::Blocked;
            (*task.as_ptr()).priority as u8
        };
        drop(slock);
        table.entry(key).or_default().push(FutexWaiter {
            task,
            priority: prio,
            bitset: bitset_mask,
        });
        task
    };

    // reschedule
    sched::SCHEDULER.lock().schedule();

    // Helper to remove this waiter from the queue (used on timeout/interruption)
    let remove_self = |reason: SyscallError| -> Result<isize, SyscallError> {
        let mut table = bucket(key).lock();
        if let Some(waiters) = table.get_mut(&key) {
            waiters.retain(|w| w.task != task_ptr);
            if waiters.is_empty() {
                table.remove(&key);
            }
        }
        Err(reason)
    };

    // If awoken, distinguish signals vs normal wake/timeout. Pending signals
    // are tracked at the process level.
    if let Some(proc) = process::current_process() {
        if proc
            .pending_signals
            .load(core::sync::atomic::Ordering::Acquire)
            != 0
        {
            return remove_self(SyscallError::Interrupted);
        }
    }

    // Check timeout after wake
    if let Some(deadline) = deadline {
        let now = get_ticks();
        if now >= deadline {
            return remove_self(SyscallError::WouldBlock);
        }
    }

    Ok(0)
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

    let pid = process::current_process()
        .ok_or(SyscallError::InvalidState)?
        .pid
        .0;
    let key = (pid, uaddr);
    let mut woken = 0;

    let mut to_wake: Vec<core::ptr::NonNull<sched::task::Task>> = Vec::new();
    {
        let mut table = bucket(key).lock();
        if let Some(waiters) = table.get_mut(&key) {
            // Sort by priority descending, then FIFO
            waiters.sort_by(|a, b| b.priority.cmp(&a.priority));

            // Drain eligible waiters whose bitset overlaps with wake_bits
            let mut i = 0;
            while i < waiters.len() && woken < num_wake {
                if waiters[i].bitset & wake_bits != 0 {
                    let w = waiters.remove(i);
                    to_wake.push(w.task);
                    woken += 1;
                } else {
                    i += 1;
                }
            }
            if waiters.is_empty() {
                table.remove(&key);
            }
        }
    }

    // Wake tasks outside the table lock to minimise lock hold time
    if woken > 0 {
        let scheduler = crate::sched::scheduler::current_scheduler();
        let slock = scheduler.lock();
        for task_ptr in to_wake {
            // SAFETY: `task_ptr` was obtained from the futex wait queue and
            // is guaranteed to be valid because blocked tasks are not freed
            // while they reside on a wait queue.  We transition the task
            // from Blocked -> Ready and re-enqueue it in the scheduler.
            unsafe {
                (*task_ptr.as_ptr()).state = process::ProcessState::Ready;
            }
            slock.enqueue(task_ptr);
        }
    }

    Ok(woken as isize)
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

    let pid = process::current_process()
        .ok_or(SyscallError::InvalidState)?
        .pid
        .0;
    let key1 = (pid, uaddr);
    let key2 = (pid, uaddr2);

    let mut woken = 0;
    let mut moved = 0;
    let mut to_wake: Vec<core::ptr::NonNull<sched::task::Task>> = Vec::new();
    let mut to_move: Vec<FutexWaiter> = Vec::new();

    {
        // Both buckets are held while waiters move, taken in index order so
        // two requeues in opposite directions cannot deadlock.
        let (i1, i2) = (bucket_index(key1), bucket_index(key2));
        let (mut first, mut second) = match i1.cmp(&i2) {
            core::cmp::Ordering::Less => (FUTEX_TABLE[i1].lock(), Some(FUTEX_TABLE[i2].lock())),
            core::cmp::Ordering::Greater => {
                let hi = FUTEX_TABLE[i2].lock();
                let lo = FUTEX_TABLE[i1].lock();
                (lo, Some(hi))
            }
            core::cmp::Ordering::Equal => (FUTEX_TABLE[i1].lock(), None),
        };
        let src = &mut *first;
        if let Some(waiters) = src.get_mut(&key1) {
            waiters.sort_by(|a, b| b.priority.cmp(&a.priority));
            let wc = core::cmp::min(wake_count, waiters.len());
            for w in waiters.drain(..wc) {
                to_wake.push(w.task);
                woken += 1;
            }
            let rc = core::cmp::min(requeue_count, waiters.len());
            to_move.extend(waiters.drain(..rc));
            moved = to_move.len();
            if waiters.is_empty() {
                src.remove(&key1);
            }
        }

        if moved > 0 {
            let dst = match second.as_mut() {
                Some(g) => &mut **g,
                None => &mut *first,
            };
            dst.entry(key2).or_default().extend(to_move);
        }
    }

    if woken > 0 {
        let scheduler = crate::sched::scheduler::current_scheduler();
        let slock = scheduler.lock();
        for task_ptr in to_wake {
            // SAFETY: `task_ptr` was obtained from the futex wait queue and
            // is valid because blocked tasks are not freed while on a queue.
            // We transition Blocked -> Ready and re-enqueue.
            unsafe {
                (*task_ptr.as_ptr()).state = process::ProcessState::Ready;
            }
            slock.enqueue(task_ptr);
        }
    }

    Ok((woken + moved) as isize)
}

// ============================================================================
// Boot-path futex spin for processes launched outside the scheduler
// ============================================================================

/// FUTEX_WAIT for a dispatched thread: re-check the word and let other
/// tasks run until it changes, the deadline passes, or the process receives
/// a fatal signal. Wakers need not find it: it polls (a futex wait queue
/// with direct wakeup replaces this with the blocking primitives).
#[cfg(feature = "alloc")]
fn dispatched_futex_wait(
    uaddr: usize,
    expected: u32,
    deadline: Option<u64>,
) -> Result<isize, SyscallError> {
    loop {
        let cur: u32 = super::userspace::read_user(uaddr)?;
        if cur != expected {
            return Ok(0);
        }
        if deadline.is_some_and(|dl| get_ticks() >= dl) {
            return Err(SyscallError::WouldBlock);
        }
        if crate::sched::dispatch::wait_in_syscall() {
            return Err(SyscallError::Interrupted);
        }
    }
}

/// Cooperative futex wait for boot-launched processes.
///
/// When the parent thread has no scheduler task (running via direct iretq from
/// the boot path), we cannot block through the normal scheduler.  Instead, we
/// spin-poll the futex word while dispatching child threads from the scheduler
/// ready queue.
///
/// Each iteration:
/// 1. Re-read the futex word -- if changed, return `Ok(0)`.
/// 2. Dequeue a ready child task from the scheduler.
/// 3. Set the `BOOT_CLONE_YIELD_PENDING` flag so `syscall_handler` yields after
///    the child's next syscall.
/// 4. Dispatch the child via `enter_forked_child_returnable` (blocks until
///    `boot_return_to_kernel` is called from the child's syscall path).
/// 5. Repeat.
///
/// Timeout handling: if a deadline is given and expires, return `WouldBlock`.
#[cfg(target_arch = "x86_64")]
fn boot_futex_spin(
    uaddr: usize,
    expected: u32,
    deadline: Option<u64>,
) -> Result<isize, SyscallError> {
    use core::sync::atomic::Ordering;

    use crate::arch::x86_64::usermode::{
        ForkChildRegs, BOOT_CLONE_YIELD_PENDING, BOOT_RETURN_CR3, BOOT_RETURN_RSP,
        BOOT_STACK_CANARY,
    };

    const MAX_SPINS: u32 = 100_000;
    // After this many consecutive iterations with no dispatchable child tasks,
    // assume all children are dead and stop waiting.
    const MAX_EMPTY_ITERS: u32 = 50;

    // Cache parent identity before the loop so we can skip the parent's own
    // task when dequeuing from the scheduler.
    let parent_pid = crate::process::current_process()
        .map(|p| p.pid)
        .unwrap_or(crate::process::ProcessId(0));
    let parent_tid = crate::process::current_thread()
        .map(|t| t.tid)
        .unwrap_or(crate::process::thread::ThreadId(0));

    let mut empty_iters: u32 = 0;

    for _ in 0..MAX_SPINS {
        // Re-check the futex word
        // Re-read fault-tolerantly: the word may have been unmapped since
        // the first check (N-43).
        let cur: u32 = super::userspace::read_user(uaddr)?;
        if cur != expected {
            return Ok(0);
        }

        // Check timeout
        if let Some(dl) = deadline {
            if get_ticks() >= dl {
                return Err(SyscallError::WouldBlock);
            }
        }

        // Try to dequeue a ready child task and dispatch it.
        // Only the parent's own clone() threads qualify: another process's
        // task or a kernel task would otherwise run in ring 3 under the
        // parent's boot context (review of the v0.26.0 stack, PR #14).
        let child_task = {
            let sched = crate::sched::scheduler::current_scheduler();
            let slock = sched.lock();
            let mut found = None;
            // Drain a bounded number of tasks looking for a sibling thread;
            // every task that does not qualify goes back on the queue.
            let mut skipped = alloc::vec::Vec::new();
            for _ in 0..BOOT_DISPATCH_SCAN {
                match slock.pick_next() {
                    Some(t) => {
                        // SAFETY: a task handed out by the scheduler is valid
                        // while we hold it; we only read its ids.
                        let (pid, tid) = unsafe { (t.as_ref().pid, t.as_ref().tid) };
                        if is_boot_dispatchable(pid, tid, parent_pid, parent_tid) {
                            found = Some(t);
                            break;
                        }
                        skipped.push(t);
                    }
                    None => break,
                }
            }
            // Re-enqueue any skipped tasks
            for t in skipped {
                slock.enqueue(t);
            }
            found
        };

        let child_task = match child_task {
            Some(t) => {
                empty_iters = 0;
                t
            }
            None => {
                empty_iters += 1;
                if empty_iters >= MAX_EMPTY_ITERS {
                    // No dispatchable child tasks for many iterations.
                    // All children likely exited/crashed. Return WouldBlock
                    // so the parent can continue.
                    return Err(SyscallError::WouldBlock);
                }
                // Halt until the APIC timer fires (advances UPTIME_MS)
                // without letting it preempt this syscall (W-13). The boot
                // path is never dispatched, so it is never interrupted.
                let _ = crate::sched::wait_for_interrupt_in_syscall();
                continue;
            }
        };

        // Extract the child's thread context for user-mode dispatch.
        // `child_task` is already off the run queue, so every path that does
        // not dispatch it must put it back, or it is lost for good.
        let requeue = |t| {
            let sched = crate::sched::scheduler::current_scheduler();
            sched.lock().enqueue(t);
        };
        // SAFETY: child_task was handed out by the scheduler and is off the
        // run queue, so this code exclusively holds it and the leaked Task
        // allocation is live; it is only read here.
        let (regs, child_pid, child_tid, cr3) = unsafe {
            let task_ref = child_task.as_ref();

            // Find the child's ThreadContext via the process table.
            let proc = match crate::process::get_process(task_ref.pid) {
                Some(p) => p,
                None => {
                    requeue(child_task);
                    continue;
                }
            };

            let thread = match proc.get_thread(task_ref.tid) {
                Some(t) => t,
                None => {
                    requeue(child_task);
                    continue;
                }
            };

            let ctx = thread.context.lock();
            let r = ForkChildRegs {
                rip: ctx.rip,
                rsp: ctx.rsp,
                rflags: ctx.rflags | 0x200, // Ensure IF is set
                rax: ctx.rax,
                rbx: ctx.rbx,
                rcx: ctx.rcx,
                rdx: ctx.rdx,
                rsi: ctx.rsi,
                rdi: ctx.rdi,
                rbp: ctx.rbp,
                r8: ctx.r8,
                r9: ctx.r9,
                r10: ctx.r10,
                r11: ctx.r11,
                r12: ctx.r12,
                r13: ctx.r13,
                r14: ctx.r14,
                r15: ctx.r15,
                fs_base: ctx.tls_base,
            };
            drop(ctx);

            let cr3 = proc.memory_space.lock().get_page_table();
            (r, task_ref.pid, task_ref.tid, cr3)
        };

        if cr3 == 0 {
            requeue(child_task);
            continue;
        }

        // Save parent's boot return context
        let saved_rsp = BOOT_RETURN_RSP.load(Ordering::SeqCst);
        let saved_cr3 = BOOT_RETURN_CR3.load(Ordering::SeqCst);
        let saved_canary = BOOT_STACK_CANARY.load(Ordering::SeqCst);

        let per_cpu = crate::arch::x86_64::syscall::per_cpu_data_ptr();
        // SAFETY: per_cpu points to this CPU's PerCpuData, initialized during
        // boot and live for the kernel lifetime; we are in syscall context on
        // this CPU, so nothing else writes it concurrently.
        let saved_kernel_rsp = unsafe { (*per_cpu).kernel_rsp };
        // SAFETY: same per-CPU pointer as above.
        let saved_user_rsp = unsafe { (*per_cpu).user_rsp };
        // The child's syscalls replace the frame pointer; the parent's
        // syscall still needs its own (sixth argument, fork, return).
        // SAFETY: same per-CPU pointer as above.
        let saved_syscall_frame = unsafe { (*per_cpu).syscall_frame };

        // Save parent's FS_BASE.  boot_return_to_kernel zeroes FS via
        // `mov fs, ax`, which clears FS_BASE.  We restore it after the
        // child dispatch so the parent's TLS remains correct.
        // SAFETY: RDMSR of IA32_FS_BASE (0xC0000100) in Ring 0 only reads the
        // current FS base.
        let saved_fs_base: u64 = unsafe {
            let lo: u32;
            let hi: u32;
            core::arch::asm!(
                "rdmsr",
                in("ecx") 0xC0000100u32,
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack),
            );
            ((hi as u64) << 32) | (lo as u64)
        };

        // Set child as boot-current
        crate::process::set_boot_current(child_pid, child_tid);

        // Set the yield flag so syscall_handler returns to us after the
        // child's next syscall.
        BOOT_CLONE_YIELD_PENDING.store(true, Ordering::Release);

        // Dispatch child to user mode (blocks until boot_return_to_kernel)
        // SAFETY: `regs` is a local ForkChildRegs in kernel memory, mapped in
        // every address space; cr3 is the child's non-zero page table root
        // (checked above); GS_BASE holds the per-CPU block (ring 0), and the
        // parent's boot return context was saved above for restoration
        // afterwards.
        unsafe {
            crate::arch::x86_64::usermode::enter_forked_child_returnable(&regs, cr3);
        }

        // Restore parent's FS_BASE (zeroed by boot_return_to_kernel)
        // SAFETY: WRMSR of IA32_FS_BASE in Ring 0 with the value read from
        // the same MSR above, so it is a canonical address.
        unsafe {
            core::arch::asm!(
                "wrmsr",
                in("ecx") 0xC0000100u32,
                in("eax") saved_fs_base as u32,
                in("edx") (saved_fs_base >> 32) as u32,
                options(nomem, nostack),
            );
        }

        // Restore parent's per-CPU state
        // SAFETY: same per-CPU pointer as above; the child has yielded, so
        // this CPU is back in the parent's syscall and is the only writer.
        unsafe {
            (*per_cpu).user_rsp = saved_user_rsp;
            (*per_cpu).syscall_frame = saved_syscall_frame;
        }
        crate::arch::percpu::set_entry_stack(saved_kernel_rsp);

        // Restore parent as boot-current
        crate::process::set_boot_current(parent_pid, parent_tid);

        // Restore boot return context
        BOOT_RETURN_RSP.store(saved_rsp, Ordering::SeqCst);
        BOOT_RETURN_CR3.store(saved_cr3, Ordering::SeqCst);
        BOOT_STACK_CANARY.store(saved_canary, Ordering::SeqCst);

        // Clear yield flag
        BOOT_CLONE_YIELD_PENDING.store(false, Ordering::Release);

        // Re-enqueue the child task if it didn't exit.
        // The child's ThreadContext was updated by the yield path in
        // syscall_handler, so the next dispatch will resume the child
        // at the correct instruction. If the child called sys_exit,
        // the process/thread is Zombie -- don't re-enqueue.
        let child_exited = crate::process::get_process(child_pid)
            .map(|p| {
                p.get_state() == crate::process::pcb::ProcessState::Zombie
                    || p.get_thread(child_tid)
                        .map(|t| t.get_state() == crate::process::thread::ThreadState::Zombie)
                        .unwrap_or(true)
            })
            .unwrap_or(true);

        if !child_exited {
            let sched = crate::sched::scheduler::current_scheduler();
            let slock = sched.lock();
            slock.enqueue(child_task);
        }
    }

    // Exhausted spins without the futex word changing
    Err(SyscallError::WouldBlock)
}

/// How many ready tasks boot dispatch examines per spin when looking for one
/// of the parent's threads.
#[cfg(target_arch = "x86_64")]
const BOOT_DISPATCH_SCAN: usize = 64;

/// Whether boot dispatch may run the task (`pid`, `tid`) while the boot-path
/// parent (`parent_pid`, `parent_tid`) waits: only the parent's own sibling
/// threads, never the parent thread itself.
#[cfg(any(target_arch = "x86_64", test))]
fn is_boot_dispatchable(
    pid: crate::process::ProcessId,
    tid: crate::process::thread::ThreadId,
    parent_pid: crate::process::ProcessId,
    parent_tid: crate::process::thread::ThreadId,
) -> bool {
    pid == parent_pid && tid != parent_tid
}

/// Stub for non-x86_64 architectures.
#[cfg(not(target_arch = "x86_64"))]
fn boot_futex_spin(
    _uaddr: usize,
    _expected: u32,
    _deadline: Option<u64>,
) -> Result<isize, SyscallError> {
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

    #[test]
    fn boot_dispatch_takes_only_the_parents_sibling_threads() {
        use crate::process::{thread::ThreadId, ProcessId};
        let (ppid, ptid) = (ProcessId(5), ThreadId(5));
        // A sibling thread of the parent qualifies.
        assert!(is_boot_dispatchable(ProcessId(5), ThreadId(6), ppid, ptid));
        // The parent thread itself does not.
        assert!(!is_boot_dispatchable(ProcessId(5), ThreadId(5), ppid, ptid));
        // Another process's thread, or a kernel task, does not.
        assert!(!is_boot_dispatchable(ProcessId(7), ThreadId(8), ppid, ptid));
        assert!(!is_boot_dispatchable(ProcessId(0), ThreadId(1), ppid, ptid));
    }

    /// Linux's FUTEX_OP() macro.
    const fn futex_op(op: u32, oparg: u32, cmp: u32, cmparg: u32) -> u32 {
        ((op & 0xF) << 28) | ((cmp & 0xF) << 24) | ((oparg & 0xFFF) << 12) | (cmparg & 0xFFF)
    }

    /// SYS-CONC-01: the fields used to be read from the wrong bits (op and
    /// cmp swapped, oparg and cmparg swapped).
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

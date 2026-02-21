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

// Bit positions for FUTEX_WAKE_OP operation encoding (Linux-compatible)
// op = oparg | (cmp << 28) | (op << 24)
const FUTEX_OP_MASK: u32 = 0xF << 24;
const FUTEX_CMP_MASK: u32 = 0xF << 28;
const FUTEX_OPARG_MASK: u32 = 0xFFFF;

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
// FUTEX_TABLE lock is held or by the scheduler after the waiter has been
// dequeued.  Send/Sync are required so the BTreeMap can live in a static
// Mutex, which is safe because all accesses are serialised by the spinlock.
unsafe impl Send for FutexWaiter {}
unsafe impl Sync for FutexWaiter {}

static FUTEX_TABLE: Mutex<BTreeMap<FutexKey, Vec<FutexWaiter>>> = Mutex::new(BTreeMap::new());

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

    // SAFETY: `uaddr` has been validated as a properly-aligned, mapped,
    // user-space pointer to a u32.  We use `read_volatile` because another
    // thread sharing this address space may concurrently modify the futex
    // word; a non-volatile read could be elided or reordered by the compiler.
    let cur = unsafe { core::ptr::read_volatile(uaddr as *const u32) };
    if cur != expected {
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
        // SAFETY: `timeout_ptr` has been validated as a properly-aligned,
        // mapped, user-space pointer to a u64.  Volatile read is used because
        // the caller could theoretically share this memory with another thread.
        let rel = unsafe { core::ptr::read_volatile(timeout_ptr as *const u64) };
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

    let pid = process::current_process()
        .ok_or(SyscallError::InvalidState)?
        .pid
        .0;
    let key = (pid, uaddr);

    let task_ptr = {
        let sched = crate::sched::scheduler::current_scheduler();
        let slock = sched.lock();
        let task = slock.current().ok_or(SyscallError::InvalidState)?;
        // SAFETY: We hold the scheduler lock, which guarantees exclusive
        // access to the current task's state field.  The pointer is valid
        // because it was obtained from the scheduler's active task list.
        unsafe {
            (*task.as_ptr()).state = process::ProcessState::Blocked;
        }
        task
    };

    {
        // SAFETY: We hold the scheduler lock above (now dropped), and the
        // task pointer remains valid because the task is Blocked and cannot
        // be freed while it is on a wait queue.  Reading priority is safe
        // because we are the only thread that can modify our own task while
        // it is in the Blocked state.
        let prio = unsafe { (*task_ptr.as_ptr()).priority as u8 };
        let mut table = FUTEX_TABLE.lock();
        table.entry(key).or_default().push(FutexWaiter {
            task: task_ptr,
            priority: prio,
            bitset: bitset_mask,
        });
    }

    // reschedule
    sched::SCHEDULER.lock().schedule();

    // Helper to remove this waiter from the queue (used on timeout/interruption)
    let remove_self = |reason: SyscallError| -> Result<isize, SyscallError> {
        let mut table = FUTEX_TABLE.lock();
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
        let mut table = FUTEX_TABLE.lock();
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
        FUTEX_WAKE_OP => sys_futex_wake_op(uaddr, val, uaddr2, val3, op),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// Perform a `FUTEX_WAKE_OP` operation: atomically apply an arithmetic
/// operation to `*uaddr2`, compare the old value against `cmparg`, and
/// conditionally wake waiters on `uaddr`.
///
/// The `op` parameter encodes both the arithmetic operation and the
/// comparison in a packed format compatible with Linux:
/// ```text
/// op = oparg[15:0] | (op_code[3:0] << 24) | (cmp_code[3:0] << 28)
/// ```
///
/// # Arguments
///
/// * `uaddr`  - Primary futex word to wake on (if comparison passes).
/// * `wake`   - Maximum number of waiters to wake on `uaddr`.
/// * `uaddr2` - Futex word to modify atomically.
/// * `op`     - Packed operation + comparison + argument.
/// * `_unused` - Reserved (unused).
///
/// # Returns
///
/// `Ok(n)` where `n` is the number of waiters woken, or `Ok(0)` if the
/// comparison failed.
pub fn sys_futex_wake_op(
    uaddr: usize,
    wake: usize,
    uaddr2: usize,
    op: usize,
    _unused: usize,
) -> Result<isize, SyscallError> {
    // For safety, require alignment and same-process addresses.
    if uaddr == 0 || uaddr & 0x3 != 0 || uaddr2 == 0 || uaddr2 & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    validate_user_ptr(uaddr2 as *const u32, core::mem::size_of::<u32>())?;

    // Decode op
    let op_code = ((op as u32) & FUTEX_OP_MASK) >> 24;
    let cmp_code = ((op as u32) & FUTEX_CMP_MASK) >> 28;
    let oparg = (op as u32) & FUTEX_OPARG_MASK;
    let cmparg = ((op >> 12) & FUTEX_OPARG_MASK as usize) as u32;

    // SAFETY: `uaddr2` has been validated as a properly-aligned, mapped,
    // user-space pointer.  Volatile read is required because another thread
    // may concurrently modify this memory location.
    let cur = unsafe { core::ptr::read_volatile(uaddr2 as *const u32) };
    let new_val = match op_code {
        FUTEX_OP_SET => oparg,
        FUTEX_OP_ADD => cur.wrapping_add(oparg),
        FUTEX_OP_OR => cur | oparg,
        FUTEX_OP_ANDN => cur & !oparg,
        FUTEX_OP_XOR => cur ^ oparg,
        _ => return Err(SyscallError::InvalidArgument),
    };
    // SAFETY: Same validation as above.  Volatile write is required because
    // other threads may be reading this memory concurrently (e.g. in a
    // FUTEX_WAIT spin).
    unsafe {
        core::ptr::write_volatile(uaddr2 as *mut u32, new_val);
    }

    // Compare
    let cmp_ok = match cmp_code {
        FUTEX_CMP_EQ => cur == cmparg,
        FUTEX_CMP_NE => cur != cmparg,
        FUTEX_CMP_LT => cur < cmparg,
        FUTEX_CMP_LE => cur <= cmparg,
        FUTEX_CMP_GT => cur > cmparg,
        FUTEX_CMP_GE => cur >= cmparg,
        _ => false,
    };

    // If compare passes, wake (use match-any bitset for WAKE_OP)
    if cmp_ok {
        sys_futex_wake(uaddr, wake, FUTEX_WAIT_BITSET_MATCH_ANY as usize)
    } else {
        Ok(0)
    }
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
        let mut table = FUTEX_TABLE.lock();
        if let Some(waiters) = table.get_mut(&key1) {
            waiters.sort_by(|a, b| b.priority.cmp(&a.priority));
            let wc = core::cmp::min(wake_count, waiters.len());
            for _ in 0..wc {
                let w = waiters.remove(0);
                to_wake.push(w.task);
                woken += 1;
            }
            let rc = core::cmp::min(requeue_count, waiters.len());
            for _ in 0..rc {
                let w = waiters.remove(0);
                to_move.push(w);
                moved += 1;
            }
            if waiters.is_empty() {
                table.remove(&key1);
            }
        }

        if moved > 0 {
            table.entry(key2).or_default().extend(to_move);
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

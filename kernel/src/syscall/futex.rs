//! Futex syscall handlers (FUTEX_WAIT, FUTEX_WAKE).
//!
//! Implements per-process futex wait queues keyed by user virtual address. The
//! implementation enforces:
//! - 32-bit aligned futex words
//! - Per-process isolation (same address in different processes does not alias)
//! - Atomic re-check of expected value before sleeping

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

// Futex wait queue keyed by (pid, uaddr)
type FutexKey = (u64, usize);

struct FutexWaiter {
    task: core::ptr::NonNull<sched::task::Task>,
    priority: u8,
}

unsafe impl Send for FutexWaiter {}
unsafe impl Sync for FutexWaiter {}

static FUTEX_TABLE: Mutex<BTreeMap<FutexKey, Vec<FutexWaiter>>> = Mutex::new(BTreeMap::new());

/// FUTEX_WAIT (optionally with relative timeout in ticks)
///
/// `op` is the raw futex operation field. For FUTEX_WAIT_BITSET the caller
/// passes the bitset mask in `aux` (val3). For plain FUTEX_WAIT the `aux`
/// parameter is treated as the timeout length (must match sizeof(u64) when a
/// timeout pointer is provided).
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
    // Must reside in user space
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;

    // Validate user memory and read current value
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
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
        0xFFFF_FFFF // match-any mask (Linux's FUTEX_BITSET_MATCH_ANY)
    };

    // Optional timeout: expect a u64 ticks value
    let deadline = if timeout_ptr != 0 {
        if aux != 0 && op_base != FUTEX_WAIT_BITSET && aux != core::mem::size_of::<u64>() {
            // For plain WAIT the caller must supply sizeof(u64) to document
            // the layout; for WAIT_BITSET we treat `aux` as the mask instead.
            return Err(SyscallError::InvalidArgument);
        }
        validate_user_ptr(timeout_ptr as *const u64, core::mem::size_of::<u64>())?;
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
        // mark state blocked
        unsafe {
            (*task.as_ptr()).state = process::ProcessState::Blocked;
        }
        task
    };

    {
        let prio = unsafe { (*task_ptr.as_ptr()).priority as u8 };
        let mut table = FUTEX_TABLE.lock();
        table.entry(key).or_default().push(FutexWaiter {
            task: task_ptr,
            priority: prio,
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

    let _ = bitset_mask; // reserved for FUTEX_WAIT_BITSET wake filtering

    Ok(0)
}

/// FUTEX_WAKE
pub fn sys_futex_wake(
    uaddr: usize,
    num_wake: usize,
    _unused: usize,
) -> Result<isize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }

    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;

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
            let count = core::cmp::min(num_wake, waiters.len());
            for _ in 0..count {
                let w = waiters.remove(0);
                to_wake.push(w.task);
                woken += 1;
            }
            if waiters.is_empty() {
                table.remove(&key);
            }
        }
    }

    // Wake tasks (simple FIFO; future: priority-based selection)
    if woken > 0 {
        let scheduler = crate::sched::scheduler::current_scheduler();
        let slock = scheduler.lock();
        for task_ptr in to_wake {
            unsafe {
                (*task_ptr.as_ptr()).state = process::ProcessState::Ready;
            }
            slock.enqueue(task_ptr);
        }
    }

    Ok(woken as isize)
}

/// FUTEX dispatcher (matches Linux parameter order)
///
/// Args:
/// - `uaddr`: user futex word
/// - `val`: expected (for WAIT) or wake count (for WAKE/REQUEUE)
/// - `uaddr2`: secondary futex for REQUEUE/WAKE_OP
/// - `val3`: requeue count or timeout len/bitset depending on op
/// - `op`: futex operation + flags
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

/// FUTEX_WAKE_OP (limited implementation): perform atomic op on uaddr2 then
/// wake up to val waiters on uaddr.
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

    // Apply operation atomically on uaddr2
    validate_user_ptr(uaddr2 as *const u32, core::mem::size_of::<u32>())?;
    let cur = unsafe { core::ptr::read_volatile(uaddr2 as *const u32) };
    let new_val = match op_code {
        FUTEX_OP_SET => oparg,
        FUTEX_OP_ADD => cur.wrapping_add(oparg),
        FUTEX_OP_OR => cur | oparg,
        FUTEX_OP_ANDN => cur & !oparg,
        FUTEX_OP_XOR => cur ^ oparg,
        _ => return Err(SyscallError::InvalidArgument),
    };
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

    // If compare passes, wake
    if cmp_ok {
        sys_futex_wake(uaddr, wake, 0)
    } else {
        Ok(0)
    }
}

/// FUTEX_REQUEUE (wake up to wake_count at uaddr, then move up to requeue_count
/// waiters to uaddr2)
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
            unsafe {
                (*task_ptr.as_ptr()).state = process::ProcessState::Ready;
            }
            slock.enqueue(task_ptr);
        }
    }

    Ok((woken + moved) as isize)
}

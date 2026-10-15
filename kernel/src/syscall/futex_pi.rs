//! Priority-inheritance futexes: FUTEX_LOCK_PI, FUTEX_LOCK_PI2,
//! FUTEX_TRYLOCK_PI and FUTEX_UNLOCK_PI, with Linux's word protocol
//! (SCHED-INC-01).
//!
//! The futex word holds the owner's thread id (`FUTEX_TID_MASK`), plus
//! `FUTEX_WAITERS` while the kernel has waiters and `FUTEX_OWNER_DIED` after
//! an owner exited holding it. User space takes and releases an uncontended
//! lock with a compare-and-swap of 0 <-> its tid; only contention reaches
//! the kernel.
//!
//! The kernel keeps, per contended lock, its holder and its waiters. While a
//! lock has waiters its holder runs boosted to the highest waiting priority
//! (`Entity::pi_priority`: real-time priority or boost, 99 for a deadline
//! task, 0 for a fair one), passed along chains of locks: if the holder
//! itself waits for a PI lock, that lock's holder is boosted too. Unlock
//! hands the lock to the highest-priority waiter (first come among equals),
//! writing its tid into the word, so the lock never becomes free while
//! someone waits for it.
//!
//! Not implemented: FUTEX_WAIT_REQUEUE_PI and FUTEX_CMP_REQUEUE_PI (glibc's
//! PI condition variables; musl does not use them), and deadline-task
//! inheritance of deadline parameters (a deadline waiter boosts to RT 99).

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use super::{
    futex::{futex_key, FutexKey},
    userspace::{cmpxchg_user_u32, read_user, validate_user_ptr},
    SyscallError,
};
use crate::{
    process::robust_list::{FUTEX_OWNER_DIED, FUTEX_TID_MASK, FUTEX_WAITERS},
    sched::dispatch::{self, WaitError, WaitQueue},
};

/// A (pid, tid) user thread.
type Owner = (u64, u64);

/// How far a boost travels along a chain of locks (also bounds a cycle,
/// which is a deadlock user space created).
const MAX_CHAIN: usize = 16;

struct Waiter {
    owner: Owner,
    /// The priority it lends (its `pi_priority` when it last changed).
    prio: u8,
    /// Arrival order: first come among equal priorities.
    seq: u64,
}

struct PiState {
    holder: Owner,
    waiters: Vec<Waiter>,
}

/// Contended PI locks by futex key.
static PI: Mutex<BTreeMap<FutexKey, PiState>> = Mutex::new(BTreeMap::new());
/// Waiters that were handed their lock (by `seq`), until they see it.
static GRANTED: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());
/// Waiters to start over (by `seq`): the holder of their shared-memory lock
/// exited, and its word can only be fixed from their own address space.
static RETRY: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());
/// PI waiters sleep here; every handoff wakes them all to check.
static PI_WAIT: WaitQueue = WaitQueue::new();
static SEQ: AtomicU64 = AtomicU64::new(1);

/// The boost `holder` needs: the highest priority among the waiters of the
/// locks it holds (`None` when that is 0: fair waiters lend nothing).
fn boost_for(table: &BTreeMap<FutexKey, PiState>, holder: Owner) -> Option<u8> {
    let b = table
        .values()
        .filter(|s| s.holder == holder)
        .flat_map(|s| s.waiters.iter().map(|w| w.prio))
        .max()
        .unwrap_or(0);
    (b > 0).then_some(b)
}

/// Recompute `holder`'s boost and pass a change along the chain of locks it
/// waits for.
fn propagate(table: &mut BTreeMap<FutexKey, PiState>, mut holder: Owner) {
    for _ in 0..MAX_CHAIN {
        dispatch::set_boost(holder, boost_for(table, holder));
        let lends = dispatch::pi_priority(holder).unwrap_or(0);
        let Some(state) = table
            .values_mut()
            .find(|s| s.waiters.iter().any(|w| w.owner == holder))
        else {
            return;
        };
        let w = state
            .waiters
            .iter_mut()
            .find(|w| w.owner == holder)
            .expect("found above");
        if w.prio == lends {
            return;
        }
        w.prio = lends;
        holder = state.holder;
    }
}

/// The highest-priority waiter (first come among equals), removed.
fn take_top(state: &mut PiState) -> Option<Waiter> {
    let i = state
        .waiters
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.prio.cmp(&b.prio).then(b.seq.cmp(&a.seq)))
        .map(|(i, _)| i)?;
    Some(state.waiters.remove(i))
}

/// The user thread whose thread id is `tid`, if it has a task.
fn owner_of_tid(tid: u32) -> Option<Owner> {
    dispatch::owners()
        .into_iter()
        .find(|&(_, t)| t == tid as u64)
}

/// Give the lock at `uaddr` (key `key`) to the top waiter of `state`, with
/// `extra` (`FUTEX_OWNER_DIED` or 0) set. Returns false if it had none.
fn hand_off(
    table: &mut BTreeMap<FutexKey, PiState>,
    key: FutexKey,
    uaddr: usize,
    extra: u32,
) -> Result<bool, SyscallError> {
    let Some(state) = table.get_mut(&key) else {
        return Ok(false);
    };
    let old_holder = state.holder;
    let Some(next) = take_top(state) else {
        return Ok(false);
    };
    let more = if state.waiters.is_empty() {
        0
    } else {
        FUTEX_WAITERS
    };
    let word = (next.owner.1 as u32 & FUTEX_TID_MASK) | more | extra;
    // The word is ours (or the dead owner's); only user space racing
    // against its own lock could change it, and the kernel's word wins.
    let mut val = read_user::<u32>(uaddr)?;
    loop {
        let seen = cmpxchg_user_u32(uaddr, val, word)?;
        if seen == val {
            break;
        }
        val = seen;
    }
    state.holder = next.owner;
    if state.waiters.is_empty() {
        table.remove(&key);
    }
    GRANTED.lock().insert(next.seq);
    propagate(table, old_holder);
    propagate(table, next.owner);
    PI_WAIT.wake_all();
    Ok(true)
}

/// FUTEX_LOCK_PI / FUTEX_LOCK_PI2 (`deadline`: absolute `clock_ns`, from
/// the caller's timeout) and FUTEX_TRYLOCK_PI (`try_only`).
pub(crate) fn lock_pi(
    uaddr: usize,
    deadline: Option<u64>,
    try_only: bool,
) -> Result<usize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    let me = dispatch::current_owner().ok_or(SyscallError::InvalidState)?;
    let my_tid = me.1 as u32 & FUTEX_TID_MASK;
    let key = futex_key(uaddr)?;

    loop {
        let seq = {
            let mut table = PI.lock();
            loop {
                let val = read_user::<u32>(uaddr)?;
                let tid = val & FUTEX_TID_MASK;
                if tid == 0 {
                    // Free (also after its owner died): take it, keeping the
                    // waiters bit if others queued meanwhile.
                    let queued = table.get(&key).is_some_and(|s| !s.waiters.is_empty());
                    let new = my_tid | if queued { FUTEX_WAITERS } else { 0 };
                    if cmpxchg_user_u32(uaddr, val, new)? != val {
                        continue;
                    }
                    if let Some(state) = table.get_mut(&key) {
                        state.holder = me;
                        propagate(&mut table, me);
                    }
                    return Ok(0);
                }
                if tid == my_tid {
                    return Err(SyscallError::Deadlock);
                }
                if try_only {
                    return Err(SyscallError::WouldBlock);
                }
                if val & FUTEX_WAITERS == 0
                    && cmpxchg_user_u32(uaddr, val, val | FUTEX_WAITERS)? != val
                {
                    continue;
                }
                // The holder must be a live thread (Linux: ESRCH).
                let holder = owner_of_tid(tid).ok_or(SyscallError::ProcessNotFound)?;
                let state = table.entry(key).or_insert(PiState {
                    holder,
                    waiters: Vec::new(),
                });
                state.holder = holder;
                let seq = SEQ.fetch_add(1, Ordering::Relaxed);
                state.waiters.push(Waiter {
                    owner: me,
                    prio: dispatch::pi_priority(me).unwrap_or(0),
                    seq,
                });
                propagate(&mut table, holder);
                break seq;
            }
        };

        let result = dispatch::wait_event(&PI_WAIT, deadline, || {
            GRANTED.lock().contains(&seq) || RETRY.lock().contains(&seq)
        });
        let mut table = PI.lock();
        if GRANTED.lock().remove(&seq) {
            // Handed the lock, whatever ended the wait.
            return Ok(0);
        }
        if RETRY.lock().remove(&seq) {
            // Its holder exited: look at the word again (taken over, or the
            // dead owner's tid: ESRCH).
            drop(table);
            continue;
        }
        // Gave up: leave the queue and drop what we lent.
        if let Some(state) = table.get_mut(&key) {
            state.waiters.retain(|w| w.seq != seq);
            let holder = state.holder;
            if state.waiters.is_empty() {
                table.remove(&key);
            }
            propagate(&mut table, holder);
        }
        return match result {
            Err(WaitError::TimedOut) => Err(SyscallError::TimedOut),
            _ => Err(SyscallError::Interrupted),
        };
    }
}

/// FUTEX_UNLOCK_PI: the caller must own the lock (EPERM otherwise). With
/// waiters it goes to the top one; otherwise the word becomes 0.
pub(crate) fn unlock_pi(uaddr: usize) -> Result<usize, SyscallError> {
    if uaddr == 0 || uaddr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(uaddr as *const u32, core::mem::size_of::<u32>())?;
    let me = dispatch::current_owner().ok_or(SyscallError::InvalidState)?;
    let my_tid = me.1 as u32 & FUTEX_TID_MASK;
    let key = futex_key(uaddr)?;

    let mut table = PI.lock();
    let val = read_user::<u32>(uaddr)?;
    if val & FUTEX_TID_MASK != my_tid {
        return Err(SyscallError::OperationNotPermitted);
    }
    if hand_off(&mut table, key, uaddr, 0)? {
        return Ok(0);
    }
    // No kernel waiters: free it (clearing a stale waiters bit).
    let mut cur = val;
    loop {
        let seen = cmpxchg_user_u32(uaddr, cur, 0)?;
        if seen == cur {
            break;
        }
        if seen & FUTEX_TID_MASK != my_tid {
            return Err(SyscallError::OperationNotPermitted);
        }
        cur = seen;
    }
    table.remove(&key);
    propagate(&mut table, me);
    Ok(0)
}

/// Thread `owner` is exiting: every PI lock it holds with waiters passes to
/// its top waiter with `FUTEX_OWNER_DIED` set (as Linux's
/// exit_pi_state_list), and any lock it waited for forgets it. Called on the
/// exiting thread, after the robust list was processed.
pub(crate) fn owner_exiting(owner: Owner) {
    let mut table = PI.lock();
    // Waits of its own (an exit_group kill can end a thread mid-wait).
    let mut changed = Vec::new();
    for state in table.values_mut() {
        let before = state.waiters.len();
        state.waiters.retain(|w| w.owner != owner);
        if state.waiters.len() != before {
            changed.push(state.holder);
        }
    }
    table.retain(|_, s| !s.waiters.is_empty());
    for holder in changed {
        propagate(&mut table, holder);
    }
    // Locks it holds. The word's address in this process is not kept, so
    // the handoff needs it: only private keys carry the address.
    let held: Vec<FutexKey> = table
        .iter()
        .filter(|(_, s)| s.holder == owner)
        .map(|(k, _)| *k)
        .collect();
    for key in held {
        let (keyspace, addr) = key;
        if keyspace == owner.0 {
            let _ = hand_off(&mut table, key, addr, FUTEX_OWNER_DIED);
        } else {
            // A shared-memory lock: its key is a physical address, and the
            // word cannot be reached from here. Its waiters start over: a
            // robust lock now shows FUTEX_OWNER_DIED with no owner and is
            // taken; otherwise the dead owner's tid gives ESRCH.
            if let Some(state) = table.remove(&key) {
                let mut retry = RETRY.lock();
                for w in state.waiters {
                    retry.insert(w.seq);
                }
            }
        }
    }
    PI_WAIT.wake_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(holder: Owner, waiters: &[(Owner, u8, u64)]) -> PiState {
        PiState {
            holder,
            waiters: waiters
                .iter()
                .map(|&(owner, prio, seq)| Waiter { owner, prio, seq })
                .collect(),
        }
    }

    #[test]
    fn top_waiter_is_highest_priority_then_first_come() {
        let mut s = state((1, 1), &[((1, 2), 10, 5), ((1, 3), 40, 7), ((1, 4), 40, 6)]);
        assert_eq!(take_top(&mut s).map(|w| w.owner), Some((1, 4)));
        assert_eq!(take_top(&mut s).map(|w| w.owner), Some((1, 3)));
        assert_eq!(take_top(&mut s).map(|w| w.owner), Some((1, 2)));
        assert!(take_top(&mut s).is_none());
    }

    #[test]
    fn boost_is_the_highest_waiter_over_every_held_lock() {
        let mut t = BTreeMap::new();
        t.insert((1, 0x1000), state((1, 1), &[((1, 2), 10, 1)]));
        t.insert((1, 0x2000), state((1, 1), &[((1, 3), 50, 2)]));
        t.insert((1, 0x3000), state((1, 9), &[((1, 4), 90, 3)]));
        assert_eq!(boost_for(&t, (1, 1)), Some(50));
        assert_eq!(boost_for(&t, (1, 9)), Some(90));
        // Fair waiters lend nothing.
        t.insert((1, 0x4000), state((1, 7), &[((1, 5), 0, 4)]));
        assert_eq!(boost_for(&t, (1, 7)), None);
        assert_eq!(boost_for(&t, (1, 8)), None);
    }
}

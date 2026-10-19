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

/// Compare-and-swap attempts on a futex word user space keeps changing
/// before giving up with EAGAIN, as Linux does.
const MAX_CAS: usize = 16;

/// The keyspace of futexes in memory shared between processes
/// (`futex::futex_key`).
const SHARED: u64 = u64::MAX;

/// Fault the futex word in, readable and writable (a same-value
/// compare-and-swap takes a copy-on-write copy), before the PI lock is
/// taken: a fault with the lock held could sleep (a page-cache read) while
/// other CPUs spin on it.
fn prefault(uaddr: usize) -> Result<(), SyscallError> {
    let v = read_user::<u32>(uaddr)?;
    cmpxchg_user_u32(uaddr, v, v)?;
    Ok(())
}

/// Whether `me` may make `holder` the holder of lock `key`, and so lend it
/// priority. A private futex's holder must be a thread of the caller's own
/// process; a shared one's a thread whose scheduling the caller could
/// change anyway (sched_setscheduler's rule). Otherwise writing another
/// process's thread id into one's own futex word boosted that thread.
fn may_hold(me: Owner, holder: Owner, key: FutexKey) -> bool {
    if key.0 == SHARED {
        super::scheduling::may_change_pid(holder.0)
    } else {
        holder.0 == me.0
    }
}

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
/// The word that takes a free PI lock (`val`, tid 0) for `tid`: the
/// waiters bit if others are queued, and FUTEX_OWNER_DIED kept -- the next
/// owner of a robust mutex reads EOWNERDEAD from it (Linux's
/// futex_lock_pi_atomic keeps it; PR #23 review).
fn free_lock_word(val: u32, tid: u32, queued: bool) -> u32 {
    tid | (val & FUTEX_OWNER_DIED) | if queued { FUTEX_WAITERS } else { 0 }
}

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
    // If the word cannot be reached (another thread unmapped it), the
    // waiter goes back on the queue: dropped, it slept forever (PR #23
    // review).
    let mut val = match read_user::<u32>(uaddr) {
        Ok(v) => v,
        Err(e) => {
            state.waiters.push(next);
            return Err(e);
        }
    };
    let mut tries = 0;
    loop {
        let seen = match cmpxchg_user_u32(uaddr, val, word) {
            Ok(seen) => seen,
            Err(e) => {
                state.waiters.push(next);
                return Err(e);
            }
        };
        if seen == val {
            break;
        }
        tries += 1;
        if tries >= MAX_CAS {
            // The waiter goes back on the queue; the lock stays as it is.
            state.waiters.push(next);
            return Err(SyscallError::WouldBlock);
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
        prefault(uaddr)?;
        let seq = {
            let mut table = PI.lock();
            let mut tries = 0;
            loop {
                tries += 1;
                if tries > MAX_CAS {
                    return Err(SyscallError::WouldBlock);
                }
                let val = read_user::<u32>(uaddr)?;
                let tid = val & FUTEX_TID_MASK;
                if tid == 0 {
                    // Free (also after its owner died): take it, keeping the
                    // waiters bit if others queued meanwhile.
                    let queued = table.get(&key).is_some_and(|s| !s.waiters.is_empty());
                    let new = free_lock_word(val, my_tid, queued);
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
                // The holder must be a live thread (Linux: ESRCH) the caller
                // may lend priority to.
                let holder = owner_of_tid(tid).ok_or(SyscallError::ProcessNotFound)?;
                if !may_hold(me, holder, key) {
                    return Err(SyscallError::OperationNotPermitted);
                }
                let state = table.entry(key).or_insert(PiState {
                    holder,
                    waiters: Vec::new(),
                });
                let stale = core::mem::replace(&mut state.holder, holder);
                let seq = SEQ.fetch_add(1, Ordering::Relaxed);
                state.waiters.push(Waiter {
                    owner: me,
                    prio: dispatch::pi_priority(me).unwrap_or(0),
                    seq,
                });
                if stale != holder {
                    // The word named someone else (user space took or
                    // passed the lock): the old holder's boost is redone.
                    propagate(&mut table, stale);
                }
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

    prefault(uaddr)?;
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
    let mut tries = 0;
    loop {
        let seen = cmpxchg_user_u32(uaddr, cur, 0)?;
        if seen == cur {
            break;
        }
        if seen & FUTEX_TID_MASK != my_tid {
            return Err(SyscallError::OperationNotPermitted);
        }
        tries += 1;
        if tries >= MAX_CAS {
            return Err(SyscallError::WouldBlock);
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
        // A private lock is handed to its top waiter. If its word cannot
        // be written (unmapped meanwhile), the waiters start over as for a
        // shared lock below: nobody else would ever hand it on.
        if keyspace != owner.0 || hand_off(&mut table, key, addr, FUTEX_OWNER_DIED).is_err() {
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

    /// A hand-off that cannot reach the futex word (unmapped meanwhile by
    /// another thread) keeps the waiter queued: dropped, it slept forever
    /// (PR #23 review).
    #[test]
    fn a_failed_hand_off_keeps_the_waiter() {
        let mut t = BTreeMap::new();
        t.insert(
            (1, 0x1000),
            state((1, 1), &[((1, 2), 10, 1), ((1, 3), 20, 2)]),
        );
        assert!(hand_off(&mut t, (1, 0x1000), 0, 0).is_err());
        assert_eq!(t.get(&(1, 0x1000)).map(|s| s.waiters.len()), Some(2));
    }

    /// Taking a free lock whose owner died keeps FUTEX_OWNER_DIED, which
    /// is how the next owner of a robust mutex learns EOWNERDEAD; it was
    /// cleared (PR #23 review).
    #[test]
    fn a_free_lock_word_keeps_owner_died() {
        assert_eq!(free_lock_word(0, 7, false), 7);
        assert_eq!(
            free_lock_word(FUTEX_OWNER_DIED, 7, false),
            7 | FUTEX_OWNER_DIED
        );
        assert_eq!(
            free_lock_word(FUTEX_OWNER_DIED | FUTEX_WAITERS, 7, true),
            7 | FUTEX_OWNER_DIED | FUTEX_WAITERS
        );
        assert_eq!(free_lock_word(FUTEX_WAITERS, 7, false), 7);
    }

    #[test]
    fn a_private_lock_holder_must_be_in_the_callers_process() {
        // Writing another process's thread id into one's own futex word
        // must not lend that thread priority.
        assert!(may_hold((5, 50), (5, 51), (5, 0x1000)));
        assert!(!may_hold((5, 50), (9, 90), (5, 0x1000)));
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

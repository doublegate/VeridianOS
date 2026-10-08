//! Robust futex lists (N-225).
//!
//! A thread registers its list with `set_robust_list`: a
//! `struct robust_list_head { list.next, futex_offset, list_op_pending }` in
//! its own memory, through which the C library links every robust mutex the
//! thread holds. When the thread ends -- `pthread_exit`, `exit_group`, a
//! fatal signal -- the kernel walks the list as Linux's `exit_robust_list`
//! does: each futex word still owned by the thread (its low 30 bits are the
//! thread's ID) gets FUTEX_OWNER_DIED, keeping FUTEX_WAITERS, and one
//! waiter is woken, so the next locker sees EOWNERDEAD instead of waiting
//! forever. The list is the thread's (a new thread and a fork child start
//! without one; exec clears it).

/// `sizeof(struct robust_list_head)` on 64-bit targets.
pub const ROBUST_LIST_HEAD_SIZE: usize = 24;

/// The futex word has waiters.
pub const FUTEX_WAITERS: u32 = 0x8000_0000;
/// The futex's owner died holding it.
pub const FUTEX_OWNER_DIED: u32 = 0x4000_0000;
/// The owner's thread ID in the futex word.
pub const FUTEX_TID_MASK: u32 = 0x3fff_ffff;

/// Entries walked at most, so a corrupt or circular list cannot keep an
/// exiting thread in the kernel (Linux's ROBUST_LIST_LIMIT).
pub const ROBUST_LIST_LIMIT: usize = 2048;

/// The exiting thread's memory, as the walk uses it. Every access may fail
/// (a bad pointer ends the walk, as in Linux).
pub trait RobustMemory {
    fn read_u64(&mut self, addr: usize) -> Option<u64>;
    fn read_u32(&mut self, addr: usize) -> Option<u32>;
    /// Compare-and-exchange; the value found.
    fn cmpxchg_u32(&mut self, addr: usize, old: u32, new: u32) -> Option<u32>;
    /// Wake one waiter on the futex at `addr`.
    fn wake_one(&mut self, addr: usize);
}

/// Which list position a futex was found at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Death {
    /// Linked in the list.
    Listed,
    /// `list_op_pending`: being locked or unlocked when the thread died.
    Pending,
}

/// Walk the robust list at `head` of the thread `tid`, which is exiting.
pub fn exit_robust_list(mem: &mut impl RobustMemory, head: usize, tid: u32) {
    // An entry's low bit marks a priority-inheritance futex.
    let fetch = |mem: &mut dyn RobustMemory, at: usize| {
        mem.read_u64(at).map(|v| ((v & !1) as usize, v & 1 != 0))
    };
    let Some((mut entry, mut pi)) = fetch(mem, head) else {
        return;
    };
    let Some(offset) = mem.read_u64(head.wrapping_add(8)).map(|v| v as i64) else {
        return;
    };
    let Some((pending, pending_pi)) = fetch(mem, head.wrapping_add(16)) else {
        return;
    };
    let futex_of = |entry: usize| entry.wrapping_add_signed(offset as isize);

    let mut limit = ROBUST_LIST_LIMIT;
    while entry != head {
        // The next entry is read before this futex can be released (and its
        // memory freed by a woken thread).
        let next = fetch(mem, entry);
        if entry != pending {
            handle_futex_death(mem, futex_of(entry), tid, pi, Death::Listed);
        }
        let Some((n, npi)) = next else {
            return;
        };
        entry = n;
        pi = npi;
        limit -= 1;
        if limit == 0 {
            break;
        }
    }
    if pending != 0 {
        handle_futex_death(mem, futex_of(pending), tid, pending_pi, Death::Pending);
    }
}

/// Linux's `handle_futex_death`: mark a futex the dead thread owns.
fn handle_futex_death(mem: &mut impl RobustMemory, uaddr: usize, tid: u32, pi: bool, death: Death) {
    if !uaddr.is_multiple_of(4) {
        return;
    }
    let Some(mut uval) = mem.read_u32(uaddr) else {
        return;
    };
    loop {
        // A pending unlock that already stored 0 may have left a waiter
        // unwoken: wake one, as Linux does for non-PI futexes.
        if death == Death::Pending && !pi && uval == 0 {
            mem.wake_one(uaddr);
            return;
        }
        if uval & FUTEX_TID_MASK != tid {
            return;
        }
        let new = (uval & FUTEX_WAITERS) | FUTEX_OWNER_DIED;
        let Some(found) = mem.cmpxchg_u32(uaddr, uval, new) else {
            return;
        };
        if found == uval {
            break;
        }
        uval = found;
    }
    // PI futexes hand over through the PI state (not implemented, so none
    // exist to wake); a plain futex wakes one waiter to find OWNER_DIED.
    if !pi && uval & FUTEX_WAITERS != 0 {
        mem.wake_one(uaddr);
    }
}

/// The calling thread's own memory, for its exit.
#[cfg(feature = "alloc")]
struct CurrentMemory;

#[cfg(feature = "alloc")]
impl RobustMemory for CurrentMemory {
    fn read_u64(&mut self, addr: usize) -> Option<u64> {
        crate::syscall::userspace::read_user::<u64>(addr).ok()
    }
    fn read_u32(&mut self, addr: usize) -> Option<u32> {
        crate::syscall::userspace::read_user::<u32>(addr).ok()
    }
    fn cmpxchg_u32(&mut self, addr: usize, old: u32, new: u32) -> Option<u32> {
        crate::syscall::userspace::cmpxchg_user_u32(addr, old, new).ok()
    }
    fn wake_one(&mut self, addr: usize) {
        let _ = crate::syscall::sys_futex_wake(addr, 1, 0);
    }
}

/// Run by an exiting thread in its own address space, before
/// CLONE_CHILD_CLEARTID is handled (as Linux orders them).
#[cfg(feature = "alloc")]
pub fn exit_thread(thread: &super::Thread) {
    let head = thread
        .robust_list
        .swap(0, core::sync::atomic::Ordering::AcqRel);
    if head != 0 {
        exit_robust_list(&mut CurrentMemory, head, thread.tid.0 as u32);
    }
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeMap, vec::Vec};

    use super::*;

    /// A sparse little-endian memory with a record of wakes.
    #[derive(Default)]
    struct Fake {
        bytes: BTreeMap<usize, u8>,
        woken: Vec<usize>,
    }

    impl Fake {
        fn put64(&mut self, at: usize, v: u64) {
            for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                self.bytes.insert(at + i, b);
            }
        }
        fn put32(&mut self, at: usize, v: u32) {
            for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                self.bytes.insert(at + i, b);
            }
        }
        fn get(&self, at: usize, n: usize) -> Option<u64> {
            let mut v = 0u64;
            for i in (0..n).rev() {
                v = (v << 8) | *self.bytes.get(&(at + i))? as u64;
            }
            Some(v)
        }
    }

    impl RobustMemory for Fake {
        fn read_u64(&mut self, addr: usize) -> Option<u64> {
            self.get(addr, 8)
        }
        fn read_u32(&mut self, addr: usize) -> Option<u32> {
            self.get(addr, 4).map(|v| v as u32)
        }
        fn cmpxchg_u32(&mut self, addr: usize, old: u32, new: u32) -> Option<u32> {
            let found = self.read_u32(addr)?;
            if found == old {
                self.put32(addr, new);
            }
            Some(found)
        }
        fn wake_one(&mut self, addr: usize) {
            self.woken.push(addr);
        }
    }

    const HEAD: usize = 0x1000;
    /// musl's pthread_mutex: the robust list node sits 16 bytes after the
    /// lock word; futex_offset points back to it.
    const OFFSET: i64 = -16;

    /// A list HEAD -> nodes... -> HEAD with the given futex values.
    fn list(values: &[u32], pending: usize) -> Fake {
        let mut m = Fake::default();
        let nodes: Vec<usize> = (0..values.len()).map(|i| 0x2000 + i * 0x100).collect();
        m.put64(HEAD, nodes.first().copied().unwrap_or(HEAD) as u64);
        m.put64(HEAD + 8, OFFSET as u64);
        m.put64(HEAD + 16, pending as u64);
        for (i, &node) in nodes.iter().enumerate() {
            let next = nodes.get(i + 1).copied().unwrap_or(HEAD);
            m.put64(node, next as u64);
            m.put32((node as i64 + OFFSET) as usize, values[i]);
        }
        m
    }

    fn futex(m: &mut Fake, i: usize) -> u32 {
        m.read_u32((0x2000 + i * 0x100) as i64 as usize - 16)
            .unwrap()
    }

    #[test]
    fn owned_futexes_are_marked_and_waiters_woken() {
        let tid = 42;
        let mut m = list(&[tid, tid | FUTEX_WAITERS, 7, 7 | FUTEX_WAITERS], 0);
        exit_robust_list(&mut m, HEAD, tid);
        assert_eq!(futex(&mut m, 0), FUTEX_OWNER_DIED);
        assert_eq!(futex(&mut m, 1), FUTEX_OWNER_DIED | FUTEX_WAITERS);
        // Another thread's locks are left alone.
        assert_eq!(futex(&mut m, 2), 7);
        assert_eq!(futex(&mut m, 3), 7 | FUTEX_WAITERS);
        assert_eq!(m.woken, [0x2100 - 16]);
    }

    #[test]
    fn pending_op_is_handled_once_and_unlocked_pending_wakes() {
        let tid = 9;
        // The pending entry is also linked: handled once, after the walk.
        let pending = 0x2000;
        let mut m = list(&[tid | FUTEX_WAITERS], pending);
        exit_robust_list(&mut m, HEAD, tid);
        assert_eq!(futex(&mut m, 0), FUTEX_OWNER_DIED | FUTEX_WAITERS);
        assert_eq!(m.woken, [0x2000 - 16]);
        // A pending unlock that already wrote 0: wake a waiter anyway.
        let mut m = list(&[0], 0x2000);
        exit_robust_list(&mut m, HEAD, tid);
        assert_eq!(futex(&mut m, 0), 0);
        assert_eq!(m.woken, [0x2000 - 16]);
    }

    #[test]
    fn pi_futexes_are_marked_but_not_woken() {
        let tid = 5;
        let mut m = list(&[tid | FUTEX_WAITERS], 0);
        // Mark the entry PI (low bit of the head's pointer to it).
        m.put64(HEAD, 0x2000 | 1);
        exit_robust_list(&mut m, HEAD, tid);
        assert_eq!(futex(&mut m, 0), FUTEX_OWNER_DIED | FUTEX_WAITERS);
        assert!(m.woken.is_empty());
    }

    #[test]
    fn bad_and_circular_lists_end_the_walk() {
        // An unreadable head: nothing happens.
        let mut empty = Fake::default();
        exit_robust_list(&mut empty, HEAD, 1);
        // A node pointing to itself: the walk stops at the limit.
        let tid = 3;
        let mut m = list(&[tid], 0);
        m.put64(0x2000, 0x2000);
        exit_robust_list(&mut m, HEAD, tid);
        assert_eq!(futex(&mut m, 0), FUTEX_OWNER_DIED);
        // A next pointer into nowhere ends it after that entry.
        let mut m = list(&[tid, tid], 0);
        m.put64(0x2000, 0xdead_0000);
        exit_robust_list(&mut m, HEAD, tid);
        assert_eq!(futex(&mut m, 0), FUTEX_OWNER_DIED);
        assert_eq!(futex(&mut m, 1), tid);
    }
}

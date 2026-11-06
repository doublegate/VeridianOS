//! PID namespaces (ADR 0022 step H5, Linux's `kernel/pid_namespace.c` and
//! `kernel/pid.c`).
//!
//! A process or thread has a number at each level of its PID namespace,
//! from the initial namespace down to its own. Its kernel id (`ProcessId`,
//! `ThreadId`) is its number in the initial namespace; the numbers below
//! it are kept in a registry entry, which ids in the initial namespace
//! alone do not need. Every interface that takes or returns a pid
//! translates it through the caller's namespace: [`find`] going in,
//! [`nr_in`] coming out (0 when the id is not visible there, as Linux's
//! pid_nr_ns).
//!
//! Numbers count up in each namespace from 1 and are not handed out
//! again, as the initial namespace's (docs/KNOWN-LIMITATIONS.md); the
//! first process made in a namespace is 1 there, its init.

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};

use spin::{Mutex, Once};

use super::{
    ns::{NsIdent, NsKind},
    userns::{Ucount, UserNs, DEFAULT_MAX_USER_NAMESPACES},
};

/// Linux's MAX_PID_NS_LEVEL: the deepest a PID namespace may be.
pub const MAX_LEVEL: usize = 32;

/// `/proc/sys/user/max_pid_namespaces` and its counts.
pub static PID_NAMESPACES: Ucount = Ucount::new(DEFAULT_MAX_USER_NAMESPACES);

/// A PID namespace (Linux's `struct pid_namespace`).
pub struct PidNs {
    pub ident: NsIdent,
    /// The user namespace it belongs to.
    pub owner: Arc<UserNs>,
    pub parent: Option<Arc<PidNs>>,
    /// 0 for the initial namespace.
    pub level: usize,
    /// The kernel uid charged for it (max_pid_namespaces).
    creator: u32,
    state: Mutex<State>,
}

struct State {
    /// Number -> kernel id; 0 while a clone has reserved it.
    ids: BTreeMap<u64, u64>,
    /// The next number to try.
    next: u64,
    /// Its init's kernel id (Linux's child_reaper), once it has one.
    reaper: Option<u64>,
    /// PIDNS_ADDING: cleared when its init exits, after which no process
    /// can be made in it (ENOMEM).
    adding: bool,
}

/// Why a PID namespace could not be made (create_pid_namespace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    /// ENOSPC: deeper than [`MAX_LEVEL`], or max_pid_namespaces reached.
    NoSpace,
    /// EPERM: its user namespace is not the parent's or below it.
    Permission,
}

/// Why no number could be given (alloc_pid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    /// EINVAL: a chosen number out of range, or not 1 in a namespace that
    /// has no init yet.
    Invalid,
    /// EPERM: choosing one without CAP_CHECKPOINT_RESTORE or CAP_SYS_ADMIN
    /// over the namespace's user namespace.
    Permission,
    /// EEXIST: the chosen number is taken.
    Exists,
    /// ENOMEM: the namespace's init has exited.
    Dead,
    /// EAGAIN: no number left below pid_max.
    Exhausted,
}

static INITIAL: Once<Arc<PidNs>> = Once::new();

/// The initial PID namespace.
pub fn initial() -> Arc<PidNs> {
    INITIAL
        .call_once(|| {
            Arc::new(PidNs {
                ident: NsIdent::initial(NsKind::Pid),
                owner: super::userns::initial(),
                parent: None,
                level: 0,
                creator: 0,
                state: Mutex::new(State {
                    ids: BTreeMap::new(),
                    next: 1,
                    reaper: Some(1),
                    adding: true,
                }),
            })
        })
        .clone()
}

impl PidNs {
    /// A new namespace below `parent` belonging to `owner`, charged to
    /// kernel uid `creator` (create_pid_namespace).
    pub fn create(
        parent: &Arc<PidNs>,
        owner: &Arc<UserNs>,
        creator: u32,
    ) -> Result<Arc<PidNs>, CreateError> {
        if parent.level + 1 > MAX_LEVEL {
            return Err(CreateError::NoSpace);
        }
        if !owner.is_in(&parent.owner) {
            return Err(CreateError::Permission);
        }
        if !PID_NAMESPACES.charge(creator) {
            return Err(CreateError::NoSpace);
        }
        Ok(Arc::new(PidNs {
            ident: NsIdent::fresh(NsKind::Pid),
            owner: owner.clone(),
            parent: Some(parent.clone()),
            level: parent.level + 1,
            creator,
            state: Mutex::new(State {
                ids: BTreeMap::new(),
                next: 1,
                reaper: None,
                adding: true,
            }),
        }))
    }

    pub fn is_initial(&self) -> bool {
        self.level == 0
    }

    /// Whether `other` is this namespace or one below it.
    pub fn contains(&self, other: &PidNs) -> bool {
        if other.level < self.level {
            return false;
        }
        let mut ns = other;
        while ns.level > self.level {
            match &ns.parent {
                Some(p) => ns = p,
                None => return false,
            }
        }
        ns.ident.id == self.ident.id
    }

    /// The kernel id of its init (1 for the initial namespace's), once it
    /// has one.
    pub fn reaper(&self) -> Option<u64> {
        self.state.lock().reaper
    }

    /// Whether processes can still be made in it.
    pub fn adding(&self) -> bool {
        self.state.lock().adding
    }

    /// Stop making processes in it: its init is exiting (Linux's
    /// disable_pid_allocation).
    pub fn stop_adding(&self) {
        if !self.is_initial() {
            self.state.lock().adding = false;
        }
    }

    /// The kernel id that number `nr` names here, if any.
    pub fn find(&self, nr: u64) -> Option<u64> {
        if nr == 0 {
            return None;
        }
        if self.is_initial() {
            return Some(nr);
        }
        self.state.lock().ids.get(&nr).copied().filter(|&k| k != 0)
    }

    /// The kernel ids with a number here (below the initial namespace).
    pub fn members(&self) -> Vec<u64> {
        self.state
            .lock()
            .ids
            .values()
            .copied()
            .filter(|&k| k != 0)
            .collect()
    }

    /// The last number handed out here (`/proc/loadavg`'s last field), 0
    /// before any; None for the initial namespace, which counts kernel ids.
    pub fn last_nr(&self) -> Option<u64> {
        (!self.is_initial()).then(|| self.state.lock().next.saturating_sub(1))
    }

    /// Its ancestor at `level` (itself at its own).
    fn at_level(&self, level: usize) -> Option<&PidNs> {
        let mut ns = self;
        while ns.level > level {
            ns = ns.parent.as_deref()?;
        }
        (ns.level == level).then_some(ns)
    }
}

impl Drop for PidNs {
    fn drop(&mut self) {
        if self.parent.is_some() {
            PID_NAMESPACES.uncharge(self.creator);
        }
    }
}

/// The numbers of an id below the initial namespace (Linux's `struct pid`).
struct Numbers {
    /// Its own (deepest) namespace.
    ns: Arc<PidNs>,
    /// The number at each level: `nrs[0]` is the kernel id.
    nrs: Vec<u64>,
}

impl Numbers {
    fn nr_at(&self, ns: &PidNs) -> u64 {
        match (self.nrs.get(ns.level), self.ns.at_level(ns.level)) {
            (Some(&nr), Some(at)) if at.ident.id == ns.ident.id => nr,
            _ => 0,
        }
    }
}

/// Kernel id -> its numbers, for ids in a namespace below the initial one.
static REGISTRY: Mutex<BTreeMap<u64, Arc<Numbers>>> = Mutex::new(BTreeMap::new());

fn numbers(kid: u64) -> Option<Arc<Numbers>> {
    REGISTRY.lock().get(&kid).cloned()
}

/// Whether kernel id `kid` has numbers below the initial namespace.
pub fn is_registered(kid: u64) -> bool {
    REGISTRY.lock().contains_key(&kid)
}

/// The namespace kernel id `kid` was made in (the initial one for an id
/// made there, or not known).
pub fn ns_of(kid: u64) -> Arc<PidNs> {
    numbers(kid).map_or_else(initial, |n| n.ns.clone())
}

/// Kernel id `kid`'s number in `ns`: 0 when it has none there (it was
/// made above `ns` or beside it), as Linux's pid_nr_ns.
pub fn nr_in(kid: u64, ns: &PidNs) -> u64 {
    if kid == 0 {
        return 0;
    }
    if ns.is_initial() {
        return kid;
    }
    numbers(kid).map_or(0, |n| n.nr_at(ns))
}

/// Kernel id `kid`'s numbers from `ns` down to its own namespace (the
/// NStgid and NSpid lines of `/proc/<pid>/status`); empty when it has none
/// in `ns`.
pub fn nrs_from(kid: u64, ns: &PidNs) -> Vec<u64> {
    if ns.is_initial() && numbers(kid).is_none() {
        return alloc::vec![kid];
    }
    match numbers(kid) {
        Some(n) if n.nr_at(ns) != 0 => n.nrs[ns.level..].to_vec(),
        _ => Vec::new(),
    }
}

/// The namespaces from `ns` down to kernel id `kid`'s own, the levels its
/// [`nrs_from`] numbers are at; empty when it has none in `ns`.
pub fn levels_from(kid: u64, ns: &Arc<PidNs>) -> Vec<Arc<PidNs>> {
    let Some(n) = numbers(kid) else {
        return if ns.is_initial() {
            alloc::vec![ns.clone()]
        } else {
            Vec::new()
        };
    };
    if n.nr_at(ns) == 0 {
        return Vec::new();
    }
    let mut chain = Vec::new();
    let mut level = Some(n.ns.clone());
    while let Some(l) = level {
        if l.level < ns.level {
            break;
        }
        level = l.parent.clone();
        chain.push(l);
    }
    chain.reverse();
    chain
}

/// Number `nr` in `ns` as a kernel id, if it names one there.
pub fn find(nr: u64, ns: &PidNs) -> Option<u64> {
    ns.find(nr)
}

/// The calling process's PID namespace (task_active_pid_ns); the initial
/// one in kernel context.
pub fn current() -> Arc<PidNs> {
    super::current_process().map_or_else(initial, |p| p.pid_ns())
}

/// Kernel id `kid` as the caller sees it (pid_vnr; 0 if not visible).
pub fn vnr(kid: u64) -> u64 {
    nr_in(kid, &current())
}

/// A pid the caller passed, as a kernel id (find_vpid), if it names one in
/// the caller's namespace. Not positive: None.
pub fn from_vnr(nr: i64) -> Option<u64> {
    if nr <= 0 {
        return None;
    }
    current().find(nr as u64)
}

/// Numbers reserved for a process or thread being made in `ns`, one at
/// each level below the initial namespace, until the kernel id they belong
/// to is known ([`Reservation::commit`]); dropped uncommitted, they are
/// free again.
pub struct Reservation {
    ns: Arc<PidNs>,
    /// `nrs[i]` is the number at level `i + 1`.
    nrs: Vec<u64>,
    committed: bool,
}

/// Reserve numbers in `ns` and each namespace above it but the initial
/// one, as Linux's alloc_pid from the innermost level out: `chosen[i]` is
/// clone3's set_tid entry for level `ns.level - i` (the innermost first;
/// the caller checks the entry for the initial namespace, the kernel id
/// itself). A chosen number must be below pid_max, and 1 in a namespace
/// with no init yet (EINVAL), needs `may_choose` over the namespace's user
/// namespace (EPERM) and must be free (EEXIST). The namespace's init having
/// exited is ENOMEM.
pub fn reserve(
    ns: &Arc<PidNs>,
    chosen: &[u64],
    may_choose: impl Fn(&UserNs) -> bool,
) -> Result<Reservation, AllocError> {
    let mut res = Reservation {
        ns: ns.clone(),
        nrs: alloc::vec![0; ns.level],
        committed: false,
    };
    let mut level_ns: &PidNs = ns;
    while level_ns.level > 0 {
        let level = level_ns.level;
        let pick = chosen.get(ns.level - level).copied();
        let mut st = level_ns.state.lock();
        let nr = match pick {
            Some(nr) => {
                if !(1..super::PID_MAX).contains(&nr) || (nr != 1 && st.reaper.is_none()) {
                    return Err(AllocError::Invalid);
                }
                drop(st);
                if !may_choose(&level_ns.owner) {
                    return Err(AllocError::Permission);
                }
                st = level_ns.state.lock();
                if st.ids.contains_key(&nr) {
                    return Err(AllocError::Exists);
                }
                nr
            }
            None => {
                let mut nr = st.next;
                while st.ids.contains_key(&nr) {
                    nr += 1;
                }
                if nr >= super::PID_MAX {
                    return Err(AllocError::Exhausted);
                }
                st.next = nr + 1;
                nr
            }
        };
        st.ids.insert(nr, 0);
        res.nrs[level - 1] = nr;
        drop(st);
        level_ns = level_ns
            .parent
            .as_deref()
            .expect("level above 0 has a parent");
    }
    if !ns.adding() {
        return Err(AllocError::Dead);
    }
    Ok(res)
}

impl Reservation {
    /// The number in the innermost namespace (the kernel id's own in the
    /// initial one: None).
    pub fn innermost(&self) -> Option<u64> {
        self.nrs.last().copied()
    }

    /// The numbers belong to kernel id `kid` from now on; the innermost
    /// namespace's number 1 makes it that namespace's init.
    pub fn commit(mut self, kid: u64) {
        self.committed = true;
        if self.ns.is_initial() {
            return;
        }
        let mut level_ns: &PidNs = &self.ns;
        while level_ns.level > 0 {
            let nr = self.nrs[level_ns.level - 1];
            let mut st = level_ns.state.lock();
            st.ids.insert(nr, kid);
            if level_ns.level == self.ns.level && nr == 1 {
                st.reaper = Some(kid);
            }
            drop(st);
            level_ns = level_ns
                .parent
                .as_deref()
                .expect("level above 0 has a parent");
        }
        let mut nrs = alloc::vec![kid];
        nrs.extend_from_slice(&self.nrs);
        REGISTRY.lock().insert(
            kid,
            Arc::new(Numbers {
                ns: self.ns.clone(),
                nrs,
            }),
        );
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut level_ns: &PidNs = &self.ns;
        while level_ns.level > 0 {
            let nr = self.nrs[level_ns.level - 1];
            if nr != 0 {
                let mut st = level_ns.state.lock();
                if st.ids.get(&nr) == Some(&0) {
                    st.ids.remove(&nr);
                }
            }
            match level_ns.parent.as_deref() {
                Some(p) => level_ns = p,
                None => break,
            }
        }
    }
}

/// A [`Reservation`] handed from the clone call that made it (where its
/// errors are reported) to the fork that gives it a kernel id; it can be
/// taken once.
#[derive(Clone)]
pub struct Pending(Arc<Mutex<Option<Reservation>>>);

impl Pending {
    pub fn new(r: Reservation) -> Self {
        Self(Arc::new(Mutex::new(Some(r))))
    }

    pub fn take(&self) -> Option<Reservation> {
        self.0.lock().take()
    }
}

impl core::fmt::Debug for Pending {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Pending")
    }
}

/// Kernel id `kid` is gone (its thread or process reaped): its numbers are
/// free (Linux's free_pid). A namespace left with no number takes no more
/// processes.
pub fn release(kid: u64) {
    let Some(n) = REGISTRY.lock().remove(&kid) else {
        return;
    };
    let mut level_ns: &PidNs = &n.ns;
    while level_ns.level > 0 {
        let mut st = level_ns.state.lock();
        if st.ids.get(&n.nrs[level_ns.level]) == Some(&kid) {
            st.ids.remove(&n.nrs[level_ns.level]);
        }
        // The last number gone (its init's, or that of a first fork that
        // failed): no process can be made in it any more, as Linux's
        // free_pid ending PIDNS_ADDING.
        if st.ids.is_empty() {
            st.adding = false;
        }
        drop(st);
        level_ns = level_ns
            .parent
            .as_deref()
            .expect("level above 0 has a parent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::userns;

    fn child_of(parent: &Arc<PidNs>) -> Arc<PidNs> {
        PidNs::create(parent, &userns::initial(), 0).unwrap()
    }

    #[test]
    fn ids_have_a_number_at_each_level() {
        let init = initial();
        let a = child_of(&init);
        let b = child_of(&a);
        assert_eq!(b.level, 2);
        assert!(init.contains(&b) && a.contains(&b) && b.contains(&b));
        assert!(!b.contains(&a));

        // The first process in `b` is its init: 1 there, 1 in `a`.
        let r = reserve(&b, &[], |_| true).unwrap();
        assert_eq!(r.innermost(), Some(1));
        r.commit(9_000_001);
        assert_eq!(b.reaper(), Some(9_000_001));
        assert_eq!(a.reaper(), None);
        assert_eq!(nr_in(9_000_001, &b), 1);
        assert_eq!(nr_in(9_000_001, &a), 1);
        assert_eq!(nr_in(9_000_001, &init), 9_000_001);
        assert_eq!(nrs_from(9_000_001, &a), [1, 1]);
        assert_eq!(nrs_from(9_000_001, &init), [9_000_001, 1, 1]);
        assert_eq!(b.find(1), Some(9_000_001));
        assert_eq!(a.find(1), Some(9_000_001));
        assert_eq!(init.find(9_000_001), Some(9_000_001));

        // A process of `a` is not seen from `b`.
        reserve(&a, &[], |_| true).unwrap().commit(9_000_002);
        assert_eq!(nr_in(9_000_002, &a), 2);
        assert_eq!(nr_in(9_000_002, &b), 0);
        assert!(nrs_from(9_000_002, &b).is_empty());
        assert_eq!(ns_of(9_000_002).ident.id, a.ident.id);

        // Nor one beside it.
        let c = child_of(&a);
        reserve(&c, &[], |_| true).unwrap().commit(9_000_003);
        assert_eq!(nr_in(9_000_003, &b), 0);
        assert_eq!(nr_in(9_000_003, &c), 1);
        assert_eq!(nr_in(9_000_003, &a), 3);

        // Released, its numbers name nothing; they are not handed out
        // again.
        release(9_000_002);
        assert_eq!(a.find(2), None);
        assert_eq!(nr_in(9_000_002, &a), 0);
        reserve(&a, &[], |_| true).unwrap().commit(9_000_004);
        assert_eq!(nr_in(9_000_004, &a), 4);
        assert_eq!(a.last_nr(), Some(4));
        for k in [9_000_001, 9_000_003, 9_000_004] {
            release(k);
        }
    }

    #[test]
    fn set_tid_follows_alloc_pid() {
        let a = child_of(&initial());
        // No init yet: only 1 may be chosen.
        assert_eq!(reserve(&a, &[5], |_| true).err(), Some(AllocError::Invalid));
        assert_eq!(
            reserve(&a, &[1], |_| false).err(),
            Some(AllocError::Permission)
        );
        let r = reserve(&a, &[1], |_| true).unwrap();
        // Reserved, it is taken; dropped, free again.
        assert_eq!(reserve(&a, &[1], |_| true).err(), Some(AllocError::Exists));
        drop(r);
        reserve(&a, &[1], |_| true).unwrap().commit(9_100_001);
        let r = reserve(&a, &[7], |_| true).unwrap();
        assert_eq!(r.innermost(), Some(7));
        r.commit(9_100_007);
        assert_eq!(a.find(7), Some(9_100_007));
        assert_eq!(reserve(&a, &[0], |_| true).err(), Some(AllocError::Invalid));
        assert_eq!(
            reserve(&a, &[crate::process::PID_MAX], |_| true).err(),
            Some(AllocError::Invalid)
        );
        // Its init gone, nothing more is made in it.
        a.stop_adding();
        assert_eq!(reserve(&a, &[], |_| true).err(), Some(AllocError::Dead));
        assert!(a.find(2).is_none(), "the failed reservation was undone");
        // The initial namespace never stops.
        initial().stop_adding();
        assert!(initial().adding());
        release(9_100_001);
        release(9_100_007);
    }

    /// A first fork that fails gives its number back and leaves the
    /// namespace with nobody in it: no process can be made there (Linux's
    /// free_pid clearing PIDNS_ADDING), rather than one without an init.
    #[test]
    fn a_namespace_whose_first_process_failed_takes_no_more() {
        let a = child_of(&initial());
        reserve(&a, &[], |_| true).unwrap().commit(9_200_001);
        release(9_200_001);
        assert!(!a.adding());
        assert_eq!(reserve(&a, &[], |_| true).err(), Some(AllocError::Dead));
    }

    #[test]
    fn creation_follows_create_pid_namespace() {
        let mut ns = initial();
        for _ in 0..MAX_LEVEL {
            ns = child_of(&ns);
        }
        assert_eq!(ns.level, MAX_LEVEL);
        assert_eq!(
            PidNs::create(&ns, &userns::initial(), 0).err(),
            Some(CreateError::NoSpace)
        );
        // Its user namespace must be its parent's or below it.
        let user = userns::UserNs::create(&userns::initial(), 4_100_000, 0, false, false).unwrap();
        let owned = PidNs::create(&initial(), &user, 4_100_000).unwrap();
        assert_eq!(
            PidNs::create(&owned, &userns::initial(), 0).err(),
            Some(CreateError::Permission)
        );
        // max_pid_namespaces counts per creating user.
        PID_NAMESPACES.fill(4_100_000);
        assert_eq!(
            PidNs::create(&initial(), &user, 4_100_000).err(),
            Some(CreateError::NoSpace)
        );
        PID_NAMESPACES.clear(4_100_000);
        drop(owned);
        let again = PidNs::create(&initial(), &user, 4_100_000).unwrap();
        drop(again);
        assert_eq!(PID_NAMESPACES.count(4_100_000), 0);
    }
}

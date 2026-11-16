//! seccomp (Linux's `kernel/seccomp.c`, v0.28 item 5, ADR 0027): a
//! thread's system call filter.
//!
//! Each thread has a mode (disabled, strict, filter) and, in filter mode, a
//! chain of classic BPF programs (newest first), shared with the threads
//! and processes it creates and kept across exec. Every call runs every
//! program in the chain on its `struct seccomp_data` and takes the action
//! of highest precedence (the lowest action value as a signed number:
//! KILL_PROCESS first, ALLOW last). The programs are checked and run by the
//! `veridian-cbpf` crate, the same code the user-space Linux runtime will
//! use (Track M), and the check runs at the top of dispatch, before the
//! call is decoded, so the private VeridianOS calls are filtered too.
//!
//! A filter installed with SECCOMP_FILTER_FLAG_NEW_LISTENER has a
//! listener ([`Notify`]): SECCOMP_RET_USER_NOTIF from it parks the calling
//! thread until a supervisor holding the listener descriptor answers. As on
//! Linux a filter counts its users -- the threads whose chain starts with
//! it and the filters installed on top of it ([`Chain`]) -- so the listener
//! can tell when nothing can notify any more (POLLHUP).

use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use spin::Mutex;
use veridian_cbpf::SeccompProgram;

/// The modes (SECCOMP_MODE_*).
pub const MODE_DISABLED: u8 = 0;
pub const MODE_STRICT: u8 = 1;
pub const MODE_FILTER: u8 = 2;

/// Filter return values (SECCOMP_RET_*): the action in the high 16 bits,
/// its data in the low 16.
pub mod ret {
    pub const KILL_PROCESS: u32 = 0x8000_0000;
    pub const KILL_THREAD: u32 = 0x0000_0000;
    pub const TRAP: u32 = 0x0003_0000;
    pub const ERRNO: u32 = 0x0005_0000;
    pub const USER_NOTIF: u32 = 0x7fc0_0000;
    pub const TRACE: u32 = 0x7ff0_0000;
    pub const LOG: u32 = 0x7ffc_0000;
    pub const ALLOW: u32 = 0x7fff_0000;
    /// SECCOMP_RET_ACTION_FULL and SECCOMP_RET_DATA.
    pub const ACTION_FULL: u32 = 0xffff_0000;
    pub const DATA: u32 = 0x0000_ffff;
}

/// The most instructions one thread's chain may hold, each program
/// counting four more (MAX_INSNS_PER_PATH).
pub const MAX_INSNS_PER_PATH: usize = 32768;

/// SECCOMP_USER_NOTIF_FLAG_CONTINUE: the supervisor lets the call run.
pub const USER_NOTIF_FLAG_CONTINUE: u32 = 1;
/// SECCOMP_USER_NOTIF_FD_SYNC_WAKE_UP (SET_FLAGS).
pub const USER_NOTIF_FD_SYNC_WAKE_UP: u64 = 1;

/// Errno values the notification paths return (as positive numbers).
pub mod errno {
    pub const ESRCH: i32 = 3;
    pub const EINTR: i32 = 4;
    pub const EBUSY: i32 = 16;
    pub const ENOENT: i32 = 2;
    pub const ENOSYS: i32 = 38;
    pub const EINPROGRESS: i32 = 115;
}

/// One installed program and the chain it was installed on.
pub struct Filter {
    prog: SeccompProgram,
    prev: Option<Arc<Filter>>,
    /// SECCOMP_FILTER_FLAG_LOG: actions other than ALLOW are logged.
    pub log: bool,
    /// The chain's length in instructions, this program included, with
    /// the per-program overhead.
    path_len: usize,
    /// How many programs the chain has (Seccomp_filters in status).
    depth: usize,
    /// Threads whose chain starts here, plus filters on top of this one
    /// that still have users (Linux's `users`).
    users: AtomicUsize,
    /// The listener (NEW_LISTENER), or None.
    notify: Option<Mutex<Notify>>,
    /// SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV: once the supervisor has
    /// received a notification, only a fatal signal ends the wait.
    pub wait_killable_recv: bool,
    /// Woken on every change of a notification and when the filter loses
    /// its last user (Linux's `wqh`).
    pub wq: crate::sched::dispatch::WaitQueue,
}

impl Filter {
    /// The chain of `prog` on top of `prev`, or None past
    /// [`MAX_INSNS_PER_PATH`] (ENOMEM). `listener` gives it a listener
    /// (NEW_LISTENER), first notification id `first_id`.
    pub fn make(
        prog: SeccompProgram,
        prev: Option<Arc<Filter>>,
        log: bool,
        listener: Option<u64>,
        wait_killable_recv: bool,
    ) -> Option<Chain> {
        let below = prev.as_ref().map_or(0, |p| p.path_len);
        let path_len = below + prog.len() + 4;
        if path_len > MAX_INSNS_PER_PATH {
            return None;
        }
        let depth = prev.as_ref().map_or(0, |p| p.depth) + 1;
        // The new filter is a user of the one below it.
        if let Some(p) = &prev {
            p.users.fetch_add(1, Ordering::AcqRel);
        }
        let filter = Arc::new(Self {
            prog,
            prev,
            log,
            path_len,
            depth,
            users: AtomicUsize::new(0),
            notify: listener.map(|first_id| Mutex::new(Notify::new(first_id))),
            wait_killable_recv,
            wq: crate::sched::dispatch::WaitQueue::new(),
        });
        Some(Chain::adopt(filter))
    }

    /// How many programs the chain holds.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Whether any thread or filter still uses this one.
    pub fn has_users(&self) -> bool {
        self.users.load(Ordering::Acquire) != 0
    }

    /// Whether `other` is this chain or one of its ancestors (TSYNC's
    /// is_ancestor: a thread whose chain is not under the caller's cannot
    /// be synchronized).
    pub fn has_ancestor(self: &Arc<Self>, other: Option<&Arc<Filter>>) -> bool {
        let Some(other) = other else {
            return true;
        };
        let mut f = Some(self);
        while let Some(cur) = f {
            if Arc::ptr_eq(cur, other) {
                return true;
            }
            f = cur.prev.as_ref();
        }
        false
    }

    /// Whether this filter or one below it has a listener still open
    /// (has_duplicate_listener: a chain has at most one).
    pub fn has_listener(&self) -> bool {
        let mut f = Some(self);
        while let Some(cur) = f {
            if cur.notify.as_ref().is_some_and(|n| n.lock().attached) {
                return true;
            }
            f = cur.prev.as_deref();
        }
        false
    }

    /// The listener's state, for a filter made with one.
    pub fn notify(&self) -> Option<&Mutex<Notify>> {
        self.notify.as_ref()
    }

    /// Report a change: wake the filter's waiters (target and supervisor)
    /// and anything polling the listener.
    pub fn changed(&self) {
        self.wq.wake_all();
        #[cfg(feature = "alloc")]
        crate::sched::dispatch::io_event();
    }
}

impl Drop for Filter {
    /// Free the chain below iteratively: a chain can hold thousands of
    /// programs, and dropping it by recursion would overflow the kernel
    /// stack.
    fn drop(&mut self) {
        let mut next = self.prev.take();
        while let Some(f) = next {
            match Arc::try_unwrap(f) {
                Ok(mut inner) => next = inner.prev.take(),
                Err(_) => break,
            }
        }
    }
}

/// A thread's hold on a chain: one user of its first filter. Dropping the
/// last hold on a filter takes it off the filter below as well, and wakes
/// a listener that may be waiting for exactly that
/// (`__seccomp_filter_orphan`).
pub struct Chain(Arc<Filter>);

impl Chain {
    fn adopt(filter: Arc<Filter>) -> Self {
        filter.users.fetch_add(1, Ordering::AcqRel);
        Self(filter)
    }

    /// The chain's first (newest) filter.
    pub fn head(&self) -> &Arc<Filter> {
        &self.0
    }
}

impl Clone for Chain {
    fn clone(&self) -> Self {
        Self::adopt(self.0.clone())
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        let mut cur = Some(&self.0);
        while let Some(f) = cur {
            if f.users.fetch_sub(1, Ordering::AcqRel) != 1 {
                break;
            }
            if f.notify.is_some() {
                f.changed();
            }
            cur = f.prev.as_ref();
        }
    }
}

/// The value of highest precedence `chain` returns on `data`
/// (seccomp_run_filters), and the program that returned it (for LOG and
/// USER_NOTIF).
pub fn run_chain(chain: &Arc<Filter>, data: &[u8; 64]) -> (u32, Arc<Filter>) {
    let mut best = ret::ALLOW;
    let mut matched = chain;
    let mut f = Some(chain);
    while let Some(cur) = f {
        let r = veridian_cbpf::run_seccomp(&cur.prog, data);
        if ((r & ret::ACTION_FULL) as i32) < ((best & ret::ACTION_FULL) as i32) {
            best = r;
            matched = cur;
        }
        f = cur.prev.as_ref();
    }
    (best, matched.clone())
}

/// Where a notification is (`enum notify_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifState {
    /// Waiting for the supervisor to receive it.
    Init,
    /// Received; waiting for the answer.
    Sent,
    /// Answered.
    Replied,
}

/// A request to install a descriptor in the notifying thread
/// (`struct seccomp_kaddfd`), carried out by that thread. Generic over the
/// file so the state machine is testable on its own.
pub struct AddFd<F> {
    pub file: F,
    /// SECCOMP_ADDFD_FLAG_SETFD: exactly this descriptor.
    pub newfd: Option<u32>,
    pub cloexec: bool,
    /// SECCOMP_ADDFD_FLAG_SEND: the descriptor is also the call's result.
    pub send: bool,
    /// The descriptor installed, or a negative errno; None until done.
    pub result: Mutex<Option<i64>>,
}

/// What taking back an install found ([`Notify::addfd_cancel`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddFdCancel {
    /// Still queued: removed, never carried out.
    Removed,
    /// Carried out meanwhile, with this result.
    Done(i64),
    /// Being carried out by the target: its result follows.
    InFlight,
}

/// One notification (`struct seccomp_knotif`).
pub struct Knotif<F> {
    pub id: u64,
    /// The notifying thread (kernel id).
    pub tid: u64,
    pub data: [u8; 64],
    pub state: NotifState,
    pub error: i32,
    pub val: i64,
    pub flags: u32,
    /// Waiting descriptor installs, newest first.
    pub addfd: VecDeque<Arc<AddFd<F>>>,
}

/// The listener's state (`struct notification`).
pub struct Notify<F = Arc<crate::fs::file::File>> {
    /// The listener descriptor is still open.
    pub attached: bool,
    /// SECCOMP_USER_NOTIF_FD_* flags.
    pub flags: u64,
    next_id: u64,
    pub list: Vec<Knotif<F>>,
}

/// What a notifying thread should do next.
pub enum TargetStep<F> {
    /// Keep waiting.
    Wait,
    /// Install this descriptor, then look again.
    AddFd(Arc<AddFd<F>>),
    /// Answered: (error, val, flags).
    Replied(i32, i64, u32),
}

impl<F> Notify<F> {
    pub fn new(first_id: u64) -> Self {
        Self {
            attached: true,
            flags: 0,
            next_id: first_id,
            list: Vec::new(),
        }
    }

    fn find(&mut self, id: u64) -> Option<&mut Knotif<F>> {
        self.list.iter_mut().find(|n| n.id == id)
    }

    /// Queue a notification from thread `tid` (seccomp_do_user_notification):
    /// its id, or None once the listener is closed (ENOSYS).
    pub fn begin(&mut self, tid: u64, data: [u8; 64]) -> Option<u64> {
        if !self.attached {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.list.push(Knotif {
            id,
            tid,
            data,
            state: NotifState::Init,
            error: 0,
            val: 0,
            flags: 0,
            addfd: VecDeque::new(),
        });
        Some(id)
    }

    /// The notifying thread's next step.
    pub fn step(&mut self, id: u64) -> TargetStep<F> {
        let Some(n) = self.find(id) else {
            return TargetStep::Replied(-errno::ENOSYS, 0, 0);
        };
        if let Some(req) = n.addfd.pop_front() {
            return TargetStep::AddFd(req);
        }
        match n.state {
            NotifState::Replied => TargetStep::Replied(n.error, n.val, n.flags),
            _ => TargetStep::Wait,
        }
    }

    /// The descriptor install `req` is done with `result`
    /// (seccomp_handle_addfd): with SEND it is also the answer, or, failed,
    /// puts the notification back to Sent.
    pub fn addfd_done(&mut self, id: u64, req: &AddFd<F>, result: i64) {
        *req.result.lock() = Some(result);
        if !req.send {
            return;
        }
        if let Some(n) = self.find(id) {
            if result < 0 {
                n.state = NotifState::Sent;
            } else {
                n.flags = 0;
                n.error = 0;
                n.val = result;
            }
        }
    }

    /// The notifying thread is done (answered or interrupted): drop the
    /// notification, failing installs still waiting with ESRCH.
    pub fn finish(&mut self, id: u64) {
        if let Some(pos) = self.list.iter().position(|n| n.id == id) {
            let n = self.list.remove(pos);
            for req in n.addfd {
                *req.result.lock() = Some(-(errno::ESRCH as i64));
            }
        }
    }

    /// Whether `id` is waiting for its answer (state Sent), the
    /// notifying thread then still blocked in the call (ID_VALID).
    pub fn id_valid(&mut self, id: u64) -> bool {
        self.find(id).is_some_and(|n| n.state == NotifState::Sent)
    }

    /// The oldest notification not yet received (NOTIF_RECV), now Sent:
    /// (id, tid, data).
    pub fn recv(&mut self) -> Option<(u64, u64, [u8; 64])> {
        let n = self.list.iter_mut().find(|n| n.state == NotifState::Init)?;
        n.state = NotifState::Sent;
        Some((n.id, n.tid, n.data))
    }

    /// Undo a [`Self::recv`] whose copy to the supervisor failed.
    pub fn unrecv(&mut self, id: u64) {
        if let Some(n) = self.find(id) {
            n.state = NotifState::Init;
        }
    }

    /// Whether some notification waits to be received (POLLIN) or answered
    /// (POLLOUT).
    pub fn readiness(&self) -> (bool, bool) {
        let pending = self.list.iter().any(|n| n.state == NotifState::Init);
        let sent = self.list.iter().any(|n| n.state == NotifState::Sent);
        (pending, sent)
    }

    /// Answer `id` (NOTIF_SEND): ENOENT if unknown, EINPROGRESS unless
    /// received and not yet answered.
    pub fn send(&mut self, id: u64, error: i32, val: i64, flags: u32) -> Result<(), i32> {
        let n = self.find(id).ok_or(errno::ENOENT)?;
        if n.state != NotifState::Sent {
            return Err(errno::EINPROGRESS);
        }
        n.state = NotifState::Replied;
        n.error = error;
        n.val = val;
        n.flags = flags;
        Ok(())
    }

    /// Queue descriptor install `req` for `id` (NOTIF_ADDFD): ENOENT,
    /// EINPROGRESS unless received and not yet answered, EBUSY for SEND
    /// with installs still waiting. With SEND the notification counts as
    /// answered.
    pub fn addfd(&mut self, id: u64, req: Arc<AddFd<F>>) -> Result<(), i32> {
        let n = self.find(id).ok_or(errno::ENOENT)?;
        if n.state != NotifState::Sent {
            return Err(errno::EINPROGRESS);
        }
        if req.send {
            if !n.addfd.is_empty() {
                return Err(errno::EBUSY);
            }
            n.state = NotifState::Replied;
        }
        n.addfd.push_front(req);
        Ok(())
    }

    /// Take back an install the supervisor stopped waiting for (a signal).
    /// Still queued, it is removed; carried out, its result is the answer;
    /// taken by the target and not yet done, the supervisor must wait for
    /// it, since the descriptor is being installed (Linux's target installs
    /// under notify_lock, so its supervisor sees the result; PR #49
    /// review). Only the target takes a request off the queue, and it
    /// always reports the result.
    pub fn addfd_cancel(&mut self, id: u64, req: &Arc<AddFd<F>>) -> AddFdCancel {
        if let Some(n) = self.find(id) {
            if let Some(pos) = n.addfd.iter().position(|r| Arc::ptr_eq(r, req)) {
                n.addfd.remove(pos);
                return AddFdCancel::Removed;
            }
        }
        match *req.result.lock() {
            Some(r) => AddFdCancel::Done(r),
            None => AddFdCancel::InFlight,
        }
    }

    /// The listener is closed (seccomp_notify_detach): every notification
    /// not yet answered is answered ENOSYS, and no new one is made.
    pub fn detach(&mut self) {
        self.attached = false;
        for n in &mut self.list {
            if n.state != NotifState::Replied {
                n.state = NotifState::Replied;
                n.error = -errno::ENOSYS;
                n.val = 0;
            }
        }
    }
}

/// A change of mode refused: strict to filter or back (EINVAL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeConflict;

/// A thread's seccomp state.
pub struct ThreadSeccomp {
    mode: AtomicU8,
    chain: Mutex<Option<Chain>>,
}

impl Default for ThreadSeccomp {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadSeccomp {
    pub const fn new() -> Self {
        Self {
            mode: AtomicU8::new(MODE_DISABLED),
            chain: Mutex::new(None),
        }
    }

    /// The mode (the fast path reads only this).
    pub fn mode(&self) -> u8 {
        self.mode.load(Ordering::Acquire)
    }

    /// The chain's first filter, in filter mode (for running it; this is
    /// not a hold on the chain).
    pub fn head(&self) -> Option<Arc<Filter>> {
        self.chain.lock().as_ref().map(|c| c.head().clone())
    }

    /// A copy for a new thread or process (copy_seccomp): the same mode and
    /// chain, which the two then share.
    pub fn inherit(&self) -> Self {
        let chain = self.chain.lock().clone();
        Self {
            mode: AtomicU8::new(self.mode()),
            chain: Mutex::new(chain),
        }
    }

    /// Copy `other`'s state into this one (TSYNC, and inheritance into an
    /// already made thread).
    pub fn copy_from(&self, other: &Self) {
        let chain = other.chain.lock().clone();
        let old = core::mem::replace(&mut *self.chain.lock(), chain);
        self.mode.store(other.mode(), Ordering::Release);
        drop(old);
    }

    /// Enter strict mode (EINVAL from filter mode, as seccomp_may_assign_mode).
    pub fn set_strict(&self) -> Result<(), ModeConflict> {
        match self.mode() {
            MODE_FILTER => Err(ModeConflict),
            _ => {
                self.mode.store(MODE_STRICT, Ordering::Release);
                Ok(())
            }
        }
    }

    /// Put `chain` (made on top of this thread's) in place and enter
    /// filter mode (EINVAL from strict mode).
    pub fn set_filter(&self, chain: Chain) -> Result<(), ModeConflict> {
        if self.mode() == MODE_STRICT {
            return Err(ModeConflict);
        }
        let old = self.chain.lock().replace(chain);
        self.mode.store(MODE_FILTER, Ordering::Release);
        drop(old);
        Ok(())
    }

    /// The thread is exiting (seccomp_filter_release): it stops being a
    /// user of its chain.
    pub fn release(&self) {
        let old = self.chain.lock().take();
        drop(old);
    }
}

/// The calls strict mode allows (mode1_syscalls): read, write, exit and
/// rt_sigreturn, by this architecture's Linux numbers.
pub fn strict_allows(nr: usize) -> bool {
    #[cfg(target_arch = "x86_64")]
    const ALLOWED: [usize; 4] = [0, 1, 60, 15];
    #[cfg(not(target_arch = "x86_64"))]
    const ALLOWED: [usize; 4] = [63, 64, 93, 139];
    ALLOWED.contains(&nr)
}

/// This architecture's AUDIT_ARCH_* value (`struct seccomp_data`'s arch).
pub const fn audit_arch() -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        0xc000_003e
    }
    #[cfg(target_arch = "aarch64")]
    {
        0xc000_00b7
    }
    #[cfg(target_arch = "riscv64")]
    {
        0xc000_00f3
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        0
    }
}

/// `struct seccomp_data` for call `nr` with `args` at `ip`.
pub fn seccomp_data(nr: usize, ip: u64, args: [u64; 6]) -> [u8; 64] {
    let mut d = [0u8; 64];
    d[0..4].copy_from_slice(&(nr as u32).to_ne_bytes());
    d[4..8].copy_from_slice(&audit_arch().to_ne_bytes());
    d[8..16].copy_from_slice(&ip.to_ne_bytes());
    for (n, a) in args.iter().enumerate() {
        d[16 + 8 * n..24 + 8 * n].copy_from_slice(&a.to_ne_bytes());
    }
    d
}

/// The actions GET_ACTION_AVAIL reports as known.
pub fn action_known(action: u32) -> bool {
    matches!(
        action,
        ret::KILL_PROCESS
            | ret::KILL_THREAD
            | ret::TRAP
            | ret::ERRNO
            | ret::USER_NOTIF
            | ret::TRACE
            | ret::LOG
            | ret::ALLOW
    )
}

/// `/proc/<pid>/status`'s Seccomp and Seccomp_filters values.
pub fn status_lines(s: &ThreadSeccomp) -> (u8, usize) {
    (s.mode(), s.head().map_or(0, |c| c.depth()))
}

/// Programs from `prev` up, newest first (tests).
#[cfg(test)]
fn programs(chain: &Arc<Filter>) -> alloc::vec::Vec<usize> {
    let mut out = alloc::vec::Vec::new();
    let mut f = Some(chain);
    while let Some(cur) = f {
        out.push(cur.prog.len());
        f = cur.prev.as_ref();
    }
    out
}

#[cfg(test)]
mod tests {
    use veridian_cbpf::{check_seccomp, op::*, Insn};

    use super::*;

    fn ret_only(v: u32) -> SeccompProgram {
        check_seccomp(&[Insn::new(RET | K, 0, 0, v)]).unwrap()
    }

    fn plain(v: u32, prev: Option<&Chain>) -> Chain {
        Filter::make(
            ret_only(v),
            prev.map(|c| c.head().clone()),
            false,
            None,
            false,
        )
        .unwrap()
    }

    /// The action of highest precedence wins across the chain, whichever
    /// program returned it.
    #[test]
    fn the_strictest_action_wins() {
        let a = plain(ret::ALLOW, None);
        let b = plain(ret::ERRNO | 1, Some(&a));
        let c = plain(ret::LOG, Some(&b));
        let d = seccomp_data(39, 0, [0; 6]);
        assert_eq!(run_chain(c.head(), &d).0, ret::ERRNO | 1);
        let k = plain(ret::KILL_PROCESS, Some(&c));
        assert_eq!(run_chain(k.head(), &d).0, ret::KILL_PROCESS);
        assert_eq!(k.head().depth(), 4);
        assert_eq!(programs(k.head()), [1, 1, 1, 1]);
        assert!(k.head().has_ancestor(Some(a.head())) && !a.head().has_ancestor(Some(k.head())));
    }

    #[test]
    fn modes_do_not_change_back() {
        let s = ThreadSeccomp::new();
        s.set_strict().unwrap();
        assert!(s.set_filter(plain(ret::ALLOW, None)).is_err());
        let t = ThreadSeccomp::new();
        t.set_filter(plain(ret::ALLOW, None)).unwrap();
        assert!(t.set_strict().is_err());
        assert_eq!(status_lines(&t.inherit()), (MODE_FILTER, 1));
    }

    #[test]
    fn a_chain_is_bounded() {
        let mut big = Vec::new();
        for _ in 0..4095 {
            big.push(Insn::new(LD | IMM, 0, 0, 0));
        }
        big.push(Insn::new(RET | K, 0, 0, ret::ALLOW));
        let mut chain: Option<Chain> = None;
        let mut n = 0;
        while let Some(f) = Filter::make(
            check_seccomp(&big).unwrap(),
            chain.as_ref().map(|c| c.head().clone()),
            false,
            None,
            false,
        ) {
            chain = Some(f);
            n += 1;
        }
        // 32768 / (4096 + 4): seven fit, the eighth does not.
        assert_eq!(n, 7);
    }

    /// The longest chain the limit allows (one-instruction programs) drops
    /// without recursing once per program.
    #[test]
    fn a_long_chain_drops_iteratively() {
        let mut chain: Option<Chain> = None;
        let mut n = 0;
        while let Some(f) = Filter::make(
            ret_only(ret::ALLOW),
            chain.as_ref().map(|c| c.head().clone()),
            false,
            None,
            false,
        ) {
            chain = Some(f);
            n += 1;
        }
        assert_eq!(n, MAX_INSNS_PER_PATH / 5);
        // A test thread's stack is small enough that a recursive drop of
        // 6,553 filters would overflow it.
        drop(chain);
    }

    /// A filter's users: the threads holding it and the filters above it;
    /// the last one gone, the filter below loses one too.
    #[test]
    fn users_follow_threads_and_children() {
        let base = plain(ret::ALLOW, None);
        let f = base.head().clone();
        let t = ThreadSeccomp::new();
        t.set_filter(base).unwrap();
        assert!(f.has_users());
        let u = t.inherit();
        let top = plain(ret::ALLOW, t.chain.lock().as_ref());
        t.set_filter(top).unwrap();
        // u still holds the base, and the top filter is a user of it.
        drop(u);
        assert!(f.has_users());
        t.release();
        assert!(!f.has_users());
    }

    /// A notification's life: received, answered, read back by the thread
    /// that made it; ID_VALID only while received and unanswered.
    #[test]
    fn a_notification_is_received_then_answered_once() {
        let mut n: Notify<u32> = Notify::new(100);
        let id = n.begin(7, [1; 64]).unwrap();
        assert_eq!(n.readiness(), (true, false));
        assert!(!n.id_valid(id));
        assert_eq!(n.send(id, 0, 0, 0), Err(errno::EINPROGRESS));
        let (rid, tid, data) = n.recv().unwrap();
        assert_eq!((rid, tid, data[0]), (id, 7, 1));
        assert!(n.recv().is_none());
        assert!(n.id_valid(id));
        assert!(matches!(n.step(id), TargetStep::Wait));
        n.send(id, -1, 0, 0).unwrap();
        assert_eq!(n.send(id, 0, 0, 0), Err(errno::EINPROGRESS));
        assert!(matches!(n.step(id), TargetStep::Replied(-1, 0, 0)));
        n.finish(id);
        assert_eq!(n.send(id, 0, 0, 0), Err(errno::ENOENT));
    }

    /// ADDFD with SEND answers the call with the new descriptor; a failed
    /// install puts the notification back to Sent.
    #[test]
    fn addfd_send_answers_with_the_descriptor() {
        let mut n: Notify<u32> = Notify::new(1);
        let id = n.begin(7, [0; 64]).unwrap();
        n.recv().unwrap();
        let req = |send| {
            Arc::new(AddFd {
                file: 5u32,
                newfd: None,
                cloexec: false,
                send,
                result: Mutex::new(None),
            })
        };
        let a = req(false);
        n.addfd(id, a.clone()).unwrap();
        let s = req(true);
        assert_eq!(n.addfd(id, s.clone()), Err(errno::EBUSY));
        let TargetStep::AddFd(got) = n.step(id) else {
            panic!("addfd expected");
        };
        n.addfd_done(id, &got, 3);
        assert_eq!(*a.result.lock(), Some(3));
        n.addfd(id, s.clone()).unwrap();
        let TargetStep::AddFd(got) = n.step(id) else {
            panic!("addfd expected");
        };
        n.addfd_done(id, &got, -24);
        assert!(matches!(n.step(id), TargetStep::Wait));
        n.addfd(id, s.clone()).unwrap();
        let TargetStep::AddFd(got) = n.step(id) else {
            panic!("addfd expected");
        };
        n.addfd_done(id, &got, 4);
        assert!(matches!(n.step(id), TargetStep::Replied(0, 4, 0)));
    }

    /// Taking back an install: queued, it goes; taken by the target and
    /// not done, it is in flight (the supervisor waits); done, its result
    /// (PR #49 review).
    #[test]
    fn a_cancelled_install_in_flight_is_waited_for() {
        let mut n: Notify<u32> = Notify::new(1);
        let id = n.begin(7, [0; 64]).unwrap();
        n.recv().unwrap();
        let req = || {
            Arc::new(AddFd {
                file: 5u32,
                newfd: None,
                cloexec: false,
                send: false,
                result: Mutex::new(None),
            })
        };
        let queued = req();
        n.addfd(id, queued.clone()).unwrap();
        assert_eq!(n.addfd_cancel(id, &queued), AddFdCancel::Removed);
        let taken = req();
        n.addfd(id, taken.clone()).unwrap();
        let TargetStep::AddFd(got) = n.step(id) else {
            panic!("addfd expected");
        };
        assert_eq!(n.addfd_cancel(id, &taken), AddFdCancel::InFlight);
        n.addfd_done(id, &got, 6);
        assert_eq!(n.addfd_cancel(id, &taken), AddFdCancel::Done(6));
    }

    /// A closed listener answers everything outstanding with ENOSYS and
    /// takes no new notifications; installs left over fail with ESRCH.
    #[test]
    fn a_closed_listener_answers_enosys() {
        let mut n: Notify<u32> = Notify::new(1);
        let a = n.begin(7, [0; 64]).unwrap();
        let b = n.begin(8, [0; 64]).unwrap();
        n.recv().unwrap();
        let req = Arc::new(AddFd {
            file: 5u32,
            newfd: None,
            cloexec: false,
            send: false,
            result: Mutex::new(None),
        });
        n.addfd(a, req.clone()).unwrap();
        n.detach();
        assert!(n.begin(9, [0; 64]).is_none());
        assert!(matches!(n.step(b), TargetStep::Replied(-38, 0, 0)));
        n.finish(a);
        assert_eq!(*req.result.lock(), Some(-(errno::ESRCH as i64)));
    }
}

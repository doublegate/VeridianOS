//! System V semaphores (`ipc/sem.c`): semget, semop, semtimedop, semctl,
//! and SEM_UNDO's adjustments, applied when the last process sharing an
//! undo list exits (CLONE_SYSVSEM shares one; a fork does not).
//!
//! A semop is all or nothing: the operations are tried in order on a copy
//! of the values, and applied only if none would block or overflow, as
//! perform_atomic_semop does. A caller that has to wait sleeps on the
//! namespace's queue and tries again after every change; it counts in
//! GETNCNT or GETZCNT for the one operation that blocked it.

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use super::{
    encode_perm, ipcget, now_secs, pid_shown, put_long, Asker, GetOutcome, IpcObject, Perm,
    SysvSpace, IPC64_PERM_SIZE, IPC_INFO, IPC_NOWAIT, IPC_RMID, IPC_SET, IPC_STAT,
};
use crate::syscall::SyscallError;

/// Linux's limits (`include/uapi/linux/sem.h`).
pub const SEMMNI: i32 = 32000;
pub const SEMMSL: i32 = 32000;
pub const SEMMNS: i32 = SEMMNI.saturating_mul(SEMMSL);
pub const SEMOPM: i32 = 500;
pub const SEMVMX: i32 = 32767;
pub const SEMAEM: i32 = SEMVMX;
const SEMUME: i32 = SEMOPM;
const SEMMNU: i32 = SEMMNS;
const SEMMAP: i32 = SEMMNS;
const SEMUSZ: i32 = 20;

/// sem_flg: undo at exit.
pub const SEM_UNDO: i16 = 0x1000;

/// semctl commands.
pub const GETPID: i32 = 11;
pub const GETVAL: i32 = 12;
pub const GETALL: i32 = 13;
pub const GETNCNT: i32 = 14;
pub const GETZCNT: i32 = 15;
pub const SETVAL: i32 = 16;
pub const SETALL: i32 = 17;
pub const SEM_STAT: i32 = 18;
pub const SEM_INFO: i32 = 19;
pub const SEM_STAT_ANY: i32 = 20;

/// One semaphore: its value and the last pid to change it (`sempid`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Sem {
    pub val: i32,
    pub pid: u64,
}

/// One operation (`struct sembuf`: num, op, flg, 6 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemBuf {
    pub num: u16,
    pub op: i16,
    pub flg: i16,
}

impl SemBuf {
    pub fn decode(b: &[u8]) -> Self {
        Self {
            num: u16::from_ne_bytes([b[0], b[1]]),
            op: i16::from_ne_bytes([b[2], b[3]]),
            flg: i16::from_ne_bytes([b[4], b[5]]),
        }
    }
}

/// A task waiting on an array: the operation that blocked it.
struct Waiting {
    token: u64,
    num: u16,
    zero: bool,
}

/// A semaphore array (`struct sem_array`).
pub struct SemArray {
    perm: Perm,
    pub sems: Vec<Sem>,
    pub otime: i64,
    pub ctime: i64,
    /// SEM_UNDO adjustments, by undo list ([`UndoList::id`]).
    adj: BTreeMap<u64, Vec<i32>>,
    waiting: Vec<Waiting>,
}

impl IpcObject for SemArray {
    fn perm(&self) -> &Perm {
        &self.perm
    }
    fn perm_mut(&mut self) -> &mut Perm {
        &mut self.perm
    }
}

/// How a semop came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    Done,
    /// Would block on operation `num`, waiting for zero or an increase.
    Block {
        num: u16,
        zero: bool,
    },
}

impl SemArray {
    /// perform_atomic_semop: try `sops` in order on a copy of the values,
    /// with the undo adjustments of list `undo` (when any operation has
    /// SEM_UNDO). A would-block with that operation's IPC_NOWAIT is EAGAIN;
    /// a value past SEMVMX, or an adjustment past SEMAEM, is ERANGE. All
    /// applied, with `pid` as sempid, or none.
    pub fn attempt(
        &mut self,
        sops: &[SemBuf],
        undo: Option<u64>,
        pid: u64,
    ) -> Result<Attempt, SyscallError> {
        let mut vals: Vec<i32> = self.sems.iter().map(|s| s.val).collect();
        let mut adj = undo.map(|u| {
            self.adj
                .get(&u)
                .cloned()
                .unwrap_or_else(|| alloc::vec![0; self.sems.len()])
        });
        for sop in sops {
            let n = sop.num as usize;
            let cur = vals[n];
            let op = i32::from(sop.op);
            let result = cur + op;
            if (op == 0 && cur != 0) || result < 0 {
                if sop.flg as i32 & IPC_NOWAIT != 0 {
                    return Err(SyscallError::WouldBlock);
                }
                return Ok(Attempt::Block {
                    num: sop.num,
                    zero: op == 0,
                });
            }
            if result > SEMVMX {
                return Err(SyscallError::RangeError);
            }
            if sop.flg & SEM_UNDO != 0 {
                if let Some(a) = adj.as_mut() {
                    let u = a[n] - op;
                    if !(-SEMAEM - 1..=SEMAEM).contains(&u) {
                        return Err(SyscallError::RangeError);
                    }
                    a[n] = u;
                }
            }
            vals[n] = result;
        }
        for sop in sops {
            let n = sop.num as usize;
            self.sems[n].val = vals[n];
            self.sems[n].pid = pid;
        }
        if let (Some(u), Some(a)) = (undo, adj) {
            self.adj.insert(u, a);
        }
        self.otime = now_secs();
        Ok(Attempt::Done)
    }

    fn count(&self, num: u16, zero: bool) -> usize {
        self.waiting
            .iter()
            .filter(|w| w.num == num && w.zero == zero)
            .count()
    }

    fn set_waiting(&mut self, token: u64, num: u16, zero: bool) {
        self.waiting.retain(|w| w.token != token);
        self.waiting.push(Waiting { token, num, zero });
    }

    fn stop_waiting(&mut self, token: u64) {
        self.waiting.retain(|w| w.token != token);
    }

    /// SETVAL and SETALL: every undo list's adjustment for semaphore
    /// `num` (or all of them) is cleared.
    fn clear_adj(&mut self, num: Option<usize>) {
        for a in self.adj.values_mut() {
            match num {
                Some(n) => a[n] = 0,
                None => a.iter_mut().for_each(|v| *v = 0),
            }
        }
    }
}

/// A process's undo list (`struct sem_undo_list`): the arrays it holds
/// adjustments in. The adjustments themselves are in the arrays, by
/// [`UndoList::id`], so SETVAL and IPC_RMID reach every list.
pub struct UndoList {
    id: u64,
    arrays: Mutex<Vec<(Weak<SysvSpace>, i32)>>,
}

static NEXT_UNDO: AtomicU64 = AtomicU64::new(1);

impl UndoList {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_UNDO.fetch_add(1, Ordering::Relaxed),
            arrays: Mutex::new(Vec::new()),
        })
    }

    fn note(&self, space: &Arc<SysvSpace>, semid: i32) {
        let mut a = self.arrays.lock();
        let w = Arc::downgrade(space);
        if !a.iter().any(|(s, id)| *id == semid && Weak::ptr_eq(s, &w)) {
            a.push((w, semid));
        }
    }

    /// exit_sem: apply this list's adjustments, each value kept in [0,
    /// SEMVMX] and its sempid set to `pid`, and wake the waiters.
    pub fn apply(&self, pid: u64) {
        let arrays = core::mem::take(&mut *self.arrays.lock());
        for (space, semid) in arrays {
            let Some(space) = space.upgrade() else {
                continue;
            };
            {
                let mut ids = space.sem.lock();
                let Ok(arr) = ids.get_mut(semid) else {
                    continue;
                };
                let Some(adj) = arr.adj.remove(&self.id) else {
                    continue;
                };
                for (n, a) in adj.into_iter().enumerate() {
                    if a != 0 {
                        let s = &mut arr.sems[n];
                        s.val = (s.val + a).clamp(0, SEMVMX);
                        s.pid = pid;
                    }
                }
                arr.otime = now_secs();
            }
            space.sem_waiters.wake_all();
        }
    }
}

impl Default for UndoList {
    fn default() -> Self {
        Self {
            id: NEXT_UNDO.fetch_add(1, Ordering::Relaxed),
            arrays: Mutex::new(Vec::new()),
        }
    }
}

/// exit_sem for `process`: it leaves its undo list, which is applied by
/// whichever sharer lets it go last. `Arc::into_inner` hands the list to
/// exactly that one, atomically: two sharers exiting at once on two CPUs
/// each saw a count of two and neither applied it (PR #40 review).
pub fn exit_sem(process: &crate::process::Process) {
    let Some(list) = process.sem_undo.lock().take() else {
        return;
    };
    if let Some(list) = Arc::into_inner(list) {
        list.apply(process.pid.0);
    }
}

/// The undo list of `process`, made on first use (find_alloc_undo).
pub fn undo_of(process: &crate::process::Process) -> Arc<UndoList> {
    process
        .sem_undo
        .lock()
        .get_or_insert_with(UndoList::new)
        .clone()
}

/// semget's checks and creation.
pub fn get(
    space: &SysvSpace,
    asker: &Asker,
    key: i32,
    nsems: i32,
    flags: i32,
) -> Result<usize, SyscallError> {
    let [semmsl, semmns, _, semmni] = space.limits.sem_limits();
    if nsems < 0 || nsems > semmsl {
        return Err(SyscallError::InvalidArgument);
    }
    let mut ids = space.sem.lock();
    match ipcget(&ids, asker, key, flags, |a: &SemArray| {
        if nsems as usize > a.sems.len() {
            Err(SyscallError::InvalidArgument)
        } else {
            Ok(())
        }
    })? {
        GetOutcome::Existing(id) => Ok(id as usize),
        GetOutcome::Create => {
            asker.may_make()?;
            if nsems == 0 {
                return Err(SyscallError::InvalidArgument);
            }
            let used: usize = ids.iter().map(|a| a.sems.len()).sum();
            if used + nsems as usize > semmns.max(0) as usize {
                return Err(SyscallError::NoSpace);
            }
            let arr = SemArray {
                perm: Perm::new(key, flags, asker.euid, asker.egid),
                sems: new_sems(nsems as usize)?,
                otime: 0,
                ctime: now_secs(),
                adj: BTreeMap::new(),
                waiting: Vec::new(),
            };
            Ok(ids.add(arr, semmni.max(0) as usize)? as usize)
        }
    }
}

static NEXT_WAITER: AtomicU64 = AtomicU64::new(1);

/// semop and semtimedop on array `id`: checks as __do_semtimedop orders
/// them (no operations or a negative id EINVAL, more than SEMOPM E2BIG,
/// then the array, a semaphore number past the array EFBIG, the access
/// EACCES), then the attempt, waiting until `deadline` (EAGAIN after it)
/// unless an operation has IPC_NOWAIT. EIDRM if the array goes meanwhile,
/// EINTR for a signal.
pub fn semop(
    space: &Arc<SysvSpace>,
    asker: &Asker,
    id: i32,
    sops: &[SemBuf],
    deadline: Option<u64>,
    undo: impl FnOnce() -> Arc<UndoList>,
) -> Result<usize, SyscallError> {
    let [_, _, semopm, _] = space.limits.sem_limits();
    if sops.is_empty() || id < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if sops.len() > semopm.max(0) as usize {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let max = sops.iter().map(|s| s.num).max().unwrap_or(0);
    let alter = sops.iter().any(|s| s.op != 0);
    let wants_undo = sops.iter().any(|s| s.flg & SEM_UNDO != 0);
    let undo = wants_undo.then(undo);
    let pid = super::current_pid();
    let first = {
        let mut ids = space.sem.lock();
        let arr = ids.get_mut(id)?;
        if max as usize >= arr.sems.len() {
            return Err(SyscallError::FileTooLarge);
        }
        if !asker.permits(&arr.perm, if alter { 0o222 } else { 0o444 }) {
            return Err(SyscallError::PermissionDenied);
        }
        arr.attempt(sops, undo.as_ref().map(|u| u.id), pid)?
    };
    if let Some(u) = &undo {
        u.note(space, id);
    }
    if first == Attempt::Done {
        if alter {
            space.sem_waiters.wake_all();
        }
        return Ok(0);
    }
    // Wait: the array must still be this one (`id`'s sequence), else EIDRM.
    let token = NEXT_WAITER.fetch_add(1, Ordering::Relaxed);
    let mut outcome: Result<Attempt, SyscallError> = Ok(first);
    if let Attempt::Block { num, zero } = first {
        if let Ok(arr) = space.sem.lock().get_mut(id) {
            arr.set_waiting(token, num, zero);
        }
    }
    let waited = crate::sched::dispatch::wait_event(&space.sem_waiters, deadline, || {
        let mut ids = space.sem.lock();
        let Ok(arr) = ids.get_mut(id) else {
            outcome = Err(SyscallError::IdRemoved);
            return true;
        };
        match arr.attempt(sops, undo.as_ref().map(|u| u.id), pid) {
            Ok(Attempt::Block { num, zero }) => {
                arr.set_waiting(token, num, zero);
                false
            }
            other => {
                arr.stop_waiting(token);
                outcome = other;
                true
            }
        }
    });
    if let Ok(arr) = space.sem.lock().get_mut(id) {
        arr.stop_waiting(token);
    }
    match waited {
        Ok(()) => match outcome {
            Ok(_) => {
                if alter {
                    space.sem_waiters.wake_all();
                }
                Ok(0)
            }
            Err(e) => Err(e),
        },
        Err(crate::sched::dispatch::WaitError::TimedOut) => Err(SyscallError::WouldBlock),
        Err(_) => Err(SyscallError::Interrupted),
    }
}

/// `struct semid64_ds`: x86_64 has a pad after each time.
#[cfg(target_arch = "x86_64")]
pub const SEMID_DS_SIZE: usize = 104;
#[cfg(not(target_arch = "x86_64"))]
pub const SEMID_DS_SIZE: usize = 88;
/// `struct seminfo`: ten ints.
pub const SEMINFO_SIZE: usize = 40;

fn encode_ds(asker: &Asker, arr: &SemArray) -> Vec<u8> {
    let mut out = alloc::vec![0u8; SEMID_DS_SIZE];
    encode_perm(&asker.shown(&arr.perm), &mut out[..IPC64_PERM_SIZE]);
    #[cfg(target_arch = "x86_64")]
    {
        put_long(&mut out, 48, arr.otime as u64);
        put_long(&mut out, 64, arr.ctime as u64);
        put_long(&mut out, 80, arr.sems.len() as u64);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        put_long(&mut out, 48, arr.otime as u64);
        put_long(&mut out, 56, arr.ctime as u64);
        put_long(&mut out, 64, arr.sems.len() as u64);
    }
    out
}

/// What semctl reads from or writes to the caller.
pub enum CtlArg {
    None,
    /// SETVAL's value (the low 32 bits of the argument).
    Val(i32),
    /// SETALL's values.
    Values(Vec<u16>),
    /// IPC_SET's uid, gid and mode.
    Set(u32, u32, u32),
}

pub enum CtlOut {
    None,
    Bytes(Vec<u8>),
    /// GETALL's values.
    Values(Vec<u16>),
}

/// The number of semaphores in array `id`, for GETALL and SETALL's buffer.
pub fn nsems(space: &SysvSpace, id: i32) -> Result<usize, SyscallError> {
    Ok(space.sem.lock().get(id)?.sems.len())
}

/// semctl.
pub fn control(
    space: &SysvSpace,
    asker: &Asker,
    id: i32,
    num: i32,
    cmd: i32,
    arg: CtlArg,
) -> Result<(usize, CtlOut), SyscallError> {
    // The commands that look without an object's own permission check
    // need READ on the namespace, as every look does (ADR 0027; PR #40
    // review).
    if matches!(cmd, IPC_INFO | SEM_INFO | SEM_STAT_ANY) && !asker.may_read {
        return Err(SyscallError::PermissionDenied);
    }
    let limits = space.limits.sem_limits();
    let mut ids = space.sem.lock();
    let r = match cmd {
        IPC_INFO | SEM_INFO => {
            let [semmsl, semmns, semopm, semmni] = limits;
            let used: usize = ids.iter().map(|a| a.sems.len()).sum();
            let info = if cmd == SEM_INFO {
                [
                    SEMMAP,
                    semmni,
                    semmns,
                    SEMMNU,
                    semmsl,
                    semopm,
                    SEMUME,
                    ids.in_use() as i32,
                    SEMVMX,
                    used as i32,
                ]
            } else {
                [
                    SEMMAP, semmni, semmns, SEMMNU, semmsl, semopm, SEMUME, SEMUSZ, SEMVMX, SEMAEM,
                ]
            };
            let mut out = alloc::vec![0u8; SEMINFO_SIZE];
            for (i, v) in info.iter().enumerate() {
                out[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
            }
            (ids.max_idx() as usize, CtlOut::Bytes(out))
        }
        IPC_STAT | SEM_STAT | SEM_STAT_ANY => {
            let arr = if cmd == IPC_STAT {
                ids.get(id)?
            } else {
                ids.by_index(id)?
            };
            if cmd != SEM_STAT_ANY && !asker.permits(&arr.perm, 0o444) {
                return Err(SyscallError::PermissionDenied);
            }
            let ret = if cmd == IPC_STAT {
                0
            } else {
                arr.perm.id as usize
            };
            (ret, CtlOut::Bytes(encode_ds(asker, arr)))
        }
        GETVAL | GETPID | GETNCNT | GETZCNT | GETALL => {
            let arr = ids.get(id)?;
            if !asker.permits(&arr.perm, 0o444) {
                return Err(SyscallError::PermissionDenied);
            }
            if cmd == GETALL {
                let v = arr.sems.iter().map(|s| s.val as u16).collect();
                (0, CtlOut::Values(v))
            } else {
                if num < 0 || num as usize >= arr.sems.len() {
                    return Err(SyscallError::InvalidArgument);
                }
                let s = arr.sems[num as usize];
                let v = match cmd {
                    GETVAL => s.val as usize,
                    GETPID => pid_shown(s.pid) as usize,
                    GETNCNT => arr.count(num as u16, false),
                    _ => arr.count(num as u16, true),
                };
                (v, CtlOut::None)
            }
        }
        SETVAL | SETALL => {
            let arr = ids.get_mut(id)?;
            if cmd == SETVAL && (num < 0 || num as usize >= arr.sems.len()) {
                return Err(SyscallError::InvalidArgument);
            }
            if !asker.permits(&arr.perm, 0o222) {
                return Err(SyscallError::PermissionDenied);
            }
            let pid = super::current_pid();
            match arg {
                CtlArg::Val(v) => {
                    if !(0..=SEMVMX).contains(&v) {
                        return Err(SyscallError::RangeError);
                    }
                    let n = num as usize;
                    arr.clear_adj(Some(n));
                    arr.sems[n] = Sem { val: v, pid };
                }
                CtlArg::Values(vs) => {
                    if vs.iter().any(|&v| i32::from(v) > SEMVMX) {
                        return Err(SyscallError::RangeError);
                    }
                    arr.clear_adj(None);
                    for (s, &v) in arr.sems.iter_mut().zip(&vs) {
                        *s = Sem {
                            val: i32::from(v),
                            pid,
                        };
                    }
                }
                _ => return Err(SyscallError::InvalidArgument),
            }
            arr.ctime = now_secs();
            drop(ids);
            space.sem_waiters.wake_all();
            return Ok((0, CtlOut::None));
        }
        IPC_RMID | IPC_SET => {
            let arr = ids.get_mut(id)?;
            if !asker.may_control(&arr.perm) {
                return Err(SyscallError::OperationNotPermitted);
            }
            if cmd == IPC_SET {
                let CtlArg::Set(uid, gid, mode) = arg else {
                    return Err(SyscallError::InvalidPointer);
                };
                asker.update(&mut arr.perm, uid, gid, mode)?;
                arr.ctime = now_secs();
                return Ok((0, CtlOut::None));
            }
            // freeary: gone at once; waiters get EIDRM, undo lists lose it.
            ids.remove(id);
            drop(ids);
            space.sem_waiters.wake_all();
            return Ok((0, CtlOut::None));
        }
        _ => return Err(SyscallError::InvalidArgument),
    };
    Ok(r)
}

/// `/proc/sysvipc/sem`'s lines (sysvipc_sem_proc_show).
pub fn proc_lines(space: &SysvSpace, asker: &Asker) -> alloc::string::String {
    use core::fmt::Write;
    let mut s = alloc::string::String::from(
        "       key      semid perms      nsems   uid   gid  cuid  cgid      otime      ctime\n",
    );
    // No READ on the namespace: the header and nothing else (ADR 0027).
    if !asker.may_read {
        return s;
    }
    for arr in space.sem.lock().iter() {
        let p = asker.shown(&arr.perm);
        let _ = writeln!(
            s,
            "{:>10} {:>10}  {:>4o} {:>10} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10}",
            p.key,
            p.id,
            p.mode,
            arr.sems.len(),
            p.uid,
            p.gid,
            p.cuid,
            p.cgid,
            arr.otime,
            arr.ctime
        );
    }
    s
}

/// `n` semaphores at 0, or ENOMEM if the heap cannot hold them (newary's
/// kvzalloc fails; a `vec!` would panic, and semmsl is the namespace
/// root's to raise).
fn new_sems(n: usize) -> Result<Vec<Sem>, SyscallError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| SyscallError::OutOfMemory)?;
    v.resize(n, Sem::default());
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::{super::tests::asker, *};

    #[test]
    fn an_array_too_big_for_the_heap_is_enomem() {
        assert_eq!(new_sems(3).map(|v| v.len()), Ok(3));
        assert_eq!(
            new_sems(usize::MAX / 2).map(|v| v.len()),
            Err(SyscallError::OutOfMemory)
        );
    }

    fn op(num: u16, op: i16, flg: i16) -> SemBuf {
        SemBuf { num, op, flg }
    }

    fn array(n: usize) -> SemArray {
        SemArray {
            perm: Perm::new(0, 0o600, 1000, 1000),
            sems: alloc::vec![Sem::default(); n],
            otime: 0,
            ctime: 0,
            adj: BTreeMap::new(),
            waiting: Vec::new(),
        }
    }

    #[test]
    fn a_semop_is_all_or_nothing() {
        let mut a = array(2);
        a.sems[0].val = 1;
        // The second would block: neither applies.
        assert_eq!(
            a.attempt(&[op(0, -1, 0), op(1, -1, 0)], None, 5),
            Ok(Attempt::Block {
                num: 1,
                zero: false
            })
        );
        assert_eq!(a.sems[0].val, 1);
        assert_eq!(
            a.attempt(&[op(0, -1, 0), op(1, -1, IPC_NOWAIT as i16)], None, 5),
            Err(SyscallError::WouldBlock)
        );
        assert_eq!(
            a.attempt(&[op(0, -1, 0), op(1, 2, 0)], None, 5),
            Ok(Attempt::Done)
        );
        assert_eq!((a.sems[0].val, a.sems[1].val, a.sems[1].pid), (0, 2, 5));
        // Waiting for zero, and an operation on the same semaphore twice.
        assert_eq!(
            a.attempt(&[op(1, 0, 0)], None, 5),
            Ok(Attempt::Block { num: 1, zero: true })
        );
        assert_eq!(
            a.attempt(&[op(1, -2, 0), op(1, 0, 0)], None, 5),
            Ok(Attempt::Done)
        );
        assert_eq!(
            a.attempt(&[op(0, SEMVMX as i16, 0), op(0, 1, 0)], None, 5),
            Err(SyscallError::RangeError)
        );
    }

    #[test]
    fn undo_adjustments_are_kept_and_bounded() {
        let mut a = array(1);
        assert_eq!(
            a.attempt(&[op(0, 3, SEM_UNDO)], Some(7), 1),
            Ok(Attempt::Done)
        );
        assert_eq!(a.adj.get(&7), Some(&alloc::vec![-3]));
        a.clear_adj(Some(0));
        assert_eq!(a.adj.get(&7), Some(&alloc::vec![0]));
        a.adj.insert(7, alloc::vec![-SEMAEM - 1]);
        assert_eq!(
            a.attempt(&[op(0, 1, SEM_UNDO)], Some(7), 1),
            Err(SyscallError::RangeError)
        );
    }

    #[test]
    fn exit_applies_the_undo_list_clamped() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 1, 0o600).unwrap() as i32;
        let list = UndoList::new();
        semop(&space, &me, id, &[op(0, 5, SEM_UNDO)], None, || {
            list.clone()
        })
        .unwrap();
        assert_eq!(space.sem.lock().get(id).unwrap().sems[0].val, 5);
        // Someone else takes 4 meanwhile: the undo of -5 stops at 0.
        semop(&space, &me, id, &[op(0, -4, 0)], None, UndoList::new).unwrap();
        list.apply(9);
        let s = space.sem.lock().get(id).unwrap().sems[0];
        assert_eq!((s.val, s.pid), (0, 9));
    }

    #[test]
    fn semget_and_semop_check_in_linux_order() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        assert_eq!(
            get(&space, &me, 0, -1, 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(
            get(&space, &me, 0, 0, 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        let id = get(&space, &me, 3, 2, 0o600 | super::super::IPC_CREAT).unwrap() as i32;
        assert_eq!(
            get(&space, &me, 3, 3, 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(get(&space, &me, 3, 1, 0), Ok(id as usize));
        assert_eq!(
            semop(&space, &me, id, &[], None, UndoList::new).err(),
            Some(SyscallError::InvalidArgument)
        );
        let many = alloc::vec![op(0, 0, 0); SEMOPM as usize + 1];
        assert_eq!(
            semop(&space, &me, id, &many, None, UndoList::new).err(),
            Some(SyscallError::ArgumentListTooLong)
        );
        assert_eq!(
            semop(&space, &me, id, &[op(2, 1, 0)], None, UndoList::new).err(),
            Some(SyscallError::FileTooLarge)
        );
        assert_eq!(
            semop(
                &space,
                &asker(2000, 2000),
                id,
                &[op(0, 1, 0)],
                None,
                UndoList::new
            )
            .err(),
            Some(SyscallError::PermissionDenied)
        );
    }

    #[test]
    fn semctl_values_and_counts() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 2, 0o600).unwrap() as i32;
        assert!(control(&space, &me, id, 1, SETVAL, CtlArg::Val(4)).is_ok());
        assert!(matches!(
            control(&space, &me, id, 1, GETVAL, CtlArg::None),
            Ok((4, CtlOut::None))
        ));
        assert_eq!(
            control(&space, &me, id, 1, SETVAL, CtlArg::Val(SEMVMX + 1)).err(),
            Some(SyscallError::RangeError)
        );
        assert_eq!(
            control(&space, &me, id, 2, GETVAL, CtlArg::None).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert!(control(
            &space,
            &me,
            id,
            0,
            SETALL,
            CtlArg::Values(alloc::vec![7, 8])
        )
        .is_ok());
        match control(&space, &me, id, 0, GETALL, CtlArg::None) {
            Ok((0, CtlOut::Values(v))) => assert_eq!(v, alloc::vec![7, 8]),
            _ => panic!("GETALL"),
        }
        space
            .sem
            .lock()
            .get_mut(id)
            .unwrap()
            .set_waiting(1, 0, true);
        assert!(matches!(
            control(&space, &me, id, 0, GETZCNT, CtlArg::None),
            Ok((1, _))
        ));
        assert!(matches!(
            control(&space, &me, id, 0, GETNCNT, CtlArg::None),
            Ok((0, _))
        ));
        assert!(control(&space, &me, id, 0, IPC_RMID, CtlArg::None).is_ok());
        assert_eq!(
            control(&space, &me, id, 0, GETVAL, CtlArg::None).err(),
            Some(SyscallError::InvalidArgument)
        );
    }
}

//! Scheduling system calls (N-221): sched_setscheduler/getscheduler,
//! sched_setparam/getparam, sched_setattr/getattr, sched_get_priority_max/
//! min, sched_rr_get_interval, sched_setaffinity/getaffinity, and
//! getpriority/setpriority.
//!
//! As on Linux these name a thread (pid 0: the caller), whose parameters are
//! kept in `Thread::sched`; the dispatcher runs the thread's task with them
//! (`dispatch::set_policy`). There are no capabilities yet, so "privileged"
//! is root. An unprivileged caller may lower a nice value only as far as
//! the target process's RLIMIT_NICE allows, take a real-time policy or
//! priority only up to its RLIMIT_RTPRIO (both 0 by default, as on Linux),
//! never take SCHED_DEADLINE, leave SCHED_IDLE only if RLIMIT_NICE allows
//! its current nice value, never clear SCHED_RESET_ON_FORK, and change only
//! threads of its own user (EPERM). SCHED_DEADLINE goes through admission
//! control (95% of each CPU; EBUSY past it).

use alloc::{sync::Arc, vec::Vec};

use spin::Mutex;

use super::{userspace, SyscallError, SyscallResult};
use crate::{
    process::{
        creds::Credentials,
        thread::{
            SchedParams, Thread, SCHED_BATCH, SCHED_DEADLINE, SCHED_FIFO, SCHED_IDLE, SCHED_NORMAL,
            SCHED_RR,
        },
    },
    sched::policy::{dl, fair},
};

/// Or'ed into sched_setscheduler's policy, reported by getscheduler.
pub const SCHED_RESET_ON_FORK: u32 = 0x4000_0000;

const SCHED_FLAG_RESET_ON_FORK: u64 = 0x01;
const SCHED_FLAG_RECLAIM: u64 = 0x02;
const SCHED_FLAG_DL_OVERRUN: u64 = 0x04;
const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP: u64 = 0x20 | 0x40;
const SCHED_FLAG_ALL: u64 = 0x7f;

/// `struct sched_attr` sizes: the first published, and with util clamps.
const SCHED_ATTR_SIZE_VER0: usize = 48;
const SCHED_ATTR_SIZE_VER1: usize = 56;

/// SCHED_RR's time slice (Linux's sched_rr_timeslice_ms default).
const RR_TIMESLICE_NS: u64 = 100_000_000;
/// SCHED_DEADLINE period bounds (Linux's sched_deadline_period_{min,max}_us).
const DL_PERIOD_MIN_NS: u64 = 100_000;
const DL_PERIOD_MAX_NS: u64 = (1 << 22) * 1000;

const PRIO_PROCESS: usize = 0;
const PRIO_PGRP: usize = 1;
const PRIO_USER: usize = 2;

/// A parameter change as a caller asks for it.
#[derive(Debug, Clone, Copy)]
struct Request {
    policy: u32,
    priority: u32,
    /// None: keep the thread's (sched_setscheduler, sched_setparam).
    nice: Option<i32>,
    /// (runtime, deadline, period) for SCHED_DEADLINE.
    dl: Option<(u64, u64, u64)>,
    reset_on_fork: bool,
}

/// What the target process's resource limits allow an unprivileged caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Allowance {
    /// The lowest nice value RLIMIT_NICE permits (`rlimit::nice_floor`).
    nice_floor: i32,
    /// RLIMIT_RTPRIO: the highest real-time priority.
    rtprio: u64,
}

impl Allowance {
    /// Linux's defaults: no lowering of nice, no real-time priority.
    #[cfg(test)]
    const NONE: Self = Self {
        nice_floor: 20,
        rtprio: 0,
    };

    /// The allowance of process `pid`'s limits (the defaults if it is gone).
    fn of(pid: u64) -> Self {
        let limits = crate::process::find_process(crate::process::ProcessId(pid))
            .map(|p| p.limits())
            .unwrap_or_default();
        Self {
            nice_floor: crate::process::rlimit::nice_floor(&limits),
            rtprio: limits.cur(crate::process::rlimit::RLIMIT_RTPRIO),
        }
    }

    /// Linux's can_nice.
    fn can_nice(&self, nice: i8) -> bool {
        nice as i32 >= self.nice_floor
    }
}

/// Validate `req` against the thread's current parameters `cur`, the
/// caller's privilege and what the target's limits allow, as Linux's
/// __sched_setscheduler; the new parameters.
fn decide(
    cur: &SchedParams,
    req: &Request,
    privileged: bool,
    allow: Allowance,
) -> Result<SchedParams, SyscallError> {
    let rt = matches!(req.policy, SCHED_FIFO | SCHED_RR);
    if !matches!(
        req.policy,
        SCHED_NORMAL | SCHED_FIFO | SCHED_RR | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE
    ) || req.priority > 99
        || rt != (req.priority != 0)
    {
        return Err(SyscallError::InvalidArgument);
    }
    let deadline = match (req.policy, req.dl) {
        (SCHED_DEADLINE, Some((runtime, deadline, period))) => {
            let period = if period == 0 { deadline } else { period };
            if deadline == 0 || !(DL_PERIOD_MIN_NS..=DL_PERIOD_MAX_NS).contains(&period) {
                return Err(SyscallError::InvalidArgument);
            }
            Some(
                dl::DlEntity::new(runtime, deadline, period)
                    .map_err(|_| SyscallError::InvalidArgument)?,
            )
        }
        (SCHED_DEADLINE, None) => return Err(SyscallError::InvalidArgument),
        _ => None,
    };
    let nice = req.nice.map_or(cur.nice, |n| n.clamp(-20, 19) as i8);
    if !privileged {
        let fair = matches!(req.policy, SCHED_NORMAL | SCHED_BATCH | SCHED_IDLE);
        let perm = SyscallError::OperationNotPermitted;
        if fair && nice < cur.nice && !allow.can_nice(nice) {
            return Err(perm);
        }
        if rt {
            if req.policy != cur.policy && allow.rtprio == 0 {
                return Err(perm);
            }
            if req.priority > cur.rt_priority as u32 && req.priority as u64 > allow.rtprio {
                return Err(perm);
            }
        }
        if req.policy == SCHED_DEADLINE {
            return Err(perm);
        }
        if cur.policy == SCHED_IDLE && req.policy != SCHED_IDLE && !allow.can_nice(cur.nice) {
            return Err(perm);
        }
        if cur.reset_on_fork && !req.reset_on_fork {
            return Err(perm);
        }
    }
    Ok(SchedParams {
        policy: req.policy,
        rt_priority: req.priority as u8,
        nice,
        deadline,
        reset_on_fork: req.reset_on_fork,
    })
}

/// Deadline bandwidth reserved by all SCHED_DEADLINE threads.
static DL_BANDWIDTH: Mutex<u64> = Mutex::new(0);

/// Move a thread's reservation from `old` to `new` if admission allows.
fn reserve_deadline(old: &SchedParams, new: &SchedParams) -> Result<(), SyscallError> {
    let bw = |p: &SchedParams| p.deadline.map_or(0, |d| d.bandwidth());
    let mut total = DL_BANDWIDTH.lock();
    let rest = total.saturating_sub(bw(old));
    let cpus = crate::sched::dispatch::online_cpus() as u32;
    if new.deadline.is_some() && !dl::admit(rest, bw(new), cpus) {
        return Err(SyscallError::Busy);
    }
    *total = rest + bw(new);
    Ok(())
}

/// A thread ends: give back its deadline reservation.
pub(crate) fn thread_exit(thread: &Thread) {
    let mut params = thread.sched.lock();
    if params.deadline.is_some() {
        let _ = reserve_deadline(&params, &SchedParams::default());
        params.deadline = None;
    }
}

/// The thread a scheduling call names (pid 0: the caller) with its process
/// ID and owner credentials. `negative` is the error for pid < 0 (EINVAL
/// for most calls, ESRCH for the affinity ones).
fn target(
    pid: usize,
    negative: SyscallError,
) -> Result<(u64, Credentials, Arc<Thread>), SyscallError> {
    let tid = pid as u32 as i32;
    if tid < 0 {
        return Err(negative);
    }
    if tid == 0 {
        let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
        let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
        return Ok((process.pid.0, process.credentials(), thread));
    }
    let mut found = None;
    crate::process::table::PROCESS_TABLE.for_each(|p| {
        if found.is_none() {
            if let Some(t) = p.get_thread(crate::process::ThreadId(tid as u64)) {
                found = Some((p.pid.0, p.credentials(), t));
            }
        }
    });
    found.ok_or(SyscallError::ProcessNotFound)
}

/// The caller's credentials.
fn caller() -> Result<Credentials, SyscallError> {
    Ok(crate::process::current_process()
        .ok_or(SyscallError::InvalidState)?
        .credentials())
}

/// Linux's check_same_owner: the caller's effective UID is the target's
/// real or effective one, or the caller is root.
fn same_owner(caller: &Credentials, target: &Credentials) -> bool {
    caller.euid == 0 || caller.euid == target.euid || caller.euid == target.ruid
}

/// Apply a request to the thread `pid` names.
fn set(
    pid: usize,
    req: impl FnOnce(&SchedParams) -> Result<Request, SyscallError>,
) -> SyscallResult {
    let (owner_pid, owner, thread) = target(pid, SyscallError::InvalidArgument)?;
    let me = caller()?;
    let mut params = thread.sched.lock();
    let request = req(&params)?;
    let new = decide(&params, &request, me.euid == 0, Allowance::of(owner_pid))?;
    if !same_owner(&me, &owner) {
        return Err(SyscallError::OperationNotPermitted);
    }
    reserve_deadline(&params, &new)?;
    *params = new;
    drop(params);
    crate::sched::dispatch::set_policy((owner_pid, thread.tid.0), new.to_policy());
    Ok(0)
}

/// sched_setscheduler(pid, policy, param).
pub fn sys_sched_setscheduler(pid: usize, policy: usize, param: usize) -> SyscallResult {
    let policy = policy as u32;
    if (policy as i32) < 0 || param == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let priority: i32 = userspace::read_user(param)?;
    // SCHED_DEADLINE needs sched_setattr's parameters: decide() refuses it
    // here (EINVAL), as Linux.
    set(pid, |_| {
        Ok(Request {
            policy: policy & !SCHED_RESET_ON_FORK,
            priority: priority as u32,
            nice: None,
            dl: None,
            reset_on_fork: policy & SCHED_RESET_ON_FORK != 0,
        })
    })
}

/// sched_setparam(pid, param): the priority, keeping the policy.
pub fn sys_sched_setparam(pid: usize, param: usize) -> SyscallResult {
    if param == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let priority: i32 = userspace::read_user(param)?;
    set(pid, |cur| {
        Ok(Request {
            policy: cur.policy,
            priority: priority as u32,
            nice: None,
            dl: cur.deadline.map(|d| (d.runtime, d.rel_deadline, d.period)),
            reset_on_fork: cur.reset_on_fork,
        })
    })
}

/// `struct sched_attr` as read from user memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SchedAttr {
    policy: u32,
    flags: u64,
    nice: i32,
    priority: u32,
    runtime: u64,
    deadline: u64,
    period: u64,
}

impl SchedAttr {
    fn parse(raw: &[u8; SCHED_ATTR_SIZE_VER1]) -> Self {
        let u32_at = |o: usize| u32::from_ne_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]);
        let u64_at = |o: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&raw[o..o + 8]);
            u64::from_ne_bytes(b)
        };
        Self {
            policy: u32_at(4),
            flags: u64_at(8),
            nice: u32_at(16) as i32,
            priority: u32_at(20),
            runtime: u64_at(24),
            deadline: u64_at(32),
            period: u64_at(40),
        }
    }

    fn of(params: &SchedParams, size: u32) -> [u8; SCHED_ATTR_SIZE_VER1] {
        let mut raw = [0u8; SCHED_ATTR_SIZE_VER1];
        let flags = if params.reset_on_fork {
            SCHED_FLAG_RESET_ON_FORK
        } else {
            0
        };
        let (runtime, deadline, period) = params
            .deadline
            .map_or((0, 0, 0), |d| (d.runtime, d.rel_deadline, d.period));
        raw[0..4].copy_from_slice(&size.to_ne_bytes());
        raw[4..8].copy_from_slice(&params.policy.to_ne_bytes());
        raw[8..16].copy_from_slice(&flags.to_ne_bytes());
        raw[16..20].copy_from_slice(&(params.nice as i32).to_ne_bytes());
        raw[20..24].copy_from_slice(&(params.rt_priority as u32).to_ne_bytes());
        raw[24..32].copy_from_slice(&runtime.to_ne_bytes());
        raw[32..40].copy_from_slice(&deadline.to_ne_bytes());
        raw[40..48].copy_from_slice(&period.to_ne_bytes());
        // Utilization clamps are not supported: the full range.
        raw[52..56].copy_from_slice(&1024u32.to_ne_bytes());
        raw
    }
}

/// sched_setattr(pid, attr, flags).
pub fn sys_sched_setattr(pid: usize, attr_ptr: usize, flags: usize) -> SyscallResult {
    if attr_ptr == 0 || flags != 0 || (pid as u32 as i32) < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // Linux's sched_copy_attr: size 0 means the first version; too small or
    // too large is E2BIG (with the kernel's size written back); a larger
    // struct is accepted if its extra bytes are zero.
    let mut size = userspace::read_user::<u32>(attr_ptr)? as usize;
    if size == 0 {
        size = SCHED_ATTR_SIZE_VER0;
    }
    if !(SCHED_ATTR_SIZE_VER0..=4096).contains(&size) {
        let _ = userspace::write_user(attr_ptr, SCHED_ATTR_SIZE_VER1 as u32);
        return Err(SyscallError::ArgumentListTooLong);
    }
    let mut raw = [0u8; SCHED_ATTR_SIZE_VER1];
    let known = size.min(SCHED_ATTR_SIZE_VER1);
    userspace::read_user_bytes(attr_ptr, &mut raw[..known])?;
    if size > SCHED_ATTR_SIZE_VER1 {
        let mut extra = alloc::vec![0u8; size - SCHED_ATTR_SIZE_VER1];
        userspace::read_user_bytes(attr_ptr + SCHED_ATTR_SIZE_VER1, &mut extra)?;
        if extra.iter().any(|&b| b != 0) {
            let _ = userspace::write_user(attr_ptr, SCHED_ATTR_SIZE_VER1 as u32);
            return Err(SyscallError::ArgumentListTooLong);
        }
    }
    let attr = SchedAttr::parse(&raw);
    if attr.flags & !SCHED_FLAG_ALL != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if attr.flags & SCHED_FLAG_UTIL_CLAMP != 0 && size < SCHED_ATTR_SIZE_VER1 {
        return Err(SyscallError::InvalidArgument);
    }
    // Utilization clamping, bandwidth reclaiming and overrun signals are
    // not implemented (Linux without CONFIG_UCLAMP_TASK answers the same
    // for clamps).
    if attr.flags & (SCHED_FLAG_UTIL_CLAMP | SCHED_FLAG_RECLAIM | SCHED_FLAG_DL_OVERRUN) != 0 {
        return Err(SyscallError::NotSupported);
    }
    set(pid, |cur| {
        let policy = if attr.flags & SCHED_FLAG_KEEP_POLICY != 0 {
            cur.policy
        } else {
            attr.policy
        };
        let keep = attr.flags & SCHED_FLAG_KEEP_PARAMS != 0;
        Ok(Request {
            policy,
            priority: if keep {
                cur.rt_priority as u32
            } else {
                attr.priority
            },
            nice: if keep { None } else { Some(attr.nice) },
            dl: if keep {
                cur.deadline.map(|d| (d.runtime, d.rel_deadline, d.period))
            } else {
                (policy == SCHED_DEADLINE).then_some((attr.runtime, attr.deadline, attr.period))
            },
            reset_on_fork: attr.flags & SCHED_FLAG_RESET_ON_FORK != 0,
        })
    })
}

/// sched_getscheduler(pid).
pub fn sys_sched_getscheduler(pid: usize) -> SyscallResult {
    let (_, _, thread) = target(pid, SyscallError::InvalidArgument)?;
    let p = *thread.sched.lock();
    let reset = if p.reset_on_fork {
        SCHED_RESET_ON_FORK
    } else {
        0
    };
    Ok((p.policy | reset) as usize)
}

/// sched_getparam(pid, param).
pub fn sys_sched_getparam(pid: usize, param: usize) -> SyscallResult {
    if param == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (_, _, thread) = target(pid, SyscallError::InvalidArgument)?;
    let prio = thread.sched.lock().rt_priority as i32;
    userspace::write_user(param, prio)?;
    Ok(0)
}

/// sched_getattr(pid, attr, size, flags).
pub fn sys_sched_getattr(pid: usize, attr_ptr: usize, size: usize, flags: usize) -> SyscallResult {
    if attr_ptr == 0 || flags != 0 || !(SCHED_ATTR_SIZE_VER0..=4096).contains(&size) {
        return Err(SyscallError::InvalidArgument);
    }
    let (_, _, thread) = target(pid, SyscallError::InvalidArgument)?;
    let params = *thread.sched.lock();
    let len = size.min(SCHED_ATTR_SIZE_VER1);
    let raw = SchedAttr::of(&params, len as u32);
    userspace::write_user_bytes(attr_ptr, &raw[..len])?;
    Ok(0)
}

/// sched_get_priority_max(policy) / sched_get_priority_min(policy).
fn priority_range(policy: usize) -> Result<(usize, usize), SyscallError> {
    match policy as u32 {
        SCHED_FIFO | SCHED_RR => Ok((1, 99)),
        SCHED_NORMAL | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE => Ok((0, 0)),
        _ => Err(SyscallError::InvalidArgument),
    }
}

pub fn sys_sched_get_priority_max(policy: usize) -> SyscallResult {
    priority_range(policy).map(|(_, max)| max)
}

pub fn sys_sched_get_priority_min(policy: usize) -> SyscallResult {
    priority_range(policy).map(|(min, _)| min)
}

/// sched_rr_get_interval(pid, tp): the thread's time slice -- SCHED_RR's
/// quantum, the fair classes' base slice, 0 for FIFO and deadline.
pub fn sys_sched_rr_get_interval(pid: usize, tp: usize) -> SyscallResult {
    let (_, _, thread) = target(pid, SyscallError::InvalidArgument)?;
    let ns = match thread.sched.lock().policy {
        SCHED_RR => RR_TIMESLICE_NS,
        SCHED_FIFO | SCHED_DEADLINE => 0,
        _ => fair::BASE_SLICE_NS,
    };
    let ts = [(ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as i64];
    userspace::write_user(tp, ts)?;
    Ok(0)
}

/// The kernel's CPU mask size in bytes (whole longs, as Linux's
/// cpumask_size()).
fn cpumask_bytes() -> usize {
    crate::sched::dispatch::online_cpus().div_ceil(64) * 8
}

/// The online CPUs as a mask.
fn online_mask() -> u64 {
    let n = crate::sched::dispatch::online_cpus().min(64);
    if n == 64 {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}

/// Linux's sched_getaffinity length rule: the buffer holds at least one
/// bit per CPU and is whole longs. Compared in bytes: `len * 8` overflows
/// for a huge `len` (a debug-build panic any caller could trigger; security
/// review of the N-221 change).
fn affinity_len_valid(len: usize, cpus: usize) -> bool {
    len >= cpus.div_ceil(8) && len.is_multiple_of(8)
}

/// sched_getaffinity(pid, len, mask): the bytes written (the kernel's mask
/// size), as the raw system call returns.
pub fn sys_sched_getaffinity(pid: usize, len: usize, mask: usize) -> SyscallResult {
    if !affinity_len_valid(len, crate::sched::dispatch::online_cpus()) {
        return Err(SyscallError::InvalidArgument);
    }
    let (_, _, thread) = target(pid, SyscallError::ProcessNotFound)?;
    let allowed = thread.get_affinity() as u64 & online_mask();
    let n = len.min(cpumask_bytes());
    let mut bytes = Vec::with_capacity(n);
    bytes.extend_from_slice(&allowed.to_ne_bytes()[..n.min(8)]);
    bytes.resize(n, 0);
    userspace::write_user_bytes(mask, &bytes)?;
    Ok(n)
}

/// sched_setaffinity(pid, len, mask): the CPUs the thread may run on, of
/// those online (EINVAL if none).
pub fn sys_sched_setaffinity(pid: usize, len: usize, mask: usize) -> SyscallResult {
    let (_, owner, thread) = target(pid, SyscallError::ProcessNotFound)?;
    let mut raw = [0u8; 8];
    let n = len.min(8);
    userspace::read_user_bytes(mask, &mut raw[..n])?;
    let wanted = u64::from_ne_bytes(raw) & online_mask();
    if !same_owner(&caller()?, &owner) {
        return Err(SyscallError::OperationNotPermitted);
    }
    if wanted == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // Only the boot CPU dispatches until SMP (sprint D5): every allowed
    // mask contains it, so the thread need not move.
    thread.set_affinity(wanted as usize);
    Ok(0)
}

/// The threads getpriority/setpriority name, with their process IDs and
/// owners: one thread (PRIO_PROCESS), every thread of a process group
/// (PRIO_PGRP) or of a user (PRIO_USER); `who` 0 is the caller's.
fn priority_targets(
    which: usize,
    who: usize,
) -> Result<Vec<(u64, Credentials, Arc<Thread>)>, SyscallError> {
    let who = who as u32;
    match which {
        PRIO_PROCESS => Ok(alloc::vec![target(
            who as usize,
            SyscallError::ProcessNotFound
        )?]),
        PRIO_PGRP | PRIO_USER => {
            let me = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
            let key = match (which, who) {
                (PRIO_PGRP, 0) => me.pgid.load(core::sync::atomic::Ordering::Acquire),
                (PRIO_USER, 0) => me.credentials().ruid as u64,
                (_, w) => w as u64,
            };
            let mut out = Vec::new();
            crate::process::table::PROCESS_TABLE.for_each(|p| {
                let creds = p.credentials();
                let hit = if which == PRIO_PGRP {
                    p.pgid.load(core::sync::atomic::Ordering::Acquire) == key
                } else {
                    creds.ruid as u64 == key
                };
                if hit {
                    for t in p.threads.lock().values() {
                        out.push((p.pid.0, creds, t.clone()));
                    }
                }
            });
            Ok(out)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// getpriority(which, who): 20 - the lowest nice value among the threads
/// named (the raw system call's encoding; the C library returns the nice).
pub fn sys_getpriority(which: usize, who: usize) -> SyscallResult {
    let targets = priority_targets(which, who)?;
    let best = targets
        .iter()
        .map(|(_, _, t)| t.sched.lock().nice)
        .min()
        .ok_or(SyscallError::ProcessNotFound)?;
    Ok((20 - best as i32) as usize)
}

/// setpriority(which, who, nice): the nice value of every thread named.
/// EPERM for another user's thread, EACCES for lowering it unprivileged
/// past what RLIMIT_NICE allows.
pub fn sys_setpriority(which: usize, who: usize, nice: usize) -> SyscallResult {
    let nice = (nice as u32 as i32).clamp(-20, 19) as i8;
    let targets = priority_targets(which, who)?;
    if targets.is_empty() {
        return Err(SyscallError::ProcessNotFound);
    }
    let me = caller()?;
    let mut result = Ok(0);
    for (pid, owner, thread) in targets {
        if !same_owner(&me, &owner) {
            result = Err(SyscallError::OperationNotPermitted);
            continue;
        }
        let allow = Allowance::of(pid);
        let mut params = thread.sched.lock();
        if nice < params.nice && me.euid != 0 && !allow.can_nice(nice) {
            result = Err(SyscallError::PermissionDenied);
            continue;
        }
        params.nice = nice;
        let policy = params.to_policy();
        drop(params);
        crate::sched::dispatch::set_policy((pid, thread.tid.0), policy);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(policy: u32, priority: u32) -> Request {
        Request {
            policy,
            priority,
            nice: None,
            dl: None,
            reset_on_fork: false,
        }
    }

    #[test]
    fn policies_and_priorities_are_validated_like_linux() {
        let cur = SchedParams::default();
        let ok = |r: Request| decide(&cur, &r, true, Allowance::NONE);
        assert_eq!(ok(req(SCHED_FIFO, 50)).unwrap().rt_priority, 50);
        assert_eq!(ok(req(SCHED_RR, 99)).unwrap().policy, SCHED_RR);
        assert_eq!(ok(req(SCHED_FIFO, 0)), Err(SyscallError::InvalidArgument));
        assert_eq!(ok(req(SCHED_FIFO, 100)), Err(SyscallError::InvalidArgument));
        assert_eq!(ok(req(SCHED_NORMAL, 1)), Err(SyscallError::InvalidArgument));
        assert_eq!(ok(req(4, 0)), Err(SyscallError::InvalidArgument));
        assert_eq!(ok(req(7, 0)), Err(SyscallError::InvalidArgument));
        assert_eq!(
            ok(req(SCHED_DEADLINE, 0)),
            Err(SyscallError::InvalidArgument)
        );
        let mut dl = req(SCHED_DEADLINE, 0);
        dl.dl = Some((1_000_000, 10_000_000, 0));
        let p = ok(dl).unwrap();
        assert_eq!(p.deadline.unwrap().period, 10_000_000);
        dl.dl = Some((1_000_000, 10_000_000, 50_000)); // period below 100 us
        assert_eq!(ok(dl), Err(SyscallError::InvalidArgument));
        dl.dl = Some((20_000_000, 10_000_000, 0)); // runtime > deadline
        assert_eq!(ok(dl), Err(SyscallError::InvalidArgument));
        // setattr's nice is clamped; setscheduler keeps the current one.
        let mut n = req(SCHED_BATCH, 0);
        n.nice = Some(40);
        assert_eq!(ok(n).unwrap().nice, 19);
        let niced = SchedParams { nice: 7, ..cur };
        assert_eq!(
            decide(&niced, &req(SCHED_NORMAL, 0), true, Allowance::NONE)
                .unwrap()
                .nice,
            7
        );
    }

    #[test]
    fn unprivileged_callers_get_eperm_where_linux_does() {
        let cur = SchedParams {
            nice: 5,
            ..SchedParams::default()
        };
        let user = |c: &SchedParams, r: Request| decide(c, &r, false, Allowance::NONE);
        let perm = Err(SyscallError::OperationNotPermitted);
        assert_eq!(user(&cur, req(SCHED_FIFO, 1)).map(|_| ()), perm);
        let mut lower = req(SCHED_NORMAL, 0);
        lower.nice = Some(0);
        assert_eq!(user(&cur, lower).map(|_| ()), perm);
        let mut higher = lower;
        higher.nice = Some(10);
        assert_eq!(user(&cur, higher).unwrap().nice, 10);
        assert!(user(&cur, req(SCHED_IDLE, 0)).is_ok());
        let idle = SchedParams {
            policy: SCHED_IDLE,
            ..cur
        };
        assert_eq!(user(&idle, req(SCHED_NORMAL, 0)).map(|_| ()), perm);
        let mut d = req(SCHED_DEADLINE, 0);
        d.dl = Some((1_000_000, 10_000_000, 0));
        assert_eq!(user(&cur, d).map(|_| ()), perm);
        // An RT thread may lower its own priority, not raise it.
        let rt = SchedParams {
            policy: SCHED_FIFO,
            rt_priority: 10,
            ..cur
        };
        assert!(user(&rt, req(SCHED_FIFO, 5)).is_ok());
        assert_eq!(user(&rt, req(SCHED_FIFO, 20)).map(|_| ()), perm);
        // Reset-on-fork cannot be cleared unprivileged.
        let reset = SchedParams {
            reset_on_fork: true,
            ..cur
        };
        assert_eq!(user(&reset, req(SCHED_NORMAL, 0)).map(|_| ()), perm);
    }

    #[test]
    fn resource_limits_widen_what_unprivileged_callers_may_do() {
        let cur = SchedParams {
            nice: 5,
            ..SchedParams::default()
        };
        // RLIMIT_NICE 30: nice down to -10, not past it.
        let nice = Allowance {
            nice_floor: -10,
            rtprio: 0,
        };
        let perm = Err(SyscallError::OperationNotPermitted);
        let to = |n: i32| {
            let mut r = req(SCHED_NORMAL, 0);
            r.nice = Some(n);
            decide(&cur, &r, false, nice).map(|p| p.nice)
        };
        assert_eq!(to(-10), Ok(-10));
        assert_eq!(to(-11), perm);
        // ... and leaving SCHED_IDLE at an allowed nice value.
        let idle = SchedParams {
            policy: SCHED_IDLE,
            ..cur
        };
        assert!(decide(&idle, &req(SCHED_NORMAL, 0), false, nice).is_ok());
        // RLIMIT_RTPRIO 20: a real-time policy up to priority 20.
        let rt = Allowance {
            nice_floor: 20,
            rtprio: 20,
        };
        assert_eq!(
            decide(&cur, &req(SCHED_FIFO, 20), false, rt).map(|p| p.rt_priority),
            Ok(20)
        );
        assert_eq!(
            decide(&cur, &req(SCHED_FIFO, 21), false, rt).map(|_| ()),
            Err(SyscallError::OperationNotPermitted)
        );
        // A thread already above the limit may stay there.
        let high = SchedParams {
            policy: SCHED_RR,
            rt_priority: 50,
            ..cur
        };
        assert!(decide(&high, &req(SCHED_RR, 40), false, rt).is_ok());
    }

    #[test]
    fn children_inherit_with_reset_on_fork() {
        let rt = SchedParams {
            policy: SCHED_RR,
            rt_priority: 30,
            nice: -5,
            deadline: None,
            reset_on_fork: true,
        };
        let c = rt.for_child();
        assert_eq!(
            (c.policy, c.rt_priority, c.nice, c.reset_on_fork),
            (SCHED_NORMAL, 0, 0, false)
        );
        let plain = SchedParams {
            reset_on_fork: false,
            ..rt
        };
        assert_eq!(plain.for_child(), plain);
    }

    #[test]
    fn deadline_admission_is_bounded() {
        let mut a = SchedParams {
            policy: SCHED_DEADLINE,
            deadline: Some(dl::DlEntity::new(6_000_000, 10_000_000, 10_000_000).unwrap()),
            ..SchedParams::default()
        };
        let none = SchedParams::default();
        // The test may share the global with others: start from a known
        // state and restore it.
        let saved = core::mem::replace(&mut *DL_BANDWIDTH.lock(), 0);
        assert!(reserve_deadline(&none, &a).is_ok());
        let b = a;
        assert_eq!(reserve_deadline(&none, &b), Err(SyscallError::Busy));
        // Changing a's own reservation counts it once.
        a.deadline = Some(dl::DlEntity::new(9_000_000, 10_000_000, 10_000_000).unwrap());
        let old = SchedParams {
            deadline: Some(dl::DlEntity::new(6_000_000, 10_000_000, 10_000_000).unwrap()),
            ..a
        };
        assert!(reserve_deadline(&old, &a).is_ok());
        assert!(reserve_deadline(&a, &none).is_ok());
        assert_eq!(*DL_BANDWIDTH.lock(), 0);
        *DL_BANDWIDTH.lock() = saved;
    }

    #[test]
    fn affinity_lengths_are_checked_without_overflow() {
        assert!(affinity_len_valid(8, 1));
        assert!(affinity_len_valid(128, 64));
        assert!(!affinity_len_valid(0, 1));
        assert!(!affinity_len_valid(4, 1));
        assert!(!affinity_len_valid(8, 65));
        assert!(!affinity_len_valid(usize::MAX, 1));
        assert!(affinity_len_valid(usize::MAX - 7, 1));
    }

    #[test]
    fn sched_attr_round_trips() {
        let p = SchedParams {
            policy: SCHED_FIFO,
            rt_priority: 42,
            nice: -3,
            deadline: None,
            reset_on_fork: true,
        };
        let raw = SchedAttr::of(&p, SCHED_ATTR_SIZE_VER1 as u32);
        let a = SchedAttr::parse(&raw);
        assert_eq!(
            (a.policy, a.priority, a.nice, a.flags),
            (SCHED_FIFO, 42, -3, SCHED_FLAG_RESET_ON_FORK)
        );
        assert_eq!(&raw[0..4], &56u32.to_ne_bytes());
        assert_eq!(priority_range(SCHED_RR as usize), Ok((1, 99)));
        assert_eq!(priority_range(SCHED_NORMAL as usize), Ok((0, 0)));
        assert_eq!(priority_range(9), Err(SyscallError::InvalidArgument));
    }
}

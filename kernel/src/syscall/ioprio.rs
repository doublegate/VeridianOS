//! ioprio_get and ioprio_set (Linux's block/ioprio.c, ADR 0027): each
//! thread's I/O priority, which the block-device arbiter
//! (`drivers::block_arbiter`) admits requests in order of.
//!
//! A value is Linux's: the class in bits 13..16 (NONE, RT, BE, IDLE), a
//! hint in bits 3..13 (stored, not used) and the level in bits 0..3. A
//! thread that never set one (class NONE) gets its priority from its
//! scheduling: SCHED_IDLE is IDLE, a real-time or deadline policy RT,
//! anything else BE, at level (nice + 20) / 5 (`__get_task_ioprio`).

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU16, Ordering};

use super::{SyscallError, SyscallResult};
use crate::process::{
    linux_caps::{CAP_SYS_ADMIN, CAP_SYS_NICE},
    thread::{SchedParams, SCHED_DEADLINE, SCHED_FIFO, SCHED_IDLE, SCHED_RR},
};

pub const IOPRIO_CLASS_NONE: u16 = 0;
pub const IOPRIO_CLASS_RT: u16 = 1;
pub const IOPRIO_CLASS_BE: u16 = 2;
pub const IOPRIO_CLASS_IDLE: u16 = 3;

const IOPRIO_WHO_PROCESS: usize = 1;
const IOPRIO_WHO_PGRP: usize = 2;
const IOPRIO_WHO_USER: usize = 3;

pub const fn class_of(ioprio: u16) -> u16 {
    ioprio >> 13
}

pub const fn level_of(ioprio: u16) -> u16 {
    ioprio & 7
}

pub const fn value(class: u16, level: u16) -> u16 {
    class << 13 | level
}

/// A thread's I/O context (Linux's `io_context`): its raw I/O priority as
/// set by ioprio_set, 0 (class NONE) until then. CLONE_IO shares it; any
/// other clone or fork gets a copy (`__copy_io`).
#[derive(Debug, Default)]
pub struct IoContext {
    ioprio: AtomicU16,
}

impl IoContext {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn raw(&self) -> u16 {
        self.ioprio.load(Ordering::Relaxed)
    }

    pub fn set(&self, ioprio: u16) {
        self.ioprio.store(ioprio, Ordering::Relaxed);
    }

    /// The child's context: this one (CLONE_IO) or a copy of it.
    pub fn for_child(this: &Arc<Self>, share: bool) -> Arc<Self> {
        if share {
            this.clone()
        } else {
            Arc::new(Self {
                ioprio: AtomicU16::new(this.raw()),
            })
        }
    }
}

/// ioprio_check_cap: RT needs CAP_SYS_ADMIN or CAP_SYS_NICE (in the initial
/// user namespace, as `capable`); BE and IDLE anyone; NONE only at level 0;
/// any other class EINVAL. `capable` answers for the caller.
pub fn check(ioprio: usize, capable: impl Fn(u32) -> bool) -> Result<u16, SyscallError> {
    let ioprio = u16::try_from(ioprio as u32 as i32).map_err(|_| SyscallError::InvalidArgument)?;
    match class_of(ioprio) {
        IOPRIO_CLASS_RT if !capable(CAP_SYS_ADMIN) && !capable(CAP_SYS_NICE) => {
            Err(SyscallError::OperationNotPermitted)
        }
        IOPRIO_CLASS_RT | IOPRIO_CLASS_BE | IOPRIO_CLASS_IDLE => Ok(ioprio),
        IOPRIO_CLASS_NONE if level_of(ioprio) == 0 => Ok(ioprio),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// The priority a thread's I/O runs at: its own, or for class NONE the one
/// its scheduling gives (task_nice_ioclass, task_nice_ioprio).
pub fn effective(raw: u16, sched: &SchedParams) -> u16 {
    if class_of(raw) != IOPRIO_CLASS_NONE {
        return raw;
    }
    let class = match sched.policy {
        SCHED_IDLE => IOPRIO_CLASS_IDLE,
        SCHED_FIFO | SCHED_RR | SCHED_DEADLINE => IOPRIO_CLASS_RT,
        _ => IOPRIO_CLASS_BE,
    };
    value(class, ((i16::from(sched.nice) + 20) / 5) as u16)
}

/// The I/O priority of the calling thread (BE 4 for a kernel thread, the
/// default of nice 0).
pub fn current() -> u16 {
    crate::process::current_thread().map_or(value(IOPRIO_CLASS_BE, 4), |t| {
        effective(t.io.raw(), &t.sched.lock())
    })
}

/// ioprio_best: the higher priority of two, the lower number (RT before
/// BE before IDLE, then by level).
fn best(a: u16, b: u16) -> u16 {
    a.min(b)
}

/// The threads `which`/`who` name (PROCESS: one thread; PGRP and USER:
/// every thread of the matching processes in the caller's namespaces), with
/// each one's process and owner. No match: ESRCH; an unknown `which`:
/// EINVAL.
fn targets(
    which: usize,
    who: usize,
) -> Result<
    Vec<(
        u64,
        crate::process::creds::Credentials,
        Arc<crate::process::thread::Thread>,
    )>,
    SyscallError,
> {
    let found = match which {
        IOPRIO_WHO_PROCESS => {
            alloc::vec![super::scheduling::target(
                who,
                SyscallError::ProcessNotFound
            )?]
        }
        IOPRIO_WHO_PGRP => super::scheduling::priority_targets(super::scheduling::PRIO_PGRP, who)?,
        IOPRIO_WHO_USER => super::scheduling::priority_targets(super::scheduling::PRIO_USER, who)?,
        _ => return Err(SyscallError::InvalidArgument),
    };
    if found.is_empty() {
        return Err(SyscallError::ProcessNotFound);
    }
    Ok(found)
}

/// ioprio_set(which, who, ioprio): the check above first, then each target
/// in turn, stopping at the first refused one (EPERM unless its owner's
/// UID is the caller's real or effective one, or the caller holds
/// CAP_SYS_NICE; set_task_ioprio), besides MODIFY over its process (ADR
/// 0024, as setpriority).
pub fn sys_ioprio_set(which: usize, who: usize, ioprio: usize) -> SyscallResult {
    let me = crate::process::creds::Caller::current();
    let ioprio = check(ioprio, |cap| me.capable(cap))?;
    for (pid, owner, thread) in targets(which, who)? {
        let same = owner.ruid == me.creds.euid || owner.ruid == me.creds.ruid;
        if !(same || me.capable(CAP_SYS_NICE)) || !super::scheduling::may_act_on_pid(pid) {
            return Err(SyscallError::OperationNotPermitted);
        }
        thread.io.set(ioprio);
    }
    Ok(0)
}

/// ioprio_get(which, who): for one thread its raw value (0 if never set,
/// so a caller can tell unset from set); for a group or user the best
/// effective one of their threads.
pub fn sys_ioprio_get(which: usize, who: usize) -> SyscallResult {
    let found = targets(which, who)?;
    if which == IOPRIO_WHO_PROCESS {
        return Ok(usize::from(found[0].2.io.raw()));
    }
    let prio = found
        .iter()
        .map(|(_, _, t)| effective(t.io.raw(), &t.sched.lock()))
        .reduce(best)
        .ok_or(SyscallError::ProcessNotFound)?;
    Ok(usize::from(prio))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rt_needs_a_capability_and_unknown_classes_are_einval() {
        let none = |_| false;
        let nice = |c| c == CAP_SYS_NICE;
        let rt = usize::from(value(IOPRIO_CLASS_RT, 0));
        assert_eq!(check(rt, none), Err(SyscallError::OperationNotPermitted));
        assert_eq!(check(rt, nice), Ok(value(IOPRIO_CLASS_RT, 0)));
        assert_eq!(
            check(rt, |c| c == CAP_SYS_ADMIN),
            Ok(value(IOPRIO_CLASS_RT, 0))
        );
        let be7 = usize::from(value(IOPRIO_CLASS_BE, 7));
        assert_eq!(check(be7, none), Ok(value(IOPRIO_CLASS_BE, 7)));
        assert!(check(usize::from(value(IOPRIO_CLASS_IDLE, 0)), none).is_ok());
        assert_eq!(check(0, none), Ok(0), "NONE, level 0");
        assert_eq!(
            check(1, none),
            Err(SyscallError::InvalidArgument),
            "NONE, level 1"
        );
        assert_eq!(
            check(usize::from(value(4, 0)), none),
            Err(SyscallError::InvalidArgument),
            "IOPRIO_CLASS_INVALID"
        );
        assert_eq!(
            check(usize::MAX, none),
            Err(SyscallError::InvalidArgument),
            "-1"
        );
        // A hint is kept, not checked (ioprio_check_cap ignores it).
        let hinted = usize::from(value(IOPRIO_CLASS_BE, 2) | 1 << 3);
        assert_eq!(check(hinted, none), Ok(hinted as u16));
    }

    #[test]
    fn an_unset_priority_follows_the_scheduling() {
        let mut s = SchedParams::default();
        assert_eq!(effective(0, &s), value(IOPRIO_CLASS_BE, 4), "nice 0");
        s.nice = -20;
        assert_eq!(effective(0, &s), value(IOPRIO_CLASS_BE, 0));
        s.nice = 19;
        assert_eq!(effective(0, &s), value(IOPRIO_CLASS_BE, 7));
        s.nice = 0;
        s.policy = SCHED_IDLE;
        assert_eq!(class_of(effective(0, &s)), IOPRIO_CLASS_IDLE);
        s.policy = SCHED_FIFO;
        assert_eq!(class_of(effective(0, &s)), IOPRIO_CLASS_RT);
        let set = value(IOPRIO_CLASS_IDLE, 0);
        assert_eq!(effective(set, &s), set, "a set value wins");
    }

    #[test]
    fn clone_io_shares_the_context_and_anything_else_copies_it() {
        let parent = IoContext::new();
        parent.set(value(IOPRIO_CLASS_BE, 1));
        let shared = IoContext::for_child(&parent, true);
        let copied = IoContext::for_child(&parent, false);
        parent.set(value(IOPRIO_CLASS_IDLE, 0));
        assert_eq!(shared.raw(), value(IOPRIO_CLASS_IDLE, 0));
        assert_eq!(copied.raw(), value(IOPRIO_CLASS_BE, 1));
    }

    #[test]
    fn the_best_of_a_group_is_the_lowest_value() {
        assert_eq!(
            best(value(IOPRIO_CLASS_BE, 4), value(IOPRIO_CLASS_RT, 7)),
            value(IOPRIO_CLASS_RT, 7)
        );
        assert_eq!(
            best(value(IOPRIO_CLASS_BE, 4), value(IOPRIO_CLASS_BE, 2)),
            value(IOPRIO_CLASS_BE, 2)
        );
    }
}

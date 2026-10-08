//! Resource limits (getrlimit, setrlimit, prlimit64; N-224).
//!
//! Each process has Linux's 16 limits, each a soft value (what is enforced)
//! and a hard ceiling (how far the soft one may rise). They are inherited
//! on fork and kept across exec. Enforcement lives where each resource is
//! used: descriptors (NOFILE), file size (FSIZE), processes (NPROC),
//! address space and data (AS, DATA), CPU time (CPU), nice and real-time
//! priority (NICE, RTPRIO), queued signals (SIGPENDING).

use crate::syscall::SyscallError;

pub const RLIMIT_CPU: usize = 0;
pub const RLIMIT_FSIZE: usize = 1;
pub const RLIMIT_DATA: usize = 2;
pub const RLIMIT_STACK: usize = 3;
pub const RLIMIT_CORE: usize = 4;
pub const RLIMIT_RSS: usize = 5;
pub const RLIMIT_NPROC: usize = 6;
pub const RLIMIT_NOFILE: usize = 7;
pub const RLIMIT_MEMLOCK: usize = 8;
pub const RLIMIT_AS: usize = 9;
pub const RLIMIT_LOCKS: usize = 10;
pub const RLIMIT_SIGPENDING: usize = 11;
pub const RLIMIT_MSGQUEUE: usize = 12;
pub const RLIMIT_NICE: usize = 13;
pub const RLIMIT_RTPRIO: usize = 14;
pub const RLIMIT_RTTIME: usize = 15;
/// The number of limits.
pub const RLIM_NLIMITS: usize = 16;

/// No limit.
pub const RLIM_INFINITY: u64 = u64::MAX;

/// One limit: `cur` is enforced, `max` caps it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rlimit {
    pub cur: u64,
    pub max: u64,
}

// SAFETY: repr(C), two u64 fields, no padding; any bit pattern is valid
// (`struct rlimit` and `struct rlimit64` on x86_64).
unsafe impl crate::syscall::userspace::UserPod for Rlimit {}

impl Rlimit {
    const fn both(v: u64) -> Self {
        Self { cur: v, max: v }
    }
}

/// A process's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits([Rlimit; RLIM_NLIMITS]);

impl Default for Limits {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Limits {
    /// The limits the first process starts with (Linux's INIT_RLIMITS,
    /// sized to this kernel: the descriptor table and process table hold
    /// 1024 each, a real-time signal queues up to RT_QUEUE_MAX).
    pub const fn defaults() -> Self {
        const INF: Rlimit = Rlimit::both(RLIM_INFINITY);
        let mut l = [INF; RLIM_NLIMITS];
        l[RLIMIT_STACK] = Rlimit {
            cur: 8 * 1024 * 1024,
            max: RLIM_INFINITY,
        };
        l[RLIMIT_CORE] = Rlimit {
            cur: 0,
            max: RLIM_INFINITY,
        };
        l[RLIMIT_NPROC] = Rlimit::both(super::MAX_PROCESSES as u64);
        l[RLIMIT_NOFILE] = Rlimit::both(crate::fs::file::MAX_FDS as u64);
        l[RLIMIT_MEMLOCK] = Rlimit::both(8 * 1024 * 1024);
        l[RLIMIT_SIGPENDING] = Rlimit::both(super::signals::RT_QUEUE_MAX as u64);
        l[RLIMIT_MSGQUEUE] = Rlimit::both(819_200);
        l[RLIMIT_NICE] = Rlimit::both(0);
        l[RLIMIT_RTPRIO] = Rlimit::both(0);
        Self(l)
    }

    /// Limit `resource` (EINVAL if there is no such resource).
    pub fn get(&self, resource: usize) -> Result<Rlimit, SyscallError> {
        self.0
            .get(resource)
            .copied()
            .ok_or(SyscallError::InvalidArgument)
    }

    /// The soft value of `resource`, which must exist.
    pub fn cur(&self, resource: usize) -> u64 {
        self.0[resource].cur
    }

    /// Set `resource` to `new` with Linux's do_prlimit checks; the old
    /// value. EINVAL for no such resource or a soft value above the hard
    /// one; EPERM for raising the hard value without privilege
    /// (CAP_SYS_RESOURCE: root here) or a descriptor limit past the table
    /// (nr_open).
    pub fn set(
        &mut self,
        resource: usize,
        new: Rlimit,
        privileged: bool,
    ) -> Result<Rlimit, SyscallError> {
        let old = self.get(resource)?;
        if new.cur > new.max {
            return Err(SyscallError::InvalidArgument);
        }
        if resource == RLIMIT_NOFILE && new.max > crate::fs::file::MAX_FDS as u64 {
            return Err(SyscallError::OperationNotPermitted);
        }
        if new.max > old.max && !privileged {
            return Err(SyscallError::OperationNotPermitted);
        }
        self.0[resource] = new;
        Ok(old)
    }
}

/// Whether a user with `tasks` tasks may make another under an
/// RLIMIT_NPROC of `limit` (Linux's is_rlimit_overlimit, counting the new
/// one).
pub fn nproc_allows(tasks: usize, limit: u64) -> bool {
    (tasks as u64) < limit
}

/// RLIMIT_NPROC (N-224) for `process` making a process or thread (Linux's
/// copy_process): its real user may not already have as many tasks --
/// threads, and processes not yet reaped -- as the limit allows. Root is
/// exempt, as Linux exempts the root user and CAP_SYS_RESOURCE.
#[cfg(feature = "alloc")]
pub fn check_nproc(process: &super::Process) -> Result<(), SyscallError> {
    let creds = process.credentials();
    if creds.ruid == 0 || creds.euid == 0 {
        return Ok(());
    }
    let limit = process.limits().cur(RLIMIT_NPROC);
    if limit == RLIM_INFINITY {
        return Ok(());
    }
    let mut tasks = 0usize;
    super::table::PROCESS_TABLE.for_each(|p| {
        if p.credentials().ruid == creds.ruid {
            tasks += p.threads.lock().len().max(1);
        }
    });
    if nproc_allows(tasks, limit) {
        Ok(())
    } else {
        Err(SyscallError::WouldBlock)
    }
}

/// What RLIMIT_CPU does to a process that has used `used_ns` of CPU time
/// with soft and hard limits of `soft_ns` and `hard_ns` (Linux's
/// check_process_timers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuLimit {
    /// Below the soft limit.
    Within,
    /// At or past the soft limit: SIGXCPU.
    Soft,
    /// At or past the hard limit: SIGKILL.
    Hard,
}

pub fn cpu_limit(used_ns: u64, soft_ns: u64, hard_ns: u64) -> CpuLimit {
    if hard_ns != u64::MAX && used_ns >= hard_ns {
        CpuLimit::Hard
    } else if soft_ns != u64::MAX && used_ns >= soft_ns {
        CpuLimit::Soft
    } else {
        CpuLimit::Within
    }
}

/// RLIMIT_CPU (N-224) for `process` on its way back to user mode: SIGKILL
/// once its threads have used the hard limit, SIGXCPU at the soft one and
/// again each further second. Free unless a limit is set.
#[cfg(feature = "alloc")]
pub fn check_cpu(process: &super::Process) {
    use core::sync::atomic::Ordering;
    let soft = process.cpu_limit_ns[0].load(Ordering::Acquire);
    let hard = process.cpu_limit_ns[1].load(Ordering::Acquire);
    if soft == u64::MAX && hard == u64::MAX {
        return;
    }
    use super::exit::signals::{SIGKILL, SIGXCPU};
    match cpu_limit(process.cpu_times().runtime_ns, soft, hard) {
        CpuLimit::Within => {}
        CpuLimit::Soft => {
            super::signals::notify(process, SIGXCPU as usize);
            process.bump_cpu_soft_limit();
        }
        CpuLimit::Hard => super::signals::notify(process, SIGKILL as usize),
    }
}

/// Linux's may_expand_vm: whether an address space using `usage` may map
/// `bytes` more, `data` saying whether RLIMIT_DATA counts them too. Limits
/// are compared in whole pages, as Linux does.
pub fn may_expand_vm(
    limits: &Limits,
    usage: crate::mm::vas::VmUsage,
    bytes: u64,
    data: bool,
) -> bool {
    let fits =
        |used: u64, resource: usize| used.saturating_add(bytes) <= limits.cur(resource) & !0xFFF;
    fits(usage.total, RLIMIT_AS) && (!data || fits(usage.data, RLIMIT_DATA))
}

/// What RLIMIT_NICE allows: lowering the nice value (raising priority) to
/// at most `20 - cur`, as Linux's can_nice reads it (nice -20..19 maps to
/// 40..1).
pub fn nice_floor(limits: &Limits) -> i32 {
    20 - limits.cur(RLIMIT_NICE).min(40) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_follows_linux_rules() {
        let mut l = Limits::defaults();
        assert_eq!(
            l.get(RLIMIT_NOFILE),
            Ok(Rlimit::both(crate::fs::file::MAX_FDS as u64))
        );
        assert_eq!(l.get(RLIM_NLIMITS), Err(SyscallError::InvalidArgument));
        // Lower soft and hard: anyone; the old value comes back.
        let old = l
            .set(RLIMIT_NOFILE, Rlimit { cur: 64, max: 512 }, false)
            .unwrap();
        assert_eq!(old.cur, crate::fs::file::MAX_FDS as u64);
        // Soft above hard: EINVAL.
        assert_eq!(
            l.set(RLIMIT_NOFILE, Rlimit { cur: 600, max: 512 }, false),
            Err(SyscallError::InvalidArgument)
        );
        // Raising the hard limit needs privilege.
        assert_eq!(
            l.set(RLIMIT_NOFILE, Rlimit::both(1000), false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert!(l.set(RLIMIT_NOFILE, Rlimit::both(1000), true).is_ok());
        // Not even root goes past the descriptor table.
        assert_eq!(
            l.set(RLIMIT_NOFILE, Rlimit::both(1 << 20), true),
            Err(SyscallError::OperationNotPermitted)
        );
        // Raising the soft value up to the hard one: anyone.
        let mut l = Limits::defaults();
        assert!(l
            .set(
                RLIMIT_CORE,
                Rlimit {
                    cur: RLIM_INFINITY,
                    max: RLIM_INFINITY
                },
                false
            )
            .is_ok());
    }

    #[test]
    fn cpu_limit_signals_at_soft_then_kills_at_hard() {
        const S: u64 = 1_000_000_000;
        assert_eq!(cpu_limit(0, u64::MAX, u64::MAX), CpuLimit::Within);
        assert_eq!(cpu_limit(S - 1, S, 3 * S), CpuLimit::Within);
        assert_eq!(cpu_limit(S, S, 3 * S), CpuLimit::Soft);
        assert_eq!(cpu_limit(3 * S, S, 3 * S), CpuLimit::Hard);
        // A soft limit with no hard one signals but never kills.
        assert_eq!(cpu_limit(100 * S, S, u64::MAX), CpuLimit::Soft);
    }

    #[test]
    fn nproc_counts_the_new_task() {
        assert!(nproc_allows(0, 1));
        assert!(!nproc_allows(1, 1));
        assert!(nproc_allows(1023, 1024));
        assert!(!nproc_allows(1024, 1024));
        assert!(!nproc_allows(0, 0));
    }

    #[test]
    fn address_space_and_data_limits_count_pages() {
        use crate::mm::vas::VmUsage;
        let mut l = Limits::defaults();
        let usage = VmUsage {
            total: 0x10_0000,
            data: 0x4000,
        };
        assert!(may_expand_vm(&l, usage, u64::MAX / 2, true));
        l.set(RLIMIT_AS, Rlimit::both(0x10_2fff), false).unwrap();
        // 0x102fff holds two whole pages more than is mapped, not three.
        assert!(may_expand_vm(&l, usage, 0x2000, true));
        assert!(!may_expand_vm(&l, usage, 0x3000, false));
        l.set(RLIMIT_DATA, Rlimit::both(0x5000), false).unwrap();
        assert!(may_expand_vm(&l, usage, 0x1000, true));
        assert!(!may_expand_vm(&l, usage, 0x2000, true));
        // A shared or read-only mapping is not data.
        assert!(may_expand_vm(&l, usage, 0x2000, false));
    }

    #[test]
    fn nice_limit_maps_to_a_floor() {
        let mut l = Limits::defaults();
        assert_eq!(nice_floor(&l), 20);
        l.set(RLIMIT_NICE, Rlimit::both(30), true).unwrap();
        assert_eq!(nice_floor(&l), -10);
        l.set(RLIMIT_NICE, Rlimit::both(RLIM_INFINITY), true)
            .unwrap();
        assert_eq!(nice_floor(&l), -20);
    }
}

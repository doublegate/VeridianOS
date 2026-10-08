//! Resource usage and system information: getrusage, times, sysinfo
//! (N-223), and the rusage wait4 and waitid report (N-213).
//!
//! CPU time comes from the dispatcher's accounting (`sched::cputime`):
//! run time split into user and system time by tick samples, and context
//! switches. Page faults, block I/O and IPC are not counted yet: those
//! fields are zero.

use super::{SyscallError, SyscallResult};
use crate::sched::cputime::CpuTimes;

/// `struct timeval`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

impl Timeval {
    fn from_ns(ns: u64) -> Self {
        Self {
            tv_sec: (ns / 1_000_000_000) as i64,
            tv_usec: ((ns % 1_000_000_000) / 1000) as i64,
        }
    }
}

/// Linux's `struct rusage` (x86_64, 144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Rusage {
    ru_utime: Timeval,
    ru_stime: Timeval,
    /// Peak resident set, in kilobytes.
    ru_maxrss: i64,
    ru_ixrss: i64,
    ru_idrss: i64,
    ru_isrss: i64,
    ru_minflt: i64,
    ru_majflt: i64,
    ru_nswap: i64,
    ru_inblock: i64,
    ru_oublock: i64,
    ru_msgsnd: i64,
    ru_msgrcv: i64,
    ru_nsignals: i64,
    ru_nvcsw: i64,
    ru_nivcsw: i64,
}

// SAFETY: repr(C), only i64 fields, no padding.
unsafe impl super::userspace::UserPod for Rusage {}

const _: () = assert!(core::mem::size_of::<Rusage>() == 144);

impl Rusage {
    /// The usage `times` and a peak resident set of `maxrss_pages`
    /// describe.
    pub(crate) fn from_times(times: CpuTimes, maxrss_pages: usize) -> Self {
        let (user, system) = times.user_system_ns();
        Self {
            ru_utime: Timeval::from_ns(user),
            ru_stime: Timeval::from_ns(system),
            ru_maxrss: (maxrss_pages as i64).saturating_mul(4),
            ru_nvcsw: times.voluntary_switches as i64,
            ru_nivcsw: times.involuntary_switches as i64,
            ..Self::default()
        }
    }
}

/// Write a `struct rusage` to `ptr` (if not NULL).
#[cfg(feature = "alloc")]
pub(crate) fn write_rusage(
    ptr: usize,
    times: CpuTimes,
    maxrss_pages: usize,
) -> Result<(), SyscallError> {
    if ptr != 0 {
        super::userspace::write_user(ptr, Rusage::from_times(times, maxrss_pages))?;
    }
    Ok(())
}

const RUSAGE_SELF: i32 = 0;
const RUSAGE_CHILDREN: i32 = -1;
const RUSAGE_THREAD: i32 = 1;

/// getrusage (Linux 98): the calling process's usage, its waited-for
/// children's, or the calling thread's; EINVAL for anything else.
#[cfg(feature = "alloc")]
pub fn sys_getrusage(who: usize, usage_ptr: usize) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let (times, maxrss) = match who as i32 {
        RUSAGE_SELF => (process.cpu_times(), process.peak_rss_pages()),
        RUSAGE_CHILDREN => (
            process.children_cpu_times(),
            process.children_peak_rss_pages(),
        ),
        RUSAGE_THREAD => {
            let tid = crate::process::current_thread()
                .ok_or(SyscallError::InvalidState)?
                .tid
                .0;
            (
                crate::sched::dispatch::thread_cpu((process.pid.0, tid)).unwrap_or_default(),
                process.peak_rss_pages(),
            )
        }
        _ => return Err(SyscallError::InvalidArgument),
    };
    super::userspace::write_user(usage_ptr, Rusage::from_times(times, maxrss))?;
    Ok(0)
}

/// Clock ticks per second of `times` (AT_CLKTCK, sysconf(_SC_CLK_TCK)).
const USER_HZ: u64 = 100;

fn ticks(ns: u64) -> i64 {
    (ns / (1_000_000_000 / USER_HZ)) as i64
}

/// `struct tms`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Tms {
    tms_utime: i64,
    tms_stime: i64,
    tms_cutime: i64,
    tms_cstime: i64,
}

// SAFETY: repr(C), four i64 fields, no padding.
unsafe impl super::userspace::UserPod for Tms {}

impl Tms {
    fn new(own: CpuTimes, children: CpuTimes) -> Self {
        let (u, s) = own.user_system_ns();
        let (cu, cs) = children.user_system_ns();
        Self {
            tms_utime: ticks(u),
            tms_stime: ticks(s),
            tms_cutime: ticks(cu),
            tms_cstime: ticks(cs),
        }
    }
}

/// times (Linux 100): the process's and its waited-for children's user
/// and system time in clock ticks (`buf` may be NULL); returns the ticks
/// since boot.
#[cfg(feature = "alloc")]
pub fn sys_times(buf: usize) -> SyscallResult {
    if buf != 0 {
        let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
        let tms = Tms::new(process.cpu_times(), process.children_cpu_times());
        super::userspace::write_user(buf, tms)?;
    }
    Ok(ticks(crate::timer::monotonic_ns()) as usize)
}

/// Linux's `struct sysinfo` (x86_64, 112 bytes; the padding is explicit).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Sysinfo {
    uptime: i64,
    /// 1, 5 and 15 minute load averages, with 16 fraction bits.
    loads: [u64; 3],
    totalram: u64,
    freeram: u64,
    sharedram: u64,
    bufferram: u64,
    totalswap: u64,
    freeswap: u64,
    procs: u16,
    pad: u16,
    pad2: u32,
    totalhigh: u64,
    freehigh: u64,
    mem_unit: u32,
    pad3: u32,
}

// SAFETY: repr(C), every field accounted for (explicit padding), so no
// implicit padding; any bit pattern is valid.
unsafe impl super::userspace::UserPod for Sysinfo {}

const _: () = assert!(core::mem::size_of::<Sysinfo>() == 112);

/// sysinfo (Linux 99): uptime, load averages, memory (in bytes:
/// `mem_unit` 1) and the number of processes. There is no swap and no high
/// memory; shared memory is the frames more than one owner maps.
#[cfg(feature = "alloc")]
pub fn sys_sysinfo(info_ptr: usize) -> SyscallResult {
    const SI_LOAD_SHIFT: u32 = 16;
    let stats = crate::mm::get_memory_stats();
    let page = 4096u64;
    let loads = crate::sched::loadavg::averages()
        .map(|l| l << (SI_LOAD_SHIFT - crate::sched::loadavg::FSHIFT));
    let procs = crate::process::get_process_list().map_or(0, |p| p.len());
    let info = Sysinfo {
        uptime: (crate::timer::monotonic_ns() / 1_000_000_000) as i64,
        loads,
        totalram: stats.total_frames as u64 * page,
        freeram: stats.free_frames as u64 * page,
        sharedram: crate::mm::frame_refs::shared_frames() as u64 * page,
        bufferram: 0,
        procs: procs.min(u16::MAX as usize) as u16,
        mem_unit: 1,
        ..Sysinfo::default()
    };
    super::userspace::write_user(info_ptr, info)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rusage_reports_split_time_rss_and_switches() {
        let times = CpuTimes {
            runtime_ns: 3_000_000_000,
            user_ticks: 2,
            system_ticks: 1,
            voluntary_switches: 5,
            involuntary_switches: 7,
        };
        let r = Rusage::from_times(times, 10);
        assert_eq!(
            r.ru_utime,
            Timeval {
                tv_sec: 2,
                tv_usec: 0
            }
        );
        assert_eq!(
            r.ru_stime,
            Timeval {
                tv_sec: 1,
                tv_usec: 0
            }
        );
        assert_eq!((r.ru_maxrss, r.ru_nvcsw, r.ru_nivcsw), (40, 5, 7));
    }

    #[test]
    fn times_counts_clock_ticks() {
        let own = CpuTimes {
            runtime_ns: 1_000_000_000,
            ..CpuTimes::ZERO
        };
        let children = CpuTimes {
            runtime_ns: 250_000_000,
            system_ticks: 1,
            ..CpuTimes::ZERO
        };
        assert_eq!(
            Tms::new(own, children),
            Tms {
                tms_utime: 100,
                tms_stime: 0,
                tms_cutime: 0,
                tms_cstime: 25,
            }
        );
    }
}

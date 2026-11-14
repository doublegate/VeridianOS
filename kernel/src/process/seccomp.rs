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

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU8, Ordering};

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
}

impl Filter {
    /// The chain of `prog` on top of `prev`, or None past
    /// [`MAX_INSNS_PER_PATH`] (ENOMEM).
    pub fn new(prog: SeccompProgram, prev: Option<Arc<Filter>>, log: bool) -> Option<Arc<Self>> {
        let below = prev.as_ref().map_or(0, |p| p.path_len);
        let path_len = below + prog.len() + 4;
        if path_len > MAX_INSNS_PER_PATH {
            return None;
        }
        let depth = prev.as_ref().map_or(0, |p| p.depth) + 1;
        Some(Arc::new(Self {
            prog,
            prev,
            log,
            path_len,
            depth,
        }))
    }

    /// How many programs the chain holds.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Whether `other` is this chain or one of its ancestors (TSYNC's
    /// is_ancestor: a thread whose chain is not under the caller's cannot
    /// be synchronized).
    pub fn has_ancestor(self: &Arc<Self>, other: Option<&Arc<Filter>>) -> bool {
        let Some(other) = other else {
            return true;
        };
        let mut f = Some(self.clone());
        while let Some(cur) = f {
            if Arc::ptr_eq(&cur, other) {
                return true;
            }
            f = cur.prev.clone();
        }
        false
    }
}

/// The value of highest precedence `chain` returns on `data`
/// (seccomp_run_filters), and the program that returned it (for LOG and
/// USER_NOTIF).
pub fn run_chain(chain: &Arc<Filter>, data: &[u8; 64]) -> (u32, Arc<Filter>) {
    let mut best = ret::ALLOW;
    let mut matched = chain.clone();
    let mut f = Some(chain.clone());
    while let Some(cur) = f {
        let r = veridian_cbpf::run_seccomp(&cur.prog, data);
        if ((r & ret::ACTION_FULL) as i32) < ((best & ret::ACTION_FULL) as i32) {
            best = r;
            matched = cur.clone();
        }
        f = cur.prev.clone();
    }
    (best, matched)
}

/// A change of mode refused: strict to filter or back (EINVAL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeConflict;

/// A thread's seccomp state.
pub struct ThreadSeccomp {
    mode: AtomicU8,
    chain: Mutex<Option<Arc<Filter>>>,
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

    /// The filter chain, in filter mode.
    pub fn chain(&self) -> Option<Arc<Filter>> {
        self.chain.lock().clone()
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
        *self.chain.lock() = chain;
        self.mode.store(other.mode(), Ordering::Release);
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

    /// Put `filter` (made on top of this thread's chain) in place and enter
    /// filter mode (EINVAL from strict mode).
    pub fn set_filter(&self, filter: Arc<Filter>) -> Result<(), ModeConflict> {
        if self.mode() == MODE_STRICT {
            return Err(ModeConflict);
        }
        *self.chain.lock() = Some(filter);
        self.mode.store(MODE_FILTER, Ordering::Release);
        Ok(())
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
    (s.mode(), s.chain().map_or(0, |c| c.depth()))
}

/// Programs from `prev` up, newest first (tests).
#[cfg(test)]
fn programs(chain: &Arc<Filter>) -> alloc::vec::Vec<usize> {
    let mut out = alloc::vec::Vec::new();
    let mut f = Some(chain.clone());
    while let Some(cur) = f {
        out.push(cur.prog.len());
        f = cur.prev.clone();
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

    /// The action of highest precedence wins across the chain, whichever
    /// program returned it.
    #[test]
    fn the_strictest_action_wins() {
        let a = Filter::new(ret_only(ret::ALLOW), None, false).unwrap();
        let b = Filter::new(ret_only(ret::ERRNO | 1), Some(a.clone()), false).unwrap();
        let c = Filter::new(ret_only(ret::LOG), Some(b.clone()), false).unwrap();
        let d = seccomp_data(39, 0, [0; 6]);
        assert_eq!(run_chain(&c, &d).0, ret::ERRNO | 1);
        let k = Filter::new(ret_only(ret::KILL_PROCESS), Some(c.clone()), false).unwrap();
        assert_eq!(run_chain(&k, &d).0, ret::KILL_PROCESS);
        assert_eq!(k.depth(), 4);
        assert_eq!(programs(&k), [1, 1, 1, 1]);
        assert!(k.has_ancestor(Some(&a)) && !a.has_ancestor(Some(&k)));
    }

    #[test]
    fn modes_do_not_change_back() {
        let s = ThreadSeccomp::new();
        s.set_strict().unwrap();
        let f = Filter::new(ret_only(ret::ALLOW), None, false).unwrap();
        assert!(s.set_filter(f.clone()).is_err());
        let t = ThreadSeccomp::new();
        t.set_filter(f).unwrap();
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
        let mut chain = None;
        let mut n = 0;
        while let Some(f) = Filter::new(check_seccomp(&big).unwrap(), chain.clone(), false) {
            chain = Some(f);
            n += 1;
        }
        // 32768 / (4096 + 4): seven fit, the eighth does not.
        assert_eq!(n, 7);
    }
}

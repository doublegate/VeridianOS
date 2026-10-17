//! Signals for dispatched threads (sprint D3; N-96, N-98, N-109, N-113).
//!
//! Semantics follow Linux x86_64, so musl and the native C library work
//! unmodified:
//!
//! - **Sets** use the Linux layout: bit `sig - 1` for signal `sig` (N-96).
//! - **State.** Each thread has its own mask and pending set (N-109); signals
//!   sent to the process pend on the process and go to whichever thread does
//!   not block them.
//! - **Generation.** SIGKILL is acted on at once (`kill_pending`); a signal
//!   whose action is to ignore it is dropped when it is sent, so it never
//!   interrupts a wait (N-98); everything else pends and wakes the target.
//! - **Delivery** happens on the way back to user mode, through the frame on
//!   the thread's kernel stack: default actions run there (terminate, with a
//!   core flag for the core-dumping signals; stop); a handler gets a Linux
//!   `rt_sigframe` on the user stack -- return address (the SA_RESTORER
//!   trampoline), `ucontext` with the interrupted registers and mask,
//!   `siginfo`, and the FXSAVE image of the x87/SSE state -- and runs with
//!   `sa_mask` and the signal itself blocked (N-113). SA_RESTART restarts a
//!   system call that a signal interrupted.
//! - **Job control.** A stop signal with the default action stops the whole
//!   process when a thread takes it: the parent gets SIGCHLD (unless it set
//!   SA_NOCLDSTOP) and a `WUNTRACED` report, and every thread parks at its next
//!   return to user mode. SIGCONT continues the process when it is sent,
//!   whatever its action, and discards pending stop signals; a stop signal
//!   discards a pending SIGCONT. SIGKILL ends a stopped process.
//! - **rt_sigreturn** restores the registers (sanitised: user selectors and
//!   RFLAGS bits, user RIP and RSP), the mask and the FPU image (MXCSR reserved
//!   bits cleared so FXRSTOR cannot fault in ring 0).

use core::sync::atomic::Ordering;

use super::{pcb::Process, thread::Thread};

/// The bit for signal `sig` in a signal set (`0` outside 1..=64).
pub const fn sig_bit(sig: usize) -> u64 {
    if sig >= 1 && sig <= 64 {
        1u64 << (sig - 1)
    } else {
        0
    }
}

/// The highest signal number (Linux `_NSIG`); 1..=NSIG are valid.
pub const NSIG: usize = 64;

/// The first real-time signal (the kernel's SIGRTMIN; C libraries reserve
/// the first few, musl 32-34). Real-time signals queue: each send is
/// delivered once, where a standard signal sent twice before delivery is
/// delivered once (N-209).
pub const SIGRTMIN: usize = 32;

/// Real-time signals a user may have queued by default (RLIMIT_SIGPENDING's
/// initial value; Linux sizes it from memory, typically in the thousands).
pub const RT_QUEUE_MAX: u32 = 1024;

/// A real-time signal could not be queued: its receiver's real user has
/// RLIMIT_SIGPENDING signals queued already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

/// Who pays for a queued real-time signal (Linux's __sigqueue_alloc): the
/// receiving process's real user, up to that process's RLIMIT_SIGPENDING.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charge {
    pub uid: u32,
    pub limit: u64,
}

impl Charge {
    /// What a signal sent to `process` is charged to.
    pub fn of(process: &Process) -> Self {
        Self {
            uid: process.credentials().ruid,
            limit: process.limits().cur(super::rlimit::RLIMIT_SIGPENDING),
        }
    }

    /// No limit (tests).
    #[cfg(test)]
    pub const UNLIMITED: Self = Self {
        uid: u32::MAX,
        limit: u64::MAX,
    };
}

/// What a real-time send does when its user's queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// Fail with EAGAIN: tkill, tgkill, sigqueue (Linux: a real-time signal
    /// whose si_code is not SI_USER).
    Fail,
    /// Still make the signal pending, without another queued instance:
    /// kill and signals the kernel sends (Linux's "silent loss of
    /// information").
    Merge,
}

/// Real-time signals queued per real user, against RLIMIT_SIGPENDING.
#[cfg(feature = "alloc")]
static QUEUED_BY_USER: spin::Mutex<alloc::collections::BTreeMap<u32, u64>> =
    spin::Mutex::new(alloc::collections::BTreeMap::new());

/// Charge one queued signal to `charge.uid` if it stays within the limit.
fn charge_user(charge: Charge) -> bool {
    #[cfg(feature = "alloc")]
    {
        let mut queued = QUEUED_BY_USER.lock();
        let count = queued.entry(charge.uid).or_insert(0);
        if *count >= charge.limit {
            return false;
        }
        *count += 1;
    }
    let _ = charge;
    true
}

/// Give back `n` queued signals charged to `uid`.
fn uncharge_user(uid: u32, n: u64) {
    #[cfg(feature = "alloc")]
    if n > 0 {
        let mut queued = QUEUED_BY_USER.lock();
        if let Some(count) = queued.get_mut(&uid) {
            *count = count.saturating_sub(n);
            if *count == 0 {
                queued.remove(&uid);
            }
        }
    }
    let _ = (uid, n);
}

/// Real-time signals the user `uid` has queued (tests, procfs).
pub fn queued_by_user(uid: u32) -> u64 {
    #[cfg(feature = "alloc")]
    return QUEUED_BY_USER.lock().get(&uid).copied().unwrap_or(0);
    #[cfg(not(feature = "alloc"))]
    {
        let _ = uid;
        0
    }
}

/// `si_code` values (Linux): who or what sent a signal.
pub const SI_USER: i32 = 0;
pub const SI_KERNEL: i32 = 0x80;
pub const SI_QUEUE: i32 = -1;
pub const SI_TKILL: i32 = -6;
/// SIGCHLD `si_code`: how the child changed state.
pub const CLD_EXITED: i32 = 1;
pub const CLD_KILLED: i32 = 2;
pub const CLD_DUMPED: i32 = 3;
pub const CLD_STOPPED: i32 = 5;
pub const CLD_CONTINUED: i32 = 6;
/// Fault `si_code`s (x86 exceptions).
pub const SEGV_MAPERR: i32 = 1;
pub const SEGV_ACCERR: i32 = 2;
pub const FPE_INTDIV: i32 = 1;
pub const FPE_FLTDIV: i32 = 3;
pub const FPE_FLTOVF: i32 = 4;
pub const FPE_FLTUND: i32 = 5;
pub const FPE_FLTRES: i32 = 6;
pub const FPE_FLTINV: i32 = 7;
pub const ILL_ILLOPC: i32 = 1;
pub const ILL_ILLOPN: i32 = 2;
pub const BUS_ADRALN: i32 = 1;
pub const TRAP_BRKPT: i32 = 1;
pub const TRAP_TRACE: i32 = 2;

/// Linux `siginfo_t` (x86_64, 128 bytes): signo, errno, code, then a union
/// whose members the constructors below fill: kill and sigqueue have the
/// sender's pid and real uid at byte 16 and the queued value at 24; SIGCHLD
/// has the child's pid, uid, status and user/system time; a fault has the
/// address at 16.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SigInfo {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    pub pad: i32,
    pub fields: [u64; 14],
}

const _: () = assert!(core::mem::size_of::<SigInfo>() == 128);

// SAFETY: repr(C), integers only, no padding bytes (the explicit `pad`
// fills the gap before the 8-byte-aligned union); any bytes are a value.
unsafe impl crate::syscall::userspace::UserPod for SigInfo {}

impl SigInfo {
    fn with(sig: usize, code: i32) -> Self {
        Self {
            si_signo: sig as i32,
            si_code: code,
            ..Default::default()
        }
    }

    /// Sent by a process (kill: SI_USER; tkill and tgkill: SI_TKILL) whose
    /// pid and real user are `pid` and `uid`.
    pub fn from_process(sig: usize, code: i32, pid: u64, uid: u32) -> Self {
        let mut info = Self::with(sig, code);
        info.fields[0] = pid as u32 as u64 | (uid as u64) << 32;
        info
    }

    /// Sent by the calling process to itself or another (SI_USER), as
    /// Linux's kill and its kernel-internal send_sig(sig, task, 0) do.
    pub fn from_current(sig: usize, code: i32) -> Self {
        match super::current_process() {
            Some(p) => Self::from_process(sig, code, p.pid.0, p.credentials().ruid),
            None => Self::kernel(sig),
        }
    }

    /// Sent by the kernel itself (SI_KERNEL).
    pub fn kernel(sig: usize) -> Self {
        Self::with(sig, SI_KERNEL)
    }

    /// A child's state change (`code` is a CLD_ value): its pid, real user,
    /// status (exit code, or the signal) and CPU times in clock ticks.
    pub fn child(sig: usize, code: i32, child: &Process, status: i32) -> Self {
        let mut info = Self::from_process(sig, code, child.pid.0, child.credentials().ruid);
        info.fields[1] = status as u32 as u64;
        let (user, system) = child.cpu_times().user_system_ns();
        // clock_t in USER_HZ (100) ticks, as times() reports them.
        info.fields[2] = user / 10_000_000;
        info.fields[3] = system / 10_000_000;
        info
    }

    /// `child` ended (its exit signal is `sig`, normally SIGCHLD):
    /// CLD_EXITED with its exit code, or CLD_KILLED / CLD_DUMPED with the
    /// signal that ended it.
    pub fn child_exited(sig: usize, child: &Process) -> Self {
        let term = child.term_signal.load(Ordering::Acquire) & 0x7f;
        let (code, status) = if term == 0 {
            (CLD_EXITED, child.get_exit_code() & 0xff)
        } else if super::pcb::wait_status(0, term) & 0x80 != 0 {
            (CLD_DUMPED, term as i32)
        } else {
            (CLD_KILLED, term as i32)
        };
        Self::child(sig, code, child, status)
    }

    /// A hardware fault at `addr` (`code` is a SEGV_, FPE_, ILL_, BUS_ or
    /// TRAP_ value; SI_KERNEL when there is none).
    pub fn fault(sig: usize, code: i32, addr: u64) -> Self {
        let mut info = Self::with(sig, code);
        info.fields[0] = addr;
        info
    }

    /// The signal a queued record is for.
    pub fn signo(&self) -> usize {
        self.si_signo as u32 as usize
    }

    /// Sender's pid (kill, sigqueue, SIGCHLD).
    pub fn pid(&self) -> u32 {
        self.fields[0] as u32
    }

    /// Sender's real user (kill, sigqueue, SIGCHLD).
    pub fn uid(&self) -> u32 {
        (self.fields[0] >> 32) as u32
    }

    /// The fault address (SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP).
    pub fn addr(&self) -> u64 {
        self.fields[0]
    }

    /// The sigval of a queued signal (si_int / si_ptr).
    pub fn value(&self) -> u64 {
        self.fields[1]
    }

    /// SIGCHLD: the child's status (si_status).
    pub fn status(&self) -> i32 {
        self.fields[1] as u32 as i32
    }

    /// SIGCHLD: the child's user and system time in clock ticks.
    pub fn times(&self) -> (u64, u64) {
        (self.fields[2], self.fields[3])
    }

    /// Whether this records a hardware fault (the union holds an address).
    pub fn is_fault(&self) -> bool {
        matches!(self.si_signo, 4 | 5 | 7 | 8 | 11) && self.si_code > 0 && self.si_code < SI_KERNEL
    }
}

struct Queued {
    /// The siginfo of every pending signal instance, oldest first: one per
    /// pending standard signal (the first sender's: a standard signal sent
    /// again while pending is not queued twice), one per queued instance of
    /// a real-time signal.
    infos: alloc::collections::VecDeque<SigInfo>,
    /// Real-time instances in `infos`, each charged to `uid`.
    rt: u64,
    /// The user the queued real-time instances are charged to: the
    /// receiver's real user when the first was queued. Instances keep that
    /// charge until taken, even if the process changes its real user
    /// meanwhile.
    uid: u32,
}

/// Pending signal instances with their siginfo, beside the pending set's
/// bits: a bit stays set while any instance of its signal is queued.
/// Changed under its lock together with the bits, so a send cannot be lost
/// between a delivery taking the last instance and clearing the bit. Every
/// queued real-time instance is charged to a user (RLIMIT_SIGPENDING) until
/// it is taken, cleared or the queue is dropped.
pub struct RtQueue(spin::Mutex<Queued>);

impl RtQueue {
    pub const fn new() -> Self {
        Self(spin::Mutex::new(Queued {
            infos: alloc::collections::VecDeque::new(),
            rt: 0,
            uid: 0,
        }))
    }

    /// Make `info.si_signo` pending in `pending` with `info`, charged as
    /// `charge` says. A standard signal already pending keeps its first
    /// siginfo. A real-time signal queues another instance; past the
    /// user's limit, `Overflow::Fail` is `QueueFull` (EAGAIN) and
    /// `Overflow::Merge` sets the bit alone: the signal is delivered once
    /// more only if no instance is queued.
    pub fn push(
        &self,
        pending: &core::sync::atomic::AtomicU64,
        info: SigInfo,
        charge: Charge,
        overflow: Overflow,
    ) -> Result<(), QueueFull> {
        let sig = info.signo();
        let bit = sig_bit(sig);
        let mut q = self.0.lock();
        if sig < SIGRTMIN {
            if pending.load(Ordering::Acquire) & bit == 0 {
                // A pending bit cleared without the queue (job control)
                // leaves its record behind: drop it for the new one.
                q.infos.retain(|i| i.signo() != sig);
                q.infos.push_back(info);
                pending.fetch_or(bit, Ordering::AcqRel);
            }
            return Ok(());
        }
        let charge = if q.rt > 0 {
            Charge {
                uid: q.uid,
                ..charge
            }
        } else {
            charge
        };
        if charge_user(charge) {
            q.uid = charge.uid;
            q.rt += 1;
            q.infos.push_back(info);
        } else if overflow == Overflow::Fail {
            return Err(QueueFull);
        }
        pending.fetch_or(bit, Ordering::AcqRel);
        Ok(())
    }

    /// Take one instance of `sig` off `pending`, with its siginfo; None if
    /// it is not pending. A real-time signal's bit stays while instances
    /// remain. A signal pending without a record (one merged past the
    /// queue limit) comes with an SI_USER siginfo that names no sender, as
    /// on Linux.
    pub fn take(&self, pending: &core::sync::atomic::AtomicU64, sig: usize) -> Option<SigInfo> {
        let bit = sig_bit(sig);
        let mut q = self.0.lock();
        if pending.load(Ordering::Acquire) & bit == 0 {
            return None;
        }
        let info = match q.infos.iter().position(|i| i.signo() == sig) {
            Some(at) => {
                let info = q.infos.remove(at).unwrap_or_default();
                if sig >= SIGRTMIN {
                    q.rt -= 1;
                    uncharge_user(q.uid, 1);
                }
                info
            }
            None => SigInfo::with(sig, SI_USER),
        };
        if sig < SIGRTMIN || !q.infos.iter().any(|i| i.signo() == sig) {
            pending.fetch_and(!bit, Ordering::AcqRel);
        }
        Some(info)
    }

    /// Drop every pending instance of the signals in `bits` (exec, a
    /// discarded signal, job control).
    pub fn clear(&self, pending: &core::sync::atomic::AtomicU64, bits: u64) {
        let mut q = self.0.lock();
        let rt_before = q.infos.iter().filter(|i| i.signo() >= SIGRTMIN).count();
        q.infos.retain(|i| bits & sig_bit(i.signo()) == 0);
        let rt_after = q.infos.iter().filter(|i| i.signo() >= SIGRTMIN).count();
        let dropped = (rt_before - rt_after) as u64;
        q.rt -= dropped;
        uncharge_user(q.uid, dropped);
        pending.fetch_and(!bits, Ordering::AcqRel);
    }
}

impl Drop for RtQueue {
    fn drop(&mut self) {
        let q = self.0.get_mut();
        uncharge_user(q.uid, q.rt);
    }
}

impl Default for RtQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub const SIGKILL: usize = 9;
pub const SIGCHLD: usize = 17;
pub const SIGCONT: usize = 18;
pub const SIGSTOP: usize = 19;

/// SIGSTOP, SIGTSTP, SIGTTIN, SIGTTOU.
pub const STOP_SIGNALS: u64 = sig_bit(19) | sig_bit(20) | sig_bit(21) | sig_bit(22);

/// `sa_flags` bit: no SIGCHLD when a child stops or continues.
const SA_NOCLDSTOP: u64 = 1;

/// Signals no mask can block.
pub const UNBLOCKABLE: u64 = sig_bit(SIGKILL) | sig_bit(SIGSTOP);

/// `sigaction` handler values.
const SIG_DFL: u64 = 0;
const SIG_IGN: u64 = 1;

/// `sigaction` flags (Linux values, the same on x86_64, arm64 and RISC-V;
/// RISC-V has no SA_RESTORER, its handlers return through the kernel's
/// trampoline).
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const SA_RESTORER: u64 = 0x0400_0000;
const SA_RESTART: u64 = 0x1000_0000;
const SA_NODEFER: u64 = 0x4000_0000;
const SA_RESETHAND: u64 = 0x8000_0000;
/// Run the handler on the alternate signal stack (sigaltstack).
const SA_ONSTACK: u64 = 0x0800_0000;

/// `stack_t.ss_flags`: the thread is running on its alternate stack.
pub const SS_ONSTACK: i32 = 1;
/// `stack_t.ss_flags`: no alternate stack.
pub const SS_DISABLE: i32 = 2;
/// `stack_t.ss_flags`: disarm the alternate stack while a handler runs on
/// it, re-arming it when the handler returns (Linux 4.7).
pub const SS_AUTODISARM: i32 = i32::MIN;
/// The smallest alternate stack sigaltstack accepts: each architecture's
/// uapi MINSIGSTKSZ (arm64 5120, for its larger signal frame; x86_64 and
/// RISC-V 2048, asm-generic's).
#[cfg(target_arch = "aarch64")]
pub const MINSIGSTKSZ: u64 = 5120;
/// The smallest alternate stack sigaltstack accepts: each architecture's
/// uapi MINSIGSTKSZ (arm64 5120; x86_64 and RISC-V 2048, asm-generic's).
#[cfg(not(target_arch = "aarch64"))]
pub const MINSIGSTKSZ: u64 = 2048;

/// The fields of Linux `stack_t`: a sigaltstack request, or the state saved
/// in a signal frame's `uc_stack`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AltStackRequest {
    pub sp: u64,
    pub flags: i32,
    pub size: u64,
}

/// Why sigaltstack refused a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AltStackError {
    /// The thread is running on the alternate stack (EPERM).
    OnStack,
    /// Unknown mode bits, or a stack that wraps the address space (EINVAL).
    Invalid,
    /// Smaller than MINSIGSTKSZ (ENOMEM).
    TooSmall,
}

/// A thread's alternate signal stack (sigaltstack, N-222). Linux keeps it
/// per thread: a new thread starts without one, a fork child inherits it,
/// exec clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigAltStack {
    sp: u64,
    size: u64,
    /// The flags last set (SS_AUTODISARM is reported back as given).
    flags: i32,
}

impl Default for SigAltStack {
    fn default() -> Self {
        Self {
            sp: 0,
            size: 0,
            flags: SS_DISABLE,
        }
    }
}

impl SigAltStack {
    /// Whether `sp` is on the stack, which grows down from `sp + size`.
    fn on_stack(&self, sp: u64) -> bool {
        self.size != 0 && sp > self.sp && sp - self.sp <= self.size
    }

    /// `stack_t.ss_flags` as sigaltstack reports them for a thread whose
    /// user stack pointer is `sp`.
    pub fn flags_at(&self, sp: u64) -> i32 {
        let state = if self.size == 0 {
            SS_DISABLE
        } else if self.on_stack(sp) {
            SS_ONSTACK
        } else {
            0
        };
        state | (self.flags & SS_AUTODISARM)
    }

    /// The first address above the stack, if one is set.
    pub fn top(&self) -> Option<u64> {
        (self.size != 0).then(|| self.sp + self.size)
    }

    /// The state to report: base, flags (for stack pointer `sp`) and size.
    pub fn report(&self, sp: u64) -> AltStackRequest {
        AltStackRequest {
            sp: self.sp,
            flags: self.flags_at(sp),
            size: self.size,
        }
    }

    /// sigaltstack(`req`) by a thread whose user stack pointer is `sp`.
    pub fn set(&mut self, req: AltStackRequest, sp: u64) -> Result<(), AltStackError> {
        if self.on_stack(sp) {
            return Err(AltStackError::OnStack);
        }
        let mode = req.flags & !SS_AUTODISARM;
        if mode != 0 && mode != SS_DISABLE && mode != SS_ONSTACK {
            return Err(AltStackError::Invalid);
        }
        if mode == SS_DISABLE {
            self.sp = 0;
            self.size = 0;
        } else {
            if req.size < MINSIGSTKSZ {
                return Err(AltStackError::TooSmall);
            }
            req.sp.checked_add(req.size).ok_or(AltStackError::Invalid)?;
            self.sp = req.sp;
            self.size = req.size;
        }
        self.flags = req.flags;
        Ok(())
    }

    /// A handler is being entered: returns the state to save in the
    /// frame's `uc_stack` (the settings as made, as Linux's
    /// save_altstack_ex; rt_sigreturn restores them), and with
    /// SS_AUTODISARM disarms the stack while the handler runs.
    pub fn enter_handler(&mut self) -> AltStackRequest {
        let saved = AltStackRequest {
            sp: self.sp,
            flags: self.flags,
            size: self.size,
        };
        if self.flags & SS_AUTODISARM != 0 {
            *self = Self::default();
        }
        saved
    }
}

/// Linux EINTR, as a system call returns it (rax, x0, a0).
const EINTR_RAX: u64 = (-4i64) as u64;

/// What a signal does when it is not caught.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAction {
    Terminate,
    CoreDump,
    Ignore,
    Stop,
    Continue,
}

/// POSIX default actions.
pub const fn default_action(sig: usize) -> DefaultAction {
    match sig {
        // SIGCHLD, SIGURG, SIGWINCH
        17 | 23 | 28 => DefaultAction::Ignore,
        // SIGCONT
        18 => DefaultAction::Continue,
        // SIGSTOP, SIGTSTP, SIGTTIN, SIGTTOU
        19..=22 => DefaultAction::Stop,
        // SIGQUIT, SIGILL, SIGTRAP, SIGABRT, SIGBUS, SIGFPE, SIGSEGV,
        // SIGXCPU, SIGXFSZ, SIGSYS
        3 | 4 | 5 | 6 | 7 | 8 | 11 | 24 | 25 | 31 => DefaultAction::CoreDump,
        _ => DefaultAction::Terminate,
    }
}

/// Whether sending `sig` to `process` would only be ignored (SIG_IGN, or
/// SIG_DFL with a default action of ignore). Such a signal is dropped when
/// generated (N-98). SIGCONT's continuing happens when it is sent, so with
/// the default action nothing is left to deliver.
pub fn ignored(process: &Process, sig: usize) -> bool {
    match process.get_signal_handler(sig).unwrap_or(SIG_DFL) {
        SIG_IGN => true,
        SIG_DFL => matches!(
            default_action(sig),
            DefaultAction::Ignore | DefaultAction::Continue
        ),
        _ => false,
    }
}

/// Generation-time job-control effects of `sig` on `process`: SIGCONT
/// continues it and discards pending stops; a stop signal discards a
/// pending SIGCONT.
#[cfg(feature = "alloc")]
fn job_control_on_send(process: &Process, sig: usize) {
    if sig == SIGCONT {
        process
            .rt_queue
            .clear(&process.pending_signals, STOP_SIGNALS);
        for thread in process.threads.lock().values() {
            thread.rt_queue.clear(&thread.sigpending, STOP_SIGNALS);
        }
        // Bump first: a stop being decided right now sees the change and
        // backs out (`stop_process`).
        process.cont_seq.fetch_add(1, Ordering::SeqCst);
        if process.stop_signal.swap(0, Ordering::SeqCst) != 0 {
            process.job_report.store(0xffff, Ordering::Release);
            notify_parent_job(process, CLD_CONTINUED, SIGCONT as i32);
        }
        crate::sched::dispatch::PROCESS_EVENTS.wake_all();
    } else if sig_bit(sig) & STOP_SIGNALS != 0 {
        let cont = sig_bit(SIGCONT);
        process.rt_queue.clear(&process.pending_signals, cont);
        for thread in process.threads.lock().values() {
            thread.rt_queue.clear(&thread.sigpending, cont);
        }
    }
}

/// Tell `process`'s parent that it stopped or continued (`code`
/// CLD_STOPPED with the stop signal, or CLD_CONTINUED with SIGCONT, as
/// `status`): SIGCHLD unless the parent asked for none (SA_NOCLDSTOP), and
/// a wakeup for `wait`.
#[cfg(feature = "alloc")]
fn notify_parent_job(process: &Process, code: i32, status: i32) {
    if let Some(parent) = process.parent().and_then(super::table::get_process) {
        let nocldstop = parent.signal_action_extra.lock()[SIGCHLD].0 & SA_NOCLDSTOP != 0;
        if !nocldstop {
            notify_info(&parent, SigInfo::child(SIGCHLD, code, process, status));
        }
    }
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
}

/// Stop `process` with `sig`, unless a SIGCONT came after `seq` was read
/// (the stop signal was taken before it, so the continue wins).
#[cfg(feature = "alloc")]
fn stop_process(process: &Process, sig: usize, seq: u32) {
    if process
        .stop_signal
        .compare_exchange(0, sig as u32, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // already stopped
    }
    if process.cont_seq.load(Ordering::SeqCst) != seq {
        let _ =
            process
                .stop_signal
                .compare_exchange(sig as u32, 0, Ordering::SeqCst, Ordering::SeqCst);
        return;
    }
    process
        .job_report
        .store(0x7f | ((sig as u32) << 8), Ordering::Release);
    notify_parent_job(process, CLD_STOPPED, sig as i32);
}

/// Park the calling thread while its process is stopped; returns once it
/// is continued or has a fatal signal to act on.
#[cfg(feature = "alloc")]
pub fn park_while_stopped() {
    let stopped = || {
        super::current_process().is_some_and(|p| {
            p.stop_signal.load(Ordering::Acquire) != 0
                && p.kill_pending.load(Ordering::Acquire) == 0
        })
    };
    while stopped() {
        crate::sched::dispatch::PROCESS_EVENTS.wait_until(|| !stopped());
    }
}

/// Send `sig` to a dispatched process from the kernel (SI_KERNEL); see
/// [`send_info_to_process`].
#[cfg(feature = "alloc")]
pub fn send_to_process(process: &Process, sig: usize) -> Result<bool, crate::error::KernelError> {
    send_info_to_process(process, SigInfo::kernel(sig))
}

/// The overflow rule for a send with `info` (Linux's override_rlimit): a
/// queued signal from a process (sigqueue, tkill: a negative si_code) fails
/// with EAGAIN past RLIMIT_SIGPENDING; kill and the kernel's own signals
/// still pend.
fn overflow_of(info: &SigInfo) -> Overflow {
    if info.si_code < 0 {
        Overflow::Fail
    } else {
        Overflow::Merge
    }
}

/// Send `info.si_signo` with `info` to a dispatched process. `Ok(false)` if
/// the process has no running threads (the caller falls back to the old
/// path); `WouldBlock` (EAGAIN) if a queued real-time signal would exceed
/// RLIMIT_SIGPENDING (see [`overflow_of`]).
#[cfg(feature = "alloc")]
pub fn send_info_to_process(
    process: &Process,
    info: SigInfo,
) -> Result<bool, crate::error::KernelError> {
    use crate::sched::dispatch;

    let sig = info.signo();
    let tasks = dispatch::tasks_of(process.pid.0);
    if tasks.is_empty() {
        return Ok(false);
    }
    job_control_on_send(process, sig);
    if sig == SIGKILL {
        super::exit::set_kill_pending(process, SIGKILL as u32);
    } else if ignored(process, sig) {
        return Ok(true);
    } else {
        process
            .rt_queue
            .push(
                &process.pending_signals,
                info,
                Charge::of(process),
                overflow_of(&info),
            )
            .map_err(|_| crate::error::KernelError::WouldBlock)?;
        crate::fs::signalfd::signal_generated();
    }
    for task in &tasks {
        dispatch::notify(task);
    }
    dispatch::PROCESS_EVENTS.wake_all();
    Ok(true)
}

/// Send `sig` to `process` from inside the kernel (terminal signals to a
/// foreground group, resource limits): see [`notify_info`].
pub fn notify(process: &Process, sig: usize) {
    notify_info(process, SigInfo::kernel(sig));
}

/// Send `info` to `process` from inside the kernel (SIGCHLD to a parent,
/// with the child's state): through the dispatched path when the process
/// runs on its own threads, so an ignored signal is dropped instead of
/// pending and interrupting waits (N-98).
pub fn notify_info(process: &Process, info: SigInfo) {
    #[cfg(feature = "alloc")]
    if process.dispatched.load(Ordering::Acquire)
        && send_info_to_process(process, info).unwrap_or(true)
    {
        return;
    }
    let _ = process.send_signal_info(info);
}

/// Send `sig` to the calling thread from inside the kernel (Linux's
/// send_sig(sig, current, 0): SI_USER from the caller itself): SIGXFSZ for
/// a write past RLIMIT_FSIZE, SIGPIPE.
pub fn send_to_current(sig: usize) {
    let (Some(process), Some(thread)) = (super::current_process(), super::current_thread()) else {
        return;
    };
    let info = SigInfo::from_process(sig, SI_USER, process.pid.0, process.credentials().ruid);
    #[cfg(feature = "alloc")]
    if process.dispatched.load(Ordering::Acquire) {
        let _ = send_to_thread(&process, &thread, info);
        return;
    }
    let _ = thread;
    let _ = process.send_signal_info(info);
}

/// Send `info.si_signo` with `info` to one thread of a dispatched process
/// (tkill, tgkill, rt_tgsigqueueinfo); `WouldBlock` (EAGAIN) if a queued
/// real-time signal would exceed RLIMIT_SIGPENDING (see [`overflow_of`]).
#[cfg(feature = "alloc")]
pub fn send_to_thread(
    process: &Process,
    thread: &Thread,
    info: SigInfo,
) -> Result<(), crate::error::KernelError> {
    let sig = info.signo();
    if sig == SIGKILL {
        send_info_to_process(process, info)?;
        return Ok(());
    }
    job_control_on_send(process, sig);
    if ignored(process, sig) {
        return Ok(());
    }
    thread
        .rt_queue
        .push(
            &thread.sigpending,
            info,
            Charge::of(process),
            overflow_of(&info),
        )
        .map_err(|_| crate::error::KernelError::WouldBlock)?;
    crate::fs::signalfd::signal_generated();
    for task in crate::sched::dispatch::tasks_of(process.pid.0) {
        if task.owner() == Some((process.pid.0, thread.tid.0)) {
            crate::sched::dispatch::notify(&task);
        }
    }
    Ok(())
}

/// Signals `thread` of `process` could take now (pending, not blocked).
pub fn deliverable(process: &Process, thread: &Thread) -> u64 {
    let pending =
        thread.sigpending.load(Ordering::Acquire) | process.pending_signals.load(Ordering::Acquire);
    pending & !thread.sigmask.load(Ordering::Acquire)
}

/// Whether the calling thread has a signal pending -- its own or its
/// process's -- in `mask`, blocked or not (signalfd readiness).
pub fn pending_in(mask: u64) -> bool {
    let (Some(process), Some(thread)) = (super::current_process(), super::current_thread()) else {
        return false;
    };
    (thread.sigpending.load(Ordering::Acquire) | process.pending_signals.load(Ordering::Acquire))
        & mask
        != 0
}

/// Take the lowest signal of `ready` pending for `thread` -- its own
/// instance first, then its process's -- with its siginfo. A real-time
/// signal's instances come one at a time, oldest first (N-209). None if
/// another thread took it first.
fn take_lowest(process: &Process, thread: &Thread, ready: u64) -> Option<SigInfo> {
    if ready == 0 {
        return None;
    }
    let sig = ready.trailing_zeros() as usize + 1;
    thread
        .rt_queue
        .take(&thread.sigpending, sig)
        .or_else(|| process.rt_queue.take(&process.pending_signals, sig))
}

/// Take the calling thread's lowest pending signal in `mask`, blocked or
/// not, as a signalfd read or sigtimedwait does: it is consumed, not
/// delivered.
pub fn take_pending_in(mask: u64) -> Option<SigInfo> {
    let process = super::current_process()?;
    let thread = super::current_thread()?;
    let pending =
        thread.sigpending.load(Ordering::Acquire) | process.pending_signals.load(Ordering::Acquire);
    take_lowest(&process, &thread, pending & mask)
}

/// Replace the calling thread's mask (UNBLOCKABLE is never blocked).
/// Returns the old mask. The process copy follows the most recent thread
/// to change it, for readers that know only processes (procfs).
pub fn set_mask(process: &Process, thread: &Thread, mask: u64) -> u64 {
    let mask = mask & !UNBLOCKABLE;
    process.signal_mask.store(mask, Ordering::Release);
    thread.sigmask.swap(mask, Ordering::AcqRel)
}

/// Take the lowest deliverable signal off the pending sets, with its
/// siginfo.
fn dequeue(process: &Process, thread: &Thread) -> Option<SigInfo> {
    take_lowest(process, thread, deliverable(process, thread))
}

#[cfg(target_arch = "x86_64")]
pub use frame::{deliver_on_return, rt_sigreturn, MIN_SIGNAL_FRAME};
#[cfg(all(
    feature = "alloc",
    target_os = "none",
    any(target_arch = "aarch64", target_arch = "riscv64")
))]
pub use generic::{deliver_on_return, rt_sigreturn, MIN_SIGNAL_FRAME};

/// Queue a synchronous fault signal (`info`) for the current thread if it
/// has a handler the signal is not blocked for, so the handler runs on the
/// way back to user mode (SIGSEGV handlers, SIGFPE handlers...). Returns
/// false when the fault must kill the process instead: the default action,
/// ignored, or blocked (Linux forces the default for a blocked or ignored
/// synchronous signal).
pub fn queue_fault(info: SigInfo) -> bool {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        if let (Some(process), Some(thread)) = (super::current_process(), super::current_thread()) {
            let sig = info.signo();
            let handler = process.get_signal_handler(sig).unwrap_or(0);
            let blocked = thread.sigmask.load(Ordering::Acquire) & sig_bit(sig) != 0;
            if handler > SIG_IGN && !blocked {
                let _ = thread.rt_queue.push(
                    &thread.sigpending,
                    info,
                    Charge::of(&process),
                    Overflow::Merge,
                );
                return true;
            }
        }
    }
    let _ = info;
    false
}

/// Signal delivery to handlers on AArch64 and RISC-V (sprint E), with
/// Linux's frames, so musl works unmodified: the decision loop is
/// x86_64's (`frame::deliver_on_return`); the frame is each architecture's
/// `struct rt_sigframe`.
///
/// - **AArch64**: `siginfo`, then a `ucontext` whose `sigcontext` holds x0-x30,
///   sp, pc and pstate, with an `fpsimd_context` record (v0-v31, FPSR, FPCR)
///   and a terminator in its 4 KiB `__reserved` area; a frame record (the
///   interrupted x29, x30) follows, and x29 points at it. The handler returns
///   to the C library's SA_RESTORER (musl `__restore_rt`).
/// - **RISC-V**: `siginfo`, then a `ucontext` whose `sigcontext` holds pc and
///   x1-x31 and the D-extension state (f0-f31, fcsr). RISC-V has no
///   SA_RESTORER: Linux returns through its vDSO; here the kernel maps one page
///   with `li a7, 139; ecall` into the process the first time it enters a
///   handler (`Process::sigtramp`).
///
/// A system call a signal interrupted with EINTR is restarted with
/// SA_RESTART by putting its first argument back (the return value
/// overwrote it; the exception path passes it in) and rewinding the 4-byte
/// `svc`/`ecall`.
#[cfg(all(
    feature = "alloc",
    target_os = "none",
    any(target_arch = "aarch64", target_arch = "riscv64")
))]
mod generic {
    use alloc::boxed::Box;

    use super::*;
    #[cfg(target_arch = "aarch64")]
    use crate::arch::aarch64::switch::UserFrame;
    #[cfg(target_arch = "riscv64")]
    use crate::arch::riscv64::switch::UserFrame;
    use crate::syscall::userspace::{is_user_address, read_user_bytes, write_user_bytes};

    /// Linux `stack_t`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct StackT {
        ss_sp: u64,
        ss_flags: i32,
        pad: i32,
        ss_size: u64,
    }

    /// arm64 `struct sigcontext`: 4384 bytes.
    #[cfg(target_arch = "aarch64")]
    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    struct SigContext {
        fault_address: u64,
        regs: [u64; 31],
        sp: u64,
        pc: u64,
        pstate: u64,
        reserved: [u8; 4096],
    }

    /// RISC-V `struct sigcontext`: pc and x1-x31, then the FP state (the
    /// 528-byte `union __riscv_fp_state`, D layout: f0-f31 and fcsr).
    #[cfg(target_arch = "riscv64")]
    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    struct SigContext {
        regs: [u64; 32],
        fpregs: [u64; 32],
        fcsr: u32,
        fp_reserved: [u32; 67],
    }

    /// Linux `struct ucontext` (arm64 and RISC-V share the layout): the
    /// signal mask is padded to 128 bytes (glibc's 1024-bit sigset).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UContext {
        uc_flags: u64,
        uc_link: u64,
        uc_stack: StackT,
        uc_sigmask: u64,
        unused: [u64; 15],
        uc_mcontext: SigContext,
    }

    /// Linux `struct rt_sigframe` for both.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RtSigFrame {
        info: SigInfo,
        uc: UContext,
    }

    #[cfg(target_arch = "aarch64")]
    const _: () = {
        assert!(core::mem::size_of::<SigContext>() == 4384);
        assert!(core::mem::size_of::<UContext>() == 4560);
        assert!(core::mem::offset_of!(UContext, uc_mcontext) == 176);
        assert!(core::mem::size_of::<RtSigFrame>() == 4688);
    };
    #[cfg(target_arch = "riscv64")]
    const _: () = {
        assert!(core::mem::size_of::<SigContext>() == 784);
        assert!(core::mem::size_of::<UContext>() == 960);
        assert!(core::mem::offset_of!(UContext, uc_mcontext) == 176);
        assert!(core::mem::size_of::<RtSigFrame>() == 1088);
    };

    /// arm64: the frame record after the sigframe (x29, x30 of the
    /// interrupted code), as Linux places it.
    #[cfg(target_arch = "aarch64")]
    const RECORD: usize = 16;
    #[cfg(target_arch = "riscv64")]
    const RECORD: usize = 0;

    /// arm64 `struct fpsimd_context` magic and size.
    #[cfg(target_arch = "aarch64")]
    const FPSIMD_MAGIC: u32 = 0x4650_8001;
    #[cfg(target_arch = "aarch64")]
    const FPSIMD_SIZE: u32 = 528;

    /// The stack one signal frame takes, rounded to 16: reported as
    /// AT_MINSIGSTKSZ.
    pub const MIN_SIGNAL_FRAME: u64 =
        ((core::mem::size_of::<RtSigFrame>() + RECORD + 15) as u64) & !15;

    /// RISC-V: `li a7, 139` (rt_sigreturn); `ecall`.
    #[cfg(target_arch = "riscv64")]
    const TRAMPOLINE: [u8; 8] = [0x93, 0x08, 0xB0, 0x08, 0x73, 0x00, 0x00, 0x00];

    #[cfg(target_arch = "aarch64")]
    fn pc(f: &UserFrame) -> u64 {
        f.elr
    }
    #[cfg(target_arch = "riscv64")]
    fn pc(f: &UserFrame) -> u64 {
        f.sepc
    }
    #[cfg(target_arch = "aarch64")]
    fn set_pc(f: &mut UserFrame, v: u64) {
        f.elr = v;
    }
    #[cfg(target_arch = "riscv64")]
    fn set_pc(f: &mut UserFrame, v: u64) {
        f.sepc = v;
    }
    #[cfg(target_arch = "aarch64")]
    fn sp(f: &UserFrame) -> u64 {
        f.sp
    }
    #[cfg(target_arch = "riscv64")]
    fn sp(f: &UserFrame) -> u64 {
        f.x[2]
    }
    /// The register a system call returns in, and the first argument.
    #[cfg(target_arch = "aarch64")]
    const RET: usize = 0;
    #[cfg(target_arch = "riscv64")]
    const RET: usize = 10;

    /// A zeroed frame on the heap (4.7 KiB on AArch64: too much for a
    /// kernel stack).
    fn zeroed_frame() -> Box<RtSigFrame> {
        // SAFETY: RtSigFrame is plain data (integers and byte arrays), for
        // which all-zero bytes are a valid value.
        unsafe { Box::<RtSigFrame>::new_zeroed().assume_init() }
    }

    fn bytes_of(frame: &RtSigFrame) -> &[u8] {
        // SAFETY: plain data, every byte initialised (built from zero).
        unsafe {
            core::slice::from_raw_parts(
                frame as *const RtSigFrame as *const u8,
                core::mem::size_of::<RtSigFrame>(),
            )
        }
    }

    fn bytes_of_mut(frame: &mut RtSigFrame) -> &mut [u8] {
        // SAFETY: as above; any bytes form a valid value.
        unsafe {
            core::slice::from_raw_parts_mut(
                frame as *mut RtSigFrame as *mut u8,
                core::mem::size_of::<RtSigFrame>(),
            )
        }
    }

    /// RISC-V: the address of this process's sigreturn trampoline, mapping
    /// it (read + execute, user) the first time, or again if the recorded
    /// page no longer holds it (exec replaced the address space).
    #[cfg(target_arch = "riscv64")]
    fn sigtramp(process: &Process) -> Option<u64> {
        use crate::mm::{vas::MappingType, PageFlags};
        let at = process.sigtramp.load(Ordering::Acquire);
        if at != 0 {
            let mut code = [0u8; 8];
            if read_user_bytes(at as usize, &mut code).is_ok() && code == TRAMPOLINE {
                return Some(at);
            }
        }
        let vas = process.memory_space.lock();
        let page = vas.reserve_mmap_area(4096).ok()?;
        vas.map_region_flags(
            page,
            4096,
            MappingType::Code,
            Some(PageFlags::PRESENT | PageFlags::USER),
        )
        .ok()?;
        // SAFETY: the process's root, through the physical map; only read.
        let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(vas.get_page_table()) };
        let (frame, _) = mapper.translate_page(page).ok()?;
        let dst = crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
        // SAFETY: the page just mapped for the trampoline, unused by
        // anything else; 8 bytes fit.
        unsafe { core::ptr::copy_nonoverlapping(TRAMPOLINE.as_ptr(), dst, TRAMPOLINE.len()) };
        vas.sync_code(page.as_u64(), TRAMPOLINE.len() as u64);
        process.sigtramp.store(page.as_u64(), Ordering::Release);
        Some(page.as_u64())
    }

    /// Build the handler frame for `info`'s signal on the user stack and
    /// point `f` at the handler. `syscall` is the interrupted system call's
    /// first argument, if this return is from one. Returns false if the
    /// frame cannot be written (the caller kills the thread with SIGSEGV).
    #[allow(clippy::too_many_arguments)]
    fn setup_frame(
        f: &mut UserFrame,
        syscall: Option<u64>,
        process: &Process,
        thread: &Thread,
        info: SigInfo,
        handler: u64,
        flags: u64,
        restorer: u64,
        sa_mask: u64,
    ) -> bool {
        let sig = info.signo();
        if !is_user_address(handler as usize) {
            return false;
        }
        #[cfg(target_arch = "aarch64")]
        let ret_to = {
            if flags & SA_RESTORER == 0 || !is_user_address(restorer as usize) {
                return false;
            }
            restorer
        };
        #[cfg(target_arch = "riscv64")]
        let ret_to = {
            let _ = restorer;
            match sigtramp(process) {
                Some(at) => at,
                None => return false,
            }
        };

        // SA_RESTART: run the interrupted call again, with its first
        // argument back in place of the EINTR it returned, unless it is one
        // that returns EINTR after a handler regardless (`no_restart`).
        let no_restart = thread.no_restart.swap(false, Ordering::AcqRel);
        if let Some(arg0) = syscall {
            if f.x[RET] == EINTR_RAX && flags & SA_RESTART != 0 && !no_restart {
                f.x[RET] = arg0;
                set_pc(f, pc(f).wrapping_sub(4));
            }
        }

        // The stack: with SA_ONSTACK the alternate one, from its top, if it
        // is armed and the thread is not already on it; otherwise the
        // interrupted one (neither ABI has a red zone).
        let alt = *thread.altstack.lock();
        let state = alt.flags_at(sp(f));
        let switch = flags & SA_ONSTACK != 0 && state & (SS_DISABLE | SS_ONSTACK) == 0;
        let base = match (switch, alt.top()) {
            (true, Some(top)) => top,
            _ => sp(f),
        };
        let size = (core::mem::size_of::<RtSigFrame>() + RECORD) as u64;
        let Some(frame_addr) = base.checked_sub(size).map(|a| a & !15) else {
            return false;
        };
        if !is_user_address(frame_addr as usize) || frame_addr < 0x1000 {
            return false;
        }
        if (switch || state & SS_ONSTACK != 0) && alt.flags_at(frame_addr) & SS_ONSTACK == 0 {
            return false;
        }

        let old_mask = if thread.has_saved_sigmask.swap(false, Ordering::AcqRel) {
            thread.saved_sigmask.load(Ordering::Acquire)
        } else {
            thread.sigmask.load(Ordering::Acquire)
        };
        let saved_alt = thread.altstack.lock().enter_handler();
        let mut frame = zeroed_frame();
        frame.info = info;
        frame.uc.uc_stack = StackT {
            ss_sp: saved_alt.sp,
            ss_flags: saved_alt.flags,
            pad: 0,
            ss_size: saved_alt.size,
        };
        frame.uc.uc_sigmask = old_mask;
        let mc = &mut frame.uc.uc_mcontext;
        #[cfg(target_arch = "aarch64")]
        {
            mc.fault_address = if info.is_fault() { info.addr() } else { 0 };
            mc.regs = f.x;
            mc.sp = f.sp;
            mc.pc = f.elr;
            mc.pstate = f.spsr;
            // The user's FP/SIMD registers: the kernel is soft-float, so
            // the CPU still holds them.
            let mut fp = crate::arch::aarch64::context::initial_fpu();
            crate::arch::aarch64::context::save_fpu_state(&mut fp);
            let r = &mut mc.reserved;
            r[0..4].copy_from_slice(&FPSIMD_MAGIC.to_le_bytes());
            r[4..8].copy_from_slice(&FPSIMD_SIZE.to_le_bytes());
            r[8..12].copy_from_slice(&fp.fpsr.to_le_bytes());
            r[12..16].copy_from_slice(&fp.fpcr.to_le_bytes());
            for (i, v) in fp.v.iter().enumerate() {
                let at = 16 + i * 16;
                r[at..at + 8].copy_from_slice(&v[0].to_le_bytes());
                r[at + 8..at + 16].copy_from_slice(&v[1].to_le_bytes());
            }
            // The terminator record after it is already zero.
        }
        #[cfg(target_arch = "riscv64")]
        {
            mc.regs = f.x;
            mc.regs[0] = f.sepc;
            if crate::arch::riscv64::fpu::present() {
                let mut fp = crate::arch::riscv64::fpu::initial();
                crate::arch::riscv64::fpu::save(&mut fp);
                for (d, s) in mc.fpregs.iter_mut().zip(fp.f.iter()) {
                    *d = s.to_bits();
                }
                mc.fcsr = fp.fcsr;
            }
        }
        if write_user_bytes(frame_addr as usize, bytes_of(&frame)).is_err() {
            return false;
        }
        let info_addr = frame_addr + core::mem::offset_of!(RtSigFrame, info) as u64;
        let uc_addr = frame_addr + core::mem::offset_of!(RtSigFrame, uc) as u64;

        // Enter the handler: handler(sig, &info, &uc) on the new frame,
        // returning to the restorer / trampoline.
        #[cfg(target_arch = "aarch64")]
        {
            let record = frame_addr + core::mem::size_of::<RtSigFrame>() as u64;
            let mut rec = [0u8; 16];
            rec[0..8].copy_from_slice(&f.x[29].to_le_bytes());
            rec[8..16].copy_from_slice(&f.x[30].to_le_bytes());
            if write_user_bytes(record as usize, &rec).is_err() {
                return false;
            }
            f.x[0] = sig as u64;
            f.x[1] = info_addr;
            f.x[2] = uc_addr;
            f.x[29] = record;
            f.x[30] = ret_to;
            f.sp = frame_addr;
            f.elr = handler;
        }
        #[cfg(target_arch = "riscv64")]
        {
            f.x[10] = sig as u64;
            f.x[11] = info_addr;
            f.x[12] = uc_addr;
            f.x[1] = ret_to;
            f.x[2] = frame_addr;
            f.sepc = handler;
        }

        // Block sa_mask, and the signal itself unless SA_NODEFER.
        let mut blocked = thread.sigmask.load(Ordering::Acquire) | sa_mask;
        if flags & SA_NODEFER == 0 {
            blocked |= sig_bit(sig);
        }
        set_mask(process, thread, blocked);
        if flags & SA_RESETHAND != 0 {
            let _ = process.set_signal_handler(sig, SIG_DFL);
            process.signal_action_extra.lock()[sig] = (0, 0, 0);
        }
        true
    }

    /// Act on pending signals on the way back to user mode: a fatal one ends
    /// the thread's process, a stop stops it, a caught one gets a handler
    /// frame (one per return). `syscall` is the first argument of the system
    /// call this return ends, if it is one (for SA_RESTART). Never returns
    /// when the process is terminated.
    pub fn deliver_on_return(f: &mut UserFrame, mut syscall: Option<u64>) {
        loop {
            park_while_stopped();
            super::super::user_return_check();
            let fatal = {
                let (Some(process), Some(thread)) = (
                    super::super::current_process(),
                    super::super::current_thread(),
                ) else {
                    return;
                };
                let seq = process.cont_seq.load(Ordering::SeqCst);
                let Some(info) = dequeue(&process, &thread) else {
                    thread.no_restart.store(false, Ordering::Release);
                    if thread.has_saved_sigmask.swap(false, Ordering::AcqRel) {
                        set_mask(
                            &process,
                            &thread,
                            thread.saved_sigmask.load(Ordering::Acquire),
                        );
                    }
                    return;
                };
                let sig = info.signo();
                let handler = process.get_signal_handler(sig).unwrap_or(SIG_DFL);
                match handler {
                    SIG_IGN => None,
                    SIG_DFL => match default_action(sig) {
                        DefaultAction::Terminate | DefaultAction::CoreDump => Some(sig),
                        DefaultAction::Stop => {
                            // The interrupted call runs again once the
                            // process continues, as on Linux.
                            thread.no_restart.store(false, Ordering::Release);
                            if let Some(arg0) = syscall.take() {
                                if f.x[RET] == EINTR_RAX {
                                    f.x[RET] = arg0;
                                    set_pc(f, pc(f).wrapping_sub(4));
                                }
                            }
                            stop_process(&process, sig, seq);
                            None
                        }
                        _ => None,
                    },
                    addr => {
                        let (flags, restorer, mask) = process.signal_action_extra.lock()[sig];
                        if setup_frame(
                            f, syscall, &process, &thread, info, addr, flags, restorer, mask,
                        ) {
                            return;
                        }
                        Some(11)
                    }
                }
            };
            if let Some(sig) = fatal {
                let _ = crate::syscall::process::exit_current(0, sig as u32);
            }
        }
    }

    /// rt_sigreturn: restore the context `setup_frame` saved into the
    /// exception frame `f`. The frame is at the user stack pointer (the
    /// handler returned with it unchanged). Returns the restored first
    /// register (what the system call "returns"), or None if the frame is
    /// unusable (the caller kills the thread with SIGSEGV, as Linux does).
    pub fn rt_sigreturn(f: &mut UserFrame) -> Option<u64> {
        let process = super::super::current_process()?;
        let thread = super::super::current_thread()?;
        let frame_addr = sp(f);
        if frame_addr & 15 != 0 {
            return None;
        }
        let mut frame = zeroed_frame();
        read_user_bytes(frame_addr as usize, bytes_of_mut(&mut frame)).ok()?;
        let mc = frame.uc.uc_mcontext;
        #[cfg(target_arch = "aarch64")]
        {
            if !is_user_address(mc.pc as usize) || !is_user_address(mc.sp as usize) {
                return None;
            }
            // The FP/SIMD record first (it can fail).
            let r = &mc.reserved;
            let word = |at: usize| u32::from_le_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]]);
            if word(0) != FPSIMD_MAGIC || word(4) != FPSIMD_SIZE {
                return None;
            }
            let mut fp = crate::arch::aarch64::context::initial_fpu();
            fp.fpsr = word(8);
            fp.fpcr = word(12);
            for (i, v) in fp.v.iter_mut().enumerate() {
                let at = 16 + i * 16;
                let q = |o: usize| {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&r[o..o + 8]);
                    u64::from_le_bytes(b)
                };
                *v = [q(at), q(at + 8)];
            }
            crate::arch::aarch64::context::restore_fpu_state(&fp);
            f.x = mc.regs;
            f.sp = mc.sp;
            f.elr = mc.pc;
            // Only the condition flags come back; the exception return
            // forces EL0 with interrupts unmasked (`switch::user_spsr`).
            f.spsr = mc.pstate & 0xF000_0000;
        }
        #[cfg(target_arch = "riscv64")]
        {
            if !is_user_address(mc.regs[0] as usize) || !is_user_address(mc.regs[2] as usize) {
                return None;
            }
            if crate::arch::riscv64::fpu::present() {
                let mut fp = crate::arch::riscv64::fpu::initial();
                for (d, s) in fp.f.iter_mut().zip(mc.fpregs.iter()) {
                    *d = f64::from_bits(*s);
                }
                fp.fcsr = mc.fcsr;
                crate::arch::riscv64::fpu::restore(&fp);
            }
            let sepc = mc.regs[0];
            f.x = mc.regs;
            f.x[0] = 0;
            f.sepc = sepc;
        }
        set_mask(&process, &thread, frame.uc.uc_sigmask);
        let uc_stack = frame.uc.uc_stack;
        let _ = thread.altstack.lock().set(
            AltStackRequest {
                sp: uc_stack.ss_sp,
                flags: uc_stack.ss_flags,
                size: uc_stack.ss_size,
            },
            sp(f),
        );
        Some(f.x[RET])
    }
}

#[cfg(target_arch = "x86_64")]
mod frame {
    use super::*;
    use crate::arch::x86_64::trap::{
        is_user_address, sanitize_user_frame, TrapFrame, SYSCALL_VECTOR,
    };

    /// Linux `struct sigcontext` (x86_64), 256 bytes.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct SigContext {
        pub r8: u64,
        pub r9: u64,
        pub r10: u64,
        pub r11: u64,
        pub r12: u64,
        pub r13: u64,
        pub r14: u64,
        pub r15: u64,
        pub rdi: u64,
        pub rsi: u64,
        pub rbp: u64,
        pub rbx: u64,
        pub rdx: u64,
        pub rax: u64,
        pub rcx: u64,
        pub rsp: u64,
        pub rip: u64,
        pub eflags: u64,
        pub cs: u16,
        pub gs: u16,
        pub fs: u16,
        pub ss: u16,
        pub err: u64,
        pub trapno: u64,
        pub oldmask: u64,
        pub cr2: u64,
        pub fpstate: u64,
        pub reserved: [u64; 8],
    }

    /// Linux `stack_t`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct StackT {
        pub ss_sp: u64,
        pub ss_flags: i32,
        pub pad: i32,
        pub ss_size: u64,
    }

    /// Linux kernel `struct ucontext` (x86_64).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct UContext {
        pub uc_flags: u64,
        pub uc_link: u64,
        pub uc_stack: StackT,
        pub uc_mcontext: SigContext,
        pub uc_sigmask: u64,
    }

    /// Linux `struct rt_sigframe` (x86_64): the handler's return address
    /// first, so the frame address is the handler's entry RSP.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct RtSigFrame {
        pub pretcode: u64,
        pub uc: UContext,
        pub info: SigInfo,
    }

    const _: () = {
        assert!(core::mem::size_of::<SigContext>() == 256);
        assert!(core::mem::size_of::<StackT>() == 24);
        assert!(core::mem::size_of::<UContext>() == 304);
        assert!(core::mem::size_of::<SigInfo>() == 128);
        assert!(core::mem::size_of::<RtSigFrame>() == 440);
    };

    /// Bytes below the interrupted RSP that x86_64 code may use without
    /// moving RSP (System V red zone).
    const RED_ZONE: u64 = 128;
    /// FXSAVE image size.
    const FXSAVE_SIZE: usize = 512;

    /// The most stack one signal frame takes (`setup_frame`): the FXSAVE
    /// image with up to 63 bytes of alignment, the frame, and the alignment
    /// that leaves RSP + 8 16-byte aligned, rounded up to 16. Reported to
    /// programs as AT_MINSIGSTKSZ, as Linux does on x86.
    pub const MIN_SIGNAL_FRAME: u64 =
        ((FXSAVE_SIZE + 63 + core::mem::size_of::<RtSigFrame>() + 15 + 8) as u64)
            .next_multiple_of(16);

    /// A 16-byte-aligned FXSAVE buffer.
    #[repr(C, align(16))]
    struct FxArea([u8; FXSAVE_SIZE]);

    fn bytes_of<T: Copy>(v: &T) -> &[u8] {
        // SAFETY: `T` is a repr(C) plain-data struct; viewing it as bytes
        // reads only initialised memory (all fields are integers, and the
        // Default/explicit constructors write every byte).
        unsafe {
            core::slice::from_raw_parts(v as *const T as *const u8, core::mem::size_of::<T>())
        }
    }

    /// Build the handler frame for `info`'s signal on the user stack and
    /// point `f` at the handler. Returns false if the frame cannot be
    /// written (the caller then kills the thread with SIGSEGV).
    fn setup_frame(
        f: &mut TrapFrame,
        process: &Process,
        thread: &Thread,
        info: SigInfo,
        handler: u64,
        flags: u64,
        restorer: u64,
        sa_mask: u64,
    ) -> bool {
        let sig = info.signo();
        if flags & SA_RESTORER == 0 || !is_user_address(restorer) || !is_user_address(handler) {
            return false;
        }

        // A system call the signal interrupted is restarted with SA_RESTART
        // (rewind over the 2-byte `syscall`, put the number back in rax),
        // unless it is one that returns EINTR after a handler regardless
        // (`no_restart`); otherwise it returns EINTR. The context saved
        // below must be the one the handler returns to.
        let no_restart = thread.no_restart.swap(false, Ordering::AcqRel);
        if f.vector == SYSCALL_VECTOR
            && f.rax == EINTR_RAX
            && flags & SA_RESTART != 0
            && !no_restart
        {
            f.rax = f.error_code;
            f.rip = f.rip.wrapping_sub(2);
        }

        // The stack: with SA_ONSTACK the alternate one, from its top, if it
        // is armed and the thread is not already on it (N-222); otherwise
        // the interrupted one, below its red zone.
        let alt = *thread.altstack.lock();
        let state = alt.flags_at(f.rsp);
        let switch = flags & SA_ONSTACK != 0 && state & (SS_DISABLE | SS_ONSTACK) == 0;
        let base = match (switch, alt.top()) {
            (true, Some(top)) => top,
            _ => match f.rsp.checked_sub(RED_ZONE) {
                Some(below_red_zone) => below_red_zone,
                None => return false,
            },
        };

        // Layout: FXSAVE image (16-aligned; 64 for headroom), then the
        // frame, aligned so that RSP + 8 is 16-aligned at handler entry, as
        // after a call.
        let Some(below) = base.checked_sub(FXSAVE_SIZE as u64) else {
            return false;
        };
        let fp_addr = below & !63;
        let Some(frame_end) = fp_addr.checked_sub(core::mem::size_of::<RtSigFrame>() as u64) else {
            return false;
        };
        let frame_addr = (frame_end & !15).wrapping_sub(8);
        if !is_user_address(frame_addr) || frame_addr < 0x1000 {
            return false;
        }
        // A frame on the alternate stack must fit in it (Linux: a nested
        // signal stack overflow is SIGSEGV).
        if (switch || state & SS_ONSTACK != 0) && alt.flags_at(frame_addr) & SS_ONSTACK == 0 {
            return false;
        }

        // The user's x87/SSE state: the kernel is soft-float, so the CPU
        // still holds it.
        let mut fx = FxArea([0; FXSAVE_SIZE]);
        // SAFETY: FXSAVE64 writes 512 bytes to a 16-byte-aligned buffer.
        unsafe {
            core::arch::asm!("fxsave64 [{}]", in(reg) fx.0.as_mut_ptr(), options(nostack, preserves_flags));
        }
        if crate::syscall::userspace::write_user_bytes(fp_addr as usize, &fx.0).is_err() {
            return false;
        }

        let old_mask = if thread.has_saved_sigmask.swap(false, Ordering::AcqRel) {
            thread.saved_sigmask.load(Ordering::Acquire)
        } else {
            thread.sigmask.load(Ordering::Acquire)
        };
        let sc = SigContext {
            r8: f.r8,
            r9: f.r9,
            r10: f.r10,
            r11: f.r11,
            r12: f.r12,
            r13: f.r13,
            r14: f.r14,
            r15: f.r15,
            rdi: f.rdi,
            rsi: f.rsi,
            rbp: f.rbp,
            rbx: f.rbx,
            rdx: f.rdx,
            rax: f.rax,
            rcx: f.rcx,
            rsp: f.rsp,
            rip: f.rip,
            eflags: f.rflags,
            cs: f.cs as u16,
            ss: f.ss as u16,
            err: if f.vector == SYSCALL_VECTOR {
                0
            } else {
                f.error_code
            },
            trapno: if f.vector == SYSCALL_VECTOR {
                0
            } else {
                f.vector
            },
            oldmask: old_mask,
            fpstate: fp_addr,
            ..Default::default()
        };
        // The alternate stack settings, restored by rt_sigreturn; with
        // SS_AUTODISARM it is disarmed while the handler runs.
        let saved_alt = thread.altstack.lock().enter_handler();
        let frame = RtSigFrame {
            pretcode: restorer,
            uc: UContext {
                uc_stack: StackT {
                    ss_sp: saved_alt.sp,
                    ss_flags: saved_alt.flags,
                    pad: 0,
                    ss_size: saved_alt.size,
                },
                uc_mcontext: sc,
                uc_sigmask: old_mask,
                ..Default::default()
            },
            info,
        };
        if crate::syscall::userspace::write_user_bytes(frame_addr as usize, bytes_of(&frame))
            .is_err()
        {
            return false;
        }

        // Enter the handler: handler(sig, &info, &uc) on the new frame.
        let info_addr = frame_addr + core::mem::offset_of!(RtSigFrame, info) as u64;
        let uc_addr = frame_addr + core::mem::offset_of!(RtSigFrame, uc) as u64;
        f.rip = handler;
        f.rsp = frame_addr;
        f.rdi = sig as u64;
        f.rsi = info_addr;
        f.rdx = uc_addr;
        f.rax = 0;
        // DF, TF and RF cleared (Linux), the rest sanitised on return.
        f.rflags &= !(0x400 | 0x100 | 0x1_0000);

        // Block sa_mask, and the signal itself unless SA_NODEFER.
        let mut blocked = thread.sigmask.load(Ordering::Acquire) | sa_mask;
        if flags & SA_NODEFER == 0 {
            blocked |= sig_bit(sig);
        }
        set_mask(process, thread, blocked);
        if flags & SA_RESETHAND != 0 {
            let _ = process.set_signal_handler(sig, SIG_DFL);
            process.signal_action_extra.lock()[sig] = (0, 0, 0);
        }
        true
    }

    /// Act on pending signals on the way back to user mode (syscall exit
    /// and trap exit): a fatal one ends the thread's process, a caught one
    /// gets a handler frame. One handler per return; the next pending
    /// signal is taken on a later return. Never returns when the process
    /// is terminated.
    pub fn deliver_on_return(f: &mut TrapFrame) {
        loop {
            // A stopped process's threads wait here; SIGKILL ends them.
            park_while_stopped();
            super::super::user_return_check();
            // Decide with the references in a scope of their own: exiting
            // must not leave an Arc behind on this stack.
            let fatal = {
                let (Some(process), Some(thread)) = (
                    super::super::current_process(),
                    super::super::current_thread(),
                ) else {
                    return;
                };
                let seq = process.cont_seq.load(Ordering::SeqCst);
                let Some(info) = dequeue(&process, &thread) else {
                    thread.no_restart.store(false, Ordering::Release);
                    // sigsuspend ended without a handler to restore its
                    // mask: put the saved one back now.
                    if thread.has_saved_sigmask.swap(false, Ordering::AcqRel) {
                        set_mask(
                            &process,
                            &thread,
                            thread.saved_sigmask.load(Ordering::Acquire),
                        );
                    }
                    return;
                };
                let sig = info.signo();
                let handler = process.get_signal_handler(sig).unwrap_or(SIG_DFL);
                match handler {
                    SIG_IGN => None,
                    SIG_DFL => match default_action(sig) {
                        DefaultAction::Terminate | DefaultAction::CoreDump => Some(sig),
                        DefaultAction::Stop => {
                            // A system call the stop interrupted runs again
                            // once the process continues, as on Linux.
                            thread.no_restart.store(false, Ordering::Release);
                            if f.vector == SYSCALL_VECTOR && f.rax == EINTR_RAX {
                                f.rax = f.error_code;
                                f.rip = f.rip.wrapping_sub(2);
                            }
                            stop_process(&process, sig, seq);
                            None
                        }
                        _ => None,
                    },
                    addr => {
                        let (flags, restorer, mask) = process.signal_action_extra.lock()[sig];
                        if setup_frame(f, &process, &thread, info, addr, flags, restorer, mask) {
                            sanitize_user_frame(f);
                            return;
                        }
                        // No usable stack or restorer: as Linux, SIGSEGV.
                        Some(11)
                    }
                }
            };
            if let Some(sig) = fatal {
                let _ = crate::syscall::process::exit_current(0, sig as u32);
            }
        }
    }

    /// rt_sigreturn: restore the context saved by `setup_frame` into the
    /// system call frame `f`. Returns the restored rax (the value the
    /// system call "returns"), or None if the frame is unusable (the
    /// caller kills the thread with SIGSEGV, as Linux does).
    pub fn rt_sigreturn(f: &mut TrapFrame) -> Option<u64> {
        let process = super::super::current_process()?;
        let thread = super::super::current_thread()?;
        // The handler's `ret` popped the return address: RSP points at uc.
        let frame_addr = f.rsp.checked_sub(8)?;
        let mut frame = RtSigFrame::default();
        // SAFETY: RtSigFrame is plain data; any bytes form a valid value.
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(
                &mut frame as *mut RtSigFrame as *mut u8,
                core::mem::size_of::<RtSigFrame>(),
            )
        };
        crate::syscall::userspace::read_user_bytes(frame_addr as usize, bytes).ok()?;
        let sc = frame.uc.uc_mcontext;
        if !is_user_address(sc.rip) || !is_user_address(sc.rsp) {
            return None;
        }

        // FPU image first (it can fail): copy, clear reserved MXCSR bits
        // (FXRSTOR faults on them), load.
        if sc.fpstate != 0 {
            let mut fx = FxArea([0; FXSAVE_SIZE]);
            crate::syscall::userspace::read_user_bytes(sc.fpstate as usize, &mut fx.0).ok()?;
            let mut current = FxArea([0; FXSAVE_SIZE]);
            // SAFETY: FXSAVE64 into an aligned 512-byte buffer.
            unsafe {
                core::arch::asm!("fxsave64 [{}]", in(reg) current.0.as_mut_ptr(), options(nostack, preserves_flags));
            }
            let mut mxcsr_mask =
                u32::from_le_bytes([current.0[28], current.0[29], current.0[30], current.0[31]]);
            if mxcsr_mask == 0 {
                mxcsr_mask = 0xFFBF;
            }
            let mxcsr = u32::from_le_bytes([fx.0[24], fx.0[25], fx.0[26], fx.0[27]]) & mxcsr_mask;
            fx.0[24..28].copy_from_slice(&mxcsr.to_le_bytes());
            // SAFETY: a 16-byte-aligned FXSAVE image whose MXCSR has no
            // reserved bits set; the other fields cannot make FXRSTOR fault.
            unsafe {
                core::arch::asm!("fxrstor64 [{}]", in(reg) fx.0.as_ptr(), options(nostack, preserves_flags));
            }
        }

        f.r8 = sc.r8;
        f.r9 = sc.r9;
        f.r10 = sc.r10;
        f.r11 = sc.r11;
        f.r12 = sc.r12;
        f.r13 = sc.r13;
        f.r14 = sc.r14;
        f.r15 = sc.r15;
        f.rdi = sc.rdi;
        f.rsi = sc.rsi;
        f.rbp = sc.rbp;
        f.rbx = sc.rbx;
        f.rdx = sc.rdx;
        f.rax = sc.rax;
        f.rcx = sc.rcx;
        f.rsp = sc.rsp;
        f.rip = sc.rip;
        f.rflags = sc.eflags;
        sanitize_user_frame(f);
        set_mask(&process, &thread, frame.uc.uc_sigmask);
        // The alternate stack settings the handler was entered with. As on
        // Linux (restore_altstack), a setting that cannot be made now is
        // dropped rather than failing the return.
        let uc_stack = frame.uc.uc_stack;
        let _ = thread.altstack.lock().set(
            AltStackRequest {
                sp: uc_stack.ss_sp,
                flags: uc_stack.ss_flags,
                size: uc_stack.ss_size,
            },
            f.rsp,
        );
        Some(sc.rax)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_layout_is_linux() {
        assert_eq!(sig_bit(1), 1);
        assert_eq!(sig_bit(9), 1 << 8);
        assert_eq!(sig_bit(64), 1 << 63);
        assert_eq!(sig_bit(0), 0);
        assert_eq!(sig_bit(65), 0);
        assert_eq!(UNBLOCKABLE, (1 << 8) | (1 << 18));
    }

    #[test]
    fn default_actions_follow_posix() {
        assert_eq!(default_action(17), DefaultAction::Ignore); // SIGCHLD
        assert_eq!(default_action(28), DefaultAction::Ignore); // SIGWINCH
        assert_eq!(default_action(15), DefaultAction::Terminate); // SIGTERM
        assert_eq!(default_action(11), DefaultAction::CoreDump); // SIGSEGV
        assert_eq!(default_action(6), DefaultAction::CoreDump); // SIGABRT
        assert_eq!(default_action(19), DefaultAction::Stop); // SIGSTOP
        assert_eq!(default_action(18), DefaultAction::Continue); // SIGCONT
        assert_eq!(default_action(10), DefaultAction::Terminate); // SIGUSR1
    }

    fn queued(sig: usize, value: u64) -> SigInfo {
        let mut info = SigInfo::from_process(sig, SI_QUEUE, 7, 1000);
        info.fields[1] = value;
        info
    }

    /// Each queued real-time instance keeps its own siginfo, oldest first
    /// (sigqueue values arrive in order).
    #[test]
    fn real_time_instances_keep_their_siginfo() {
        let q = RtQueue::new();
        let pending = core::sync::atomic::AtomicU64::new(0);
        for v in 1..=3 {
            q.push(&pending, queued(40, v), Charge::UNLIMITED, Overflow::Fail)
                .unwrap();
        }
        let values: alloc::vec::Vec<u64> = (0..3)
            .map(|_| q.take(&pending, 40).unwrap().value())
            .collect();
        assert_eq!(values, [1, 2, 3]);
        assert_eq!(q.take(&pending, 40), None);
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }

    /// A standard signal sent again while pending keeps the first
    /// sender's siginfo, and is taken once.
    #[test]
    fn standard_signal_keeps_first_siginfo() {
        let q = RtQueue::new();
        let pending = core::sync::atomic::AtomicU64::new(0);
        q.push(
            &pending,
            SigInfo::from_process(10, SI_USER, 42, 0),
            Charge::UNLIMITED,
            Overflow::Merge,
        )
        .unwrap();
        q.push(
            &pending,
            SigInfo::from_process(10, SI_USER, 43, 0),
            Charge::UNLIMITED,
            Overflow::Merge,
        )
        .unwrap();
        let info = q.take(&pending, 10).unwrap();
        assert_eq!((info.pid(), info.si_code), (42, SI_USER));
        assert_eq!(q.take(&pending, 10), None);
        // Pending again later: the new sender, not a stale record.
        q.push(
            &pending,
            SigInfo::from_process(10, SI_USER, 44, 0),
            Charge::UNLIMITED,
            Overflow::Merge,
        )
        .unwrap();
        assert_eq!(q.take(&pending, 10).unwrap().pid(), 44);
    }

    /// A bit cleared without the queue (as job control used to) leaves no
    /// stale siginfo for the next send.
    #[test]
    fn cleared_signal_leaves_no_stale_siginfo() {
        let q = RtQueue::new();
        let pending = core::sync::atomic::AtomicU64::new(0);
        q.push(
            &pending,
            SigInfo::from_process(18, SI_USER, 5, 0),
            Charge::UNLIMITED,
            Overflow::Merge,
        )
        .unwrap();
        pending.fetch_and(!sig_bit(18), Ordering::AcqRel);
        q.push(
            &pending,
            SigInfo::kernel(18),
            Charge::UNLIMITED,
            Overflow::Merge,
        )
        .unwrap();
        assert_eq!(q.take(&pending, 18).unwrap().si_code, SI_KERNEL);
        // clear() drops records and bits together.
        q.push(&pending, queued(33, 9), Charge::UNLIMITED, Overflow::Fail)
            .unwrap();
        q.clear(&pending, sig_bit(33));
        assert_eq!(q.take(&pending, 33), None);
    }

    /// Past the queue limit a kill-style send pends without a record and
    /// is delivered with an SI_USER siginfo naming no sender.
    #[test]
    fn merged_signal_comes_without_sender() {
        let q = RtQueue::new();
        let pending = core::sync::atomic::AtomicU64::new(0);
        let full = Charge {
            uid: 0xfeed,
            limit: 0,
        };
        assert_eq!(
            q.push(&pending, queued(50, 1), full, Overflow::Fail),
            Err(QueueFull)
        );
        q.push(
            &pending,
            SigInfo::from_process(50, SI_USER, 9, 0),
            full,
            Overflow::Merge,
        )
        .unwrap();
        let info = q.take(&pending, 50).unwrap();
        assert_eq!((info.signo(), info.si_code, info.pid()), (50, SI_USER, 0));
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }

    /// The siginfo union members sit where Linux puts them.
    #[test]
    fn siginfo_union_is_linux_layout() {
        let bytes = |i: &SigInfo| -> [u8; 128] {
            let mut b = [0u8; 128];
            // SAFETY: SigInfo is 128 bytes of plain integers.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    i as *const SigInfo as *const u8,
                    b.as_mut_ptr(),
                    128,
                )
            };
            b
        };
        let i32_at =
            |b: &[u8; 128], at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let u64_at =
            |b: &[u8; 128], at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        // kill / sigqueue: si_pid 16, si_uid 20, si_value 24.
        let b = bytes(&queued(40, 0x55));
        assert_eq!(i32_at(&b, 0), 40);
        assert_eq!(i32_at(&b, 8), SI_QUEUE);
        assert_eq!(i32_at(&b, 16), 7);
        assert_eq!(i32_at(&b, 20), 1000);
        assert_eq!(u64_at(&b, 24), 0x55);
        // A fault: si_addr 16.
        let b = bytes(&SigInfo::fault(11, SEGV_ACCERR, 0x1234_0000));
        assert_eq!(i32_at(&b, 8), SEGV_ACCERR);
        assert_eq!(u64_at(&b, 16), 0x1234_0000);
    }

    fn stack(sp: u64, flags: i32, size: u64) -> AltStackRequest {
        AltStackRequest { sp, flags, size }
    }

    #[test]
    fn altstack_reports_linux_flags() {
        let mut alt = SigAltStack::default();
        // No alternate stack: disabled, whatever the stack pointer.
        assert_eq!(alt.flags_at(0x7000), SS_DISABLE);
        alt.set(stack(0x10000, 0, 0x4000), 0x7000).unwrap();
        // A stack grows down from sp + size: inside means above sp, at
        // most sp + size.
        assert_eq!(alt.flags_at(0x7000), 0);
        assert_eq!(alt.flags_at(0x10000), 0);
        assert_eq!(alt.flags_at(0x10001), SS_ONSTACK);
        assert_eq!(alt.flags_at(0x14000), SS_ONSTACK);
        assert_eq!(alt.flags_at(0x14001), 0);
        // SS_AUTODISARM is reported along with the state.
        alt.set(stack(0x10000, SS_AUTODISARM, 0x4000), 0x7000)
            .unwrap();
        assert_eq!(alt.flags_at(0x7000), SS_AUTODISARM);
        assert_eq!(alt.flags_at(0x12000), SS_ONSTACK | SS_AUTODISARM);
    }

    #[test]
    fn altstack_changes_follow_linux_rules() {
        let mut alt = SigAltStack::default();
        // Smaller than MINSIGSTKSZ: ENOMEM. Unknown mode bits: EINVAL.
        assert_eq!(
            alt.set(stack(0x10000, 0, MINSIGSTKSZ - 1), 0),
            Err(AltStackError::TooSmall)
        );
        assert_eq!(
            alt.set(stack(0x10000, 4, 0x4000), 0),
            Err(AltStackError::Invalid)
        );
        // SS_ONSTACK as the mode is accepted (old programs pass it) and
        // means enabled.
        alt.set(stack(0x10000, SS_ONSTACK, 0x4000), 0).unwrap();
        assert_eq!(alt.top(), Some(0x14000));
        // Not while running on it: EPERM.
        assert_eq!(
            alt.set(stack(0x20000, 0, 0x4000), 0x12000),
            Err(AltStackError::OnStack)
        );
        // SS_DISABLE takes effect whatever size is passed.
        alt.set(stack(0x99, SS_DISABLE, 1), 0x7000).unwrap();
        assert_eq!(alt.top(), None);
        assert_eq!(alt.flags_at(0x7000), SS_DISABLE);
    }

    #[test]
    fn altstack_autodisarm_resets_for_the_handler() {
        let mut alt = SigAltStack::default();
        alt.set(stack(0x10000, SS_AUTODISARM, 0x4000), 0).unwrap();
        // Delivery records the settings (for uc_stack) and disarms them.
        let saved = alt.enter_handler();
        assert_eq!(saved, stack(0x10000, SS_AUTODISARM, 0x4000));
        assert_eq!(alt.top(), None);
        // rt_sigreturn restores them.
        alt.set(saved, 0x7000).unwrap();
        assert_eq!(alt.top(), Some(0x14000));
        // Without SS_AUTODISARM the stack stays armed.
        alt.set(stack(0x10000, 0, 0x4000), 0).unwrap();
        let saved = alt.enter_handler();
        assert_eq!(saved, stack(0x10000, 0, 0x4000));
        assert_eq!(alt.top(), Some(0x14000));
    }
}

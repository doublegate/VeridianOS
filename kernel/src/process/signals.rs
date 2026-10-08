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

/// Instances of one real-time signal a process or thread can have queued;
/// beyond it a send fails with EAGAIN, as Linux does at RLIMIT_SIGPENDING.
pub const RT_QUEUE_MAX: u32 = 1024;

/// A real-time signal's queue is full (`RT_QUEUE_MAX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

/// Queued instances of each real-time signal, beside the pending set's
/// bit: the bit stays set while any instance is queued. Changed under its
/// lock together with the bit, so a send cannot be lost between a delivery
/// taking the last instance and clearing the bit.
pub struct RtQueue(spin::Mutex<[u32; NSIG - SIGRTMIN + 1]>);

impl RtQueue {
    pub const fn new() -> Self {
        Self(spin::Mutex::new([0; NSIG - SIGRTMIN + 1]))
    }

    /// Queue one instance of `sig` in `pending`. A standard signal only
    /// sets its bit. `QueueFull` (EAGAIN) when the real-time queue is full.
    pub fn push(
        &self,
        pending: &core::sync::atomic::AtomicU64,
        sig: usize,
    ) -> Result<(), QueueFull> {
        if sig < SIGRTMIN {
            pending.fetch_or(sig_bit(sig), Ordering::AcqRel);
            return Ok(());
        }
        let mut counts = self.0.lock();
        let count = &mut counts[sig - SIGRTMIN];
        if *count >= RT_QUEUE_MAX {
            return Err(QueueFull);
        }
        *count += 1;
        pending.fetch_or(sig_bit(sig), Ordering::AcqRel);
        Ok(())
    }

    /// Take one instance of `sig` off `pending`: whether there was one.
    /// The bit stays while real-time instances remain.
    pub fn take(&self, pending: &core::sync::atomic::AtomicU64, sig: usize) -> bool {
        let bit = sig_bit(sig);
        if sig < SIGRTMIN {
            return pending.fetch_and(!bit, Ordering::AcqRel) & bit != 0;
        }
        let mut counts = self.0.lock();
        if pending.load(Ordering::Acquire) & bit == 0 {
            return false;
        }
        let count = &mut counts[sig - SIGRTMIN];
        *count = count.saturating_sub(1);
        if *count == 0 {
            pending.fetch_and(!bit, Ordering::AcqRel);
        }
        true
    }

    /// Drop every queued instance and pending bit (exec, a discarded
    /// signal).
    pub fn clear(&self, pending: &core::sync::atomic::AtomicU64, bits: u64) {
        let mut counts = self.0.lock();
        for sig in SIGRTMIN..=NSIG {
            if bits & sig_bit(sig) != 0 {
                counts[sig - SIGRTMIN] = 0;
            }
        }
        pending.fetch_and(!bits, Ordering::AcqRel);
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

/// `sigaction` flags (Linux x86_64 values).
#[cfg(target_arch = "x86_64")]
const SA_RESTORER: u64 = 0x0400_0000;
#[cfg(target_arch = "x86_64")]
const SA_RESTART: u64 = 0x1000_0000;
#[cfg(target_arch = "x86_64")]
const SA_NODEFER: u64 = 0x4000_0000;
#[cfg(target_arch = "x86_64")]
const SA_RESETHAND: u64 = 0x8000_0000;
/// Run the handler on the alternate signal stack (sigaltstack).
#[cfg(target_arch = "x86_64")]
const SA_ONSTACK: u64 = 0x0800_0000;

/// `stack_t.ss_flags`: the thread is running on its alternate stack.
pub const SS_ONSTACK: i32 = 1;
/// `stack_t.ss_flags`: no alternate stack.
pub const SS_DISABLE: i32 = 2;
/// `stack_t.ss_flags`: disarm the alternate stack while a handler runs on
/// it, re-arming it when the handler returns (Linux 4.7).
pub const SS_AUTODISARM: i32 = i32::MIN;
/// The smallest alternate stack sigaltstack accepts (x86_64 MINSIGSTKSZ).
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

/// Linux EINTR, as a system call returns it in rax.
#[cfg(target_arch = "x86_64")]
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
            .pending_signals
            .fetch_and(!STOP_SIGNALS, Ordering::AcqRel);
        for thread in process.threads.lock().values() {
            thread.sigpending.fetch_and(!STOP_SIGNALS, Ordering::AcqRel);
        }
        // Bump first: a stop being decided right now sees the change and
        // backs out (`stop_process`).
        process.cont_seq.fetch_add(1, Ordering::SeqCst);
        if process.stop_signal.swap(0, Ordering::SeqCst) != 0 {
            process.job_report.store(0xffff, Ordering::Release);
            notify_parent_job(process);
        }
        crate::sched::dispatch::PROCESS_EVENTS.wake_all();
    } else if sig_bit(sig) & STOP_SIGNALS != 0 {
        let cont = !sig_bit(SIGCONT);
        process.pending_signals.fetch_and(cont, Ordering::AcqRel);
        for thread in process.threads.lock().values() {
            thread.sigpending.fetch_and(cont, Ordering::AcqRel);
        }
    }
}

/// Tell `process`'s parent that it stopped or continued: SIGCHLD unless
/// the parent asked for none (SA_NOCLDSTOP), and a wakeup for `wait`.
#[cfg(feature = "alloc")]
fn notify_parent_job(process: &Process) {
    if let Some(parent) = process.parent().and_then(super::table::get_process) {
        let nocldstop = parent.signal_action_extra.lock()[SIGCHLD].0 & SA_NOCLDSTOP != 0;
        if !nocldstop {
            notify(&parent, SIGCHLD);
        }
    }
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
}

/// Stop `process` with `sig`, unless a SIGCONT came after `seq` was read
/// (the stop signal was taken before it, so the continue wins).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
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
    notify_parent_job(process);
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

/// Send `sig` to a dispatched process. `Ok(false)` if the process has no
/// running threads (the caller falls back to the old path); `WouldBlock`
/// (EAGAIN) if a real-time signal's queue is full.
#[cfg(feature = "alloc")]
pub fn send_to_process(process: &Process, sig: usize) -> Result<bool, crate::error::KernelError> {
    use crate::sched::dispatch;

    let tasks = dispatch::tasks_of(process.pid.0);
    if tasks.is_empty() {
        return Ok(false);
    }
    job_control_on_send(process, sig);
    if sig == SIGKILL {
        let _ = process.kill_pending.compare_exchange(
            0,
            SIGKILL as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    } else if ignored(process, sig) {
        return Ok(true);
    } else {
        process
            .rt_queue
            .push(&process.pending_signals, sig)
            .map_err(|_| crate::error::KernelError::WouldBlock)?;
    }
    for task in &tasks {
        dispatch::wake(task);
    }
    dispatch::PROCESS_EVENTS.wake_all();
    Ok(true)
}

/// Send `sig` to `process` from inside the kernel (SIGCHLD to a parent,
/// terminal signals to a foreground group): through the dispatched path
/// when the process runs on its own threads, so an ignored signal is
/// dropped instead of pending and interrupting waits (N-98).
pub fn notify(process: &Process, sig: usize) {
    #[cfg(feature = "alloc")]
    if process.dispatched.load(Ordering::Acquire) && send_to_process(process, sig).unwrap_or(true) {
        return;
    }
    let _ = process.send_signal(sig);
}

/// Send `sig` to one thread of a dispatched process (tkill, tgkill);
/// `WouldBlock` (EAGAIN) if a real-time signal's queue is full.
#[cfg(feature = "alloc")]
pub fn send_to_thread(
    process: &Process,
    thread: &Thread,
    sig: usize,
) -> Result<(), crate::error::KernelError> {
    if sig == SIGKILL {
        send_to_process(process, sig)?;
        return Ok(());
    }
    job_control_on_send(process, sig);
    if ignored(process, sig) {
        return Ok(());
    }
    thread
        .rt_queue
        .push(&thread.sigpending, sig)
        .map_err(|_| crate::error::KernelError::WouldBlock)?;
    for task in crate::sched::dispatch::tasks_of(process.pid.0) {
        if task.owner() == Some((process.pid.0, thread.tid.0)) {
            crate::sched::dispatch::wake(&task);
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

/// Replace the calling thread's mask (UNBLOCKABLE is never blocked).
/// Returns the old mask. The process copy follows the most recent thread
/// to change it, for readers that know only processes (procfs).
pub fn set_mask(process: &Process, thread: &Thread, mask: u64) -> u64 {
    let mask = mask & !UNBLOCKABLE;
    process.signal_mask.store(mask, Ordering::Release);
    thread.sigmask.swap(mask, Ordering::AcqRel)
}

/// Take the lowest deliverable signal off the pending sets.
#[cfg(target_arch = "x86_64")]
fn dequeue(process: &Process, thread: &Thread) -> Option<usize> {
    let ready = deliverable(process, thread);
    if ready == 0 {
        return None;
    }
    let sig = ready.trailing_zeros() as usize + 1;
    // The thread's own instance first; a real-time signal's bit stays set
    // while more instances are queued (N-209).
    if !thread.rt_queue.take(&thread.sigpending, sig) {
        process.rt_queue.take(&process.pending_signals, sig);
    }
    Some(sig)
}

#[cfg(target_arch = "x86_64")]
pub use frame::{deliver_on_return, rt_sigreturn, MIN_SIGNAL_FRAME};

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

    /// Linux `siginfo_t` (128 bytes): signo, errno, code, then a union.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct SigInfo {
        pub si_signo: i32,
        pub si_errno: i32,
        pub si_code: i32,
        pub pad: i32,
        pub fields: [u64; 14],
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

    /// Build the handler frame for `sig` on the user stack and point `f`
    /// at the handler. Returns false if the frame cannot be written (the
    /// caller then kills the thread with SIGSEGV).
    fn setup_frame(
        f: &mut TrapFrame,
        process: &Process,
        thread: &Thread,
        sig: usize,
        handler: u64,
        flags: u64,
        restorer: u64,
        sa_mask: u64,
    ) -> bool {
        if flags & SA_RESTORER == 0 || !is_user_address(restorer) || !is_user_address(handler) {
            return false;
        }

        // A system call the signal interrupted is restarted with SA_RESTART
        // (rewind over the 2-byte `syscall`, put the number back in rax);
        // otherwise it returns EINTR. The context saved below must be the
        // one the handler returns to.
        if f.vector == SYSCALL_VECTOR && f.rax == EINTR_RAX && flags & SA_RESTART != 0 {
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
            info: SigInfo {
                si_signo: sig as i32,
                ..Default::default()
            },
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
                let Some(sig) = dequeue(&process, &thread) else {
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
                let handler = process.get_signal_handler(sig).unwrap_or(SIG_DFL);
                match handler {
                    SIG_IGN => None,
                    SIG_DFL => match default_action(sig) {
                        DefaultAction::Terminate | DefaultAction::CoreDump => Some(sig),
                        DefaultAction::Stop => {
                            // A system call the stop interrupted runs again
                            // once the process continues, as on Linux.
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
                        if setup_frame(f, &process, &thread, sig, addr, flags, restorer, mask) {
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

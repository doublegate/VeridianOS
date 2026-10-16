//! Task dispatcher (ADR 0006): every task runs on its own kernel stack and
//! all switching happens between kernel stacks.
//!
//! Stage D1: the switch primitive, the boot flow adopted as CPU 0's first
//! task, an idle task, kernel threads, blocking on wait queues, and the
//! reaping of exited threads. Stage D2: user threads as tasks
//! ([`spawn_user`]), each with its own kernel stack and saved vector state
//! (XSAVE, N-41), entering ring 3 from `user_trampoline`. Timer preemption
//! (D3) and other CPUs (D5) build on it.
//!
//! Per-CPU state is a run queue of the ADR 0007 policy plus the running
//! task, the idle task and the task just switched away from. Rules:
//!
//! - **No lock across a switch.** [`schedule`] holds the CPU lock only to pick
//!   the next task, and switches with interrupts off and no lock held. The
//!   outgoing task stays alive through `CpuSched::prev` and keeps `on_cpu` set
//!   until the incoming task runs [`finish_switch`], so it is never resumed or
//!   freed while its registers are still being saved.
//! - **Prepare to wait.** A task sets itself `Blocked` before it checks the
//!   condition it waits for, and [`block_current`] takes it off the run queue
//!   only if it is still `Blocked` under the CPU lock. A [`wake`] in between
//!   turns it `Ready`, so the wake is never lost.
//! - **Reaping.** An exited task is removed and its stack freed in
//!   [`finish_switch`], on the next task's stack.
//!
//! x86_64 and RISC-V have the switch primitive (the `dispatch` cfg, set by
//! build.rs); elsewhere -- AArch64 until its MMU work, and host tests --
//! [`start`] declines and every entry point is a no-op.

// Only the `dispatch` architectures switch tasks (host tests exercise the
// logic, not the switch), so elsewhere the task machinery is compiled but
// unused.
#![cfg_attr(not(dispatch), allow(dead_code))]

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicU8, AtomicUsize, Ordering},
};

use spin::Mutex;

use super::policy::{
    balance::{self, CpuSnapshot},
    Entity, Policy, RunQueue, TaskKey,
};
use crate::error::KernelError;

/// Kernel stack size of a kernel thread and of the idle task, in pages.
const KTHREAD_STACK_PAGES: usize = 16;

/// A task's dispatch state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    /// Runnable, on a run queue.
    Ready = 0,
    /// Running on a CPU.
    Running = 1,
    /// Waiting for [`wake`]; off every run queue (once `block_current` ran).
    Blocked = 2,
    /// Exited; reaped once off the CPU.
    Dead = 3,
}

impl State {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => State::Ready,
            1 => State::Running,
            2 => State::Blocked,
            _ => State::Dead,
        }
    }
}

/// Architecture state a task carries across switches.
#[derive(Debug, Default, Clone, Copy)]
struct ArchState {
    /// x86_64: page table root, user TLS base, and this CPU's syscall
    /// entry state (entry stack, user RSP scratch, frame pointer), which
    /// belong to whatever user context the task is in the middle of.
    #[cfg(target_arch = "x86_64")]
    cr3: u64,
    #[cfg(target_arch = "x86_64")]
    fs_base: u64,
    /// The user GS base, which sits in IA32_KERNEL_GS_BASE while ring 0
    /// runs (the entry's `swapgs`): it belongs to the task, not the CPU.
    #[cfg(target_arch = "x86_64")]
    user_gs_base: u64,
    #[cfg(target_arch = "x86_64")]
    entry_stack: u64,
    #[cfg(target_arch = "x86_64")]
    user_rsp: u64,
    #[cfg(target_arch = "x86_64")]
    syscall_frame: u64,
    /// RISC-V: the physical root of the task's address space (satp).
    #[cfg(target_arch = "riscv64")]
    root: u64,
}

/// A dispatchable task.
pub struct Task {
    key: TaskKey,
    name: &'static str,
    state: AtomicU8,
    /// Set from the moment a CPU picks the task until the next task on that
    /// CPU has finished switching away from it (Linux `p->on_cpu`).
    on_cpu: AtomicBool,
    /// Scheduling state while off the run queue.
    entity: Mutex<Entity>,
    /// Saved stack pointer while not running. Written by `switch_stacks` on
    /// the CPU switching the task out, read by the CPU switching it in;
    /// `on_cpu` orders the two.
    saved_sp: UnsafeCell<usize>,
    /// Architecture state, same access rule as `saved_sp`.
    arch: UnsafeCell<ArchState>,
    /// The task's own kernel stack (`None`: the boot flow's stack).
    stack: Option<crate::mm::kstack::KernelStack>,
    /// The user thread this task runs, as (pid, tid); `None` for kernel
    /// tasks.
    owner: Option<(u64, u64)>,
    /// The CPU whose run queue holds the task while it is runnable, or
    /// last ran it. Changed only while the task is on no run queue and on
    /// no CPU (wake placement), or with both CPUs' locks held (migration).
    cpu: AtomicUsize,
    /// The CPUs the task may run on, a bit per CPU (sched_setaffinity).
    affinity: AtomicU64,
    /// When the task last stopped running (`monotonic_ns`): whether its
    /// cache is still warm where it ran, for placement and balancing.
    last_ran: AtomicU64,
    /// Taken off its run queue by `schedule` because its CPU is no longer
    /// allowed; `finish_switch` places it once it is off the CPU.
    migrate_out: AtomicBool,
    /// A CPU's idle task: it runs when nothing else can and never sleeps.
    idle: bool,
    /// CPU time the task has used (N-218).
    usage: super::cputime::CpuUsage,
    /// User vector state (x87/SSE/AVX), switched eagerly: the kernel itself
    /// is soft-float, so only tasks that run user code need it. Same access
    /// rule as `saved_sp`.
    #[cfg(target_arch = "x86_64")]
    xsave: UnsafeCell<Option<crate::arch::x86_64::switch::xsave::Area>>,
}

// SAFETY: `saved_sp` and `arch` are accessed only by the CPU switching the
// task in or out, with interrupts off, and the dispatch protocol (on_cpu,
// the CPU lock) makes those accesses ordered and exclusive. Every other
// field is atomic or locked.
unsafe impl Sync for Task {}
// SAFETY: as above; nothing in a task is tied to the thread that made it.
unsafe impl Send for Task {}

impl Task {
    fn new(
        key: TaskKey,
        name: &'static str,
        policy: Policy,
        stack: Option<crate::mm::kstack::KernelStack>,
    ) -> Self {
        Self {
            key,
            name,
            state: AtomicU8::new(State::Ready as u8),
            on_cpu: AtomicBool::new(false),
            entity: Mutex::new(Entity::new(policy)),
            saved_sp: UnsafeCell::new(0),
            arch: UnsafeCell::new(ArchState::default()),
            stack,
            owner: None,
            cpu: AtomicUsize::new(0),
            affinity: AtomicU64::new(u64::MAX),
            last_ran: AtomicU64::new(0),
            migrate_out: AtomicBool::new(false),
            idle: false,
            usage: super::cputime::CpuUsage::new(),
            #[cfg(target_arch = "x86_64")]
            xsave: UnsafeCell::new(None),
        }
    }

    /// The (pid, tid) of the user thread this task runs.
    pub fn owner(&self) -> Option<(u64, u64)> {
        self.owner
    }

    pub fn key(&self) -> TaskKey {
        self.key
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, s: State) {
        self.state.store(s as u8, Ordering::Release);
    }

    /// Whether a CPU is running the task or still switching away from it.
    pub fn on_cpu(&self) -> bool {
        self.on_cpu.load(Ordering::Acquire)
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if let Some(stack) = self.stack.take() {
            crate::mm::kstack::free(stack);
        }
    }
}

/// One CPU's dispatch state.
struct CpuSched {
    rq: RunQueue,
    current: Arc<Task>,
    idle: Arc<Task>,
    /// The task this CPU just switched away from, until `finish_switch`.
    prev: Option<Arc<Task>>,
    need_resched: bool,
    /// When `current`'s running was last charged to it.
    acct_start: u64,
}

impl CpuSched {
    /// Charge the running task up to `now`.
    fn charge(&mut self, now: u64) {
        self.current
            .usage
            .charge(now.saturating_sub(self.acct_start));
        self.acct_start = now;
    }
}

/// Per-CPU dispatch state (SMP stage S2, sprint D5): the boot CPU from
/// `start`, every secondary from `run_secondary`.
static CPUS: [Mutex<Option<CpuSched>>; MAX_CPUS] = [const { Mutex::new(None) }; MAX_CPUS];

/// The running task of each dispatching CPU, readable without the CPU lock
/// (for `current_process()` on every syscall and in fault handlers). Only
/// that CPU changes it, in `schedule` with interrupts off, and the task it
/// points to stays alive while it is current.
static CURRENT: [AtomicPtr<Task>; MAX_CPUS] =
    [const { AtomicPtr::new(core::ptr::null_mut()) }; MAX_CPUS];

/// Bit per CPU that dispatches (has its `CpuSched`).
static DISPATCHING: AtomicU64 = AtomicU64::new(0);

/// What placement and balancing read of a CPU without taking its lock:
/// published by the CPU's owner of the moment, under its lock, whenever its
/// run queue changes (`publish`). A reader may see a slightly old value;
/// the decisions only need to be good, not exact.
struct CpuStat {
    nr_running: AtomicUsize,
    load: AtomicU64,
    /// [`CpuSnapshot::dl_earliest`] and [`CpuSnapshot::dl_next`].
    dl_earliest: AtomicU64,
    dl_next: AtomicU64,
    /// When this CPU last balanced periodically.
    balanced_at: AtomicU64,
}

static STATS: [CpuStat; MAX_CPUS] = [const {
    CpuStat {
        nr_running: AtomicUsize::new(0),
        load: AtomicU64::new(0),
        dl_earliest: AtomicU64::new(balance::NO_DL),
        dl_next: AtomicU64::new(balance::NO_DL),
        balanced_at: AtomicU64::new(0),
    }
}; MAX_CPUS];

/// Publish `st`'s run queue numbers for CPU `cpu` (its lock held).
fn publish(cpu: usize, st: &mut CpuSched) {
    let s = &STATS[cpu];
    s.nr_running.store(st.rq.nr_running(), Ordering::Release);
    s.load.store(st.rq.load_avg(now()), Ordering::Release);
    let earliest = st.rq.dl_earliest().unwrap_or(balance::NO_DL);
    let next = st.rq.dl_waiting().next().map_or(balance::NO_DL, |(d, _)| d);
    s.dl_earliest.store(earliest, Ordering::Release);
    s.dl_next.store(next, Ordering::Release);
}

/// A snapshot of every CPU for the placement and balancing decisions.
fn snapshots() -> [CpuSnapshot; MAX_CPUS] {
    let mut out = [CpuSnapshot::default(); MAX_CPUS];
    for (cpu, snap) in out.iter_mut().enumerate() {
        let nr = STATS[cpu].nr_running.load(Ordering::Acquire);
        *snap = CpuSnapshot {
            online: dispatching(cpu),
            idle: nr == 0,
            nr_running: nr,
            load: STATS[cpu].load.load(Ordering::Acquire),
            dl_earliest: STATS[cpu].dl_earliest.load(Ordering::Acquire),
            dl_next: STATS[cpu].dl_next.load(Ordering::Acquire),
        };
    }
    out
}

/// Whether `mask` allows CPU `cpu`.
fn allows(mask: u64, cpu: usize) -> bool {
    cpu < 64 && mask & (1 << cpu) != 0
}

/// The CPU a task that is on no run queue and no CPU should be queued on:
/// [`balance::select_cpu`] over the dispatching CPUs it may use. `prev`
/// (its last CPU) if none of them is allowed, which cannot happen through
/// sched_setaffinity (it refuses a mask without a dispatching CPU). A
/// deadline task goes where it runs soonest ([`balance::select_cpu_dl`],
/// by the deadline its wakeup gives it).
fn place(task: &Task, prev: usize) -> usize {
    let t = now();
    let since = t.saturating_sub(task.last_ran.load(Ordering::Acquire));
    let affinity = task.affinity.load(Ordering::Acquire);
    let entity = *task.entity.lock();
    match entity.policy {
        Policy::Deadline(mut d) => {
            d.on_wakeup(t);
            balance::select_cpu_dl(prev, affinity, &snapshots(), since, d.deadline)
        }
        _ => balance::select_cpu(prev, affinity, &snapshots(), since),
    }
    .unwrap_or(prev)
}

use super::smp::MAX_CPUS;

/// Woken whenever a process changes state (exit, stop, a fatal signal);
/// `wait` and the program launchers wait here.
pub static PROCESS_EVENTS: WaitQueue = WaitQueue::new();

/// Woken whenever a file object changes readiness (pipes today; the other
/// file types are re-checked periodically until they wake it too). Blocking
/// read, write and poll sleep here.
pub static IO_EVENTS: WaitQueue = WaitQueue::new();

/// Counts readiness changes, so a waiter can tell whether one happened
/// since it last looked.
static IO_SEQ: AtomicU64 = AtomicU64::new(0);

/// Report a readiness change on some file object (see [`IO_EVENTS`]).
pub fn io_event() {
    IO_SEQ.fetch_add(1, Ordering::AcqRel);
    if started() {
        IO_EVENTS.wake_all();
    }
}

/// [`io_event`] for interrupt context; see [`WaitQueue::wake_all_from_irq`].
pub fn io_event_from_irq() -> bool {
    IO_SEQ.fetch_add(1, Ordering::AcqRel);
    !started() || IO_EVENTS.wake_all_from_irq()
}

/// The readiness-change count; take it before scanning file objects and
/// pass it to [`wait_io`].
pub fn io_seq() -> u64 {
    IO_SEQ.load(Ordering::Acquire)
}

/// Re-check interval for file objects that do not report their readiness
/// changes yet ([`crate::fs::VfsNode::wakes_io_waiters`]).
pub const IO_RECHECK_NS: u64 = 10_000_000;

/// Sleep after a readiness scan (poll, epoll) found nothing ready: until a
/// file object reports a change since `seq`, `deadline` (`monotonic_ns`)
/// passes (`TimedOut`), or a signal must be acted on (`Interrupted`). With
/// `precise` false -- some scanned object does not wake waiters -- it also
/// returns after [`IO_RECHECK_NS`], and at `wake_at`, when a scanned object
/// becomes ready by itself (`VfsNode::ready_at_ns`). `Ok` means: scan
/// again.
pub fn wait_io(
    seq: u64,
    deadline: Option<u64>,
    precise: bool,
    wake_at: Option<u64>,
) -> Result<(), WaitError> {
    let recheck = (!precise).then(|| now().saturating_add(IO_RECHECK_NS));
    let until = [deadline, recheck, wake_at].into_iter().flatten().min();
    match wait_event(&IO_EVENTS, until, || io_seq() != seq) {
        Err(WaitError::TimedOut) if deadline.is_none_or(|d| now() < d) => Ok(()),
        r => r,
    }
}

/// Tasks sleeping in [`wait_event`] with a deadline, woken by the timer
/// tick once it passes (`monotonic_ns` deadline, task).
static SLEEPERS: Mutex<Vec<(u64, Arc<Task>)>> = Mutex::new(Vec::new());

/// Every live task except the idle tasks, by key.
static TASKS: Mutex<BTreeMap<TaskKey, Arc<Task>>> = Mutex::new(BTreeMap::new());

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Woken whenever a task exits (for [`join`]).
static EXITED: WaitQueue = WaitQueue::new();

/// The kernel's own page table root, for kernel threads.
static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// Whether the dispatcher runs on this machine.
pub fn started() -> bool {
    STARTED.load(Ordering::Acquire)
}

fn now() -> u64 {
    crate::arch::timer::monotonic_ns()
}

/// The dispatching CPU's slot, if this CPU dispatches.
fn this_cpu() -> Option<usize> {
    let cpu = crate::arch::percpu::this_cpu_id() as usize;
    (cpu < MAX_CPUS && DISPATCHING.load(Ordering::Acquire) & (1 << cpu) != 0).then_some(cpu)
}

/// Whether CPU `cpu` dispatches.
fn dispatching(cpu: usize) -> bool {
    cpu < MAX_CPUS && DISPATCHING.load(Ordering::Acquire) & (1 << cpu) != 0
}

/// Interrupt state saved by [`irq_save`].
#[derive(Clone, Copy)]
struct IrqFlag(bool);

fn irq_save() -> IrqFlag {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        let on = x86_64::instructions::interrupts::are_enabled();
        x86_64::instructions::interrupts::disable();
        IrqFlag(on)
    }
    #[cfg(all(target_arch = "riscv64", dispatch))]
    {
        IrqFlag(crate::arch::riscv64::switch::irq_save())
    }
    #[cfg(not(dispatch))]
    {
        IrqFlag(false)
    }
}

fn irq_restore(flag: IrqFlag) {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    if flag.0 {
        x86_64::instructions::interrupts::enable();
    }
    #[cfg(all(target_arch = "riscv64", dispatch))]
    if flag.0 {
        crate::arch::riscv64::switch::irq_enable();
    }
    let _ = flag;
}

fn lookup(key: TaskKey) -> Option<Arc<Task>> {
    TASKS.lock().get(&key).cloned()
}

/// Adopt the running boot flow as CPU 0's first task and create the idle
/// task. Call once, on CPU 0, with the heap and kernel stacks available.
pub fn start() -> Result<(), KernelError> {
    #[cfg(not(dispatch))]
    {
        Err(KernelError::NotImplemented {
            feature: "task dispatch on this architecture",
        })
    }
    #[cfg(dispatch)]
    {
        if started() {
            return Err(KernelError::AlreadyExists {
                resource: "dispatcher",
                id: 0,
            });
        }
        #[cfg(target_arch = "x86_64")]
        let cr3 = x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64();
        #[cfg(target_arch = "riscv64")]
        let cr3 = crate::arch::riscv64::switch::current_root();
        KERNEL_CR3.store(cr3, Ordering::Release);

        #[cfg(target_arch = "x86_64")]
        crate::arch::x86_64::switch::xsave::init();
        #[cfg_attr(not(target_arch = "x86_64"), allow(unused_mut))]
        let mut boot = Task::new(alloc_key(), "boot", Policy::default(), None);
        // The boot flow runs the nested user programs of the old model, so
        // its vector state is switched too.
        #[cfg(target_arch = "x86_64")]
        {
            *boot.xsave.get_mut() = crate::arch::x86_64::switch::xsave::Area::new();
        }
        // Its address space is the kernel's.
        #[cfg(target_arch = "riscv64")]
        {
            boot.arch.get_mut().root = cr3;
        }
        // The boot flow stays on the boot CPU: it is the timekeeper's
        // first task and runs the console.
        *boot.affinity.get_mut() = 1;
        let boot = Arc::new(boot);
        boot.set_state(State::Running);
        boot.on_cpu.store(true, Ordering::Release);

        let mut idle = new_kernel_task("idle", Policy::Idle, idle_entry, 0)?;
        idle.idle = true;
        let idle = Arc::new(idle);
        idle.set_state(State::Ready);

        let t = now();
        let mut rq = RunQueue::new();
        rq.enqueue(boot.key, *boot.entity.lock(), t);
        rq.pick_next(t);
        TASKS.lock().insert(boot.key, boot.clone());
        CURRENT[0].store(Arc::as_ptr(&boot) as *mut Task, Ordering::Release);
        let mut guard = CPUS[0].lock();
        let st = guard.insert(CpuSched {
            rq,
            current: boot,
            idle,
            prev: None,
            need_resched: false,
            acct_start: t,
        });
        publish(0, st);
        drop(guard);
        DISPATCHING.fetch_or(1, Ordering::AcqRel);
        STARTED.store(true, Ordering::Release);
        spawn_kernel("reaper", reaper, 0)?;
        Ok(())
    }
}

fn alloc_key() -> TaskKey {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

/// A kernel task whose first switch-in runs `entry(arg)`.
#[cfg(dispatch)]
fn new_kernel_task(
    name: &'static str,
    policy: Policy,
    entry: extern "C" fn(usize),
    arg: usize,
) -> Result<Task, KernelError> {
    let stack = crate::mm::kstack::allocate(KTHREAD_STACK_PAGES)?;
    let top = stack.base + stack.pages * 4096;
    let task = Task::new(alloc_key(), name, policy, Some(stack));
    // SAFETY: the stack was just allocated for this task and is unused.
    #[cfg(target_arch = "x86_64")]
    let sp = unsafe { crate::arch::x86_64::switch::seed_kernel_thread(top, entry, arg) };
    // SAFETY: as above.
    #[cfg(target_arch = "riscv64")]
    let sp = unsafe { crate::arch::riscv64::switch::seed_kernel_thread(top, entry, arg) };
    // SAFETY: the task is not shared yet.
    unsafe {
        *task.saved_sp.get() = sp;
        let arch = &mut *task.arch.get();
        #[cfg(target_arch = "x86_64")]
        {
            arch.cr3 = KERNEL_CR3.load(Ordering::Acquire);
            arch.entry_stack = top as u64;
        }
        #[cfg(target_arch = "riscv64")]
        {
            arch.root = KERNEL_CR3.load(Ordering::Acquire);
        }
    }
    Ok(task)
}

/// Start a kernel thread running `entry(arg)`. Returns its key.
pub fn spawn_kernel(
    name: &'static str,
    entry: extern "C" fn(usize),
    arg: usize,
) -> Result<TaskKey, KernelError> {
    #[cfg(not(dispatch))]
    {
        let _ = (name, entry, arg);
        Err(KernelError::NotImplemented {
            feature: "kernel threads on this architecture",
        })
    }
    #[cfg(dispatch)]
    {
        if !started() {
            return Err(KernelError::InvalidState {
                expected: "dispatcher started",
                actual: "not started",
            });
        }
        let task = Arc::new(new_kernel_task(name, Policy::default(), entry, arg)?);
        let key = task.key;
        TASKS.lock().insert(key, task.clone());
        enqueue_new(&task);
        Ok(key)
    }
}

/// Start a kernel thread bound to CPU `cpu`, which must dispatch (Linux
/// `kthread_bind`: it is never placed or balanced elsewhere). Returns its
/// key.
pub fn spawn_kernel_on(
    cpu: usize,
    name: &'static str,
    entry: extern "C" fn(usize),
    arg: usize,
) -> Result<TaskKey, KernelError> {
    #[cfg(not(dispatch))]
    {
        let _ = (cpu, name, entry, arg);
        Err(KernelError::NotImplemented {
            feature: "kernel threads on this architecture",
        })
    }
    #[cfg(dispatch)]
    {
        if !dispatching(cpu) {
            return Err(KernelError::InvalidArgument {
                name: "cpu",
                value: "does not dispatch",
            });
        }
        let task = Arc::new(new_kernel_task(name, Policy::default(), entry, arg)?);
        let key = task.key;
        task.cpu.store(cpu, Ordering::Release);
        task.affinity.store(1 << cpu, Ordering::Release);
        TASKS.lock().insert(key, task.clone());
        let irq = irq_save();
        if let Some(st) = CPUS[cpu].lock().as_mut() {
            st.rq.enqueue(key, *task.entity.lock(), now());
            publish(cpu, st);
        }
        irq_restore(irq);
        kick_cpu(cpu);
        Ok(key)
    }
}

/// The running task.
pub fn current() -> Option<Arc<Task>> {
    if !started() {
        return None;
    }
    let cpu = this_cpu()?;
    let irq = irq_save();
    let cur = CPUS[cpu].lock().as_ref().map(|st| st.current.clone());
    irq_restore(irq);
    cur
}

/// Switch to the most eligible task (possibly the current one).
pub fn schedule() {
    if !started() {
        return;
    }
    let Some(cpu) = this_cpu() else {
        return;
    };
    let decide_start = super::metrics::read_tsc();
    let irq = irq_save();
    // Raw pointers: neither task's `Arc` may live on the outgoing stack
    // across the switch (a dead task never comes back to drop it). The
    // CPU state keeps both alive: `prev` until finish_switch, `current`
    // while it runs.
    let switch: Option<(*const Task, *const Task, bool)> = {
        let mut guard = CPUS[cpu].lock();
        let st = guard.as_mut().expect("dispatching CPU has state");
        st.need_resched = false;
        let t = now();
        st.charge(t);
        // A running task whose CPU is no longer allowed (sched_setaffinity)
        // leaves the run queue here; finish_switch places it elsewhere once
        // it is off this CPU.
        let cur = st.current.clone();
        if cur.state() == State::Running
            && !allows(cur.affinity.load(Ordering::Acquire), cpu)
            && st.rq.contains(cur.key)
        {
            if let Some(entity) = st.rq.dequeue(cur.key, t) {
                *cur.entity.lock() = entity;
            }
            cur.migrate_out.store(true, Ordering::Release);
        }
        drop(cur);
        let next = match st.rq.pick_next(t) {
            Some(key) => lookup(key).expect("queued task is registered"),
            None => st.idle.clone(),
        };
        let switch = if Arc::ptr_eq(&next, &st.current) {
            None
        } else {
            let prev = core::mem::replace(&mut st.current, next.clone());
            prev.last_ran.store(t, Ordering::Release);
            // Still running: preempted; otherwise it blocked or exited.
            let preempted = prev.state() == State::Running;
            prev.usage.switched(!preempted);
            if preempted {
                prev.set_state(State::Ready);
            }
            next.set_state(State::Running);
            next.on_cpu.store(true, Ordering::Release);
            CURRENT[cpu].store(Arc::as_ptr(&next) as *mut Task, Ordering::Release);
            let pair = (Arc::as_ptr(&prev), Arc::as_ptr(&next), !preempted);
            st.prev = Some(prev);
            Some(pair)
        };
        publish(cpu, st);
        switch
    };
    if let Some((prev, next, voluntary)) = switch {
        // The shell's and panel's switch counters (the cycles taken to
        // decide the switch, not the switch itself).
        super::metrics::SCHEDULER_METRICS.record_context_switch(
            super::metrics::read_tsc().saturating_sub(decide_start),
            voluntary,
        );
        // SAFETY: interrupts are off, no dispatch lock is held, both tasks
        // are kept alive by this CPU's state, and `next` was seeded or last
        // switched out by `arch_switch`.
        unsafe { arch_switch(prev, next) };
        // Back in `prev`, switched in again by some later `schedule`.
        finish_switch();
    }
    irq_restore(irq);
}

/// Save the outgoing task's architecture state, load the incoming one's,
/// and switch stacks.
///
/// # Safety
/// See `schedule`.
unsafe fn arch_switch(prev: *const Task, next: *const Task) {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    // SAFETY: per the contract; this CPU is the only one touching either
    // task's saved state now.
    unsafe {
        use crate::arch::x86_64::switch;
        let cpu = crate::arch::percpu::this_arch_cpu();
        let a = &mut *(*prev).arch.get();
        a.cr3 = x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64();
        a.fs_base = switch::read_fs_base();
        a.user_gs_base = switch::read_user_gs_base();
        a.entry_stack = (*cpu).kernel_rsp;
        a.user_rsp = (*cpu).user_rsp;
        a.syscall_frame = (*cpu).syscall_frame;

        if let Some(area) = (*(*prev).xsave.get()).as_mut() {
            area.save();
        }
        if let Some(area) = (*(*next).xsave.get()).as_ref() {
            area.restore();
        }

        let b = &*(*next).arch.get();
        switch::switch_address_space(b.cr3);
        // Validated where it was set; a non-canonical value would #GP here.
        switch::write_fs_base(if crate::arch::x86_64::trap::is_user_address(b.fs_base) {
            b.fs_base
        } else {
            0
        });
        // Read from the MSR when the task was switched out, so canonical.
        switch::write_user_gs_base(b.user_gs_base);
        crate::arch::percpu::set_entry_stack(b.entry_stack);
        (*cpu).user_rsp = b.user_rsp;
        (*cpu).syscall_frame = b.syscall_frame;

        switch::switch_stacks((*prev).saved_sp.get(), *(*next).saved_sp.get());
    }
    // RISC-V: the address space, then the stacks. Kernel tasks carry the
    // kernel root; user state (FP, tp) comes with user mode.
    #[cfg(all(target_arch = "riscv64", dispatch))]
    // SAFETY: as above.
    unsafe {
        use crate::arch::riscv64::switch;
        (*(*prev).arch.get()).root = switch::current_root();
        switch::switch_address_space((*(*next).arch.get()).root);
        switch::switch_stacks((*prev).saved_sp.get(), *(*next).saved_sp.get());
    }
    #[cfg(not(dispatch))]
    let _ = (prev, next);
}

/// First thing on this CPU after a switch: release the task switched away
/// from, and reap it if it exited. Called by `schedule` and by the start
/// trampolines of new tasks.
pub extern "C" fn finish_switch() {
    let Some(cpu) = this_cpu() else {
        return;
    };
    let prev = CPUS[cpu].lock().as_mut().and_then(|st| st.prev.take());
    if let Some(prev) = prev {
        prev.on_cpu.store(false, Ordering::Release);
        if prev.migrate_out.swap(false, Ordering::AcqRel) {
            push(&prev);
        }
        if prev.state() == State::Dead {
            // Reaping may sleep (the process's locks) and frees the kernel
            // stack with a TLB shootdown, so it is the reaper thread's job:
            // this runs on whatever task comes next, often the idle task,
            // which must never sleep.
            let irq = irq_save();
            DEAD.lock().push(prev);
            irq_restore(irq);
            REAP.wake_all();
        }
    }
}

/// Exited tasks, off their CPUs, waiting for the reaper.
static DEAD: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());

/// The reaper sleeps here until `DEAD` has work.
static REAP: WaitQueue = WaitQueue::new();

/// The reaper thread: frees exited tasks (their kernel stacks, the page
/// tables of a process whose last task this was) and wakes their joiners.
extern "C" fn reaper(_: usize) {
    loop {
        REAP.wait_until(|| {
            let irq = irq_save();
            let pending = !DEAD.lock().is_empty();
            irq_restore(irq);
            pending
        });
        let irq = irq_save();
        let dead = core::mem::take(&mut *DEAD.lock());
        irq_restore(irq);
        for task in dead {
            reap(task);
        }
    }
}

/// Free exited `task`, which no CPU uses any more.
fn reap(task: Arc<Task>) {
    let owner = task.owner;
    // Before the task leaves the table: see `process_for_reap`.
    let process = owner.and_then(|(pid, _)| crate::process::process_for_reap(pid));
    // What the thread ran after its exit took its CPU time (tearing itself
    // down, switching away) moves to the process too, before the task stops
    // counting as live: dropped, it made the process's CPU clock go back.
    if let Some(p) = &process {
        p.add_exited_cpu_from(|| task.usage.take());
    }
    TASKS.lock().remove(&task.key);
    // Dropping the last reference frees the kernel stack.
    drop(task);
    if let Some((_, tid)) = owner {
        crate::process::user_task_reaped(process, tid);
    }
    EXITED.wake_all();
}

/// Let other runnable tasks of at least the current one's class run.
pub fn yield_now() {
    if !started() {
        return;
    }
    let Some(cpu) = this_cpu() else {
        return;
    };
    let irq = irq_save();
    if let Some(st) = CPUS[cpu].lock().as_mut() {
        st.rq.yield_current(now());
    }
    irq_restore(irq);
    schedule();
}

/// Block the running task until [`wake`]. The caller must have set its
/// state to `Blocked` (through [`WaitQueue::wait_until`] or
/// [`prepare_to_block`]) before it checked its wait condition; if a wake
/// arrived since, this returns at once.
pub fn block_current() {
    if !started() {
        return;
    }
    let Some(cpu) = this_cpu() else {
        return;
    };
    let irq = irq_save();
    let blocked = {
        let mut guard = CPUS[cpu].lock();
        let st = guard.as_mut().expect("dispatching CPU has state");
        let me = st.current.clone();
        if me.state() == State::Blocked {
            if let Some(entity) = st.rq.dequeue(me.key, now()) {
                *me.entity.lock() = entity;
            }
            publish(cpu, st);
            true
        } else {
            false
        }
    };
    if blocked {
        schedule();
    }
    irq_restore(irq);
}

/// Mark the running task as about to block (see [`block_current`]).
pub fn prepare_to_block() {
    if let Some(me) = current() {
        me.set_state(State::Blocked);
    }
}

/// Undo [`prepare_to_block`] when the condition turned true before blocking.
fn cancel_block(me: &Task) {
    // Blocked -> Running; a concurrent wake may already have made it Ready,
    // which also means it is still queued and running.
    me.set_state(State::Running);
}

/// Make a blocked task runnable. Returns false if it was not blocked.
pub fn wake(task: &Arc<Task>) -> bool {
    if task
        .state
        .compare_exchange(
            State::Blocked as u8,
            State::Ready as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return false;
    }
    // Only this waker may queue the task now: it is Ready, so no other wake
    // succeeds, and balancing moves only queued tasks.
    let mut waiting_since: Option<u64> = None;
    let mut reported = false;
    loop {
        let old = task.cpu.load(Ordering::Acquire);
        let irq = irq_save();
        let guard = CPUS[old].lock();
        if guard.as_ref().is_some_and(|st| st.rq.contains(task.key)) {
            // It had not reached block_current yet: it is still queued (and
            // running) where it was.
            drop(guard);
            irq_restore(irq);
            return true;
        }
        if task.on_cpu() {
            // Still being switched away from on `old` (another CPU, with
            // interrupts off): it may run nowhere until that is done
            // (Linux try_to_wake_up waits for `on_cpu` the same way).
            drop(guard);
            irq_restore(irq);
            let since = *waiting_since.get_or_insert_with(now);
            if !reported && now().saturating_sub(since) > 100_000_000 {
                reported = true;
                report_stuck_wake(task, old);
            }
            core::hint::spin_loop();
            continue;
        }
        drop(guard);
        // Off every queue and every CPU: it may go wherever it runs best.
        let cpu = place(task, old);
        task.cpu.store(cpu, Ordering::Release);
        let kick = CPUS[cpu]
            .lock()
            .as_mut()
            .is_some_and(|st| requeue(cpu, st, task));
        irq_restore(irq);
        if kick {
            kick_cpu(cpu);
        }
        return true;
    }
}

/// A wake has waited over 100 ms for `task` to leave CPU `cpu`: report it
/// on the raw console (no locks), once. A switch takes microseconds, so
/// this means a task marked as on a CPU that no longer runs it.
fn report_stuck_wake(task: &Arc<Task>, cpu: usize) {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    // SAFETY: raw COM1 output, no locks taken; only atomics are read.
    unsafe {
        use crate::arch::x86_64::idt::{raw_serial_hex as hex, raw_serial_str as out};
        out(b"[SCHED] wake still waiting after 100 ms: task 0x");
        hex(Arc::as_ptr(task) as u64);
        out(b" key 0x");
        hex(task.key);
        out(b" pid 0x");
        hex(task.owner.map_or(0, |o| o.0));
        out(b" tid 0x");
        hex(task.owner.map_or(0, |o| o.1));
        out(b" cpu 0x");
        hex(cpu as u64);
        out(b" state 0x");
        hex(task.state.load(Ordering::Acquire) as u64);
        out(b" here 0x");
        hex(crate::arch::percpu::this_cpu_id() as u64);
        for (c, cur) in CURRENT.iter().enumerate() {
            if dispatching(c) {
                out(b" cur");
                hex(c as u64);
                out(b"=0x");
                hex(cur.load(Ordering::Acquire) as u64);
            }
        }
        out(b"\n");
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    let _ = (task, cpu);
}

/// Put `task` on `st`, the run queue of CPU `cpu`, unless it is still there
/// (it had not reached `block_current` yet). Returns whether that CPU must
/// be told to reschedule: the task should preempt what runs there, or the
/// CPU is idle (it may be halted).
fn requeue(cpu: usize, st: &mut CpuSched, task: &Arc<Task>) -> bool {
    if st.rq.contains(task.key) {
        return false;
    }
    let t = now();
    st.rq.enqueue(task.key, *task.entity.lock(), t);
    if st.rq.should_preempt(task.key, t) {
        st.need_resched = true;
    }
    publish(cpu, st);
    st.need_resched || Arc::ptr_eq(&st.current, &st.idle)
}

/// Queue `task`, which is on no run queue and no CPU, where it runs best.
fn push(task: &Arc<Task>) {
    let cpu = place(task, task.cpu.load(Ordering::Acquire));
    task.cpu.store(cpu, Ordering::Release);
    let irq = irq_save();
    let kick = CPUS[cpu]
        .lock()
        .as_mut()
        .is_some_and(|st| requeue(cpu, st, task));
    irq_restore(irq);
    if kick {
        kick_cpu(cpu);
    }
}

/// Queue a new task where it runs best, starting from this CPU (an idle
/// allowed CPU if there is one; CPU 0 before any other dispatches).
fn enqueue_new(task: &Arc<Task>) {
    let here = this_cpu().unwrap_or(0);
    // Its cache is with its creator.
    task.last_ran.store(now(), Ordering::Release);
    task.cpu.store(here, Ordering::Release);
    push(task);
}

/// Make CPU `cpu` look at its run queue: an interrupt (the reschedule IPI)
/// ends its halt, or brings it to its next preemption point. Nothing to do
/// for this CPU: it reschedules at its own next point.
fn kick_cpu(cpu: usize) {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    if Some(cpu) != this_cpu() {
        let _ = crate::arch::x86_64::apic::send_ipi_to_cpu(
            cpu,
            crate::arch::x86_64::apic::SCHED_WAKE_VECTOR,
        );
    }
    #[cfg(all(target_arch = "riscv64", dispatch))]
    if Some(cpu) != this_cpu() {
        crate::arch::riscv64::switch::kick(cpu);
    }
    #[cfg(not(dispatch))]
    let _ = cpu;
}

/// Make `task` look at its signals now: wake it if it sleeps, or, if it is
/// running on another CPU, interrupt that CPU (the reschedule IPI): the
/// return to user mode from that interrupt delivers them. Without the
/// interrupt a thread spinning in user mode would not notice until that
/// CPU's next tick.
pub fn notify(task: &Arc<Task>) {
    if wake(task) || !task.on_cpu() {
        return;
    }
    let cpu = task.cpu.load(Ordering::Acquire);
    if dispatching(cpu) {
        kick_cpu(cpu);
    }
}

/// Wake a task by key.
pub fn wake_key(key: TaskKey) -> bool {
    lookup(key).is_some_and(|t| wake(&t))
}

/// Terminate the running kernel thread. Never returns.
pub extern "C" fn exit_kernel_thread() -> ! {
    exit_current_task()
}

/// Terminate the running task (kernel thread or user thread, after the
/// caller released what the thread owns). It is reaped once off the CPU.
/// Never returns.
pub fn exit_current_task() -> ! {
    if let Some(cpu) = this_cpu() {
        let irq = irq_save();
        {
            let mut guard = CPUS[cpu].lock();
            if let Some(st) = guard.as_mut() {
                let key = st.current.key;
                st.current.set_state(State::Dead);
                st.rq.dequeue(key, now());
                publish(cpu, st);
            }
        }
        schedule();
        irq_restore(irq);
    }
    unreachable!("a dead task was switched back in");
}

/// Wait until task `key` has exited and been reaped.
pub fn join(key: TaskKey) {
    EXITED.wait_until(|| lookup(key).is_none());
}

/// Timer tick on this CPU (interrupt context), which interrupted user mode
/// if `user`. Charges the running task its time and the tick sample, and
/// marks it for rescheduling when the policy says so; the switch itself
/// happens at the next scheduling point.
pub fn tick(user: bool) {
    if !started() {
        return;
    }
    let Some(cpu) = this_cpu() else {
        return;
    };
    if let Some(mut guard) = CPUS[cpu].try_lock() {
        if let Some(st) = guard.as_mut() {
            let t = now();
            st.charge(t);
            st.current.usage.tick(user);
            if st.rq.tick(t) {
                st.need_resched = true;
            }
            publish(cpu, st);
        }
    }
    // A waiting deadline task that would run sooner here moves now; the
    // other classes balance less often.
    pull_dl(cpu, true);
    let t = now();
    let stat = &STATS[cpu];
    if t.saturating_sub(stat.balanced_at.load(Ordering::Relaxed)) >= balance::BALANCE_INTERVAL_NS {
        stat.balanced_at.store(t, Ordering::Relaxed);
        pull(cpu, false, true);
    }
    // Machine-wide work on the timekeeping CPU only.
    if crate::arch::percpu::is_timekeeper() {
        super::loadavg::tick(now(), runnable_everywhere());
        wake_expired_sleepers();
    }
}

/// Runnable tasks on every dispatching CPU (try-locked: a CPU whose lock
/// is held is left out of this sample).
fn runnable_everywhere() -> u64 {
    (0..MAX_CPUS)
        .filter(|&c| dispatching(c))
        .filter_map(|c| {
            CPUS[c]
                .try_lock()
                .and_then(|g| g.as_ref().map(|st| st.rq.nr_running()))
        })
        .sum::<usize>() as u64
}

/// Wake the sleepers whose deadline has passed (interrupt context). Locks
/// are only tried: an interrupted holder keeps them, and a sleeper not
/// woken now is woken on a later tick.
fn wake_expired_sleepers() {
    let t = now();
    let Some(mut sleepers) = SLEEPERS.try_lock() else {
        return;
    };
    let mut i = 0;
    while i < sleepers.len() {
        if sleepers[i].0 <= t && try_wake(&sleepers[i].1) {
            sleepers.swap_remove(i);
        } else {
            i += 1;
        }
    }
}

/// [`wake`] for interrupt context: returns false, changing nothing, when
/// the run queue lock is held (the caller retries later). A task that is
/// not blocked counts as woken.
fn try_wake(task: &Arc<Task>) -> bool {
    let old = task.cpu.load(Ordering::Acquire);
    let Some(mut old_guard) = CPUS[old].try_lock() else {
        return false;
    };
    if task.state() != State::Blocked {
        return true;
    }
    // Moved only when it is off every queue and every CPU (as in `wake`),
    // and only if the target's lock is free: an interrupt may not wait.
    let movable = old_guard
        .as_ref()
        .is_some_and(|st| !st.rq.contains(task.key))
        && !task.on_cpu();
    let target = if movable { place(task, old) } else { old };
    let mut target_guard = if target != old {
        CPUS[target].try_lock()
    } else {
        None
    };
    let cpu = if target_guard.is_some() { target } else { old };
    if task
        .state
        .compare_exchange(
            State::Blocked as u8,
            State::Ready as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return true;
    }
    task.cpu.store(cpu, Ordering::Release);
    let st = match target_guard.as_mut() {
        Some(g) => g.as_mut(),
        None => old_guard.as_mut(),
    };
    let kick = st.is_some_and(|st| requeue(cpu, st, task));
    drop(target_guard);
    drop(old_guard);
    if kick {
        kick_cpu(cpu);
    }
    true
}

fn add_sleeper(deadline: u64, t: Arc<Task>) {
    let irq = irq_save();
    SLEEPERS.lock().push((deadline, t));
    irq_restore(irq);
}

fn remove_sleeper(t: &Arc<Task>) {
    let irq = irq_save();
    SLEEPERS.lock().retain(|(_, s)| !Arc::ptr_eq(s, t));
    irq_restore(irq);
}

/// Why a [`wait_event`] ended without its condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    /// A signal must be acted on (the caller fails with EINTR and unwinds).
    Interrupted,
    /// The deadline passed.
    TimedOut,
}

/// The current `monotonic_ns` clock, for [`wait_event`] deadlines.
pub fn clock_ns() -> u64 {
    now()
}

/// Sleep until `cond` holds, a signal must be acted on, or `deadline`
/// (`monotonic_ns`; see [`clock_ns`]) passes. The task really blocks:
/// whoever makes `cond` true calls `wq.wake_all()` (or `wake_one`), a
/// signal wakes every task of its process, and the timer wakes the task at
/// its deadline. `cond` is checked after the task is queued, so a wake in
/// between is never lost. Outside a dispatched task it polls.
///
/// The caller must hold no spinlock: the task sleeps here.
pub fn wait_event(
    wq: &WaitQueue,
    deadline: Option<u64>,
    mut cond: impl FnMut() -> bool,
) -> Result<(), WaitError> {
    let check = |cond: &mut dyn FnMut() -> bool| {
        if cond() {
            Some(Ok(()))
        } else if crate::process::wait_interrupted() {
            Some(Err(WaitError::Interrupted))
        } else if deadline.is_some_and(|d| now() >= d) {
            Some(Err(WaitError::TimedOut))
        } else {
            None
        }
    };
    let Some(me) = current().filter(|t| !t.idle && (current_owner().is_some() || started())) else {
        loop {
            if let Some(r) = check(&mut cond) {
                return r;
            }
            core::hint::spin_loop();
        }
    };
    loop {
        me.set_state(State::Blocked);
        wq.add(me.clone());
        if let Some(d) = deadline {
            add_sleeper(d, me.clone());
        }
        if let Some(r) = check(&mut cond) {
            cancel_block(&me);
            wq.remove(&me);
            remove_sleeper(&me);
            return r;
        }
        block_current();
        wq.remove(&me);
        remove_sleeper(&me);
    }
}

/// Preemption point on the way back to user mode (stage D3): if the tick
/// found the running task's slice used up, or a woken task should run
/// first, switch now. Only user tasks are preempted -- the kernel itself is
/// not preemptible -- and only here, where the task holds no locks and its
/// whole user context is in the frame on its own kernel stack.
pub fn preempt_user() {
    if current_owner().is_some() && need_resched() {
        schedule();
    }
}

/// Whether the running task should give up the CPU.
pub fn need_resched() -> bool {
    let Some(cpu) = this_cpu() else {
        return false;
    };
    let irq = irq_save();
    let r = CPUS[cpu].lock().as_ref().is_some_and(|st| st.need_resched);
    irq_restore(irq);
    r
}

/// Most tasks one balancing pass moves.
const MAX_PULL: usize = 8;

/// Move queued tasks from the busiest CPU to `this` when the load is out of
/// balance ([`balance::find_busiest`]); with `newly_idle`, any waiting task
/// will do. A task moves only if it is queued but not running and not
/// still being switched away from (`on_cpu`), `this` is allowed for it,
/// and -- unless `this` is idle -- its cache has gone cold. Both CPU locks
/// are taken in CPU order; in interrupt context (`in_irq`) every lock is
/// only tried (the heap masks interrupts, so the run queue may allocate).
/// Returns whether a task moved.
fn pull(this: usize, newly_idle: bool, in_irq: bool) -> bool {
    let snap = snapshots();
    let Some(src) = balance::find_busiest(this, &snap, newly_idle) else {
        return false;
    };
    let want = balance::tasks_to_move(&snap[src], &snap[this]).min(MAX_PULL);
    let (lo, hi) = (src.min(this), src.max(this));
    let irq = irq_save();
    let guards = if in_irq {
        CPUS[lo]
            .try_lock()
            .and_then(|a| CPUS[hi].try_lock().map(|b| (a, b)))
    } else {
        let a = CPUS[lo].lock();
        Some((a, CPUS[hi].lock()))
    };
    let mut moved = 0;
    if let Some((mut g_lo, mut g_hi)) = guards {
        let (s, d) = if src < this {
            (g_lo.as_mut(), g_hi.as_mut())
        } else {
            (g_hi.as_mut(), g_lo.as_mut())
        };
        let tasks = if in_irq {
            TASKS.try_lock()
        } else {
            Some(TASKS.lock())
        };
        if let (Some(s), Some(d), Some(tasks)) = (s, d, tasks) {
            let t = now();
            let mut keys = [0 as TaskKey; MAX_PULL];
            let mut n = 0;
            for k in s.rq.migratable().take(MAX_PULL) {
                keys[n] = k;
                n += 1;
            }
            for &k in &keys[..n] {
                if moved == want {
                    break;
                }
                let Some(task) = tasks.get(&k) else {
                    continue;
                };
                if task.on_cpu()
                    || !allows(task.affinity.load(Ordering::Acquire), this)
                    || (!newly_idle
                        && t.saturating_sub(task.last_ran.load(Ordering::Acquire))
                            < balance::CACHE_HOT_NS)
                {
                    continue;
                }
                if let Some(entity) = s.rq.dequeue(k, t) {
                    task.cpu.store(this, Ordering::Release);
                    d.rq.enqueue(k, entity, t);
                    if d.rq.should_preempt(k, t) {
                        d.need_resched = true;
                    }
                    moved += 1;
                }
            }
            if moved > 0 {
                if Arc::ptr_eq(&d.current, &d.idle) {
                    d.need_resched = true;
                }
                publish(src, s);
                publish(this, d);
            }
        }
    }
    irq_restore(irq);
    moved > 0
}

/// Take one waiting deadline task that would run sooner on `this` than
/// where it waits ([`balance::find_dl_source`], rechecked under both
/// locks): global EDF across CPUs, approximately, as Linux's
/// `pull_dl_task`. Runs on every tick and when a CPU goes idle, and costs
/// one read per CPU when there is nothing to pull. The task keeps its
/// reservation. Locking as [`pull`]. Returns whether a task moved.
fn pull_dl(this: usize, in_irq: bool) -> bool {
    let snap = snapshots();
    let Some(src) = balance::find_dl_source(this, &snap) else {
        return false;
    };
    let (lo, hi) = (src.min(this), src.max(this));
    let irq = irq_save();
    let guards = if in_irq {
        CPUS[lo]
            .try_lock()
            .and_then(|a| CPUS[hi].try_lock().map(|b| (a, b)))
    } else {
        let a = CPUS[lo].lock();
        Some((a, CPUS[hi].lock()))
    };
    let mut moved = false;
    if let Some((mut g_lo, mut g_hi)) = guards {
        let (s, d) = if src < this {
            (g_lo.as_mut(), g_hi.as_mut())
        } else {
            (g_hi.as_mut(), g_lo.as_mut())
        };
        let tasks = if in_irq {
            TASKS.try_lock()
        } else {
            Some(TASKS.lock())
        };
        if let (Some(s), Some(d), Some(tasks)) = (s, d, tasks) {
            let t = now();
            let mine = d.rq.dl_earliest().unwrap_or(balance::NO_DL);
            let pick =
                s.rq.dl_waiting()
                    .take_while(|&(deadline, _)| deadline < mine)
                    .map(|(_, k)| k)
                    .find(|k| {
                        tasks.get(k).is_some_and(|task| {
                            !task.on_cpu() && allows(task.affinity.load(Ordering::Acquire), this)
                        })
                    });
            if let Some(k) = pick {
                if let (Some(task), Some(entity)) = (tasks.get(&k), s.rq.dequeue(k, t)) {
                    task.cpu.store(this, Ordering::Release);
                    d.rq.migrate_in(k, entity, t);
                    if d.rq.should_preempt(k, t) || Arc::ptr_eq(&d.current, &d.idle) {
                        d.need_resched = true;
                    }
                    publish(src, s);
                    publish(this, d);
                    moved = true;
                }
            }
        }
    }
    irq_restore(irq);
    moved
}

fn has_runnable(cpu: usize) -> bool {
    CPUS[cpu]
        .lock()
        .as_ref()
        .is_some_and(|st| !st.rq.is_empty())
}

/// Body of every CPU's idle task: run whatever becomes runnable, halt
/// otherwise. The check and the halt are atomic with respect to interrupts
/// (`sti; hlt`), so a wake from an interrupt cannot be missed.
extern "C" fn idle_entry(_: usize) {
    loop {
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            x86_64::instructions::interrupts::disable();
            // Nothing here: take waiting work from a busier CPU first.
            if this_cpu().is_some_and(|cpu| {
                has_runnable(cpu) || pull_dl(cpu, false) || pull(cpu, true, false)
            }) {
                x86_64::instructions::interrupts::enable();
                schedule();
            } else {
                x86_64::instructions::interrupts::enable_and_hlt();
            }
        }
        #[cfg(all(target_arch = "riscv64", dispatch))]
        {
            use crate::arch::riscv64::switch;
            switch::irq_save();
            if this_cpu().is_some_and(|cpu| {
                has_runnable(cpu) || pull_dl(cpu, false) || pull(cpu, true, false)
            }) {
                switch::irq_enable();
                schedule();
            } else {
                switch::wait_for_interrupt();
            }
        }
        #[cfg(not(dispatch))]
        core::hint::spin_loop();
    }
}

/// The key of the task running on this CPU (0 before the dispatcher
/// starts), without taking any lock.
pub fn current_key() -> TaskKey {
    if !started() {
        return 0;
    }
    let Some(cpu) = this_cpu() else {
        return 0;
    };
    let p = CURRENT[cpu].load(Ordering::Acquire);
    if p.is_null() {
        return 0;
    }
    // SAFETY: as in `current_owner` -- the current task stays alive while
    // it is current, and only this CPU replaces it.
    unsafe { (*p).key }
}

/// The (pid, tid) of the user thread running on this CPU, without taking
/// any lock (safe in fault handlers).
pub fn current_owner() -> Option<(u64, u64)> {
    if !started() {
        return None;
    }
    let cpu = this_cpu()?;
    let p = CURRENT[cpu].load(Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    // SAFETY: CURRENT holds the task running on this CPU, which stays
    // alive (in the CPU state) while it is current; reading it from the
    // same CPU cannot race with its replacement.
    unsafe { (*p).owner }
}

/// Start a user thread as a task: its first switch-in enters ring 3 with
/// `frame`, on address space `cr3`, with TLS base `fs_base` and vector
/// state `xsave` and user GS base `gs_base`, scheduled with the thread's
/// `policy` on the CPUs in `affinity`.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
pub fn spawn_user(
    owner: (u64, u64),
    frame: &crate::arch::x86_64::switch::UserFrame,
    cr3: u64,
    fs_base: u64,
    xsave: crate::arch::x86_64::switch::xsave::Area,
    gs_base: u64,
    policy: Policy,
    affinity: u64,
) -> Result<TaskKey, KernelError> {
    #[cfg(not(target_os = "none"))]
    {
        let _ = (owner, frame, cr3, fs_base, xsave, gs_base, policy, affinity);
        Err(KernelError::NotImplemented {
            feature: "user tasks on the host",
        })
    }
    #[cfg(target_os = "none")]
    {
        if !started() {
            return Err(KernelError::InvalidState {
                expected: "dispatcher started",
                actual: "not started",
            });
        }
        let stack = crate::mm::kstack::allocate(KTHREAD_STACK_PAGES)?;
        let top = stack.base + stack.pages * 4096;
        let mut task = Task::new(alloc_key(), "user", policy, Some(stack));
        task.owner = Some(owner);
        // A mask with no CPU that dispatches (inherited from a time with
        // more CPUs, say) would leave it nowhere to run: use them all.
        let usable = affinity & dispatching_mask();
        *task.affinity.get_mut() = if usable != 0 { affinity } else { u64::MAX };
        *task.xsave.get_mut() = Some(xsave);
        // SAFETY: the stack was just allocated for this task and is unused.
        let sp = unsafe { crate::arch::x86_64::switch::seed_user_thread(top, frame) };
        *task.saved_sp.get_mut() = sp;
        let arch = task.arch.get_mut();
        arch.cr3 = cr3;
        arch.fs_base = fs_base;
        // Written to the MSR at every switch-in: it must be canonical.
        arch.user_gs_base = if crate::arch::x86_64::trap::is_user_address(gs_base) {
            gs_base
        } else {
            0
        };
        arch.entry_stack = top as u64;
        let task = Arc::new(task);
        let key = task.key;
        TASKS.lock().insert(key, task.clone());
        enqueue_new(&task);
        Ok(key)
    }
}

/// Run the task of user thread `owner` (pid, tid) with `policy` from now on
/// (sched_setscheduler and friends; N-221). A queued or running task is
/// requeued in its new class, keeping its fair lag, and the CPU is asked to
/// reschedule (a raised priority may now preempt, a lowered one yield); a
/// sleeping one takes the policy when it wakes. No task (the thread does
/// not run under the dispatcher): nothing to change.
pub fn set_policy(owner: (u64, u64), policy: Policy) {
    let Some(task) = task_of_owner(owner) else {
        return;
    };
    let irq = irq_save();
    let mut queued = false;
    for cpu in CPUS.iter() {
        if let Some(st) = cpu.lock().as_mut() {
            if st.rq.set_policy(task.key, policy, now()) {
                queued = true;
                st.need_resched = true;
            }
        }
    }
    if !queued {
        task.entity.lock().set_policy(policy);
    }
    irq_restore(irq);
}

/// The task of user thread `owner` (pid, tid).
fn task_of_owner(owner: (u64, u64)) -> Option<Arc<Task>> {
    TASKS
        .lock()
        .values()
        .find(|t| t.owner == Some(owner))
        .cloned()
}

/// Set or clear the priority-inheritance boost of user thread `owner`
/// (SCHED-INC-01): a queued or running task is requeued in its boosted
/// class and its CPU asked to reschedule; a sleeping one takes the boost
/// when it wakes. No task: nothing to change.
pub fn set_boost(owner: (u64, u64), boost: Option<u8>) {
    let Some(task) = task_of_owner(owner) else {
        return;
    };
    let irq = irq_save();
    let mut queued = false;
    for cpu in CPUS.iter() {
        if let Some(st) = cpu.lock().as_mut() {
            if st.rq.set_boost(task.key, boost, now()) {
                queued = true;
                st.need_resched = true;
            }
        }
    }
    if !queued {
        task.entity.lock().boost = boost;
    }
    irq_restore(irq);
}

/// The priority user thread `owner` passes on to the owner of a
/// priority-inheritance lock it waits for (`Entity::pi_priority`), or
/// `None` if it has no task.
pub fn pi_priority(owner: (u64, u64)) -> Option<u8> {
    let task = task_of_owner(owner)?;
    let irq = irq_save();
    let mut found = None;
    for cpu in CPUS.iter() {
        if let Some(st) = cpu.lock().as_ref() {
            if let Some(e) = st.rq.entity_of(task.key) {
                found = Some(e.pi_priority());
                break;
            }
        }
    }
    let prio = found.unwrap_or_else(|| task.entity.lock().pi_priority());
    irq_restore(irq);
    Some(prio)
}

/// Number of CPUs that run tasks (sched_getaffinity's online set, the
/// processor count programs see, deadline admission).
pub fn online_cpus() -> usize {
    DISPATCHING.load(Ordering::Acquire).count_ones() as usize
}

/// The CPUs that run tasks, a bit per CPU.
pub fn dispatching_mask() -> u64 {
    DISPATCHING.load(Ordering::Acquire)
}

/// Restrict user thread `owner` (pid, tid) to the CPUs in `mask`, which
/// must include a dispatching CPU (sched_setaffinity checks). A queued task
/// on a CPU no longer allowed moves at once; a running one is told to
/// reschedule, and `schedule` moves it (a thread that changes its own mask
/// has moved before its system call returns, at the preemption point on
/// the way out); a sleeping one is placed by its wake.
pub fn set_affinity(owner: (u64, u64), mask: u64) {
    let Some(task) = task_of_owner(owner) else {
        return;
    };
    task.affinity.store(mask, Ordering::Release);
    loop {
        let cpu = task.cpu.load(Ordering::Acquire);
        if allows(mask, cpu) {
            return;
        }
        let irq = irq_save();
        let mut guard = CPUS[cpu].lock();
        let Some(st) = guard.as_mut() else {
            drop(guard);
            irq_restore(irq);
            return;
        };
        if task.cpu.load(Ordering::Acquire) != cpu {
            // Moved meanwhile (balanced or woken elsewhere): look again.
            drop(guard);
            irq_restore(irq);
            continue;
        }
        if Arc::ptr_eq(&st.current, &task) {
            st.need_resched = true;
            drop(guard);
            irq_restore(irq);
            kick_cpu(cpu);
            return;
        }
        if !st.rq.contains(task.key) {
            // Asleep: its wake places it by the new mask.
            drop(guard);
            irq_restore(irq);
            return;
        }
        if task.on_cpu() {
            // Being switched away from on `cpu`: wait until it is off.
            drop(guard);
            irq_restore(irq);
            core::hint::spin_loop();
            continue;
        }
        if let Some(entity) = st.rq.dequeue(task.key, now()) {
            *task.entity.lock() = entity;
        }
        publish(cpu, st);
        drop(guard);
        irq_restore(irq);
        push(&task);
        return;
    }
}

/// Make this secondary CPU dispatch (stage S2): its boot flow becomes its
/// idle task, and it runs whatever reaches its run queue. Called once by
/// the secondary after its local setup. Never returns.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn run_secondary() -> ! {
    let cpu = crate::arch::percpu::this_cpu_id() as usize;
    if !started() || cpu == 0 || cpu >= MAX_CPUS {
        loop {
            x86_64::instructions::interrupts::enable_and_hlt();
        }
    }
    let mut idle = Task::new(alloc_key(), "idle", Policy::Idle, None);
    idle.idle = true;
    let idle = Arc::new(idle);
    idle.cpu.store(cpu, Ordering::Release);
    idle.set_state(State::Running);
    idle.on_cpu.store(true, Ordering::Release);
    let irq = irq_save();
    CURRENT[cpu].store(Arc::as_ptr(&idle) as *mut Task, Ordering::Release);
    let mut guard = CPUS[cpu].lock();
    let st = guard.insert(CpuSched {
        rq: RunQueue::new(),
        current: idle.clone(),
        idle,
        prev: None,
        need_resched: false,
        acct_start: now(),
    });
    publish(cpu, st);
    drop(guard);
    DISPATCHING.fetch_or(1 << cpu, Ordering::AcqRel);
    irq_restore(irq);
    idle_entry(0);
    unreachable!("idle returned")
}

/// The CPU time of `task`, with the slice it is running now if it is this
/// CPU's current task.
fn task_cpu(task: &Task) -> super::cputime::CpuTimes {
    let Some(cpu) = this_cpu() else {
        return task.usage.snapshot();
    };
    // The snapshot and the running slice are read together, with
    // interrupts off and this CPU's lock held: a tick between them charged
    // the slice to the counter after the snapshot was taken, and the slice
    // added afterwards started at that tick, so up to a tick went missing
    // and the thread's CPU clock went back.
    let irq = irq_save();
    let mut times = task.usage.snapshot();
    if let Some(st) = CPUS[cpu].lock().as_ref() {
        if st.current.key == task.key {
            times.runtime_ns = times
                .runtime_ns
                .saturating_add(now().saturating_sub(st.acct_start));
        }
    }
    irq_restore(irq);
    times
}

/// The CPU time of the user thread `owner` (pid, tid), if a task runs it.
pub fn thread_cpu(owner: (u64, u64)) -> Option<super::cputime::CpuTimes> {
    let task = TASKS
        .lock()
        .values()
        .find(|t| t.owner == Some(owner))
        .cloned()?;
    Some(task_cpu(&task))
}

/// The CPU time of the live threads of process `pid` (the process adds what
/// its exited threads used).
pub fn process_tasks_cpu(pid: u64) -> super::cputime::CpuTimes {
    tasks_of(pid)
        .iter()
        .fold(super::cputime::CpuTimes::default(), |sum, t| {
            sum.plus(task_cpu(t))
        })
}

/// The running task's CPU time, taken from it (it is exiting: the process
/// keeps the total, so it is counted once).
pub fn take_current_usage() -> super::cputime::CpuTimes {
    let Some(cpu) = this_cpu() else {
        return super::cputime::CpuTimes::default();
    };
    let irq = irq_save();
    let taken = CPUS[cpu].lock().as_mut().map(|st| {
        st.charge(now());
        st.current.usage.take()
    });
    irq_restore(irq);
    taken.unwrap_or_default()
}

/// The (pid, tid) of every user thread that has a task.
pub fn owners() -> Vec<(u64, u64)> {
    TASKS.lock().values().filter_map(|t| t.owner).collect()
}

/// Live tasks running threads of process `pid`.
pub fn tasks_of(pid: u64) -> Vec<Arc<Task>> {
    TASKS
        .lock()
        .values()
        .filter(|t| t.owner.is_some_and(|(p, _)| p == pid))
        .cloned()
        .collect()
}

/// Wait inside a system call for "something to happen" (data, a child, a
/// timeout): let other tasks run if any are runnable, otherwise halt until
/// the next interrupt. The caller re-checks its condition afterwards.
///
/// Returns true when the wait must end instead: a signal is waiting to be
/// acted on (a fatal one, or one with a handler). The caller then fails
/// with EINTR, the system call unwinds normally -- dropping everything it
/// holds -- and the signal is acted on at the system-call exit. (Exiting
/// from in here leaked whatever the callers above held: security review of
/// 9302803.)
#[must_use]
pub fn wait_in_syscall() -> bool {
    if crate::process::wait_interrupted() {
        return true;
    }
    let others = this_cpu().is_some_and(|cpu| {
        let irq = irq_save();
        let n = CPUS[cpu].lock().as_ref().map_or(0, |st| st.rq.nr_running());
        irq_restore(irq);
        // The running task is still queued.
        n > 1
    });
    if others {
        yield_now();
    } else {
        // Halt until the next interrupt, then put the interrupt flag back as
        // it was (system calls run with interrupts on since N-139).
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            let was_on = x86_64::instructions::interrupts::are_enabled();
            x86_64::instructions::interrupts::enable_and_hlt();
            if !was_on {
                x86_64::instructions::interrupts::disable();
            }
        }
    }
    crate::process::wait_interrupted()
}

/// Number of live tasks (the idle tasks excluded).
pub fn task_count() -> usize {
    TASKS.lock().len()
}

/// Tasks waiting for an event.
pub struct WaitQueue {
    waiters: Mutex<Vec<Arc<Task>>>,
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            waiters: Mutex::new(Vec::new()),
        }
    }

    fn add(&self, t: Arc<Task>) {
        let irq = irq_save();
        self.waiters.lock().push(t);
        irq_restore(irq);
    }

    fn remove(&self, t: &Arc<Task>) {
        let irq = irq_save();
        self.waiters.lock().retain(|w| !Arc::ptr_eq(w, t));
        irq_restore(irq);
    }

    /// Block until `cond` returns true. `cond` is evaluated after the task
    /// is on the queue, so a [`WaitQueue::wake_all`] that makes it true is
    /// never missed. Before the dispatcher starts this polls.
    pub fn wait_until(&self, mut cond: impl FnMut() -> bool) {
        // An idle task must stay runnable (it runs when nothing else can):
        // it polls instead.
        let Some(me) = current().filter(|t| !t.idle) else {
            while !cond() {
                core::hint::spin_loop();
            }
            return;
        };
        loop {
            me.set_state(State::Blocked);
            self.add(me.clone());
            if cond() {
                cancel_block(&me);
                self.remove(&me);
                return;
            }
            block_current();
            self.remove(&me);
            if cond() {
                return;
            }
        }
    }

    /// [`WaitQueue::wake_all`] for interrupt context: locks are only tried,
    /// and waiters that could not be woken stay queued. Returns whether the
    /// queue was emptied; if not, the caller retries on a later interrupt.
    pub fn wake_all_from_irq(&self) -> bool {
        let Some(mut waiters) = self.waiters.try_lock() else {
            return false;
        };
        waiters.retain(|t| !try_wake(t));
        waiters.is_empty()
    }

    /// Wake every waiter.
    pub fn wake_all(&self) {
        let irq = irq_save();
        let waiters = core::mem::take(&mut *self.waiters.lock());
        irq_restore(irq);
        for t in &waiters {
            wake(t);
        }
    }

    /// Wake the longest-waiting task, if any.
    pub fn wake_one(&self) {
        let irq = irq_save();
        let first = {
            let mut w = self.waiters.lock();
            (!w.is_empty()).then(|| w.remove(0))
        };
        irq_restore(irq);
        if let Some(t) = first {
            wake(&t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips() {
        for s in [State::Ready, State::Running, State::Blocked, State::Dead] {
            assert_eq!(State::from_u8(s as u8), s);
        }
    }

    #[test]
    fn waiting_before_start_returns_when_the_condition_holds() {
        // Not started on the host: wait_until polls the condition.
        assert!(!started());
        let wq = WaitQueue::new();
        let mut calls = 0;
        wq.wait_until(|| {
            calls += 1;
            true
        });
        assert_eq!(calls, 1);
        wq.wake_all();
    }

    #[test]
    fn wake_refuses_a_task_that_is_not_blocked() {
        let t = Arc::new(Task::new(alloc_key(), "t", Policy::default(), None));
        t.set_state(State::Running);
        assert!(!wake(&t));
        t.set_state(State::Blocked);
        assert!(wake(&t));
        assert_eq!(t.state(), State::Ready);
        assert!(!wake(&t), "a second wake is a no-op");
    }
}

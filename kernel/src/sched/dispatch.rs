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
//! Only x86_64 has the switch primitive yet; elsewhere [`start`] declines
//! and every entry point is a no-op.

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicU8, Ordering},
};

use spin::Mutex;

use super::policy::{Entity, Policy, RunQueue, TaskKey};
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
    #[cfg(target_arch = "x86_64")]
    entry_stack: u64,
    #[cfg(target_arch = "x86_64")]
    user_rsp: u64,
    #[cfg(target_arch = "x86_64")]
    syscall_frame: u64,
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
}

/// Per-CPU dispatch state; only CPU 0 dispatches until stage D5.
static CPUS: [Mutex<Option<CpuSched>>; 1] = [const { Mutex::new(None) }];

/// The running task of each dispatching CPU, readable without the CPU lock
/// (for `current_process()` on every syscall and in fault handlers). Only
/// that CPU changes it, in `schedule` with interrupts off, and the task it
/// points to stays alive while it is current.
static CURRENT: [AtomicPtr<Task>; 1] = [const { AtomicPtr::new(core::ptr::null_mut()) }];

/// Woken whenever a process changes state (exit, stop, a fatal signal);
/// `wait` and the program launchers wait here.
pub static PROCESS_EVENTS: WaitQueue = WaitQueue::new();

/// Every live task except the idle tasks, by key.
static TASKS: Mutex<BTreeMap<TaskKey, Arc<Task>>> = Mutex::new(BTreeMap::new());

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Woken whenever a task exits (for [`join`]).
static EXITED: WaitQueue = WaitQueue::new();

/// The kernel's own page table root, for kernel threads (x86_64).
#[cfg(target_arch = "x86_64")]
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
    (cpu < CPUS.len()).then_some(cpu)
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
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        IrqFlag(false)
    }
}

fn irq_restore(flag: IrqFlag) {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    if flag.0 {
        x86_64::instructions::interrupts::enable();
    }
    let _ = flag;
}

fn lookup(key: TaskKey) -> Option<Arc<Task>> {
    TASKS.lock().get(&key).cloned()
}

/// Adopt the running boot flow as CPU 0's first task and create the idle
/// task. Call once, on CPU 0, with the heap and kernel stacks available.
pub fn start() -> Result<(), KernelError> {
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        Err(KernelError::NotImplemented {
            feature: "task dispatch on this architecture",
        })
    }
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        if started() {
            return Err(KernelError::AlreadyExists {
                resource: "dispatcher",
                id: 0,
            });
        }
        let cr3 = x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64();
        KERNEL_CR3.store(cr3, Ordering::Release);

        crate::arch::x86_64::switch::xsave::init();
        let mut boot = Task::new(alloc_key(), "boot", Policy::default(), None);
        // The boot flow runs the nested user programs of the old model, so
        // its vector state is switched too.
        *boot.xsave.get_mut() = crate::arch::x86_64::switch::xsave::Area::new();
        let boot = Arc::new(boot);
        boot.set_state(State::Running);
        boot.on_cpu.store(true, Ordering::Release);

        let idle = Arc::new(new_kernel_task("idle", Policy::Idle, idle_entry, 0)?);
        idle.set_state(State::Ready);

        let t = now();
        let mut rq = RunQueue::new();
        rq.enqueue(boot.key, *boot.entity.lock(), t);
        rq.pick_next(t);
        TASKS.lock().insert(boot.key, boot.clone());
        CURRENT[0].store(Arc::as_ptr(&boot) as *mut Task, Ordering::Release);
        *CPUS[0].lock() = Some(CpuSched {
            rq,
            current: boot,
            idle,
            prev: None,
            need_resched: false,
        });
        STARTED.store(true, Ordering::Release);
        Ok(())
    }
}

fn alloc_key() -> TaskKey {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

/// A kernel task whose first switch-in runs `entry(arg)`.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
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
    let sp = unsafe { crate::arch::x86_64::switch::seed_kernel_thread(top, entry, arg) };
    // SAFETY: the task is not shared yet.
    unsafe {
        *task.saved_sp.get() = sp;
        let arch = &mut *task.arch.get();
        arch.cr3 = KERNEL_CR3.load(Ordering::Acquire);
        arch.entry_stack = top as u64;
    }
    Ok(task)
}

/// Start a kernel thread running `entry(arg)`. Returns its key.
pub fn spawn_kernel(
    name: &'static str,
    entry: extern "C" fn(usize),
    arg: usize,
) -> Result<TaskKey, KernelError> {
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        let _ = (name, entry, arg);
        Err(KernelError::NotImplemented {
            feature: "kernel threads on this architecture",
        })
    }
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
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
        let irq = irq_save();
        if let Some(st) = CPUS[0].lock().as_mut() {
            st.rq.enqueue(key, *task.entity.lock(), now());
        }
        irq_restore(irq);
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
    let irq = irq_save();
    // Raw pointers: neither task's `Arc` may live on the outgoing stack
    // across the switch (a dead task never comes back to drop it). The
    // CPU state keeps both alive: `prev` until finish_switch, `current`
    // while it runs.
    let switch: Option<(*const Task, *const Task)> = {
        let mut guard = CPUS[cpu].lock();
        let st = guard.as_mut().expect("dispatching CPU has state");
        st.need_resched = false;
        let t = now();
        let next = match st.rq.pick_next(t) {
            Some(key) => lookup(key).expect("queued task is registered"),
            None => st.idle.clone(),
        };
        if Arc::ptr_eq(&next, &st.current) {
            None
        } else {
            let prev = core::mem::replace(&mut st.current, next.clone());
            if prev.state() == State::Running {
                prev.set_state(State::Ready);
            }
            next.set_state(State::Running);
            next.on_cpu.store(true, Ordering::Release);
            CURRENT[cpu].store(Arc::as_ptr(&next) as *mut Task, Ordering::Release);
            let pair = (Arc::as_ptr(&prev), Arc::as_ptr(&next));
            st.prev = Some(prev);
            Some(pair)
        }
    };
    if let Some((prev, next)) = switch {
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
        crate::arch::percpu::set_entry_stack(b.entry_stack);
        (*cpu).user_rsp = b.user_rsp;
        (*cpu).syscall_frame = b.syscall_frame;

        switch::switch_stacks((*prev).saved_sp.get(), *(*next).saved_sp.get());
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
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
        if prev.state() == State::Dead {
            TASKS.lock().remove(&prev.key);
            let owner = prev.owner;
            // Dropping the last reference frees its kernel stack, which no
            // CPU uses any more.
            drop(prev);
            if let Some((pid, tid)) = owner {
                crate::process::user_task_reaped(pid, tid);
            }
            EXITED.wake_all();
        }
    }
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
    let irq = irq_save();
    if let Some(st) = CPUS[0].lock().as_mut() {
        // Still queued if it had not reached `block_current` yet.
        if !st.rq.contains(task.key) {
            let t = now();
            st.rq.enqueue(task.key, *task.entity.lock(), t);
            if st.rq.should_preempt(task.key, t) {
                st.need_resched = true;
            }
        }
    }
    irq_restore(irq);
    true
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

/// Timer tick on this CPU (interrupt context). Marks the running task for
/// rescheduling when the policy says so; the switch itself happens at the
/// next scheduling point.
pub fn tick() {
    if !started() {
        return;
    }
    let Some(cpu) = this_cpu() else {
        return;
    };
    if let Some(mut guard) = CPUS[cpu].try_lock() {
        if let Some(st) = guard.as_mut() {
            if st.rq.tick(now()) {
                st.need_resched = true;
            }
        }
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
            if this_cpu().is_some_and(has_runnable) {
                x86_64::instructions::interrupts::enable();
                schedule();
            } else {
                x86_64::instructions::interrupts::enable_and_hlt();
            }
        }
        #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
        core::hint::spin_loop();
    }
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
/// state `xsave`.
#[cfg(target_arch = "x86_64")]
pub fn spawn_user(
    owner: (u64, u64),
    frame: &crate::arch::x86_64::switch::UserFrame,
    cr3: u64,
    fs_base: u64,
    xsave: crate::arch::x86_64::switch::xsave::Area,
) -> Result<TaskKey, KernelError> {
    #[cfg(not(target_os = "none"))]
    {
        let _ = (owner, frame, cr3, fs_base, xsave);
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
        let mut task = Task::new(alloc_key(), "user", Policy::default(), Some(stack));
        task.owner = Some(owner);
        *task.xsave.get_mut() = Some(xsave);
        // SAFETY: the stack was just allocated for this task and is unused.
        let sp = unsafe { crate::arch::x86_64::switch::seed_user_thread(top, frame) };
        *task.saved_sp.get_mut() = sp;
        let arch = task.arch.get_mut();
        arch.cr3 = cr3;
        arch.fs_base = fs_base;
        arch.entry_stack = top as u64;
        let task = Arc::new(task);
        let key = task.key;
        TASKS.lock().insert(key, task.clone());
        let irq = irq_save();
        if let Some(st) = CPUS[0].lock().as_mut() {
            st.rq.enqueue(key, *task.entity.lock(), now());
        }
        irq_restore(irq);
        Ok(key)
    }
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
pub fn wait_in_syscall() {
    // A thread whose process received a fatal signal exits here rather than
    // wait on (callers wait with no lock held). Without this a process
    // blocked in a read, poll or sleep could not be killed.
    crate::process::user_return_check();
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
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        x86_64::instructions::interrupts::enable_and_hlt();
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        x86_64::instructions::interrupts::disable();
    }
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
        let Some(me) = current() else {
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

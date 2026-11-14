//! Process management module
//!
//! This module provides the core process and thread management functionality
//! for the VeridianOS microkernel, including:
//! - Process Control Block (PCB) management
//! - Thread creation and management
//! - Process lifecycle (creation, termination, state transitions)
//! - Global process table
//! - Memory space management
//! - Capability integration

// Process management is fully implemented but many functions are not yet
// called from user-space syscall paths. Will be exercised once the process
// lifecycle is driven by real user-space programs.

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

// Import println! macro - may be no-op on some architectures
#[allow(unused_imports)]
use crate::println;

// Re-export submodules
pub mod creation;
pub mod creds;
pub mod cwd;
pub mod exit;
pub mod fork;
pub mod itimer;
pub mod lifecycle;
pub mod linux_caps;
pub mod memory;
#[cfg(feature = "alloc")]
pub mod ns;
pub mod pcb;
pub mod personality;
pub mod pidns;
pub mod posix_timer;
pub mod rlimit;
pub mod robust_list;
pub mod session;
pub mod signals;
pub mod table;
pub mod thread;
pub mod userns;

// Re-export common types
pub use lifecycle::{exec_process, fork_process, wait_process as wait_for_child};
pub use pcb::{Process, ProcessId, ProcessPriority, ProcessState, TracerCred};
pub use table::get_process;
pub use thread::{Thread, ThreadId, ThreadState};

// Re-export thread context types for compatibility
pub use crate::arch::context::{ArchThreadContext, ThreadContext};

/// Maximum number of concurrent processes (including zombies awaiting reaping).
///
/// This limit is enforced in fork() to prevent unbounded process table growth
/// during workloads like BusyBox native compilation (213+ sequential gcc
/// invocations). Zombie processes count against this limit until reaped by
/// their parent via waitpid().
pub const MAX_PROCESSES: usize = 1024;

/// Process and thread IDs share one space, as on Linux: a process's first
/// thread has the process's ID as its thread ID (`ThreadBuilder::tid`), and
/// every other thread an ID no process has, so gettid, tkill and tgkill name
/// threads and processes without ambiguity (N-217).
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// What /proc/sys/kernel/pid_max reports: Linux's limit on 64-bit systems
/// (`PID_MAX_LIMIT`). Linux allocates ids below it and wraps around to
/// reuse freed ones; `NEXT_ID` only counts up and never reuses an id, so the
/// file is a bound programs size tables by, not where ids stop
/// (docs/KNOWN-LIMITATIONS.md).
pub const PID_MAX: u64 = 4_194_304;

/// Allocate a new process ID
pub fn alloc_pid() -> ProcessId {
    ProcessId(alloc_id())
}

/// Allocate a new thread ID for a thread other than a process's first
/// (from the same space as process IDs).
pub fn alloc_tid() -> ThreadId {
    ThreadId(alloc_id())
}

/// The next ID, marked pending until its process or thread is visible
/// ([`published`]), so that clone3's set_tid cannot claim it meanwhile. An
/// ID set_tid has claimed, or given to a process or thread still alive, is
/// skipped: a claim does not move the counter (a claim of an ID near
/// `PID_MAX` would else use up every ID above it at once).
fn alloc_id() -> u64 {
    #[cfg(feature = "alloc")]
    loop {
        // CLAIMED_IDS before PENDING_IDS, as in `IdClaim::new`.
        let mut claimed = CLAIMED_IDS.lock();
        let mut pending = PENDING_IDS.lock();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        // The counter never comes back to `id`: whatever happens to it, it
        // is checked out of turn now or never.
        let maybe_used = claimed.ahead.remove(&id);
        if claimed.held.contains(&id) {
            continue;
        }
        pending.insert(id);
        drop(pending);
        drop(claimed);
        // Not under PENDING_IDS: dropping a process takes it.
        if maybe_used && id_in_use(id) {
            published(id);
            continue;
        }
        return id;
    }
    #[cfg(not(feature = "alloc"))]
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// `id`'s process is in the table, or its thread in its (listed) process,
/// so [`id_in_use`] sees it from now on; or the process or thread of a
/// fork or clone that failed is dropped. Either way it is not pending.
#[cfg(feature = "alloc")]
pub(crate) fn published(id: u64) {
    PENDING_IDS.lock().remove(&id);
}

/// IDs allocated whose process or thread is not yet visible: a claim
/// refuses them, so an allocation and a claim never hand out one ID twice
/// (PR #35 review). A claim looks here before it looks at the table, and a
/// publication puts the process or thread there before it leaves here.
/// Its own lock, which nothing holds while a process is dropped (whose
/// drop takes it).
#[cfg(feature = "alloc")]
static PENDING_IDS: spin::Mutex<alloc::collections::BTreeSet<u64>> =
    spin::Mutex::new(alloc::collections::BTreeSet::new());

/// The IDs clone3's set_tid has claimed (`IdClaims`). Lock order: this
/// before both PENDING_IDS and what [`id_in_use`] takes (the process table,
/// the reap list, the pidfd table, a process's threads), and those before
/// PENDING_IDS, a leaf: it is taken under the process table and a
/// process's threads (`published`, `IdReservation::drop`), and nothing is
/// taken while it is held. Never the other way (`IdClaim::new`,
/// `IdClaim::drop` and `alloc_id` keep it; PR #38 review).
#[cfg(feature = "alloc")]
static CLAIMED_IDS: spin::Mutex<IdClaims> = spin::Mutex::new(IdClaims {
    held: alloc::vec::Vec::new(),
    ahead: alloc::collections::BTreeSet::new(),
});

/// What clone3's set_tid has claimed.
#[cfg(feature = "alloc")]
struct IdClaims {
    /// Claimed for a process or thread not yet in the process table.
    held: alloc::vec::Vec<u64>,
    /// Claimed at or ahead of the counter, and not yet reached by it: the
    /// IDs [`alloc_id`] must check are in use before it hands them out,
    /// each once, as the counter passes it. Only these: every other ID
    /// ahead of the counter has never been used. (A highest-claimed bound
    /// made every allocation below one high claim scan the process table,
    /// for as long as the counter took to reach it; PR #37 review.)
    ahead: alloc::collections::BTreeSet<u64>,
}

/// An ID clone3's set_tid chose, held until its process or thread is in
/// the table, or the clone has failed.
#[cfg(feature = "alloc")]
pub struct IdClaim(u64);

#[cfg(feature = "alloc")]
impl IdClaim {
    /// Claim `id` unless a process, a zombie awaiting its reap, a thread, an
    /// open pidfd, another clone, or a fork or clone still under way has it
    /// (None: Linux's EEXIST). The counter stays where it is, as Linux's
    /// cyclic allocator keeps its cursor: [`alloc_id`] skips the ID while it
    /// is claimed or in use (`IdClaims::ahead`).
    pub fn new(id: u64) -> Option<Self> {
        let mut claimed = CLAIMED_IDS.lock();
        let pending = PENDING_IDS.lock().contains(&id);
        if claimed.held.contains(&id) || pending || id_in_use(id) {
            return None;
        }
        claimed.held.push(id);
        // The counter moves only under this lock (`alloc_id`).
        if id >= NEXT_ID.load(Ordering::Relaxed) {
            claimed.ahead.insert(id);
        }
        Some(Self(id))
    }

    pub fn id(&self) -> u64 {
        self.0
    }
}

#[cfg(feature = "alloc")]
impl Drop for IdClaim {
    fn drop(&mut self) {
        let mut claimed = CLAIMED_IDS.lock();
        claimed.held.retain(|&id| id != self.0);
        // A clone that failed gave the ID to nothing: the allocator need
        // not check it out of turn (PR #38 review). One it was given to is
        // in use now, and stays listed until the counter passes it. Only an
        // ID still listed is checked, so a claim the counter has passed
        // costs no scan of the tables (PR #38 review).
        if claimed.ahead.contains(&self.0) && !id_in_use(self.0) {
            claimed.ahead.remove(&self.0);
        }
    }
}

/// Whether a process (in the table or awaiting its reap) or a thread has
/// the ID `id`, or it still names a live process group or session whose
/// leader is gone (Linux keeps that struct pid while the group lives; PR
/// #37 review).
#[cfg(feature = "alloc")]
fn id_in_use(id: u64) -> bool {
    if process_for_reap(id).is_some() || crate::fs::pidfd::holds(id) {
        return true;
    }
    let mut used = false;
    table::PROCESS_TABLE.for_each(|p| {
        if !used {
            used = p.pgid.load(Ordering::Acquire) == id
                || p.sid.load(Ordering::Acquire) == id
                || p.get_thread(ThreadId(id)).is_some();
        }
    });
    used
}

/// Initialize process management subsystem without creating init process
///
/// This is used during bootstrap to initialize process structures
/// without creating the init process (which requires scheduler).
pub fn init_without_init_process() -> crate::error::KernelResult<()> {
    println!("[PROCESS] Initializing process management structures...");

    // Initialize process table
    table::init();

    println!("[PROCESS] Process management structures initialized");
    Ok(())
}

/// Initialize process management subsystem (legacy)
///
/// This creates the init process, so scheduler must be initialized first.
pub fn init() {
    println!("[PROCESS] Initializing process management...");

    // Initialize process table
    table::init();

    // Create init process (PID 1)
    #[cfg(feature = "alloc")]
    {
        use alloc::string::String;
        match lifecycle::create_process(String::from("init"), 0) {
            Ok(_pid) => {
                println!("[PROCESS] Created init process with PID {}", _pid.0);
            }
            Err(_e) => {
                // Log the error but do not panic. The bootstrap sequence
                // creates its own init process as a fallback, so this path
                // is recoverable. The legacy init() path is rarely used.
                println!("[PROCESS] WARNING: Failed to create init process: {}", _e);
            }
        }
    }

    println!("[PROCESS] Process management initialized");
}

/// The (pid, tid) of the user thread the running dispatcher task belongs
/// to (ADR 0006 stage D2). Lock-free.
fn dispatched_context() -> Option<(u64, u64)> {
    #[cfg(feature = "alloc")]
    {
        crate::sched::dispatch::current_owner()
    }
    #[cfg(not(feature = "alloc"))]
    {
        None
    }
}

/// Get current process: the process of the user thread running as the
/// current dispatcher task. `None` for kernel threads and the boot task (the
/// old scheduler's current task and the boot-launched registration went in
/// sprint D4).
pub fn current_process() -> Option<alloc::sync::Arc<Process>> {
    let (pid, _) = dispatched_context()?;
    table::get_process(ProcessId(pid))
}

/// Like [`current_process`]; takes no scheduler lock, so fault handlers may
/// use it whatever the interrupted code holds.
pub fn try_current_process() -> Option<alloc::sync::Arc<Process>> {
    current_process()
}

/// Find process by ID
pub fn find_process(pid: ProcessId) -> Option<alloc::sync::Arc<Process>> {
    table::get_process(pid)
}

/// Get current process (alias for compatibility)
pub fn get_current_process() -> Option<alloc::sync::Arc<Process>> {
    current_process()
}

/// Get current thread (same rule as [`current_process`], so the two never
/// name different processes).
pub fn current_thread() -> Option<alloc::sync::Arc<Thread>> {
    let (pid, tid) = dispatched_context()?;
    table::get_process(ProcessId(pid))?.get_thread(ThreadId(tid))
}

/// Start `thread` of `process` as a dispatcher task (ADR 0006 stage D2):
/// it enters ring 3 with the registers in its saved context. `inherit_fpu`
/// gives it a copy of the calling thread's live vector registers (fork,
/// clone) instead of the initial state (a new program).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub fn start_thread(
    process: &Process,
    thread: &Thread,
    inherit_fpu: bool,
) -> crate::error::KernelResult<()> {
    use crate::arch::x86_64::switch::{xsave::Area, UserFrame};

    let (frame, ctx_tls) = {
        let ctx = thread.context.lock();
        let frame =
            UserFrame::from_context(&ctx).ok_or(crate::error::KernelError::InvalidArgument {
                name: "thread context",
                value: "entry point or stack outside user space",
            })?;
        (frame, ctx.tls_base)
    };
    let cr3 = process.memory_space.lock().get_page_table();
    if cr3 == 0 {
        return Err(crate::error::KernelError::InvalidState {
            expected: "address space",
            actual: "no page table",
        });
    }
    let fs_base = if ctx_tls != 0 {
        ctx_tls
    } else {
        process.tls_fs_base.load(Ordering::Acquire)
    };
    // Loaded into IA32_FS_BASE at every switch: it must be a user address
    // (a non-canonical value faults in ring 0).
    if !crate::arch::x86_64::trap::is_user_address(fs_base) {
        return Err(crate::error::KernelError::InvalidArgument {
            name: "thread TLS base",
            value: "outside user space",
        });
    }
    let mut area = Area::new().ok_or(crate::error::KernelError::OutOfMemory {
        requested: crate::arch::x86_64::switch::xsave::size(),
        available: 0,
    })?;
    // A new thread or a fork child inherits the caller's GS base (Linux
    // copies it with the thread); a fresh program starts with 0.
    let mut gs_base = 0;
    if inherit_fpu {
        // The kernel is soft-float, so the CPU still holds the calling
        // thread's user vector state.
        area.save();
        gs_base = crate::arch::x86_64::switch::read_user_gs_base();
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    let policy = thread.sched.lock().to_policy();
    crate::sched::dispatch::spawn_user(
        (process.pid.0, thread.tid.0),
        &frame,
        cr3,
        fs_base,
        area,
        gs_base,
        policy,
        thread.get_affinity() as u64,
    )?;
    Ok(())
}

/// RISC-V: start `thread` as a dispatcher task entering user mode with the
/// registers of its saved context (pc, sp, gp, tp, the argument registers).
/// `inherit_fpu` gives it a copy of the calling thread's live FP registers
/// (fork, clone) instead of the initial state (a new program).
#[cfg(all(feature = "alloc", target_arch = "riscv64", target_os = "none"))]
pub fn start_thread(
    process: &Process,
    thread: &Thread,
    inherit_fpu: bool,
) -> crate::error::KernelResult<()> {
    use crate::arch::riscv64::switch::UserFrame;
    let mut fp = crate::arch::riscv64::fpu::initial();
    if inherit_fpu {
        // The kernel is soft-float: the FP registers still hold the calling
        // thread's user state.
        crate::arch::riscv64::fpu::save(&mut fp);
    }
    let frame = {
        let ctx = thread.context.lock();
        let user = crate::syscall::userspace::is_user_address;
        if !user(ctx.pc) || !user(ctx.sp) {
            return Err(crate::error::KernelError::InvalidArgument {
                name: "thread context",
                value: "entry point or stack outside user space",
            });
        }
        let mut f = UserFrame::default();
        // The thread pointer: the context's (a clone's CLONE_SETTLS, a
        // fork's), else the static program's TLS block (`setup_static_tls`).
        let tp = if ctx.tp != 0 {
            ctx.tp
        } else {
            process.tls_fs_base.load(Ordering::Acquire) as usize
        };
        let regs = [
            ctx.ra, ctx.sp, ctx.gp, tp, ctx.t0, ctx.t1, ctx.t2, ctx.s0, ctx.s1, ctx.a0, ctx.a1,
            ctx.a2, ctx.a3, ctx.a4, ctx.a5, ctx.a6, ctx.a7, ctx.s2, ctx.s3, ctx.s4, ctx.s5, ctx.s6,
            ctx.s7, ctx.s8, ctx.s9, ctx.s10, ctx.s11, ctx.t3, ctx.t4, ctx.t5, ctx.t6,
        ];
        for (i, r) in regs.iter().enumerate() {
            f.x[i + 1] = *r as u64;
        }
        f.sepc = ctx.pc as u64;
        f
    };
    let root = process.memory_space.lock().get_page_table();
    if root == 0 {
        return Err(crate::error::KernelError::InvalidState {
            expected: "address space",
            actual: "no page table",
        });
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    let policy = thread.sched.lock().to_policy();
    crate::sched::dispatch::spawn_user(
        (process.pid.0, thread.tid.0),
        &frame,
        root,
        fp,
        policy,
        thread.get_affinity() as u64,
    )?;
    Ok(())
}

/// Overwrite `ctx` with the caller's live user registers, the ones the
/// system call in progress returns with (fork and clone children start
/// from them). Without a live frame (not in a system call) `ctx` is kept.
#[cfg(all(
    target_os = "none",
    any(target_arch = "aarch64", target_arch = "riscv64")
))]
pub(crate) fn load_live_user_registers(ctx: &mut crate::arch::context::ArchThreadContext) {
    #[cfg(target_arch = "aarch64")]
    if let Some(frame) = crate::arch::aarch64::switch::live_user_frame() {
        // The kernel never writes TPIDR_EL0, so it still holds the
        // caller's thread pointer (musl sets it from user mode).
        ctx.load_user_frame(&frame, crate::arch::aarch64::switch::read_tls());
    }
    #[cfg(target_arch = "riscv64")]
    if let Some(frame) = crate::arch::riscv64::switch::live_user_frame() {
        ctx.load_user_frame(&frame);
    }
}

/// AArch64: start `thread` as a dispatcher task entering EL0 with the
/// registers of its saved context (x0-x30, sp, pc) and its TLS register
/// (TPIDR_EL0). `inherit_fpu` gives it a copy of the calling thread's live
/// FP/SIMD registers (fork, clone) instead of the initial state.
#[cfg(all(feature = "alloc", target_arch = "aarch64", target_os = "none"))]
pub fn start_thread(
    process: &Process,
    thread: &Thread,
    inherit_fpu: bool,
) -> crate::error::KernelResult<()> {
    use crate::arch::aarch64::switch::UserFrame;
    let mut fp = crate::arch::aarch64::context::initial_fpu();
    if inherit_fpu {
        // The kernel is soft-float: the FP registers still hold the calling
        // thread's user state.
        crate::arch::aarch64::context::save_fpu_state(&mut fp);
    }
    let (frame, tls) = {
        let ctx = thread.context.lock();
        let user = crate::syscall::userspace::is_user_address;
        if !user(ctx.pc as usize) || !user(ctx.sp as usize) {
            return Err(crate::error::KernelError::InvalidArgument {
                name: "thread context",
                value: "entry point or stack outside user space",
            });
        }
        let f = UserFrame {
            x: ctx.x,
            sp: ctx.sp,
            elr: ctx.pc,
            spsr: 0,
        };
        // The thread pointer: the context's (a clone's CLONE_SETTLS, a
        // fork's), else the static program's TLS block (`setup_static_tls`).
        let tls = if ctx.tpidr_el0 != 0 {
            ctx.tpidr_el0
        } else {
            process.tls_fs_base.load(Ordering::Acquire)
        };
        (f, tls)
    };
    let root = process.memory_space.lock().get_page_table();
    if root == 0 {
        return Err(crate::error::KernelError::InvalidState {
            expected: "address space",
            actual: "no page table",
        });
    }
    process
        .dispatched
        .store(true, core::sync::atomic::Ordering::Release);
    let policy = thread.sched.lock().to_policy();
    crate::sched::dispatch::spawn_user(
        (process.pid.0, thread.tid.0),
        &frame,
        root,
        tls,
        fp,
        policy,
        thread.get_affinity() as u64,
    )?;
    Ok(())
}

/// Run a freshly created (or exec'd) process as dispatcher tasks and wait
/// for it to end (stage D2): its first thread is started, the caller blocks
/// until the process is a zombie whose last task is off the CPU, and the
/// zombie is reaped. Returns the exit code, or 128 + the signal that killed
/// it (the shell convention). `None` if the process could not be started.
#[cfg(all(
    feature = "alloc",
    any(
        target_arch = "x86_64",
        all(
            any(target_arch = "riscv64", target_arch = "aarch64"),
            target_os = "none"
        )
    )
))]
pub fn run_and_wait(pid: ProcessId) -> Option<i32> {
    run_and_wait_all(&[pid])[0]
}

/// [`run_and_wait`] for several processes at once: all are started before
/// any is waited for, so they run concurrently. One status per pid, in
/// order (`None` for one that could not be started).
#[cfg(all(
    feature = "alloc",
    any(
        target_arch = "x86_64",
        all(
            any(target_arch = "riscv64", target_arch = "aarch64"),
            target_os = "none"
        )
    )
))]
pub fn run_and_wait_all(pids: &[ProcessId]) -> alloc::vec::Vec<Option<i32>> {
    let started: alloc::vec::Vec<Option<alloc::sync::Arc<Process>>> = pids
        .iter()
        .map(|&pid| {
            let process = table::get_process(pid)?;
            let thread = process.threads.lock().values().next().cloned()?;
            if start_thread(&process, &thread, false).is_err() {
                // Never ran: free its address space, tables and stacks, as
                // the shell's launch path did (PR #19 review).
                exit::discard_unstarted(&process);
                return None;
            }
            Some(process)
        })
        .collect();
    pids.iter()
        .zip(started)
        .map(|(&pid, process)| {
            let process = process?;
            crate::sched::dispatch::PROCESS_EVENTS.wait_until(|| {
                process.get_state() == ProcessState::Zombie
                    && crate::sched::dispatch::tasks_of(pid.0).is_empty()
            });
            let sig = process.term_signal.load(Ordering::Acquire);
            let code = if sig != 0 {
                128 + sig as i32
            } else {
                process.get_exit_code()
            };
            drop(process);
            table::remove_process(pid);
            Some(code)
        })
        .collect()
}

/// The process of a dispatcher task about to be reaped, looked up before
/// the task leaves the task table: once it has, a waiting parent may remove
/// the process, and [`user_task_reaped`] must still free its page tables.
#[cfg(feature = "alloc")]
pub fn process_for_reap(pid: u64) -> Option<alloc::sync::Arc<Process>> {
    table::get_process(ProcessId(pid)).or_else(|| {
        AWAITING_REAP
            .lock()
            .iter()
            .find(|p| p.pid.0 == pid)
            .cloned()
    })
}

/// Zombies a parent has waited for while a task of theirs had still to be
/// reaped. A zombie can be waited for as soon as it is one, as on Linux --
/// its last thread marks it before leaving the CPU -- and the reap of that
/// task frees the page tables, which needs the process: out of the table,
/// it was not found and they leaked (PR #22 review). Each stays here until
/// its page tables are freed.
#[cfg(feature = "alloc")]
static AWAITING_REAP: spin::Mutex<alloc::vec::Vec<alloc::sync::Arc<Process>>> =
    spin::Mutex::new(alloc::vec::Vec::new());

/// Take the waited-for zombie `pid` out of the process table, keeping it
/// for its tasks' reap if they still need it.
#[cfg(feature = "alloc")]
pub(crate) fn remove_waited(pid: ProcessId) {
    let Some(process) = table::get_process(pid) else {
        return;
    };
    // Kept before it leaves the table, so a reap never finds it in
    // neither place; dropped again if its page tables are already freed
    // (a reap that came first).
    AWAITING_REAP.lock().push(process.clone());
    table::remove_process(pid);
    if process.memory_space.lock().get_page_table() == 0 {
        release_reaped(pid.0);
    }
    release_numbers(&process);
    // An init waiting for its namespace to empty (`exit::zap_pid_ns`) looks
    // again.
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
}

/// A reaped process's numbers in PID namespaces below the initial one are
/// free, and its threads' (Linux's release_task freeing their struct pid):
/// its own once no live process has it as its process group or session,
/// as a group's id stays its leader's while the group lives. Its group and
/// session ids are looked at again too, for a leader reaped before them.
#[cfg(feature = "alloc")]
fn release_numbers(process: &Process) {
    let tids: alloc::vec::Vec<u64> = process
        .threads
        .lock()
        .keys()
        .map(|t| t.0)
        .filter(|&t| t != process.pid.0)
        .collect();
    for tid in tids {
        pidns::release(tid);
    }
    let pgid = process.pgid.load(Ordering::Acquire);
    let sid = process.sid.load(Ordering::Acquire);
    for kid in [process.pid.0, pgid, sid] {
        release_if_unused(kid);
    }
}

/// Free `kid`'s PID namespace numbers if it no longer names a live
/// process, process group or session: after a reap, and after setpgid or
/// setsid leaves a group or session whose leader is gone.
#[cfg(feature = "alloc")]
pub(crate) fn release_if_unused(kid: u64) {
    if pidns::is_registered(kid) && !names_live(kid) {
        pidns::release(kid);
    }
}

/// Whether `kid` is a live process's id, process group or session.
#[cfg(feature = "alloc")]
fn names_live(kid: u64) -> bool {
    if table::get_process(ProcessId(kid)).is_some() {
        return true;
    }
    let mut used = false;
    table::PROCESS_TABLE.for_each(|p| {
        used |= p.pgid.load(Ordering::Acquire) == kid || p.sid.load(Ordering::Acquire) == kid
    });
    used
}

/// The page tables of `pid` are freed: nothing of it is waited for.
#[cfg(feature = "alloc")]
fn release_reaped(pid: u64) {
    AWAITING_REAP.lock().retain(|p| p.pid.0 != pid);
}

/// A dispatcher task of `process` has been reaped (it is off every CPU and
/// out of the task table). Once a zombie process has no task left, its page
/// tables -- which a running thread was still using when it tore the rest
/// down -- are freed. Runs in the reaper thread: it may sleep.
#[cfg(feature = "alloc")]
pub fn user_task_reaped(process: Option<alloc::sync::Arc<Process>>, tid: u64) {
    // A thread other than the main one that has exited while its process
    // lives on is gone for good, as on Linux: its entry (and the kernel
    // stack its builder allocated) is freed now. C libraries join through
    // the CLONE_CHILD_CLEARTID futex, not the kernel, so nothing else ever
    // removed it. Until then it is not live, so RLIMIT_NPROC and
    // threads-max do not count it (`rlimit::tasks_of`).
    if let Some(p) = &process {
        let exited = p.get_thread(ThreadId(tid)).is_some_and(|t| {
            matches!(
                t.get_state(),
                thread::ThreadState::Zombie | thread::ThreadState::Dead
            )
        });
        if tid != p.pid.0 && exited && p.get_state() != ProcessState::Zombie {
            let _ = exit::cleanup_thread(p, ThreadId(tid));
        }
    }
    // Every reap: an exec waits for its process's other tasks to go
    // (N-101), not only for the whole process.
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
    let Some(process) = process else {
        return;
    };
    if process.get_state() != ProcessState::Zombie
        || !crate::sched::dispatch::tasks_of(process.pid.0).is_empty()
    {
        return;
    }
    free_page_tables(&process);
    release_reaped(process.pid.0);
    crate::sched::dispatch::PROCESS_EVENTS.wake_all();
}

/// Free a process's page tables (its root and what is left below it). Only
/// once no CPU can be running in its address space.
#[cfg(feature = "alloc")]
pub(crate) fn free_page_tables(process: &Process) {
    let root = {
        let vas = process.memory_space.lock();
        let root = vas.get_page_table();
        vas.set_page_table(0);
        root
    };
    if root != 0 {
        crate::mm::vas::free_user_page_table_frames(root);
    }
}

/// Whether a dispatched thread waiting in a system call must stop waiting
/// and fail with EINTR: its process has a fatal signal to act on. The
/// signal is acted on at the system-call exit, once the call has unwound.
pub fn wait_interrupted() -> bool {
    if dispatched_context().is_none() {
        return false;
    }
    let (Some(process), Some(thread)) = (current_process(), current_thread()) else {
        return false;
    };
    match process.kill_pending.load(Ordering::Acquire) {
        0 => signals::interrupting(&process, &thread) != 0,
        // An exec in progress interrupts every thread but the exec'ing one.
        exit::EXEC_PENDING => process.exec_tid.load(Ordering::Acquire) != thread.tid.0,
        _ => true,
    }
}

/// Run on every return to user mode of a dispatched thread: a thread whose
/// process received a fatal signal exits here instead (the signal is acted
/// on by the process's own threads, never torn down under them).
pub extern "C" fn user_return_check() {
    #[cfg(feature = "alloc")]
    if dispatched_context().is_some() {
        if let Some(process) = current_process() {
            // RLIMIT_CPU: SIGXCPU or SIGKILL, acted on just below or by
            // the signal delivery that follows.
            rlimit::check_cpu(&process);
            // ITIMER_VIRTUAL and ITIMER_PROF: SIGVTALRM, SIGPROF.
            itimer::check_cpu(&process);
            // POSIX timers on CPU clocks.
            posix_timer::check_cpu(&process);
            let sig = process.kill_pending.load(Ordering::Acquire);
            if sig == exit::GROUP_EXIT_PENDING {
                drop(process);
                exit::exit_dispatched(0, true);
            } else if sig == exit::EXEC_PENDING {
                // Another thread is exec'ing: this one leaves.
                let me = current_thread().map(|t| t.tid.0);
                if me != Some(process.exec_tid.load(Ordering::Acquire)) {
                    drop(process);
                    exit::exit_dispatched(0, false);
                }
            } else if sig != 0 {
                drop(process);
                let _ = crate::syscall::process::exit_current(0, sig);
            }
        }
    }
}

/// Exit the calling thread (pthread_exit). Every user thread is a
/// dispatcher task, which leaves through `exit::exit_dispatched` (robust
/// futexes, CLONE_CHILD_CLEARTID, CPU time, and the process's teardown if it
/// is the last thread). Returns only when called outside a user thread.
pub fn exit_thread(exit_code: i32) {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        exit::exit_dispatched(exit_code, false);
    }
    let _ = exit_code;
}

/// Terminate a specific thread
pub fn terminate_thread(pid: ProcessId, tid: ThreadId) -> crate::error::KernelResult<()> {
    if let Some(process) = find_process(pid) {
        if let Some(thread) = process.get_thread(tid) {
            println!(
                "[PROCESS] Terminating thread {} in process {}",
                tid.0, pid.0
            );

            // Mark thread as dead
            thread.set_state(thread::ThreadState::Dead);
            Ok(())
        } else {
            Err(crate::error::KernelError::ThreadNotFound { tid: tid.0 })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: pid.0 })
    }
}

/// Create a new thread in the current process
///
/// Allocates real stack frames for the thread via the frame allocator using
/// [`ThreadBuilder`]. If `stack_ptr` is non-zero, it overrides the user stack
/// pointer. If `tls_ptr` is non-zero, it sets the TLS base address.
pub fn create_thread(
    entry_point: usize,
    stack_ptr: usize,
    arg: usize,
    tls_ptr: usize,
) -> crate::error::KernelResult<ThreadId> {
    if let Some(process) = current_process() {
        #[cfg(feature = "alloc")]
        {
            use alloc::string::String;

            use thread::ThreadBuilder;

            // RLIMIT_NPROC and kernel.threads-max, as clone checks them:
            // with no per-process limit, every path that makes a thread for a
            // caller must (commit security review). Checked before anything
            // is allocated, so a refusal leaks nothing (PR #43 review), and
            // held until the thread is in its process.
            let _nproc = crate::process::rlimit::check_nproc(&process)
                .map_err(|_| crate::error::KernelError::WouldBlock)?;

            // Build thread with real stack allocation via ThreadBuilder
            let sched = current_thread()
                .map(|t| t.sched.lock().for_child())
                .unwrap_or_default();
            let io = current_thread().map_or_else(crate::syscall::ioprio::IoContext::new, |t| {
                crate::syscall::ioprio::IoContext::for_child(&t.io, false)
            });
            let thread = ThreadBuilder::new(process.pid, String::from("user_thread"), entry_point)
                .sched(sched)
                .io(io)
                .user_stack_size(1024 * 1024) // 1MB user stack
                .kernel_stack_size(64 * 1024) // 64KB kernel stack
                .build()?;

            let tid = thread.tid;

            // Override the stack pointer if provided by caller
            if stack_ptr != 0 {
                thread.user_stack.set_sp(stack_ptr);
            }

            // Set up thread-local storage if provided
            if tls_ptr != 0 {
                thread.tls.lock().base = tls_ptr;
            }

            // Store argument in a register (architecture-specific)
            // For now, we'll skip this as it requires arch-specific code
            let _ = arg;

            // The creator's no_new_privs and capability sets, as clone copies
            // them (ADR 0020): a new thread has no more privilege than the
            // thread that made it. Thread::new starts from root's sets, for
            // the kernel's own threads (PR #33 review).
            let creator = current_thread();
            if let Some(creator) = &creator {
                thread.no_new_privs.store(
                    creator.no_new_privs.load(Ordering::Acquire),
                    Ordering::Release,
                );
                let sets = *creator.caps.lock();
                *thread.caps.lock() = sets;
            }

            // Add thread to process
            let thread = process.add_thread(thread)?;
            // Copied again under the thread map, which a set*id call on
            // another thread fixes every thread's sets under
            // (`Process::change_uids`), as clone does.
            if let Some(creator) = &creator {
                let _threads = process.threads.lock();
                let sets = *creator.caps.lock();
                *thread.caps.lock() = sets;
            }

            Ok(tid)
        }

        #[cfg(not(feature = "alloc"))]
        {
            let _ = (entry_point, stack_ptr, arg, tls_ptr);
            Err(crate::error::KernelError::NotImplemented {
                feature: "create_thread (requires alloc)",
            })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: 0 })
    }
}

/// Set thread CPU affinity
pub fn set_thread_affinity(tid: ThreadId, cpu_mask: u64) -> crate::error::KernelResult<()> {
    if let Some(process) = current_process() {
        if let Some(thread) = process.get_thread(tid) {
            thread
                .cpu_affinity
                .store(cpu_mask as usize, Ordering::SeqCst);
            Ok(())
        } else {
            Err(crate::error::KernelError::ThreadNotFound { tid: tid.0 })
        }
    } else {
        Err(crate::error::KernelError::ProcessNotFound { pid: 0 })
    }
}

/// Get current thread ID
pub fn get_thread_tid() -> ThreadId {
    if let Some(thread) = current_thread() {
        thread.tid
    } else {
        // Fallback to main thread ID
        ThreadId(0)
    }
}

/// Get a list of all process IDs
pub fn get_process_list() -> Option<alloc::vec::Vec<u64>> {
    #[cfg(feature = "alloc")]
    {
        use table::PROCESS_TABLE;
        let mut pids = alloc::vec::Vec::new();

        // Iterate through all processes
        PROCESS_TABLE.for_each(|process| {
            pids.push(process.pid.0);
        });

        if pids.is_empty() {
            None
        } else {
            Some(pids)
        }
    }
    #[cfg(not(feature = "alloc"))]
    None
}

// get_process is already re-exported at the top of the module

#[cfg(all(test, feature = "alloc"))]
mod id_tests {
    use super::*;

    /// An ID a fork or clone has allocated but not yet made visible cannot
    /// be claimed by clone3's set_tid: two processes or threads never get
    /// one ID (PR #35 review). Once it is published, or its process or
    /// thread dropped, only the table decides.
    #[test]
    fn allocated_ids_are_not_claimable_until_published() {
        let id = alloc_tid().0;
        assert!(IdClaim::new(id).is_none(), "pending: refused");
        published(id);
        // Not in any table (this test made nothing), so free to claim.
        let claim = IdClaim::new(id).expect("published and unused");
        assert!(IdClaim::new(id).is_none(), "claimed once");
        drop(claim);
        assert!(IdClaim::new(id).is_some());
    }

    /// A claim does not move the counter (one of PID_MAX - 1 used to push
    /// every later fork past pid_max), and the allocator skips an ID while
    /// a claim holds it (PR #35 review).
    #[test]
    fn a_claim_does_not_move_the_counter() {
        let high = IdClaim::new(PID_MAX - 1).expect("unused");
        assert!(NEXT_ID.load(Ordering::Relaxed) < PID_MAX - 1);
        drop(high);
        // Claim an ID about to be handed out (other tests allocate too, so
        // the first one still free ahead); the next ones skip it.
        let base = NEXT_ID.load(Ordering::Relaxed);
        let (next, claim) = (base + 1..base + 1_000)
            .find_map(|id| IdClaim::new(id).map(|c| (id, c)))
            .expect("an ID ahead of the counter is free");
        while NEXT_ID.load(Ordering::Relaxed) <= next {
            let id = alloc_tid().0;
            assert_ne!(id, next, "a claimed ID is not handed out");
            published(id);
        }
        drop(claim);
    }

    /// Only the IDs claimed ahead of the counter are checked out of turn,
    /// each until the counter passes it: a claim of an ID near `PID_MAX`
    /// no longer makes every allocation below it scan the process table
    /// (PR #37 review), and a claim behind the counter adds nothing.
    #[test]
    fn only_ids_claimed_ahead_are_checked_until_passed() {
        // Held throughout: a claim dropped unused is no longer listed.
        let high = IdClaim::new(PID_MAX - 3).expect("unused");
        let behind = alloc_tid().0;
        published(behind);
        drop(IdClaim::new(behind).expect("published and unused"));
        // The counter moves only under CLAIMED_IDS, so under it an ID is
        // listed exactly while the counter has not reached it. Other tests
        // allocate meanwhile and may pass the ID first; then try again.
        let mut seen_ahead = None;
        for _ in 0..5 {
            let base = NEXT_ID.load(Ordering::Relaxed);
            let (near, claim) = (base + 20..base + 1_000)
                .find_map(|id| IdClaim::new(id).map(|c| (id, c)))
                .expect("an ID ahead of the counter is free");
            let ahead = {
                let claimed = CLAIMED_IDS.lock();
                assert!(claimed.ahead.contains(&(PID_MAX - 3)));
                assert!(!claimed.ahead.contains(&behind), "behind the counter");
                let ahead = NEXT_ID.load(Ordering::Relaxed) <= near;
                assert_eq!(claimed.ahead.contains(&near), ahead);
                ahead
            };
            if ahead {
                seen_ahead = Some((near, claim));
                break;
            }
            drop(claim);
        }
        let (near, claim) = seen_ahead.expect("the counter passed the claim every time");
        while NEXT_ID.load(Ordering::Relaxed) <= near {
            published(alloc_tid().0);
        }
        {
            let claimed = CLAIMED_IDS.lock();
            assert!(!claimed.ahead.contains(&near), "passed: never again");
            assert!(claimed.ahead.contains(&(PID_MAX - 3)), "not reached yet");
        }
        drop(claim);
        drop(high);
    }

    /// A claim dropped with its ID given to nothing (a clone3 that failed
    /// after set_tid's claim) is not left for the allocator to check out
    /// of turn (PR #38 review).
    #[test]
    fn a_failed_clones_claim_is_not_left_ahead() {
        let high = IdClaim::new(PID_MAX - 5).expect("unused");
        assert!(CLAIMED_IDS.lock().ahead.contains(&(PID_MAX - 5)));
        drop(high);
        assert!(!CLAIMED_IDS.lock().ahead.contains(&(PID_MAX - 5)));
    }

    /// A process whose ID set_tid chose, dropped only after a later
    /// allocation has handed that ID out (an `Arc` kept past its reap),
    /// leaves the new allocation's reservation alone: only the object an ID
    /// was allocated for clears it (PR #37 review).
    #[test]
    fn a_stale_process_leaves_a_new_reservation_alone() {
        for _ in 0..5 {
            let base = NEXT_ID.load(Ordering::Relaxed);
            let (id, claim) = (base + 20..base + 1_000)
                .find_map(|id| IdClaim::new(id).map(|c| (id, c)))
                .expect("an ID ahead of the counter is free");
            let stale = pcb::ProcessBuilder::new(alloc::string::String::from("stale"))
                .pid(Some(ProcessId(id)))
                .build();
            drop(claim);
            // Counted up to it; other tests allocate meanwhile, and may
            // take it first, then try again.
            let mut mine = false;
            while NEXT_ID.load(Ordering::Relaxed) <= id {
                let got = alloc_tid().0;
                if got == id {
                    mine = true;
                    break;
                }
                published(got);
            }
            if !mine {
                drop(stale);
                continue;
            }
            drop(stale);
            assert!(
                IdClaim::new(id).is_none(),
                "still pending for its allocation"
            );
            published(id);
            return;
        }
        panic!("the ID was taken by another test every time");
    }

    /// An ID that still names a live process group or session, whose leader
    /// is gone, is in use: neither a set_tid claim nor the allocator may
    /// hand it out, as Linux keeps the group's struct pid (PR #37 review).
    #[test]
    fn a_live_group_or_session_id_is_in_use() {
        for field in ["pgid", "sid"] {
            let base = NEXT_ID.load(Ordering::Relaxed);
            let (id, claim) = (base + 20..base + 1_000)
                .find_map(|id| IdClaim::new(id).map(|c| (id, c)))
                .expect("an ID ahead of the counter is free");
            drop(claim);
            let member = pcb::ProcessBuilder::new(alloc::string::String::from("member")).build();
            let target = if field == "pgid" {
                &member.pgid
            } else {
                &member.sid
            };
            target.store(id, Ordering::Release);
            let pid = table::add_process(member).expect("added");
            let refused = IdClaim::new(id).is_none();
            let in_use = id_in_use(id);
            table::remove_process(pid);
            assert!(refused, "{field} {id} is claimable while it names a group");
            assert!(in_use, "{field} {id} is not in use for the allocator");
        }
    }

    /// A fork child discarded before it ran gives up the group and session
    /// numbers it held the last use of, as a reap does (`release_numbers`):
    /// its parent may have left that group meanwhile, and the group's id
    /// then names nothing (PR #37 review).
    #[test]
    fn a_discarded_child_releases_a_group_it_was_the_last_in() {
        let ns = pidns::PidNs::create(&pidns::initial(), &userns::initial(), 0).unwrap();
        // Kernel IDs no other test registers (the registry is global,
        // shared by tests running in parallel).
        let (group, session) = (9_500_001, 9_500_002);
        for kid in [group, session] {
            pidns::reserve(&ns, &[], |_| true).unwrap().commit(kid);
        }
        let child = pcb::ProcessBuilder::new(alloc::string::String::from("unstarted")).build();
        child.pgid.store(group, Ordering::Release);
        child.sid.store(session, Ordering::Release);
        let pid = table::add_process(child).expect("added");
        let child = table::get_process(pid).expect("in the table");
        // discard_unstarted without the page-table and memory teardown,
        // which needs real page tables.
        exit::forget_unstarted(&child);
        assert!(table::get_process(pid).is_none(), "out of the table");
        assert!(!pidns::is_registered(group), "group number released");
        assert!(!pidns::is_registered(session), "session number released");
    }

    /// A process dropped before it was published (a fork that failed)
    /// still gives its allocated ID's reservation up.
    #[test]
    fn an_unpublished_process_gives_its_reservation_up() {
        let p = pcb::ProcessBuilder::new(alloc::string::String::from("failed")).build();
        let id = p.pid.0;
        assert!(IdClaim::new(id).is_none(), "pending while it lives");
        drop(p);
        assert!(IdClaim::new(id).is_some(), "free once it is dropped");
    }
}

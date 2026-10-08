//! Process Control Block (PCB) implementation
//!
//! The PCB is the core data structure representing a process in the kernel.
//! It contains all the information needed to manage a process.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use spin::Mutex;

use super::thread::{Thread, ThreadId};
#[allow(unused_imports)]
use crate::{
    cap::{CapabilityId, CapabilitySpace},
    error::KernelError,
    fs::file::FileTable,
    ipc::EndpointId,
    mm::VirtualAddressSpace,
    println,
};

/// Process ID type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessId(pub u64);

impl core::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Process state
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Process is being created
    Creating = 0,
    /// Process is ready to run
    Ready = 1,
    /// Process is currently running
    Running = 2,
    /// Process is blocked waiting
    Blocked = 3,
    /// Process is sleeping
    Sleeping = 4,
    /// Process has exited but not yet reaped
    Zombie = 5,
    /// Process has been terminated
    Dead = 6,
}

/// Process priority
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProcessPriority {
    /// Real-time priority (highest)
    RealTime = 0,
    /// System priority
    System = 1,
    /// Normal user priority
    Normal = 2,
    /// Low priority
    Low = 3,
    /// Idle priority (lowest)
    Idle = 4,
}

/// Process Control Block
pub struct Process {
    /// Process ID
    pub pid: ProcessId,

    /// Parent process ID (None for init)
    /// Parent process (reparented to init when the parent exits). Behind a
    /// lock because processes are shared through `Arc` (PROC-SEC-01).
    parent: Mutex<Option<ProcessId>>,

    /// Process name
    #[cfg(feature = "alloc")]
    pub name: String,

    /// Process state
    pub state: AtomicU32,

    /// Priority
    pub priority: Mutex<ProcessPriority>,

    /// Virtual address space
    /// The address space. A sleeping lock: a page fault that finds it held
    /// waits instead of failing (N-138).
    pub memory_space: crate::sync::sleep_mutex::SleepMutex<VirtualAddressSpace>,

    /// Capability space
    pub capability_space: Mutex<CapabilitySpace>,

    /// File descriptor table
    pub file_table: Mutex<FileTable>,

    /// Threads in this process
    #[cfg(feature = "alloc")]
    /// Threads, shared through `Arc`: the map moves its values when nodes
    /// split or merge, so references into it were never stable.
    pub threads: Mutex<BTreeMap<ThreadId, Arc<Thread>>>,

    /// IPC endpoints owned by this process
    #[cfg(feature = "alloc")]
    pub ipc_endpoints: Mutex<BTreeMap<EndpointId, CapabilityId>>,

    /// Child processes
    #[cfg(feature = "alloc")]
    pub children: Mutex<Vec<ProcessId>>,

    /// Exit code (set when process exits)
    pub exit_code: AtomicU32,

    /// Signal that terminated the process (0: it exited normally).
    pub term_signal: AtomicU32,

    /// Its threads run as dispatcher tasks (ADR 0006 stage D2) rather than
    /// nested inside the context that launched them.
    pub dispatched: core::sync::atomic::AtomicBool,

    /// A fatal signal waiting to be acted on by the process's own threads
    /// (dispatched processes only): each thread exits at its next return
    /// to user mode or wakeup, and the last one tears the process down.
    pub kill_pending: AtomicU32,

    /// The signal the parent gets when this process exits: SIGCHLD, or
    /// clone's low byte (0: none) (N-210).
    pub exit_signal: AtomicU32,

    /// A CLONE_VFORK child whose parent waits until it execs or exits;
    /// cleared (and PROCESS_EVENTS woken) by either (N-210).
    pub vfork_pending: core::sync::atomic::AtomicBool,

    /// The process has exec'd since it was created: its parent may no
    /// longer change its process group (setpgid EACCES, N-216).
    pub did_exec: core::sync::atomic::AtomicBool,

    /// Job control (D3): the signal that stopped the process, 0 while it
    /// runs. Its threads park on their way back to user mode until SIGCONT
    /// or SIGKILL.
    pub stop_signal: AtomicU32,

    /// Counts SIGCONTs, so a stop decided before a SIGCONT arrived is not
    /// applied after it.
    pub cont_seq: AtomicU32,

    /// A stop or continue the parent has not collected with `wait` yet, as
    /// a wait status (`0x7f | sig << 8` or `0xffff`); 0 for none.
    pub job_report: AtomicU32,

    /// CPU time used (in microseconds)
    pub cpu_time: AtomicU64,

    /// Memory usage statistics
    pub memory_stats: MemoryStats,

    /// Creation timestamp
    pub created_at: u64,

    /// User and group IDs and supplementary groups (N-248). A lock
    /// because processes are shared through `Arc` and a set*id call
    /// updates several fields at once.
    creds: Mutex<super::creds::Credentials>,

    /// Process group ID (initialized to pid)
    pub pgid: AtomicU64,

    /// The pid of the process tracing this one with ptrace, or 0. Not
    /// inherited on fork, as on Linux.
    pub tracer: AtomicU64,

    /// Linux's cred_guard_mutex: exec decides and installs set-ID
    /// credentials under it, and ptrace attaches under it, so a tracer
    /// either attaches first (and exec grants no privilege) or sees the new
    /// credentials (and may not attach). Held only briefly, never across
    /// anything that waits.
    pub cred_guard: Mutex<()>,

    /// Linux's mm dumpable flag (PR_SET_DUMPABLE): when clear, only root
    /// may ptrace the process or read its robust list. Inherited on fork,
    /// set by exec as `install_exec_credentials` decides.
    pub dumpable: core::sync::atomic::AtomicBool,

    /// CPU time of the process's exited threads (its live threads' is the
    /// dispatcher's), and of its children it has waited for, with theirs
    /// (Linux's signal_struct sums; N-218, N-223).
    cpu_exited: Mutex<crate::sched::cputime::CpuTimes>,
    cpu_children: Mutex<crate::sched::cputime::CpuTimes>,
    /// The largest resident set seen (pages), and the largest of the
    /// children waited for (getrusage ru_maxrss).
    peak_rss_pages: core::sync::atomic::AtomicUsize,
    children_peak_rss_pages: core::sync::atomic::AtomicUsize,

    /// Session ID (initialized to pid)
    pub sid: AtomicU64,

    /// Environment variables (populated during exec, inherited on fork)
    #[cfg(feature = "alloc")]
    pub env_vars: Mutex<alloc::collections::BTreeMap<String, String>>,

    /// Absolute path of the running executable (target of /proc/self/exe).
    /// Set by the loader and by a successful exec, inherited on fork.
    #[cfg(feature = "alloc")]
    pub exe_path: Mutex<String>,

    /// Signal handlers (signal number -> handler action)
    /// 0 = default, 1 = ignore, other values = handler address
    pub signal_handlers: Mutex<[u64; super::signals::NSIG + 1]>,

    /// The rest of each signal's action: (sa_flags, sa_restorer, sa_mask),
    /// kept so sigaction returns what was set (N-95) and for delivery.
    pub signal_action_extra: Mutex<[(u64, u64, u64); super::signals::NSIG + 1]>,

    /// Pending signals bitmap
    pub pending_signals: AtomicU64,

    /// Queued real-time signals behind `pending_signals` (N-209).
    pub rt_queue: super::signals::RtQueue,

    /// Signal mask (blocked signals)
    pub signal_mask: AtomicU64,

    /// File creation mask (umask). Default 0o022.
    pub umask: AtomicU32,

    /// TLS FS_BASE address for x86_64 (Thread-Local Storage).
    /// Set by exec_process when loading an ELF with PT_TLS segment.
    /// Read by sys_exec before enter_usermode to set MSR 0xC0000100.
    pub tls_fs_base: AtomicU64,

    /// Container ID (0 = not containerized). Inherited by forked children
    /// so processes cannot escape their container namespace.
    pub container_id: AtomicU64,

    /// User-space address to zero and futex-wake on thread exit
    /// (set by set_tid_address syscall, used by pthread_join).
    pub clear_child_tid: AtomicU64,
}

/// Memory usage statistics
#[derive(Debug, Default)]
pub struct MemoryStats {
    /// Virtual memory size (bytes)
    pub virtual_size: AtomicU64,
    /// Resident set size (bytes)
    pub resident_size: AtomicU64,
    /// Shared memory size (bytes)
    pub shared_size: AtomicU64,
}

impl Process {
    /// Parent process ID (None for init).
    pub fn parent(&self) -> Option<ProcessId> {
        *self.parent.lock()
    }

    /// Change the parent (used when reparenting orphans to init).
    pub fn set_parent(&self, parent: Option<ProcessId>) {
        *self.parent.lock() = parent;
    }

    /// A copy of the credentials. Permission checks use the effective IDs
    /// and `Credentials::gid_for`.
    pub fn credentials(&self) -> super::creds::Credentials {
        *self.creds.lock()
    }

    /// Change the credentials atomically; `f` enforces the set*id rules.
    /// As Linux's commit_creds, a change of the effective user or group
    /// makes the process non-dumpable: memory it read with the old
    /// identity (a dropped root's secrets) must not become readable to the
    /// new one through ptrace. exec decides dumpability itself afterwards.
    pub fn update_credentials<R>(&self, f: impl FnOnce(&mut super::creds::Credentials) -> R) -> R {
        let mut creds = self.creds.lock();
        let (euid, egid) = (creds.euid, creds.egid);
        let result = f(&mut creds);
        if creds.euid != euid || creds.egid != egid {
            self.dumpable
                .store(false, core::sync::atomic::Ordering::Release);
        }
        result
    }

    /// Real user ID.
    pub fn uid(&self) -> u32 {
        self.creds.lock().ruid
    }

    /// Effective user ID, the one permission checks use.
    pub fn euid(&self) -> u32 {
        self.creds.lock().euid
    }

    /// Real group ID.
    pub fn gid(&self) -> u32 {
        self.creds.lock().rgid
    }

    /// Effective group ID.
    pub fn egid(&self) -> u32 {
        self.creds.lock().egid
    }

    /// Create a new process
    #[cfg(feature = "alloc")]
    pub fn new(
        pid: ProcessId,
        parent: Option<ProcessId>,
        name: String,
        priority: ProcessPriority,
    ) -> Self {
        Self {
            pid,
            parent: Mutex::new(parent),
            name,
            state: AtomicU32::new(ProcessState::Creating as u32),
            priority: Mutex::new(priority),
            memory_space: crate::sync::sleep_mutex::SleepMutex::new(VirtualAddressSpace::new()),
            capability_space: Mutex::new(CapabilitySpace::new()),
            file_table: Mutex::new(FileTable::new()),
            threads: Mutex::new(BTreeMap::new()),
            ipc_endpoints: Mutex::new(BTreeMap::new()),
            children: Mutex::new(Vec::new()),
            exit_code: AtomicU32::new(0),
            term_signal: AtomicU32::new(0),
            dispatched: core::sync::atomic::AtomicBool::new(false),
            kill_pending: AtomicU32::new(0),
            exit_signal: AtomicU32::new(super::signals::SIGCHLD as u32),
            vfork_pending: core::sync::atomic::AtomicBool::new(false),
            did_exec: core::sync::atomic::AtomicBool::new(false),
            stop_signal: AtomicU32::new(0),
            cont_seq: AtomicU32::new(0),
            job_report: AtomicU32::new(0),
            cpu_time: AtomicU64::new(0),
            memory_stats: MemoryStats::default(),
            created_at: crate::arch::timer::get_ticks(),
            creds: Mutex::new(super::creds::Credentials::new(0, 0)),
            pgid: AtomicU64::new(pid.0),
            tracer: AtomicU64::new(0),
            cred_guard: Mutex::new(()),
            dumpable: core::sync::atomic::AtomicBool::new(true),
            cpu_exited: Mutex::new(crate::sched::cputime::CpuTimes::ZERO),
            cpu_children: Mutex::new(crate::sched::cputime::CpuTimes::ZERO),
            peak_rss_pages: core::sync::atomic::AtomicUsize::new(0),
            children_peak_rss_pages: core::sync::atomic::AtomicUsize::new(0),
            sid: AtomicU64::new(pid.0),
            env_vars: Mutex::new(BTreeMap::new()),
            #[cfg(feature = "alloc")]
            exe_path: Mutex::new(String::new()),
            signal_handlers: Mutex::new([0u64; super::signals::NSIG + 1]),
            signal_action_extra: Mutex::new([(0, 0, 0); super::signals::NSIG + 1]),
            pending_signals: AtomicU64::new(0),
            rt_queue: super::signals::RtQueue::new(),
            signal_mask: AtomicU64::new(0),
            umask: AtomicU32::new(0o022),
            tls_fs_base: AtomicU64::new(0),
            container_id: AtomicU64::new(0),
            clear_child_tid: AtomicU64::new(0),
        }
    }

    /// Get process state
    pub fn get_state(&self) -> ProcessState {
        match self.state.load(Ordering::Acquire) {
            0 => ProcessState::Creating,
            1 => ProcessState::Ready,
            2 => ProcessState::Running,
            3 => ProcessState::Blocked,
            4 => ProcessState::Sleeping,
            5 => ProcessState::Zombie,
            6 => ProcessState::Dead,
            _ => ProcessState::Dead,
        }
    }

    /// Set process state
    pub fn set_state(&self, state: ProcessState) {
        self.state.store(state as u32, Ordering::Release);
    }

    /// Get the main thread ID of this process
    #[cfg(feature = "alloc")]
    pub fn get_main_thread_id(&self) -> Option<ThreadId> {
        let threads = self.threads.lock();
        // The main thread is typically the first one created (lowest TID)
        threads.values().min_by_key(|t| t.tid.0).map(|t| t.tid)
    }

    /// Add a thread to this process
    #[cfg(feature = "alloc")]
    pub fn add_thread(&self, thread: Thread) -> Result<Arc<Thread>, KernelError> {
        let tid = thread.tid;
        let mut threads = self.threads.lock();

        if threads.len() >= super::MAX_THREADS_PER_PROCESS {
            return Err(KernelError::ResourceExhausted {
                resource: "threads per process",
            });
        }

        if threads.contains_key(&tid) {
            return Err(KernelError::AlreadyExists {
                resource: "thread",
                id: tid.0,
            });
        }

        let thread = Arc::new(thread);
        threads.insert(tid, thread.clone());
        Ok(thread)
    }

    /// Remove a thread from this process
    #[cfg(feature = "alloc")]
    pub fn remove_thread(&self, tid: ThreadId) -> Option<Arc<Thread>> {
        self.threads.lock().remove(&tid)
    }

    /// Get a thread by ID
    #[cfg(feature = "alloc")]
    pub fn get_thread(&self, tid: ThreadId) -> Option<Arc<Thread>> {
        self.threads.lock().get(&tid).cloned()
    }

    /// Get number of threads
    #[cfg(feature = "alloc")]
    pub fn thread_count(&self) -> usize {
        self.threads.lock().len()
    }

    /// Check if process is alive
    pub fn is_alive(&self) -> bool {
        !matches!(self.get_state(), ProcessState::Dead | ProcessState::Zombie)
    }

    /// Update CPU time
    pub fn add_cpu_time(&self, microseconds: u64) {
        self.cpu_time.fetch_add(microseconds, Ordering::Relaxed);
    }

    /// A thread of this process exited having used `times`.
    pub fn add_exited_cpu(&self, times: crate::sched::cputime::CpuTimes) {
        let mut sum = self.cpu_exited.lock();
        *sum = sum.plus(times);
    }

    /// The CPU time the process has used: its exited threads' and its live
    /// threads' (getrusage RUSAGE_SELF, CLOCK_PROCESS_CPUTIME_ID).
    pub fn cpu_times(&self) -> crate::sched::cputime::CpuTimes {
        let exited = *self.cpu_exited.lock();
        #[cfg(feature = "alloc")]
        let exited = exited.plus(crate::sched::dispatch::process_tasks_cpu(self.pid.0));
        exited
    }

    /// The CPU time of the children waited for and their own children
    /// (getrusage RUSAGE_CHILDREN, times' cutime and cstime).
    pub fn children_cpu_times(&self) -> crate::sched::cputime::CpuTimes {
        *self.cpu_children.lock()
    }

    /// A child was reaped: its time and its children's count as this
    /// process's children's, and its peak resident set as theirs.
    pub fn add_reaped_child_cpu(&self, child: &Self) {
        let theirs = child.cpu_times().plus(child.children_cpu_times());
        let mut sum = self.cpu_children.lock();
        *sum = sum.plus(theirs);
        let peak = child.peak_rss_pages().max(child.children_peak_rss_pages());
        self.children_peak_rss_pages
            .fetch_max(peak, Ordering::Relaxed);
    }

    /// The largest resident set seen, in pages, counting the current one
    /// while the process still has its memory.
    pub fn peak_rss_pages(&self) -> usize {
        self.note_rss();
        self.peak_rss_pages.load(Ordering::Relaxed)
    }

    /// The largest resident set of the children waited for, in pages.
    pub fn children_peak_rss_pages(&self) -> usize {
        self.children_peak_rss_pages.load(Ordering::Relaxed)
    }

    /// Record the current resident set in the peak (on reads, and before
    /// the memory goes at exit).
    pub fn note_rss(&self) {
        #[cfg(feature = "alloc")]
        if let Some(space) = self.memory_space.try_lock() {
            self.peak_rss_pages
                .fetch_max(space.resident_pages(), Ordering::Relaxed);
        }
    }

    /// Get total CPU time
    pub fn get_cpu_time(&self) -> u64 {
        self.cpu_time.load(Ordering::Relaxed)
    }

    /// Set exit code
    pub fn set_exit_code(&self, code: i32) {
        self.exit_code.store(code as u32, Ordering::Release);
    }

    /// Get exit code
    pub fn get_exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Acquire) as i32
    }

    /// Record that signal `sig` terminated the process.
    pub fn set_term_signal(&self, sig: u32) {
        self.term_signal.store(sig & 0x7f, Ordering::Release);
    }

    /// The POSIX/Linux wait status for this (exited) process.
    pub fn wait_status(&self) -> i32 {
        wait_status(
            self.get_exit_code(),
            self.term_signal.load(Ordering::Acquire),
        )
    }

    /// Set process priority
    pub fn set_priority(&self, new_priority: ProcessPriority) {
        *self.priority.lock() = new_priority;
    }

    /// Set user-space address to zero and futex-wake on thread exit.
    /// Called by set_tid_address syscall; the kernel will write 0 to this
    /// address and issue a futex wake when the thread terminates, enabling
    /// pthread_join to detect thread completion.
    pub fn set_clear_child_tid(&self, addr: usize) {
        self.clear_child_tid.store(addr as u64, Ordering::Release);
    }

    /// Get mutable reference to memory space
    pub fn memory_space_mut(&mut self) -> Option<&mut VirtualAddressSpace> {
        Some(self.memory_space.get_mut())
    }

    /// Set process name
    #[cfg(feature = "alloc")]
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Reset all signal handlers to default (used during exec)
    pub fn reset_signal_handlers(&self) {
        // Caught signals revert to their default action; ignored ones stay
        // ignored (POSIX exec), so a parent's SIG_IGN still applies.
        let mut handlers = self.signal_handlers.lock();
        let mut ignored = 0u64;
        for (sig, handler) in handlers.iter_mut().enumerate() {
            if *handler == 1 {
                ignored |= super::signals::sig_bit(sig);
            } else {
                *handler = 0; // 0 = default action
            }
        }
        *self.signal_action_extra.lock() = [(0, 0, 0); super::signals::NSIG + 1];
        // Pending signals survive exec (POSIX), except the ignored ones.
        self.rt_queue.clear(&self.pending_signals, ignored);
    }

    /// Set a signal handler
    /// handler: 0 = default, 1 = ignore, other = handler address
    pub fn set_signal_handler(&self, signum: usize, handler: u64) -> Result<u64, KernelError> {
        if signum > super::signals::NSIG {
            return Err(KernelError::InvalidArgument {
                name: "signum",
                value: "signal number out of range (0-64)",
            });
        }
        // SIGKILL (9) and SIGSTOP (19) cannot be caught or ignored
        if signum == 9 || signum == 19 {
            return Err(KernelError::PermissionDenied {
                operation: "change handler for SIGKILL or SIGSTOP",
            });
        }
        let mut handlers = self.signal_handlers.lock();
        let old = handlers[signum];
        handlers[signum] = handler;
        Ok(old)
    }

    /// Get a signal handler
    pub fn get_signal_handler(&self, signum: usize) -> Option<u64> {
        if signum > super::signals::NSIG {
            return None;
        }
        Some(self.signal_handlers.lock()[signum])
    }

    /// A CLONE_VFORK child has exec'd or exited: its parent may resume.
    pub fn release_vfork_parent(&self) {
        if self.vfork_pending.swap(false, Ordering::AcqRel) {
            #[cfg(feature = "alloc")]
            crate::sched::dispatch::PROCESS_EVENTS.wake_all();
        }
    }

    /// Send a signal to this process
    pub fn send_signal(&self, signum: usize) -> Result<(), KernelError> {
        if signum > super::signals::NSIG {
            return Err(KernelError::InvalidArgument {
                name: "signum",
                value: "signal number out of range (0-64)",
            });
        }
        if signum == 0 {
            return Ok(());
        }
        // Linux set layout: bit `signum - 1` (N-96); real-time signals
        // queue (N-209), EAGAIN when the queue is full.
        self.rt_queue
            .push(&self.pending_signals, signum)
            .map_err(|_| KernelError::WouldBlock)?;
        crate::fs::signalfd::signal_generated();
        Ok(())
    }

    /// Check if a signal is pending
    pub fn is_signal_pending(&self, signum: usize) -> bool {
        if signum > super::signals::NSIG {
            return false;
        }
        let pending = self.pending_signals.load(Ordering::Acquire);
        let mask = self.signal_mask.load(Ordering::Acquire);
        let effective_pending = pending & !mask;
        (effective_pending & super::signals::sig_bit(signum)) != 0
    }

    /// Get the next pending signal (lowest numbered, unmasked)
    pub fn get_next_pending_signal(&self) -> Option<usize> {
        let pending = self.pending_signals.load(Ordering::Acquire);
        let mask = self.signal_mask.load(Ordering::Acquire);
        let effective_pending = pending & !mask;
        if effective_pending == 0 {
            return None;
        }
        // Lowest set bit; bit n is signal n + 1.
        Some(effective_pending.trailing_zeros() as usize + 1)
    }

    /// Clear a pending signal
    pub fn clear_pending_signal(&self, signum: usize) {
        self.rt_queue.take(&self.pending_signals, signum);
    }

    /// Set signal mask (returns old mask)
    pub fn set_signal_mask(&self, new_mask: u64) -> u64 {
        // Cannot mask SIGKILL (9) or SIGSTOP (19)
        let actual_mask = new_mask & !super::signals::UNBLOCKABLE;
        self.signal_mask.swap(actual_mask, Ordering::AcqRel)
    }

    /// Get current signal mask
    pub fn get_signal_mask(&self) -> u64 {
        self.signal_mask.load(Ordering::Acquire)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        println!("[PROCESS] Dropping process {}", self.pid.0);
        // Cleanup will be handled by the process lifecycle manager
    }
}

/// Linux wait-status encoding (N-99): a normal exit is `code << 8`
/// (`WIFEXITED`, `WEXITSTATUS`); death by signal is the signal number in the
/// low 7 bits (`WIFSIGNALED`, `WTERMSIG`), with 0x80 set when the signal
/// dumps core by default (`WCOREDUMP`).
pub fn wait_status(exit_code: i32, term_signal: u32) -> i32 {
    match term_signal & 0x7f {
        0 => (exit_code & 0xff) << 8,
        sig => {
            // SIGQUIT, SIGILL, SIGTRAP, SIGABRT, SIGBUS, SIGFPE, SIGSEGV,
            // SIGXCPU, SIGXFSZ, SIGSYS default to a core dump.
            let core = matches!(sig, 3 | 4 | 5 | 6 | 7 | 8 | 11 | 24 | 25 | 31);
            sig as i32 | if core { 0x80 } else { 0 }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wait_status_encodes_exit_and_signal() {
        assert_eq!(wait_status(3, 0), 0x300); // WIFEXITED, WEXITSTATUS 3
        assert_eq!(wait_status(0, 9), 9); // WIFSIGNALED, SIGKILL, no core
        assert_eq!(wait_status(0, 11), 0x8b); // SIGSEGV with WCOREDUMP
        assert_eq!(wait_status(0, 15), 15);
        assert_eq!(wait_status(0x1ff, 0), 0xff00);
    }

    use super::*;

    fn make_process(pid: u64, name: &str) -> Process {
        Process::new(
            ProcessId(pid),
            None,
            alloc::string::String::from(name),
            ProcessPriority::Normal,
        )
    }

    /// A change of effective user or group makes the process non-dumpable
    /// (Linux's commit_creds); other credential changes leave it.
    #[test]
    fn effective_id_changes_clear_dumpable() {
        use core::sync::atomic::Ordering;
        let proc = make_process(3, "creds");
        assert!(proc.dumpable.load(Ordering::Acquire));
        proc.update_credentials(|c| c.setgroups(&[5]).unwrap());
        assert!(proc.dumpable.load(Ordering::Acquire));
        proc.update_credentials(|c| c.setuid(1000).unwrap());
        assert!(!proc.dumpable.load(Ordering::Acquire));
    }

    // --- ProcessState tests ---

    #[test]
    fn test_initial_state_is_creating() {
        let proc = make_process(1, "test");
        assert_eq!(proc.get_state(), ProcessState::Creating);
    }

    #[test]
    fn test_state_transitions() {
        let proc = make_process(2, "test_transitions");

        proc.set_state(ProcessState::Ready);
        assert_eq!(proc.get_state(), ProcessState::Ready);

        proc.set_state(ProcessState::Running);
        assert_eq!(proc.get_state(), ProcessState::Running);

        proc.set_state(ProcessState::Blocked);
        assert_eq!(proc.get_state(), ProcessState::Blocked);

        proc.set_state(ProcessState::Sleeping);
        assert_eq!(proc.get_state(), ProcessState::Sleeping);

        proc.set_state(ProcessState::Zombie);
        assert_eq!(proc.get_state(), ProcessState::Zombie);

        proc.set_state(ProcessState::Dead);
        assert_eq!(proc.get_state(), ProcessState::Dead);
    }

    #[test]
    fn test_get_state_unknown_value() {
        let proc = make_process(3, "unknown_state");
        // Force an invalid state value
        proc.state.store(255, Ordering::Release);
        // Should default to Dead for unknown values
        assert_eq!(proc.get_state(), ProcessState::Dead);
    }

    // --- is_alive tests ---

    #[test]
    fn test_is_alive_creating() {
        let proc = make_process(4, "alive_test");
        assert!(proc.is_alive());
    }

    #[test]
    fn test_is_alive_ready() {
        let proc = make_process(5, "alive_ready");
        proc.set_state(ProcessState::Ready);
        assert!(proc.is_alive());
    }

    #[test]
    fn test_is_alive_running() {
        let proc = make_process(6, "alive_running");
        proc.set_state(ProcessState::Running);
        assert!(proc.is_alive());
    }

    #[test]
    fn test_is_not_alive_zombie() {
        let proc = make_process(7, "zombie");
        proc.set_state(ProcessState::Zombie);
        assert!(!proc.is_alive());
    }

    #[test]
    fn test_is_not_alive_dead() {
        let proc = make_process(8, "dead");
        proc.set_state(ProcessState::Dead);
        assert!(!proc.is_alive());
    }

    // --- CPU time tests ---

    #[test]
    fn test_cpu_time_initial_zero() {
        let proc = make_process(10, "cpu_time");
        assert_eq!(proc.get_cpu_time(), 0);
    }

    #[test]
    fn test_add_cpu_time() {
        let proc = make_process(11, "cpu_time_add");
        proc.add_cpu_time(100);
        assert_eq!(proc.get_cpu_time(), 100);
        proc.add_cpu_time(200);
        assert_eq!(proc.get_cpu_time(), 300);
    }

    // --- Exit code tests ---

    #[test]
    fn test_exit_code_initial_zero() {
        let proc = make_process(12, "exit_code");
        assert_eq!(proc.get_exit_code(), 0);
    }

    #[test]
    fn test_set_exit_code() {
        let proc = make_process(13, "exit_set");
        proc.set_exit_code(42);
        assert_eq!(proc.get_exit_code(), 42);
    }

    #[test]
    fn test_set_exit_code_negative() {
        let proc = make_process(14, "exit_neg");
        proc.set_exit_code(-1);
        assert_eq!(proc.get_exit_code(), -1);
    }

    // --- Priority tests ---

    #[test]
    fn test_initial_priority() {
        let proc = make_process(15, "priority");
        assert_eq!(*proc.priority.lock(), ProcessPriority::Normal);
    }

    #[test]
    fn test_set_priority() {
        let proc = make_process(16, "priority_set");
        proc.set_priority(ProcessPriority::RealTime);
        assert_eq!(*proc.priority.lock(), ProcessPriority::RealTime);

        proc.set_priority(ProcessPriority::Idle);
        assert_eq!(*proc.priority.lock(), ProcessPriority::Idle);
    }

    // --- Signal tests ---

    #[test]
    fn test_signal_handler_default() {
        let proc = make_process(20, "sig_default");
        // All signal handlers should initially be 0 (default action)
        for i in 0..32 {
            assert_eq!(proc.get_signal_handler(i), Some(0));
        }
    }

    #[test]
    fn test_signal_handler_invalid_signal() {
        let proc = make_process(21, "sig_invalid");
        assert_eq!(proc.get_signal_handler(65), None);
        assert_eq!(proc.get_signal_handler(100), None);
    }

    #[test]
    fn test_set_signal_handler() {
        let proc = make_process(22, "sig_set");
        let result = proc.set_signal_handler(2, 0xDEAD_BEEF);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0); // Old handler was default (0)
        assert_eq!(proc.get_signal_handler(2), Some(0xDEAD_BEEF));
    }

    #[test]
    fn test_set_signal_handler_sigkill_refused() {
        let proc = make_process(23, "sig_kill");
        let result = proc.set_signal_handler(9, 1); // SIGKILL
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KernelError::PermissionDenied {
                operation: "change handler for SIGKILL or SIGSTOP",
            }
        );
    }

    #[test]
    fn test_set_signal_handler_sigstop_refused() {
        let proc = make_process(24, "sig_stop");
        let result = proc.set_signal_handler(19, 1); // SIGSTOP
        assert!(result.is_err());
    }

    #[test]
    fn test_set_signal_handler_out_of_range() {
        let proc = make_process(25, "sig_range");
        let result = proc.set_signal_handler(65, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_send_signal() {
        let proc = make_process(26, "sig_send");
        assert!(proc.send_signal(2).is_ok());
        assert!(proc.is_signal_pending(2));
    }

    #[test]
    fn test_send_signal_invalid() {
        let proc = make_process(27, "sig_send_inv");
        assert!(proc.send_signal(65).is_err());
        assert!(proc.set_signal_handler(65, 0x1000).is_err());
        assert_eq!(proc.get_signal_handler(65), None);
    }

    /// Signals 32-64 exist (musl's pthread_cancel uses 33) and real-time
    /// signals queue: each send is delivered once, while a standard signal
    /// sent twice before delivery is delivered once (N-209).
    #[test]
    fn real_time_signals_queue() {
        let proc = make_process(29, "sig_rt");
        proc.set_signal_handler(64, 0x1000).unwrap();
        assert_eq!(proc.get_signal_handler(64), Some(0x1000));
        for _ in 0..3 {
            proc.send_signal(33).unwrap();
            proc.send_signal(2).unwrap();
        }
        let mut taken = alloc::vec::Vec::new();
        while let Some(sig) = proc.get_next_pending_signal() {
            proc.clear_pending_signal(sig);
            taken.push(sig);
        }
        assert_eq!(taken, alloc::vec![2, 33, 33, 33]);
    }

    #[test]
    fn real_time_queue_is_bounded() {
        let proc = make_process(30, "sig_rt_full");
        for _ in 0..crate::process::signals::RT_QUEUE_MAX {
            proc.send_signal(40).unwrap();
        }
        assert!(matches!(proc.send_signal(40), Err(KernelError::WouldBlock)));
        // A standard signal never queues, so never fills.
        for _ in 0..2000 {
            proc.send_signal(10).unwrap();
        }
    }

    #[test]
    fn exec_keeps_pending_signals_except_ignored() {
        let proc = make_process(31, "sig_exec");
        proc.set_signal_handler(10, 1).unwrap(); // SIG_IGN
        proc.set_signal_handler(12, 0x2000).unwrap();
        proc.send_signal(10).unwrap();
        proc.send_signal(12).unwrap();
        proc.send_signal(35).unwrap();
        proc.reset_signal_handlers();
        assert!(!proc.is_signal_pending(10));
        assert!(proc.is_signal_pending(12));
        assert!(proc.is_signal_pending(35));
        assert_eq!(proc.get_signal_handler(10), Some(1));
        assert_eq!(proc.get_signal_handler(12), Some(0));
    }

    #[test]
    fn test_signal_pending_with_mask() {
        let proc = make_process(28, "sig_mask");
        proc.send_signal(3).unwrap();

        // Before masking, signal is pending
        assert!(proc.is_signal_pending(3));

        // Mask signal 3 (Linux layout: bit 2; N-96)
        proc.set_signal_mask(1u64 << 2);

        // Now it should NOT be seen as pending (masked)
        assert!(!proc.is_signal_pending(3));
    }

    #[test]
    fn test_signal_mask_protects_sigkill_sigstop() {
        let proc = make_process(29, "sig_mask_protect");
        // Try to mask SIGKILL (9) and SIGSTOP (19)
        let old = proc.set_signal_mask(0xFFFF_FFFF_FFFF_FFFF);
        assert_eq!(old, 0); // Previous mask was 0

        // SIGKILL and SIGSTOP should NOT be masked
        let mask = proc.get_signal_mask();
        // Linux layout: signal n is bit n - 1 (N-96).
        assert_eq!(mask & (1u64 << 8), 0, "SIGKILL should not be maskable");
        assert_eq!(mask & (1u64 << 18), 0, "SIGSTOP should not be maskable");
    }

    #[test]
    fn test_get_next_pending_signal() {
        let proc = make_process(30, "sig_next");
        assert!(proc.get_next_pending_signal().is_none());

        proc.send_signal(5).unwrap();
        proc.send_signal(3).unwrap();

        // Should return lowest numbered pending signal
        assert_eq!(proc.get_next_pending_signal(), Some(3));
    }

    #[test]
    fn test_clear_pending_signal() {
        let proc = make_process(31, "sig_clear");
        proc.send_signal(7).unwrap();
        assert!(proc.is_signal_pending(7));

        proc.clear_pending_signal(7);
        assert!(!proc.is_signal_pending(7));
    }

    #[test]
    fn test_reset_signal_handlers() {
        let proc = make_process(32, "sig_reset");
        // Set some handlers
        proc.set_signal_handler(2, 0x1000).unwrap();
        proc.set_signal_handler(15, 0x2000).unwrap();
        proc.send_signal(5).unwrap();

        proc.reset_signal_handlers();

        // All handlers should be reset to default
        assert_eq!(proc.get_signal_handler(2), Some(0));
        assert_eq!(proc.get_signal_handler(15), Some(0));
        // A pending signal survives exec (POSIX execve).
        assert!(proc.is_signal_pending(5));
    }

    // --- Process identity tests ---

    #[test]
    fn test_process_pid() {
        let proc = make_process(100, "pid_test");
        assert_eq!(proc.pid, ProcessId(100));
    }

    #[test]
    fn test_process_parent() {
        let proc = Process::new(
            ProcessId(50),
            Some(ProcessId(1)),
            alloc::string::String::from("child"),
            ProcessPriority::Normal,
        );
        assert_eq!(proc.parent(), Some(ProcessId(1)));
    }

    #[test]
    fn test_process_no_parent() {
        let proc = make_process(1, "init");
        assert_eq!(proc.parent(), None);
    }

    #[test]
    fn test_process_name() {
        let mut proc = make_process(60, "original_name");
        assert_eq!(proc.name, "original_name");

        proc.set_name(alloc::string::String::from("new_name"));
        assert_eq!(proc.name, "new_name");
    }

    // --- Thread management tests ---

    #[test]
    fn test_thread_count_initially_zero() {
        let proc = make_process(70, "threads");
        assert_eq!(proc.thread_count(), 0);
    }

    // --- ProcessId display ---

    #[test]
    fn test_process_id_display() {
        let pid = ProcessId(42);
        let display = alloc::format!("{}", pid);
        assert_eq!(display, "42");
    }

    // --- ProcessPriority ordering ---

    #[test]
    fn test_priority_ordering() {
        assert!(ProcessPriority::RealTime < ProcessPriority::System);
        assert!(ProcessPriority::System < ProcessPriority::Normal);
        assert!(ProcessPriority::Normal < ProcessPriority::Low);
        assert!(ProcessPriority::Low < ProcessPriority::Idle);
    }
}

/// Process builder for convenient process creation
#[cfg(feature = "alloc")]
pub struct ProcessBuilder {
    name: String,
    parent: Option<ProcessId>,
    priority: ProcessPriority,
    creds: super::creds::Credentials,
}

#[cfg(feature = "alloc")]
impl ProcessBuilder {
    /// Create a new process builder
    pub fn new(name: String) -> Self {
        Self {
            name,
            parent: None,
            priority: ProcessPriority::Normal,
            creds: super::creds::Credentials::new(0, 0),
        }
    }

    /// Set parent process
    pub fn parent(mut self, pid: ProcessId) -> Self {
        self.parent = Some(pid);
        self
    }

    /// Set priority
    pub fn priority(mut self, priority: ProcessPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the credentials (a forked child takes its parent's).
    pub fn credentials(mut self, creds: super::creds::Credentials) -> Self {
        self.creds = creds;
        self
    }

    /// Build the process.
    ///
    /// Note: The VAS is created but not initialized (no page table root).
    /// Callers that need a real address space must call
    /// `memory_space.lock().init()` afterwards (as
    /// `create_process_with_options` does), or clone from an existing
    /// address space (as `fork_process` does).
    pub fn build(self) -> Process {
        let pid = super::alloc_pid();
        let process = Process::new(pid, self.parent, self.name, self.priority);
        process.update_credentials(|c| *c = self.creds);
        process
    }

    /// Build the process with an initialized address space.
    ///
    /// Allocates a root page table frame and maps kernel regions into the
    /// new address space. This is the preferred method for creating
    /// standalone processes (not forked from an existing process).
    pub fn build_with_address_space(self) -> Result<Process, KernelError> {
        let pid = super::alloc_pid();
        let process = Process::new(pid, self.parent, self.name, self.priority);
        process.update_credentials(|c| *c = self.creds);

        // Initialize the virtual address space with a real page table root
        // and kernel space mappings
        {
            let mut memory_space = process.memory_space.lock();
            memory_space.init()?;
        }

        println!(
            "[PROCESS] Created process {} with initialized address space",
            pid.0
        );

        Ok(process)
    }
}

//! Process Control Block (PCB) implementation
//!
//! The PCB is the core data structure representing a process in the kernel.
//! It contains all the information needed to manage a process.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

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
    pub parent: Option<ProcessId>,

    /// Process name
    #[cfg(feature = "alloc")]
    pub name: String,

    /// Process state
    pub state: AtomicU32,

    /// Priority
    pub priority: Mutex<ProcessPriority>,

    /// Virtual address space
    pub memory_space: Mutex<VirtualAddressSpace>,

    /// Capability space
    pub capability_space: Mutex<CapabilitySpace>,

    /// File descriptor table
    pub file_table: Mutex<FileTable>,

    /// Threads in this process
    #[cfg(feature = "alloc")]
    pub threads: Mutex<BTreeMap<ThreadId, Thread>>,

    /// IPC endpoints owned by this process
    #[cfg(feature = "alloc")]
    pub ipc_endpoints: Mutex<BTreeMap<EndpointId, CapabilityId>>,

    /// Child processes
    #[cfg(feature = "alloc")]
    pub children: Mutex<Vec<ProcessId>>,

    /// Exit code (set when process exits)
    pub exit_code: AtomicU32,

    /// CPU time used (in microseconds)
    pub cpu_time: AtomicU64,

    /// Memory usage statistics
    pub memory_stats: MemoryStats,

    /// Creation timestamp
    pub created_at: u64,

    /// User ID (for future use)
    pub uid: u32,

    /// Group ID (for future use)
    pub gid: u32,

    /// Process group ID (initialized to pid)
    pub pgid: AtomicU64,

    /// Session ID (initialized to pid)
    pub sid: AtomicU64,

    /// Environment variables (populated during exec, inherited on fork)
    #[cfg(feature = "alloc")]
    pub env_vars: Mutex<alloc::collections::BTreeMap<String, String>>,

    /// Signal handlers (signal number -> handler action)
    /// 0 = default, 1 = ignore, other values = handler address
    pub signal_handlers: Mutex<[u64; 32]>,

    /// Pending signals bitmap
    pub pending_signals: AtomicU64,

    /// Signal mask (blocked signals)
    pub signal_mask: AtomicU64,

    /// File creation mask (umask). Default 0o022.
    pub umask: AtomicU32,

    /// TLS FS_BASE address for x86_64 (Thread-Local Storage).
    /// Set by exec_process when loading an ELF with PT_TLS segment.
    /// Read by sys_exec before enter_usermode to set MSR 0xC0000100.
    pub tls_fs_base: AtomicU64,
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
            parent,
            name,
            state: AtomicU32::new(ProcessState::Creating as u32),
            priority: Mutex::new(priority),
            memory_space: Mutex::new(VirtualAddressSpace::new()),
            capability_space: Mutex::new(CapabilitySpace::new()),
            file_table: Mutex::new(FileTable::new()),
            threads: Mutex::new(BTreeMap::new()),
            ipc_endpoints: Mutex::new(BTreeMap::new()),
            children: Mutex::new(Vec::new()),
            exit_code: AtomicU32::new(0),
            cpu_time: AtomicU64::new(0),
            memory_stats: MemoryStats::default(),
            created_at: crate::arch::timer::get_ticks(),
            uid: 0,
            gid: 0,
            pgid: AtomicU64::new(pid.0),
            sid: AtomicU64::new(pid.0),
            env_vars: Mutex::new(BTreeMap::new()),
            signal_handlers: Mutex::new([0u64; 32]),
            pending_signals: AtomicU64::new(0),
            signal_mask: AtomicU64::new(0),
            umask: AtomicU32::new(0o022),
            tls_fs_base: AtomicU64::new(0),
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
    pub fn add_thread(&self, thread: Thread) -> Result<(), KernelError> {
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

        threads.insert(tid, thread);
        Ok(())
    }

    /// Remove a thread from this process
    #[cfg(feature = "alloc")]
    pub fn remove_thread(&self, tid: ThreadId) -> Option<Thread> {
        self.threads.lock().remove(&tid)
    }

    /// Get a thread by ID
    #[cfg(feature = "alloc")]
    pub fn get_thread(&self, tid: ThreadId) -> Option<&Thread> {
        // This is a bit tricky - we need to return a reference that outlives the lock
        // In a real implementation, we'd use more sophisticated synchronization
        // SAFETY: The Thread is stored in a BTreeMap behind a Mutex, providing
        // a stable heap address. Casting to *const and back to a reference
        // extends the borrow lifetime beyond the lock scope. Sound because
        // threads are not moved or deallocated while references exist in the
        // current kernel model.
        unsafe {
            let threads = self.threads.lock();
            threads.get(&tid).map(|t| &*(t as *const Thread))
        }
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

    /// Set process priority
    pub fn set_priority(&self, new_priority: ProcessPriority) {
        *self.priority.lock() = new_priority;
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

    /// Get main thread (first thread created)
    #[cfg(feature = "alloc")]
    pub fn get_main_thread_mut(&mut self) -> Option<&mut Thread> {
        self.threads.get_mut().values_mut().next()
    }

    /// Reset all signal handlers to default (used during exec)
    pub fn reset_signal_handlers(&self) {
        let mut handlers = self.signal_handlers.lock();
        for handler in handlers.iter_mut() {
            *handler = 0; // 0 = default action
        }
        // Clear pending signals that were ignored
        self.pending_signals.store(0, Ordering::Release);
    }

    /// Set a signal handler
    /// handler: 0 = default, 1 = ignore, other = handler address
    pub fn set_signal_handler(&self, signum: usize, handler: u64) -> Result<u64, KernelError> {
        if signum >= 32 {
            return Err(KernelError::InvalidArgument {
                name: "signum",
                value: "signal number out of range (0-31)",
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
        if signum >= 32 {
            return None;
        }
        Some(self.signal_handlers.lock()[signum])
    }

    /// Send a signal to this process
    pub fn send_signal(&self, signum: usize) -> Result<(), KernelError> {
        if signum >= 32 {
            return Err(KernelError::InvalidArgument {
                name: "signum",
                value: "signal number out of range (0-31)",
            });
        }
        // Set the signal bit in pending signals
        let mask = 1u64 << signum;
        self.pending_signals.fetch_or(mask, Ordering::AcqRel);
        Ok(())
    }

    /// Check if a signal is pending
    pub fn is_signal_pending(&self, signum: usize) -> bool {
        if signum >= 32 {
            return false;
        }
        let pending = self.pending_signals.load(Ordering::Acquire);
        let mask = self.signal_mask.load(Ordering::Acquire);
        let effective_pending = pending & !mask;
        (effective_pending & (1u64 << signum)) != 0
    }

    /// Get the next pending signal (lowest numbered, unmasked)
    pub fn get_next_pending_signal(&self) -> Option<usize> {
        let pending = self.pending_signals.load(Ordering::Acquire);
        let mask = self.signal_mask.load(Ordering::Acquire);
        let effective_pending = pending & !mask;
        if effective_pending == 0 {
            return None;
        }
        // Find lowest set bit
        Some(effective_pending.trailing_zeros() as usize)
    }

    /// Clear a pending signal
    pub fn clear_pending_signal(&self, signum: usize) {
        if signum < 32 {
            let mask = !(1u64 << signum);
            self.pending_signals.fetch_and(mask, Ordering::AcqRel);
        }
    }

    /// Set signal mask (returns old mask)
    pub fn set_signal_mask(&self, new_mask: u64) -> u64 {
        // Cannot mask SIGKILL (9) or SIGSTOP (19)
        let protected = (1u64 << 9) | (1u64 << 19);
        let actual_mask = new_mask & !protected;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_process(pid: u64, name: &str) -> Process {
        Process::new(
            ProcessId(pid),
            None,
            alloc::string::String::from(name),
            ProcessPriority::Normal,
        )
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
        assert_eq!(proc.get_signal_handler(32), None);
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
        let result = proc.set_signal_handler(32, 1);
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
        assert!(proc.send_signal(32).is_err());
    }

    #[test]
    fn test_signal_pending_with_mask() {
        let proc = make_process(28, "sig_mask");
        proc.send_signal(3).unwrap();

        // Before masking, signal is pending
        assert!(proc.is_signal_pending(3));

        // Mask signal 3
        proc.set_signal_mask(1u64 << 3);

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
        assert_eq!(mask & (1u64 << 9), 0, "SIGKILL should not be maskable");
        assert_eq!(mask & (1u64 << 19), 0, "SIGSTOP should not be maskable");
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
        // Pending signals should be cleared
        assert!(!proc.is_signal_pending(5));
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
        assert_eq!(proc.parent, Some(ProcessId(1)));
    }

    #[test]
    fn test_process_no_parent() {
        let proc = make_process(1, "init");
        assert_eq!(proc.parent, None);
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
    uid: u32,
    gid: u32,
}

#[cfg(feature = "alloc")]
impl ProcessBuilder {
    /// Create a new process builder
    pub fn new(name: String) -> Self {
        Self {
            name,
            parent: None,
            priority: ProcessPriority::Normal,
            uid: 0,
            gid: 0,
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

    /// Set user ID
    pub fn uid(mut self, uid: u32) -> Self {
        self.uid = uid;
        self
    }

    /// Set group ID
    pub fn gid(mut self, gid: u32) -> Self {
        self.gid = gid;
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
        let mut process = Process::new(pid, self.parent, self.name, self.priority);
        process.uid = self.uid;
        process.gid = self.gid;
        process
    }

    /// Build the process with an initialized address space.
    ///
    /// Allocates a root page table frame and maps kernel regions into the
    /// new address space. This is the preferred method for creating
    /// standalone processes (not forked from an existing process).
    pub fn build_with_address_space(self) -> Result<Process, KernelError> {
        let pid = super::alloc_pid();
        let mut process = Process::new(pid, self.parent, self.name, self.priority);
        process.uid = self.uid;
        process.gid = self.gid;

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

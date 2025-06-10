//! Process Control Block (PCB) implementation
//!
//! The PCB is the core data structure representing a process in the kernel.
//! It contains all the information needed to manage a process.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{
    collections::BTreeMap,
    string::String,
    vec::Vec,
};

use spin::Mutex;

use crate::{
    cap::{CapabilitySpace, CapabilityId},
    ipc::EndpointId,
    mm::VirtualAddressSpace,
};

use super::{
    thread::{Thread, ThreadId},
    ProcessState,
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
    pub priority: ProcessPriority,
    
    /// Virtual address space
    pub memory_space: Mutex<VirtualAddressSpace>,
    
    /// Capability space
    pub capability_space: Mutex<CapabilitySpace>,
    
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
            priority,
            memory_space: Mutex::new(VirtualAddressSpace::new()),
            capability_space: Mutex::new(CapabilitySpace::new()),
            threads: Mutex::new(BTreeMap::new()),
            ipc_endpoints: Mutex::new(BTreeMap::new()),
            children: Mutex::new(Vec::new()),
            exit_code: AtomicU32::new(0),
            cpu_time: AtomicU64::new(0),
            memory_stats: MemoryStats::default(),
            created_at: crate::arch::time::get_ticks(),
            uid: 0,
            gid: 0,
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
    
    /// Add a thread to this process
    #[cfg(feature = "alloc")]
    pub fn add_thread(&self, thread: Thread) -> Result<(), &'static str> {
        let tid = thread.tid;
        let mut threads = self.threads.lock();
        
        if threads.len() >= super::MAX_THREADS_PER_PROCESS {
            return Err("Too many threads in process");
        }
        
        if threads.contains_key(&tid) {
            return Err("Thread ID already exists");
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
        match self.get_state() {
            ProcessState::Dead | ProcessState::Zombie => false,
            _ => true,
        }
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
}

impl Drop for Process {
    fn drop(&mut self) {
        println!("[PROCESS] Dropping process {}", self.pid.0);
        // Cleanup will be handled by the process lifecycle manager
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
    
    /// Build the process
    pub fn build(self) -> Process {
        let pid = super::alloc_pid();
        let mut process = Process::new(pid, self.parent, self.name, self.priority);
        process.uid = self.uid;
        process.gid = self.gid;
        process
    }
}
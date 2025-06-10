//! Thread management implementation
//!
//! Threads are the unit of execution within a process. Each thread has its own
//! stack and CPU context but shares memory and other resources with its process.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::string::String;

use spin::Mutex;

use super::ProcessId;
use crate::arch::context::ThreadContext;

/// Thread ID type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u64);

impl core::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Thread state
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread is being created
    Creating = 0,
    /// Thread is ready to run
    Ready = 1,
    /// Thread is currently running
    Running = 2,
    /// Thread is blocked waiting
    Blocked = 3,
    /// Thread is sleeping
    Sleeping = 4,
    /// Thread has exited
    Exited = 5,
}

/// Thread Local Storage (TLS) data
pub struct ThreadLocalStorage {
    /// TLS base address
    pub base: usize,
    /// TLS size in bytes
    pub size: usize,
    /// TLS data pointer (architecture-specific)
    pub data_ptr: usize,
}

impl ThreadLocalStorage {
    /// Create new TLS area
    pub fn new() -> Self {
        Self {
            base: 0,
            size: 0,
            data_ptr: 0,
        }
    }
    
    /// Allocate TLS area
    pub fn allocate(&mut self, size: usize) -> Result<(), &'static str> {
        // TODO: Allocate memory for TLS
        self.size = size;
        Ok(())
    }
}

/// Thread control block
pub struct Thread {
    /// Thread ID
    pub tid: ThreadId,
    
    /// Parent process ID
    pub process: ProcessId,
    
    /// Thread name
    #[cfg(feature = "alloc")]
    pub name: String,
    
    /// Thread state
    pub state: AtomicU32,
    
    /// CPU context (registers, etc.)
    pub context: Mutex<ThreadContext>,
    
    /// User stack
    pub user_stack: Stack,
    
    /// Kernel stack
    pub kernel_stack: Stack,
    
    /// Thread-local storage
    pub tls: Mutex<ThreadLocalStorage>,
    
    /// CPU affinity mask
    pub cpu_affinity: AtomicUsize,
    
    /// Current CPU (if running)
    pub current_cpu: AtomicU32,
    
    /// Time slice remaining
    pub time_slice: AtomicU32,
    
    /// Total CPU time used (microseconds)
    pub cpu_time: AtomicU64,
    
    /// Wake up time (for sleeping threads)
    pub wake_time: AtomicU64,
    
    /// Exit code
    pub exit_code: AtomicU32,
    
    /// Thread priority (inherited from process)
    pub priority: u8,
    
    /// Floating point state saved flag
    pub fpu_used: AtomicU32,
}

/// Stack information
#[derive(Debug)]
pub struct Stack {
    /// Base address (lowest address)
    pub base: usize,
    /// Size in bytes
    pub size: usize,
    /// Current stack pointer
    pub sp: AtomicUsize,
}

impl Stack {
    /// Create a new stack
    pub fn new(base: usize, size: usize) -> Self {
        Self {
            base,
            size,
            sp: AtomicUsize::new(base + size), // Stack grows down
        }
    }
    
    /// Get stack top (initial SP)
    pub fn top(&self) -> usize {
        self.base + self.size
    }
    
    /// Check if address is within stack
    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.base + self.size
    }
    
    /// Get current stack pointer
    pub fn get_sp(&self) -> usize {
        self.sp.load(Ordering::Acquire)
    }
    
    /// Set stack pointer
    pub fn set_sp(&self, sp: usize) {
        self.sp.store(sp, Ordering::Release);
    }
}

impl Thread {
    /// Create a new thread
    #[cfg(feature = "alloc")]
    pub fn new(
        tid: ThreadId,
        process: ProcessId,
        name: String,
        entry_point: usize,
        user_stack_base: usize,
        user_stack_size: usize,
        kernel_stack_base: usize,
        kernel_stack_size: usize,
    ) -> Self {
        let mut thread = Self {
            tid,
            process,
            name,
            state: AtomicU32::new(ThreadState::Creating as u32),
            context: Mutex::new(ThreadContext::new()),
            user_stack: Stack::new(user_stack_base, user_stack_size),
            kernel_stack: Stack::new(kernel_stack_base, kernel_stack_size),
            tls: Mutex::new(ThreadLocalStorage::new()),
            cpu_affinity: AtomicUsize::new(usize::MAX), // All CPUs
            current_cpu: AtomicU32::new(u32::MAX),
            time_slice: AtomicU32::new(10), // Default time slice
            cpu_time: AtomicU64::new(0),
            wake_time: AtomicU64::new(0),
            exit_code: AtomicU32::new(0),
            priority: 2, // Normal priority
            fpu_used: AtomicU32::new(0),
        };
        
        // Initialize context with entry point and stacks
        {
            let mut ctx = thread.context.lock();
            ctx.set_instruction_pointer(entry_point);
            ctx.set_stack_pointer(thread.user_stack.top());
            ctx.set_kernel_stack(thread.kernel_stack.top());
        }
        
        thread
    }
    
    /// Get thread state
    pub fn get_state(&self) -> ThreadState {
        match self.state.load(Ordering::Acquire) {
            0 => ThreadState::Creating,
            1 => ThreadState::Ready,
            2 => ThreadState::Running,
            3 => ThreadState::Blocked,
            4 => ThreadState::Sleeping,
            5 => ThreadState::Exited,
            _ => ThreadState::Exited,
        }
    }
    
    /// Set thread state
    pub fn set_state(&self, state: ThreadState) {
        self.state.store(state as u32, Ordering::Release);
    }
    
    /// Check if thread is runnable
    pub fn is_runnable(&self) -> bool {
        matches!(self.get_state(), ThreadState::Ready | ThreadState::Running)
    }
    
    /// Set CPU affinity
    pub fn set_affinity(&self, mask: usize) {
        self.cpu_affinity.store(mask, Ordering::Release);
    }
    
    /// Get CPU affinity
    pub fn get_affinity(&self) -> usize {
        self.cpu_affinity.load(Ordering::Acquire)
    }
    
    /// Check if thread can run on CPU
    pub fn can_run_on_cpu(&self, cpu: u8) -> bool {
        let mask = self.get_affinity();
        (mask & (1 << cpu)) != 0
    }
    
    /// Mark thread as using FPU
    pub fn mark_fpu_used(&self) {
        self.fpu_used.store(1, Ordering::Release);
    }
    
    /// Check if thread uses FPU
    pub fn uses_fpu(&self) -> bool {
        self.fpu_used.load(Ordering::Acquire) != 0
    }
    
    /// Sleep thread until specified time
    pub fn sleep_until(&self, wake_time: u64) {
        self.wake_time.store(wake_time, Ordering::Release);
        self.set_state(ThreadState::Sleeping);
    }
    
    /// Wake up thread if it's time
    pub fn check_wake(&self, current_time: u64) -> bool {
        if self.get_state() == ThreadState::Sleeping {
            let wake_time = self.wake_time.load(Ordering::Acquire);
            if current_time >= wake_time {
                self.set_state(ThreadState::Ready);
                return true;
            }
        }
        false
    }
    
    /// Update CPU time
    pub fn add_cpu_time(&self, microseconds: u64) {
        self.cpu_time.fetch_add(microseconds, Ordering::Relaxed);
    }
    
    /// Get total CPU time
    pub fn get_cpu_time(&self) -> u64 {
        self.cpu_time.load(Ordering::Relaxed)
    }
}

/// Thread builder for convenient thread creation
#[cfg(feature = "alloc")]
pub struct ThreadBuilder {
    process: ProcessId,
    name: String,
    entry_point: usize,
    user_stack_size: usize,
    kernel_stack_size: usize,
    priority: u8,
    cpu_affinity: usize,
}

#[cfg(feature = "alloc")]
impl ThreadBuilder {
    /// Create a new thread builder
    pub fn new(process: ProcessId, name: String, entry_point: usize) -> Self {
        Self {
            process,
            name,
            entry_point,
            user_stack_size: 1024 * 1024, // 1MB default
            kernel_stack_size: 64 * 1024,  // 64KB default
            priority: 2,
            cpu_affinity: usize::MAX,
        }
    }
    
    /// Set user stack size
    pub fn user_stack_size(mut self, size: usize) -> Self {
        self.user_stack_size = size;
        self
    }
    
    /// Set kernel stack size
    pub fn kernel_stack_size(mut self, size: usize) -> Self {
        self.kernel_stack_size = size;
        self
    }
    
    /// Set priority
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
    
    /// Set CPU affinity
    pub fn cpu_affinity(mut self, mask: usize) -> Self {
        self.cpu_affinity = mask;
        self
    }
    
    /// Build the thread
    pub fn build(self) -> Result<Thread, &'static str> {
        // TODO: Allocate stacks from memory manager
        let user_stack_base = 0x1000_0000; // Placeholder
        let kernel_stack_base = 0x2000_0000; // Placeholder
        
        let tid = super::alloc_tid();
        let mut thread = Thread::new(
            tid,
            self.process,
            self.name,
            self.entry_point,
            user_stack_base,
            self.user_stack_size,
            kernel_stack_base,
            self.kernel_stack_size,
        );
        
        thread.priority = self.priority;
        thread.set_affinity(self.cpu_affinity);
        
        Ok(thread)
    }
}
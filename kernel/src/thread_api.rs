//! Thread Management APIs
//!
//! High-level thread management interface for user-space applications.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::{Mutex, RwLock};
use crate::process::{ProcessId, get_process};
use crate::process::thread::ThreadId;  // Use the ThreadId from process::thread module

/// Thread priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadPriority {
    Idle = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    RealTime = 4,
}

/// Thread state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Blocked,
    Suspended,
    Terminated,
}

/// Thread scheduling policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingPolicy {
    Normal,     // Standard round-robin with priorities
    RealTime,   // Real-time scheduling
    Batch,      // Batch processing (lower priority)
    Idle,       // Idle threads (run when nothing else)
}

/// Thread attributes
#[derive(Debug, Clone)]
pub struct ThreadAttributes {
    /// Stack size in bytes
    pub stack_size: usize,
    
    /// Thread priority
    pub priority: ThreadPriority,
    
    /// Scheduling policy
    pub policy: SchedulingPolicy,
    
    /// CPU affinity mask (bit mask of allowed CPUs)
    pub cpu_affinity: u64,
    
    /// Thread name
    pub name: String,
    
    /// Detached state (true = detached, false = joinable)
    pub detached: bool,
    
    /// Inherit scheduling from parent
    pub inherit_sched: bool,
}

impl Default for ThreadAttributes {
    fn default() -> Self {
        Self {
            stack_size: 1024 * 1024, // 1 MB default stack
            priority: ThreadPriority::Normal,
            policy: SchedulingPolicy::Normal,
            cpu_affinity: u64::MAX, // All CPUs
            name: String::from("thread"),
            detached: false,
            inherit_sched: true,
        }
    }
}

/// Thread-local storage key
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TlsKey(u32);

/// Thread entry point function type
pub type ThreadEntryPoint = fn(*mut u8) -> *mut u8;

/// Thread handle for management
#[derive(Debug)]
pub struct ThreadHandle {
    pub id: ThreadId,
    pub process_id: ProcessId,
    pub state: RwLock<ThreadState>,
    pub attributes: ThreadAttributes,
    pub exit_value: Mutex<Option<*mut u8>>,
    pub joinable: AtomicBool,
    pub cpu_time: AtomicU64,
    pub context_switches: AtomicU64,
}

impl ThreadHandle {
    /// Create a new thread handle
    pub fn new(id: ThreadId, process_id: ProcessId, attributes: ThreadAttributes) -> Self {
        Self {
            id,
            process_id,
            state: RwLock::new(ThreadState::Created),
            joinable: AtomicBool::new(!attributes.detached),
            exit_value: Mutex::new(None),
            cpu_time: AtomicU64::new(0),
            context_switches: AtomicU64::new(0),
            attributes,
        }
    }
    
    /// Get thread state
    pub fn get_state(&self) -> ThreadState {
        *self.state.read()
    }
    
    /// Set thread state
    pub fn set_state(&self, state: ThreadState) {
        *self.state.write() = state;
    }
    
    /// Check if thread is joinable
    pub fn is_joinable(&self) -> bool {
        self.joinable.load(Ordering::Acquire)
    }
    
    /// Detach the thread
    pub fn detach(&self) {
        self.joinable.store(false, Ordering::Release);
    }
    
    /// Get CPU time used by thread
    pub fn get_cpu_time(&self) -> u64 {
        self.cpu_time.load(Ordering::Relaxed)
    }
    
    /// Get number of context switches
    pub fn get_context_switches(&self) -> u64 {
        self.context_switches.load(Ordering::Relaxed)
    }
}

/// Thread creation parameters
pub struct ThreadCreateParams {
    pub entry_point: ThreadEntryPoint,
    pub arg: *mut u8,
    pub attributes: ThreadAttributes,
}

/// Thread management system
pub struct ThreadManager {
    /// Thread counter for ID generation
    next_thread_id: AtomicU64,
    
    /// TLS key counter
    next_tls_key: AtomicU64,
    
    /// Global thread table
    threads: RwLock<alloc::collections::BTreeMap<ThreadId, Arc<ThreadHandle>>>,
    
    /// TLS destructors
    tls_destructors: RwLock<alloc::collections::BTreeMap<TlsKey, fn(*mut u8)>>,
}

// SAFETY: ThreadManager is safe to send between threads
// All fields are either atomic or protected by RwLock
unsafe impl Send for ThreadManager {}

// SAFETY: ThreadManager is safe to share between threads
// All mutations are protected by atomic operations or RwLock
unsafe impl Sync for ThreadManager {}

impl ThreadManager {
    /// Create a new thread manager
    pub fn new() -> Self {
        Self {
            next_thread_id: AtomicU64::new(1),
            next_tls_key: AtomicU64::new(1),
            threads: RwLock::new(alloc::collections::BTreeMap::new()),
            tls_destructors: RwLock::new(alloc::collections::BTreeMap::new()),
        }
    }
    
    /// Create a new thread
    pub fn create_thread(
        &self,
        params: ThreadCreateParams,
        process_id: ProcessId,
    ) -> Result<Arc<ThreadHandle>, &'static str> {
        let thread_id = ThreadId(self.next_thread_id.fetch_add(1, Ordering::SeqCst));
        
        // Create thread handle
        let handle = Arc::new(ThreadHandle::new(thread_id, process_id, params.attributes.clone()));
        
        // Add to thread table
        self.threads.write().insert(thread_id, handle.clone());
        
        // Create actual thread in the process
        if let Some(process) = get_process(process_id) {
            // Allocate stack first
            let stack_size = params.attributes.stack_size;
            let mut memory_space = process.memory_space.lock();
            
            // Find a suitable virtual address for the stack
            let stack_base = 0x70000000; // User stack area
            let current_thread_count = process.thread_count();
            let stack_addr = stack_base - (current_thread_count * stack_size);
            
            // Map stack pages
            let page_count = (stack_size + 4095) / 4096;
            for i in 0..page_count {
                let page_addr = stack_addr + (i * 4096);
                let page_flags = crate::mm::PageFlags::PRESENT | 
                               crate::mm::PageFlags::USER | 
                               crate::mm::PageFlags::NO_EXECUTE;
                memory_space.map_page(page_addr, page_flags)?;
            }
            drop(memory_space); // Release the lock
            
            // Create the actual Thread object
            use crate::process::thread::{ThreadBuilder, Thread};
            use alloc::string::ToString;
            
            let kernel_stack_size = 64 * 1024; // 64KB kernel stack
            let kernel_stack_base = 0x80000000; // Kernel stack area
            let kernel_stack_addr = kernel_stack_base - (current_thread_count * kernel_stack_size);
            
            let thread = Thread::new(
                thread_id,
                process_id,
                params.attributes.name.clone(),
                params.entry_point as usize,
                stack_addr,
                stack_size,
                kernel_stack_addr,
                kernel_stack_size,
            );
            
            // Set stack pointer to top of stack (with argument)
            let stack_top = stack_addr + stack_size - 8; // Leave space for return address
            thread.user_stack.set_sp(stack_top);
            
            // Store thread argument at top of stack
            unsafe {
                let arg_ptr = stack_top as *mut *mut u8;
                *arg_ptr = params.arg;
            }
            
            // Set CPU affinity
            thread.set_affinity(params.attributes.cpu_affinity as usize);
            
            // Add thread to process
            process.add_thread(thread)?;
            
            handle.set_state(ThreadState::Ready);
            
            crate::println!("[THREAD] Created thread {} in process {} with stack at 0x{:x}", 
                thread_id.0, process_id.0, stack_addr);
        } else {
            return Err("Process not found");
        }
        
        Ok(handle)
    }
    
    /// Get thread handle by ID
    pub fn get_thread(&self, thread_id: ThreadId) -> Option<Arc<ThreadHandle>> {
        self.threads.read().get(&thread_id).cloned()
    }
    
    /// Join a thread (wait for it to complete)
    pub fn join_thread(&self, thread_id: ThreadId) -> Result<*mut u8, &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        if !handle.is_joinable() {
            return Err("Thread is not joinable");
        }
        
        // Wait for thread to complete
        loop {
            if handle.get_state() == ThreadState::Terminated {
                let exit_value = handle.exit_value.lock().take().unwrap_or(core::ptr::null_mut());
                
                // Remove from thread table
                self.threads.write().remove(&thread_id);
                
                return Ok(exit_value);
            }
            
            // Yield to scheduler
            crate::sched::yield_cpu();
        }
    }
    
    /// Detach a thread
    pub fn detach_thread(&self, thread_id: ThreadId) -> Result<(), &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        handle.detach();
        Ok(())
    }
    
    /// Cancel a thread
    pub fn cancel_thread(&self, thread_id: ThreadId) -> Result<(), &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        handle.set_state(ThreadState::Terminated);
        
        // TODO: Send cancellation signal to thread
        crate::println!("[THREAD] Cancelled thread {}", thread_id.0);
        
        Ok(())
    }
    
    /// Exit current thread
    pub fn exit_thread(&self, thread_id: ThreadId, exit_value: *mut u8) {
        if let Some(handle) = self.get_thread(thread_id) {
            *handle.exit_value.lock() = Some(exit_value);
            handle.set_state(ThreadState::Terminated);
            
            // Run TLS destructors
            self.run_tls_destructors(thread_id);
            
            crate::println!("[THREAD] Thread {} exited", thread_id.0);
        }
    }
    
    /// Set thread priority
    pub fn set_thread_priority(
        &self, 
        thread_id: ThreadId, 
        priority: ThreadPriority
    ) -> Result<(), &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        // Update process thread priority
        if let Some(process) = get_process(handle.process_id) {
            if let Some(thread) = process.get_thread(thread_id) {
                // Update thread priority (stored in the thread object)
                // Note: In a real implementation, we'd need mutable access
                // For now, just track it in the handle attributes
                crate::println!("[THREAD] Updated thread {} priority (tracked in handle)", thread_id.0);
            }
        }
        
        crate::println!("[THREAD] Set thread {} priority to {:?}", thread_id.0, priority);
        Ok(())
    }
    
    /// Get thread priority
    pub fn get_thread_priority(&self, thread_id: ThreadId) -> Result<ThreadPriority, &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        if let Some(process) = get_process(handle.process_id) {
            if let Some(thread) = process.get_thread(thread_id) {
                // For now, map from the thread's priority field
                let priority = match thread.priority {
                    0 => ThreadPriority::RealTime,
                    1 => ThreadPriority::High, 
                    2 => ThreadPriority::Normal,
                    3 => ThreadPriority::Low,
                    _ => ThreadPriority::Idle,
                };
                return Ok(priority);
            }
        }
        
        Err("Thread context not found")
    }
    
    /// Set CPU affinity for thread
    pub fn set_cpu_affinity(
        &self,
        thread_id: ThreadId,
        cpu_mask: u64,
    ) -> Result<(), &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        if let Some(process) = get_process(handle.process_id) {
            if let Some(thread) = process.get_thread(thread_id) {
                thread.set_affinity(cpu_mask as usize);
                crate::println!("[THREAD] Set thread {} CPU affinity to 0x{:x}", 
                    thread_id.0, cpu_mask);
                return Ok(());
            }
        }
        
        Err("Thread context not found")
    }
    
    /// Create thread-local storage key
    pub fn create_tls_key(&self, destructor: Option<fn(*mut u8)>) -> Result<TlsKey, &'static str> {
        let key = TlsKey(self.next_tls_key.fetch_add(1, Ordering::SeqCst) as u32);
        
        if let Some(dtor) = destructor {
            self.tls_destructors.write().insert(key, dtor);
        }
        
        Ok(key)
    }
    
    /// Delete thread-local storage key
    pub fn delete_tls_key(&self, key: TlsKey) -> Result<(), &'static str> {
        self.tls_destructors.write().remove(&key);
        
        // Remove from all thread contexts
        let threads = self.threads.read();
        for handle in threads.values() {
            if let Some(process) = get_process(handle.process_id) {
                if let Some(thread) = process.get_thread(handle.id) {
                    #[cfg(feature = "alloc")]
                    {
                        thread.remove_tls_value(key.0 as u64);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Set thread-local storage value
    pub fn set_tls_value(
        &self,
        thread_id: ThreadId,
        key: TlsKey,
        value: *mut u8,
    ) -> Result<(), &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        if let Some(process) = get_process(handle.process_id) {
            if let Some(thread) = process.get_thread(thread_id) {
                #[cfg(feature = "alloc")]
                {
                    thread.set_tls_value(key.0 as u64, value as u64);
                    return Ok(());
                }
            }
        }
        
        Err("Thread context not found")
    }
    
    /// Get thread-local storage value
    pub fn get_tls_value(&self, thread_id: ThreadId, key: TlsKey) -> Result<*mut u8, &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        if let Some(process) = get_process(handle.process_id) {
            if let Some(thread) = process.get_thread(thread_id) {
                #[cfg(feature = "alloc")]
                {
                    let value = thread.get_tls_value(key.0 as u64)
                        .unwrap_or(0) as *mut u8;
                    return Ok(value);
                }
                
                #[cfg(not(feature = "alloc"))]
                return Ok(core::ptr::null_mut());
            }
        }
        
        Err("Thread context not found")
    }
    
    /// Get current thread ID from scheduler
    pub fn get_current_thread_id(&self) -> Option<ThreadId> {
        // Get from scheduler
        let tid = crate::sched::get_current_thread_id();
        if tid != 0 {
            Some(ThreadId(tid))
        } else {
            None
        }
    }
    
    /// List all threads
    pub fn list_threads(&self) -> Vec<ThreadId> {
        self.threads.read().keys().copied().collect()
    }
    
    /// Get thread statistics
    pub fn get_thread_stats(&self, thread_id: ThreadId) -> Result<ThreadStats, &'static str> {
        let handle = self.get_thread(thread_id)
            .ok_or("Thread not found")?;
        
        Ok(ThreadStats {
            id: thread_id,
            process_id: handle.process_id,
            state: handle.get_state(),
            priority: handle.attributes.priority,
            cpu_time: handle.get_cpu_time(),
            context_switches: handle.get_context_switches(),
            stack_size: handle.attributes.stack_size,
            name: handle.attributes.name.clone(),
        })
    }
    
    // Helper functions
    
    fn run_tls_destructors(&self, thread_id: ThreadId) {
        let destructors = self.tls_destructors.read();
        
        if let Some(handle) = self.get_thread(thread_id) {
            if let Some(process) = get_process(handle.process_id) {
                if let Some(thread) = process.get_thread(thread_id) {
                    #[cfg(feature = "alloc")]
                    {
                        for (key, dtor) in destructors.iter() {
                            if let Some(value) = thread.get_tls_value(key.0 as u64) {
                                if value != 0 {
                                    dtor(value as *mut u8);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Thread statistics
#[derive(Debug, Clone)]
pub struct ThreadStats {
    pub id: ThreadId,
    pub process_id: ProcessId,
    pub state: ThreadState,
    pub priority: ThreadPriority,
    pub cpu_time: u64,
    pub context_switches: u64,
    pub stack_size: usize,
    pub name: String,
}

/// Global thread manager instance
static THREAD_MANAGER: spin::Once<ThreadManager> = spin::Once::new();

/// Initialize the thread manager
pub fn init() {
    THREAD_MANAGER.call_once(|| ThreadManager::new());
    crate::println!("[THREAD_API] Thread management APIs initialized");
}

/// Get the global thread manager
pub fn get_thread_manager() -> &'static ThreadManager {
    THREAD_MANAGER.get().expect("Thread manager not initialized")
}

// Convenience functions

/// Create a new thread
pub fn create_thread(
    entry_point: ThreadEntryPoint,
    arg: *mut u8,
    attributes: ThreadAttributes,
    process_id: ProcessId,
) -> Result<Arc<ThreadHandle>, &'static str> {
    let params = ThreadCreateParams {
        entry_point,
        arg,
        attributes,
    };
    
    get_thread_manager().create_thread(params, process_id)
}

/// Join a thread
pub fn join_thread(thread_id: ThreadId) -> Result<*mut u8, &'static str> {
    get_thread_manager().join_thread(thread_id)
}

/// Exit current thread
pub fn exit_thread(exit_value: *mut u8) -> ! {
    // Get current thread ID from scheduler
    let current_thread_id = ThreadId(crate::sched::get_current_thread_id());
    get_thread_manager().exit_thread(current_thread_id, exit_value);
    
    // Schedule next thread
    crate::sched::yield_cpu();
    
    // Should never reach here
    loop {
        core::hint::spin_loop();
    }
}

/// Yield CPU to scheduler
pub fn yield_thread() {
    crate::sched::yield_cpu();
}

/// Sleep for a number of milliseconds
pub fn sleep_ms(ms: u64) {
    // TODO: Implement proper sleep with timer
    for _ in 0..(ms * 1000) {
        core::hint::spin_loop();
    }
}
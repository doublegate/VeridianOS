//! Process Server for VeridianOS
//!
//! Manages process lifecycle, resource limits, and process enumeration.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use libveridian::{println, print, sys};

/// Process information
#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u64,
    parent_pid: Option<u64>,
    name: String,
    state: ProcessState,
    uid: u32,
    gid: u32,
    memory_usage: usize,
    cpu_time: u64,
    children: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProcessState {
    Running,
    Sleeping,
    Waiting,
    Zombie,
    Stopped,
}

/// Resource limits for a process
#[derive(Debug, Clone)]
struct ResourceLimits {
    max_memory: usize,
    max_cpu_time: u64,
    max_file_descriptors: usize,
    max_threads: usize,
    max_processes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory: 64 * 1024 * 1024,       // 64MB
            max_cpu_time: u64::MAX,             // Unlimited
            max_file_descriptors: 1024,         // 1024 FDs
            max_threads: 256,                   // 256 threads
            max_processes: 1024,                // 1024 child processes
        }
    }
}

/// Process server state
struct ProcessServer {
    processes: BTreeMap<u64, ProcessInfo>,
    resource_limits: BTreeMap<u64, ResourceLimits>,
    next_pid: AtomicU64,
}

impl ProcessServer {
    /// Create a new process server
    fn new() -> Self {
        let mut server = Self {
            processes: BTreeMap::new(),
            resource_limits: BTreeMap::new(),
            next_pid: AtomicU64::new(1000), // User PIDs start at 1000
        };
        
        // Register init process
        server.register_process(1, None, String::from("init"), 0, 0);
        
        server
    }
    
    /// Register a new process
    fn register_process(
        &mut self,
        pid: u64,
        parent_pid: Option<u64>,
        name: String,
        uid: u32,
        gid: u32,
    ) -> u64 {
        let info = ProcessInfo {
            pid,
            parent_pid,
            name,
            state: ProcessState::Running,
            uid,
            gid,
            memory_usage: 0,
            cpu_time: 0,
            children: Vec::new(),
        };
        
        // Update parent's children list
        if let Some(ppid) = parent_pid {
            if let Some(parent) = self.processes.get_mut(&ppid) {
                parent.children.push(pid);
            }
        }
        
        self.processes.insert(pid, info);
        self.resource_limits.insert(pid, ResourceLimits::default());
        
        pid
    }
    
    /// Create a new process
    fn create_process(
        &mut self,
        parent_pid: u64,
        name: String,
        uid: u32,
        gid: u32,
    ) -> Result<u64, &'static str> {
        // Check if parent exists
        if !self.processes.contains_key(&parent_pid) {
            return Err("Parent process not found");
        }
        
        // Check resource limits
        if let Some(parent) = self.processes.get(&parent_pid) {
            if let Some(limits) = self.resource_limits.get(&parent_pid) {
                if parent.children.len() >= limits.max_processes {
                    return Err("Process limit exceeded");
                }
            }
        }
        
        // Allocate new PID
        let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
        
        // Register the process
        self.register_process(pid, Some(parent_pid), name, uid, gid);
        
        Ok(pid)
    }
    
    /// Terminate a process
    fn terminate_process(&mut self, pid: u64) -> Result<(), &'static str> {
        // Check if process exists
        let process = self.processes.get(&pid)
            .ok_or("Process not found")?;
        
        // Can't terminate init
        if pid == 1 {
            return Err("Cannot terminate init process");
        }
        
        // Reparent children to init
        let children = process.children.clone();
        for child_pid in children {
            if let Some(child) = self.processes.get_mut(&child_pid) {
                child.parent_pid = Some(1);
                if let Some(init) = self.processes.get_mut(&1) {
                    init.children.push(child_pid);
                }
            }
        }
        
        // Mark as zombie until parent waits
        if let Some(process) = self.processes.get_mut(&pid) {
            process.state = ProcessState::Zombie;
        }
        
        Ok(())
    }
    
    /// Enumerate processes
    fn enumerate_processes(&self) -> Vec<ProcessInfo> {
        self.processes.values().cloned().collect()
    }
    
    /// Get process information
    fn get_process_info(&self, pid: u64) -> Option<ProcessInfo> {
        self.processes.get(&pid).cloned()
    }
    
    /// Set resource limits
    fn set_resource_limits(&mut self, pid: u64, limits: ResourceLimits) -> Result<(), &'static str> {
        if !self.processes.contains_key(&pid) {
            return Err("Process not found");
        }
        
        self.resource_limits.insert(pid, limits);
        Ok(())
    }
    
    /// Get resource limits
    fn get_resource_limits(&self, pid: u64) -> Option<ResourceLimits> {
        self.resource_limits.get(&pid).cloned()
    }
    
    /// Update process state
    fn update_process_state(&mut self, pid: u64, state: ProcessState) -> Result<(), &'static str> {
        let process = self.processes.get_mut(&pid)
            .ok_or("Process not found")?;
        
        process.state = state;
        Ok(())
    }
    
    /// Wait for child process
    fn wait_for_child(&mut self, parent_pid: u64) -> Result<(u64, i32), &'static str> {
        let parent = self.processes.get(&parent_pid)
            .ok_or("Parent process not found")?;
        
        // Find a zombie child
        for &child_pid in &parent.children {
            if let Some(child) = self.processes.get(&child_pid) {
                if child.state == ProcessState::Zombie {
                    // Remove the zombie process
                    self.processes.remove(&child_pid);
                    self.resource_limits.remove(&child_pid);
                    
                    // Remove from parent's children list
                    if let Some(parent) = self.processes.get_mut(&parent_pid) {
                        parent.children.retain(|&pid| pid != child_pid);
                    }
                    
                    return Ok((child_pid, 0)); // Return exit status
                }
            }
        }
        
        Err("No zombie children")
    }
}

/// Process server entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    libveridian::init();
    main();
    sys::exit(0);
}

fn main() {
    println!("[PROCESS-SERVER] VeridianOS Process Server v0.1.0");
    println!("[PROCESS-SERVER] Initializing process management...");
    
    let mut server = ProcessServer::new();
    
    println!("[PROCESS-SERVER] Process server initialized");
    println!("[PROCESS-SERVER] Registered processes:");
    
    for process in server.enumerate_processes() {
        println!("  PID {} - {}", process.pid, process.name);
    }
    
    // Main service loop
    println!("[PROCESS-SERVER] Entering service loop...");
    
    loop {
        // TODO: Wait for IPC messages and handle requests
        // For now, just sleep
        sys::sleep(1000).ok();
        
        // Periodically clean up zombie processes
        cleanup_zombies(&mut server);
    }
}

/// Clean up zombie processes that have been waiting too long
fn cleanup_zombies(server: &mut ProcessServer) {
    let zombies: Vec<u64> = server.processes
        .iter()
        .filter(|(_, p)| p.state == ProcessState::Zombie)
        .map(|(&pid, _)| pid)
        .collect();
    
    for pid in zombies {
        // If parent hasn't waited after some time, clean up
        println!("[PROCESS-SERVER] Cleaning up zombie process {}", pid);
        server.processes.remove(&pid);
        server.resource_limits.remove(&pid);
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[PROCESS-SERVER] PANIC: {}", info);
    sys::exit(255);
}

// Simple allocator for the process server
use core::alloc::{GlobalAlloc, Layout};

struct ProcessServerAllocator;

static mut HEAP: [u8; 131072] = [0; 131072]; // 128KB heap
static mut HEAP_POS: usize = 0;

unsafe impl GlobalAlloc for ProcessServerAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        
        // Align heap position
        let aligned_pos = (HEAP_POS + align - 1) & !(align - 1);
        
        // Check if we have enough space
        if aligned_pos + size > HEAP.len() {
            return core::ptr::null_mut();
        }
        
        // Allocate memory
        let ptr = HEAP.as_mut_ptr().add(aligned_pos);
        HEAP_POS = aligned_pos + size;
        
        ptr
    }
    
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Simple bump allocator - no deallocation
    }
}

#[global_allocator]
static ALLOCATOR: ProcessServerAllocator = ProcessServerAllocator;

#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    panic!("Process server allocation error");
}
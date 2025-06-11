//! Process lifecycle management
//!
//! This module handles process creation, termination, and state transitions.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::Ordering;

use super::{
    pcb::{Process, ProcessBuilder, ProcessState},
    table,
    thread::{Thread, ThreadBuilder, ThreadId},
    ProcessId, ProcessPriority,
};
use crate::{arch::context::ThreadContext, println, sched};

/// Default stack sizes
pub const DEFAULT_USER_STACK_SIZE: usize = 8 * 1024 * 1024; // 8MB
pub const DEFAULT_KERNEL_STACK_SIZE: usize = 64 * 1024; // 64KB

/// Process creation options
#[cfg(feature = "alloc")]
pub struct ProcessCreateOptions {
    pub name: String,
    pub parent: Option<ProcessId>,
    pub priority: ProcessPriority,
    pub entry_point: usize,
    pub argv: Vec<String>,
    pub envp: Vec<String>,
    pub user_stack_size: usize,
    pub kernel_stack_size: usize,
}

#[cfg(feature = "alloc")]
impl Default for ProcessCreateOptions {
    fn default() -> Self {
        Self {
            name: String::from("unnamed"),
            parent: None,
            priority: ProcessPriority::Normal,
            entry_point: 0,
            argv: Vec::new(),
            envp: Vec::new(),
            user_stack_size: DEFAULT_USER_STACK_SIZE,
            kernel_stack_size: DEFAULT_KERNEL_STACK_SIZE,
        }
    }
}

/// Create a new process
#[cfg(feature = "alloc")]
pub fn create_process(name: String, entry_point: usize) -> Result<ProcessId, &'static str> {
    let options = ProcessCreateOptions {
        name,
        entry_point,
        ..Default::default()
    };

    create_process_with_options(options)
}

/// Create a new process with options
#[cfg(feature = "alloc")]
pub fn create_process_with_options(
    options: ProcessCreateOptions,
) -> Result<ProcessId, &'static str> {
    // Create the process
    let process = ProcessBuilder::new(options.name.clone())
        .parent(options.parent.unwrap_or(ProcessId(0)))
        .priority(options.priority)
        .build();

    let pid = process.pid;

    // Set up the process's address space
    {
        let mut memory_space = process.memory_space.lock();
        memory_space.init()?;

        // Map kernel space (higher half)
        // This is typically shared among all processes
        memory_space.map_kernel_space()?;
    }

    // Create the main thread
    let main_thread =
        ThreadBuilder::new(pid, format!("{}-main", options.name), options.entry_point)
            .user_stack_size(options.user_stack_size)
            .kernel_stack_size(options.kernel_stack_size)
            .build()?;

    let tid = main_thread.tid;

    // Add thread to process
    process.add_thread(main_thread)?;

    // Add process to process table
    table::add_process(process)?;

    // Mark process as ready
    if let Some(process) = table::get_process(pid) {
        process.set_state(ProcessState::Ready);

        // Add main thread to scheduler
        if let Some(thread) = process.get_thread(tid) {
            // Create a scheduler task for this thread
            create_scheduler_task(process, thread)?;
        }
    }

    println!(
        "[PROCESS] Created process {} ({}) with main thread {}",
        pid.0, options.name, tid.0
    );

    Ok(pid)
}

/// Fork current process
#[cfg(feature = "alloc")]
pub fn fork_process() -> Result<ProcessId, &'static str> {
    let current_process = super::current_process().ok_or("No current process")?;

    let current_thread = super::current_thread().ok_or("No current thread")?;

    // Create new process as copy of current
    let new_process = ProcessBuilder::new(format!("{}-fork", current_process.name))
        .parent(current_process.pid)
        .priority(current_process.priority)
        .build();

    let new_pid = new_process.pid;

    // Clone address space
    {
        let current_space = current_process.memory_space.lock();
        let mut new_space = new_process.memory_space.lock();

        // TODO: Implement copy-on-write for efficiency
        new_space.clone_from(&current_space)?;
    }

    // Clone capabilities
    {
        let _current_caps = current_process.capability_space.lock();
        let _new_caps = new_process.capability_space.lock();

        // TODO: Implement capability space cloning
        // new_caps.clone_from(&*current_caps)?;
    }

    // Create thread in new process matching current thread
    let new_thread = {
        let ctx = current_thread.context.lock();
        let thread = ThreadBuilder::new(
            new_pid,
            current_thread.name.clone(),
            ctx.get_instruction_pointer(),
        )
        .user_stack_size(current_thread.user_stack.size)
        .kernel_stack_size(current_thread.kernel_stack.size)
        .priority(current_thread.priority)
        .cpu_affinity(current_thread.get_affinity())
        .build()?;

        // Copy thread context and set return value to 0 for child
        {
            let mut new_ctx = thread.context.lock();
            // Clone context manually
            *new_ctx = (*ctx).clone();

            // Set return value to 0 for child
            new_ctx.set_return_value(0);
        } // Drop lock here

        thread
    };

    let new_tid = new_thread.tid;
    new_process.add_thread(new_thread)?;

    // Add to parent's children list
    #[cfg(feature = "alloc")]
    {
        current_process.children.lock().push(new_pid);
    }

    // Add process to table
    table::add_process(new_process)?;

    // Mark as ready and add to scheduler
    if let Some(process) = table::get_process(new_pid) {
        process.set_state(ProcessState::Ready);

        if let Some(thread) = process.get_thread(new_tid) {
            create_scheduler_task(process, thread)?;
        }
    }

    println!(
        "[PROCESS] Forked process {} from {}",
        new_pid.0, current_process.pid.0
    );

    // Return child PID to parent
    Ok(new_pid)
}

/// Execute a new program in current process
pub fn exec_process(_path: &str, _argv: &[&str], _envp: &[&str]) -> Result<(), &'static str> {
    // TODO: Implement exec
    // This would:
    // 1. Load new program from filesystem
    // 2. Replace current address space
    // 3. Reset thread to new entry point
    // 4. Clear signals, close files, etc.

    Err("exec not yet implemented")
}

/// Exit current process
pub fn exit_process(exit_code: i32) {
    if let Some(process) = super::current_process() {
        println!(
            "[PROCESS] Process {} exiting with code {}",
            process.pid.0, exit_code
        );

        // Set exit code
        process.set_exit_code(exit_code);

        // Mark all threads as exited
        #[cfg(feature = "alloc")]
        {
            let threads = process.threads.lock();
            for (_, thread) in threads.iter() {
                thread.set_state(super::thread::ThreadState::Zombie);
            }
        }

        // Clean up resources
        cleanup_process(process);

        // Mark process as zombie (parent needs to reap)
        process.set_state(ProcessState::Zombie);

        // TODO: Wake up parent if waiting

        // Schedule another process
        sched::exit_task(exit_code);
    }
}

/// Wait for child process to exit
#[cfg(feature = "alloc")]
pub fn wait_process(pid: Option<ProcessId>) -> Result<(ProcessId, i32), &'static str> {
    let current = super::current_process().ok_or("No current process")?;

    loop {
        // Check for zombie children
        let children = table::PROCESS_TABLE.find_children(current.pid);

        for child_pid in children {
            if let Some(child) = table::get_process(child_pid) {
                if (pid.is_none() || pid == Some(child_pid))
                    && child.get_state() == ProcessState::Zombie
                {
                    // Reap the zombie
                    let exit_code = child.get_exit_code();

                    // Remove from children list
                    current.children.lock().retain(|&p| p != child_pid);

                    // Remove from process table
                    table::remove_process(child_pid);

                    return Ok((child_pid, exit_code));
                }
            }
        }

        // No zombie children found, block
        // TODO: Implement proper blocking wait
        sched::yield_cpu();
    }
}

/// Kill a process
pub fn kill_process(pid: ProcessId, _signal: i32) -> Result<(), &'static str> {
    let process = table::get_process(pid).ok_or("Process not found")?;

    if !process.is_alive() {
        return Err("Process already dead");
    }

    println!("[PROCESS] Killing process {}", pid.0);

    // TODO: Implement signal delivery
    // For now, just force exit

    // Mark all threads as exited
    #[cfg(feature = "alloc")]
    {
        let threads = process.threads.lock();
        for (_, thread) in threads.iter() {
            thread.set_state(super::thread::ThreadState::Zombie);

            // Remove from scheduler if scheduled
            if let Some(task_ptr) = thread.get_task_ptr() {
                unsafe {
                    let task = task_ptr.as_ptr();
                    (*task).state = ProcessState::Dead;
                    // Note: The scheduler will clean up dead tasks
                }
            }
        }
    }

    // Clean up and mark as zombie
    cleanup_process(process);
    process.set_state(ProcessState::Zombie);

    Ok(())
}

/// Clean up process resources
fn cleanup_process(process: &Process) {
    // Release memory
    {
        let memory_space = process.memory_space.lock();
        // TODO: Implement proper memory space cleanup
        // memory_space.destroy();
        drop(memory_space);
    }

    // Release capabilities
    {
        let cap_space = process.capability_space.lock();
        // TODO: Implement proper capability space cleanup
        // cap_space.destroy();
        drop(cap_space);
    }

    // Close IPC endpoints
    #[cfg(feature = "alloc")]
    {
        let endpoints = process.ipc_endpoints.lock();
        for (_endpoint_id, _) in endpoints.iter() {
            // TODO: Close endpoint
            println!("[PROCESS] Closing IPC endpoint {}", _endpoint_id);
        }
    }

    // TODO: Close files, release other resources
}

/// Clean up a dead thread
#[cfg(feature = "alloc")]
pub fn cleanup_thread(process: &Process, tid: ThreadId) -> Result<(), &'static str> {
    // Remove thread from process
    let mut threads = process.threads.lock();

    if let Some(thread) = threads.remove(&tid) {
        println!("[PROCESS] Cleaning up thread {}", tid.0);

        // Make sure thread is marked as dead
        thread.set_state(super::thread::ThreadState::Dead);

        // Clean up scheduler task if exists
        if let Some(task_ptr) = thread.get_task_ptr() {
            unsafe {
                let task = task_ptr.as_ptr();

                // Clear thread reference in task
                (*task).thread_ref = None;

                // Mark task for cleanup
                (*task).state = ProcessState::Dead;

                // The scheduler will eventually free the task memory
            }
        }

        // Free thread stacks
        // Free user stack
        if thread.user_stack.size > 0 {
            let stack_base = thread.user_stack.base;
            let stack_size = thread.user_stack.size;

            // TODO: Convert to physical address and free frames
            // This requires VMM integration which is not yet complete
            println!(
                "[PROCESS] TODO: Free user stack at {:#x}, size {} (deferred)",
                stack_base, stack_size
            );
        }

        // Free kernel stack
        if thread.kernel_stack.size > 0 {
            let stack_base = thread.kernel_stack.base;
            let stack_size = thread.kernel_stack.size;

            // TODO: Kernel stacks are in kernel space, directly free them
            // This requires frame allocator integration
            println!(
                "[PROCESS] TODO: Free kernel stack at {:#x}, size {} (deferred)",
                stack_base, stack_size
            );
        }

        // Clean up TLS area
        {
            let tls = thread.tls.lock();
            if tls.base != 0 && tls.size > 0 {
                // TODO: Free TLS memory
                // This requires VMM integration
                println!(
                    "[PROCESS] TODO: Free TLS area at {:#x}, size {} (deferred)",
                    tls.base, tls.size
                );
            }
        }

        Ok(())
    } else {
        Err("Thread not found")
    }
}

/// Reap zombie threads in a process
#[cfg(feature = "alloc")]
pub fn reap_zombie_threads(process: &Process) -> Vec<(ThreadId, i32)> {
    let mut reaped = Vec::new();
    let threads = process.threads.lock();

    // Find all zombie threads
    let zombies: Vec<ThreadId> = threads
        .iter()
        .filter(|(_, thread)| thread.get_state() == super::thread::ThreadState::Zombie)
        .map(|(tid, _)| *tid)
        .collect();

    drop(threads);

    // Clean up each zombie thread
    for tid in zombies {
        if let Ok(()) = cleanup_thread(process, tid) {
            // Get exit code before cleanup
            if let Some(thread) = process.get_thread(tid) {
                let exit_code = thread.exit_code.load(Ordering::Acquire) as i32;
                reaped.push((tid, exit_code));
            }
        }
    }

    reaped
}

/// Create scheduler task for thread
fn create_scheduler_task(process: &Process, thread: &Thread) -> Result<(), &'static str> {
    // Create a sched::Task from our Thread
    // Get thread context info
    let ctx = thread.context.lock();
    let instruction_pointer = ctx.get_instruction_pointer();
    let stack_pointer = ctx.get_stack_pointer();
    drop(ctx);

    // Create a sched::Task using the constructor
    #[cfg(feature = "alloc")]
    let mut task = sched::task::Task::new(
        process.pid,
        thread.tid,
        process.name.clone(),
        instruction_pointer,
        stack_pointer,
        process.memory_space.lock().get_page_table() as usize,
    );

    // Update task fields based on thread/process state
    task.priority = match process.priority {
        ProcessPriority::RealTime => sched::task::Priority::RealTimeHigh,
        ProcessPriority::System => sched::task::Priority::SystemHigh,
        ProcessPriority::Normal => sched::task::Priority::UserNormal,
        ProcessPriority::Low => sched::task::Priority::UserLow,
        ProcessPriority::Idle => sched::task::Priority::Idle,
    };

    task.sched_class = match process.priority {
        ProcessPriority::RealTime => sched::task::SchedClass::RealTime,
        _ => sched::task::SchedClass::Normal,
    };

    task.time_slice = thread
        .time_slice
        .load(core::sync::atomic::Ordering::Acquire);

    // Get task pointer
    let task_ptr = core::ptr::NonNull::new(&task as *const _ as *mut _)
        .ok_or("Failed to create task pointer")?;

    // Add to scheduler
    let scheduler = sched::SCHEDULER.lock();
    scheduler.enqueue(task_ptr);

    Ok(())
}

/// Process statistics
#[cfg(feature = "alloc")]
pub struct ProcessStats {
    pub total_processes: usize,
    pub running_processes: usize,
    pub blocked_processes: usize,
    pub zombie_processes: usize,
    pub total_threads: usize,
    pub total_cpu_time: u64,
    pub total_memory_usage: u64,
}

/// Get system-wide process statistics
#[cfg(feature = "alloc")]
pub fn get_process_stats() -> ProcessStats {
    let mut stats = ProcessStats {
        total_processes: 0,
        running_processes: 0,
        blocked_processes: 0,
        zombie_processes: 0,
        total_threads: 0,
        total_cpu_time: 0,
        total_memory_usage: 0,
    };

    table::PROCESS_TABLE.for_each(|process| {
        stats.total_processes += 1;
        stats.total_threads += process.thread_count();
        stats.total_cpu_time += process.get_cpu_time();
        stats.total_memory_usage += process
            .memory_stats
            .virtual_size
            .load(core::sync::atomic::Ordering::Relaxed);

        match process.get_state() {
            ProcessState::Running => stats.running_processes += 1,
            ProcessState::Blocked => stats.blocked_processes += 1,
            ProcessState::Zombie => stats.zombie_processes += 1,
            _ => {}
        }
    });

    stats
}

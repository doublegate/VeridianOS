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
#[allow(unused_imports)]
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
        // init() already maps kernel space, so we don't need to call map_kernel_space()
        // again
        memory_space.init()?;
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
        .priority(*current_process.priority.lock())
        .build();

    let new_pid = new_process.pid;

    // Clone address space
    {
        let current_space = current_process.memory_space.lock();
        let mut new_space = new_process.memory_space.lock();

        // Note: Currently using full copy instead of copy-on-write (CoW).
        // CoW optimization deferred to Phase 5 (Performance Optimization) as it
        // requires:
        // - Page table flags for CoW pages (read-only + CoW marker)
        // - Page fault handler integration for CoW page faults
        // - Reference counting for shared physical pages
        // - Memory zone integration for CoW tracking
        // The current implementation is correct, just less memory efficient.
        new_space.clone_from(&current_space)?;
    }

    // Clone capabilities
    {
        let current_caps = current_process.capability_space.lock();
        let new_caps = new_process.capability_space.lock();

        // Clone capability space so child has same capabilities as parent
        new_caps.clone_from(&current_caps)?;
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
///
/// Replaces the current process image with a new program.
/// This function does not return on success - the new program begins execution.
#[cfg(feature = "alloc")]
pub fn exec_process(path: &str, argv: &[&str], envp: &[&str]) -> Result<(), &'static str> {
    use crate::{elf::ElfLoader, fs};

    let process = super::current_process().ok_or("No current process")?;
    let current_thread = super::current_thread().ok_or("No current thread")?;

    println!(
        "[PROCESS] exec() called for process {} with path: {}",
        process.pid.0, path
    );

    // Step 1: Load new program from filesystem
    let file_data = fs::read_file(path).map_err(|_| "Failed to read executable file")?;

    // Step 2: Clear current address space and load new program
    let entry_point = {
        let mut memory_space = process.memory_space.lock();

        // Clear existing mappings before loading new program
        memory_space.clear();

        // Reinitialize the address space for the new program
        memory_space.init()?;

        // Load ELF segments into address space and get entry point
        ElfLoader::load(&file_data, &mut memory_space)?
    };

    // Step 3: Setup new stack with arguments and environment
    let stack_top = setup_exec_stack(process, argv, envp)?;

    // Step 4: Reset thread context to new entry point
    {
        let mut ctx = current_thread.context.lock();

        // Set new instruction pointer to program entry
        ctx.set_instruction_pointer(entry_point as usize);

        // Set stack pointer to new stack top
        ctx.set_stack_pointer(stack_top);

        // Clear return value (argc is passed differently)
        ctx.set_return_value(0);
    }

    // Step 5: Close file descriptors marked close-on-exec
    {
        let file_table = process.file_table.lock();
        file_table.close_on_exec();
    }

    // Step 6: Reset signal handlers to defaults
    process.reset_signal_handlers();

    // Step 7: Update process name to reflect new executable
    #[cfg(feature = "alloc")]
    {
        // Extract filename from path
        let _name = path.rsplit('/').next().unwrap_or(path);
        // Note: Can't directly modify process.name since it's behind shared ref
        // In a full impl, we'd need interior mutability here
        println!(
            "[PROCESS] Process {} now executing: {}",
            process.pid.0, _name
        );
    }

    println!(
        "[PROCESS] exec() completed for process {}, entry: {:#x}",
        process.pid.0, entry_point
    );

    // The actual execution resumes when we return to user mode
    // The modified thread context will cause execution at the new entry point
    Ok(())
}

#[cfg(not(feature = "alloc"))]
pub fn exec_process(_path: &str, _argv: &[&str], _envp: &[&str]) -> Result<(), &'static str> {
    Err("exec requires alloc feature")
}

/// Setup stack for exec with arguments and environment
#[cfg(feature = "alloc")]
fn setup_exec_stack(
    process: &Process,
    argv: &[&str],
    envp: &[&str],
) -> Result<usize, &'static str> {
    let memory_space = process.memory_space.lock();

    // Get stack region (typically at end of user address space)
    let stack_base = memory_space.user_stack_base();
    let stack_size = memory_space.user_stack_size();
    let stack_top = stack_base + stack_size;

    // Layout: [env strings] [arg strings] [env pointers] [arg pointers] [argc]
    // Stack grows downward, so we start from top

    let mut sp = stack_top;

    // Align stack to 16 bytes
    sp &= !0xF;

    // Reserve space for strings and pointers
    // Calculate total string size
    let argv_total: usize = argv.iter().map(|s| s.len() + 1).sum();
    let envp_total: usize = envp.iter().map(|s| s.len() + 1).sum();

    // Push null terminator for envp array
    sp -= core::mem::size_of::<usize>();

    // Push envp pointers (will be filled in)
    let envp_ptrs_start = sp - (envp.len() * core::mem::size_of::<usize>());
    sp = envp_ptrs_start;

    // Push null terminator for argv array
    sp -= core::mem::size_of::<usize>();

    // Push argv pointers (will be filled in)
    let argv_ptrs_start = sp - (argv.len() * core::mem::size_of::<usize>());
    sp = argv_ptrs_start;

    // Push argc
    sp -= core::mem::size_of::<usize>();

    // Reserve space for strings
    sp -= argv_total + envp_total;

    // Align final sp to 16 bytes
    sp &= !0xF;

    // Note: In a full implementation, we would actually copy the strings
    // and pointers to the stack. For now, we just set up the layout.

    // The actual argument passing will be handled by the C runtime (crt0)
    // which expects argc at sp, argv at sp+8, envp at sp+16 (for 64-bit)

    // Store argc at stack pointer
    let _argc = argv.len();
    // In real implementation: unsafe { *(sp as *mut usize) = argc; }

    println!(
        "[PROCESS] Stack setup: base={:#x}, top={:#x}, sp={:#x}, argc={}",
        stack_base, stack_top, sp, _argc
    );

    Ok(sp)
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

        // Wake up parent if waiting
        if let Some(parent_pid) = process.parent {
            if let Some(parent) = table::get_process(parent_pid) {
                let parent_state = parent.get_state();
                if parent_state == ProcessState::Blocked {
                    parent.set_state(ProcessState::Ready);
                    sched::wake_up_process(parent_pid);
                }
            }
        }

        // Schedule another process
        sched::exit_task(exit_code);
    }
}

/// Wait for child process to exit
#[cfg(feature = "alloc")]
pub fn wait_process(pid: Option<ProcessId>) -> Result<(ProcessId, i32), &'static str> {
    wait_process_with_options(pid, WaitOptions::default())
}

/// Wait options for wait_process_with_options
#[derive(Debug, Clone, Copy, Default)]
pub struct WaitOptions {
    /// Don't block if no child has exited (WNOHANG)
    pub no_hang: bool,
    /// Also return if a child has stopped (WUNTRACED)
    pub untraced: bool,
    /// Also return if a stopped child has been resumed (WCONTINUED)
    pub continued: bool,
}

impl WaitOptions {
    /// Non-blocking wait
    pub fn no_hang() -> Self {
        Self {
            no_hang: true,
            untraced: false,
            continued: false,
        }
    }
}

/// Wait for child process with options
#[cfg(feature = "alloc")]
pub fn wait_process_with_options(
    pid: Option<ProcessId>,
    options: WaitOptions,
) -> Result<(ProcessId, i32), &'static str> {
    let current = super::current_process().ok_or("No current process")?;
    let current_pid = current.pid;

    loop {
        // Check for zombie children
        let children = table::PROCESS_TABLE.find_children(current_pid);

        // No children at all
        if children.is_empty() {
            return Err("No child processes");
        }

        // Check if any matching child exists
        let mut matching_child_exists = false;

        for child_pid in &children {
            // Check if this child matches our pid filter
            let matches_filter = pid.is_none() || pid == Some(*child_pid);
            if !matches_filter {
                continue;
            }

            matching_child_exists = true;

            if let Some(child) = table::get_process(*child_pid) {
                let child_state = child.get_state();

                // Check for zombie (exited)
                if child_state == ProcessState::Zombie {
                    // Reap the zombie
                    let exit_code = child.get_exit_code();

                    // Remove from children list
                    current.children.lock().retain(|&p| p != *child_pid);

                    // Remove from process table
                    table::remove_process(*child_pid);

                    println!(
                        "[PROCESS] Process {} reaped child {} (exit code {})",
                        current_pid.0, child_pid.0, exit_code
                    );

                    return Ok((*child_pid, exit_code));
                }

                // Check for stopped child if WUNTRACED is set
                if options.untraced && child_state == ProcessState::Blocked {
                    // Return status indicating stopped (signal number in bits 8-15)
                    // Use 0x7f as the stopped indicator with SIGSTOP (19)
                    let status = 0x7f | (19 << 8);
                    return Ok((*child_pid, status));
                }

                // Check for continued child if WCONTINUED is set
                if options.continued && child_state == ProcessState::Running {
                    // Return status indicating continued
                    let status = 0xffff; // WIFCONTINUED indicator
                    return Ok((*child_pid, status));
                }
            }
        }

        // No matching child found
        if pid.is_some() && !matching_child_exists {
            return Err("No matching child process");
        }

        // No zombie children found
        if options.no_hang {
            // WNOHANG: return immediately with (0, 0) to indicate no child changed state
            return Ok((ProcessId(0), 0));
        }

        // Block current process until a child changes state
        // The child will wake us up when it exits (see exit_process)
        current.set_state(ProcessState::Blocked);

        // Register for child termination notification
        // This is done by setting up a wait queue entry
        println!(
            "[PROCESS] Process {} blocking in wait() for child {}",
            current_pid.0,
            pid.map_or(-1, |p| p.0 as i64)
        );

        // Yield to scheduler - we're now blocked
        sched::yield_cpu();

        // When we wake up, loop back to check for zombie children
        // The wakeup can come from:
        // 1. A child exiting (sets parent to Ready and calls wake_up_process)
        // 2. A signal being delivered to this process
        current.set_state(ProcessState::Running);

        // Check if we were interrupted by a signal
        if let Some(signum) = current.get_next_pending_signal() {
            // Clear the signal and return EINTR
            current.clear_pending_signal(signum);
            return Err("Interrupted by signal");
        }
    }
}

/// Standard signal numbers (POSIX)
pub mod signals {
    pub const SIGHUP: i32 = 1; // Hangup
    pub const SIGINT: i32 = 2; // Interrupt
    pub const SIGQUIT: i32 = 3; // Quit
    pub const SIGILL: i32 = 4; // Illegal instruction
    pub const SIGTRAP: i32 = 5; // Trace trap
    pub const SIGABRT: i32 = 6; // Abort
    pub const SIGBUS: i32 = 7; // Bus error
    pub const SIGFPE: i32 = 8; // Floating point exception
    pub const SIGKILL: i32 = 9; // Kill (cannot be caught)
    pub const SIGUSR1: i32 = 10; // User signal 1
    pub const SIGSEGV: i32 = 11; // Segmentation violation
    pub const SIGUSR2: i32 = 12; // User signal 2
    pub const SIGPIPE: i32 = 13; // Broken pipe
    pub const SIGALRM: i32 = 14; // Alarm clock
    pub const SIGTERM: i32 = 15; // Termination
    pub const SIGSTKFLT: i32 = 16; // Stack fault
    pub const SIGCHLD: i32 = 17; // Child status changed
    pub const SIGCONT: i32 = 18; // Continue
    pub const SIGSTOP: i32 = 19; // Stop (cannot be caught)
    pub const SIGTSTP: i32 = 20; // Terminal stop
    pub const SIGTTIN: i32 = 21; // Background read from tty
    pub const SIGTTOU: i32 = 22; // Background write to tty
    pub const SIGURG: i32 = 23; // Urgent data on socket
    pub const SIGXCPU: i32 = 24; // CPU time limit exceeded
    pub const SIGXFSZ: i32 = 25; // File size limit exceeded
    pub const SIGVTALRM: i32 = 26; // Virtual timer expired
    pub const SIGPROF: i32 = 27; // Profiling timer expired
    pub const SIGWINCH: i32 = 28; // Window size changed
    pub const SIGIO: i32 = 29; // I/O possible
    pub const SIGPWR: i32 = 30; // Power failure
    pub const SIGSYS: i32 = 31; // Bad system call
}

/// Signal action types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    /// Default action for signal
    Default,
    /// Ignore signal
    Ignore,
    /// Terminate process
    Terminate,
    /// Terminate and dump core
    CoreDump,
    /// Stop process
    Stop,
    /// Continue stopped process
    Continue,
    /// Call user handler at given address
    Handler(usize),
}

/// Get default action for a signal
pub fn default_signal_action(signal: i32) -> SignalAction {
    use signals::*;
    match signal {
        SIGHUP | SIGINT | SIGKILL | SIGPIPE | SIGALRM | SIGTERM | SIGUSR1 | SIGUSR2 => {
            SignalAction::Terminate
        }
        SIGQUIT | SIGILL | SIGABRT | SIGFPE | SIGSEGV | SIGBUS | SIGSYS | SIGTRAP | SIGXCPU
        | SIGXFSZ => SignalAction::CoreDump,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => SignalAction::Stop,
        SIGCONT => SignalAction::Continue,
        SIGCHLD | SIGURG | SIGWINCH | SIGIO => SignalAction::Ignore,
        _ => SignalAction::Terminate, // Unknown signals terminate by default
    }
}

/// Send a signal to a process (kill syscall)
pub fn kill_process(pid: ProcessId, signal: i32) -> Result<(), &'static str> {
    // Validate signal number
    if !(0..=31).contains(&signal) {
        return Err("Invalid signal number");
    }

    // Special case: signal 0 is used to check if process exists
    if signal == 0 {
        if table::get_process(pid).is_some() {
            return Ok(());
        } else {
            return Err("Process not found");
        }
    }

    let process = table::get_process(pid).ok_or("Process not found")?;

    if !process.is_alive() {
        return Err("Process already dead");
    }

    println!("[PROCESS] Sending signal {} to process {}", signal, pid.0);

    // Queue the signal to the process
    process.send_signal(signal as usize)?;

    // Determine the action to take
    let handler = process.get_signal_handler(signal as usize).unwrap_or(0);
    let action = if handler == 0 {
        default_signal_action(signal)
    } else if handler == 1 {
        SignalAction::Ignore
    } else {
        SignalAction::Handler(handler as usize)
    };

    // Handle uncatchable signals immediately
    match signal {
        signals::SIGKILL => {
            // SIGKILL always terminates immediately
            force_terminate_process(process)?;
        }
        signals::SIGSTOP => {
            // SIGSTOP always stops immediately
            process.set_state(ProcessState::Blocked);
            sched::block_process(pid);
            println!("[PROCESS] Process {} stopped by SIGSTOP", pid.0);
        }
        _ => {
            // Handle based on action
            match action {
                SignalAction::Ignore => {
                    // Clear the pending signal since we're ignoring it
                    process.clear_pending_signal(signal as usize);
                }
                SignalAction::Terminate | SignalAction::CoreDump => {
                    // For default terminate/core dump actions, do it now
                    force_terminate_process(process)?;
                }
                SignalAction::Stop => {
                    process.set_state(ProcessState::Blocked);
                    sched::block_process(pid);
                    println!("[PROCESS] Process {} stopped", pid.0);
                }
                SignalAction::Continue => {
                    if process.get_state() == ProcessState::Blocked {
                        process.set_state(ProcessState::Ready);
                        sched::wake_up_process(pid);
                        println!("[PROCESS] Process {} continued", pid.0);
                    }
                    process.clear_pending_signal(signal as usize);
                }
                SignalAction::Handler(_addr) => {
                    // Signal will be delivered when process returns to user mode
                    // The signal handling is done in the syscall return path
                    println!(
                        "[PROCESS] Signal {} queued for process {}, handler at {:#x}",
                        signal, pid.0, _addr
                    );

                    // Wake up process if blocked so it can handle the signal
                    if process.get_state() == ProcessState::Blocked {
                        process.set_state(ProcessState::Ready);
                        sched::wake_up_process(pid);
                    }
                }
                SignalAction::Default => {
                    // Should not reach here, but handle it as terminate
                    force_terminate_process(process)?;
                }
            }
        }
    }

    Ok(())
}

/// Force terminate a process (used by SIGKILL and unhandled fatal signals)
fn force_terminate_process(process: &Process) -> Result<(), &'static str> {
    let _pid = process.pid;
    println!("[PROCESS] Force terminating process {}", _pid.0);

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

    // Wake up parent if waiting
    if let Some(parent_pid) = process.parent {
        if let Some(parent) = table::get_process(parent_pid) {
            // Send SIGCHLD to parent
            let _ = parent.send_signal(signals::SIGCHLD as usize);

            if parent.get_state() == ProcessState::Blocked {
                parent.set_state(ProcessState::Ready);
                sched::wake_up_process(parent_pid);
            }
        }
    }

    Ok(())
}

/// Clean up process resources
fn cleanup_process(process: &Process) {
    println!(
        "[PROCESS] Cleaning up resources for process {}",
        process.pid.0
    );

    // Release memory
    {
        let mut memory_space = process.memory_space.lock();
        // Clear all mappings
        memory_space.clear();
    }

    // Release capabilities
    {
        let cap_space = process.capability_space.lock();
        // Clear all capabilities
        cap_space.clear();
    }

    // Close IPC endpoints
    #[cfg(feature = "alloc")]
    {
        use crate::ipc;

        // Remove all endpoints owned by this process from the global registry
        match ipc::remove_process_endpoints(process.pid) {
            Ok(count) => {
                if count > 0 {
                    println!(
                        "[PROCESS] Removed {} IPC endpoints for process {}",
                        count, process.pid.0
                    );
                }
            }
            Err(_e) => {
                println!(
                    "[PROCESS] Warning: Failed to remove IPC endpoints for process {}: {:?}",
                    process.pid.0, _e
                );
            }
        }

        // Clear the local endpoint map
        process.ipc_endpoints.lock().clear();
    }

    // Close all open file descriptors
    {
        let file_table = process.file_table.lock();
        file_table.close_all();
    }

    // Reparent children to init if not zombie
    #[cfg(feature = "alloc")]
    {
        let children: Vec<ProcessId> = process.children.lock().clone();
        if !children.is_empty() && process.get_state() != ProcessState::Zombie {
            if let Some(init_process) = table::get_process_mut(ProcessId(1)) {
                for child_pid in children {
                    if let Some(child) = table::get_process_mut(child_pid) {
                        child.parent = Some(ProcessId(1));
                        init_process.children.lock().push(child_pid);
                        println!("[PROCESS] Reparented process {} to init", child_pid);
                    }
                }
            }
            process.children.lock().clear();
        }
    }

    // Update CPU time statistics
    let _cpu_time = process.cpu_time.load(Ordering::Relaxed);
    println!(
        "[PROCESS] Process {} used {} microseconds of CPU time",
        process.pid.0, _cpu_time
    );
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

        // Free thread stacks using memory space unmap
        // Free user stack
        if thread.user_stack.size > 0 {
            let stack_base = thread.user_stack.base;
            let stack_size = thread.user_stack.size;

            // Unmap user stack from process's virtual address space
            let memory_space = process.memory_space.lock();
            if let Err(_e) = memory_space.unmap(stack_base, stack_size) {
                println!(
                    "[PROCESS] Warning: Failed to unmap user stack at {:#x}: {}",
                    stack_base, _e
                );
            } else {
                println!(
                    "[PROCESS] Freed user stack at {:#x}, size {}",
                    stack_base, stack_size
                );
            }
        }

        // Free kernel stack
        if thread.kernel_stack.size > 0 {
            let stack_base = thread.kernel_stack.base;
            let stack_size = thread.kernel_stack.size;

            // Free kernel stack frames directly using the frame allocator
            // Kernel stacks are physically allocated, so we need to free the frames
            let num_pages = stack_size.div_ceil(0x1000);
            for i in 0..num_pages {
                let frame_addr = stack_base + i * 0x1000;
                // Convert kernel virtual to physical address (identity mapped in kernel space)
                // For kernel addresses above 0xFFFF_8000_0000_0000, subtract the offset
                let phys_addr = if frame_addr >= 0xFFFF_8000_0000_0000 {
                    frame_addr - 0xFFFF_8000_0000_0000
                } else {
                    frame_addr
                };
                // Wrap in PhysicalAddress newtype for mm::free_frame
                crate::mm::free_frame(crate::mm::PhysicalAddress::new(phys_addr as u64));
            }
            println!(
                "[PROCESS] Freed kernel stack at {:#x}, size {} ({} frames)",
                stack_base, stack_size, num_pages
            );
        }

        // Clean up TLS area
        {
            let tls = thread.tls.lock();
            if tls.base != 0 && tls.size > 0 {
                // Unmap TLS from process's virtual address space
                let memory_space = process.memory_space.lock();
                if let Err(_e) = memory_space.unmap(tls.base, tls.size) {
                    println!(
                        "[PROCESS] Warning: Failed to unmap TLS at {:#x}: {}",
                        tls.base, _e
                    );
                } else {
                    println!(
                        "[PROCESS] Freed TLS area at {:#x}, size {}",
                        tls.base, tls.size
                    );
                }
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
    task.priority = match *process.priority.lock() {
        ProcessPriority::RealTime => sched::task::Priority::RealTimeHigh,
        ProcessPriority::System => sched::task::Priority::SystemHigh,
        ProcessPriority::Normal => sched::task::Priority::UserNormal,
        ProcessPriority::Low => sched::task::Priority::UserLow,
        ProcessPriority::Idle => sched::task::Priority::Idle,
    };

    task.sched_class = match *process.priority.lock() {
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

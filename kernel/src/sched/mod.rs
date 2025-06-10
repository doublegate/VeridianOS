//! Process and thread scheduling module
//!
//! Implements a multi-level scheduler with support for:
//! - Multiple scheduling algorithms (round-robin, priority, CFS)
//! - Real-time scheduling classes
//! - SMP load balancing
//! - CPU affinity
//! - Context switching for x86_64, AArch64, and RISC-V

#![allow(dead_code)]

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

// Re-export submodules
pub mod queue;
pub mod scheduler;
pub mod smp;
pub mod task;
pub mod task_ptr;

// Re-export common types
pub use queue::READY_QUEUE;
pub use scheduler::{SchedAlgorithm, SCHEDULER};
pub use task::{Priority, Task};

/// Process ID type
pub type ProcessId = u64;

/// Thread ID type
pub type ThreadId = u64;

/// Process state
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Process is ready to run
    Ready = 0,
    /// Process is currently running
    Running = 1,
    /// Process is blocked waiting for IPC receive
    ReceiveBlocked = 2,
    /// Process is blocked waiting for IPC reply
    ReplyBlocked = 3,
    /// Process is sleeping
    Sleeping = 4,
    /// Process has exited
    Exited = 5,
}

/// Process structure (compatibility wrapper)
pub struct Process {
    pub pid: ProcessId,
    pub state: ProcessState,
    pub blocked_on: Option<u64>,
    /// Underlying task
    task: Option<NonNull<Task>>,
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Allocate new process ID
pub fn alloc_pid() -> ProcessId {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

/// Get the current process
pub fn current_process() -> &'static mut Process {
    // Get from per-CPU scheduler
    if let Some(task_ptr) = SCHEDULER.lock().current() {
        unsafe {
            let task = task_ptr.as_ref();
            static mut CURRENT_PROCESS: Process = Process {
                pid: 0,
                state: ProcessState::Running,
                blocked_on: None,
                task: None,
            };

            CURRENT_PROCESS.pid = task.pid;
            CURRENT_PROCESS.state = task.state;
            CURRENT_PROCESS.blocked_on = task.blocked_on;
            CURRENT_PROCESS.task = Some(task_ptr);

            &mut *(&raw mut CURRENT_PROCESS)
        }
    } else {
        // No current task, return dummy
        static mut DUMMY_PROCESS: Process = Process {
            pid: 0,
            state: ProcessState::Running,
            blocked_on: None,
            task: None,
        };
        unsafe { &mut *(&raw mut DUMMY_PROCESS) }
    }
}

/// Switch to another process
pub fn switch_to_process(target: &Process) {
    if let Some(task_ptr) = target.task {
        let mut scheduler = SCHEDULER.lock();
        scheduler.enqueue(task_ptr);
        scheduler.schedule();
    }
}

/// Find process by PID
pub fn find_process(pid: ProcessId) -> Option<&'static mut Process> {
    // TODO: Implement process table lookup
    // For now, check if it's the current process
    let current = current_process();
    if current.pid == pid {
        Some(current)
    } else {
        None
    }
}

/// Yield CPU to scheduler
pub fn yield_cpu() {
    SCHEDULER.lock().schedule();
}

/// Block current process on IPC
pub fn block_on_ipc(endpoint: u64) {
    if let Some(current_task) = SCHEDULER.lock().current() {
        unsafe {
            let task_mut = current_task.as_ptr();
            (*task_mut).state = ProcessState::ReceiveBlocked;
            (*task_mut).blocked_on = Some(endpoint);
        }
        SCHEDULER.lock().schedule();
    }
}

/// Wake up process blocked on IPC
pub fn wake_up_process(pid: ProcessId) {
    // TODO: Search all task queues for this PID
    // For now, just mark it ready if it's current
    let scheduler = SCHEDULER.lock();
    if let Some(current_task) = scheduler.current() {
        unsafe {
            let task = current_task.as_ref();
            if task.pid == pid {
                let task_mut = current_task.as_ptr();
                (*task_mut).state = ProcessState::Ready;
                (*task_mut).blocked_on = None;
                scheduler.enqueue(current_task);
            }
        }
    }
}

/// Initialize scheduler
pub fn init() {
    println!("[SCHED] Initializing scheduler...");

    // Initialize SMP support
    smp::init();

    // Create idle task for BSP
    #[cfg(feature = "alloc")]
    {
        extern crate alloc;
        use alloc::string::String;
        let _idle_task = Task::new(
            0, // PID 0 for idle
            0, // TID 0
            String::from("idle"),
            idle_task_entry as usize,
            0, // Will be set to proper stack
            0, // Will be set to kernel page table
        );

        // TODO: Allocate actual idle task structure
        // For now, we'll skip this as it needs memory allocator
    }

    // Set up timer interrupt for preemption
    #[cfg(target_arch = "x86_64")]
    {
        // TODO: Configure APIC timer
    }

    #[cfg(target_arch = "aarch64")]
    {
        // TODO: Configure generic timer
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // TODO: Configure RISC-V timer
    }

    println!("[SCHED] Scheduler initialized");
}

/// Run scheduler main loop (called by idle task)
pub fn run() -> ! {
    println!("[SCHED] Entering scheduler main loop");
    loop {
        // Check for ready tasks
        if READY_QUEUE.lock().has_ready_tasks() {
            SCHEDULER.lock().schedule();
        }

        // Enter low power state
        crate::arch::idle();
    }
}

/// Idle task entry point
extern "C" fn idle_task_entry() -> ! {
    run()
}

/// Handle timer tick
pub fn timer_tick() {
    SCHEDULER.lock().tick();
}

/// Set scheduling algorithm
pub fn set_algorithm(algorithm: SchedAlgorithm) {
    SCHEDULER.lock().algorithm = algorithm;
}

/// Create new user task
#[cfg(feature = "alloc")]
pub fn create_task(
    name: &str,
    entry_point: usize,
    stack_size: usize,
    priority: Priority,
) -> Result<ProcessId, &'static str> {
    extern crate alloc;
    use alloc::string::String;

    // Allocate PID and TID
    let pid = alloc_pid();
    let tid = task::alloc_tid();

    // TODO: Allocate stack
    let stack_base = 0; // Placeholder

    // TODO: Create page table
    let page_table = 0; // Placeholder

    // Create task
    let mut task = Task::new(
        pid,
        tid,
        String::from(name),
        entry_point,
        stack_base + stack_size,
        page_table,
    );

    task.priority = priority;

    // TODO: Add to task table
    // For now, just enqueue it
    // let task_ptr = NonNull::new(&mut task as *mut _).unwrap();
    // SCHEDULER.enqueue(task_ptr);

    Ok(pid)
}

/// Exit current task
#[allow(unused_variables)]
pub fn exit_task(exit_code: i32) {
    if let Some(current_task) = SCHEDULER.lock().current() {
        unsafe {
            let task_mut = current_task.as_ptr();
            (*task_mut).state = ProcessState::Exited;
        }
        SCHEDULER.lock().schedule();
    }

    // Should not return
    loop {
        crate::arch::idle();
    }
}

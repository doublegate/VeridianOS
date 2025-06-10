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

use crate::{
    arch::context::ThreadContext,
    process::{thread::ThreadState, ProcessId as ProcId, ThreadId as ThrId},
    sched::task::{CpuSet, TaskContext},
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
pub use task::{Priority, SchedClass, SchedPolicy, Task};

/// Process ID type
pub type ProcessId = u64;

/// Thread ID type
pub type ThreadId = u64;

// Import ProcessState from process module
use crate::process::ProcessState;

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
#[allow(static_mut_refs)]
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
            CURRENT_PROCESS.state = match task.state {
                ProcessState::Creating => ProcessState::Ready,
                ProcessState::Ready => ProcessState::Ready,
                ProcessState::Running => ProcessState::Running,
                ProcessState::Blocked => ProcessState::Blocked,
                ProcessState::Sleeping => ProcessState::Sleeping,
                ProcessState::Zombie => ProcessState::Dead,
                ProcessState::Dead => ProcessState::Dead,
            };
            CURRENT_PROCESS.blocked_on = task.blocked_on;
            CURRENT_PROCESS.task = Some(task_ptr);

            &mut CURRENT_PROCESS
        }
    } else {
        // No current task, return dummy
        static mut DUMMY_PROCESS: Process = Process {
            pid: 0,
            state: ProcessState::Running,
            blocked_on: None,
            task: None,
        };
        unsafe { &mut DUMMY_PROCESS }
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
            (*task_mut).state = ProcessState::Blocked;
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
        use alloc::{boxed::Box, string::String};

        // Allocate stack for idle task (8KB)
        const IDLE_STACK_SIZE: usize = 8192;
        let idle_stack = Box::leak(Box::new([0u8; IDLE_STACK_SIZE]));
        let idle_stack_top = idle_stack.as_ptr() as usize + IDLE_STACK_SIZE;

        // Get kernel page table
        let kernel_page_table = crate::mm::get_kernel_page_table();

        // Create idle task
        let mut idle_task = Box::new(Task::new(
            0, // PID 0 for idle
            0, // TID 0
            String::from("idle"),
            idle_task_entry as usize,
            idle_stack_top,
            kernel_page_table,
        ));

        // Set as idle priority
        idle_task.priority = Priority::Idle;
        idle_task.sched_class = SchedClass::Idle;
        idle_task.sched_policy = SchedPolicy::Idle;

        // Get raw pointer to idle task
        let idle_ptr = NonNull::new(Box::leak(idle_task) as *mut _).unwrap();

        // Initialize scheduler with idle task
        SCHEDULER.lock().init(idle_ptr);

        println!("[SCHED] Created idle task with PID 0");
    }

    // Set up timer interrupt for preemption
    #[cfg(target_arch = "x86_64")]
    {
        // Configure timer for 10ms tick (100Hz)
        crate::arch::x86_64::timer::setup_timer(10);
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Configure generic timer for 10ms tick
        crate::arch::aarch64::timer::setup_timer(10);
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // Configure RISC-V timer for 10ms tick
        crate::arch::riscv::timer::setup_timer(10);
    }

    println!("[SCHED] Scheduler initialized");
}

/// Run scheduler main loop (called by idle task)
pub fn run() -> ! {
    println!("[SCHED] Entering scheduler main loop");

    let mut balance_counter = 0u64;

    loop {
        // Check for ready tasks
        if READY_QUEUE.lock().has_ready_tasks() {
            SCHEDULER.lock().schedule();
        }

        // Periodically perform load balancing
        balance_counter = balance_counter.wrapping_add(1);
        if balance_counter % 1000 == 0 {
            #[cfg(feature = "alloc")]
            balance_load();
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
    let mut scheduler = SCHEDULER.lock();

    if let Some(current_task) = scheduler.current() {
        unsafe {
            let task_mut = current_task.as_ptr();
            let task_ref = &*task_mut;

            // Mark task as dead
            (*task_mut).state = ProcessState::Dead;

            // Clean up thread reference if exists
            if let Some(thread_ptr) = task_ref.thread_ref {
                let thread = thread_ptr.as_ref();

                // Remove task pointer from thread
                thread.set_task_ptr(None);

                // Mark thread as dead
                thread.set_state(ThreadState::Dead);

                // Store exit code
                thread.exit_code.store(exit_code as u32, Ordering::Release);
            }

            // Clean up scheduler data structures
            // Remove from ready queue if present
            if let Some(ready_link) = (*task_mut).ready_link {
                // TODO: Remove from ready queue
                (*task_mut).ready_link = None;
            }

            // Remove from wait queue if blocked
            if let Some(wait_link) = (*task_mut).wait_link {
                // TODO: Remove from wait queue
                (*task_mut).wait_link = None;
            }

            // Clear current CPU assignment
            (*task_mut).current_cpu = None;

            // TODO: Free task memory after ensuring no references
            // For now, we leak it as other parts may still have pointers
        }

        // Schedule another task
        scheduler.schedule();
    }

    // Should not return
    loop {
        crate::arch::idle();
    }
}

/// Create task from process thread
#[cfg(feature = "alloc")]
pub fn create_task_from_thread(
    process_id: ProcId,
    thread_id: ThrId,
    thread: &crate::process::Thread,
) -> Result<NonNull<Task>, &'static str> {
    extern crate alloc;
    use alloc::{boxed::Box, string::String};

    // Get thread context to extract entry point and stack
    let ctx = thread.context.lock();
    let entry_point = ctx.get_instruction_pointer();
    let kernel_stack_top = thread.kernel_stack.top();
    drop(ctx);

    // Create scheduler task from process thread
    let mut task = Box::new(Task::new(
        process_id.0,
        thread_id.0,
        String::from(&thread.name),
        entry_point,
        kernel_stack_top,
        0, // Will be set to process page table
    ));

    // Set priority based on thread priority (numeric value)
    task.priority = match thread.priority {
        0..=10 => Priority::RealTimeHigh,
        11..=20 => Priority::RealTimeNormal,
        21..=30 => Priority::RealTimeLow,
        31..=40 => Priority::SystemHigh,
        41..=50 => Priority::SystemNormal,
        51..=60 => Priority::UserHigh,
        61..=70 => Priority::UserNormal,
        71..=80 => Priority::UserLow,
        _ => Priority::Idle,
    };

    // Set scheduling class
    task.sched_class = if task.priority <= Priority::RealTimeLow {
        SchedClass::RealTime
    } else if task.priority == Priority::Idle {
        SchedClass::Idle
    } else {
        SchedClass::Normal
    };

    // Set CPU affinity
    task.cpu_affinity = CpuSet::from_mask(thread.cpu_affinity.load(Ordering::Relaxed) as u64);

    // Copy thread context - create new task context from thread context
    let thread_ctx = thread.context.lock();
    task.context = TaskContext::new(entry_point, kernel_stack_top);
    drop(thread_ctx);

    // Set user stack
    task.user_stack = thread.user_stack.top();

    // Get thread pointer
    let thread_ptr = NonNull::new(thread as *const _ as *mut _);
    task.thread_ref = thread_ptr;

    // Get the task pointer
    let task_ptr = NonNull::new(Box::leak(task) as *mut _).unwrap();

    // Link thread and task bidirectionally
    thread.set_task_ptr(Some(task_ptr));

    // Return pointer to leaked task
    Ok(task_ptr)
}

/// Schedule a process thread
#[cfg(feature = "alloc")]
pub fn schedule_thread(
    process_id: ProcId,
    thread_id: ThrId,
    thread: &crate::process::Thread,
) -> Result<(), &'static str> {
    let task_ptr = create_task_from_thread(process_id, thread_id, thread)?;

    // Find best CPU for this task
    let target_cpu = if thread.cpu_affinity.load(Ordering::Relaxed) == !0usize {
        // No affinity restriction, use least loaded CPU
        smp::find_least_loaded_cpu()
    } else {
        // Find least loaded CPU that matches affinity
        let mut best_cpu = 0;
        let mut min_load = 100;
        let affinity = thread.cpu_affinity.load(Ordering::Relaxed) as u64;

        for cpu in 0..8 {
            // Check first 8 CPUs
            if (affinity & (1 << cpu)) != 0 {
                if let Some(cpu_data) = smp::per_cpu(cpu) {
                    if cpu_data.cpu_info.is_online() {
                        let load = cpu_data.cpu_info.load.load(Ordering::Relaxed);
                        if load < min_load {
                            min_load = load;
                            best_cpu = cpu;
                        }
                    }
                }
            }
        }
        best_cpu
    };

    // Schedule on target CPU
    scheduler::schedule_on_cpu(target_cpu, task_ptr);
    Ok(())
}

/// Perform load balancing across CPUs
#[cfg(feature = "alloc")]
#[allow(unused_variables, unused_assignments)]
fn balance_load() {
    use core::sync::atomic::Ordering;

    // Find most loaded and least loaded CPUs
    let mut max_load = 0u8;
    let mut min_load = 100u8;
    let mut busiest_cpu = 0u8;
    let mut idlest_cpu = 0u8;

    for cpu_id in 0..smp::MAX_CPUS as u8 {
        if let Some(cpu_data) = smp::per_cpu(cpu_id) {
            if cpu_data.cpu_info.is_online() {
                let load = cpu_data.cpu_info.load.load(Ordering::Relaxed);

                if load > max_load {
                    max_load = load;
                    busiest_cpu = cpu_id;
                }

                if load < min_load {
                    min_load = load;
                    idlest_cpu = cpu_id;
                }
            }
        }
    }

    // If imbalance is significant, migrate tasks
    if max_load > min_load + 20 {
        // TODO: Implement task migration
        // For now, just log the imbalance
        if max_load > min_load + 50 {
            println!(
                "[SCHED] Load imbalance detected: CPU {} load={}, CPU {} load={}",
                busiest_cpu, max_load, idlest_cpu, min_load
            );
        }
    }
}

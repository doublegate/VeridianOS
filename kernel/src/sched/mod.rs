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
pub mod metrics;
pub mod queue;
pub mod scheduler;
pub mod smp;
pub mod task;
pub mod task_ptr;

// Re-export common types
pub use queue::READY_QUEUE;
pub use scheduler::{SchedAlgorithm, SCHEDULER};
pub use task::{Priority, SchedClass, SchedPolicy, Task};

// Import ProcessState from process module
use crate::process::ProcessState;
// Use process module types
use crate::process::{ProcessId, ThreadId};

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
    ProcessId(NEXT_PID.fetch_add(1, Ordering::Relaxed))
}

/// Get the current process
#[allow(static_mut_refs)]
pub fn current_process() -> &'static mut Process {
    // Get from per-CPU scheduler
    if let Some(task_ptr) = SCHEDULER.lock().current() {
        unsafe {
            let task = task_ptr.as_ref();
            static mut CURRENT_PROCESS: Process = Process {
                pid: ProcessId(0),
                state: ProcessState::Running,
                blocked_on: None,
                task: None,
            };

            let current_ref = &mut *core::ptr::addr_of_mut!(CURRENT_PROCESS);
            current_ref.pid = task.pid;
            current_ref.state = match task.state {
                ProcessState::Creating => ProcessState::Ready,
                ProcessState::Ready => ProcessState::Ready,
                ProcessState::Running => ProcessState::Running,
                ProcessState::Blocked => ProcessState::Blocked,
                ProcessState::Sleeping => ProcessState::Sleeping,
                ProcessState::Zombie => ProcessState::Dead,
                ProcessState::Dead => ProcessState::Dead,
            };
            current_ref.blocked_on = task.blocked_on;
            current_ref.task = Some(task_ptr);

            current_ref
        }
    } else {
        // No current task, return dummy
        static mut DUMMY_PROCESS: Process = Process {
            pid: ProcessId(0),
            state: ProcessState::Running,
            blocked_on: None,
            task: None,
        };
        unsafe { &mut *core::ptr::addr_of_mut!(DUMMY_PROCESS) }
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
    // First check if it's the current process (fast path)
    let current = current_process();
    if current.pid == pid {
        return Some(current);
    }

    // Otherwise, look it up in the process table
    #[cfg(feature = "alloc")]
    {
        // Get the actual process from the process table
        if let Some(process) = crate::process::table::get_process_mut(pid) {
            // Create a Process wrapper for the scheduler
            static mut FOUND_PROCESS: Process = Process {
                pid: ProcessId(0),
                state: ProcessState::Running,
                blocked_on: None,
                task: None,
            };

            unsafe {
                FOUND_PROCESS.pid = process.pid;
                FOUND_PROCESS.state = process.get_state();
                FOUND_PROCESS.blocked_on = None; // Would need to be tracked
                FOUND_PROCESS.task = None; // Would need task mapping

                Some(&mut *core::ptr::addr_of_mut!(FOUND_PROCESS))
            }
        } else {
            None
        }
    }

    #[cfg(not(feature = "alloc"))]
    None
}

/// Yield CPU to scheduler
pub fn yield_cpu() {
    SCHEDULER.lock().schedule();
}

/// Block current process on IPC
pub fn block_on_ipc(endpoint: u64) {
    let scheduler = scheduler::current_scheduler();
    let mut sched = scheduler.lock();

    if let Some(current_task) = sched.current() {
        unsafe {
            let task_mut = current_task.as_ptr();
            (*task_mut).state = ProcessState::Blocked;
            (*task_mut).blocked_on = Some(endpoint);

            // Update thread state if linked
            if let Some(thread_ptr) = (*task_mut).thread_ref {
                thread_ptr
                    .as_ref()
                    .set_state(crate::process::thread::ThreadState::Blocked);
            }
        }

        // Add task to wait queue for this endpoint
        add_to_wait_queue(current_task, endpoint);

        // Record IPC block metric
        metrics::SCHEDULER_METRICS.record_ipc_block();

        // Force a reschedule
        sched.schedule();
    }
}

/// Wake up process blocked on IPC
pub fn wake_up_process(pid: ProcessId) {
    // First check if task is in any wait queue
    if let Some(task_ptr) = remove_from_wait_queue(pid) {
        unsafe {
            let task_mut = task_ptr.as_ptr();
            let previous_state = (*task_mut).state;
            (*task_mut).state = ProcessState::Ready;
            (*task_mut).blocked_on = None;

            // Update thread state if linked
            if let Some(thread_ptr) = (*task_mut).thread_ref {
                thread_ptr
                    .as_ref()
                    .set_state(crate::process::thread::ThreadState::Ready);
            }

            // Record IPC wakeup metric if it was blocked on IPC
            if previous_state == ProcessState::Blocked {
                metrics::SCHEDULER_METRICS.record_ipc_wakeup();
            }

            // Find the best CPU to schedule on
            let target_cpu = if (*task_mut).cpu_affinity.mask() != 0 {
                // Find least loaded CPU that matches affinity
                smp::find_least_loaded_cpu_with_affinity((*task_mut).cpu_affinity.mask())
            } else {
                // No affinity restriction, use least loaded CPU
                smp::find_least_loaded_cpu()
            };

            // Schedule on target CPU
            scheduler::schedule_on_cpu(target_cpu, task_ptr);
            return;
        }
    }

    // If not in wait queue, search all CPU ready queues
    for cpu_id in 0..smp::MAX_CPUS as u8 {
        if let Some(cpu_data) = smp::per_cpu(cpu_id) {
            if cpu_data.cpu_info.is_online() {
                // Check if the scheduler has this task
                // TODO: Per-CPU schedulers not yet implemented, using global scheduler
                let sched = SCHEDULER.lock();

                // Search through the scheduler's tasks
                if let Some(current) = sched.current() {
                    unsafe {
                        if (*current.as_ptr()).pid == pid {
                            // Found it as current task - just update state
                            (*current.as_ptr()).state = ProcessState::Ready;
                            if let Some(thread_ptr) = (*current.as_ptr()).thread_ref {
                                thread_ptr
                                    .as_ref()
                                    .set_state(crate::process::thread::ThreadState::Ready);
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    // If still not found, try to look up in process table and create task if needed
    #[cfg(feature = "alloc")]
    {
        if let Some(process) = crate::process::table::get_process_mut(pid) {
            // Update process state
            process
                .state
                .store(ProcessState::Ready as u32, Ordering::Release);

            // Find main thread and wake it
            // TODO: Implement get_main_thread() method on Process
            // if let Some(main_thread) = process.get_main_thread() {
            //     main_thread.
            // set_state(crate::process::thread::ThreadState::Ready);

            //     // Try to schedule the thread if it has a task
            //     if let Some(task_ptr) = main_thread.get_task_ptr() {
            //         let target_cpu = smp::find_least_loaded_cpu();
            //         scheduler::schedule_on_cpu(target_cpu, task_ptr);
            //     }
            // }
        }
    }
}

/// Wake up all processes blocked on a specific endpoint
pub fn wake_up_endpoint_waiters(endpoint: u64) {
    #[cfg(feature = "alloc")]
    {
        let waiters = get_endpoint_waiters(endpoint);
        for task_ptr in waiters {
            unsafe {
                let task = task_ptr.as_ref();
                wake_up_process(task.pid);
            }
        }
    }
    #[cfg(not(feature = "alloc"))]
    {
        // Without alloc, we can't maintain wait queues
        let _ = endpoint;
    }
}

/// Wait queue for blocked tasks
#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[cfg(feature = "alloc")]
use spin::Lazy;

/// Wrapper to make NonNull<Task> Send/Sync
/// Safety: We ensure that Tasks are only accessed with proper synchronization
#[derive(Clone, Copy)]
struct TaskPtr(core::ptr::NonNull<Task>);

unsafe impl Send for TaskPtr {}
unsafe impl Sync for TaskPtr {}

#[cfg(feature = "alloc")]
static WAIT_QUEUES: Lazy<spin::Mutex<BTreeMap<u64, Vec<TaskPtr>>>> =
    Lazy::new(|| spin::Mutex::new(BTreeMap::new()));

#[cfg(feature = "alloc")]
fn wait_queues() -> &'static spin::Mutex<BTreeMap<u64, Vec<TaskPtr>>> {
    &WAIT_QUEUES
}

/// Add task to wait queue for endpoint
#[cfg(feature = "alloc")]
fn add_to_wait_queue(task: core::ptr::NonNull<Task>, endpoint: u64) {
    let mut queues = wait_queues().lock();
    queues.entry(endpoint).or_default().push(TaskPtr(task));
}

/// Remove task from wait queue by PID
#[cfg(feature = "alloc")]
fn remove_from_wait_queue(pid: ProcessId) -> Option<core::ptr::NonNull<Task>> {
    let mut queues = wait_queues().lock();

    for (_endpoint, waiters) in queues.iter_mut() {
        if let Some(pos) = waiters
            .iter()
            .position(|&TaskPtr(task_ptr)| unsafe { task_ptr.as_ref().pid == pid })
        {
            return Some(waiters.remove(pos).0);
        }
    }

    None
}

/// Get all waiters for an endpoint
#[cfg(feature = "alloc")]
fn get_endpoint_waiters(endpoint: u64) -> Vec<core::ptr::NonNull<Task>> {
    let mut queues = wait_queues().lock();
    queues
        .remove(&endpoint)
        .unwrap_or_default()
        .into_iter()
        .map(|TaskPtr(ptr)| ptr)
        .collect()
}

// Stub implementations for no_std without alloc
#[cfg(not(feature = "alloc"))]
fn add_to_wait_queue(_task: core::ptr::NonNull<Task>, _endpoint: u64) {
    // No-op without alloc
}

#[cfg(not(feature = "alloc"))]
fn remove_from_wait_queue(_pid: ProcessId) -> Option<core::ptr::NonNull<Task>> {
    None
}

#[cfg(not(feature = "alloc"))]
fn get_endpoint_waiters(_endpoint: u64) -> [core::ptr::NonNull<Task>; 0] {
    []
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
            ProcessId(0), // PID 0 for idle
            ThreadId(0),  // TID 0
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
        println!("[SCHED] x86_64 timer configured for preemptive scheduling");
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Configure generic timer for 10ms tick
        crate::arch::aarch64::timer::setup_timer(10);
        println!("[SCHED] AArch64 timer configured for preemptive scheduling");
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // Configure RISC-V timer for 10ms tick
        crate::arch::riscv::timer::setup_timer(10);
        println!("[SCHED] RISC-V timer configured for preemptive scheduling");
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

        // Periodically perform load balancing and cleanup
        balance_counter = balance_counter.wrapping_add(1);
        if balance_counter % 1000 == 0 {
            #[cfg(feature = "alloc")]
            {
                balance_load();

                // Also clean up dead tasks
                if balance_counter % 10000 == 0 {
                    cleanup_dead_tasks();
                }
            }
        }

        // Enter low power state
        crate::arch::idle();
    }
}

/// Idle task entry point
pub extern "C" fn idle_task_entry() -> ! {
    run()
}

/// Handle timer tick
pub fn timer_tick() {
    scheduler::current_scheduler().lock().tick();
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

            // Mark task for deferred cleanup
            // We can't free immediately as other CPUs might have references
            #[cfg(feature = "alloc")]
            {
                // Add to cleanup queue for deferred deallocation
                static CLEANUP_QUEUE: Lazy<spin::Mutex<Vec<(TaskPtr, u64)>>> =
                    Lazy::new(|| spin::Mutex::new(Vec::new()));

                // Get current tick count for deferred cleanup
                let cleanup_tick = crate::arch::timer::get_ticks() + 100; // Cleanup after 100 ticks
                CLEANUP_QUEUE
                    .lock()
                    .push((TaskPtr(current_task), cleanup_tick));
            }
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
        process_id,
        thread_id,
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

/// Clean up dead tasks that have been marked for deferred deallocation
#[cfg(feature = "alloc")]
pub fn cleanup_dead_tasks() {
    extern crate alloc;
    use alloc::boxed::Box;

    use spin::Lazy;

    static CLEANUP_QUEUE: Lazy<spin::Mutex<Vec<(TaskPtr, u64)>>> =
        Lazy::new(|| spin::Mutex::new(Vec::new()));

    let current_tick = crate::arch::timer::get_ticks();
    let mut queue = CLEANUP_QUEUE.lock();

    // Find tasks that are ready to be cleaned up
    let mut i = 0;
    while i < queue.len() {
        let (TaskPtr(task_ptr), cleanup_tick) = queue[i];

        if current_tick >= cleanup_tick {
            // Remove from queue
            queue.swap_remove(i);

            // Safety: We've waited long enough that no CPU should have references
            unsafe {
                // Deallocate the task
                let task_box = Box::from_raw(task_ptr.as_ptr());
                drop(task_box);
            }

            println!("[SCHED] Cleaned up dead task");
        } else {
            i += 1;
        }
    }
}

/// Perform load balancing across CPUs
#[cfg(feature = "alloc")]
fn balance_load() {
    use core::sync::atomic::Ordering;

    // Find most loaded and least loaded CPUs
    let mut max_load = 0u8;
    let mut min_load = 100u8;
    let mut busiest_cpu = 0u8;
    let mut _idlest_cpu = 0u8;

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
                    _idlest_cpu = cpu_id;
                }
            }
        }
    }

    // If imbalance is significant, migrate tasks
    let imbalance = max_load.saturating_sub(min_load);
    if imbalance > 20 {
        // Calculate how many tasks to migrate
        let tasks_to_migrate = ((imbalance / 20) as u32).min(3); // Migrate up to 3 tasks

        if tasks_to_migrate > 0 {
            println!(
                "[SCHED] Load balancing: CPU {} (load={}) -> CPU {} (load={}), migrating {} tasks",
                busiest_cpu, max_load, _idlest_cpu, min_load, tasks_to_migrate
            );

            // Record load balance metric
            metrics::SCHEDULER_METRICS.record_load_balance();

            // Try to migrate tasks from busiest to idlest CPU
            if let Some(busy_cpu_data) = smp::per_cpu(busiest_cpu) {
                let migrated = 0u32;

                // Get a snapshot of tasks to potentially migrate
                let _candidates: Vec<core::ptr::NonNull<Task>> = Vec::new();
                {
                    let _queue = busy_cpu_data.cpu_info.ready_queue.lock();
                    // Just peek at tasks, don't remove them yet
                    // This is a simplified approach - in reality we'd need a
                    // better way to iterate through the
                    // queue without modifying it
                }

                // For now, we can't easily iterate the queue, so let's try a different approach
                // We'll attempt to steal tasks when they're being dequeued
                // This is handled in the scheduler's pick_next methods

                // Set a flag to indicate this CPU should try to push tasks
                if migrated < tasks_to_migrate {
                    // In a real implementation, we'd set a flag on the busy CPU
                    // to push tasks to other CPUs when scheduling
                }
            }
        }
    }
}

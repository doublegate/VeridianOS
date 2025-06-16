//! Bootstrap module for kernel initialization
//!
//! This module handles the multi-stage initialization process to avoid
//! circular dependencies between subsystems.

use crate::{
    arch, cap,
    error::{KernelError, KernelResult},
    ipc, mm, process, sched,
};

#[cfg(feature = "alloc")]
extern crate alloc;

/// Bootstrap task ID (runs before scheduler is fully initialized)
pub const BOOTSTRAP_PID: u64 = 0;
pub const BOOTSTRAP_TID: u64 = 0;

/// Multi-stage kernel initialization
///
/// This function implements the recommended boot sequence from
/// DEEP-RECOMMENDATIONS.md to avoid circular dependencies between process
/// management and scheduler.
pub fn kernel_init() -> KernelResult<()> {
    // For AArch64, use direct UART writes instead of println!
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        // Write "[BOOTSTRAP] Starting...\n"
        let msg = b"[BOOTSTRAP] Starting...\n";
        let mut i = 0;
        while i < 24 {
            *uart = msg[i];
            i += 1;
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] Starting multi-stage kernel initialization...");

    // Stage 1: Core hardware initialization
    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] Stage 1: Hardware initialization");

    arch::init();

    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] Architecture initialized");

    // Stage 2: Memory management
    println!("[BOOTSTRAP] Stage 2: Memory management");
    mm::init_default();
    mm::init_heap().map_err(|_| KernelError::OutOfMemory {
        requested: 0,
        available: 0,
    })?;
    println!("[BOOTSTRAP] Memory management initialized");

    // Stage 3: Create bootstrap context for scheduler
    println!("[BOOTSTRAP] Stage 3: Bootstrap context");

    // Initialize scheduler with bootstrap task
    // This allows scheduler to be ready before process creation
    #[cfg(feature = "alloc")]
    {
        use alloc::{boxed::Box, string::String};
        use core::ptr::NonNull;

        use crate::{
            process::{ProcessId, ThreadId},
            sched::{Priority, SchedClass, SchedPolicy, Task},
        };

        // Create bootstrap task that will initialize remaining subsystems
        const BOOTSTRAP_STACK_SIZE: usize = 16 * 1024; // 16KB stack
        let bootstrap_stack = Box::leak(Box::new([0u8; BOOTSTRAP_STACK_SIZE]));
        let bootstrap_stack_top = bootstrap_stack.as_ptr() as usize + BOOTSTRAP_STACK_SIZE;

        let kernel_page_table = mm::get_kernel_page_table();

        let mut bootstrap_task = Box::new(Task::new(
            ProcessId(BOOTSTRAP_PID),
            ThreadId(BOOTSTRAP_TID),
            String::from("bootstrap"),
            bootstrap_stage4 as usize,
            bootstrap_stack_top,
            kernel_page_table,
        ));

        // Set highest priority for bootstrap
        bootstrap_task.priority = Priority::SystemHigh;
        bootstrap_task.sched_class = SchedClass::Normal; // System class doesn't exist, use Normal
        bootstrap_task.sched_policy = SchedPolicy::Fifo;

        let bootstrap_ptr = NonNull::new(Box::leak(bootstrap_task) as *mut _).unwrap();

        // Initialize scheduler with bootstrap task
        sched::init_with_bootstrap(bootstrap_ptr).map_err(|_| KernelError::InvalidState {
            expected: "scheduler ready",
            actual: "initialization failed",
        })?;
        println!("[BOOTSTRAP] Scheduler initialized with bootstrap task");
    }

    // Stage 4: Kernel services (IPC, capabilities)
    println!("[BOOTSTRAP] Stage 4: Kernel services");
    ipc::init();
    cap::init();
    println!("[BOOTSTRAP] IPC and capability systems initialized");

    // Stage 5: Now safe to initialize process management
    println!("[BOOTSTRAP] Stage 5: Process management");
    process::init_without_init_process().map_err(|_| KernelError::InvalidState {
        expected: "process system initialized",
        actual: "initialization failed",
    })?;
    println!("[BOOTSTRAP] Process management initialized (without init process)");

    // Stage 6: Transfer control to scheduler
    println!("[BOOTSTRAP] Stage 6: Starting scheduler");
    println!("[BOOTSTRAP] Kernel initialization complete!");

    // The scheduler will run the bootstrap task which continues initialization
    sched::start();
}

/// Bootstrap stage 4 - runs as first scheduled task
///
/// This function runs within the scheduler context and completes
/// the remaining initialization steps.
#[no_mangle]
pub extern "C" fn bootstrap_stage4() -> ! {
    println!("[BOOTSTRAP] Stage 4 task running in scheduler context");

    // Now we can safely create the init process
    #[cfg(feature = "alloc")]
    {
        use alloc::string::String;

        let init_entry = init_process_main as usize;
        match process::lifecycle::create_process(String::from("init"), init_entry) {
            Ok(init_pid) => {
                println!("[BOOTSTRAP] Created init process with PID {}", init_pid.0);

                // The init process already has a main thread created by create_process
                // We just need to schedule it
                if let Some(init_proc) = process::table::get_process(init_pid) {
                    // Get the main thread ID
                    if let Some(tid) = init_proc.get_main_thread_id() {
                        // Get the thread itself
                        if let Some(main_thread) = init_proc.get_thread(tid) {
                            if let Err(e) = sched::schedule_thread(init_pid, tid, main_thread) {
                                println!(
                                    "[BOOTSTRAP] Warning: Failed to schedule init thread: {}",
                                    e
                                );
                            }
                        } else {
                            panic!("[BOOTSTRAP] Failed to get init main thread!");
                        }
                    } else {
                        panic!("[BOOTSTRAP] Init process has no main thread!");
                    }
                } else {
                    panic!("[BOOTSTRAP] Failed to find init process after creation!");
                }
            }
            Err(e) => {
                panic!("[BOOTSTRAP] Failed to create init process: {}", e);
            }
        }

        // Create test tasks for demonstration (after init process is ready)
        // This demonstrates context switching between multiple tasks
        println!("[BOOTSTRAP] Creating test tasks for context switch demonstration");
        crate::test_tasks::create_test_tasks();
    }

    #[cfg(not(feature = "alloc"))]
    {
        println!("[BOOTSTRAP] Warning: alloc feature not enabled, skipping init process creation");
    }

    println!("[BOOTSTRAP] Bootstrap complete, transitioning to idle task");

    // Transform into idle task
    idle_task_main()
}

/// Init process main function
#[no_mangle]
extern "C" fn init_process_main() -> ! {
    println!("[INIT] Init process started!");

    // TODO: Mount root filesystem
    // TODO: Start core services
    // TODO: Start user shell

    let mut counter = 0u64;
    loop {
        if counter % 1_000_000 == 0 {
            println!("[INIT] Running... ({})", counter / 1_000_000);
        }
        counter = counter.wrapping_add(1);

        // Yield periodically
        if counter % 10_000 == 0 {
            sched::yield_cpu();
        }
    }
}

/// Idle task main function
fn idle_task_main() -> ! {
    println!("[IDLE] Entering idle loop");

    let mut idle_counter = 0u64;
    loop {
        // Check for work
        if sched::has_ready_tasks() {
            sched::yield_cpu();
        }

        // Periodic maintenance
        idle_counter = idle_counter.wrapping_add(1);
        if idle_counter % 100_000 == 0 {
            // Perform cleanup, load balancing, etc.
            #[cfg(feature = "alloc")]
            {
                if idle_counter % 1_000_000 == 0 {
                    sched::cleanup_dead_tasks();
                }
            }
        }

        // Enter low power state
        arch::idle();
    }
}

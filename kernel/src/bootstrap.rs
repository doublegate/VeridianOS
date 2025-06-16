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
    // AArch64-specific bypass to avoid LLVM bug
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'1';
            *uart = b'\n';
        }
        arch::init();
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'2';
            *uart = b'\n';
        }
    }

    // Use boot_println! for other architectures
    #[cfg(not(target_arch = "aarch64"))]
    {
        boot_println!("[BOOTSTRAP] Starting multi-stage kernel initialization...");
        boot_println!("[BOOTSTRAP] Stage 1: Hardware initialization");
        arch::init();
        boot_println!("[BOOTSTRAP] Architecture initialized");
    }

    // Stage 2: Memory management
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'M';
            *uart = b'M';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Stage 2: Memory management");

    mm::init_default();
    mm::init_heap().map_err(|_| KernelError::OutOfMemory {
        requested: 0,
        available: 0,
    })?;

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'3';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Memory management initialized");

    // Stage 3: Create bootstrap context for scheduler
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Stage 3: Bootstrap context");

    // Use static allocation to avoid heap allocation issues during early boot
    #[cfg(feature = "alloc")]
    {
        boot_println!("[BOOTSTRAP] Using static bootstrap stack to avoid heap allocation...");

        // Static bootstrap stack to avoid heap allocation during early boot
        static mut BOOTSTRAP_STACK: [u8; 8192] = [0u8; 8192];
        let _bootstrap_stack_top =
            unsafe { core::ptr::addr_of_mut!(BOOTSTRAP_STACK).add(8192) as usize };
        boot_println!("[BOOTSTRAP] Static bootstrap stack prepared");

        boot_println!("[BOOTSTRAP] About to get kernel page table...");
        let _kernel_page_table = mm::get_kernel_page_table();
        boot_println!("[BOOTSTRAP] Got kernel page table");

        boot_println!("[BOOTSTRAP] About to initialize scheduler without bootstrap task...");
        boot_println!("[BOOTSTRAP] About to initialize SMP...");
        boot_println!("[BOOTSTRAP] Skipping SMP initialization due to heap allocation issues");
        boot_println!("[BOOTSTRAP] SMP initialization skipped");

        boot_println!("[BOOTSTRAP] About to initialize basic scheduler...");
        boot_println!("[BOOTSTRAP] Using RISC-V scheduler initialization");

        // For RISC-V, skip scheduler initialization that requires heap allocations
        #[cfg(target_arch = "riscv64")]
        {
            // Minimal scheduler setup without heap allocations
            boot_println!("[BOOTSTRAP] Skipping complex scheduler init for RISC-V");
        }

        #[cfg(not(target_arch = "riscv64"))]
        {
            sched::init();
        }
        boot_println!("[BOOTSTRAP] Scheduler initialized without bootstrap task");
    }

    // Stage 4: Kernel services (IPC, capabilities)
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'4';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Stage 4: Kernel services");

    ipc::init();
    cap::init();

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'I';
            *uart = b'P';
            *uart = b'C';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] IPC and capability systems initialized");

    // Stage 5: Now safe to initialize process management
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'5';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Stage 5: Process management");

    process::init_without_init_process().map_err(|_| KernelError::InvalidState {
        expected: "process system initialized",
        actual: "initialization failed",
    })?;

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'P';
            *uart = b'R';
            *uart = b'O';
            *uart = b'C';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Process management initialized (without init process)");

    // Stage 6: Completing initialization
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'6';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Stage 6: Completing initialization");

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'D';
            *uart = b'O';
            *uart = b'N';
            *uart = b'E';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Kernel initialization complete!");

    // Continue initialization inline instead of scheduler transfer
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'C';
            *uart = b'O';
            *uart = b'N';
            *uart = b'T';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Continuing initialization inline...");

    bootstrap_stage4_inline();
}

/// Inline bootstrap stage 4 - simplified approach
fn bootstrap_stage4_inline() -> ! {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'T';
            *uart = b'G';
            *uart = b'4';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Stage 4 inline running in bootstrap context");

    // Skip init process creation due to heap allocation issues
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'K';
            *uart = b'I';
            *uart = b'P';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Skipping init process creation due to heap allocation issues");

    // Create test tasks for demonstration would normally go here
    // but we skip it to avoid heap allocations

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'B';
            *uart = b'O';
            *uart = b'O';
            *uart = b'T';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Bootstrap complete - heap allocations need to be fixed");

    // Enter idle loop
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'I';
            *uart = b'D';
            *uart = b'L';
            *uart = b'E';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[BOOTSTRAP] Entering idle loop");

    idle_task_main()
}

/// Bootstrap stage 4 - runs as first scheduled task
///
/// This function runs within the scheduler context and completes
/// the remaining initialization steps.
#[no_mangle]
pub extern "C" fn bootstrap_stage4() -> ! {
    boot_println!("[BOOTSTRAP] Stage 4 task running in scheduler context");

    // Now we can safely create the init process
    #[cfg(feature = "alloc")]
    {
        use alloc::string::String;

        let init_entry = init_process_main as usize;
        match process::lifecycle::create_process(String::from("init"), init_entry) {
            Ok(init_pid) => {
                boot_print_num!("[BOOTSTRAP] Created init process with PID ", init_pid.0);

                // The init process already has a main thread created by create_process
                // We just need to schedule it
                if let Some(init_proc) = process::table::get_process(init_pid) {
                    // Get the main thread ID
                    if let Some(tid) = init_proc.get_main_thread_id() {
                        // Get the thread itself
                        if let Some(main_thread) = init_proc.get_thread(tid) {
                            if let Err(_e) = sched::schedule_thread(init_pid, tid, main_thread) {
                                #[cfg(not(target_arch = "aarch64"))]
                                boot_println!(
                                    "[BOOTSTRAP] Warning: Failed to schedule init thread"
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
            Err(_e) => {
                panic!("[BOOTSTRAP] Failed to create init process");
            }
        }

        // Create test tasks for demonstration (after init process is ready)
        // This demonstrates context switching between multiple tasks
        boot_println!("[BOOTSTRAP] Creating test tasks for context switch demonstration");
        crate::test_tasks::create_test_tasks();
    }

    #[cfg(not(feature = "alloc"))]
    {
        boot_println!(
            "[BOOTSTRAP] Warning: alloc feature not enabled, skipping init process creation"
        );
    }

    boot_println!("[BOOTSTRAP] Bootstrap complete, transitioning to idle task");

    // Transform into idle task
    idle_task_main()
}

/// Init process main function
#[no_mangle]
extern "C" fn init_process_main() -> ! {
    boot_println!("[INIT] Init process started!");

    // TODO: Mount root filesystem
    // TODO: Start core services
    // TODO: Start user shell

    let mut counter = 0u64;
    loop {
        if counter % 1_000_000 == 0 {
            boot_print_num!("[INIT] Running... ", counter / 1_000_000);
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
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'I';
            *uart = b'D';
            *uart = b'L';
            *uart = b'E';
            *uart = b' ';
            *uart = b'L';
            *uart = b'O';
            *uart = b'O';
            *uart = b'P';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    boot_println!("[IDLE] Entering idle loop");

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

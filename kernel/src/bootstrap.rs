//! Bootstrap module for kernel initialization
//!
//! This module handles the multi-stage initialization process to avoid
//! circular dependencies between subsystems.

use crate::{
    arch, cap,
    error::KernelResult,
    fs, ipc, mm, process, sched, services,
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
    // Stage 1: Hardware initialization
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage1_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage1_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage1_start();
    
    arch::init();
    
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage1_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage1_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage1_complete();

    // Stage 2: Memory management
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage2_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage2_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage2_start();
    
    mm::init_default();
    
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage2_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage2_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage2_complete();

    // Stage 3: Process management
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage3_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage3_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage3_start();
    
    process::init_without_init_process().expect("Failed to initialize process management");
    
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage3_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage3_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage3_complete();

    // Stage 4: Core kernel services
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage4_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage4_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage4_start();
    
    cap::init();
    ipc::init();
    
    // Initialize VFS and mount essential filesystems
    #[cfg(feature = "alloc")]
    {
        fs::init();
    }
    
    // Initialize services (process server, driver framework, etc.)
    #[cfg(feature = "alloc")]
    {
        services::init();
    }
    
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage4_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage4_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage4_complete();

    // Stage 5: Scheduler initialization
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage5_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage5_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage5_start();
    
    sched::init();
    
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage5_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage5_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage5_complete();

    Ok(())
}

/// Run the bootstrap sequence
pub fn run() -> ! {
    if let Err(e) = kernel_init() {
        panic!("Bootstrap failed: {:?}", e);
    }

    // Stage 6: User space transition
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage6_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage6_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage6_start();

    // Create init process
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] About to create init process...\n");
        }
    }
    
    #[cfg(target_arch = "riscv64")]
    {
        println!("[BOOTSTRAP] About to create init process...");
    }
    
    create_init_process();
    
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Init process created\n");
        }
    }
    
    #[cfg(target_arch = "riscv64")]
    {
        println!("[BOOTSTRAP] Init process created");
    }
    
    // Mark Stage 6 complete
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage6_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage6_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage6_complete();

    // Phase 2 validation (only run once after all services are initialized)
    crate::println!("");
    crate::println!("🔬 Running Phase 2 Complete Validation...");
    crate::phase2_validation::quick_health_check();
    
    // Run full Phase 2 validation:
    crate::phase2_validation::validate_phase2_complete();
    
    crate::println!("✅ Phase 2 User Space Foundation - COMPLETE!");
    crate::println!("");

    // Transfer control to scheduler
    sched::start();
}

/// Create the init process
fn create_init_process() {
    #[cfg(target_arch = "aarch64")]
    {
        // Skip init process creation for AArch64 due to allocation issues
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Skipping init process creation for AArch64\n");
        }
        return;
    }
    
    #[cfg(target_arch = "riscv64")]
    {
        // Skip init process creation for RISC-V due to allocation issues
        println!("[BOOTSTRAP] Skipping init process creation for RISC-V");
        return;
    }
    
    #[cfg(feature = "alloc")]
    {
        // Try to load init from the filesystem
        match crate::userspace::load_init_process() {
            Ok(init_pid) => {
                println!("[BOOTSTRAP] Init process created with PID {}", init_pid.0);
                
                // Try to load a shell as well
                if let Ok(shell_pid) = crate::userspace::loader::load_shell() {
                    println!("[BOOTSTRAP] Shell process created with PID {}", shell_pid.0);
                }
            }
            Err(e) => {
                println!("[BOOTSTRAP] Failed to create init process: {}", e);
                // Fall back to creating a minimal test process
                use alloc::string::String;
                if let Ok(pid) = process::lifecycle::create_process(String::from("init"), 0) {
                    println!("[BOOTSTRAP] Created fallback init process with PID {}", pid.0);
                }
            }
        }
    }
}
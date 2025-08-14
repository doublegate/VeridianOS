//! Bootstrap module for kernel initialization
//!
//! This module handles the multi-stage initialization process to avoid
//! circular dependencies between subsystems.

use crate::{
    arch, cap,
    error::KernelResult,
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
    
    process::init();
    
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
    create_init_process();

    // Transfer control to scheduler
    sched::start();
}

/// Create the init process
fn create_init_process() {
    #[cfg(feature = "alloc")]
    {
        use alloc::string::String;
        
        // TODO: Load init from filesystem
        // For now, create a test process
        // Process is automatically marked as ready in create_process
        if let Ok(_init_pid) = process::lifecycle::create_process(String::from("init"), 0) {
            // Process created and marked as ready
        }
    }
}
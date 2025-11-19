//! Bootstrap module for kernel initialization
//!
//! This module handles the multi-stage initialization process to avoid
//! circular dependencies between subsystems.

use crate::{
    arch, cap, error::KernelResult, fs, graphics, ipc, mm, net, perf, pkg, process, sched,
    security, services,
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
    // Direct UART output for RISC-V debugging
    #[cfg(target_arch = "riscv64")]
    unsafe {
        let uart_base = 0x1000_0000 as *mut u8;
        // Write "KINIT" to show kernel_init reached
        uart_base.write_volatile(b'K');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'N');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'T');
        uart_base.write_volatile(b'\n');
    }

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

    println!("[BOOTSTRAP] Initializing capabilities...");
    cap::init();
    println!("[BOOTSTRAP] Capabilities initialized");

    println!("[BOOTSTRAP] Initializing security subsystem...");
    security::init().expect("Failed to initialize security");
    println!("[BOOTSTRAP] Security subsystem initialized");

    println!("[BOOTSTRAP] Initializing performance monitoring...");
    perf::init().expect("Failed to initialize performance monitoring");
    println!("[BOOTSTRAP] Performance monitoring initialized");

    println!("[BOOTSTRAP] Initializing IPC...");
    ipc::init();
    println!("[BOOTSTRAP] IPC initialized");

    // Initialize VFS and mount essential filesystems
    #[cfg(feature = "alloc")]
    {
        // Add early debug output for AArch64
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] About to initialize VFS (AArch64 direct UART)...\n");
        }

        println!("[BOOTSTRAP] Initializing VFS...");
        fs::init();

        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] VFS initialized (AArch64 direct UART)\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[BOOTSTRAP] VFS initialized");
    }

    // Initialize services (process server, driver framework, etc.)
    #[cfg(feature = "alloc")]
    {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Initializing services (AArch64)...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[BOOTSTRAP] Initializing services...");

        services::init();

        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Services initialized (AArch64)\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[BOOTSTRAP] Services initialized");
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

    // Initialize package manager
    #[cfg(feature = "alloc")]
    {
        println!("[BOOTSTRAP] Initializing package manager...");
        pkg::init();
        println!("[BOOTSTRAP] Package manager initialized");
    }

    // Initialize network stack
    #[cfg(feature = "alloc")]
    {
        println!("[BOOTSTRAP] Initializing network stack...");
        net::init().expect("Failed to initialize network stack");
        println!("[BOOTSTRAP] Network stack initialized");
    }

    // Initialize graphics subsystem
    println!("[BOOTSTRAP] Initializing graphics subsystem...");
    graphics::init().expect("Failed to initialize graphics");
    println!("[BOOTSTRAP] Graphics subsystem initialized");

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
    // Direct UART output for RISC-V debugging
    #[cfg(target_arch = "riscv64")]
    unsafe {
        let uart_base = 0x1000_0000 as *mut u8;
        // Write "RUN" to show run() reached
        uart_base.write_volatile(b'R');
        uart_base.write_volatile(b'U');
        uart_base.write_volatile(b'N');
        uart_base.write_volatile(b'\n');
    }

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
                    println!(
                        "[BOOTSTRAP] Created fallback init process with PID {}",
                        pid.0
                    );
                }
            }
        }
    }
}

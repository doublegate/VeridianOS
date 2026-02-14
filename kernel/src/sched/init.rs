//! Scheduler initialization and timer setup
//!
//! Contains the bootstrap initialization path (`init_with_bootstrap`) used
//! during early kernel boot, the normal initialization path (`init`), and
//! architecture-specific preemption timer configuration.

use core::ptr::NonNull;

use super::{smp, task::Task};
use crate::error::KernelResult;

/// Initialize scheduler with bootstrap task
///
/// This is used during early boot to initialize the scheduler with a
/// bootstrap task that will complete kernel initialization.
pub fn init_with_bootstrap(bootstrap_task: NonNull<Task>) -> KernelResult<()> {
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] Initializing scheduler with bootstrap task...");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: uart_write_str performs a volatile write to the UART MMIO
        // register at 0x09000000, which is always mapped on the QEMU virt
        // machine. No Rust memory is aliased.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] Initializing scheduler with bootstrap task...\n");
        }
    }

    // Initialize SMP support
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] About to initialize SMP...");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] About to initialize SMP...\n");
        }
    }

    smp::init();

    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] SMP initialization complete");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] SMP initialization complete\n");
        }
    }

    // Initialize scheduler with bootstrap task
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] About to get scheduler lock...");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] About to get scheduler lock...\n");
        }
    }

    super::SCHEDULER.lock().init(bootstrap_task);

    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] Scheduler init complete");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] Scheduler init complete\n");
        }
    }

    // Set up timer interrupt for preemption
    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] About to setup preemption timer...");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] About to setup preemption timer...\n");
        }
    }

    setup_preemption_timer();

    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] Preemption timer setup complete");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] Preemption timer setup complete\n");
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    println!("[SCHED] Scheduler initialized with bootstrap task");

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] Scheduler initialized with bootstrap task\n");
        }
    }

    Ok(())
}

/// Initialize scheduler normally (after bootstrap)
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    println!("[SCHED] Initializing scheduler...");

    // Skip println for RISC-V to avoid serial issues

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: UART MMIO write to 0x09000000 on QEMU virt machine.
        // No Rust memory aliased.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] Initializing scheduler...\n");
        }
    }

    // Initialize SMP support
    smp::init();

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Same as above -- UART MMIO write.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SCHED] Skipping idle task creation for AArch64\n");
            uart_write_str("[SCHED] Scheduler initialized (minimal for AArch64)\n");
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // Skip println for RISC-V to avoid serial issues
        // Scheduler initialized (minimal for RISC-V)
    }

    // Skip complex scheduler setup on all architectures for now.
    // kernel_init_main() tests run before sched::init() and don't need the
    // scheduler. The idle task creation and PIT timer setup can hang or panic
    // during early boot.
    #[cfg(target_arch = "x86_64")]
    println!("[SCHED] Scheduler initialized (minimal for x86_64)");
}

/// Set up preemption timer
fn setup_preemption_timer() {
    #[cfg(target_arch = "x86_64")]
    {
        // Configure timer for 10ms tick (100Hz)
        crate::arch::x86_64::timer::setup_timer(10);
        #[cfg(not(target_arch = "aarch64"))]
        println!("[SCHED] x86_64 timer configured for preemptive scheduling");

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: Same as above -- UART MMIO write.
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[SCHED] x86_64 timer configured for preemptive scheduling\n");
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Configure generic timer for 10ms tick
        crate::arch::aarch64::timer::setup_timer(10);
        #[cfg(not(target_arch = "aarch64"))]
        println!("[SCHED] AArch64 timer configured for preemptive scheduling");

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: Same as above -- UART MMIO write.
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[SCHED] AArch64 timer configured for preemptive scheduling\n");
            }
        }
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // Configure RISC-V timer for 10ms tick.
        // NOTE: setup_timer() configures the SBI timer but does NOT enable
        // STIE (supervisor timer interrupt enable) because no trap handler
        // (stvec) is registered yet. Enabling STIE without stvec causes
        // the CPU to jump to address 0 on timer fire, rebooting the system.
        crate::arch::riscv::timer::setup_timer(10);
        #[cfg(not(target_arch = "aarch64"))]
        println!("[SCHED] RISC-V timer configured for preemptive scheduling");

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: Same as above -- UART MMIO write.
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[SCHED] RISC-V timer configured for preemptive scheduling\n");
            }
        }
    }
}

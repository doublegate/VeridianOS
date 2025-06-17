/// Test tasks for verifying context switching functionality
///
/// These tasks are designed to test that context switching works properly
/// on all architectures, with special handling for AArch64's loop limitations.
use crate::sched;

/// Test task A - prints messages and yields
#[no_mangle]
pub extern "C" fn test_task_a() -> ! {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::arch::aarch64::direct_uart::{uart_write_str, direct_print_num};

        unsafe {
            let mut counter = 0u64;

            uart_write_str("[TASK A] Started\n");

            loop {
                // Print message
                uart_write_str("[TASK A] Running - count: ");
                direct_print_num(counter);
                uart_write_str("\n");

                // Yield to other tasks
                sched::yield_cpu();

                // Increment counter
                counter = counter.wrapping_add(1);

                // Manual delay using assembly
                core::arch::asm!("mov x0, #50000");
                core::arch::asm!("mov x1, #10");
                core::arch::asm!("2: mov x2, x0");
                core::arch::asm!("1: sub x2, x2, #1");
                core::arch::asm!("cbnz x2, 1b");
                core::arch::asm!("sub x1, x1, #1");
                core::arch::asm!("cbnz x1, 2b");
            }
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut counter = 0u64;

        println!("[TASK A] Started");

        loop {
            println!("[TASK A] Running - count: {}", counter);

            // Yield to other tasks
            sched::yield_cpu();

            counter = counter.wrapping_add(1);

            // Delay
            for _ in 0..500000 {
                core::hint::spin_loop();
            }
        }
    }
}

/// Test task B - prints different messages and yields
#[no_mangle]
pub extern "C" fn test_task_b() -> ! {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::arch::aarch64::direct_uart::{uart_write_str, direct_print_num};

        unsafe {
            let mut counter = 0u64;

            uart_write_str("[TASK B] Started\n");

            loop {
                // Print message
                uart_write_str("[TASK B] Executing - value: ");
                direct_print_num(counter);
                uart_write_str("\n");

                // Yield to other tasks
                sched::yield_cpu();

                // Increment counter by 10
                counter = counter.wrapping_add(10);

                // Manual delay using assembly
                core::arch::asm!("mov x0, #50000");
                core::arch::asm!("mov x1, #15");
                core::arch::asm!("2: mov x2, x0");
                core::arch::asm!("1: sub x2, x2, #1");
                core::arch::asm!("cbnz x2, 1b");
                core::arch::asm!("sub x1, x1, #1");
                core::arch::asm!("cbnz x1, 2b");
            }
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut counter = 0u64;

        println!("[TASK B] Started");

        loop {
            println!("[TASK B] Executing - value: 0x{:x}", counter);

            // Yield to other tasks
            sched::yield_cpu();

            counter = counter.wrapping_add(10);

            // Delay
            for _ in 0..750000 {
                core::hint::spin_loop();
            }
        }
    }
}

/// Create test tasks for context switching verification
pub fn create_test_tasks() {
    #[cfg(feature = "alloc")]
    {
        use alloc::string::String;

        use crate::process;

        #[cfg(not(target_arch = "aarch64"))]
        println!("[TEST] Creating test tasks for context switch verification");
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[TEST] Creating test tasks for context switch verification\n");
        }

        // Create Task A
        match process::lifecycle::create_process(String::from("test_task_a"), 0) {
            Ok(_pid_a) => {
                #[cfg(not(target_arch = "aarch64"))]
                println!("[TEST] Created process A with PID {}", _pid_a.0);
                
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[TEST] Created process A\n");
                }

                if let Err(_e) = process::create_thread(test_task_a as usize, 0, 0, 0) {
                    #[cfg(not(target_arch = "aarch64"))]
                    println!("[TEST] Failed to create thread for task A: {}", _e);
                    
                    #[cfg(target_arch = "aarch64")]
                    unsafe {
                        use crate::arch::aarch64::direct_uart::uart_write_str;
                        uart_write_str("[TEST] Failed to create thread for task A\n");
                    }
                } else {
                    #[cfg(not(target_arch = "aarch64"))]
                    println!("[TEST] Created thread for task A");
                    
                    #[cfg(target_arch = "aarch64")]
                    unsafe {
                        use crate::arch::aarch64::direct_uart::uart_write_str;
                        uart_write_str("[TEST] Created thread for task A\n");
                    }
                }
            }
            Err(_e) => {
                #[cfg(not(target_arch = "aarch64"))]
                println!("[TEST] Failed to create task A: {}", _e);
                
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[TEST] Failed to create task A\n");
                }
            }
        }

        // Create Task B
        match process::lifecycle::create_process(String::from("test_task_b"), 0) {
            Ok(_pid_b) => {
                #[cfg(not(target_arch = "aarch64"))]
                println!("[TEST] Created process B with PID {}", _pid_b.0);
                
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[TEST] Created process B\n");
                }

                if let Err(_e) = process::create_thread(test_task_b as usize, 0, 0, 0) {
                    #[cfg(not(target_arch = "aarch64"))]
                    println!("[TEST] Failed to create thread for task B: {}", _e);
                    
                    #[cfg(target_arch = "aarch64")]
                    unsafe {
                        use crate::arch::aarch64::direct_uart::uart_write_str;
                        uart_write_str("[TEST] Failed to create thread for task B\n");
                    }
                } else {
                    #[cfg(not(target_arch = "aarch64"))]
                    println!("[TEST] Created thread for task B");
                    
                    #[cfg(target_arch = "aarch64")]
                    unsafe {
                        use crate::arch::aarch64::direct_uart::uart_write_str;
                        uart_write_str("[TEST] Created thread for task B\n");
                    }
                }
            }
            Err(_e) => {
                #[cfg(not(target_arch = "aarch64"))]
                println!("[TEST] Failed to create task B: {}", _e);
                
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[TEST] Failed to create task B\n");
                }
            }
        }

        #[cfg(not(target_arch = "aarch64"))]
        println!("[TEST] Test tasks created successfully");
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[TEST] Test tasks created successfully\n");
        }
    }

    #[cfg(not(feature = "alloc"))]
    {
        #[cfg(not(target_arch = "aarch64"))]
        println!("[TEST] Cannot create test tasks: alloc feature not enabled");
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[TEST] Cannot create test tasks: alloc feature not enabled\n");
        }
    }
}

//! Example tests using the custom test framework
//!
//! This demonstrates how to write tests that bypass lang_items conflicts.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_framework::test_runner)]
#![reexport_test_harness_main = "test_main"]

use veridian_kernel::{kernel_assert, kernel_assert_eq, kernel_test, test_module};

// Example unit tests
test_module!(basic_tests,
    test_addition => {
        let result = 2 + 2;
        kernel_assert_eq!(result, 4);
        Ok(())
    },

    test_memory_allocation => {
        // Test basic memory operations
        let value = 42u32;
        let ptr = &value as *const u32;
        kernel_assert!(!ptr.is_null());
        kernel_assert_eq!(unsafe { *ptr }, 42);
        Ok(())
    },

    test_capability_token => {
        use veridian_kernel::cap::{CapabilityToken, Rights};

        // Test capability token creation
        let rights = Rights::READ | Rights::WRITE;
        kernel_assert!(rights.contains(Rights::READ));
        kernel_assert!(rights.contains(Rights::WRITE));
        kernel_assert!(!rights.contains(Rights::EXECUTE));
        Ok(())
    }
);

// Example integration tests
test_module!(integration_tests,
    test_ipc_channel => {
        use veridian_kernel::ipc::{create_channel, Channel};

        // Create a channel
        let (tx, rx) = create_channel(16)?;

        // Send a message
        let msg = [1u8, 2, 3, 4];
        tx.send(&msg)?;

        // Receive the message
        let mut buf = [0u8; 4];
        let len = rx.receive(&mut buf)?;

        kernel_assert_eq!(len, 4);
        kernel_assert_eq!(buf, [1, 2, 3, 4]);
        Ok(())
    },

    test_process_creation => {
        use veridian_kernel::process;

        // Test process creation (will fail in test environment)
        // This demonstrates error handling
        match process::create_process("test", 0) {
            Ok(_) => Err("Process creation should fail in test environment"),
            Err(_) => Ok(()), // Expected to fail
        }
    }
);

// Example benchmark
#[cfg(feature = "benchmarks")]
mod benchmarks {
    use veridian_kernel::{kernel_bench, test_framework::BenchmarkRunner};

    kernel_bench!(bench_atomic_increment, {
        use core::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        COUNTER.fetch_add(1, Ordering::Relaxed);
    });

    kernel_bench!(bench_spinlock, {
        use veridian_kernel::sync::SpinLock;
        static LOCK: SpinLock<u64> = SpinLock::new(0);

        let mut guard = LOCK.lock();
        *guard += 1;
    });
}

// Entry point for test binary
#[no_mangle]
pub extern "C" fn _start() -> ! {
    veridian_kernel::arch::init();
    veridian_kernel::serial_println!("\n=== Running Example Tests ===\n");

    test_main();

    loop {
        veridian_kernel::arch::halt();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_framework::test_panic_handler(info)
}

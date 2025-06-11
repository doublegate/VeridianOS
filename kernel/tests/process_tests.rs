//! Process Management Tests
//!
//! Tests for process lifecycle, threads, and synchronization primitives

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use core::sync::atomic::{AtomicBool, Ordering};

use veridian_kernel::{
    kernel_assert, kernel_assert_eq, kernel_bench,
    process::{
        self,
        sync::{KernelBarrier, KernelCondVar, KernelMutex, KernelRwLock, KernelSemaphore},
        Process, ProcessId, ProcessState, Thread, ThreadId, ThreadState,
    },
    serial_println,
    test_framework::{cycles_to_ns, read_timestamp, BenchmarkRunner},
};

#[path = "common/mod.rs"]
mod common;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    common::init_test_env("Process Management Tests");
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

// ===== Process Lifecycle Tests =====

#[test_case]
fn test_process_creation() {
    process::init();

    let pid = ProcessId(100);
    let process = Process::new(pid, "test_process");

    kernel_assert_eq!(process.pid, pid);
    kernel_assert_eq!(
        process.state.load(Ordering::Relaxed),
        ProcessState::Created as u8
    );
    kernel_assert_eq!(process.exit_code.load(Ordering::Relaxed), 0);

    // Add to process table
    process::table::insert_process(process);

    // Verify it's in the table
    let retrieved = process::table::get_process(pid);
    kernel_assert!(retrieved.is_some());

    // Cleanup
    process::table::remove_process(pid);
}

#[test_case]
fn test_process_state_transitions() {
    process::init();

    let pid = ProcessId(101);
    let mut process = Process::new(pid, "state_test");

    // Created -> Ready
    process.set_state(ProcessState::Ready);
    kernel_assert_eq!(
        process.state.load(Ordering::Relaxed),
        ProcessState::Ready as u8
    );

    // Ready -> Running
    process.set_state(ProcessState::Running);
    kernel_assert_eq!(
        process.state.load(Ordering::Relaxed),
        ProcessState::Running as u8
    );

    // Running -> Blocked
    process.set_state(ProcessState::Blocked);
    kernel_assert_eq!(
        process.state.load(Ordering::Relaxed),
        ProcessState::Blocked as u8
    );

    // Blocked -> Ready
    process.set_state(ProcessState::Ready);
    kernel_assert_eq!(
        process.state.load(Ordering::Relaxed),
        ProcessState::Ready as u8
    );

    // Ready -> Exited
    process.set_state(ProcessState::Exited);
    process.exit_code.store(42, Ordering::Relaxed);
    kernel_assert_eq!(
        process.state.load(Ordering::Relaxed),
        ProcessState::Exited as u8
    );
    kernel_assert_eq!(process.exit_code.load(Ordering::Relaxed), 42);
}

// ===== Thread Management Tests =====

#[test_case]
fn test_thread_creation() {
    let tid = ThreadId(200);
    let thread = Thread::new(tid, ProcessId(100));

    kernel_assert_eq!(thread.tid, tid);
    kernel_assert_eq!(thread.owner_pid, ProcessId(100));
    kernel_assert_eq!(
        thread.state.load(Ordering::Relaxed),
        ThreadState::Creating as u8
    );
}

#[test_case]
fn test_thread_state_transitions() {
    let mut thread = Thread::new(ThreadId(201), ProcessId(101));

    // Creating -> Ready
    thread.set_state(ThreadState::Ready);
    kernel_assert_eq!(
        thread.state.load(Ordering::Relaxed),
        ThreadState::Ready as u8
    );

    // Ready -> Running
    thread.set_state(ThreadState::Running);
    kernel_assert_eq!(
        thread.state.load(Ordering::Relaxed),
        ThreadState::Running as u8
    );

    // Running -> Blocked
    thread.set_state(ThreadState::Blocked);
    kernel_assert_eq!(
        thread.state.load(Ordering::Relaxed),
        ThreadState::Blocked as u8
    );

    // Blocked -> Ready
    thread.set_state(ThreadState::Ready);
    kernel_assert_eq!(
        thread.state.load(Ordering::Relaxed),
        ThreadState::Ready as u8
    );

    // Ready -> Exited
    thread.set_state(ThreadState::Exited);
    kernel_assert_eq!(
        thread.state.load(Ordering::Relaxed),
        ThreadState::Exited as u8
    );
}

// ===== Synchronization Primitive Tests =====

#[test_case]
fn test_mutex_basic() {
    let mutex = KernelMutex::new();

    // Initial state
    kernel_assert!(!mutex.is_locked());

    // Lock
    kernel_assert!(mutex.try_lock());
    kernel_assert!(mutex.is_locked());

    // Can't lock again
    kernel_assert!(!mutex.try_lock());

    // Unlock
    mutex.unlock();
    kernel_assert!(!mutex.is_locked());

    // Can lock again
    kernel_assert!(mutex.try_lock());
    mutex.unlock();
}

#[test_case]
fn test_semaphore_basic() {
    let sem = KernelSemaphore::new(2);

    // Initial permits
    kernel_assert_eq!(sem.available_permits(), 2);

    // Acquire permits
    kernel_assert!(sem.try_wait());
    kernel_assert_eq!(sem.available_permits(), 1);

    kernel_assert!(sem.try_wait());
    kernel_assert_eq!(sem.available_permits(), 0);

    // No more permits
    kernel_assert!(!sem.try_wait());

    // Release permits
    sem.signal();
    kernel_assert_eq!(sem.available_permits(), 1);

    sem.signal();
    kernel_assert_eq!(sem.available_permits(), 2);
}

#[test_case]
fn test_rwlock_basic() {
    let rwlock = KernelRwLock::new();

    // Multiple readers
    kernel_assert!(rwlock.try_read());
    kernel_assert!(rwlock.try_read());
    kernel_assert_eq!(rwlock.reader_count(), 2);

    // Can't write while readers exist
    kernel_assert!(!rwlock.try_write());

    // Release readers
    rwlock.read_unlock();
    rwlock.read_unlock();
    kernel_assert_eq!(rwlock.reader_count(), 0);

    // Now can write
    kernel_assert!(rwlock.try_write());
    kernel_assert!(rwlock.is_write_locked());

    // Can't read or write while write locked
    kernel_assert!(!rwlock.try_read());
    kernel_assert!(!rwlock.try_write());

    // Release writer
    rwlock.write_unlock();
    kernel_assert!(!rwlock.is_write_locked());
}

#[test_case]
fn test_barrier() {
    let barrier = KernelBarrier::new(3);

    // Not enough threads yet
    kernel_assert!(!barrier.wait());
    kernel_assert_eq!(barrier.waiting_count(), 1);

    kernel_assert!(!barrier.wait());
    kernel_assert_eq!(barrier.waiting_count(), 2);

    // Last thread releases all
    kernel_assert!(barrier.wait());
    kernel_assert_eq!(barrier.waiting_count(), 0);

    // Can use again
    kernel_assert!(!barrier.wait());
    kernel_assert_eq!(barrier.waiting_count(), 1);
}

// ===== Process Table Tests =====

#[test_case]
fn test_process_table_operations() {
    process::init();

    // Insert multiple processes
    for i in 300..310 {
        let pid = ProcessId(i);
        let process = Process::new(pid, "table_test");
        process::table::insert_process(process);
    }

    // Verify all exist
    for i in 300..310 {
        let pid = ProcessId(i);
        kernel_assert!(process::table::get_process(pid).is_some());
    }

    // Remove half
    for i in 300..305 {
        process::table::remove_process(ProcessId(i));
    }

    // Verify removed
    for i in 300..305 {
        kernel_assert!(process::table::get_process(ProcessId(i)).is_none());
    }

    // Verify remaining
    for i in 305..310 {
        kernel_assert!(process::table::get_process(ProcessId(i)).is_some());
    }

    // Cleanup
    for i in 305..310 {
        process::table::remove_process(ProcessId(i));
    }
}

// ===== Performance Benchmarks =====

kernel_bench!(bench_process_creation, {
    static mut COUNTER: u64 = 1000;
    unsafe {
        let pid = ProcessId(COUNTER);
        let process = Process::new(pid, "bench_process");
        COUNTER += 1;
        black_box(process);
    }
});

kernel_bench!(bench_thread_creation, {
    static mut COUNTER: u64 = 2000;
    unsafe {
        let tid = ThreadId(COUNTER);
        let thread = Thread::new(tid, ProcessId(100));
        COUNTER += 1;
        black_box(thread);
    }
});

kernel_bench!(bench_mutex_lock_unlock, {
    static MUTEX: KernelMutex = KernelMutex::new();

    MUTEX.try_lock();
    MUTEX.unlock();
});

#[test_case]
fn bench_process_table_lookup() {
    process::init();

    // Pre-populate table
    for i in 0..100 {
        let pid = ProcessId(i);
        let process = Process::new(pid, "lookup_bench");
        process::table::insert_process(process);
    }

    let runner = BenchmarkRunner::new();
    let result = runner.run_benchmark("process_table_lookup", || {
        let process = process::table::get_process(ProcessId(50));
        black_box(process);
    });

    serial_println!("Process table lookup: {} ns", result.avg_time_ns);

    // Lookup should be fast (O(1) or O(log n))
    assert_performance!(result.avg_time_ns, < 500);

    // Cleanup
    for i in 0..100 {
        process::table::remove_process(ProcessId(i));
    }
}

// ===== Thread Safety Tests =====

#[test_case]
fn test_atomic_operations() {
    static COUNTER: AtomicBool = AtomicBool::new(false);

    // Test compare_exchange
    kernel_assert!(COUNTER
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok());

    // Should fail now
    kernel_assert!(COUNTER
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err());

    // Reset
    COUNTER.store(false, Ordering::SeqCst);
    kernel_assert!(!COUNTER.load(Ordering::SeqCst));
}

use core::hint::black_box;

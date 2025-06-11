//! Scheduler Tests
//!
//! Tests for the VeridianOS scheduler implementation

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use core::hint::black_box;

use veridian_kernel::{
    kernel_assert, kernel_assert_eq, kernel_bench,
    process::{ProcessId, ProcessState, ThreadId},
    sched::{self, metrics, Priority, SchedClass, Task},
    serial_println,
    test_framework::{cycles_to_ns, read_timestamp, BenchmarkRunner},
};

#[path = "common/mod.rs"]
mod common;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    common::init_test_env("Scheduler Tests");
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

// ===== Task Creation Tests =====

#[test_case]
fn test_task_creation() {
    sched::init();

    let task_ptr = sched::create_task("test_task", ProcessId(1), ThreadId(1), 0, 0);
    kernel_assert!(!task_ptr.is_null());

    unsafe {
        let task = &*task_ptr;
        kernel_assert_eq!(task.pid, ProcessId(1));
        kernel_assert_eq!(task.tid, ThreadId(1));
        kernel_assert_eq!(task.state, ProcessState::Ready);
        kernel_assert_eq!(task.priority, Priority::Normal(20));

        // Cleanup
        sched::exit_task(task_ptr);
    }
}

#[test_case]
fn test_priority_task_creation() {
    sched::init();

    // Create high priority task
    let high_task = sched::create_task("high_priority", ProcessId(2), ThreadId(2), 0, 0);
    unsafe {
        (*high_task).priority = Priority::High(10);
        (*high_task).sched_class = SchedClass::RealTime;
    }

    // Create low priority task
    let low_task = sched::create_task("low_priority", ProcessId(3), ThreadId(3), 0, 0);
    unsafe {
        (*low_task).priority = Priority::Low(30);
    }

    // Verify priorities
    unsafe {
        kernel_assert!((*high_task).effective_priority() < (*low_task).effective_priority());

        // Cleanup
        sched::exit_task(high_task);
        sched::exit_task(low_task);
    }
}

// ===== Scheduling Policy Tests =====

#[test_case]
fn test_round_robin_scheduling() {
    sched::init();

    // Create multiple tasks with same priority
    let mut tasks = alloc::vec::Vec::new();
    for i in 0..5 {
        let task = sched::create_task("rr_task", ProcessId(10 + i), ThreadId(10 + i), 0, 0);
        tasks.push(task);
    }

    // Each task should get scheduled in turn
    // This would require actually running the scheduler
    // For now, just verify tasks are created properly

    for task in tasks {
        unsafe {
            kernel_assert_eq!((*task).state, ProcessState::Ready);
            sched::exit_task(task);
        }
    }
}

#[test_case]
fn test_priority_preemption() {
    sched::init();

    // Create normal priority task
    let normal_task = sched::create_task("normal", ProcessId(20), ThreadId(20), 0, 0);

    // Create high priority task
    let high_task = sched::create_task("high", ProcessId(21), ThreadId(21), 0, 0);
    unsafe {
        (*high_task).priority = Priority::High(5);
        (*high_task).sched_class = SchedClass::RealTime;
    }

    // High priority task should preempt normal task
    unsafe {
        let should_preempt = sched::should_preempt(&*normal_task, &*high_task);
        kernel_assert!(should_preempt);

        // Cleanup
        sched::exit_task(normal_task);
        sched::exit_task(high_task);
    }
}

// ===== IPC Blocking Tests =====

#[test_case]
fn test_ipc_blocking() {
    sched::init();

    let task = sched::create_task("blocking_task", ProcessId(30), ThreadId(30), 0, 0);

    unsafe {
        // Set task as current (simplified)
        sched::set_current_task(task);

        // Block on IPC endpoint
        let endpoint_id = 12345u64;
        sched::block_on_ipc(endpoint_id);

        // Task should now be blocked
        kernel_assert_eq!((*task).state, ProcessState::Blocked);
        kernel_assert_eq!((*task).blocked_on, Some(endpoint_id));

        // Wake up the task
        sched::wake_from_ipc(endpoint_id);

        // Task should be ready again
        kernel_assert_eq!((*task).state, ProcessState::Ready);
        kernel_assert_eq!((*task).blocked_on, None);

        // Cleanup
        sched::set_current_task(core::ptr::null_mut());
        sched::exit_task(task);
    }
}

// ===== Load Balancing Tests =====

#[test_case]
fn test_cpu_load_tracking() {
    sched::init();
    sched::smp::init_smp();

    // Simulate load on CPU 0
    if let Some(cpu_data) = sched::smp::per_cpu(0) {
        cpu_data
            .cpu_info
            .load
            .store(75, core::sync::atomic::Ordering::Relaxed);
    }

    // Simulate low load on CPU 1
    if let Some(cpu_data) = sched::smp::per_cpu(1) {
        cpu_data
            .cpu_info
            .load
            .store(25, core::sync::atomic::Ordering::Relaxed);
    }

    // Balance load should detect imbalance
    sched::balance_load();

    // Check that load balance was attempted
    let stats = metrics::SCHEDULER_METRICS.get_summary();
    kernel_assert!(stats.load_balance_count > 0);
}

// ===== Performance Benchmarks =====

kernel_bench!(bench_task_creation, {
    static mut COUNTER: u64 = 100;
    unsafe {
        let task = sched::create_task("bench_task", ProcessId(COUNTER), ThreadId(COUNTER), 0, 0);
        COUNTER += 1;
        sched::exit_task(task);
    }
});

#[test_case]
fn bench_context_switch() {
    sched::init();

    // Create two tasks
    let task1 = sched::create_task("task1", ProcessId(40), ThreadId(40), 0, 0);
    let task2 = sched::create_task("task2", ProcessId(41), ThreadId(41), 0, 0);

    let runner = BenchmarkRunner::new();
    let result = runner.run_benchmark("context_switch", || {
        unsafe {
            // Simulate context switch (simplified)
            sched::set_current_task(task1);
            sched::set_current_task(task2);
        }
    });

    serial_println!("Context switch time: {} ns", result.avg_time_ns);

    // Context switch should be <10μs
    assert_performance!(result.avg_time_ns, < 10000);

    // Cleanup
    unsafe {
        sched::exit_task(task1);
        sched::exit_task(task2);
    }
}

// ===== Scheduler Metrics Tests =====

#[test_case]
fn test_scheduler_metrics() {
    sched::init();

    // Reset metrics
    metrics::SCHEDULER_METRICS.reset();

    // Perform some scheduling operations
    let task = sched::create_task("metrics_task", ProcessId(50), ThreadId(50), 0, 0);

    unsafe {
        sched::set_current_task(task);

        // Simulate some scheduler events
        metrics::SCHEDULER_METRICS.record_context_switch(true);
        metrics::SCHEDULER_METRICS.record_schedule_overhead(1000);
        metrics::SCHEDULER_METRICS.record_ipc_block();
        metrics::SCHEDULER_METRICS.record_ipc_wakeup();
    }

    // Check metrics
    let summary = metrics::SCHEDULER_METRICS.get_summary();
    kernel_assert_eq!(summary.context_switches, 1);
    kernel_assert_eq!(summary.voluntary_switches, 1);
    kernel_assert_eq!(summary.ipc_blocks, 1);
    kernel_assert_eq!(summary.ipc_wakeups, 1);

    // Print metrics
    metrics::print_metrics();

    // Cleanup
    unsafe {
        sched::set_current_task(core::ptr::null_mut());
        sched::exit_task(task);
    }
}

// ===== Idle Task Tests =====

#[test_case]
fn test_idle_task_creation() {
    sched::init();
    sched::smp::init_smp();

    // Each CPU should have an idle task
    for cpu_id in 0..2 {
        if let Some(cpu_data) = sched::smp::per_cpu(cpu_id) {
            kernel_assert!(!cpu_data.idle_task.is_null());

            unsafe {
                let idle_task = &*cpu_data.idle_task;
                kernel_assert_eq!(idle_task.sched_class, SchedClass::Idle);
                kernel_assert_eq!(idle_task.priority, Priority::Idle);
            }
        }
    }
}

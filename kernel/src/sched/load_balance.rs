//! Load balancing and task migration between CPUs
//!
//! Implements periodic load balancing across CPUs, task migration from
//! overloaded to underloaded CPUs, and deferred cleanup of dead tasks.

use core::sync::atomic::Ordering;

use super::{metrics, smp, task::Task};

/// Wrapper to make NonNull<Task> Send/Sync for load balancing data structures.
///
/// # Safety
///
/// TaskPtr instances in load balancing are only accessed under appropriate
/// locks (cleanup queue mutex or CPU ready queue locks). Task memory is
/// managed by the kernel allocator.
#[derive(Clone, Copy)]
struct TaskPtr(core::ptr::NonNull<Task>);

// SAFETY: TaskPtr is only accessed under mutex locks in the cleanup queue or
// during load balancing with CPU ready queue locks held. No unsynchronized
// concurrent access occurs.
unsafe impl Send for TaskPtr {}
// SAFETY: Same as Send -- all access is synchronized via mutexes.
unsafe impl Sync for TaskPtr {}

/// Clean up dead tasks that have been marked for deferred deallocation
#[cfg(feature = "alloc")]
pub fn cleanup_dead_tasks() {
    extern crate alloc;
    use alloc::{boxed::Box, vec::Vec};

    use spin::Lazy;

    static CLEANUP_QUEUE: Lazy<spin::Mutex<Vec<(TaskPtr, u64)>>> =
        Lazy::new(|| spin::Mutex::new(Vec::new()));

    let current_tick = crate::arch::timer::get_ticks();
    let mut queue = CLEANUP_QUEUE.lock();

    // Find tasks that are ready to be cleaned up
    let mut i = 0;
    while i < queue.len() {
        let (TaskPtr(task_ptr), cleanup_tick) = queue[i];

        if current_tick >= cleanup_tick {
            // Remove from queue
            queue.swap_remove(i);

            // SAFETY: This task pointer was placed in the cleanup queue by
            // `exit_task` after being removed from the scheduler. We waited
            // at least 100 ticks (the cleanup delay) to ensure no other CPU
            // holds a reference to this task. The pointer was originally
            // created via `Box::leak` and is valid to reconstruct.
            unsafe {
                let task_box = Box::from_raw(task_ptr.as_ptr());
                drop(task_box);
            }

            #[cfg(not(target_arch = "aarch64"))]
            println!("[SCHED] Cleaned up dead task");

            #[cfg(target_arch = "aarch64")]
            {
                // SAFETY: uart_write_str writes to the UART MMIO register at
                // 0x09000000 on the QEMU virt machine. This is always mapped
                // and does not alias Rust memory.
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[SCHED] Cleaned up dead task\n");
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Perform load balancing across CPUs
#[cfg(feature = "alloc")]
pub fn balance_load() {
    use core::sync::atomic::Ordering;

    // Find most loaded and least loaded CPUs
    let mut max_load = 0u8;
    let mut min_load = 100u8;
    let mut busiest_cpu = 0u8;
    let mut idlest_cpu = 0u8;

    for cpu_id in 0..smp::MAX_CPUS as u8 {
        if let Some(cpu_data) = smp::per_cpu(cpu_id) {
            if cpu_data.cpu_info.is_online() {
                let load = cpu_data.cpu_info.load.load(Ordering::Relaxed);

                if load > max_load {
                    max_load = load;
                    busiest_cpu = cpu_id;
                }

                if load < min_load {
                    min_load = load;
                    idlest_cpu = cpu_id;
                }
            }
        }
    }

    // If imbalance is significant, migrate tasks
    let imbalance = max_load.saturating_sub(min_load);
    if imbalance > 20 {
        // Calculate how many tasks to migrate
        let tasks_to_migrate = ((imbalance / 20) as u32).min(3); // Migrate up to 3 tasks

        if tasks_to_migrate > 0 {
            #[cfg(not(target_arch = "aarch64"))]
            println!(
                "[SCHED] Load balancing: CPU {} (load={}) -> CPU {} (load={}), migrating {} tasks",
                busiest_cpu, max_load, idlest_cpu, min_load, tasks_to_migrate
            );

            #[cfg(target_arch = "aarch64")]
            {
                // SAFETY: UART MMIO write to 0x09000000. No Rust memory aliased.
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[SCHED] Load balancing: migrating tasks\n");
                }
            }

            // Record load balance metric
            metrics::SCHEDULER_METRICS.record_load_balance();

            // Perform actual task migration
            migrate_tasks(busiest_cpu, idlest_cpu, tasks_to_migrate);
        }
    }
}

/// Migrate tasks from source CPU to target CPU
#[cfg(feature = "alloc")]
fn migrate_tasks(source_cpu: u8, target_cpu: u8, count: u32) {
    use alloc::vec::Vec;
    let mut migrated = 0u32;

    // Try to get tasks from source CPU's ready queue
    if let Some(source_cpu_data) = smp::per_cpu(source_cpu) {
        // Collect tasks to migrate
        let mut tasks_to_migrate = Vec::new();

        {
            let mut queue = source_cpu_data.cpu_info.ready_queue.lock();

            // Try to dequeue tasks that can run on target CPU
            for _ in 0..count {
                if let Some(task_ptr) = queue.dequeue() {
                    // SAFETY: `task_ptr` is a valid NonNull<Task> returned by
                    // `queue.dequeue()`. We hold the queue lock so the task
                    // is not concurrently modified. We read `can_run_on` to
                    // check affinity.
                    unsafe {
                        let task = task_ptr.as_ref();
                        if task.can_run_on(target_cpu) {
                            tasks_to_migrate.push(task_ptr);
                        } else {
                            // Put it back if it can't run on target
                            queue.enqueue(task_ptr);
                        }
                    }
                }
            }

            // Update source CPU load
            source_cpu_data
                .cpu_info
                .nr_running
                .fetch_sub(tasks_to_migrate.len() as u32, Ordering::Relaxed);
            source_cpu_data.cpu_info.update_load();
        }

        // Migrate collected tasks to target CPU
        if let Some(target_cpu_data) = smp::per_cpu(target_cpu) {
            let mut target_queue = target_cpu_data.cpu_info.ready_queue.lock();

            for task_ptr in tasks_to_migrate {
                // SAFETY: `task_ptr` is a valid NonNull<Task> that was just
                // dequeued from the source CPU. We hold the target queue lock
                // and update the task's migration tracking fields before
                // enqueuing it on the target CPU.
                unsafe {
                    let task_mut = task_ptr.as_ptr();

                    // Update task's CPU assignment
                    (*task_mut).last_cpu = Some(source_cpu);
                    (*task_mut).migrations += 1;

                    // Enqueue on target CPU
                    target_queue.enqueue(task_ptr);
                    migrated += 1;
                }
            }

            // Update target CPU load
            target_cpu_data
                .cpu_info
                .nr_running
                .fetch_add(migrated, Ordering::Relaxed);
            target_cpu_data.cpu_info.update_load();

            // Wake up target CPU if idle
            if target_cpu_data.cpu_info.is_idle() {
                smp::send_ipi(target_cpu, 0);
            }
        }

        if migrated > 0 {
            #[cfg(not(target_arch = "aarch64"))]
            println!("[SCHED] Successfully migrated {} tasks", migrated);

            #[cfg(target_arch = "aarch64")]
            {
                // SAFETY: UART MMIO write to 0x09000000. No Rust memory aliased.
                unsafe {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[SCHED] Successfully migrated tasks\n");
                }
            }

            // Record migration metrics
            for _ in 0..migrated {
                metrics::SCHEDULER_METRICS.record_migration();
            }
        }
    }
}

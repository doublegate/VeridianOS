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

/// Ticks to wait after a task or thread dies before freeing it, so that no
/// CPU still running on its stack or holding its pointer is affected.
#[cfg(feature = "alloc")]
const REAP_DELAY_TICKS: u64 = 100;

/// Dead tasks awaiting deallocation, with the tick at which they may go.
/// One module-level queue: `exit_task` used to push onto a function-local
/// queue that nothing drained, so every dead task leaked (N-03).
#[cfg(feature = "alloc")]
static DEAD_TASKS: spin::Mutex<alloc::vec::Vec<(TaskPtr, u64)>> =
    spin::Mutex::new(alloc::vec::Vec::new());

/// Detached threads that exited and await reaping (PROC-SEC-02). A thread
/// cannot free its own kernel stack while running on it, so it queues
/// itself and the idle loop reaps it after the delay.
#[cfg(feature = "alloc")]
static DETACHED_THREADS: spin::Mutex<
    alloc::vec::Vec<((crate::process::ProcessId, crate::process::ThreadId), u64)>,
> = spin::Mutex::new(alloc::vec::Vec::new());

/// Remove and return the entries of `queue` whose deadline is `now` or
/// earlier, keeping the rest.
#[cfg(feature = "alloc")]
fn take_expired<T>(queue: &mut alloc::vec::Vec<(T, u64)>, now: u64) -> alloc::vec::Vec<T> {
    let mut expired = alloc::vec::Vec::new();
    let mut i = 0;
    while i < queue.len() {
        if queue[i].1 <= now {
            expired.push(queue.swap_remove(i).0);
        } else {
            i += 1;
        }
    }
    expired
}

/// Queue a dead task for deallocation once the reap delay has passed.
///
/// # Safety
///
/// `task` must come from `Box::leak`, be off every run and wait queue, and
/// be referenced by nothing else once it is no longer the running task.
#[cfg(feature = "alloc")]
pub(crate) unsafe fn defer_task_free(task: core::ptr::NonNull<Task>) {
    let deadline = crate::arch::timer::get_ticks() + REAP_DELAY_TICKS;
    DEAD_TASKS.lock().push((TaskPtr(task), deadline));
}

/// Queue an exited detached thread for reaping once the delay has passed.
#[cfg(feature = "alloc")]
pub(crate) fn defer_thread_reap(pid: crate::process::ProcessId, tid: crate::process::ThreadId) {
    let deadline = crate::arch::timer::get_ticks() + REAP_DELAY_TICKS;
    DETACHED_THREADS.lock().push(((pid, tid), deadline));
}

/// Free dead tasks and reap detached threads whose reap delay has passed.
#[cfg(feature = "alloc")]
pub fn cleanup_dead_tasks() {
    use alloc::boxed::Box;

    let now = crate::arch::timer::get_ticks();
    // Read the running task before taking a queue lock (lock order).
    let running = super::SCHEDULER.lock().current();

    let mut keep = alloc::vec::Vec::new();
    let expired = take_expired(&mut DEAD_TASKS.lock(), now);
    for TaskPtr(task_ptr) in expired {
        if Some(task_ptr) == running {
            // Still on this CPU (schedule() found nothing else to run):
            // its stack is in use, so try again later.
            keep.push((TaskPtr(task_ptr), now + REAP_DELAY_TICKS));
            continue;
        }
        // SAFETY: `exit_task` queued this pointer (from `Box::leak`) after
        // taking the task off the scheduler and the PID registry, and it is
        // not the running task. The reap delay has passed, so no CPU still
        // uses it.
        drop(unsafe { Box::from_raw(task_ptr.as_ptr()) });
    }
    if !keep.is_empty() {
        DEAD_TASKS.lock().extend(keep);
    }

    let threads = take_expired(&mut DETACHED_THREADS.lock(), now);
    for (pid, tid) in threads {
        // The process may already be gone (cleanup_process freed it).
        if let Some(process) = crate::process::table::get_process(pid) {
            let _ = crate::process::exit::cleanup_thread(&process, tid);
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod reap_tests {
    use alloc::vec;

    use super::take_expired;

    #[test]
    fn take_expired_splits_on_deadline() {
        let mut q = vec![(1u32, 10u64), (2, 5), (3, 20), (4, 10)];
        let mut got = take_expired(&mut q, 10);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2, 4]);
        assert_eq!(q, vec![(3, 20)]);
    }

    #[test]
    fn take_expired_keeps_everything_before_deadline() {
        let mut q = vec![(1u32, 10u64)];
        assert!(take_expired(&mut q, 9).is_empty());
        assert_eq!(q.len(), 1);
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
            kprintln!("[SCHED] Load balancing: migrating tasks");

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
            kprintln!("[SCHED] Successfully migrated tasks");

            // Record migration metrics
            for _ in 0..migrated {
                metrics::SCHEDULER_METRICS.record_migration();
            }
        }
    }
}

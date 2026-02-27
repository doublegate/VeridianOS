//! Read-Copy-Update (RCU) Synchronization
//!
//! RCU provides extremely fast read-side access to shared data structures
//! without locks. Readers proceed without synchronization overhead while
//! writers create copies, update pointers atomically, and reclaim old
//! versions after a grace period.
//!
//! This implementation uses a simplified epoch-based reclamation scheme
//! suitable for a microkernel with a small number of CPUs:
//!
//! - Readers call `rcu_read_lock()` / `rcu_read_unlock()` to mark critical
//!   sections (these compile to atomic counter increments, no actual locks).
//! - Writers call `synchronize_rcu()` to wait for all pre-existing readers to
//!   complete, or `call_rcu()` to defer cleanup to a callback.
//! - Grace period detection uses per-CPU counters: when all CPUs have passed
//!   through a quiescent state (context switch or explicit `rcu_quiescent()`),
//!   the grace period is complete.

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use spin::Mutex;

// ---------------------------------------------------------------------------
// Per-CPU RCU state
// ---------------------------------------------------------------------------

/// Maximum number of CPUs for RCU tracking.
const RCU_MAX_CPUS: usize = 16;

/// Per-CPU nesting depth of RCU read-side critical sections.
///
/// When > 0, the CPU is inside an RCU read-side critical section and
/// cannot be considered quiescent.
#[allow(clippy::declare_interior_mutable_const)]
static RCU_NESTING: [AtomicUsize; RCU_MAX_CPUS] = {
    const INIT: AtomicUsize = AtomicUsize::new(0);
    [INIT; RCU_MAX_CPUS]
};

/// Global RCU grace period counter. Incremented each time a grace period
/// completes. Writers snapshot this before waiting; when all CPUs have
/// observed a quiescent state since the snapshot, the grace period is done.
static RCU_GP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Per-CPU last-observed grace period. Updated each time a CPU passes
/// through a quiescent state.
#[allow(clippy::declare_interior_mutable_const)]
static RCU_CPU_GP: [AtomicU64; RCU_MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; RCU_MAX_CPUS]
};

// ---------------------------------------------------------------------------
// Deferred callback queue
// ---------------------------------------------------------------------------

/// A deferred cleanup callback registered via `call_rcu()`.
struct RcuCallback {
    /// The grace period after which this callback can execute.
    target_gp: u64,
    /// The callback function.
    func: Box<dyn FnOnce() + Send>,
}

/// Queue of deferred RCU callbacks.
static RCU_CALLBACKS: Mutex<Vec<RcuCallback>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------
// Reader API
// ---------------------------------------------------------------------------

/// Enter an RCU read-side critical section.
///
/// This is extremely lightweight: a single atomic increment. No locks,
/// no memory barriers beyond the atomic ordering.
///
/// Must be paired with `rcu_read_unlock()`. Nesting is supported.
#[inline]
pub fn rcu_read_lock() {
    let cpu = current_cpu();
    RCU_NESTING[cpu].fetch_add(1, Ordering::Relaxed);
}

/// Exit an RCU read-side critical section.
#[inline]
pub fn rcu_read_unlock() {
    let cpu = current_cpu();
    let prev = RCU_NESTING[cpu].fetch_sub(1, Ordering::Relaxed);
    debug_assert!(prev > 0, "rcu_read_unlock without matching rcu_read_lock");
}

/// Check whether the current CPU is inside an RCU read-side critical section.
pub fn rcu_is_reading() -> bool {
    let cpu = current_cpu();
    RCU_NESTING[cpu].load(Ordering::Relaxed) > 0
}

// ---------------------------------------------------------------------------
// Writer API
// ---------------------------------------------------------------------------

/// Wait for all pre-existing RCU read-side critical sections to complete.
///
/// After this function returns, it is safe to free memory that was visible
/// to readers before the call. This is the synchronous grace period wait.
///
/// On a single-CPU system, a single quiescent state check is sufficient.
/// On SMP, we spin until all CPUs have reported a quiescent state.
pub fn synchronize_rcu() {
    let target_gp = RCU_GP_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;

    // Wait for all CPUs to observe the new grace period.
    // On a single-CPU system, if we're not in an RCU read section, we're
    // already past a quiescent point.
    loop {
        let mut all_quiescent = true;
        for cpu in 0..RCU_MAX_CPUS {
            // A CPU is quiescent if it has no active read-side sections
            // AND has observed the current grace period.
            let nesting = RCU_NESTING[cpu].load(Ordering::Acquire);
            let cpu_gp = RCU_CPU_GP[cpu].load(Ordering::Acquire);

            if nesting > 0 || cpu_gp < target_gp {
                // CPU 0 is always online; others may not exist.
                // If a CPU doesn't exist, it's vacuously quiescent.
                if cpu == 0 || crate::sched::smp::per_cpu(cpu as u8).is_some() {
                    all_quiescent = false;
                    break;
                }
            }
        }

        if all_quiescent {
            break;
        }

        // Yield to other tasks while waiting.
        core::hint::spin_loop();
    }

    // Process any deferred callbacks that are now ready.
    process_callbacks(target_gp);
}

/// Register a deferred callback to be called after the next grace period.
///
/// The callback will be invoked after all CPUs have passed through a
/// quiescent state following this call. The callback must be `Send`
/// since it may execute on a different CPU.
pub fn call_rcu<F: FnOnce() + Send + 'static>(func: F) {
    let target_gp = RCU_GP_COUNTER.load(Ordering::Relaxed) + 1;
    let mut callbacks = RCU_CALLBACKS.lock();
    callbacks.push(RcuCallback {
        target_gp,
        func: Box::new(func),
    });
}

// ---------------------------------------------------------------------------
// Quiescent State Reporting
// ---------------------------------------------------------------------------

/// Report that the current CPU has passed through a quiescent state.
///
/// Called from the scheduler during context switch, timer tick, or
/// explicit idle. A CPU in a quiescent state is not holding any RCU
/// read-side references from before the current grace period.
pub fn rcu_quiescent() {
    let cpu = current_cpu();
    let nesting = RCU_NESTING[cpu].load(Ordering::Relaxed);
    if nesting == 0 {
        let current_gp = RCU_GP_COUNTER.load(Ordering::Acquire);
        RCU_CPU_GP[cpu].store(current_gp, Ordering::Release);
    }
}

/// Process deferred callbacks whose grace periods have completed.
fn process_callbacks(completed_gp: u64) {
    let mut callbacks = RCU_CALLBACKS.lock();
    let mut i = 0;
    while i < callbacks.len() {
        if callbacks[i].target_gp <= completed_gp {
            let cb = callbacks.swap_remove(i);
            // Release lock before executing callback to avoid deadlock.
            drop(callbacks);
            (cb.func)();
            callbacks = RCU_CALLBACKS.lock();
            // Don't increment i since swap_remove moved the last element here.
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the current CPU ID (0 on single-CPU systems).
fn current_cpu() -> usize {
    crate::sched::smp::current_cpu_id() as usize
}

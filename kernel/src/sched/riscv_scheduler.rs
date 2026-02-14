//! RISC-V specific scheduler wrapper to bypass spin lock issues

use core::{cell::UnsafeCell, sync::atomic::AtomicBool};

use super::scheduler::Scheduler;

/// RISC-V safe scheduler wrapper
pub struct RiscvScheduler {
    inner: UnsafeCell<Scheduler>,
    _lock_flag: AtomicBool,
}

impl RiscvScheduler {
    /// Create new scheduler
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(Scheduler::new()),
            _lock_flag: AtomicBool::new(false),
        }
    }

    /// Get a "lock" that just returns the inner scheduler
    /// This bypasses the spin lock issue during bootstrap
    pub fn lock(&self) -> RiscvSchedulerGuard<'_> {
        // For RISC-V, we bypass the lock during early boot
        // This is safe during single-threaded bootstrap
        RiscvSchedulerGuard {
            // SAFETY: During RISC-V early boot, only a single hart (CPU) is
            // active and interrupts are disabled, so there is no concurrent
            // access to the inner Scheduler. The UnsafeCell is used to bypass
            // spin::Mutex which causes deadlocks during RISC-V bootstrap.
            // After SMP initialization, proper per-CPU schedulers should be
            // used instead.
            scheduler: unsafe { &mut *self.inner.get() },
        }
    }
}

/// Guard for RISC-V scheduler
pub struct RiscvSchedulerGuard<'a> {
    scheduler: &'a mut Scheduler,
}

impl core::ops::Deref for RiscvSchedulerGuard<'_> {
    type Target = Scheduler;

    fn deref(&self) -> &Self::Target {
        self.scheduler
    }
}

impl core::ops::DerefMut for RiscvSchedulerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.scheduler
    }
}

// SAFETY: RiscvScheduler uses UnsafeCell internally but is only accessed
// during single-hart bootstrap on RISC-V where no concurrent access occurs.
// The _lock_flag AtomicBool provides a synchronization primitive if needed
// after SMP initialization.
unsafe impl Send for RiscvScheduler {}
// SAFETY: Same as Send -- the scheduler is accessed through the lock() method
// which returns a guard, and during early boot only one hart is active.
unsafe impl Sync for RiscvScheduler {}

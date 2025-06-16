//! RISC-V specific scheduler wrapper to bypass spin lock issues

use core::cell::UnsafeCell;
use core::sync::atomic::AtomicBool;
use super::scheduler::Scheduler;

/// RISC-V safe scheduler wrapper
pub struct RiscvScheduler {
    inner: UnsafeCell<Scheduler>,
    _lock_flag: AtomicBool,
}

impl RiscvScheduler {
    /// Create new scheduler
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
            scheduler: unsafe { &mut *self.inner.get() }
        }
    }
}

/// Guard for RISC-V scheduler
pub struct RiscvSchedulerGuard<'a> {
    scheduler: &'a mut Scheduler,
}

impl<'a> core::ops::Deref for RiscvSchedulerGuard<'a> {
    type Target = Scheduler;
    
    fn deref(&self) -> &Self::Target {
        self.scheduler
    }
}

impl<'a> core::ops::DerefMut for RiscvSchedulerGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.scheduler
    }
}

unsafe impl Send for RiscvScheduler {}
unsafe impl Sync for RiscvScheduler {}
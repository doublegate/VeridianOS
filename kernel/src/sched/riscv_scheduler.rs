//! RISC-V specific scheduler wrapper using spin::Mutex for proper
//! synchronization

use spin::Mutex;

use super::scheduler::Scheduler;

/// RISC-V safe scheduler wrapper using spin::Mutex for proper locking.
///
/// Previously used UnsafeCell with manual Send/Sync impls, which was
/// unsound because it returned &mut references without synchronization.
/// Now delegates to spin::Mutex which provides correct locking.
pub struct RiscvScheduler {
    inner: Mutex<Scheduler>,
}

impl RiscvScheduler {
    /// Create new scheduler
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(Scheduler::new()),
        }
    }

    /// Acquire the scheduler lock and return a guard that derefs to Scheduler.
    pub fn lock(&self) -> spin::MutexGuard<'_, Scheduler> {
        self.inner.lock()
    }
}

impl Default for RiscvScheduler {
    fn default() -> Self {
        Self::new()
    }
}

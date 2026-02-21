//! Process Wait/Exit Infrastructure
//!
//! Provides `waitpid`-style semantics for parent processes to wait on children.
//! Implements a global wait queue with notification and zombie collection.

// waitpid infrastructure -- exercised via SYS_WAIT syscall
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use spin::Mutex;

use super::{pcb::ProcessState, ProcessId};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Wait Options
// ---------------------------------------------------------------------------

/// Options controlling `waitpid` behavior, modeled after POSIX flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaitOptions {
    flags: u32,
}

impl WaitOptions {
    /// Do not block if no child has changed state.
    pub const WNOHANG: u32 = 1;
    /// Also return if a child has stopped (traced or SIGSTOP).
    pub const WUNTRACED: u32 = 2;
    /// Also return if a stopped child has been resumed by SIGCONT.
    pub const WCONTINUED: u32 = 8;

    /// Create options from raw flags.
    pub fn from_flags(flags: u32) -> Self {
        Self { flags }
    }

    /// Check whether WNOHANG is set.
    pub fn is_nohang(&self) -> bool {
        self.flags & Self::WNOHANG != 0
    }

    /// Check whether WUNTRACED is set.
    pub fn is_untraced(&self) -> bool {
        self.flags & Self::WUNTRACED != 0
    }

    /// Check whether WCONTINUED is set.
    pub fn is_continued(&self) -> bool {
        self.flags & Self::WCONTINUED != 0
    }
}

// ---------------------------------------------------------------------------
// Wait Status
// ---------------------------------------------------------------------------

/// Status returned by `waitpid` describing how a child changed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitStatus {
    /// Child exited normally with the given status code.
    Exited(i32),
    /// Child was terminated by a signal.
    Signaled(i32),
    /// Child was stopped by a signal (only with WUNTRACED).
    Stopped(i32),
    /// Child was resumed by SIGCONT (only with WCONTINUED).
    Continued,
}

impl WaitStatus {
    /// Encode the status as a raw `i32` matching POSIX `wstatus` layout.
    ///
    /// - Exited: `(code & 0xFF) << 8`
    /// - Signaled: `signum & 0x7F`
    /// - Stopped: `0x7F | (signum << 8)`
    /// - Continued: `0xFFFF`
    pub fn to_raw(self) -> i32 {
        match self {
            Self::Exited(code) => (code & 0xFF) << 8,
            Self::Signaled(sig) => sig & 0x7F,
            Self::Stopped(sig) => 0x7F | (sig << 8),
            Self::Continued => 0xFFFF_u16 as i32,
        }
    }
}

// ---------------------------------------------------------------------------
// Wait Entry
// ---------------------------------------------------------------------------

/// A single entry in a wait queue tracking a parent waiting for children.
#[derive(Debug, Clone, Copy)]
struct WaitEntry {
    /// The PID of the waiting parent.
    waiter_pid: ProcessId,
    /// The specific child PID being waited for, or `None` for any child.
    target_pid: Option<ProcessId>,
    /// Options controlling wait behavior.
    options: WaitOptions,
}

// ---------------------------------------------------------------------------
// Wait Queue
// ---------------------------------------------------------------------------

/// Global wait queue tracking which parents are waiting for children.
///
/// Keyed by the parent PID, each entry contains a list of outstanding waits.
#[cfg(feature = "alloc")]
struct WaitQueue {
    /// Map from parent PID to list of wait entries.
    entries: BTreeMap<ProcessId, Vec<WaitEntry>>,
}

#[cfg(feature = "alloc")]
impl WaitQueue {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Register a wait entry for a parent process.
    fn register(&mut self, entry: WaitEntry) {
        self.entries
            .entry(entry.waiter_pid)
            .or_default()
            .push(entry);
    }

    /// Remove all wait entries for a parent process.
    fn remove_waiter(&mut self, parent_pid: ProcessId) {
        self.entries.remove(&parent_pid);
    }

    /// Check whether a parent has any outstanding waits.
    fn has_waiter(&self, parent_pid: ProcessId) -> bool {
        self.entries.get(&parent_pid).is_some_and(|v| !v.is_empty())
    }
}

/// Global wait queue instance.
#[cfg(feature = "alloc")]
static WAIT_QUEUE: Mutex<WaitQueue> = Mutex::new(WaitQueue::new());

// ---------------------------------------------------------------------------
// System Call: waitpid
// ---------------------------------------------------------------------------

/// Wait for a child process to change state.
///
/// # Arguments
/// * `pid` - Process to wait for:
///   - `pid > 0`: Wait for the specific child with that PID.
///   - `pid == -1`: Wait for any child process.
///   - Other negative values are reserved (currently treated as any child).
/// * `options` - [`WaitOptions`] controlling blocking and status filters.
///
/// # Returns
/// A tuple of `(child_pid, status)` on success.
#[cfg(feature = "alloc")]
pub fn sys_waitpid(pid: i64, options: WaitOptions) -> Result<(ProcessId, WaitStatus), KernelError> {
    let current = super::current_process().ok_or(KernelError::NotInitialized {
        subsystem: "current process",
    })?;
    let parent_pid = current.pid;

    // Determine which child we are waiting for.
    let target: Option<ProcessId> = if pid > 0 {
        Some(ProcessId(pid as u64))
    } else {
        None // -1 or other negative => any child.
    };

    loop {
        // Scan children for one matching the target that has changed state.
        let children = super::table::PROCESS_TABLE.find_children(parent_pid);

        if children.is_empty() {
            return Err(KernelError::NotFound {
                resource: "child process",
                id: 0,
            });
        }

        let mut target_exists = false;

        for child_pid in &children {
            if let Some(target_pid) = target {
                if *child_pid != target_pid {
                    continue;
                }
            }
            target_exists = true;

            if let Some(child) = super::table::get_process(*child_pid) {
                let state = child.get_state();

                // Zombie => child has exited.
                if state == ProcessState::Zombie {
                    let exit_code = child.get_exit_code();
                    let status = WaitStatus::Exited(exit_code);

                    // Collect the zombie.
                    collect_zombie(*child_pid, parent_pid)?;

                    return Ok((*child_pid, status));
                }

                // Stopped child (WUNTRACED).
                if options.is_untraced() && state == ProcessState::Blocked {
                    return Ok((*child_pid, WaitStatus::Stopped(19))); // SIGSTOP
                                                                      // = 19
                }

                // Continued child (WCONTINUED).
                if options.is_continued()
                    && (state == ProcessState::Running || state == ProcessState::Ready)
                {
                    return Ok((*child_pid, WaitStatus::Continued));
                }
            }
        }

        // If we asked for a specific child that does not exist, error out.
        if let Some(target_pid) = target {
            if !target_exists {
                return Err(KernelError::ProcessNotFound { pid: target_pid.0 });
            }
        }

        // WNOHANG: return immediately.
        if options.is_nohang() {
            return Err(KernelError::WouldBlock);
        }

        // Register in the wait queue and block.
        {
            let mut wq = WAIT_QUEUE.lock();
            wq.register(WaitEntry {
                waiter_pid: parent_pid,
                target_pid: target,
                options,
            });
        }

        // Block the current process. When a child exits, `notify_parent` will
        // wake us up and we will loop back to check again.
        current.set_state(ProcessState::Blocked);
        crate::sched::yield_cpu();
        current.set_state(ProcessState::Running);

        // If a signal interrupted us, return early.
        if let Some(signum) = current.get_next_pending_signal() {
            current.clear_pending_signal(signum);
            // Clean up wait queue entry.
            WAIT_QUEUE.lock().remove_waiter(parent_pid);
            return Err(KernelError::WouldBlock);
        }
    }
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Notify a parent process that a child has changed state.
///
/// Called from the child exit path to wake the parent if it is blocked in
/// `waitpid`. Also sends SIGCHLD to the parent.
#[cfg(feature = "alloc")]
pub fn notify_parent(child_pid: ProcessId, status: WaitStatus) {
    // Find the child's parent.
    let parent_pid = if let Some(child) = super::table::get_process(child_pid) {
        child.parent
    } else {
        None
    };

    let parent_pid = match parent_pid {
        Some(pid) => pid,
        None => return, // No parent (e.g., init process).
    };

    let _ = status; // Status is implicit in the child's state.

    // Send SIGCHLD to the parent.
    if let Some(parent) = super::table::get_process(parent_pid) {
        use super::exit::signals::SIGCHLD;
        if let Err(_e) = parent.send_signal(SIGCHLD as usize) {
            crate::kprintln!("[PROCESS] Warning: Failed to send SIGCHLD to parent");
        }

        // Wake parent if blocked.
        if parent.get_state() == ProcessState::Blocked {
            parent.set_state(ProcessState::Ready);
            crate::sched::wake_up_process(parent_pid);
        }
    }

    // Remove wait queue entries for this parent (they will re-register if
    // they loop).
    WAIT_QUEUE.lock().remove_waiter(parent_pid);
}

// ---------------------------------------------------------------------------
// Zombie Collection
// ---------------------------------------------------------------------------

/// Clean up a zombie process after a successful wait.
///
/// Removes the child from the parent's children list and from the global
/// process table.
#[cfg(feature = "alloc")]
pub fn collect_zombie(child_pid: ProcessId, parent_pid: ProcessId) -> Result<(), KernelError> {
    // Remove from parent's children list.
    if let Some(parent) = super::table::get_process(parent_pid) {
        parent.children.lock().retain(|&p| p != child_pid);
    }

    // Remove from the process table.
    super::table::remove_process(child_pid);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- WaitOptions tests ---

    #[test]
    fn test_wait_options_default() {
        let opts = WaitOptions::default();
        assert!(!opts.is_nohang());
        assert!(!opts.is_untraced());
        assert!(!opts.is_continued());
    }

    #[test]
    fn test_wait_options_nohang() {
        let opts = WaitOptions::from_flags(WaitOptions::WNOHANG);
        assert!(opts.is_nohang());
        assert!(!opts.is_untraced());
        assert!(!opts.is_continued());
    }

    #[test]
    fn test_wait_options_combined() {
        let opts = WaitOptions::from_flags(WaitOptions::WNOHANG | WaitOptions::WUNTRACED);
        assert!(opts.is_nohang());
        assert!(opts.is_untraced());
        assert!(!opts.is_continued());
    }

    #[test]
    fn test_wait_options_all_flags() {
        let opts = WaitOptions::from_flags(
            WaitOptions::WNOHANG | WaitOptions::WUNTRACED | WaitOptions::WCONTINUED,
        );
        assert!(opts.is_nohang());
        assert!(opts.is_untraced());
        assert!(opts.is_continued());
    }

    // --- WaitStatus tests ---

    #[test]
    fn test_wait_status_exited() {
        let status = WaitStatus::Exited(42);
        assert_eq!(status, WaitStatus::Exited(42));
        // Raw encoding: (42 & 0xFF) << 8 = 42 << 8 = 10752
        assert_eq!(status.to_raw(), 42 << 8);
    }

    #[test]
    fn test_wait_status_signaled() {
        let status = WaitStatus::Signaled(11); // SIGSEGV
        assert_eq!(status.to_raw(), 11);
    }

    #[test]
    fn test_wait_status_stopped() {
        let status = WaitStatus::Stopped(19); // SIGSTOP
                                              // Raw: 0x7F | (19 << 8) = 127 | 4864 = 4991
        assert_eq!(status.to_raw(), 0x7F | (19 << 8));
    }

    #[test]
    fn test_wait_status_continued() {
        let status = WaitStatus::Continued;
        assert_eq!(status.to_raw(), 0xFFFF_u16 as i32);
    }

    #[test]
    fn test_wait_status_equality() {
        assert_eq!(WaitStatus::Exited(0), WaitStatus::Exited(0));
        assert_ne!(WaitStatus::Exited(0), WaitStatus::Exited(1));
        assert_ne!(WaitStatus::Exited(0), WaitStatus::Continued);
    }
}

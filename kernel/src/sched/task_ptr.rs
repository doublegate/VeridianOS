//! Safe wrapper for task pointers

use core::ptr::NonNull;

use super::task::Task;

/// A wrapper around NonNull<Task> that implements Send and Sync
///
/// # Safety
/// This is safe because:
/// 1. Tasks are only accessed with proper synchronization (scheduler lock)
/// 2. The scheduler ensures exclusive access during context switches
/// 3. Task memory is managed by the kernel and won't be deallocated while
///    referenced
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskPtr(NonNull<Task>);

impl TaskPtr {
    /// Create a new TaskPtr from a NonNull<Task>
    pub fn new(ptr: NonNull<Task>) -> Self {
        Self(ptr)
    }

    /// Get the underlying NonNull<Task>
    pub fn as_ptr(&self) -> NonNull<Task> {
        self.0
    }

    /// Get a raw pointer to the task
    pub fn as_raw(&self) -> *mut Task {
        self.0.as_ptr()
    }
}

// Safety: Tasks are only modified by the owning CPU with interrupts disabled
unsafe impl Send for TaskPtr {}
unsafe impl Sync for TaskPtr {}

impl From<NonNull<Task>> for TaskPtr {
    fn from(ptr: NonNull<Task>) -> Self {
        Self::new(ptr)
    }
}

impl From<TaskPtr> for NonNull<Task> {
    fn from(ptr: TaskPtr) -> Self {
        ptr.0
    }
}

//! Async Runtime Type Definitions for VeridianOS
//!
//! Defines the kernel-side contract for user-space async runtime primitives.
//! These types describe task states, priorities, channels, and timers that the
//! kernel's scheduler exposes to user-space async runtimes.
//!
//! TODO(user-space): The actual async runtime implementation requires
//! user-space process execution. This module provides the type definitions
//! that both kernel scheduler primitives and user-space runtimes agree upon.

// User-space async runtime contract -- see TODO(user-space) above
#![allow(dead_code)]

// ============================================================================
// TaskState
// ============================================================================

/// Execution state of an asynchronous task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Task has been created but is waiting to be scheduled.
    Pending,
    /// Task is currently executing on a CPU.
    Running,
    /// Task has finished successfully.
    Completed,
    /// Task was cancelled before completion.
    Cancelled,
    /// Task encountered an error during execution.
    Failed,
}

impl TaskState {
    /// Returns `true` if this state is terminal (the task will not run again).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

// ============================================================================
// TaskPriority
// ============================================================================

/// Priority level for an asynchronous task.
///
/// Higher priority tasks are scheduled before lower priority tasks when
/// contending for CPU time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Background tasks with the lowest scheduling priority.
    Low,
    /// Default priority for most tasks.
    Normal,
    /// Elevated priority for latency-sensitive tasks.
    High,
    /// Highest priority for time-critical operations.
    Critical,
}

impl TaskPriority {
    /// Return a numeric priority level (0 = Low, 3 = Critical).
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
}

// ============================================================================
// TaskHandle
// ============================================================================

/// Handle to a scheduled asynchronous task.
///
/// Combines the task's unique identifier with its current state and priority,
/// allowing the caller to inspect and manage task lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskHandle {
    /// Unique task identifier.
    pub id: u64,
    /// Current execution state.
    pub state: TaskState,
    /// Scheduling priority.
    pub priority: TaskPriority,
}

impl TaskHandle {
    /// Create a new task handle in the `Pending` state with `Normal` priority.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            state: TaskState::Pending,
            priority: TaskPriority::Normal,
        }
    }

    /// Create a new task handle in the `Pending` state with the given priority.
    pub fn with_priority(id: u64, priority: TaskPriority) -> Self {
        Self {
            id,
            state: TaskState::Pending,
            priority,
        }
    }

    /// Returns `true` if the task has reached a terminal state.
    pub fn is_done(&self) -> bool {
        self.state.is_terminal()
    }
}

// ============================================================================
// ChannelConfig
// ============================================================================

/// Configuration for an asynchronous communication channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelConfig {
    /// Maximum number of messages the channel can buffer.
    pub capacity: usize,
    /// Whether this channel supports broadcast (one-to-many) delivery.
    pub allow_broadcast: bool,
}

impl ChannelConfig {
    /// Create a point-to-point channel configuration with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            allow_broadcast: false,
        }
    }

    /// Create a broadcast-capable channel configuration with the given
    /// capacity.
    pub fn with_broadcast(capacity: usize) -> Self {
        Self {
            capacity,
            allow_broadcast: true,
        }
    }
}

// ============================================================================
// TimerMode / TimerSpec
// ============================================================================

/// Determines whether a timer fires once or repeatedly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerMode {
    /// Fire once after the specified duration.
    OneShot,
    /// Fire repeatedly at the specified interval.
    Repeating,
}

/// Specification for an asynchronous timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerSpec {
    /// Duration or interval in milliseconds.
    pub duration_ms: u64,
    /// Whether this timer fires once or repeats.
    pub mode: TimerMode,
}

impl TimerSpec {
    /// Create a one-shot timer that fires after the given duration.
    pub fn one_shot(duration_ms: u64) -> Self {
        Self {
            duration_ms,
            mode: TimerMode::OneShot,
        }
    }

    /// Create a repeating timer that fires at the given interval.
    pub fn repeating(duration_ms: u64) -> Self {
        Self {
            duration_ms,
            mode: TimerMode::Repeating,
        }
    }
}

// ============================================================================
// AsyncRuntimeConfig
// ============================================================================

/// Configuration for a user-space async runtime instance.
///
/// Provides tuning parameters that the kernel uses when allocating scheduler
/// resources for a process's async runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsyncRuntimeConfig {
    /// Maximum number of concurrent async tasks.
    pub max_tasks: usize,
    /// Default priority for newly spawned tasks.
    pub default_priority: TaskPriority,
    /// Timer resolution in milliseconds (minimum timer granularity).
    pub timer_resolution_ms: u64,
}

impl AsyncRuntimeConfig {
    /// Create a configuration with sensible defaults.
    ///
    /// Defaults: 256 max tasks, `Normal` priority, 1 ms timer resolution.
    pub fn new() -> Self {
        Self {
            max_tasks: 256,
            default_priority: TaskPriority::Normal,
            timer_resolution_ms: 1,
        }
    }

    /// Return a new configuration with the given maximum task count.
    pub fn with_max_tasks(mut self, max_tasks: usize) -> Self {
        self.max_tasks = max_tasks;
        self
    }
}

impl Default for AsyncRuntimeConfig {
    fn default() -> Self {
        Self::new()
    }
}

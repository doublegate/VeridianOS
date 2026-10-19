//! Synchronization Primitives
//!
//! Safe synchronization and global state management for Rust 2024 edition.
//!
//! Includes lock-free data structures for high-performance kernel paths:
//! - RCU (Read-Copy-Update) for read-heavy data structures
//! - Hazard pointers for safe memory reclamation
//!
//! (A lock-free queue was removed: nothing used it, and its safe `pop` let
//! two consumers race though it supports one; PR #25 review, N-59.)

pub mod hazard;
pub mod once_lock;
pub mod rcu;
pub mod sleep_mutex;

pub use once_lock::{GlobalState, LazyLock, OnceLock};

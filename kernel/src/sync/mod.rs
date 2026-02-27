//! Synchronization Primitives
//!
//! Safe synchronization and global state management for Rust 2024 edition.
//!
//! Includes lock-free data structures for high-performance kernel paths:
//! - RCU (Read-Copy-Update) for read-heavy data structures
//! - Hazard pointers for safe memory reclamation
//! - Lock-free MPSC queue for scheduler ready queues

pub mod hazard;
pub mod lockfree_queue;
pub mod once_lock;
pub mod rcu;

pub use lockfree_queue::LockFreeQueue;
pub use once_lock::{GlobalState, LazyLock, OnceLock};

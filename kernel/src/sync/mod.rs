//! Synchronization Primitives
//!
//! Safe synchronization and global state management for Rust 2024 edition.

pub mod once_lock;

pub use once_lock::{OnceLock, LazyLock, GlobalState};

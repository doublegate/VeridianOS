//! User-space support module
//!
//! This module provides support for loading and executing user-space programs,
//! including the init process and other user applications.

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod enhanced_loader;
pub mod loader;

pub use loader::{load_init_process, load_user_program};

/// Initialize user-space support
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;
    println!("[USERSPACE] Initializing user-space support...");

    // User-space support is ready
    println!("[USERSPACE] User-space support initialized");
}

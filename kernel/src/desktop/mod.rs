//! Desktop subsystem
//!
//! Provides desktop environment functionality including font rendering,
//! window management, and graphical user interface components.

pub mod font;
pub mod window_manager;

use crate::error::KernelError;

/// Initialize the desktop subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[DESKTOP] Initializing desktop subsystem...");

    // Initialize font rendering
    font::init()?;

    // Initialize window manager
    window_manager::init()?;

    println!("[DESKTOP] Desktop subsystem initialized");
    Ok(())
}

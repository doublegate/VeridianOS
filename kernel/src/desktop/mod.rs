//! Desktop subsystem
//!
//! Provides desktop environment functionality including font rendering,
//! window management, and graphical user interface components.

pub mod font;

use crate::error::KernelError;

/// Initialize the desktop subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[DESKTOP] Initializing desktop subsystem...");

    // Initialize font rendering
    font::init()?;

    println!("[DESKTOP] Desktop subsystem initialized");
    Ok(())
}

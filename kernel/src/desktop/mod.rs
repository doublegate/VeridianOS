//! Desktop subsystem
//!
//! Provides desktop environment functionality including font rendering,
//! window management, and graphical user interface components.

pub mod font;
pub mod window_manager;
pub mod wayland;
pub mod terminal;
pub mod file_manager;
pub mod text_editor;

use crate::error::KernelError;

/// Initialize the desktop subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[DESKTOP] Initializing desktop subsystem...");

    // Initialize font rendering
    font::init()?;

    // Initialize window manager
    window_manager::init()?;

    // Initialize terminal system
    terminal::init()?;

    // Initialize file manager
    file_manager::init()?;

    // Initialize text editor
    text_editor::init()?;

    println!("[DESKTOP] Desktop subsystem initialized");
    Ok(())
}

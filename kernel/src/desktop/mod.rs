//! Desktop subsystem
//!
//! Provides desktop environment functionality including font rendering,
//! window management, and graphical user interface components.

pub mod animation;
pub mod app_switcher;
pub mod file_manager;
pub mod font;
pub mod image_viewer;
#[allow(unused)]
pub mod launcher;
pub mod mime;
pub mod notification;
pub mod panel;
pub mod renderer;
pub mod screen_lock;
pub mod settings;
pub mod syntax;
pub mod systray;
pub mod terminal;
pub mod text_editor;
pub mod wayland;
pub mod window_manager;
pub mod xwayland;

use crate::error::KernelError;

/// Initialize the desktop subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[DESKTOP] Initializing desktop subsystem...");

    // Initialize font rendering
    font::init()?;

    // Initialize Wayland compositor
    wayland::init()?;

    // Initialize window manager
    window_manager::init()?;

    // Initialize terminal system
    terminal::init()?;

    // Initialize file manager
    file_manager::init()?;

    // Initialize text editor
    text_editor::init()?;

    // Initialize system tray
    systray::init();

    // Initialize application launcher
    let _ = launcher::init();

    println!("[DESKTOP] Desktop subsystem initialized");
    Ok(())
}

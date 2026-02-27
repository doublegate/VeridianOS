//! Window compositor
//!
//! Provides a higher-level window abstraction on top of the Wayland surface
//! model. Each `Window` maps to a Wayland surface with an xdg_toplevel role.
//! The compositor maintains a window list, focus tracking, and coordinates
//! rendering by delegating to the Wayland compositor's compositing engine.

use alloc::vec::Vec;

use spin::RwLock;

use super::Rect;
use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Window handle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowId(pub u32);

/// Window
#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub rect: Rect,
    pub title: &'static str,
    pub visible: bool,
    pub focused: bool,
    /// Associated Wayland surface ID (0 if none)
    #[allow(dead_code)] // Phase 6: links Window to Wayland surface for rendering
    pub surface_id: u32,
}

impl Window {
    pub fn new(id: WindowId, rect: Rect, title: &'static str) -> Self {
        Self {
            id,
            rect,
            title,
            visible: true,
            focused: false,
            surface_id: 0,
        }
    }
}

/// Compositor state
pub struct Compositor {
    windows: Vec<Window>,
    next_id: u32,
    focused_window: Option<WindowId>,
    /// Desktop background color (ARGB packed u32)
    #[allow(dead_code)] // Phase 6: used by render() when drawing desktop
    bg_color: u32,
}

impl Compositor {
    pub const fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_id: 1,
            focused_window: None,
            bg_color: 0xFF2D_3436,
        }
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    /// Create a new window
    pub fn create_window(&mut self, rect: Rect, title: &'static str) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;

        let window = Window::new(id, rect, title);
        self.windows.push(window);

        if self.focused_window.is_none() {
            self.focused_window = Some(id);
        }

        id
    }

    /// Destroy a window
    pub fn destroy_window(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
        if self.focused_window == Some(id) {
            self.focused_window = self.windows.first().map(|w| w.id);
        }
    }

    /// Get window
    pub fn get_window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Focus window
    pub fn focus_window(&mut self, id: WindowId) {
        for window in &mut self.windows {
            window.focused = window.id == id;
        }
        self.focused_window = Some(id);
    }

    /// Get the currently focused window ID.
    #[allow(dead_code)] // Phase 6: keyboard/pointer focus routing
    pub fn focused_window(&self) -> Option<WindowId> {
        self.focused_window
    }

    /// Number of windows.
    #[allow(dead_code)] // Phase 6: window count queries
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Render all windows.
    ///
    /// Delegates actual pixel compositing to the Wayland compositor. This
    /// method handles the window-manager level logic (visibility, ordering).
    pub fn render(&mut self) {
        // Collect visible windows -- in the future this feeds into the
        // Wayland compositor's surface Z-order.
        for _window in &self.windows {
            // Window rendering is handled by Wayland compositor composite()
        }
    }
}

static COMPOSITOR: GlobalState<RwLock<Compositor>> = GlobalState::new();

/// Execute a function with the compositor
pub fn with_compositor<R, F: FnOnce(&mut Compositor) -> R>(f: F) -> Option<R> {
    COMPOSITOR.with(|comp| {
        let mut compositor = comp.write();
        f(&mut compositor)
    })
}

/// Initialize compositor
pub fn init() -> Result<(), KernelError> {
    println!("[COMP] Initializing compositor...");

    // Try to initialize, but don't fail if already initialized
    if COMPOSITOR.init(RwLock::new(Compositor::new())).is_err() {
        // Already initialized - this is fine
        println!("[COMP] Compositor already initialized, skipping...");
        return Ok(());
    }

    // Create a test window
    with_compositor(|comp| {
        comp.create_window(
            Rect {
                x: 100,
                y: 100,
                width: 640,
                height: 480,
            },
            "VeridianOS",
        )
    });

    println!("[COMP] Compositor initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compositor_create_window() {
        let mut comp = Compositor::new();
        let id = comp.create_window(
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            "Test",
        );
        assert!(comp.get_window(id).is_some());
    }

    #[test]
    fn test_compositor_destroy_window() {
        let mut comp = Compositor::new();
        let id = comp.create_window(
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            "Test",
        );
        comp.destroy_window(id);
        assert!(comp.get_window(id).is_none());
    }
}

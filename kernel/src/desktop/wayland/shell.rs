//! XDG Shell
//!
//! Desktop shell protocol (windows, popups, etc.)


/// XDG toplevel (window)
pub struct XdgToplevel {
    /// Surface ID
    pub surface_id: u32,
    /// Window title
    pub title: alloc::string::String,
    /// App ID
    pub app_id: alloc::string::String,
    /// Window state
    pub state: WindowState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Maximized,
    Fullscreen,
    Minimized,
}

impl XdgToplevel {
    pub fn new(surface_id: u32) -> Self {
        Self {
            surface_id,
            title: alloc::string::String::new(),
            app_id: alloc::string::String::new(),
            state: WindowState::Normal,
        }
    }

    pub fn set_title(&mut self, title: alloc::string::String) {
        self.title = title;
    }

    pub fn set_maximized(&mut self) {
        self.state = WindowState::Maximized;
    }

    pub fn set_fullscreen(&mut self) {
        self.state = WindowState::Fullscreen;
    }
}

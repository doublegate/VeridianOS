//! Browser Main Module
//!
//! Top-level browser struct that ties together the tab manager, process
//! isolation, rendering, navigation, and shell command integration.
//! Provides the public API for creating, driving, and rendering the
//! browser from the VeridianOS desktop environment.

#![allow(dead_code)]

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use super::{
    tab_isolation::{ProcessIsolation, TabCapabilities},
    tabs::{TabAction, TabId, TabManager},
};

// ---------------------------------------------------------------------------
// Browser configuration
// ---------------------------------------------------------------------------

/// Browser configuration
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    /// Default home page URL
    pub home_page: String,
    /// Viewport width in pixels
    pub viewport_width: u32,
    /// Viewport height in pixels
    pub viewport_height: u32,
    /// Tab bar height in pixels
    pub tab_bar_height: u32,
    /// Address bar height in pixels
    pub address_bar_height: u32,
    /// Whether to show the address bar
    pub show_address_bar: bool,
    /// Whether to show navigation buttons
    pub show_nav_buttons: bool,
    /// Maximum tabs
    pub max_tabs: usize,
    /// Enable JavaScript
    pub enable_js: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            home_page: "veridian://newtab".to_string(),
            viewport_width: 1024,
            viewport_height: 768,
            tab_bar_height: 32,
            address_bar_height: 36,
            show_address_bar: true,
            show_nav_buttons: true,
            max_tabs: 32,
            enable_js: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Address bar state
// ---------------------------------------------------------------------------

/// Address bar editing state
#[derive(Debug, Clone)]
pub struct AddressBar {
    /// Current text content
    pub text: String,
    /// Cursor position (byte offset)
    pub cursor: usize,
    /// Whether the address bar is focused (editing)
    pub focused: bool,
    /// Selection start (if any)
    pub selection_start: Option<usize>,
    /// Autocomplete suggestions
    pub suggestions: Vec<String>,
    /// Whether suggestions are visible
    pub showing_suggestions: bool,
}

impl Default for AddressBar {
    fn default() -> Self {
        Self::new()
    }
}

impl AddressBar {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            focused: false,
            selection_start: None,
            suggestions: Vec::new(),
            showing_suggestions: false,
        }
    }

    /// Set the URL text without triggering navigation
    pub fn set_url(&mut self, url: &str) {
        self.text = url.to_string();
        self.cursor = self.text.len();
        self.selection_start = None;
        self.showing_suggestions = false;
    }

    /// Focus the address bar and select all text
    pub fn focus(&mut self) {
        self.focused = true;
        self.selection_start = Some(0);
        self.cursor = self.text.len();
    }

    /// Unfocus the address bar
    pub fn unfocus(&mut self) {
        self.focused = false;
        self.selection_start = None;
        self.showing_suggestions = false;
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, ch: char) {
        // If there's a selection, delete it first
        if let Some(sel_start) = self.selection_start {
            let start = sel_start.min(self.cursor);
            let end = sel_start.max(self.cursor);
            self.text.drain(start..end);
            self.cursor = start;
            self.selection_start = None;
        }

        if self.cursor <= self.text.len() {
            self.text.insert(self.cursor, ch);
            self.cursor += ch.len_utf8();
        }
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) {
        if let Some(sel_start) = self.selection_start {
            let start = sel_start.min(self.cursor);
            let end = sel_start.max(self.cursor);
            self.text.drain(start..end);
            self.cursor = start;
            self.selection_start = None;
        } else if self.cursor > 0 {
            self.cursor -= 1;
            self.text.remove(self.cursor);
        }
    }

    /// Delete character after cursor
    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.selection_start = None;
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor += 1;
        }
        self.selection_start = None;
    }

    /// Move cursor to start
    pub fn home(&mut self) {
        self.cursor = 0;
        self.selection_start = None;
    }

    /// Move cursor to end
    pub fn end(&mut self) {
        self.cursor = self.text.len();
        self.selection_start = None;
    }

    /// Get the current text, potentially normalizing it as a URL
    pub fn get_navigation_url(&self) -> String {
        let trimmed = self.text.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        // If it looks like a URL, use it directly
        if trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("veridian://")
        {
            return trimmed.to_string();
        }
        // If it looks like a domain (contains a dot), add https://
        if trimmed.contains('.') && !trimmed.contains(' ') {
            return format!("https://{}", trimmed);
        }
        // Otherwise, treat as a search query (placeholder URL)
        format!("veridian://search?q={}", url_encode(trimmed))
    }
}

// ---------------------------------------------------------------------------
// Navigation button state
// ---------------------------------------------------------------------------

/// Navigation button identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavButton {
    Back,
    Forward,
    Reload,
    Home,
    Stop,
}

/// Navigation bar with buttons
pub struct NavigationBar {
    /// Button width in pixels
    pub button_width: u32,
    /// Button height in pixels
    pub button_height: u32,
    /// Spacing between buttons
    pub spacing: u32,
    /// Which button is hovered
    pub hovered: Option<NavButton>,
    /// Which button is pressed
    pub pressed: Option<NavButton>,
}

impl Default for NavigationBar {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationBar {
    pub fn new() -> Self {
        Self {
            button_width: 28,
            button_height: 28,
            spacing: 4,
            hovered: None,
            pressed: None,
        }
    }

    /// Total width of the navigation buttons area
    pub fn total_width(&self) -> u32 {
        // 4 buttons (back, forward, reload, home) + spacing
        4 * self.button_width + 3 * self.spacing
    }

    /// Hit test: which button is at this x position?
    pub fn button_at(&self, x: u32) -> Option<NavButton> {
        let stride = self.button_width + self.spacing;
        let index = x / stride;
        let offset = x % stride;
        if offset > self.button_width {
            return None; // in spacing gap
        }
        match index {
            0 => Some(NavButton::Back),
            1 => Some(NavButton::Forward),
            2 => Some(NavButton::Reload),
            3 => Some(NavButton::Home),
            _ => None,
        }
    }

    /// Render navigation buttons into a pixel buffer row
    pub fn render(
        &self,
        buf: &mut [u32],
        buf_width: usize,
        y_offset: usize,
        can_back: bool,
        can_forward: bool,
        is_loading: bool,
    ) {
        let buttons = [
            (NavButton::Back, "<", can_back),
            (NavButton::Forward, ">", can_forward),
            (
                if is_loading {
                    NavButton::Stop
                } else {
                    NavButton::Reload
                },
                if is_loading { "X" } else { "R" },
                true,
            ),
            (NavButton::Home, "H", true),
        ];

        let stride = (self.button_width + self.spacing) as usize;
        let bw = self.button_width as usize;
        let bh = self.button_height as usize;

        for (i, (btn, label, enabled)) in buttons.iter().enumerate() {
            let bx = i * stride;
            let is_hovered = self.hovered == Some(*btn);
            let is_pressed = self.pressed == Some(*btn);

            let bg = if !enabled {
                0xFF3C3C3C_u32 // disabled
            } else if is_pressed {
                0xFF555555_u32
            } else if is_hovered {
                0xFF4A4A4A_u32
            } else {
                0xFF3C3C3C_u32
            };

            let fg = if *enabled {
                0xFFFFFFFF_u32
            } else {
                0xFF606060_u32
            };

            // Draw button background
            for dy in 0..bh {
                let py = y_offset + dy;
                for dx in 0..bw {
                    let px = bx + dx;
                    if py * buf_width + px < buf.len() {
                        buf[py * buf_width + px] = bg;
                    }
                }
            }

            // Draw label (centered, single char)
            let lx = bx + bw / 2;
            let ly = y_offset + bh / 2;
            if ly * buf_width + lx < buf.len() {
                buf[ly * buf_width + lx] = fg;
            }
            // Tiny glyph approximation
            for d in 1..3_usize {
                if ly * buf_width + lx + d < buf.len() {
                    buf[ly * buf_width + lx + d] = fg;
                }
                if lx >= d && ly * buf_width + lx - d < buf.len() {
                    buf[ly * buf_width + lx - d] = fg;
                }
            }

            let _ = label; // used above symbolically
        }
    }
}

// ---------------------------------------------------------------------------
// Browser
// ---------------------------------------------------------------------------

/// The main browser struct, integrating tabs, isolation, and rendering
pub struct Browser {
    /// Configuration
    pub config: BrowserConfig,
    /// Tab manager
    pub tabs: TabManager,
    /// Process isolation
    pub isolation: ProcessIsolation,
    /// Address bar state
    pub address_bar: AddressBar,
    /// Navigation bar
    pub nav_bar: NavigationBar,
    /// Pixel buffer for the entire browser window (BGRA u32)
    pub framebuffer: Vec<u32>,
    /// Whether the browser needs a repaint
    pub needs_repaint: bool,
    /// History of visited URLs (global, for autocomplete)
    pub history: Vec<String>,
    /// Maximum history entries
    pub max_history: usize,
    /// Whether the browser is running
    pub running: bool,
    /// Status bar text
    pub status_text: String,
}

impl Default for Browser {
    fn default() -> Self {
        Self::new(BrowserConfig::default())
    }
}

impl Browser {
    /// Create a new browser with the given configuration
    pub fn new(config: BrowserConfig) -> Self {
        let fb_size = (config.viewport_width * config.viewport_height) as usize;
        Self {
            tabs: TabManager::new(),
            isolation: ProcessIsolation::new(),
            address_bar: AddressBar::new(),
            nav_bar: NavigationBar::new(),
            framebuffer: vec![0xFF2D2D30; fb_size],
            needs_repaint: true,
            history: Vec::new(),
            max_history: 1000,
            running: true,
            status_text: String::new(),
            config,
        }
    }

    /// Initialize the browser: open the home page in a new tab
    pub fn init(&mut self) {
        let url = self.config.home_page.clone();
        self.open_url(&url);
    }

    /// Open a URL in a new tab (or the current tab if it's blank)
    pub fn open_url(&mut self, url: &str) -> Option<TabId> {
        // If current tab is blank "New Tab", navigate it instead of creating new
        if let Some(active) = self.tabs.active_tab() {
            if active.url.is_empty() {
                let id = active.id;
                self.navigate_tab(id, url);
                return Some(id);
            }
        }

        // Create new tab
        let tab_id = self.tabs.new_tab(url)?;

        // Spawn isolated process
        let caps = if url.starts_with("veridian://") {
            TabCapabilities::trusted()
        } else {
            TabCapabilities::default_web()
        };
        let _ = self.isolation.spawn_with_capabilities(tab_id, caps);

        // Set origin
        if let Some(origin) = extract_origin(url) {
            if let Some(proc) = self.isolation.get_process_mut(tab_id) {
                proc.set_origin(&origin);
            }
        }

        self.update_address_bar();
        self.needs_repaint = true;
        Some(tab_id)
    }

    /// Navigate the active tab to a URL
    pub fn navigate_active(&mut self, url: &str) {
        if let Some(id) = self.tabs.active_tab_id() {
            self.navigate_tab(id, url);
        }
    }

    /// Navigate a specific tab to a URL
    pub fn navigate_tab(&mut self, tab_id: TabId, url: &str) {
        if let Some(tab) = self.tabs.get_tab_mut(tab_id) {
            tab.navigate(url);
        }

        // Update origin for process
        if let Some(origin) = extract_origin(url) {
            if let Some(proc) = self.isolation.get_process_mut(tab_id) {
                proc.set_origin(&origin);
            }
        }

        // Add to history
        if !url.is_empty() && !url.starts_with("veridian://newtab") {
            self.history.push(url.to_string());
            if self.history.len() > self.max_history {
                self.history.remove(0);
            }
        }

        // Load the page content (simplified: handle veridian:// pages inline)
        self.load_page(tab_id, url);
        self.update_address_bar();
        self.needs_repaint = true;
    }

    /// Handle the "load" of a page (in a real browser, this would be async)
    fn load_page(&mut self, tab_id: TabId, url: &str) {
        if url.starts_with("veridian://") {
            // Internal pages
            let page_html = generate_internal_page(url);
            if let Some(tab) = self.tabs.get_tab_mut(tab_id) {
                tab.set_title(&internal_page_title(url));
                tab.finish_loading();
            }

            // Execute any scripts in the page
            if let Some(proc) = self.isolation.get_process_mut(tab_id) {
                // Parse and set up DOM from HTML (simplified)
                let _doc = proc.dom_api.create_element("html");
                // Process script tags
                use super::js_integration::ScriptEngine;
                let mut engine = ScriptEngine::new();
                engine.process_script_tags(&page_html);
            }
        } else {
            // External pages would be fetched via network (stub)
            if let Some(tab) = self.tabs.get_tab_mut(tab_id) {
                tab.finish_loading();
                self.status_text = format!("Loaded: {}", url);
            }
        }
    }

    /// Handle keyboard input
    pub fn handle_key(&mut self, scancode: u8, ctrl: bool, shift: bool, alt: bool) {
        // Address bar focused: route to address bar
        if self.address_bar.focused {
            match scancode {
                0x0D => {
                    // Enter: navigate
                    let url = self.address_bar.get_navigation_url();
                    if !url.is_empty() {
                        self.address_bar.unfocus();
                        self.navigate_active(&url);
                    }
                }
                0x1B => {
                    // Escape: cancel editing
                    self.address_bar.unfocus();
                    self.update_address_bar();
                }
                0x08 => self.address_bar.backspace(),
                0x7F => self.address_bar.delete(),
                0x80 => self.address_bar.move_left(), // left arrow
                0x81 => self.address_bar.move_right(), // right arrow
                _ => {
                    if (0x20..0x7F).contains(&scancode) {
                        self.address_bar.insert_char(scancode as char);
                    }
                }
            }
            self.needs_repaint = true;
            return;
        }

        // Tab shortcuts (Ctrl+key)
        if ctrl {
            let action = TabManager::decode_shortcut(ctrl, shift, scancode);
            match action {
                TabAction::NewTab => {
                    self.open_url("");
                    self.address_bar.focus();
                }
                TabAction::CloseTab => {
                    if let Some(id) = self.tabs.active_tab_id() {
                        self.close_tab(id);
                    }
                }
                TabAction::NextTab => {
                    self.tabs.next_tab();
                    self.update_address_bar();
                }
                TabAction::PrevTab => {
                    self.tabs.prev_tab();
                    self.update_address_bar();
                }
                TabAction::SwitchToIndex(idx) => {
                    self.tabs.switch_to_index(idx);
                    self.update_address_bar();
                }
                TabAction::ReopenClosed => {
                    self.tabs.reopen_closed_tab();
                    self.update_address_bar();
                }
                TabAction::None => {
                    // Ctrl+L: focus address bar
                    if scancode == b'l' || scancode == b'L' {
                        self.address_bar.focus();
                    }
                }
            }
            self.needs_repaint = true;
            return;
        }

        // Alt+Left/Right for back/forward
        if alt {
            match scancode {
                0x80 => self.go_back(),    // Alt+Left
                0x81 => self.go_forward(), // Alt+Right
                _ => {}
            }
            self.needs_repaint = true;
            return;
        }

        // Pass key to active tab's page
        // (In a real browser, this goes to the focused element)
        let _ = (scancode, shift);
        self.needs_repaint = true;
    }

    /// Handle mouse click at (x, y) relative to browser window
    pub fn handle_click(&mut self, x: i32, y: i32) {
        let tab_bar_h = self.config.tab_bar_height as i32;
        let addr_bar_h = if self.config.show_address_bar {
            self.config.address_bar_height as i32
        } else {
            0
        };

        // Tab bar area
        if y < tab_bar_h {
            let action = self.tabs.handle_tab_bar_click(x, y);
            match action {
                TabAction::NewTab => {
                    self.open_url("");
                    self.address_bar.focus();
                }
                _ => {
                    self.update_address_bar();
                }
            }
            self.needs_repaint = true;
            return;
        }

        // Address bar area
        if y < tab_bar_h + addr_bar_h {
            let nav_width = if self.config.show_nav_buttons {
                self.nav_bar.total_width() as i32 + 8
            } else {
                0
            };

            if x < nav_width {
                // Navigation button click
                if let Some(btn) = self.nav_bar.button_at(x as u32) {
                    match btn {
                        NavButton::Back => self.go_back(),
                        NavButton::Forward => self.go_forward(),
                        NavButton::Reload => self.reload_active(),
                        NavButton::Home => self.go_home(),
                        NavButton::Stop => self.stop_loading(),
                    }
                }
            } else {
                // Address bar click
                self.address_bar.focus();
            }
            self.needs_repaint = true;
            return;
        }

        // Content area: forward click to active tab's page
        let content_y = y - tab_bar_h - addr_bar_h;
        if let Some(tab_id) = self.tabs.active_tab_id() {
            if let Some(proc) = self.isolation.get_process_mut(tab_id) {
                // Forward to event system
                let _target = proc
                    .dom_api
                    .event_dispatcher
                    .dispatch_click(x, content_y, 0);
            }
        }
        self.needs_repaint = true;
    }

    /// Go back in the active tab
    pub fn go_back(&mut self) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            if let Some(url) = tab.go_back() {
                let id = tab.id;
                self.load_page(id, &url);
            }
        }
        self.update_address_bar();
        self.needs_repaint = true;
    }

    /// Go forward in the active tab
    pub fn go_forward(&mut self) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            if let Some(url) = tab.go_forward() {
                let id = tab.id;
                self.load_page(id, &url);
            }
        }
        self.update_address_bar();
        self.needs_repaint = true;
    }

    /// Reload the active tab
    pub fn reload_active(&mut self) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.reload();
            let id = tab.id;
            let url = tab.url.clone();
            self.load_page(id, &url);
        }
        self.needs_repaint = true;
    }

    /// Navigate to home page
    pub fn go_home(&mut self) {
        let home = self.config.home_page.clone();
        self.navigate_active(&home);
    }

    /// Stop loading the active tab
    pub fn stop_loading(&mut self) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.finish_loading();
        }
        self.needs_repaint = true;
    }

    /// Close a tab
    pub fn close_tab(&mut self, tab_id: TabId) {
        self.isolation.kill_tab_process(tab_id);
        self.tabs.close_tab(tab_id);
        self.update_address_bar();
        self.needs_repaint = true;
    }

    /// Update the address bar to reflect the active tab's URL
    fn update_address_bar(&mut self) {
        if !self.address_bar.focused {
            if let Some(tab) = self.tabs.active_tab() {
                self.address_bar.set_url(&tab.url);
            } else {
                self.address_bar.set_url("");
            }
        }
    }

    /// Render the entire browser to the framebuffer
    pub fn render(&mut self) {
        if !self.needs_repaint {
            return;
        }

        let w = self.config.viewport_width as usize;
        let h = self.config.viewport_height as usize;
        let fb_len = w * h;

        // Ensure framebuffer size
        if self.framebuffer.len() != fb_len {
            self.framebuffer.resize(fb_len, 0xFF2D2D30);
        }

        // Clear
        for pixel in self.framebuffer.iter_mut() {
            *pixel = 0xFF2D2D30;
        }

        // 1. Render tab bar
        let tab_bar_h = self.config.tab_bar_height as usize;
        let tabs_ordered = self.tabs.tabs_in_order();
        let tab_refs: Vec<&super::tabs::Tab> = tabs_ordered;
        self.tabs
            .tab_bar
            .render_tab_bar(&tab_refs, &mut self.framebuffer, w);

        // 2. Render address bar
        if self.config.show_address_bar {
            let addr_y = tab_bar_h;
            let addr_h = self.config.address_bar_height as usize;
            let addr_bg: u32 = 0xFF252526;

            // Background
            for dy in 0..addr_h {
                let py = addr_y + dy;
                for px in 0..w {
                    if py * w + px < fb_len {
                        self.framebuffer[py * w + px] = addr_bg;
                    }
                }
            }

            // Navigation buttons
            if self.config.show_nav_buttons {
                let can_back = self.tabs.active_tab().is_some_and(|t| t.can_go_back());
                let can_fwd = self.tabs.active_tab().is_some_and(|t| t.can_go_forward());
                let is_loading = self
                    .tabs
                    .active_tab()
                    .is_some_and(|t| t.load_state == super::tabs::TabLoadState::Loading);
                self.nav_bar.render(
                    &mut self.framebuffer,
                    w,
                    addr_y + 4,
                    can_back,
                    can_fwd,
                    is_loading,
                );
            }

            // Address bar text field
            let text_x = if self.config.show_nav_buttons {
                self.nav_bar.total_width() as usize + 12
            } else {
                8
            };
            let text_y = addr_y + addr_h / 2;
            let text_w = w.saturating_sub(text_x).saturating_sub(8);

            // Text field background
            let field_bg: u32 = if self.address_bar.focused {
                0xFF3C3C3C
            } else {
                0xFF333333
            };
            for dy in 4..addr_h.saturating_sub(4) {
                let py = addr_y + dy;
                for dx in 0..text_w {
                    let px = text_x + dx;
                    if py * w + px < fb_len {
                        self.framebuffer[py * w + px] = field_bg;
                    }
                }
            }

            // Render URL text (simplified)
            let display_text = if self.address_bar.text.len() > text_w / 8 {
                &self.address_bar.text[..text_w / 8]
            } else {
                &self.address_bar.text
            };
            render_simple_text(
                &mut self.framebuffer,
                w,
                text_x + 4,
                text_y,
                display_text,
                0xFFD4D4D4,
            );

            // Cursor
            if self.address_bar.focused {
                let cursor_x = text_x + 4 + self.address_bar.cursor * 8;
                for dy in 0..12_usize {
                    let py = text_y.saturating_sub(6) + dy;
                    if py * w + cursor_x < fb_len {
                        self.framebuffer[py * w + cursor_x] = 0xFFFFFFFF;
                    }
                }
            }
        }

        // 3. Render content area (placeholder: show tab info)
        let content_y = tab_bar_h
            + if self.config.show_address_bar {
                self.config.address_bar_height as usize
            } else {
                0
            };

        if let Some(tab) = self.tabs.active_tab() {
            let content_bg: u32 = 0xFFFFFFFF; // white background for content
            for dy in 0..h.saturating_sub(content_y) {
                let py = content_y + dy;
                for px in 0..w {
                    if py * w + px < fb_len {
                        self.framebuffer[py * w + px] = content_bg;
                    }
                }
            }

            // Show page title and URL
            render_simple_text(
                &mut self.framebuffer,
                w,
                20,
                content_y + 30,
                &tab.title,
                0xFF000000,
            );

            if tab.load_state == super::tabs::TabLoadState::Error {
                if let Some(err) = &tab.error_message {
                    render_simple_text(
                        &mut self.framebuffer,
                        w,
                        20,
                        content_y + 60,
                        err,
                        0xFFCC0000,
                    );
                }
            }
        }

        self.needs_repaint = false;
    }

    /// Tick the browser (advance timers, GC, etc.)
    pub fn tick(&mut self) {
        self.tabs.tick();
        self.isolation.tick_all();
    }

    /// Get the framebuffer for display
    pub fn framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }

    /// Get browser dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.config.viewport_width, self.config.viewport_height)
    }

    /// Number of open tabs
    pub fn tab_count(&self) -> usize {
        self.tabs.tab_count()
    }

    /// Whether the browser is still running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Quit the browser
    pub fn quit(&mut self) {
        // Close all tabs
        let ids: Vec<TabId> = self.tabs.tab_order().to_vec();
        for id in ids {
            self.isolation.kill_tab_process(id);
        }
        self.running = false;
    }
}

// ---------------------------------------------------------------------------
// Shell command integration
// ---------------------------------------------------------------------------

/// Execute a browser command from the shell (e.g., `browser open https://...`)
pub fn handle_shell_command(browser: &mut Browser, args: &[&str]) -> String {
    if args.is_empty() {
        return "Usage: browser <open|back|forward|reload|tabs|close|quit> [args]".to_string();
    }

    match args[0] {
        "open" => {
            let url = if args.len() > 1 { args[1] } else { "" };
            match browser.open_url(url) {
                Some(id) => format!("Opened tab {} with URL: {}", id, url),
                None => "Failed to open tab (max tabs reached?)".to_string(),
            }
        }
        "back" => {
            browser.go_back();
            "Navigated back".to_string()
        }
        "forward" => {
            browser.go_forward();
            "Navigated forward".to_string()
        }
        "reload" => {
            browser.reload_active();
            "Reloading".to_string()
        }
        "home" => {
            browser.go_home();
            "Navigated to home page".to_string()
        }
        "tabs" => {
            let tabs = browser.tabs.tabs_in_order();
            let mut out = format!("{} tab(s) open:\n", tabs.len());
            for tab in tabs {
                let marker = if tab.active { "* " } else { "  " };
                out.push_str(&format!(
                    "{}[{}] {} - {}\n",
                    marker, tab.id, tab.title, tab.url
                ));
            }
            out
        }
        "close" => {
            if args.len() > 1 {
                if let Ok(id) = args[1].parse::<u64>() {
                    browser.close_tab(id);
                    format!("Closed tab {}", id)
                } else {
                    "Invalid tab ID".to_string()
                }
            } else if let Some(id) = browser.tabs.active_tab_id() {
                browser.close_tab(id);
                format!("Closed active tab {}", id)
            } else {
                "No active tab".to_string()
            }
        }
        "quit" => {
            browser.quit();
            "Browser closed".to_string()
        }
        _ => format!("Unknown browser command: {}", args[0]),
    }
}

// ---------------------------------------------------------------------------
// Internal pages
// ---------------------------------------------------------------------------

/// Generate HTML for internal veridian:// pages
fn generate_internal_page(url: &str) -> String {
    let path = url.strip_prefix("veridian://").unwrap_or("");
    match path {
        "newtab" => "<html><head><title>New Tab</title></head><body><h1>VeridianOS \
                     Browser</h1><p>Welcome to the VeridianOS built-in browser.</p></body></html>"
            .to_string(),
        "settings" => "<html><head><title>Settings</title></head><body><h1>Browser \
                       Settings</h1><p>Configuration options will appear here.</p></body></html>"
            .to_string(),
        "about" => "<html><head><title>About</title></head><body><h1>About VeridianOS \
                    Browser</h1><p>Built-in web browser for VeridianOS.</p><p>Kernel-space \
                    rendering with per-tab isolation.</p></body></html>"
            .to_string(),
        "history" => "<html><head><title>History</title></head><body><h1>Browsing \
                      History</h1><p>History entries will appear here.</p></body></html>"
            .to_string(),
        _ => {
            format!(
                "<html><head><title>Not Found</title></head><body><h1>Page Not Found</h1><p>The \
                 page veridian://{} does not exist.</p></body></html>",
                path
            )
        }
    }
}

/// Get the title for an internal page
fn internal_page_title(url: &str) -> String {
    let path = url.strip_prefix("veridian://").unwrap_or("");
    match path {
        "newtab" => "New Tab".to_string(),
        "settings" => "Settings".to_string(),
        "about" => "About".to_string(),
        "history" => "History".to_string(),
        _ => "Not Found".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the origin (scheme + host) from a URL
fn extract_origin(url: &str) -> Option<String> {
    // Find scheme
    let after_scheme = if let Some(rest) = url.strip_prefix("https://") {
        ("https://", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http://", rest)
    } else if let Some(rest) = url.strip_prefix("veridian://") {
        ("veridian://", rest)
    } else {
        return None;
    };

    let host = after_scheme.1.split('/').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(format!("{}{}", after_scheme.0, host))
    }
}

/// Simple URL encoding for search queries
fn url_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(hex_char(b >> 4));
                result.push(hex_char(b & 0x0F));
            }
        }
    }
    result
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + nibble - 10) as char,
        _ => '0',
    }
}

/// Simplified text rendering (placeholder for real font system)
fn render_simple_text(
    buf: &mut [u32],
    buf_width: usize,
    x: usize,
    y: usize,
    text: &str,
    color: u32,
) {
    for (i, _ch) in text.chars().enumerate() {
        let px = x + i * 8;
        if y > 0 && y * buf_width + px < buf.len() {
            // Draw a simple dot pattern for each character
            for dy in 0..5_usize {
                for dx in 0..5_usize {
                    let ppx = px + dx;
                    let ppy = y.wrapping_sub(2) + dy;
                    if ppy * buf_width + ppx < buf.len() && (dy + dx) % 2 == 0 {
                        buf[ppy * buf_width + ppx] = color;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_config_default() {
        let cfg = BrowserConfig::default();
        assert_eq!(cfg.viewport_width, 1024);
        assert_eq!(cfg.viewport_height, 768);
        assert!(cfg.enable_js);
    }

    #[test]
    fn test_browser_new() {
        let browser = Browser::new(BrowserConfig::default());
        assert_eq!(browser.tab_count(), 0);
        assert!(browser.is_running());
    }

    #[test]
    fn test_browser_init() {
        let mut browser = Browser::new(BrowserConfig::default());
        browser.init();
        assert_eq!(browser.tab_count(), 1);
        assert!(browser.tabs.active_tab().is_some());
    }

    #[test]
    fn test_browser_open_url() {
        let mut browser = Browser::default();
        let id = browser.open_url("https://example.com").unwrap();
        assert_eq!(browser.tab_count(), 1);
        let tab = browser.tabs.get_tab(id).unwrap();
        assert_eq!(tab.url, "https://example.com");
    }

    #[test]
    fn test_browser_open_multiple_tabs() {
        let mut browser = Browser::default();
        browser.open_url("https://a.com");
        browser.open_url("https://b.com");
        assert_eq!(browser.tab_count(), 2);
    }

    #[test]
    fn test_browser_close_tab() {
        let mut browser = Browser::default();
        let id = browser.open_url("https://a.com").unwrap();
        browser.open_url("https://b.com");
        browser.close_tab(id);
        assert_eq!(browser.tab_count(), 1);
    }

    #[test]
    fn test_browser_navigate() {
        let mut browser = Browser::default();
        browser.open_url("https://a.com");
        browser.navigate_active("https://b.com");
        assert_eq!(browser.tabs.active_tab().unwrap().url, "https://b.com");
    }

    #[test]
    fn test_browser_back_forward() {
        let mut browser = Browser::default();
        browser.open_url("https://a.com");
        browser.navigate_active("https://b.com");
        browser.go_back();
        assert_eq!(browser.tabs.active_tab().unwrap().url, "https://a.com");
        browser.go_forward();
        assert_eq!(browser.tabs.active_tab().unwrap().url, "https://b.com");
    }

    #[test]
    fn test_browser_go_home() {
        let mut browser = Browser::default();
        browser.open_url("https://example.com");
        browser.go_home();
        assert_eq!(browser.tabs.active_tab().unwrap().url, "veridian://newtab");
    }

    #[test]
    fn test_browser_render() {
        let mut browser = Browser::new(BrowserConfig {
            viewport_width: 100,
            viewport_height: 100,
            ..BrowserConfig::default()
        });
        browser.open_url("veridian://newtab");
        browser.render();
        assert!(!browser.needs_repaint);
        assert_eq!(browser.framebuffer.len(), 10000);
    }

    #[test]
    fn test_browser_tick() {
        let mut browser = Browser::default();
        browser.open_url("https://example.com");
        browser.tick();
        // Should not crash
    }

    #[test]
    fn test_browser_quit() {
        let mut browser = Browser::default();
        browser.open_url("https://example.com");
        browser.quit();
        assert!(!browser.is_running());
    }

    #[test]
    fn test_address_bar_new() {
        let bar = AddressBar::new();
        assert_eq!(bar.text, "");
        assert!(!bar.focused);
    }

    #[test]
    fn test_address_bar_set_url() {
        let mut bar = AddressBar::new();
        bar.set_url("https://example.com");
        assert_eq!(bar.text, "https://example.com");
        assert_eq!(bar.cursor, bar.text.len());
    }

    #[test]
    fn test_address_bar_editing() {
        let mut bar = AddressBar::new();
        bar.focus();
        bar.insert_char('h');
        bar.insert_char('i');
        assert_eq!(bar.text, "hi");
        bar.backspace();
        assert_eq!(bar.text, "h");
        bar.move_left();
        bar.insert_char('x');
        assert_eq!(bar.text, "xh");
    }

    #[test]
    fn test_address_bar_navigation_url() {
        let mut bar = AddressBar::new();
        bar.text = "https://example.com".to_string();
        assert_eq!(bar.get_navigation_url(), "https://example.com");

        bar.text = "example.com".to_string();
        assert_eq!(bar.get_navigation_url(), "https://example.com");

        bar.text = "search terms".to_string();
        assert!(bar.get_navigation_url().starts_with("veridian://search?q="));
    }

    #[test]
    fn test_address_bar_home_end() {
        let mut bar = AddressBar::new();
        bar.text = "hello".to_string();
        bar.cursor = 3;
        bar.home();
        assert_eq!(bar.cursor, 0);
        bar.end();
        assert_eq!(bar.cursor, 5);
    }

    #[test]
    fn test_nav_bar_button_at() {
        let bar = NavigationBar::new();
        assert_eq!(bar.button_at(0), Some(NavButton::Back));
        assert_eq!(bar.button_at(32), Some(NavButton::Forward));
        assert_eq!(bar.button_at(64), Some(NavButton::Reload));
        assert_eq!(bar.button_at(96), Some(NavButton::Home));
    }

    #[test]
    fn test_nav_bar_total_width() {
        let bar = NavigationBar::new();
        assert_eq!(bar.total_width(), 4 * 28 + 3 * 4); // 124
    }

    #[test]
    fn test_extract_origin() {
        assert_eq!(
            extract_origin("https://example.com/path"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            extract_origin("http://test.org"),
            Some("http://test.org".to_string())
        );
        assert_eq!(
            extract_origin("veridian://newtab"),
            Some("veridian://newtab".to_string())
        );
        assert_eq!(extract_origin("ftp://nope"), None);
    }

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a&b"), "a%26b");
        assert_eq!(url_encode("test"), "test");
    }

    #[test]
    fn test_internal_page_generation() {
        let html = generate_internal_page("veridian://newtab");
        assert!(html.contains("VeridianOS Browser"));
        let html = generate_internal_page("veridian://about");
        assert!(html.contains("About"));
        let html = generate_internal_page("veridian://unknown");
        assert!(html.contains("Not Found"));
    }

    #[test]
    fn test_internal_page_title() {
        assert_eq!(internal_page_title("veridian://newtab"), "New Tab");
        assert_eq!(internal_page_title("veridian://settings"), "Settings");
        assert_eq!(internal_page_title("veridian://xyz"), "Not Found");
    }

    #[test]
    fn test_shell_command_open() {
        let mut browser = Browser::default();
        let result = handle_shell_command(&mut browser, &["open", "https://test.com"]);
        assert!(result.contains("Opened tab"));
        assert_eq!(browser.tab_count(), 1);
    }

    #[test]
    fn test_shell_command_tabs() {
        let mut browser = Browser::default();
        browser.open_url("https://a.com");
        browser.open_url("https://b.com");
        let result = handle_shell_command(&mut browser, &["tabs"]);
        assert!(result.contains("2 tab(s)"));
    }

    #[test]
    fn test_shell_command_close() {
        let mut browser = Browser::default();
        let id = browser.open_url("https://a.com").unwrap();
        browser.open_url("https://b.com");
        let result = handle_shell_command(&mut browser, &["close", &format!("{}", id)]);
        assert!(result.contains("Closed"));
        assert_eq!(browser.tab_count(), 1);
    }

    #[test]
    fn test_shell_command_quit() {
        let mut browser = Browser::default();
        browser.open_url("https://a.com");
        let result = handle_shell_command(&mut browser, &["quit"]);
        assert_eq!(result, "Browser closed");
        assert!(!browser.is_running());
    }

    #[test]
    fn test_shell_command_unknown() {
        let mut browser = Browser::default();
        let result = handle_shell_command(&mut browser, &["xyz"]);
        assert!(result.contains("Unknown"));
    }

    #[test]
    fn test_shell_command_empty() {
        let mut browser = Browser::default();
        let result = handle_shell_command(&mut browser, &[]);
        assert!(result.contains("Usage"));
    }

    #[test]
    fn test_browser_dimensions() {
        let browser = Browser::new(BrowserConfig {
            viewport_width: 800,
            viewport_height: 600,
            ..BrowserConfig::default()
        });
        assert_eq!(browser.dimensions(), (800, 600));
    }

    #[test]
    fn test_address_bar_delete() {
        let mut bar = AddressBar::new();
        bar.text = "abc".to_string();
        bar.cursor = 1;
        bar.delete();
        assert_eq!(bar.text, "ac");
    }
}

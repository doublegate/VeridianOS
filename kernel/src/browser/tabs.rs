//! Tabbed Browsing
//!
//! Manages multiple browser tabs with independent browsing contexts.
//! Each tab maintains its own URL, title, navigation history, and
//! rendering state. The tab bar provides visual tab switching with
//! keyboard shortcuts (Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab).

#![allow(dead_code)]

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

// ---------------------------------------------------------------------------
// Tab identity
// ---------------------------------------------------------------------------

/// Unique identifier for a tab
pub type TabId = u64;

// ---------------------------------------------------------------------------
// Navigation history
// ---------------------------------------------------------------------------

/// Navigation history for a single tab
#[derive(Debug, Clone)]
pub struct NavigationHistory {
    /// Back stack (most recent at end)
    back: Vec<String>,
    /// Current URL
    current: String,
    /// Forward stack (most recent at end)
    forward: Vec<String>,
    /// Maximum history entries per direction
    max_entries: usize,
}

impl Default for NavigationHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationHistory {
    pub fn new() -> Self {
        Self {
            back: Vec::new(),
            current: String::new(),
            forward: Vec::new(),
            max_entries: 50,
        }
    }

    /// Navigate to a new URL, pushing current to back stack
    pub(crate) fn navigate(&mut self, url: &str) {
        if !self.current.is_empty() {
            self.back.push(self.current.clone());
            if self.back.len() > self.max_entries {
                self.back.remove(0);
            }
        }
        self.current = url.to_string();
        self.forward.clear();
    }

    /// Go back one page, returns the URL to navigate to
    pub(crate) fn go_back(&mut self) -> Option<String> {
        let prev = self.back.pop()?;
        self.forward.push(self.current.clone());
        self.current = prev.clone();
        Some(prev)
    }

    /// Go forward one page, returns the URL to navigate to
    pub(crate) fn go_forward(&mut self) -> Option<String> {
        let next = self.forward.pop()?;
        self.back.push(self.current.clone());
        self.current = next.clone();
        Some(next)
    }

    /// Whether back navigation is available
    pub(crate) fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether forward navigation is available
    pub(crate) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Current URL
    pub(crate) fn current_url(&self) -> &str {
        &self.current
    }

    /// Number of entries in back stack
    pub(crate) fn back_count(&self) -> usize {
        self.back.len()
    }

    /// Number of entries in forward stack
    pub(crate) fn forward_count(&self) -> usize {
        self.forward.len()
    }
}

// ---------------------------------------------------------------------------
// Tab loading state
// ---------------------------------------------------------------------------

/// Loading state of a tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabLoadState {
    /// Tab is idle / fully loaded
    #[default]
    Idle,
    /// DNS resolution in progress
    Resolving,
    /// Connecting to server
    Connecting,
    /// Downloading page content
    Loading,
    /// Parsing and rendering
    Rendering,
    /// Load failed
    Error,
}

// ---------------------------------------------------------------------------
// Tab
// ---------------------------------------------------------------------------

/// A single browser tab
#[derive(Clone)]
pub struct Tab {
    /// Unique tab identifier
    pub id: TabId,
    /// Tab title (from <title> tag or URL)
    pub title: String,
    /// Current URL
    pub url: String,
    /// Whether this tab is the active (visible) tab
    pub active: bool,
    /// Loading state
    pub load_state: TabLoadState,
    /// Favicon data (raw pixel bytes, 16x16 BGRA)
    pub favicon: Option<Vec<u8>>,
    /// Navigation history
    pub history: NavigationHistory,
    /// Whether the tab has been modified (e.g., form data)
    pub dirty: bool,
    /// Whether the tab is pinned
    pub pinned: bool,
    /// Tab creation order (for sorting)
    pub creation_order: u64,
    /// Last active timestamp (tick count)
    pub last_active_tick: u64,
    /// Optional error message
    pub error_message: Option<String>,
}

impl Tab {
    pub fn new(id: TabId, url: &str, creation_order: u64) -> Self {
        let mut history = NavigationHistory::new();
        if !url.is_empty() {
            history.navigate(url);
        }
        Self {
            id,
            title: title_from_url(url),
            url: url.to_string(),
            active: false,
            load_state: TabLoadState::Idle,
            favicon: None,
            history,
            dirty: false,
            pinned: false,
            creation_order,
            last_active_tick: 0,
            error_message: None,
        }
    }

    /// Navigate this tab to a new URL
    pub(crate) fn navigate(&mut self, url: &str) {
        self.history.navigate(url);
        self.url = url.to_string();
        self.title = title_from_url(url);
        self.load_state = TabLoadState::Loading;
        self.error_message = None;
    }

    /// Mark loading complete
    pub(crate) fn finish_loading(&mut self) {
        self.load_state = TabLoadState::Idle;
    }

    /// Mark loading failed
    pub(crate) fn fail_loading(&mut self, error: &str) {
        self.load_state = TabLoadState::Error;
        self.error_message = Some(error.to_string());
    }

    /// Set the tab title (from <title> tag)
    pub(crate) fn set_title(&mut self, title: &str) {
        self.title = if title.is_empty() {
            title_from_url(&self.url)
        } else {
            truncate_title(title, 64)
        };
    }

    /// Get display title (truncated for tab bar)
    pub(crate) fn display_title(&self, max_len: usize) -> String {
        truncate_title(&self.title, max_len)
    }

    /// Whether the tab can go back
    pub(crate) fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    /// Whether the tab can go forward
    pub(crate) fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    /// Go back, returns URL if successful
    pub(crate) fn go_back(&mut self) -> Option<String> {
        let url = self.history.go_back()?;
        self.url = url.clone();
        self.title = title_from_url(&url);
        self.load_state = TabLoadState::Loading;
        Some(url)
    }

    /// Go forward, returns URL if successful
    pub(crate) fn go_forward(&mut self) -> Option<String> {
        let url = self.history.go_forward()?;
        self.url = url.clone();
        self.title = title_from_url(&url);
        self.load_state = TabLoadState::Loading;
        Some(url)
    }

    /// Reload the current page
    pub(crate) fn reload(&mut self) {
        self.load_state = TabLoadState::Loading;
        self.error_message = None;
    }
}

// ---------------------------------------------------------------------------
// Tab bar (visual representation)
// ---------------------------------------------------------------------------

/// Visual tab bar state for rendering
pub struct TabBar {
    /// Width of each tab in pixels
    pub tab_width: i32,
    /// Minimum tab width
    pub min_tab_width: i32,
    /// Maximum tab width
    pub max_tab_width: i32,
    /// Height of the tab bar
    pub height: i32,
    /// Scroll offset for many tabs
    pub scroll_offset: i32,
    /// Total visible width
    pub visible_width: i32,
    /// Close button size
    pub close_button_size: i32,
    /// Hovered tab (for visual feedback)
    pub hovered_tab: Option<TabId>,
    /// Hovered close button
    pub hovered_close: Option<TabId>,
    /// Tab being dragged
    pub dragging_tab: Option<TabId>,
    /// Drag x offset
    pub drag_offset_x: i32,
}

impl Default for TabBar {
    fn default() -> Self {
        Self::new(800)
    }
}

impl TabBar {
    pub fn new(visible_width: i32) -> Self {
        Self {
            tab_width: 200,
            min_tab_width: 80,
            max_tab_width: 250,
            height: 32,
            scroll_offset: 0,
            visible_width,
            close_button_size: 16,
            hovered_tab: None,
            hovered_close: None,
            dragging_tab: None,
            drag_offset_x: 0,
        }
    }

    /// Calculate the width for each tab given the number of tabs
    pub(crate) fn compute_tab_width(&self, num_tabs: usize) -> i32 {
        if num_tabs == 0 {
            return self.max_tab_width;
        }
        // Leave space for new-tab button (32px)
        let available = self.visible_width - 32;
        let per_tab = available / (num_tabs as i32);
        per_tab.clamp(self.min_tab_width, self.max_tab_width)
    }

    /// Get the tab at a given x position, given an ordered list of tab IDs
    pub(crate) fn tab_at_x(&self, x: i32, tab_ids: &[TabId], num_tabs: usize) -> Option<TabId> {
        let tw = self.compute_tab_width(num_tabs);
        let local_x = x + self.scroll_offset;
        if local_x < 0 {
            return None;
        }
        let idx = local_x / tw;
        if (idx as usize) < tab_ids.len() {
            Some(tab_ids[idx as usize])
        } else {
            None
        }
    }

    /// Check if the click is on the close button of a tab
    pub(crate) fn is_close_button_hit(
        &self,
        x: i32,
        tab_ids: &[TabId],
        num_tabs: usize,
    ) -> Option<TabId> {
        let tw = self.compute_tab_width(num_tabs);
        let local_x = x + self.scroll_offset;
        if local_x < 0 {
            return None;
        }
        let idx = local_x / tw;
        if (idx as usize) >= tab_ids.len() {
            return None;
        }
        // Close button is in the right portion of the tab
        let tab_start = idx * tw;
        let close_start = tab_start + tw - self.close_button_size - 4;
        if local_x >= close_start && local_x <= close_start + self.close_button_size {
            Some(tab_ids[idx as usize])
        } else {
            None
        }
    }

    /// Check if click is on the new-tab button
    pub(crate) fn is_new_tab_button_hit(&self, x: i32, num_tabs: usize) -> bool {
        let tw = self.compute_tab_width(num_tabs);
        let tabs_end = (num_tabs as i32) * tw - self.scroll_offset;
        x >= tabs_end && x <= tabs_end + 32
    }

    /// Render the tab bar into a pixel buffer (BGRA format)
    /// Returns the rendered line data for the tab bar area
    pub(crate) fn render_tab_bar(&self, tabs: &[&Tab], buf: &mut [u32], buf_width: usize) {
        let height = self.height as usize;
        let tw = self.compute_tab_width(tabs.len()) as usize;

        // Background
        let bg_color: u32 = 0xFF2D2D30; // dark gray
        for y in 0..height {
            for x in 0..buf_width {
                if y * buf_width + x < buf.len() {
                    buf[y * buf_width + x] = bg_color;
                }
            }
        }

        // Draw each tab
        for (i, tab) in tabs.iter().enumerate() {
            let tab_x = (i * tw).saturating_sub(self.scroll_offset as usize);
            if tab_x >= buf_width {
                break;
            }

            let tab_bg = if tab.active {
                0xFF3C3C3C_u32 // active tab: lighter
            } else if self.hovered_tab == Some(tab.id) {
                0xFF353535_u32 // hovered: slightly lighter
            } else {
                0xFF2D2D30_u32 // inactive: same as bar
            };

            // Tab body
            for y in 2..height {
                let end_x = (tab_x + tw).min(buf_width);
                for x in (tab_x + 1)..end_x.saturating_sub(1) {
                    if y * buf_width + x < buf.len() {
                        buf[y * buf_width + x] = tab_bg;
                    }
                }
            }

            // Active tab indicator (blue line at top)
            if tab.active {
                let indicator_color: u32 = 0xFF007ACC;
                for x in (tab_x + 1)..(tab_x + tw).min(buf_width).saturating_sub(1) {
                    if x < buf_width {
                        buf[x] = indicator_color;
                        if buf_width + x < buf.len() {
                            buf[buf_width + x] = indicator_color;
                        }
                    }
                }
            }

            // Tab title (simplified: 1 char = 8px)
            let title = tab.display_title((tw.saturating_sub(30)) / 8);
            let text_y = height / 2;
            let text_x = tab_x + 8;
            let text_color: u32 = if tab.active { 0xFFFFFFFF } else { 0xFFA0A0A0 };
            render_text_simple(buf, buf_width, text_x, text_y, &title, text_color);

            // Close button (X)
            if !tab.pinned {
                let cx = tab_x + tw - (self.close_button_size as usize) - 4;
                let cy = (height - self.close_button_size as usize) / 2;
                let close_color: u32 = if self.hovered_close == Some(tab.id) {
                    0xFFFF0000
                } else {
                    0xFF808080
                };
                render_close_button(
                    buf,
                    buf_width,
                    cx,
                    cy,
                    self.close_button_size as usize,
                    close_color,
                );
            }

            // Loading indicator
            if tab.load_state == TabLoadState::Loading {
                let lx = tab_x + tw - 24;
                let ly = height / 2 - 2;
                for dx in 0..4 {
                    let px = lx + dx;
                    let py = ly;
                    if py * buf_width + px < buf.len() {
                        buf[py * buf_width + px] = 0xFF00AAFF_u32;
                    }
                }
            }
        }

        // New tab button (+)
        let plus_x = tabs.len() * tw;
        if plus_x < buf_width.saturating_sub(32) {
            render_text_simple(buf, buf_width, plus_x + 10, height / 2, "+", 0xFFA0A0A0);
        }
    }

    /// Scroll to ensure a tab is visible
    pub(crate) fn ensure_visible(&mut self, tab_index: usize, num_tabs: usize) {
        let tw = self.compute_tab_width(num_tabs);
        let tab_start = (tab_index as i32) * tw;
        let tab_end = tab_start + tw;

        if tab_start < self.scroll_offset {
            self.scroll_offset = tab_start;
        } else if tab_end > self.scroll_offset + self.visible_width - 32 {
            self.scroll_offset = tab_end - self.visible_width + 32;
        }
    }
}

// ---------------------------------------------------------------------------
// Tab manager
// ---------------------------------------------------------------------------

/// Keyboard shortcut actions for tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAction {
    /// Ctrl+T: new tab
    NewTab,
    /// Ctrl+W: close current tab
    CloseTab,
    /// Ctrl+Tab: next tab
    NextTab,
    /// Ctrl+Shift+Tab: previous tab
    PrevTab,
    /// Ctrl+1..9: switch to tab N
    SwitchToIndex(usize),
    /// Ctrl+Shift+T: reopen last closed tab
    ReopenClosed,
    /// None
    None,
}

/// Manages all browser tabs
pub struct TabManager {
    /// All tabs, keyed by TabId
    tabs: BTreeMap<TabId, Tab>,
    /// Ordered list of tab IDs (display order)
    tab_order: Vec<TabId>,
    /// Currently active tab ID
    active_tab: Option<TabId>,
    /// Next tab ID to assign
    next_id: TabId,
    /// Maximum number of tabs allowed
    max_tabs: usize,
    /// Recently closed tab URLs (for reopen)
    recently_closed: Vec<String>,
    /// Maximum recently closed entries
    max_recently_closed: usize,
    /// Tab bar visual state
    pub tab_bar: TabBar,
    /// Current tick counter
    current_tick: u64,
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TabManager {
    /// Maximum tabs default
    const DEFAULT_MAX_TABS: usize = 32;

    pub fn new() -> Self {
        Self {
            tabs: BTreeMap::new(),
            tab_order: Vec::new(),
            active_tab: None,
            next_id: 1,
            max_tabs: Self::DEFAULT_MAX_TABS,
            recently_closed: Vec::new(),
            max_recently_closed: 10,
            tab_bar: TabBar::new(800),
            current_tick: 0,
        }
    }

    /// Create a new tab with the given URL.
    /// Returns the TabId, or None if max tabs reached.
    pub(crate) fn new_tab(&mut self, url: &str) -> Option<TabId> {
        if self.tabs.len() >= self.max_tabs {
            return None;
        }

        let id = self.next_id;
        self.next_id += 1;

        let tab = Tab::new(id, url, id);
        self.tabs.insert(id, tab);
        self.tab_order.push(id);

        // If no active tab, make this one active
        if self.active_tab.is_none() {
            self.switch_tab(id);
        }

        Some(id)
    }

    /// Close a tab by ID. Returns true if closed.
    /// If closing the active tab, switches to an adjacent tab.
    pub(crate) fn close_tab(&mut self, id: TabId) -> bool {
        // Don't close the last tab - open a blank tab instead
        if self.tabs.len() <= 1 {
            if let Some(tab) = self.tabs.get_mut(&id) {
                // Save URL for reopen
                if !tab.url.is_empty() {
                    self.recently_closed.push(tab.url.clone());
                    if self.recently_closed.len() > self.max_recently_closed {
                        self.recently_closed.remove(0);
                    }
                }
                tab.url.clear();
                tab.title = "New Tab".to_string();
                tab.history = NavigationHistory::new();
                tab.load_state = TabLoadState::Idle;
                return true;
            }
            return false;
        }

        // Find index of tab being closed
        let order_idx = match self.tab_order.iter().position(|&t| t == id) {
            Some(idx) => idx,
            None => return false,
        };

        // Save URL for reopen
        if let Some(tab) = self.tabs.get(&id) {
            if !tab.url.is_empty() {
                self.recently_closed.push(tab.url.clone());
                if self.recently_closed.len() > self.max_recently_closed {
                    self.recently_closed.remove(0);
                }
            }
        }

        // Remove from data structures
        self.tabs.remove(&id);
        self.tab_order.remove(order_idx);

        // If we closed the active tab, switch to adjacent
        if self.active_tab == Some(id) {
            let new_idx = if order_idx >= self.tab_order.len() {
                self.tab_order.len().saturating_sub(1)
            } else {
                order_idx
            };
            if let Some(&new_id) = self.tab_order.get(new_idx) {
                self.switch_tab(new_id);
            } else {
                self.active_tab = None;
            }
        }

        true
    }

    /// Switch to a tab by ID
    pub(crate) fn switch_tab(&mut self, id: TabId) -> bool {
        if !self.tabs.contains_key(&id) {
            return false;
        }

        // Deactivate current
        if let Some(old_id) = self.active_tab {
            if let Some(old_tab) = self.tabs.get_mut(&old_id) {
                old_tab.active = false;
            }
        }

        // Activate new
        if let Some(tab) = self.tabs.get_mut(&id) {
            tab.active = true;
            tab.last_active_tick = self.current_tick;
        }
        self.active_tab = Some(id);

        // Ensure visible in tab bar
        if let Some(idx) = self.tab_order.iter().position(|&t| t == id) {
            self.tab_bar.ensure_visible(idx, self.tab_order.len());
        }

        true
    }

    /// Switch to the next tab (wraps around)
    pub(crate) fn next_tab(&mut self) -> bool {
        if self.tab_order.len() <= 1 {
            return false;
        }
        let current_idx = self
            .active_tab
            .and_then(|id| self.tab_order.iter().position(|&t| t == id))
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.tab_order.len();
        let next_id = self.tab_order[next_idx];
        self.switch_tab(next_id)
    }

    /// Switch to the previous tab (wraps around)
    pub(crate) fn prev_tab(&mut self) -> bool {
        if self.tab_order.len() <= 1 {
            return false;
        }
        let current_idx = self
            .active_tab
            .and_then(|id| self.tab_order.iter().position(|&t| t == id))
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            self.tab_order.len() - 1
        } else {
            current_idx - 1
        };
        let prev_id = self.tab_order[prev_idx];
        self.switch_tab(prev_id)
    }

    /// Switch to tab at a specific index (0-based)
    pub(crate) fn switch_to_index(&mut self, index: usize) -> bool {
        if let Some(&id) = self.tab_order.get(index) {
            self.switch_tab(id)
        } else {
            false
        }
    }

    /// Move a tab to a new position in the tab order
    pub(crate) fn move_tab(&mut self, id: TabId, new_index: usize) -> bool {
        let current_idx = match self.tab_order.iter().position(|&t| t == id) {
            Some(idx) => idx,
            None => return false,
        };
        let clamped = new_index.min(self.tab_order.len().saturating_sub(1));
        self.tab_order.remove(current_idx);
        self.tab_order.insert(clamped, id);
        true
    }

    /// Reopen the most recently closed tab
    pub(crate) fn reopen_closed_tab(&mut self) -> Option<TabId> {
        let url = self.recently_closed.pop()?;
        self.new_tab(&url)
    }

    /// Get the active tab
    pub(crate) fn active_tab(&self) -> Option<&Tab> {
        self.active_tab.and_then(|id| self.tabs.get(&id))
    }

    /// Get the active tab mutably
    pub(crate) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let id = self.active_tab?;
        self.tabs.get_mut(&id)
    }

    /// Get a tab by ID
    pub(crate) fn get_tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.get(&id)
    }

    /// Get a tab mutably by ID
    pub(crate) fn get_tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.get_mut(&id)
    }

    /// Get all tabs in display order
    pub(crate) fn tabs_in_order(&self) -> Vec<&Tab> {
        self.tab_order
            .iter()
            .filter_map(|id| self.tabs.get(id))
            .collect()
    }

    /// Number of open tabs
    pub(crate) fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Active tab ID
    pub(crate) fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab
    }

    /// Ordered tab IDs
    pub(crate) fn tab_order(&self) -> &[TabId] {
        &self.tab_order
    }

    /// Process a keyboard shortcut, returning the action to take
    pub(crate) fn decode_shortcut(ctrl: bool, shift: bool, key: u8) -> TabAction {
        if !ctrl {
            return TabAction::None;
        }
        match key {
            b't' | b'T' if !shift => TabAction::NewTab,
            b'w' | b'W' if !shift => TabAction::CloseTab,
            b'T' if shift => TabAction::ReopenClosed,
            // Tab key (scancode approximation)
            0x09 if !shift => TabAction::NextTab,
            0x09 if shift => TabAction::PrevTab,
            // Ctrl+1 through Ctrl+9
            b'1'..=b'9' => TabAction::SwitchToIndex((key - b'1') as usize),
            _ => TabAction::None,
        }
    }

    /// Execute a tab action
    pub(crate) fn execute_action(&mut self, action: TabAction) -> bool {
        match action {
            TabAction::NewTab => self.new_tab("").is_some(),
            TabAction::CloseTab => {
                if let Some(id) = self.active_tab {
                    self.close_tab(id)
                } else {
                    false
                }
            }
            TabAction::NextTab => self.next_tab(),
            TabAction::PrevTab => self.prev_tab(),
            TabAction::SwitchToIndex(idx) => self.switch_to_index(idx),
            TabAction::ReopenClosed => self.reopen_closed_tab().is_some(),
            TabAction::None => false,
        }
    }

    /// Update tick counter
    pub(crate) fn tick(&mut self) {
        self.current_tick += 1;
    }

    /// Set the tab bar width
    pub(crate) fn set_viewport_width(&mut self, width: i32) {
        self.tab_bar.visible_width = width;
    }

    /// Handle tab bar click at position (x, y)
    /// Returns an action to take
    pub(crate) fn handle_tab_bar_click(&mut self, x: i32, _y: i32) -> TabAction {
        let num_tabs = self.tab_order.len();
        let ids: Vec<TabId> = self.tab_order.clone();

        // Check close button first
        if let Some(id) = self.tab_bar.is_close_button_hit(x, &ids, num_tabs) {
            self.close_tab(id);
            return TabAction::CloseTab;
        }

        // Check new tab button
        if self.tab_bar.is_new_tab_button_hit(x, num_tabs) {
            return TabAction::NewTab;
        }

        // Check tab click
        if let Some(id) = self.tab_bar.tab_at_x(x, &ids, num_tabs) {
            self.switch_tab(id);
            return TabAction::SwitchToIndex(0); // signal that we switched
        }

        TabAction::None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a display title from a URL
fn title_from_url(url: &str) -> String {
    if url.is_empty() {
        return "New Tab".to_string();
    }
    // Strip protocol
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("veridian://"))
        .unwrap_or(url);
    // Take up to first /
    let host = stripped.split('/').next().unwrap_or(stripped);
    if host.is_empty() {
        "New Tab".to_string()
    } else {
        host.to_string()
    }
}

/// Truncate a title to max_len characters, adding ellipsis if needed
fn truncate_title(title: &str, max_len: usize) -> String {
    if title.len() <= max_len {
        title.to_string()
    } else if max_len <= 3 {
        title[..max_len].to_string()
    } else {
        let mut s = title[..max_len - 3].to_string();
        s.push_str("...");
        s
    }
}

/// Simple text rendering: draw text as colored pixels (1 char = 8px wide)
/// This is a placeholder; real rendering uses the font system.
fn render_text_simple(
    buf: &mut [u32],
    buf_width: usize,
    x: usize,
    y: usize,
    text: &str,
    color: u32,
) {
    for (i, _ch) in text.chars().enumerate() {
        let px = x + i * 8 + 2;
        let py = y;
        if py * buf_width + px < buf.len() {
            buf[py * buf_width + px] = color;
        }
        // Draw a small dot pattern for each character
        for dy in 0..5_usize {
            for dx in 0..5_usize {
                let ppx = x + i * 8 + dx + 1;
                let ppy = y.saturating_sub(2) + dy;
                if ppy * buf_width + ppx < buf.len() && (dy + dx) % 2 == 0 {
                    buf[ppy * buf_width + ppx] = color;
                }
            }
        }
    }
}

/// Render a small X close button
fn render_close_button(
    buf: &mut [u32],
    buf_width: usize,
    x: usize,
    y: usize,
    size: usize,
    color: u32,
) {
    for i in 0..size {
        // Diagonal \
        let px1 = x + i;
        let py1 = y + i;
        if py1 * buf_width + px1 < buf.len() {
            buf[py1 * buf_width + px1] = color;
        }
        // Diagonal /
        let px2 = x + size - 1 - i;
        let py2 = y + i;
        if py2 * buf_width + px2 < buf.len() {
            buf[py2 * buf_width + px2] = color;
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
    fn test_navigation_history_new() {
        let h = NavigationHistory::new();
        assert_eq!(h.current_url(), "");
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
    }

    #[test]
    fn test_navigation_navigate() {
        let mut h = NavigationHistory::new();
        h.navigate("https://example.com");
        assert_eq!(h.current_url(), "https://example.com");
        h.navigate("https://test.com");
        assert_eq!(h.current_url(), "https://test.com");
        assert!(h.can_go_back());
        assert!(!h.can_go_forward());
    }

    #[test]
    fn test_navigation_back_forward() {
        let mut h = NavigationHistory::new();
        h.navigate("https://a.com");
        h.navigate("https://b.com");
        h.navigate("https://c.com");

        let back = h.go_back().unwrap();
        assert_eq!(back, "https://b.com");
        assert!(h.can_go_forward());

        let fwd = h.go_forward().unwrap();
        assert_eq!(fwd, "https://c.com");
    }

    #[test]
    fn test_navigation_clears_forward_on_navigate() {
        let mut h = NavigationHistory::new();
        h.navigate("https://a.com");
        h.navigate("https://b.com");
        h.go_back();
        h.navigate("https://c.com");
        assert!(!h.can_go_forward());
    }

    #[test]
    fn test_tab_new() {
        let tab = Tab::new(1, "https://example.com", 1);
        assert_eq!(tab.id, 1);
        assert_eq!(tab.url, "https://example.com");
        assert_eq!(tab.title, "example.com");
        assert!(!tab.active);
    }

    #[test]
    fn test_tab_navigate() {
        let mut tab = Tab::new(1, "", 1);
        tab.navigate("https://test.org/page");
        assert_eq!(tab.url, "https://test.org/page");
        assert_eq!(tab.load_state, TabLoadState::Loading);
    }

    #[test]
    fn test_tab_back_forward() {
        let mut tab = Tab::new(1, "https://a.com", 1);
        tab.navigate("https://b.com");
        assert!(tab.can_go_back());
        let back = tab.go_back().unwrap();
        assert_eq!(back, "https://a.com");
        let fwd = tab.go_forward().unwrap();
        assert_eq!(fwd, "https://b.com");
    }

    #[test]
    fn test_tab_manager_new() {
        let tm = TabManager::new();
        assert_eq!(tm.tab_count(), 0);
        assert!(tm.active_tab_id().is_none());
    }

    #[test]
    fn test_new_tab() {
        let mut tm = TabManager::new();
        let id = tm.new_tab("https://example.com").unwrap();
        assert_eq!(tm.tab_count(), 1);
        assert_eq!(tm.active_tab_id(), Some(id));
        let tab = tm.get_tab(id).unwrap();
        assert!(tab.active);
    }

    #[test]
    fn test_close_tab() {
        let mut tm = TabManager::new();
        let id1 = tm.new_tab("https://a.com").unwrap();
        let id2 = tm.new_tab("https://b.com").unwrap();
        tm.switch_tab(id2);
        tm.close_tab(id2);
        assert_eq!(tm.tab_count(), 1);
        assert_eq!(tm.active_tab_id(), Some(id1));
    }

    #[test]
    fn test_close_last_tab() {
        let mut tm = TabManager::new();
        let id = tm.new_tab("https://a.com").unwrap();
        tm.close_tab(id);
        // Should still have one tab (blank)
        assert_eq!(tm.tab_count(), 1);
        let tab = tm.get_tab(id).unwrap();
        assert_eq!(tab.url, "");
    }

    #[test]
    fn test_next_prev_tab() {
        let mut tm = TabManager::new();
        let id1 = tm.new_tab("https://a.com").unwrap();
        let id2 = tm.new_tab("https://b.com").unwrap();
        let id3 = tm.new_tab("https://c.com").unwrap();
        tm.switch_tab(id1);

        tm.next_tab();
        assert_eq!(tm.active_tab_id(), Some(id2));
        tm.next_tab();
        assert_eq!(tm.active_tab_id(), Some(id3));
        tm.next_tab(); // wraps
        assert_eq!(tm.active_tab_id(), Some(id1));

        tm.prev_tab(); // wraps back
        assert_eq!(tm.active_tab_id(), Some(id3));
    }

    #[test]
    fn test_move_tab() {
        let mut tm = TabManager::new();
        let id1 = tm.new_tab("a").unwrap();
        let id2 = tm.new_tab("b").unwrap();
        let id3 = tm.new_tab("c").unwrap();
        tm.move_tab(id3, 0);
        assert_eq!(tm.tab_order()[0], id3);
        assert_eq!(tm.tab_order()[1], id1);
        assert_eq!(tm.tab_order()[2], id2);
    }

    #[test]
    fn test_switch_to_index() {
        let mut tm = TabManager::new();
        let _id1 = tm.new_tab("a").unwrap();
        let id2 = tm.new_tab("b").unwrap();
        tm.switch_to_index(1);
        assert_eq!(tm.active_tab_id(), Some(id2));
        assert!(!tm.switch_to_index(99));
    }

    #[test]
    fn test_max_tabs() {
        let mut tm = TabManager::new();
        tm.max_tabs = 3;
        tm.new_tab("1").unwrap();
        tm.new_tab("2").unwrap();
        tm.new_tab("3").unwrap();
        assert!(tm.new_tab("4").is_none());
    }

    #[test]
    fn test_reopen_closed() {
        let mut tm = TabManager::new();
        let id1 = tm.new_tab("https://saved.com").unwrap();
        let _id2 = tm.new_tab("https://keep.com").unwrap();
        tm.switch_tab(id1);
        tm.close_tab(id1);
        let reopened = tm.reopen_closed_tab().unwrap();
        let tab = tm.get_tab(reopened).unwrap();
        assert_eq!(tab.url, "https://saved.com");
    }

    #[test]
    fn test_decode_shortcut() {
        assert_eq!(
            TabManager::decode_shortcut(true, false, b't'),
            TabAction::NewTab
        );
        assert_eq!(
            TabManager::decode_shortcut(true, false, b'w'),
            TabAction::CloseTab
        );
        assert_eq!(
            TabManager::decode_shortcut(true, true, b'T'),
            TabAction::ReopenClosed
        );
        assert_eq!(
            TabManager::decode_shortcut(true, false, b'1'),
            TabAction::SwitchToIndex(0)
        );
        assert_eq!(
            TabManager::decode_shortcut(true, false, b'5'),
            TabAction::SwitchToIndex(4)
        );
        assert_eq!(
            TabManager::decode_shortcut(false, false, b't'),
            TabAction::None
        );
    }

    #[test]
    fn test_execute_action() {
        let mut tm = TabManager::new();
        tm.execute_action(TabAction::NewTab);
        assert_eq!(tm.tab_count(), 1);
        tm.execute_action(TabAction::NewTab);
        assert_eq!(tm.tab_count(), 2);
        tm.execute_action(TabAction::NextTab);
        // Should have moved to next tab
    }

    #[test]
    fn test_title_from_url() {
        assert_eq!(title_from_url(""), "New Tab");
        assert_eq!(title_from_url("https://example.com"), "example.com");
        assert_eq!(title_from_url("https://example.com/path"), "example.com");
        assert_eq!(title_from_url("veridian://settings"), "settings");
    }

    #[test]
    fn test_truncate_title() {
        assert_eq!(truncate_title("hello", 10), "hello");
        assert_eq!(truncate_title("hello world test", 10), "hello w...");
        assert_eq!(truncate_title("ab", 2), "ab");
    }

    #[test]
    fn test_tab_bar_compute_width() {
        let bar = TabBar::new(800);
        let w = bar.compute_tab_width(5);
        assert!(w >= bar.min_tab_width);
        assert!(w <= bar.max_tab_width);
    }

    #[test]
    fn test_tab_bar_tab_at_x() {
        let bar = TabBar::new(800);
        let ids = [1u64, 2, 3];
        let tw = bar.compute_tab_width(3);
        assert_eq!(bar.tab_at_x(tw / 2, &ids, 3), Some(1));
        assert_eq!(bar.tab_at_x(tw + tw / 2, &ids, 3), Some(2));
    }

    #[test]
    fn test_tabs_in_order() {
        let mut tm = TabManager::new();
        let id1 = tm.new_tab("a").unwrap();
        let id2 = tm.new_tab("b").unwrap();
        let tabs = tm.tabs_in_order();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].id, id1);
        assert_eq!(tabs[1].id, id2);
    }

    #[test]
    fn test_tab_set_title() {
        let mut tab = Tab::new(1, "https://example.com", 1);
        tab.set_title("My Page");
        assert_eq!(tab.title, "My Page");
        tab.set_title("");
        assert_eq!(tab.title, "example.com");
    }

    #[test]
    fn test_tab_reload() {
        let mut tab = Tab::new(1, "https://example.com", 1);
        tab.finish_loading();
        assert_eq!(tab.load_state, TabLoadState::Idle);
        tab.reload();
        assert_eq!(tab.load_state, TabLoadState::Loading);
    }

    #[test]
    fn test_tab_fail_loading() {
        let mut tab = Tab::new(1, "https://example.com", 1);
        tab.fail_loading("Connection refused");
        assert_eq!(tab.load_state, TabLoadState::Error);
        assert_eq!(tab.error_message.as_deref(), Some("Connection refused"));
    }

    #[test]
    fn test_tab_bar_default() {
        let bar = TabBar::default();
        assert_eq!(bar.visible_width, 800);
        assert_eq!(bar.height, 32);
    }

    #[test]
    fn test_tab_manager_tick() {
        let mut tm = TabManager::new();
        tm.tick();
        tm.tick();
        assert_eq!(tm.current_tick, 2);
    }

    #[test]
    fn test_navigation_history_counts() {
        let mut h = NavigationHistory::new();
        h.navigate("a");
        h.navigate("b");
        h.navigate("c");
        assert_eq!(h.back_count(), 2);
        assert_eq!(h.forward_count(), 0);
        h.go_back();
        assert_eq!(h.back_count(), 1);
        assert_eq!(h.forward_count(), 1);
    }
}

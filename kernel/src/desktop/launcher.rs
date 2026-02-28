//! Application Launcher
//!
//! Provides an application launcher overlay with search functionality,
//! grid display, and keyboard navigation. The launcher renders as a
//! semi-transparent overlay on top of the desktop, showing available
//! applications in a grid layout. Users can search by typing, navigate
//! with arrow keys, and launch applications with Enter.

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

use crate::sync::once_lock::GlobalState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Application category for grouping and icon color coding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCategory {
    /// Core system utilities (task manager, system monitor)
    System,
    /// General-purpose utilities (calculator, clock)
    Utility,
    /// Development tools (editors, compilers, debuggers)
    Development,
    /// Graphics and image applications
    Graphics,
    /// Networking and internet applications
    Network,
    /// Audio, video, and media applications
    Multimedia,
    /// Office and productivity applications
    Office,
    /// System configuration and preferences
    Settings,
    /// Uncategorized applications
    Other,
}

/// A single application entry in the launcher.
#[derive(Debug, Clone)]
pub struct AppEntry {
    /// Human-readable application name.
    pub name: String,
    /// Path to the executable binary.
    pub exec_path: String,
    /// Icon name (used for future icon theme lookup).
    pub icon_name: String,
    /// Application category.
    pub category: AppCategory,
    /// Short description of the application.
    pub description: String,
}

impl AppEntry {
    /// Create a new application entry.
    pub fn new(
        name: &str,
        exec_path: &str,
        icon_name: &str,
        category: AppCategory,
        description: &str,
    ) -> Self {
        Self {
            name: String::from(name),
            exec_path: String::from(exec_path),
            icon_name: String::from(icon_name),
            category,
            description: String::from(description),
        }
    }
}

/// Launcher visibility state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherState {
    /// Launcher is not visible.
    Hidden,
    /// Launcher is visible, showing the application grid.
    Visible,
    /// Launcher is visible and the search bar is actively receiving input.
    SearchActive,
}

/// Action returned by input handlers to the caller.
#[derive(Debug, Clone)]
pub enum LauncherAction {
    /// Launch the application at the given exec path.
    Launch(String),
    /// Hide the launcher overlay.
    Hide,
}

// ---------------------------------------------------------------------------
// Launcher
// ---------------------------------------------------------------------------

/// Application launcher with search, grid display, and keyboard navigation.
pub struct AppLauncher {
    /// All registered application entries.
    entries: Vec<AppEntry>,
    /// Indices into `entries` for the current filter result.
    filtered: Vec<usize>,
    /// Current search query string.
    search_query: String,
    /// Index into `filtered` of the currently selected (highlighted) entry.
    selected_index: usize,
    /// Current launcher state.
    state: LauncherState,
    /// Number of columns in the application grid.
    grid_columns: usize,
    /// Number of visible rows in the application grid.
    grid_rows: usize,
    /// Scroll offset (in number of entries) for the grid.
    scroll_offset: usize,
    /// Overlay X position in pixels (relative to screen).
    overlay_x: usize,
    /// Overlay Y position in pixels (relative to screen).
    overlay_y: usize,
    /// Overlay width in pixels.
    overlay_width: usize,
    /// Overlay height in pixels.
    overlay_height: usize,
}

impl AppLauncher {
    /// Create a new application launcher pre-populated with default
    /// applications.
    pub fn new() -> Self {
        let entries = default_applications();
        let filtered: Vec<usize> = (0..entries.len()).collect();

        Self {
            entries,
            filtered,
            search_query: String::new(),
            selected_index: 0,
            state: LauncherState::Hidden,
            grid_columns: 4,
            grid_rows: 3,
            scroll_offset: 0,
            overlay_x: 0,
            overlay_y: 0,
            overlay_width: 640,
            overlay_height: 480,
        }
    }

    /// Show the launcher overlay, clearing any previous search.
    pub fn show(&mut self) {
        self.state = LauncherState::Visible;
        self.search_query.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        // Reset filter to show all entries
        self.filtered = (0..self.entries.len()).collect();
    }

    /// Hide the launcher overlay.
    pub fn hide(&mut self) {
        self.state = LauncherState::Hidden;
    }

    /// Toggle the launcher between visible and hidden.
    pub fn toggle(&mut self) {
        match self.state {
            LauncherState::Hidden => self.show(),
            LauncherState::Visible | LauncherState::SearchActive => self.hide(),
        }
    }

    /// Returns `true` if the launcher is currently visible.
    pub fn is_visible(&self) -> bool {
        self.state != LauncherState::Hidden
    }

    /// Set the overlay position and dimensions (called when screen size is
    /// known).
    pub fn set_overlay_rect(&mut self, x: usize, y: usize, width: usize, height: usize) {
        self.overlay_x = x;
        self.overlay_y = y;
        self.overlay_width = width;
        self.overlay_height = height;
    }

    /// Register a new application entry with the launcher.
    pub fn register_app(&mut self, entry: AppEntry) {
        self.entries.push(entry);
        // Refresh filter
        self.filter_entries();
    }

    /// Remove an application by exec path.
    pub fn unregister_app(&mut self, exec_path: &str) {
        self.entries.retain(|e| e.exec_path.as_str() != exec_path);
        self.filter_entries();
    }

    /// Handle a keyboard input event.
    ///
    /// Returns an optional `LauncherAction` indicating what the caller should
    /// do (launch an app, hide the launcher, or nothing).
    ///
    /// Key mappings:
    /// - Enter (0x0A or 0x0D): launch the selected application
    /// - Escape (0x1B): hide the launcher
    /// - Backspace (0x08): delete last character from search query
    /// - Arrow Up (0x80): move selection up one row
    /// - Arrow Down (0x81): move selection down one row
    /// - Arrow Left (0x82): move selection left one column
    /// - Arrow Right (0x83): move selection right one column
    /// - Printable ASCII (0x20..=0x7E): append to search query, filter entries
    pub fn handle_key(&mut self, key: u8) -> Option<LauncherAction> {
        if self.state == LauncherState::Hidden {
            return None;
        }

        match key {
            // Enter -- launch selected
            0x0A | 0x0D => {
                if let Some(entry) = self.selected_entry() {
                    let path = entry.exec_path.clone();
                    self.hide();
                    return Some(LauncherAction::Launch(path));
                }
                None
            }
            // Escape -- hide
            0x1B => {
                self.hide();
                Some(LauncherAction::Hide)
            }
            // Backspace -- delete last search char
            0x08 => {
                if !self.search_query.is_empty() {
                    self.search_query.pop();
                    self.filter_entries();
                    self.clamp_selection();
                }
                // If search is now empty, revert to Visible state
                if self.search_query.is_empty() {
                    self.state = LauncherState::Visible;
                }
                None
            }
            // Arrow Up
            0x80 => {
                if self.selected_index >= self.grid_columns {
                    self.selected_index -= self.grid_columns;
                    self.ensure_visible();
                }
                None
            }
            // Arrow Down
            0x81 => {
                let new_idx = self.selected_index + self.grid_columns;
                if new_idx < self.filtered.len() {
                    self.selected_index = new_idx;
                    self.ensure_visible();
                }
                None
            }
            // Arrow Left
            0x82 => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.ensure_visible();
                }
                None
            }
            // Arrow Right
            0x83 => {
                if self.selected_index + 1 < self.filtered.len() {
                    self.selected_index += 1;
                    self.ensure_visible();
                }
                None
            }
            // Printable ASCII -- add to search
            0x20..=0x7E => {
                self.state = LauncherState::SearchActive;
                self.search_query.push(key as char);
                self.filter_entries();
                self.clamp_selection();
                None
            }
            _ => None,
        }
    }

    /// Handle a mouse click at absolute screen coordinates `(x, y)`.
    ///
    /// Returns `Some(LauncherAction::Launch(...))` if an app entry was clicked,
    /// or `None` if the click was outside the grid area (but still inside the
    /// overlay).
    pub fn handle_click(&mut self, x: usize, y: usize) -> Option<LauncherAction> {
        if self.state == LauncherState::Hidden {
            return None;
        }

        // Convert from screen coordinates to overlay-local coordinates
        let local_x = if x >= self.overlay_x {
            x - self.overlay_x
        } else {
            return None;
        };
        let local_y = if y >= self.overlay_y {
            y - self.overlay_y
        } else {
            return None;
        };

        // Check bounds
        if local_x >= self.overlay_width || local_y >= self.overlay_height {
            return None;
        }

        // Grid layout constants (must match render_to_buffer)
        let search_bar_height = SEARCH_BAR_HEIGHT;
        let grid_top = search_bar_height + GRID_PADDING_TOP;
        let cell_w = self.cell_width();
        let cell_h = self.cell_height();

        if local_y < grid_top {
            // Click in search bar area -- activate search mode
            self.state = LauncherState::SearchActive;
            return None;
        }

        let grid_y = local_y - grid_top;
        let grid_x = if local_x >= GRID_PADDING_LEFT {
            local_x - GRID_PADDING_LEFT
        } else {
            return None;
        };

        // Determine which cell was clicked
        let col = grid_x / cell_w;
        let row = grid_y / cell_h;

        if col >= self.grid_columns {
            return None;
        }

        let entry_idx = self.scroll_offset + row * self.grid_columns + col;
        if entry_idx < self.filtered.len() {
            self.selected_index = entry_idx;
            let real_idx = self.filtered[entry_idx];
            if real_idx < self.entries.len() {
                let path = self.entries[real_idx].exec_path.clone();
                self.hide();
                return Some(LauncherAction::Launch(path));
            }
        }

        None
    }

    /// Filter `entries` by the current `search_query`.
    ///
    /// Performs a case-insensitive substring match on the application name.
    /// If the query is empty, all entries are shown.
    pub fn filter_entries(&mut self) {
        self.filtered.clear();

        if self.search_query.is_empty() {
            for i in 0..self.entries.len() {
                self.filtered.push(i);
            }
            return;
        }

        // Build a lowercase copy of the query for case-insensitive matching.
        // We do this manually to avoid pulling in Unicode tables.
        let query_lower: Vec<u8> = self.search_query.bytes().map(ascii_to_lower).collect();

        for (i, entry) in self.entries.iter().enumerate() {
            if ascii_contains_lower(entry.name.as_bytes(), &query_lower) {
                self.filtered.push(i);
            }
        }
    }

    /// Get a reference to the currently selected application entry.
    pub fn selected_entry(&self) -> Option<&AppEntry> {
        let idx = self.filtered.get(self.selected_index)?;
        self.entries.get(*idx)
    }

    /// Get the indices of visible (filtered) entries.
    pub fn visible_entries(&self) -> &[usize] {
        &self.filtered
    }

    /// Get the total number of filtered entries.
    pub fn filtered_count(&self) -> usize {
        self.filtered.len()
    }

    /// Get a reference to all registered entries.
    pub fn entries(&self) -> &[AppEntry] {
        &self.entries
    }

    /// Render the launcher overlay into a u32 BGRA pixel buffer.
    ///
    /// The buffer dimensions must be `buf_width * buf_height` elements.
    /// The launcher is drawn at its configured overlay position.
    pub fn render_to_buffer(&self, buffer: &mut [u32], buf_width: usize, buf_height: usize) {
        if self.state == LauncherState::Hidden {
            return;
        }

        let ov_x = self.overlay_x;
        let ov_y = self.overlay_y;
        let ov_w = self.overlay_width;
        let ov_h = self.overlay_height;

        // --- 1. Semi-transparent dark background overlay ---
        let bg_color: u32 = 0xCC222222; // ~80% opaque dark grey
        for row in ov_y..(ov_y + ov_h).min(buf_height) {
            for col in ov_x..(ov_x + ov_w).min(buf_width) {
                let idx = row * buf_width + col;
                if idx < buffer.len() {
                    buffer[idx] = alpha_blend(buffer[idx], bg_color);
                }
            }
        }

        // --- 2. Overlay border (1px lighter line) ---
        let border_color: u32 = 0xFF555555;
        // Top edge
        if ov_y < buf_height {
            for col in ov_x..(ov_x + ov_w).min(buf_width) {
                let idx = ov_y * buf_width + col;
                if idx < buffer.len() {
                    buffer[idx] = border_color;
                }
            }
        }
        // Bottom edge
        let bottom_y = ov_y + ov_h - 1;
        if bottom_y < buf_height {
            for col in ov_x..(ov_x + ov_w).min(buf_width) {
                let idx = bottom_y * buf_width + col;
                if idx < buffer.len() {
                    buffer[idx] = border_color;
                }
            }
        }
        // Left edge
        for row in ov_y..(ov_y + ov_h).min(buf_height) {
            let idx = row * buf_width + ov_x;
            if idx < buffer.len() && ov_x < buf_width {
                buffer[idx] = border_color;
            }
        }
        // Right edge
        let right_x = ov_x + ov_w - 1;
        if right_x < buf_width {
            for row in ov_y..(ov_y + ov_h).min(buf_height) {
                let idx = row * buf_width + right_x;
                if idx < buffer.len() {
                    buffer[idx] = border_color;
                }
            }
        }

        // --- 3. Search bar ---
        let search_y = ov_y + SEARCH_BAR_MARGIN_TOP;
        let search_x = ov_x + SEARCH_BAR_MARGIN_LEFT;
        let search_w = ov_w - SEARCH_BAR_MARGIN_LEFT * 2;
        let search_h = SEARCH_BAR_INNER_HEIGHT;

        // Search bar background
        let search_bg = if self.state == LauncherState::SearchActive {
            0xFF3A3A3A
        } else {
            0xFF333333
        };
        for row in search_y..(search_y + search_h).min(buf_height) {
            for col in search_x..(search_x + search_w).min(buf_width) {
                let idx = row * buf_width + col;
                if idx < buffer.len() {
                    buffer[idx] = search_bg;
                }
            }
        }

        // Search bar border
        let search_border = if self.state == LauncherState::SearchActive {
            0xFF6688AA
        } else {
            0xFF555555
        };
        draw_rect_outline(
            buffer,
            buf_width,
            buf_height,
            search_x,
            search_y,
            search_w,
            search_h,
            search_border,
        );

        // Search text or placeholder
        let text_y = search_y + (search_h.saturating_sub(FONT_HEIGHT)) / 2;
        let text_x = search_x + 8;
        if self.search_query.is_empty() {
            // Placeholder
            draw_text_u32(
                buffer,
                buf_width,
                buf_height,
                b"Search applications...",
                text_x,
                text_y,
                0xFF777777,
            );
        } else {
            draw_text_u32(
                buffer,
                buf_width,
                buf_height,
                self.search_query.as_bytes(),
                text_x,
                text_y,
                0xFFDDDDDD,
            );
            // Cursor (blinking approximation: always show)
            let cursor_x = text_x + self.search_query.len() * FONT_WIDTH;
            for row in text_y..(text_y + FONT_HEIGHT).min(buf_height) {
                let idx = row * buf_width + cursor_x;
                if idx < buffer.len() && cursor_x < buf_width {
                    buffer[idx] = 0xFFCCCCCC;
                }
            }
        }

        // --- 4. Application grid ---
        let grid_top = ov_y + SEARCH_BAR_HEIGHT + GRID_PADDING_TOP;
        let grid_left = ov_x + GRID_PADDING_LEFT;
        let cell_w = self.cell_width();
        let cell_h = self.cell_height();

        let visible_count = self.grid_columns * self.grid_rows;
        let start = self.scroll_offset;
        let end = (start + visible_count).min(self.filtered.len());

        for display_idx in start..end {
            let local_idx = display_idx - start;
            let col = local_idx % self.grid_columns;
            let row = local_idx / self.grid_columns;

            let cell_x = grid_left + col * cell_w;
            let cell_y = grid_top + row * cell_h;

            let real_idx = self.filtered[display_idx];
            if real_idx >= self.entries.len() {
                continue;
            }
            let entry = &self.entries[real_idx];

            // Highlight selected entry
            let is_selected = display_idx == self.selected_index;
            if is_selected {
                let hl_color: u32 = 0xFF445566;
                for ry in cell_y..(cell_y + cell_h).min(buf_height) {
                    for rx in cell_x..(cell_x + cell_w).min(buf_width) {
                        let idx = ry * buf_width + rx;
                        if idx < buffer.len() {
                            buffer[idx] = hl_color;
                        }
                    }
                }
            }

            // Icon placeholder: a colored rectangle based on category
            let icon_size = ICON_SIZE;
            let icon_x = cell_x + (cell_w.saturating_sub(icon_size)) / 2;
            let icon_y = cell_y + ICON_MARGIN_TOP;
            let icon_color = category_color(&entry.category);

            for ry in icon_y..(icon_y + icon_size).min(buf_height) {
                for rx in icon_x..(icon_x + icon_size).min(buf_width) {
                    let idx = ry * buf_width + rx;
                    if idx < buffer.len() {
                        buffer[idx] = icon_color;
                    }
                }
            }

            // Draw a small letter inside the icon (first char of name)
            if !entry.name.is_empty() {
                let first_char = entry.name.as_bytes()[0];
                let char_x = icon_x + (icon_size.saturating_sub(FONT_WIDTH)) / 2;
                let char_y = icon_y + (icon_size.saturating_sub(FONT_HEIGHT)) / 2;
                draw_char_u32(
                    buffer, buf_width, buf_height, first_char, char_x, char_y, 0xFFFFFFFF,
                );
            }

            // Application name below the icon (centered, truncated)
            let name_bytes = entry.name.as_bytes();
            let max_name_chars = cell_w / FONT_WIDTH;
            let name_len = name_bytes.len().min(max_name_chars);
            let name_pixel_w = name_len * FONT_WIDTH;
            let name_x = cell_x + (cell_w.saturating_sub(name_pixel_w)) / 2;
            let name_y = icon_y + icon_size + NAME_MARGIN_TOP;
            let name_color = if is_selected { 0xFFFFFFFF } else { 0xFFCCCCCC };
            draw_text_u32(
                buffer,
                buf_width,
                buf_height,
                &name_bytes[..name_len],
                name_x,
                name_y,
                name_color,
            );

            // Description below the name (smaller, dimmer, single line)
            if !entry.description.is_empty() {
                let desc_bytes = entry.description.as_bytes();
                let max_desc_chars = cell_w / FONT_WIDTH;
                let desc_len = desc_bytes.len().min(max_desc_chars);
                let desc_pixel_w = desc_len * FONT_WIDTH;
                let desc_x = cell_x + (cell_w.saturating_sub(desc_pixel_w)) / 2;
                let desc_y = name_y + FONT_HEIGHT + 2;
                draw_text_u32(
                    buffer,
                    buf_width,
                    buf_height,
                    &desc_bytes[..desc_len],
                    desc_x,
                    desc_y,
                    0xFF888888,
                );
            }
        }

        // --- 5. Scroll indicator ---
        let total_pages = (self.filtered.len() + visible_count - 1) / visible_count.max(1);
        if total_pages > 1 {
            let current_page = self.scroll_offset / visible_count.max(1);
            // Draw small dots at the bottom center of the overlay
            let dots_y = ov_y + ov_h - 16;
            let dots_total_w = total_pages * 12;
            let dots_x = ov_x + (ov_w.saturating_sub(dots_total_w)) / 2;

            for page in 0..total_pages {
                let dot_x = dots_x + page * 12 + 2;
                let dot_color = if page == current_page {
                    0xFFDDDDDD
                } else {
                    0xFF666666
                };
                // Draw a 6x6 dot
                for ry in dots_y..(dots_y + 6).min(buf_height) {
                    for rx in dot_x..(dot_x + 6).min(buf_width) {
                        let idx = ry * buf_width + rx;
                        if idx < buffer.len() {
                            buffer[idx] = dot_color;
                        }
                    }
                }
            }
        }

        // --- 6. Result count indicator ---
        let count_text = format_count(self.filtered.len(), self.entries.len());
        let count_y = ov_y + ov_h - 16;
        let count_x = ov_x + 8;
        draw_text_u32(
            buffer,
            buf_width,
            buf_height,
            count_text.as_bytes(),
            count_x,
            count_y,
            0xFF666666,
        );
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Width of each grid cell in pixels.
    fn cell_width(&self) -> usize {
        let usable = self.overlay_width - GRID_PADDING_LEFT * 2;
        usable / self.grid_columns.max(1)
    }

    /// Height of each grid cell in pixels.
    fn cell_height(&self) -> usize {
        let grid_area_h =
            self.overlay_height - SEARCH_BAR_HEIGHT - GRID_PADDING_TOP - GRID_PADDING_BOTTOM;
        grid_area_h / self.grid_rows.max(1)
    }

    /// Clamp `selected_index` to the valid range after filtering.
    fn clamp_selection(&mut self) {
        if self.filtered.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= self.filtered.len() {
            self.selected_index = self.filtered.len() - 1;
        }
    }

    /// Adjust `scroll_offset` so the `selected_index` is within the visible
    /// grid page.
    fn ensure_visible(&mut self) {
        let page_size = self.grid_columns * self.grid_rows;
        if page_size == 0 {
            return;
        }

        // Scroll down if selection is below visible range
        while self.selected_index >= self.scroll_offset + page_size {
            self.scroll_offset += self.grid_columns;
        }
        // Scroll up if selection is above visible range
        while self.selected_index < self.scroll_offset && self.scroll_offset > 0 {
            self.scroll_offset = self.scroll_offset.saturating_sub(self.grid_columns);
        }
    }
}

impl Default for AppLauncher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

/// Font glyph width in pixels (8x16 VGA font).
const FONT_WIDTH: usize = 8;
/// Font glyph height in pixels (8x16 VGA font).
const FONT_HEIGHT: usize = 16;

/// Total height reserved for the search bar area (including margins).
const SEARCH_BAR_HEIGHT: usize = 48;
/// Top margin above the search bar input field.
const SEARCH_BAR_MARGIN_TOP: usize = 12;
/// Left/right margin of the search bar input field.
const SEARCH_BAR_MARGIN_LEFT: usize = 16;
/// Inner height of the search bar input field.
const SEARCH_BAR_INNER_HEIGHT: usize = 28;

/// Top padding between search bar and grid area.
const GRID_PADDING_TOP: usize = 12;
/// Bottom padding below the grid area.
const GRID_PADDING_BOTTOM: usize = 24;
/// Left padding before the first grid column.
const GRID_PADDING_LEFT: usize = 16;

/// Icon placeholder size in pixels (square).
const ICON_SIZE: usize = 48;
/// Margin above the icon within a grid cell.
const ICON_MARGIN_TOP: usize = 8;
/// Margin between the icon and the application name text.
const NAME_MARGIN_TOP: usize = 4;

// ---------------------------------------------------------------------------
// .desktop file parser
// ---------------------------------------------------------------------------

/// Parse a freedesktop .desktop file and extract an `AppEntry`.
///
/// Supports the `[Desktop Entry]` section and the following keys:
/// - `Name=` -- application name
/// - `Exec=` -- executable path (first token only, `%f/%u/%F/%U` stripped)
/// - `Icon=` -- icon name
/// - `Comment=` -- short description
/// - `Categories=` -- semicolon-separated category list (first recognized
///   category is used)
///
/// Returns `None` if `Name` or `Exec` is missing.
pub fn parse_desktop_file(content: &str) -> Option<AppEntry> {
    let mut name: Option<&str> = None;
    let mut exec: Option<&str> = None;
    let mut icon: Option<&str> = None;
    let mut comment: Option<&str> = None;
    let mut categories_raw: Option<&str> = None;
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Section header
        if trimmed.starts_with('[') {
            in_desktop_entry = trimmed == "[Desktop Entry]";
            continue;
        }

        if !in_desktop_entry {
            continue;
        }

        // Skip comments
        if trimmed.starts_with('#') {
            continue;
        }

        if let Some(val) = strip_key(trimmed, "Name=") {
            name = Some(val);
        } else if let Some(val) = strip_key(trimmed, "Exec=") {
            exec = Some(val);
        } else if let Some(val) = strip_key(trimmed, "Icon=") {
            icon = Some(val);
        } else if let Some(val) = strip_key(trimmed, "Comment=") {
            comment = Some(val);
        } else if let Some(val) = strip_key(trimmed, "Categories=") {
            categories_raw = Some(val);
        }
    }

    let name_str = name?;
    let exec_str = exec?;

    // Strip field codes (%f, %u, %F, %U, etc.) from exec path
    let exec_clean = strip_field_codes(exec_str);

    let category = categories_raw
        .and_then(parse_category_string)
        .unwrap_or(AppCategory::Other);

    Some(AppEntry {
        name: String::from(name_str),
        exec_path: exec_clean,
        icon_name: String::from(icon.unwrap_or("")),
        category,
        description: String::from(comment.unwrap_or("")),
    })
}

/// Strip a key prefix from a line and return the value portion.
fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key).map(|s| s.trim())
}

/// Strip freedesktop field codes (%f, %u, etc.) from an Exec value.
fn strip_field_codes(exec: &str) -> String {
    let mut result = String::with_capacity(exec.len());
    let bytes = exec.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() {
            // Skip the % and the following character
            i += 2;
            // Skip any trailing space after the field code
            if i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    // Trim trailing whitespace
    while result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Parse a semicolon-separated categories string and return the first
/// recognized `AppCategory`.
fn parse_category_string(cats: &str) -> Option<AppCategory> {
    for segment in cats.split(';') {
        let cat = segment.trim();
        if cat.is_empty() {
            continue;
        }
        match cat {
            "System" | "Monitor" | "PackageManager" => return Some(AppCategory::System),
            "Utility" | "Accessibility" | "Calculator" | "Clock" => {
                return Some(AppCategory::Utility)
            }
            "Development" | "IDE" | "TextEditor" | "Debugger" | "WebDevelopment" => {
                return Some(AppCategory::Development)
            }
            "Graphics" | "2DGraphics" | "3DGraphics" | "RasterGraphics" | "VectorGraphics" => {
                return Some(AppCategory::Graphics)
            }
            "Network" | "WebBrowser" | "Email" | "Chat" | "IRCClient" | "FileTransfer" => {
                return Some(AppCategory::Network)
            }
            "AudioVideo" | "Audio" | "Video" | "Multimedia" | "Player" | "Recorder" => {
                return Some(AppCategory::Multimedia)
            }
            "Office" | "WordProcessor" | "Spreadsheet" | "Presentation" => {
                return Some(AppCategory::Office)
            }
            "Settings" | "Preferences" | "DesktopSettings" | "HardwareSettings" => {
                return Some(AppCategory::Settings)
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Default applications
// ---------------------------------------------------------------------------

/// Build the list of built-in default applications.
fn default_applications() -> Vec<AppEntry> {
    vec![
        AppEntry::new(
            "Terminal",
            "/usr/bin/terminal",
            "utilities-terminal",
            AppCategory::System,
            "Terminal emulator",
        ),
        AppEntry::new(
            "File Manager",
            "/usr/bin/files",
            "system-file-manager",
            AppCategory::System,
            "Browse files",
        ),
        AppEntry::new(
            "Text Editor",
            "/usr/bin/editor",
            "accessories-text-editor",
            AppCategory::Utility,
            "Edit text files",
        ),
        AppEntry::new(
            "Settings",
            "/usr/bin/settings",
            "preferences-system",
            AppCategory::Settings,
            "System settings",
        ),
        AppEntry::new(
            "System Monitor",
            "/usr/bin/sysmonitor",
            "utilities-system-monitor",
            AppCategory::System,
            "Monitor CPU and memory",
        ),
        AppEntry::new(
            "Image Viewer",
            "/usr/bin/image-viewer",
            "eog",
            AppCategory::Graphics,
            "View images",
        ),
        AppEntry::new(
            "Media Player",
            "/usr/bin/mediaplayer",
            "multimedia-player",
            AppCategory::Multimedia,
            "Play audio and video",
        ),
    ]
}

/// Return a BGRA color for a category icon placeholder.
///
/// Each category gets a distinctive color so users can quickly identify
/// application types at a glance.
pub fn category_color(cat: &AppCategory) -> u32 {
    match cat {
        AppCategory::System => 0xFF4488AA,      // Teal
        AppCategory::Utility => 0xFF66AA44,     // Green
        AppCategory::Development => 0xFF886644, // Brown/Amber
        AppCategory::Graphics => 0xFFAA6688,    // Rose
        AppCategory::Network => 0xFF4466AA,     // Blue
        AppCategory::Multimedia => 0xFFAA4466,  // Magenta
        AppCategory::Office => 0xFF666699,      // Slate blue
        AppCategory::Settings => 0xFF888888,    // Grey
        AppCategory::Other => 0xFF555555,       // Dark grey
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers (u32 pixel buffer)
// ---------------------------------------------------------------------------

/// Draw a single 8x16 character into a u32 (BGRA/XRGB) pixel buffer.
fn draw_char_u32(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    ch: u8,
    px: usize,
    py: usize,
    color: u32,
) {
    let glyph = crate::graphics::font8x16::glyph(ch);
    for (row, &bits) in glyph.iter().enumerate() {
        let y = py + row;
        if y >= buf_height {
            break;
        }
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 != 0 {
                let x = px + col;
                if x >= buf_width {
                    break;
                }
                let idx = y * buf_width + x;
                if idx < buffer.len() {
                    buffer[idx] = color;
                }
            }
        }
    }
}

/// Draw a byte string into a u32 pixel buffer using the 8x16 VGA font.
fn draw_text_u32(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    text: &[u8],
    px: usize,
    py: usize,
    color: u32,
) {
    for (i, &ch) in text.iter().enumerate() {
        draw_char_u32(
            buffer,
            buf_width,
            buf_height,
            ch,
            px + i * FONT_WIDTH,
            py,
            color,
        );
    }
}

/// Draw a 1px rectangle outline into a u32 pixel buffer.
fn draw_rect_outline(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    // Top and bottom edges
    for col in x..(x + w).min(buf_width) {
        if y < buf_height {
            let idx = y * buf_width + col;
            if idx < buffer.len() {
                buffer[idx] = color;
            }
        }
        let bottom = y + h - 1;
        if bottom < buf_height {
            let idx = bottom * buf_width + col;
            if idx < buffer.len() {
                buffer[idx] = color;
            }
        }
    }
    // Left and right edges
    for row in y..(y + h).min(buf_height) {
        if x < buf_width {
            let idx = row * buf_width + x;
            if idx < buffer.len() {
                buffer[idx] = color;
            }
        }
        let right = x + w - 1;
        if right < buf_width {
            let idx = row * buf_width + right;
            if idx < buffer.len() {
                buffer[idx] = color;
            }
        }
    }
}

/// Simple alpha blending of a foreground pixel over a background pixel.
///
/// Both pixels are in 0xAARRGGBB format. Uses integer-only arithmetic.
fn alpha_blend(bg: u32, fg: u32) -> u32 {
    let fg_a = (fg >> 24) & 0xFF;
    if fg_a == 0xFF {
        return fg;
    }
    if fg_a == 0 {
        return bg;
    }

    let inv_a = 255 - fg_a;

    let fg_r = (fg >> 16) & 0xFF;
    let fg_g = (fg >> 8) & 0xFF;
    let fg_b = fg & 0xFF;

    let bg_r = (bg >> 16) & 0xFF;
    let bg_g = (bg >> 8) & 0xFF;
    let bg_b = bg & 0xFF;

    // out = fg * alpha + bg * (1 - alpha), with integer division by 255
    let r = (fg_r * fg_a + bg_r * inv_a) / 255;
    let g = (fg_g * fg_a + bg_g * inv_a) / 255;
    let b = (fg_b * fg_a + bg_b * inv_a) / 255;

    0xFF000000 | (r << 16) | (g << 8) | b
}

// ---------------------------------------------------------------------------
// ASCII utility helpers
// ---------------------------------------------------------------------------

/// Convert an ASCII byte to lowercase (identity for non-alpha bytes).
fn ascii_to_lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

/// Case-insensitive check whether `haystack` contains the already-lowered
/// `needle` as a substring.
fn ascii_contains_lower(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    let limit = haystack.len() - needle.len();
    for start in 0..=limit {
        let mut matches = true;
        for (j, &nb) in needle.iter().enumerate() {
            if ascii_to_lower(haystack[start + j]) != nb {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }
    false
}

/// Format a "N of M apps" count string without pulling in `format!`.
fn format_count(filtered: usize, total: usize) -> String {
    let mut s = String::with_capacity(32);
    append_usize(&mut s, filtered);
    s.push_str(" of ");
    append_usize(&mut s, total);
    s.push_str(" apps");
    s
}

/// Append a `usize` as decimal digits to a `String`.
fn append_usize(s: &mut String, n: usize) {
    if n == 0 {
        s.push('0');
        return;
    }
    // Max digits for usize on 64-bit is 20
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    let mut val = n;
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    for &ch in &buf[pos..] {
        s.push(ch as char);
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Global application launcher instance.
static LAUNCHER: GlobalState<spin::Mutex<AppLauncher>> = GlobalState::new();

/// Initialize the application launcher.
pub fn init() -> Result<(), crate::error::KernelError> {
    LAUNCHER
        .init(spin::Mutex::new(AppLauncher::new()))
        .map_err(|_| crate::error::KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    crate::println!("[LAUNCHER] Application launcher initialized ({} apps)", 7);
    Ok(())
}

/// Execute a function with the application launcher (mutable access).
pub fn with_launcher<R, F: FnOnce(&mut AppLauncher) -> R>(f: F) -> Option<R> {
    LAUNCHER.with(|lock| {
        let mut launcher = lock.lock();
        f(&mut launcher)
    })
}

/// Execute a function with the application launcher (read-only access).
pub fn with_launcher_ref<R, F: FnOnce(&AppLauncher) -> R>(f: F) -> Option<R> {
    LAUNCHER.with(|lock| {
        let launcher = lock.lock();
        f(&launcher)
    })
}

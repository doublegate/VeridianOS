//! Alt-Tab Application Switcher
//!
//! Provides an overlay for cycling between open windows with thumbnails
//! and application icons. The switcher is shown while Alt is held and Tab
//! is pressed to cycle; releasing Alt commits the selection.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Application icon (simple geometric shape for now)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    Terminal,
    FileManager,
    TextEditor,
    Settings,
    ImageViewer,
    Generic,
}

/// Application switcher entry
pub struct SwitcherEntry {
    pub window_id: u32,
    pub title: String,
    pub app_icon: AppIcon,
    /// Down-scaled window content (XRGB8888 pixels, row-major)
    pub thumbnail: Option<Vec<u32>>,
    pub thumbnail_width: u32,
    pub thumbnail_height: u32,
}

// ---------------------------------------------------------------------------
// AppSwitcher
// ---------------------------------------------------------------------------

/// Switcher overlay state.
///
/// The overlay is drawn centered on screen and shows one entry per open
/// window. Entries are rendered as an icon + title, with the currently
/// selected entry highlighted.
pub struct AppSwitcher {
    entries: Vec<SwitcherEntry>,
    selected_index: usize,
    visible: bool,
    /// Top-left corner of the overlay (computed from screen size)
    overlay_x: u32,
    overlay_y: u32,
    overlay_width: u32,
    overlay_height: u32,
    /// Dimensions of a single entry cell
    entry_width: u32,
    entry_height: u32,
    padding: u32,
}

impl AppSwitcher {
    /// Create a new (hidden) application switcher.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected_index: 0,
            visible: false,
            overlay_x: 0,
            overlay_y: 0,
            overlay_width: 0,
            overlay_height: 0,
            entry_width: 120,
            entry_height: 100,
            padding: 12,
        }
    }

    /// Show the switcher overlay populated with the given windows.
    ///
    /// `windows` is a list of `(window_id, title)` pairs in Z-order (top
    /// first). The first entry is pre-selected.
    pub fn show(&mut self, windows: Vec<(u32, String)>) {
        self.entries.clear();
        self.selected_index = 0;

        for (wid, title) in windows {
            let icon = guess_icon(&title);
            self.entries.push(SwitcherEntry {
                window_id: wid,
                title,
                app_icon: icon,
                thumbnail: None,
                thumbnail_width: 0,
                thumbnail_height: 0,
            });
        }

        if !self.entries.is_empty() {
            // Start with the second entry selected (Alt-Tab skips current)
            if self.entries.len() > 1 {
                self.selected_index = 1;
            }
            self.visible = true;
        }
    }

    /// Hide the overlay and return the selected window ID (if any).
    pub fn hide(&mut self) -> Option<u32> {
        self.visible = false;
        let wid = self.entries.get(self.selected_index).map(|e| e.window_id);
        self.entries.clear();
        wid
    }

    /// Advance selection to the next entry (wraps around).
    pub fn next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.entries.len();
    }

    /// Move selection to the previous entry (wraps around).
    pub fn previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.entries.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Returns `true` if the overlay is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Get the currently selected window ID (without hiding the overlay).
    pub fn selected_window_id(&self) -> Option<u32> {
        self.entries.get(self.selected_index).map(|e| e.window_id)
    }

    /// Get the number of entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Render the switcher overlay into a u32 XRGB8888 buffer.
    ///
    /// The overlay is drawn centered on the screen. `buf_width` and
    /// `buf_height` are the full screen dimensions.
    pub fn render(&self, buffer: &mut [u32], buf_width: u32, buf_height: u32) {
        if !self.visible || self.entries.is_empty() {
            return;
        }

        let count = self.entries.len() as u32;
        let ew = self.entry_width;
        let eh = self.entry_height;
        let pad = self.padding;

        // Compute overlay bounds
        let total_width = count * ew + (count + 1) * pad;
        let total_height = eh + pad * 2;

        // Center on screen
        let ox = if total_width < buf_width {
            (buf_width - total_width) / 2
        } else {
            0
        };
        let oy = if total_height < buf_height {
            (buf_height - total_height) / 2
        } else {
            0
        };

        let bw = buf_width as usize;

        // Draw semi-transparent dark background
        let bg_color: u32 = 0xE0202020; // ~88% opaque dark gray
        for y in 0..total_height {
            for x in 0..total_width {
                let px = (ox + x) as usize;
                let py = (oy + y) as usize;
                if px < buf_width as usize && py < buf_height as usize {
                    let idx = py * bw + px;
                    if idx < buffer.len() {
                        // Alpha blend with existing content
                        let src_a = (bg_color >> 24) & 0xFF;
                        let src_r = (bg_color >> 16) & 0xFF;
                        let src_g = (bg_color >> 8) & 0xFF;
                        let src_b = bg_color & 0xFF;

                        let dst = buffer[idx];
                        let dst_r = (dst >> 16) & 0xFF;
                        let dst_g = (dst >> 8) & 0xFF;
                        let dst_b = dst & 0xFF;

                        let inv_a = 255 - src_a;
                        let r = (src_r * src_a + dst_r * inv_a) / 255;
                        let g = (src_g * src_a + dst_g * inv_a) / 255;
                        let b = (src_b * src_a + dst_b * inv_a) / 255;

                        buffer[idx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
                    }
                }
            }
        }

        // Draw rounded border
        let border_color: u32 = 0xFF5294E2; // Blue accent
                                            // Top and bottom edges
        for x in 0..total_width {
            let px = (ox + x) as usize;
            let py_top = oy as usize;
            let py_bot = (oy + total_height).saturating_sub(1) as usize;
            if px < buf_width as usize {
                let idx_t = py_top * bw + px;
                let idx_b = py_bot * bw + px;
                if idx_t < buffer.len() {
                    buffer[idx_t] = border_color;
                }
                if idx_b < buffer.len() {
                    buffer[idx_b] = border_color;
                }
            }
        }
        // Left and right edges
        for y in 0..total_height {
            let py = (oy + y) as usize;
            let px_left = ox as usize;
            let px_right = (ox + total_width).saturating_sub(1) as usize;
            if py < buf_height as usize {
                let idx_l = py * bw + px_left;
                let idx_r = py * bw + px_right;
                if idx_l < buffer.len() {
                    buffer[idx_l] = border_color;
                }
                if idx_r < buffer.len() {
                    buffer[idx_r] = border_color;
                }
            }
        }

        // Draw each entry
        for (i, entry) in self.entries.iter().enumerate() {
            let entry_x = ox + pad + (i as u32) * (ew + pad);
            let entry_y = oy + pad;
            let selected = i == self.selected_index;

            self.render_entry(buffer, buf_width, entry_x, entry_y, entry, selected);
        }
    }

    /// Render a single switcher entry at position `(x, y)` in the buffer.
    fn render_entry(
        &self,
        buffer: &mut [u32],
        buf_width: u32,
        x: u32,
        y: u32,
        entry: &SwitcherEntry,
        selected: bool,
    ) {
        let bw = buf_width as usize;
        let ew = self.entry_width as usize;
        let eh = self.entry_height as usize;

        // Selection highlight background
        let entry_bg = if selected {
            0xFF3A5F8A // Highlighted blue
        } else {
            0xFF2A2A2A // Dark background
        };

        for dy in 0..eh {
            for dx in 0..ew {
                let px = x as usize + dx;
                let py = y as usize + dy;
                if px < buf_width as usize {
                    let idx = py * bw + px;
                    if idx < buffer.len() {
                        buffer[idx] = entry_bg;
                    }
                }
            }
        }

        // Selection border
        if selected {
            let sel_border: u32 = 0xFF7AB4FF; // Light blue
            for dx in 0..ew {
                let px = x as usize + dx;
                // Top
                let idx_t = y as usize * bw + px;
                if idx_t < buffer.len() {
                    buffer[idx_t] = sel_border;
                }
                // Bottom
                let idx_b = (y as usize + eh - 1) * bw + px;
                if idx_b < buffer.len() {
                    buffer[idx_b] = sel_border;
                }
            }
            for dy in 0..eh {
                let py = y as usize + dy;
                // Left
                let idx_l = py * bw + x as usize;
                if idx_l < buffer.len() {
                    buffer[idx_l] = sel_border;
                }
                // Right
                let idx_r = py * bw + x as usize + ew - 1;
                if idx_r < buffer.len() {
                    buffer[idx_r] = sel_border;
                }
            }
        }

        // Draw icon centered in the upper portion
        let icon_size: u32 = 32;
        let icon_x = x + (self.entry_width.saturating_sub(icon_size)) / 2;
        let icon_y = y + 8;
        render_icon(buffer, buf_width, icon_x, icon_y, icon_size, entry.app_icon);

        // Draw title text centered below the icon (truncated to fit)
        let title_y = y + 8 + icon_size + 8;
        let max_chars = (self.entry_width / 8) as usize;
        let title_bytes = entry.title.as_bytes();
        let display_len = title_bytes.len().min(max_chars);
        let text_pixel_w = display_len * 8;
        let title_x = x + (self.entry_width.saturating_sub(text_pixel_w as u32)) / 2;

        let text_color: u32 = 0xFFFFFFFF;
        let r = (text_color >> 16) & 0xFF;
        let g = (text_color >> 8) & 0xFF;
        let b = text_color & 0xFF;
        let pixel = 0xFF00_0000 | (r << 16) | (g << 8) | b;

        for (ci, &ch) in title_bytes[..display_len].iter().enumerate() {
            let glyph = crate::graphics::font8x16::glyph(ch);
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if (bits >> (7 - col)) & 1 != 0 {
                        let px = title_x as usize + ci * 8 + col;
                        let py = title_y as usize + row;
                        if px < buf_width as usize {
                            let idx = py * bw + px;
                            if idx < buffer.len() {
                                buffer[idx] = pixel;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Default for AppSwitcher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Icon rendering
// ---------------------------------------------------------------------------

/// Render a simple geometric icon for an application type.
///
/// Uses basic line/shape drawing (no bitmaps) into a u32 XRGB8888 buffer.
pub fn render_icon(buffer: &mut [u32], buf_width: u32, x: u32, y: u32, size: u32, icon: AppIcon) {
    let bw = buf_width as usize;
    let sz = size as usize;
    let bx = x as usize;
    let by = y as usize;

    match icon {
        AppIcon::Terminal => {
            // Terminal: >_ prompt shape
            let fg: u32 = 0xFF00CC66; // Green
            let bg: u32 = 0xFF1A1A1A; // Dark background

            // Fill background
            fill_rect(buffer, bw, bx, by, sz, sz, bg);

            // Draw ">" character (two diagonal lines forming an arrow)
            let m = sz / 6; // margin
            let mid_y = sz / 2;
            // Top arm of >
            for i in 0..(mid_y - m) {
                let px = bx + m + i;
                let py = by + m + i;
                if px < buf_width as usize {
                    let idx = py * bw + px;
                    if idx < buffer.len() {
                        buffer[idx] = fg;
                    }
                }
            }
            // Bottom arm of >
            for i in 0..(mid_y - m) {
                let px = bx + m + i;
                let py = by + mid_y + (mid_y - m).saturating_sub(1 + i);
                if px < buf_width as usize {
                    let idx = py * bw + px;
                    if idx < buffer.len() {
                        buffer[idx] = fg;
                    }
                }
            }
            // Draw "_" underscore
            let uy = by + mid_y + m;
            let ux_start = bx + mid_y;
            let ux_end = bx + sz - m;
            for px in ux_start..ux_end {
                if px < buf_width as usize {
                    let idx = uy * bw + px;
                    if idx < buffer.len() {
                        buffer[idx] = fg;
                    }
                }
            }
        }
        AppIcon::FileManager => {
            // Folder shape
            let fg: u32 = 0xFFDDAA22; // Golden yellow

            // Fill main folder body
            let tab_h = sz / 4;
            let tab_w = sz / 2;
            // Tab on top-left
            fill_rect(buffer, bw, bx + 2, by + 2, tab_w, tab_h, fg);
            // Main body
            fill_rect(
                buffer,
                bw,
                bx + 2,
                by + 2 + tab_h,
                sz - 4,
                sz - tab_h - 4,
                fg,
            );
        }
        AppIcon::TextEditor => {
            // Document shape with lines
            let bg: u32 = 0xFFEEEEEE; // Light paper
            let fg: u32 = 0xFF333333; // Dark text lines

            // Paper background
            fill_rect(buffer, bw, bx + 4, by + 2, sz - 8, sz - 4, bg);

            // Horizontal text lines
            let line_gap = sz / 6;
            for i in 1..5 {
                let ly = by + 4 + i * line_gap;
                let lx_start = bx + 8;
                let lx_end = bx + sz - 8;
                for px in lx_start..lx_end.min(bx + sz) {
                    if px < buf_width as usize && ly < by + sz {
                        let idx = ly * bw + px;
                        if idx < buffer.len() {
                            buffer[idx] = fg;
                        }
                    }
                }
            }
        }
        AppIcon::Settings => {
            // Gear shape (simplified as octagon)
            let fg: u32 = 0xFF888888; // Gray

            let center = sz / 2;
            let outer_r = (sz / 2).saturating_sub(2);
            let inner_r = outer_r / 2;
            let outer_sq = (outer_r * outer_r) as i32;
            let inner_sq = (inner_r * inner_r) as i32;

            for dy in 0..sz {
                for dx in 0..sz {
                    let cx = dx as i32 - center as i32;
                    let cy = dy as i32 - center as i32;
                    let dist_sq = cx * cx + cy * cy;
                    if dist_sq <= outer_sq && dist_sq >= inner_sq {
                        let px = bx + dx;
                        let py = by + dy;
                        if px < buf_width as usize {
                            let idx = py * bw + px;
                            if idx < buffer.len() {
                                buffer[idx] = fg;
                            }
                        }
                    }
                }
            }
        }
        AppIcon::ImageViewer => {
            // Mountain/landscape shape
            let sky: u32 = 0xFF6699CC; // Light blue
            let mtn: u32 = 0xFF336633; // Dark green

            // Sky background
            fill_rect(buffer, bw, bx + 2, by + 2, sz - 4, sz - 4, sky);

            // Mountain triangle (centered)
            let base_y = by + sz - 4;
            let peak_x = bx + sz / 2;
            let peak_y = by + sz / 4;
            let half_base = sz / 3;

            for row_y in peak_y..base_y {
                let progress = row_y - peak_y;
                let total = base_y - peak_y;
                if total == 0 {
                    continue;
                }
                let half_w = (progress * half_base) / total;
                let start_x = peak_x.saturating_sub(half_w);
                let end_x = peak_x + half_w;
                for px in start_x..end_x.min(bx + sz - 2) {
                    if px < buf_width as usize {
                        let idx = row_y * bw + px;
                        if idx < buffer.len() {
                            buffer[idx] = mtn;
                        }
                    }
                }
            }
        }
        AppIcon::Generic => {
            // Default: simple bordered square
            let fg: u32 = 0xFF6688AA;
            let bg: u32 = 0xFF334455;

            fill_rect(buffer, bw, bx + 2, by + 2, sz - 4, sz - 4, bg);

            // Border
            for i in 0..sz {
                // Top
                set_pixel(buffer, bw, bx + i, by + 2, fg);
                // Bottom
                set_pixel(buffer, bw, bx + i, by + sz - 3, fg);
                // Left
                set_pixel(buffer, bw, bx + 2, by + i, fg);
                // Right
                set_pixel(buffer, bw, bx + sz - 3, by + i, fg);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: guess icon from window title
// ---------------------------------------------------------------------------

/// Heuristic to guess an AppIcon from the window title.
fn guess_icon(title: &str) -> AppIcon {
    let lower: Vec<u8> = title.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let lower_str = core::str::from_utf8(&lower).unwrap_or("");

    if lower_str.contains("terminal")
        || lower_str.contains("shell")
        || lower_str.contains("console")
    {
        AppIcon::Terminal
    } else if lower_str.contains("file") || lower_str.contains("folder") {
        AppIcon::FileManager
    } else if lower_str.contains("editor")
        || lower_str.contains("text")
        || lower_str.contains("code")
    {
        AppIcon::TextEditor
    } else if lower_str.contains("setting") || lower_str.contains("config") {
        AppIcon::Settings
    } else if lower_str.contains("image")
        || lower_str.contains("photo")
        || lower_str.contains("view")
    {
        AppIcon::ImageViewer
    } else {
        AppIcon::Generic
    }
}

// ---------------------------------------------------------------------------
// Pixel helpers
// ---------------------------------------------------------------------------

/// Fill a rectangle in a u32 pixel buffer.
fn fill_rect(
    buffer: &mut [u32],
    buf_width: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    for dy in 0..h {
        for dx in 0..w {
            let idx = (y + dy) * buf_width + (x + dx);
            if idx < buffer.len() {
                buffer[idx] = color;
            }
        }
    }
}

/// Set a single pixel in a u32 buffer (bounds-checked).
fn set_pixel(buffer: &mut [u32], buf_width: usize, x: usize, y: usize, color: u32) {
    let idx = y * buf_width + x;
    if idx < buffer.len() {
        buffer[idx] = color;
    }
}

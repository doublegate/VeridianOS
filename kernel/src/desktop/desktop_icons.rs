//! Desktop Icons
//!
//! Provides desktop icon rendering, interaction (click, double-click, drag),
//! and `.desktop`-file parsing. Icons are arranged on a grid and can be
//! dragged to new positions with snap-to-grid alignment.

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

// ---------------------------------------------------------------------------
// Desktop file parser
// ---------------------------------------------------------------------------

/// Parsed `.desktop` file (freedesktop-style).
#[derive(Debug, Clone)]
pub struct DesktopFile {
    /// Display name (from `Name=`).
    pub name: String,
    /// Command to execute (from `Exec=`).
    pub exec_command: String,
    /// Icon identifier (from `Icon=`).
    pub icon: String,
    /// Categories list (from `Categories=`, semicolon-separated).
    pub categories: Vec<String>,
    /// Supported MIME types (from `MimeType=`, semicolon-separated).
    pub mime_types: Vec<String>,
}

impl DesktopFile {
    /// Parse a `.desktop` file from its textual content.
    pub fn parse(content: &str) -> Option<Self> {
        let mut name = String::new();
        let mut exec_command = String::new();
        let mut icon = String::new();
        let mut categories = Vec::new();
        let mut mime_types = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }

            if let Some(val) = line.strip_prefix("Name=") {
                name = String::from(val);
            } else if let Some(val) = line.strip_prefix("Exec=") {
                exec_command = String::from(val);
            } else if let Some(val) = line.strip_prefix("Icon=") {
                icon = String::from(val);
            } else if let Some(val) = line.strip_prefix("Categories=") {
                categories = val
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
            } else if let Some(val) = line.strip_prefix("MimeType=") {
                mime_types = val
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(Self {
            name,
            exec_command,
            icon,
            categories,
            mime_types,
        })
    }
}

// ---------------------------------------------------------------------------
// Desktop icon
// ---------------------------------------------------------------------------

/// Icon size in pixels (square).
pub const ICON_SIZE: u32 = 16;

/// A single desktop icon.
#[derive(Debug, Clone)]
pub struct DesktopIcon {
    /// Display name (rendered below the icon).
    pub name: String,
    /// 16x16 pixel data (ARGB8888, row-major).
    pub icon_data: Vec<u32>,
    /// X position on the desktop (pixels).
    pub x: i32,
    /// Y position on the desktop (pixels).
    pub y: i32,
    /// Whether this icon is currently selected.
    pub selected: bool,
    /// Path to the associated `.desktop` file.
    pub desktop_file_path: String,
    /// Parsed desktop file data.
    pub desktop_file: Option<DesktopFile>,
}

impl DesktopIcon {
    /// Create a new icon with a default solid colour.
    pub fn new(name: &str, x: i32, y: i32) -> Self {
        Self {
            name: String::from(name),
            icon_data: vec![0xFF4488CC; (ICON_SIZE * ICON_SIZE) as usize],
            x,
            y,
            selected: false,
            desktop_file_path: String::new(),
            desktop_file: None,
        }
    }

    /// Set custom icon pixel data (must be ICON_SIZE x ICON_SIZE).
    pub fn set_icon_data(&mut self, data: &[u32]) {
        let expected = (ICON_SIZE * ICON_SIZE) as usize;
        if data.len() == expected {
            self.icon_data.clear();
            self.icon_data.extend_from_slice(data);
        }
    }

    /// Total bounding box height (icon + label gap + label line).
    pub fn total_height(&self) -> u32 {
        // Icon (16) + gap (4) + label line (16)
        ICON_SIZE + 4 + 16
    }

    /// Hit test: does the point lie within this icon's bounding box?
    pub fn hit_test(&self, px: i32, py: i32) -> bool {
        let w = ICON_SIZE as i32;
        let h = self.total_height() as i32;
        px >= self.x && px < self.x + w && py >= self.y && py < self.y + h
    }
}

// ---------------------------------------------------------------------------
// Icon grid
// ---------------------------------------------------------------------------

/// Grid-based layout manager for desktop icons.
#[derive(Debug)]
pub struct IconGrid {
    /// All desktop icons.
    pub icons: Vec<DesktopIcon>,
    /// Horizontal spacing between grid cells (pixels).
    pub grid_spacing_x: u32,
    /// Vertical spacing between grid cells (pixels).
    pub grid_spacing_y: u32,
    /// Icon cell size (width = height = ICON_SIZE + padding).
    pub cell_size: u32,
    /// Desktop width for grid calculations.
    desktop_width: u32,
    /// Desktop height for grid calculations.
    desktop_height: u32,
}

impl IconGrid {
    /// Create a new icon grid for a desktop of the given dimensions.
    pub fn new(desktop_width: u32, desktop_height: u32) -> Self {
        Self {
            icons: Vec::new(),
            grid_spacing_x: 80,
            grid_spacing_y: 80,
            cell_size: 64,
            desktop_width,
            desktop_height,
        }
    }

    /// Add an icon to the grid.
    pub fn add_icon(&mut self, icon: DesktopIcon) {
        self.icons.push(icon);
    }

    /// Remove an icon by index.
    pub fn remove_icon(&mut self, index: usize) -> Option<DesktopIcon> {
        if index < self.icons.len() {
            Some(self.icons.remove(index))
        } else {
            None
        }
    }

    /// Auto-arrange all icons in a grid layout (top-left to bottom-right).
    pub fn arrange(&mut self) {
        let margin_x: i32 = 20;
        let margin_y: i32 = 40;
        let cols = if self.grid_spacing_x == 0 {
            1
        } else {
            ((self.desktop_width as i32 - margin_x * 2) / self.grid_spacing_x as i32).max(1)
        };

        for (i, icon) in self.icons.iter_mut().enumerate() {
            let col = (i as i32) % cols;
            let row = (i as i32) / cols;
            icon.x = margin_x + col * self.grid_spacing_x as i32;
            icon.y = margin_y + row * self.grid_spacing_y as i32;
        }
    }

    /// Snap a position to the nearest grid cell.
    pub fn snap_to_grid(&self, x: i32, y: i32) -> (i32, i32) {
        let gx = self.grid_spacing_x as i32;
        let gy = self.grid_spacing_y as i32;
        if gx == 0 || gy == 0 {
            return (x, y);
        }
        let sx = ((x + gx / 2) / gx) * gx;
        let sy = ((y + gy / 2) / gy) * gy;
        (sx.max(0), sy.max(0))
    }

    /// Render a single icon into a pixel buffer.
    ///
    /// `buf` is `buf_width x buf_height`, ARGB8888 row-major.
    pub fn render_icon(icon: &DesktopIcon, buf: &mut [u32], buf_width: u32, buf_height: u32) {
        let bw = buf_width as i32;
        let bh = buf_height as i32;
        let iw = ICON_SIZE as i32;

        // Draw icon bitmap
        for row in 0..ICON_SIZE as i32 {
            let dy = icon.y + row;
            if dy < 0 || dy >= bh {
                continue;
            }
            for col in 0..iw {
                let dx = icon.x + col;
                if dx < 0 || dx >= bw {
                    continue;
                }
                let src = icon.icon_data[(row * iw + col) as usize];
                buf[(dy * bw + dx) as usize] = src;
            }
        }

        // Draw selection highlight (1px border around icon)
        if icon.selected {
            let color = 0xFF44AAFF; // light blue
            let x0 = icon.x - 1;
            let y0 = icon.y - 1;
            let x1 = icon.x + iw;
            let y1 = icon.y + ICON_SIZE as i32;
            for dx in x0..=x1 {
                if dx >= 0 && dx < bw {
                    if y0 >= 0 && y0 < bh {
                        buf[(y0 * bw + dx) as usize] = color;
                    }
                    if y1 >= 0 && y1 < bh {
                        buf[(y1 * bw + dx) as usize] = color;
                    }
                }
            }
            for dy in y0..=y1 {
                if dy >= 0 && dy < bh {
                    if x0 >= 0 && x0 < bw {
                        buf[(dy * bw + x0) as usize] = color;
                    }
                    if x1 >= 0 && x1 < bw {
                        buf[(dy * bw + x1) as usize] = color;
                    }
                }
            }
        }
    }

    /// Handle a click at `(px, py)`.
    ///
    /// Returns the index of the clicked icon (if any). Deselects all others.
    pub fn handle_click(&mut self, px: i32, py: i32) -> Option<usize> {
        let mut clicked = None;

        for (i, icon) in self.icons.iter_mut().enumerate() {
            if icon.hit_test(px, py) {
                icon.selected = true;
                clicked = Some(i);
            } else {
                icon.selected = false;
            }
        }

        clicked
    }

    /// Handle a double-click at `(px, py)`.
    ///
    /// Returns the exec command of the double-clicked icon, if any.
    pub fn handle_double_click(&mut self, px: i32, py: i32) -> Option<String> {
        for icon in &self.icons {
            if icon.hit_test(px, py) {
                if let Some(ref df) = icon.desktop_file {
                    if !df.exec_command.is_empty() {
                        return Some(df.exec_command.clone());
                    }
                }
            }
        }
        None
    }

    /// Handle drag: move the selected icon to `(px, py)` with grid snapping.
    pub fn handle_drag(&mut self, px: i32, py: i32) {
        let (sx, sy) = self.snap_to_grid(px, py);
        for icon in &mut self.icons {
            if icon.selected {
                icon.x = sx;
                icon.y = sy;
                break;
            }
        }
    }

    /// Deselect all icons.
    pub fn deselect_all(&mut self) {
        for icon in &mut self.icons {
            icon.selected = false;
        }
    }

    /// Number of icons.
    pub fn icon_count(&self) -> usize {
        self.icons.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_file_parse() {
        let content = concat!(
            "[Desktop Entry]\n",
            "Name=Terminal\n",
            "Exec=vterm\n",
            "Icon=terminal\n",
            "Categories=System;Utility;\n",
            "MimeType=text/plain;\n",
        );
        let df = DesktopFile::parse(content).unwrap();
        assert_eq!(df.name, "Terminal");
        assert_eq!(df.exec_command, "vterm");
        assert_eq!(df.categories.len(), 2);
        assert_eq!(df.mime_types.len(), 1);
    }

    #[test]
    fn test_desktop_file_missing_name() {
        let content = "Exec=foo\n";
        assert!(DesktopFile::parse(content).is_none());
    }

    #[test]
    fn test_icon_hit_test() {
        let icon = DesktopIcon::new("Test", 100, 200);
        assert!(icon.hit_test(108, 210));
        assert!(!icon.hit_test(50, 50));
    }

    #[test]
    fn test_icon_grid_arrange() {
        let mut grid = IconGrid::new(800, 600);
        for i in 0..5 {
            grid.add_icon(DesktopIcon::new(&alloc::format!("Icon{}", i), 0, 0));
        }
        grid.arrange();
        // First icon should be near top-left
        assert!(grid.icons[0].x >= 20);
        assert!(grid.icons[0].y >= 40);
    }

    #[test]
    fn test_icon_grid_click() {
        let mut grid = IconGrid::new(800, 600);
        grid.add_icon(DesktopIcon::new("A", 100, 100));
        grid.add_icon(DesktopIcon::new("B", 200, 100));
        let clicked = grid.handle_click(108, 110);
        assert_eq!(clicked, Some(0));
        assert!(grid.icons[0].selected);
        assert!(!grid.icons[1].selected);
    }

    #[test]
    fn test_snap_to_grid() {
        let grid = IconGrid::new(800, 600);
        let (sx, sy) = grid.snap_to_grid(45, 95);
        assert_eq!(sx, 80);
        assert_eq!(sy, 80);
    }

    #[test]
    fn test_icon_drag() {
        let mut grid = IconGrid::new(800, 600);
        grid.add_icon(DesktopIcon::new("A", 0, 0));
        grid.icons[0].selected = true;
        grid.handle_drag(90, 90);
        // Should snap to grid
        assert_eq!(grid.icons[0].x, 80);
        assert_eq!(grid.icons[0].y, 80);
    }

    #[test]
    fn test_remove_icon() {
        let mut grid = IconGrid::new(800, 600);
        grid.add_icon(DesktopIcon::new("A", 0, 0));
        grid.add_icon(DesktopIcon::new("B", 80, 0));
        let removed = grid.remove_icon(0);
        assert!(removed.is_some());
        assert_eq!(grid.icon_count(), 1);
    }

    #[test]
    fn test_deselect_all() {
        let mut grid = IconGrid::new(800, 600);
        grid.add_icon(DesktopIcon::new("A", 0, 0));
        grid.icons[0].selected = true;
        grid.deselect_all();
        assert!(!grid.icons[0].selected);
    }

    #[test]
    fn test_render_icon_no_panic() {
        let icon = DesktopIcon::new("Test", 0, 0);
        let mut buf = vec![0u32; 64 * 64];
        IconGrid::render_icon(&icon, &mut buf, 64, 64);
        // Just verify it doesn't panic
    }
}

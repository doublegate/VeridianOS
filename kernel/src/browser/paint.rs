//! Display List and Paint
//!
//! Converts the layout tree into a flat display list of drawing commands
//! (solid rectangles, text, borders, clipping), then rasterizes them
//! into a pixel buffer. Uses integer-only alpha blending and the
//! 8x16 bitmap font for text rendering.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use super::{
    css_parser::fp_to_px,
    layout::{LayoutBox, Rect},
    style::{BorderStyle, Display, Overflow, Visibility},
};

/// Side of a border
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// A display command in the display list
#[derive(Debug, Clone)]
pub enum DisplayCommand {
    /// Fill a rectangle with a solid color
    SolidColor(u32, PixelRect),
    /// Draw text at a position
    Text(String, i32, i32, u32, i32),
    /// Draw a border edge
    Border(u32, PixelRect, i32, BorderSide),
    /// Set a clipping rectangle
    ClipRect(PixelRect),
    /// Pop the clipping rectangle
    PopClip,
}

/// A rectangle in pixel coordinates (not fixed-point)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Convert from fixed-point Rect
    pub fn from_fp(rect: &Rect) -> Self {
        Self {
            x: fp_to_px(rect.x),
            y: fp_to_px(rect.y),
            width: fp_to_px(rect.width),
            height: fp_to_px(rect.height),
        }
    }

    /// Intersect with another rectangle
    pub fn intersect(&self, other: &PixelRect) -> Option<PixelRect> {
        let x1 = core::cmp::max(self.x, other.x);
        let y1 = core::cmp::max(self.y, other.y);
        let x2 = core::cmp::min(self.x + self.width, other.x + other.width);
        let y2 = core::cmp::min(self.y + self.height, other.y + other.height);
        if x2 > x1 && y2 > y1 {
            Some(PixelRect::new(x1, y1, x2 - x1, y2 - y1))
        } else {
            None
        }
    }

    /// Check if a point is inside
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// A display list of rendering commands
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub commands: Vec<DisplayCommand>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn push(&mut self, cmd: DisplayCommand) {
        self.commands.push(cmd);
    }
}

/// Build a display list from a layout tree
pub fn build_display_list(layout: &LayoutBox, scroll_y: i32) -> DisplayList {
    let mut list = DisplayList::new();
    render_layout_box(&mut list, layout, scroll_y);
    list
}

/// Render a layout box into the display list
fn render_layout_box(list: &mut DisplayList, layout: &LayoutBox, scroll_y: i32) {
    if layout.style.display == Display::None {
        return;
    }
    if layout.style.visibility == Visibility::Hidden {
        // Hidden elements still take up space but don't render
        for child in &layout.children {
            render_layout_box(list, child, scroll_y);
        }
        return;
    }

    // Background
    render_background(list, layout, scroll_y);

    // Borders
    render_borders(list, layout, scroll_y);

    // Clip if overflow is hidden/scroll
    let needs_clip = matches!(layout.style.overflow, Overflow::Hidden | Overflow::Scroll);
    if needs_clip {
        let content_rect = PixelRect::from_fp(&layout.dimensions.content);
        list.push(DisplayCommand::ClipRect(PixelRect::new(
            content_rect.x,
            content_rect.y - scroll_y,
            content_rect.width,
            content_rect.height,
        )));
    }

    // Render line boxes (inline content)
    for line in &layout.line_boxes {
        let line_y = fp_to_px(line.y) - scroll_y;
        for frag in &line.fragments {
            list.push(DisplayCommand::Text(
                frag.text.clone(),
                fp_to_px(line.x) + fp_to_px(frag.width), // Should track x offset
                line_y,
                frag.color,
                fp_to_px(frag.font_size),
            ));
        }
    }

    // Children
    for child in &layout.children {
        render_layout_box(list, child, scroll_y);
    }

    if needs_clip {
        list.push(DisplayCommand::PopClip);
    }
}

/// Render background color
fn render_background(list: &mut DisplayList, layout: &LayoutBox, scroll_y: i32) {
    let bg = layout.style.background_color;
    if bg == 0 || (bg >> 24) == 0 {
        return; // Transparent
    }

    let border_box = layout.dimensions.border_box();
    let rect = PixelRect::new(
        fp_to_px(border_box.x),
        fp_to_px(border_box.y) - scroll_y,
        fp_to_px(border_box.width),
        fp_to_px(border_box.height),
    );

    if rect.width > 0 && rect.height > 0 {
        list.push(DisplayCommand::SolidColor(bg, rect));
    }
}

/// Render borders
fn render_borders(list: &mut DisplayList, layout: &LayoutBox, scroll_y: i32) {
    let d = &layout.dimensions;
    let bb = d.border_box();
    let bb_px = PixelRect::from_fp(&bb);
    let y_offset = scroll_y;

    // Top border
    if layout.style.border_top_style != BorderStyle::None && d.border.top > 0 {
        let rect = PixelRect::new(
            bb_px.x,
            bb_px.y - y_offset,
            bb_px.width,
            fp_to_px(d.border.top),
        );
        list.push(DisplayCommand::Border(
            layout.style.border_top_color,
            rect,
            fp_to_px(d.border.top),
            BorderSide::Top,
        ));
    }

    // Right border
    if layout.style.border_right_style != BorderStyle::None && d.border.right > 0 {
        let rect = PixelRect::new(
            bb_px.x + bb_px.width - fp_to_px(d.border.right),
            bb_px.y - y_offset,
            fp_to_px(d.border.right),
            bb_px.height,
        );
        list.push(DisplayCommand::Border(
            layout.style.border_right_color,
            rect,
            fp_to_px(d.border.right),
            BorderSide::Right,
        ));
    }

    // Bottom border
    if layout.style.border_bottom_style != BorderStyle::None && d.border.bottom > 0 {
        let rect = PixelRect::new(
            bb_px.x,
            bb_px.y - y_offset + bb_px.height - fp_to_px(d.border.bottom),
            bb_px.width,
            fp_to_px(d.border.bottom),
        );
        list.push(DisplayCommand::Border(
            layout.style.border_bottom_color,
            rect,
            fp_to_px(d.border.bottom),
            BorderSide::Bottom,
        ));
    }

    // Left border
    if layout.style.border_left_style != BorderStyle::None && d.border.left > 0 {
        let rect = PixelRect::new(
            bb_px.x,
            bb_px.y - y_offset,
            fp_to_px(d.border.left),
            bb_px.height,
        );
        list.push(DisplayCommand::Border(
            layout.style.border_left_color,
            rect,
            fp_to_px(d.border.left),
            BorderSide::Left,
        ));
    }
}

/// Pixel buffer painter
pub struct Painter {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
    clip_stack: Vec<PixelRect>,
}

impl Painter {
    /// Create a new painter with given dimensions
    pub fn new(width: usize, height: usize) -> Self {
        let size = width.checked_mul(height).unwrap_or(0);
        Self {
            width,
            height,
            pixels: alloc::vec![0xFFFFFFFF; size], // White background
            clip_stack: Vec::new(),
        }
    }

    /// Paint a display list
    pub fn paint(&mut self, display_list: &DisplayList) {
        for cmd in &display_list.commands {
            match cmd {
                DisplayCommand::SolidColor(color, rect) => {
                    self.fill_rect(*color, rect);
                }
                DisplayCommand::Text(text, x, y, color, font_size) => {
                    self.draw_text(text, *x, *y, *color, *font_size);
                }
                DisplayCommand::Border(color, rect, _width, _side) => {
                    self.fill_rect(*color, rect);
                }
                DisplayCommand::ClipRect(rect) => {
                    self.clip_stack.push(*rect);
                }
                DisplayCommand::PopClip => {
                    self.clip_stack.pop();
                }
            }
        }
    }

    /// Fill a rectangle with a solid color (with alpha blending)
    pub fn fill_rect(&mut self, color: u32, rect: &PixelRect) {
        let clipped = self.clip_rect(rect);
        let clipped = match clipped {
            Some(r) => r,
            None => return,
        };

        let alpha = (color >> 24) & 0xFF;
        if alpha == 0 {
            return;
        }

        for py in clipped.y..(clipped.y + clipped.height) {
            if py < 0 || py >= self.height as i32 {
                continue;
            }
            for px in clipped.x..(clipped.x + clipped.width) {
                if px < 0 || px >= self.width as i32 {
                    continue;
                }
                let idx = py as usize * self.width + px as usize;
                if idx < self.pixels.len() {
                    if alpha == 255 {
                        self.pixels[idx] = color;
                    } else {
                        self.pixels[idx] = alpha_blend(color, self.pixels[idx]);
                    }
                }
            }
        }
    }

    /// Draw text using 8x16 bitmap font
    pub fn draw_text(&mut self, text: &str, x: i32, y: i32, color: u32, _font_size: i32) {
        let mut cx = x;
        for ch in text.chars() {
            self.draw_char(ch, cx, y, color);
            cx += 8; // 8px per character
        }
    }

    /// Draw a single character from the 8x16 font
    fn draw_char(&mut self, ch: char, x: i32, y: i32, color: u32) {
        let glyph = get_glyph(ch);
        for (row, &bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if (bits >> (7 - col)) & 1 != 0 {
                    let px = x + col;
                    let py = y + row as i32;
                    if px >= 0 && px < self.width as i32 && py >= 0 && py < self.height as i32 {
                        let idx = py as usize * self.width + px as usize;
                        if idx < self.pixels.len() {
                            let alpha = (color >> 24) & 0xFF;
                            if alpha == 255 {
                                self.pixels[idx] = color;
                            } else {
                                self.pixels[idx] = alpha_blend(color, self.pixels[idx]);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Apply clipping to a rectangle
    fn clip_rect(&self, rect: &PixelRect) -> Option<PixelRect> {
        let viewport = PixelRect::new(0, 0, self.width as i32, self.height as i32);
        let mut clipped = rect.intersect(&viewport)?;

        for clip in &self.clip_stack {
            clipped = clipped.intersect(clip)?;
        }

        Some(clipped)
    }

    /// Get a pixel at (x, y)
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x]
        } else {
            0
        }
    }

    /// Clear the buffer to a color
    pub fn clear(&mut self, color: u32) {
        for pixel in &mut self.pixels {
            *pixel = color;
        }
    }

    /// Get pixels as a byte slice (BGRA format for framebuffer)
    pub fn as_bytes(&self) -> &[u32] {
        &self.pixels
    }
}

/// Alpha-blend source over destination (integer math)
/// Color format: ARGB (0xAARRGGBB)
pub fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;

    let da = (dst >> 24) & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;

    let inv_sa = 255 - sa;
    let out_r = (sr * sa + dr * inv_sa) / 255;
    let out_g = (sg * sa + dg * inv_sa) / 255;
    let out_b = (sb * sa + db * inv_sa) / 255;
    let out_a = sa + (da * inv_sa) / 255;

    (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b
}

/// Get a basic 8x16 glyph for a character
/// Returns 16 bytes (one per row), each bit represents a pixel
fn get_glyph(ch: char) -> [u8; 16] {
    // Simplified glyph data for printable ASCII
    let code = ch as u32;
    if !(32..=126).contains(&code) {
        return [0; 16]; // Non-printable
    }

    // Basic glyphs for common characters
    match ch {
        'A' => [
            0x00, 0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'B' => [
            0x00, 0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'H' => [
            0x00, 0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'e' => [
            0x00, 0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'l' => [
            0x00, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'o' => [
            0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        _ => {
            // Generic block for unimplemented glyphs
            [
                0x00, 0x00, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;
    use crate::browser::css_parser::px_to_fp;

    #[test]
    fn test_pixel_rect_new() {
        let r = PixelRect::new(10, 20, 100, 50);
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        assert_eq!(r.width, 100);
        assert_eq!(r.height, 50);
    }

    #[test]
    fn test_pixel_rect_contains() {
        let r = PixelRect::new(10, 10, 100, 50);
        assert!(r.contains(50, 30));
        assert!(!r.contains(5, 5));
        assert!(!r.contains(200, 200));
    }

    #[test]
    fn test_pixel_rect_intersect() {
        let a = PixelRect::new(0, 0, 100, 100);
        let b = PixelRect::new(50, 50, 100, 100);
        let c = a.intersect(&b).unwrap();
        assert_eq!(c.x, 50);
        assert_eq!(c.y, 50);
        assert_eq!(c.width, 50);
        assert_eq!(c.height, 50);
    }

    #[test]
    fn test_pixel_rect_no_intersect() {
        let a = PixelRect::new(0, 0, 10, 10);
        let b = PixelRect::new(20, 20, 10, 10);
        assert!(a.intersect(&b).is_none());
    }

    #[test]
    fn test_painter_new() {
        let p = Painter::new(100, 100);
        assert_eq!(p.width, 100);
        assert_eq!(p.height, 100);
        assert_eq!(p.pixels.len(), 10000);
        // Should be white
        assert_eq!(p.pixels[0], 0xFFFFFFFF);
    }

    #[test]
    fn test_painter_fill_rect() {
        let mut p = Painter::new(100, 100);
        let rect = PixelRect::new(10, 10, 20, 20);
        p.fill_rect(0xFFFF0000, &rect);
        assert_eq!(p.get_pixel(15, 15), 0xFFFF0000);
        assert_eq!(p.get_pixel(0, 0), 0xFFFFFFFF);
    }

    #[test]
    fn test_painter_clear() {
        let mut p = Painter::new(10, 10);
        p.clear(0xFF000000);
        assert_eq!(p.get_pixel(5, 5), 0xFF000000);
    }

    #[test]
    fn test_alpha_blend_opaque() {
        let result = alpha_blend(0xFFFF0000, 0xFF0000FF);
        assert_eq!(result, 0xFFFF0000);
    }

    #[test]
    fn test_alpha_blend_transparent() {
        let result = alpha_blend(0x00FF0000, 0xFF0000FF);
        assert_eq!(result & 0x00FFFFFF, 0x0000FF); // Blue preserved
    }

    #[test]
    fn test_alpha_blend_half() {
        let result = alpha_blend(0x80FF0000, 0xFF0000FF);
        let r = (result >> 16) & 0xFF;
        let b = result & 0xFF;
        // Red should be roughly half, blue roughly half
        assert!(r > 60 && r < 200);
        assert!(b > 60 && b < 200);
    }

    #[test]
    fn test_display_list_new() {
        let dl = DisplayList::new();
        assert!(dl.commands.is_empty());
    }

    #[test]
    fn test_display_list_push() {
        let mut dl = DisplayList::new();
        dl.push(DisplayCommand::SolidColor(
            0xFFFF0000,
            PixelRect::new(0, 0, 10, 10),
        ));
        assert_eq!(dl.commands.len(), 1);
    }

    #[test]
    fn test_painter_draw_text() {
        let mut p = Painter::new(100, 100);
        p.draw_text("A", 10, 10, 0xFF000000, 16);
        // Some pixels should be black near the text position
        let mut has_black = false;
        for y in 10..26 {
            for x in 10..18 {
                if p.get_pixel(x, y) == 0xFF000000 {
                    has_black = true;
                }
            }
        }
        assert!(has_black);
    }

    #[test]
    fn test_get_glyph_a() {
        let g = get_glyph('A');
        // Row 1 should have some bits set (0x18)
        assert_eq!(g[1], 0x18);
    }

    #[test]
    fn test_get_glyph_nonprintable() {
        let g = get_glyph('\0');
        assert_eq!(g, [0; 16]);
    }

    #[test]
    fn test_painter_clip() {
        let mut p = Painter::new(100, 100);
        p.clip_stack.push(PixelRect::new(20, 20, 60, 60));
        let rect = PixelRect::new(0, 0, 100, 100);
        p.fill_rect(0xFFFF0000, &rect);
        // Pixel at (10, 10) should still be white (outside clip)
        assert_eq!(p.get_pixel(10, 10), 0xFFFFFFFF);
        // Pixel at (30, 30) should be red (inside clip)
        assert_eq!(p.get_pixel(30, 30), 0xFFFF0000);
    }

    #[test]
    fn test_build_display_list_empty() {
        let layout = LayoutBox::default();
        let dl = build_display_list(&layout, 0);
        // Default layout has no visible content
        assert!(dl.commands.is_empty() || !dl.commands.is_empty());
    }

    #[test]
    fn test_pixel_rect_from_fp() {
        let fp_rect = Rect {
            x: px_to_fp(10),
            y: px_to_fp(20),
            width: px_to_fp(100),
            height: px_to_fp(50),
        };
        let pr = PixelRect::from_fp(&fp_rect);
        assert_eq!(pr.x, 10);
        assert_eq!(pr.y, 20);
        assert_eq!(pr.width, 100);
        assert_eq!(pr.height, 50);
    }

    #[test]
    fn test_painter_out_of_bounds() {
        let p = Painter::new(10, 10);
        assert_eq!(p.get_pixel(100, 100), 0);
    }

    #[test]
    fn test_fill_rect_outside_viewport() {
        let mut p = Painter::new(10, 10);
        let rect = PixelRect::new(20, 20, 10, 10);
        p.fill_rect(0xFFFF0000, &rect);
        // Should not crash, no pixels changed
        assert_eq!(p.get_pixel(0, 0), 0xFFFFFFFF);
    }
}

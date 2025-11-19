//! Font Rendering System for Desktop Applications
//!
//! Provides bitmap font rendering with support for multiple font sizes and
//! styles.

use alloc::{vec, vec::Vec};

use crate::error::KernelError;

/// Font size in pixels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontSize {
    Small = 8,
    Medium = 12,
    Large = 16,
    ExtraLarge = 24,
}

/// Font style
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

/// Font weight
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    Thin = 100,
    Light = 300,
    Regular = 400,
    Medium = 500,
    Bold = 700,
    Black = 900,
}

/// Glyph (single character) representation
#[derive(Debug, Clone)]
pub struct Glyph {
    /// Character code
    pub character: char,

    /// Bitmap data (1 bit per pixel)
    pub bitmap: Vec<u8>,

    /// Width in pixels
    pub width: u8,

    /// Height in pixels
    pub height: u8,

    /// X offset for rendering
    pub x_offset: i8,

    /// Y offset for rendering
    pub y_offset: i8,

    /// Advance width (spacing to next character)
    pub advance: u8,
}

impl Glyph {
    /// Create a new glyph
    pub fn new(character: char, width: u8, height: u8) -> Self {
        let bitmap_size = ((width as usize * height as usize) + 7) / 8;
        Self {
            character,
            bitmap: vec![0; bitmap_size],
            width,
            height,
            x_offset: 0,
            y_offset: 0,
            advance: width,
        }
    }

    /// Get pixel value at (x, y)
    pub fn get_pixel(&self, x: u8, y: u8) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }

        let bit_index = y as usize * self.width as usize + x as usize;
        let byte_index = bit_index / 8;
        let bit_offset = bit_index % 8;

        if byte_index >= self.bitmap.len() {
            return false;
        }

        (self.bitmap[byte_index] & (1 << bit_offset)) != 0
    }

    /// Set pixel value at (x, y)
    pub fn set_pixel(&mut self, x: u8, y: u8, value: bool) {
        if x >= self.width || y >= self.height {
            return;
        }

        let bit_index = y as usize * self.width as usize + x as usize;
        let byte_index = bit_index / 8;
        let bit_offset = bit_index % 8;

        if byte_index >= self.bitmap.len() {
            return;
        }

        if value {
            self.bitmap[byte_index] |= 1 << bit_offset;
        } else {
            self.bitmap[byte_index] &= !(1 << bit_offset);
        }
    }
}

/// Bitmap font
pub struct Font {
    /// Font name
    pub name: &'static str,

    /// Font size
    pub size: FontSize,

    /// Font style
    pub style: FontStyle,

    /// Glyphs for ASCII characters (32-126)
    glyphs: Vec<Glyph>,

    /// Line height
    pub line_height: u8,

    /// Baseline offset
    pub baseline: u8,
}

impl Font {
    /// Create a new font
    pub fn new(name: &'static str, size: FontSize, style: FontStyle) -> Self {
        let height = size as u8;
        let line_height = height + 2;
        let baseline = height - 2;

        let mut font = Self {
            name,
            size,
            style,
            glyphs: Vec::new(),
            line_height,
            baseline,
        };

        // Initialize ASCII glyphs
        font.initialize_ascii_glyphs();

        font
    }

    /// Initialize basic ASCII glyphs (stub implementation)
    fn initialize_ascii_glyphs(&mut self) {
        // ASCII printable characters: 32-126
        for ch in 32u8..=126 {
            let character = ch as char;
            let width = (self.size as u8 * 2) / 3; // Approximate width
            let glyph = Glyph::new(character, width, self.size as u8);
            self.glyphs.push(glyph);
        }
    }

    /// Get glyph for a character
    pub fn get_glyph(&self, ch: char) -> Option<&Glyph> {
        let code = ch as u32;
        if code >= 32 && code <= 126 {
            Some(&self.glyphs[(code - 32) as usize])
        } else {
            None
        }
    }

    /// Measure text width
    pub fn measure_text(&self, text: &str) -> u32 {
        let mut width = 0u32;
        for ch in text.chars() {
            if let Some(glyph) = self.get_glyph(ch) {
                width += glyph.advance as u32;
            }
        }
        width
    }

    /// Render text to a bitmap buffer
    pub fn render_text(
        &self,
        text: &str,
        buffer: &mut [u8],
        buffer_width: usize,
        buffer_height: usize,
        x: i32,
        y: i32,
    ) -> Result<(), KernelError> {
        let mut cursor_x = x;
        let cursor_y = y;

        for ch in text.chars() {
            if let Some(glyph) = self.get_glyph(ch) {
                // Render glyph
                for gy in 0..glyph.height {
                    for gx in 0..glyph.width {
                        if glyph.get_pixel(gx, gy) {
                            let px = cursor_x + gx as i32 + glyph.x_offset as i32;
                            let py = cursor_y + gy as i32 + glyph.y_offset as i32;

                            if px >= 0
                                && py >= 0
                                && (px as usize) < buffer_width
                                && (py as usize) < buffer_height
                            {
                                let pixel_index = py as usize * buffer_width + px as usize;
                                if pixel_index < buffer.len() {
                                    buffer[pixel_index] = 255; // White pixel
                                }
                            }
                        }
                    }
                }

                cursor_x += glyph.advance as i32;
            }
        }

        Ok(())
    }
}

/// Font manager for loading and caching fonts
pub struct FontManager {
    /// Loaded fonts
    fonts: Vec<Font>,

    /// Default font index
    default_font: usize,
}

impl FontManager {
    /// Create a new font manager
    pub fn new() -> Self {
        let mut manager = Self {
            fonts: Vec::new(),
            default_font: 0,
        };

        // Load default fonts
        manager.load_default_fonts();

        manager
    }

    /// Load default fonts
    fn load_default_fonts(&mut self) {
        // Create default font set
        self.fonts
            .push(Font::new("Monospace", FontSize::Small, FontStyle::Regular));
        self.fonts
            .push(Font::new("Monospace", FontSize::Medium, FontStyle::Regular));
        self.fonts
            .push(Font::new("Monospace", FontSize::Large, FontStyle::Regular));
        self.fonts
            .push(Font::new("Monospace", FontSize::Medium, FontStyle::Bold));

        println!("[FONT] Loaded {} default fonts", self.fonts.len());
    }

    /// Get default font
    pub fn get_default_font(&self) -> &Font {
        &self.fonts[self.default_font]
    }

    /// Get font by size and style
    pub fn get_font(&self, size: FontSize, style: FontStyle) -> Option<&Font> {
        self.fonts
            .iter()
            .find(|f| f.size == size && f.style == style)
    }

    /// Add a font
    pub fn add_font(&mut self, font: Font) {
        self.fonts.push(font);
    }
}

impl Default for FontManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global font manager
static mut FONT_MANAGER: Option<FontManager> = None;

/// Initialize the font system
pub fn init() -> Result<(), KernelError> {
    unsafe {
        if FONT_MANAGER.is_some() {
            return Err(KernelError::InvalidState {
                expected: "uninitialized",
                actual: "initialized",
            });
        }

        let manager = FontManager::new();
        FONT_MANAGER = Some(manager);

        println!("[FONT] Font rendering system initialized");
        Ok(())
    }
}

/// Get the global font manager
pub fn get_font_manager() -> Result<&'static mut FontManager, KernelError> {
    unsafe {
        FONT_MANAGER.as_mut().ok_or(KernelError::InvalidState {
            expected: "initialized",
            actual: "uninitialized",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_font_creation() {
        let font = Font::new("Test", FontSize::Medium, FontStyle::Regular);
        assert_eq!(font.name, "Test");
        assert_eq!(font.size, FontSize::Medium);
    }

    #[test_case]
    fn test_glyph_pixel_operations() {
        let mut glyph = Glyph::new('A', 8, 12);
        assert!(!glyph.get_pixel(0, 0));

        glyph.set_pixel(0, 0, true);
        assert!(glyph.get_pixel(0, 0));

        glyph.set_pixel(0, 0, false);
        assert!(!glyph.get_pixel(0, 0));
    }

    #[test_case]
    fn test_text_measurement() {
        let font = Font::new("Test", FontSize::Medium, FontStyle::Regular);
        let width = font.measure_text("Hello");
        assert!(width > 0);
    }
}

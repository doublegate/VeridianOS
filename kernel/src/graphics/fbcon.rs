//! Framebuffer console (fbcon) — text rendering onto a pixel framebuffer.
//!
//! Renders characters using the 8x16 bitmap font onto the UEFI-provided
//! (or ramfb-provided) pixel framebuffer. Supports cursor tracking, newlines,
//! tab stops, backspace, scrolling, and basic ANSI color escape sequences.
//!
//! Thread-safety: The global `FBCON` is protected by a spinlock. Interrupt
//! handlers must NOT call `_fbcon_print` (use raw serial output for
//! diagnostics in ISRs).

use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use spin::Mutex;

use super::font8x16::{self, FONT_HEIGHT, FONT_WIDTH};

/// Controls whether `_fbcon_print` actually renders to the framebuffer.
/// Starts `false` — boot messages go to serial only (too many lines to
/// render at 1280x800 in QEMU's emulated CPU). Set to `true` via
/// `enable_output()` just before the shell launches.
static FBCON_OUTPUT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Pixel format of the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbPixelFormat {
    /// Blue-Green-Red-Reserved (UEFI default with OVMF)
    Bgr,
    /// Red-Green-Blue-Reserved
    Rgb,
}

/// ANSI escape sequence parser state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscapeState {
    /// Normal text rendering
    Normal,
    /// Saw ESC (0x1B), waiting for '['
    Escape,
    /// Inside CSI sequence, accumulating parameters
    Csi,
}

/// Framebuffer console state.
pub struct FramebufferConsole {
    fb_ptr: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
    pixel_format: FbPixelFormat,
    cols: usize,
    rows: usize,
    cursor_col: usize,
    cursor_row: usize,
    fg_color: (u8, u8, u8),
    bg_color: (u8, u8, u8),
    default_fg: (u8, u8, u8),
    default_bg: (u8, u8, u8),
    esc_state: EscapeState,
    esc_params: [u8; 16],
    esc_param_idx: usize,
}

// SAFETY: FramebufferConsole is only accessed through the FBCON spinlock.
// The raw pointer fb_ptr points to MMIO memory that is valid for the
// kernel's lifetime (mapped by the bootloader or ramfb init).
unsafe impl Send for FramebufferConsole {}

/// Default foreground: light gray
const DEFAULT_FG: (u8, u8, u8) = (0xAA, 0xAA, 0xAA);
/// Default background: black
const DEFAULT_BG: (u8, u8, u8) = (0x00, 0x00, 0x00);

/// ANSI standard colors (SGR 30-37 foreground, 40-47 background)
const ANSI_COLORS: [(u8, u8, u8); 8] = [
    (0x00, 0x00, 0x00), // 0: black
    (0xAA, 0x00, 0x00), // 1: red
    (0x00, 0xAA, 0x00), // 2: green
    (0xAA, 0x55, 0x00), // 3: yellow/brown
    (0x00, 0x00, 0xAA), // 4: blue
    (0xAA, 0x00, 0xAA), // 5: magenta
    (0x00, 0xAA, 0xAA), // 6: cyan
    (0xAA, 0xAA, 0xAA), // 7: white/light gray
];

static FBCON: Mutex<Option<FramebufferConsole>> = Mutex::new(None);

impl FramebufferConsole {
    /// Create a new framebuffer console.
    fn new(
        fb_ptr: *mut u8,
        width: usize,
        height: usize,
        stride: usize,
        bpp: usize,
        pixel_format: FbPixelFormat,
    ) -> Self {
        Self {
            fb_ptr,
            width,
            height,
            stride,
            bpp,
            pixel_format,
            cols: width / FONT_WIDTH,
            rows: height / FONT_HEIGHT,
            cursor_col: 0,
            cursor_row: 0,
            fg_color: DEFAULT_FG,
            bg_color: DEFAULT_BG,
            default_fg: DEFAULT_FG,
            default_bg: DEFAULT_BG,
            esc_state: EscapeState::Normal,
            esc_params: [0; 16],
            esc_param_idx: 0,
        }
    }

    /// Write a single pixel at (x, y) with the given RGB color.
    ///
    /// Uses a single 32-bit write instead of four byte writes for performance.
    /// The framebuffer is regular RAM (not MMIO registers), so volatile writes
    /// are not necessary for correctness.
    #[inline(always)]
    fn write_pixel(&self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = y * self.stride + x * self.bpp;
        let word = match self.pixel_format {
            FbPixelFormat::Bgr => u32::from_ne_bytes([b, g, r, 0]),
            FbPixelFormat::Rgb => u32::from_ne_bytes([r, g, b, 0]),
        };
        // SAFETY: The framebuffer pointer and dimensions come from the
        // bootloader (UEFI) or ramfb init. The offset is bounds-checked
        // above. The pixel address is 4-byte aligned (stride and bpp are
        // both multiples of 4).
        unsafe {
            (self.fb_ptr.add(offset) as *mut u32).write(word);
        }
    }

    /// Render a glyph at pixel position (px, py).
    fn render_glyph(&self, ch: u8, px: usize, py: usize) {
        let glyph_data = font8x16::glyph(ch);
        let (fg_r, fg_g, fg_b) = self.fg_color;
        let (bg_r, bg_g, bg_b) = self.bg_color;

        for (row, &bits) in glyph_data.iter().enumerate() {
            for col in 0..FONT_WIDTH {
                let is_set = (bits >> (7 - col)) & 1 != 0;
                if is_set {
                    self.write_pixel(px + col, py + row, fg_r, fg_g, fg_b);
                } else {
                    self.write_pixel(px + col, py + row, bg_r, bg_g, bg_b);
                }
            }
        }
    }

    /// Scroll the framebuffer up by one text row (FONT_HEIGHT pixels).
    fn scroll_up(&self) {
        let row_bytes = self.stride * FONT_HEIGHT;
        let total_bytes = self.stride * self.height;

        // SAFETY: Copying within the framebuffer to shift content up by one
        // text row. The source and destination regions overlap, but copy()
        // handles overlapping regions correctly (memmove semantics).
        unsafe {
            core::ptr::copy(
                self.fb_ptr.add(row_bytes),
                self.fb_ptr,
                total_bytes - row_bytes,
            );
        }

        // Clear the last row. When bg is black (0,0,0), use write_bytes
        // (memset) for maximum speed. Otherwise fall back to per-pixel writes.
        let last_row_start = self.stride * (self.height - FONT_HEIGHT);
        let clear_bytes = self.stride * FONT_HEIGHT;
        if self.bg_color == (0, 0, 0) {
            // SAFETY: Zeroing the last row of the framebuffer.
            unsafe {
                core::ptr::write_bytes(self.fb_ptr.add(last_row_start), 0, clear_bytes);
            }
        } else {
            let (bg_r, bg_g, bg_b) = self.bg_color;
            let word = match self.pixel_format {
                FbPixelFormat::Bgr => u32::from_ne_bytes([bg_b, bg_g, bg_r, 0]),
                FbPixelFormat::Rgb => u32::from_ne_bytes([bg_r, bg_g, bg_b, 0]),
            };
            // SAFETY: Writing within the last row of the framebuffer.
            unsafe {
                let row_ptr = self.fb_ptr.add(last_row_start) as *mut u32;
                let pixel_count = self.width * FONT_HEIGHT;
                for i in 0..pixel_count {
                    row_ptr.add(i).write(word);
                }
            }
        }
    }

    /// Write a character at the current cursor position and advance.
    fn write_char(&mut self, ch: u8) {
        match self.esc_state {
            EscapeState::Normal => self.write_char_normal(ch),
            EscapeState::Escape => self.write_char_escape(ch),
            EscapeState::Csi => self.write_char_csi(ch),
        }
    }

    /// Handle a character in normal (non-escape) mode.
    fn write_char_normal(&mut self, ch: u8) {
        match ch {
            b'\n' => {
                self.cursor_col = 0;
                self.cursor_row += 1;
                if self.cursor_row >= self.rows {
                    self.scroll_up();
                    self.cursor_row = self.rows - 1;
                }
            }
            b'\r' => {
                self.cursor_col = 0;
            }
            b'\t' => {
                let next_tab = (self.cursor_col + 8) & !7;
                self.cursor_col = if next_tab < self.cols {
                    next_tab
                } else {
                    self.cols - 1
                };
            }
            0x08 => {
                // Backspace
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                    let px = self.cursor_col * FONT_WIDTH;
                    let py = self.cursor_row * FONT_HEIGHT;
                    self.render_glyph(b' ', px, py);
                }
            }
            0x1B => {
                // ESC — start escape sequence
                self.esc_state = EscapeState::Escape;
                self.esc_param_idx = 0;
                self.esc_params = [0; 16];
            }
            _ => {
                // Printable character
                let px = self.cursor_col * FONT_WIDTH;
                let py = self.cursor_row * FONT_HEIGHT;
                self.render_glyph(ch, px, py);
                self.cursor_col += 1;
                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                    if self.cursor_row >= self.rows {
                        self.scroll_up();
                        self.cursor_row = self.rows - 1;
                    }
                }
            }
        }
    }

    /// Handle a character after ESC was received.
    fn write_char_escape(&mut self, ch: u8) {
        if ch == b'[' {
            self.esc_state = EscapeState::Csi;
        } else {
            // Unknown escape sequence — discard and return to normal
            self.esc_state = EscapeState::Normal;
        }
    }

    /// Handle a character inside a CSI sequence (ESC [ ...).
    fn write_char_csi(&mut self, ch: u8) {
        match ch {
            b'0'..=b'9' => {
                // Accumulate parameter digit
                if self.esc_param_idx < self.esc_params.len() {
                    self.esc_params[self.esc_param_idx] = self.esc_params[self.esc_param_idx]
                        .wrapping_mul(10)
                        .wrapping_add(ch - b'0');
                }
            }
            b';' => {
                // Parameter separator
                if self.esc_param_idx < self.esc_params.len() - 1 {
                    self.esc_param_idx += 1;
                }
            }
            b'm' => {
                // SGR (Select Graphic Rendition)
                self.handle_sgr();
                self.esc_state = EscapeState::Normal;
            }
            b'J' => {
                // Erase in Display
                let param = self.esc_params[0];
                if param == 2 {
                    self.clear();
                }
                self.esc_state = EscapeState::Normal;
            }
            b'H' => {
                // Cursor Position
                let row = if self.esc_params[0] > 0 {
                    (self.esc_params[0] - 1) as usize
                } else {
                    0
                };
                let col = if self.esc_param_idx >= 1 && self.esc_params[1] > 0 {
                    (self.esc_params[1] - 1) as usize
                } else {
                    0
                };
                self.cursor_row = if row < self.rows { row } else { self.rows - 1 };
                self.cursor_col = if col < self.cols { col } else { self.cols - 1 };
                self.esc_state = EscapeState::Normal;
            }
            b'A' => {
                // Cursor Up
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_row = self.cursor_row.saturating_sub(n);
                self.esc_state = EscapeState::Normal;
            }
            b'B' => {
                // Cursor Down
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_row = core::cmp::min(self.cursor_row + n, self.rows - 1);
                self.esc_state = EscapeState::Normal;
            }
            b'C' => {
                // Cursor Forward
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_col = core::cmp::min(self.cursor_col + n, self.cols - 1);
                self.esc_state = EscapeState::Normal;
            }
            b'D' => {
                // Cursor Back
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_col = self.cursor_col.saturating_sub(n);
                self.esc_state = EscapeState::Normal;
            }
            b'K' => {
                // Erase in Line (param 0 = cursor to end, 1 = start to cursor, 2 = whole line)
                let param = self.esc_params[0];
                let py = self.cursor_row * FONT_HEIGHT;
                let (start_col, end_col) = match param {
                    1 => (0, self.cursor_col),
                    2 => (0, self.cols),
                    _ => (self.cursor_col, self.cols), // 0 or default
                };
                for col in start_col..end_col {
                    self.render_glyph(b' ', col * FONT_WIDTH, py);
                }
                self.esc_state = EscapeState::Normal;
            }
            _ => {
                // Unknown CSI command — discard sequence
                self.esc_state = EscapeState::Normal;
            }
        }
    }

    /// Handle SGR (Select Graphic Rendition) escape codes.
    fn handle_sgr(&mut self) {
        let param_count = self.esc_param_idx + 1;
        for i in 0..param_count {
            let code = self.esc_params[i];
            match code {
                0 => {
                    // Reset
                    self.fg_color = self.default_fg;
                    self.bg_color = self.default_bg;
                }
                1 => {
                    // Bold — use bright variants (add 0x55 to each channel, cap at 0xFF)
                    self.fg_color = (
                        self.fg_color.0.saturating_add(0x55),
                        self.fg_color.1.saturating_add(0x55),
                        self.fg_color.2.saturating_add(0x55),
                    );
                }
                30..=37 => {
                    self.fg_color = ANSI_COLORS[(code - 30) as usize];
                }
                40..=47 => {
                    self.bg_color = ANSI_COLORS[(code - 40) as usize];
                }
                _ => {} // Ignore unsupported SGR codes
            }
        }
    }

    /// Clear the entire framebuffer.
    fn clear(&mut self) {
        let total_bytes = self.stride * self.height;
        if self.bg_color == (0, 0, 0) {
            // SAFETY: Zeroing the entire framebuffer.
            unsafe {
                core::ptr::write_bytes(self.fb_ptr, 0, total_bytes);
            }
        } else {
            let (bg_r, bg_g, bg_b) = self.bg_color;
            let word = match self.pixel_format {
                FbPixelFormat::Bgr => u32::from_ne_bytes([bg_b, bg_g, bg_r, 0]),
                FbPixelFormat::Rgb => u32::from_ne_bytes([bg_r, bg_g, bg_b, 0]),
            };
            // SAFETY: Writing within the framebuffer bounds.
            unsafe {
                let ptr = self.fb_ptr as *mut u32;
                let pixel_count = self.width * self.height;
                for i in 0..pixel_count {
                    ptr.add(i).write(word);
                }
            }
        }
        self.cursor_col = 0;
        self.cursor_row = 0;
    }
}

impl fmt::Write for FramebufferConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.write_char(byte);
        }
        Ok(())
    }
}

/// Initialize the framebuffer console with the given parameters.
///
/// Must be called after the framebuffer is available (after UEFI boot
/// on x86_64, or after ramfb init on AArch64/RISC-V).
pub fn init(
    fb_ptr: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
    format: FbPixelFormat,
) {
    let mut fbcon = FramebufferConsole::new(fb_ptr, width, height, stride, bpp, format);
    fbcon.clear();
    *FBCON.lock() = Some(fbcon);
}

/// Enable fbcon output. Called after boot completes (just before the
/// shell launches) so that the hundreds of boot log lines don't get
/// rendered pixel-by-pixel to the framebuffer in QEMU.
pub fn enable_output() {
    FBCON_OUTPUT_ENABLED.store(true, Ordering::Release);
}

/// Print formatted text to the framebuffer console.
///
/// Silently returns if fbcon has not been initialized yet or if output
/// has not been enabled (boot messages go to serial only for performance).
pub fn _fbcon_print(args: fmt::Arguments) {
    if !FBCON_OUTPUT_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    use fmt::Write;
    let mut guard = FBCON.lock();
    if let Some(ref mut fbcon) = *guard {
        let _ = fbcon.write_fmt(args);
    }
}

/// Check if the framebuffer console has been initialized.
pub fn is_initialized() -> bool {
    FBCON.lock().is_some()
}

//! Framebuffer console (fbcon) — text rendering onto a pixel framebuffer.
//!
//! Renders characters using the 8x16 bitmap font onto the UEFI-provided
//! (or ramfb-provided) pixel framebuffer. Supports cursor tracking, newlines,
//! tab stops, backspace, scrolling, and basic ANSI color escape sequences.
//!
//! # Performance architecture
//!
//! All rendering goes through a three-layer pipeline to minimize slow MMIO
//! writes to QEMU-intercepted framebuffer memory:
//!
//! 1. **Text cell grid** — character/color pairs in a ring buffer. Scrolling is
//!    O(cols) (advance ring pointer + clear one row of cells).
//! 2. **RAM back-buffer** — full pixel buffer in kernel heap (~4MB). Glyph
//!    rendering writes here (fast regular-memory writes).
//! 3. **Hardware framebuffer** — MMIO memory. Touched once per `_fbcon_print`
//!    call, only for rows that changed (dirty row tracking).
//!
//! Thread-safety: The global `FBCON` is protected by a spinlock. Interrupt
//! handlers must NOT call `_fbcon_print` (use raw serial output for
//! diagnostics in ISRs).

use alloc::{vec, vec::Vec};
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

/// Maximum text rows supported by the console.
const MAX_ROWS: usize = 64;
/// Maximum text columns supported by the console.
const MAX_COLS: usize = 192;

/// A single character cell in the text grid.
#[derive(Clone, Copy)]
struct TextCell {
    ch: u8,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
}

impl TextCell {
    const fn blank(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> Self {
        Self { ch: b' ', fg, bg }
    }
}

/// Framebuffer console state.
pub struct FramebufferConsole {
    // Hardware framebuffer (MMIO, slow in QEMU)
    fb_ptr: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
    pixel_format: FbPixelFormat,

    // Text grid dimensions
    cols: usize,
    rows: usize,

    // Cursor
    cursor_col: usize,
    cursor_row: usize,

    // Colors
    fg_color: (u8, u8, u8),
    bg_color: (u8, u8, u8),
    default_fg: (u8, u8, u8),
    default_bg: (u8, u8, u8),

    // ANSI escape parser
    esc_state: EscapeState,
    esc_params: [u8; 16],
    esc_param_idx: usize,

    // Text cell ring buffer (Phase 2)
    cells: Vec<TextCell>,
    ring_start: usize,

    // RAM back-buffer (Phase 1) — `None` if allocation failed (fallback to direct MMIO)
    back_buf: Option<Vec<u8>>,

    // Dirty tracking (Phase 1+2)
    dirty_rows: [bool; MAX_ROWS],
    dirty_all: bool,
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
        let cols = (width / FONT_WIDTH).min(MAX_COLS);
        let rows = (height / FONT_HEIGHT).min(MAX_ROWS);

        // Allocate text cell grid
        let cell_count = MAX_ROWS * MAX_COLS;
        let cells = vec![TextCell::blank(DEFAULT_FG, DEFAULT_BG); cell_count];

        // Allocate RAM back-buffer. If heap is exhausted, fall back to
        // direct MMIO rendering (same as old behavior).
        let buf_size = stride * height;
        let back_buf = try_alloc_vec(buf_size).ok();

        Self {
            fb_ptr,
            width,
            height,
            stride,
            bpp,
            pixel_format,
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            fg_color: DEFAULT_FG,
            bg_color: DEFAULT_BG,
            default_fg: DEFAULT_FG,
            default_bg: DEFAULT_BG,
            esc_state: EscapeState::Normal,
            esc_params: [0; 16],
            esc_param_idx: 0,
            cells,
            ring_start: 0,
            back_buf,
            dirty_rows: [false; MAX_ROWS],
            dirty_all: true, // Force initial full blit
        }
    }

    /// Zero the hardware framebuffer and back-buffer on initial setup.
    /// This clears any UEFI boot garbage from the screen.
    fn clear_hw_and_backbuf(&mut self) {
        let total_bytes = self.stride * self.height;
        // SAFETY: fb_ptr is valid for total_bytes (caller guarantees).
        unsafe {
            core::ptr::write_bytes(self.fb_ptr, 0, total_bytes);
        }
        if let Some(ref mut buf) = self.back_buf {
            // SAFETY: buf is exactly total_bytes in size.
            unsafe {
                core::ptr::write_bytes(buf.as_mut_ptr(), 0, buf.len());
            }
        }
        self.dirty_all = false; // Screen is already clear
    }

    /// Map a logical text row (0 = top of screen) to the physical ring index.
    #[inline(always)]
    fn phys_row(&self, logical_row: usize) -> usize {
        (self.ring_start + logical_row) % MAX_ROWS
    }

    /// Get a reference to a text cell.
    #[inline(always)]
    fn cell(&self, logical_row: usize, col: usize) -> &TextCell {
        let idx = self.phys_row(logical_row) * MAX_COLS + col;
        &self.cells[idx]
    }

    /// Compute the flat index for a text cell.
    #[inline(always)]
    fn cell_idx(&self, logical_row: usize, col: usize) -> usize {
        self.phys_row(logical_row) * MAX_COLS + col
    }

    /// Mark a logical text row as dirty (needs re-rendering on next blit).
    #[inline(always)]
    fn mark_dirty(&mut self, logical_row: usize) {
        if logical_row < MAX_ROWS {
            self.dirty_rows[logical_row] = true;
        }
    }

    /// Convert an RGB triple to a u32 pixel word in the framebuffer's format.
    #[inline(always)]
    fn color_to_word(&self, r: u8, g: u8, b: u8) -> u32 {
        match self.pixel_format {
            FbPixelFormat::Bgr => u32::from_ne_bytes([b, g, r, 0]),
            FbPixelFormat::Rgb => u32::from_ne_bytes([r, g, b, 0]),
        }
    }

    /// Render a single glyph to the back-buffer (or directly to HW FB if no
    /// back-buffer is available). Writes 8x16 = 128 pixels.
    ///
    /// Phase 3 optimization: computes row base pointer once, writes u32
    /// directly without per-pixel bounds checks (caller ensures in-bounds).
    fn render_glyph_to_buf(
        &mut self,
        ch: u8,
        px: usize,
        py: usize,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) {
        let fg_word = self.color_to_word(fg.0, fg.1, fg.2);
        let bg_word = self.color_to_word(bg.0, bg.1, bg.2);
        let glyph_data = font8x16::glyph(ch);
        let stride = self.stride;
        let bpp = self.bpp;

        let buf_ptr = match self.back_buf {
            Some(ref mut buf) => buf.as_mut_ptr(),
            None => self.fb_ptr,
        };

        for (row, &bits) in glyph_data.iter().enumerate() {
            let y = py + row;
            if y >= self.height {
                break;
            }
            let base = y * stride + px * bpp;
            // SAFETY: px is < cols * FONT_WIDTH <= width, y is < height,
            // and the buffer is stride * height bytes. Each row writes
            // 8 * 4 = 32 bytes starting at `base`, which is within bounds.
            unsafe {
                let ptr = buf_ptr.add(base) as *mut u32;
                for col in 0..FONT_WIDTH {
                    let word = if (bits >> (7 - col)) & 1 != 0 {
                        fg_word
                    } else {
                        bg_word
                    };
                    ptr.add(col).write(word);
                }
            }
        }
    }

    /// Render one logical text row from the cell grid to the back-buffer.
    ///
    /// Phase 3 optimization: detects blank rows (all spaces with default bg)
    /// and uses memset instead of rendering individual glyphs.
    fn render_row_to_backbuf(&mut self, logical_row: usize) {
        let py = logical_row * FONT_HEIGHT;

        // Check if the row is all blank cells with black bg (common case for
        // empty rows after scroll).
        let mut all_blank_black = true;
        for col in 0..self.cols {
            let c = self.cell(logical_row, col);
            if c.ch != b' ' || c.bg != (0, 0, 0) {
                all_blank_black = false;
                break;
            }
        }

        if all_blank_black {
            // Fast path: zero the pixel region with memset
            let row_start = py * self.stride;
            let row_bytes = FONT_HEIGHT * self.stride;
            let buf_ptr = match self.back_buf {
                Some(ref mut buf) => buf.as_mut_ptr(),
                None => self.fb_ptr,
            };
            // SAFETY: row_start + row_bytes <= stride * height = buffer size.
            unsafe {
                core::ptr::write_bytes(buf_ptr.add(row_start), 0, row_bytes);
            }
            return;
        }

        // Check if the row is all blank cells with uniform non-black bg
        let first_bg = self.cell(logical_row, 0).bg;
        let mut all_blank_uniform = true;
        for col in 0..self.cols {
            let c = self.cell(logical_row, col);
            if c.ch != b' ' || c.bg != first_bg {
                all_blank_uniform = false;
                break;
            }
        }

        if all_blank_uniform {
            // Fill with uniform bg color
            let bg_word = self.color_to_word(first_bg.0, first_bg.1, first_bg.2);
            let buf_ptr = match self.back_buf {
                Some(ref mut buf) => buf.as_mut_ptr(),
                None => self.fb_ptr,
            };
            let stride = self.stride;
            for row in 0..FONT_HEIGHT {
                let y = py + row;
                if y >= self.height {
                    break;
                }
                // SAFETY: writing within buffer bounds (checked by y < height).
                unsafe {
                    let base = y * stride;
                    let ptr = buf_ptr.add(base) as *mut u32;
                    for x in 0..self.width {
                        ptr.add(x).write(bg_word);
                    }
                }
            }
            return;
        }

        // General case: render each cell's glyph
        for col in 0..self.cols {
            let c = *self.cell(logical_row, col);
            self.render_glyph_to_buf(c.ch, col * FONT_WIDTH, py, c.fg, c.bg);
        }
    }

    /// Blit dirty regions from back-buffer to the hardware framebuffer.
    ///
    /// This is the ONLY place that writes to HW MMIO (besides the fallback
    /// path when no back-buffer is available). Called once per `_fbcon_print`.
    fn blit_to_framebuffer(&mut self) {
        let back_buf = match self.back_buf {
            Some(ref buf) => buf.as_ptr(),
            None => return, // No back-buffer — rendering went directly to HW FB
        };

        if self.dirty_all {
            // Render all visible rows from cells to back-buffer, then blit all
            for row in 0..self.rows {
                self.render_row_to_backbuf(row);
            }
            // Single large copy to HW FB
            let total_bytes = self.stride * self.height;
            // SAFETY: back_buf and fb_ptr are both `total_bytes` in size,
            // and they do not overlap (back_buf is heap, fb_ptr is MMIO).
            unsafe {
                core::ptr::copy_nonoverlapping(back_buf, self.fb_ptr, total_bytes);
            }
            self.dirty_all = false;
            for flag in self.dirty_rows[..self.rows].iter_mut() {
                *flag = false;
            }
        } else {
            // Render and blit only dirty rows
            let stride = self.stride;
            let row_pixels = FONT_HEIGHT * stride;
            for row in 0..self.rows {
                if self.dirty_rows[row] {
                    self.render_row_to_backbuf(row);
                    let offset = row * FONT_HEIGHT * stride;
                    // SAFETY: offset + row_pixels <= stride * height.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            back_buf.add(offset),
                            self.fb_ptr.add(offset),
                            row_pixels,
                        );
                    }
                    self.dirty_rows[row] = false;
                }
            }
        }
    }

    /// Scroll the text grid up by one row using ring-buffer advancement.
    ///
    /// Phase 2: O(cols) cell operations instead of ~4MB memory copy.
    fn scroll_up(&mut self) {
        // The row that was at the top is recycled to become the new bottom
        let old_top_phys = self.ring_start;
        self.ring_start = (self.ring_start + 1) % MAX_ROWS;

        // Clear the recycled row (now the last visible row)
        let new_bottom_phys = old_top_phys;
        let base = new_bottom_phys * MAX_COLS;
        for col in 0..self.cols {
            self.cells[base + col] = TextCell::blank(self.fg_color, self.bg_color);
        }

        // Full re-render needed since row mapping changed
        self.dirty_all = true;
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
                    let fg = self.fg_color;
                    let bg = self.bg_color;
                    let idx = self.cell_idx(self.cursor_row, self.cursor_col);
                    self.cells[idx] = TextCell { ch: b' ', fg, bg };
                    self.mark_dirty(self.cursor_row);
                }
            }
            0x1B => {
                // ESC — start escape sequence
                self.esc_state = EscapeState::Escape;
                self.esc_param_idx = 0;
                self.esc_params = [0; 16];
            }
            _ => {
                // Printable character — update text cell
                let fg = self.fg_color;
                let bg = self.bg_color;
                let idx = self.cell_idx(self.cursor_row, self.cursor_col);
                self.cells[idx] = TextCell { ch, fg, bg };
                self.mark_dirty(self.cursor_row);

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
                let (start_col, end_col) = match param {
                    1 => (0, self.cursor_col),
                    2 => (0, self.cols),
                    _ => (self.cursor_col, self.cols), // 0 or default
                };
                let fg = self.fg_color;
                let bg = self.bg_color;
                for col in start_col..end_col {
                    let idx = self.cell_idx(self.cursor_row, col);
                    self.cells[idx] = TextCell { ch: b' ', fg, bg };
                }
                self.mark_dirty(self.cursor_row);
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

    /// Clear the entire screen — reset text cells and cursor.
    fn clear(&mut self) {
        let fg = self.fg_color;
        let bg = self.bg_color;
        let blank = TextCell { ch: b' ', fg, bg };
        for row in 0..self.rows {
            for col in 0..self.cols {
                let idx = self.cell_idx(row, col);
                self.cells[idx] = blank;
            }
        }
        self.cursor_col = 0;
        self.cursor_row = 0;
        self.dirty_all = true;
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
///
/// # Safety
///
/// `fb_ptr` must point to a valid framebuffer of at least `stride * height`
/// bytes, mapped for the kernel's lifetime.
pub unsafe fn init(
    fb_ptr: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
    format: FbPixelFormat,
) {
    let mut fbcon = FramebufferConsole::new(fb_ptr, width, height, stride, bpp, format);
    fbcon.clear_hw_and_backbuf();
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
///
/// All text is written to the cell grid and back-buffer (fast RAM), then
/// dirty regions are blitted to the hardware framebuffer (slow MMIO) in
/// a single pass at the end.
pub fn _fbcon_print(args: fmt::Arguments) {
    if !FBCON_OUTPUT_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    use fmt::Write;
    let mut guard = FBCON.lock();
    if let Some(ref mut fbcon) = *guard {
        let _ = fbcon.write_fmt(args);
        fbcon.blit_to_framebuffer();
    }
}

/// Check if the framebuffer console has been initialized.
pub fn is_initialized() -> bool {
    FBCON.lock().is_some()
}

/// Try to allocate a Vec<u8> of the given size, zeroed.
/// Returns Err if allocation fails (OOM).
fn try_alloc_vec(size: usize) -> Result<Vec<u8>, ()> {
    let mut v = Vec::new();
    if v.try_reserve_exact(size).is_err() {
        return Err(());
    }
    v.resize(size, 0);
    Ok(v)
}

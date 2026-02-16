//! Framebuffer console (fbcon) — text rendering onto a pixel framebuffer.
//!
//! Renders characters using the 8x16 bitmap font onto the UEFI-provided
//! (or ramfb-provided) pixel framebuffer. Supports cursor tracking, newlines,
//! tab stops, backspace, scrolling, and basic ANSI color escape sequences.
//!
//! # Performance architecture
//!
//! Three-layer pipeline with **eager rendering**, **glyph cache**, and
//! **pixel ring buffer scroll**, modelled after Linux `fbcon` + `cfb_copyarea`:
//!
//! 1. **Text cell grid** — character/color pairs in a ring buffer. Scrolling is
//!    O(cols) (advance ring pointer + clear one row of cells).
//! 2. **RAM back-buffer** — always in sync via eager rendering (each glyph
//!    rendered to pixels immediately on write). Scrolling advances a pixel ring
//!    offset (O(1), no memmove). A **glyph cache** pre-renders all 256 glyphs
//!    as u32 pixel arrays for the current color pair, making glyph rendering a
//!    `copy_nonoverlapping` of 32 bytes per row.
//! 3. **Hardware framebuffer** — MMIO memory. Touched once per `flush()` call,
//!    pure `copy_nonoverlapping` — zero font lookups, zero conditionals, zero
//!    per-pixel branching. On x86_64, write-combining (PAT) provides 5-150x
//!    faster MMIO writes.
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

/// Pre-rendered glyph pixel data for the current (fg, bg) color pair.
///
/// All 256 glyphs are expanded from 1-bit-per-pixel bitmap to u32-per-pixel
/// words, so rendering a glyph becomes a `copy_nonoverlapping` of 32 bytes
/// per row (8 pixels * 4 bytes) instead of 128 per-pixel bit-extractions.
///
/// Memory: 256 * 8 * 16 * 4 = 128KB per color pair.
struct GlyphCache {
    /// Pre-rendered pixel data: `pixels[glyph * (FONT_WIDTH * FONT_HEIGHT) +
    /// row * FONT_WIDTH + col]`
    pixels: Vec<u32>,
    /// Foreground color word this cache was built for.
    fg_word: u32,
    /// Background color word this cache was built for.
    bg_word: u32,
}

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

    // Glyph cache — pre-rendered pixel data for the current (fg, bg) pair.
    // `None` if allocation failed (OOM fallback to per-pixel path).
    glyph_cache: Option<GlyphCache>,

    // Pixel ring buffer offset — byte offset into back-buffer for screen row 0.
    // Always a multiple of `FONT_HEIGHT * stride`. Eliminates memmove on scroll.
    pixel_ring_offset: usize,

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

        // Allocate glyph cache (128KB). OOM is non-fatal — falls back to
        // per-pixel rendering.
        let glyph_cache = try_alloc_glyph_cache();

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
            glyph_cache,
            pixel_ring_offset: 0,
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

    /// Byte offset in back-buffer for the start of a logical text row's pixels.
    ///
    /// Accounts for the pixel ring buffer offset. The result is always within
    /// `[0, rows * FONT_HEIGHT * stride)` and aligned to a full text-row
    /// boundary, so no glyph ever straddles the ring wrap point.
    #[inline(always)]
    fn pixel_row_offset(&self, logical_row: usize) -> usize {
        let text_row_bytes = FONT_HEIGHT * self.stride;
        let pixel_buf_size = self.rows * text_row_bytes;
        (self.pixel_ring_offset + logical_row * text_row_bytes) % pixel_buf_size
    }

    /// Rebuild the glyph cache for a new (fg, bg) color pair.
    ///
    /// Expands all 256 glyphs from 1-bit-per-pixel bitmaps into u32 pixel
    /// words. Called on first render and on every SGR color change (rare).
    fn rebuild_glyph_cache(&mut self, fg_word: u32, bg_word: u32) {
        let cache = match self.glyph_cache {
            Some(ref mut c) => c,
            None => return,
        };
        cache.fg_word = fg_word;
        cache.bg_word = bg_word;

        for glyph in 0..256u16 {
            let glyph_data = font8x16::glyph(glyph as u8);
            let base = glyph as usize * FONT_WIDTH * FONT_HEIGHT;
            for (row, &bits) in glyph_data.iter().enumerate() {
                for col in 0..FONT_WIDTH {
                    let word = if (bits >> (7 - col)) & 1 != 0 {
                        fg_word
                    } else {
                        bg_word
                    };
                    cache.pixels[base + row * FONT_WIDTH + col] = word;
                }
            }
        }
    }

    /// Render a single glyph to the back-buffer (or directly to HW FB if no
    /// back-buffer is available). Writes 8x16 = 128 pixels.
    ///
    /// Optimization layers:
    /// 1. **Glyph cache hit**: `copy_nonoverlapping` of 32 bytes per row (16
    ///    rows = 512 bytes total) from pre-rendered cache.
    /// 2. **Cache rebuild**: If colors changed, rebuild all 256 glyphs (~300us)
    ///    then cache hit.
    /// 3. **Fallback**: Per-pixel bit extraction (no cache available).
    ///
    /// Pixel ring: When a back-buffer is present, row offsets are computed via
    /// `pixel_row_offset()` to account for the ring buffer.
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
        let stride = self.stride;
        let bpp = self.bpp;

        let has_back_buf = self.back_buf.is_some();
        let buf_ptr = match self.back_buf {
            Some(ref mut buf) => buf.as_mut_ptr(),
            None => self.fb_ptr,
        };

        // Compute ring-adjusted base for this text row.
        // `py` is always a multiple of FONT_HEIGHT (top of a text row).
        let ring_base = if has_back_buf {
            self.pixel_row_offset(py / FONT_HEIGHT)
        } else {
            py * stride // No ring for direct MMIO
        };

        // Try glyph cache path
        if self.glyph_cache.is_some() {
            // Check if cache colors match; rebuild if needed
            {
                let needs_rebuild = match self.glyph_cache {
                    Some(ref c) => c.fg_word != fg_word || c.bg_word != bg_word,
                    None => false,
                };
                if needs_rebuild {
                    self.rebuild_glyph_cache(fg_word, bg_word);
                }
            }

            if let Some(ref cache) = self.glyph_cache {
                let glyph_base = ch as usize * FONT_WIDTH * FONT_HEIGHT;
                // SAFETY: buf_ptr is valid for stride * height bytes (back-buffer)
                // or the HW FB. ring_base is within the ring buffer. Each row
                // copies FONT_WIDTH u32 words (32 bytes) which is within bounds.
                unsafe {
                    for row in 0..FONT_HEIGHT {
                        let buf_offset = ring_base + row * stride + px * bpp;
                        let dst = buf_ptr.add(buf_offset) as *mut u32;
                        let src = cache.pixels.as_ptr().add(glyph_base + row * FONT_WIDTH);
                        core::ptr::copy_nonoverlapping(src, dst, FONT_WIDTH);
                    }
                }
                return;
            }
        }

        // Fallback: per-pixel bit extraction (no glyph cache)
        let glyph_data = font8x16::glyph(ch);
        for (row, &bits) in glyph_data.iter().enumerate() {
            let buf_offset = ring_base + row * stride + px * bpp;
            // SAFETY: Same bounds guarantee as the cached path.
            unsafe {
                let ptr = buf_ptr.add(buf_offset) as *mut u32;
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
    /// No longer called in the normal rendering path (eager rendering +
    /// pixel ring scroll keep the back-buffer up-to-date). Retained for
    /// diagnostic/debug use.
    #[allow(dead_code)]
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
            let ring_base = if self.back_buf.is_some() {
                self.pixel_row_offset(logical_row)
            } else {
                py * self.stride
            };
            let row_bytes = FONT_HEIGHT * self.stride;
            let buf_ptr = match self.back_buf {
                Some(ref mut buf) => buf.as_mut_ptr(),
                None => self.fb_ptr,
            };
            // SAFETY: ring_base + row_bytes is within the ring buffer bounds.
            unsafe {
                core::ptr::write_bytes(buf_ptr.add(ring_base), 0, row_bytes);
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
            let ring_base = if self.back_buf.is_some() {
                self.pixel_row_offset(logical_row)
            } else {
                py * stride
            };
            for row in 0..FONT_HEIGHT {
                // SAFETY: writing within ring buffer bounds.
                unsafe {
                    let base = ring_base + row * stride;
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
    /// Back-buffer is always up-to-date from eager rendering — this is a
    /// pure memcpy with zero font lookups or per-pixel branching.
    ///
    /// The back-buffer uses a pixel ring: logical screen row 0 starts at
    /// `pixel_ring_offset` bytes. The blit linearizes the ring into the
    /// MMIO framebuffer with a two-chunk copy for `dirty_all`, or per-row
    /// ring-adjusted copies for individual dirty rows.
    fn blit_to_framebuffer(&mut self) {
        let back_buf = match self.back_buf {
            Some(ref buf) => buf.as_ptr(),
            None => return, // No back-buffer — eager rendering went directly to HW FB
        };

        let text_row_bytes = FONT_HEIGHT * self.stride;
        let pixel_buf_size = self.rows * text_row_bytes;

        if self.dirty_all {
            // Two-chunk copy: [ring_offset..end] then [0..ring_offset]
            let first_chunk = pixel_buf_size - self.pixel_ring_offset;
            // SAFETY: back_buf is `stride * height` bytes (>= pixel_buf_size).
            // fb_ptr is at least pixel_buf_size bytes. The two chunks together
            // copy exactly pixel_buf_size bytes to MMIO.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    back_buf.add(self.pixel_ring_offset),
                    self.fb_ptr,
                    first_chunk,
                );
                if self.pixel_ring_offset > 0 {
                    core::ptr::copy_nonoverlapping(
                        back_buf,
                        self.fb_ptr.add(first_chunk),
                        self.pixel_ring_offset,
                    );
                }
            }
            self.dirty_all = false;
            for flag in self.dirty_rows[..self.rows].iter_mut() {
                *flag = false;
            }
        } else {
            // Per dirty row: ring-adjusted source, linear MMIO dest
            for row in 0..self.rows {
                if self.dirty_rows[row] {
                    let ring_offset =
                        (self.pixel_ring_offset + row * text_row_bytes) % pixel_buf_size;
                    let mmio_offset = row * text_row_bytes;
                    // SAFETY: ring_offset + text_row_bytes <= pixel_buf_size
                    // (because ring offsets are always text-row-aligned and
                    // pixel_buf_size is a multiple of text_row_bytes).
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            back_buf.add(ring_offset),
                            self.fb_ptr.add(mmio_offset),
                            text_row_bytes,
                        );
                    }
                    self.dirty_rows[row] = false;
                }
            }
        }
    }

    /// Scroll the text grid up by one row.
    ///
    /// Cell ring: O(cols) (advance pointer + clear one row).
    /// Pixel ring (back-buffer): O(1) pointer advance + clear one row of
    /// pixels. No memmove — the ring offset makes the old top row become
    /// the new bottom row.
    /// Fallback (no back-buffer): memmove on MMIO (slow but correct).
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

        if self.back_buf.is_some() {
            // Pixel ring: advance offset (no memmove!)
            let text_row_bytes = FONT_HEIGHT * self.stride;
            let pixel_buf_size = self.rows * text_row_bytes;
            self.pixel_ring_offset = (self.pixel_ring_offset + text_row_bytes) % pixel_buf_size;

            // Clear the new bottom row's pixels in the ring
            let bottom_offset = self.pixel_row_offset(self.rows - 1);
            let buf_ptr = self.back_buf.as_mut().unwrap().as_mut_ptr();
            // SAFETY: bottom_offset + text_row_bytes is within the ring
            // buffer (text_row_bytes aligned offsets, pixel_buf_size is a
            // multiple of text_row_bytes).
            unsafe {
                core::ptr::write_bytes(buf_ptr.add(bottom_offset), 0, text_row_bytes);
            }
        } else {
            // No back-buffer: memmove on MMIO (fallback, slow but correct)
            let row_bytes = FONT_HEIGHT * self.stride;
            let visible_bytes = self.rows * row_bytes;
            // SAFETY: fb_ptr valid for stride * height bytes. Source and dest
            // overlap so we use copy (memmove semantics).
            unsafe {
                core::ptr::copy(
                    self.fb_ptr.add(row_bytes),
                    self.fb_ptr,
                    visible_bytes - row_bytes,
                );
                core::ptr::write_bytes(self.fb_ptr.add(visible_bytes - row_bytes), 0, row_bytes);
            }
        }

        // All rows shifted — need full MMIO blit, but NO glyph re-rendering
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
                    // Eagerly render blank glyph to back-buffer
                    let px = self.cursor_col * FONT_WIDTH;
                    let py = self.cursor_row * FONT_HEIGHT;
                    self.render_glyph_to_buf(b' ', px, py, fg, bg);
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
                // Eagerly render glyph to back-buffer (1 glyph = 128 pixel writes to RAM)
                let px = self.cursor_col * FONT_WIDTH;
                let py = self.cursor_row * FONT_HEIGHT;
                self.render_glyph_to_buf(ch, px, py, fg, bg);
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
                // Eagerly render erased region to back-buffer
                let py = self.cursor_row * FONT_HEIGHT;
                let end = end_col.min(self.cols);
                if bg == (0, 0, 0) {
                    // Fast path: zero pixel region (ring-adjusted)
                    let px_start = start_col * FONT_WIDTH * self.bpp;
                    let px_width = (end - start_col) * FONT_WIDTH * self.bpp;
                    let has_back_buf = self.back_buf.is_some();
                    let ring_base = if has_back_buf {
                        self.pixel_row_offset(self.cursor_row)
                    } else {
                        py * self.stride
                    };
                    let buf_ptr = match self.back_buf {
                        Some(ref mut buf) => buf.as_mut_ptr(),
                        None => self.fb_ptr,
                    };
                    for row in 0..FONT_HEIGHT {
                        let offset = ring_base + row * self.stride + px_start;
                        // SAFETY: offset + px_width is within the ring buffer
                        // (or MMIO buffer) bounds.
                        unsafe {
                            core::ptr::write_bytes(buf_ptr.add(offset), 0, px_width);
                        }
                    }
                } else {
                    for col in start_col..end {
                        self.render_glyph_to_buf(b' ', col * FONT_WIDTH, py, fg, bg);
                    }
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

    /// Clear the entire screen — reset text cells, cursor, and pixel ring.
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
        // Reset pixel ring (screen starts fresh, no offset)
        self.pixel_ring_offset = 0;
        // Eagerly clear back-buffer pixels
        let total_bytes = self.stride * self.height;
        if bg == (0, 0, 0) {
            if let Some(ref mut buf) = self.back_buf {
                // SAFETY: buf is exactly total_bytes in size.
                unsafe {
                    core::ptr::write_bytes(buf.as_mut_ptr(), 0, total_bytes);
                }
            } else {
                // SAFETY: fb_ptr is valid for total_bytes.
                unsafe {
                    core::ptr::write_bytes(self.fb_ptr, 0, total_bytes);
                }
            }
        } else {
            let bg_word = self.color_to_word(bg.0, bg.1, bg.2);
            let buf_ptr = match self.back_buf {
                Some(ref mut buf) => buf.as_mut_ptr(),
                None => self.fb_ptr,
            };
            // SAFETY: total_bytes is stride * height, writing u32 words.
            unsafe {
                let ptr = buf_ptr as *mut u32;
                for i in 0..(total_bytes / 4) {
                    ptr.add(i).write(bg_word);
                }
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
/// Text is written to the cell grid (fast RAM) only. Call [`flush()`] to
/// blit pending changes to the hardware framebuffer. This decoupling
/// allows multi-line output (e.g. `help`) to accumulate in RAM and then
/// hit the slow MMIO path only once.
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

/// Blit all pending changes to the hardware framebuffer.
///
/// Call this after a logical group of output is complete (e.g., after a
/// command finishes, after printing the prompt, after echoing a keystroke).
/// This is the ONLY code path that writes to MMIO.
pub fn flush() {
    if !FBCON_OUTPUT_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut guard = FBCON.lock();
    if let Some(ref mut fbcon) = *guard {
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

/// Try to allocate a glyph cache (128KB). Returns `None` on OOM.
fn try_alloc_glyph_cache() -> Option<GlyphCache> {
    let count = 256 * FONT_WIDTH * FONT_HEIGHT; // 32,768 u32 entries = 128KB
    let mut pixels = Vec::new();
    if pixels.try_reserve_exact(count).is_err() {
        return None;
    }
    pixels.resize(count, 0u32);
    Some(GlyphCache {
        pixels,
        fg_word: 0,
        bg_word: 0,
    })
}

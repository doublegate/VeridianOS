//! Terminal Emulator Application
//!
//! Combines PTY, font rendering, and window manager to provide a graphical
//! terminal.

use alloc::{vec, vec::Vec};

use spin::RwLock;

use crate::{
    desktop::window_manager::{with_window_manager, InputEvent, WindowId},
    error::KernelError,
    fs::pty::with_pty_manager,
    sync::once_lock::GlobalState,
};

/// Terminal dimensions
const TERMINAL_COLS: usize = 80;
const TERMINAL_ROWS: usize = 24;

/// Terminal colors
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    pub const GREEN: Color = Color { r: 0, g: 255, b: 0 };
    pub const BLUE: Color = Color {
        r: 0,
        g: 128,
        b: 255,
    };
}

/// Terminal cell
#[derive(Debug, Clone, Copy)]
struct Cell {
    character: char,
    foreground: Color,
    background: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            foreground: Color::WHITE,
            background: Color::BLACK,
        }
    }
}

/// ANSI escape parser state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscapeState {
    Normal,
    Escape,
    Csi,
}

/// ANSI standard colors (SGR 30-37 foreground, 40-47 background).
const ANSI_COLORS: [Color; 8] = [
    Color { r: 0, g: 0, b: 0 }, // 0: black
    Color {
        r: 0xAA,
        g: 0,
        b: 0,
    }, // 1: red
    Color {
        r: 0,
        g: 0xAA,
        b: 0,
    }, // 2: green
    Color {
        r: 0xAA,
        g: 0x55,
        b: 0,
    }, // 3: yellow/brown
    Color {
        r: 0,
        g: 0,
        b: 0xAA,
    }, // 4: blue
    Color {
        r: 0xAA,
        g: 0,
        b: 0xAA,
    }, // 5: magenta
    Color {
        r: 0,
        g: 0xAA,
        b: 0xAA,
    }, // 6: cyan
    Color {
        r: 0xAA,
        g: 0xAA,
        b: 0xAA,
    }, // 7: white/light gray
];

/// Pixel dimensions for terminal rendering (8x16 font)
const TERMINAL_PX_WIDTH: u32 = (TERMINAL_COLS * 8) as u32;
const TERMINAL_PX_HEIGHT: u32 = (TERMINAL_ROWS * 16) as u32;

/// Terminal emulator
pub struct TerminalEmulator {
    /// Window ID
    window_id: WindowId,

    /// Compositor surface ID
    surface_id: u32,
    /// SHM pool ID
    pool_id: u32,
    /// Pool buffer ID
    pool_buf_id: u32,

    /// PTY master ID
    pty_master_id: u32,

    /// PTY slave ID
    pty_slave_id: u32,

    /// Screen buffer
    buffer: Vec<Vec<Cell>>,

    /// Cursor position
    cursor_x: usize,
    cursor_y: usize,

    /// Current colors
    current_fg: Color,
    current_bg: Color,

    /// Default colors (for SGR reset)
    default_fg: Color,
    default_bg: Color,

    /// Scrollback buffer
    scrollback: Vec<Vec<Cell>>,

    /// Maximum scrollback lines
    max_scrollback: usize,

    /// ANSI escape parser state
    esc_state: EscapeState,
    /// ESC sequence parameter accumulator
    esc_params: [u8; 16],
    /// Current parameter index
    esc_param_idx: usize,
}

impl TerminalEmulator {
    /// Create a new terminal emulator
    pub fn new(width: u32, height: u32) -> Result<Self, KernelError> {
        // Create window
        let window_id = with_window_manager(|wm| wm.create_window(100, 100, width, height, 0))
            .ok_or(KernelError::InvalidState {
            expected: "initialized",
            actual: "uninitialized",
        })??;

        // Create compositor surface at the same position as the WM window
        let (surface_id, pool_id, pool_buf_id) =
            super::renderer::create_app_surface(100, 100, width, height);

        // Create PTY pair
        let (pty_master_id, pty_slave_id) = with_pty_manager(|manager| manager.create_pty())
            .ok_or(KernelError::InvalidState {
                expected: "initialized",
                actual: "uninitialized",
            })??;

        // Initialize buffer
        let mut buffer = Vec::new();
        for _ in 0..TERMINAL_ROWS {
            buffer.push(vec![Cell::default(); TERMINAL_COLS]);
        }

        println!(
            "[TERMINAL] Created terminal emulator: window={}, surface={}, pty={}",
            window_id, surface_id, pty_master_id
        );

        Ok(Self {
            window_id,
            surface_id,
            pool_id,
            pool_buf_id,
            pty_master_id,
            pty_slave_id,
            buffer,
            cursor_x: 0,
            cursor_y: 0,
            current_fg: Color::GREEN,
            current_bg: Color::BLACK,
            default_fg: Color::GREEN,
            default_bg: Color::BLACK,
            scrollback: Vec::new(),
            max_scrollback: 1000,
            esc_state: EscapeState::Normal,
            esc_params: [0; 16],
            esc_param_idx: 0,
        })
    }

    /// Render the terminal contents to its compositor surface.
    pub fn render_to_surface(&self) {
        let w = TERMINAL_PX_WIDTH as usize;
        let h = TERMINAL_PX_HEIGHT as usize;
        let mut pixels = vec![0u8; w * h * 4];
        let _ = self.render(&mut pixels, w, h);
        super::renderer::update_surface_pixels(
            self.surface_id,
            self.pool_id,
            self.pool_buf_id,
            &pixels,
        );
    }

    /// Process input event
    pub fn process_input(&mut self, event: InputEvent) -> Result<(), KernelError> {
        match event {
            InputEvent::KeyPress { character, .. } => {
                // Send character to PTY
                let master_id = self.pty_master_id;
                if let Some(master) =
                    with_pty_manager(|manager| manager.get_master(master_id)).flatten()
                {
                    let mut buf = [0u8; 4];
                    let encoded = character.encode_utf8(&mut buf);
                    master.write(encoded.as_bytes())?;
                }
            }
            InputEvent::KeyRelease { .. } => {
                // Ignore key releases for now
            }
            _ => {
                // Ignore mouse events in terminal
            }
        }

        Ok(())
    }

    /// Update terminal from PTY output
    pub fn update(&mut self) -> Result<(), KernelError> {
        // Read from PTY
        let master_id = self.pty_master_id;
        if let Some(master) = with_pty_manager(|manager| manager.get_master(master_id)).flatten() {
            let mut buf = [0u8; 1024];
            match master.read(&mut buf) {
                Ok(bytes_read) => {
                    if bytes_read > 0 {
                        // Process output
                        for &byte in &buf[..bytes_read] {
                            self.process_output_byte(byte);
                        }
                    }
                }
                Err(_) => {
                    // No data available
                }
            }
        }

        Ok(())
    }

    /// Process a single output byte with ANSI escape sequence support.
    fn process_output_byte(&mut self, byte: u8) {
        match self.esc_state {
            EscapeState::Normal => self.process_normal(byte),
            EscapeState::Escape => self.process_escape(byte),
            EscapeState::Csi => self.process_csi(byte),
        }
    }

    /// Handle a byte in normal (non-escape) mode.
    fn process_normal(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= TERMINAL_ROWS {
                    self.scroll_up();
                }
            }
            b'\r' => {
                self.cursor_x = 0;
            }
            b'\t' => {
                self.cursor_x = (self.cursor_x + 8) & !7;
                if self.cursor_x >= TERMINAL_COLS {
                    self.cursor_x = 0;
                    self.cursor_y += 1;
                    if self.cursor_y >= TERMINAL_ROWS {
                        self.scroll_up();
                    }
                }
            }
            b'\x08' => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                    self.buffer[self.cursor_y][self.cursor_x] = Cell::default();
                }
            }
            0x1B => {
                // ESC — start escape sequence
                self.esc_state = EscapeState::Escape;
                self.esc_param_idx = 0;
                self.esc_params = [0; 16];
            }
            0x20..=0x7E => {
                self.buffer[self.cursor_y][self.cursor_x] = Cell {
                    character: byte as char,
                    foreground: self.current_fg,
                    background: self.current_bg,
                };
                self.cursor_x += 1;
                if self.cursor_x >= TERMINAL_COLS {
                    self.cursor_x = 0;
                    self.cursor_y += 1;
                    if self.cursor_y >= TERMINAL_ROWS {
                        self.scroll_up();
                    }
                }
            }
            _ => {}
        }
    }

    /// Handle a byte after ESC was received.
    fn process_escape(&mut self, byte: u8) {
        if byte == b'[' {
            self.esc_state = EscapeState::Csi;
        } else {
            self.esc_state = EscapeState::Normal;
        }
    }

    /// Handle a byte inside a CSI sequence (ESC [ ...).
    fn process_csi(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                if self.esc_param_idx < self.esc_params.len() {
                    self.esc_params[self.esc_param_idx] = self.esc_params[self.esc_param_idx]
                        .wrapping_mul(10)
                        .wrapping_add(byte - b'0');
                }
            }
            b';' => {
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
                    // Clear entire screen
                    for row in self.buffer.iter_mut() {
                        for cell in row.iter_mut() {
                            *cell = Cell::default();
                        }
                    }
                    self.cursor_x = 0;
                    self.cursor_y = 0;
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
                self.cursor_y = row.min(TERMINAL_ROWS - 1);
                self.cursor_x = col.min(TERMINAL_COLS - 1);
                self.esc_state = EscapeState::Normal;
            }
            b'A' => {
                // Cursor Up
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_y = self.cursor_y.saturating_sub(n);
                self.esc_state = EscapeState::Normal;
            }
            b'B' => {
                // Cursor Down
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_y = (self.cursor_y + n).min(TERMINAL_ROWS - 1);
                self.esc_state = EscapeState::Normal;
            }
            b'C' => {
                // Cursor Forward
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_x = (self.cursor_x + n).min(TERMINAL_COLS - 1);
                self.esc_state = EscapeState::Normal;
            }
            b'D' => {
                // Cursor Back
                let n = if self.esc_params[0] > 0 {
                    self.esc_params[0] as usize
                } else {
                    1
                };
                self.cursor_x = self.cursor_x.saturating_sub(n);
                self.esc_state = EscapeState::Normal;
            }
            b'K' => {
                // Erase in Line
                let param = self.esc_params[0];
                let (start, end) = match param {
                    1 => (0, self.cursor_x),
                    2 => (0, TERMINAL_COLS),
                    _ => (self.cursor_x, TERMINAL_COLS),
                };
                for col in start..end.min(TERMINAL_COLS) {
                    self.buffer[self.cursor_y][col] = Cell::default();
                }
                self.esc_state = EscapeState::Normal;
            }
            _ => {
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
                    self.current_fg = self.default_fg;
                    self.current_bg = self.default_bg;
                }
                1 => {
                    // Bold: brighten foreground
                    self.current_fg = Color {
                        r: self.current_fg.r.saturating_add(0x55),
                        g: self.current_fg.g.saturating_add(0x55),
                        b: self.current_fg.b.saturating_add(0x55),
                    };
                }
                30..=37 => {
                    self.current_fg = ANSI_COLORS[(code - 30) as usize];
                }
                40..=47 => {
                    self.current_bg = ANSI_COLORS[(code - 40) as usize];
                }
                _ => {}
            }
        }
    }

    /// Scroll buffer up one line
    fn scroll_up(&mut self) {
        // Save first line to scrollback
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.remove(0);
        }
        self.scrollback.push(self.buffer[0].clone());

        // Shift lines up
        for y in 0..TERMINAL_ROWS - 1 {
            self.buffer[y] = self.buffer[y + 1].clone();
        }

        // Clear bottom line
        self.buffer[TERMINAL_ROWS - 1] = vec![Cell::default(); TERMINAL_COLS];
        self.cursor_y = TERMINAL_ROWS - 1;
    }

    /// Render terminal to a BGRA pixel buffer.
    ///
    /// `buf` is width*height*4 bytes in BGRA format.
    pub fn render(&self, buf: &mut [u8], width: usize, _height: usize) -> Result<(), KernelError> {
        use super::renderer::draw_char_into_buffer;

        let char_w = 8;
        let char_h = 16;

        // Clear to black background (BGRA)
        for chunk in buf.chunks_exact_mut(4) {
            chunk[0] = 0x00; // B
            chunk[1] = 0x00; // G
            chunk[2] = 0x00; // R
            chunk[3] = 0xFF; // A
        }

        // Render each cell with its foreground color
        for y in 0..TERMINAL_ROWS {
            for x in 0..TERMINAL_COLS {
                let cell = &self.buffer[y][x];
                if cell.character == ' ' {
                    continue;
                }
                // Background fill for non-black cells
                if cell.background.r != 0 || cell.background.g != 0 || cell.background.b != 0 {
                    let px0 = x * char_w;
                    let py0 = y * char_h;
                    for dy in 0..char_h {
                        for dx in 0..char_w {
                            let offset = ((py0 + dy) * width + (px0 + dx)) * 4;
                            if offset + 3 < buf.len() {
                                buf[offset] = cell.background.b;
                                buf[offset + 1] = cell.background.g;
                                buf[offset + 2] = cell.background.r;
                                buf[offset + 3] = 0xFF;
                            }
                        }
                    }
                }
                let fg_color = ((cell.foreground.r as u32) << 16)
                    | ((cell.foreground.g as u32) << 8)
                    | (cell.foreground.b as u32);
                let ch = cell.character as u8;
                draw_char_into_buffer(buf, width, ch, x * char_w, y * char_h, fg_color);
            }
        }

        // Draw block cursor
        let cx = self.cursor_x * char_w;
        let cy = self.cursor_y * char_h;
        for dy in 0..char_h {
            for dx in 0..char_w {
                let offset = ((cy + dy) * width + (cx + dx)) * 4;
                if offset + 3 < buf.len() {
                    // Invert: use foreground color with some transparency effect
                    buf[offset] = 0xCC; // B
                    buf[offset + 1] = 0xCC; // G
                    buf[offset + 2] = 0xCC; // R
                    buf[offset + 3] = 0xFF; // A
                }
            }
        }

        Ok(())
    }

    /// Get window ID
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// Get PTY slave ID for shell connection
    pub fn pty_slave_id(&self) -> u32 {
        self.pty_slave_id
    }
}

/// Terminal manager for multiple terminals
pub struct TerminalManager {
    terminals: RwLock<Vec<TerminalEmulator>>,
}

impl TerminalManager {
    /// Create a new terminal manager
    pub fn new() -> Self {
        Self {
            terminals: RwLock::new(Vec::new()),
        }
    }

    /// Create a new terminal
    pub fn create_terminal(&self, width: u32, height: u32) -> Result<usize, KernelError> {
        let terminal = TerminalEmulator::new(width, height)?;
        let mut terminals = self.terminals.write();
        terminals.push(terminal);
        Ok(terminals.len() - 1)
    }

    /// Process input for a terminal
    pub fn process_input(&self, terminal_id: usize, event: InputEvent) -> Result<(), KernelError> {
        let mut terminals = self.terminals.write();
        if let Some(terminal) = terminals.get_mut(terminal_id) {
            terminal.process_input(event)
        } else {
            Err(KernelError::NotFound {
                resource: "terminal",
                id: terminal_id as u64,
            })
        }
    }

    /// Update all terminals (read PTY output).
    pub fn update_all(&self) -> Result<(), KernelError> {
        let mut terminals = self.terminals.write();
        for terminal in terminals.iter_mut() {
            terminal.update()?;
        }
        Ok(())
    }

    /// Get the window ID of a terminal by index.
    pub fn get_window_id(&self, terminal_id: usize) -> Option<WindowId> {
        let terminals = self.terminals.read();
        terminals.get(terminal_id).map(|t| t.window_id())
    }

    /// Render all terminal surfaces to the compositor.
    pub fn render_all_surfaces(&self) {
        let terminals = self.terminals.read();
        for terminal in terminals.iter() {
            terminal.render_to_surface();
        }
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global terminal manager
static TERMINAL_MANAGER: GlobalState<TerminalManager> = GlobalState::new();

/// Initialize terminal system
pub fn init() -> Result<(), KernelError> {
    let manager = TerminalManager::new();
    TERMINAL_MANAGER
        .init(manager)
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    println!("[TERMINAL] Terminal emulator system initialized");
    Ok(())
}

/// Execute a function with the terminal manager
pub fn with_terminal_manager<R, F: FnOnce(&TerminalManager) -> R>(f: F) -> Option<R> {
    TERMINAL_MANAGER.with(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_default() {
        let cell = Cell::default();
        assert_eq!(cell.character, ' ');
    }

    #[test]
    fn test_terminal_dimensions() {
        assert_eq!(TERMINAL_COLS, 80);
        assert_eq!(TERMINAL_ROWS, 24);
    }
}

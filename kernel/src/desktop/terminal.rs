//! Terminal Emulator Application
//!
//! Combines PTY, font rendering, and window manager to provide a graphical
//! terminal.

use alloc::{string::ToString, vec, vec::Vec};

use spin::RwLock;

use crate::{
    desktop::{
        font::{with_font_manager, FontSize, FontStyle},
        window_manager::{with_window_manager, InputEvent, WindowId},
    },
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
    #[allow(dead_code)] // Rendering field -- used when terminal display is connected
    foreground: Color,
    #[allow(dead_code)] // Rendering field -- used when terminal display is connected
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

/// Terminal emulator
pub struct TerminalEmulator {
    /// Window ID
    window_id: WindowId,

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
            "[TERMINAL] Created terminal emulator: window={}, pty={}",
            window_id, pty_master_id
        );

        Ok(Self {
            window_id,
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

    /// Render terminal to framebuffer
    pub fn render(
        &self,
        framebuffer: &mut [u8],
        fb_width: usize,
        fb_height: usize,
    ) -> Result<(), KernelError> {
        // Clear framebuffer to background color
        for pixel in framebuffer.iter_mut() {
            *pixel = 0; // Black background
        }

        // Render each cell
        let char_width = 8; // Approximate character width
        let char_height = 12; // Font size

        for y in 0..TERMINAL_ROWS {
            for x in 0..TERMINAL_COLS {
                let cell = &self.buffer[y][x];

                if cell.character != ' ' {
                    // Render character
                    let screen_x = (x * char_width) as i32;
                    let screen_y = (y * char_height) as i32;

                    // Use font rendering (simplified - would need proper glyph rendering)
                    let char_str = cell.character.to_string();
                    let _ = with_font_manager(|fm| {
                        if let Some(font) = fm.get_font(FontSize::Medium, FontStyle::Regular) {
                            let _ = font.render_text(
                                &char_str,
                                framebuffer,
                                fb_width,
                                fb_height,
                                screen_x,
                                screen_y,
                            );
                        }
                    });
                }
            }
        }

        // Render cursor
        let cursor_screen_x = self.cursor_x * char_width;
        let cursor_screen_y = self.cursor_y * char_height;

        for dx in 0..char_width {
            for dy in 0..2 {
                let px = cursor_screen_x + dx;
                let py = cursor_screen_y + char_height - 1 - dy;

                if px < fb_width && py < fb_height {
                    let index = py * fb_width + px;
                    if index < framebuffer.len() {
                        framebuffer[index] = 255; // White cursor
                    }
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

    /// Update all terminals
    pub fn update_all(&self) -> Result<(), KernelError> {
        let mut terminals = self.terminals.write();
        for terminal in terminals.iter_mut() {
            terminal.update()?;
        }
        Ok(())
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

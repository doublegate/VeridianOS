//! Terminal Emulator Application
//!
//! Combines PTY, font rendering, and window manager to provide a graphical terminal.

use crate::error::KernelError;
use crate::desktop::font::{FontSize, FontStyle, get_font_manager};
use crate::desktop::window_manager::{WindowId, get_window_manager, InputEvent};
use crate::fs::pty::with_pty_manager;
use crate::sync::once_lock::GlobalState;
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::ToString;
use spin::RwLock;

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
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
    pub const GREEN: Color = Color { r: 0, g: 255, b: 0 };
    pub const BLUE: Color = Color { r: 0, g: 128, b: 255 };
}

/// Terminal cell
#[derive(Debug, Clone, Copy)]
struct Cell {
    character: char,
    #[allow(dead_code)]
    foreground: Color,
    #[allow(dead_code)]
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

    /// Scrollback buffer
    scrollback: Vec<Vec<Cell>>,

    /// Maximum scrollback lines
    max_scrollback: usize,
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

        println!("[TERMINAL] Created terminal emulator: window={}, pty={}",
                 window_id, pty_master_id);

        Ok(Self {
            window_id,
            pty_master_id,
            pty_slave_id,
            buffer,
            cursor_x: 0,
            cursor_y: 0,
            current_fg: Color::GREEN,
            current_bg: Color::BLACK,
            scrollback: Vec::new(),
            max_scrollback: 1000,
        })
    }

    /// Process input event
    pub fn process_input(&mut self, event: InputEvent) -> Result<(), KernelError> {
        match event {
            InputEvent::KeyPress { character, .. } => {
                // Send character to PTY
                let master_id = self.pty_master_id;
                if let Some(master) = with_pty_manager(|manager| manager.get_master(master_id)).flatten() {
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

    /// Process a single output byte
    fn process_output_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                // Newline
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= TERMINAL_ROWS {
                    self.scroll_up();
                }
            }
            b'\r' => {
                // Carriage return
                self.cursor_x = 0;
            }
            b'\t' => {
                // Tab
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
                // Backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                    self.buffer[self.cursor_y][self.cursor_x] = Cell::default();
                }
            }
            0x20..=0x7E => {
                // Printable ASCII
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
            _ => {
                // Ignore other control characters for now
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
    pub fn render(&self, framebuffer: &mut [u8], fb_width: usize, fb_height: usize) -> Result<(), KernelError> {
        // Get font
        let font_manager = get_font_manager()?;
        let font = font_manager.get_font(FontSize::Medium, FontStyle::Regular)
            .ok_or(KernelError::NotFound { resource: "font", id: 0 })?;

        // Clear framebuffer to background color
        for pixel in framebuffer.iter_mut() {
            *pixel = 0; // Black background
        }

        // Render each cell
        let char_width = 8;  // Approximate character width
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
                    let _ = font.render_text(&char_str, framebuffer, fb_width, fb_height, screen_x, screen_y);
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
            Err(KernelError::NotFound { resource: "terminal", id: terminal_id as u64 })
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
    TERMINAL_MANAGER.init(manager).map_err(|_| KernelError::InvalidState {
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

    #[test_case]
    fn test_cell_default() {
        let cell = Cell::default();
        assert_eq!(cell.character, ' ');
    }

    #[test_case]
    fn test_terminal_dimensions() {
        assert_eq!(TERMINAL_COLS, 80);
        assert_eq!(TERMINAL_ROWS, 24);
    }
}

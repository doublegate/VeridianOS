//! VeridianOS Terminal Emulator
//!
//! A VT100-compatible terminal emulator for VeridianOS.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::String;
use core::panic::PanicInfo;

/// Terminal dimensions
const TERM_COLS: usize = 80;
const TERM_ROWS: usize = 25;

/// Character cell
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg_color: u32,
    pub bg_color: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Cell {
    pub fn new(ch: char) -> Self {
        Self {
            ch,
            fg_color: 0xFFECEFF4, // Nord Snow Storm
            bg_color: 0xFF2E3440, // Nord Polar Night
            bold: false,
            italic: false,
            underline: false,
        }
    }

    pub fn blank() -> Self {
        Self::new(' ')
    }
}

/// Terminal buffer
pub struct TerminalBuffer {
    cells: Vec<Cell>,
    cols: usize,
    rows: usize,
    cursor_x: usize,
    cursor_y: usize,
    cursor_visible: bool,
    scroll_top: usize,
    scroll_bottom: usize,
}

impl TerminalBuffer {
    /// Create a new terminal buffer
    pub fn new(cols: usize, rows: usize) -> Self {
        let cells = alloc::vec![Cell::blank(); cols * rows];

        Self {
            cells,
            cols,
            rows,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            scroll_top: 0,
            scroll_bottom: rows - 1,
        }
    }

    /// Get cell at position
    pub fn get(&self, x: usize, y: usize) -> Option<&Cell> {
        if x < self.cols && y < self.rows {
            Some(&self.cells[y * self.cols + x])
        } else {
            None
        }
    }

    /// Set cell at position
    pub fn set(&mut self, x: usize, y: usize, cell: Cell) {
        if x < self.cols && y < self.rows {
            self.cells[y * self.cols + x] = cell;
        }
    }

    /// Write a character at cursor position
    pub fn write_char(&mut self, ch: char) {
        match ch {
            '\n' => self.newline(),
            '\r' => self.cursor_x = 0,
            '\t' => {
                // Tab to next 8-column boundary
                let next_tab = ((self.cursor_x / 8) + 1) * 8;
                self.cursor_x = next_tab.min(self.cols - 1);
            }
            '\x08' => {
                // Backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            ch => {
                if self.cursor_x >= self.cols {
                    self.newline();
                }

                self.set(self.cursor_x, self.cursor_y, Cell::new(ch));
                self.cursor_x += 1;

                if self.cursor_x >= self.cols {
                    self.newline();
                }
            }
        }
    }

    /// Write a string at cursor position
    pub fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }

    /// Move to new line
    fn newline(&mut self) {
        self.cursor_x = 0;
        if self.cursor_y < self.scroll_bottom {
            self.cursor_y += 1;
        } else {
            self.scroll_up(1);
        }
    }

    /// Scroll up by n lines
    fn scroll_up(&mut self, n: usize) {
        let start = self.scroll_top;
        let end = self.scroll_bottom;
        let height = end - start + 1;

        if n >= height {
            // Clear entire scroll region
            for y in start..=end {
                for x in 0..self.cols {
                    self.set(x, y, Cell::blank());
                }
            }
        } else {
            // Shift lines up
            for y in start..=(end - n) {
                for x in 0..self.cols {
                    let cell = *self.get(x, y + n).unwrap();
                    self.set(x, y, cell);
                }
            }

            // Clear bottom n lines
            for y in (end - n + 1)..=end {
                for x in 0..self.cols {
                    self.set(x, y, Cell::blank());
                }
            }
        }
    }

    /// Clear screen
    pub fn clear(&mut self) {
        for cell in self.cells.iter_mut() {
            *cell = Cell::blank();
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    /// Move cursor to position
    pub fn move_cursor(&mut self, x: usize, y: usize) {
        self.cursor_x = x.min(self.cols - 1);
        self.cursor_y = y.min(self.rows - 1);
    }

    /// Get cursor position
    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }
}

/// ANSI escape sequence parser state
#[derive(Debug, Clone, Copy, PartialEq)]
enum ParserState {
    Normal,
    Escape,
    Csi,
    OscString,
}

/// Terminal emulator with VT100 support
pub struct Terminal {
    buffer: TerminalBuffer,
    parser_state: ParserState,
    params: Vec<usize>,
    current_param: Option<usize>,
    fg_color: u32,
    bg_color: u32,
}

impl Terminal {
    /// Create a new terminal
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            buffer: TerminalBuffer::new(cols, rows),
            parser_state: ParserState::Normal,
            params: Vec::new(),
            current_param: None,
            fg_color: 0xFFECEFF4,
            bg_color: 0xFF2E3440,
        }
    }

    /// Process input byte
    pub fn process_byte(&mut self, byte: u8) {
        match self.parser_state {
            ParserState::Normal => {
                if byte == 0x1B {
                    // ESC
                    self.parser_state = ParserState::Escape;
                } else {
                    self.buffer.write_char(byte as char);
                }
            }
            ParserState::Escape => {
                if byte == b'[' {
                    // CSI
                    self.parser_state = ParserState::Csi;
                    self.params.clear();
                    self.current_param = None;
                } else if byte == b']' {
                    // OSC
                    self.parser_state = ParserState::OscString;
                } else {
                    // Unknown escape sequence
                    self.parser_state = ParserState::Normal;
                }
            }
            ParserState::Csi => {
                if byte >= b'0' && byte <= b'9' {
                    // Parameter digit
                    let digit = (byte - b'0') as usize;
                    if let Some(param) = self.current_param {
                        self.current_param = Some(param * 10 + digit);
                    } else {
                        self.current_param = Some(digit);
                    }
                } else if byte == b';' {
                    // Parameter separator
                    if let Some(param) = self.current_param {
                        self.params.push(param);
                    }
                    self.current_param = None;
                } else {
                    // Final byte - execute command
                    if let Some(param) = self.current_param {
                        self.params.push(param);
                    }
                    self.execute_csi(byte);
                    self.parser_state = ParserState::Normal;
                }
            }
            ParserState::OscString => {
                // OSC string terminator
                if byte == 0x07 || byte == 0x1B {
                    self.parser_state = ParserState::Normal;
                }
            }
        }
    }

    /// Execute CSI command
    fn execute_csi(&mut self, cmd: u8) {
        match cmd {
            b'A' => {
                // Cursor up
                let n = self.params.get(0).copied().unwrap_or(1);
                let (x, y) = self.buffer.cursor_pos();
                self.buffer.move_cursor(x, y.saturating_sub(n));
            }
            b'B' => {
                // Cursor down
                let n = self.params.get(0).copied().unwrap_or(1);
                let (x, y) = self.buffer.cursor_pos();
                self.buffer.move_cursor(x, y + n);
            }
            b'C' => {
                // Cursor forward
                let n = self.params.get(0).copied().unwrap_or(1);
                let (x, y) = self.buffer.cursor_pos();
                self.buffer.move_cursor(x + n, y);
            }
            b'D' => {
                // Cursor back
                let n = self.params.get(0).copied().unwrap_or(1);
                let (x, y) = self.buffer.cursor_pos();
                self.buffer.move_cursor(x.saturating_sub(n), y);
            }
            b'H' | b'f' => {
                // Cursor position
                let row = self.params.get(0).copied().unwrap_or(1).saturating_sub(1);
                let col = self.params.get(1).copied().unwrap_or(1).saturating_sub(1);
                self.buffer.move_cursor(col, row);
            }
            b'J' => {
                // Erase in display
                let n = self.params.get(0).copied().unwrap_or(0);
                if n == 2 {
                    self.buffer.clear();
                }
            }
            b'K' => {
                // Erase in line
                let _n = self.params.get(0).copied().unwrap_or(0);
                // TODO: Implement line erase
            }
            b'm' => {
                // SGR - Select Graphic Rendition
                // TODO: Implement color and style changes
            }
            _ => {
                // Unknown command
            }
        }
    }

    /// Write data to terminal
    pub fn write(&mut self, data: &[u8]) {
        for &byte in data {
            self.process_byte(byte);
        }
    }

    /// Get terminal buffer
    pub fn buffer(&self) -> &TerminalBuffer {
        &self.buffer
    }

    /// Render terminal to framebuffer
    pub fn render(&self, fb: &mut [u32], fb_width: usize, fb_height: usize) {
        // Character size in pixels
        let char_width = 8;
        let char_height = 16;

        // Calculate terminal area
        let term_width = self.buffer.cols * char_width;
        let term_height = self.buffer.rows * char_height;

        // Clear framebuffer
        for pixel in fb.iter_mut() {
            *pixel = 0xFF2E3440; // Background color
        }

        // Render each character
        for y in 0..self.buffer.rows {
            for x in 0..self.buffer.cols {
                if let Some(cell) = self.buffer.get(x, y) {
                    let px = x * char_width;
                    let py = y * char_height;

                    // Draw character background
                    for dy in 0..char_height {
                        for dx in 0..char_width {
                            let fx = px + dx;
                            let fy = py + dy;
                            if fx < fb_width && fy < fb_height {
                                let offset = fy * fb_width + fx;
                                if offset < fb.len() {
                                    fb[offset] = cell.bg_color;
                                }
                            }
                        }
                    }

                    // Draw character (simplified - real implementation would use font bitmap)
                    // For now, just draw a rectangle for visible characters
                    if cell.ch != ' ' {
                        for dy in 2..14 {
                            for dx in 2..6 {
                                let fx = px + dx;
                                let fy = py + dy;
                                if fx < fb_width && fy < fb_height {
                                    let offset = fy * fb_width + fx;
                                    if offset < fb.len() {
                                        fb[offset] = cell.fg_color;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Draw cursor
        if self.buffer.cursor_visible {
            let (cx, cy) = self.buffer.cursor_pos();
            let px = cx * char_width;
            let py = cy * char_height + char_height - 2;

            for dx in 0..char_width {
                let fx = px + dx;
                let fy = py;
                if fx < fb_width && fy < fb_height {
                    let offset = fy * fb_width + fx;
                    if offset < fb.len() {
                        fb[offset] = 0xFFECEFF4; // Cursor color
                    }
                }
            }
        }
    }
}

/// Main entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Create terminal
    let mut term = Terminal::new(TERM_COLS, TERM_ROWS);

    // Write some test output
    term.write(b"VeridianOS Terminal v0.1.0\n");
    term.write(b"Type 'help' for available commands\n\n");
    term.write(b"$ ");

    // In a real implementation, this would:
    // 1. Connect to window manager to get a window
    // 2. Set up shell process with PTY
    // 3. Enter event loop:
    //    - Receive keyboard input
    //    - Send to shell PTY
    //    - Receive output from shell PTY
    //    - Update terminal buffer
    //    - Render to framebuffer
    //    - Send update to window manager

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

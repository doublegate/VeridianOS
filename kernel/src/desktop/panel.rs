//! Desktop Panel (Taskbar)
//!
//! Renders a taskbar at the bottom of the screen showing open windows,
//! system clock, and system tray area.

use alloc::{string::String, vec::Vec};

use spin::RwLock;

use super::window_manager::{with_window_manager, WindowId};
use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Panel height in pixels.
pub const PANEL_HEIGHT: u32 = 32;

/// Panel button representing a window in the taskbar.
#[derive(Debug, Clone)]
struct PanelButton {
    window_id: WindowId,
    title: String,
    x: i32,
    width: u32,
    focused: bool,
}

/// Desktop panel state.
pub struct Panel {
    /// Screen width.
    screen_width: u32,
    /// Screen height.
    screen_height: u32,
    /// Window buttons in the taskbar.
    buttons: RwLock<Vec<PanelButton>>,
    /// Clock string (updated periodically).
    clock_text: RwLock<String>,
}

impl Panel {
    /// Create a new panel for the given screen dimensions.
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        Self {
            screen_width,
            screen_height,
            buttons: RwLock::new(Vec::new()),
            clock_text: RwLock::new(String::from("00:00")),
        }
    }

    /// Get the Y coordinate of the panel (bottom of screen).
    pub fn y(&self) -> i32 {
        (self.screen_height - PANEL_HEIGHT) as i32
    }

    /// Update the panel button list from the window manager.
    pub fn update_buttons(&self) {
        let windows = with_window_manager(|wm| wm.get_all_windows()).unwrap_or_default();

        let mut buttons = self.buttons.write();
        buttons.clear();

        let button_width = 120u32;
        let mut x = 4i32;

        for window in &windows {
            if !window.visible {
                continue;
            }
            buttons.push(PanelButton {
                window_id: window.id,
                title: String::from(window.title_str()),
                x,
                width: button_width,
                focused: window.focused,
            });
            x += button_width as i32 + 4;
        }
    }

    /// Update the clock display.
    pub fn update_clock(&self) {
        let uptime_ticks = crate::arch::timer::read_hw_timestamp();
        // Approximate: convert ticks to seconds (assume ~1GHz TSC)
        let secs = uptime_ticks / 1_000_000_000;
        let hours = (secs / 3600) % 24;
        let minutes = (secs / 60) % 60;
        let mut clock = self.clock_text.write();
        clock.clear();
        // Manual formatting without format! to avoid alloc overhead
        if hours < 10 {
            clock.push('0');
        }
        for ch in fmt_u64(hours).chars() {
            clock.push(ch);
        }
        clock.push(':');
        if minutes < 10 {
            clock.push('0');
        }
        for ch in fmt_u64(minutes).chars() {
            clock.push(ch);
        }
    }

    /// Handle a click on the panel.
    ///
    /// Returns the window ID that should be focused, if any.
    pub fn handle_click(&self, x: i32, _y: i32) -> Option<WindowId> {
        let buttons = self.buttons.read();
        for button in buttons.iter() {
            if x >= button.x && x < button.x + button.width as i32 {
                return Some(button.window_id);
            }
        }
        None
    }

    /// Render the panel into a pixel buffer.
    ///
    /// The buffer is assumed to be `screen_width * PANEL_HEIGHT * 4` bytes
    /// (BGRA format). Renders a dark background with button labels and clock.
    pub fn render(&self, buf: &mut [u8]) {
        let w = self.screen_width as usize;
        let h = PANEL_HEIGHT as usize;
        let stride = w * 4;

        // Dark background (0x2D2D2D)
        for y in 0..h {
            for x in 0..w {
                let offset = y * stride + x * 4;
                if offset + 3 < buf.len() {
                    buf[offset] = 0x2D; // B
                    buf[offset + 1] = 0x2D; // G
                    buf[offset + 2] = 0x2D; // R
                    buf[offset + 3] = 0xFF; // A
                }
            }
        }

        // Render window buttons
        let buttons = self.buttons.read();
        for button in buttons.iter() {
            let btn_x = button.x as usize;
            let btn_w = button.width as usize;

            // Button background (lighter if focused)
            let (br, bg, bb) = if button.focused {
                (0x50, 0x50, 0x70)
            } else {
                (0x40, 0x40, 0x40)
            };

            for y in 4..h - 4 {
                for x in btn_x..(btn_x + btn_w).min(w) {
                    let offset = y * stride + x * 4;
                    if offset + 3 < buf.len() {
                        buf[offset] = bb;
                        buf[offset + 1] = bg;
                        buf[offset + 2] = br;
                        buf[offset + 3] = 0xFF;
                    }
                }
            }

            // Render button title (first 14 chars, using 8px font)
            let title_bytes = button.title.as_bytes();
            let max_chars = (btn_w / 8).min(14);
            for (i, &ch) in title_bytes.iter().take(max_chars).enumerate() {
                render_char_to_buf(buf, stride, btn_x + 4 + i * 8, 10, ch, (0xCC, 0xCC, 0xCC));
            }
        }

        // Render clock in the right side
        let clock = self.clock_text.read();
        let clock_x = w - (clock.len() * 8) - 8;
        for (i, &ch) in clock.as_bytes().iter().enumerate() {
            render_char_to_buf(buf, stride, clock_x + i * 8, 10, ch, (0xAA, 0xAA, 0xAA));
        }
    }
}

/// Render a single 8x16 character into a pixel buffer at (px, py).
fn render_char_to_buf(
    buf: &mut [u8],
    stride: usize,
    px: usize,
    py: usize,
    ch: u8,
    color: (u8, u8, u8),
) {
    use crate::graphics::font8x16;

    let glyph = font8x16::glyph(ch);
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 != 0 {
                let x = px + col;
                let y = py + row;
                let offset = y * stride + x * 4;
                if offset + 3 < buf.len() {
                    buf[offset] = color.2; // B
                    buf[offset + 1] = color.1; // G
                    buf[offset + 2] = color.0; // R
                    buf[offset + 3] = 0xFF;
                }
            }
        }
    }
}

/// Format a u64 (0-59) as a decimal string (no heap allocation).
fn fmt_u64(n: u64) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        10 => "10",
        11 => "11",
        12 => "12",
        13 => "13",
        14 => "14",
        15 => "15",
        16 => "16",
        17 => "17",
        18 => "18",
        19 => "19",
        20 => "20",
        21 => "21",
        22 => "22",
        23 => "23",
        24 => "24",
        25 => "25",
        26 => "26",
        27 => "27",
        28 => "28",
        29 => "29",
        30 => "30",
        31 => "31",
        32 => "32",
        33 => "33",
        34 => "34",
        35 => "35",
        36 => "36",
        37 => "37",
        38 => "38",
        39 => "39",
        40 => "40",
        41 => "41",
        42 => "42",
        43 => "43",
        44 => "44",
        45 => "45",
        46 => "46",
        47 => "47",
        48 => "48",
        49 => "49",
        50 => "50",
        51 => "51",
        52 => "52",
        53 => "53",
        54 => "54",
        55 => "55",
        56 => "56",
        57 => "57",
        58 => "58",
        59 => "59",
        _ => "??",
    }
}

/// Global panel instance
static PANEL: GlobalState<Panel> = GlobalState::new();

/// Initialize the desktop panel.
pub fn init(screen_width: u32, screen_height: u32) -> Result<(), KernelError> {
    PANEL
        .init(Panel::new(screen_width, screen_height))
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    crate::println!(
        "[PANEL] Desktop panel initialized ({}x{})",
        screen_width,
        PANEL_HEIGHT
    );
    Ok(())
}

/// Execute a function with the panel.
pub fn with_panel<R, F: FnOnce(&Panel) -> R>(f: F) -> Option<R> {
    PANEL.with(f)
}

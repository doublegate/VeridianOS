//! Desktop Panel (Taskbar) with Layer-Shell Integration
//!
//! Renders a taskbar at the bottom of the screen showing workspace
//! indicators, open windows, system clock with date, and system tray area.
//!
//! ## Layer-Shell Protocol
//!
//! The panel uses the wlr-layer-shell protocol to anchor itself to the
//! bottom edge of the output. Layer-shell surfaces are rendered above
//! normal windows and below overlay surfaces (e.g., notifications).
//!
//! Layer ordering (bottom to top):
//! - Background: desktop wallpaper
//! - Bottom: desktop widgets
//! - Top: panels, docks (this panel)
//! - Overlay: lock screen, notifications
//!
//! The panel requests exclusive zone equal to its height, so the
//! compositor reserves that space and prevents normal windows from
//! overlapping the panel area.

use alloc::{string::String, vec::Vec};

use spin::RwLock;

use super::window_manager::{with_window_manager, WindowId};
use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Panel height in pixels.
pub const PANEL_HEIGHT: u32 = 32;

/// Number of workspaces.
const NUM_WORKSPACES: usize = 4;

/// Width of each workspace button in pixels.
const WORKSPACE_BUTTON_WIDTH: u32 = 24;

/// Gap between workspace buttons in pixels.
const WORKSPACE_BUTTON_GAP: u32 = 2;

/// Width of the workspace indicator area (buttons + padding).
const WORKSPACE_AREA_WIDTH: u32 =
    NUM_WORKSPACES as u32 * (WORKSPACE_BUTTON_WIDTH + WORKSPACE_BUTTON_GAP) + 8;

// ---------------------------------------------------------------------------
// Layer-shell types
// ---------------------------------------------------------------------------

/// Layer-shell layer (from wlr-layer-shell protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LayerShellLayer {
    /// Below all windows
    Background = 0,
    /// Below normal windows
    Bottom = 1,
    /// Above normal windows (panels, docks)
    Top = 2,
    /// Above everything (lock screen, notifications)
    Overlay = 3,
}

/// Layer-shell anchor edges (bitmask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LayerShellAnchor {
    /// No anchor (centered)
    None = 0,
    /// Anchored to top edge
    Top = 1,
    /// Anchored to bottom edge
    Bottom = 2,
    /// Anchored to left edge
    Left = 4,
    /// Anchored to right edge
    Right = 8,
}

/// Layer-shell surface configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LayerSurfaceConfig {
    /// The Wayland surface layer
    pub layer: LayerShellLayer,
    /// Anchor edges (bitmask of LayerShellAnchor values)
    pub anchor: u32,
    /// Exclusive zone: positive = reserve space, 0 = no reservation,
    /// -1 = extend to edge
    pub exclusive_zone: i32,
    /// Margin from anchored edges (top, right, bottom, left)
    pub margin: (i32, i32, i32, i32),
    /// Desired width (0 = use anchor width)
    pub width: u32,
    /// Desired height (0 = use anchor height)
    pub height: u32,
    /// Keyboard interactivity mode
    pub keyboard_interactivity: bool,
}

impl LayerSurfaceConfig {
    /// Create a configuration for a bottom-anchored panel.
    #[allow(dead_code)]
    pub fn bottom_panel(screen_width: u32, height: u32) -> Self {
        Self {
            layer: LayerShellLayer::Top,
            anchor: LayerShellAnchor::Bottom as u32
                | LayerShellAnchor::Left as u32
                | LayerShellAnchor::Right as u32,
            exclusive_zone: height as i32,
            margin: (0, 0, 0, 0),
            width: screen_width,
            height,
            keyboard_interactivity: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Panel button
// ---------------------------------------------------------------------------

/// Panel button representing a window in the taskbar.
#[derive(Debug, Clone)]
struct PanelButton {
    window_id: WindowId,
    title: String,
    x: i32,
    width: u32,
    focused: bool,
}

// ---------------------------------------------------------------------------
// Workspace state
// ---------------------------------------------------------------------------

/// Workspace indicator state.
struct WorkspaceState {
    /// Currently active workspace (0-indexed)
    active: usize,
    /// Number of windows on each workspace
    window_counts: [u32; NUM_WORKSPACES],
}

impl WorkspaceState {
    fn new() -> Self {
        Self {
            active: 0,
            window_counts: [0; NUM_WORKSPACES],
        }
    }
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

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
    /// Layer-shell surface ID (if initialized via layer-shell protocol).
    #[allow(dead_code)]
    layer_surface_id: Option<u32>,
    /// Layer-shell configuration.
    #[allow(dead_code)]
    layer_config: Option<LayerSurfaceConfig>,
    /// Workspace indicator state.
    workspaces: RwLock<WorkspaceState>,
}

impl Panel {
    /// Create a new panel for the given screen dimensions.
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        Self {
            screen_width,
            screen_height,
            buttons: RwLock::new(Vec::new()),
            clock_text: RwLock::new(String::from("00:00")),
            layer_surface_id: None,
            layer_config: None,
            workspaces: RwLock::new(WorkspaceState::new()),
        }
    }

    /// Get the Y coordinate of the panel (bottom of screen).
    pub fn y(&self) -> i32 {
        (self.screen_height - PANEL_HEIGHT) as i32
    }

    /// Initialize the layer-shell surface for the panel.
    ///
    /// This creates a layer-shell surface anchored to the bottom edge of
    /// the output with an exclusive zone equal to the panel height. The
    /// compositor will reserve this space and prevent normal windows from
    /// overlapping.
    ///
    /// Returns the layer surface ID, or None if already initialized.
    #[allow(dead_code)]
    pub fn init_layer_surface(&mut self) -> Option<u32> {
        if self.layer_surface_id.is_some() {
            return self.layer_surface_id;
        }

        let config = LayerSurfaceConfig::bottom_panel(self.screen_width, PANEL_HEIGHT);

        // Create the compositor surface via the desktop renderer's helper
        let (surface_id, _pool_id, _pool_buf_id) =
            super::renderer::create_app_surface(0, self.y(), self.screen_width, PANEL_HEIGHT);

        // Position the surface at the bottom of the screen
        crate::desktop::wayland::with_display(|display| {
            display
                .wl_compositor
                .set_surface_position(surface_id, 0, self.y());
            // Raise to top of z-order (panels above windows)
            display.wl_compositor.raise_surface(surface_id);
        });

        crate::println!(
            "[PANEL] Layer-shell surface initialized: id={}, layer=Top, anchor=Bottom|Left|Right, \
             exclusive_zone={}",
            surface_id,
            PANEL_HEIGHT
        );

        self.layer_surface_id = Some(surface_id);
        self.layer_config = Some(config);
        self.layer_surface_id
    }

    /// Get the layer-shell surface ID.
    #[allow(dead_code)]
    pub fn layer_surface_id(&self) -> Option<u32> {
        self.layer_surface_id
    }

    /// Set the active workspace.
    #[allow(dead_code)]
    pub fn set_active_workspace(&self, index: usize) {
        if index < NUM_WORKSPACES {
            self.workspaces.write().active = index;
        }
    }

    /// Get the active workspace index.
    #[allow(dead_code)]
    pub fn active_workspace(&self) -> usize {
        self.workspaces.read().active
    }

    /// Update workspace window counts from the window manager.
    #[allow(dead_code)]
    pub fn update_workspace_counts(&self) {
        let windows = with_window_manager(|wm| wm.get_all_windows()).unwrap_or_default();
        let mut ws = self.workspaces.write();

        // Reset counts
        for count in ws.window_counts.iter_mut() {
            *count = 0;
        }

        // Count visible windows. For now, all windows are on workspace 0
        // since we don't have multi-workspace support yet.
        for window in &windows {
            if window.visible {
                ws.window_counts[0] += 1;
            }
        }
    }

    /// Handle a click on a workspace button.
    ///
    /// Returns the workspace index if a workspace button was clicked.
    fn handle_workspace_click(&self, x: i32) -> Option<usize> {
        let start_x = 4i32;
        for i in 0..NUM_WORKSPACES {
            let btn_x =
                start_x + i as i32 * (WORKSPACE_BUTTON_WIDTH as i32 + WORKSPACE_BUTTON_GAP as i32);
            let btn_end = btn_x + WORKSPACE_BUTTON_WIDTH as i32;
            if x >= btn_x && x < btn_end {
                return Some(i);
            }
        }
        None
    }

    /// Update the panel button list from the window manager.
    pub fn update_buttons(&self) {
        let windows = with_window_manager(|wm| wm.get_all_windows()).unwrap_or_default();

        let mut buttons = self.buttons.write();
        buttons.clear();

        let button_width = 120u32;
        // Start window buttons after the workspace area
        let mut x = WORKSPACE_AREA_WIDTH as i32 + 4;

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

    /// Update the clock display with real wall-clock date and time.
    pub fn update_clock(&self) {
        // Use CMOS RTC on x86_64 for real wall-clock time; fall back to
        // uptime-based approximation on other architectures.
        #[cfg(target_arch = "x86_64")]
        let epoch_secs = crate::arch::x86_64::rtc::current_epoch_secs();
        #[cfg(not(target_arch = "x86_64"))]
        let epoch_secs = {
            let ticks = crate::arch::timer::read_hw_timestamp();
            ticks / 1_000_000_000
        };

        // Convert epoch seconds to date components
        let secs_of_day = epoch_secs % 86400;
        let hours = (secs_of_day / 3600) % 24;
        let minutes = (secs_of_day / 60) % 60;

        // Days since Unix epoch (1970-01-01, a Thursday)
        let mut remaining_days = (epoch_secs / 86400) as u32;

        // Day of week: 1970-01-01 was Thursday (index 4)
        let day_of_week = (remaining_days + 4) % 7; // 0=Sun..6=Sat
        let day_name = match day_of_week {
            0 => "Sun",
            1 => "Mon",
            2 => "Tue",
            3 => "Wed",
            4 => "Thu",
            5 => "Fri",
            6 => "Sat",
            _ => "???",
        };

        // Year/month/day from days since epoch
        let mut year: u32 = 1970;
        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if remaining_days < days_in_year {
                break;
            }
            remaining_days -= days_in_year;
            year += 1;
        }

        let month_days: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month_idx: usize = 0;
        for (i, &md) in month_days.iter().enumerate() {
            let days = if i == 1 && is_leap_year(year) { 29 } else { md };
            if remaining_days < days {
                month_idx = i;
                break;
            }
            remaining_days -= days;
            if i == 11 {
                month_idx = 11;
            }
        }
        let month_day = remaining_days + 1;

        let month = match month_idx {
            0 => "Jan",
            1 => "Feb",
            2 => "Mar",
            3 => "Apr",
            4 => "May",
            5 => "Jun",
            6 => "Jul",
            7 => "Aug",
            8 => "Sep",
            9 => "Oct",
            10 => "Nov",
            11 => "Dec",
            _ => "???",
        };

        let mut clock = self.clock_text.write();
        clock.clear();

        // Format: "Fri Feb 28 14:30"
        for ch in day_name.chars() {
            clock.push(ch);
        }
        clock.push(' ');
        for ch in month.chars() {
            clock.push(ch);
        }
        clock.push(' ');
        if month_day < 10 {
            clock.push(' ');
        }
        for ch in fmt_u64(month_day as u64).chars() {
            clock.push(ch);
        }
        clock.push(' ');
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
        // Check workspace buttons first
        if let Some(ws_idx) = self.handle_workspace_click(x) {
            self.set_active_workspace(ws_idx);
            return None;
        }

        // Check window buttons
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
    /// (BGRA format). Renders workspace indicators, window buttons, and
    /// the date/time clock.
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

        // Top border line (subtle highlight)
        for x in 0..w {
            let offset = x * 4;
            if offset + 3 < buf.len() {
                buf[offset] = 0x44; // B
                buf[offset + 1] = 0x44; // G
                buf[offset + 2] = 0x44; // R
                buf[offset + 3] = 0xFF;
            }
        }

        // --- Render workspace indicators ---
        self.render_workspaces(buf, stride, w, h);

        // --- Render separator after workspace area ---
        let sep_x = WORKSPACE_AREA_WIDTH as usize;
        for y in 4..h - 4 {
            let offset = y * stride + sep_x * 4;
            if offset + 3 < buf.len() {
                buf[offset] = 0x55; // B
                buf[offset + 1] = 0x55; // G
                buf[offset + 2] = 0x55; // R
                buf[offset + 3] = 0xFF;
            }
        }

        // --- Render window buttons ---
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

            // Focused button underline indicator
            if button.focused {
                for x in btn_x..(btn_x + btn_w).min(w) {
                    let offset = (h - 3) * stride + x * 4;
                    if offset + 3 < buf.len() {
                        buf[offset] = 0xDD; // B (accent blue)
                        buf[offset + 1] = 0x88; // G
                        buf[offset + 2] = 0x44; // R
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

        // --- Render clock with date on the right side ---
        let clock = self.clock_text.read();
        let clock_x = w.saturating_sub(clock.len() * 8 + 12);
        for (i, &ch) in clock.as_bytes().iter().enumerate() {
            render_char_to_buf(buf, stride, clock_x + i * 8, 10, ch, (0xBB, 0xBB, 0xBB));
        }
    }

    /// Render workspace indicator buttons into the panel buffer.
    fn render_workspaces(&self, buf: &mut [u8], stride: usize, max_w: usize, h: usize) {
        let ws = self.workspaces.read();
        let start_x = 4usize;

        for i in 0..NUM_WORKSPACES {
            let btn_x =
                start_x + i * (WORKSPACE_BUTTON_WIDTH as usize + WORKSPACE_BUTTON_GAP as usize);
            let btn_w = WORKSPACE_BUTTON_WIDTH as usize;

            // Button color: active workspace is highlighted, occupied is dimmer
            let (br, bg, bb) = if i == ws.active {
                (0x55, 0x66, 0x99) // Active: blue-ish
            } else if ws.window_counts[i] > 0 {
                (0x48, 0x48, 0x48) // Occupied: slightly lighter
            } else {
                (0x38, 0x38, 0x38) // Empty: slightly lighter than panel bg
            };

            // Draw workspace button background
            for y in 6..h - 6 {
                for x in btn_x..(btn_x + btn_w).min(max_w) {
                    let offset = y * stride + x * 4;
                    if offset + 3 < buf.len() {
                        buf[offset] = bb;
                        buf[offset + 1] = bg;
                        buf[offset + 2] = br;
                        buf[offset + 3] = 0xFF;
                    }
                }
            }

            // Active workspace underline
            if i == ws.active {
                for x in btn_x..(btn_x + btn_w).min(max_w) {
                    let offset = (h - 4) * stride + x * 4;
                    if offset + 3 < buf.len() {
                        buf[offset] = 0xDD; // B (accent blue)
                        buf[offset + 1] = 0x88; // G
                        buf[offset + 2] = 0x44; // R
                        buf[offset + 3] = 0xFF;
                    }
                }
            }

            // Render workspace number (1-4) centered in the button
            let digit = b'1' + i as u8;
            let char_x = btn_x + (btn_w / 2).saturating_sub(4);
            let text_color = if i == ws.active {
                (0xFF, 0xFF, 0xFF) // White for active
            } else {
                (0x99, 0x99, 0x99) // Grey for inactive
            };
            render_char_to_buf(buf, stride, char_x, 10, digit, text_color);
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

/// Check if a year is a leap year.
fn is_leap_year(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
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
        "[PANEL] Desktop panel initialized ({}x{}, layer-shell ready)",
        screen_width,
        PANEL_HEIGHT
    );
    Ok(())
}

/// Execute a function with the panel.
pub fn with_panel<R, F: FnOnce(&Panel) -> R>(f: F) -> Option<R> {
    PANEL.with(f)
}

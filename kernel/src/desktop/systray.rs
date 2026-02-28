//! System Tray
//!
//! Provides a system tray area in the desktop panel for status indicators
//! such as clock, CPU usage, memory usage, network status, and battery.
//! Items are rendered right-aligned in the panel and can be clicked.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use crate::sync::once_lock::GlobalState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Font dimensions (8x16 bitmap font).
const CHAR_W: usize = 8;
const CHAR_H: usize = 16;

/// Default tray item padding (pixels between items).
const TRAY_PADDING: usize = 4;

/// Default tray height (matches panel inner area).
const TRAY_HEIGHT: usize = 24;

/// Color for normal tray text (light gray).
const COLOR_NORMAL: u32 = 0xFFBBBBBB;

/// Color for CPU usage below 50% (green).
const COLOR_CPU_LOW: u32 = 0xFF44CC44;

/// Color for CPU usage 50-79% (yellow).
const COLOR_CPU_MED: u32 = 0xFFCCCC44;

/// Color for CPU usage >= 80% (red).
const COLOR_CPU_HIGH: u32 = 0xFFCC4444;

/// Separator color (dim).
const COLOR_SEPARATOR: u32 = 0xFF555555;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Type of system tray item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysTrayItemType {
    /// System clock showing time and date.
    Clock,
    /// CPU usage monitor.
    CpuMonitor,
    /// Memory usage monitor.
    MemoryMonitor,
    /// Network connection status.
    NetworkStatus,
    /// Battery charge level.
    BatteryStatus,
    /// Audio volume level.
    Volume,
    /// Custom/user-defined tray item.
    Custom,
}

/// A single item in the system tray.
#[derive(Debug, Clone)]
pub struct SysTrayItem {
    /// What kind of tray item this is.
    pub item_type: SysTrayItemType,
    /// Display label (rendered text).
    pub label: String,
    /// Tooltip text (for future hover support).
    pub tooltip: String,
    /// Whether this item is currently visible.
    pub visible: bool,
    /// Width of this item in pixels (based on label length).
    pub width: usize,
}

impl SysTrayItem {
    /// Create a new tray item with auto-calculated width.
    pub fn new(item_type: SysTrayItemType, label: &str, tooltip: &str) -> Self {
        let width = label.len() * CHAR_W + TRAY_PADDING * 2;
        Self {
            item_type,
            label: String::from(label),
            tooltip: String::from(tooltip),
            visible: true,
            width,
        }
    }

    /// Update the label and recalculate width.
    fn set_label(&mut self, new_label: &str) {
        self.label.clear();
        self.label.push_str(new_label);
        self.width = self.label.len() * CHAR_W + TRAY_PADDING * 2;
    }
}

/// System tray managing a collection of status indicator items.
pub struct SystemTray {
    /// Ordered list of tray items (rendered left-to-right within the tray
    /// area).
    items: Vec<SysTrayItem>,
    /// Height of the tray area in pixels.
    tray_height: usize,
    /// Padding between items.
    padding: usize,
}

impl SystemTray {
    /// Create a new system tray with default status items.
    pub fn new() -> Self {
        let mut tray = Self {
            items: Vec::new(),
            tray_height: TRAY_HEIGHT,
            padding: TRAY_PADDING,
        };

        // Default items: CPU, Memory, Clock (left-to-right order)
        tray.items.push(SysTrayItem::new(
            SysTrayItemType::CpuMonitor,
            "CPU: 0%",
            "CPU usage",
        ));
        tray.items.push(SysTrayItem::new(
            SysTrayItemType::MemoryMonitor,
            "MEM: 0/0 MB",
            "Memory usage",
        ));
        tray.items.push(SysTrayItem::new(
            SysTrayItemType::Clock,
            "00:00 Jan 01",
            "System clock",
        ));

        tray
    }

    /// Add a new item to the tray.
    pub fn add_item(&mut self, item: SysTrayItem) {
        // Avoid duplicate types (except Custom)
        if item.item_type != SysTrayItemType::Custom
            && self.items.iter().any(|i| i.item_type == item.item_type)
        {
            return;
        }
        self.items.push(item);
    }

    /// Remove the first item matching the given type.
    pub fn remove_item(&mut self, item_type: SysTrayItemType) {
        if let Some(pos) = self.items.iter().position(|i| i.item_type == item_type) {
            self.items.remove(pos);
        }
    }

    /// Update the clock display with the given time and date.
    pub fn update_clock(&mut self, hours: u8, minutes: u8, _seconds: u8, month: u8, day: u8) {
        let label = format_clock(hours, minutes, month, day);
        for item in &mut self.items {
            if item.item_type == SysTrayItemType::Clock {
                item.set_label(&label);
                return;
            }
        }
    }

    /// Update the CPU usage display.
    pub fn update_cpu(&mut self, usage_percent: u8) {
        let label = format_cpu(usage_percent);
        for item in &mut self.items {
            if item.item_type == SysTrayItemType::CpuMonitor {
                item.set_label(&label);
                return;
            }
        }
    }

    /// Update the memory usage display.
    pub fn update_memory(&mut self, used_mb: u32, total_mb: u32) {
        let label = format_memory(used_mb, total_mb);
        for item in &mut self.items {
            if item.item_type == SysTrayItemType::MemoryMonitor {
                item.set_label(&label);
                return;
            }
        }
    }

    /// Total pixel width of all visible tray items plus inter-item padding.
    pub fn total_width(&self) -> usize {
        let visible: Vec<&SysTrayItem> = self.items.iter().filter(|i| i.visible).collect();
        if visible.is_empty() {
            return 0;
        }
        let items_width: usize = visible.iter().map(|i| i.width).sum();
        let separators = if visible.len() > 1 {
            (visible.len() - 1) * (self.padding + 1 + self.padding) // pad + 1px
                                                                    // sep + pad
        } else {
            0
        };
        items_width + separators
    }

    /// Render the system tray into a u32 (BGRA) pixel buffer.
    ///
    /// Items are drawn starting at `(x_start, y_start)` and proceed
    /// left-to-right. Separators are drawn between items.
    pub fn render_to_buffer(
        &self,
        buffer: &mut [u32],
        buf_width: usize,
        x_start: usize,
        y_start: usize,
    ) {
        let mut cx = x_start;

        for (idx, item) in self.items.iter().filter(|i| i.visible).enumerate() {
            // Draw separator before each item except the first
            if idx > 0 {
                cx += self.padding;
                // 1-pixel vertical separator line
                let sep_top = y_start + 2;
                let sep_bot = y_start + self.tray_height.saturating_sub(2);
                for sy in sep_top..sep_bot {
                    let offset = sy * buf_width + cx;
                    if offset < buffer.len() {
                        buffer[offset] = COLOR_SEPARATOR;
                    }
                }
                cx += 1 + self.padding;
            }

            // Determine text color based on item type and content
            let color = item_color(item);

            // Center text vertically within tray height
            let text_y = y_start + (self.tray_height.saturating_sub(CHAR_H)) / 2;

            // Render label characters
            let text_x = cx + self.padding;
            for (ci, &ch) in item.label.as_bytes().iter().enumerate() {
                render_glyph_u32(buffer, buf_width, text_x + ci * CHAR_W, text_y, ch, color);
            }

            cx += item.width;
        }
    }

    /// Determine which tray item (if any) was clicked at position `(x, y)`.
    ///
    /// Coordinates are relative to the tray area origin (the same coordinate
    /// system used for `render_to_buffer`). Returns `None` if the click is
    /// outside any item.
    pub fn handle_click(&self, x: usize, _y: usize) -> Option<SysTrayItemType> {
        let mut cx: usize = 0;

        for (idx, item) in self.items.iter().filter(|i| i.visible).enumerate() {
            if idx > 0 {
                cx += self.padding + 1 + self.padding; // separator
            }

            if x >= cx && x < cx + item.width {
                return Some(item.item_type);
            }
            cx += item.width;
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers (no floating-point, no format! macro for small strings)
// ---------------------------------------------------------------------------

/// Format clock label: "HH:MM Mon DD"
fn format_clock(hours: u8, minutes: u8, month: u8, day: u8) -> String {
    let mut s = String::with_capacity(14);
    push_2digit(&mut s, hours);
    s.push(':');
    push_2digit(&mut s, minutes);
    s.push(' ');
    s.push_str(month_abbr(month));
    s.push(' ');
    push_2digit(&mut s, day);
    s
}

/// Format CPU label: "CPU: XX%"
fn format_cpu(usage: u8) -> String {
    let mut s = String::with_capacity(10);
    s.push_str("CPU:");
    push_u8_decimal(&mut s, usage);
    s.push('%');
    s
}

/// Format memory label: "MEM: XXX/YYY MB"
fn format_memory(used_mb: u32, total_mb: u32) -> String {
    let mut s = String::with_capacity(18);
    s.push_str("MEM:");
    push_u32_decimal(&mut s, used_mb);
    s.push('/');
    push_u32_decimal(&mut s, total_mb);
    s.push_str("MB");
    s
}

/// Push a zero-padded 2-digit number (0-99).
fn push_2digit(s: &mut String, v: u8) {
    let v = v % 100;
    s.push((b'0' + v / 10) as char);
    s.push((b'0' + v % 10) as char);
}

/// Push a u8 as a decimal string (no leading zeros).
fn push_u8_decimal(s: &mut String, v: u8) {
    if v >= 100 {
        s.push((b'0' + v / 100) as char);
    }
    if v >= 10 {
        s.push((b'0' + (v / 10) % 10) as char);
    }
    s.push((b'0' + v % 10) as char);
}

/// Push a u32 as a decimal string (no leading zeros).
fn push_u32_decimal(s: &mut String, v: u32) {
    if v == 0 {
        s.push('0');
        return;
    }
    // Find highest power of 10
    let mut div = 1u32;
    let mut tmp = v;
    while tmp >= 10 {
        tmp /= 10;
        div *= 10;
    }
    // Emit digits
    let mut remaining = v;
    while div > 0 {
        let digit = remaining / div;
        s.push((b'0' + digit as u8) as char);
        remaining %= div;
        div /= 10;
    }
}

/// 3-letter month abbreviation (1-indexed).
fn month_abbr(month: u8) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}

/// Determine the display color for a tray item based on its type and content.
fn item_color(item: &SysTrayItem) -> u32 {
    match item.item_type {
        SysTrayItemType::CpuMonitor => {
            // Parse usage from label "CPU:XX%"
            let usage = parse_cpu_usage(&item.label);
            if usage >= 80 {
                COLOR_CPU_HIGH
            } else if usage >= 50 {
                COLOR_CPU_MED
            } else {
                COLOR_CPU_LOW
            }
        }
        _ => COLOR_NORMAL,
    }
}

/// Extract the numeric CPU usage from a label like "CPU:42%".
fn parse_cpu_usage(label: &str) -> u8 {
    // Find digits between ':' and '%'
    let bytes = label.as_bytes();
    let mut start = 0;
    let mut end = bytes.len();

    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' {
            start = i + 1;
        } else if b == b'%' {
            end = i;
            break;
        }
    }

    let mut result: u8 = 0;
    for &b in &bytes[start..end] {
        if b.is_ascii_digit() {
            result = result.saturating_mul(10).saturating_add(b - b'0');
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Glyph rendering helper (u32 pixel buffer)
// ---------------------------------------------------------------------------

/// Render a single 8x16 glyph into a u32 (BGRA packed) pixel buffer.
fn render_glyph_u32(buf: &mut [u32], buf_width: usize, px: usize, py: usize, ch: u8, color: u32) {
    use crate::graphics::font8x16;

    let glyph = font8x16::glyph(ch);
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 != 0 {
                let x = px + col;
                let y = py + row;
                let offset = y * buf_width + x;
                if offset < buf.len() {
                    buf[offset] = color;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Global instance
// ---------------------------------------------------------------------------

static SYSTEM_TRAY: GlobalState<spin::Mutex<SystemTray>> = GlobalState::new();

/// Initialize the global system tray.
pub fn init() {
    let _ = SYSTEM_TRAY.init(spin::Mutex::new(SystemTray::new()));
    crate::println!("[SYSTRAY] System tray initialized (3 default items)");
}

/// Execute a closure with a mutable reference to the system tray.
pub fn with_system_tray<R, F: FnOnce(&mut SystemTray) -> R>(f: F) -> Option<R> {
    SYSTEM_TRAY.with(|lock| {
        let mut tray = lock.lock();
        f(&mut tray)
    })
}

/// Execute a closure with a shared reference to the system tray.
pub fn with_system_tray_ref<R, F: FnOnce(&SystemTray) -> R>(f: F) -> Option<R> {
    SYSTEM_TRAY.with(|lock| {
        let tray = lock.lock();
        f(&tray)
    })
}

impl Default for SystemTray {
    fn default() -> Self {
        Self::new()
    }
}

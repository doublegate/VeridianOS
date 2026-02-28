//! Desktop Notification System
//!
//! Provides toast-style notification popups displayed as overlay surfaces.
//! Notifications stack from the top-right corner of the screen and auto-expire
//! based on urgency level. Supports programmatic dismiss and tick-based expiry.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use spin::Mutex;

use crate::sync::once_lock::GlobalState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default expiry in ticks for Low urgency notifications (~3 seconds at
/// 1000Hz).
const EXPIRE_TICKS_LOW: u64 = 3_000;

/// Default expiry in ticks for Normal urgency notifications (~5 seconds).
const EXPIRE_TICKS_NORMAL: u64 = 5_000;

/// Default expiry in ticks for Critical urgency notifications (~10 seconds).
const EXPIRE_TICKS_CRITICAL: u64 = 10_000;

/// Toast background color (dark semi-transparent: 0xE0303030 ARGB -> BGRA u32).
const TOAST_BG_COLOR: u32 = 0xE0303030;

/// Toast border color for Normal urgency.
const TOAST_BORDER_NORMAL: u32 = 0xFF5588CC;

/// Toast border color for Critical urgency (red accent).
const TOAST_BORDER_CRITICAL: u32 = 0xFFCC4444;

/// Toast border color for Low urgency (subtle gray).
const TOAST_BORDER_LOW: u32 = 0xFF606060;

/// Summary text color (white).
const SUMMARY_COLOR: u32 = 0xFFEEEEEE;

/// Body text color (light gray).
const BODY_COLOR: u32 = 0xFFAAAAAA;

/// App name text color (dim).
const APP_NAME_COLOR: u32 = 0xFF777777;

/// Font dimensions (8x16 bitmap font).
const CHAR_W: usize = 8;
const CHAR_H: usize = 16;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Urgency level for a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationUrgency {
    /// Low priority -- short display time, subtle styling.
    Low,
    /// Normal priority -- standard display time.
    Normal,
    /// Critical priority -- longer display time, red accent.
    Critical,
}

impl NotificationUrgency {
    /// Convert a raw u8 to an urgency level.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Low,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }

    /// Default expiry ticks for this urgency level.
    fn default_expire_ticks(self) -> u64 {
        match self {
            Self::Low => EXPIRE_TICKS_LOW,
            Self::Normal => EXPIRE_TICKS_NORMAL,
            Self::Critical => EXPIRE_TICKS_CRITICAL,
        }
    }

    /// Border color for this urgency level.
    fn border_color(self) -> u32 {
        match self {
            Self::Low => TOAST_BORDER_LOW,
            Self::Normal => TOAST_BORDER_NORMAL,
            Self::Critical => TOAST_BORDER_CRITICAL,
        }
    }
}

/// A single desktop notification.
#[derive(Debug, Clone)]
pub struct Notification {
    /// Unique notification ID.
    pub id: u32,
    /// Short summary / title line.
    pub summary: String,
    /// Longer body text (may be empty).
    pub body: String,
    /// Urgency level.
    pub urgency: NotificationUrgency,
    /// Name of the application that sent this notification.
    pub app_name: String,
    /// Tick count when the notification was created.
    pub created_tick: u64,
    /// Number of ticks after creation when the notification expires.
    pub expire_ticks: u64,
    /// Whether the notification has been dismissed by the user.
    pub dismissed: bool,
}

impl Notification {
    /// Returns `true` if the notification has expired at `current_tick`.
    pub fn is_expired(&self, current_tick: u64) -> bool {
        current_tick >= self.created_tick.saturating_add(self.expire_ticks)
    }
}

/// Manages the set of active desktop notifications.
pub struct NotificationManager {
    /// All tracked notifications (active + recently expired).
    notifications: Vec<Notification>,
    /// Next notification ID to assign.
    next_id: u32,
    /// Maximum number of toasts visible simultaneously.
    max_visible: usize,
    /// Width of each toast popup in pixels.
    toast_width: usize,
    /// Height of each toast popup in pixels.
    toast_height: usize,
    /// Vertical margin between stacked toasts.
    toast_margin: usize,
    /// X position (left edge) for the toast column (top-right aligned).
    position_x: usize,
    /// Y position (top edge) for the first toast.
    position_y: usize,
}

impl NotificationManager {
    /// Create a new notification manager positioned in the top-right corner.
    pub fn new(screen_width: usize, screen_height: usize) -> Self {
        let toast_width = 300;
        let toast_margin = 10;
        let _ = screen_height; // available for future use
        Self {
            notifications: Vec::new(),
            next_id: 1,
            max_visible: 3,
            toast_width,
            toast_height: 80,
            toast_margin,
            position_x: screen_width.saturating_sub(toast_width + toast_margin),
            position_y: toast_margin,
        }
    }

    /// Post a new notification. Returns the assigned notification ID.
    pub fn notify(
        &mut self,
        summary: String,
        body: String,
        urgency: NotificationUrgency,
        app_name: String,
    ) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let current_tick = read_tick();
        let expire_ticks = urgency.default_expire_ticks();

        self.notifications.push(Notification {
            id,
            summary,
            body,
            urgency,
            app_name,
            created_tick: current_tick,
            expire_ticks,
            dismissed: false,
        });

        id
    }

    /// Dismiss a specific notification by ID.
    pub fn dismiss(&mut self, id: u32) {
        for n in &mut self.notifications {
            if n.id == id {
                n.dismissed = true;
                return;
            }
        }
    }

    /// Dismiss all active notifications.
    pub fn dismiss_all(&mut self) {
        for n in &mut self.notifications {
            n.dismissed = true;
        }
    }

    /// Tick the notification manager: remove expired and dismissed entries.
    pub fn tick(&mut self, current_tick: u64) {
        self.notifications
            .retain(|n| !n.dismissed && !n.is_expired(current_tick));
    }

    /// Return references to the currently visible (non-dismissed, non-expired)
    /// notifications, up to `max_visible`.
    pub fn visible_notifications(&self) -> Vec<&Notification> {
        let current_tick = read_tick();
        self.notifications
            .iter()
            .filter(|n| !n.dismissed && !n.is_expired(current_tick))
            .take(self.max_visible)
            .collect()
    }

    /// Return total count of active (non-dismissed, non-expired) notifications.
    pub fn active_count(&self) -> usize {
        let current_tick = read_tick();
        self.notifications
            .iter()
            .filter(|n| !n.dismissed && !n.is_expired(current_tick))
            .count()
    }

    /// Render all visible toast notifications into a u32 pixel buffer.
    ///
    /// The buffer is `buf_width * buf_height` pixels in BGRA format (one u32
    /// per pixel). Toasts are rendered from the top-right corner, stacking
    /// downward. Only pixels belonging to toast rectangles are written; the
    /// caller is responsible for compositing this buffer as an overlay.
    pub fn render_to_buffer(
        &self,
        buffer: &mut [u32],
        buf_width: usize,
        buf_height: usize,
        current_tick: u64,
    ) {
        let visible = self.visible_notifications_at(current_tick);
        if visible.is_empty() {
            return;
        }

        let tw = self.toast_width;
        let th = self.toast_height;

        for (idx, notif) in visible.iter().enumerate() {
            let tx = self.position_x;
            let ty = self.position_y + idx * (th + self.toast_margin);

            // Skip if toast would be off-screen
            if ty + th > buf_height || tx + tw > buf_width {
                continue;
            }

            let border_color = notif.urgency.border_color();

            // Draw toast rectangle (background + 1px border)
            for row in 0..th {
                for col in 0..tw {
                    let px = tx + col;
                    let py = ty + row;
                    let offset = py * buf_width + px;
                    if offset >= buffer.len() {
                        continue;
                    }

                    // 1-pixel border
                    if row == 0 || row == th - 1 || col == 0 || col == tw - 1 {
                        buffer[offset] = border_color;
                    } else {
                        buffer[offset] = TOAST_BG_COLOR;
                    }
                }
            }

            // Render app name (top-left, small and dim)
            let app_bytes = notif.app_name.as_bytes();
            let app_max = (tw - 16) / CHAR_W;
            let app_y = ty + 4;
            for (i, &ch) in app_bytes.iter().take(app_max).enumerate() {
                render_glyph_u32(
                    buffer,
                    buf_width,
                    tx + 8 + i * CHAR_W,
                    app_y,
                    ch,
                    APP_NAME_COLOR,
                );
            }

            // Render summary (bold-ish white, below app name)
            let sum_bytes = notif.summary.as_bytes();
            let sum_max = (tw - 16) / CHAR_W;
            let sum_y = ty + 4 + CHAR_H + 2;
            for (i, &ch) in sum_bytes.iter().take(sum_max).enumerate() {
                render_glyph_u32(
                    buffer,
                    buf_width,
                    tx + 8 + i * CHAR_W,
                    sum_y,
                    ch,
                    SUMMARY_COLOR,
                );
            }

            // Render body (gray, below summary, may truncate)
            let body_bytes = notif.body.as_bytes();
            let body_max = (tw - 16) / CHAR_W;
            let body_y = ty + 4 + (CHAR_H + 2) * 2;
            if body_y + CHAR_H <= ty + th {
                for (i, &ch) in body_bytes.iter().take(body_max).enumerate() {
                    render_glyph_u32(
                        buffer,
                        buf_width,
                        tx + 8 + i * CHAR_W,
                        body_y,
                        ch,
                        BODY_COLOR,
                    );
                }
            }
        }
    }

    /// Internal: get visible notifications at a specific tick (avoids
    /// calling `read_tick()` again when the caller already has it).
    fn visible_notifications_at(&self, current_tick: u64) -> Vec<&Notification> {
        self.notifications
            .iter()
            .filter(|n| !n.dismissed && !n.is_expired(current_tick))
            .take(self.max_visible)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Glyph rendering helper (u32 pixel buffer)
// ---------------------------------------------------------------------------

/// Render a single 8x16 glyph into a u32 (BGRA packed) pixel buffer.
///
/// Only foreground pixels are written; background pixels are left untouched
/// so the toast background shows through.
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
// Tick source helper
// ---------------------------------------------------------------------------

/// Read the current hardware tick counter (architecture-independent).
fn read_tick() -> u64 {
    crate::arch::timer::read_hw_timestamp() / 1_000_000 // approximate ms-scale
}

// ---------------------------------------------------------------------------
// Global instance
// ---------------------------------------------------------------------------

static NOTIFICATION_MANAGER: GlobalState<Mutex<NotificationManager>> = GlobalState::new();

/// Initialize the global notification manager.
pub fn init(screen_width: usize, screen_height: usize) {
    let _ = NOTIFICATION_MANAGER.init(Mutex::new(NotificationManager::new(
        screen_width,
        screen_height,
    )));
    crate::println!(
        "[NOTIFY] Notification manager initialized (toast area {}x{})",
        300,
        screen_height
    );
}

/// Execute a closure with a mutable reference to the notification manager.
pub fn with_notification_manager<R, F: FnOnce(&mut NotificationManager) -> R>(f: F) -> Option<R> {
    NOTIFICATION_MANAGER.with(|lock| {
        let mut mgr = lock.lock();
        f(&mut mgr)
    })
}

/// Convenience: post a notification from anywhere in the kernel.
pub fn notify(summary: &str, body: &str, urgency: NotificationUrgency, app_name: &str) -> u32 {
    with_notification_manager(|mgr| {
        mgr.notify(
            String::from(summary),
            String::from(body),
            urgency,
            String::from(app_name),
        )
    })
    .unwrap_or(0)
}

/// Convenience: dismiss a notification by ID.
pub fn dismiss(id: u32) {
    with_notification_manager(|mgr| mgr.dismiss(id));
}

/// Convenience: dismiss all notifications.
pub fn dismiss_all() {
    with_notification_manager(|mgr| mgr.dismiss_all());
}

/// Convenience: tick the notification manager (call from render loop).
pub fn tick() {
    let current = read_tick();
    with_notification_manager(|mgr| mgr.tick(current));
}

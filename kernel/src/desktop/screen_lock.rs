//! Screen Lock
//!
//! Provides a fullscreen lock screen with password authentication.
//! Triggered via Ctrl+Alt+L or idle timeout. Renders a dark gradient
//! background with a padlock icon, username, and dot-masked password
//! field. Integrates with the security/auth module for credential
//! verification.
//!
//! All rendering uses integer math only (no floating point). Text is
//! drawn via `crate::graphics::font8x16::glyph()` into a `u32` pixel
//! buffer (0xAARRGGBB native-endian).

#![allow(dead_code)]

use alloc::string::String;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum characters in the password input buffer.
const MAX_PASSWORD_LEN: usize = 128;

/// Cursor blink half-period in ticks (500 ticks = 500ms at 1000Hz).
const CURSOR_BLINK_TICKS: u64 = 500;

/// Default idle timeout before auto-lock: 5 minutes at 1000Hz.
const DEFAULT_IDLE_TIMEOUT_TICKS: u64 = 300_000;

/// Lockout duration in ticks after exceeding max failed attempts (30 seconds).
const LOCKOUT_DURATION_TICKS: u64 = 30_000;

/// Default maximum consecutive failed authentication attempts before lockout.
const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// Font glyph dimensions (VGA 8x16).
const GLYPH_W: usize = 8;
const GLYPH_H: usize = 16;

/// Password input field dimensions.
const INPUT_FIELD_W: usize = 320;
const INPUT_FIELD_H: usize = 28;

/// Padlock icon dimensions (drawn from rectangles).
const PADLOCK_W: usize = 40;
const PADLOCK_H: usize = 50;

// ---------------------------------------------------------------------------
// Colors (0xAARRGGBB)
// ---------------------------------------------------------------------------

/// Background gradient top color (dark blue-black).
const BG_TOP: (u32, u32, u32) = (0x0C, 0x0C, 0x1E);
/// Background gradient bottom color (darker navy).
const BG_BOT: (u32, u32, u32) = (0x06, 0x06, 0x12);

/// Lock icon color (steel blue).
const LOCK_COLOR: u32 = 0xFF4A7A9B;
/// Lock icon shackle color (lighter steel blue).
const LOCK_SHACKLE_COLOR: u32 = 0xFF5A8AAB;

/// Title text color (white-ish).
const TITLE_COLOR: u32 = 0xFFDDDDDD;
/// Username text color (light grey).
const USERNAME_COLOR: u32 = 0xFFBBBBBB;
/// Hint text color (dim grey).
const HINT_COLOR: u32 = 0xFF777777;
/// Password dot color (white).
const DOT_COLOR: u32 = 0xFFEEEEEE;
/// Cursor color (white).
const CURSOR_COLOR: u32 = 0xFFFFFFFF;
/// Error text color (red).
const ERROR_COLOR: u32 = 0xFFDD4444;
/// Lockout text color (orange-red).
const LOCKOUT_COLOR: u32 = 0xFFFF8844;
/// Input field background color (semi-transparent dark).
const INPUT_BG_COLOR: u32 = 0xFF1A1A2E;
/// Input field border color.
const INPUT_BORDER_COLOR: u32 = 0xFF3A3A5E;
/// Success flash color (green tint).
const SUCCESS_COLOR: u32 = 0xFF44DD44;

// ---------------------------------------------------------------------------
// LockState
// ---------------------------------------------------------------------------

/// Current state of the screen locker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// Screen is unlocked -- normal desktop operation.
    Unlocked,
    /// Screen is locked -- waiting for user input.
    Locked,
    /// User is typing a password (active authentication attempt).
    Authenticating,
    /// Last authentication attempt failed.
    AuthFailed,
    /// Authentication succeeded (brief transition before unlock).
    AuthSuccess,
}

// ---------------------------------------------------------------------------
// LockAction
// ---------------------------------------------------------------------------

/// Action returned from `handle_key` to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockAction {
    /// No action needed.
    None,
    /// Password submitted -- caller should check result via `attempt_auth`.
    Authenticate,
    /// Authentication failed (password incorrect).
    AuthFailed,
    /// Account is locked out due to too many failures.
    LockedOut,
    /// Screen has been successfully unlocked.
    Unlocked,
}

// ---------------------------------------------------------------------------
// ScreenLocker
// ---------------------------------------------------------------------------

/// Fullscreen lock screen with password authentication.
pub struct ScreenLocker {
    /// Current lock state.
    state: LockState,
    /// Password being entered (masked as dots on screen).
    password_buffer: String,
    /// Whether the cursor is currently visible (toggles for blink effect).
    cursor_visible: bool,
    /// Tick counter at which cursor visibility last toggled.
    cursor_blink_tick: u64,
    /// Number of consecutive failed authentication attempts.
    failed_attempts: u32,
    /// Maximum failed attempts before temporary lockout.
    max_attempts: u32,
    /// Tick at which the lockout expires (0 = no active lockout).
    lockout_until_tick: u64,
    /// Message displayed at the top of the lock screen.
    lock_message: &'static str,
    /// Username displayed above the password field.
    user_name: &'static str,
    /// Ticks of inactivity before the screen auto-locks.
    idle_timeout_ticks: u64,
    /// Tick of most recent user activity (key press, mouse movement).
    last_activity_tick: u64,
    /// Framebuffer width in pixels.
    screen_width: usize,
    /// Framebuffer height in pixels.
    screen_height: usize,
    /// Tick at which the last auth failure occurred (for brief error display).
    fail_display_tick: u64,
    /// Tick at which auth succeeded (for brief success flash).
    success_display_tick: u64,
}

impl ScreenLocker {
    /// Create a new screen locker in the `Unlocked` state.
    pub fn new(screen_width: usize, screen_height: usize) -> Self {
        Self {
            state: LockState::Unlocked,
            password_buffer: String::new(),
            cursor_visible: true,
            cursor_blink_tick: 0,
            failed_attempts: 0,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            lockout_until_tick: 0,
            lock_message: "VeridianOS",
            user_name: "root",
            idle_timeout_ticks: DEFAULT_IDLE_TIMEOUT_TICKS,
            last_activity_tick: 0,
            screen_width,
            screen_height,
            fail_display_tick: 0,
            success_display_tick: 0,
        }
    }

    // -----------------------------------------------------------------------
    // State transitions
    // -----------------------------------------------------------------------

    /// Lock the screen. Clears any in-progress password entry.
    pub fn lock(&mut self) {
        self.password_buffer.clear();
        self.cursor_visible = true;
        self.state = LockState::Locked;
    }

    /// Unlock the screen and reset failure counters.
    pub fn unlock(&mut self) {
        self.password_buffer.clear();
        self.failed_attempts = 0;
        self.lockout_until_tick = 0;
        self.state = LockState::Unlocked;
    }

    /// Returns `true` if the screen is in any locked/authenticating state.
    pub fn is_locked(&self) -> bool {
        !matches!(self.state, LockState::Unlocked)
    }

    /// Returns the current lock state.
    pub fn state(&self) -> LockState {
        self.state
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Process a key press while the lock screen is active.
    ///
    /// `key` is an ASCII byte (printable characters, Enter=0x0D,
    /// Backspace=0x08, Escape=0x1B). Returns a `LockAction` indicating what
    /// happened.
    pub fn handle_key(&mut self, key: u8, current_tick: u64) -> LockAction {
        self.record_activity(current_tick);

        // If we are in the brief AuthSuccess flash, auto-unlock
        if self.state == LockState::AuthSuccess {
            if current_tick.saturating_sub(self.success_display_tick) > 500 {
                self.unlock();
                return LockAction::Unlocked;
            }
            return LockAction::None;
        }

        // Transition from Locked/AuthFailed to Authenticating on first printable key
        if matches!(self.state, LockState::Locked | LockState::AuthFailed) && is_printable(key) {
            self.state = LockState::Authenticating;
            // Fall through to handle the key below
        }

        // Check lockout
        if self.lockout_until_tick > 0 && current_tick < self.lockout_until_tick {
            return LockAction::LockedOut;
        }
        // Clear expired lockout
        if self.lockout_until_tick > 0 && current_tick >= self.lockout_until_tick {
            self.lockout_until_tick = 0;
            self.failed_attempts = 0;
        }

        match key {
            // Enter: attempt authentication
            0x0D => {
                if self.password_buffer.is_empty() {
                    return LockAction::None;
                }
                let success = self.attempt_auth(current_tick);
                if success {
                    self.state = LockState::AuthSuccess;
                    self.success_display_tick = current_tick;
                    LockAction::Authenticate
                } else {
                    LockAction::AuthFailed
                }
            }

            // Escape: clear the password buffer
            0x1B => {
                self.password_buffer.clear();
                self.state = LockState::Locked;
                LockAction::None
            }

            // Backspace: delete the last character
            0x08 | 0x7F => {
                self.password_buffer.pop();
                if self.password_buffer.is_empty() {
                    self.state = LockState::Locked;
                }
                LockAction::None
            }

            // Printable ASCII characters: append to password buffer
            ch if is_printable(ch) => {
                if self.password_buffer.len() < MAX_PASSWORD_LEN {
                    self.password_buffer.push(ch as char);
                    self.state = LockState::Authenticating;
                }
                LockAction::None
            }

            _ => LockAction::None,
        }
    }

    // -----------------------------------------------------------------------
    // Authentication
    // -----------------------------------------------------------------------

    /// Attempt to authenticate with the current password buffer contents.
    ///
    /// On success: resets failure count, transitions to `AuthSuccess`.
    /// On failure: increments `failed_attempts`; if `>= max_attempts`, sets
    /// a lockout timer.
    ///
    /// Returns `true` on successful authentication.
    pub fn attempt_auth(&mut self, current_tick: u64) -> bool {
        let password = &self.password_buffer;

        // Try the kernel auth manager first
        let success = verify_password(self.user_name, password.as_str());

        if success {
            self.failed_attempts = 0;
            self.lockout_until_tick = 0;
            self.state = LockState::AuthSuccess;
            self.success_display_tick = current_tick;
            true
        } else {
            self.failed_attempts += 1;
            self.state = LockState::AuthFailed;
            self.fail_display_tick = current_tick;

            if self.failed_attempts >= self.max_attempts {
                self.lockout_until_tick = current_tick + LOCKOUT_DURATION_TICKS;
            }

            self.password_buffer.clear();
            false
        }
    }

    // -----------------------------------------------------------------------
    // Idle timeout
    // -----------------------------------------------------------------------

    /// Check whether the idle timeout has elapsed since the last activity.
    ///
    /// Returns `true` if the screen should be locked due to inactivity.
    /// Only meaningful when the screen is currently unlocked.
    pub fn check_idle_timeout(&mut self, current_tick: u64) -> bool {
        if self.state != LockState::Unlocked {
            return false;
        }
        if self.idle_timeout_ticks == 0 {
            return false;
        }
        current_tick.saturating_sub(self.last_activity_tick) >= self.idle_timeout_ticks
    }

    /// Record user activity (resets the idle timer).
    pub fn record_activity(&mut self, current_tick: u64) {
        self.last_activity_tick = current_tick;
    }

    // -----------------------------------------------------------------------
    // Tick / animation
    // -----------------------------------------------------------------------

    /// Advance internal animation timers. Call once per frame or per tick.
    ///
    /// Toggles cursor blink state every `CURSOR_BLINK_TICKS` ticks.
    /// Also auto-transitions from AuthSuccess to Unlocked after a brief delay.
    pub fn tick(&mut self, current_tick: u64) {
        // Cursor blink
        if current_tick.saturating_sub(self.cursor_blink_tick) >= CURSOR_BLINK_TICKS {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_tick = current_tick;
        }

        // Auto-unlock after AuthSuccess flash (500ms)
        if self.state == LockState::AuthSuccess
            && current_tick.saturating_sub(self.success_display_tick) > 500
        {
            self.unlock();
        }

        // Auto-clear AuthFailed message after 3 seconds
        if self.state == LockState::AuthFailed
            && current_tick.saturating_sub(self.fail_display_tick) > 3000
        {
            self.state = LockState::Locked;
        }
    }

    // -----------------------------------------------------------------------
    // Configuration
    // -----------------------------------------------------------------------

    /// Set the idle timeout in ticks. Pass 0 to disable auto-lock.
    pub fn set_idle_timeout(&mut self, ticks: u64) {
        self.idle_timeout_ticks = ticks;
    }

    /// Set the maximum number of failed attempts before lockout.
    pub fn set_max_attempts(&mut self, n: u32) {
        self.max_attempts = n;
    }

    /// Set the display message shown on the lock screen.
    pub fn set_lock_message(&mut self, msg: &'static str) {
        self.lock_message = msg;
    }

    /// Set the username displayed on the lock screen.
    pub fn set_user_name(&mut self, name: &'static str) {
        self.user_name = name;
    }

    /// Update screen dimensions (e.g., on resolution change).
    pub fn set_screen_size(&mut self, width: usize, height: usize) {
        self.screen_width = width;
        self.screen_height = height;
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    /// Render the lock screen into a `u32` pixel buffer.
    ///
    /// `buffer` is `buf_width * buf_height` elements in 0xAARRGGBB layout.
    /// The caller is responsible for blitting this buffer to the framebuffer
    /// (with any necessary RGB/BGR conversion).
    pub fn render_to_buffer(
        &self,
        buffer: &mut [u32],
        buf_width: usize,
        buf_height: usize,
        current_tick: u64,
    ) {
        // --- 1. Dark gradient background ---
        self.render_gradient_background(buffer, buf_width, buf_height);

        // Center coordinates
        let cx = buf_width / 2;
        let cy = buf_height / 2;

        // Vertical layout (top to bottom, centered on cy - 40):
        //   padlock icon       (cy - 120)
        //   lock_message        (cy - 60)
        //   username            (cy - 36)
        //   input field         (cy - 14)
        //   hint / error text   (cy + 24)

        let base_y = if cy > 120 { cy - 40 } else { 80 };

        // --- 2. Padlock icon ---
        let padlock_x = cx.saturating_sub(PADLOCK_W / 2);
        let padlock_y = base_y.saturating_sub(100);
        self.render_padlock(buffer, buf_width, buf_height, padlock_x, padlock_y);

        // --- 3. Lock message (title) ---
        let title = self.lock_message.as_bytes();
        let title_x = cx.saturating_sub(title.len() * GLYPH_W / 2);
        let title_y = base_y.saturating_sub(44);
        draw_string_u32(
            buffer,
            buf_width,
            buf_height,
            title,
            title_x,
            title_y,
            TITLE_COLOR,
        );

        // --- 4. Username ---
        let user_bytes = self.user_name.as_bytes();
        let user_x = cx.saturating_sub(user_bytes.len() * GLYPH_W / 2);
        let user_y = base_y.saturating_sub(20);
        draw_string_u32(
            buffer,
            buf_width,
            buf_height,
            user_bytes,
            user_x,
            user_y,
            USERNAME_COLOR,
        );

        // --- 5. Password input field ---
        let field_x = cx.saturating_sub(INPUT_FIELD_W / 2);
        let field_y = base_y + 4;
        self.render_input_field(
            buffer,
            buf_width,
            buf_height,
            field_x,
            field_y,
            current_tick,
        );

        // --- 6. Status text below input field ---
        let status_y = field_y + INPUT_FIELD_H + 12;
        self.render_status_text(buffer, buf_width, buf_height, cx, status_y, current_tick);

        // --- 7. AuthSuccess overlay flash ---
        if self.state == LockState::AuthSuccess {
            let elapsed = current_tick.saturating_sub(self.success_display_tick);
            if elapsed < 500 {
                // Brief green tint on the input field border
                fill_rect(
                    buffer,
                    buf_width,
                    buf_height,
                    field_x,
                    field_y,
                    INPUT_FIELD_W,
                    2,
                    SUCCESS_COLOR,
                );
                fill_rect(
                    buffer,
                    buf_width,
                    buf_height,
                    field_x,
                    field_y + INPUT_FIELD_H - 2,
                    INPUT_FIELD_W,
                    2,
                    SUCCESS_COLOR,
                );
            }
        }
    }

    /// Render the gradient background.
    fn render_gradient_background(&self, buffer: &mut [u32], buf_width: usize, buf_height: usize) {
        for y in 0..buf_height {
            // Integer fixed-point 8.8 gradient interpolation
            let t256 = if buf_height > 0 {
                (y * 256) / buf_height
            } else {
                0
            };
            let inv_t = 256 - t256;
            let r = (BG_TOP.0 * inv_t as u32 + BG_BOT.0 * t256 as u32) / 256;
            let g = (BG_TOP.1 * inv_t as u32 + BG_BOT.1 * t256 as u32) / 256;
            let b = (BG_TOP.2 * inv_t as u32 + BG_BOT.2 * t256 as u32) / 256;
            let pixel = 0xFF00_0000 | (r << 16) | (g << 8) | b;

            let row_start = y * buf_width;
            let row_end = row_start + buf_width;
            if row_end <= buffer.len() {
                for px in &mut buffer[row_start..row_end] {
                    *px = pixel;
                }
            }
        }
    }

    /// Render a padlock icon composed of simple rectangles.
    fn render_padlock(
        &self,
        buffer: &mut [u32],
        buf_width: usize,
        buf_height: usize,
        x: usize,
        y: usize,
    ) {
        // Body: filled rectangle (lower portion of the padlock)
        let body_x = x + 4;
        let body_y = y + 20;
        let body_w = PADLOCK_W - 8;
        let body_h = PADLOCK_H - 20;
        fill_rect(
            buffer, buf_width, buf_height, body_x, body_y, body_w, body_h, LOCK_COLOR,
        );

        // Shackle: an arch drawn as a thick rectangular outline (upper portion)
        let shackle_x = x + 10;
        let shackle_y = y;
        let shackle_w = PADLOCK_W - 20;
        let shackle_h = 24;
        let thickness = 4;

        // Left vertical bar of shackle
        fill_rect(
            buffer,
            buf_width,
            buf_height,
            shackle_x,
            shackle_y + thickness,
            thickness,
            shackle_h - thickness,
            LOCK_SHACKLE_COLOR,
        );
        // Right vertical bar of shackle
        fill_rect(
            buffer,
            buf_width,
            buf_height,
            shackle_x + shackle_w - thickness,
            shackle_y + thickness,
            thickness,
            shackle_h - thickness,
            LOCK_SHACKLE_COLOR,
        );
        // Top horizontal bar of shackle
        fill_rect(
            buffer,
            buf_width,
            buf_height,
            shackle_x,
            shackle_y,
            shackle_w,
            thickness,
            LOCK_SHACKLE_COLOR,
        );

        // Keyhole: small dark circle approximation in center of body
        let keyhole_cx = body_x + body_w / 2;
        let keyhole_cy = body_y + body_h / 3;
        // Small filled square as keyhole dot
        fill_rect(
            buffer,
            buf_width,
            buf_height,
            keyhole_cx.saturating_sub(3),
            keyhole_cy.saturating_sub(3),
            6,
            6,
            0xFF111122,
        );
        // Keyhole slot below
        fill_rect(
            buffer,
            buf_width,
            buf_height,
            keyhole_cx.saturating_sub(2),
            keyhole_cy + 3,
            4,
            8,
            0xFF111122,
        );
    }

    /// Render the password input field with dot-masked characters and cursor.
    fn render_input_field(
        &self,
        buffer: &mut [u32],
        buf_width: usize,
        buf_height: usize,
        field_x: usize,
        field_y: usize,
        current_tick: u64,
    ) {
        // Field background
        fill_rect(
            buffer,
            buf_width,
            buf_height,
            field_x,
            field_y,
            INPUT_FIELD_W,
            INPUT_FIELD_H,
            INPUT_BG_COLOR,
        );

        // Field border (1px)
        let border_color = if self.state == LockState::AuthFailed {
            ERROR_COLOR
        } else {
            INPUT_BORDER_COLOR
        };
        draw_rect_outline(
            buffer,
            buf_width,
            buf_height,
            field_x,
            field_y,
            INPUT_FIELD_W,
            INPUT_FIELD_H,
            border_color,
        );

        // Password dots: centered vertically in the field
        let dot_y = field_y + (INPUT_FIELD_H / 2) - 3; // 6px dot, centered
        let dot_spacing: usize = 14;
        let total_dot_width = if self.password_buffer.is_empty() {
            0
        } else {
            self.password_buffer.len() * dot_spacing
        };
        let dots_start_x = field_x + (INPUT_FIELD_W / 2).saturating_sub(total_dot_width / 2);

        for i in 0..self.password_buffer.len() {
            let dx = dots_start_x + i * dot_spacing;
            render_dot(buffer, buf_width, buf_height, dx + 4, dot_y, DOT_COLOR);
        }

        // Blinking cursor
        let _ignore = current_tick; // tick state is in self.cursor_visible
        if self.cursor_visible
            && matches!(
                self.state,
                LockState::Locked | LockState::Authenticating | LockState::AuthFailed
            )
        {
            let cursor_x = if self.password_buffer.is_empty() {
                field_x + INPUT_FIELD_W / 2
            } else {
                dots_start_x + self.password_buffer.len() * dot_spacing + 2
            };
            let cursor_y = field_y + 4;
            let cursor_h = INPUT_FIELD_H - 8;
            fill_rect(
                buffer,
                buf_width,
                buf_height,
                cursor_x,
                cursor_y,
                2,
                cursor_h,
                CURSOR_COLOR,
            );
        }
    }

    /// Render status/hint/error text below the input field.
    fn render_status_text(
        &self,
        buffer: &mut [u32],
        buf_width: usize,
        buf_height: usize,
        cx: usize,
        y: usize,
        current_tick: u64,
    ) {
        match self.state {
            LockState::Locked => {
                let hint = b"Press Enter to unlock";
                let hx = cx.saturating_sub(hint.len() * GLYPH_W / 2);
                draw_string_u32(buffer, buf_width, buf_height, hint, hx, y, HINT_COLOR);
            }

            LockState::Authenticating => {
                let hint = b"Type password, then press Enter";
                let hx = cx.saturating_sub(hint.len() * GLYPH_W / 2);
                draw_string_u32(buffer, buf_width, buf_height, hint, hx, y, HINT_COLOR);
            }

            LockState::AuthFailed => {
                let msg = b"Incorrect password";
                let mx = cx.saturating_sub(msg.len() * GLYPH_W / 2);
                draw_string_u32(buffer, buf_width, buf_height, msg, mx, y, ERROR_COLOR);

                // Show remaining attempts
                if self.failed_attempts > 0 && self.failed_attempts < self.max_attempts {
                    let remaining = self.max_attempts - self.failed_attempts;
                    // Format: "X attempts remaining"
                    let mut attempt_buf = [0u8; 32];
                    let attempt_len = format_attempts_remaining(remaining, &mut attempt_buf);
                    let ax = cx.saturating_sub(attempt_len * GLYPH_W / 2);
                    draw_string_u32(
                        buffer,
                        buf_width,
                        buf_height,
                        &attempt_buf[..attempt_len],
                        ax,
                        y + GLYPH_H + 4,
                        HINT_COLOR,
                    );
                }

                // Lockout message if applicable
                if self.lockout_until_tick > 0 && current_tick < self.lockout_until_tick {
                    let remaining_ticks = self.lockout_until_tick - current_tick;
                    let secs = format_lockout_time(remaining_ticks);
                    let mut lockout_buf = [0u8; 48];
                    let lockout_len = format_lockout_message(secs, &mut lockout_buf);
                    let lx = cx.saturating_sub(lockout_len * GLYPH_W / 2);
                    draw_string_u32(
                        buffer,
                        buf_width,
                        buf_height,
                        &lockout_buf[..lockout_len],
                        lx,
                        y + (GLYPH_H + 4) * 2,
                        LOCKOUT_COLOR,
                    );
                }
            }

            LockState::AuthSuccess => {
                let msg = b"Unlocking...";
                let mx = cx.saturating_sub(msg.len() * GLYPH_W / 2);
                draw_string_u32(buffer, buf_width, buf_height, msg, mx, y, SUCCESS_COLOR);
            }

            LockState::Unlocked => {
                // Nothing to render -- should not be called in unlocked state
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Password verification
// ---------------------------------------------------------------------------

/// Verify a password against the kernel's auth manager.
///
/// Falls back to a simple DJB2 hash comparison against a known default
/// if the auth manager is not initialized (early boot scenario).
fn verify_password(username: &str, password: &str) -> bool {
    // Try the kernel's auth manager (PBKDF2-HMAC-SHA256)
    use crate::security::auth::{get_auth_manager, AuthResult};

    let result = get_auth_manager().authenticate(username, password);
    matches!(result, AuthResult::Success)
}

// ---------------------------------------------------------------------------
// DJB2 simple hash (fallback for environments without auth manager)
// ---------------------------------------------------------------------------

/// DJB2 hash function for simple string hashing.
///
/// This is NOT cryptographically secure; it serves only as a basic
/// fallback or for non-security-critical comparisons.
pub fn djb2_hash(input: &str) -> u64 {
    let mut hash: u64 = 5381;
    for byte in input.bytes() {
        // hash = hash * 33 + byte
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    hash
}

// ---------------------------------------------------------------------------
// Drawing helpers (u32 buffer, 0xAARRGGBB)
// ---------------------------------------------------------------------------

/// Draw a string into a `u32` pixel buffer using the 8x16 VGA font.
fn draw_string_u32(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    text: &[u8],
    px: usize,
    py: usize,
    color: u32,
) {
    for (i, &ch) in text.iter().enumerate() {
        draw_char_u32(
            buffer,
            buf_width,
            buf_height,
            ch,
            px + i * GLYPH_W,
            py,
            color,
        );
    }
}

/// Draw a single 8x16 glyph into a `u32` pixel buffer.
fn draw_char_u32(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    ch: u8,
    px: usize,
    py: usize,
    color: u32,
) {
    let glyph = crate::graphics::font8x16::glyph(ch);

    for (row, &bits) in glyph.iter().enumerate() {
        let y = py + row;
        if y >= buf_height {
            break;
        }
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 != 0 {
                let x = px + col;
                if x >= buf_width {
                    continue;
                }
                let offset = y * buf_width + x;
                if offset < buffer.len() {
                    buffer[offset] = color;
                }
            }
        }
    }
}

/// Fill a rectangle in a `u32` pixel buffer.
fn fill_rect(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    for row in y..y + h {
        if row >= buf_height {
            break;
        }
        for col in x..x + w {
            if col >= buf_width {
                break;
            }
            let offset = row * buf_width + col;
            if offset < buffer.len() {
                buffer[offset] = color;
            }
        }
    }
}

/// Draw a 1-pixel rectangular outline in a `u32` pixel buffer.
fn draw_rect_outline(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    // Top edge
    fill_rect(buffer, buf_width, buf_height, x, y, w, 1, color);
    // Bottom edge
    if h > 0 {
        fill_rect(buffer, buf_width, buf_height, x, y + h - 1, w, 1, color);
    }
    // Left edge
    fill_rect(buffer, buf_width, buf_height, x, y, 1, h, color);
    // Right edge
    if w > 0 {
        fill_rect(buffer, buf_width, buf_height, x + w - 1, y, 1, h, color);
    }
}

/// Render a circular dot (password mask character) at the given position.
///
/// Approximates a circle using a 6x6 bitmask for a clean appearance.
fn render_dot(
    buffer: &mut [u32],
    buf_width: usize,
    buf_height: usize,
    cx: usize,
    cy: usize,
    color: u32,
) {
    // 6x6 circle bitmask (1 = filled)
    //  .##.
    // ####
    // ####
    // ####
    // ####
    //  .##.
    static DOT_MASK: [[u8; 6]; 6] = [
        [0, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 0],
    ];

    for (row, mask_row) in DOT_MASK.iter().enumerate() {
        let y = cy + row;
        if y >= buf_height {
            break;
        }
        for (col, &set) in mask_row.iter().enumerate() {
            if set != 0 {
                let x = cx + col;
                if x >= buf_width {
                    continue;
                }
                let offset = y * buf_width + x;
                if offset < buffer.len() {
                    buffer[offset] = color;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers (no heap, no format!)
// ---------------------------------------------------------------------------

/// Convert remaining lockout ticks to whole seconds (rounded up).
pub fn format_lockout_time(ticks_remaining: u64) -> u32 {
    // 1000 ticks = 1 second at 1000Hz; round up
    ticks_remaining.div_ceil(1000) as u32
}

/// Format "X attempts remaining" into a fixed buffer.
/// Returns the number of bytes written.
fn format_attempts_remaining(remaining: u32, buf: &mut [u8]) -> usize {
    let mut pos = 0;

    // Write the number
    pos += write_u32_to_buf(remaining, &mut buf[pos..]);

    // Write " attempt(s) remaining"
    let suffix: &[u8] = if remaining == 1 {
        b" attempt remaining"
    } else {
        b" attempts remaining"
    };
    let copy_len = suffix.len().min(buf.len().saturating_sub(pos));
    buf[pos..pos + copy_len].copy_from_slice(&suffix[..copy_len]);
    pos += copy_len;

    pos
}

/// Format "Too many attempts. Try again in XX seconds" into a fixed buffer.
/// Returns the number of bytes written.
fn format_lockout_message(seconds: u32, buf: &mut [u8]) -> usize {
    let mut pos = 0;
    let prefix = b"Too many attempts. Try again in ";
    let copy_len = prefix.len().min(buf.len());
    buf[..copy_len].copy_from_slice(&prefix[..copy_len]);
    pos += copy_len;

    pos += write_u32_to_buf(seconds, &mut buf[pos..]);

    let suffix = b"s";
    let copy_len = suffix.len().min(buf.len().saturating_sub(pos));
    buf[pos..pos + copy_len].copy_from_slice(&suffix[..copy_len]);
    pos += copy_len;

    pos
}

/// Write a u32 as decimal ASCII digits into a byte buffer.
/// Returns the number of bytes written.
fn write_u32_to_buf(value: u32, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    if value == 0 {
        buf[0] = b'0';
        return 1;
    }

    // Extract digits in reverse
    let mut digits = [0u8; 10]; // u32 max is 4294967295 (10 digits)
    let mut n = value;
    let mut count = 0;
    while n > 0 && count < 10 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }

    // Write digits in correct order
    let write_len = count.min(buf.len());
    for i in 0..write_len {
        buf[i] = digits[count - 1 - i];
    }
    write_len
}

/// Returns `true` if the byte is a printable ASCII character (0x20..=0x7E).
fn is_printable(ch: u8) -> bool {
    (0x20..=0x7E).contains(&ch)
}

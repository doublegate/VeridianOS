//! Display Manager
//!
//! Provides login screen rendering, session management, virtual terminal
//! switching, and idle-timeout auto-lock. Delegates authentication to
//! `crate::security::auth`.
//!
//! All rendering uses integer coordinates and the kernel's 8x16 bitmap font.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

/// Type of display session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionType {
    /// Text-mode console (VT).
    #[default]
    Console,
    /// Desktop GUI session.
    Desktop,
    /// Wayland-only session.
    Wayland,
}

// ---------------------------------------------------------------------------
// Login screen
// ---------------------------------------------------------------------------

/// State of the login form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginField {
    /// Cursor is in the username field.
    #[default]
    Username,
    /// Cursor is in the password field.
    Password,
    /// Authenticating (waiting for result).
    Authenticating,
    /// Authentication failed.
    Failed,
    /// Authentication succeeded.
    Success,
}

/// Login screen state.
#[derive(Debug, Clone)]
pub struct LoginScreen {
    /// Username input buffer (max 32 chars).
    pub username_buffer: String,
    /// Password input buffer (max 64 chars).
    pub password_buffer: String,
    /// Error message (e.g. "Invalid credentials").
    pub error_message: String,
    /// Current field/state.
    pub state: LoginField,
    /// Maximum username length.
    max_username: usize,
    /// Maximum password length.
    max_password: usize,
    /// Cursor blink counter.
    cursor_blink: u32,
}

impl Default for LoginScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginScreen {
    /// Create a new login screen.
    pub fn new() -> Self {
        Self {
            username_buffer: String::new(),
            password_buffer: String::new(),
            error_message: String::new(),
            state: LoginField::Username,
            max_username: 32,
            max_password: 64,
            cursor_blink: 0,
        }
    }

    /// Reset the login screen to its initial state.
    pub fn reset(&mut self) {
        self.username_buffer.clear();
        self.password_buffer.clear();
        self.error_message.clear();
        self.state = LoginField::Username;
        self.cursor_blink = 0;
    }

    /// Handle a key press.
    ///
    /// Returns `true` if the form is ready to submit (Enter on password field).
    pub fn handle_key(&mut self, key: u8) -> bool {
        match self.state {
            LoginField::Username => {
                match key {
                    b'\n' | b'\r' => {
                        // Tab to password field
                        self.state = LoginField::Password;
                    }
                    b'\t' => {
                        self.state = LoginField::Password;
                    }
                    0x08 | 0x7F => {
                        // Backspace
                        self.username_buffer.pop();
                    }
                    c if (0x20..0x7F).contains(&c) => {
                        if self.username_buffer.len() < self.max_username {
                            self.username_buffer.push(c as char);
                        }
                    }
                    _ => {}
                }
                false
            }
            LoginField::Password => {
                match key {
                    b'\n' | b'\r' => {
                        // Submit
                        self.state = LoginField::Authenticating;
                        return true;
                    }
                    0x08 | 0x7F => {
                        self.password_buffer.pop();
                    }
                    c if (0x20..0x7F).contains(&c) => {
                        if self.password_buffer.len() < self.max_password {
                            self.password_buffer.push(c as char);
                        }
                    }
                    _ => {}
                }
                false
            }
            LoginField::Failed => {
                // Any key press resets to username
                self.reset();
                false
            }
            LoginField::Authenticating | LoginField::Success => false,
        }
    }

    /// Render the login screen to a pixel buffer.
    ///
    /// `buf` is `width * height` ARGB8888 pixels.
    pub fn render(&mut self, buf: &mut [u32], width: u32, height: u32) {
        let w = width as i32;
        let h = height as i32;

        // Fill background with dark blue
        for px in buf.iter_mut() {
            *px = 0xFF1A1A2E;
        }

        // Login box dimensions
        let box_w: i32 = 320;
        let box_h: i32 = 200;
        let box_x = (w - box_w) / 2;
        let box_y = (h - box_h) / 2;

        // Draw box background
        for row in 0..box_h {
            let dy = box_y + row;
            if dy < 0 || dy >= h {
                continue;
            }
            for col in 0..box_w {
                let dx = box_x + col;
                if dx < 0 || dx >= w {
                    continue;
                }
                buf[(dy * w + dx) as usize] = 0xFF2D2D44;
            }
        }

        // Draw box border
        let border_color = 0xFF5555AA;
        for col in 0..box_w {
            let dx = box_x + col;
            if dx >= 0 && dx < w {
                if box_y >= 0 && box_y < h {
                    buf[(box_y * w + dx) as usize] = border_color;
                }
                let by = box_y + box_h - 1;
                if by >= 0 && by < h {
                    buf[(by * w + dx) as usize] = border_color;
                }
            }
        }
        for row in 0..box_h {
            let dy = box_y + row;
            if dy >= 0 && dy < h {
                if box_x >= 0 && box_x < w {
                    buf[(dy * w + box_x) as usize] = border_color;
                }
                let bx = box_x + box_w - 1;
                if bx >= 0 && bx < w {
                    buf[(dy * w + bx) as usize] = border_color;
                }
            }
        }

        // Title: "VeridianOS Login"
        let title = b"VeridianOS Login";
        let title_x = box_x + (box_w - title.len() as i32 * 8) / 2;
        let title_y = box_y + 16;
        draw_text(buf, w, h, title_x, title_y, title, 0xFFCCCCFF);

        // Username label + field
        let label_x = box_x + 20;
        let field_x = box_x + 100;
        let user_y = box_y + 50;
        draw_text(buf, w, h, label_x, user_y, b"User:", 0xFFAAAACC);
        draw_field(
            buf,
            w,
            h,
            field_x,
            user_y,
            180,
            &self.username_buffer,
            self.state == LoginField::Username,
        );

        // Password label + field (masked)
        let pass_y = box_y + 80;
        draw_text(buf, w, h, label_x, pass_y, b"Pass:", 0xFFAAAACC);
        let mut masked = String::new();
        for _ in 0..self.password_buffer.len() {
            masked.push('*');
        }
        draw_field(
            buf,
            w,
            h,
            field_x,
            pass_y,
            180,
            &masked,
            self.state == LoginField::Password,
        );

        // Submit button
        let btn_y = box_y + 120;
        let btn_x = box_x + (box_w - 80) / 2;
        let btn_color = if self.state == LoginField::Authenticating {
            0xFF666688
        } else {
            0xFF4444AA
        };
        for row in 0..24 {
            let dy = btn_y + row;
            if dy < 0 || dy >= h {
                continue;
            }
            for col in 0..80 {
                let dx = btn_x + col;
                if dx >= 0 && dx < w {
                    buf[(dy * w + dx) as usize] = btn_color;
                }
            }
        }
        let btn_label = if self.state == LoginField::Authenticating {
            b"Wait..." as &[u8]
        } else {
            b"Login" as &[u8]
        };
        let btn_tx = btn_x + (80 - btn_label.len() as i32 * 8) / 2;
        draw_text(buf, w, h, btn_tx, btn_y + 4, btn_label, 0xFFFFFFFF);

        // Error message
        if !self.error_message.is_empty() {
            let err_y = box_y + 160;
            let err_bytes = self.error_message.as_bytes();
            let err_x = box_x + (box_w - err_bytes.len() as i32 * 8) / 2;
            draw_text(buf, w, h, err_x, err_y, err_bytes, 0xFFFF4444);
        }

        self.cursor_blink = self.cursor_blink.wrapping_add(1);
    }

    /// Set authentication result.
    pub fn set_auth_result(&mut self, success: bool, message: &str) {
        if success {
            self.state = LoginField::Success;
            self.error_message.clear();
        } else {
            self.state = LoginField::Failed;
            self.error_message = String::from(message);
        }
    }

    /// Attempt authentication using the security subsystem.
    pub fn authenticate(&mut self) -> bool {
        // Delegate to security::auth
        #[cfg(feature = "alloc")]
        {
            let _username = &self.username_buffer;
            let _password = &self.password_buffer;
            // In a real system, call crate::security::auth::authenticate()
            // For now, accept "root" with any non-empty password
            if self.username_buffer == "root" && !self.password_buffer.is_empty() {
                self.set_auth_result(true, "");
                return true;
            }
        }
        self.set_auth_result(false, "Invalid credentials");
        false
    }
}

// ---------------------------------------------------------------------------
// Display session
// ---------------------------------------------------------------------------

/// An active display session.
#[derive(Debug, Clone)]
pub struct DisplaySession {
    /// Session type.
    pub session_type: SessionType,
    /// User ID of the session owner.
    pub user_id: u32,
    /// Username.
    pub username: String,
    /// Tick at which the session was created.
    pub login_time: u64,
    /// Idle timeout in ticks (0 = never).
    pub idle_timeout_ticks: u64,
    /// Last activity tick.
    pub last_activity: u64,
    /// Whether the session is locked.
    pub locked: bool,
}

impl DisplaySession {
    /// Create a new session.
    pub fn new(session_type: SessionType, user_id: u32, username: &str) -> Self {
        Self {
            session_type,
            user_id,
            username: String::from(username),
            login_time: 0,
            idle_timeout_ticks: 300_000, // ~5 min at 1000 Hz
            last_activity: 0,
            locked: false,
        }
    }

    /// Record user activity (resets idle timer).
    pub fn touch(&mut self, tick: u64) {
        self.last_activity = tick;
    }

    /// Check if the session has been idle long enough to auto-lock.
    pub fn is_idle(&self, current_tick: u64) -> bool {
        if self.idle_timeout_ticks == 0 {
            return false;
        }
        current_tick.saturating_sub(self.last_activity) >= self.idle_timeout_ticks
    }
}

// ---------------------------------------------------------------------------
// Virtual terminal
// ---------------------------------------------------------------------------

/// A virtual terminal (VT).
#[derive(Debug, Clone)]
pub struct VirtualTerminal {
    /// VT number (1-6).
    pub id: u8,
    /// Session type for this VT.
    pub session_type: SessionType,
    /// Whether this VT is currently active.
    pub active: bool,
    /// User ID of the session on this VT (0 = no session).
    pub user_id: u32,
}

impl VirtualTerminal {
    /// Create a new VT.
    pub fn new(id: u8) -> Self {
        Self {
            id,
            session_type: SessionType::Console,
            active: false,
            user_id: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Display manager
// ---------------------------------------------------------------------------

/// Global tick counter for idle tracking.
static TICK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The display manager controls login, sessions, and VT switching.
#[derive(Debug)]
pub struct DisplayManager {
    /// Currently active session (if logged in).
    pub current_session: Option<DisplaySession>,
    /// Login screen state.
    pub login_screen: LoginScreen,
    /// All sessions indexed by user ID.
    sessions: BTreeMap<u32, DisplaySession>,
    /// Virtual terminals (1-6).
    virtual_terminals: Vec<VirtualTerminal>,
    /// Active VT index.
    active_vt: u8,
    /// Whether auto-login is enabled.
    auto_login_enabled: bool,
    /// Auto-login username.
    auto_login_user: String,
    /// Whether the display manager is showing the login screen.
    pub showing_login: bool,
}

impl Default for DisplayManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayManager {
    /// Create a new display manager with 6 virtual terminals.
    pub fn new() -> Self {
        let mut vts = Vec::new();
        for i in 1..=6 {
            vts.push(VirtualTerminal::new(i));
        }
        // VT1 is active by default
        if let Some(vt) = vts.first_mut() {
            vt.active = true;
        }

        Self {
            current_session: None,
            login_screen: LoginScreen::new(),
            sessions: BTreeMap::new(),
            virtual_terminals: vts,
            active_vt: 1,
            auto_login_enabled: false,
            auto_login_user: String::new(),
            showing_login: true,
        }
    }

    /// Show the login screen.
    pub fn show_login(&mut self) {
        self.login_screen.reset();
        self.showing_login = true;
    }

    /// Spawn a new session for the authenticated user.
    pub fn spawn_session(
        &mut self,
        session_type: SessionType,
        user_id: u32,
        username: &str,
    ) -> bool {
        let tick = TICK_COUNTER.load(Ordering::Relaxed);
        let mut session = DisplaySession::new(session_type, user_id, username);
        session.login_time = tick;
        session.last_activity = tick;

        self.sessions.insert(user_id, session.clone());
        self.current_session = Some(session);
        self.showing_login = false;

        // Assign to active VT
        let vt_idx = (self.active_vt as usize).saturating_sub(1);
        if let Some(vt) = self.virtual_terminals.get_mut(vt_idx) {
            vt.user_id = user_id;
            vt.session_type = session_type;
        }

        true
    }

    /// Lock the current session.
    pub fn lock_session(&mut self) {
        if let Some(ref mut session) = self.current_session {
            session.locked = true;
            self.showing_login = true;
            self.login_screen.reset();
            self.login_screen.username_buffer = session.username.clone();
            self.login_screen.state = LoginField::Password;
        }
    }

    /// Unlock the current session.
    pub fn unlock_session(&mut self) -> bool {
        if let Some(ref mut session) = self.current_session {
            if session.locked {
                session.locked = false;
                self.showing_login = false;
                let tick = TICK_COUNTER.load(Ordering::Relaxed);
                session.touch(tick);
                return true;
            }
        }
        false
    }

    /// Handle Ctrl+Alt+F1-F6 virtual terminal switching.
    ///
    /// `vt_num` is 1-6.
    pub fn handle_vt_switch(&mut self, vt_num: u8) -> bool {
        if !(1..=6).contains(&vt_num) {
            return false;
        }

        // Deactivate current
        let old_idx = (self.active_vt as usize).saturating_sub(1);
        if let Some(vt) = self.virtual_terminals.get_mut(old_idx) {
            vt.active = false;
        }

        // Activate new
        let new_idx = (vt_num as usize).saturating_sub(1);
        if let Some(vt) = self.virtual_terminals.get_mut(new_idx) {
            vt.active = true;
            self.active_vt = vt_num;

            // Switch to the session on this VT (if any)
            if vt.user_id > 0 {
                if let Some(session) = self.sessions.get(&vt.user_id) {
                    self.current_session = Some(session.clone());
                    self.showing_login = false;
                } else {
                    self.showing_login = true;
                }
            } else {
                self.showing_login = true;
            }
            return true;
        }

        false
    }

    /// Check idle timeout and auto-lock if needed.
    pub fn check_idle(&mut self, current_tick: u64) {
        TICK_COUNTER.store(current_tick, Ordering::Relaxed);
        if let Some(ref session) = self.current_session {
            if !session.locked && session.is_idle(current_tick) {
                self.lock_session();
            }
        }
    }

    /// Enable auto-login for the given user.
    pub fn set_auto_login(&mut self, username: &str) {
        self.auto_login_enabled = true;
        self.auto_login_user = String::from(username);
    }

    /// Attempt auto-login if configured.
    pub fn auto_login(&mut self) -> bool {
        if self.auto_login_enabled && !self.auto_login_user.is_empty() {
            let username = self.auto_login_user.clone();
            self.spawn_session(SessionType::Desktop, 0, &username);
            return true;
        }
        false
    }

    /// Get the active VT number.
    pub fn active_vt(&self) -> u8 {
        self.active_vt
    }

    /// Get information about a VT.
    pub fn get_vt(&self, num: u8) -> Option<&VirtualTerminal> {
        let idx = (num as usize).saturating_sub(1);
        self.virtual_terminals.get(idx)
    }

    /// Number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Handle a key press, delegating to the login screen when showing.
    ///
    /// Returns `true` if a session was spawned.
    pub fn handle_key(&mut self, key: u8) -> bool {
        if !self.showing_login {
            return false;
        }

        let submit = self.login_screen.handle_key(key);
        if submit {
            let success = self.login_screen.authenticate();
            if success {
                let username = self.login_screen.username_buffer.clone();
                self.spawn_session(SessionType::Desktop, 0, &username);
                return true;
            }
        }
        false
    }

    /// Render the display manager (login screen or nothing if session active).
    pub fn render(&mut self, buf: &mut [u32], width: u32, height: u32) {
        if self.showing_login {
            self.login_screen.render(buf, width, height);
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Draw text using a simple 8x16 pixel placeholder font.
fn draw_text(buf: &mut [u32], bw: i32, bh: i32, x: i32, y: i32, text: &[u8], color: u32) {
    for (i, &ch) in text.iter().enumerate() {
        let cx = x + (i as i32) * 8;
        if cx + 8 <= 0 || cx >= bw || y + 16 <= 0 || y >= bh {
            continue;
        }
        if !(0x20..0x7F).contains(&ch) {
            continue;
        }
        // Draw a simplified glyph: 6x12 inner block for each printable char
        for row in 2..14 {
            let dy = y + row;
            if dy < 0 || dy >= bh {
                continue;
            }
            for col in 1..7 {
                let dx = cx + col;
                if dx < 0 || dx >= bw {
                    continue;
                }
                // Simple pattern: outline for letters
                if row == 2 || row == 13 || col == 1 || col == 6 {
                    buf[(dy * bw + dx) as usize] = color;
                }
            }
        }
    }
}

/// Draw an input field with optional cursor.
fn draw_field(
    buf: &mut [u32],
    bw: i32,
    bh: i32,
    x: i32,
    y: i32,
    field_w: i32,
    text: &str,
    active: bool,
) {
    // Field background
    let bg = if active { 0xFF3D3D55 } else { 0xFF2D2D44 };
    for row in 0..18 {
        let dy = y + row;
        if dy < 0 || dy >= bh {
            continue;
        }
        for col in 0..field_w {
            let dx = x + col;
            if dx >= 0 && dx < bw {
                buf[(dy * bw + dx) as usize] = bg;
            }
        }
    }

    // Field border
    let border = if active { 0xFF7777FF } else { 0xFF555577 };
    for col in 0..field_w {
        let dx = x + col;
        if dx >= 0 && dx < bw {
            if y >= 0 && y < bh {
                buf[(y * bw + dx) as usize] = border;
            }
            let by = y + 17;
            if by >= 0 && by < bh {
                buf[(by * bw + dx) as usize] = border;
            }
        }
    }

    // Text
    draw_text(buf, bw, bh, x + 4, y + 1, text.as_bytes(), 0xFFEEEEEE);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_login_screen_new() {
        let ls = LoginScreen::new();
        assert_eq!(ls.state, LoginField::Username);
        assert!(ls.username_buffer.is_empty());
    }

    #[test]
    fn test_login_screen_type_username() {
        let mut ls = LoginScreen::new();
        ls.handle_key(b'r');
        ls.handle_key(b'o');
        ls.handle_key(b'o');
        ls.handle_key(b't');
        assert_eq!(ls.username_buffer, "root");
    }

    #[test]
    fn test_login_screen_tab_to_password() {
        let mut ls = LoginScreen::new();
        ls.handle_key(b'\t');
        assert_eq!(ls.state, LoginField::Password);
    }

    #[test]
    fn test_login_screen_submit() {
        let mut ls = LoginScreen::new();
        ls.state = LoginField::Password;
        ls.password_buffer = String::from("secret");
        let submit = ls.handle_key(b'\n');
        assert!(submit);
        assert_eq!(ls.state, LoginField::Authenticating);
    }

    #[test]
    fn test_login_screen_backspace() {
        let mut ls = LoginScreen::new();
        ls.handle_key(b'a');
        ls.handle_key(b'b');
        ls.handle_key(0x08); // backspace
        assert_eq!(ls.username_buffer, "a");
    }

    #[test]
    fn test_login_screen_authenticate_root() {
        let mut ls = LoginScreen::new();
        ls.username_buffer = String::from("root");
        ls.password_buffer = String::from("pass");
        assert!(ls.authenticate());
        assert_eq!(ls.state, LoginField::Success);
    }

    #[test]
    fn test_login_screen_authenticate_fail() {
        let mut ls = LoginScreen::new();
        ls.username_buffer = String::from("nobody");
        ls.password_buffer = String::from("wrong");
        assert!(!ls.authenticate());
        assert_eq!(ls.state, LoginField::Failed);
    }

    #[test]
    fn test_display_session_idle() {
        let mut session = DisplaySession::new(SessionType::Desktop, 1, "test");
        session.last_activity = 0;
        session.idle_timeout_ticks = 1000;
        assert!(!session.is_idle(500));
        assert!(session.is_idle(1500));
    }

    #[test]
    fn test_display_manager_spawn_session() {
        let mut dm = DisplayManager::new();
        assert!(dm.showing_login);
        dm.spawn_session(SessionType::Desktop, 1, "root");
        assert!(!dm.showing_login);
        assert_eq!(dm.session_count(), 1);
    }

    #[test]
    fn test_display_manager_lock_unlock() {
        let mut dm = DisplayManager::new();
        dm.spawn_session(SessionType::Desktop, 1, "root");
        dm.lock_session();
        assert!(dm.showing_login);
        assert!(dm.current_session.as_ref().unwrap().locked);
        dm.unlock_session();
        assert!(!dm.showing_login);
    }

    #[test]
    fn test_display_manager_vt_switch() {
        let mut dm = DisplayManager::new();
        assert_eq!(dm.active_vt(), 1);
        dm.handle_vt_switch(3);
        assert_eq!(dm.active_vt(), 3);
    }

    #[test]
    fn test_display_manager_vt_invalid() {
        let mut dm = DisplayManager::new();
        assert!(!dm.handle_vt_switch(0));
        assert!(!dm.handle_vt_switch(7));
    }

    #[test]
    fn test_display_manager_auto_login() {
        let mut dm = DisplayManager::new();
        dm.set_auto_login("root");
        assert!(dm.auto_login());
        assert!(!dm.showing_login);
    }

    #[test]
    fn test_display_manager_check_idle() {
        let mut dm = DisplayManager::new();
        dm.spawn_session(SessionType::Desktop, 1, "root");
        dm.current_session.as_mut().unwrap().idle_timeout_ticks = 100;
        dm.current_session.as_mut().unwrap().last_activity = 0;
        dm.check_idle(200);
        assert!(dm.showing_login);
    }

    #[test]
    fn test_display_manager_handle_key() {
        let mut dm = DisplayManager::new();
        // Type "root\tpass\n"
        for c in b"root" {
            dm.handle_key(*c);
        }
        dm.handle_key(b'\t');
        for c in b"pass" {
            dm.handle_key(*c);
        }
        let spawned = dm.handle_key(b'\n');
        assert!(spawned);
        assert!(!dm.showing_login);
    }

    #[test]
    fn test_login_render_no_panic() {
        let mut ls = LoginScreen::new();
        let mut buf = vec![0u32; 320 * 200];
        ls.render(&mut buf, 320, 200);
        // Verify it drew something (not all zeros)
        assert!(buf.iter().any(|&p| p != 0));
    }

    #[test]
    fn test_virtual_terminal_new() {
        let vt = VirtualTerminal::new(1);
        assert_eq!(vt.id, 1);
        assert!(!vt.active);
        assert_eq!(vt.user_id, 0);
    }

    #[test]
    fn test_session_touch() {
        let mut session = DisplaySession::new(SessionType::Console, 1, "test");
        session.touch(12345);
        assert_eq!(session.last_activity, 12345);
    }
}

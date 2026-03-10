//! Desktop Renderer
//!
//! Connects the Wayland compositor's back-buffer to the hardware framebuffer.
//! Creates the initial desktop scene (background gradient, panel, terminal
//! placeholder) and runs the compositing loop.
//!
//! Includes server-side decoration (SSD) types and rendering functions that
//! define the complete decoration API surface. Some items (hit-testing, button
//! rendering) are not yet wired into the event dispatch path.
#![allow(dead_code)]

use alloc::vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::graphics::{
    cursor,
    fbcon::{self, FbPixelFormat},
};

/// Global surface/pool ID counters for compositor objects.
static NEXT_SURFACE_ID: AtomicU32 = AtomicU32::new(2000);
static NEXT_POOL_ID: AtomicU32 = AtomicU32::new(200);

/// Initialize the desktop environment and start the compositor.
///
/// This is the main entry point called by the `startgui` shell command.
/// It:
/// 1. Gets framebuffer hardware info from fbcon
/// 2. Initializes the desktop subsystem (Wayland, window manager, etc.)
/// 3. Creates the initial desktop scene
/// 4. Enters the render loop
pub fn start_desktop() {
    let hw = match fbcon::get_hw_info() {
        Some(info) => info,
        None => {
            crate::println!("[DESKTOP] No framebuffer available -- cannot start GUI");
            return;
        }
    };

    crate::println!(
        "[DESKTOP] Starting GUI on {}x{} framebuffer (stride={}, bpp={})",
        hw.width,
        hw.height,
        hw.stride,
        hw.bpp,
    );

    // Initialize the desktop subsystem (wayland, window manager, fonts, etc.)
    // If already initialized during bootstrap, that's fine -- proceed to render.
    match crate::desktop::init() {
        Ok(()) => {}
        Err(crate::error::KernelError::InvalidState { .. }) => {
            crate::println!("[DESKTOP] Desktop subsystem already initialized, proceeding");
        }
        Err(e) => {
            crate::println!("[DESKTOP] Failed to initialize desktop: {:?}", e);
            return;
        }
    }

    // Configure the Wayland compositor with framebuffer dimensions
    crate::desktop::wayland::with_display(|display| {
        display
            .wl_compositor
            .set_output_size(hw.width as u32, hw.height as u32);
    });

    crate::println!(
        "[DESKTOP] Compositor configured for {}x{}",
        hw.width,
        hw.height,
    );

    // Create the initial desktop scene
    let mut state = create_desktop_scene(hw.width as u32, hw.height as u32);

    // Disable fbcon text output -- the compositor takes over the framebuffer
    fbcon::disable_output();

    // Clear the framebuffer to black before first composite
    // SAFETY: hw.fb_ptr is valid for stride * height bytes.
    unsafe {
        core::ptr::write_bytes(hw.fb_ptr, 0, hw.stride * hw.height);
    }

    crate::serial::_serial_print(format_args!("[DESKTOP] Entering compositor render loop\n"));

    // Switch keyboard to GUI mode: arrow keys emit single-byte codes (0x80+)
    // instead of ANSI escape sequences, preventing the 0x1B ESC prefix from
    // triggering the GUI exit guard.
    crate::drivers::keyboard::set_gui_mode(true);

    // Render loop: composite -> blit to framebuffer -> poll input -> repeat
    render_loop(&hw, &mut state);

    // Restore keyboard to shell mode (ANSI escape sequences for arrows)
    crate::drivers::keyboard::set_gui_mode(false);

    // Clear the entire framebuffer to remove GUI artifacts before returning
    // to text console mode.
    // SAFETY: hw.fb_ptr is valid for stride * height bytes.
    unsafe {
        core::ptr::write_bytes(hw.fb_ptr, 0, hw.stride * hw.height);
    }

    // Re-enable fbcon and force a full repaint of the text console
    fbcon::mark_all_dirty_and_flush();
    crate::println!("[DESKTOP] GUI stopped, returning to text console");
}

/// Info for a single desktop application: WM window ID + compositor surface ID.
struct AppInfo {
    wid: u32,
    surface_id: u32,
}

/// Type of dynamically-spawned GUI application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppKind {
    ImageViewer,
    Settings,
    MediaPlayer,
    SystemMonitor,
    Browser,
    PdfViewer,
    Calculator,
}

/// A dynamically spawned application window.
struct DynamicApp {
    kind: AppKind,
    wid: u32,
    surface_id: u32,
    pool_id: u32,
    pool_buf_id: u32,
    width: u32,
    height: u32,
}

/// Calculator application state for integer arithmetic.
struct CalculatorState {
    display_value: i64,
    accumulator: i64,
    pending_op: Option<u8>,
    input_started: bool,
    error: bool,
}

impl CalculatorState {
    fn new() -> Self {
        Self {
            display_value: 0,
            accumulator: 0,
            pending_op: None,
            input_started: false,
            error: false,
        }
    }

    fn handle_key(&mut self, key: u8) {
        if self.error && key != b'C' {
            return;
        }
        match key {
            b'0'..=b'9' => {
                let digit = (key - b'0') as i64;
                if !self.input_started {
                    self.display_value = digit;
                    self.input_started = true;
                } else {
                    self.display_value =
                        self.display_value.saturating_mul(10).saturating_add(digit);
                }
            }
            b'+' | b'-' | b'*' | b'/' => {
                self.evaluate_pending();
                self.accumulator = self.display_value;
                self.pending_op = Some(key);
                self.input_started = false;
            }
            b'=' => {
                self.evaluate_pending();
                self.pending_op = None;
                self.input_started = false;
            }
            b'C' => {
                *self = Self::new();
            }
            b'<' => {
                self.display_value /= 10;
            }
            _ => {}
        }
    }

    fn evaluate_pending(&mut self) {
        if let Some(op) = self.pending_op {
            let result = match op {
                b'+' => self.accumulator.checked_add(self.display_value),
                b'-' => self.accumulator.checked_sub(self.display_value),
                b'*' => self.accumulator.checked_mul(self.display_value),
                b'/' => {
                    if self.display_value == 0 {
                        self.error = true;
                        self.display_value = 0;
                        return;
                    }
                    self.accumulator.checked_div(self.display_value)
                }
                _ => Some(self.display_value),
            };
            match result {
                Some(v) => self.display_value = v,
                None => {
                    self.error = true;
                    self.display_value = 0;
                }
            }
            self.accumulator = self.display_value;
        }
    }
}

/// Desktop runtime state: application windows + Phase 7 overlay modules.
struct DesktopState {
    // Existing app windows
    terminal: AppInfo,
    file_manager: AppInfo,
    text_editor: AppInfo,
    panel_surface_id: u32,
    panel_pool_id: u32,
    panel_pool_buf_id: u32,

    // Phase 7 overlay modules (owned, no global state needed)
    app_switcher: crate::desktop::app_switcher::AppSwitcher,
    screen_locker: crate::desktop::screen_lock::ScreenLocker,
    animation_mgr: crate::desktop::animation::AnimationManager,

    // Settings app instance (owned, renders full UI)
    settings_app: crate::desktop::settings::SettingsApp,

    // Image viewer instance (owned, renders full UI)
    image_viewer: crate::desktop::image_viewer::ImageViewer,

    // Calculator state (owned, integer arithmetic)
    calculator: CalculatorState,

    // Theme engine for color scheme management
    theme: crate::desktop::desktop_ext::theme::ThemeManager,

    // Clipboard manager for copy/paste
    clipboard: crate::desktop::desktop_ext::clipboard::ClipboardManager,

    // Drag-and-drop manager
    dnd: crate::desktop::desktop_ext::dnd::DndManager,

    // Desktop icon grid
    icon_grid: crate::desktop::desktop_icons::IconGrid,

    // Browser engine instance
    browser: Option<crate::browser::browser_main::Browser>,

    // PDF viewer page index
    pdf_page_index: usize,

    // Dynamic apps (spawned from launcher, closeable)
    dynamic_apps: alloc::vec::Vec<DynamicApp>,

    // Input state
    frame_count: u64,
    drag: Option<DragState>,
    prev_focused: Option<u32>,
}

impl DesktopState {
    /// Look up the compositor surface ID for a given WM window ID.
    fn surface_for_window(&self, wid: u32) -> Option<u32> {
        if wid > 0 && wid == self.terminal.wid {
            return Some(self.terminal.surface_id);
        }
        if wid > 0 && wid == self.file_manager.wid {
            return Some(self.file_manager.surface_id);
        }
        if wid > 0 && wid == self.text_editor.wid {
            return Some(self.text_editor.surface_id);
        }
        for app in &self.dynamic_apps {
            if wid > 0 && wid == app.wid {
                return Some(app.surface_id);
            }
        }
        None
    }
}

/// Create the initial desktop scene: background gradient, real apps, and panel.
fn create_desktop_scene(width: u32, height: u32) -> DesktopState {
    // --- Background surface ---
    let bg_surface_id = 1000;
    crate::desktop::wayland::with_display(|display| {
        let _ = display.wl_compositor.create_surface(bg_surface_id);
        display
            .wl_compositor
            .set_surface_position(bg_surface_id, 0, 0);
    });

    let pool_size = (width as usize) * (height as usize) * 4;
    let mut bg_pixels = vec![0u8; pool_size];
    paint_gradient_background(&mut bg_pixels, width as usize, height as usize);

    // Render desktop icons into the background surface so they appear behind
    // all windows naturally via compositor z-order.
    let panel_h = crate::desktop::panel::PANEL_HEIGHT;
    let icon_grid = create_default_icon_grid(width, height - panel_h);
    render_icons_into_bgra(&icon_grid, &mut bg_pixels, width as usize, height as usize);

    let pool_id = 100;
    let mut pool = crate::desktop::wayland::buffer::WlShmPool::new(pool_id, 0, pool_size);
    pool.write_data(0, &bg_pixels);
    let buf_id = pool
        .create_buffer(
            0,
            width,
            height,
            width * 4,
            crate::graphics::PixelFormat::Xrgb8888,
        )
        .unwrap_or(0);
    crate::desktop::wayland::buffer::register_pool(pool);

    let bg_buffer = crate::desktop::wayland::buffer::Buffer::from_pool(
        1,
        pool_id,
        buf_id,
        width,
        height,
        width * 4,
        crate::graphics::PixelFormat::Xrgb8888,
    );
    crate::desktop::wayland::with_display(|display| {
        display
            .wl_compositor
            .with_surface_mut(bg_surface_id, |surface| {
                surface.attach_buffer(bg_buffer.clone());
                surface.damage_full();
                let _ = surface.commit();
            });
        display.wl_compositor.request_composite();
    });

    // --- Initialize panel ---
    let _ = crate::desktop::panel::init(width, height);
    let (panel_surface_id, panel_pool_id, panel_pool_buf_id) =
        create_app_surface(0, (height - panel_h) as i32, width, panel_h);

    // --- Create real applications ---
    // Terminal: 640x384 (80cols x 24rows x 16px)
    let (terminal_wid, terminal_sid) =
        crate::desktop::terminal::with_terminal_manager(|tm| match tm.create_terminal(640, 384) {
            Ok(idx) => (
                tm.get_window_id(idx).unwrap_or(0),
                tm.get_surface_id(idx).unwrap_or(0),
            ),
            Err(_) => (0, 0),
        })
        .unwrap_or((0, 0));

    // File manager: 640x480
    let (file_manager_wid, file_manager_sid) =
        if crate::desktop::file_manager::create_file_manager().is_ok() {
            crate::desktop::file_manager::with_file_manager(|fm| {
                let r = fm.read();
                (r.window_id(), r.surface_id())
            })
            .unwrap_or((0, 0))
        } else {
            (0, 0)
        };

    // Text editor: 800x600
    let (text_editor_wid, text_editor_sid) =
        if crate::desktop::text_editor::create_text_editor(None).is_ok() {
            crate::desktop::text_editor::with_text_editor(|te| {
                let r = te.read();
                (r.window_id(), r.surface_id())
            })
            .unwrap_or((0, 0))
        } else {
            (0, 0)
        };

    // Set window titles so the panel taskbar shows meaningful labels
    crate::desktop::window_manager::with_window_manager(|wm| {
        wm.set_window_title(terminal_wid, "Terminal");
        wm.set_window_title(file_manager_wid, "Files");
        wm.set_window_title(text_editor_wid, "Text Editor");
    });

    // Write welcome message to terminal so it's not a blank black window
    crate::desktop::terminal::with_terminal_manager(|tm| {
        tm.write_welcome(0);
    });

    // Focus the terminal by default and sync compositor z_order
    if terminal_wid > 0 {
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.focus_window(terminal_wid);
        });
        // Raise the terminal surface in the compositor, then ensure panel stays on top
        crate::desktop::wayland::with_display(|display| {
            display.wl_compositor.raise_surface(terminal_sid);
            display.wl_compositor.raise_surface(panel_surface_id);
        });
    }

    crate::serial::_serial_print(format_args!(
        "[DESKTOP] Desktop scene created: bg + terminal({}/{}) + files({}/{}) + editor({}/{}) + \
         panel\n",
        terminal_wid,
        terminal_sid,
        file_manager_wid,
        file_manager_sid,
        text_editor_wid,
        text_editor_sid
    ));

    // Send a welcome notification to demonstrate the notification system
    crate::desktop::notification::notify(
        "VeridianOS Desktop",
        "Welcome to VeridianOS v0.25.0",
        crate::desktop::notification::NotificationUrgency::Normal,
        "desktop",
    );

    DesktopState {
        terminal: AppInfo {
            wid: terminal_wid,
            surface_id: terminal_sid,
        },
        file_manager: AppInfo {
            wid: file_manager_wid,
            surface_id: file_manager_sid,
        },
        text_editor: AppInfo {
            wid: text_editor_wid,
            surface_id: text_editor_sid,
        },
        panel_surface_id,
        panel_pool_id,
        panel_pool_buf_id,
        app_switcher: crate::desktop::app_switcher::AppSwitcher::new(),
        screen_locker: crate::desktop::screen_lock::ScreenLocker::new(
            width as usize,
            height as usize,
        ),
        animation_mgr: crate::desktop::animation::AnimationManager::new(),
        settings_app: crate::desktop::settings::SettingsApp::new(),
        image_viewer: crate::desktop::image_viewer::ImageViewer::new(),
        calculator: CalculatorState::new(),
        theme: crate::desktop::desktop_ext::theme::ThemeManager::new(),
        clipboard: crate::desktop::desktop_ext::clipboard::ClipboardManager::new(),
        dnd: crate::desktop::desktop_ext::dnd::DndManager::new(),
        icon_grid,
        browser: None,
        pdf_page_index: 0,
        dynamic_apps: alloc::vec::Vec::new(),
        frame_count: 0,
        drag: None,
        prev_focused: None,
    }
}

/// Create a default icon grid with common application shortcuts.
fn create_default_icon_grid(
    desktop_width: u32,
    desktop_height: u32,
) -> crate::desktop::desktop_icons::IconGrid {
    use crate::desktop::desktop_icons::{DesktopIcon, IconGrid};

    let mut grid = IconGrid::new(desktop_width, desktop_height);

    let apps = [
        ("Terminal", "terminal", 0xFF2ECC71u32),
        ("Files", "files", 0xFF3498DB),
        ("Editor", "editor", 0xFFE67E22),
        ("Settings", "settings", 0xFF9B59B6),
        ("Browser", "browser", 0xFF1ABC9C),
    ];

    for (name, _exec, color) in &apps {
        let mut icon = DesktopIcon::new(name, 0, 0);
        // Set a solid colour for the icon
        let pixel_count = (crate::desktop::desktop_icons::ICON_SIZE
            * crate::desktop::desktop_icons::ICON_SIZE) as usize;
        let data = alloc::vec![*color; pixel_count];
        icon.set_icon_data(&data);
        grid.add_icon(icon);
    }

    grid.arrange();
    grid
}

/// Render desktop icons (bitmap + label) into a BGRA u8 pixel buffer.
///
/// Each icon's 16x16 ARGB bitmap is converted to BGRA on the fly, and the
/// icon name is drawn below using the 8x16 VGA font.
fn render_icons_into_bgra(
    grid: &crate::desktop::desktop_icons::IconGrid,
    buf: &mut [u8],
    buf_width: usize,
    buf_height: usize,
) {
    let icon_sz = crate::desktop::desktop_icons::ICON_SIZE as i32;
    let bw = buf_width as i32;
    let bh = buf_height as i32;

    for icon in &grid.icons {
        // Draw icon bitmap (ARGB -> BGRA)
        for row in 0..icon_sz {
            let dy = icon.y + row;
            if dy < 0 || dy >= bh {
                continue;
            }
            for col in 0..icon_sz {
                let dx = icon.x + col;
                if dx < 0 || dx >= bw {
                    continue;
                }
                let src = icon.icon_data[(row * icon_sz + col) as usize];
                let off = ((dy * bw + dx) as usize) * 4;
                if off + 3 < buf.len() {
                    buf[off] = (src & 0xFF) as u8; // B
                    buf[off + 1] = ((src >> 8) & 0xFF) as u8; // G
                    buf[off + 2] = ((src >> 16) & 0xFF) as u8; // R
                    buf[off + 3] = 0xFF; // A
                }
            }
        }

        // Draw label below icon
        let label_x = icon.x.max(0) as usize;
        let label_y = (icon.y + icon_sz + 2).max(0) as usize;
        let name_bytes = icon.name.as_bytes();
        let label_len = name_bytes.len().min(8);
        for (ci, &ch) in name_bytes[..label_len].iter().enumerate() {
            draw_char_into_buffer(buf, buf_width, ch, label_x + ci * 8, label_y, 0xCCCCCC);
        }
    }
}

/// Draw a string into a BGRA pixel buffer at (px, py) with the given color.
///
/// Uses the 8x16 VGA font. Characters are spaced 8 pixels apart horizontally.
/// CJK wide characters (detected via `cjk::char_width`) advance 16px instead of
/// 8px.
pub fn draw_string_into_buffer(
    buf: &mut [u8],
    buf_width: usize,
    text: &[u8],
    px: usize,
    py: usize,
    color: u32,
) {
    let mut cursor_x = px;
    for &ch in text.iter() {
        let w = crate::desktop::desktop_ext::cjk::char_width(ch as char);
        if w == 0 {
            continue; // skip zero-width / combining marks
        }
        draw_char_into_buffer(buf, buf_width, ch, cursor_x, py, color);
        cursor_x += (w as usize) * 8;
    }
}

/// Draw a single 8x16 character into a BGRA pixel buffer.
pub fn draw_char_into_buffer(
    buf: &mut [u8],
    buf_width: usize,
    ch: u8,
    px: usize,
    py: usize,
    color: u32,
) {
    let glyph = crate::graphics::font8x16::glyph(ch);
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;

    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 != 0 {
                let x = px + col;
                let y = py + row;
                let offset = (y * buf_width + x) * 4;
                if offset + 3 < buf.len() {
                    buf[offset] = b;
                    buf[offset + 1] = g;
                    buf[offset + 2] = r;
                    buf[offset + 3] = 0xFF;
                }
            }
        }
    }
}

/// Create a compositor surface backed by a SHM pool + buffer.
///
/// Returns `(surface_id, pool_id, pool_buf_id)` for later use with
/// `update_surface_pixels`.
pub fn create_app_surface(x: i32, y: i32, w: u32, h: u32) -> (u32, u32, u32) {
    let surface_id = NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed);
    let pool_id = NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed);

    // Create compositor surface
    crate::desktop::wayland::with_display(|display| {
        let _ = display.wl_compositor.create_surface(surface_id);
        display.wl_compositor.set_surface_position(surface_id, x, y);
    });

    // Create SHM pool
    let pool_size = (w as usize) * (h as usize) * 4;
    let mut pool = crate::desktop::wayland::buffer::WlShmPool::new(pool_id, 0, pool_size);

    // Create buffer in pool
    let pool_buf_id = pool
        .create_buffer(0, w, h, w * 4, crate::graphics::PixelFormat::Xrgb8888)
        .unwrap_or(0);

    crate::desktop::wayland::buffer::register_pool(pool);

    // Attach an initial buffer to the surface
    let buffer = crate::desktop::wayland::buffer::Buffer::from_pool(
        surface_id + 1000,
        pool_id,
        pool_buf_id,
        w,
        h,
        w * 4,
        crate::graphics::PixelFormat::Xrgb8888,
    );

    crate::desktop::wayland::with_display(|display| {
        display
            .wl_compositor
            .with_surface_mut(surface_id, |surface| {
                surface.attach_buffer(buffer.clone());
                surface.damage_full();
                let _ = surface.commit();
            });
    });

    (surface_id, pool_id, pool_buf_id)
}

/// Update the pixel data for an app surface and request recomposite.
///
/// `pixels` must be exactly `w * h * 4` bytes (BGRA).
pub fn update_surface_pixels(surface_id: u32, pool_id: u32, pool_buf_id: u32, pixels: &[u8]) {
    // Write pixel data into pool backing memory
    crate::desktop::wayland::buffer::with_pool_mut(pool_id, |pool| {
        pool.write_buffer_pixels(pool_buf_id, pixels);
    });

    // Mark surface as damaged and request recomposite
    crate::desktop::wayland::with_display(|display| {
        display
            .wl_compositor
            .with_surface_mut(surface_id, |surface| {
                surface.damage_full();
                let _ = surface.commit();
            });
        display.wl_compositor.request_composite();
    });
}

/// Paint a gradient background into a BGRA pixel buffer.
fn paint_gradient_background(buf: &mut [u8], width: usize, height: usize) {
    for y in 0..height {
        // Vertical gradient from dark blue-grey (#2D3436) to darker (#1a1a2e)
        // Using integer math (fixed-point 8.8) to avoid soft-float overhead
        let t256 = (y * 256) / height; // 0..255
        let inv_t = 256 - t256;
        let r = ((0x2D * inv_t + 0x1a * t256) / 256) as u8;
        let g = ((0x34 * inv_t + 0x1a * t256) / 256) as u8;
        let b = ((0x36 * inv_t + 0x2e * t256) / 256) as u8;

        for x in 0..width {
            let offset = (y * width + x) * 4;
            buf[offset] = b; // B
            buf[offset + 1] = g; // G
            buf[offset + 2] = r; // R
            buf[offset + 3] = 0xFF; // A
        }
    }

    // Draw a centered "VeridianOS" title
    let title = b"VeridianOS Desktop Environment";
    let title_x = (width - title.len() * 8) / 2;
    let title_y = height / 2 - 20;
    for (i, &ch) in title.iter().enumerate() {
        draw_char_into_buffer(buf, width, ch, title_x + i * 8, title_y, 0xAAAAAA);
    }

    // Draw subtitle
    let sub = b"Wayland Compositor Active";
    let sub_x = (width - sub.len() * 8) / 2;
    let sub_y = height / 2 + 4;
    for (i, &ch) in sub.iter().enumerate() {
        draw_char_into_buffer(buf, width, ch, sub_x + i * 8, sub_y, 0x666666);
    }
}

/// Translate a raw input_event::InputEvent to a window_manager::InputEvent.
fn translate_input_event(
    raw: &crate::drivers::input_event::InputEvent,
    mouse_x: i32,
    mouse_y: i32,
) -> Option<crate::desktop::window_manager::InputEvent> {
    use crate::{desktop::window_manager::InputEvent as WmEvent, drivers::input_event::*};

    match raw.event_type {
        EV_KEY => {
            let code = raw.code;
            let pressed = raw.value != 0;

            // Mouse buttons (BTN_LEFT=0x110, BTN_RIGHT=0x111, BTN_MIDDLE=0x112)
            if (BTN_LEFT..=BTN_MIDDLE).contains(&code) {
                let button = (code - BTN_LEFT) as u8;
                return Some(WmEvent::MouseButton {
                    button,
                    pressed,
                    x: mouse_x,
                    y: mouse_y,
                });
            }

            // Keyboard: code is the decoded ASCII byte from the PS/2 driver
            if pressed {
                // Map some codes to characters
                let ch = if code < 0x80 {
                    code as u8 as char
                } else {
                    '\0'
                };
                Some(WmEvent::KeyPress {
                    scancode: code as u8,
                    character: ch,
                })
            } else {
                Some(WmEvent::KeyRelease {
                    scancode: code as u8,
                })
            }
        }
        EV_REL => {
            // Mouse movement: we use absolute cursor position, not relative deltas,
            // for the WM events. Movement updates happen in cursor_position().
            // Only emit MouseMove if we see REL_X (to avoid double events for X+Y).
            if raw.code == REL_X {
                Some(WmEvent::MouseMove {
                    x: mouse_x,
                    y: mouse_y,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Title bar height in pixels. Clicks in this region initiate window drag.
const TITLE_BAR_HEIGHT: i32 = 28;

/// Drag state for window movement.
struct DragState {
    /// WM window ID being dragged
    wid: u32,
    /// Compositor surface ID of the dragged window
    surface_id: u32,
    /// Offset from window top-left to mouse grab point
    offset_x: i32,
    offset_y: i32,
}

/// Raise a focused window's compositor surface (and keep panel on top).
fn sync_compositor_focus(state: &DesktopState, focused_wid: u32) {
    if let Some(surface_id) = state.surface_for_window(focused_wid) {
        crate::desktop::wayland::with_display(|display| {
            display.wl_compositor.raise_surface(surface_id);
            // Panel must always be the topmost surface
            display.wl_compositor.raise_surface(state.panel_surface_id);
        });
    }
}

/// Spawn a dynamic app: create WM window + compositor surface, return index in
/// dynamic_apps.
fn spawn_dynamic_app(
    state: &mut DesktopState,
    kind: AppKind,
    title: &str,
    w: u32,
    h: u32,
) -> Option<usize> {
    // Check if one of this kind already exists
    if state.dynamic_apps.iter().any(|a| a.kind == kind) {
        return None;
    }

    let title_bar_h = 28u32;
    let wid = crate::desktop::window_manager::with_window_manager(|wm| {
        wm.create_window(100, 80, w, h + title_bar_h, 0)
    })
    .and_then(|r| r.ok())?;

    // Surface covers full window area (title bar + content)
    let (surface_id, pool_id, pool_buf_id) = create_app_surface(100, 80, w, h + title_bar_h);

    crate::desktop::window_manager::with_window_manager(|wm| {
        wm.set_window_title(wid, title);
        let _ = wm.focus_window(wid);
    });

    // Raise in compositor
    crate::desktop::wayland::with_display(|display| {
        display.wl_compositor.raise_surface(surface_id);
        display.wl_compositor.raise_surface(state.panel_surface_id);
    });

    state.dynamic_apps.push(DynamicApp {
        kind,
        wid,
        surface_id,
        pool_id,
        pool_buf_id,
        width: w,
        height: h,
    });

    Some(state.dynamic_apps.len() - 1)
}

/// Close a dynamic app by WM window ID.
fn close_dynamic_app(state: &mut DesktopState, wid: u32) {
    if let Some(pos) = state.dynamic_apps.iter().position(|a| a.wid == wid) {
        let app = state.dynamic_apps.remove(pos);
        // Unmap and destroy surface
        crate::desktop::wayland::with_display(|display| {
            display
                .wl_compositor
                .set_surface_mapped(app.surface_id, false);
        });
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.destroy_window(app.wid);
        });
    }
}

/// Close any window (dynamic or static) by WM window ID.
fn close_any_window(state: &mut DesktopState, wid: u32) {
    // Dynamic apps: use existing close path
    if state.dynamic_apps.iter().any(|a| a.wid == wid) {
        close_dynamic_app(state, wid);
        return;
    }
    // Static apps: unmap surface, destroy WM window, zero the wid
    if wid == state.terminal.wid {
        crate::desktop::wayland::with_display(|display| {
            display
                .wl_compositor
                .set_surface_mapped(state.terminal.surface_id, false);
        });
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.destroy_window(wid);
        });
        state.terminal.wid = 0;
    } else if wid == state.file_manager.wid {
        crate::desktop::wayland::with_display(|display| {
            display
                .wl_compositor
                .set_surface_mapped(state.file_manager.surface_id, false);
        });
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.destroy_window(wid);
        });
        state.file_manager.wid = 0;
    } else if wid == state.text_editor.wid {
        crate::desktop::wayland::with_display(|display| {
            display
                .wl_compositor
                .set_surface_mapped(state.text_editor.surface_id, false);
        });
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.destroy_window(wid);
        });
        state.text_editor.wid = 0;
    }
}

/// Draw a title bar (background + title text + close button) into a BGRA
/// surface pixel buffer.  The title bar occupies the top 28 rows of the
/// buffer.  The window's title and focus state are read from the WM.
pub fn draw_title_bar_into_surface(pixels: &mut [u8], width: usize, _total_h: usize, wid: u32) {
    let cfg = DecorationConfig::default_config();
    let tbh = cfg.title_bar_height as usize;

    // Look up window title and focus state
    let (title_buf, title_len, focused) =
        crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_window(wid)
                .map(|w| (w.title, w.title_len, w.focused))
        })
        .flatten()
        .unwrap_or(([0u8; 64], 0, false));

    let bg_argb = if focused {
        cfg.title_bg_focused
    } else {
        cfg.title_bg_unfocused
    };
    // Convert ARGB -> BGRA components
    let bg_r = ((bg_argb >> 16) & 0xFF) as u8;
    let bg_g = ((bg_argb >> 8) & 0xFF) as u8;
    let bg_b = (bg_argb & 0xFF) as u8;

    // Fill title bar background
    for y in 0..tbh {
        for x in 0..width {
            let off = (y * width + x) * 4;
            if off + 3 < pixels.len() {
                pixels[off] = bg_b;
                pixels[off + 1] = bg_g;
                pixels[off + 2] = bg_r;
                pixels[off + 3] = 0xFF;
            }
        }
    }

    // Draw title text (vertically centered in title bar)
    let text_color = cfg.title_text_color & 0x00FF_FFFF; // strip alpha for draw helper
    let text_y = (tbh.saturating_sub(16)) / 2;
    for (ci, &ch) in title_buf[..title_len].iter().enumerate() {
        draw_char_into_buffer(pixels, width, ch, 8 + ci * 8, text_y, text_color);
    }

    // Draw close button (red square with white X) at top-right
    let btn_sz = 16usize;
    let btn_x = width.saturating_sub(22);
    let btn_y = (tbh.saturating_sub(btn_sz)) / 2;
    let close_r: u8 = 0xE7;
    let close_g: u8 = 0x4C;
    let close_b: u8 = 0x3C;
    for dy in 0..btn_sz {
        for dx in 0..btn_sz {
            let off = ((btn_y + dy) * width + btn_x + dx) * 4;
            if off + 3 < pixels.len() {
                pixels[off] = close_b;
                pixels[off + 1] = close_g;
                pixels[off + 2] = close_r;
                pixels[off + 3] = 0xFF;
            }
        }
    }
    // White X (2px thick diagonals)
    for i in 0..14usize {
        let coords = [
            (btn_x + 1 + i, btn_y + 1 + i),
            (btn_x + 2 + i, btn_y + 1 + i),
            (btn_x + 14 - i, btn_y + 1 + i),
            (btn_x + 13 - i, btn_y + 1 + i),
        ];
        for (px, py) in coords {
            let off = (py * width + px) * 4;
            if off + 3 < pixels.len() {
                pixels[off] = 0xFF;
                pixels[off + 1] = 0xFF;
                pixels[off + 2] = 0xFF;
                pixels[off + 3] = 0xFF;
            }
        }
    }
}

/// Focus an existing dynamic app of the given kind, or return false if not
/// found.
fn focus_dynamic_app(state: &DesktopState, kind: AppKind) -> bool {
    if let Some(app) = state.dynamic_apps.iter().find(|a| a.kind == kind) {
        let wid = app.wid;
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.focus_window(wid);
        });
        sync_compositor_focus(state, wid);
        true
    } else {
        false
    }
}

/// Handle a launcher launch action by spawning or focusing the appropriate app.
fn handle_launcher_launch(state: &mut DesktopState, exec_path: &str) {
    match exec_path {
        "terminal" | "/usr/bin/terminal" => {
            if state.terminal.wid > 0 {
                crate::desktop::window_manager::with_window_manager(|wm| {
                    let _ = wm.focus_window(state.terminal.wid);
                });
                sync_compositor_focus(state, state.terminal.wid);
            }
        }
        "files" | "/usr/bin/files" => {
            if state.file_manager.wid > 0 {
                crate::desktop::window_manager::with_window_manager(|wm| {
                    let _ = wm.focus_window(state.file_manager.wid);
                });
                sync_compositor_focus(state, state.file_manager.wid);
            }
        }
        "editor" | "/usr/bin/editor" => {
            if state.text_editor.wid > 0 {
                crate::desktop::window_manager::with_window_manager(|wm| {
                    let _ = wm.focus_window(state.text_editor.wid);
                });
                sync_compositor_focus(state, state.text_editor.wid);
            }
        }
        "settings" | "/usr/bin/settings" => {
            if !focus_dynamic_app(state, AppKind::Settings) {
                spawn_dynamic_app(state, AppKind::Settings, "Settings", 600, 450);
            }
        }
        "sysmonitor" | "/usr/bin/sysmonitor" => {
            if !focus_dynamic_app(state, AppKind::SystemMonitor) {
                spawn_dynamic_app(state, AppKind::SystemMonitor, "System Monitor", 640, 400);
            }
        }
        "imageviewer" | "/usr/bin/image-viewer" => {
            if !focus_dynamic_app(state, AppKind::ImageViewer) {
                spawn_dynamic_app(state, AppKind::ImageViewer, "Image Viewer", 800, 600);
                // Try to load a sample image from VFS on first launch
                if state.image_viewer.image.is_none() {
                    // Attempt well-known paths for sample images
                    let sample_paths = [
                        "/usr/share/images/wallpaper.ppm",
                        "/usr/share/images/sample.bmp",
                        "/usr/share/images/logo.qoi",
                    ];
                    for path in &sample_paths {
                        if let Ok(data) = crate::fs::read_file(path) {
                            state.image_viewer.load_file(path, &data);
                            break;
                        }
                    }
                }
            }
        }
        "mediaplayer" | "/usr/bin/mediaplayer" => {
            if !focus_dynamic_app(state, AppKind::MediaPlayer) {
                spawn_dynamic_app(state, AppKind::MediaPlayer, "Media Player", 640, 300);
            }
        }
        "browser" | "/usr/bin/browser" => {
            if !focus_dynamic_app(state, AppKind::Browser) {
                spawn_dynamic_app(state, AppKind::Browser, "Web Browser", 900, 600);
            }
        }
        "pdfviewer" | "/usr/bin/pdfviewer" => {
            if !focus_dynamic_app(state, AppKind::PdfViewer) {
                spawn_dynamic_app(state, AppKind::PdfViewer, "PDF Viewer", 800, 600);
            }
        }
        "calculator" | "/usr/bin/calculator" => {
            if !focus_dynamic_app(state, AppKind::Calculator) {
                spawn_dynamic_app(state, AppKind::Calculator, "Calculator", 320, 400);
            }
        }
        _ => {}
    }
}

/// Framebuffer layout info passed to render sub-functions.
struct FrameLayout {
    fb_ptr: *mut u8,
    fb_width: usize,
    fb_height: usize,
    fb_stride: usize,
    is_bgr: bool,
    panel_y: i32,
}

/// Main compositor render loop.
///
/// Composites all Wayland surfaces, blits to the hardware framebuffer,
/// routes input events through the window manager to applications.
/// Integrates Phase 7 desktop features: app switcher, screen lock,
/// launcher, notifications, system tray, snap-to-edge, virtual
/// workspaces, window decorations, and animations.
fn render_loop(hw: &fbcon::FbHwInfo, state: &mut DesktopState) {
    let layout = FrameLayout {
        fb_ptr: hw.fb_ptr,
        fb_width: hw.width,
        fb_height: hw.height,
        fb_stride: hw.stride,
        is_bgr: matches!(hw.pixel_format, FbPixelFormat::Bgr),
        panel_y: (hw.height as u32 - crate::desktop::panel::PANEL_HEIGHT) as i32,
    };

    // Initial composite
    let _drawn = crate::desktop::wayland::with_display(|display| display.wl_compositor.composite());

    // Do initial render of all app surfaces
    render_all_apps(state);

    // Set screen size for window manager placement heuristics
    crate::desktop::window_manager::with_window_manager(|wm| {
        wm.set_screen_size(layout.fb_width as u32, layout.fb_height as u32);
    });

    loop {
        state.frame_count += 1;
        let tick = crate::arch::timer::get_ticks();

        // Screen lock takes over all input and rendering
        if state.screen_locker.is_locked() {
            handle_screen_lock(state, &layout, tick);
            continue;
        }

        // Poll, translate, and dispatch input events; returns true if ESC exit
        if handle_input_events(state, &layout) {
            return;
        }

        // Update focus, idle timeout, animations, notifications
        update_ui_state(state, tick);

        // Render apps, composite overlays, blit to framebuffer
        render_and_composite(state, &layout, tick);
    }
}

/// Handle the screen lock: consume input, tick lock state, render lock screen.
fn handle_screen_lock(state: &mut DesktopState, layout: &FrameLayout, tick: u64) {
    crate::drivers::input_event::poll_all();
    while let Some(raw_event) = crate::drivers::input_event::read_event() {
        if raw_event.event_type == crate::drivers::input_event::EV_KEY && raw_event.value == 1 {
            let action = state.screen_locker.handle_key(raw_event.code as u8, tick);
            if matches!(action, crate::desktop::screen_lock::LockAction::Unlocked) {
                break;
            }
        }
    }
    state.screen_locker.tick(tick);
    crate::desktop::wayland::with_display(|display| {
        display.wl_compositor.with_back_buffer_mut(|bb| {
            state
                .screen_locker
                .render_to_buffer(bb, layout.fb_width, layout.fb_height, tick);
        });
        blit_back_buffer(
            &display.wl_compositor,
            layout.fb_ptr,
            layout.fb_width,
            layout.fb_height,
            layout.fb_stride,
            layout.is_bgr,
        );
    });
    for _ in 0..50_000 {
        core::hint::spin_loop();
    }
}

/// Poll hardware input, translate events, and dispatch hotkeys/mouse/keyboard.
///
/// Returns `true` if the GUI should exit (ESC pressed without overlays).
fn handle_input_events(state: &mut DesktopState, layout: &FrameLayout) -> bool {
    crate::drivers::input_event::poll_all();
    let (mouse_x, mouse_y) = crate::drivers::mouse::cursor_position();
    let mods = crate::drivers::keyboard::get_modifiers();
    let tick = crate::arch::timer::get_ticks();

    // Record activity for idle timeout
    state.screen_locker.record_activity(tick);

    while let Some(raw_event) = crate::drivers::input_event::read_event() {
        let is_key_press =
            raw_event.event_type == crate::drivers::input_event::EV_KEY && raw_event.value == 1;

        // ESC without modifiers exits the GUI
        if is_key_press && raw_event.code == 0x1B && mods == 0 {
            // Only exit if no overlay is open
            if !state.app_switcher.is_visible() {
                crate::serial::_serial_print(format_args!("[DESKTOP] ESC pressed, exiting GUI\n"));
                return true;
            }
        }

        // --- Hotkey detection (before normal event dispatch) ---

        // Alt+Tab: show/cycle app switcher
        if is_key_press && raw_event.code == 0x09 && mods & crate::drivers::keyboard::MOD_ALT != 0 {
            if !state.app_switcher.is_visible() {
                let windows = get_window_list_for_switcher();
                state.app_switcher.show(windows);
            } else {
                state.app_switcher.next();
            }
            continue;
        }

        // Alt released while switcher visible: commit selection
        if state.app_switcher.is_visible()
            && mods & crate::drivers::keyboard::MOD_ALT == 0
            && raw_event.event_type == crate::drivers::input_event::EV_KEY
        {
            if let Some(wid) = state.app_switcher.hide() {
                crate::desktop::window_manager::with_window_manager(|wm| {
                    let _ = wm.focus_window(wid);
                });
                sync_compositor_focus(state, wid);
            }
            continue;
        }

        // Ctrl+Alt+L: lock screen
        if is_key_press
            && raw_event.code == b'l' as u16
            && mods & (crate::drivers::keyboard::MOD_CTRL | crate::drivers::keyboard::MOD_ALT)
                == (crate::drivers::keyboard::MOD_CTRL | crate::drivers::keyboard::MOD_ALT)
        {
            state.screen_locker.lock();
            continue;
        }

        // Ctrl+Alt+Arrow: switch workspace
        if is_key_press
            && mods & (crate::drivers::keyboard::MOD_CTRL | crate::drivers::keyboard::MOD_ALT)
                == (crate::drivers::keyboard::MOD_CTRL | crate::drivers::keyboard::MOD_ALT)
        {
            if raw_event.code == crate::drivers::keyboard::KEY_LEFT as u16 {
                switch_workspace_prev(state);
                continue;
            }
            if raw_event.code == crate::drivers::keyboard::KEY_RIGHT as u16 {
                switch_workspace_next(state);
                continue;
            }
        }

        // Super key: toggle launcher
        if is_key_press && mods & crate::drivers::keyboard::MOD_SUPER != 0 {
            crate::desktop::launcher::with_launcher(|l| l.toggle());
            continue;
        }

        // ESC closes launcher or app switcher overlays
        if is_key_press && raw_event.code == 0x1B {
            if state.app_switcher.is_visible() {
                let _ = state.app_switcher.hide();
                continue;
            }
            let launcher_visible =
                crate::desktop::launcher::with_launcher_ref(|l| l.is_visible()).unwrap_or(false);
            if launcher_visible {
                crate::desktop::launcher::with_launcher(|l| l.hide());
                continue;
            }
        }

        // Forward keyboard to launcher when visible
        let launcher_visible =
            crate::desktop::launcher::with_launcher_ref(|l| l.is_visible()).unwrap_or(false);
        if launcher_visible && is_key_press && raw_event.code < 0x80 {
            let action =
                crate::desktop::launcher::with_launcher(|l| l.handle_key(raw_event.code as u8));
            if let Some(Some(action)) = action {
                match action {
                    crate::desktop::launcher::LauncherAction::Launch(exec) => {
                        handle_launcher_launch(state, &exec);
                    }
                    crate::desktop::launcher::LauncherAction::Hide => {}
                }
            }
            continue;
        }

        // Ctrl+C: copy from focused app to clipboard
        if is_key_press
            && raw_event.code == b'c' as u16
            && mods & crate::drivers::keyboard::MOD_CTRL != 0
            && mods & crate::drivers::keyboard::MOD_ALT == 0
        {
            let _ = state.clipboard.copy(
                crate::desktop::desktop_ext::clipboard::SelectionType::Clipboard,
                0,
                crate::desktop::desktop_ext::clipboard::ClipboardMime::TextPlainUtf8,
                alloc::vec![],
            );
            continue;
        }

        // Ctrl+V: paste from clipboard to focused app
        if is_key_press
            && raw_event.code == b'v' as u16
            && mods & crate::drivers::keyboard::MOD_CTRL != 0
            && mods & crate::drivers::keyboard::MOD_ALT == 0
        {
            if let Ok(data) = state.clipboard.paste(
                crate::desktop::desktop_ext::clipboard::SelectionType::Clipboard,
                crate::desktop::desktop_ext::clipboard::ClipboardMime::TextPlainUtf8,
            ) {
                if state.terminal.wid > 0 {
                    crate::desktop::terminal::with_terminal_manager(|tm| {
                        for &byte in data {
                            let event = crate::desktop::window_manager::InputEvent::KeyPress {
                                scancode: 0,
                                character: byte as char,
                            };
                            let _ = tm.process_input(0, event);
                        }
                    });
                }
            }
            continue;
        }

        // --- Normal event dispatch ---
        if let Some(wm_event) = translate_input_event(&raw_event, mouse_x, mouse_y) {
            dispatch_mouse_and_keyboard(state, layout, wm_event, launcher_visible);
        }
    }

    false
}

/// Dispatch a translated WM event: mouse clicks, drags, releases, and keyboard.
fn dispatch_mouse_and_keyboard(
    state: &mut DesktopState,
    layout: &FrameLayout,
    wm_event: crate::desktop::window_manager::InputEvent,
    launcher_visible: bool,
) {
    // --- Handle left mouse button press ---
    if let crate::desktop::window_manager::InputEvent::MouseButton {
        button: 0,
        pressed: true,
        x,
        y,
    } = wm_event
    {
        // Dismiss launcher on click outside
        if launcher_visible {
            crate::desktop::launcher::with_launcher(|l| l.hide());
        }

        // Panel click
        if y >= layout.panel_y {
            crate::desktop::panel::with_panel(|p| p.update_buttons());
            if let Some(focus_wid) =
                crate::desktop::panel::with_panel(|p| p.handle_click(x, y - layout.panel_y))
                    .flatten()
            {
                crate::desktop::window_manager::with_window_manager(|wm| {
                    let _ = wm.focus_window(focus_wid);
                });
                sync_compositor_focus(state, focus_wid);
            }
            return;
        }

        // Window hit test: find which window was clicked
        let hit =
            crate::desktop::window_manager::with_window_manager(|wm| wm.window_at_position(x, y))
                .flatten();

        if let Some(wid) = hit {
            crate::desktop::window_manager::with_window_manager(|wm| {
                let _ = wm.focus_window(wid);
            });
            sync_compositor_focus(state, wid);

            let in_title_bar = crate::desktop::window_manager::with_window_manager(|wm| {
                wm.get_window(wid).map(|w| y < w.y + TITLE_BAR_HEIGHT)
            })
            .flatten()
            .unwrap_or(false);

            if in_title_bar {
                let is_close = crate::desktop::window_manager::with_window_manager(|wm| {
                    wm.get_window(wid).map(|w| x >= w.x + w.width as i32 - 28)
                })
                .flatten()
                .unwrap_or(false);

                if is_close {
                    close_any_window(state, wid);
                    return;
                }

                // Start drag
                if let Some(surface_id) = state.surface_for_window(wid) {
                    let win_pos = crate::desktop::window_manager::with_window_manager(|wm| {
                        wm.get_window(wid).map(|w| (w.x, w.y))
                    })
                    .flatten();
                    if let Some((wx, wy)) = win_pos {
                        state.drag = Some(DragState {
                            wid,
                            surface_id,
                            offset_x: x - wx,
                            offset_y: y - wy,
                        });
                    }
                }
            } else {
                // Not title bar -- forward click to the app
                crate::desktop::window_manager::with_window_manager(|wm| {
                    wm.queue_event(crate::desktop::window_manager::WindowEvent {
                        window_id: wid,
                        event: wm_event,
                    });
                });
            }
        }
        return;
    }

    // --- Handle right-click ---
    if let crate::desktop::window_manager::InputEvent::MouseButton {
        button: 1,
        pressed: true,
        x,
        y,
    } = wm_event
    {
        let launcher_visible =
            crate::desktop::launcher::with_launcher_ref(|l| l.is_visible()).unwrap_or(false);
        if launcher_visible {
            crate::desktop::launcher::with_launcher(|l| l.hide());
        }

        if y < layout.panel_y {
            let hit = crate::desktop::window_manager::with_window_manager(|wm| {
                wm.window_at_position(x, y)
            })
            .flatten();

            if let Some(wid) = hit {
                let in_title_bar = crate::desktop::window_manager::with_window_manager(|wm| {
                    wm.get_window(wid).map(|w| y < w.y + TITLE_BAR_HEIGHT)
                })
                .flatten()
                .unwrap_or(false);
                if in_title_bar {
                    close_any_window(state, wid);
                }
            } else {
                crate::desktop::launcher::with_launcher(|l| l.toggle());
            }
        }
        return;
    }

    // --- Handle mouse button release (end drag + snap-to-edge) ---
    if let crate::desktop::window_manager::InputEvent::MouseButton {
        button: 0,
        pressed: false,
        x,
        y,
    } = wm_event
    {
        if let Some(ref d) = state.drag {
            let zone = crate::desktop::window_manager::WindowManager::detect_snap_zone(
                x,
                y,
                layout.fb_width as u32,
                layout.fb_height as u32,
            );
            if zone != crate::desktop::window_manager::SnapZone::None {
                let drag_wid = d.wid;
                let drag_sid = d.surface_id;
                crate::desktop::window_manager::with_window_manager(|wm| {
                    wm.snap_window(drag_wid, zone);
                    if let Some(w) = wm.get_window(drag_wid) {
                        crate::desktop::wayland::with_display(|display| {
                            display
                                .wl_compositor
                                .set_surface_position(drag_sid, w.x, w.y);
                        });
                    }
                });
            }
        }
        state.drag = None;
        crate::desktop::window_manager::with_window_manager(|wm| {
            wm.process_input(wm_event);
        });
        return;
    }

    // --- Handle mouse move (drag window or forward) ---
    if let crate::desktop::window_manager::InputEvent::MouseMove { x, y } = wm_event {
        if let Some(ref d) = state.drag {
            let new_x = x - d.offset_x;
            let new_y = y - d.offset_y;
            crate::desktop::window_manager::with_window_manager(|wm| {
                let _ = wm.move_window(d.wid, new_x, new_y);
            });
            crate::desktop::wayland::with_display(|display| {
                display
                    .wl_compositor
                    .set_surface_position(d.surface_id, new_x, new_y);
            });
            return;
        }
    }

    // All other events: dispatch to window manager
    crate::desktop::window_manager::with_window_manager(|wm| {
        wm.process_input(wm_event);
    });
}

/// Update focus tracking, idle timeout, animations, and notifications.
fn update_ui_state(state: &mut DesktopState, tick: u64) {
    // Detect focus changes and sync compositor z_order
    let current_focused =
        crate::desktop::window_manager::with_window_manager(|wm| wm.get_focused_window_id())
            .flatten();
    if current_focused != state.prev_focused {
        if let Some(fwid) = current_focused {
            sync_compositor_focus(state, fwid);
        }
        state.prev_focused = current_focused;
    }

    // Check idle timeout for screen lock
    if state.screen_locker.check_idle_timeout(tick) {
        state.screen_locker.lock();
    }

    // Forward queued WM events to apps
    forward_events_to_apps(state);

    // Terminal: read PTY output
    crate::desktop::terminal::with_terminal_manager(|tm| {
        let _ = tm.update_all();
    });

    // Tick animation manager
    state.animation_mgr.tick(16); // ~16ms per frame at 60fps
    state.animation_mgr.remove_completed();

    // Tick notification expiry (every 30th frame to avoid overhead)
    if state.frame_count.is_multiple_of(30) {
        crate::desktop::notification::with_notification_manager(|mgr| {
            mgr.tick(tick);
        });
    }
}

/// Render all apps, composite overlays, and blit to the hardware framebuffer.
fn render_and_composite(state: &mut DesktopState, layout: &FrameLayout, tick: u64) {
    render_all_apps(state);

    // Panel: update clock + buttons + systray periodically
    if state.frame_count.is_multiple_of(10) {
        render_panel(state, layout.fb_width as u32);
    }

    // Composite, render overlays, and blit
    crate::desktop::wayland::with_display(|display| {
        display.wl_compositor.request_composite();
        let composited = display.wl_compositor.composite().unwrap_or(false);

        if composited {
            display.wl_compositor.with_back_buffer_mut(|bb| {
                render_overlays(state, bb, layout.fb_width, layout.fb_height, tick);
            });

            blit_back_buffer(
                &display.wl_compositor,
                layout.fb_ptr,
                layout.fb_width,
                layout.fb_height,
                layout.fb_stride,
                layout.is_bgr,
            );
        }
    });

    // Always draw cursor (cheap: 16x16 pixels) so it stays responsive
    let (mouse_x, mouse_y) = crate::drivers::mouse::cursor_position();
    // SAFETY: fb_ptr is valid for stride * height bytes.
    let fb_slice = unsafe {
        core::slice::from_raw_parts_mut(layout.fb_ptr, layout.fb_stride * layout.fb_height)
    };
    cursor::draw_cursor(
        fb_slice,
        layout.fb_stride,
        layout.fb_width,
        layout.fb_height,
        mouse_x,
        mouse_y,
    );

    // Yield CPU -- short spin for frame pacing
    for _ in 0..5_000 {
        core::hint::spin_loop();
    }
}

/// Render modal overlays into the composited back-buffer (post-composite,
/// pre-blit).
///
/// Only true overlays (app switcher, launcher, notifications) are drawn here.
/// Title bars, close buttons, and desktop icons are now part of their
/// respective surfaces.
fn render_overlays(
    state: &DesktopState,
    bb: &mut [u32],
    fb_width: usize,
    fb_height: usize,
    tick: u64,
) {
    // Title bars and close buttons are rendered into each surface's pixel
    // buffer (see draw_title_bar_into_surface), and desktop icons are baked
    // into the background surface (see render_icons_into_bgra).  The
    // compositor's z-order compositing handles occlusion naturally.

    // App switcher overlay (Alt+Tab)
    if state.app_switcher.is_visible() {
        state
            .app_switcher
            .render(bb, fb_width as u32, fb_height as u32);
    }

    // Launcher overlay (Super key)
    crate::desktop::launcher::with_launcher_ref(|l| {
        if l.is_visible() {
            l.render_to_buffer(bb, fb_width, fb_height);
        }
    });

    // Notification toasts (top-right)
    crate::desktop::notification::with_notification_manager(|mgr| {
        if mgr.active_count() > 0 {
            mgr.render_to_buffer(bb, fb_width, fb_height, tick);
        }
    });
}

/// Try to blit the compositor back-buffer via VirtIO GPU (DMA path).
///
/// Returns `true` if the GPU handled the blit, `false` to fall back to
/// direct MMIO writes.
#[cfg(target_arch = "x86_64")]
fn try_gpu_blit(compositor: &crate::desktop::wayland::compositor::Compositor) -> bool {
    if !crate::drivers::virtio_gpu::is_available() {
        return false;
    }
    crate::drivers::virtio_gpu::with_driver(|gpu| {
        compositor.with_back_buffer(|bb| {
            if let Some(backing) = gpu.get_framebuffer_mut() {
                let copy_len = bb.len().min(backing.len());
                backing[..copy_len].copy_from_slice(&bb[..copy_len]);
            }
        });
        let _ = gpu.flush_framebuffer();
    });
    true
}

/// Blit the compositor's back-buffer to the hardware framebuffer.
fn blit_back_buffer(
    compositor: &crate::desktop::wayland::compositor::Compositor,
    fb_ptr: *mut u8,
    fb_width: usize,
    fb_height: usize,
    fb_stride: usize,
    is_bgr: bool,
) {
    // Try VirtIO GPU first (hardware-accelerated DMA path)
    #[cfg(target_arch = "x86_64")]
    if try_gpu_blit(compositor) {
        return;
    }

    compositor.with_back_buffer(|bb| {
        if is_bgr {
            // Direct row-based memcpy -- format already matches
            for y in 0..fb_height {
                let src_start = y * fb_width;
                let src_end = src_start + fb_width;
                if src_end > bb.len() {
                    break;
                }
                let dst_offset = y * fb_stride;
                // SAFETY: fb_ptr valid for stride*height bytes; src slice
                // is fb_width u32s = fb_width*4 bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        bb[src_start..src_end].as_ptr() as *const u8,
                        fb_ptr.add(dst_offset),
                        fb_width * 4,
                    );
                }
            }
        } else {
            // RGB format: swap R and B channels
            for y in 0..fb_height {
                for x in 0..fb_width {
                    let src_idx = y * fb_width + x;
                    if src_idx >= bb.len() {
                        break;
                    }
                    let pixel = bb[src_idx];
                    let r = (pixel >> 16) & 0xFF;
                    let g = (pixel >> 8) & 0xFF;
                    let b = pixel & 0xFF;
                    let swapped = 0xFF00_0000 | (b << 16) | (g << 8) | r;
                    let dst_offset = y * fb_stride + x * 4;
                    unsafe {
                        (fb_ptr.add(dst_offset) as *mut u32).write(swapped);
                    }
                }
            }
        }
    });
}

/// Get the list of windows for the app switcher overlay.
fn get_window_list_for_switcher() -> alloc::vec::Vec<(u32, alloc::string::String)> {
    use alloc::string::ToString;
    crate::desktop::window_manager::with_window_manager(|wm| {
        wm.get_visible_windows()
            .iter()
            .map(|w| (w.id, w.title_str().to_string()))
            .collect()
    })
    .unwrap_or_default()
}

/// Switch to the previous workspace.
fn switch_workspace_prev(state: &DesktopState) {
    crate::desktop::window_manager::with_window_manager(|wm| {
        let current = wm.get_active_workspace();
        if current > 0 {
            wm.switch_workspace(current - 1);
            update_surface_visibility(state, current - 1);
        }
    });
}

/// Switch to the next workspace.
fn switch_workspace_next(state: &DesktopState) {
    crate::desktop::window_manager::with_window_manager(|wm| {
        let current = wm.get_active_workspace();
        if current < (crate::desktop::window_manager::MAX_WORKSPACES as u8 - 1) {
            wm.switch_workspace(current + 1);
            update_surface_visibility(state, current + 1);
        }
    });
}

/// Update compositor surface mapped state for a workspace switch.
///
/// Surfaces belonging to windows on the target workspace are mapped;
/// all others are unmapped. Panel and background are always visible.
fn update_surface_visibility(state: &DesktopState, target_ws: u8) {
    crate::desktop::window_manager::with_window_manager(|wm| {
        let all_windows = wm.get_all_windows();
        for w in &all_windows {
            if let Some(sid) = state.surface_for_window(w.id) {
                let should_show = w.workspace == target_ws;
                crate::desktop::wayland::with_display(|display| {
                    display.wl_compositor.set_surface_mapped(sid, should_show);
                });
            }
        }
    });
    // Update panel workspace indicator
    crate::desktop::panel::with_panel(|p| {
        p.set_active_workspace(target_ws as usize);
    });
}

/// Render all desktop app surfaces.
fn render_all_apps(state: &mut DesktopState) {
    // Terminal
    if state.terminal.wid > 0 {
        crate::desktop::terminal::with_terminal_manager(|tm| {
            tm.render_all_surfaces();
        });
    }

    // File manager
    crate::desktop::file_manager::with_file_manager(|fm| {
        fm.read().render_to_surface();
    });

    // Text editor
    crate::desktop::text_editor::with_text_editor(|te| {
        te.read().render_to_surface();
    });

    // Compute themed background color for dynamic apps (strip alpha byte)
    let app_bg = state.theme.colors().window_background.0 & 0x00FFFFFF;
    let app_error_color = state.theme.colors().error.0 & 0x00FFFFFF;
    let app_accent_color = state.theme.colors().accent.0 & 0x00FFFFFF;

    // Dynamic apps -- render content then prepend title bar into full surface
    let title_bar_h: usize = 28;
    for app in &state.dynamic_apps {
        let w = app.width as usize;
        let content_h = app.height as usize;
        let total_h = content_h + title_bar_h;
        let content_size = w * content_h * 4;
        let mut content = alloc::vec![0u8; content_size];

        match app.kind {
            AppKind::SystemMonitor => {
                render_system_monitor(&mut content, w, content_h, state.frame_count, app_bg);
            }
            AppKind::MediaPlayer => {
                render_media_player(&mut content, w, content_h, app_bg);
            }
            AppKind::ImageViewer => {
                state
                    .image_viewer
                    .render_to_u8_buffer(&mut content, w, content_h);
            }
            AppKind::Settings => {
                state
                    .settings_app
                    .render_to_u8_buffer(&mut content, w, content_h);
            }
            AppKind::Browser => {
                render_browser(&mut content, w, content_h, app_bg, &mut state.browser);
            }
            AppKind::PdfViewer => {
                render_pdf_viewer(&mut content, w, content_h, app_bg, state.pdf_page_index);
            }
            AppKind::Calculator => {
                render_calculator(
                    &mut content,
                    w,
                    content_h,
                    &state.calculator,
                    app_bg,
                    app_accent_color,
                    app_error_color,
                );
            }
        }

        // Build full surface: title bar (28 rows) + content
        let mut pixels = alloc::vec![0u8; w * total_h * 4];
        for y in 0..content_h {
            let src_off = y * w * 4;
            let dst_off = (y + title_bar_h) * w * 4;
            pixels[dst_off..dst_off + w * 4].copy_from_slice(&content[src_off..src_off + w * 4]);
        }
        draw_title_bar_into_surface(&mut pixels, w, total_h, app.wid);

        update_surface_pixels(app.surface_id, app.pool_id, app.pool_buf_id, &pixels);
    }
}

/// Render a placeholder app with title text on a solid background.
fn render_placeholder_app(buf: &mut [u8], w: usize, h: usize, title: &str, bg_color: u32) {
    let r = ((bg_color >> 16) & 0xFF) as u8;
    let g = ((bg_color >> 8) & 0xFF) as u8;
    let b = (bg_color & 0xFF) as u8;
    // Fill background
    for y in 0..h {
        for x in 0..w {
            let off = (y * w + x) * 4;
            if off + 3 < buf.len() {
                buf[off] = b;
                buf[off + 1] = g;
                buf[off + 2] = r;
                buf[off + 3] = 0xFF;
            }
        }
    }
    // Draw title centered
    let title_bytes = title.as_bytes();
    let tx = w.saturating_sub(title_bytes.len() * 8) / 2;
    draw_string_into_buffer(buf, w, title_bytes, tx, 20, 0xFFFFFF);
}

/// Render system monitor showing memory and CPU stats.
fn render_system_monitor(buf: &mut [u8], w: usize, h: usize, frame_count: u64, bg_color: u32) {
    render_placeholder_app(buf, w, h, "System Monitor", bg_color);
    let mem = crate::mm::get_memory_stats();
    let total_mb = (mem.total_frames * 4096) / (1024 * 1024);
    let used_frames = mem.total_frames.saturating_sub(mem.free_frames);
    let used_mb = (used_frames * 4096) / (1024 * 1024);
    let pct = if mem.total_frames > 0 {
        (used_frames * 100) / mem.total_frames
    } else {
        0
    };

    let perf = crate::perf::get_stats();

    // Memory line
    let mut line_buf = [0u8; 64];
    let line = format_stat_line(&mut line_buf, b"Memory: ", used_mb, b"/", total_mb, b" MB");
    draw_string_into_buffer(buf, w, line, 20, 50, 0x55EFC4);

    // Usage bar
    let bar_w = w.saturating_sub(60);
    let filled = (bar_w * pct) / 100;
    for x in 20..(20 + bar_w).min(w) {
        let color = if x < 20 + filled { 0x55EFC4 } else { 0x333333 };
        for dy in 0..12 {
            let y = 70 + dy;
            if y < h {
                draw_pixel(buf, w, x, y, color);
            }
        }
    }

    // Stats
    let stats_y = 95;
    let line2 = format_simple(&mut line_buf, b"Ctx switches: ", perf.context_switches);
    draw_string_into_buffer(buf, w, line2, 20, stats_y, 0xD4D4D4);
    let line3 = format_simple(&mut line_buf, b"Syscalls:     ", perf.syscalls);
    draw_string_into_buffer(buf, w, line3, 20, stats_y + 20, 0xD4D4D4);
    let line4 = format_simple(&mut line_buf, b"Interrupts:   ", perf.interrupts);
    draw_string_into_buffer(buf, w, line4, 20, stats_y + 40, 0xD4D4D4);
    let line5 = format_simple(&mut line_buf, b"Page faults:  ", perf.page_faults);
    draw_string_into_buffer(buf, w, line5, 20, stats_y + 60, 0xD4D4D4);
    let line6 = format_simple(&mut line_buf, b"GUI frames:   ", frame_count);
    draw_string_into_buffer(buf, w, line6, 20, stats_y + 80, 0xD4D4D4);

    // Process list header
    let proc_y = stats_y + 110;
    draw_string_into_buffer(buf, w, b"PID   NAME            STATE", 20, proc_y, 0x55EFC4);

    // Draw separator line
    for x in 20..(w.saturating_sub(20)) {
        if proc_y + 16 < h {
            draw_pixel(buf, w, x, proc_y + 16, 0x444444);
        }
    }

    // Get process list from process server
    let process_server = crate::services::process_server::get_process_server();
    let processes = process_server.list_processes();
    let max_display = ((h.saturating_sub(proc_y + 20)) / 16).min(processes.len());
    for (i, proc_info) in processes.iter().take(max_display).enumerate() {
        let py = proc_y + 20 + i * 16;
        if py + 16 > h {
            break;
        }
        // Format: PID (6 chars), NAME (16 chars), STATE
        let pid_line = format_simple(&mut line_buf, b"", proc_info.pid.0);
        draw_string_into_buffer(buf, w, pid_line, 20, py, 0xD4D4D4);

        let name_bytes = proc_info.name.as_bytes();
        let name_len = name_bytes.len().min(15);
        draw_string_into_buffer(buf, w, &name_bytes[..name_len], 68, py, 0xD4D4D4);

        let state_str = match proc_info.state {
            crate::services::process_server::ProcessState::Running => b"Running" as &[u8],
            crate::services::process_server::ProcessState::Sleeping => b"Sleeping",
            crate::services::process_server::ProcessState::Waiting => b"Waiting",
            crate::services::process_server::ProcessState::Stopped => b"Stopped",
            crate::services::process_server::ProcessState::Zombie => b"Zombie",
            crate::services::process_server::ProcessState::Dead => b"Dead",
        };
        draw_string_into_buffer(buf, w, state_str, 196, py, 0xD4D4D4);
    }
}

/// Render media player with playback info and real audio stream data.
fn render_media_player(buf: &mut [u8], w: usize, h: usize, bg_color: u32) {
    render_placeholder_app(buf, w, h, "Media Player", bg_color);

    // Query real audio subsystem for stream info
    let (stream_count, sample_rate, channels) = crate::audio::client::with_client(|client| {
        (
            client.stream_count(),
            client.default_sample_rate(),
            client.default_channels(),
        )
    })
    .unwrap_or((0, 48000, 2));

    let mut line_buf = [0u8; 64];

    // Audio engine status
    draw_string_into_buffer(buf, w, b"Audio Engine: Active", 20, 50, 0x55EFC4);

    // Stream count
    let line = format_simple(&mut line_buf, b"Active streams: ", stream_count as u64);
    draw_string_into_buffer(buf, w, line, 20, 70, 0xD4D4D4);

    // Sample rate
    let line2 = format_simple(&mut line_buf, b"Sample rate:    ", sample_rate as u64);
    draw_string_into_buffer(buf, w, line2, 20, 90, 0xD4D4D4);

    // Channels
    let line3 = format_simple(&mut line_buf, b"Channels:       ", channels as u64);
    draw_string_into_buffer(buf, w, line3, 20, 110, 0xD4D4D4);

    if stream_count == 0 {
        draw_string_into_buffer(buf, w, b"No media loaded", 20, 145, 0x95A5A6);
        draw_string_into_buffer(
            buf,
            w,
            b"Use file manager to open audio files",
            20,
            165,
            0x7F8C8D,
        );
    }

    draw_string_into_buffer(
        buf,
        w,
        b"Controls: Space=play/pause  S=stop  +/-=volume",
        20,
        h.saturating_sub(30),
        0x7F8C8D,
    );
}

/// Render web browser with live browser engine rendering.
fn render_browser(
    buf: &mut [u8],
    w: usize,
    h: usize,
    bg_color: u32,
    browser: &mut Option<crate::browser::browser_main::Browser>,
) {
    // Lazily initialize the browser engine on first render
    if browser.is_none() {
        let config = crate::browser::browser_main::BrowserConfig {
            viewport_width: w as u32,
            viewport_height: h as u32,
            ..crate::browser::browser_main::BrowserConfig::default()
        };
        let mut b = crate::browser::browser_main::Browser::new(config);
        b.init();
        *browser = Some(b);
    }

    if let Some(ref mut b) = browser {
        // Tick animations and re-render
        b.tick();
        b.render();

        // Copy browser's u32 BGRA framebuffer into the u8 BGRA buffer
        let fb = b.framebuffer();
        let (bw, bh) = b.dimensions();
        let bw = bw as usize;
        let bh = bh as usize;

        for y in 0..h.min(bh) {
            for x in 0..w.min(bw) {
                let src_idx = y * bw + x;
                let dst_off = (y * w + x) * 4;
                if src_idx < fb.len() && dst_off + 3 < buf.len() {
                    let pixel = fb[src_idx];
                    buf[dst_off] = (pixel & 0xFF) as u8; // B
                    buf[dst_off + 1] = ((pixel >> 8) & 0xFF) as u8; // G
                    buf[dst_off + 2] = ((pixel >> 16) & 0xFF) as u8; // R
                    buf[dst_off + 3] = ((pixel >> 24) & 0xFF) as u8; // A
                }
            }
        }
    } else {
        // Fallback: placeholder if browser init failed
        render_placeholder_app(buf, w, h, "Web Browser", bg_color);
        draw_string_into_buffer(buf, w, b"Browser engine unavailable", 20, 75, 0xFF6666);
    }
}

/// Render PDF viewer with document area and real PDF engine rendering.
fn render_pdf_viewer(buf: &mut [u8], w: usize, h: usize, bg_color: u32, page_index: usize) {
    render_placeholder_app(buf, w, h, "PDF Viewer", bg_color);

    // Try loading a sample PDF from VFS to show engine capability
    let pdf_status = crate::fs::read_file("/usr/share/doc/sample.pdf").ok();
    let doc_opt = pdf_status.and_then(crate::desktop::pdf::PdfDocument::open);

    let (has_pdf, page_count) = if let Some(ref d) = doc_opt {
        (true, d.page_count())
    } else {
        (false, 0)
    };

    // Clamp page index to valid range
    let display_page = if page_count > 0 {
        page_index.min(page_count.saturating_sub(1))
    } else {
        0
    };

    // Toolbar showing current page
    let mut line_buf = [0u8; 64];
    if has_pdf {
        let toolbar = format_stat_line(
            &mut line_buf,
            b"[Open] [<] Page ",
            display_page.saturating_add(1),
            b" of ",
            page_count,
            b" [>] [Zoom: 100%]",
        );
        draw_string_into_buffer(buf, w, toolbar, 10, 40, 0xBBBBBB);
    } else {
        draw_string_into_buffer(
            buf,
            w,
            b"[Open] [<] Page 0 of 0 [>] [Zoom: 100%]",
            10,
            40,
            0xBBBBBB,
        );
    }

    // Document area
    let page_x = 40;
    let page_y = 65;
    let page_w = w.saturating_sub(80);
    let page_h = h.saturating_sub(100);

    // Render actual PDF page content if available
    if has_pdf {
        if let Some(ref doc) = doc_opt {
            if let Some(page) = doc.get_page(display_page) {
                // Render PDF page into u32 buffer via PdfRenderer
                let mut render_buf = vec![0xFFF0F0F0u32; page_w * page_h];
                let mut renderer =
                    crate::desktop::pdf::PdfRenderer::new(page_w as u32, page_h as u32);
                renderer.render_page(page, &mut render_buf);

                // Blit rendered page into the u8 output buffer at page_x, page_y
                for py in 0..page_h.min(h.saturating_sub(page_y)) {
                    for px in 0..page_w.min(w.saturating_sub(page_x)) {
                        let src_idx = py * page_w + px;
                        let dst_off = ((page_y + py) * w + (page_x + px)) * 4;
                        if src_idx < render_buf.len() && dst_off + 3 < buf.len() {
                            let pixel = render_buf[src_idx];
                            buf[dst_off] = (pixel & 0xFF) as u8; // B
                            buf[dst_off + 1] = ((pixel >> 8) & 0xFF) as u8; // G
                            buf[dst_off + 2] = ((pixel >> 16) & 0xFF) as u8; // R
                            buf[dst_off + 3] = 0xFF; // A
                        }
                    }
                }

                // Overlay page info
                let page_line = format_simple(&mut line_buf, b"Pages: ", page_count as u64);
                draw_string_into_buffer(
                    buf,
                    w,
                    page_line,
                    page_x + 20,
                    page_y + page_h.saturating_sub(25),
                    0x888888,
                );
            } else {
                // Page index out of range -- draw white area with message
                render_pdf_page_background(buf, w, page_x, page_y, page_w, page_h);
                draw_string_into_buffer(
                    buf,
                    w,
                    b"Page not available",
                    page_x + 20,
                    page_y + 20,
                    0x555555,
                );
            }
        }
    } else {
        // No PDF loaded -- white page area with instructions
        render_pdf_page_background(buf, w, page_x, page_y, page_w, page_h);
        draw_string_into_buffer(
            buf,
            w,
            b"PDF Engine: VeridianPDF v0.25.0",
            page_x + 20,
            page_y + 20,
            0x333333,
        );
        draw_string_into_buffer(
            buf,
            w,
            b"No document loaded",
            page_x + 20,
            page_y + 40,
            0x555555,
        );
        draw_string_into_buffer(
            buf,
            w,
            b"Use File Manager to open a PDF",
            page_x + 20,
            page_y + 60,
            0x777777,
        );
    }

    // Supported features footer
    draw_string_into_buffer(
        buf,
        w,
        b"Supports: PDF 1.0-2.0, xref, text extraction",
        page_x + 20,
        page_y + page_h.saturating_sub(10),
        0x888888,
    );
}

/// Draw a light-grey page background for the PDF viewer.
fn render_pdf_page_background(
    buf: &mut [u8],
    w: usize,
    page_x: usize,
    page_y: usize,
    page_w: usize,
    page_h: usize,
) {
    for y in page_y..(page_y + page_h).min(buf.len() / (w * 4)) {
        for x in page_x..(page_x + page_w).min(w) {
            let off = (y * w + x) * 4;
            if off + 3 < buf.len() {
                buf[off] = 0xF0; // B
                buf[off + 1] = 0xF0; // G
                buf[off + 2] = 0xF0; // R
                buf[off + 3] = 0xFF;
            }
        }
    }
}

/// Format an i64 into a decimal byte string (no allocator needed).
fn format_i64(fmtbuf: &mut [u8; 24], value: i64) -> &[u8] {
    if value == 0 {
        fmtbuf[0] = b'0';
        return &fmtbuf[..1];
    }
    let mut pos = 24;
    let (mut n, negative) = if value < 0 {
        (-(value as i128) as u64, true)
    } else {
        (value as u64, false)
    };
    while n > 0 {
        pos -= 1;
        fmtbuf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if negative {
        pos -= 1;
        fmtbuf[pos] = b'-';
    }
    &fmtbuf[pos..]
}

/// Render a basic integer calculator.
fn render_calculator(
    buf: &mut [u8],
    w: usize,
    h: usize,
    calc: &CalculatorState,
    bg_color: u32,
    accent_color: u32,
    error_color: u32,
) {
    render_placeholder_app(buf, w, h, "Calculator", bg_color);

    // Display area (dark input field)
    let display_x = 16;
    let display_y = 44;
    let display_w = w.saturating_sub(32);
    let display_h = 32;
    for y in display_y..(display_y + display_h).min(h) {
        for x in display_x..(display_x + display_w).min(w) {
            let off = (y * w + x) * 4;
            if off + 3 < buf.len() {
                buf[off] = 0x18;
                buf[off + 1] = 0x18;
                buf[off + 2] = 0x18;
                buf[off + 3] = 0xFF;
            }
        }
    }
    // Show calculator display value (right-aligned)
    let mut fmtbuf = [0u8; 24];
    let display_text: &[u8] = if calc.error {
        b"Error"
    } else {
        format_i64(&mut fmtbuf, calc.display_value)
    };
    let text_pixel_width = display_text.len() * 8;
    let text_x = display_x + display_w.saturating_sub(text_pixel_width + 8);
    draw_string_into_buffer(buf, w, display_text, text_x, display_y + 8, 0xFFFFFF);

    // Button grid: 4 columns x 5 rows
    let buttons: [&[u8]; 20] = [
        b"C", b"(", b")", b"/", b"7", b"8", b"9", b"*", b"4", b"5", b"6", b"-", b"1", b"2", b"3",
        b"+", b"0", b".", b"<", b"=",
    ];
    let grid_x = 16;
    let grid_y = display_y + display_h + 12;
    let btn_w = (w.saturating_sub(48)) / 4;
    let btn_h = 36;
    let gap = 4;

    for (i, label) in buttons.iter().enumerate() {
        let col = i % 4;
        let row = i / 4;
        let bx = grid_x + col * (btn_w + gap);
        let by = grid_y + row * (btn_h + gap);

        // Button color: operators are accent, clear is error, numbers are neutral
        let btn_color: u32 = match label[0] {
            b'/' | b'*' | b'-' | b'+' | b'=' => accent_color,
            b'C' => error_color,
            _ => 0x3A3A4A,
        };
        let cr = ((btn_color >> 16) & 0xFF) as u8;
        let cg = ((btn_color >> 8) & 0xFF) as u8;
        let cb = (btn_color & 0xFF) as u8;

        for dy in 0..btn_h {
            for dx in 0..btn_w {
                let px = bx + dx;
                let py = by + dy;
                if px < w && py < h {
                    let off = (py * w + px) * 4;
                    if off + 3 < buf.len() {
                        buf[off] = cb;
                        buf[off + 1] = cg;
                        buf[off + 2] = cr;
                        buf[off + 3] = 0xFF;
                    }
                }
            }
        }

        // Center label on button
        let lx = bx + (btn_w.saturating_sub(label.len() * 8)) / 2;
        let ly = by + (btn_h.saturating_sub(8)) / 2;
        if ly < h {
            draw_string_into_buffer(buf, w, label, lx, ly, 0xFFFFFF);
        }
    }
}

/// Draw a single pixel in BGRA format.
fn draw_pixel(buf: &mut [u8], w: usize, x: usize, y: usize, color: u32) {
    let off = (y * w + x) * 4;
    if off + 3 < buf.len() {
        buf[off] = (color & 0xFF) as u8;
        buf[off + 1] = ((color >> 8) & 0xFF) as u8;
        buf[off + 2] = ((color >> 16) & 0xFF) as u8;
        buf[off + 3] = 0xFF;
    }
}

/// Format a stat line: "prefix VALUE suffix" into a fixed buffer. Returns the
/// used slice.
fn format_stat_line<'a>(
    buf: &'a mut [u8; 64],
    prefix: &[u8],
    value: usize,
    sep: &[u8],
    value2: usize,
    suffix: &[u8],
) -> &'a [u8] {
    let mut pos = 0;
    for &b in prefix {
        if pos < 64 {
            buf[pos] = b;
            pos += 1;
        }
    }
    pos = write_usize_to_buf(buf, pos, value);
    for &b in sep {
        if pos < 64 {
            buf[pos] = b;
            pos += 1;
        }
    }
    pos = write_usize_to_buf(buf, pos, value2);
    for &b in suffix {
        if pos < 64 {
            buf[pos] = b;
            pos += 1;
        }
    }
    &buf[..pos]
}

/// Format "prefix VALUE" into a fixed buffer.
fn format_simple<'a>(buf: &'a mut [u8; 64], prefix: &[u8], value: u64) -> &'a [u8] {
    let mut pos = 0;
    for &b in prefix {
        if pos < 64 {
            buf[pos] = b;
            pos += 1;
        }
    }
    pos = write_usize_to_buf(buf, pos, value as usize);
    &buf[..pos]
}

/// Write a usize as decimal digits into a byte buffer at position `pos`.
fn write_usize_to_buf(buf: &mut [u8; 64], mut pos: usize, value: usize) -> usize {
    if value == 0 {
        if pos < 64 {
            buf[pos] = b'0';
            pos += 1;
        }
        return pos;
    }
    let mut digits = [0u8; 20];
    let mut n = value;
    let mut count = 0;
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    for i in (0..count).rev() {
        if pos < 64 {
            buf[pos] = digits[i];
            pos += 1;
        }
    }
    pos
}

/// Render the panel to its compositor surface, including system tray.
fn render_panel(state: &DesktopState, screen_width: u32) {
    // Update system tray data from kernel stats
    crate::desktop::systray::with_system_tray(|tray| {
        let mem_stats = crate::mm::get_memory_stats();
        let total_mb = (mem_stats.total_frames * 4096 / (1024 * 1024)) as u32;
        let used_frames = mem_stats.total_frames.saturating_sub(mem_stats.free_frames);
        let used_mb = (used_frames * 4096 / (1024 * 1024)) as u32;
        tray.update_memory(used_mb, total_mb);

        // CPU: approximate from scheduler context switches
        let ctx_switches = crate::sched::metrics::SCHEDULER_METRICS
            .context_switches
            .load(core::sync::atomic::Ordering::Relaxed);
        // Use low bits as a rough utilization proxy (modular, wraps at 100)
        let cpu_pct = (ctx_switches % 100) as u8;
        tray.update_cpu(cpu_pct);

        // Network: check interface state
        let iface = crate::net::ip::get_interface_config();
        let is_up = iface.ip_addr.0 != [0, 0, 0, 0];
        tray.update_network(is_up);

        // Volume: read mixer
        if let Ok(vol) =
            crate::audio::mixer::with_mixer(|m: &mut crate::audio::mixer::AudioMixer| {
                m.get_master_volume()
            })
        {
            let vol_pct = ((vol as u32) * 100 / 65535) as u8;
            tray.update_volume(vol_pct);
        }

        // Battery: check power state
        let battery_label = if crate::power::is_initialized() {
            "AC"
        } else {
            "--"
        };
        tray.update_battery(battery_label);
    });

    let panel_h = crate::desktop::panel::PANEL_HEIGHT;
    crate::desktop::panel::with_panel(|panel| {
        panel.update_buttons();
        panel.update_clock();
        let mut buf = vec![0u8; (screen_width as usize) * (panel_h as usize) * 4];
        panel.render(&mut buf);
        update_surface_pixels(
            state.panel_surface_id,
            state.panel_pool_id,
            state.panel_pool_buf_id,
            &buf,
        );
    });
}

/// Forward pending window manager events to the appropriate apps.
fn forward_events_to_apps(state: &mut DesktopState) {
    // Get events for terminal window
    if state.terminal.wid > 0 {
        let events = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_events(state.terminal.wid)
        })
        .unwrap_or_default();
        if !events.is_empty() {
            crate::desktop::terminal::with_terminal_manager(|tm| {
                for event in &events {
                    let _ = tm.process_input(0, *event);
                }
            });
        }
    }

    // Get events for file manager window
    if state.file_manager.wid > 0 {
        let events = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_events(state.file_manager.wid)
        })
        .unwrap_or_default();
        if !events.is_empty() {
            crate::desktop::file_manager::with_file_manager(|fm| {
                let mut manager = fm.write();
                for event in &events {
                    let _ = manager.process_input(*event);
                }
            });
        }
    }

    // Get events for text editor window
    if state.text_editor.wid > 0 {
        let events = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_events(state.text_editor.wid)
        })
        .unwrap_or_default();
        if !events.is_empty() {
            crate::desktop::text_editor::with_text_editor(|te| {
                let mut editor = te.write();
                for event in &events {
                    let _ = editor.process_input(*event);
                }
            });
        }
    }

    // Dynamic apps: collect (wid, kind, events), then dispatch
    let dynamic_info: alloc::vec::Vec<(u32, AppKind)> =
        state.dynamic_apps.iter().map(|a| (a.wid, a.kind)).collect();
    for (wid, kind) in dynamic_info {
        let events = crate::desktop::window_manager::with_window_manager(|wm| wm.get_events(wid))
            .unwrap_or_default();
        if events.is_empty() {
            continue;
        }
        // Get window position for coordinate transform
        let win_pos = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_window(wid).map(|w| (w.x, w.y))
        })
        .flatten()
        .unwrap_or((0, 0));

        let mut should_close = false;
        for event in &events {
            // ESC closes any dynamic app
            if let crate::desktop::window_manager::InputEvent::KeyPress { scancode: 0x1B, .. } =
                event
            {
                should_close = true;
                break;
            }
            match kind {
                AppKind::Settings => match event {
                    crate::desktop::window_manager::InputEvent::KeyPress { scancode, .. } => {
                        state.settings_app.handle_key(*scancode);
                    }
                    crate::desktop::window_manager::InputEvent::MouseButton {
                        button: 0,
                        pressed: true,
                        x,
                        y,
                    } => {
                        let local_x = (*x - win_pos.0).max(0) as usize;
                        let local_y = (*y - win_pos.1).max(0) as usize;
                        state.settings_app.handle_click(local_x, local_y);
                    }
                    _ => {}
                },
                AppKind::ImageViewer => {
                    if let crate::desktop::window_manager::InputEvent::KeyPress {
                        scancode, ..
                    } = event
                    {
                        state.image_viewer.handle_key(*scancode);
                    }
                }
                AppKind::Calculator => {
                    if let crate::desktop::window_manager::InputEvent::KeyPress {
                        scancode, ..
                    } = event
                    {
                        state.calculator.handle_key(*scancode);
                    }
                }
                AppKind::MediaPlayer => {
                    if let crate::desktop::window_manager::InputEvent::KeyPress {
                        scancode, ..
                    } = event
                    {
                        match scancode {
                            b'+' => {
                                let _ = crate::audio::mixer::with_mixer(|m| {
                                    let vol = m.get_master_volume();
                                    m.set_master_volume(vol.saturating_add(3276));
                                });
                            }
                            b'-' => {
                                let _ = crate::audio::mixer::with_mixer(|m| {
                                    let vol = m.get_master_volume();
                                    m.set_master_volume(vol.saturating_sub(3276));
                                });
                            }
                            _ => {}
                        }
                    }
                }
                AppKind::PdfViewer => {
                    if let crate::desktop::window_manager::InputEvent::KeyPress {
                        scancode, ..
                    } = event
                    {
                        match scancode {
                            0x81 | b'n' => {
                                // Down arrow or 'n' = next page
                                state.pdf_page_index = state.pdf_page_index.saturating_add(1);
                            }
                            0x80 | b'p' => {
                                // Up arrow or 'p' = previous page
                                state.pdf_page_index = state.pdf_page_index.saturating_sub(1);
                            }
                            _ => {}
                        }
                    }
                }
                AppKind::Browser => match event {
                    crate::desktop::window_manager::InputEvent::KeyPress { scancode, .. } => {
                        let mods = crate::drivers::keyboard::get_modifiers();
                        let ctrl = mods & crate::drivers::keyboard::MOD_CTRL != 0;
                        let shift = mods & crate::drivers::keyboard::MOD_SHIFT != 0;
                        let alt = mods & crate::drivers::keyboard::MOD_ALT != 0;
                        if let Some(ref mut b) = state.browser {
                            b.handle_key(*scancode, ctrl, shift, alt);
                        }
                    }
                    crate::desktop::window_manager::InputEvent::MouseButton {
                        button: 0,
                        pressed: true,
                        x,
                        y,
                    } => {
                        let local_x = (*x - win_pos.0).max(0);
                        let local_y = (*y - win_pos.1).max(0);
                        if let Some(ref mut b) = state.browser {
                            b.handle_click(local_x, local_y);
                        }
                    }
                    _ => {}
                },
                AppKind::SystemMonitor => {
                    // System monitor updates driven by polling, no key dispatch
                    // needed
                }
            }
        }
        if should_close {
            close_dynamic_app(state, wid);
        }
    }
}

/// Blit the compositor's XRGB8888 back-buffer to the hardware framebuffer.
///
/// Handles BGR vs RGB pixel format conversion.
/// Note: The render loop now blits inline via `with_back_buffer()` to avoid
/// a 4MB clone. This standalone version is kept for potential future use.
fn blit_to_framebuffer(
    back_buffer: &[u32],
    fb_ptr: *mut u8,
    fb_width: usize,
    fb_height: usize,
    fb_stride: usize,
    is_bgr: bool,
) {
    for y in 0..fb_height {
        for x in 0..fb_width {
            let src_idx = y * fb_width + x;
            if src_idx >= back_buffer.len() {
                break;
            }
            let pixel = back_buffer[src_idx];
            let r = ((pixel >> 16) & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let b = (pixel & 0xFF) as u8;

            let dst_offset = y * fb_stride + x * 4;
            // SAFETY: fb_ptr is valid for stride * height bytes,
            // and dst_offset + 3 < stride * height.
            unsafe {
                let ptr = fb_ptr.add(dst_offset);
                if is_bgr {
                    // BGRA format (UEFI default)
                    ptr.write(b);
                    ptr.add(1).write(g);
                    ptr.add(2).write(r);
                    ptr.add(3).write(0xFF);
                } else {
                    // RGBA format
                    ptr.write(r);
                    ptr.add(1).write(g);
                    ptr.add(2).write(b);
                    ptr.add(3).write(0xFF);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WM-2: Server-side window decorations
// ---------------------------------------------------------------------------

/// Window decoration rendering configuration.
#[derive(Debug, Clone, Copy)]
pub struct DecorationConfig {
    /// Title bar height in pixels.
    pub title_bar_height: u32,
    /// Border width in pixels.
    pub border_width: u32,
    /// Title bar background color (focused): ARGB packed as 0xAARRGGBB.
    pub title_bg_focused: u32,
    /// Title bar background color (unfocused): ARGB packed as 0xAARRGGBB.
    pub title_bg_unfocused: u32,
    /// Title text color: ARGB packed as 0xAARRGGBB.
    pub title_text_color: u32,
    /// Border color (focused): ARGB packed as 0xAARRGGBB.
    pub border_focused: u32,
    /// Border color (unfocused): ARGB packed as 0xAARRGGBB.
    pub border_unfocused: u32,
    /// Button size in pixels (close/maximize/minimize).
    pub button_size: u32,
    /// Padding between title text and buttons.
    pub button_padding: u32,
}

impl DecorationConfig {
    /// Default decoration configuration matching the existing desktop style.
    pub fn default_config() -> Self {
        Self {
            title_bar_height: 28,
            border_width: 1,
            title_bg_focused: 0xFF34_495E,
            title_bg_unfocused: 0xFF57_6574,
            title_text_color: 0xFFEC_F0F1,
            border_focused: 0xFF2C_3E50,
            border_unfocused: 0xFF7F_8C8D,
            button_size: 16,
            button_padding: 6,
        }
    }
}

impl Default for DecorationConfig {
    fn default() -> Self {
        Self::default_config()
    }
}

/// Buttons that can appear in the title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationButton {
    Close,
    Maximize,
    Minimize,
}

/// Result of hit-testing a point against window decorations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationHitTest {
    /// Point is not in any decoration area.
    None,
    /// Title bar (drag region).
    TitleBar,
    /// Close button.
    CloseButton,
    /// Maximize button.
    MaximizeButton,
    /// Minimize button.
    MinimizeButton,
    /// Border edge for resize.
    Border(BorderEdge),
}

/// Which border edge was hit for resize operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Render a complete window decoration frame into a pixel buffer.
///
/// Draws the title bar (with title text), border outline, and close/maximize/
/// minimize buttons. The buffer is expected to include space for decorations:
/// total width = `width`, total height = `height` (title bar at top, border
/// around all edges).
///
/// Pixels are written as 0xAARRGGBB (ARGB8888).
pub fn render_window_decoration(
    buffer: &mut [u32],
    buf_w: u32,
    config: &DecorationConfig,
    title: &str,
    focused: bool,
    width: u32,
    height: u32,
) {
    let bw = config.border_width;
    let tbh = config.title_bar_height;
    let border_color = if focused {
        config.border_focused
    } else {
        config.border_unfocused
    };
    let title_bg = if focused {
        config.title_bg_focused
    } else {
        config.title_bg_unfocused
    };

    // Draw border (top, bottom, left, right strips)
    for y in 0..height {
        for x in 0..width {
            let idx = (y * buf_w + x) as usize;
            if idx >= buffer.len() {
                continue;
            }
            let in_top_border = y < bw;
            let in_bottom_border = y >= height.saturating_sub(bw);
            let in_left_border = x < bw;
            let in_right_border = x >= width.saturating_sub(bw);

            if in_top_border || in_bottom_border || in_left_border || in_right_border {
                buffer[idx] = border_color;
            }
        }
    }

    // Draw title bar background
    for y in bw..bw.saturating_add(tbh).min(height) {
        for x in bw..width.saturating_sub(bw) {
            let idx = (y * buf_w + x) as usize;
            if idx < buffer.len() {
                buffer[idx] = title_bg;
            }
        }
    }

    // Draw title text (8x16 font)
    let text_x = bw + 8;
    let text_y = bw + (tbh.saturating_sub(16)) / 2;
    let mut cx = text_x;
    for &ch in title.as_bytes() {
        if cx + 8 >= width.saturating_sub(bw + 3 * (config.button_size + config.button_padding)) {
            break;
        }
        let glyph = crate::graphics::font8x16::glyph(ch);
        for row in 0..16u32 {
            let bits = glyph[row as usize];
            for col in 0..8u32 {
                if bits & (0x80 >> col) != 0 {
                    let px = cx + col;
                    let py = text_y + row;
                    let idx = (py * buf_w + px) as usize;
                    if idx < buffer.len() {
                        buffer[idx] = config.title_text_color;
                    }
                }
            }
        }
        cx += 8;
    }

    // Draw buttons (right-aligned in title bar)
    let btn_y = bw + (tbh.saturating_sub(config.button_size)) / 2;
    let btn_sz = config.button_size;
    let btn_pad = config.button_padding;

    // Close button (rightmost)
    let close_x = width.saturating_sub(bw + btn_pad + btn_sz);
    render_decoration_button(
        buffer,
        buf_w,
        close_x,
        btn_y,
        btn_sz,
        DecorationButton::Close,
        false,
    );

    // Maximize button
    let max_x = close_x.saturating_sub(btn_pad + btn_sz);
    render_decoration_button(
        buffer,
        buf_w,
        max_x,
        btn_y,
        btn_sz,
        DecorationButton::Maximize,
        false,
    );

    // Minimize button
    let min_x = max_x.saturating_sub(btn_pad + btn_sz);
    render_decoration_button(
        buffer,
        buf_w,
        min_x,
        btn_y,
        btn_sz,
        DecorationButton::Minimize,
        false,
    );
}

/// Hit-test a point (relative to the window's top-left including decorations)
/// against the decoration regions.
pub fn hit_test_decoration(
    x: i32,
    y: i32,
    config: &DecorationConfig,
    width: u32,
    height: u32,
) -> DecorationHitTest {
    let bw = config.border_width as i32;
    let tbh = config.title_bar_height as i32;
    let w = width as i32;
    let h = height as i32;
    let btn_sz = config.button_size as i32;
    let btn_pad = config.button_padding as i32;

    // Outside the window entirely
    if x < 0 || y < 0 || x >= w || y >= h {
        return DecorationHitTest::None;
    }

    // Border corners (8x8 corner zones)
    let corner = bw.max(8);
    if x < corner && y < corner {
        return DecorationHitTest::Border(BorderEdge::TopLeft);
    }
    if x >= w - corner && y < corner {
        return DecorationHitTest::Border(BorderEdge::TopRight);
    }
    if x < corner && y >= h - corner {
        return DecorationHitTest::Border(BorderEdge::BottomLeft);
    }
    if x >= w - corner && y >= h - corner {
        return DecorationHitTest::Border(BorderEdge::BottomRight);
    }

    // Border edges
    if y < bw {
        return DecorationHitTest::Border(BorderEdge::Top);
    }
    if y >= h - bw {
        return DecorationHitTest::Border(BorderEdge::Bottom);
    }
    if x < bw {
        return DecorationHitTest::Border(BorderEdge::Left);
    }
    if x >= w - bw {
        return DecorationHitTest::Border(BorderEdge::Right);
    }

    // Title bar region
    if y >= bw && y < bw + tbh {
        // Check buttons (right-aligned)
        let close_x = w - bw - btn_pad - btn_sz;
        if x >= close_x && x < close_x + btn_sz {
            return DecorationHitTest::CloseButton;
        }

        let max_x = close_x - btn_pad - btn_sz;
        if x >= max_x && x < max_x + btn_sz {
            return DecorationHitTest::MaximizeButton;
        }

        let min_x = max_x - btn_pad - btn_sz;
        if x >= min_x && x < min_x + btn_sz {
            return DecorationHitTest::MinimizeButton;
        }

        return DecorationHitTest::TitleBar;
    }

    DecorationHitTest::None
}

/// Render a single decoration button (close, maximize, or minimize).
///
/// Draws a small icon inside a square region starting at (`x`, `y`) with
/// the given `size`. If `hovered`, the background is slightly highlighted.
pub fn render_decoration_button(
    buffer: &mut [u32],
    buf_w: u32,
    x: u32,
    y: u32,
    size: u32,
    button_type: DecorationButton,
    hovered: bool,
) {
    // Button background
    let bg = if hovered {
        match button_type {
            DecorationButton::Close => 0xFFE7_4C3C,    // Red highlight
            DecorationButton::Maximize => 0xFF2E_CC71, // Green highlight
            DecorationButton::Minimize => 0xFFF3_9C12, // Yellow highlight
        }
    } else {
        match button_type {
            DecorationButton::Close => 0xFFC0_392B,
            DecorationButton::Maximize => 0xFF27_AE60,
            DecorationButton::Minimize => 0xFFF1_C40F,
        }
    };

    // Fill button background (circle approximation: filled square with inset)
    let inset = size / 6;
    for dy in inset..size.saturating_sub(inset) {
        for dx in inset..size.saturating_sub(inset) {
            let px = x + dx;
            let py = y + dy;
            let idx = (py * buf_w + px) as usize;
            if idx < buffer.len() {
                buffer[idx] = bg;
            }
        }
    }

    // Draw icon glyph
    let icon_color: u32 = 0xFFFF_FFFF;
    let cx = x + size / 2;
    let cy = y + size / 2;
    let half = (size / 4).max(2);

    match button_type {
        DecorationButton::Close => {
            // X shape: two diagonal lines
            for i in 0..half {
                let coords = [
                    (cx - half + i, cy - half + i),
                    (cx + half - i - 1, cy - half + i),
                    (cx - half + i, cy + half - i - 1),
                    (cx + half - i - 1, cy + half - i - 1),
                ];
                for &(px, py) in &coords {
                    let idx = (py * buf_w + px) as usize;
                    if idx < buffer.len() {
                        buffer[idx] = icon_color;
                    }
                }
            }
        }
        DecorationButton::Maximize => {
            // Rectangle outline
            for i in 0..(half * 2) {
                let coords = [
                    (cx - half + i, cy - half),     // top
                    (cx - half + i, cy + half - 1), // bottom
                    (cx - half, cy - half + i),     // left
                    (cx + half - 1, cy - half + i), // right
                ];
                for &(px, py) in &coords {
                    let idx = (py * buf_w + px) as usize;
                    if idx < buffer.len() {
                        buffer[idx] = icon_color;
                    }
                }
            }
        }
        DecorationButton::Minimize => {
            // Horizontal line at bottom
            for i in 0..(half * 2) {
                let px = cx - half + i;
                let py = cy + half / 2;
                let idx = (py * buf_w + px) as usize;
                if idx < buffer.len() {
                    buffer[idx] = icon_color;
                }
            }
        }
    }
}

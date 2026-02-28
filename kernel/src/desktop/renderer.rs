//! Desktop Renderer
//!
//! Connects the Wayland compositor's back-buffer to the hardware framebuffer.
//! Creates the initial desktop scene (background gradient, panel, terminal
//! placeholder) and runs the compositing loop.

use alloc::{vec, vec::Vec};
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
    if let Err(e) = crate::desktop::init() {
        crate::println!("[DESKTOP] Failed to initialize desktop: {:?}", e);
        return;
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
    let apps = create_desktop_scene(hw.width as u32, hw.height as u32);

    // Disable fbcon text output -- the compositor takes over the framebuffer
    fbcon::disable_output();

    // Clear the framebuffer to black before first composite
    // SAFETY: hw.fb_ptr is valid for stride * height bytes.
    unsafe {
        core::ptr::write_bytes(hw.fb_ptr, 0, hw.stride * hw.height);
    }

    crate::serial::_serial_print(format_args!("[DESKTOP] Entering compositor render loop\n"));

    // Render loop: composite -> blit to framebuffer -> poll input -> repeat
    render_loop(&hw, &apps);

    // If we exit the render loop, re-enable fbcon
    fbcon::enable_output();
    crate::println!("[DESKTOP] GUI stopped, returning to text console");
}

/// Window IDs for the three desktop applications, stored after scene creation.
struct DesktopApps {
    terminal_wid: u32,
    file_manager_wid: u32,
    text_editor_wid: u32,
    panel_surface_id: u32,
    panel_pool_id: u32,
    panel_pool_buf_id: u32,
}

/// Create the initial desktop scene: background gradient, real apps, and panel.
fn create_desktop_scene(width: u32, height: u32) -> DesktopApps {
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

    let pool_id = 100;
    let mut pool = crate::desktop::wayland::buffer::WlShmPool::new(pool_id, 0, pool_size);
    pool.write_data(0, &bg_pixels);
    let buf_id = pool
        .create_buffer(
            0,
            width,
            height,
            width * 4,
            crate::desktop::wayland::buffer::PixelFormat::Xrgb8888,
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
        crate::desktop::wayland::buffer::PixelFormat::Xrgb8888,
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
    let panel_h = crate::desktop::panel::PANEL_HEIGHT;
    let _ = crate::desktop::panel::init(width, height);
    let (panel_surface_id, panel_pool_id, panel_pool_buf_id) =
        create_app_surface(0, (height - panel_h) as i32, width, panel_h);

    // --- Create real applications ---
    // Terminal: 640x384 (80cols x 24rows x 16px)
    let terminal_wid =
        crate::desktop::terminal::with_terminal_manager(|tm| match tm.create_terminal(640, 384) {
            Ok(idx) => tm.get_window_id(idx).unwrap_or(0),
            Err(_) => 0,
        })
        .unwrap_or(0);

    // File manager: 640x480
    let file_manager_wid = if crate::desktop::file_manager::create_file_manager().is_ok() {
        crate::desktop::file_manager::with_file_manager(|fm| fm.read().window_id()).unwrap_or(0)
    } else {
        0
    };

    // Text editor: 800x600
    let text_editor_wid = if crate::desktop::text_editor::create_text_editor(None).is_ok() {
        crate::desktop::text_editor::with_text_editor(|te| te.read().window_id()).unwrap_or(0)
    } else {
        0
    };

    // Focus the terminal by default
    if terminal_wid > 0 {
        crate::desktop::window_manager::with_window_manager(|wm| {
            let _ = wm.focus_window(terminal_wid as u32);
        });
    }

    crate::serial::_serial_print(format_args!(
        "[DESKTOP] Desktop scene created: bg + terminal({}) + files({}) + editor({}) + panel\n",
        terminal_wid, file_manager_wid, text_editor_wid
    ));

    DesktopApps {
        terminal_wid: terminal_wid as u32,
        file_manager_wid,
        text_editor_wid,
        panel_surface_id,
        panel_pool_id,
        panel_pool_buf_id,
    }
}

/// Draw a string into a BGRA pixel buffer at (px, py) with the given color.
///
/// Uses the 8x16 VGA font. Characters are spaced 8 pixels apart horizontally.
pub fn draw_string_into_buffer(
    buf: &mut [u8],
    buf_width: usize,
    text: &[u8],
    px: usize,
    py: usize,
    color: u32,
) {
    for (i, &ch) in text.iter().enumerate() {
        draw_char_into_buffer(buf, buf_width, ch, px + i * 8, py, color);
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
        .create_buffer(
            0,
            w,
            h,
            w * 4,
            crate::desktop::wayland::buffer::PixelFormat::Xrgb8888,
        )
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
        crate::desktop::wayland::buffer::PixelFormat::Xrgb8888,
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

/// Main compositor render loop.
///
/// Composites all Wayland surfaces, blits to the hardware framebuffer,
/// routes input events through the window manager to applications.
fn render_loop(hw: &fbcon::FbHwInfo, apps: &DesktopApps) {
    let fb_ptr = hw.fb_ptr;
    let fb_width = hw.width;
    let fb_height = hw.height;
    let fb_stride = hw.stride;
    let is_bgr = matches!(hw.pixel_format, FbPixelFormat::Bgr);
    let panel_y = (fb_height as u32 - crate::desktop::panel::PANEL_HEIGHT) as i32;

    // Initial composite
    let _drawn = crate::desktop::wayland::with_display(|display| display.wl_compositor.composite());

    // Do initial render of all app surfaces
    render_all_apps(apps);

    let mut frame_count: u64 = 0;

    loop {
        frame_count += 1;

        // --- 1. Poll hardware input ---
        crate::drivers::input_event::poll_all();
        let (mouse_x, mouse_y) = crate::drivers::mouse::cursor_position();

        // --- 2. Translate and dispatch input events ---
        while let Some(raw_event) = crate::drivers::input_event::read_event() {
            // ESC exits the GUI
            if raw_event.event_type == crate::drivers::input_event::EV_KEY
                && raw_event.code == 0x1B
                && raw_event.value == 1
            {
                crate::serial::_serial_print(format_args!("[DESKTOP] ESC pressed, exiting GUI\n"));
                return;
            }

            if let Some(wm_event) = translate_input_event(&raw_event, mouse_x, mouse_y) {
                // Check for panel click
                if let crate::desktop::window_manager::InputEvent::MouseButton {
                    pressed: true,
                    y,
                    x,
                    ..
                } = wm_event
                {
                    if y >= panel_y {
                        // Click is in the panel area -- handle panel click
                        if let Some(focus_wid) =
                            crate::desktop::panel::with_panel(|p| p.handle_click(x, y - panel_y))
                                .flatten()
                        {
                            crate::desktop::window_manager::with_window_manager(|wm| {
                                let _ = wm.focus_window(focus_wid);
                            });
                        }
                        continue;
                    }
                }

                // Dispatch to window manager (handles focus, queuing)
                crate::desktop::window_manager::with_window_manager(|wm| {
                    wm.process_input(wm_event);
                });
            }
        }

        // --- 3. Forward queued WM events to apps ---
        forward_events_to_apps(apps);

        // --- 4. Update app state ---
        // Terminal: read PTY output
        crate::desktop::terminal::with_terminal_manager(|tm| {
            let _ = tm.update_all();
        });

        // --- 5. Render apps and panel to surfaces (every 4th frame ~7.5fps for
        // content) ---
        if frame_count.is_multiple_of(4) {
            render_all_apps(apps);
        }

        // Panel: update clock + buttons periodically (every 60th frame ~0.5fps)
        if frame_count.is_multiple_of(60) {
            render_panel(apps, fb_width as u32);
        }

        // --- 6. Composite and blit ---
        // Composite every frame since apps may have updated
        crate::desktop::wayland::with_display(|display| {
            display.wl_compositor.request_composite();
            let _ = display.wl_compositor.composite();
        });

        let back_buffer: Vec<u32> =
            crate::desktop::wayland::with_display(|display| display.wl_compositor.back_buffer())
                .unwrap_or_default();

        if !back_buffer.is_empty() {
            blit_to_framebuffer(&back_buffer, fb_ptr, fb_width, fb_height, fb_stride, is_bgr);

            // Draw mouse cursor on top
            // SAFETY: fb_ptr is valid for stride * height bytes.
            let fb_slice =
                unsafe { core::slice::from_raw_parts_mut(fb_ptr, fb_stride * fb_height) };
            cursor::draw_cursor(fb_slice, fb_stride, fb_width, fb_height, mouse_x, mouse_y);
        }

        // ~30fps: yield CPU time
        for _ in 0..100_000 {
            core::hint::spin_loop();
        }
    }
}

/// Render all desktop app surfaces.
fn render_all_apps(apps: &DesktopApps) {
    // Terminal
    if apps.terminal_wid > 0 {
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
}

/// Render the panel to its compositor surface.
fn render_panel(apps: &DesktopApps, screen_width: u32) {
    let panel_h = crate::desktop::panel::PANEL_HEIGHT;
    crate::desktop::panel::with_panel(|panel| {
        panel.update_buttons();
        panel.update_clock();
        let mut buf = vec![0u8; (screen_width as usize) * (panel_h as usize) * 4];
        panel.render(&mut buf);
        update_surface_pixels(
            apps.panel_surface_id,
            apps.panel_pool_id,
            apps.panel_pool_buf_id,
            &buf,
        );
    });
}

/// Forward pending window manager events to the appropriate apps.
fn forward_events_to_apps(apps: &DesktopApps) {
    // Get events for terminal window
    if apps.terminal_wid > 0 {
        let events = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_events(apps.terminal_wid)
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
    if apps.file_manager_wid > 0 {
        let events = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_events(apps.file_manager_wid)
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
    if apps.text_editor_wid > 0 {
        let events = crate::desktop::window_manager::with_window_manager(|wm| {
            wm.get_events(apps.text_editor_wid)
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
}

/// Blit the compositor's XRGB8888 back-buffer to the hardware framebuffer.
///
/// Handles BGR vs RGB pixel format conversion.
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

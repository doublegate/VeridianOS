//! Desktop Renderer
//!
//! Connects the Wayland compositor's back-buffer to the hardware framebuffer.
//! Creates the initial desktop scene (background gradient, panel, terminal
//! placeholder) and runs the compositing loop.

use alloc::{vec, vec::Vec};

use crate::graphics::{
    cursor,
    fbcon::{self, FbPixelFormat},
};

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
    create_desktop_scene(hw.width as u32, hw.height as u32);

    // Disable fbcon text output -- the compositor takes over the framebuffer
    fbcon::disable_output();

    // Clear the framebuffer to black before first composite
    // SAFETY: hw.fb_ptr is valid for stride * height bytes.
    unsafe {
        core::ptr::write_bytes(hw.fb_ptr, 0, hw.stride * hw.height);
    }

    crate::serial::_serial_print(format_args!("[DESKTOP] Entering compositor render loop\n"));

    // Render loop: composite -> blit to framebuffer -> poll input -> repeat
    render_loop(&hw);

    // If we exit the render loop, re-enable fbcon
    fbcon::enable_output();
    crate::println!("[DESKTOP] GUI stopped, returning to text console");
}

/// Create the initial desktop scene with a background gradient and demo
/// windows.
fn create_desktop_scene(width: u32, height: u32) {
    // Create a background surface covering the entire screen
    let bg_surface_id = 1000;
    crate::desktop::wayland::with_display(|display| {
        let _ = display.wl_compositor.create_surface(bg_surface_id);
        display
            .wl_compositor
            .set_surface_position(bg_surface_id, 0, 0);
    });

    // Create a gradient background in an SHM pool
    let pool_size = (width as usize) * (height as usize) * 4;
    let mut bg_pixels = vec![0u8; pool_size];
    paint_gradient_background(&mut bg_pixels, width as usize, height as usize);

    // Register the pool and buffer
    let pool_id = 100;
    let mut pool = crate::desktop::wayland::buffer::WlShmPool::new(pool_id, 0, pool_size);

    // Write the gradient pixels into the pool's backing memory
    pool.write_data(0, &bg_pixels);

    // Create a buffer in the pool
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

    // Attach the buffer to the background surface and commit
    let bg_buffer = crate::desktop::wayland::buffer::Buffer::from_pool(
        1,       // buffer object ID
        pool_id, // pool ID
        buf_id,  // pool buffer ID (returned by create_buffer)
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

    // Create a demo window (colored rectangle) in the center
    create_demo_window(width, height, 200, 100, 400, 300, 0xFF1A73E8); // Blue
    create_demo_window(width, height, 350, 200, 350, 250, 0xFF34A853); // Green

    crate::serial::_serial_print(format_args!(
        "[DESKTOP] Desktop scene created: bg + 2 demo windows\n"
    ));
}

/// Create a demo window surface with a solid color + title bar.
fn create_demo_window(_screen_w: u32, _screen_h: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    static NEXT_SURFACE_ID: core::sync::atomic::AtomicU32 =
        core::sync::atomic::AtomicU32::new(2000);
    static NEXT_POOL_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(200);

    let surface_id = NEXT_SURFACE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let pool_id = NEXT_POOL_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

    // Create surface
    crate::desktop::wayland::with_display(|display| {
        let _ = display.wl_compositor.create_surface(surface_id);
        display.wl_compositor.set_surface_position(surface_id, x, y);
    });

    // Create pixel data: title bar (dark) + body (color)
    let pool_size = (w as usize) * (h as usize) * 4;
    let mut pixels = vec![0u8; pool_size];
    let title_bar_height = 28;

    for row in 0..h as usize {
        for col in 0..w as usize {
            let offset = (row * w as usize + col) * 4;
            let pixel_color = if row < title_bar_height {
                // Title bar: dark gray
                0xFF333333u32
            } else if row == title_bar_height {
                // Border line
                0xFF555555u32
            } else {
                color
            };
            // BGRA format
            pixels[offset] = (pixel_color & 0xFF) as u8; // B
            pixels[offset + 1] = ((pixel_color >> 8) & 0xFF) as u8; // G
            pixels[offset + 2] = ((pixel_color >> 16) & 0xFF) as u8; // R
            pixels[offset + 3] = 0xFF; // A
        }
    }

    // Draw title bar text "Window" using font glyphs
    let title = b"VeridianOS";
    for (i, &ch) in title.iter().enumerate() {
        draw_char_into_buffer(&mut pixels, w as usize, ch, 8 + i * 8, 6, 0xCCCCCC);
    }

    // Draw close button (X) in top-right
    let close_x = w as usize - 24;
    draw_char_into_buffer(&mut pixels, w as usize, b'X', close_x, 6, 0xFF4444);

    // Register pool + buffer
    let mut pool = crate::desktop::wayland::buffer::WlShmPool::new(pool_id, 0, pool_size);
    pool.write_data(0, &pixels);
    let win_buf_id = pool
        .create_buffer(
            0,
            w,
            h,
            w * 4,
            crate::desktop::wayland::buffer::PixelFormat::Xrgb8888,
        )
        .unwrap_or(0);
    crate::desktop::wayland::buffer::register_pool(pool);

    let buffer = crate::desktop::wayland::buffer::Buffer::from_pool(
        surface_id + 100,
        pool_id,
        win_buf_id,
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
        display.wl_compositor.request_composite();
    });
}

/// Draw a single 8x16 character into a BGRA pixel buffer.
fn draw_char_into_buffer(
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

/// Main compositor render loop.
///
/// Composites all Wayland surfaces, blits to the hardware framebuffer,
/// and polls for mouse/keyboard input.
fn render_loop(hw: &fbcon::FbHwInfo) {
    let fb_ptr = hw.fb_ptr;
    let fb_width = hw.width;
    let fb_height = hw.height;
    let fb_stride = hw.stride;
    let is_bgr = matches!(hw.pixel_format, FbPixelFormat::Bgr);

    // Initial composite
    let _drawn = crate::desktop::wayland::with_display(|display| display.wl_compositor.composite());

    let mut frame_count: u64 = 0;

    loop {
        frame_count += 1;

        // Poll input events
        crate::drivers::input_event::poll_all();

        // Get mouse position for cursor rendering
        let (mouse_x, mouse_y) = crate::drivers::mouse::cursor_position();

        // Check if any key was pressed (ESC to exit GUI)
        while let Some(event) = crate::drivers::input_event::read_event() {
            if event.event_type == crate::drivers::input_event::EV_KEY
                && event.code == 0x1B  // ESC key
                && event.value == 1
            {
                crate::serial::_serial_print(format_args!("[DESKTOP] ESC pressed, exiting GUI\n"));
                return;
            }
        }

        // Composite if needed (or every 60th frame for periodic refresh)
        let should_composite = frame_count.is_multiple_of(60);

        if should_composite {
            crate::desktop::wayland::with_display(|display| {
                display.wl_compositor.request_composite();
                let _ = display.wl_compositor.composite();
            });
        }

        // Blit compositor back-buffer to hardware framebuffer
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

        // ~30fps: yield CPU time to other tasks
        // On QEMU with KVM this is fast enough; without KVM it'll be slower
        for _ in 0..100_000 {
            core::hint::spin_loop();
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

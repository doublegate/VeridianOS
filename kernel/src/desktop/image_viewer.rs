//! Image Viewer
//!
//! Displays PPM (P3/P6) and BMP images with scaling support.

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

use super::renderer::draw_string_into_buffer;

// ---------------------------------------------------------------------------
// Image types
// ---------------------------------------------------------------------------

/// Supported image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Ppm,
    Bmp,
    Tga,
    Qoi,
    Unknown,
}

/// Decoded image stored as BGRA `u32` pixels.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// Row-major BGRA pixels: `pixels[y * width + x]`.
    pub pixels: Vec<u32>,
    pub format: ImageFormat,
}

/// State of the viewer.
#[derive(Debug, Clone)]
pub enum ImageViewerState {
    Empty,
    Loading,
    Loaded,
    Error(String),
}

// ---------------------------------------------------------------------------
// Actions returned by input handlers
// ---------------------------------------------------------------------------

/// Action produced by image viewer interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageViewerAction {
    None,
    Close,
    ZoomIn,
    ZoomOut,
    ZoomFit,
}

// ---------------------------------------------------------------------------
// Main application struct
// ---------------------------------------------------------------------------

/// Image viewer application state.
pub struct ImageViewer {
    pub state: ImageViewerState,
    pub image: Option<Image>,
    pub filename: String,

    /// Zoom level as a percentage (100 = 1:1). Range 25..=400.
    pub zoom_level: usize,

    /// Pan offset (pixels in image-space).
    pub offset_x: isize,
    pub offset_y: isize,

    /// Compositor surface ID (set when wired to the desktop).
    pub surface_id: Option<u32>,

    /// Window dimensions.
    pub width: usize,
    pub height: usize,

    /// Height of the toolbar in pixels.
    pub toolbar_height: usize,
}

const ZOOM_MIN: usize = 25;
const ZOOM_MAX: usize = 400;
const ZOOM_STEP: usize = 25;
const PAN_STEP: isize = 16;

impl ImageViewer {
    /// Create a new, empty image viewer.
    pub fn new() -> Self {
        Self {
            state: ImageViewerState::Empty,
            image: None,
            filename: String::new(),
            zoom_level: 100,
            offset_x: 0,
            offset_y: 0,
            surface_id: None,
            width: 640,
            height: 480,
            toolbar_height: 32,
        }
    }

    // -----------------------------------------------------------------------
    // Loaders
    // -----------------------------------------------------------------------

    /// Load a PPM image (P3 ASCII or P6 binary).
    pub fn load_ppm(data: &[u8]) -> Result<Image, &'static str> {
        if data.len() < 3 {
            return Err("ppm: data too short");
        }

        let is_p6 = data.starts_with(b"P6");
        let is_p3 = data.starts_with(b"P3");
        if !is_p3 && !is_p6 {
            return Err("ppm: unsupported magic (expected P3 or P6)");
        }

        // Find the header portion (width, height, maxval).
        // Skip comments (lines starting with '#').
        let mut pos: usize = 2; // skip "Px"
        let mut tokens: Vec<usize> = Vec::new();

        while tokens.len() < 3 && pos < data.len() {
            // Skip whitespace
            while pos < data.len()
                && (data[pos] == b' '
                    || data[pos] == b'\n'
                    || data[pos] == b'\r'
                    || data[pos] == b'\t')
            {
                pos += 1;
            }
            // Skip comment
            if pos < data.len() && data[pos] == b'#' {
                while pos < data.len() && data[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            // Read number
            let start = pos;
            while pos < data.len() && data[pos] >= b'0' && data[pos] <= b'9' {
                pos += 1;
            }
            if pos > start {
                let num = parse_ascii_usize(&data[start..pos])?;
                tokens.push(num);
            }
        }

        if tokens.len() < 3 {
            return Err("ppm: incomplete header");
        }

        let width = tokens[0];
        let height = tokens[1];
        let max_val = tokens[2];
        if width == 0 || height == 0 || max_val == 0 {
            return Err("ppm: zero dimension or maxval");
        }

        let pixel_count = width * height;
        let mut pixels = Vec::with_capacity(pixel_count);

        if is_p6 {
            // Binary -- skip exactly one whitespace byte after maxval
            if pos < data.len() {
                pos += 1;
            }
            let bpp = if max_val > 255 { 6 } else { 3 };
            let needed = pixel_count * bpp;
            if pos + needed > data.len() {
                return Err("ppm: P6 data truncated");
            }
            if bpp == 3 {
                for i in 0..pixel_count {
                    let base = pos + i * 3;
                    let r = data[base] as u32;
                    let g = data[base + 1] as u32;
                    let b = data[base + 2] as u32;
                    pixels.push(0xFF00_0000 | (r << 16) | (g << 8) | b);
                }
            } else {
                // 16-bit channels -- take high byte
                for i in 0..pixel_count {
                    let base = pos + i * 6;
                    let r = data[base] as u32;
                    let g = data[base + 2] as u32;
                    let b = data[base + 4] as u32;
                    pixels.push(0xFF00_0000 | (r << 16) | (g << 8) | b);
                }
            }
        } else {
            // ASCII (P3) -- read 3 * pixel_count integers
            let mut rgb_vals: Vec<u32> = Vec::new();
            while rgb_vals.len() < pixel_count * 3 && pos < data.len() {
                // Skip whitespace / comments
                while pos < data.len()
                    && (data[pos] == b' '
                        || data[pos] == b'\n'
                        || data[pos] == b'\r'
                        || data[pos] == b'\t')
                {
                    pos += 1;
                }
                if pos < data.len() && data[pos] == b'#' {
                    while pos < data.len() && data[pos] != b'\n' {
                        pos += 1;
                    }
                    continue;
                }
                let start = pos;
                while pos < data.len() && data[pos] >= b'0' && data[pos] <= b'9' {
                    pos += 1;
                }
                if pos > start {
                    let v = parse_ascii_usize(&data[start..pos]).unwrap_or(0) as u32;
                    // Normalise to 0..255 if max_val != 255
                    let normalised = if max_val != 255 {
                        v * 255 / (max_val as u32)
                    } else {
                        v
                    };
                    rgb_vals.push(normalised);
                }
            }

            if rgb_vals.len() < pixel_count * 3 {
                return Err("ppm: P3 data truncated");
            }

            for i in 0..pixel_count {
                let r = rgb_vals[i * 3];
                let g = rgb_vals[i * 3 + 1];
                let b = rgb_vals[i * 3 + 2];
                pixels.push(0xFF00_0000 | (r << 16) | (g << 8) | b);
            }
        }

        Ok(Image {
            width,
            height,
            pixels,
            format: ImageFormat::Ppm,
        })
    }

    /// Load a BMP image (24-bit or 32-bit uncompressed).
    pub fn load_bmp(data: &[u8]) -> Result<Image, &'static str> {
        if data.len() < 54 {
            return Err("bmp: data too short for header");
        }
        if data[0] != 0x42 || data[1] != 0x4D {
            return Err("bmp: invalid magic");
        }

        // BMP header fields (little-endian)
        let data_offset = read_le_u32(data, 10) as usize;
        let dib_size = read_le_u32(data, 14);
        if dib_size < 40 {
            return Err("bmp: unsupported DIB header size");
        }

        let width = read_le_i32(data, 18);
        let height_raw = read_le_i32(data, 22);
        let bpp = read_le_u16(data, 28) as usize;
        let compression = read_le_u32(data, 30);

        if compression != 0 && compression != 3 {
            return Err("bmp: compressed BMPs not supported");
        }
        if bpp != 24 && bpp != 32 {
            return Err("bmp: only 24-bit and 32-bit supported");
        }
        if width <= 0 {
            return Err("bmp: invalid width");
        }

        let w = width as usize;
        // Negative height means top-down storage.
        let (h, bottom_up) = if height_raw < 0 {
            ((-height_raw) as usize, false)
        } else {
            (height_raw as usize, true)
        };

        if w == 0 || h == 0 {
            return Err("bmp: zero dimension");
        }

        let bytes_per_pixel = bpp / 8;
        let row_size_raw = w * bytes_per_pixel;
        // BMP rows are padded to 4-byte boundaries.
        let row_stride = (row_size_raw + 3) & !3;

        let needed = data_offset + row_stride * h;
        if data.len() < needed {
            return Err("bmp: pixel data truncated");
        }

        let pixel_count = w * h;
        let mut pixels = vec![0u32; pixel_count];

        for row in 0..h {
            let src_row = if bottom_up { h - 1 - row } else { row };
            let src_off = data_offset + src_row * row_stride;
            let dst_off = row * w;

            for col in 0..w {
                let px_off = src_off + col * bytes_per_pixel;
                let b = data[px_off] as u32;
                let g = data[px_off + 1] as u32;
                let r = data[px_off + 2] as u32;
                let a = if bpp == 32 {
                    data[px_off + 3] as u32
                } else {
                    0xFF
                };
                pixels[dst_off + col] = (a << 24) | (r << 16) | (g << 8) | b;
            }
        }

        Ok(Image {
            width: w,
            height: h,
            pixels,
            format: ImageFormat::Bmp,
        })
    }

    /// Load a TGA image via the video decoder, converting to BGRA u32 pixels.
    pub fn load_tga(data: &[u8]) -> Result<Image, &'static str> {
        let frame = crate::video::decode::decode_tga(data).map_err(|_| "tga: decode failed")?;
        Ok(video_frame_to_image(&frame, ImageFormat::Tga))
    }

    /// Load a QOI image via the video decoder, converting to BGRA u32 pixels.
    pub fn load_qoi(data: &[u8]) -> Result<Image, &'static str> {
        let frame = crate::video::decode::decode_qoi(data).map_err(|_| "qoi: decode failed")?;
        Ok(video_frame_to_image(&frame, ImageFormat::Qoi))
    }

    /// Load an image file from raw byte data, auto-detecting the format.
    pub fn load_file(&mut self, filename: &str, data: &[u8]) {
        self.filename = String::from(filename);
        self.state = ImageViewerState::Loading;

        let fmt = detect_image_format(data);
        let result = match fmt {
            ImageFormat::Ppm => Self::load_ppm(data),
            ImageFormat::Bmp => Self::load_bmp(data),
            ImageFormat::Tga => Self::load_tga(data),
            ImageFormat::Qoi => Self::load_qoi(data),
            ImageFormat::Unknown => {
                // Try extension-based detection
                if filename.ends_with(".ppm") || filename.ends_with(".pnm") {
                    Self::load_ppm(data)
                } else if filename.ends_with(".bmp") {
                    Self::load_bmp(data)
                } else if filename.ends_with(".tga") {
                    Self::load_tga(data)
                } else if filename.ends_with(".qoi") {
                    Self::load_qoi(data)
                } else {
                    Err("unknown image format")
                }
            }
        };

        match result {
            Ok(img) => {
                self.image = Some(img);
                self.state = ImageViewerState::Loaded;
                self.zoom_level = 100;
                self.offset_x = 0;
                self.offset_y = 0;
            }
            Err(e) => {
                self.image = None;
                self.state = ImageViewerState::Error(String::from(e));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Zoom & pan
    // -----------------------------------------------------------------------

    /// Increase zoom by one step.
    pub fn zoom_in(&mut self) {
        if self.zoom_level < ZOOM_MAX {
            self.zoom_level += ZOOM_STEP;
        }
    }

    /// Decrease zoom by one step.
    pub fn zoom_out(&mut self) {
        if self.zoom_level > ZOOM_MIN {
            self.zoom_level -= ZOOM_STEP;
        }
    }

    /// Fit image to viewer area.
    pub fn zoom_fit(&mut self) {
        if let Some(ref img) = self.image {
            if img.width == 0 || img.height == 0 {
                return;
            }
            let view_w = self.width;
            let view_h = self.height.saturating_sub(self.toolbar_height);
            // zoom = min(view_w * 100 / img_w, view_h * 100 / img_h), clamped
            let zx = view_w * 100 / img.width;
            let zy = view_h * 100 / img.height;
            let z = zx.min(zy).clamp(ZOOM_MIN, ZOOM_MAX);
            self.zoom_level = z;
            self.offset_x = 0;
            self.offset_y = 0;
        }
    }

    // -----------------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------------

    /// Handle a keyboard event and return the resulting action.
    pub fn handle_key(&mut self, key: u8) -> ImageViewerAction {
        match key {
            b'+' | b'=' => {
                self.zoom_in();
                ImageViewerAction::ZoomIn
            }
            b'-' => {
                self.zoom_out();
                ImageViewerAction::ZoomOut
            }
            b'0' => {
                self.zoom_fit();
                ImageViewerAction::ZoomFit
            }
            // Arrow-key navigation (vi-style j/k/h/l)
            b'h' | b'H' => {
                self.offset_x -= PAN_STEP;
                ImageViewerAction::None
            }
            b'l' | b'L' => {
                self.offset_x += PAN_STEP;
                ImageViewerAction::None
            }
            b'k' | b'K' => {
                self.offset_y -= PAN_STEP;
                ImageViewerAction::None
            }
            b'j' | b'J' => {
                self.offset_y += PAN_STEP;
                ImageViewerAction::None
            }
            0x1B => ImageViewerAction::Close,
            _ => ImageViewerAction::None,
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    /// Render the image viewer into a `u32` BGRA pixel buffer.
    ///
    /// `buffer` must be at least `buf_width * buf_height` elements.
    pub fn render_to_buffer(&self, buffer: &mut [u32], buf_width: usize, buf_height: usize) {
        let byte_len = buf_width * buf_height * 4;
        let mut byte_buf = vec![0u8; byte_len];

        // -- checkerboard background below toolbar --
        let tb_h = self.toolbar_height;
        for y in tb_h..buf_height {
            for x in 0..buf_width {
                let off = (y * buf_width + x) * 4;
                if off + 3 >= byte_buf.len() {
                    break;
                }
                // 16x16 checkerboard
                let gray: u8 = if ((x >> 4) ^ ((y - tb_h) >> 4)) & 1 == 0 {
                    0x3C
                } else {
                    0x48
                };
                byte_buf[off] = gray;
                byte_buf[off + 1] = gray;
                byte_buf[off + 2] = gray;
                byte_buf[off + 3] = 0xFF;
            }
        }

        // -- toolbar background (dark) --
        for y in 0..tb_h.min(buf_height) {
            for x in 0..buf_width {
                let off = (y * buf_width + x) * 4;
                if off + 3 < byte_buf.len() {
                    byte_buf[off] = 0x2A;
                    byte_buf[off + 1] = 0x2A;
                    byte_buf[off + 2] = 0x2A;
                    byte_buf[off + 3] = 0xFF;
                }
            }
        }

        // -- toolbar text --
        {
            // Filename
            if !self.filename.is_empty() {
                draw_string_into_buffer(
                    &mut byte_buf,
                    buf_width,
                    self.filename.as_bytes(),
                    8,
                    8,
                    0xDDDDDD,
                );
            } else {
                draw_string_into_buffer(&mut byte_buf, buf_width, b"(no image)", 8, 8, 0x888888);
            }

            // Zoom indicator on right side
            let zoom_str = format_zoom(self.zoom_level);
            let zoom_x = buf_width.saturating_sub(zoom_str.len() * 8 + 8);
            draw_string_into_buffer(&mut byte_buf, buf_width, &zoom_str, zoom_x, 8, 0xAAFFAA);

            // Zoom buttons: [-] [+]  (visual hint; not clickable yet)
            let btn_x = zoom_x.saturating_sub(56);
            draw_string_into_buffer(&mut byte_buf, buf_width, b"[-] [+]", btn_x, 8, 0x888888);
        }

        // -- separator line under toolbar --
        {
            let y = tb_h.saturating_sub(1);
            for x in 0..buf_width {
                let off = (y * buf_width + x) * 4;
                if off + 3 < byte_buf.len() {
                    byte_buf[off] = 0x55;
                    byte_buf[off + 1] = 0x55;
                    byte_buf[off + 2] = 0x55;
                    byte_buf[off + 3] = 0xFF;
                }
            }
        }

        // -- draw image --
        if let Some(ref img) = self.image {
            let view_w = buf_width;
            let view_h = buf_height.saturating_sub(tb_h);

            // Scaled image dimensions (integer math)
            let scaled_w = img.width * self.zoom_level / 100;
            let scaled_h = img.height * self.zoom_level / 100;

            // Center if smaller than viewport, otherwise use offset
            let base_x: isize = if scaled_w < view_w {
                ((view_w - scaled_w) / 2) as isize
            } else {
                -self.offset_x
            };
            let base_y: isize = if scaled_h < view_h {
                ((view_h - scaled_h) / 2) as isize
            } else {
                -self.offset_y
            };

            // Blit with nearest-neighbor scaling
            for dy in 0..view_h {
                let dst_y = tb_h + dy;
                if dst_y >= buf_height {
                    break;
                }

                let img_rel_y = dy as isize - base_y;
                if img_rel_y < 0 || img_rel_y >= scaled_h as isize {
                    continue;
                }
                // Map back to source pixel: src_y = rel_y * 100 / zoom
                let src_y = (img_rel_y as usize) * 100 / self.zoom_level;
                if src_y >= img.height {
                    continue;
                }

                for dx in 0..view_w {
                    if dx >= buf_width {
                        break;
                    }

                    let img_rel_x = dx as isize - base_x;
                    if img_rel_x < 0 || img_rel_x >= scaled_w as isize {
                        continue;
                    }
                    let src_x = (img_rel_x as usize) * 100 / self.zoom_level;
                    if src_x >= img.width {
                        continue;
                    }

                    let src_px = img.pixels[src_y * img.width + src_x];
                    let a = ((src_px >> 24) & 0xFF) as u8;
                    let r = ((src_px >> 16) & 0xFF) as u8;
                    let g = ((src_px >> 8) & 0xFF) as u8;
                    let b = (src_px & 0xFF) as u8;

                    let off = (dst_y * buf_width + dx) * 4;
                    if off + 3 < byte_buf.len() {
                        if a == 0xFF {
                            byte_buf[off] = b;
                            byte_buf[off + 1] = g;
                            byte_buf[off + 2] = r;
                            byte_buf[off + 3] = 0xFF;
                        } else if a > 0 {
                            // Alpha blend with background (integer math)
                            let inv = 255 - a as u16;
                            let a16 = a as u16;
                            byte_buf[off] =
                                ((b as u16 * a16 + byte_buf[off] as u16 * inv) / 255) as u8;
                            byte_buf[off + 1] =
                                ((g as u16 * a16 + byte_buf[off + 1] as u16 * inv) / 255) as u8;
                            byte_buf[off + 2] =
                                ((r as u16 * a16 + byte_buf[off + 2] as u16 * inv) / 255) as u8;
                            byte_buf[off + 3] = 0xFF;
                        }
                        // a == 0: fully transparent, keep background
                    }
                }
            }
        } else {
            // No image -- show status message
            let msg: &[u8] = match &self.state {
                ImageViewerState::Empty => b"No image loaded. Open PPM, BMP, TGA, or QOI.",
                ImageViewerState::Loading => b"Loading...",
                ImageViewerState::Error(_) => b"Error loading image.",
                ImageViewerState::Loaded => b"(empty)",
            };
            let msg_x = buf_width / 2 - (msg.len() * 8) / 2;
            let msg_y = tb_h + (buf_height.saturating_sub(tb_h)) / 2;
            draw_string_into_buffer(&mut byte_buf, buf_width, msg, msg_x, msg_y, 0x888888);
        }

        // Convert byte buffer (BGRA u8) into u32 buffer
        for (i, chunk) in byte_buf.chunks_exact(4).enumerate() {
            if i < buffer.len() {
                buffer[i] = (chunk[3] as u32) << 24
                    | (chunk[2] as u32) << 16
                    | (chunk[1] as u32) << 8
                    | (chunk[0] as u32);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone helpers
// ---------------------------------------------------------------------------

/// Detect image format from the first bytes of the data.
pub fn detect_image_format(data: &[u8]) -> ImageFormat {
    if data.len() >= 4 {
        // QOI: magic "qoif"
        if data[0] == b'q' && data[1] == b'o' && data[2] == b'i' && data[3] == b'f' {
            return ImageFormat::Qoi;
        }
    }
    if data.len() >= 2 {
        // PPM magic: P3 or P6
        if data[0] == b'P' && (data[1] == b'3' || data[1] == b'6') {
            return ImageFormat::Ppm;
        }
        // BMP magic: 0x42 0x4D ("BM")
        if data[0] == 0x42 && data[1] == 0x4D {
            return ImageFormat::Bmp;
        }
    }
    // TGA heuristic (no reliable magic): check header fields
    if data.len() >= 18 {
        let color_map_type = data[1];
        let image_type = data[2];
        let pixel_depth = data[16];
        let valid_cmt = color_map_type <= 1;
        let valid_type = matches!(image_type, 1 | 2 | 3 | 9 | 10 | 11);
        let valid_depth = matches!(pixel_depth, 8 | 15 | 16 | 24 | 32);
        if valid_cmt && valid_type && valid_depth {
            return ImageFormat::Tga;
        }
    }
    ImageFormat::Unknown
}

/// Convert a `VideoFrame` from the video subsystem into the image viewer's
/// `Image` format (BGRA u32 pixels).
fn video_frame_to_image(frame: &crate::video::VideoFrame, fmt: ImageFormat) -> Image {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let pixel_count = w * h;
    let mut pixels = Vec::with_capacity(pixel_count);

    for y in 0..frame.height {
        for x in 0..frame.width {
            let (r, g, b, a) = frame.get_pixel(x, y);
            // Image stores BGRA as u32: A(31:24) R(23:16) G(15:8) B(7:0)
            let px = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            pixels.push(px);
        }
    }

    Image {
        width: w,
        height: h,
        pixels,
        format: fmt,
    }
}

/// Scale an image to `dst_width x dst_height` using nearest-neighbor sampling.
///
/// Returns a new pixel buffer.
pub fn nearest_neighbor_scale(src: &Image, dst_width: usize, dst_height: usize) -> Vec<u32> {
    if dst_width == 0 || dst_height == 0 || src.width == 0 || src.height == 0 {
        return Vec::new();
    }

    let mut out = vec![0u32; dst_width * dst_height];
    for dy in 0..dst_height {
        let sy = dy * src.height / dst_height;
        let sy = sy.min(src.height - 1);
        for dx in 0..dst_width {
            let sx = dx * src.width / dst_width;
            let sx = sx.min(src.width - 1);
            out[dy * dst_width + dx] = src.pixels[sy * src.width + sx];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Integer parsing / formatting helpers (no_std friendly)
// ---------------------------------------------------------------------------

/// Parse an ASCII decimal number from a byte slice.
fn parse_ascii_usize(bytes: &[u8]) -> Result<usize, &'static str> {
    let mut val: usize = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return Err("non-digit in number");
        }
        val = val
            .checked_mul(10)
            .and_then(|v| v.checked_add((b - b'0') as usize))
            .ok_or("number overflow")?;
    }
    Ok(val)
}

/// Read a little-endian u32 from a byte slice at the given offset.
fn read_le_u32(data: &[u8], off: usize) -> u32 {
    (data[off] as u32)
        | ((data[off + 1] as u32) << 8)
        | ((data[off + 2] as u32) << 16)
        | ((data[off + 3] as u32) << 24)
}

/// Read a little-endian i32 from a byte slice at the given offset.
fn read_le_i32(data: &[u8], off: usize) -> i32 {
    read_le_u32(data, off) as i32
}

/// Read a little-endian u16 from a byte slice at the given offset.
fn read_le_u16(data: &[u8], off: usize) -> u16 {
    (data[off] as u16) | ((data[off + 1] as u16) << 8)
}

impl ImageViewer {
    /// Render the image viewer into a `u8` BGRA pixel buffer.
    ///
    /// Delegates to `render_to_buffer` (u32), then converts to u8 bytes.
    pub fn render_to_u8_buffer(&self, buf: &mut [u8], buf_width: usize, buf_height: usize) {
        let pixel_count = buf_width * buf_height;
        let mut u32_buf = vec![0u32; pixel_count];
        self.render_to_buffer(&mut u32_buf, buf_width, buf_height);
        // Convert u32 BGRA pixels to u8 BGRA bytes
        for (i, &px) in u32_buf.iter().enumerate() {
            let off = i * 4;
            if off + 3 < buf.len() {
                buf[off] = (px & 0xFF) as u8; // B
                buf[off + 1] = ((px >> 8) & 0xFF) as u8; // G
                buf[off + 2] = ((px >> 16) & 0xFF) as u8; // R
                buf[off + 3] = ((px >> 24) & 0xFF) as u8; // A
            }
        }
    }
}

impl Default for ImageViewer {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a zoom percentage as a stack-allocated ASCII string.
///
/// Returns a `Vec<u8>` like `b"100%"`.
fn format_zoom(pct: usize) -> Vec<u8> {
    use alloc::format;
    let s = format!("{}%", pct);
    s.into_bytes()
}

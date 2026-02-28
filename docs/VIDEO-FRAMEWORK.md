# VeridianOS Video Framework

**Version:** v0.9.0 (Phase 7 Wave 5)
**Status:** Planned -- pixel formats, image decoding, scaling, media playback

---

## Overview

The VeridianOS video framework (`kernel/src/video/`) provides pixel format
handling, image decoding, frame scaling, and basic media playback. It builds on
the existing framebuffer infrastructure (UEFI GOP on x86_64, ramfb on
AArch64/RISC-V) and integrates with the desktop compositor for windowed display.

Design principles:

1. **Integer-only math** -- all scaling, color conversion, and blending use
   fixed-point or integer arithmetic. No floating point in kernel context.
2. **Format-aware pipeline** -- pixel formats are explicit at every stage.
   Conversions happen at well-defined boundaries, not implicitly.
3. **Decoder extensibility** -- image decoders register via a common trait.
   Adding a new format requires implementing `ImageDecoder` without touching
   the rest of the pipeline.

---

## Architecture Diagram

```
+-----------------------------------------------------------------------+
|                       User Space / Desktop                             |
|   +-------------------+  +-------------------+  +------------------+  |
|   | image-viewer      |  | media-player      |  | desktop wallpaper|  |
|   | (TGA/QOI/PPM/BMP) |  | (raw video)       |  | (static image)   |  |
|   +---------+---------+  +---------+---------+  +--------+---------+  |
|             |                      |                     |            |
+=============|======================|=====================|============+
              |   syscall / kernel API                     |
+=============|======================|=====================|============+
|             v                      v                     v            |
|   +---------+-------------------------------------------+---------+  |
|   |                    VideoFrame                                 |  |
|   |  Decoded pixel buffer + format + stride metadata              |  |
|   |                                          video/mod.rs         |  |
|   +----------+---------------------+-------------------+----------+  |
|              |                     |                   |             |
|     +--------v--------+  +--------v--------+  +-------v--------+    |
|     | Image Decoders  |  | Scaler / Color  |  | Media Player   |    |
|     | TGA, QOI, auto- |  | Nearest/bilinear|  | RawVideoStream |    |
|     | detect by magic |  | YUV<->RGB, alpha|  | frame timing   |    |
|     | video/decode.rs |  | video/          |  | video/player.rs|    |
|     +--------+--------+  | framebuffer.rs  |  +-------+--------+    |
|              |            +--------+--------+          |             |
|              |                     |                   |             |
|              v                     v                   v             |
|   +----------+---------------------+-------------------+----------+  |
|   |              Framebuffer Blit                                 |  |
|   |  MMIO write to GOP / ramfb / VirtIO-GPU scanout               |  |
|   +---------------------------------------------------------------+  |
+-----------------------------------------------------------------------+
```

---

## Pixel Formats

Every pixel buffer in the framework carries an explicit `PixelFormat` tag.
Format conversion is performed only when source and destination differ.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit: 0xXXRRGGBB (X = ignored). Native for VirtIO-GPU.
    Xrgb8888,
    /// 32-bit: 0xAARRGGBB. Pre-multiplied alpha.
    Argb8888,
    /// 24-bit packed: R, G, B bytes. Common in image files.
    Rgb888,
    /// 16-bit: 5-6-5 bit packing. Low-memory displays.
    Rgb565,
    /// 24-bit packed: B, G, R bytes. VeridianOS UEFI GOP native.
    Bgr888,
    /// 8-bit grayscale. Single-channel images.
    Gray8,
}
```

### Conversion Routines

Conversions are implemented as `pixel_to_xrgb8888()` functions since
XRGB8888 is the compositor's internal format:

| Source | Target | Method |
|--------|--------|--------|
| RGB888 | XRGB8888 | `(r << 16) \| (g << 8) \| b` |
| BGR888 | XRGB8888 | `(b << 16) \| (g << 8) \| r` (swap R/B) |
| ARGB8888 | XRGB8888 | Alpha-blend against background, drop alpha |
| RGB565 | XRGB8888 | Expand 5-6-5 to 8-8-8 with bit replication |
| Gray8 | XRGB8888 | `(v << 16) \| (v << 8) \| v` |

BGR888 is the native framebuffer format on x86_64 UEFI GOP (1280x800). The
compositor stores surfaces as XRGB8888 `u32` arrays and converts to BGR888
during the final blit to MMIO.

---

## Video Frame (`video/mod.rs`)

`VideoFrame` is the central data structure. It holds decoded pixel data with
format metadata and provides indexed pixel access.

```rust
pub struct VideoFrame {
    /// Pixel data, row-major. Layout depends on `format`.
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Bytes per row (may include padding beyond width * bpp).
    pub stride: usize,
    pub format: PixelFormat,
}

impl VideoFrame {
    /// Read a pixel as XRGB8888 regardless of underlying format.
    pub fn pixel_xrgb(&self, x: usize, y: usize) -> u32 { ... }

    /// Bytes per pixel for the current format.
    pub fn bpp(&self) -> usize { ... }

    /// Create a new frame filled with a solid color.
    pub fn solid(width: usize, height: usize, color: u32) -> Self { ... }

    /// Create from raw pixel data with explicit format.
    pub fn from_raw(
        data: Vec<u8>,
        width: usize,
        height: usize,
        format: PixelFormat,
    ) -> Self { ... }
}
```

`stride` allows for row padding, which some image formats and hardware
scanouts require for alignment. All decoders set `stride = width * bpp`
unless the source format specifies otherwise.

---

## Scaling and Color Conversion (`video/framebuffer.rs`)

### Nearest-Neighbor Scaling

For performance-critical paths (real-time preview, thumbnails), nearest-neighbor
scaling maps destination pixels to source pixels using integer division:

```rust
pub fn scale_nearest(
    src: &VideoFrame,
    dst_width: usize,
    dst_height: usize,
) -> VideoFrame {
    let mut dst = VideoFrame::solid(dst_width, dst_height, 0);
    for dy in 0..dst_height {
        let sy = dy * src.height / dst_height;
        for dx in 0..dst_width {
            let sx = dx * src.width / dst_width;
            dst.set_pixel(dx, dy, src.pixel_xrgb(sx, sy));
        }
    }
    dst
}
```

### Bilinear Scaling (Integer)

For higher-quality scaling (image viewer zoom, wallpaper fit), bilinear
interpolation uses 8.8 fixed-point weights:

```rust
/// Bilinear interpolation between four pixels.
/// Weights are 8-bit fractions (0..=255).
fn bilerp(tl: u32, tr: u32, bl: u32, br: u32, fx: u8, fy: u8) -> u32 {
    // Per-channel interpolation using 8.8 fixed-point
    let ifx = 255 - fx;
    let ify = 255 - fy;

    let mix = |shift: u32| -> u32 {
        let tl_c = (tl >> shift) & 0xFF;
        let tr_c = (tr >> shift) & 0xFF;
        let bl_c = (bl >> shift) & 0xFF;
        let br_c = (br >> shift) & 0xFF;

        let top = tl_c * ifx as u32 + tr_c * fx as u32;
        let bot = bl_c * ifx as u32 + br_c * fx as u32;
        let val = (top * ify as u32 + bot * fy as u32) >> 16;
        val.min(255)
    };

    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}
```

### YUV to RGB Conversion (BT.601)

For video frame data encoded in YUV (common in media streams), conversion
to RGB uses the BT.601 standard with integer coefficients scaled by 256:

```
R = clamp((298 * (Y - 16) + 409 * (Cr - 128) + 128) >> 8)
G = clamp((298 * (Y - 16) - 100 * (Cb - 128) - 208 * (Cr - 128) + 128) >> 8)
B = clamp((298 * (Y - 16) + 516 * (Cb - 128) + 128) >> 8)
```

The reverse (RGB to YUV) is provided for potential future capture or encoding
paths but is not used by the current output pipeline.

### Alpha Blending

Pre-multiplied ARGB8888 surfaces are composited using:

```rust
/// Blend src (pre-multiplied ARGB) over dst (XRGB).
fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    let inv_a = 255 - sa;

    let blend = |shift: u32| -> u32 {
        let s = (src >> shift) & 0xFF;
        let d = (dst >> shift) & 0xFF;
        // src is pre-multiplied, so: s + d * (1 - sa)
        let val = s + ((d * inv_a + 127) / 255);
        val.min(255)
    };

    (blend(16) << 16) | (blend(8) << 8) | blend(0)
}
```

---

## Image Decoders (`video/decode.rs`)

Image decoders implement a common trait and are dispatched by file magic bytes.

### Decoder Trait

```rust
pub trait ImageDecoder {
    /// Probe: does this decoder recognize the data?
    fn probe(data: &[u8]) -> bool;

    /// Decode the image data into a VideoFrame.
    fn decode(data: &[u8]) -> Result<VideoFrame, VideoError>;
}
```

### Auto-Detection

The `decode_image()` entry point inspects the first bytes of the input:

| Format | Magic Bytes | Decoder |
|--------|-------------|---------|
| TGA | No reliable magic; fallback by extension or explicit type | `TgaDecoder` |
| QOI | `qoif` (0x716F6966) at offset 0 | `QoiDecoder` |
| PPM (P6) | `P6` at offset 0 | Existing `image_viewer.rs` |
| BMP | `BM` (0x424D) at offset 0 | Existing `image_viewer.rs` |

If no magic matches, the function returns `VideoError::UnsupportedFormat`.

### TGA Decoder

Supports uncompressed and RLE-compressed TGA images in 24-bit (RGB) and
32-bit (RGBA) color depths.

TGA header layout (18 bytes):

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | ID length |
| 1 | 1 | Color map type (0 = none) |
| 2 | 1 | Image type (2 = uncompressed true-color, 10 = RLE) |
| 3 | 5 | Color map spec (ignored) |
| 8 | 2 | X origin |
| 10 | 2 | Y origin |
| 12 | 2 | Width (little-endian) |
| 14 | 2 | Height (little-endian) |
| 16 | 1 | Bits per pixel (24 or 32) |
| 17 | 1 | Image descriptor (bit 5 = top-to-bottom) |

RLE decoding processes packets where the high bit of the count byte
distinguishes run-length (bit 7 = 1, repeat next pixel N+1 times) from
raw (bit 7 = 0, copy next N+1 pixels literally).

### QOI Decoder

Implements the Quite OK Image format, a simple lossless format with four
operation types encoded in a single-pass stream:

| Op | Tag Bits | Description |
|----|----------|-------------|
| QOI_OP_INDEX | `00xxxxxx` | Index into 64-entry running hash table |
| QOI_OP_DIFF | `01xxxxxx` | Small delta from previous pixel (-2..1 per channel) |
| QOI_OP_LUMA | `10xxxxxx` | Larger delta with luma-based encoding |
| QOI_OP_RUN | `11xxxxxx` | Repeat previous pixel 1..62 times |
| QOI_OP_RGB | `0xFE` tag | Explicit 3-byte RGB follows |
| QOI_OP_RGBA | `0xFF` tag | Explicit 4-byte RGBA follows |

The hash table uses `(r * 3 + g * 5 + b * 7 + a * 11) % 64` for indexing.
The decoder outputs ARGB8888 frames.

---

## Media Player (`video/player.rs`)

The media player provides frame-sequential video playback for raw (uncompressed)
video streams. Compressed codec support is deferred to a future phase.

### RawVideoStream

```rust
/// A sequence of video frames at a fixed frame rate.
pub struct RawVideoStream {
    /// Frames stored as sequential VideoFrame objects.
    frames: Vec<VideoFrame>,
    /// Target frame rate (frames per second).
    fps: u32,
    /// Current frame index.
    current: usize,
}

impl RawVideoStream {
    /// Advance to the next frame. Returns None at end-of-stream.
    pub fn next_frame(&mut self) -> Option<&VideoFrame> { ... }

    /// Seek to a specific frame index.
    pub fn seek(&mut self, frame: usize) { ... }

    /// Total duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        (self.frames.len() as u64 * 1000) / self.fps as u64
    }
}
```

### MediaPlayer

The `MediaPlayer` wraps a `RawVideoStream` with display integration:

```rust
pub struct MediaPlayer {
    stream: RawVideoStream,
    /// Display rectangle (x, y, width, height) in compositor coordinates.
    display_rect: (usize, usize, usize, usize),
    /// Playback state.
    state: PlaybackState,
    /// Tick counter for frame timing.
    last_frame_tick: u64,
}

pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}
```

Frame timing uses the kernel's `PlatformTimer` tick counter. Each call to
`tick()` checks whether enough time has elapsed for the next frame at the
target FPS. If so, the player advances the stream and blits the frame
(scaled to `display_rect`) to the compositor surface.

```rust
impl MediaPlayer {
    pub fn tick(&mut self, current_tick: u64) {
        if self.state != PlaybackState::Playing {
            return;
        }
        let ticks_per_frame = TIMER_HZ / self.stream.fps as u64;
        if current_tick - self.last_frame_tick >= ticks_per_frame {
            if let Some(frame) = self.stream.next_frame() {
                self.blit_to_surface(frame);
            } else {
                self.state = PlaybackState::Stopped;
            }
            self.last_frame_tick = current_tick;
        }
    }
}
```

---

## Desktop Integration

The video framework extends the existing image viewer (which already supports
PPM and BMP) with TGA and QOI decoding. Image format detection is unified:

```
image_viewer::open(path) -->
    read file bytes from VFS -->
    video::decode::decode_image(bytes) -->
    match format:
        PPM/BMP  --> existing image_viewer decoders
        TGA/QOI  --> video::decode::{TgaDecoder, QoiDecoder}
    --> VideoFrame -->
    scale to window size -->
    blit to compositor surface
```

The image viewer's zoom (25%--400%) and pan controls work identically across
all formats since they operate on the decoded `VideoFrame` / `Image` data,
not the source encoding.

---

## Module Layout

```
kernel/src/video/
    mod.rs              -- VideoFrame, PixelFormat, VideoError, init()
    framebuffer.rs      -- Scaling (nearest/bilinear), YUV<->RGB, alpha blend
    decode.rs           -- ImageDecoder trait, TGA, QOI, auto-detect dispatch
    player.rs           -- RawVideoStream, MediaPlayer, frame timing
```

---

## Error Types

```rust
#[derive(Debug)]
pub enum VideoError {
    /// Image data is too short or structurally invalid.
    InvalidFormat(&'static str),
    /// Format not recognized by any registered decoder.
    UnsupportedFormat,
    /// Pixel dimensions exceed maximum (16384 x 16384).
    DimensionsTooLarge,
    /// Insufficient memory for decoded frame buffer.
    OutOfMemory,
    /// RLE stream ended prematurely or produced excess pixels.
    DecodingError(&'static str),
}
```

---

## Configuration Constants

| Constant | Default | Description |
|----------|---------|-------------|
| `MAX_IMAGE_WIDTH` | 16384 | Maximum decoded image width in pixels |
| `MAX_IMAGE_HEIGHT` | 16384 | Maximum decoded image height in pixels |
| `DEFAULT_FPS` | 30 | Default playback frame rate |
| `QOI_HASH_SIZE` | 64 | QOI running pixel hash table entries |
| `BILINEAR_FRAC_BITS` | 8 | Fixed-point fractional bits for bilinear weights |

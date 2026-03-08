//! V4L2 (Video for Linux 2) Device Interface
//!
//! Provides a kernel-side V4L2-compatible device interface for video
//! capture devices (webcams, capture cards).  Implements the standard
//! V4L2 ioctl commands and a test pattern generator.
//!
//! All color math uses integer-only operations (no floating point).

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// V4L2 ioctl command numbers
// ---------------------------------------------------------------------------

/// V4L2 ioctl base
const VIDIOC_BASE: u32 = 0x5600;

/// Query device capabilities
const VIDIOC_QUERYCAP: u32 = VIDIOC_BASE;
/// Enumerate image formats
const VIDIOC_ENUM_FMT: u32 = VIDIOC_BASE + 0x02;
/// Get current format
const VIDIOC_G_FMT: u32 = VIDIOC_BASE + 0x04;
/// Set format
const VIDIOC_S_FMT: u32 = VIDIOC_BASE + 0x05;
/// Request buffers
const VIDIOC_REQBUFS: u32 = VIDIOC_BASE + 0x08;
/// Query buffer status
const VIDIOC_QUERYBUF: u32 = VIDIOC_BASE + 0x09;
/// Queue buffer for capture
const VIDIOC_QBUF: u32 = VIDIOC_BASE + 0x0F;
/// Dequeue filled buffer
const VIDIOC_DQBUF: u32 = VIDIOC_BASE + 0x11;
/// Start streaming
const VIDIOC_STREAMON: u32 = VIDIOC_BASE + 0x12;
/// Stop streaming
const VIDIOC_STREAMOFF: u32 = VIDIOC_BASE + 0x13;

// ---------------------------------------------------------------------------
// V4L2 pixel format FourCC codes
// ---------------------------------------------------------------------------

/// YUYV 4:2:2 packed (commonly used by USB webcams)
const V4L2_PIX_FMT_YUYV: u32 = fourcc(b'Y', b'U', b'Y', b'V');
/// RGB24 packed
const V4L2_PIX_FMT_RGB24: u32 = fourcc(b'R', b'G', b'B', b'3');
/// BGR24 packed
const V4L2_PIX_FMT_BGR24: u32 = fourcc(b'B', b'G', b'R', b'3');
/// MJPEG compressed
const V4L2_PIX_FMT_MJPEG: u32 = fourcc(b'M', b'J', b'P', b'G');

/// Create a FourCC value from 4 bytes
const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

// ---------------------------------------------------------------------------
// V4L2 capability flags
// ---------------------------------------------------------------------------

/// Device supports video capture
const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
/// Device supports streaming I/O
const V4L2_CAP_STREAMING: u32 = 0x0400_0000;
/// Device supports read/write I/O
const V4L2_CAP_READWRITE: u32 = 0x0100_0000;

// ---------------------------------------------------------------------------
// V4L2 buffer flags
// ---------------------------------------------------------------------------

/// Buffer is mapped into user space
const V4L2_BUF_FLAG_MAPPED: u32 = 0x0001;
/// Buffer is queued for input
const V4L2_BUF_FLAG_QUEUED: u32 = 0x0002;
/// Buffer has been filled with data
const V4L2_BUF_FLAG_DONE: u32 = 0x0004;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of buffers in the pool
const MAX_BUFFERS: usize = 4;
/// Default capture width
const DEFAULT_WIDTH: u32 = 640;
/// Default capture height
const DEFAULT_HEIGHT: u32 = 480;
/// YUYV bytes per line: width * 2 (each pixel is 2 bytes in YUYV 4:2:2)
const YUYV_BYTES_PER_LINE: u32 = DEFAULT_WIDTH * 2;
/// YUYV frame size in bytes
const YUYV_FRAME_SIZE: u32 = YUYV_BYTES_PER_LINE * DEFAULT_HEIGHT;
/// Maximum driver name length
const MAX_DRIVER_NAME: usize = 16;
/// Maximum card name length
const MAX_CARD_NAME: usize = 32;
/// Maximum bus info length
const MAX_BUS_INFO: usize = 32;

/// Number of color bars in the test pattern
const NUM_COLOR_BARS: usize = 8;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// V4L2 device capabilities
#[derive(Debug, Clone)]
pub struct V4l2Capability {
    /// Driver name (e.g., "veridian-v4l2")
    pub driver: [u8; MAX_DRIVER_NAME],
    /// Card/device name
    pub card: [u8; MAX_CARD_NAME],
    /// Bus location info
    pub bus_info: [u8; MAX_BUS_INFO],
    /// Kernel version
    pub version: u32,
    /// Capability flags
    pub capabilities: u32,
    /// Device capabilities (for multi-function devices)
    pub device_caps: u32,
}

impl Default for V4l2Capability {
    fn default() -> Self {
        let mut cap = Self {
            driver: [0u8; MAX_DRIVER_NAME],
            card: [0u8; MAX_CARD_NAME],
            bus_info: [0u8; MAX_BUS_INFO],
            version: 0x0016_0000, // v0.22.0
            capabilities: V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_STREAMING | V4L2_CAP_READWRITE,
            device_caps: V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_STREAMING,
        };
        copy_str_to_buf(&mut cap.driver, b"veridian-v4l2");
        copy_str_to_buf(&mut cap.card, b"VeridianOS Virtual Camera");
        copy_str_to_buf(&mut cap.bus_info, b"platform:veridian-v4l2");
        cap
    }
}

/// V4L2 pixel format descriptor
#[derive(Debug, Clone, Copy)]
pub struct V4l2PixFormat {
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// FourCC pixel format code
    pub pixelformat: u32,
    /// Bytes per line
    pub bytesperline: u32,
    /// Total image size in bytes
    pub sizeimage: u32,
}

impl Default for V4l2PixFormat {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            pixelformat: V4L2_PIX_FMT_YUYV,
            bytesperline: YUYV_BYTES_PER_LINE,
            sizeimage: YUYV_FRAME_SIZE,
        }
    }
}

/// V4L2 format description (for enumeration)
#[derive(Debug, Clone, Copy)]
pub struct V4l2FmtDesc {
    /// Format index
    pub index: u32,
    /// FourCC pixel format
    pub pixelformat: u32,
    /// Description string
    pub description: [u8; 32],
    /// Flags
    pub flags: u32,
}

/// V4L2 buffer state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V4l2BufState {
    /// Buffer is available (dequeued)
    Idle,
    /// Buffer is queued for capture
    Queued,
    /// Buffer has been filled with data
    Done,
}

/// V4L2 buffer descriptor
#[derive(Debug, Clone, Copy)]
pub struct V4l2Buffer {
    /// Buffer index
    pub index: u32,
    /// Buffer state
    pub state: V4l2BufState,
    /// Buffer flags
    pub flags: u32,
    /// Data offset in shared buffer pool
    pub offset: u32,
    /// Bytes used in this buffer
    pub bytesused: u32,
    /// Frame sequence number
    pub sequence: u32,
}

impl Default for V4l2Buffer {
    fn default() -> Self {
        Self {
            index: 0,
            state: V4l2BufState::Idle,
            flags: 0,
            offset: 0,
            bytesused: 0,
            sequence: 0,
        }
    }
}

/// V4L2 device instance
pub struct V4l2Device {
    /// Device capabilities
    capability: V4l2Capability,
    /// Current pixel format
    format: V4l2PixFormat,
    /// Buffer descriptors
    buffers: [V4l2Buffer; MAX_BUFFERS],
    /// Number of allocated buffers
    num_buffers: u32,
    /// Whether the device is streaming
    streaming: bool,
    /// Frame counter (monotonically increasing)
    frame_counter: u32,
    /// Queue head index (next buffer to fill)
    queue_head: usize,
    /// Queue tail index (next buffer to dequeue)
    queue_tail: usize,
    /// Number of buffers currently queued
    queued_count: usize,
}

impl Default for V4l2Device {
    fn default() -> Self {
        Self::new()
    }
}

impl V4l2Device {
    /// Create a new V4L2 device
    pub const fn new() -> Self {
        const DEFAULT_BUF: V4l2Buffer = V4l2Buffer {
            index: 0,
            state: V4l2BufState::Idle,
            flags: 0,
            offset: 0,
            bytesused: 0,
            sequence: 0,
        };
        Self {
            capability: V4l2Capability {
                driver: [0u8; MAX_DRIVER_NAME],
                card: [0u8; MAX_CARD_NAME],
                bus_info: [0u8; MAX_BUS_INFO],
                version: 0x0016_0000,
                capabilities: V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_STREAMING | V4L2_CAP_READWRITE,
                device_caps: V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_STREAMING,
            },
            format: V4l2PixFormat {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
                pixelformat: V4L2_PIX_FMT_YUYV,
                bytesperline: YUYV_BYTES_PER_LINE,
                sizeimage: YUYV_FRAME_SIZE,
            },
            buffers: [DEFAULT_BUF; MAX_BUFFERS],
            num_buffers: 0,
            streaming: false,
            frame_counter: 0,
            queue_head: 0,
            queue_tail: 0,
            queued_count: 0,
        }
    }

    /// Initialize the device
    pub fn init(&mut self) {
        self.capability = V4l2Capability::default();
        self.format = V4l2PixFormat::default();
        self.frame_counter = 0;
        self.streaming = false;
        self.num_buffers = 0;
        self.queue_head = 0;
        self.queue_tail = 0;
        self.queued_count = 0;
    }

    /// Handle VIDIOC_QUERYCAP
    pub fn query_cap(&self) -> V4l2Capability {
        self.capability.clone()
    }

    /// Handle VIDIOC_ENUM_FMT
    pub fn enum_fmt(&self, index: u32) -> Option<V4l2FmtDesc> {
        match index {
            0 => {
                let mut desc = V4l2FmtDesc {
                    index: 0,
                    pixelformat: V4L2_PIX_FMT_YUYV,
                    description: [0u8; 32],
                    flags: 0,
                };
                copy_str_to_buf(&mut desc.description, b"YUYV 4:2:2");
                Some(desc)
            }
            1 => {
                let mut desc = V4l2FmtDesc {
                    index: 1,
                    pixelformat: V4L2_PIX_FMT_RGB24,
                    description: [0u8; 32],
                    flags: 0,
                };
                copy_str_to_buf(&mut desc.description, b"RGB24");
                Some(desc)
            }
            _ => None,
        }
    }

    /// Handle VIDIOC_G_FMT
    pub fn get_format(&self) -> V4l2PixFormat {
        self.format
    }

    /// Handle VIDIOC_S_FMT
    pub fn set_format(&mut self, fmt: V4l2PixFormat) -> Result<V4l2PixFormat, KernelError> {
        if self.streaming {
            return Err(KernelError::InvalidState {
                expected: "idle",
                actual: "busy",
            });
        }

        // Validate and clamp dimensions
        let width = clamp(fmt.width, 160, 1920);
        let height = clamp(fmt.height, 120, 1080);

        // Only YUYV is fully supported for now
        let pixelformat = if fmt.pixelformat == V4L2_PIX_FMT_RGB24 {
            V4L2_PIX_FMT_RGB24
        } else {
            V4L2_PIX_FMT_YUYV
        };

        let bytesperline = if pixelformat == V4L2_PIX_FMT_YUYV {
            width.checked_mul(2).unwrap_or(width)
        } else {
            width.checked_mul(3).unwrap_or(width)
        };

        let sizeimage = bytesperline.checked_mul(height).unwrap_or(bytesperline);

        self.format = V4l2PixFormat {
            width,
            height,
            pixelformat,
            bytesperline,
            sizeimage,
        };

        Ok(self.format)
    }

    /// Handle VIDIOC_REQBUFS
    pub fn request_buffers(&mut self, count: u32) -> Result<u32, KernelError> {
        if self.streaming {
            return Err(KernelError::InvalidState {
                expected: "idle",
                actual: "busy",
            });
        }

        let actual = if count > MAX_BUFFERS as u32 {
            MAX_BUFFERS as u32
        } else {
            count
        };

        self.num_buffers = actual;
        for i in 0..actual as usize {
            self.buffers[i] = V4l2Buffer {
                index: i as u32,
                state: V4l2BufState::Idle,
                flags: 0,
                offset: (i as u32).checked_mul(self.format.sizeimage).unwrap_or(0),
                bytesused: 0,
                sequence: 0,
            };
        }

        self.queue_head = 0;
        self.queue_tail = 0;
        self.queued_count = 0;

        Ok(actual)
    }

    /// Handle VIDIOC_QUERYBUF
    pub fn query_buffer(&self, index: u32) -> Option<V4l2Buffer> {
        if index < self.num_buffers {
            Some(self.buffers[index as usize])
        } else {
            None
        }
    }

    /// Handle VIDIOC_QBUF -- queue a buffer for capture
    pub fn queue_buffer(&mut self, index: u32) -> Result<(), KernelError> {
        if index >= self.num_buffers {
            return Err(KernelError::InvalidArgument {
                name: "v4l2",
                value: "invalid",
            });
        }

        let buf = &mut self.buffers[index as usize];
        if buf.state != V4l2BufState::Idle {
            return Err(KernelError::InvalidState {
                expected: "idle",
                actual: "busy",
            });
        }

        buf.state = V4l2BufState::Queued;
        buf.flags = V4L2_BUF_FLAG_QUEUED;
        self.queued_count += 1;
        Ok(())
    }

    /// Handle VIDIOC_DQBUF -- dequeue a filled buffer
    pub fn dequeue_buffer(&mut self) -> Result<V4l2Buffer, KernelError> {
        if !self.streaming {
            return Err(KernelError::InvalidState {
                expected: "idle",
                actual: "busy",
            });
        }

        // Find the next buffer marked as Done
        for i in 0..self.num_buffers as usize {
            if self.buffers[i].state == V4l2BufState::Done {
                self.buffers[i].state = V4l2BufState::Idle;
                self.buffers[i].flags = 0;
                let result = self.buffers[i];
                return Ok(result);
            }
        }

        // No buffers ready; in a real driver we'd block or return EAGAIN
        Err(KernelError::WouldBlock)
    }

    /// Handle VIDIOC_STREAMON
    pub fn stream_on(&mut self) -> Result<(), KernelError> {
        if self.streaming {
            return Err(KernelError::InvalidState {
                expected: "idle",
                actual: "busy",
            });
        }
        if self.num_buffers == 0 {
            return Err(KernelError::InvalidArgument {
                name: "v4l2",
                value: "invalid",
            });
        }
        self.streaming = true;
        self.frame_counter = 0;
        Ok(())
    }

    /// Handle VIDIOC_STREAMOFF
    pub fn stream_off(&mut self) -> Result<(), KernelError> {
        self.streaming = false;

        // Return all buffers to idle
        for i in 0..self.num_buffers as usize {
            self.buffers[i].state = V4l2BufState::Idle;
            self.buffers[i].flags = 0;
        }
        self.queued_count = 0;
        Ok(())
    }

    /// Generate a test frame into a queued buffer
    ///
    /// Produces SMPTE-style color bars in YUYV 4:2:2 format.
    /// Called by the capture loop to fill queued buffers.
    pub fn generate_test_frame(&mut self, output: &mut [u8]) -> Result<usize, KernelError> {
        if !self.streaming {
            return Err(KernelError::InvalidState {
                expected: "idle",
                actual: "busy",
            });
        }

        // Find a queued buffer
        let mut buf_idx = None;
        for i in 0..self.num_buffers as usize {
            if self.buffers[i].state == V4l2BufState::Queued {
                buf_idx = Some(i);
                break;
            }
        }

        let idx = buf_idx.ok_or(KernelError::WouldBlock)?;

        let width = self.format.width as usize;
        let height = self.format.height as usize;
        let frame_size = width * height * 2; // YUYV: 2 bytes per pixel

        if output.len() < frame_size {
            return Err(KernelError::ResourceExhausted {
                resource: "v4l2 buffer",
            });
        }

        // Generate SMPTE color bars in YUYV format
        // 8 bars: white, yellow, cyan, green, magenta, red, blue, black
        generate_color_bars_yuyv(output, width, height, self.frame_counter);

        // Mark buffer as done
        self.buffers[idx].state = V4l2BufState::Done;
        self.buffers[idx].flags = V4L2_BUF_FLAG_DONE;
        self.buffers[idx].bytesused = frame_size as u32;
        self.buffers[idx].sequence = self.frame_counter;
        self.frame_counter = self.frame_counter.wrapping_add(1);

        if self.queued_count > 0 {
            self.queued_count -= 1;
        }

        Ok(frame_size)
    }

    /// Check if the device is currently streaming
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Get the current frame counter
    pub fn frame_counter(&self) -> u32 {
        self.frame_counter
    }

    /// Get the number of allocated buffers
    pub fn num_buffers(&self) -> u32 {
        self.num_buffers
    }
}

// ---------------------------------------------------------------------------
// Test Pattern Generator
// ---------------------------------------------------------------------------

/// SMPTE color bar RGB values
/// Order: white, yellow, cyan, green, magenta, red, blue, black
const COLOR_BAR_RGB: [(u8, u8, u8); NUM_COLOR_BARS] = [
    (255, 255, 255), // White
    (255, 255, 0),   // Yellow
    (0, 255, 255),   // Cyan
    (0, 255, 0),     // Green
    (255, 0, 255),   // Magenta
    (255, 0, 0),     // Red
    (0, 0, 255),     // Blue
    (0, 0, 0),       // Black
];

/// Convert RGB to Y (luminance) using integer math
///
/// Formula: Y = (66*R + 129*G + 25*B + 128) >> 8 + 16
/// This gives values in [16, 235] per ITU-R BT.601.
fn rgb_to_y(r: u8, g: u8, b: u8) -> u8 {
    let r32 = r as u32;
    let g32 = g as u32;
    let b32 = b as u32;
    let y = ((66u32
        .checked_mul(r32)
        .unwrap_or(0)
        .checked_add(129u32.checked_mul(g32).unwrap_or(0))
        .unwrap_or(0)
        .checked_add(25u32.checked_mul(b32).unwrap_or(0))
        .unwrap_or(0)
        .checked_add(128)
        .unwrap_or(0))
        >> 8)
        .checked_add(16)
        .unwrap_or(16);
    if y > 255 {
        255u8
    } else {
        y as u8
    }
}

/// Convert RGB to U (Cb) chrominance using integer math
///
/// Formula: U = (-38*R - 74*G + 112*B + 128) >> 8 + 128
fn rgb_to_u(r: u8, g: u8, b: u8) -> u8 {
    let r32 = r as i32;
    let g32 = g as i32;
    let b32 = b as i32;
    let u = ((-38i32 * r32 - 74i32 * g32 + 112i32 * b32 + 128i32) >> 8) + 128i32;
    if u < 0 {
        0u8
    } else if u > 255 {
        255u8
    } else {
        u as u8
    }
}

/// Convert RGB to V (Cr) chrominance using integer math
///
/// Formula: V = (112*R - 94*G - 18*B + 128) >> 8 + 128
fn rgb_to_v(r: u8, g: u8, b: u8) -> u8 {
    let r32 = r as i32;
    let g32 = g as i32;
    let b32 = b as i32;
    let v = ((112i32 * r32 - 94i32 * g32 - 18i32 * b32 + 128i32) >> 8) + 128i32;
    if v < 0 {
        0u8
    } else if v > 255 {
        255u8
    } else {
        v as u8
    }
}

/// Generate SMPTE color bars in YUYV 4:2:2 format
///
/// YUYV packing: [Y0, U01, Y1, V01] for each pair of horizontal pixels.
fn generate_color_bars_yuyv(output: &mut [u8], width: usize, height: usize, frame_num: u32) {
    let bar_width = width / NUM_COLOR_BARS;

    for y in 0..height {
        let row_offset = y * width * 2;

        // Process pixels in pairs (YUYV = 2 pixels per 4 bytes)
        let mut x = 0usize;
        while x < width {
            let bar0 = if bar_width > 0 { x / bar_width } else { 0 };
            let bar1 = if bar_width > 0 {
                (x + 1) / bar_width
            } else {
                0
            };
            let bar0 = if bar0 >= NUM_COLOR_BARS {
                NUM_COLOR_BARS - 1
            } else {
                bar0
            };
            let bar1 = if bar1 >= NUM_COLOR_BARS {
                NUM_COLOR_BARS - 1
            } else {
                bar1
            };

            let (r0, g0, b0) = COLOR_BAR_RGB[bar0];
            let (r1, g1, b1) = COLOR_BAR_RGB[bar1];

            let y0 = rgb_to_y(r0, g0, b0);
            let y1 = rgb_to_y(r1, g1, b1);
            // Average the U/V of the two pixels in the pair
            let u = rgb_to_u(
                ((r0 as u16 + r1 as u16) / 2) as u8,
                ((g0 as u16 + g1 as u16) / 2) as u8,
                ((b0 as u16 + b1 as u16) / 2) as u8,
            );
            let v = rgb_to_v(
                ((r0 as u16 + r1 as u16) / 2) as u8,
                ((g0 as u16 + g1 as u16) / 2) as u8,
                ((b0 as u16 + b1 as u16) / 2) as u8,
            );

            let offset = row_offset + x * 2;
            if offset + 3 < output.len() {
                output[offset] = y0;
                output[offset + 1] = u;
                output[offset + 2] = y1;
                output[offset + 3] = v;
            }

            x += 2;
        }
    }

    // Overlay frame counter in top-left corner (8x8 block, simple)
    // Toggle between bright and dark based on frame number for visibility
    let marker_val = if frame_num & 1 == 0 { 235u8 } else { 16u8 };
    for py in 0..8usize {
        if py >= height {
            break;
        }
        for px in 0..8usize {
            if px >= width {
                break;
            }
            let offset = py * width * 2 + px * 2;
            if offset < output.len() {
                output[offset] = marker_val;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// V4L2 ioctl dispatcher
// ---------------------------------------------------------------------------

/// Process a V4L2 ioctl command
pub fn v4l2_ioctl(device: &mut V4l2Device, cmd: u32, arg: u64) -> Result<u64, KernelError> {
    match cmd {
        VIDIOC_QUERYCAP => {
            let _cap = device.query_cap();
            Ok(0)
        }
        VIDIOC_ENUM_FMT => {
            let index = arg as u32;
            match device.enum_fmt(index) {
                Some(_desc) => Ok(0),
                None => Err(KernelError::InvalidArgument {
                    name: "v4l2",
                    value: "invalid",
                }),
            }
        }
        VIDIOC_G_FMT => {
            let _fmt = device.get_format();
            Ok(0)
        }
        VIDIOC_S_FMT => {
            // In a real implementation, arg would point to a user-space format struct
            let fmt = V4l2PixFormat {
                width: (arg & 0xFFFF) as u32,
                height: ((arg >> 16) & 0xFFFF) as u32,
                pixelformat: V4L2_PIX_FMT_YUYV,
                bytesperline: 0, // will be computed
                sizeimage: 0,    // will be computed
            };
            device.set_format(fmt)?;
            Ok(0)
        }
        VIDIOC_REQBUFS => {
            let count = arg as u32;
            let actual = device.request_buffers(count)?;
            Ok(actual as u64)
        }
        VIDIOC_QUERYBUF => {
            let index = arg as u32;
            match device.query_buffer(index) {
                Some(_buf) => Ok(0),
                None => Err(KernelError::InvalidArgument {
                    name: "v4l2",
                    value: "invalid",
                }),
            }
        }
        VIDIOC_QBUF => {
            let index = arg as u32;
            device.queue_buffer(index)?;
            Ok(0)
        }
        VIDIOC_DQBUF => {
            let buf = device.dequeue_buffer()?;
            Ok(buf.index as u64)
        }
        VIDIOC_STREAMON => {
            device.stream_on()?;
            Ok(0)
        }
        VIDIOC_STREAMOFF => {
            device.stream_off()?;
            Ok(0)
        }
        _ => Err(KernelError::InvalidArgument {
            name: "v4l2",
            value: "invalid",
        }),
    }
}

// ---------------------------------------------------------------------------
// Global V4L2 device
// ---------------------------------------------------------------------------

static V4L2_DEVICE: Mutex<V4l2Device> = Mutex::new(V4l2Device::new());

static V4L2_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the V4L2 subsystem
pub fn v4l2_init() {
    let mut dev = V4L2_DEVICE.lock();
    dev.init();
    V4L2_INITIALIZED.store(true, Ordering::Release);
    crate::println!("[V4L2] Virtual camera device initialized (640x480 YUYV)");
}

/// Process a V4L2 ioctl on the global device
pub fn v4l2_global_ioctl(cmd: u32, arg: u64) -> Result<u64, KernelError> {
    if !V4L2_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized { subsystem: "v4l2" });
    }
    let mut dev = V4L2_DEVICE.lock();
    v4l2_ioctl(&mut dev, cmd, arg)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy a byte string into a fixed-size buffer, null-terminating
fn copy_str_to_buf(buf: &mut [u8], src: &[u8]) {
    let len = if src.len() < buf.len() - 1 {
        src.len()
    } else {
        buf.len() - 1
    };
    buf[..len].copy_from_slice(&src[..len]);
    buf[len] = 0;
}

/// Clamp a value to a range
fn clamp(val: u32, min: u32, max: u32) -> u32 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fourcc() {
        let yuyv = fourcc(b'Y', b'U', b'Y', b'V');
        assert_ne!(yuyv, 0);
        // Check byte order
        assert_eq!(yuyv & 0xFF, b'Y' as u32);
        assert_eq!((yuyv >> 8) & 0xFF, b'U' as u32);
    }

    #[test]
    fn test_rgb_to_y() {
        // White should give high Y
        let y_white = rgb_to_y(255, 255, 255);
        assert!(y_white > 200);

        // Black should give low Y
        let y_black = rgb_to_y(0, 0, 0);
        assert!(y_black < 30);

        // Y should be in BT.601 range
        assert!(y_white <= 255);
        assert!(y_black >= 16);
    }

    #[test]
    fn test_rgb_to_u() {
        // For neutral gray, U should be ~128
        let u_gray = rgb_to_u(128, 128, 128);
        assert!((u_gray as i32 - 128).unsigned_abs() < 5);
    }

    #[test]
    fn test_rgb_to_v() {
        // For neutral gray, V should be ~128
        let v_gray = rgb_to_v(128, 128, 128);
        assert!((v_gray as i32 - 128).unsigned_abs() < 5);
    }

    #[test]
    fn test_v4l2_capability_default() {
        let cap = V4l2Capability::default();
        assert!(cap.capabilities & V4L2_CAP_VIDEO_CAPTURE != 0);
        assert!(cap.capabilities & V4L2_CAP_STREAMING != 0);
        assert_eq!(cap.driver[0], b'v');
    }

    #[test]
    fn test_v4l2_device_new() {
        let dev = V4l2Device::new();
        assert!(!dev.is_streaming());
        assert_eq!(dev.frame_counter(), 0);
        assert_eq!(dev.num_buffers(), 0);
    }

    #[test]
    fn test_v4l2_device_init() {
        let mut dev = V4l2Device::new();
        dev.init();
        let fmt = dev.get_format();
        assert_eq!(fmt.width, DEFAULT_WIDTH);
        assert_eq!(fmt.height, DEFAULT_HEIGHT);
        assert_eq!(fmt.pixelformat, V4L2_PIX_FMT_YUYV);
    }

    #[test]
    fn test_v4l2_enum_fmt() {
        let mut dev = V4l2Device::new();
        dev.init();

        assert!(dev.enum_fmt(0).is_some());
        assert!(dev.enum_fmt(1).is_some());
        assert!(dev.enum_fmt(2).is_none());
    }

    #[test]
    fn test_v4l2_set_format() {
        let mut dev = V4l2Device::new();
        dev.init();

        let fmt = V4l2PixFormat {
            width: 320,
            height: 240,
            pixelformat: V4L2_PIX_FMT_YUYV,
            bytesperline: 0,
            sizeimage: 0,
        };
        let result = dev.set_format(fmt);
        assert!(result.is_ok());

        let actual = result.unwrap();
        assert_eq!(actual.width, 320);
        assert_eq!(actual.height, 240);
        assert_eq!(actual.bytesperline, 640); // 320 * 2
    }

    #[test]
    fn test_v4l2_set_format_clamping() {
        let mut dev = V4l2Device::new();
        dev.init();

        let fmt = V4l2PixFormat {
            width: 10000,
            height: 10000,
            pixelformat: V4L2_PIX_FMT_YUYV,
            bytesperline: 0,
            sizeimage: 0,
        };
        let result = dev.set_format(fmt).unwrap();
        assert_eq!(result.width, 1920);
        assert_eq!(result.height, 1080);
    }

    #[test]
    fn test_v4l2_request_buffers() {
        let mut dev = V4l2Device::new();
        dev.init();

        let count = dev.request_buffers(4).unwrap();
        assert_eq!(count, 4);
        assert_eq!(dev.num_buffers(), 4);
    }

    #[test]
    fn test_v4l2_request_buffers_capped() {
        let mut dev = V4l2Device::new();
        dev.init();

        let count = dev.request_buffers(100).unwrap();
        assert_eq!(count, MAX_BUFFERS as u32);
    }

    #[test]
    fn test_v4l2_query_buffer() {
        let mut dev = V4l2Device::new();
        dev.init();
        dev.request_buffers(4).unwrap();

        let buf = dev.query_buffer(0);
        assert!(buf.is_some());
        let buf = buf.unwrap();
        assert_eq!(buf.index, 0);
        assert_eq!(buf.state, V4l2BufState::Idle);

        assert!(dev.query_buffer(4).is_none());
    }

    #[test]
    fn test_v4l2_queue_dequeue() {
        let mut dev = V4l2Device::new();
        dev.init();
        dev.request_buffers(2).unwrap();

        // Queue buffer 0
        assert!(dev.queue_buffer(0).is_ok());

        // Can't dequeue while not streaming
        assert!(dev.dequeue_buffer().is_err());

        // Start streaming
        assert!(dev.stream_on().is_ok());

        // Generate frame into queued buffer
        let mut frame = [0u8; (DEFAULT_WIDTH * DEFAULT_HEIGHT * 2) as usize];
        assert!(dev.generate_test_frame(&mut frame).is_ok());

        // Now dequeue
        let buf = dev.dequeue_buffer();
        assert!(buf.is_ok());
        assert_eq!(buf.unwrap().sequence, 0);
    }

    #[test]
    fn test_v4l2_stream_on_off() {
        let mut dev = V4l2Device::new();
        dev.init();
        dev.request_buffers(2).unwrap();

        assert!(dev.stream_on().is_ok());
        assert!(dev.is_streaming());

        // Can't stream on twice
        assert!(dev.stream_on().is_err());

        assert!(dev.stream_off().is_ok());
        assert!(!dev.is_streaming());
    }

    #[test]
    fn test_v4l2_no_buffers_stream_on() {
        let mut dev = V4l2Device::new();
        dev.init();

        // Can't stream without buffers
        assert!(dev.stream_on().is_err());
    }

    #[test]
    fn test_v4l2_ioctl_dispatch() {
        let mut dev = V4l2Device::new();
        dev.init();

        assert!(v4l2_ioctl(&mut dev, VIDIOC_QUERYCAP, 0).is_ok());
        assert!(v4l2_ioctl(&mut dev, VIDIOC_ENUM_FMT, 0).is_ok());
        assert!(v4l2_ioctl(&mut dev, VIDIOC_ENUM_FMT, 99).is_err());
        assert!(v4l2_ioctl(&mut dev, VIDIOC_G_FMT, 0).is_ok());

        // Request buffers
        let result = v4l2_ioctl(&mut dev, VIDIOC_REQBUFS, 4);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 4);

        // Unknown ioctl
        assert!(v4l2_ioctl(&mut dev, 0xFFFF, 0).is_err());
    }

    #[test]
    fn test_generate_color_bars() {
        let width = 64usize;
        let height = 4usize;
        let mut buf = [0u8; 64 * 4 * 2];
        generate_color_bars_yuyv(&mut buf, width, height, 0);

        // First pixel should be from white bar (high Y)
        assert!(buf[0] > 200); // Y of white
    }

    #[test]
    fn test_copy_str_to_buf() {
        let mut buf = [0xFFu8; 16];
        copy_str_to_buf(&mut buf, b"hello");
        assert_eq!(&buf[..5], b"hello");
        assert_eq!(buf[5], 0);
    }

    #[test]
    fn test_clamp() {
        assert_eq!(clamp(50, 0, 100), 50);
        assert_eq!(clamp(0, 10, 100), 10);
        assert_eq!(clamp(200, 10, 100), 100);
    }

    #[test]
    fn test_ioctl_constants() {
        assert_eq!(VIDIOC_QUERYCAP, 0x5600);
        assert_eq!(VIDIOC_ENUM_FMT, 0x5602);
        assert_eq!(VIDIOC_STREAMON, 0x5612);
        assert_eq!(VIDIOC_STREAMOFF, 0x5613);
    }
}

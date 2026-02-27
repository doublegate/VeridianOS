//! Hardware cursor sprite rendering.
//!
//! Provides a 16x16 arrow cursor bitmap and a compositing function
//! to overlay the cursor onto the framebuffer back-buffer.

/// Cursor width in pixels.
pub const CURSOR_WIDTH: usize = 16;
/// Cursor height in pixels.
pub const CURSOR_HEIGHT: usize = 16;

/// 16x16 arrow cursor bitmap.
///
/// Each row is a u16 where bit 15 = leftmost pixel.
/// Value 1 = white foreground, 0 = transparent.
/// A separate mask defines the black outline.
const CURSOR_DATA: [u16; CURSOR_HEIGHT] = [
    0b1000_0000_0000_0000, // *
    0b1100_0000_0000_0000, // **
    0b1110_0000_0000_0000, // ***
    0b1111_0000_0000_0000, // ****
    0b1111_1000_0000_0000, // *****
    0b1111_1100_0000_0000, // ******
    0b1111_1110_0000_0000, // *******
    0b1111_1111_0000_0000, // ********
    0b1111_1111_1000_0000, // *********
    0b1111_1100_0000_0000, // ******
    0b1111_0110_0000_0000, // ** **
    0b1110_0011_0000_0000, // * **
    0b1100_0011_0000_0000, //    **
    0b0000_0001_1000_0000, //     **
    0b0000_0001_1000_0000, //     **
    0b0000_0000_0000_0000, //
];

/// Cursor outline mask (black border for visibility on any background).
const CURSOR_MASK: [u16; CURSOR_HEIGHT] = [
    0b1100_0000_0000_0000,
    0b1110_0000_0000_0000,
    0b1111_0000_0000_0000,
    0b1111_1000_0000_0000,
    0b1111_1100_0000_0000,
    0b1111_1110_0000_0000,
    0b1111_1111_0000_0000,
    0b1111_1111_1000_0000,
    0b1111_1111_1100_0000,
    0b1111_1111_1100_0000,
    0b1111_1111_0000_0000,
    0b1111_0111_1000_0000,
    0b1110_0111_1000_0000,
    0b0000_0011_1100_0000,
    0b0000_0011_1100_0000,
    0b0000_0001_1100_0000,
];

/// Draw the cursor sprite onto a pixel buffer.
///
/// # Arguments
/// - `buf`: Pixel buffer (BGRA format, 4 bytes/pixel).
/// - `stride`: Buffer stride in bytes.
/// - `buf_width`: Buffer width in pixels.
/// - `buf_height`: Buffer height in pixels.
/// - `cx`, `cy`: Cursor position (top-left of sprite).
pub fn draw_cursor(
    buf: &mut [u8],
    stride: usize,
    buf_width: usize,
    buf_height: usize,
    cx: i32,
    cy: i32,
) {
    for row in 0..CURSOR_HEIGHT {
        let py = cy as usize + row;
        if py >= buf_height {
            break;
        }
        let mask_bits = CURSOR_MASK[row];
        let data_bits = CURSOR_DATA[row];

        for col in 0..CURSOR_WIDTH {
            let px = cx as usize + col;
            if px >= buf_width {
                break;
            }

            let bit = 15 - col;
            let in_mask = (mask_bits >> bit) & 1 != 0;
            let in_data = (data_bits >> bit) & 1 != 0;

            if in_mask {
                let offset = py * stride + px * 4;
                if offset + 3 < buf.len() {
                    if in_data {
                        // White foreground
                        buf[offset] = 0xFF;
                        buf[offset + 1] = 0xFF;
                        buf[offset + 2] = 0xFF;
                        buf[offset + 3] = 0xFF;
                    } else {
                        // Black outline
                        buf[offset] = 0x00;
                        buf[offset + 1] = 0x00;
                        buf[offset + 2] = 0x00;
                        buf[offset + 3] = 0xFF;
                    }
                }
            }
        }
    }
}

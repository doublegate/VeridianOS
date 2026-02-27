//! PS/2 mouse driver.
//!
//! Reads 3-byte mouse packets from the PS/2 auxiliary port (x86_64 only).
//! Maintains absolute cursor position clamped to screen bounds.
//! On non-x86_64 architectures, all functions are no-op stubs.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, Ordering};

/// Mouse button state flags.
pub const BUTTON_LEFT: u8 = 0x01;
pub const BUTTON_RIGHT: u8 = 0x02;
pub const BUTTON_MIDDLE: u8 = 0x04;

/// Mouse event produced by the driver.
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub dx: i16,
    pub dy: i16,
    pub buttons: u8,
}

// Screen bounds for clamping absolute position
static SCREEN_WIDTH: AtomicU16 = AtomicU16::new(1280);
static SCREEN_HEIGHT: AtomicU16 = AtomicU16::new(800);

// Absolute cursor position
static CURSOR_X: AtomicI32 = AtomicI32::new(640);
static CURSOR_Y: AtomicI32 = AtomicI32::new(400);

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Get the current absolute cursor position.
pub fn cursor_position() -> (i32, i32) {
    (
        CURSOR_X.load(Ordering::Relaxed),
        CURSOR_Y.load(Ordering::Relaxed),
    )
}

/// Set screen bounds for cursor clamping.
pub fn set_screen_bounds(width: u16, height: u16) {
    SCREEN_WIDTH.store(width, Ordering::Relaxed);
    SCREEN_HEIGHT.store(height, Ordering::Relaxed);
}

/// Check if the mouse driver is initialized.
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// x86_64 implementation
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
mod x86_64_impl {
    use core::sync::atomic::AtomicUsize;

    use super::*;

    /// Ring buffer for mouse events.
    const EVENT_BUFFER_SIZE: usize = 64;

    struct MouseBuffer {
        buf: [MouseEvent; EVENT_BUFFER_SIZE],
        head: AtomicUsize,
        tail: AtomicUsize,
    }

    impl MouseBuffer {
        const fn new() -> Self {
            Self {
                buf: [MouseEvent {
                    dx: 0,
                    dy: 0,
                    buttons: 0,
                }; EVENT_BUFFER_SIZE],
                head: AtomicUsize::new(0),
                tail: AtomicUsize::new(0),
            }
        }

        fn push(&mut self, event: MouseEvent) {
            let head = self.head.load(Ordering::Relaxed);
            let next = (head + 1) & (EVENT_BUFFER_SIZE - 1);
            let tail = self.tail.load(Ordering::Acquire);
            if next == tail {
                return; // Buffer full
            }
            self.buf[head] = event;
            self.head.store(next, Ordering::Release);
        }

        fn pop(&self) -> Option<MouseEvent> {
            let tail = self.tail.load(Ordering::Relaxed);
            let head = self.head.load(Ordering::Acquire);
            if tail == head {
                return None;
            }
            let event = self.buf[tail];
            self.tail
                .store((tail + 1) & (EVENT_BUFFER_SIZE - 1), Ordering::Release);
            Some(event)
        }

        fn has_data(&self) -> bool {
            self.tail.load(Ordering::Relaxed) != self.head.load(Ordering::Acquire)
        }
    }

    // SAFETY: MouseBuffer uses atomic operations for synchronization.
    // Single producer (poll_mouse) / single consumer (read_event).
    unsafe impl Send for MouseBuffer {}
    unsafe impl Sync for MouseBuffer {}

    static mut MOUSE_BUFFER: MouseBuffer = MouseBuffer::new();

    /// Packet accumulation state
    static mut PACKET_BUF: [u8; 3] = [0; 3];
    static mut PACKET_IDX: usize = 0;

    /// Wait for the PS/2 controller input buffer to be empty.
    #[inline]
    fn wait_input() {
        for _ in 0..10_000 {
            // SAFETY: Reading PS/2 status register (port 0x64).
            let status = unsafe { crate::arch::x86_64::inb(0x64) };
            if (status & 0x02) == 0 {
                return;
            }
        }
    }

    /// Wait for PS/2 controller output buffer to have data.
    #[inline]
    fn wait_output() -> bool {
        for _ in 0..10_000 {
            // SAFETY: Reading PS/2 status register.
            let status = unsafe { crate::arch::x86_64::inb(0x64) };
            if (status & 0x01) != 0 {
                return true;
            }
        }
        false
    }

    /// Send a command byte to the PS/2 auxiliary device (mouse).
    fn mouse_write(cmd: u8) {
        wait_input();
        // SAFETY: Writing to PS/2 command port.
        unsafe {
            crate::arch::x86_64::outb(0x64, 0xD4); // Next byte goes to aux
                                                   // device
        }
        wait_input();
        // SAFETY: Writing data byte to PS/2 data port.
        unsafe {
            crate::arch::x86_64::outb(0x60, cmd);
        }
    }

    /// Read a byte from the PS/2 data port (with timeout).
    fn mouse_read() -> Option<u8> {
        if wait_output() {
            // SAFETY: Reading PS/2 data port.
            Some(unsafe { crate::arch::x86_64::inb(0x60) })
        } else {
            None
        }
    }

    /// Initialize the PS/2 mouse.
    pub fn init() {
        // Enable the auxiliary device (mouse port)
        wait_input();
        // SAFETY: Sending PS/2 controller commands.
        unsafe {
            crate::arch::x86_64::outb(0x64, 0xA8); // Enable aux port
        }

        // Get controller configuration byte
        wait_input();
        unsafe {
            crate::arch::x86_64::outb(0x64, 0x20);
        }
        let config_byte = if wait_output() {
            Some(unsafe { crate::arch::x86_64::inb(0x60) })
        } else {
            None
        };
        if let Some(mut config) = config_byte {
            // Enable IRQ12 (aux interrupt) and ensure aux clock is enabled
            config |= 0x02; // Enable IRQ12
            config &= !0x20; // Disable aux clock inhibit

            wait_input();
            unsafe {
                crate::arch::x86_64::outb(0x64, 0x60); // Write config
            }
            wait_input();
            unsafe {
                crate::arch::x86_64::outb(0x60, config);
            }
        }

        // Reset mouse
        mouse_write(0xFF);
        let _ = mouse_read(); // ACK
        let _ = mouse_read(); // Self-test result
        let _ = mouse_read(); // Device ID

        // Set defaults
        mouse_write(0xF6);
        let _ = mouse_read(); // ACK

        // Enable data streaming
        mouse_write(0xF4);
        let _ = mouse_read(); // ACK

        INITIALIZED.store(true, Ordering::Release);
        crate::println!("[MOUSE] PS/2 mouse initialized");
    }

    /// Poll for mouse data and push complete packets to the event buffer.
    ///
    /// Called periodically from the input polling loop.
    pub fn poll_mouse() {
        if !INITIALIZED.load(Ordering::Acquire) {
            return;
        }

        // Check if aux port has data (status bit 5 = aux data, bit 0 = data ready)
        // SAFETY: Reading PS/2 status register.
        let status = unsafe { crate::arch::x86_64::inb(0x64) };
        if (status & 0x21) != 0x21 {
            // Bit 0 (data available) + bit 5 (from aux port)
            return;
        }

        // SAFETY: Reading PS/2 data port.
        let byte = unsafe { crate::arch::x86_64::inb(0x60) };

        // SAFETY: Single-threaded access (polling from main loop).
        #[allow(static_mut_refs)]
        unsafe {
            PACKET_BUF[PACKET_IDX] = byte;
            PACKET_IDX += 1;

            if PACKET_IDX >= 3 {
                PACKET_IDX = 0;
                let status_byte = PACKET_BUF[0];

                // Validate: bit 3 must always be set in a standard PS/2 packet
                if (status_byte & 0x08) == 0 {
                    return; // Resync
                }

                let buttons = status_byte & 0x07;

                // Reconstruct signed deltas
                let mut dx = PACKET_BUF[1] as i16;
                let mut dy = PACKET_BUF[2] as i16;
                if (status_byte & 0x10) != 0 {
                    dx -= 256; // X sign bit
                }
                if (status_byte & 0x20) != 0 {
                    dy -= 256; // Y sign bit
                }

                // PS/2 Y axis is inverted (up = positive)
                dy = -dy;

                // Update absolute cursor position
                let sw = SCREEN_WIDTH.load(Ordering::Relaxed) as i32;
                let sh = SCREEN_HEIGHT.load(Ordering::Relaxed) as i32;
                let mut cx = CURSOR_X.load(Ordering::Relaxed) + dx as i32;
                let mut cy = CURSOR_Y.load(Ordering::Relaxed) + dy as i32;
                cx = cx.clamp(0, sw - 1);
                cy = cy.clamp(0, sh - 1);
                CURSOR_X.store(cx, Ordering::Relaxed);
                CURSOR_Y.store(cy, Ordering::Relaxed);

                let event = MouseEvent { dx, dy, buttons };
                // SAFETY: Single producer (this function).
                #[allow(static_mut_refs)]
                MOUSE_BUFFER.push(event);
            }
        }
    }

    /// Read a mouse event from the buffer.
    pub fn read_event() -> Option<MouseEvent> {
        // SAFETY: Single consumer.
        #[allow(static_mut_refs)]
        unsafe {
            MOUSE_BUFFER.pop()
        }
    }

    /// Check if mouse buffer has pending events.
    pub fn has_data() -> bool {
        // SAFETY: Atomic reads only.
        #[allow(static_mut_refs)]
        unsafe {
            MOUSE_BUFFER.has_data()
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub use x86_64_impl::{has_data, init, poll_mouse, read_event};

// ---------------------------------------------------------------------------
// Stubs for non-x86_64 architectures
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "x86_64"))]
pub fn init() {}

#[cfg(not(target_arch = "x86_64"))]
pub fn poll_mouse() {}

#[cfg(not(target_arch = "x86_64"))]
pub fn read_event() -> Option<MouseEvent> {
    None
}

#[cfg(not(target_arch = "x86_64"))]
pub fn has_data() -> bool {
    false
}

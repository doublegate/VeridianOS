//! Unified input event subsystem.
//!
//! Collects keyboard and mouse input into a single event stream using
//! a Linux-compatible event structure. User-space reads events via
//! `sys_input_read()`.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Input event types (Linux evdev compatible).
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
#[allow(dead_code)] // Future: touchscreen, tablet
pub const EV_ABS: u16 = 0x03;

/// Relative axis codes.
pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;

/// Mouse button codes (Linux BTN_* values).
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

/// Input event structure (ABI-stable for user space).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

impl InputEvent {
    pub const fn key(code: u16, pressed: bool) -> Self {
        Self {
            timestamp: 0, // Filled on push
            event_type: EV_KEY,
            code,
            value: if pressed { 1 } else { 0 },
        }
    }

    pub const fn rel(code: u16, value: i32) -> Self {
        Self {
            timestamp: 0,
            event_type: EV_REL,
            code,
            value,
        }
    }
}

/// Ring buffer for input events.
const EVENT_BUFFER_SIZE: usize = 256;

struct EventBuffer {
    buf: [InputEvent; EVENT_BUFFER_SIZE],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl EventBuffer {
    const fn new() -> Self {
        Self {
            buf: [InputEvent {
                timestamp: 0,
                event_type: 0,
                code: 0,
                value: 0,
            }; EVENT_BUFFER_SIZE],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn push(&mut self, mut event: InputEvent) {
        event.timestamp = crate::arch::timer::read_hw_timestamp();
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & (EVENT_BUFFER_SIZE - 1);
        let tail = self.tail.load(Ordering::Acquire);
        if next == tail {
            return; // Buffer full, drop event
        }
        self.buf[head] = event;
        self.head.store(next, Ordering::Release);
    }

    fn pop(&self) -> Option<InputEvent> {
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
}

// SAFETY: EventBuffer uses atomic head/tail. Single producer (poll_all)
// and single consumer (read_event).
unsafe impl Send for EventBuffer {}
unsafe impl Sync for EventBuffer {}

static mut EVENT_BUFFER: EventBuffer = EventBuffer::new();

/// Push an input event into the global event queue.
///
/// Called by keyboard and mouse drivers.
pub fn push_event(event: InputEvent) {
    // SAFETY: Single producer from polling loop.
    #[allow(static_mut_refs)]
    unsafe {
        EVENT_BUFFER.push(event);
    }
}

/// Read the next input event from the queue.
pub fn read_event() -> Option<InputEvent> {
    // SAFETY: Single consumer.
    #[allow(static_mut_refs)]
    unsafe {
        EVENT_BUFFER.pop()
    }
}

/// Poll all input sources and convert to input events.
///
/// Called periodically (e.g., from APIC timer or shell loop).
pub fn poll_all() {
    // Poll keyboard and mouse hardware, then drain decoded buffers
    #[cfg(target_arch = "x86_64")]
    {
        // Poll PS/2 controller directly for both keyboard and mouse data.
        // The APIC takes over interrupt routing from the PIC, so IRQ1
        // (keyboard) and IRQ12 (mouse) may never fire. We must poll the
        // controller status port (0x64) to capture input from the QEMU
        // graphical window.
        //
        // Status register bits:
        //   bit 0 = output buffer full (data available in port 0x60)
        //   bit 5 = data is from auxiliary (mouse) port
        //
        // We loop to drain all pending PS/2 bytes in a single poll_all()
        // call, dispatching keyboard bytes to handle_scancode() and mouse
        // bytes to poll_mouse_byte().
        for _ in 0..64 {
            // SAFETY: Reading PS/2 status register (port 0x64).
            let status = unsafe { crate::arch::x86_64::inb(0x64) };
            if (status & 0x01) == 0 {
                break; // No data available
            }
            if (status & 0x20) != 0 {
                // Bit 5 set: data is from auxiliary (mouse) port
                // SAFETY: Reading PS/2 data port (port 0x60).
                let byte = unsafe { crate::arch::x86_64::inb(0x60) };
                crate::drivers::mouse::poll_mouse_byte(byte);
            } else {
                // Bit 5 clear: data is from keyboard port
                // SAFETY: Reading PS/2 data port (port 0x60).
                let scancode = unsafe { crate::arch::x86_64::inb(0x60) };
                crate::drivers::keyboard::handle_scancode(scancode);
            }
        }
    }

    // Drain decoded keyboard buffer into unified event stream
    #[cfg(target_arch = "x86_64")]
    {
        while let Some(key_byte) = crate::drivers::keyboard::read_key() {
            push_event(InputEvent::key(key_byte as u16, true));
        }
    }

    // Drain decoded mouse events into unified event stream
    #[cfg(target_arch = "x86_64")]
    {
        while let Some(mouse_event) = crate::drivers::mouse::read_event() {
            // Relative movement events
            if mouse_event.dx != 0 {
                push_event(InputEvent::rel(REL_X, mouse_event.dx as i32));
            }
            if mouse_event.dy != 0 {
                push_event(InputEvent::rel(REL_Y, mouse_event.dy as i32));
            }
            // Button events
            static mut PREV_BUTTONS: u8 = 0;
            #[allow(static_mut_refs)]
            unsafe {
                let changed = mouse_event.buttons ^ PREV_BUTTONS;
                if (changed & super::mouse::BUTTON_LEFT) != 0 {
                    push_event(InputEvent::key(
                        BTN_LEFT,
                        (mouse_event.buttons & super::mouse::BUTTON_LEFT) != 0,
                    ));
                }
                if (changed & super::mouse::BUTTON_RIGHT) != 0 {
                    push_event(InputEvent::key(
                        BTN_RIGHT,
                        (mouse_event.buttons & super::mouse::BUTTON_RIGHT) != 0,
                    ));
                }
                if (changed & super::mouse::BUTTON_MIDDLE) != 0 {
                    push_event(InputEvent::key(
                        BTN_MIDDLE,
                        (mouse_event.buttons & super::mouse::BUTTON_MIDDLE) != 0,
                    ));
                }
                PREV_BUTTONS = mouse_event.buttons;
            }
        }
    }
}

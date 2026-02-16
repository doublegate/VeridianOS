//! PS/2 keyboard driver for x86_64.
//!
//! Reads scancodes from I/O port 0x60, decodes them via the `pc_keyboard`
//! crate (ScancodeSet1, US 104-key layout), and pushes decoded ASCII bytes
//! to a lock-free ring buffer. The shell reads from this buffer.
//!
//! On non-x86_64 architectures, all functions are no-op stubs.

use core::sync::atomic::{AtomicBool, Ordering};

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Check if the keyboard driver has been initialized.
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// x86_64 implementation
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
mod x86_64_impl {
    use core::sync::atomic::AtomicUsize;

    use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};
    use spin::Mutex;

    use super::*;

    /// Ring buffer size for decoded key bytes (must be power of 2).
    const KEY_BUFFER_SIZE: usize = 256;

    /// Lock-free single-producer single-consumer ring buffer for decoded keys.
    struct KeyBuffer {
        buf: [u8; KEY_BUFFER_SIZE],
        head: AtomicUsize,
        tail: AtomicUsize,
    }

    impl KeyBuffer {
        const fn new() -> Self {
            Self {
                buf: [0; KEY_BUFFER_SIZE],
                head: AtomicUsize::new(0),
                tail: AtomicUsize::new(0),
            }
        }

        /// Push a byte (called from interrupt handler -- single producer).
        fn push(&mut self, byte: u8) {
            let head = self.head.load(Ordering::Relaxed);
            let next = (head + 1) & (KEY_BUFFER_SIZE - 1);
            let tail = self.tail.load(Ordering::Acquire);
            if next == tail {
                return; // Buffer full, drop key
            }
            self.buf[head] = byte;
            self.head.store(next, Ordering::Release);
        }

        /// Pop a byte (called from shell main loop -- single consumer).
        fn pop(&self) -> Option<u8> {
            let tail = self.tail.load(Ordering::Relaxed);
            let head = self.head.load(Ordering::Acquire);
            if tail == head {
                return None;
            }
            let byte = self.buf[tail];
            self.tail
                .store((tail + 1) & (KEY_BUFFER_SIZE - 1), Ordering::Release);
            Some(byte)
        }
    }

    // SAFETY: KeyBuffer uses atomic operations for head/tail synchronization.
    // The push side (interrupt handler) is single-producer and pop side
    // (shell loop) is single-consumer, making concurrent access safe.
    unsafe impl Send for KeyBuffer {}
    unsafe impl Sync for KeyBuffer {}

    static mut KEY_BUFFER: KeyBuffer = KeyBuffer::new();

    static KEYBOARD: Mutex<Option<Keyboard<layouts::Us104Key, ScancodeSet1>>> = Mutex::new(None);

    /// Initialize the PS/2 keyboard driver.
    pub fn init() {
        let kb = Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::MapLettersToUnicode,
        );
        *KEYBOARD.lock() = Some(kb);
        INITIALIZED.store(true, Ordering::Release);
    }

    /// Handle a scancode from the PS/2 keyboard interrupt (vector 33).
    ///
    /// This function must NOT call println! or acquire any spinlock used
    /// by the serial/fbcon output path.
    pub fn handle_scancode(scancode: u8) {
        let mut kb_guard = KEYBOARD.lock();
        if let Some(ref mut keyboard) = *kb_guard {
            if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
                if let Some(key) = keyboard.process_keyevent(key_event) {
                    match key {
                        DecodedKey::Unicode(ch) => {
                            if ch.is_ascii() {
                                // SAFETY: handle_scancode is the sole producer
                                // (called from IRQ1 with interrupts disabled).
                                #[allow(static_mut_refs)]
                                unsafe {
                                    KEY_BUFFER.push(ch as u8);
                                }
                            }
                        }
                        DecodedKey::RawKey(_) => {}
                    }
                }
            }
        }
    }

    /// Read a decoded key byte from the keyboard buffer (non-blocking).
    pub fn read_key() -> Option<u8> {
        // SAFETY: read_key is the sole consumer, called from the shell loop.
        #[allow(static_mut_refs)]
        unsafe {
            KEY_BUFFER.pop()
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub use x86_64_impl::{handle_scancode, init, read_key};

// ---------------------------------------------------------------------------
// Stubs for non-x86_64 architectures
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "x86_64"))]
pub fn init() {}

#[cfg(not(target_arch = "x86_64"))]
pub fn read_key() -> Option<u8> {
    None
}

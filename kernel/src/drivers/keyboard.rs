//! PS/2 keyboard driver for x86_64.
//!
//! Reads scancodes from I/O port 0x60, decodes them via the `pc_keyboard`
//! crate (ScancodeSet1, US 104-key layout), and pushes decoded ASCII bytes
//! to a lock-free ring buffer. The shell reads from this buffer.
//!
//! On non-x86_64 architectures, all functions are no-op stubs.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Check if the keyboard driver has been initialized.
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Modifier state tracking
// ---------------------------------------------------------------------------

/// Bitmask: Shift is held.
pub const MOD_SHIFT: u8 = 0x01;
/// Bitmask: Ctrl is held.
pub const MOD_CTRL: u8 = 0x02;
/// Bitmask: Alt is held.
pub const MOD_ALT: u8 = 0x04;
/// Bitmask: Super/Win is held.
pub const MOD_SUPER: u8 = 0x08;

static MODIFIER_STATE: AtomicU8 = AtomicU8::new(0);

/// Get the current modifier key bitmask.
pub fn get_modifiers() -> u8 {
    MODIFIER_STATE.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// GUI mode: single-byte key codes for special keys
// ---------------------------------------------------------------------------

/// When true, arrow/special keys emit single-byte codes (0x80+) instead of
/// multi-byte ANSI escape sequences. This prevents the 0x1B ESC prefix from
/// triggering the GUI exit guard.
static GUI_MODE: AtomicBool = AtomicBool::new(false);

/// Enable or disable GUI key encoding mode.
pub fn set_gui_mode(enabled: bool) {
    GUI_MODE.store(enabled, Ordering::Release);
}

/// Single-byte key code for Up arrow (GUI mode).
pub const KEY_UP: u8 = 0x80;
/// Single-byte key code for Down arrow (GUI mode).
pub const KEY_DOWN: u8 = 0x81;
/// Single-byte key code for Left arrow (GUI mode).
pub const KEY_LEFT: u8 = 0x82;
/// Single-byte key code for Right arrow (GUI mode).
pub const KEY_RIGHT: u8 = 0x83;
/// Single-byte key code for Home (GUI mode).
pub const KEY_HOME: u8 = 0x84;
/// Single-byte key code for End (GUI mode).
pub const KEY_END: u8 = 0x85;
/// Single-byte key code for Delete (GUI mode).
pub const KEY_DELETE: u8 = 0x86;

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
        use pc_keyboard::KeyCode;

        let mut kb_guard = KEYBOARD.lock();
        if let Some(ref mut keyboard) = *kb_guard {
            if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
                // Track modifier state from the raw key event BEFORE
                // process_keyevent consumes it. Modifiers are tracked
                // globally so the render loop can detect hotkeys.
                let code = key_event.code;
                let is_down = key_event.state == pc_keyboard::KeyState::Down;
                match code {
                    KeyCode::LShift | KeyCode::RShift => update_modifier(MOD_SHIFT, is_down),
                    KeyCode::LControl | KeyCode::RControl => update_modifier(MOD_CTRL, is_down),
                    KeyCode::LAlt | KeyCode::RAltGr => update_modifier(MOD_ALT, is_down),
                    KeyCode::LWin | KeyCode::RWin => update_modifier(MOD_SUPER, is_down),
                    _ => {}
                }

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
                        DecodedKey::RawKey(key) => {
                            if GUI_MODE.load(Ordering::Relaxed) {
                                // GUI mode: emit single-byte codes for special
                                // keys to avoid ANSI escape sequence conflicts.
                                let gui_byte = match key {
                                    KeyCode::ArrowUp => Some(KEY_UP),
                                    KeyCode::ArrowDown => Some(KEY_DOWN),
                                    KeyCode::ArrowRight => Some(KEY_RIGHT),
                                    KeyCode::ArrowLeft => Some(KEY_LEFT),
                                    KeyCode::Home => Some(KEY_HOME),
                                    KeyCode::End => Some(KEY_END),
                                    KeyCode::Delete => Some(KEY_DELETE),
                                    _ => None,
                                };
                                if let Some(byte) = gui_byte {
                                    #[allow(static_mut_refs)]
                                    // SAFETY: sole producer (IRQ1, interrupts disabled).
                                    unsafe {
                                        KEY_BUFFER.push(byte);
                                    }
                                }
                            } else {
                                // Shell mode: emit ANSI escape sequences as before.
                                let seq: &[u8] = match key {
                                    KeyCode::ArrowUp => b"\x1b[A",
                                    KeyCode::ArrowDown => b"\x1b[B",
                                    KeyCode::ArrowRight => b"\x1b[C",
                                    KeyCode::ArrowLeft => b"\x1b[D",
                                    KeyCode::Home => b"\x1b[H",
                                    KeyCode::End => b"\x1b[F",
                                    KeyCode::Delete => b"\x1b[3~",
                                    _ => b"",
                                };
                                // SAFETY: handle_scancode is the sole producer
                                // (called from IRQ1 with interrupts disabled).
                                #[allow(static_mut_refs)]
                                unsafe {
                                    for &byte in seq {
                                        KEY_BUFFER.push(byte);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Update a modifier bit in the global modifier state.
    fn update_modifier(bit: u8, down: bool) {
        if down {
            MODIFIER_STATE.fetch_or(bit, Ordering::Relaxed);
        } else {
            MODIFIER_STATE.fetch_and(!bit, Ordering::Relaxed);
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

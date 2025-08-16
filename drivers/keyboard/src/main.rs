//! PS/2 Keyboard driver for VeridianOS
//!
//! This driver handles PS/2 keyboard input as a user-space driver.

#![no_std]
#![no_main]

extern crate libveridian;
use libveridian::{println, sys};

extern crate veridian_driver_common;
use veridian_driver_common::{
    CharDriver, Driver, DriverError, DriverInfo, DriverState,
};

// PS/2 keyboard ports
const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;
const COMMAND_PORT: u16 = 0x64;

// PS/2 keyboard commands
const CMD_SET_LEDS: u8 = 0xED;
const CMD_ENABLE_SCANNING: u8 = 0xF4;
const CMD_DISABLE_SCANNING: u8 = 0xF5;
const CMD_RESET: u8 = 0xFF;

/// Keyboard driver structure
struct KeyboardDriver {
    info: DriverInfo,
    state: DriverState,
    buffer: [u8; 256],
    buffer_head: usize,
    buffer_tail: usize,
    shift_pressed: bool,
    ctrl_pressed: bool,
    alt_pressed: bool,
}

impl KeyboardDriver {
    /// Create a new keyboard driver
    fn new() -> Self {
        Self {
            info: DriverInfo {
                name: "ps2kbd",
                version: (1, 0, 0),
                author: "VeridianOS Team",
                description: "PS/2 Keyboard driver",
                device_ids: &[(0x0000, 0x0001)], // Generic PS/2 keyboard
                required_caps: veridian_driver_common::CAP_DEVICE_ACCESS 
                    | veridian_driver_common::CAP_INTERRUPT_HANDLER,
            },
            state: DriverState::Initializing,
            buffer: [0; 256],
            buffer_head: 0,
            buffer_tail: 0,
            shift_pressed: false,
            ctrl_pressed: false,
            alt_pressed: false,
        }
    }
    
    /// Add a character to the buffer
    fn add_to_buffer(&mut self, ch: u8) {
        let next_head = (self.buffer_head + 1) % self.buffer.len();
        if next_head != self.buffer_tail {
            self.buffer[self.buffer_head] = ch;
            self.buffer_head = next_head;
        }
    }
    
    /// Read a character from the buffer
    fn read_from_buffer(&mut self) -> Option<u8> {
        if self.buffer_head == self.buffer_tail {
            None
        } else {
            let ch = self.buffer[self.buffer_tail];
            self.buffer_tail = (self.buffer_tail + 1) % self.buffer.len();
            Some(ch)
        }
    }
    
    /// Process a scan code and convert to ASCII
    fn process_scancode(&mut self, scancode: u8) {
        // Simple scancode to ASCII mapping (US layout)
        let ascii = match scancode {
            0x01 => Some(27),  // ESC
            0x02 => Some(if self.shift_pressed { b'!' } else { b'1' }),
            0x03 => Some(if self.shift_pressed { b'@' } else { b'2' }),
            0x04 => Some(if self.shift_pressed { b'#' } else { b'3' }),
            0x05 => Some(if self.shift_pressed { b'$' } else { b'4' }),
            0x06 => Some(if self.shift_pressed { b'%' } else { b'5' }),
            0x07 => Some(if self.shift_pressed { b'^' } else { b'6' }),
            0x08 => Some(if self.shift_pressed { b'&' } else { b'7' }),
            0x09 => Some(if self.shift_pressed { b'*' } else { b'8' }),
            0x0A => Some(if self.shift_pressed { b'(' } else { b'9' }),
            0x0B => Some(if self.shift_pressed { b')' } else { b'0' }),
            0x0C => Some(if self.shift_pressed { b'_' } else { b'-' }),
            0x0D => Some(if self.shift_pressed { b'+' } else { b'=' }),
            0x0E => Some(8),   // Backspace
            0x0F => Some(9),   // Tab
            
            // Letters (Q-P)
            0x10 => Some(if self.shift_pressed { b'Q' } else { b'q' }),
            0x11 => Some(if self.shift_pressed { b'W' } else { b'w' }),
            0x12 => Some(if self.shift_pressed { b'E' } else { b'e' }),
            0x13 => Some(if self.shift_pressed { b'R' } else { b'r' }),
            0x14 => Some(if self.shift_pressed { b'T' } else { b't' }),
            0x15 => Some(if self.shift_pressed { b'Y' } else { b'y' }),
            0x16 => Some(if self.shift_pressed { b'U' } else { b'u' }),
            0x17 => Some(if self.shift_pressed { b'I' } else { b'i' }),
            0x18 => Some(if self.shift_pressed { b'O' } else { b'o' }),
            0x19 => Some(if self.shift_pressed { b'P' } else { b'p' }),
            
            0x1C => Some(b'\n'), // Enter
            
            // Letters (A-L)
            0x1E => Some(if self.shift_pressed { b'A' } else { b'a' }),
            0x1F => Some(if self.shift_pressed { b'S' } else { b's' }),
            0x20 => Some(if self.shift_pressed { b'D' } else { b'd' }),
            0x21 => Some(if self.shift_pressed { b'F' } else { b'f' }),
            0x22 => Some(if self.shift_pressed { b'G' } else { b'g' }),
            0x23 => Some(if self.shift_pressed { b'H' } else { b'h' }),
            0x24 => Some(if self.shift_pressed { b'J' } else { b'j' }),
            0x25 => Some(if self.shift_pressed { b'K' } else { b'k' }),
            0x26 => Some(if self.shift_pressed { b'L' } else { b'l' }),
            
            // Letters (Z-M)
            0x2C => Some(if self.shift_pressed { b'Z' } else { b'z' }),
            0x2D => Some(if self.shift_pressed { b'X' } else { b'x' }),
            0x2E => Some(if self.shift_pressed { b'C' } else { b'c' }),
            0x2F => Some(if self.shift_pressed { b'V' } else { b'v' }),
            0x30 => Some(if self.shift_pressed { b'B' } else { b'b' }),
            0x31 => Some(if self.shift_pressed { b'N' } else { b'n' }),
            0x32 => Some(if self.shift_pressed { b'M' } else { b'm' }),
            
            0x39 => Some(b' '), // Space
            
            // Special keys (press)
            0x2A | 0x36 => { // Left/Right Shift
                self.shift_pressed = true;
                None
            }
            0x1D => { // Ctrl
                self.ctrl_pressed = true;
                None
            }
            0x38 => { // Alt
                self.alt_pressed = true;
                None
            }
            
            // Special keys (release)
            0xAA | 0xB6 => { // Left/Right Shift release
                self.shift_pressed = false;
                None
            }
            0x9D => { // Ctrl release
                self.ctrl_pressed = false;
                None
            }
            0xB8 => { // Alt release
                self.alt_pressed = false;
                None
            }
            
            _ => None,
        };
        
        if let Some(ch) = ascii {
            self.add_to_buffer(ch);
        }
    }
}

impl Driver for KeyboardDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }
    
    fn init(&mut self) -> Result<(), DriverError> {
        println!("[KBD] Initializing PS/2 keyboard driver");
        
        // TODO: Request I/O port access from kernel
        // TODO: Send initialization commands to keyboard
        
        self.state = DriverState::Ready;
        Ok(())
    }
    
    fn probe(&self) -> Result<bool, DriverError> {
        // TODO: Check if PS/2 keyboard is present
        Ok(true)
    }
    
    fn start(&mut self) -> Result<(), DriverError> {
        println!("[KBD] Starting PS/2 keyboard driver");
        
        // TODO: Register interrupt handler with kernel
        // TODO: Enable keyboard scanning
        
        self.state = DriverState::Ready;
        Ok(())
    }
    
    fn stop(&mut self) -> Result<(), DriverError> {
        println!("[KBD] Stopping PS/2 keyboard driver");
        
        // TODO: Disable keyboard scanning
        // TODO: Unregister interrupt handler
        
        self.state = DriverState::Stopped;
        Ok(())
    }
    
    fn state(&self) -> DriverState {
        self.state
    }
    
    fn handle_interrupt(&mut self, _irq: u32) -> Result<(), DriverError> {
        // TODO: Read scan code from data port
        // For now, simulate a scan code
        let scancode = 0x1C; // Simulate Enter key
        
        self.process_scancode(scancode);
        Ok(())
    }
}

impl CharDriver for KeyboardDriver {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, DriverError> {
        let mut count = 0;
        
        while count < buffer.len() {
            if let Some(ch) = self.read_from_buffer() {
                buffer[count] = ch;
                count += 1;
            } else {
                break;
            }
        }
        
        Ok(count)
    }
    
    fn write(&mut self, _data: &[u8]) -> Result<usize, DriverError> {
        // Keyboard is input-only
        Err(DriverError::NotSupported)
    }
    
    fn poll(&self) -> Result<bool, DriverError> {
        Ok(self.buffer_head != self.buffer_tail)
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("[KBD] PS/2 Keyboard driver starting...");
    
    // Create driver instance
    let mut driver = KeyboardDriver::new();
    
    // Initialize driver
    if let Err(e) = driver.init() {
        println!("[KBD] Failed to initialize: {:?}", e);
        sys::exit(1);
    }
    
    // Start driver
    if let Err(e) = driver.start() {
        println!("[KBD] Failed to start: {:?}", e);
        sys::exit(1);
    }
    
    println!("[KBD] Driver ready");
    
    // Main driver loop
    loop {
        // Wait for interrupt or IPC message
        sys::yield_cpu();
        
        // TODO: Handle actual interrupts and IPC messages
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("[KBD] Driver panic!");
    sys::exit(255);
}
//! PS/2 Keyboard Driver for VeridianOS

#![no_std]
#![no_main]
#![feature(asm_const)]

extern crate driver_common;

use core::sync::atomic::{AtomicBool, Ordering};
use driver_common::{Driver, DriverError, DriverInfo, DriverState, CharDriver};

// PS/2 Controller I/O ports
const PS2_DATA_PORT: u16 = 0x60;
const PS2_STATUS_PORT: u16 = 0x64;
const PS2_COMMAND_PORT: u16 = 0x64;

// PS/2 Controller commands
const PS2_CMD_READ_CONFIG: u8 = 0x20;
const PS2_CMD_WRITE_CONFIG: u8 = 0x60;
const PS2_CMD_DISABLE_PORT2: u8 = 0xA7;
const PS2_CMD_ENABLE_PORT1: u8 = 0xAE;
const PS2_CMD_DISABLE_PORT1: u8 = 0xAD;
const PS2_CMD_TEST_CONTROLLER: u8 = 0xAA;
const PS2_CMD_TEST_PORT1: u8 = 0xAB;

// Keyboard commands
const KBD_CMD_SET_LEDS: u8 = 0xED;
const KBD_CMD_RESET: u8 = 0xFF;
const KBD_CMD_ENABLE_SCANNING: u8 = 0xF4;
const KBD_CMD_DISABLE_SCANNING: u8 = 0xF5;

// Keyboard responses
const KBD_RESP_ACK: u8 = 0xFA;
const KBD_RESP_RESEND: u8 = 0xFE;
const KBD_RESP_SELF_TEST_PASSED: u8 = 0xAA;

// Status register bits
const PS2_STATUS_OUTPUT_FULL: u8 = 0x01;
const PS2_STATUS_INPUT_FULL: u8 = 0x02;

/// Simple scancode to ASCII conversion table (US layout)
static SCANCODE_TO_ASCII: [u8; 128] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 8, b'\t',
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0, b'a', b's',
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v',
    b'b', b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, b'7', b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',
    b'2', b'3', b'0', b'.', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// PS/2 Keyboard Driver
pub struct Ps2KeyboardDriver {
    info: DriverInfo,
    state: DriverState,
    buffer: [u8; 256],
    buffer_start: usize,
    buffer_end: usize,
    caps_lock: AtomicBool,
    num_lock: AtomicBool,
    scroll_lock: AtomicBool,
    shift_pressed: AtomicBool,
    ctrl_pressed: AtomicBool,
    alt_pressed: AtomicBool,
}

impl Ps2KeyboardDriver {
    /// Create a new PS/2 keyboard driver
    pub const fn new() -> Self {
        let info = DriverInfo {
            name: "ps2-keyboard",
            version: (0, 1, 0),
            author: "VeridianOS Team",
            description: "PS/2 Keyboard Driver",
            device_ids: &[],
            required_caps: driver_common::CAP_DEVICE_ACCESS,
        };
        
        Self {
            info,
            state: DriverState::Stopped,
            buffer: [0; 256],
            buffer_start: 0,
            buffer_end: 0,
            caps_lock: AtomicBool::new(false),
            num_lock: AtomicBool::new(false),
            scroll_lock: AtomicBool::new(false),
            shift_pressed: AtomicBool::new(false),
            ctrl_pressed: AtomicBool::new(false),
            alt_pressed: AtomicBool::new(false),
        }
    }
    
    /// Read from PS/2 data port
    #[inline]
    unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        #[cfg(target_arch = "x86_64")]
        {
            use core::arch::asm;
            asm!("in al, dx", out("al") value, in("dx") port);
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            value = 0; // Placeholder for other architectures
        }
        value
    }
    
    /// Write to PS/2 data port
    #[inline]
    unsafe fn outb(port: u16, value: u8) {
        #[cfg(target_arch = "x86_64")]
        {
            use core::arch::asm;
            asm!("out dx, al", in("dx") port, in("al") value);
        }
    }
    
    /// Wait for PS/2 controller input buffer to be empty
    fn wait_input_empty(&self) -> Result<(), DriverError> {
        for _ in 0..10000 {
            unsafe {
                if (Self::inb(PS2_STATUS_PORT) & PS2_STATUS_INPUT_FULL) == 0 {
                    return Ok(());
                }
            }
        }
        Err(DriverError::Timeout)
    }
    
    /// Wait for PS/2 controller output buffer to be full
    fn wait_output_full(&self) -> Result<(), DriverError> {
        for _ in 0..10000 {
            unsafe {
                if (Self::inb(PS2_STATUS_PORT) & PS2_STATUS_OUTPUT_FULL) != 0 {
                    return Ok(());
                }
            }
        }
        Err(DriverError::Timeout)
    }
    
    /// Send command to PS/2 controller
    fn send_controller_command(&self, cmd: u8) -> Result<(), DriverError> {
        self.wait_input_empty()?;
        unsafe {
            Self::outb(PS2_COMMAND_PORT, cmd);
        }
        Ok(())
    }
    
    /// Send command to keyboard
    fn send_keyboard_command(&self, cmd: u8) -> Result<(), DriverError> {
        self.wait_input_empty()?;
        unsafe {
            Self::outb(PS2_DATA_PORT, cmd);
        }
        
        // Wait for ACK
        self.wait_output_full()?;
        unsafe {
            let response = Self::inb(PS2_DATA_PORT);
            if response == KBD_RESP_ACK {
                Ok(())
            } else if response == KBD_RESP_RESEND {
                Err(DriverError::IoError)
            } else {
                Err(DriverError::DeviceNotSupported)
            }
        }
    }
    
    /// Process a scancode
    fn process_scancode(&mut self, scancode: u8) {
        // Handle key release (bit 7 set)
        let released = (scancode & 0x80) != 0;
        let key_code = scancode & 0x7F;
        
        // Handle special keys
        match key_code {
            0x2A | 0x36 => { // Left/Right Shift
                self.shift_pressed.store(!released, Ordering::Relaxed);
                return;
            }
            0x1D => { // Control
                self.ctrl_pressed.store(!released, Ordering::Relaxed);
                return;
            }
            0x38 => { // Alt
                self.alt_pressed.store(!released, Ordering::Relaxed);
                return;
            }
            0x3A => { // Caps Lock
                if !released {
                    let new_state = !self.caps_lock.load(Ordering::Relaxed);
                    self.caps_lock.store(new_state, Ordering::Relaxed);
                    self.update_leds().ok();
                }
                return;
            }
            0x45 => { // Num Lock
                if !released {
                    let new_state = !self.num_lock.load(Ordering::Relaxed);
                    self.num_lock.store(new_state, Ordering::Relaxed);
                    self.update_leds().ok();
                }
                return;
            }
            0x46 => { // Scroll Lock
                if !released {
                    let new_state = !self.scroll_lock.load(Ordering::Relaxed);
                    self.scroll_lock.store(new_state, Ordering::Relaxed);
                    self.update_leds().ok();
                }
                return;
            }
            _ => {}
        }
        
        // Only process key press events for regular keys
        if !released && key_code < 128 {
            if let Some(mut ascii) = SCANCODE_TO_ASCII.get(key_code as usize).copied() {
                if ascii != 0 {
                    // Apply modifiers
                    let shift = self.shift_pressed.load(Ordering::Relaxed);
                    let caps = self.caps_lock.load(Ordering::Relaxed);
                    
                    // Handle shift for letters
                    if ascii >= b'a' && ascii <= b'z' {
                        if shift ^ caps {
                            ascii = ascii - b'a' + b'A';
                        }
                    } else if shift {
                        // Handle shift for other characters
                        ascii = match ascii {
                            b'1' => b'!',
                            b'2' => b'@',
                            b'3' => b'#',
                            b'4' => b'$',
                            b'5' => b'%',
                            b'6' => b'^',
                            b'7' => b'&',
                            b'8' => b'*',
                            b'9' => b'(',
                            b'0' => b')',
                            b'-' => b'_',
                            b'=' => b'+',
                            b'[' => b'{',
                            b']' => b'}',
                            b';' => b':',
                            b'\'' => b'"',
                            b',' => b'<',
                            b'.' => b'>',
                            b'/' => b'?',
                            b'\\' => b'|',
                            b'`' => b'~',
                            _ => ascii,
                        };
                    }
                    
                    // Add to buffer
                    let next_end = (self.buffer_end + 1) % self.buffer.len();
                    if next_end != self.buffer_start {
                        self.buffer[self.buffer_end] = ascii;
                        self.buffer_end = next_end;
                    }
                }
            }
        }
    }
    
    /// Update keyboard LEDs
    fn update_leds(&self) -> Result<(), DriverError> {
        let mut leds = 0u8;
        if self.scroll_lock.load(Ordering::Relaxed) {
            leds |= 0x01;
        }
        if self.num_lock.load(Ordering::Relaxed) {
            leds |= 0x02;
        }
        if self.caps_lock.load(Ordering::Relaxed) {
            leds |= 0x04;
        }
        
        self.send_keyboard_command(KBD_CMD_SET_LEDS)?;
        self.wait_input_empty()?;
        unsafe {
            Self::outb(PS2_DATA_PORT, leds);
        }
        Ok(())
    }
}

impl Driver for Ps2KeyboardDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }
    
    fn init(&mut self) -> Result<(), DriverError> {
        // Disable devices
        self.send_controller_command(PS2_CMD_DISABLE_PORT1)?;
        self.send_controller_command(PS2_CMD_DISABLE_PORT2)?;
        
        // Flush output buffer
        unsafe {
            while (Self::inb(PS2_STATUS_PORT) & PS2_STATUS_OUTPUT_FULL) != 0 {
                Self::inb(PS2_DATA_PORT);
            }
        }
        
        // Set controller configuration
        self.send_controller_command(PS2_CMD_READ_CONFIG)?;
        self.wait_output_full()?;
        let mut config = unsafe { Self::inb(PS2_DATA_PORT) };
        
        // Disable IRQs and translation
        config &= !(0x01 | 0x02 | 0x40);
        
        self.send_controller_command(PS2_CMD_WRITE_CONFIG)?;
        self.wait_input_empty()?;
        unsafe {
            Self::outb(PS2_DATA_PORT, config);
        }
        
        // Test controller
        self.send_controller_command(PS2_CMD_TEST_CONTROLLER)?;
        self.wait_output_full()?;
        if unsafe { Self::inb(PS2_DATA_PORT) } != 0x55 {
            return Err(DriverError::InitFailed);
        }
        
        // Enable port 1
        self.send_controller_command(PS2_CMD_ENABLE_PORT1)?;
        
        // Reset keyboard
        self.send_keyboard_command(KBD_CMD_RESET)?;
        self.wait_output_full()?;
        if unsafe { Self::inb(PS2_DATA_PORT) } != KBD_RESP_SELF_TEST_PASSED {
            return Err(DriverError::InitFailed);
        }
        
        // Enable scanning
        self.send_keyboard_command(KBD_CMD_ENABLE_SCANNING)?;
        
        // Enable IRQs
        self.send_controller_command(PS2_CMD_READ_CONFIG)?;
        self.wait_output_full()?;
        config = unsafe { Self::inb(PS2_DATA_PORT) };
        config |= 0x01; // Enable IRQ1
        
        self.send_controller_command(PS2_CMD_WRITE_CONFIG)?;
        self.wait_input_empty()?;
        unsafe {
            Self::outb(PS2_DATA_PORT, config);
        }
        
        self.state = DriverState::Ready;
        Ok(())
    }
    
    fn probe(&self) -> Result<bool, DriverError> {
        // PS/2 controller is always present on x86 systems
        #[cfg(target_arch = "x86_64")]
        {
            Ok(true)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Ok(false)
        }
    }
    
    fn start(&mut self) -> Result<(), DriverError> {
        if self.state != DriverState::Ready {
            self.init()?;
        }
        self.state = DriverState::Ready;
        Ok(())
    }
    
    fn stop(&mut self) -> Result<(), DriverError> {
        self.send_keyboard_command(KBD_CMD_DISABLE_SCANNING)?;
        self.state = DriverState::Stopped;
        Ok(())
    }
    
    fn state(&self) -> DriverState {
        self.state
    }
    
    fn handle_interrupt(&mut self, _irq: u32) -> Result<(), DriverError> {
        // Check if data is available
        unsafe {
            if (Self::inb(PS2_STATUS_PORT) & PS2_STATUS_OUTPUT_FULL) != 0 {
                let scancode = Self::inb(PS2_DATA_PORT);
                self.process_scancode(scancode);
            }
        }
        Ok(())
    }
}

impl CharDriver for Ps2KeyboardDriver {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, DriverError> {
        let mut count = 0;
        
        while count < buffer.len() && self.buffer_start != self.buffer_end {
            buffer[count] = self.buffer[self.buffer_start];
            self.buffer_start = (self.buffer_start + 1) % self.buffer.len();
            count += 1;
        }
        
        Ok(count)
    }
    
    fn write(&mut self, _data: &[u8]) -> Result<usize, DriverError> {
        // Keyboard doesn't support writing
        Err(DriverError::NotSupported)
    }
    
    fn poll(&self) -> Result<bool, DriverError> {
        Ok(self.buffer_start != self.buffer_end)
    }
}

/// Global driver instance
static mut DRIVER: Ps2KeyboardDriver = Ps2KeyboardDriver::new();

/// Driver entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        // Initialize the driver
        match DRIVER.init() {
            Ok(_) => {
                // Driver initialized successfully
                loop {
                    // Poll for keyboard input
                    if (Ps2KeyboardDriver::inb(PS2_STATUS_PORT) & PS2_STATUS_OUTPUT_FULL) != 0 {
                        let scancode = Ps2KeyboardDriver::inb(PS2_DATA_PORT);
                        DRIVER.process_scancode(scancode);
                    }
                }
            }
            Err(_) => {
                // Driver initialization failed
                loop {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
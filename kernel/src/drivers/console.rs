//! Console Device Drivers
//!
//! Implements console drivers for VGA text mode and serial console.

// Console driver provides VGA text mode and serial console. Not all color
// variants and driver methods are exercised yet.
#![allow(dead_code, static_mut_refs)]

use alloc::{boxed::Box, format, string::String, vec, vec::Vec};

use spin::Mutex;

use crate::services::driver_framework::{DeviceClass, DeviceInfo, Driver};

/// Console colors (VGA text mode)
#[allow(dead_code)]
#[repr(u8)]
pub enum ConsoleColor {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

/// Console character with color attributes
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ConsoleChar {
    pub ascii: u8,
    pub color: u8,
}

impl ConsoleChar {
    pub fn new(ascii: u8, foreground: ConsoleColor, background: ConsoleColor) -> Self {
        Self {
            ascii,
            color: ((background as u8) << 4) | (foreground as u8),
        }
    }
}

/// Console device trait
pub trait ConsoleDevice: Send + Sync {
    /// Get console name
    fn name(&self) -> &str;

    /// Get console dimensions
    fn dimensions(&self) -> (usize, usize); // (width, height)

    /// Clear the screen
    fn clear(&mut self) -> Result<(), &'static str>;

    /// Write a character at position
    fn write_char(&mut self, x: usize, y: usize, ch: ConsoleChar) -> Result<(), &'static str>;

    /// Write a string at position
    fn write_string(&mut self, x: usize, y: usize, s: &str, color: u8) -> Result<(), &'static str>;

    /// Scroll up by one line
    fn scroll_up(&mut self) -> Result<(), &'static str>;

    /// Set cursor position
    fn set_cursor(&mut self, x: usize, y: usize) -> Result<(), &'static str>;

    /// Get cursor position
    fn get_cursor(&self) -> (usize, usize);

    /// Show/hide cursor
    fn set_cursor_visible(&mut self, visible: bool) -> Result<(), &'static str>;
}

/// VGA text mode console driver
pub struct VgaConsole {
    buffer: *mut ConsoleChar,
    width: usize,
    height: usize,
    cursor_x: usize,
    cursor_y: usize,
    cursor_visible: bool,
    default_color: u8,
}

// SAFETY: VgaConsole is safe to send between threads as the buffer
// is a fixed hardware address and all methods requiring mutation take &mut self
unsafe impl Send for VgaConsole {}

// SAFETY: VgaConsole is safe to share between threads as the buffer
// is a fixed hardware address and mutation is protected by &mut self
unsafe impl Sync for VgaConsole {}

impl Default for VgaConsole {
    fn default() -> Self {
        Self::new()
    }
}

impl VgaConsole {
    /// Create a new VGA console
    pub fn new() -> Self {
        Self {
            buffer: 0xB8000 as *mut ConsoleChar,
            width: 80,
            height: 25,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            default_color: ((ConsoleColor::Black as u8) << 4) | (ConsoleColor::LightGray as u8),
        }
    }

    /// Get buffer index for position
    fn buffer_index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    /// Update hardware cursor
    fn update_cursor(&self) {
        let pos = self.cursor_y * self.width + self.cursor_x;

        // SAFETY: I/O port writes to the VGA CRT controller (0x3D4/0x3D5)
        // are standard VGA cursor position updates. We are in kernel
        // mode with I/O privileges. These ports are always safe to access.
        unsafe {
            // Cursor low byte
            crate::arch::outb(0x3D4, 0x0F);
            crate::arch::outb(0x3D5, (pos & 0xFF) as u8);

            // Cursor high byte
            crate::arch::outb(0x3D4, 0x0E);
            crate::arch::outb(0x3D5, ((pos >> 8) & 0xFF) as u8);
        }
    }
}

impl ConsoleDevice for VgaConsole {
    fn name(&self) -> &str {
        "vga"
    }

    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn clear(&mut self) -> Result<(), &'static str> {
        let blank = ConsoleChar::new(b' ', ConsoleColor::LightGray, ConsoleColor::Black);

        // SAFETY: self.buffer points to the VGA text buffer at 0xB8000,
        // which is width * height ConsoleChar entries (80*25 = 2000).
        // We write exactly that many entries, staying within bounds.
        unsafe {
            for i in 0..(self.width * self.height) {
                *self.buffer.add(i) = blank;
            }
        }

        self.cursor_x = 0;
        self.cursor_y = 0;
        self.update_cursor();

        Ok(())
    }

    fn write_char(&mut self, x: usize, y: usize, ch: ConsoleChar) -> Result<(), &'static str> {
        if x >= self.width || y >= self.height {
            return Err("Position out of bounds");
        }

        let index = self.buffer_index(x, y);
        // SAFETY: Bounds are checked above (x < width, y < height),
        // so index is within the VGA buffer at 0xB8000.
        unsafe {
            *self.buffer.add(index) = ch;
        }

        Ok(())
    }

    fn write_string(&mut self, x: usize, y: usize, s: &str, color: u8) -> Result<(), &'static str> {
        let mut pos_x = x;
        let pos_y = y;

        if pos_y >= self.height {
            return Err("Y position out of bounds");
        }

        for byte in s.bytes() {
            if pos_x >= self.width {
                break; // Don't wrap lines
            }

            let ch = ConsoleChar { ascii: byte, color };
            self.write_char(pos_x, pos_y, ch)?;
            pos_x += 1;
        }

        Ok(())
    }

    fn scroll_up(&mut self) -> Result<(), &'static str> {
        // SAFETY: All buffer accesses use buffer_index(x, y) where
        // x < self.width and y < self.height. The VGA buffer at
        // 0xB8000 is 80*25 = 2000 ConsoleChar entries. Moving lines
        // and clearing the last line stays within bounds.
        unsafe {
            // Move all lines up by one
            for y in 1..self.height {
                for x in 0..self.width {
                    let src_index = self.buffer_index(x, y);
                    let dst_index = self.buffer_index(x, y - 1);
                    *self.buffer.add(dst_index) = *self.buffer.add(src_index);
                }
            }

            // Clear the last line
            let blank = ConsoleChar::new(b' ', ConsoleColor::LightGray, ConsoleColor::Black);
            for x in 0..self.width {
                let index = self.buffer_index(x, self.height - 1);
                *self.buffer.add(index) = blank;
            }
        }

        Ok(())
    }

    fn set_cursor(&mut self, x: usize, y: usize) -> Result<(), &'static str> {
        if x >= self.width || y >= self.height {
            return Err("Cursor position out of bounds");
        }

        self.cursor_x = x;
        self.cursor_y = y;
        self.update_cursor();

        Ok(())
    }

    fn get_cursor(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }

    fn set_cursor_visible(&mut self, visible: bool) -> Result<(), &'static str> {
        self.cursor_visible = visible;

        // SAFETY: I/O port writes to VGA CRT controller (0x3D4/0x3D5)
        // for cursor shape control. Standard VGA register access.
        unsafe {
            // Set cursor shape
            crate::arch::outb(0x3D4, 0x0A);
            if visible {
                crate::arch::outb(0x3D5, 0x0E); // Cursor on
            } else {
                crate::arch::outb(0x3D5, 0x20); // Cursor off
            }
        }

        Ok(())
    }
}

/// Serial console driver
pub struct SerialConsole {
    port: u16,
    name: String,
    cursor_x: usize,
    cursor_y: usize,
    width: usize,
    height: usize,
}

impl SerialConsole {
    /// Create a new serial console
    pub fn new(port: u16) -> Self {
        let mut console = Self {
            port,
            name: format!(
                "serial{}",
                match port {
                    0x3F8 => 0, // COM1
                    0x2F8 => 1, // COM2
                    0x3E8 => 2, // COM3
                    0x2E8 => 3, // COM4
                    _ => 9,
                }
            ),
            cursor_x: 0,
            cursor_y: 0,
            width: 80,
            height: 25,
        };

        console.init();
        console
    }

    /// Initialize serial port
    fn init(&mut self) {
        // SAFETY: Standard 16550 UART initialization sequence via I/O
        // ports. We are in kernel mode with I/O privileges. The port
        // base address (self.port) is set at construction to a valid
        // COM port (0x3F8, 0x2F8, 0x3E8, or 0x2E8).
        unsafe {
            // Disable interrupts
            crate::arch::outb(self.port + 1, 0x00);

            // Enable DLAB (set baud rate divisor)
            crate::arch::outb(self.port + 3, 0x80);

            // Set divisor to 3 (38400 baud)
            crate::arch::outb(self.port, 0x03);
            crate::arch::outb(self.port + 1, 0x00);

            // 8 bits, no parity, one stop bit
            crate::arch::outb(self.port + 3, 0x03);

            // Enable FIFO, clear them, with 14-byte threshold
            crate::arch::outb(self.port + 2, 0xC7);

            // IRQs enabled, RTS/DSR set
            crate::arch::outb(self.port + 4, 0x0B);
        }
    }

    /// Write a byte to serial port
    fn write_byte(&self, byte: u8) {
        // SAFETY: Reading the line status register (port+5) and writing
        // to the transmit register (port) are standard 16550 UART I/O
        // operations. Kernel mode with I/O privileges.
        unsafe {
            // Wait for transmit holding register empty
            while (crate::arch::inb(self.port + 5) & 0x20) == 0 {
                core::hint::spin_loop();
            }

            crate::arch::outb(self.port, byte);
        }
    }

    /// Read a byte from serial port (non-blocking)
    fn read_byte(&self) -> Option<u8> {
        // SAFETY: Reading line status (port+5) and data register (port)
        // are standard 16550 UART I/O operations.
        unsafe {
            if (crate::arch::inb(self.port + 5) & 0x01) != 0 {
                Some(crate::arch::inb(self.port))
            } else {
                None
            }
        }
    }

    /// Write string to serial port
    fn write_str(&self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r'); // Convert LF to CRLF
            }
            self.write_byte(byte);
        }
    }
}

impl ConsoleDevice for SerialConsole {
    fn name(&self) -> &str {
        &self.name
    }

    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn clear(&mut self) -> Result<(), &'static str> {
        // Send ANSI clear screen sequence
        self.write_str("\x1b[2J\x1b[H");
        self.cursor_x = 0;
        self.cursor_y = 0;
        Ok(())
    }

    fn write_char(&mut self, x: usize, y: usize, ch: ConsoleChar) -> Result<(), &'static str> {
        // Position cursor and write character
        self.write_str(&alloc::format!(
            "\x1b[{};{}H{}",
            y + 1,
            x + 1,
            ch.ascii as char
        ));
        Ok(())
    }

    fn write_string(
        &mut self,
        x: usize,
        y: usize,
        s: &str,
        _color: u8,
    ) -> Result<(), &'static str> {
        // Position cursor and write string
        self.write_str(&alloc::format!("\x1b[{};{}H{}", y + 1, x + 1, s));
        Ok(())
    }

    fn scroll_up(&mut self) -> Result<(), &'static str> {
        // Send ANSI scroll up sequence
        self.write_str("\x1b[S");
        Ok(())
    }

    fn set_cursor(&mut self, x: usize, y: usize) -> Result<(), &'static str> {
        if x >= self.width || y >= self.height {
            return Err("Cursor position out of bounds");
        }

        self.cursor_x = x;
        self.cursor_y = y;

        // Send ANSI cursor position sequence
        self.write_str(&alloc::format!("\x1b[{};{}H", y + 1, x + 1));
        Ok(())
    }

    fn get_cursor(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }

    fn set_cursor_visible(&mut self, visible: bool) -> Result<(), &'static str> {
        if visible {
            self.write_str("\x1b[?25h"); // Show cursor
        } else {
            self.write_str("\x1b[?25l"); // Hide cursor
        }
        Ok(())
    }
}

/// Console driver that manages multiple console devices
pub struct ConsoleDriver {
    devices: Vec<Box<dyn ConsoleDevice>>,
    active_device: usize,
    name: String,
}

impl Default for ConsoleDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleDriver {
    /// Create a new console driver
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            active_device: 0,
            name: String::from("console"),
        }
    }

    /// Add a console device
    pub fn add_device(&mut self, device: Box<dyn ConsoleDevice>) {
        crate::println!("[CONSOLE] Added console device: {}", device.name());
        self.devices.push(device);
    }

    /// Set active console device
    pub fn set_active_device(&mut self, index: usize) -> Result<(), &'static str> {
        if index >= self.devices.len() {
            return Err("Invalid device index");
        }

        self.active_device = index;
        crate::println!(
            "[CONSOLE] Switched to console device: {}",
            self.devices[index].name()
        );
        Ok(())
    }

    /// Get active console device
    pub fn get_active_device(&mut self) -> Option<&mut dyn ConsoleDevice> {
        match self.devices.get_mut(self.active_device) {
            Some(device) => Some(device.as_mut()),
            None => None,
        }
    }

    /// Write to all console devices
    pub fn write_to_all(&mut self, s: &str) {
        for device in &mut self.devices {
            let (x, y) = device.get_cursor();
            device.write_string(x, y, s, 0x07).ok(); // Light gray on black
        }
    }
}

impl Driver for ConsoleDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_classes(&self) -> Vec<DeviceClass> {
        vec![DeviceClass::Display, DeviceClass::Serial]
    }

    fn supports_device(&self, device: &DeviceInfo) -> bool {
        matches!(device.class, DeviceClass::Display | DeviceClass::Serial)
    }

    fn probe(&mut self, _device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[CONSOLE] Probing device: {}", _device.name);
        Ok(())
    }

    fn attach(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[CONSOLE] Attaching to device: {}", device.name);

        match device.class {
            DeviceClass::Display => {
                // Add VGA console
                let vga_console = VgaConsole::new();
                self.add_device(Box::new(vga_console));
            }
            DeviceClass::Serial => {
                // Add serial console (COM1 by default)
                let serial_console = SerialConsole::new(0x3F8);
                self.add_device(Box::new(serial_console));
            }
            _ => return Err("Unsupported device class"),
        }

        Ok(())
    }

    fn detach(&mut self, _device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[CONSOLE] Detaching from device: {}", _device.name);
        // TODO(phase4): Remove specific console device from device list
        Ok(())
    }

    fn suspend(&mut self) -> Result<(), &'static str> {
        crate::println!("[CONSOLE] Suspending console driver");
        Ok(())
    }

    fn resume(&mut self) -> Result<(), &'static str> {
        crate::println!("[CONSOLE] Resuming console driver");
        Ok(())
    }

    fn handle_interrupt(&mut self, _irq: u8) -> Result<(), &'static str> {
        crate::println!("[CONSOLE] Handling interrupt {} for console", _irq);
        Ok(())
    }

    fn read(&mut self, _offset: u64, _buffer: &mut [u8]) -> Result<usize, &'static str> {
        // TODO(phase4): Read input from console keyboard driver
        Ok(0)
    }

    fn write(&mut self, _offset: u64, data: &[u8]) -> Result<usize, &'static str> {
        if let Ok(s) = core::str::from_utf8(data) {
            self.write_to_all(s);
            Ok(data.len())
        } else {
            Err("Invalid UTF-8 data")
        }
    }

    fn ioctl(&mut self, cmd: u32, arg: u64) -> Result<u64, &'static str> {
        match cmd {
            0x2000 => {
                // Get active device index
                Ok(self.active_device as u64)
            }
            0x2001 => {
                // Set active device index
                self.set_active_device(arg as usize)?;
                Ok(0)
            }
            0x2002 => {
                // Get device count
                Ok(self.devices.len() as u64)
            }
            0x2003 => {
                // Clear screen
                if let Some(device) = self.get_active_device() {
                    device.clear()?;
                }
                Ok(0)
            }
            _ => Err("Unknown ioctl command"),
        }
    }
}

/// Global console driver instance
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
static CONSOLE_DRIVER: spin::Once<Mutex<ConsoleDriver>> = spin::Once::new();

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static mut CONSOLE_DRIVER_STATIC: Option<Mutex<ConsoleDriver>> = None;

/// Initialize console subsystem
pub fn init() {
    let mut console_driver = ConsoleDriver::new();

    // Add VGA console
    let vga_console = VgaConsole::new();
    console_driver.add_device(Box::new(vga_console));

    // Add serial console (COM1)
    let serial_console = SerialConsole::new(0x3F8);
    console_driver.add_device(Box::new(serial_console));

    // Initialize VGA console
    if let Some(device) = console_driver.get_active_device() {
        device.clear().ok();
        device.set_cursor_visible(true).ok();
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        CONSOLE_DRIVER.call_once(|| Mutex::new(console_driver));
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    // SAFETY: Called once during early init before concurrent access.
    // CONSOLE_DRIVER_STATIC is only written here and read later.
    unsafe {
        CONSOLE_DRIVER_STATIC = Some(Mutex::new(console_driver));
    }

    // Register with driver framework
    let driver_framework = crate::services::driver_framework::get_driver_framework();
    let console_instance = ConsoleDriver::new();

    if let Err(_e) = driver_framework.register_driver(Box::new(console_instance)) {
        crate::println!("[CONSOLE] Failed to register console driver: {}", _e);
    } else {
        crate::println!("[CONSOLE] Console subsystem initialized");
    }
}

/// Get the global console driver
pub fn get_console_driver() -> &'static Mutex<ConsoleDriver> {
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        CONSOLE_DRIVER
            .get()
            .expect("Console driver not initialized")
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    // SAFETY: CONSOLE_DRIVER_STATIC was set during init(). Once set,
    // it is never modified. The Option is always Some after init.
    unsafe {
        CONSOLE_DRIVER_STATIC
            .as_ref()
            .expect("Console driver not initialized")
    }
}

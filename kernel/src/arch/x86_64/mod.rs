pub mod boot;
pub mod gdt;
pub mod idt;
pub mod serial;
pub mod vga;

use super::Architecture;

pub fn init() {
    gdt::init();
    idt::init();
    unsafe { interrupts::enable() };
}

pub fn halt() -> ! {
    use x86_64::instructions::hlt;
    interrupts::disable();
    loop {
        hlt();
    }
}

pub fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}

pub fn disable_interrupts() {
    x86_64::instructions::interrupts::disable();
}

pub fn idle() {
    x86_64::instructions::hlt();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::arch::x86_64::vga::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[cfg(test)]
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::arch::x86_64::serial::_print(format_args!($($arg)*)));
}

#[cfg(test)]
#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}

mod interrupts {
    pub unsafe fn enable() {
        x86_64::instructions::interrupts::enable();
    }
    
    pub fn disable() {
        x86_64::instructions::interrupts::disable();
    }
}
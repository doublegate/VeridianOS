//! Hello World program for VeridianOS

#![no_std]
#![no_main]

use libveridian::{println, sys};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("Hello, VeridianOS!");
    println!("This is a user-space program running in Phase 2!");
    
    match sys::getpid() {
        Ok(pid) => println!("My PID is: {}", pid),
        Err(_) => println!("Failed to get PID"),
    }
    
    sys::exit(0);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    sys::exit(255);
}
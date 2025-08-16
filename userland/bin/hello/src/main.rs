//! Hello World program for VeridianOS

#![no_std]
#![no_main]

use libveridian::{println, getpid, exit};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    libveridian::init();
    main();
    exit(0);
}

fn main() {
    println!("Hello, VeridianOS!");
    
    match getpid() {
        Ok(pid) => println!("My process ID is: {}", pid),
        Err(_) => println!("Failed to get process ID"),
    }
    
    println!("This is a user-space program running on VeridianOS.");
    println!("The kernel has successfully loaded and executed this ELF binary!");
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    libveridian::panic_handler_impl(info)
}
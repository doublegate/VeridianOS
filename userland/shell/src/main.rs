//! Veridian Shell (vsh) - The VeridianOS command shell
//!
//! A simple shell for VeridianOS that provides command-line interaction.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::str;
use libveridian::{print, println, sys};

const MAX_COMMAND_LENGTH: usize = 256;
const STDIN: usize = 0;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    main();
    sys::exit(0);
}

fn main() {
    println!("VeridianOS Shell v0.3.0");
    println!("Type 'help' for available commands");
    
    let mut command_buffer = [0u8; MAX_COMMAND_LENGTH];
    
    loop {
        print!("vsh> ");
        
        match read_line(&mut command_buffer) {
            Ok(line) => {
                if !line.is_empty() {
                    execute_command(line);
                }
            }
            Err(_) => {
                println!("Error reading input");
            }
        }
    }
}

fn read_line(buffer: &mut [u8]) -> Result<&str, ()> {
    let mut pos = 0;
    
    loop {
        let mut char_buf = [0u8; 1];
        match sys::read(STDIN, &mut char_buf) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let ch = char_buf[0];
                
                if ch == b'\n' || ch == b'\r' {
                    buffer[pos] = 0;
                    println!(); // New line after input
                    return str::from_utf8(&buffer[..pos]).map_err(|_| ());
                } else if ch == 0x7F || ch == 0x08 { // Backspace
                    if pos > 0 {
                        pos -= 1;
                        print!("\x08 \x08"); // Move back, space, move back
                    }
                } else if ch >= 0x20 && ch < 0x7F { // Printable ASCII
                    if pos < buffer.len() - 1 {
                        buffer[pos] = ch;
                        pos += 1;
                        print!("{}", ch as char);
                    }
                }
            }
            Err(_) => return Err(()),
        }
    }
    
    str::from_utf8(&buffer[..pos]).map_err(|_| ())
}

fn execute_command(command: &str) {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }
    
    match parts[0] {
        "help" => show_help(),
        "echo" => {
            if parts.len() > 1 {
                for (i, part) in parts[1..].iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    print!("{}", part);
                }
                println!();
            }
        }
        "pid" => {
            match sys::getpid() {
                Ok(pid) => println!("Current PID: {}", pid),
                Err(_) => println!("Failed to get PID"),
            }
        }
        "clear" => {
            // ANSI escape sequence to clear screen
            print!("\x1b[2J\x1b[H");
        }
        "exit" => {
            println!("Goodbye!");
            sys::exit(0);
        }
        "exec" => {
            if parts.len() > 1 {
                exec_program(parts[1], &parts[2..]);
            } else {
                println!("Usage: exec <program> [args...]");
            }
        }
        "fork" => {
            test_fork();
        }
        _ => {
            // Try to execute as external program
            exec_program(parts[0], &parts[1..]);
        }
    }
}

fn show_help() {
    println!("Available commands:");
    println!("  help    - Show this help message");
    println!("  echo    - Echo arguments to screen");
    println!("  pid     - Show current process ID");
    println!("  clear   - Clear the screen");
    println!("  exit    - Exit the shell");
    println!("  exec    - Execute a program");
    println!("  fork    - Test fork system call");
}

fn exec_program(program: &str, args: &[&str]) {
    match sys::fork() {
        Ok(0) => {
            // Child process
            match sys::exec(program, args) {
                Ok(_) => unreachable!(),
                Err(e) => {
                    println!("Failed to execute '{}': {:?}", program, e);
                    sys::exit(1);
                }
            }
        }
        Ok(pid) => {
            // Parent process - wait for child
            match sys::wait() {
                Ok((child_pid, status)) => {
                    if child_pid == pid && status != 0 {
                        println!("Process {} exited with status {}", pid, status);
                    }
                }
                Err(_) => {
                    println!("Failed to wait for child process");
                }
            }
        }
        Err(e) => {
            println!("Failed to fork: {:?}", e);
        }
    }
}

fn test_fork() {
    println!("Testing fork system call...");
    
    match sys::fork() {
        Ok(0) => {
            // Child process
            println!("Hello from child process!");
            match sys::getpid() {
                Ok(pid) => println!("Child PID: {}", pid),
                Err(_) => {}
            }
            sys::exit(0);
        }
        Ok(pid) => {
            // Parent process
            println!("Parent: Created child with PID {}", pid);
            match sys::wait() {
                Ok((child_pid, status)) => {
                    println!("Parent: Child {} exited with status {}", child_pid, status);
                }
                Err(_) => {
                    println!("Parent: Failed to wait for child");
                }
            }
        }
        Err(e) => {
            println!("Fork failed: {:?}", e);
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[VSH] PANIC: {}", info);
    sys::exit(255);
}

// Simple allocator for the shell
use core::alloc::{GlobalAlloc, Layout};

struct DummyAllocator;

unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut() // Temporary - will be replaced with real allocator
    }
    
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    panic!("Allocation error");
}
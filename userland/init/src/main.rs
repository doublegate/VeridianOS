//! Init process - the first user-space process
//! 
//! This is the init process for VeridianOS. It's the first user-space process
//! started by the kernel and is responsible for bootstrapping the system.

#![no_std]
#![no_main]

use libveridian::{println, sys};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    main();
    sys::exit(0);
}

fn main() {
    println!("[INIT] VeridianOS Init Process v0.3.0");
    println!("[INIT] System initialization starting...");
    
    // Phase 1: Basic system setup
    println!("[INIT] Phase 1: Basic system setup");
    setup_system_directories();
    
    // Phase 2: Start essential services
    println!("[INIT] Phase 2: Starting essential services");
    start_essential_services();
    
    // Phase 3: Start shell
    println!("[INIT] Phase 3: Starting shell");
    start_shell();
    
    // Init process should never exit, but if all children exit, we halt
    println!("[INIT] All services started, entering monitor mode");
    monitor_loop();
}

fn setup_system_directories() {
    println!("[INIT] Setting up system directories...");
    // TODO: Create /dev, /proc, /sys when we have a filesystem
    println!("[INIT] System directories ready");
}

fn start_essential_services() {
    println!("[INIT] Starting essential services...");
    
    // TODO: Start device manager
    // TODO: Start network stack
    // TODO: Start other essential services
    
    println!("[INIT] Essential services started");
}

fn start_shell() {
    println!("[INIT] Starting shell...");
    
    match sys::fork() {
        Ok(0) => {
            // Child process - exec the shell
            println!("[INIT] Child process: executing shell");
            match sys::exec("/bin/vsh", &[]) {
                Ok(_) => unreachable!(),
                Err(e) => {
                    println!("[INIT] Failed to exec shell: {:?}", e);
                    sys::exit(1);
                }
            }
        }
        Ok(pid) => {
            println!("[INIT] Started shell with PID {}", pid);
        }
        Err(e) => {
            println!("[INIT] Failed to fork for shell: {:?}", e);
        }
    }
}

fn monitor_loop() -> ! {
    loop {
        // Wait for any child process to exit
        match sys::wait() {
            Ok((pid, status)) => {
                println!("[INIT] Process {} exited with status {}", pid, status);
                
                // If the shell exits, restart it
                if status != 0 {
                    println!("[INIT] Restarting shell...");
                    start_shell();
                }
            }
            Err(_) => {
                // No children to wait for, just sleep
                sys::sleep(1000);
            }
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[INIT] PANIC: {}", info);
    sys::exit(255);
}
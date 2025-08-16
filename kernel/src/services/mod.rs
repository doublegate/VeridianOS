//! System services module
//!
//! Provides core system services including process management, 
//! driver framework, and system daemons.

pub mod process_server;
pub mod driver_framework;
pub mod init_system;
pub mod shell;

pub use process_server::ProcessServer;
pub use driver_framework::DriverFramework;
pub use init_system::InitSystem;
pub use shell::Shell;

/// Initialize all system services
pub fn init() {
    use crate::println;
    println!("[SERVICES] Initializing system services...");
    
    // Initialize process server
    println!("[SERVICES] About to initialize process server...");
    process_server::init();
    println!("[SERVICES] Process server initialized");
    
    // Initialize driver framework
    println!("[SERVICES] About to initialize driver framework...");
    driver_framework::init();
    println!("[SERVICES] Driver framework initialized");
    
    // Initialize init system
    println!("[SERVICES] About to initialize init system...");
    init_system::init();
    println!("[SERVICES] Init system initialized");
    
    // Initialize thread management APIs
    println!("[SERVICES] About to initialize thread management...");
    crate::thread_api::init();
    println!("[SERVICES] Thread management initialized");
    
    // Initialize standard library
    println!("[SERVICES] About to initialize standard library...");
    crate::stdlib::init();
    println!("[SERVICES] Standard library initialized");
    
    // Initialize network subsystem
    println!("[SERVICES] About to initialize network subsystem...");
    crate::drivers::network::init();
    println!("[SERVICES] Network subsystem initialized");
    
    println!("[SERVICES] System services initialized");
}
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
    process_server::init();
    
    // Initialize driver framework
    driver_framework::init();
    
    // Initialize init system
    init_system::init();
    
    println!("[SERVICES] System services initialized");
}
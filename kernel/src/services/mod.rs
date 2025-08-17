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
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[SERVICES] Initializing system services (AArch64)...\n");
        uart_write_str("[SERVICES] About to initialize process server...\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[SERVICES] Initializing system services...");
        println!("[SERVICES] About to initialize process server...");
    }
    
    // Initialize process server
    process_server::init();
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[SERVICES] Process server initialized\n");
        uart_write_str("[SERVICES] About to initialize driver framework...\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[SERVICES] Process server initialized");
        println!("[SERVICES] About to initialize driver framework...");
    }
    
    // Initialize driver framework
    driver_framework::init();
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[SERVICES] Driver framework initialized\n");
        uart_write_str("[SERVICES] About to initialize init system...\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[SERVICES] Driver framework initialized");
        println!("[SERVICES] About to initialize init system...");
    }
    
    // Initialize init system
    init_system::init();
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[SERVICES] Init system initialized\n");
        uart_write_str("[SERVICES] About to initialize thread management...\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[SERVICES] Init system initialized");
        println!("[SERVICES] About to initialize thread management...");
    }
    
    // Initialize thread management APIs
    crate::thread_api::init();
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[SERVICES] Thread management initialized\n");
        uart_write_str("[SERVICES] About to initialize standard library...\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[SERVICES] Thread management initialized");
        println!("[SERVICES] About to initialize standard library...");
    }
    
    // Initialize standard library
    crate::stdlib::init();
    
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[SERVICES] Standard library initialized\n");
        uart_write_str("[SERVICES] System services initialized\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[SERVICES] Standard library initialized");
        println!("[SERVICES] System services initialized");
    }
    
    // NOTE: Network initialization removed - was causing kernel hang
    // The network subsystem should be initialized lazily when needed
}
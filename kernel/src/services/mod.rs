//! System services module
//!
//! Provides core system services including process management,
//! driver framework, and system daemons.

pub mod desktop_ipc;
pub mod driver_framework;
pub mod init_system;
pub mod notification_ipc;
pub mod process_server;
pub mod shell;
pub mod shell_utils;

pub use driver_framework::DriverFramework;
pub use init_system::InitSystem;
pub use process_server::ProcessServer;
pub use shell::Shell;

/// Initialize all system services
pub fn init() {
    kprintln!("[SERVICES] Initializing system services...");

    kprintln!("[SERVICES] Initializing process server...");
    process_server::init();
    kprintln!("[SERVICES] Process server initialized");

    kprintln!("[SERVICES] Initializing driver framework...");
    driver_framework::init();
    kprintln!("[SERVICES] Driver framework initialized");

    kprintln!("[SERVICES] Initializing init system...");
    init_system::init();
    kprintln!("[SERVICES] Init system initialized");

    kprintln!("[SERVICES] Initializing thread management...");
    crate::thread_api::init();
    kprintln!("[SERVICES] Thread management initialized");

    kprintln!("[SERVICES] Initializing standard library...");
    crate::stdlib::init();
    kprintln!("[SERVICES] Standard library initialized");

    kprintln!("[SERVICES] Initializing shell...");
    shell::init();
    kprintln!("[SERVICES] Shell initialized");

    kprintln!("[SERVICES] System services initialized");

    // NOTE: Network initialization removed - was causing kernel hang
    // The network subsystem should be initialized lazily when needed
}

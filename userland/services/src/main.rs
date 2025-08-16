//! Service Manager for VeridianOS
//!
//! Manages system services and daemons.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::String;
use libveridian::{println, print, fork, exec, wait, exit, sleep, getpid};

/// Service configuration
struct Service {
    name: &'static str,
    path: &'static str,
    args: &'static [&'static str],
    auto_restart: bool,
    pid: Option<usize>,
    state: ServiceState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

/// Service manager state
struct ServiceManager {
    services: Vec<Service>,
}

impl ServiceManager {
    /// Create a new service manager
    fn new() -> Self {
        Self {
            services: Vec::new(),
        }
    }
    
    /// Register a service
    fn register_service(
        &mut self,
        name: &'static str,
        path: &'static str,
        args: &'static [&'static str],
        auto_restart: bool,
    ) {
        self.services.push(Service {
            name,
            path,
            args,
            auto_restart,
            pid: None,
            state: ServiceState::Stopped,
        });
    }
    
    /// Start a service
    fn start_service(&mut self, name: &str) -> Result<(), &'static str> {
        let service = self.services.iter_mut()
            .find(|s| s.name == name)
            .ok_or("Service not found")?;
        
        if service.state == ServiceState::Running {
            return Ok(());
        }
        
        println!("[SERVICE] Starting service: {}", name);
        service.state = ServiceState::Starting;
        
        match fork() {
            Ok(0) => {
                // Child process - exec the service
                match exec(service.path, service.args) {
                    Ok(_) => unreachable!(),
                    Err(_) => {
                        println!("[SERVICE] Failed to exec service: {}", name);
                        exit(1);
                    }
                }
            }
            Ok(pid) => {
                // Parent process
                service.pid = Some(pid);
                service.state = ServiceState::Running;
                println!("[SERVICE] Started service {} with PID {}", name, pid);
                Ok(())
            }
            Err(_) => {
                service.state = ServiceState::Failed;
                println!("[SERVICE] Failed to fork for service: {}", name);
                Err("Fork failed")
            }
        }
    }
    
    /// Stop a service
    fn stop_service(&mut self, name: &str) -> Result<(), &'static str> {
        let service = self.services.iter_mut()
            .find(|s| s.name == name)
            .ok_or("Service not found")?;
        
        if service.state != ServiceState::Running {
            return Ok(());
        }
        
        println!("[SERVICE] Stopping service: {}", name);
        service.state = ServiceState::Stopping;
        
        // TODO: Send SIGTERM to service process
        // For now, just mark as stopped
        service.state = ServiceState::Stopped;
        service.pid = None;
        
        Ok(())
    }
    
    /// Monitor services and restart if needed
    fn monitor_services(&mut self) {
        // Check for exited services
        while let Ok((pid, status)) = wait() {
            // Find the service with this PID
            if let Some(service) = self.services.iter_mut()
                .find(|s| s.pid == Some(pid))
            {
                println!("[SERVICE] Service {} (PID {}) exited with status {}", 
                         service.name, pid, status);
                
                service.pid = None;
                service.state = if status == 0 {
                    ServiceState::Stopped
                } else {
                    ServiceState::Failed
                };
                
                // Auto-restart if configured
                if service.auto_restart && service.state == ServiceState::Failed {
                    println!("[SERVICE] Auto-restarting service: {}", service.name);
                    sleep(1000).ok(); // Wait 1 second before restart
                    self.start_service(service.name).ok();
                }
            }
        }
    }
    
    /// List all services
    fn list_services(&self) {
        println!("Service Status:");
        println!("----------------");
        for service in &self.services {
            let state_str = match service.state {
                ServiceState::Stopped => "STOPPED",
                ServiceState::Starting => "STARTING",
                ServiceState::Running => "RUNNING",
                ServiceState::Stopping => "STOPPING",
                ServiceState::Failed => "FAILED",
            };
            
            if let Some(pid) = service.pid {
                println!("{:20} {:10} PID: {}", service.name, state_str, pid);
            } else {
                println!("{:20} {:10}", service.name, state_str);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    libveridian::init();
    main();
    exit(0);
}

fn main() {
    println!("[SERVICE] VeridianOS Service Manager v0.1.0");
    println!("[SERVICE] Initializing service management...");
    
    let mut manager = ServiceManager::new();
    
    // Register core services
    manager.register_service(
        "device-manager",
        "/sbin/devmgr",
        &[],
        true,
    );
    
    manager.register_service(
        "network-stack",
        "/sbin/netd",
        &[],
        true,
    );
    
    manager.register_service(
        "filesystem-daemon",
        "/sbin/fsd",
        &[],
        true,
    );
    
    // Start essential services
    println!("[SERVICE] Starting essential services...");
    
    // Start device manager first
    if manager.start_service("device-manager").is_err() {
        println!("[SERVICE] Warning: Failed to start device manager");
    }
    
    // Give device manager time to initialize
    sleep(100).ok();
    
    // Start other services
    if manager.start_service("network-stack").is_err() {
        println!("[SERVICE] Warning: Failed to start network stack");
    }
    
    if manager.start_service("filesystem-daemon").is_err() {
        println!("[SERVICE] Warning: Failed to start filesystem daemon");
    }
    
    println!("[SERVICE] Service initialization complete");
    manager.list_services();
    
    // Main service monitoring loop
    println!("[SERVICE] Entering service monitoring loop...");
    loop {
        // Monitor services
        manager.monitor_services();
        
        // Sleep for a bit
        sleep(1000).ok();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[SERVICE] PANIC: {}", info);
    exit(255);
}

// Simple allocator for the service manager
use core::alloc::{GlobalAlloc, Layout};

struct ServiceAllocator;

static mut HEAP: [u8; 65536] = [0; 65536];
static mut HEAP_POS: usize = 0;

unsafe impl GlobalAlloc for ServiceAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        
        // Align heap position
        let aligned_pos = (HEAP_POS + align - 1) & !(align - 1);
        
        // Check if we have enough space
        if aligned_pos + size > HEAP.len() {
            return core::ptr::null_mut();
        }
        
        // Allocate memory
        let ptr = HEAP.as_mut_ptr().add(aligned_pos);
        HEAP_POS = aligned_pos + size;
        
        ptr
    }
    
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Simple bump allocator - no deallocation
    }
}

#[global_allocator]
static ALLOCATOR: ServiceAllocator = ServiceAllocator;

#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    panic!("Service manager allocation error");
}
//! Init System Implementation
//!
//! The init process (PID 1) that starts all system services and manages the
//! system lifecycle.

#![allow(clippy::if_same_then_else)]

use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use spin::RwLock;

use crate::process::ProcessId;

/// Service state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
    Restarting,
}

/// Service restart policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,         // Never restart
    OnFailure,     // Restart only on failure
    Always,        // Always restart when service stops
    UnlessStopped, // Restart unless explicitly stopped
}

/// Service dependency type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyType {
    Requires, // Service must be running
    Wants,    // Service should be running if possible
    After,    // Start after this service
    Before,   // Start before this service
}

/// Service definition
#[derive(Debug, Clone)]
pub struct ServiceDefinition {
    pub name: String,
    pub description: String,
    pub command: String,
    pub arguments: Vec<String>,
    pub environment: Vec<String>,
    pub working_directory: String,
    pub user: u32,
    pub group: u32,
    pub restart_policy: RestartPolicy,
    pub restart_delay_ms: u32,
    pub max_restarts: u32,
    pub timeout_ms: u32,
    pub dependencies: Vec<(String, DependencyType)>,
    pub start_level: u32, // 0-99, lower starts first
}

/// Service runtime information
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub definition: ServiceDefinition,
    pub state: ServiceState,
    pub pid: Option<ProcessId>,
    pub start_time: u64,
    pub restart_count: u32,
    pub exit_code: Option<i32>,
    pub last_error: Option<String>,
}

/// Runlevel definition
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Runlevel {
    Halt = 0,      // System halt
    Single = 1,    // Single user mode
    Multi = 2,     // Multi-user without networking
    Network = 3,   // Multi-user with networking
    Reserved = 4,  // Reserved
    Graphical = 5, // Multi-user with networking and GUI
    Reboot = 6,    // System reboot
}

/// Init system manager
pub struct InitSystem {
    /// Registered services
    services: RwLock<BTreeMap<String, ServiceInfo>>,

    /// Current runlevel
    current_runlevel: AtomicU32,

    /// Target runlevel
    target_runlevel: AtomicU32,

    /// System is shutting down
    shutting_down: AtomicBool,

    /// Init process PID
    init_pid: AtomicU32,

    /// Service start order (computed from dependencies)
    start_order: RwLock<Vec<String>>,

    /// Service monitoring thread running
    monitoring_active: AtomicBool,
}

impl InitSystem {
    /// Create a new init system
    pub fn new() -> Self {
        Self {
            services: RwLock::new(BTreeMap::new()),
            current_runlevel: AtomicU32::new(Runlevel::Halt as u32),
            target_runlevel: AtomicU32::new(Runlevel::Multi as u32),
            shutting_down: AtomicBool::new(false),
            init_pid: AtomicU32::new(1),
            start_order: RwLock::new(Vec::new()),
            monitoring_active: AtomicBool::new(false),
        }
    }
}

impl Default for InitSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl InitSystem {
    /// Initialize the init system
    pub fn initialize(&self) -> Result<(), &'static str> {
        crate::println!("[INIT] Initializing init system...");

        // Register core system services
        self.register_core_services()?;

        // Compute service start order
        self.compute_start_order()?;

        // Start service monitoring
        self.monitoring_active.store(true, Ordering::SeqCst);

        // Switch to runlevel 1 (single user)
        self.switch_runlevel(Runlevel::Single)?;

        crate::println!("[INIT] Init system initialized");
        Ok(())
    }

    /// Register a service
    pub fn register_service(&self, definition: ServiceDefinition) -> Result<(), &'static str> {
        let name = definition.name.clone();

        if self.services.read().contains_key(&name) {
            return Err("Service already registered");
        }

        let info = ServiceInfo {
            definition,
            state: ServiceState::Stopped,
            pid: None,
            start_time: 0,
            restart_count: 0,
            exit_code: None,
            last_error: None,
        };

        self.services.write().insert(name.clone(), info);
        crate::println!("[INIT] Registered service: {}", name);

        // Recompute start order
        self.compute_start_order()?;

        Ok(())
    }

    /// Start a service
    pub fn start_service(&self, name: &str) -> Result<(), &'static str> {
        let mut services = self.services.write();

        let service = services.get_mut(name).ok_or("Service not found")?;

        if service.state == ServiceState::Running {
            return Ok(()); // Already running
        }

        crate::println!("[INIT] Starting service: {}", name);
        service.state = ServiceState::Starting;

        // Check dependencies
        drop(services); // Release lock for dependency check
        self.check_dependencies(name)?;
        let mut services = self.services.write();
        let service = services
            .get_mut(name)
            .expect("service disappeared between dependency check and start");

        // Create process for service
        let process_server = crate::services::process_server::get_process_server();

        let pid = process_server.create_process(
            ProcessId(self.init_pid.load(Ordering::SeqCst) as u64),
            service.definition.command.clone(),
            service.definition.user,
            service.definition.group,
            {
                let mut args = vec![service.definition.command.clone()];
                args.extend(service.definition.arguments.clone());
                args
            },
            service.definition.environment.clone(),
        )?;

        service.pid = Some(pid);
        service.state = ServiceState::Running;
        service.start_time = self.get_system_time();
        service.last_error = None;

        crate::println!("[INIT] Service {} started with PID {}", name, pid.0);
        Ok(())
    }

    /// Stop a service
    pub fn stop_service(&self, name: &str) -> Result<(), &'static str> {
        let mut services = self.services.write();

        let service = services.get_mut(name).ok_or("Service not found")?;

        if service.state != ServiceState::Running {
            return Ok(()); // Not running
        }

        crate::println!("[INIT] Stopping service: {}", name);
        service.state = ServiceState::Stopping;

        if let Some(pid) = service.pid {
            let process_server = crate::services::process_server::get_process_server();

            // Send SIGTERM
            process_server.send_signal(pid, 15)?;

            // TODO(phase3): Wait for process to exit with configurable timeout
            service.state = ServiceState::Stopped;
            service.pid = None;

            crate::println!("[INIT] Service {} stopped", name);
        }

        Ok(())
    }

    /// Restart a service
    pub fn restart_service(&self, name: &str) -> Result<(), &'static str> {
        self.stop_service(name)?;
        self.start_service(name)?;
        Ok(())
    }

    /// Get service status
    pub fn get_service_status(&self, name: &str) -> Option<ServiceInfo> {
        self.services.read().get(name).cloned()
    }

    /// List all services
    pub fn list_services(&self) -> Vec<ServiceInfo> {
        self.services.read().values().cloned().collect()
    }

    /// Switch runlevel
    pub fn switch_runlevel(&self, runlevel: Runlevel) -> Result<(), &'static str> {
        let current = self.current_runlevel.load(Ordering::SeqCst);

        if current == runlevel as u32 {
            return Ok(()); // Already at target runlevel
        }

        crate::println!(
            "[INIT] Switching from runlevel {} to {}",
            current,
            runlevel as u32
        );
        self.target_runlevel
            .store(runlevel as u32, Ordering::SeqCst);

        // Stop services not needed in new runlevel
        let services_to_stop =
            self.get_services_for_runlevel_transition(current, runlevel as u32, false);

        for service_name in services_to_stop {
            self.stop_service(&service_name).ok();
        }

        // Start services needed in new runlevel
        let services_to_start =
            self.get_services_for_runlevel_transition(current, runlevel as u32, true);

        for service_name in services_to_start {
            self.start_service(&service_name).ok();
        }

        self.current_runlevel
            .store(runlevel as u32, Ordering::SeqCst);
        crate::println!("[INIT] Switched to runlevel {}", runlevel as u32);

        Ok(())
    }

    /// Handle service exit
    pub fn handle_service_exit(&self, pid: ProcessId, exit_code: i32) {
        let mut services = self.services.write();

        // Find service by PID
        for service in services.values_mut() {
            if service.pid == Some(pid) {
                crate::println!(
                    "[INIT] Service {} exited with code {}",
                    service.definition.name,
                    exit_code
                );

                service.state = ServiceState::Stopped;
                service.pid = None;
                service.exit_code = Some(exit_code);

                // Check restart policy
                let should_restart = match service.definition.restart_policy {
                    RestartPolicy::Never => false,
                    RestartPolicy::OnFailure => exit_code != 0,
                    RestartPolicy::Always => !self.shutting_down.load(Ordering::SeqCst),
                    RestartPolicy::UnlessStopped => {
                        !self.shutting_down.load(Ordering::SeqCst)
                            && service.state != ServiceState::Stopped
                    }
                };

                if should_restart && service.restart_count < service.definition.max_restarts {
                    service.state = ServiceState::Restarting;
                    service.restart_count += 1;
                    crate::println!(
                        "[INIT] Scheduling restart for service {} (attempt {})",
                        service.definition.name,
                        service.restart_count
                    );
                    // TODO(phase3): Schedule restart with configurable back-off
                    // delay
                } else if service.restart_count >= service.definition.max_restarts {
                    service.state = ServiceState::Failed;
                    service.last_error = Some(String::from("Max restart attempts exceeded"));
                }

                break;
            }
        }
    }

    /// Shutdown the system
    pub fn shutdown(&self) -> Result<(), &'static str> {
        crate::println!("[INIT] System shutdown initiated");
        self.shutting_down.store(true, Ordering::SeqCst);

        // Stop monitoring
        self.monitoring_active.store(false, Ordering::SeqCst);

        // Switch to runlevel 0 (halt)
        self.switch_runlevel(Runlevel::Halt)?;

        // Stop all remaining services in reverse order
        let services: Vec<String> = self.start_order.read().iter().rev().cloned().collect();
        for service_name in services {
            self.stop_service(&service_name).ok();
        }

        crate::println!("[INIT] System shutdown complete");
        Ok(())
    }

    /// Reboot the system
    pub fn reboot(&self) -> Result<(), &'static str> {
        crate::println!("[INIT] System reboot initiated");

        // Shutdown first
        self.shutdown()?;

        // Switch to runlevel 6 (reboot)
        self.switch_runlevel(Runlevel::Reboot)?;

        // TODO(phase3): Trigger actual system reboot via architecture-specific
        // mechanism
        crate::println!("[INIT] System rebooting...");

        Ok(())
    }

    // Helper functions

    fn register_core_services(&self) -> Result<(), &'static str> {
        // Register console service
        self.register_service(ServiceDefinition {
            name: String::from("console"),
            description: String::from("System Console"),
            command: String::from("/sbin/getty"),
            arguments: vec![String::from("tty0")],
            environment: vec![],
            working_directory: String::from("/"),
            user: 0,
            group: 0,
            restart_policy: RestartPolicy::Always,
            restart_delay_ms: 1000,
            max_restarts: 5,
            timeout_ms: 5000,
            dependencies: vec![],
            start_level: 10,
        })?;

        // Register logger service
        self.register_service(ServiceDefinition {
            name: String::from("logger"),
            description: String::from("System Logger"),
            command: String::from("/sbin/syslogd"),
            arguments: vec![],
            environment: vec![],
            working_directory: String::from("/"),
            user: 0,
            group: 0,
            restart_policy: RestartPolicy::Always,
            restart_delay_ms: 1000,
            max_restarts: 5,
            timeout_ms: 5000,
            dependencies: vec![],
            start_level: 5,
        })?;

        // Register device manager
        self.register_service(ServiceDefinition {
            name: String::from("devmgr"),
            description: String::from("Device Manager"),
            command: String::from("/sbin/devmgr"),
            arguments: vec![],
            environment: vec![],
            working_directory: String::from("/"),
            user: 0,
            group: 0,
            restart_policy: RestartPolicy::OnFailure,
            restart_delay_ms: 2000,
            max_restarts: 3,
            timeout_ms: 10000,
            dependencies: vec![(String::from("logger"), DependencyType::After)],
            start_level: 15,
        })?;

        // Register network service
        self.register_service(ServiceDefinition {
            name: String::from("network"),
            description: String::from("Network Service"),
            command: String::from("/sbin/netd"),
            arguments: vec![],
            environment: vec![],
            working_directory: String::from("/"),
            user: 0,
            group: 0,
            restart_policy: RestartPolicy::OnFailure,
            restart_delay_ms: 3000,
            max_restarts: 3,
            timeout_ms: 30000,
            dependencies: vec![
                (String::from("devmgr"), DependencyType::Requires),
                (String::from("logger"), DependencyType::After),
            ],
            start_level: 20,
        })?;

        Ok(())
    }

    fn compute_start_order(&self) -> Result<(), &'static str> {
        let services = self.services.read();
        let mut order = Vec::new();
        let mut visited = BTreeMap::new();

        // Topological sort based on dependencies and start levels
        let mut sorted_services: Vec<_> = services.keys().cloned().collect();
        sorted_services.sort_by_key(|name| {
            services
                .get(name)
                .map(|s| s.definition.start_level)
                .unwrap_or(99)
        });

        for service_name in sorted_services {
            if !visited.contains_key(&service_name) {
                self.visit_service(&service_name, &services, &mut visited, &mut order)?;
            }
        }

        *self.start_order.write() = order;
        Ok(())
    }

    fn visit_service(
        &self,
        name: &str,
        services: &BTreeMap<String, ServiceInfo>,
        visited: &mut BTreeMap<String, bool>,
        order: &mut Vec<String>,
    ) -> Result<(), &'static str> {
        if let Some(&in_progress) = visited.get(name) {
            if in_progress {
                return Err("Circular dependency detected");
            }
            return Ok(()); // Already visited
        }

        visited.insert(name.into(), true);

        if let Some(service) = services.get(name) {
            // Visit dependencies first
            for (dep_name, dep_type) in &service.definition.dependencies {
                if matches!(dep_type, DependencyType::Requires | DependencyType::After) {
                    self.visit_service(dep_name, services, visited, order)?;
                }
            }
        }

        visited.insert(name.into(), false);
        order.push(name.into());
        Ok(())
    }

    fn check_dependencies(&self, name: &str) -> Result<(), &'static str> {
        let services = self.services.read();

        if let Some(service) = services.get(name) {
            for (dep_name, dep_type) in &service.definition.dependencies {
                if *dep_type == DependencyType::Requires {
                    if let Some(dep_service) = services.get(dep_name) {
                        if dep_service.state != ServiceState::Running {
                            return Err("Required dependency not running");
                        }
                    } else {
                        return Err("Required dependency not found");
                    }
                }
            }
        }

        Ok(())
    }

    fn get_services_for_runlevel_transition(
        &self,
        _from: u32,
        to: u32,
        start: bool,
    ) -> Vec<String> {
        let services = self.services.read();
        let mut result = Vec::new();

        for (name, service) in services.iter() {
            let should_run = match to {
                0 => false,                                // Halt - stop everything
                1 => service.definition.start_level <= 10, // Single user - basic services
                2 => service.definition.start_level <= 30, // Multi without network
                3 => service.definition.start_level <= 50, // Multi with network
                5 => true,                                 // Graphical - everything
                6 => false,                                // Reboot - stop everything
                _ => service.definition.start_level <= 30,
            };

            if start && should_run && service.state != ServiceState::Running {
                result.push(name.clone());
            } else if !start && !should_run && service.state == ServiceState::Running {
                result.push(name.clone());
            }
        }

        if start {
            // Sort by start order
            let order = self.start_order.read();
            result.sort_by_key(|name| order.iter().position(|n| n == name).unwrap_or(999));
        } else {
            // Reverse order for stopping
            result.reverse();
        }

        result
    }

    fn get_system_time(&self) -> u64 {
        // TODO(phase3): Get actual system time from clock subsystem
        0
    }
}

/// Global init system using OnceLock for safe initialization.
static INIT_SYSTEM: crate::sync::once_lock::OnceLock<InitSystem> =
    crate::sync::once_lock::OnceLock::new();

/// Initialize the init system
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[INIT] Creating new InitSystem...");
    match INIT_SYSTEM.set(InitSystem::new()) {
        Ok(()) => println!("[INIT] Init system module loaded"),
        Err(_) => println!("[INIT] Already initialized, skipping..."),
    }
}

/// Try to get the global init system without panicking.
///
/// Returns `None` if the init system has not been initialized via [`init`].
pub fn try_get_init_system() -> Option<&'static InitSystem> {
    INIT_SYSTEM.get()
}

/// Get the global init system.
///
/// Panics if the init system has not been initialized via [`init`].
/// Prefer [`try_get_init_system`] in contexts where a panic is unacceptable.
pub fn get_init_system() -> &'static InitSystem {
    INIT_SYSTEM
        .get()
        .expect("Init system not initialized: init() was not called")
}

/// Run the init process
pub fn run_init() -> ! {
    let init_system = get_init_system();

    // Initialize the system
    init_system
        .initialize()
        .expect("Failed to initialize init system");

    // Switch to multi-user mode
    init_system
        .switch_runlevel(Runlevel::Multi)
        .expect("Failed to switch to multi-user mode");

    // Main init loop - wait for child processes and handle events
    loop {
        // Check for exited children
        let process_server = crate::services::process_server::get_process_server();

        // Try to reap any zombie children
        while let Ok((child_pid, exit_code)) = process_server.wait_for_child(ProcessId(1), None) {
            init_system.handle_service_exit(child_pid, exit_code);
        }

        // Clean up any orphaned zombies
        process_server.reap_zombies();

        // Sleep for a bit
        // TODO(phase3): Use proper wait/signal mechanism instead of spin loop
        for _ in 0..1000000 {
            core::hint::spin_loop();
        }
    }
}

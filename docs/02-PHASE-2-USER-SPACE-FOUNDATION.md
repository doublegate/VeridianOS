# Phase 2: User Space Foundation (Months 10-15)

## Overview

Phase 2 establishes the user space environment, creating essential system services, user libraries, and the foundational components needed for a functional operating system. This phase transforms the microkernel into a usable system by implementing init, device drivers, file systems, and basic utilities.

## Objectives

1. **Init System**: Process 1 and service management
2. **Device Driver Framework**: User-space driver infrastructure
3. **Virtual File System**: Unified file system interface
4. **Network Stack**: Basic TCP/IP implementation
5. **Standard Library**: POSIX-compatible C library (Rust-based for safety)
6. **Basic Utilities**: Shell and core system tools

## POSIX Compatibility Strategy

**Three-Layer Architecture** (AI Recommendation):
1. **POSIX API Layer**: Standard POSIX functions (open, read, write, etc.)
2. **Translation Layer**: Convert POSIX semantics to capabilities
3. **Native IPC Layer**: VeridianOS zero-copy IPC

**Implementation Priority**:
1. Memory allocation (malloc/free)
2. Basic I/O (open/read/write/close)
3. Process management (spawn, not fork initially)
4. Threading (pthreads)
5. Networking (BSD sockets)

**libc Choice**: Port musl libc with VeridianOS syscall backend
- Musl is MIT-licensed and standards-compliant
- Implement `src/internal/syscall_arch.h` for VeridianOS
- Replace Linux-specific code with capability-based operations

## Implementation Details

### Process Creation Model

VeridianOS uses **spawn** instead of fork for security:
```c
// Instead of fork/exec pattern:
pid_t pid = fork();
if (pid == 0) {
    execve(path, argv, envp);
}

// VeridianOS pattern:
pid_t pid;
posix_spawn(&pid, path, NULL, NULL, argv, envp);
```

### File Descriptor Translation

Each POSIX fd maps to a capability:
```
POSIX Layer: fd (int) → Translation Table → VeridianOS: capability_t
```

### Signal Handling

Signals implemented via user-space daemon:
- Kernel sends IPC to signal daemon
- Daemon injects signal handlers into target process
- Async-signal-safe functions use capability-protected shared memory

## Architecture Components

### 1. Init System

#### 1.1 Init Process

**init/src/main.rs**
```rust
#![no_std]
#![no_main]

extern crate alloc;
use alloc::{vec::Vec, string::String, collections::BTreeMap};
use veridian_abi::{syscall, Process, Capability};

/// Service definition
struct Service {
    name: String,
    path: String,
    args: Vec<String>,
    dependencies: Vec<String>,
    restart_policy: RestartPolicy,
    capabilities: Vec<Capability>,
    state: ServiceState,
    pid: Option<Pid>,
}

#[derive(Clone, Copy, PartialEq)]
enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Copy)]
enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

/// Service manager
struct ServiceManager {
    services: BTreeMap<String, Service>,
    start_order: Vec<String>,
    ipc_endpoint: EndpointId,
}

impl ServiceManager {
    fn new() -> Result<Self, Error> {
        let endpoint = syscall::endpoint_create(4096)?;
        
        Ok(Self {
            services: BTreeMap::new(),
            start_order: Vec::new(),
            ipc_endpoint: endpoint,
        })
    }
    
    /// Load service configuration
    fn load_services(&mut self) -> Result<(), Error> {
        // Parse /etc/init/services.toml
        let config = self.read_config("/etc/init/services.toml")?;
        
        for service_config in config.services {
            let service = Service {
                name: service_config.name.clone(),
                path: service_config.path,
                args: service_config.args,
                dependencies: service_config.depends_on,
                restart_policy: service_config.restart_policy.into(),
                capabilities: self.parse_capabilities(&service_config.capabilities)?,
                state: ServiceState::Stopped,
                pid: None,
            };
            
            self.services.insert(service_config.name, service);
        }
        
        // Topological sort for start order
        self.start_order = self.topological_sort()?;
        
        Ok(())
    }
    
    /// Start all services in order
    fn start_all(&mut self) -> Result<(), Error> {
        for service_name in &self.start_order.clone() {
            self.start_service(service_name)?;
        }
        Ok(())
    }
    
    /// Start a single service
    fn start_service(&mut self, name: &str) -> Result<(), Error> {
        let service = self.services.get_mut(name)
            .ok_or(Error::ServiceNotFound)?;
            
        if service.state == ServiceState::Running {
            return Ok(());
        }
        
        // Check dependencies
        for dep in &service.dependencies.clone() {
            if let Some(dep_service) = self.services.get(dep) {
                if dep_service.state != ServiceState::Running {
                    return Err(Error::DependencyNotRunning);
                }
            }
        }
        
        service.state = ServiceState::Starting;
        
        // Create process with capabilities
        let pid = syscall::process_create(&service.path)?;
        
        // Grant capabilities
        for cap in &service.capabilities {
            syscall::capability_grant(pid, cap.clone())?;
        }
        
        // Start process
        syscall::process_start(pid, &service.args)?;
        
        service.pid = Some(pid);
        service.state = ServiceState::Running;
        
        println!("[init] Started service: {}", name);
        
        Ok(())
    }
    
    /// Monitor services and handle restarts
    fn monitor_services(&mut self) -> ! {
        loop {
            // Wait for process exit or IPC message
            match syscall::wait_event() {
                Event::ProcessExit { pid, status } => {
                    self.handle_process_exit(pid, status);
                }
                Event::IpcMessage { sender, message } => {
                    self.handle_control_message(sender, message);
                }
                _ => {}
            }
        }
    }
    
    fn handle_process_exit(&mut self, pid: Pid, status: i32) {
        // Find service by PID
        for (name, service) in &mut self.services {
            if service.pid == Some(pid) {
                println!("[init] Service {} exited with status {}", name, status);
                
                service.state = ServiceState::Failed;
                service.pid = None;
                
                // Handle restart policy
                match service.restart_policy {
                    RestartPolicy::Always => {
                        println!("[init] Restarting service {}", name);
                        let _ = self.start_service(name);
                    }
                    RestartPolicy::OnFailure if status != 0 => {
                        println!("[init] Restarting failed service {}", name);
                        let _ = self.start_service(name);
                    }
                    _ => {}
                }
                
                break;
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Initialize heap
    veridian_abi::heap::init();
    
    println!("VeridianOS Init v{}", env!("CARGO_PKG_VERSION"));
    
    // Create service manager
    let mut manager = ServiceManager::new()
        .expect("Failed to create service manager");
    
    // Load service configuration
    manager.load_services()
        .expect("Failed to load services");
    
    // Mount initial filesystems
    mount_initial_filesystems()
        .expect("Failed to mount filesystems");
    
    // Start all services
    manager.start_all()
        .expect("Failed to start services");
    
    // Monitor services
    manager.monitor_services();
}

fn mount_initial_filesystems() -> Result<(), Error> {
    // Mount devfs
    syscall::mount("devfs", "/dev", "devfs", 0)?;
    
    // Mount procfs
    syscall::mount("procfs", "/proc", "procfs", 0)?;
    
    // Mount tmpfs for /tmp
    syscall::mount("tmpfs", "/tmp", "tmpfs", 0)?;
    
    Ok(())
}
```

#### 1.2 Service Configuration

**init/etc/services.toml**
```toml
# VeridianOS Service Configuration

[[services]]
name = "devmgr"
path = "/sbin/devmgr"
args = []
restart_policy = "always"
capabilities = ["CAP_DEVICE_MANAGE", "CAP_MEMORY_MAP"]

[[services]]
name = "vfs"
path = "/sbin/vfs"
args = []
restart_policy = "always"
capabilities = ["CAP_FS_MOUNT", "CAP_IPC_CREATE"]

[[services]]
name = "netstack"
path = "/sbin/netstack"
args = ["--config=/etc/network.conf"]
depends_on = ["devmgr"]
restart_policy = "always"
capabilities = ["CAP_NET_ADMIN", "CAP_NET_RAW"]

[[services]]
name = "logger"
path = "/sbin/logger"
args = ["--output=/var/log/system.log"]
restart_policy = "always"
capabilities = ["CAP_FS_WRITE"]

[[services]]
name = "shell"
path = "/bin/vsh"
args = []
depends_on = ["vfs", "devmgr"]
restart_policy = "on_failure"
capabilities = ["CAP_PROCESS_CREATE", "CAP_FS_ALL"]
```

### 2. Device Driver Framework

#### 2.1 Driver Manager

**drivers/devmgr/src/main.rs**
```rust
use alloc::collections::BTreeMap;
use veridian_driver::{Driver, DeviceId, DeviceClass};

/// Device manager service
struct DeviceManager {
    /// Registered drivers
    drivers: BTreeMap<DeviceClass, Vec<DriverInfo>>,
    /// Active devices
    devices: BTreeMap<DeviceId, DeviceInfo>,
    /// Driver processes
    driver_processes: BTreeMap<Pid, DriverHandle>,
    /// IPC endpoint for driver communication
    endpoint: EndpointId,
}

struct DriverInfo {
    name: String,
    path: String,
    supported_devices: Vec<DeviceMatch>,
    capabilities_required: Vec<Capability>,
}

struct DeviceInfo {
    id: DeviceId,
    class: DeviceClass,
    vendor_id: u16,
    device_id: u16,
    driver: Option<Pid>,
    resources: Vec<Resource>,
}

#[derive(Clone)]
enum Resource {
    Memory { base: PhysAddr, size: usize },
    Io { base: u16, size: u16 },
    Interrupt { vector: u8 },
    Dma { channel: u8 },
}

impl DeviceManager {
    /// Scan for devices and load drivers
    pub fn scan_and_load(&mut self) -> Result<(), Error> {
        // Scan PCI bus
        self.scan_pci()?;
        
        // Scan platform devices
        self.scan_platform()?;
        
        // Match devices with drivers
        self.match_and_load_drivers()?;
        
        Ok(())
    }
    
    fn scan_pci(&mut self) -> Result<(), Error> {
        // Access PCI configuration space
        let pci_cap = self.get_capability("CAP_PCI_CONFIG")?;
        
        for bus in 0..256 {
            for device in 0..32 {
                for function in 0..8 {
                    let vendor_id = pci_read_u16(pci_cap, bus, device, function, 0x00)?;
                    if vendor_id == 0xFFFF {
                        continue;
                    }
                    
                    let device_id = pci_read_u16(pci_cap, bus, device, function, 0x02)?;
                    let class_code = pci_read_u8(pci_cap, bus, device, function, 0x0B)?;
                    
                    let dev_info = DeviceInfo {
                        id: DeviceId::new(),
                        class: DeviceClass::from_pci_class(class_code),
                        vendor_id,
                        device_id,
                        driver: None,
                        resources: self.probe_pci_resources(bus, device, function)?,
                    };
                    
                    self.devices.insert(dev_info.id, dev_info);
                }
            }
        }
        
        Ok(())
    }
    
    fn match_and_load_drivers(&mut self) -> Result<(), Error> {
        for (dev_id, device) in &self.devices.clone() {
            if device.driver.is_some() {
                continue; // Already has driver
            }
            
            // Find matching driver
            if let Some(drivers) = self.drivers.get(&device.class) {
                for driver_info in drivers {
                    if driver_info.matches(device) {
                        self.load_driver_for_device(*dev_id, driver_info.clone())?;
                        break;
                    }
                }
            }
        }
        
        Ok(())
    }
    
    fn load_driver_for_device(
        &mut self,
        device_id: DeviceId,
        driver_info: DriverInfo,
    ) -> Result<(), Error> {
        let device = self.devices.get_mut(&device_id)
            .ok_or(Error::DeviceNotFound)?;
        
        // Create driver process
        let pid = syscall::process_create(&driver_info.path)?;
        
        // Grant required capabilities
        for cap in &driver_info.capabilities_required {
            syscall::capability_grant(pid, cap.clone())?;
        }
        
        // Grant device resources
        for resource in &device.resources {
            match resource {
                Resource::Memory { base, size } => {
                    let cap = syscall::capability_create_memory(*base, *size)?;
                    syscall::capability_grant(pid, cap)?;
                }
                Resource::Interrupt { vector } => {
                    let cap = syscall::capability_create_interrupt(*vector)?;
                    syscall::capability_grant(pid, cap)?;
                }
                _ => {}
            }
        }
        
        // Send device info to driver
        let init_msg = DriverInitMessage {
            device_id,
            vendor_id: device.vendor_id,
            device_id: device.device_id,
            resources: device.resources.clone(),
        };
        
        syscall::ipc_send(self.endpoint, &init_msg)?;
        
        // Start driver
        syscall::process_start(pid, &[&device_id.to_string()])?;
        
        device.driver = Some(pid);
        
        self.driver_processes.insert(pid, DriverHandle {
            name: driver_info.name,
            device_id,
        });
        
        println!("[devmgr] Loaded driver {} for device {:?}", 
                 driver_info.name, device_id);
        
        Ok(())
    }
}
```

#### 2.2 Driver Library

**libs/veridian-driver/src/lib.rs**
```rust
#![no_std]

use veridian_abi::{syscall, Capability, EndpointId};

/// Driver trait that all drivers must implement
pub trait Driver {
    /// Initialize the driver with device info
    fn init(&mut self, device: DeviceInfo) -> Result<(), Error>;
    
    /// Handle interrupt
    fn handle_interrupt(&mut self, vector: u8);
    
    /// Handle control message
    fn handle_message(&mut self, msg: Message) -> Result<Response, Error>;
    
    /// Cleanup on shutdown
    fn cleanup(&mut self);
}

/// Driver framework
pub struct DriverFramework<D: Driver> {
    driver: D,
    device_id: DeviceId,
    control_endpoint: EndpointId,
    interrupt_caps: Vec<Capability>,
}

impl<D: Driver> DriverFramework<D> {
    /// Run the driver main loop
    pub fn run(mut driver: D) -> ! {
        // Receive initialization message from devmgr
        let init_msg: DriverInitMessage = syscall::ipc_receive()
            .expect("Failed to receive init message");
        
        // Initialize driver
        driver.init(init_msg.into())
            .expect("Driver initialization failed");
        
        // Create control endpoint
        let control_endpoint = syscall::endpoint_create(4096)
            .expect("Failed to create endpoint");
        
        // Register interrupt handlers
        let mut interrupt_caps = Vec::new();
        for resource in init_msg.resources {
            if let Resource::Interrupt { vector } = resource {
                let cap = syscall::capability_for_interrupt(vector)
                    .expect("Failed to get interrupt capability");
                syscall::interrupt_register(vector, cap)
                    .expect("Failed to register interrupt");
                interrupt_caps.push(cap);
            }
        }
        
        let mut framework = Self {
            driver,
            device_id: init_msg.device_id,
            control_endpoint,
            interrupt_caps,
        };
        
        // Main event loop
        framework.event_loop();
    }
    
    fn event_loop(&mut self) -> ! {
        loop {
            match syscall::wait_event() {
                Event::Interrupt { vector } => {
                    self.driver.handle_interrupt(vector);
                }
                Event::IpcMessage { sender, message } => {
                    match self.driver.handle_message(message) {
                        Ok(response) => {
                            let _ = syscall::ipc_reply(sender, response);
                        }
                        Err(e) => {
                            let _ = syscall::ipc_reply_error(sender, e);
                        }
                    }
                }
                Event::Shutdown => {
                    self.driver.cleanup();
                    syscall::process_exit(0);
                }
                _ => {}
            }
        }
    }
}

/// Helper macros for drivers
#[macro_export]
macro_rules! driver_main {
    ($driver_type:ty) => {
        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            // Initialize heap
            veridian_abi::heap::init();
            
            // Create driver instance
            let driver = <$driver_type>::new();
            
            // Run driver framework
            veridian_driver::DriverFramework::run(driver);
        }
    };
}
```

### 3. Virtual File System

#### 3.1 VFS Service

**services/vfs/src/main.rs**
```rust
use alloc::collections::BTreeMap;
use veridian_abi::{Path, FileHandle, OpenFlags};

/// Virtual file system node
struct VNode {
    /// Unique node ID
    id: VNodeId,
    /// Node type
    node_type: VNodeType,
    /// Parent node
    parent: Option<VNodeId>,
    /// Children (for directories)
    children: BTreeMap<String, VNodeId>,
    /// File system this node belongs to
    fs: Option<FsId>,
    /// File system specific data
    fs_data: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum VNodeType {
    Directory,
    RegularFile,
    SymbolicLink,
    Device,
    Pipe,
    Socket,
}

/// Mounted file system
struct MountedFs {
    /// File system ID
    id: FsId,
    /// File system type
    fs_type: String,
    /// Root vnode
    root: VNodeId,
    /// IPC endpoint to FS driver
    endpoint: EndpointId,
    /// Mount flags
    flags: MountFlags,
}

/// VFS service
struct VirtualFileSystem {
    /// All vnodes
    vnodes: BTreeMap<VNodeId, VNode>,
    /// Mounted file systems
    filesystems: BTreeMap<FsId, MountedFs>,
    /// Mount points (vnode -> fs)
    mount_points: BTreeMap<VNodeId, FsId>,
    /// Open files
    open_files: BTreeMap<FileHandle, OpenFile>,
    /// Process file descriptors
    process_fds: BTreeMap<Pid, Vec<FileHandle>>,
    /// Next IDs
    next_vnode_id: u64,
    next_fs_id: u64,
    next_handle: u64,
}

impl VirtualFileSystem {
    /// Mount a file system
    pub fn mount(
        &mut self,
        fs_type: &str,
        device: Option<&str>,
        mount_point: &Path,
        flags: MountFlags,
    ) -> Result<(), Error> {
        // Resolve mount point
        let mount_vnode = self.path_lookup(mount_point)?;
        
        // Check if already mounted
        if self.mount_points.contains_key(&mount_vnode) {
            return Err(Error::AlreadyMounted);
        }
        
        // Find file system driver
        let fs_endpoint = self.find_fs_driver(fs_type)?;
        
        // Send mount request to driver
        let mount_req = FsMountRequest {
            device: device.map(String::from),
            flags,
        };
        
        let mount_resp: FsMountResponse = 
            syscall::ipc_call(fs_endpoint, &mount_req)?;
        
        // Create root vnode for mounted FS
        let root_vnode = self.create_vnode(
            VNodeType::Directory,
            None,
            Some(mount_resp.root_id),
        );
        
        // Record mount
        let fs_id = FsId(self.next_fs_id);
        self.next_fs_id += 1;
        
        self.filesystems.insert(fs_id, MountedFs {
            id: fs_id,
            fs_type: fs_type.to_string(),
            root: root_vnode,
            endpoint: fs_endpoint,
            flags,
        });
        
        self.mount_points.insert(mount_vnode, fs_id);
        
        Ok(())
    }
    
    /// Open a file
    pub fn open(
        &mut self,
        process: Pid,
        path: &Path,
        flags: OpenFlags,
        mode: Mode,
    ) -> Result<FileHandle, Error> {
        // Resolve path
        let vnode = if flags.contains(OpenFlags::CREATE) {
            self.create_file(path, mode)?
        } else {
            self.path_lookup(path)?
        };
        
        // Check permissions
        self.check_access(process, vnode, flags)?;
        
        // Get file system for this vnode
        let fs = self.get_fs_for_vnode(vnode)?;
        
        // Open in file system
        let fs_handle = if let Some(fs) = fs {
            let open_req = FsOpenRequest {
                node_id: self.vnodes[&vnode].fs_data,
                flags,
                mode,
            };
            
            let open_resp: FsOpenResponse = 
                syscall::ipc_call(fs.endpoint, &open_req)?;
            
            Some(open_resp.handle)
        } else {
            None // Virtual file
        };
        
        // Create file handle
        let handle = FileHandle(self.next_handle);
        self.next_handle += 1;
        
        self.open_files.insert(handle, OpenFile {
            vnode,
            flags,
            offset: 0,
            fs_handle,
        });
        
        // Track for process
        self.process_fds.entry(process)
            .or_insert_with(Vec::new)
            .push(handle);
        
        Ok(handle)
    }
    
    /// Read from file
    pub fn read(
        &mut self,
        process: Pid,
        handle: FileHandle,
        buffer: &mut [u8],
    ) -> Result<usize, Error> {
        let open_file = self.open_files.get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        
        // Check if process owns this handle
        if !self.process_owns_handle(process, handle) {
            return Err(Error::PermissionDenied);
        }
        
        // Get file system
        let fs = self.get_fs_for_vnode(open_file.vnode)?;
        
        if let Some(fs) = fs {
            // Forward to file system
            let read_req = FsReadRequest {
                handle: open_file.fs_handle.unwrap(),
                offset: open_file.offset,
                size: buffer.len(),
            };
            
            let read_resp: FsReadResponse = 
                syscall::ipc_call(fs.endpoint, &read_req)?;
            
            // Copy data
            buffer[..read_resp.data.len()].copy_from_slice(&read_resp.data);
            open_file.offset += read_resp.data.len() as u64;
            
            Ok(read_resp.data.len())
        } else {
            // Handle virtual files (like /proc)
            self.read_virtual(open_file.vnode, open_file.offset, buffer)
        }
    }
    
    /// Path lookup
    fn path_lookup(&self, path: &Path) -> Result<VNodeId, Error> {
        let components = path.components();
        let mut current = self.root_vnode();
        
        for component in components {
            match component {
                Component::RootDir => current = self.root_vnode(),
                Component::CurDir => {} // Stay at current
                Component::ParentDir => {
                    if let Some(vnode) = self.vnodes.get(&current) {
                        current = vnode.parent.unwrap_or(current);
                    }
                }
                Component::Normal(name) => {
                    let vnode = self.vnodes.get(&current)
                        .ok_or(Error::NotFound)?;
                        
                    // Check if mount point
                    if let Some(fs_id) = self.mount_points.get(&current) {
                        let fs = &self.filesystems[fs_id];
                        current = fs.root;
                    }
                    
                    // Look up child
                    current = *vnode.children.get(name)
                        .ok_or(Error::NotFound)?;
                }
            }
        }
        
        Ok(current)
    }
}

/// File handle operations
struct OpenFile {
    vnode: VNodeId,
    flags: OpenFlags,
    offset: u64,
    fs_handle: Option<u64>,
}
```

### 4. Network Stack

#### 4.1 Network Service

**services/netstack/src/main.rs**
```rust
use smoltcp::iface::{Interface, InterfaceBuilder};
use smoltcp::wire::{IpCidr, Ipv4Address};
use smoltcp::socket::{TcpSocket, UdpSocket};

/// Network stack service
struct NetworkStack {
    /// Network interfaces
    interfaces: Vec<NetworkInterface>,
    /// TCP connections
    tcp_sockets: Slab<TcpSocket>,
    /// UDP sockets
    udp_sockets: Slab<UdpSocket>,
    /// Routing table
    routes: RoutingTable,
    /// ARP cache
    arp_cache: ArpCache,
    /// Configuration
    config: NetworkConfig,
}

struct NetworkInterface {
    name: String,
    device: Box<dyn Device>,
    interface: Interface,
    addresses: Vec<IpCidr>,
}

impl NetworkStack {
    /// Initialize network stack
    pub fn init(config: NetworkConfig) -> Result<Self, Error> {
        let mut stack = Self {
            interfaces: Vec::new(),
            tcp_sockets: Slab::new(),
            udp_sockets: Slab::new(),
            routes: RoutingTable::new(),
            arp_cache: ArpCache::new(),
            config,
        };
        
        // Initialize interfaces from config
        for iface_config in &config.interfaces {
            stack.add_interface(iface_config)?;
        }
        
        // Set up default routes
        for route in &config.routes {
            stack.routes.add_route(route.clone())?;
        }
        
        Ok(stack)
    }
    
    /// Add network interface
    fn add_interface(&mut self, config: &InterfaceConfig) -> Result<(), Error> {
        // Get device from driver
        let device = self.get_network_device(&config.device)?;
        
        // Create smoltcp interface
        let interface = InterfaceBuilder::new(device.clone())
            .ip_addrs(&config.addresses)
            .finalize();
        
        self.interfaces.push(NetworkInterface {
            name: config.name.clone(),
            device,
            interface,
            addresses: config.addresses.clone(),
        });
        
        Ok(())
    }
    
    /// Main network processing loop
    pub fn run(&mut self) -> ! {
        let mut last_poll = Instant::now();
        
        loop {
            // Process each interface
            for iface in &mut self.interfaces {
                // Receive packets
                while let Some(packet) = iface.device.receive() {
                    iface.interface.process_packet(packet);
                }
                
                // Process sockets
                iface.interface.poll(&mut self.tcp_sockets);
                iface.interface.poll(&mut self.udp_sockets);
                
                // Transmit packets
                while let Some(packet) = iface.interface.transmit() {
                    iface.device.transmit(packet);
                }
            }
            
            // Handle IPC requests
            if let Ok(msg) = syscall::ipc_receive_nonblock(self.endpoint) {
                self.handle_request(msg);
            }
            
            // Rate limit polling
            let now = Instant::now();
            if now - last_poll < Duration::from_millis(10) {
                syscall::sleep(10 - (now - last_poll).as_millis());
            }
            last_poll = now;
        }
    }
    
    /// Handle socket operations
    fn handle_request(&mut self, msg: Message) {
        match msg.request {
            NetRequest::TcpConnect { addr, port } => {
                let socket_id = self.tcp_connect(addr, port);
                msg.reply(NetResponse::SocketId(socket_id));
            }
            NetRequest::TcpListen { port } => {
                let socket_id = self.tcp_listen(port);
                msg.reply(NetResponse::SocketId(socket_id));
            }
            NetRequest::Send { socket_id, data } => {
                let result = self.send_data(socket_id, data);
                msg.reply(NetResponse::Result(result));
            }
            NetRequest::Receive { socket_id, max_len } => {
                let data = self.receive_data(socket_id, max_len);
                msg.reply(NetResponse::Data(data));
            }
            _ => {}
        }
    }
    
    /// Create TCP connection
    fn tcp_connect(&mut self, addr: IpAddress, port: u16) -> Result<SocketId, Error> {
        let socket = TcpSocket::new(
            TcpSocketBuffer::new(vec![0; 65536]),
            TcpSocketBuffer::new(vec![0; 65536])
        );
        
        let socket_id = self.tcp_sockets.insert(socket);
        
        // Initiate connection
        self.tcp_sockets[socket_id].connect(addr, port)?;
        
        Ok(SocketId::Tcp(socket_id))
    }
}

/// Network device trait
trait Device: Send {
    fn receive(&mut self) -> Option<&[u8]>;
    fn transmit(&mut self, packet: &[u8]) -> Result<(), Error>;
    fn capabilities(&self) -> DeviceCapabilities;
}
```

### 5. Standard Library

#### 5.1 Core Library

**libs/libveridian/src/lib.rs**
```rust
#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod syscall;
pub mod process;
pub mod memory;
pub mod fs;
pub mod net;
pub mod time;
pub mod thread;
pub mod sync;

use core::panic::PanicInfo;
use core::alloc::{GlobalAlloc, Layout};

/// System call wrappers
pub mod syscall {
    use crate::sys;
    
    /// Exit current process
    pub fn exit(code: i32) -> ! {
        unsafe {
            sys::syscall1(sys::SYS_EXIT, code as usize);
        }
        unreachable!()
    }
    
    /// Create new process
    pub fn fork() -> Result<Pid, Error> {
        let ret = unsafe {
            sys::syscall0(sys::SYS_FORK)
        };
        
        if ret == 0 {
            Ok(Pid::child())
        } else if ret > 0 {
            Ok(Pid(ret as u32))
        } else {
            Err(Error::from_errno(-ret as i32))
        }
    }
    
    /// Execute program
    pub fn exec(path: &str, args: &[&str]) -> Result<!, Error> {
        let ret = unsafe {
            sys::syscall3(
                sys::SYS_EXEC,
                path.as_ptr() as usize,
                path.len(),
                args.as_ptr() as usize,
            )
        };
        
        Err(Error::from_errno(-ret as i32))
    }
    
    /// Memory map
    pub fn mmap(
        addr: Option<*mut u8>,
        len: usize,
        prot: Protection,
        flags: MapFlags,
    ) -> Result<*mut u8, Error> {
        let ret = unsafe {
            sys::syscall4(
                sys::SYS_MMAP,
                addr.unwrap_or(core::ptr::null_mut()) as usize,
                len,
                prot.bits() as usize,
                flags.bits() as usize,
            )
        };
        
        if ret < KERNEL_BASE {
            Ok(ret as *mut u8)
        } else {
            Err(Error::from_errno(-ret as i32))
        }
    }
}

/// File operations
pub mod fs {
    use super::*;
    
    pub struct File {
        fd: FileDescriptor,
    }
    
    impl File {
        /// Open file
        pub fn open(path: &str, flags: OpenFlags) -> Result<Self, Error> {
            let fd = syscall::open(path, flags, 0o666)?;
            Ok(Self { fd })
        }
        
        /// Read from file
        pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            syscall::read(self.fd, buf)
        }
        
        /// Write to file
        pub fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
            syscall::write(self.fd, buf)
        }
        
        /// Seek in file
        pub fn seek(&mut self, pos: SeekFrom) -> Result<u64, Error> {
            syscall::lseek(self.fd, pos)
        }
    }
    
    impl Drop for File {
        fn drop(&mut self) {
            let _ = syscall::close(self.fd);
        }
    }
}

/// Memory allocator
struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        
        // Round up to alignment
        let size = (size + align - 1) & !(align - 1);
        
        match syscall::mmap(None, size, Protection::READ | Protection::WRITE, MapFlags::PRIVATE) {
            Ok(ptr) => ptr,
            Err(_) => core::ptr::null_mut(),
        }
    }
    
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size();
        let align = layout.align();
        let size = (size + align - 1) & !(align - 1);
        
        let _ = syscall::munmap(ptr, size);
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    eprintln!("PANIC: {}", info);
    syscall::exit(1);
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!("Allocation error: {:?}", layout);
}
```

### 6. Basic Shell

#### 6.1 Veridian Shell (vsh)

**userland/vsh/src/main.rs**
```rust
use std::io::{self, Write};
use std::process::Command;
use std::env;
use std::path::Path;

/// Shell state
struct Shell {
    /// Current working directory
    cwd: String,
    /// Environment variables
    env: BTreeMap<String, String>,
    /// Command history
    history: Vec<String>,
    /// Exit code of last command
    last_exit: i32,
}

impl Shell {
    fn new() -> Self {
        let cwd = env::current_dir()
            .unwrap_or_else(|_| Path::new("/").to_path_buf())
            .to_string_lossy()
            .into_owned();
            
        let mut env = BTreeMap::new();
        for (key, value) in env::vars() {
            env.insert(key, value);
        }
        
        Self {
            cwd,
            env,
            history: Vec::new(),
            last_exit: 0,
        }
    }
    
    /// Run shell main loop
    fn run(&mut self) {
        println!("VeridianOS Shell v{}", env!("CARGO_PKG_VERSION"));
        println!("Type 'help' for available commands");
        
        loop {
            // Print prompt
            print!("{}> ", self.cwd);
            io::stdout().flush().unwrap();
            
            // Read command
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }
            
            let input = input.trim();
            if input.is_empty() {
                continue;
            }
            
            // Add to history
            self.history.push(input.to_string());
            
            // Parse and execute
            match self.execute(input) {
                Ok(exit_code) => self.last_exit = exit_code,
                Err(e) => {
                    eprintln!("vsh: {}", e);
                    self.last_exit = 1;
                }
            }
        }
    }
    
    /// Execute command
    fn execute(&mut self, input: &str) -> Result<i32, Error> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(0);
        }
        
        let cmd = parts[0];
        let args = &parts[1..];
        
        // Built-in commands
        match cmd {
            "cd" => self.builtin_cd(args),
            "pwd" => self.builtin_pwd(args),
            "export" => self.builtin_export(args),
            "exit" => std::process::exit(0),
            "help" => self.builtin_help(args),
            "history" => self.builtin_history(args),
            _ => self.execute_external(cmd, args),
        }
    }
    
    /// Change directory
    fn builtin_cd(&mut self, args: &[&str]) -> Result<i32, Error> {
        let path = args.get(0).unwrap_or(&"~");
        let path = if *path == "~" {
            self.env.get("HOME").unwrap_or(&String::from("/")).clone()
        } else {
            path.to_string()
        };
        
        env::set_current_dir(&path)?;
        self.cwd = env::current_dir()?.to_string_lossy().into_owned();
        Ok(0)
    }
    
    /// Execute external command
    fn execute_external(&self, cmd: &str, args: &[&str]) -> Result<i32, Error> {
        let mut command = Command::new(cmd);
        command.args(args);
        
        // Set environment
        for (key, value) in &self.env {
            command.env(key, value);
        }
        
        let status = command.status()?;
        Ok(status.code().unwrap_or(-1))
    }
}

fn main() {
    let mut shell = Shell::new();
    shell.run();
}
```

## Implementation Timeline

### Month 10-11: Init System & Driver Framework
- Week 1-2: Init process and service manager
- Week 3-4: Device manager and driver framework
- Week 5-6: Basic device enumeration (PCI, platform)
- Week 7-8: Driver loading and resource allocation

### Month 12: Virtual File System
- Week 1-2: VFS core and vnode management
- Week 3-4: Mount operations and path resolution

### Month 13: File System Drivers
- Week 1-2: tmpfs implementation
- Week 3-4: devfs and procfs

### Month 14: Network Stack
- Week 1-2: Network service architecture
- Week 3-4: Basic TCP/IP with smoltcp

### Month 15: Standard Library & Shell
- Week 1-2: libveridian implementation
- Week 3-4: Basic shell and core utilities

## Testing Strategy

### Unit Tests
- Service manager state machine tests
- VFS path resolution tests
- Network protocol tests
- Library API tests

### Integration Tests
- Service dependency resolution
- File system mounting/unmounting
- Network connectivity tests
- Shell command execution

### System Tests
- Full boot sequence
- Multi-service interaction
- File I/O performance
- Network throughput

## Success Criteria

1. **Init System**: Reliable service management with dependencies
2. **Drivers**: Framework supports common hardware
3. **VFS**: POSIX-compatible file operations
4. **Network**: Basic TCP/IP connectivity
5. **Library**: Sufficient for simple applications
6. **Shell**: Interactive command execution

## Dependencies for Phase 3

- Stable user-space environment
- Working file system
- Network connectivity
- Development tools (compiler, debugger)
- Testing infrastructure
# Veridian OS: Advanced Driver Development Guide

## Table of Contents

1. [Introduction](#introduction)
2. [Driver Architecture Overview](#driver-architecture-overview)
3. [Driver Framework Design](#driver-framework-design)
4. [Hardware Abstraction Layer](#hardware-abstraction-layer)
5. [Driver Types and Interfaces](#driver-types-and-interfaces)
6. [Memory-Mapped I/O and Port I/O](#memory-mapped-io-and-port-io)
7. [Interrupt Handling in Drivers](#interrupt-handling-in-drivers)
8. [DMA Operations](#dma-operations)
9. [Power Management](#power-management)
10. [Bus Drivers](#bus-drivers)
11. [Block Device Drivers](#block-device-drivers)
12. [Network Device Drivers](#network-device-drivers)
13. [Graphics Drivers](#graphics-drivers)
14. [USB Stack Implementation](#usb-stack-implementation)
15. [Driver Testing and Debugging](#driver-testing-and-debugging)
16. [Performance Optimization](#performance-optimization)
17. [Security Considerations](#security-considerations)
18. [Driver Development Workflow](#driver-development-workflow)

## Introduction

This guide provides comprehensive coverage of driver development for Veridian OS, focusing on userspace drivers with capability-based security. Our approach prioritizes safety, performance, and maintainability while leveraging Rust's type system.

### Design Principles

1. **Userspace Drivers**: Most drivers run in userspace for isolation
2. **Capability-Based Access**: Hardware access through capabilities
3. **Zero-Copy I/O**: Minimize data movement for performance
4. **Async-First**: Leverage Rust's async ecosystem
5. **Hot-Plug Support**: Dynamic device addition/removal

### Prerequisites

- Understanding of Veridian OS architecture
- Familiarity with hardware interfaces
- Knowledge of Rust async programming
- Basic understanding of DMA and interrupts

## Driver Architecture Overview

### System Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Applications                          │
├─────────────────────────────────────────────────────────┤
│                   Device Manager                         │
├─────────────────────────────────────────────────────────┤
│    Block Driver │ Network Driver │ Graphics Driver      │
├─────────────────────────────────────────────────────────┤
│              Driver Framework Library                    │
├─────────────────────────────────────────────────────────┤
│          Hardware Abstraction Layer (HAL)               │
├─────────────────────────────────────────────────────────┤
│    Capability System │ IPC │ Memory Management          │
├─────────────────────────────────────────────────────────┤
│                    Microkernel                          │
└─────────────────────────────────────────────────────────┘
```

### Driver Isolation Model

```rust
/// Driver process isolation
pub struct DriverProcess {
    /// Process ID
    pid: ProcessId,
    /// Capabilities for hardware access
    capabilities: Vec<Capability>,
    /// Memory regions for MMIO
    mmio_regions: Vec<MemoryRegion>,
    /// DMA buffer allocations
    dma_buffers: Vec<DmaBuffer>,
    /// Interrupt handlers
    interrupt_handlers: HashMap<u32, InterruptHandler>,
}
```

## Driver Framework Design

### Core Driver Trait

Create `libs/driver_framework/src/lib.rs`:

```rust
#![no_std]

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

/// Result type for driver operations
pub type Result<T> = core::result::Result<T, DriverError>;

/// Driver error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    InvalidDevice,
    HardwareError,
    Timeout,
    ResourceBusy,
    NotSupported,
    InvalidParameter,
    OutOfMemory,
    PermissionDenied,
}

/// Core driver trait
pub trait Driver: Send + Sync {
    /// Driver name
    fn name(&self) -> &str;
    
    /// Driver version
    fn version(&self) -> &str;
    
    /// Initialize the driver
    fn init(&mut self) -> impl Future<Output = Result<()>>;
    
    /// Probe for supported devices
    fn probe(&mut self, device: &DeviceInfo) -> impl Future<Output = Result<bool>>;
    
    /// Attach to a device
    fn attach(&mut self, device: DeviceHandle) -> impl Future<Output = Result<()>>;
    
    /// Detach from a device
    fn detach(&mut self) -> impl Future<Output = Result<()>>;
    
    /// Suspend the device
    fn suspend(&mut self) -> impl Future<Output = Result<()>>;
    
    /// Resume the device
    fn resume(&mut self) -> impl Future<Output = Result<()>>;
    
    /// Handle power state changes
    fn set_power_state(&mut self, state: PowerState) -> impl Future<Output = Result<()>>;
}

/// Device information
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device class
    pub class: DeviceClass,
    /// Vendor ID
    pub vendor_id: u16,
    /// Device ID
    pub device_id: u16,
    /// Subsystem vendor ID
    pub subsystem_vendor_id: u16,
    /// Subsystem device ID
    pub subsystem_device_id: u16,
    /// Device capabilities
    pub capabilities: DeviceCapabilities,
}

/// Device classes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Storage,
    Network,
    Display,
    Audio,
    Input,
    USB,
    Serial,
    Other(u32),
}

/// Device handle for driver operations
pub struct DeviceHandle {
    /// Unique device identifier
    device_id: u64,
    /// Memory-mapped I/O capability
    mmio_cap: Option<Capability>,
    /// Port I/O capability
    pio_cap: Option<Capability>,
    /// DMA capability
    dma_cap: Option<Capability>,
    /// Interrupt capability
    interrupt_cap: Option<Capability>,
}

/// Power states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    D0, // Fully operational
    D1, // Light sleep
    D2, // Deep sleep
    D3, // Off
}
```

### Driver Registration

Create `libs/driver_framework/src/registry.rs`:

```rust
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use spin::RwLock;

/// Global driver registry
static DRIVER_REGISTRY: RwLock<DriverRegistry> = RwLock::new(DriverRegistry::new());

/// Driver metadata
#[derive(Clone)]
pub struct DriverMetadata {
    pub name: &'static str,
    pub version: &'static str,
    pub author: &'static str,
    pub description: &'static str,
    pub supported_devices: &'static [DeviceId],
}

/// Device ID for matching
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceId {
    pub vendor: u16,
    pub device: u16,
}

/// Driver constructor function
pub type DriverConstructor = fn() -> Result<Box<dyn Driver>>;

/// Driver registry
pub struct DriverRegistry {
    drivers: BTreeMap<&'static str, (DriverMetadata, DriverConstructor)>,
}

impl DriverRegistry {
    const fn new() -> Self {
        Self {
            drivers: BTreeMap::new(),
        }
    }
    
    /// Register a driver
    pub fn register(
        &mut self,
        metadata: DriverMetadata,
        constructor: DriverConstructor,
    ) -> Result<()> {
        if self.drivers.contains_key(metadata.name) {
            return Err(DriverError::AlreadyRegistered);
        }
        
        self.drivers.insert(metadata.name, (metadata.clone(), constructor));
        Ok(())
    }
    
    /// Find driver for device
    pub fn find_driver(&self, device: &DeviceInfo) -> Option<(&DriverMetadata, DriverConstructor)> {
        let device_id = DeviceId {
            vendor: device.vendor_id,
            device: device.device_id,
        };
        
        for (metadata, constructor) in self.drivers.values() {
            if metadata.supported_devices.contains(&device_id) {
                return Some((metadata, *constructor));
            }
        }
        
        None
    }
}

/// Register driver macro
#[macro_export]
macro_rules! register_driver {
    ($metadata:expr, $constructor:expr) => {
        #[used]
        #[link_section = ".init_array"]
        static DRIVER_INIT: extern "C" fn() = {
            extern "C" fn init() {
                DRIVER_REGISTRY.write().register($metadata, $constructor).unwrap();
            }
            init
        };
    };
}
```

## Hardware Abstraction Layer

### HAL Core

Create `libs/hal/src/lib.rs`:

```rust
#![no_std]

use core::ptr::{read_volatile, write_volatile};

/// Memory-mapped I/O wrapper
pub struct Mmio<T> {
    ptr: *mut T,
}

impl<T> Mmio<T> {
    /// Create new MMIO wrapper
    /// 
    /// # Safety
    /// Caller must ensure the pointer is valid and properly mapped
    pub const unsafe fn new(addr: usize) -> Self {
        Self {
            ptr: addr as *mut T,
        }
    }
    
    /// Read from MMIO register
    pub fn read(&self) -> T
    where
        T: Copy,
    {
        unsafe { read_volatile(self.ptr) }
    }
    
    /// Write to MMIO register
    pub fn write(&mut self, value: T)
    where
        T: Copy,
    {
        unsafe { write_volatile(self.ptr, value) }
    }
    
    /// Modify MMIO register
    pub fn modify<F>(&mut self, f: F)
    where
        T: Copy,
        F: FnOnce(T) -> T,
    {
        let value = self.read();
        self.write(f(value));
    }
}

/// Port I/O operations (x86-specific)
#[cfg(target_arch = "x86_64")]
pub mod port {
    use core::arch::asm;
    
    /// Read byte from port
    pub unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        asm!("in al, dx", out("al") value, in("dx") port);
        value
    }
    
    /// Write byte to port
    pub unsafe fn outb(port: u16, value: u8) {
        asm!("out dx, al", in("dx") port, in("al") value);
    }
    
    /// Read word from port
    pub unsafe fn inw(port: u16) -> u16 {
        let value: u16;
        asm!("in ax, dx", out("ax") value, in("dx") port);
        value
    }
    
    /// Write word to port
    pub unsafe fn outw(port: u16, value: u16) {
        asm!("out dx, ax", in("dx") port, in("ax") value);
    }
    
    /// Read dword from port
    pub unsafe fn inl(port: u16) -> u32 {
        let value: u32;
        asm!("in eax, dx", out("eax") value, in("dx") port);
        value
    }
    
    /// Write dword to port
    pub unsafe fn outl(port: u16, value: u32) {
        asm!("out dx, eax", in("dx") port, in("eax") value);
    }
}

/// DMA buffer abstraction
pub struct DmaBuffer {
    /// Virtual address
    virt_addr: *mut u8,
    /// Physical address
    phys_addr: u64,
    /// Buffer size
    size: usize,
    /// Capability for DMA access
    capability: Capability,
}

impl DmaBuffer {
    /// Allocate DMA buffer
    pub fn allocate(size: usize, capability: Capability) -> Result<Self> {
        // Request DMA buffer from kernel
        let (virt_addr, phys_addr) = syscall::allocate_dma_buffer(size, &capability)?;
        
        Ok(Self {
            virt_addr,
            phys_addr,
            size,
            capability,
        })
    }
    
    /// Get virtual address
    pub fn virt_addr(&self) -> *mut u8 {
        self.virt_addr
    }
    
    /// Get physical address
    pub fn phys_addr(&self) -> u64 {
        self.phys_addr
    }
    
    /// Get buffer as slice
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.virt_addr, self.size) }
    }
    
    /// Get buffer as mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.virt_addr, self.size) }
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        // Free DMA buffer
        let _ = syscall::free_dma_buffer(self.virt_addr, self.size, &self.capability);
    }
}
```

### Device Tree Support

Create `libs/hal/src/device_tree.rs`:

```rust
/// Device tree node
#[derive(Debug, Clone)]
pub struct DeviceTreeNode {
    pub name: String,
    pub compatible: Vec<String>,
    pub properties: HashMap<String, Property>,
    pub children: Vec<DeviceTreeNode>,
}

/// Device tree property
#[derive(Debug, Clone)]
pub enum Property {
    Empty,
    U32(u32),
    U64(u64),
    String(String),
    Binary(Vec<u8>),
    Reference(String),
    U32Array(Vec<u32>),
    U64Array(Vec<u64>),
}

impl DeviceTreeNode {
    /// Find node by path
    pub fn find_node(&self, path: &str) -> Option<&DeviceTreeNode> {
        if path.is_empty() {
            return Some(self);
        }
        
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.find_node_recursive(&parts)
    }
    
    fn find_node_recursive(&self, path: &[&str]) -> Option<&DeviceTreeNode> {
        if path.is_empty() {
            return Some(self);
        }
        
        for child in &self.children {
            if child.name == path[0] {
                return child.find_node_recursive(&path[1..]);
            }
        }
        
        None
    }
    
    /// Get property value
    pub fn get_property(&self, name: &str) -> Option<&Property> {
        self.properties.get(name)
    }
    
    /// Get register addresses
    pub fn get_reg(&self) -> Option<Vec<(u64, u64)>> {
        match self.get_property("reg")? {
            Property::Binary(data) => {
                // Parse reg property (assumes 64-bit addresses and sizes)
                let mut regs = Vec::new();
                for chunk in data.chunks_exact(16) {
                    let addr = u64::from_be_bytes(chunk[0..8].try_into().ok()?);
                    let size = u64::from_be_bytes(chunk[8..16].try_into().ok()?);
                    regs.push((addr, size));
                }
                Some(regs)
            }
            _ => None,
        }
    }
}
```

## Driver Types and Interfaces

### Character Device Interface

Create `libs/driver_framework/src/char_device.rs`:

```rust
use core::future::Future;
use core::pin::Pin;

/// Character device operations
pub trait CharDevice: Driver {
    /// Read data from device
    fn read(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize>>;
    
    /// Write data to device
    fn write(&mut self, buffer: &[u8]) -> impl Future<Output = Result<usize>>;
    
    /// IO control operations
    fn ioctl(&mut self, cmd: u32, arg: usize) -> impl Future<Output = Result<usize>>;
    
    /// Poll for device readiness
    fn poll(&self) -> impl Future<Output = PollStatus>;
    
    /// Flush any pending data
    fn flush(&mut self) -> impl Future<Output = Result<()>>;
}

/// Poll status flags
bitflags::bitflags! {
    pub struct PollStatus: u32 {
        const READABLE  = 0b0001;
        const WRITABLE  = 0b0010;
        const ERROR     = 0b0100;
        const HANGUP    = 0b1000;
    }
}
```

### Block Device Interface

Create `libs/driver_framework/src/block_device.rs`:

```rust
/// Block device operations
pub trait BlockDevice: Driver {
    /// Get device information
    fn info(&self) -> BlockDeviceInfo;
    
    /// Read blocks
    fn read_blocks(
        &mut self,
        start_block: u64,
        blocks: &mut [Block],
    ) -> impl Future<Output = Result<()>>;
    
    /// Write blocks
    fn write_blocks(
        &mut self,
        start_block: u64,
        blocks: &[Block],
    ) -> impl Future<Output = Result<()>>;
    
    /// Flush write cache
    fn flush(&mut self) -> impl Future<Output = Result<()>>;
    
    /// TRIM/discard blocks
    fn discard(
        &mut self,
        start_block: u64,
        block_count: u64,
    ) -> impl Future<Output = Result<()>>;
}

/// Block device information
#[derive(Debug, Clone, Copy)]
pub struct BlockDeviceInfo {
    /// Block size in bytes
    pub block_size: u32,
    /// Total number of blocks
    pub total_blocks: u64,
    /// Device is read-only
    pub read_only: bool,
    /// Device is removable
    pub removable: bool,
    /// Optimal I/O size
    pub optimal_io_size: u32,
}

/// Block buffer
#[repr(align(512))]
pub struct Block {
    data: [u8; 512],
}

impl Block {
    pub const SIZE: usize = 512;
    
    pub fn new() -> Self {
        Self { data: [0; Self::SIZE] }
    }
    
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
    
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
```

### Network Device Interface

Create `libs/driver_framework/src/net_device.rs`:

```rust
/// Network device operations
pub trait NetworkDevice: Driver {
    /// Get device information
    fn info(&self) -> NetworkDeviceInfo;
    
    /// Transmit packet
    fn transmit(&mut self, packet: Packet) -> impl Future<Output = Result<()>>;
    
    /// Receive packet
    fn receive(&mut self) -> impl Future<Output = Result<Packet>>;
    
    /// Set MAC address
    fn set_mac_address(&mut self, mac: MacAddress) -> impl Future<Output = Result<()>>;
    
    /// Enable promiscuous mode
    fn set_promiscuous(&mut self, enable: bool) -> impl Future<Output = Result<()>>;
    
    /// Get link status
    fn link_status(&self) -> LinkStatus;
    
    /// Get statistics
    fn statistics(&self) -> NetworkStatistics;
}

/// Network device information
#[derive(Debug, Clone)]
pub struct NetworkDeviceInfo {
    /// MAC address
    pub mac_address: MacAddress,
    /// Maximum transmission unit
    pub mtu: u16,
    /// Supported features
    pub features: NetworkFeatures,
    /// Link speed in Mbps
    pub link_speed: u32,
}

/// MAC address
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
    
    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }
}

/// Network packet
pub struct Packet {
    /// Packet data
    data: Vec<u8>,
    /// Packet metadata
    metadata: PacketMetadata,
}

/// Packet metadata
#[derive(Debug, Clone)]
pub struct PacketMetadata {
    /// Timestamp
    pub timestamp: u64,
    /// Checksum offload info
    pub checksum: ChecksumInfo,
    /// VLAN tag
    pub vlan: Option<u16>,
}

bitflags::bitflags! {
    pub struct NetworkFeatures: u32 {
        const CHECKSUM_IPV4     = 0b0000_0001;
        const CHECKSUM_TCP      = 0b0000_0010;
        const CHECKSUM_UDP      = 0b0000_0100;
        const TSO               = 0b0000_1000;
        const GSO               = 0b0001_0000;
        const VLAN              = 0b0010_0000;
        const RSS               = 0b0100_0000;
        const MULTIQUEUE        = 0b1000_0000;
    }
}
```

## Memory-Mapped I/O and Port I/O

### MMIO Safety Wrapper

Create `libs/driver_framework/src/mmio.rs`:

```rust
use core::marker::PhantomData;

/// Type-safe MMIO register access
pub struct MmioRegister<T, A = ReadWrite> {
    addr: usize,
    _phantom: PhantomData<(T, A)>,
}

/// Read-only access
pub struct ReadOnly;
/// Write-only access
pub struct WriteOnly;
/// Read-write access
pub struct ReadWrite;

impl<T: Copy> MmioRegister<T, ReadOnly> {
    pub fn read(&self) -> T {
        unsafe { core::ptr::read_volatile(self.addr as *const T) }
    }
}

impl<T: Copy> MmioRegister<T, WriteOnly> {
    pub fn write(&mut self, value: T) {
        unsafe { core::ptr::write_volatile(self.addr as *mut T, value) }
    }
}

impl<T: Copy> MmioRegister<T, ReadWrite> {
    pub fn read(&self) -> T {
        unsafe { core::ptr::read_volatile(self.addr as *const T) }
    }
    
    pub fn write(&mut self, value: T) {
        unsafe { core::ptr::write_volatile(self.addr as *mut T, value) }
    }
    
    pub fn modify<F>(&mut self, f: F)
    where
        F: FnOnce(T) -> T,
    {
        let value = self.read();
        self.write(f(value));
    }
}

/// MMIO register block
#[macro_export]
macro_rules! mmio_struct {
    (
        $(#[$attr:meta])*
        pub struct $name:ident {
            $(
                $(#[$field_attr:meta])*
                ($offset:expr => $field:ident: $type:ty $([$access:ty])?),
            )+
        }
    ) => {
        $(#[$attr])*
        #[repr(C)]
        pub struct $name {
            base: usize,
        }
        
        impl $name {
            pub const unsafe fn new(base: usize) -> Self {
                Self { base }
            }
            
            $(
                $(#[$field_attr])*
                pub fn $field(&self) -> &MmioRegister<$type $(, $access)?> {
                    unsafe {
                        &*(self.base + $offset as usize) as *const MmioRegister<$type $(, $access)?>
                    }
                }
            )+
        }
    };
}
```

### Port I/O Wrapper

Create `libs/driver_framework/src/port_io.rs`:

```rust
/// Type-safe port I/O
pub struct Port<T> {
    port: u16,
    _phantom: PhantomData<T>,
}

impl Port<u8> {
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            _phantom: PhantomData,
        }
    }
    
    pub fn read(&self) -> u8 {
        unsafe { hal::port::inb(self.port) }
    }
    
    pub fn write(&mut self, value: u8) {
        unsafe { hal::port::outb(self.port, value) }
    }
}

impl Port<u16> {
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            _phantom: PhantomData,
        }
    }
    
    pub fn read(&self) -> u16 {
        unsafe { hal::port::inw(self.port) }
    }
    
    pub fn write(&mut self, value: u16) {
        unsafe { hal::port::outw(self.port, value) }
    }
}

impl Port<u32> {
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            _phantom: PhantomData,
        }
    }
    
    pub fn read(&self) -> u32 {
        unsafe { hal::port::inl(self.port) }
    }
    
    pub fn write(&mut self, value: u32) {
        unsafe { hal::port::outl(self.port, value) }
    }
}
```

## Interrupt Handling in Drivers

### Interrupt Management

Create `libs/driver_framework/src/interrupts.rs`:

```rust
use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

/// Interrupt handler trait
pub trait InterruptHandler: Send + Sync {
    /// Handle interrupt
    fn handle(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Interrupt registration
pub struct InterruptRegistration {
    irq: u32,
    handler: Box<dyn InterruptHandler>,
    capability: Capability,
}

impl InterruptRegistration {
    /// Register interrupt handler
    pub async fn register(
        irq: u32,
        handler: impl InterruptHandler + 'static,
        capability: Capability,
    ) -> Result<Self> {
        // Register with kernel
        syscall::register_interrupt(irq, &capability).await?;
        
        Ok(Self {
            irq,
            handler: Box::new(handler),
            capability,
        })
    }
    
    /// Enable interrupt
    pub async fn enable(&self) -> Result<()> {
        syscall::enable_interrupt(self.irq, &self.capability).await
    }
    
    /// Disable interrupt
    pub async fn disable(&self) -> Result<()> {
        syscall::disable_interrupt(self.irq, &self.capability).await
    }
}

impl Drop for InterruptRegistration {
    fn drop(&mut self) {
        // Unregister interrupt
        let _ = syscall::unregister_interrupt(self.irq, &self.capability);
    }
}

/// Message Signaled Interrupts (MSI)
pub struct MsiCapability {
    /// MSI capability offset
    offset: u8,
    /// Number of vectors
    num_vectors: u8,
}

impl MsiCapability {
    /// Enable MSI
    pub fn enable(&self, device: &mut PciDevice, vector: u8) -> Result<()> {
        if vector >= self.num_vectors {
            return Err(DriverError::InvalidParameter);
        }
        
        // Configure MSI
        let control = device.read_config_u16(self.offset + 2)?;
        device.write_config_u16(self.offset + 2, control | 0x0001)?;
        
        Ok(())
    }
}

/// MSI-X support
pub struct MsixCapability {
    /// MSI-X capability offset
    offset: u8,
    /// Table size
    table_size: u16,
    /// Table BAR
    table_bar: u8,
    /// Table offset
    table_offset: u32,
}

impl MsixCapability {
    /// Configure MSI-X vector
    pub fn configure_vector(
        &self,
        device: &mut PciDevice,
        vector: u16,
        address: u64,
        data: u32,
    ) -> Result<()> {
        if vector >= self.table_size {
            return Err(DriverError::InvalidParameter);
        }
        
        // Map MSI-X table
        let table_addr = device.bar_address(self.table_bar)? + self.table_offset as u64;
        
        // Write vector entry
        unsafe {
            let entry = (table_addr + vector as u64 * 16) as *mut u32;
            entry.write_volatile(address as u32);
            entry.offset(1).write_volatile((address >> 32) as u32);
            entry.offset(2).write_volatile(data);
            entry.offset(3).write_volatile(0); // Unmask
        }
        
        Ok(())
    }
}
```

### Interrupt Coalescing

```rust
/// Interrupt coalescing configuration
pub struct InterruptCoalescing {
    /// Maximum packets before interrupt
    pub max_packets: u32,
    /// Maximum time before interrupt (microseconds)
    pub max_time_us: u32,
    /// Adaptive coalescing enabled
    pub adaptive: bool,
}

impl InterruptCoalescing {
    /// Configure interrupt coalescing
    pub fn configure(&self, device: &mut dyn NetworkDevice) -> Result<()> {
        // Device-specific implementation
        device.set_coalescing(self)
    }
    
    /// Adaptive algorithm
    pub fn adapt(&mut self, packet_rate: u32, cpu_usage: f32) {
        if !self.adaptive {
            return;
        }
        
        // High packet rate: increase coalescing
        if packet_rate > 100_000 {
            self.max_packets = self.max_packets.saturating_add(10).min(64);
            self.max_time_us = self.max_time_us.saturating_add(10).min(1000);
        }
        // Low packet rate: decrease coalescing
        else if packet_rate < 10_000 {
            self.max_packets = self.max_packets.saturating_sub(5).max(1);
            self.max_time_us = self.max_time_us.saturating_sub(10).max(10);
        }
        
        // High CPU usage: increase coalescing
        if cpu_usage > 0.8 {
            self.max_packets = self.max_packets.saturating_add(5).min(64);
        }
    }
}
```

## DMA Operations

### DMA Engine

Create `libs/driver_framework/src/dma.rs`:

```rust
use core::sync::atomic::{AtomicU32, Ordering};

/// DMA direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    ToDevice,
    FromDevice,
    Bidirectional,
}

/// DMA descriptor
#[repr(C, align(16))]
pub struct DmaDescriptor {
    /// Physical address
    pub addr: u64,
    /// Length
    pub len: u32,
    /// Flags
    pub flags: DmaFlags,
    /// Next descriptor (for chaining)
    pub next: u64,
}

bitflags::bitflags! {
    pub struct DmaFlags: u32 {
        const VALID     = 0b0000_0001;
        const INTERRUPT = 0b0000_0010;
        const LAST      = 0b0000_0100;
        const ERROR     = 0b0001_0000;
        const DONE      = 0b0010_0000;
    }
}

/// DMA channel
pub struct DmaChannel {
    /// Channel ID
    id: u32,
    /// Current descriptor
    current: AtomicU32,
    /// Descriptor ring
    descriptors: Vec<DmaDescriptor>,
    /// DMA buffers
    buffers: Vec<DmaBuffer>,
    /// Capability
    capability: Capability,
}

impl DmaChannel {
    /// Allocate DMA channel
    pub async fn allocate(
        num_descriptors: usize,
        capability: Capability,
    ) -> Result<Self> {
        let id = syscall::allocate_dma_channel(&capability).await?;
        
        Ok(Self {
            id,
            current: AtomicU32::new(0),
            descriptors: vec![DmaDescriptor::default(); num_descriptors],
            buffers: Vec::with_capacity(num_descriptors),
            capability,
        })
    }
    
    /// Queue DMA transfer
    pub fn queue_transfer(
        &mut self,
        buffer: DmaBuffer,
        direction: DmaDirection,
        interrupt: bool,
    ) -> Result<u32> {
        let index = self.current.fetch_add(1, Ordering::Relaxed) as usize
            % self.descriptors.len();
        
        let mut flags = DmaFlags::VALID;
        if interrupt {
            flags |= DmaFlags::INTERRUPT;
        }
        
        self.descriptors[index] = DmaDescriptor {
            addr: buffer.phys_addr(),
            len: buffer.size() as u32,
            flags,
            next: if index == self.descriptors.len() - 1 {
                self.descriptors.as_ptr() as u64
            } else {
                &self.descriptors[index + 1] as *const _ as u64
            },
        };
        
        self.buffers.push(buffer);
        
        Ok(index as u32)
    }
    
    /// Start DMA transfers
    pub async fn start(&self) -> Result<()> {
        syscall::start_dma(self.id, &self.capability).await
    }
    
    /// Stop DMA transfers
    pub async fn stop(&self) -> Result<()> {
        syscall::stop_dma(self.id, &self.capability).await
    }
    
    /// Check transfer completion
    pub fn is_complete(&self, index: u32) -> bool {
        self.descriptors[index as usize].flags.contains(DmaFlags::DONE)
    }
}

/// Scatter-gather DMA
pub struct ScatterGatherList {
    entries: Vec<ScatterGatherEntry>,
}

#[repr(C)]
pub struct ScatterGatherEntry {
    pub addr: u64,
    pub len: u32,
    pub offset: u32,
}

impl ScatterGatherList {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    
    pub fn add_buffer(&mut self, buffer: &DmaBuffer, offset: usize, len: usize) {
        self.entries.push(ScatterGatherEntry {
            addr: buffer.phys_addr() + offset as u64,
            len: len as u32,
            offset: offset as u32,
        });
    }
    
    pub fn total_len(&self) -> usize {
        self.entries.iter().map(|e| e.len as usize).sum()
    }
}
```

### IOMMU Support

```rust
/// IOMMU operations
pub struct Iommu {
    /// Domain ID
    domain: u32,
    /// Capability
    capability: Capability,
}

impl Iommu {
    /// Create IOMMU domain
    pub async fn create_domain(capability: Capability) -> Result<Self> {
        let domain = syscall::iommu_create_domain(&capability).await?;
        
        Ok(Self {
            domain,
            capability,
        })
    }
    
    /// Map DMA address
    pub async fn map(
        &self,
        iova: u64,
        phys: u64,
        size: usize,
        prot: IommuProtection,
    ) -> Result<()> {
        syscall::iommu_map(self.domain, iova, phys, size, prot, &self.capability).await
    }
    
    /// Unmap DMA address
    pub async fn unmap(&self, iova: u64, size: usize) -> Result<()> {
        syscall::iommu_unmap(self.domain, iova, size, &self.capability).await
    }
    
    /// Attach device to domain
    pub async fn attach_device(&self, device: &DeviceHandle) -> Result<()> {
        syscall::iommu_attach_device(self.domain, device.device_id, &self.capability).await
    }
}

bitflags::bitflags! {
    pub struct IommuProtection: u32 {
        const READ  = 0b001;
        const WRITE = 0b010;
        const EXEC  = 0b100;
    }
}
```

## Power Management

### Power Management Framework

Create `libs/driver_framework/src/power.rs`:

```rust
/// Power management operations
pub trait PowerManagement: Driver {
    /// Get current power state
    fn power_state(&self) -> PowerState;
    
    /// Set power state
    fn set_power_state(&mut self, state: PowerState) -> impl Future<Output = Result<()>>;
    
    /// Get power capabilities
    fn power_caps(&self) -> PowerCapabilities;
    
    /// Runtime suspend
    fn runtime_suspend(&mut self) -> impl Future<Output = Result<()>>;
    
    /// Runtime resume
    fn runtime_resume(&mut self) -> impl Future<Output = Result<()>>;
}

/// Power capabilities
#[derive(Debug, Clone, Copy)]
pub struct PowerCapabilities {
    /// Supported power states
    pub states: PowerStates,
    /// Wake capabilities
    pub wake_caps: WakeCapabilities,
    /// Power consumption in each state (milliwatts)
    pub power_consumption: [u32; 4],
    /// Transition latencies (microseconds)
    pub transition_latency: [[u32; 4]; 4],
}

bitflags::bitflags! {
    pub struct PowerStates: u8 {
        const D0 = 0b0001;
        const D1 = 0b0010;
        const D2 = 0b0100;
        const D3 = 0b1000;
    }
    
    pub struct WakeCapabilities: u32 {
        const PME       = 0b0000_0001;
        const MAGIC     = 0b0000_0010;
        const LINK      = 0b0000_0100;
        const PATTERN   = 0b0000_1000;
    }
}

/// Runtime power management
pub struct RuntimePm {
    /// Current state
    state: RwLock<RuntimePmState>,
    /// Configuration
    config: RuntimePmConfig,
    /// Statistics
    stats: RuntimePmStats,
}

#[derive(Debug, Clone, Copy)]
enum RuntimePmState {
    Active,
    Suspending,
    Suspended,
    Resuming,
}

#[derive(Debug, Clone)]
pub struct RuntimePmConfig {
    /// Auto-suspend delay (milliseconds)
    pub autosuspend_delay_ms: u32,
    /// Use auto-suspend
    pub use_autosuspend: bool,
    /// Aggressive power saving
    pub aggressive: bool,
}

#[derive(Debug, Default)]
pub struct RuntimePmStats {
    /// Number of suspends
    pub suspend_count: AtomicU64,
    /// Number of resumes
    pub resume_count: AtomicU64,
    /// Total suspended time (microseconds)
    pub suspended_time_us: AtomicU64,
    /// Failed suspend attempts
    pub failed_suspends: AtomicU64,
}
```

### ACPI Power Management

```rust
/// ACPI power management
pub struct AcpiPowerManagement {
    /// ACPI handle
    handle: AcpiHandle,
    /// Power resource dependencies
    power_resources: Vec<PowerResource>,
}

/// Power resource
pub struct PowerResource {
    /// Resource name
    name: String,
    /// System level
    system_level: u8,
    /// Resource order
    order: u16,
}

impl AcpiPowerManagement {
    /// Evaluate _PS0 (power on)
    pub async fn power_on(&self) -> Result<()> {
        self.evaluate_method("_PS0", &[]).await
    }
    
    /// Evaluate _PS3 (power off)
    pub async fn power_off(&self) -> Result<()> {
        self.evaluate_method("_PS3", &[]).await
    }
    
    /// Get power state
    pub async fn get_power_state(&self) -> Result<PowerState> {
        let result = self.evaluate_method("_PSC", &[]).await?;
        
        match result {
            AcpiValue::Integer(0) => Ok(PowerState::D0),
            AcpiValue::Integer(1) => Ok(PowerState::D1),
            AcpiValue::Integer(2) => Ok(PowerState::D2),
            AcpiValue::Integer(3) => Ok(PowerState::D3),
            _ => Err(DriverError::InvalidDevice),
        }
    }
    
    async fn evaluate_method(&self, method: &str, args: &[AcpiValue]) -> Result<AcpiValue> {
        syscall::acpi_evaluate(self.handle, method, args).await
            .map_err(|_| DriverError::HardwareError)
    }
}
```

## Bus Drivers

### PCI Bus Driver

Create `drivers/pci/src/lib.rs`:

```rust
use driver_framework::prelude::*;

/// PCI configuration space
#[repr(C)]
pub struct PciConfigSpace {
    pub vendor_id: u16,
    pub device_id: u16,
    pub command: u16,
    pub status: u16,
    pub revision_id: u8,
    pub prog_if: u8,
    pub subclass: u8,
    pub class: u8,
    pub cache_line_size: u8,
    pub latency_timer: u8,
    pub header_type: u8,
    pub bist: u8,
    pub bars: [u32; 6],
    pub cardbus_cis: u32,
    pub subsystem_vendor_id: u16,
    pub subsystem_id: u16,
    pub expansion_rom: u32,
    pub capabilities_ptr: u8,
    pub reserved: [u8; 7],
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
    pub min_grant: u8,
    pub max_latency: u8,
}

/// PCI device
pub struct PciDevice {
    /// Bus, device, function
    pub bdf: (u8, u8, u8),
    /// Configuration space
    config: PciConfigSpace,
    /// Memory mapped config
    mmio_config: Option<*mut PciConfigSpace>,
    /// Capabilities
    capabilities: Vec<PciCapability>,
}

impl PciDevice {
    /// Read configuration register
    pub fn read_config_u32(&self, offset: u8) -> Result<u32> {
        if offset & 0x3 != 0 {
            return Err(DriverError::InvalidParameter);
        }
        
        if let Some(mmio) = self.mmio_config {
            unsafe {
                Ok((mmio as *const u32).add(offset as usize / 4).read_volatile())
            }
        } else {
            // Use port I/O
            let addr = 0x80000000 
                | (self.bdf.0 as u32) << 16
                | (self.bdf.1 as u32) << 11
                | (self.bdf.2 as u32) << 8
                | (offset as u32);
            
            unsafe {
                hal::port::outl(0xCF8, addr);
                Ok(hal::port::inl(0xCFC))
            }
        }
    }
    
    /// Write configuration register
    pub fn write_config_u32(&mut self, offset: u8, value: u32) -> Result<()> {
        if offset & 0x3 != 0 {
            return Err(DriverError::InvalidParameter);
        }
        
        if let Some(mmio) = self.mmio_config {
            unsafe {
                (mmio as *mut u32).add(offset as usize / 4).write_volatile(value);
            }
        } else {
            // Use port I/O
            let addr = 0x80000000 
                | (self.bdf.0 as u32) << 16
                | (self.bdf.1 as u32) << 11
                | (self.bdf.2 as u32) << 8
                | (offset as u32);
            
            unsafe {
                hal::port::outl(0xCF8, addr);
                hal::port::outl(0xCFC, value);
            }
        }
        
        Ok(())
    }
    
    /// Get BAR address
    pub fn bar_address(&self, bar: u8) -> Result<u64> {
        if bar >= 6 {
            return Err(DriverError::InvalidParameter);
        }
        
        let bar_value = self.config.bars[bar as usize];
        
        // Check if 64-bit BAR
        if bar_value & 0x4 != 0 && bar < 5 {
            let high = self.config.bars[bar as usize + 1];
            Ok(((high as u64) << 32) | (bar_value as u64 & !0xF))
        } else {
            Ok(bar_value as u64 & !0xF)
        }
    }
    
    /// Enable bus mastering
    pub fn enable_bus_master(&mut self) -> Result<()> {
        let mut command = self.read_config_u16(0x04)?;
        command |= 0x0004; // Bus master bit
        self.write_config_u16(0x04, command)
    }
}

/// PCI capability
#[derive(Debug, Clone)]
pub enum PciCapability {
    PowerManagement(PmCapability),
    Msi(MsiCapability),
    MsiX(MsixCapability),
    PciExpress(PcieCapability),
    Other { id: u8, offset: u8 },
}

/// PCI bus driver
pub struct PciBusDriver {
    /// Discovered devices
    devices: Vec<PciDevice>,
    /// Root complex
    root_complex: Option<PciExpressRootComplex>,
}

#[async_trait]
impl Driver for PciBusDriver {
    fn name(&self) -> &str {
        "pci_bus"
    }
    
    fn version(&self) -> &str {
        "0.1.0"
    }
    
    async fn init(&mut self) -> Result<()> {
        // Scan PCI bus
        self.scan_bus(0).await?;
        
        println!("PCI: Found {} devices", self.devices.len());
        
        Ok(())
    }
    
    async fn probe(&mut self, device: &DeviceInfo) -> Result<bool> {
        // PCI bus driver handles all PCI devices
        Ok(device.class == DeviceClass::Other(0xFF))
    }
    
    async fn attach(&mut self, device: DeviceHandle) -> Result<()> {
        // Nothing to do for bus driver
        Ok(())
    }
}

impl PciBusDriver {
    async fn scan_bus(&mut self, bus: u8) -> Result<()> {
        for device in 0..32 {
            if let Some(pci_device) = self.scan_device(bus, device, 0).await? {
                self.devices.push(pci_device);
                
                // Check for multi-function device
                if self.is_multifunction(bus, device) {
                    for function in 1..8 {
                        if let Some(pci_device) = self.scan_device(bus, device, function).await? {
                            self.devices.push(pci_device);
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    async fn scan_device(&self, bus: u8, device: u8, function: u8) -> Result<Option<PciDevice>> {
        let bdf = (bus, device, function);
        
        // Read vendor ID
        let vendor_id = self.read_config_u16(bdf, 0x00)?;
        if vendor_id == 0xFFFF {
            return Ok(None);
        }
        
        // Read full config header
        let mut config = PciConfigSpace {
            vendor_id,
            device_id: self.read_config_u16(bdf, 0x02)?,
            // ... read rest of config
        };
        
        // Scan capabilities
        let capabilities = self.scan_capabilities(bdf, config.capabilities_ptr)?;
        
        Ok(Some(PciDevice {
            bdf,
            config,
            mmio_config: None,
            capabilities,
        }))
    }
}
```

### USB Bus Driver

Create `drivers/usb/src/lib.rs`:

```rust
/// USB device
pub struct UsbDevice {
    /// Device address
    address: u8,
    /// Device descriptor
    descriptor: DeviceDescriptor,
    /// Configuration
    configuration: Option<ConfigurationDescriptor>,
    /// Device state
    state: UsbDeviceState,
}

/// USB device descriptor
#[repr(C, packed)]
pub struct DeviceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub usb_version: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_index: u8,
    pub num_configurations: u8,
}

/// USB transfer
pub struct UsbTransfer {
    /// Transfer type
    pub transfer_type: TransferType,
    /// Endpoint
    pub endpoint: u8,
    /// Data buffer
    pub buffer: DmaBuffer,
    /// Completion callback
    pub callback: Option<Box<dyn FnOnce(UsbTransferResult) + Send>>,
}

#[derive(Debug, Clone, Copy)]
pub enum TransferType {
    Control,
    Bulk,
    Interrupt,
    Isochronous,
}

/// USB host controller interface
pub trait UsbHostController: Driver {
    /// Reset port
    fn reset_port(&mut self, port: u8) -> impl Future<Output = Result<()>>;
    
    /// Get port status
    fn port_status(&self, port: u8) -> PortStatus;
    
    /// Submit transfer
    fn submit_transfer(&mut self, transfer: UsbTransfer) -> impl Future<Output = Result<()>>;
    
    /// Cancel transfer
    fn cancel_transfer(&mut self, transfer_id: u64) -> Result<()>;
}

/// XHCI driver
pub struct XhciDriver {
    /// MMIO registers
    regs: XhciRegisters,
    /// Command ring
    command_ring: CommandRing,
    /// Event rings
    event_rings: Vec<EventRing>,
    /// Device contexts
    device_contexts: Vec<Option<DeviceContext>>,
}

mmio_struct! {
    /// XHCI registers
    pub struct XhciRegisters {
        // Capability registers
        (0x00 => cap_length: u8 [ReadOnly]),
        (0x02 => hci_version: u16 [ReadOnly]),
        (0x04 => hcs_params1: u32 [ReadOnly]),
        (0x08 => hcs_params2: u32 [ReadOnly]),
        (0x0C => hcs_params3: u32 [ReadOnly]),
        (0x10 => hcc_params1: u32 [ReadOnly]),
        (0x14 => db_offset: u32 [ReadOnly]),
        (0x18 => rts_offset: u32 [ReadOnly]),
        
        // Operational registers
        (0x20 => usb_command: u32 [ReadWrite]),
        (0x24 => usb_status: u32 [ReadWrite]),
        (0x28 => page_size: u32 [ReadOnly]),
        (0x30 => device_notification: u32 [ReadWrite]),
        (0x38 => command_ring_control: u64 [ReadWrite]),
        (0x50 => device_context_base: u64 [ReadWrite]),
        (0x58 => config: u32 [ReadWrite]),
    }
}

#[async_trait]
impl Driver for XhciDriver {
    fn name(&self) -> &str {
        "xhci"
    }
    
    fn version(&self) -> &str {
        "0.1.0"
    }
    
    async fn init(&mut self) -> Result<()> {
        // Reset controller
        self.reset_controller().await?;
        
        // Initialize data structures
        self.init_device_contexts()?;
        self.init_command_ring()?;
        self.init_event_rings()?;
        
        // Start controller
        self.start_controller().await?;
        
        Ok(())
    }
}
```

## Block Device Drivers

### NVMe Driver Implementation

Create `drivers/nvme/src/lib.rs`:

```rust
use driver_framework::prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

/// NVMe driver
pub struct NvmeDriver {
    /// Controller registers
    regs: NvmeRegisters,
    /// Admin queue
    admin_queue: NvmeQueue,
    /// I/O queues
    io_queues: Vec<NvmeQueue>,
    /// Controller capabilities
    capabilities: ControllerCapabilities,
    /// Namespace information
    namespaces: Vec<NamespaceInfo>,
}

mmio_struct! {
    /// NVMe controller registers
    pub struct NvmeRegisters {
        (0x00 => capabilities: u64 [ReadOnly]),
        (0x08 => version: u32 [ReadOnly]),
        (0x0C => interrupt_mask_set: u32 [WriteOnly]),
        (0x10 => interrupt_mask_clear: u32 [WriteOnly]),
        (0x14 => configuration: u32 [ReadWrite]),
        (0x1C => status: u32 [ReadOnly]),
        (0x20 => admin_queue_attrs: u32 [ReadWrite]),
        (0x28 => admin_sq_base: u64 [ReadWrite]),
        (0x30 => admin_cq_base: u64 [ReadWrite]),
    }
}

/// NVMe queue
pub struct NvmeQueue {
    /// Queue ID
    id: u16,
    /// Submission queue
    sq: SubmissionQueue,
    /// Completion queue
    cq: CompletionQueue,
    /// Doorbell registers
    sq_doorbell: *mut u32,
    cq_doorbell: *mut u32,
}

/// Submission queue
pub struct SubmissionQueue {
    /// Queue entries
    entries: Vec<SubmissionQueueEntry>,
    /// Current tail
    tail: AtomicU32,
    /// Queue size
    size: u16,
}

/// Submission queue entry
#[repr(C, align(64))]
pub struct SubmissionQueueEntry {
    /// Command dword 0
    pub cdw0: u32,
    /// Namespace ID
    pub nsid: u32,
    /// Reserved
    pub reserved: u64,
    /// Metadata pointer
    pub mptr: u64,
    /// Data pointer
    pub dptr: [u64; 2],
    /// Command dwords 10-15
    pub cdw10: [u32; 6],
}

/// NVMe command
pub struct NvmeCommand {
    /// Opcode
    pub opcode: u8,
    /// Flags
    pub flags: u8,
    /// Command ID
    pub cid: u16,
    /// Namespace ID
    pub nsid: u32,
    /// Metadata
    pub metadata: Option<DmaBuffer>,
    /// Data
    pub data: Option<DmaBuffer>,
    /// Command-specific fields
    pub cdw10_15: [u32; 6],
}

impl NvmeCommand {
    /// Create read command
    pub fn read(nsid: u32, lba: u64, blocks: u16, buffer: DmaBuffer) -> Self {
        let mut cdw10_15 = [0u32; 6];
        cdw10_15[0] = lba as u32;
        cdw10_15[1] = (lba >> 32) as u32;
        cdw10_15[2] = blocks as u32 - 1;
        
        Self {
            opcode: 0x02, // Read
            flags: 0,
            cid: 0,
            nsid,
            metadata: None,
            data: Some(buffer),
            cdw10_15,
        }
    }
    
    /// Create write command
    pub fn write(nsid: u32, lba: u64, blocks: u16, buffer: DmaBuffer) -> Self {
        let mut cdw10_15 = [0u32; 6];
        cdw10_15[0] = lba as u32;
        cdw10_15[1] = (lba >> 32) as u32;
        cdw10_15[2] = blocks as u32 - 1;
        
        Self {
            opcode: 0x01, // Write
            flags: 0,
            cid: 0,
            nsid,
            metadata: None,
            data: Some(buffer),
            cdw10_15,
        }
    }
}

#[async_trait]
impl Driver for NvmeDriver {
    fn name(&self) -> &str {
        "nvme"
    }
    
    fn version(&self) -> &str {
        "0.1.0"
    }
    
    async fn init(&mut self) -> Result<()> {
        // Read capabilities
        let cap = self.regs.capabilities().read();
        self.capabilities = ControllerCapabilities::from_raw(cap);
        
        // Disable controller
        self.disable_controller().await?;
        
        // Configure admin queues
        self.configure_admin_queue().await?;
        
        // Enable controller
        self.enable_controller().await?;
        
        // Identify controller
        self.identify_controller().await?;
        
        // Create I/O queues
        self.create_io_queues().await?;
        
        // Identify namespaces
        self.identify_namespaces().await?;
        
        Ok(())
    }
}

#[async_trait]
impl BlockDevice for NvmeDriver {
    fn info(&self) -> BlockDeviceInfo {
        // Return info for first namespace
        if let Some(ns) = self.namespaces.first() {
            BlockDeviceInfo {
                block_size: ns.block_size,
                total_blocks: ns.total_blocks,
                read_only: false,
                removable: false,
                optimal_io_size: 128 * 1024, // 128KB
            }
        } else {
            BlockDeviceInfo::default()
        }
    }
    
    async fn read_blocks(
        &mut self,
        start_block: u64,
        blocks: &mut [Block],
    ) -> Result<()> {
        // Use first I/O queue
        let queue = &mut self.io_queues[0];
        
        // Allocate DMA buffer
        let buffer_size = blocks.len() * Block::SIZE;
        let dma_buffer = DmaBuffer::allocate(buffer_size, self.dma_cap.clone())?;
        
        // Submit read command
        let cmd = NvmeCommand::read(1, start_block, blocks.len() as u16, dma_buffer);
        let completion = queue.submit_and_wait(cmd).await?;
        
        // Check status
        if completion.status() != 0 {
            return Err(DriverError::HardwareError);
        }
        
        // Copy data to blocks
        let data = dma_buffer.as_slice();
        for (i, block) in blocks.iter_mut().enumerate() {
            block.as_bytes_mut().copy_from_slice(
                &data[i * Block::SIZE..(i + 1) * Block::SIZE]
            );
        }
        
        Ok(())
    }
    
    async fn write_blocks(
        &mut self,
        start_block: u64,
        blocks: &[Block],
    ) -> Result<()> {
        // Use first I/O queue
        let queue = &mut self.io_queues[0];
        
        // Allocate and fill DMA buffer
        let buffer_size = blocks.len() * Block::SIZE;
        let mut dma_buffer = DmaBuffer::allocate(buffer_size, self.dma_cap.clone())?;
        
        let data = dma_buffer.as_mut_slice();
        for (i, block) in blocks.iter().enumerate() {
            data[i * Block::SIZE..(i + 1) * Block::SIZE]
                .copy_from_slice(block.as_bytes());
        }
        
        // Submit write command
        let cmd = NvmeCommand::write(1, start_block, blocks.len() as u16, dma_buffer);
        let completion = queue.submit_and_wait(cmd).await?;
        
        // Check status
        if completion.status() != 0 {
            return Err(DriverError::HardwareError);
        }
        
        Ok(())
    }
    
    async fn flush(&mut self) -> Result<()> {
        // Submit flush command
        let cmd = NvmeCommand {
            opcode: 0x00, // Flush
            flags: 0,
            cid: 0,
            nsid: 1,
            metadata: None,
            data: None,
            cdw10_15: [0; 6],
        };
        
        let completion = self.admin_queue.submit_and_wait(cmd).await?;
        
        if completion.status() != 0 {
            return Err(DriverError::HardwareError);
        }
        
        Ok(())
    }
}

/// Register driver
static DRIVER_METADATA: DriverMetadata = DriverMetadata {
    name: "nvme",
    version: "0.1.0",
    author: "Veridian OS Team",
    description: "NVMe storage driver",
    supported_devices: &[
        DeviceId { vendor: 0x1234, device: 0x5678 }, // Example device
    ],
};

register_driver!(DRIVER_METADATA, || {
    Ok(Box::new(NvmeDriver::new()))
});
```

## Network Device Drivers

### Intel E1000 Driver

Create `drivers/e1000/src/lib.rs`:

```rust
/// Intel E1000 network driver
pub struct E1000Driver {
    /// Memory mapped registers
    regs: E1000Registers,
    /// Receive descriptors
    rx_ring: DescriptorRing<RxDescriptor>,
    /// Transmit descriptors
    tx_ring: DescriptorRing<TxDescriptor>,
    /// MAC address
    mac_address: MacAddress,
    /// Link status
    link_status: LinkStatus,
    /// Statistics
    stats: NetworkStatistics,
}

mmio_struct! {
    /// E1000 registers
    pub struct E1000Registers {
        (0x0000 => ctrl: u32 [ReadWrite]),      // Device Control
        (0x0008 => status: u32 [ReadOnly]),     // Device Status
        (0x0010 => eecd: u32 [ReadWrite]),      // EEPROM Control
        (0x0100 => rctl: u32 [ReadWrite]),      // Receive Control
        (0x0400 => tctl: u32 [ReadWrite]),      // Transmit Control
        (0x2800 => rdbal: u32 [ReadWrite]),     // RX Descriptor Base Low
        (0x2804 => rdbah: u32 [ReadWrite]),     // RX Descriptor Base High
        (0x2808 => rdlen: u32 [ReadWrite]),     // RX Descriptor Length
        (0x2810 => rdh: u32 [ReadWrite]),       // RX Descriptor Head
        (0x2818 => rdt: u32 [ReadWrite]),       // RX Descriptor Tail
        (0x3800 => tdbal: u32 [ReadWrite]),     // TX Descriptor Base Low
        (0x3804 => tdbah: u32 [ReadWrite]),     // TX Descriptor Base High
        (0x3808 => tdlen: u32 [ReadWrite]),     // TX Descriptor Length
        (0x3810 => tdh: u32 [ReadWrite]),       // TX Descriptor Head
        (0x3818 => tdt: u32 [ReadWrite]),       // TX Descriptor Tail
        (0x5400 => ral0: u32 [ReadWrite]),      // Receive Address Low
        (0x5404 => rah0: u32 [ReadWrite]),      // Receive Address High
    }
}

/// Receive descriptor
#[repr(C, align(16))]
pub struct RxDescriptor {
    pub addr: u64,
    pub length: u16,
    pub checksum: u16,
    pub status: u8,
    pub errors: u8,
    pub special: u16,
}

/// Transmit descriptor
#[repr(C, align(16))]
pub struct TxDescriptor {
    pub addr: u64,
    pub length: u16,
    pub cso: u8,
    pub cmd: u8,
    pub status: u8,
    pub css: u8,
    pub special: u16,
}

/// Descriptor ring
pub struct DescriptorRing<T> {
    /// Ring entries
    entries: Vec<T>,
    /// DMA buffers
    buffers: Vec<DmaBuffer>,
    /// Current index
    index: AtomicU32,
    /// Ring size
    size: u32,
}

impl<T: Default + Clone> DescriptorRing<T> {
    pub fn new(size: u32) -> Self {
        Self {
            entries: vec![T::default(); size as usize],
            buffers: Vec::with_capacity(size as usize),
            index: AtomicU32::new(0),
            size,
        }
    }
    
    pub fn next_index(&self) -> u32 {
        self.index.fetch_add(1, Ordering::Relaxed) % self.size
    }
}

#[async_trait]
impl NetworkDevice for E1000Driver {
    fn info(&self) -> NetworkDeviceInfo {
        NetworkDeviceInfo {
            mac_address: self.mac_address,
            mtu: 1500,
            features: NetworkFeatures::CHECKSUM_IPV4 
                | NetworkFeatures::CHECKSUM_TCP 
                | NetworkFeatures::CHECKSUM_UDP,
            link_speed: if self.link_status == LinkStatus::Up { 1000 } else { 0 },
        }
    }
    
    async fn transmit(&mut self, packet: Packet) -> Result<()> {
        let index = self.tx_ring.next_index() as usize;
        
        // Get or allocate buffer
        if self.tx_ring.buffers.len() <= index {
            let buffer = DmaBuffer::allocate(2048, self.dma_cap.clone())?;
            self.tx_ring.buffers.push(buffer);
        }
        
        // Copy packet data
        let buffer = &mut self.tx_ring.buffers[index];
        buffer.as_mut_slice()[..packet.data.len()].copy_from_slice(&packet.data);
        
        // Setup descriptor
        self.tx_ring.entries[index] = TxDescriptor {
            addr: buffer.phys_addr(),
            length: packet.data.len() as u16,
            cso: 0,
            cmd: 0x0B, // EOP, IFCS, RS
            status: 0,
            css: 0,
            special: 0,
        };
        
        // Update tail register
        self.regs.tdt().write(((index + 1) % self.tx_ring.size as usize) as u32);
        
        // Wait for completion
        while self.tx_ring.entries[index].status & 0x01 == 0 {
            tokio::task::yield_now().await;
        }
        
        Ok(())
    }
    
    async fn receive(&mut self) -> Result<Packet> {
        loop {
            let index = self.rx_ring.index.load(Ordering::Relaxed) as usize;
            let desc = &self.rx_ring.entries[index];
            
            // Check if descriptor is ready
            if desc.status & 0x01 != 0 {
                // Extract packet
                let buffer = &self.rx_ring.buffers[index];
                let data = buffer.as_slice()[..desc.length as usize].to_vec();
                
                // Reset descriptor
                self.rx_ring.entries[index].status = 0;
                
                // Update tail
                self.regs.rdt().write(index as u32);
                
                // Update index
                self.rx_ring.index.store(
                    ((index + 1) % self.rx_ring.size as usize) as u32,
                    Ordering::Relaxed
                );
                
                return Ok(Packet {
                    data,
                    metadata: PacketMetadata {
                        timestamp: get_timestamp(),
                        checksum: ChecksumInfo::default(),
                        vlan: None,
                    },
                });
            }
            
            // Wait for packet
            tokio::task::yield_now().await;
        }
    }
}

impl E1000Driver {
    async fn init_hardware(&mut self) -> Result<()> {
        // Disable interrupts
        self.regs.write(0x00D0, 0xFFFFFFFF); // IMC
        
        // Reset device
        self.regs.ctrl().modify(|ctrl| ctrl | 0x04000000);
        tokio::time::sleep(Duration::from_millis(10)).await;
        
        // Setup receive ring
        let rx_ring_phys = self.rx_ring.entries.as_ptr() as u64;
        self.regs.rdbal().write(rx_ring_phys as u32);
        self.regs.rdbah().write((rx_ring_phys >> 32) as u32);
        self.regs.rdlen().write((self.rx_ring.size * 16) as u32);
        self.regs.rdh().write(0);
        self.regs.rdt().write(self.rx_ring.size - 1);
        
        // Setup transmit ring
        let tx_ring_phys = self.tx_ring.entries.as_ptr() as u64;
        self.regs.tdbal().write(tx_ring_phys as u32);
        self.regs.tdbah().write((tx_ring_phys >> 32) as u32);
        self.regs.tdlen().write((self.tx_ring.size * 16) as u32);
        self.regs.tdh().write(0);
        self.regs.tdt().write(0);
        
        // Enable receiver
        self.regs.rctl().write(0x04008002); // EN, BAM, BSIZE=2048
        
        // Enable transmitter
        self.regs.tctl().write(0x0103F0FA); // EN, PSP, CT=0x0F, COLD=0x3F
        
        Ok(())
    }
}
```

## Graphics Drivers

### Display Controller Interface

Create `libs/driver_framework/src/display.rs`:

```rust
/// Display driver interface
pub trait DisplayDriver: Driver {
    /// Get display information
    fn info(&self) -> DisplayInfo;
    
    /// Set display mode
    fn set_mode(&mut self, mode: DisplayMode) -> impl Future<Output = Result<()>>;
    
    /// Get supported modes
    fn supported_modes(&self) -> Vec<DisplayMode>;
    
    /// Create framebuffer
    fn create_framebuffer(
        &mut self,
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> impl Future<Output = Result<Framebuffer>>;
    
    /// Present framebuffer
    fn present(&mut self, fb: &Framebuffer) -> impl Future<Output = Result<()>>;
    
    /// Set cursor
    fn set_cursor(&mut self, cursor: Option<&Cursor>) -> impl Future<Output = Result<()>>;
}

/// Display information
#[derive(Debug, Clone)]
pub struct DisplayInfo {
    /// Display name
    pub name: String,
    /// Physical size in mm
    pub physical_size: (u32, u32),
    /// Current mode
    pub current_mode: DisplayMode,
    /// EDID data
    pub edid: Option<Vec<u8>>,
}

/// Display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMode {
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Refresh rate in Hz
    pub refresh_rate: u32,
    /// Pixel clock in kHz
    pub pixel_clock: u32,
    /// Timing information
    pub timing: DisplayTiming,
}

/// Display timing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayTiming {
    pub hsync_start: u32,
    pub hsync_end: u32,
    pub htotal: u32,
    pub vsync_start: u32,
    pub vsync_end: u32,
    pub vtotal: u32,
    pub flags: TimingFlags,
}

bitflags::bitflags! {
    pub struct TimingFlags: u32 {
        const HSYNC_POSITIVE = 0b0001;
        const VSYNC_POSITIVE = 0b0010;
        const INTERLACED     = 0b0100;
        const DOUBLESCAN     = 0b1000;
    }
}

/// Framebuffer
pub struct Framebuffer {
    /// Buffer ID
    pub id: u32,
    /// Width
    pub width: u32,
    /// Height
    pub height: u32,
    /// Stride in bytes
    pub stride: u32,
    /// Pixel format
    pub format: PixelFormat,
    /// DMA buffer
    pub buffer: DmaBuffer,
}

/// Pixel formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    RGB888,
    RGBA8888,
    BGR888,
    BGRA8888,
    RGB565,
    RGBA5551,
}

impl PixelFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::RGB888 | Self::BGR888 => 3,
            Self::RGBA8888 | Self::BGRA8888 => 4,
            Self::RGB565 | Self::RGBA5551 => 2,
        }
    }
}
```

### Simple Framebuffer Driver

Create `drivers/framebuffer/src/lib.rs`:

```rust
/// Simple framebuffer driver
pub struct FramebufferDriver {
    /// Framebuffer info from boot
    info: FramebufferInfo,
    /// Memory mapping
    mapping: MemoryMapping,
    /// Current mode
    current_mode: DisplayMode,
}

#[async_trait]
impl Driver for FramebufferDriver {
    fn name(&self) -> &str {
        "simple_framebuffer"
    }
    
    fn version(&self) -> &str {
        "0.1.0"
    }
    
    async fn init(&mut self) -> Result<()> {
        // Map framebuffer memory
        self.mapping = MemoryMapping::new(
            self.info.phys_addr,
            self.info.size,
            MemoryProtection::READ | MemoryProtection::WRITE,
        )?;
        
        // Set current mode based on boot info
        self.current_mode = DisplayMode {
            width: self.info.width as u32,
            height: self.info.height as u32,
            refresh_rate: 60, // Assume 60Hz
            pixel_clock: 0,   // Unknown
            timing: DisplayTiming::default(),
        };
        
        println!("Framebuffer: {}x{} @ {:?}",
                 self.info.width, self.info.height, self.info.format);
        
        Ok(())
    }
}

#[async_trait]
impl DisplayDriver for FramebufferDriver {
    fn info(&self) -> DisplayInfo {
        DisplayInfo {
            name: "Boot Framebuffer".to_string(),
            physical_size: (0, 0), // Unknown
            current_mode: self.current_mode,
            edid: None,
        }
    }
    
    async fn set_mode(&mut self, mode: DisplayMode) -> Result<()> {
        // Simple framebuffer doesn't support mode changes
        if mode != self.current_mode {
            return Err(DriverError::NotSupported);
        }
        Ok(())
    }
    
    fn supported_modes(&self) -> Vec<DisplayMode> {
        vec![self.current_mode]
    }
    
    async fn create_framebuffer(
        &mut self,
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Result<Framebuffer> {
        if width != self.current_mode.width || height != self.current_mode.height {
            return Err(DriverError::InvalidParameter);
        }
        
        let size = (width * height * format.bytes_per_pixel() as u32) as usize;
        let buffer = DmaBuffer::allocate(size, self.dma_cap.clone())?;
        
        Ok(Framebuffer {
            id: 0,
            width,
            height,
            stride: width * format.bytes_per_pixel() as u32,
            format,
            buffer,
        })
    }
    
    async fn present(&mut self, fb: &Framebuffer) -> Result<()> {
        // Copy framebuffer to display
        let src = fb.buffer.as_slice();
        let dst = self.mapping.as_mut_slice();
        
        let copy_size = src.len().min(dst.len());
        dst[..copy_size].copy_from_slice(&src[..copy_size]);
        
        Ok(())
    }
}
```

## USB Stack Implementation

### USB Core Framework

Create `libs/usb_core/src/lib.rs`:

```rust
/// USB device state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceState {
    Attached,
    Powered,
    Default,
    Address,
    Configured,
    Suspended,
}

/// USB endpoint
pub struct UsbEndpoint {
    /// Endpoint number
    pub number: u8,
    /// Direction
    pub direction: EndpointDirection,
    /// Transfer type
    pub transfer_type: TransferType,
    /// Maximum packet size
    pub max_packet_size: u16,
    /// Interval (for interrupt/isochronous)
    pub interval: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointDirection {
    In,
    Out,
}

/// USB interface
pub struct UsbInterface {
    /// Interface number
    pub number: u8,
    /// Alternate setting
    pub alternate: u8,
    /// Interface class
    pub class: u8,
    /// Interface subclass
    pub subclass: u8,
    /// Interface protocol
    pub protocol: u8,
    /// Endpoints
    pub endpoints: Vec<UsbEndpoint>,
}

/// USB configuration
pub struct UsbConfiguration {
    /// Configuration value
    pub value: u8,
    /// Attributes
    pub attributes: u8,
    /// Maximum power (in 2mA units)
    pub max_power: u8,
    /// Interfaces
    pub interfaces: Vec<UsbInterface>,
}

/// USB device driver
pub trait UsbDeviceDriver: Send + Sync {
    /// Probe device
    fn probe(&self, device: &UsbDeviceDescriptor) -> bool;
    
    /// Attach to device
    fn attach(&mut self, device: UsbDeviceHandle) -> impl Future<Output = Result<()>>;
    
    /// Detach from device
    fn detach(&mut self) -> impl Future<Output = Result<()>>;
}

/// USB hub driver
pub struct UsbHubDriver {
    /// Hub descriptor
    descriptor: HubDescriptor,
    /// Port status
    port_status: Vec<PortStatus>,
    /// Child devices
    children: Vec<Option<UsbDevice>>,
}

/// Hub descriptor
#[repr(C, packed)]
pub struct HubDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub num_ports: u8,
    pub characteristics: u16,
    pub power_on_to_good: u8,
    pub current: u8,
    // Variable length fields follow
}

impl UsbHubDriver {
    async fn poll_ports(&mut self) -> Result<()> {
        for port in 0..self.descriptor.num_ports {
            let status = self.get_port_status(port).await?;
            
            if status.changed() {
                if status.connected() && self.children[port as usize].is_none() {
                    // New device connected
                    self.handle_connect(port).await?;
                } else if !status.connected() && self.children[port as usize].is_some() {
                    // Device disconnected
                    self.handle_disconnect(port).await?;
                }
                
                // Clear change bits
                self.clear_port_feature(port, PortFeature::ConnectChange).await?;
            }
        }
        
        Ok(())
    }
    
    async fn handle_connect(&mut self, port: u8) -> Result<()> {
        // Reset port
        self.set_port_feature(port, PortFeature::Reset).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Clear reset
        self.clear_port_feature(port, PortFeature::ResetChange).await?;
        
        // Get speed
        let status = self.get_port_status(port).await?;
        let speed = match (status.low_speed(), status.high_speed()) {
            (true, false) => UsbSpeed::Low,
            (false, false) => UsbSpeed::Full,
            (false, true) => UsbSpeed::High,
            _ => return Err(DriverError::InvalidDevice),
        };
        
        // Enumerate device
        let device = self.enumerate_device(port, speed).await?;
        self.children[port as usize] = Some(device);
        
        Ok(())
    }
}
```

### USB Mass Storage Driver

Create `drivers/usb_storage/src/lib.rs`:

```rust
/// USB mass storage driver
pub struct UsbStorageDriver {
    /// Device handle
    device: UsbDeviceHandle,
    /// Bulk IN endpoint
    bulk_in: u8,
    /// Bulk OUT endpoint
    bulk_out: u8,
    /// Maximum LUN
    max_lun: u8,
}

/// SCSI command
pub struct ScsiCommand {
    /// Command bytes
    pub cdb: Vec<u8>,
    /// Data direction
    pub direction: DataDirection,
    /// Expected data length
    pub data_length: u32,
}

impl ScsiCommand {
    /// INQUIRY command
    pub fn inquiry() -> Self {
        Self {
            cdb: vec![0x12, 0, 0, 0, 36, 0],
            direction: DataDirection::In,
            data_length: 36,
        }
    }
    
    /// READ CAPACITY command
    pub fn read_capacity() -> Self {
        Self {
            cdb: vec![0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            direction: DataDirection::In,
            data_length: 8,
        }
    }
    
    /// READ(10) command
    pub fn read10(lba: u32, blocks: u16) -> Self {
        let mut cdb = vec![0x28, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[7..9].copy_from_slice(&blocks.to_be_bytes());
        
        Self {
            cdb,
            direction: DataDirection::In,
            data_length: blocks as u32 * 512,
        }
    }
}

/// Command Block Wrapper (CBW)
#[repr(C, packed)]
struct CommandBlockWrapper {
    signature: u32,
    tag: u32,
    data_length: u32,
    flags: u8,
    lun: u8,
    cdb_length: u8,
    cdb: [u8; 16],
}

impl CommandBlockWrapper {
    const SIGNATURE: u32 = 0x43425355; // "USBC"
    
    fn new(tag: u32, data_length: u32, direction: DataDirection, lun: u8, cdb: &[u8]) -> Self {
        let mut cbw = Self {
            signature: Self::SIGNATURE.to_le(),
            tag: tag.to_le(),
            data_length: data_length.to_le(),
            flags: if direction == DataDirection::In { 0x80 } else { 0x00 },
            lun,
            cdb_length: cdb.len() as u8,
            cdb: [0; 16],
        };
        
        cbw.cdb[..cdb.len()].copy_from_slice(cdb);
        cbw
    }
}

#[async_trait]
impl UsbStorageDriver {
    async fn execute_scsi(&mut self, lun: u8, cmd: &ScsiCommand) -> Result<Vec<u8>> {
        static TAG: AtomicU32 = AtomicU32::new(1);
        let tag = TAG.fetch_add(1, Ordering::Relaxed);
        
        // Send CBW
        let cbw = CommandBlockWrapper::new(
            tag,
            cmd.data_length,
            cmd.direction,
            lun,
            &cmd.cdb,
        );
        
        self.device.bulk_transfer(
            self.bulk_out,
            unsafe { as_bytes(&cbw) },
            Duration::from_secs(5),
        ).await?;
        
        // Transfer data
        let mut data = vec![0u8; cmd.data_length as usize];
        if cmd.data_length > 0 {
            match cmd.direction {
                DataDirection::In => {
                    self.device.bulk_transfer(
                        self.bulk_in,
                        &mut data,
                        Duration::from_secs(30),
                    ).await?;
                }
                DataDirection::Out => {
                    self.device.bulk_transfer(
                        self.bulk_out,
                        &data,
                        Duration::from_secs(30),
                    ).await?;
                }
            }
        }
        
        // Receive CSW
        let mut csw = CommandStatusWrapper::default();
        self.device.bulk_transfer(
            self.bulk_in,
            unsafe { as_bytes_mut(&mut csw) },
            Duration::from_secs(5),
        ).await?;
        
        // Check status
        if csw.signature != CommandStatusWrapper::SIGNATURE.to_le() {
            return Err(DriverError::InvalidDevice);
        }
        
        if csw.tag != tag.to_le() {
            return Err(DriverError::InvalidDevice);
        }
        
        if csw.status != 0 {
            return Err(DriverError::HardwareError);
        }
        
        Ok(data)
    }
}

#[async_trait]
impl BlockDevice for UsbStorageDriver {
    async fn read_blocks(
        &mut self,
        start_block: u64,
        blocks: &mut [Block],
    ) -> Result<()> {
        // USB storage typically uses 512-byte blocks
        let data = self.execute_scsi(
            0,
            &ScsiCommand::read10(start_block as u32, blocks.len() as u16),
        ).await?;
        
        for (i, block) in blocks.iter_mut().enumerate() {
            block.as_bytes_mut().copy_from_slice(
                &data[i * 512..(i + 1) * 512]
            );
        }
        
        Ok(())
    }
}
```

## Driver Testing and Debugging

### Test Framework

Create `libs/driver_test/src/lib.rs`:

```rust
/// Driver test harness
pub struct DriverTestHarness {
    /// Mock hardware
    mock_hw: MockHardware,
    /// Driver under test
    driver: Box<dyn Driver>,
    /// Test results
    results: TestResults,
}

impl DriverTestHarness {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        Self {
            mock_hw: MockHardware::new(),
            driver,
            results: TestResults::default(),
        }
    }
    
    /// Run all tests
    pub async fn run_tests(&mut self) -> TestResults {
        self.test_init().await;
        self.test_probe().await;
        self.test_attach_detach().await;
        self.test_power_management().await;
        self.test_error_handling().await;
        
        self.results.clone()
    }
    
    async fn test_init(&mut self) {
        let start = Instant::now();
        
        match self.driver.init().await {
            Ok(()) => {
                self.results.add_pass("init", start.elapsed());
            }
            Err(e) => {
                self.results.add_fail("init", format!("Init failed: {:?}", e));
            }
        }
    }
}

/// Mock hardware for testing
pub struct MockHardware {
    /// MMIO regions
    mmio: HashMap<u64, Vec<u8>>,
    /// Port I/O
    ports: HashMap<u16, u8>,
    /// Interrupts
    interrupts: Vec<u32>,
    /// DMA buffers
    dma_buffers: HashMap<u64, Vec<u8>>,
}

impl MockHardware {
    pub fn add_mmio_region(&mut self, addr: u64, size: usize) {
        self.mmio.insert(addr, vec![0; size]);
    }
    
    pub fn read_mmio<T>(&self, addr: u64) -> T
    where
        T: Copy,
    {
        let region = self.mmio.get(&(addr & !0xFFF)).unwrap();
        let offset = (addr & 0xFFF) as usize;
        
        unsafe {
            *(region.as_ptr().add(offset) as *const T)
        }
    }
    
    pub fn write_mmio<T>(&mut self, addr: u64, value: T)
    where
        T: Copy,
    {
        let region = self.mmio.get_mut(&(addr & !0xFFF)).unwrap();
        let offset = (addr & 0xFFF) as usize;
        
        unsafe {
            *(region.as_mut_ptr().add(offset) as *mut T) = value;
        }
    }
}

/// Test results
#[derive(Debug, Clone, Default)]
pub struct TestResults {
    pub passed: Vec<TestResult>,
    pub failed: Vec<TestResult>,
    pub total_time: Duration,
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub duration: Duration,
    pub error: Option<String>,
}
```

### Driver Debugging Tools

```rust
/// Driver debug interface
pub trait DriverDebug {
    /// Dump driver state
    fn dump_state(&self) -> String;
    
    /// Get statistics
    fn statistics(&self) -> DriverStatistics;
    
    /// Inject error for testing
    fn inject_error(&mut self, error: ErrorInjection);
    
    /// Enable debug logging
    fn set_debug_level(&mut self, level: DebugLevel);
}

/// Driver statistics
#[derive(Debug, Default)]
pub struct DriverStatistics {
    pub operations: u64,
    pub errors: u64,
    pub bytes_transferred: u64,
    pub interrupts: u64,
    pub dma_transfers: u64,
}

/// Error injection
#[derive(Debug, Clone)]
pub enum ErrorInjection {
    /// Simulate hardware timeout
    Timeout { operation: String, delay: Duration },
    /// Simulate hardware error
    HardwareError { register: u64, value: u64 },
    /// Simulate DMA error
    DmaError { address: u64 },
    /// Simulate interrupt storm
    InterruptStorm { irq: u32, count: u32 },
}

/// Debug tracing
#[macro_export]
macro_rules! driver_trace {
    ($level:expr, $($arg:tt)*) => {
        if $crate::debug_enabled($level) {
            println!("[{}:{}] {}", module_path!(), line!(), format_args!($($arg)*));
        }
    };
}
```

## Performance Optimization

### DMA Optimization

```rust
/// Optimized DMA operations
pub struct DmaOptimizer {
    /// Scatter-gather support
    sg_enabled: bool,
    /// Maximum SG entries
    max_sg_entries: usize,
    /// DMA alignment requirements
    alignment: usize,
    /// Cache line size
    cache_line_size: usize,
}

impl DmaOptimizer {
    /// Optimize buffer for DMA
    pub fn optimize_buffer(&self, buffer: &mut [u8]) -> DmaStrategy {
        // Check alignment
        let addr = buffer.as_ptr() as usize;
        let aligned = addr % self.alignment == 0;
        
        // Check cache line alignment
        let cache_aligned = addr % self.cache_line_size == 0;
        
        // Determine strategy
        if aligned && cache_aligned {
            DmaStrategy::Direct
        } else if self.sg_enabled {
            DmaStrategy::ScatterGather
        } else {
            DmaStrategy::Bounce
        }
    }
    
    /// Create scatter-gather list
    pub fn create_sg_list(&self, buffer: &[u8]) -> ScatterGatherList {
        let mut sg_list = ScatterGatherList::new();
        
        // Align to page boundaries for efficiency
        let start = buffer.as_ptr() as usize;
        let end = start + buffer.len();
        
        let first_page = start & !0xFFF;
        let last_page = (end - 1) & !0xFFF;
        
        // First partial page
        if start != first_page {
            let len = (first_page + 0x1000 - start).min(buffer.len());
            sg_list.add_entry(start as u64, len);
        }
        
        // Full pages
        for page in ((first_page + 0x1000)..=last_page).step_by(0x1000) {
            let offset = page - start;
            let len = 0x1000.min(buffer.len() - offset);
            sg_list.add_entry(page as u64, len);
        }
        
        sg_list
    }
}

pub enum DmaStrategy {
    Direct,
    ScatterGather,
    Bounce,
}
```

### Interrupt Mitigation

```rust
/// Interrupt mitigation strategies
pub struct InterruptMitigation {
    /// Current strategy
    strategy: MitigationStrategy,
    /// Packet rate threshold
    rate_threshold: u32,
    /// CPU usage threshold
    cpu_threshold: f32,
    /// Statistics
    stats: InterruptStats,
}

#[derive(Debug, Clone, Copy)]
pub enum MitigationStrategy {
    /// No mitigation
    None,
    /// Fixed coalescing
    Fixed { packets: u32, time_us: u32 },
    /// Adaptive coalescing
    Adaptive,
    /// Polling mode
    Polling,
}

impl InterruptMitigation {
    pub fn update(&mut self, packet_rate: u32, cpu_usage: f32) {
        match self.strategy {
            MitigationStrategy::Adaptive => {
                if packet_rate > 100_000 && cpu_usage > 0.5 {
                    // Switch to polling
                    self.strategy = MitigationStrategy::Polling;
                } else if packet_rate > 50_000 {
                    // Increase coalescing
                    self.strategy = MitigationStrategy::Fixed {
                        packets: 32,
                        time_us: 100,
                    };
                } else {
                    // Low rate, minimize latency
                    self.strategy = MitigationStrategy::Fixed {
                        packets: 1,
                        time_us: 0,
                    };
                }
            }
            _ => {}
        }
    }
}
```

## Security Considerations

### Driver Isolation

```rust
/// Driver security context
pub struct DriverSecurityContext {
    /// Allowed capabilities
    capabilities: CapabilitySet,
    /// Memory access restrictions
    memory_policy: MemoryAccessPolicy,
    /// I/O access restrictions
    io_policy: IoAccessPolicy,
    /// Rate limiting
    rate_limits: RateLimits,
}

/// Memory access policy
pub struct MemoryAccessPolicy {
    /// Allowed physical memory regions
    allowed_regions: Vec<(PhysAddr, usize)>,
    /// Maximum allocation size
    max_allocation: usize,
    /// Total memory limit
    memory_limit: usize,
}

/// I/O access policy
pub struct IoAccessPolicy {
    /// Allowed MMIO regions
    allowed_mmio: Vec<(PhysAddr, usize)>,
    /// Allowed port I/O ranges
    allowed_ports: Vec<(u16, u16)>,
    /// DMA restrictions
    dma_policy: DmaPolicy,
}

/// DMA policy
pub struct DmaPolicy {
    /// Use IOMMU
    require_iommu: bool,
    /// Allowed DMA addresses
    allowed_addresses: Vec<(PhysAddr, usize)>,
    /// Maximum DMA buffer size
    max_buffer_size: usize,
}

/// Rate limiting
pub struct RateLimits {
    /// Maximum operations per second
    max_ops_per_sec: u32,
    /// Maximum bandwidth
    max_bandwidth: u64,
    /// Burst allowance
    burst_size: u32,
}

impl DriverSecurityContext {
    /// Check memory access
    pub fn check_memory_access(&self, addr: PhysAddr, size: usize) -> Result<()> {
        for (base, len) in &self.memory_policy.allowed_regions {
            if addr >= *base && addr + size <= *base + len {
                return Ok(());
            }
        }
        
        Err(DriverError::PermissionDenied)
    }
    
    /// Check I/O access
    pub fn check_io_access(&self, port: u16) -> Result<()> {
        for (start, end) in &self.io_policy.allowed_ports {
            if port >= *start && port <= *end {
                return Ok(());
            }
        }
        
        Err(DriverError::PermissionDenied)
    }
}
```

### Input Validation

```rust
/// Input validation helpers
pub mod validation {
    /// Validate DMA buffer
    pub fn validate_dma_buffer(buffer: &DmaBuffer, min_size: usize, max_size: usize) -> Result<()> {
        if buffer.size() < min_size || buffer.size() > max_size {
            return Err(DriverError::InvalidParameter);
        }
        
        // Check alignment
        if buffer.phys_addr() % 16 != 0 {
            return Err(DriverError::InvalidParameter);
        }
        
        Ok(())
    }
    
    /// Validate device configuration
    pub fn validate_config<T: DeviceConfig>(config: &T) -> Result<()> {
        config.validate()
    }
    
    /// Sanitize user input
    pub fn sanitize_string(input: &str, max_len: usize) -> String {
        input.chars()
            .filter(|c| c.is_ascii_graphic() || c.is_ascii_whitespace())
            .take(max_len)
            .collect()
    }
}
```

## Driver Development Workflow

### Development Process

1. **Requirements Analysis**
   - Hardware specifications
   - Feature requirements
   - Performance targets
   - Security requirements

2. **Design Phase**
   - Driver architecture
   - Interface design
   - State machine design
   - Error handling strategy

3. **Implementation**
   - Core functionality
   - Hardware initialization
   - Interrupt handling
   - DMA setup

4. **Testing**
   - Unit tests
   - Integration tests
   - Hardware testing
   - Performance testing

5. **Optimization**
   - Profile performance
   - Optimize hot paths
   - Reduce memory usage
   - Improve latency

6. **Documentation**
   - API documentation
   - Hardware notes
   - Known issues
   - Performance characteristics

### Template for New Driver

```rust
//! Driver for [Device Name]
//! 
//! This driver implements support for [device description].

#![no_std]
#![no_main]

use driver_framework::prelude::*;
use log::{info, error, debug};

/// Driver state
pub struct MyDriver {
    /// Device handle
    device: DeviceHandle,
    /// Hardware registers
    regs: MyDeviceRegisters,
    /// Driver state
    state: DriverState,
    /// Statistics
    stats: DriverStatistics,
}

/// Driver state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriverState {
    Uninitialized,
    Initializing,
    Ready,
    Error(DriverError),
}

/// Hardware registers
mmio_struct! {
    pub struct MyDeviceRegisters {
        (0x00 => control: u32 [ReadWrite]),
        (0x04 => status: u32 [ReadOnly]),
        // Add more registers
    }
}

#[async_trait]
impl Driver for MyDriver {
    fn name(&self) -> &str {
        "my_driver"
    }
    
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    
    async fn init(&mut self) -> Result<()> {
        info!("Initializing {} driver", self.name());
        
        self.state = DriverState::Initializing;
        
        // Hardware initialization
        self.reset_hardware().await?;
        self.configure_hardware().await?;
        
        self.state = DriverState::Ready;
        info!("{} driver initialized successfully", self.name());
        
        Ok(())
    }
    
    async fn probe(&mut self, device: &DeviceInfo) -> Result<bool> {
        // Check if we support this device
        Ok(device.vendor_id == 0x1234 && device.device_id == 0x5678)
    }
    
    async fn attach(&mut self, device: DeviceHandle) -> Result<()> {
        self.device = device;
        
        // Map registers
        let mmio_cap = self.device.mmio_cap.as_ref()
            .ok_or(DriverError::InvalidDevice)?;
        
        // Setup hardware access
        
        Ok(())
    }
    
    async fn detach(&mut self) -> Result<()> {
        // Cleanup
        self.state = DriverState::Uninitialized;
        Ok(())
    }
}

impl MyDriver {
    /// Create new driver instance
    pub fn new() -> Self {
        Self {
            device: DeviceHandle::invalid(),
            regs: unsafe { MyDeviceRegisters::new(0) },
            state: DriverState::Uninitialized,
            stats: DriverStatistics::default(),
        }
    }
    
    /// Reset hardware
    async fn reset_hardware(&mut self) -> Result<()> {
        debug!("Resetting hardware");
        
        // Issue reset
        self.regs.control().write(0x01);
        
        // Wait for reset complete
        let timeout = Duration::from_millis(100);
        let start = Instant::now();
        
        while self.regs.status().read() & 0x01 != 0 {
            if start.elapsed() > timeout {
                error!("Hardware reset timeout");
                return Err(DriverError::Timeout);
            }
            
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        
        Ok(())
    }
    
    /// Configure hardware
    async fn configure_hardware(&mut self) -> Result<()> {
        debug!("Configuring hardware");
        
        // Configure registers
        // ...
        
        Ok(())
    }
}

// Register driver
static DRIVER_METADATA: DriverMetadata = DriverMetadata {
    name: "my_driver",
    version: env!("CARGO_PKG_VERSION"),
    author: env!("CARGO_PKG_AUTHORS"),
    description: env!("CARGO_PKG_DESCRIPTION"),
    supported_devices: &[
        DeviceId { vendor: 0x1234, device: 0x5678 },
    ],
};

register_driver!(DRIVER_METADATA, || {
    Ok(Box::new(MyDriver::new()))
});

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_driver_init() {
        let mut driver = MyDriver::new();
        
        // Mock hardware setup
        // ...
        
        assert!(driver.init().await.is_ok());
        assert_eq!(driver.state, DriverState::Ready);
    }
}
```

## Conclusion

This comprehensive driver development guide provides the foundation for building robust, efficient, and secure drivers for Veridian OS. Key takeaways:

1. **Safety First**: Use Rust's type system to prevent driver bugs
2. **Userspace Isolation**: Run drivers in userspace when possible
3. **Capability Security**: Control hardware access through capabilities
4. **Performance**: Optimize for zero-copy I/O and efficient DMA
5. **Testability**: Design drivers with testing in mind

By following these patterns and guidelines, you can create drivers that are:
- **Safe**: Memory and type safe by default
- **Fast**: Optimized for modern hardware
- **Secure**: Isolated and capability-controlled
- **Maintainable**: Well-structured and documented

The driver framework provides the building blocks, but each driver will have unique requirements based on the hardware it controls. Use this guide as a starting point and adapt the patterns to your specific needs.
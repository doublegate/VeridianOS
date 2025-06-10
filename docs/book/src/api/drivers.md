# Driver API

VeridianOS implements a user-space driver model with capability-based access control and isolation. This API reference covers the framework for developing secure, high-performance drivers.

## Overview

### Design Principles

1. **User-Space Isolation**: Drivers run in separate processes for fault tolerance
2. **Capability-Based Access**: Hardware access requires explicit capabilities
3. **Zero-Copy I/O**: Minimize data movement for optimal performance
4. **Async-First**: Built on Rust's async ecosystem
5. **Hot-Plug Support**: Dynamic device addition and removal

### Driver Architecture

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

## Core Driver Framework

### Base Driver Trait

```rust
/// Core driver interface that all drivers must implement
#[async_trait]
pub trait Driver: Send + Sync {
    /// Driver name and version information
    fn info(&self) -> DriverInfo;
    
    /// Initialize the driver with hardware capabilities
    async fn init(&mut self, caps: HardwareCapabilities) -> Result<()>;
    
    /// Start driver operations
    async fn start(&mut self) -> Result<()>;
    
    /// Stop driver operations gracefully
    async fn stop(&mut self) -> Result<()>;
    
    /// Handle power management events
    async fn power_event(&mut self, event: PowerEvent) -> Result<()>;
    
    /// Handle hot-plug events
    async fn device_event(&mut self, event: DeviceEvent) -> Result<()>;
}

/// Driver metadata
pub struct DriverInfo {
    pub name: &'static str,
    pub version: Version,
    pub vendor: &'static str,
    pub device_types: &'static [DeviceType],
    pub capabilities_required: &'static [CapabilityType],
}
```

### Hardware Capabilities

```rust
/// Hardware access capabilities
pub struct HardwareCapabilities {
    /// Memory-mapped I/O regions
    pub mmio_regions: Vec<MmioRegion>,
    
    /// Port I/O access (x86 only)
    pub port_ranges: Vec<PortRange>,
    
    /// Interrupt vectors
    pub interrupts: Vec<InterruptLine>,
    
    /// DMA capabilities
    pub dma_capability: Option<DmaCapability>,
    
    /// PCI configuration access
    pub pci_access: Option<PciCapability>,
}

/// Memory-mapped I/O region
pub struct MmioRegion {
    pub base: PhysAddr,
    pub size: usize,
    pub access: MmioAccess,
    pub cacheable: bool,
}

/// Port I/O range (x86)
pub struct PortRange {
    pub base: u16,
    pub size: u16,
    pub access: PortAccess,
}
```

## Device Types

### Block Device Interface

```rust
/// Block device driver interface
#[async_trait]
pub trait BlockDevice: Driver {
    /// Get device geometry
    fn geometry(&self) -> BlockGeometry;
    
    /// Read blocks asynchronously
    async fn read_blocks(
        &self,
        start_lba: u64,
        buffer: DmaBuffer,
        count: u32,
    ) -> Result<()>;
    
    /// Write blocks asynchronously
    async fn write_blocks(
        &self,
        start_lba: u64,
        buffer: DmaBuffer,
        count: u32,
    ) -> Result<()>;
    
    /// Flush write cache
    async fn flush(&self) -> Result<()>;
    
    /// Get device status
    fn status(&self) -> DeviceStatus;
}

/// Block device geometry
pub struct BlockGeometry {
    pub block_size: u32,
    pub total_blocks: u64,
    pub max_transfer_blocks: u32,
    pub alignment: u32,
}
```

### Network Device Interface

```rust
/// Network device driver interface
#[async_trait]
pub trait NetworkDevice: Driver {
    /// Get MAC address
    fn mac_address(&self) -> MacAddress;
    
    /// Get link status
    fn link_status(&self) -> LinkStatus;
    
    /// Set promiscuous mode
    async fn set_promiscuous(&mut self, enable: bool) -> Result<()>;
    
    /// Send packet
    async fn send_packet(&self, packet: NetworkPacket) -> Result<()>;
    
    /// Receive packet (called by framework)
    async fn packet_received(&mut self, packet: NetworkPacket) -> Result<()>;
    
    /// Get statistics
    fn statistics(&self) -> NetworkStatistics;
}

/// Network packet representation
pub struct NetworkPacket {
    pub buffer: DmaBuffer,
    pub length: usize,
    pub timestamp: Instant,
    pub flags: PacketFlags,
}
```

## Memory Management

### DMA Operations

```rust
/// DMA buffer management
pub struct DmaBuffer {
    virtual_addr: VirtAddr,
    physical_addr: PhysAddr,
    size: usize,
    direction: DmaDirection,
}

impl DmaBuffer {
    /// Allocate DMA-coherent buffer
    pub fn alloc_coherent(size: usize, direction: DmaDirection) -> Result<Self>;
    
    /// Map existing memory for DMA
    pub fn map_memory(
        buffer: &[u8],
        direction: DmaDirection,
    ) -> Result<Self>;
    
    /// Synchronize buffer (for non-coherent DMA)
    pub fn sync(&self, sync_type: DmaSyncType);
    
    /// Get physical address for hardware
    pub fn physical_addr(&self) -> PhysAddr;
    
    /// Get virtual address for CPU access
    pub fn as_slice(&self) -> &[u8];
    
    /// Get mutable slice (write/bidirectional only)
    pub fn as_mut_slice(&mut self) -> Option<&mut [u8]>;
}

/// DMA direction
#[derive(Clone, Copy)]
pub enum DmaDirection {
    ToDevice,
    FromDevice,
    Bidirectional,
}
```

## Interrupt Handling

### Interrupt Management

```rust
/// Interrupt handler interface
#[async_trait]
pub trait InterruptHandler: Send + Sync {
    /// Handle interrupt
    async fn handle_interrupt(&self, vector: u32) -> Result<()>;
}

/// Register interrupt handler
pub async fn register_interrupt_handler(
    vector: u32,
    handler: Box<dyn InterruptHandler>,
    flags: InterruptFlags,
) -> Result<InterruptHandle>;

/// Interrupt registration flags
#[derive(Clone, Copy)]
pub struct InterruptFlags {
    pub shared: bool,
    pub edge_triggered: bool,
    pub active_low: bool,
}
```

## Bus Interfaces

### PCI Device Access

```rust
/// PCI device interface
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    capability: PciCapability,
}

impl PciDevice {
    /// Read PCI configuration space
    pub fn config_read_u32(&self, offset: u8) -> Result<u32>;
    
    /// Write PCI configuration space
    pub fn config_write_u32(&self, offset: u8, value: u32) -> Result<()>;
    
    /// Enable bus mastering
    pub fn enable_bus_mastering(&self) -> Result<()>;
    
    /// Get BAR information
    pub fn get_bar(&self, bar: u8) -> Result<PciBar>;
    
    /// Find capability
    pub fn find_capability(&self, cap_id: u8) -> Option<u8>;
}

/// PCI Base Address Register
pub enum PciBar {
    Memory {
        base: PhysAddr,
        size: usize,
        prefetchable: bool,
        address_64bit: bool,
    },
    Io {
        base: u16,
        size: u16,
    },
}
```

## Driver Registration

### Device Manager Integration

```rust
/// Register a driver with the device manager
pub async fn register_driver(
    driver: Box<dyn Driver>,
    device_matcher: DeviceMatcher,
) -> Result<DriverHandle>;

/// Device matching criteria
pub struct DeviceMatcher {
    pub vendor_id: Option<u16>,
    pub device_id: Option<u16>,
    pub class_code: Option<u8>,
    pub subclass: Option<u8>,
    pub interface: Option<u8>,
    pub custom_match: Option<Box<dyn Fn(&DeviceInfo) -> bool>>,
}

/// Driver handle for management
pub struct DriverHandle {
    id: DriverId,
    // Internal management fields
}

impl DriverHandle {
    /// Unregister the driver
    pub async fn unregister(self) -> Result<()>;
    
    /// Get driver statistics
    pub fn statistics(&self) -> DriverStatistics;
}
```

## Error Handling

### Driver Error Types

```rust
/// Comprehensive driver error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// Hardware not found or not responding
    HardwareNotFound,
    
    /// Insufficient capabilities
    InsufficientCapabilities,
    
    /// Hardware initialization failed
    InitializationFailed,
    
    /// Operation timeout
    Timeout,
    
    /// DMA operation failed
    DmaError,
    
    /// Interrupt registration failed
    InterruptError,
    
    /// Device is busy
    DeviceBusy,
    
    /// Invalid parameter
    InvalidParameter,
    
    /// Resource exhaustion
    OutOfResources,
    
    /// Hardware error
    HardwareError,
}
```

## Performance Optimization

### Best Practices

1. **Use Zero-Copy I/O**: Leverage DMA buffers for large transfers
2. **Batch Operations**: Group small operations when possible
3. **Async Design**: Use async/await for non-blocking operations
4. **Interrupt Coalescing**: Reduce interrupt frequency for bulk operations
5. **Memory Locality**: Keep frequently accessed data in cache-friendly layouts

### Performance Monitoring

```rust
/// Driver performance metrics
pub struct DriverStatistics {
    pub operations_completed: u64,
    pub bytes_transferred: u64,
    pub errors_encountered: u64,
    pub average_latency_ns: u64,
    pub peak_bandwidth_mbps: u32,
}
```

## Example Implementation

### Simple Block Driver

```rust
use veridian_driver_framework::*;

pub struct RamDiskDriver {
    storage: Vec<u8>,
    block_size: u32,
    total_blocks: u64,
}

#[async_trait]
impl Driver for RamDiskDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            name: "RAM Disk Driver",
            version: Version::new(1, 0, 0),
            vendor: "VeridianOS",
            device_types: &[DeviceType::Block],
            capabilities_required: &[CapabilityType::Memory],
        }
    }
    
    async fn init(&mut self, caps: HardwareCapabilities) -> Result<()> {
        // Initialize RAM disk
        self.storage = vec![0; (self.total_blocks * self.block_size as u64) as usize];
        Ok(())
    }
    
    async fn start(&mut self) -> Result<()> {
        // Register with block device manager
        Ok(())
    }
    
    async fn stop(&mut self) -> Result<()> {
        // Clean shutdown
        Ok(())
    }
    
    async fn power_event(&mut self, event: PowerEvent) -> Result<()> {
        // Handle power management
        Ok(())
    }
    
    async fn device_event(&mut self, event: DeviceEvent) -> Result<()> {
        // Handle hot-plug events
        Ok(())
    }
}

#[async_trait]
impl BlockDevice for RamDiskDriver {
    fn geometry(&self) -> BlockGeometry {
        BlockGeometry {
            block_size: self.block_size,
            total_blocks: self.total_blocks,
            max_transfer_blocks: 256,
            alignment: 1,
        }
    }
    
    async fn read_blocks(
        &self,
        start_lba: u64,
        buffer: DmaBuffer,
        count: u32,
    ) -> Result<()> {
        let start_offset = (start_lba * self.block_size as u64) as usize;
        let size = (count * self.block_size) as usize;
        
        let data = &self.storage[start_offset..start_offset + size];
        buffer.as_mut_slice().unwrap().copy_from_slice(data);
        
        Ok(())
    }
    
    async fn write_blocks(
        &self,
        start_lba: u64,
        buffer: DmaBuffer,
        count: u32,
    ) -> Result<()> {
        let start_offset = (start_lba * self.block_size as u64) as usize;
        let size = (count * self.block_size) as usize;
        
        self.storage[start_offset..start_offset + size]
            .copy_from_slice(buffer.as_slice());
        
        Ok(())
    }
    
    async fn flush(&self) -> Result<()> {
        // No-op for RAM disk
        Ok(())
    }
    
    fn status(&self) -> DeviceStatus {
        DeviceStatus::Ready
    }
}
```

This driver API provides a comprehensive framework for developing secure, high-performance drivers in VeridianOS while maintaining the safety and isolation guarantees of the microkernel architecture.
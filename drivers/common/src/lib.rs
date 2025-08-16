//! Common driver framework for VeridianOS
//!
//! This module provides the base traits and utilities for all user-space drivers.

#![no_std]

// Driver capabilities
pub const CAP_DEVICE_ACCESS: u64 = 0x1000;
pub const CAP_DMA_ACCESS: u64 = 0x2000;
pub const CAP_INTERRUPT_HANDLER: u64 = 0x4000;
pub const CAP_MMIO_ACCESS: u64 = 0x8000;

/// Driver information structure
#[derive(Debug, Clone)]
pub struct DriverInfo {
    /// Driver name
    pub name: &'static str,
    
    /// Driver version
    pub version: (u32, u32, u32),
    
    /// Driver author
    pub author: &'static str,
    
    /// Driver description
    pub description: &'static str,
    
    /// Supported device IDs (vendor, device)
    pub device_ids: &'static [(u16, u16)],
    
    /// Required capabilities
    pub required_caps: u64,
}

/// Driver state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverState {
    /// Driver is initializing
    Initializing,
    
    /// Driver is ready
    Ready,
    
    /// Driver is busy
    Busy,
    
    /// Driver has encountered an error
    Error,
    
    /// Driver is shutting down
    Stopping,
    
    /// Driver is stopped
    Stopped,
}

/// Base trait for all drivers
pub trait Driver: Send + Sync {
    /// Get driver information
    fn info(&self) -> &DriverInfo;
    
    /// Initialize the driver
    fn init(&mut self) -> Result<(), DriverError>;
    
    /// Probe for supported devices
    fn probe(&self) -> Result<bool, DriverError>;
    
    /// Start the driver
    fn start(&mut self) -> Result<(), DriverError>;
    
    /// Stop the driver
    fn stop(&mut self) -> Result<(), DriverError>;
    
    /// Get driver state
    fn state(&self) -> DriverState;
    
    /// Handle an interrupt
    fn handle_interrupt(&mut self, irq: u32) -> Result<(), DriverError>;
}

/// Character device driver trait
pub trait CharDriver: Driver {
    /// Read from the device
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, DriverError>;
    
    /// Write to the device
    fn write(&mut self, data: &[u8]) -> Result<usize, DriverError>;
    
    /// Check if data is available
    fn poll(&self) -> Result<bool, DriverError>;
}

/// Block device driver trait
pub trait BlockDriver: Driver {
    /// Get block size
    fn block_size(&self) -> usize;
    
    /// Get number of blocks
    fn block_count(&self) -> u64;
    
    /// Read blocks
    fn read_blocks(&mut self, start_block: u64, num_blocks: u32, buffer: &mut [u8]) 
        -> Result<(), DriverError>;
    
    /// Write blocks
    fn write_blocks(&mut self, start_block: u64, num_blocks: u32, data: &[u8])
        -> Result<(), DriverError>;
    
    /// Flush any pending writes
    fn flush(&mut self) -> Result<(), DriverError>;
}

/// Network device driver trait
pub trait NetworkDriver: Driver {
    /// Get MAC address
    fn mac_address(&self) -> [u8; 6];
    
    /// Get link status
    fn link_up(&self) -> bool;
    
    /// Get link speed in Mbps
    fn link_speed(&self) -> u32;
    
    /// Send a packet
    fn send_packet(&mut self, data: &[u8]) -> Result<(), DriverError>;
    
    /// Receive a packet
    fn receive_packet(&mut self, buffer: &mut [u8]) -> Result<usize, DriverError>;
    
    /// Set promiscuous mode
    fn set_promiscuous(&mut self, enable: bool) -> Result<(), DriverError>;
}

/// Driver error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// Device not found
    DeviceNotFound,
    
    /// Device not supported
    DeviceNotSupported,
    
    /// Initialization failed
    InitFailed,
    
    /// I/O error
    IoError,
    
    /// Invalid parameter
    InvalidParameter,
    
    /// Operation not supported
    NotSupported,
    
    /// Device busy
    Busy,
    
    /// Buffer too small
    BufferTooSmall,
    
    /// Timeout
    Timeout,
    
    /// Permission denied
    PermissionDenied,
    
    /// Resource exhausted
    ResourceExhausted,
    
    /// Invalid state
    InvalidState,
}

/// Device manager interface
pub trait DeviceManager {
    /// Register a driver
    fn register_driver(&mut self, driver: Box<dyn Driver>) -> Result<u32, DriverError>;
    
    /// Unregister a driver
    fn unregister_driver(&mut self, driver_id: u32) -> Result<(), DriverError>;
    
    /// Get driver by ID
    fn get_driver(&self, driver_id: u32) -> Option<&dyn Driver>;
    
    /// Get mutable driver by ID
    fn get_driver_mut(&mut self, driver_id: u32) -> Option<&mut dyn Driver>;
    
    /// List all drivers
    fn list_drivers(&self) -> Vec<u32>;
    
    /// Find driver for device
    fn find_driver_for_device(&self, vendor_id: u16, device_id: u16) -> Option<u32>;
}

/// DMA buffer for driver use
#[repr(C)]
pub struct DmaBuffer {
    /// Virtual address
    pub virt_addr: *mut u8,
    
    /// Physical address
    pub phys_addr: u64,
    
    /// Buffer size
    pub size: usize,
    
    /// Alignment requirement
    pub alignment: usize,
}

impl DmaBuffer {
    /// Create a new DMA buffer
    pub fn new(size: usize, alignment: usize) -> Result<Self, DriverError> {
        // TODO: Allocate DMA-capable memory from kernel
        Err(DriverError::NotSupported)
    }
    
    /// Free the DMA buffer
    pub fn free(self) {
        // TODO: Free DMA memory
    }
}

/// MMIO region for driver use
#[repr(C)]
pub struct MmioRegion {
    /// Virtual address
    pub virt_addr: *mut u8,
    
    /// Physical address
    pub phys_addr: u64,
    
    /// Region size
    pub size: usize,
}

impl MmioRegion {
    /// Map an MMIO region
    pub fn map(phys_addr: u64, size: usize) -> Result<Self, DriverError> {
        // TODO: Map MMIO region through kernel
        Err(DriverError::NotSupported)
    }
    
    /// Unmap the MMIO region
    pub fn unmap(self) {
        // TODO: Unmap MMIO region
    }
    
    /// Read a value from the MMIO region
    pub unsafe fn read<T>(&self, offset: usize) -> T {
        let addr = self.virt_addr.add(offset) as *const T;
        core::ptr::read_volatile(addr)
    }
    
    /// Write a value to the MMIO region
    pub unsafe fn write<T>(&self, offset: usize, value: T) {
        let addr = self.virt_addr.add(offset) as *mut T;
        core::ptr::write_volatile(addr, value);
    }
}
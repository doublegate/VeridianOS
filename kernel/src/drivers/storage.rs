//! Storage Device Drivers
//!
//! Implements storage drivers including ATA/IDE, AHCI/SATA, and NVMe.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::vec;
use spin::Mutex;
use crate::services::driver_framework::{
    Driver, DeviceInfo, DeviceClass, DeviceStatus
};

/// Storage device types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    HardDisk,
    SolidState,
    OpticalDisk,
    FloppyDisk,
    Unknown,
}

/// Storage interface types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageInterface {
    ATA,     // IDE/PATA
    SATA,    // Serial ATA
    SCSI,    // Small Computer System Interface
    NVMe,    // NVM Express
    USB,     // USB Mass Storage
    Unknown,
}

/// Storage device information
#[derive(Debug, Clone)]
pub struct StorageInfo {
    pub model: String,
    pub serial: String,
    pub firmware: String,
    pub capacity: u64,          // in bytes
    pub sector_size: u32,       // in bytes
    pub storage_type: StorageType,
    pub interface: StorageInterface,
    pub removable: bool,
    pub read_only: bool,
}

/// Storage device statistics
#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    pub reads: u64,
    pub writes: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_errors: u64,
    pub write_errors: u64,
    pub read_time_ms: u64,
    pub write_time_ms: u64,
}

/// Storage device trait
pub trait StorageDevice: Send + Sync {
    /// Get device name
    fn name(&self) -> &str;
    
    /// Get device information
    fn get_info(&self) -> StorageInfo;
    
    /// Get device statistics
    fn get_stats(&self) -> StorageStats;
    
    /// Reset statistics
    fn reset_stats(&mut self);
    
    /// Read sectors
    fn read_sectors(&mut self, lba: u64, count: u32, buffer: &mut [u8]) -> Result<u32, &'static str>;
    
    /// Write sectors
    fn write_sectors(&mut self, lba: u64, count: u32, data: &[u8]) -> Result<u32, &'static str>;
    
    /// Flush cache
    fn flush(&mut self) -> Result<(), &'static str>;
    
    /// Check if device is ready
    fn is_ready(&self) -> bool;
    
    /// Get maximum transfer size in sectors
    fn max_transfer_sectors(&self) -> u32;
}

/// ATA (IDE) driver implementation
pub struct AtaDriver {
    name: String,
    base_port: u16,
    control_port: u16,
    is_master: bool,
    info: StorageInfo,
    stats: Mutex<StorageStats>,
    device_info: DeviceInfo,
}

impl AtaDriver {
    /// Create a new ATA driver
    pub fn new(name: String, base_port: u16, is_master: bool, device_info: DeviceInfo) -> Self {
        let control_port = base_port + 0x206;
        
        Self {
            name: name.clone(),
            base_port,
            control_port,
            is_master,
            info: StorageInfo {
                model: String::from("ATA Drive"),
                serial: String::from("Unknown"),
                firmware: String::from("Unknown"),
                capacity: 0,
                sector_size: 512,
                storage_type: StorageType::HardDisk,
                interface: StorageInterface::ATA,
                removable: false,
                read_only: false,
            },
            stats: Mutex::new(StorageStats::default()),
            device_info,
        }
    }
    
    /// Initialize ATA device
    pub fn init(&mut self) -> Result<(), &'static str> {
        crate::println!("[ATA] Initializing {} at port 0x{:x}", self.name, self.base_port);
        
        // Select drive
        let drive_select = if self.is_master { 0xA0 } else { 0xB0 };
        self.write_register(6, drive_select);
        self.wait_busy()?;
        
        // Send IDENTIFY command
        self.write_register(7, 0xEC);
        self.wait_busy()?;
        
        // Check if device exists
        let status = self.read_register(7);
        if status == 0 {
            return Err("No device present");
        }
        
        // Read identification data
        let mut identify_data = [0u16; 256];
        for i in 0..256 {
            identify_data[i] = self.read_data();
        }
        
        // Parse identification data
        self.parse_identify_data(&identify_data);
        
        crate::println!("[ATA] Initialized {}: {} ({} sectors)", 
            self.name, self.info.model, self.info.capacity / self.info.sector_size as u64);
        
        Ok(())
    }
    
    /// Parse IDENTIFY data
    fn parse_identify_data(&mut self, data: &[u16; 256]) {
        // Model string (words 27-46)
        let mut model = String::new();
        for i in 27..47 {
            let word = data[i];
            let bytes = [(word >> 8) as u8, word as u8];
            for &byte in &bytes {
                if byte != 0 && byte != b' ' {
                    model.push(byte as char);
                }
            }
        }
        self.info.model = model.trim().into();
        
        // Serial number (words 10-19)
        let mut serial = String::new();
        for i in 10..20 {
            let word = data[i];
            let bytes = [(word >> 8) as u8, word as u8];
            for &byte in &bytes {
                if byte != 0 && byte != b' ' {
                    serial.push(byte as char);
                }
            }
        }
        self.info.serial = serial.trim().into();
        
        // Capacity (words 60-61 for 28-bit LBA)
        let capacity_sectors = data[60] as u64 | ((data[61] as u64) << 16);
        self.info.capacity = capacity_sectors * self.info.sector_size as u64;
        
        // Check for 48-bit LBA support
        if data[83] & (1 << 10) != 0 {
            let capacity_48 = data[100] as u64 |
                             ((data[101] as u64) << 16) |
                             ((data[102] as u64) << 32) |
                             ((data[103] as u64) << 48);
            if capacity_48 > capacity_sectors {
                self.info.capacity = capacity_48 * self.info.sector_size as u64;
            }
        }
    }
    
    /// Read ATA register
    fn read_register(&self, offset: u8) -> u8 {
        unsafe { crate::arch::inb(self.base_port + offset as u16) }
    }
    
    /// Write ATA register
    fn write_register(&self, offset: u8, value: u8) {
        unsafe { crate::arch::outb(self.base_port + offset as u16, value); }
    }
    
    /// Read data port
    fn read_data(&self) -> u16 {
        unsafe { crate::arch::inw(self.base_port) }
    }
    
    /// Write data port
    fn write_data(&self, value: u16) {
        unsafe { crate::arch::outw(self.base_port, value); }
    }
    
    /// Wait for device to not be busy
    fn wait_busy(&self) -> Result<(), &'static str> {
        for _ in 0..10000 {
            let status = self.read_register(7);
            if status & 0x80 == 0 { // BSY bit clear
                return Ok(());
            }
            // Small delay
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
        Err("Device timeout")
    }
    
    /// Wait for device ready
    fn wait_ready(&self) -> Result<(), &'static str> {
        for _ in 0..10000 {
            let status = self.read_register(7);
            if status & 0x80 == 0 && status & 0x40 != 0 { // BSY clear, RDY set
                return Ok(());
            }
            // Small delay
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
        Err("Device not ready")
    }
    
    /// Select drive and set LBA
    fn select_drive_lba(&self, lba: u64) -> Result<(), &'static str> {
        let drive_select = if self.is_master { 0xE0 } else { 0xF0 };
        
        // LBA mode, drive select
        self.write_register(6, drive_select | ((lba >> 24) & 0x0F) as u8);
        self.write_register(2, 1); // Sector count
        self.write_register(3, lba as u8); // LBA 0-7
        self.write_register(4, (lba >> 8) as u8); // LBA 8-15
        self.write_register(5, (lba >> 16) as u8); // LBA 16-23
        
        self.wait_ready()
    }
}

impl StorageDevice for AtaDriver {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn get_info(&self) -> StorageInfo {
        self.info.clone()
    }
    
    fn get_stats(&self) -> StorageStats {
        self.stats.lock().clone()
    }
    
    fn reset_stats(&mut self) {
        *self.stats.lock() = StorageStats::default();
    }
    
    fn read_sectors(&mut self, lba: u64, count: u32, buffer: &mut [u8]) -> Result<u32, &'static str> {
        if buffer.len() < (count * self.info.sector_size) as usize {
            return Err("Buffer too small");
        }
        
        let mut sectors_read = 0;
        let mut current_lba = lba;
        let mut buffer_offset = 0;
        
        while sectors_read < count {
            // Read one sector at a time for simplicity
            self.select_drive_lba(current_lba)?;
            
            // Send READ SECTORS command
            self.write_register(7, 0x20);
            self.wait_ready()?;
            
            // Read sector data
            for i in 0..256 { // 512 bytes = 256 words
                let word = self.read_data();
                buffer[buffer_offset + i * 2] = word as u8;
                buffer[buffer_offset + i * 2 + 1] = (word >> 8) as u8;
            }
            
            sectors_read += 1;
            current_lba += 1;
            buffer_offset += self.info.sector_size as usize;
            
            // Update statistics
            let mut stats = self.stats.lock();
            stats.reads += 1;
            stats.bytes_read += self.info.sector_size as u64;
        }
        
        Ok(sectors_read)
    }
    
    fn write_sectors(&mut self, lba: u64, count: u32, data: &[u8]) -> Result<u32, &'static str> {
        if data.len() < (count * self.info.sector_size) as usize {
            return Err("Data too small");
        }
        
        if self.info.read_only {
            return Err("Device is read-only");
        }
        
        let mut sectors_written = 0;
        let mut current_lba = lba;
        let mut data_offset = 0;
        
        while sectors_written < count {
            // Write one sector at a time
            self.select_drive_lba(current_lba)?;
            
            // Send WRITE SECTORS command
            self.write_register(7, 0x30);
            self.wait_ready()?;
            
            // Write sector data
            for i in 0..256 { // 512 bytes = 256 words
                let word = data[data_offset + i * 2] as u16 |
                          ((data[data_offset + i * 2 + 1] as u16) << 8);
                self.write_data(word);
            }
            
            // Wait for completion
            self.wait_ready()?;
            
            sectors_written += 1;
            current_lba += 1;
            data_offset += self.info.sector_size as usize;
            
            // Update statistics
            let mut stats = self.stats.lock();
            stats.writes += 1;
            stats.bytes_written += self.info.sector_size as u64;
        }
        
        Ok(sectors_written)
    }
    
    fn flush(&mut self) -> Result<(), &'static str> {
        // Send FLUSH CACHE command
        self.write_register(7, 0xE7);
        self.wait_ready()
    }
    
    fn is_ready(&self) -> bool {
        let status = self.read_register(7);
        status & 0x80 == 0 && status & 0x40 != 0 // BSY clear, RDY set
    }
    
    fn max_transfer_sectors(&self) -> u32 {
        256 // ATA can transfer up to 256 sectors per command
    }
}

impl Driver for AtaDriver {
    fn name(&self) -> &str {
        "ata"
    }
    
    fn supported_classes(&self) -> Vec<DeviceClass> {
        vec![DeviceClass::Storage]
    }
    
    fn supports_device(&self, device: &DeviceInfo) -> bool {
        device.class == DeviceClass::Storage &&
        device.bus == "pci" &&
        device.device_id.as_ref().map_or(false, |id| {
            // Check for IDE/ATA controller class codes
            id.class_code == 0x01 && // Mass storage
            (id.subclass == 0x01 || id.subclass == 0x05) // IDE or ATA
        })
    }
    
    fn probe(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[ATA] Probing device: {}", device.name);
        
        // Try to initialize the ATA device
        self.init()
    }
    
    fn attach(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[ATA] Attaching to device: {}", device.name);
        
        // Device should already be initialized from probe
        crate::println!("[ATA] Successfully attached to {}", device.name);
        Ok(())
    }
    
    fn detach(&mut self, device: &DeviceInfo) -> Result<(), &'static str> {
        crate::println!("[ATA] Detaching from device: {}", device.name);
        
        // Flush any pending writes
        self.flush().ok();
        
        crate::println!("[ATA] Successfully detached from {}", device.name);
        Ok(())
    }
    
    fn suspend(&mut self) -> Result<(), &'static str> {
        // Flush cache and put device in standby
        self.flush()?;
        
        // Send STANDBY command
        self.write_register(7, 0xE2);
        self.wait_ready()?;
        
        crate::println!("[ATA] Device suspended");
        Ok(())
    }
    
    fn resume(&mut self) -> Result<(), &'static str> {
        // Device should wake up automatically on next access
        self.wait_ready()?;
        crate::println!("[ATA] Device resumed");
        Ok(())
    }
    
    fn handle_interrupt(&mut self, irq: u8) -> Result<(), &'static str> {
        crate::println!("[ATA] Handling interrupt {} for {}", irq, self.name);
        
        // Read status to clear interrupt
        let status = self.read_register(7);
        
        // Check for errors
        if status & 0x01 != 0 { // ERR bit set
            let error = self.read_register(1);
            crate::println!("[ATA] Error detected: 0x{:02x}", error);
            return Err("ATA error");
        }
        
        Ok(())
    }
    
    fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize, &'static str> {
        // Convert byte offset to sector
        let sector_size = self.info.sector_size as u64;
        let lba = offset / sector_size;
        let sector_offset = (offset % sector_size) as usize;
        
        // Calculate number of sectors to read
        let bytes_needed = buffer.len() + sector_offset;
        let sectors_needed = (bytes_needed + sector_size as usize - 1) / sector_size as usize;
        
        // Allocate temporary buffer for sector-aligned reads
        let mut sector_buffer = vec![0u8; sectors_needed * sector_size as usize];
        
        // Read sectors
        let sectors_read = self.read_sectors(lba, sectors_needed as u32, &mut sector_buffer)?;
        
        // Copy requested data
        let copy_len = buffer.len().min(sector_buffer.len() - sector_offset);
        buffer[..copy_len].copy_from_slice(&sector_buffer[sector_offset..sector_offset + copy_len]);
        
        Ok(copy_len)
    }
    
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<usize, &'static str> {
        // Convert byte offset to sector
        let sector_size = self.info.sector_size as u64;
        let lba = offset / sector_size;
        let sector_offset = (offset % sector_size) as usize;
        
        // For simplicity, require sector-aligned writes
        if sector_offset != 0 {
            return Err("Non-aligned writes not supported");
        }
        
        if data.len() % sector_size as usize != 0 {
            return Err("Write size must be multiple of sector size");
        }
        
        let sectors_to_write = data.len() / sector_size as usize;
        let sectors_written = self.write_sectors(lba, sectors_to_write as u32, data)?;
        
        Ok(sectors_written as usize * sector_size as usize)
    }
    
    fn ioctl(&mut self, cmd: u32, arg: u64) -> Result<u64, &'static str> {
        match cmd {
            0x3000 => { // Get capacity
                Ok(self.info.capacity)
            }
            0x3001 => { // Get sector size
                Ok(self.info.sector_size as u64)
            }
            0x3002 => { // Flush cache
                self.flush()?;
                Ok(0)
            }
            0x3003 => { // Get device ready status
                Ok(if self.is_ready() { 1 } else { 0 })
            }
            0x3004 => { // Reset statistics
                self.reset_stats();
                Ok(0)
            }
            _ => Err("Unknown ioctl command"),
        }
    }
}

/// Storage manager for managing multiple storage devices
pub struct StorageManager {
    devices: Vec<Box<dyn StorageDevice>>,
}

impl StorageManager {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }
    
    /// Add a storage device
    pub fn add_device(&mut self, device: Box<dyn StorageDevice>) {
        crate::println!("[STORAGE] Added storage device: {}", device.name());
        self.devices.push(device);
    }
    
    /// Get device by index
    pub fn get_device(&mut self, index: usize) -> Option<&mut dyn StorageDevice> {
        match self.devices.get_mut(index) {
            Some(device) => Some(device.as_mut()),
            None => None,
        }
    }
    
    /// List all devices
    pub fn list_devices(&self) -> Vec<StorageInfo> {
        self.devices.iter().map(|d| d.get_info()).collect()
    }
    
    /// Get total storage capacity
    pub fn get_total_capacity(&self) -> u64 {
        self.devices.iter().map(|d| d.get_info().capacity).sum()
    }
}

/// Global storage manager instance
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
static STORAGE_MANAGER: spin::Once<Mutex<StorageManager>> = spin::Once::new();

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static mut STORAGE_MANAGER_STATIC: Option<Mutex<StorageManager>> = None;

/// Initialize storage subsystem
pub fn init() {
    let storage_manager = StorageManager::new();
    
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        STORAGE_MANAGER.call_once(|| Mutex::new(storage_manager));
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        STORAGE_MANAGER_STATIC = Some(Mutex::new(storage_manager));
    }
    
    // Register ATA driver with driver framework
    let driver_framework = crate::services::driver_framework::get_driver_framework();
    
    // Create dummy ATA driver for demonstration
    let dummy_device = DeviceInfo {
        id: 0,
        name: String::from("IDE Controller"),
        class: DeviceClass::Storage,
        device_id: Some(crate::services::driver_framework::DeviceId {
            vendor_id: 0x8086,
            device_id: 0x7010,
            class_code: 0x01, // Mass storage
            subclass: 0x01,   // IDE
            prog_if: 0x80,
            revision: 0x01,
        }),
        driver: None,
        bus: String::from("pci"),
        address: 0x1F0,
        irq: Some(14),
        dma_channels: Vec::new(),
        io_ports: vec![(0x1F0, 0x1F7), (0x3F6, 0x3F6)],
        memory_regions: Vec::new(),
        status: DeviceStatus::Uninitialized,
    };
    
    let ata_driver = AtaDriver::new(
        String::from("ata0"),
        0x1F0, // Primary IDE base port
        true,   // Master drive
        dummy_device,
    );
    
    if let Err(e) = driver_framework.register_driver(Box::new(ata_driver)) {
        crate::println!("[STORAGE] Failed to register ATA driver: {}", e);
    } else {
        crate::println!("[STORAGE] Storage subsystem initialized");
    }
}

/// Get the global storage manager
pub fn get_storage_manager() -> &'static Mutex<StorageManager> {
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        STORAGE_MANAGER.get().expect("Storage manager not initialized")
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    unsafe {
        STORAGE_MANAGER_STATIC.as_ref().expect("Storage manager not initialized")
    }
}
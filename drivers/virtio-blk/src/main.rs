//! VirtIO Block Device Driver
//!
//! Provides block storage access for virtual machines using the VirtIO interface.

#![no_std]
#![no_main]

extern crate driver_common;

use core::mem;
use driver_common::{BlockDriver, Driver, DriverError, DriverInfo, DriverState};

/// VirtIO MMIO registers
#[repr(C)]
struct VirtIoMmioRegs {
    magic: u32,           // 0x000
    version: u32,         // 0x004
    device_id: u32,       // 0x008
    vendor_id: u32,       // 0x00c
    device_features: u32, // 0x010
    device_features_sel: u32, // 0x014
    _reserved1: [u32; 2], // 0x018
    driver_features: u32, // 0x020
    driver_features_sel: u32, // 0x024
    _reserved2: [u32; 2], // 0x028
    queue_sel: u32,       // 0x030
    queue_num_max: u32,   // 0x034
    queue_num: u32,       // 0x038
    _reserved3: [u32; 2], // 0x03c
    queue_ready: u32,     // 0x044
    _reserved4: [u32; 2], // 0x048
    queue_notify: u32,    // 0x050
    _reserved5: [u32; 3], // 0x054
    interrupt_status: u32, // 0x060
    interrupt_ack: u32,   // 0x064
    _reserved6: [u32; 2], // 0x068
    status: u32,          // 0x070
    _reserved7: [u32; 3], // 0x074
    queue_desc_low: u32,  // 0x080
    queue_desc_high: u32, // 0x084
    _reserved8: [u32; 2], // 0x088
    queue_avail_low: u32, // 0x090
    queue_avail_high: u32, // 0x094
    _reserved9: [u32; 2], // 0x098
    queue_used_low: u32,  // 0x0a0
    queue_used_high: u32, // 0x0a4
}

/// VirtIO block device request types
#[repr(u32)]
enum VirtIoBlkType {
    In = 0,
    Out = 1,
    Flush = 4,
    GetId = 8,
    GetLifetime = 10,
    Discard = 11,
    WriteZeroes = 13,
    SecureErase = 14,
}

/// VirtIO block request header
#[repr(C)]
struct VirtIoBlkRequest {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

/// VirtIO block request status
#[repr(u8)]
enum VirtIoBlkStatus {
    Ok = 0,
    IoErr = 1,
    Unsupp = 2,
}

/// VirtIO descriptor
#[repr(C)]
struct VirtIoDescriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

/// VirtIO available ring
#[repr(C)]
struct VirtIoAvailableRing {
    flags: u16,
    idx: u16,
    ring: [u16; 256],
    used_event: u16,
}

/// VirtIO used ring element
#[repr(C)]
struct VirtIoUsedElement {
    id: u32,
    len: u32,
}

/// VirtIO used ring
#[repr(C)]
struct VirtIoUsedRing {
    flags: u16,
    idx: u16,
    ring: [VirtIoUsedElement; 256],
    avail_event: u16,
}

/// VirtIO block configuration
#[repr(C)]
struct VirtIoBlkConfig {
    capacity: u64,
    size_max: u32,
    seg_max: u32,
    geometry: VirtIoBlkGeometry,
    blk_size: u32,
    physical_block_exp: u8,
    alignment_offset: u8,
    min_io_size: u16,
    opt_io_size: u32,
}

#[repr(C)]
struct VirtIoBlkGeometry {
    cylinders: u16,
    heads: u8,
    sectors: u8,
}

/// VirtIO block device driver
pub struct VirtIoBlkDriver {
    info: DriverInfo,
    state: DriverState,
    mmio_base: usize,
    capacity: u64,
    block_size: u32,
    queue_size: u16,
    descriptors: *mut VirtIoDescriptor,
    available: *mut VirtIoAvailableRing,
    used: *mut VirtIoUsedRing,
    next_desc: u16,
}

impl VirtIoBlkDriver {
    /// Create a new VirtIO block driver
    pub fn new(mmio_base: usize) -> Self {
        let info = DriverInfo {
            name: "virtio-blk",
            version: (0, 1, 0),
            author: "VeridianOS Team",
            description: "VirtIO Block Device Driver",
            device_ids: &[(0x1AF4, 0x1001)], // VirtIO vendor and block device
            required_caps: driver_common::CAP_DEVICE_ACCESS | driver_common::CAP_MMIO_ACCESS,
        };
        
        Self {
            info,
            state: DriverState::Stopped,
            mmio_base,
            capacity: 0,
            block_size: 512,
            queue_size: 256,
            descriptors: core::ptr::null_mut(),
            available: core::ptr::null_mut(),
            used: core::ptr::null_mut(),
            next_desc: 0,
        }
    }
    
    /// Read MMIO register
    unsafe fn read_reg(&self, offset: usize) -> u32 {
        let addr = (self.mmio_base + offset) as *const u32;
        core::ptr::read_volatile(addr)
    }
    
    /// Write MMIO register
    unsafe fn write_reg(&self, offset: usize, value: u32) {
        let addr = (self.mmio_base + offset) as *mut u32;
        core::ptr::write_volatile(addr, value);
    }
    
    /// Initialize virtqueue
    fn init_queue(&mut self) -> Result<(), DriverError> {
        unsafe {
            // Select queue 0 (request queue)
            self.write_reg(0x030, 0);
            
            // Get maximum queue size
            let max_size = self.read_reg(0x034);
            if max_size < self.queue_size as u32 {
                self.queue_size = max_size as u16;
            }
            
            // Set queue size
            self.write_reg(0x038, self.queue_size as u32);
            
            // Allocate queue memory (simplified - should use DMA allocator)
            let desc_size = mem::size_of::<VirtIoDescriptor>() * self.queue_size as usize;
            let avail_size = mem::size_of::<VirtIoAvailableRing>();
            let used_size = mem::size_of::<VirtIoUsedRing>();
            
            // For now, use static buffers (should be DMA-capable memory)
            static mut DESC_BUFFER: [u8; 4096] = [0; 4096];
            static mut AVAIL_BUFFER: [u8; 1024] = [0; 1024];
            static mut USED_BUFFER: [u8; 2048] = [0; 2048];
            
            self.descriptors = DESC_BUFFER.as_mut_ptr() as *mut VirtIoDescriptor;
            self.available = AVAIL_BUFFER.as_mut_ptr() as *mut VirtIoAvailableRing;
            self.used = USED_BUFFER.as_mut_ptr() as *mut VirtIoUsedRing;
            
            // Set queue addresses
            let desc_addr = self.descriptors as u64;
            let avail_addr = self.available as u64;
            let used_addr = self.used as u64;
            
            self.write_reg(0x080, desc_addr as u32);
            self.write_reg(0x084, (desc_addr >> 32) as u32);
            self.write_reg(0x090, avail_addr as u32);
            self.write_reg(0x094, (avail_addr >> 32) as u32);
            self.write_reg(0x0a0, used_addr as u32);
            self.write_reg(0x0a4, (used_addr >> 32) as u32);
            
            // Enable queue
            self.write_reg(0x044, 1);
        }
        
        Ok(())
    }
    
    /// Submit a request to the device
    fn submit_request(
        &mut self,
        request_type: VirtIoBlkType,
        sector: u64,
        buffer: &[u8],
    ) -> Result<(), DriverError> {
        unsafe {
            // Create request header
            let req = VirtIoBlkRequest {
                request_type: request_type as u32,
                reserved: 0,
                sector,
            };
            
            // Get next descriptor
            let desc_idx = self.next_desc;
            self.next_desc = (self.next_desc + 1) % self.queue_size;
            
            // Setup descriptor chain
            let desc = &mut *self.descriptors.add(desc_idx as usize);
            desc.addr = &req as *const _ as u64;
            desc.len = mem::size_of::<VirtIoBlkRequest>() as u32;
            desc.flags = 1; // NEXT flag
            desc.next = (desc_idx + 1) % self.queue_size;
            
            // Data descriptor
            let data_desc = &mut *self.descriptors.add(desc.next as usize);
            data_desc.addr = buffer.as_ptr() as u64;
            data_desc.len = buffer.len() as u32;
            data_desc.flags = if matches!(request_type, VirtIoBlkType::In) { 2 } else { 1 }; // WRITE or NEXT
            data_desc.next = (desc.next + 1) % self.queue_size;
            
            // Status descriptor
            let mut status: u8 = 0;
            let status_desc = &mut *self.descriptors.add(data_desc.next as usize);
            status_desc.addr = &mut status as *mut _ as u64;
            status_desc.len = 1;
            status_desc.flags = 2; // WRITE flag
            status_desc.next = 0;
            
            // Add to available ring
            let avail = &mut *self.available;
            let avail_idx = avail.idx;
            avail.ring[avail_idx as usize % self.queue_size as usize] = desc_idx;
            avail.idx = avail_idx.wrapping_add(1);
            
            // Notify device
            self.write_reg(0x050, 0);
            
            // Wait for completion (simplified - should use interrupts)
            let used = &*self.used;
            while used.idx == avail_idx {
                core::hint::spin_loop();
            }
            
            // Check status
            if status != VirtIoBlkStatus::Ok as u8 {
                return Err(DriverError::IoError);
            }
        }
        
        Ok(())
    }
}

impl Driver for VirtIoBlkDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }
    
    fn init(&mut self) -> Result<(), DriverError> {
        unsafe {
            // Check magic value
            let magic = self.read_reg(0x000);
            if magic != 0x74726976 { // "virt" in little-endian
                return Err(DriverError::DeviceNotFound);
            }
            
            // Check version
            let version = self.read_reg(0x004);
            if version != 2 {
                return Err(DriverError::DeviceNotSupported);
            }
            
            // Check device ID (2 = block device)
            let device_id = self.read_reg(0x008);
            if device_id != 2 {
                return Err(DriverError::DeviceNotSupported);
            }
            
            // Reset device
            self.write_reg(0x070, 0);
            
            // Set ACKNOWLEDGE status
            self.write_reg(0x070, 1);
            
            // Set DRIVER status
            self.write_reg(0x070, 3);
            
            // Negotiate features (simplified - accept defaults)
            let features = self.read_reg(0x010);
            self.write_reg(0x020, features);
            
            // Set FEATURES_OK status
            self.write_reg(0x070, 11);
            
            // Check if features were accepted
            let status = self.read_reg(0x070);
            if (status & 8) == 0 {
                return Err(DriverError::InitFailed);
            }
            
            // Initialize queue
            self.init_queue()?;
            
            // Read device configuration
            let config_base = self.mmio_base + 0x100;
            let config = &*(config_base as *const VirtIoBlkConfig);
            self.capacity = config.capacity;
            self.block_size = if config.blk_size != 0 { config.blk_size } else { 512 };
            
            // Set DRIVER_OK status
            self.write_reg(0x070, 15);
            
            self.state = DriverState::Ready;
        }
        
        Ok(())
    }
    
    fn probe(&self) -> Result<bool, DriverError> {
        unsafe {
            let magic = self.read_reg(0x000);
            Ok(magic == 0x74726976)
        }
    }
    
    fn start(&mut self) -> Result<(), DriverError> {
        if self.state != DriverState::Ready {
            self.init()?;
        }
        self.state = DriverState::Ready;
        Ok(())
    }
    
    fn stop(&mut self) -> Result<(), DriverError> {
        unsafe {
            // Reset device
            self.write_reg(0x070, 0);
        }
        self.state = DriverState::Stopped;
        Ok(())
    }
    
    fn state(&self) -> DriverState {
        self.state
    }
    
    fn handle_interrupt(&mut self, _irq: u32) -> Result<(), DriverError> {
        unsafe {
            // Read and acknowledge interrupt
            let status = self.read_reg(0x060);
            self.write_reg(0x064, status);
        }
        Ok(())
    }
}

impl BlockDriver for VirtIoBlkDriver {
    fn block_size(&self) -> usize {
        self.block_size as usize
    }
    
    fn block_count(&self) -> u64 {
        self.capacity
    }
    
    fn read_blocks(
        &mut self,
        start_block: u64,
        num_blocks: u32,
        buffer: &mut [u8],
    ) -> Result<(), DriverError> {
        if buffer.len() < (num_blocks as usize * self.block_size as usize) {
            return Err(DriverError::BufferTooSmall);
        }
        
        for i in 0..num_blocks {
            let offset = i as usize * self.block_size as usize;
            let block_buffer = &mut buffer[offset..offset + self.block_size as usize];
            self.submit_request(VirtIoBlkType::In, start_block + i as u64, block_buffer)?;
        }
        
        Ok(())
    }
    
    fn write_blocks(
        &mut self,
        start_block: u64,
        num_blocks: u32,
        data: &[u8],
    ) -> Result<(), DriverError> {
        if data.len() < (num_blocks as usize * self.block_size as usize) {
            return Err(DriverError::InvalidParameter);
        }
        
        for i in 0..num_blocks {
            let offset = i as usize * self.block_size as usize;
            let block_data = &data[offset..offset + self.block_size as usize];
            self.submit_request(VirtIoBlkType::Out, start_block + i as u64, block_data)?;
        }
        
        Ok(())
    }
    
    fn flush(&mut self) -> Result<(), DriverError> {
        self.submit_request(VirtIoBlkType::Flush, 0, &[])?;
        Ok(())
    }
}

/// Driver entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Initialize driver with default MMIO base address for QEMU
    let mut driver = VirtIoBlkDriver::new(0x10001000);
    
    // Initialize the driver
    match driver.init() {
        Ok(_) => {
            // Driver initialized successfully
            // Enter main driver loop
            loop {
                // Handle requests from kernel
                // This would normally receive commands via IPC
                core::hint::spin_loop();
            }
        }
        Err(_) => {
            // Driver initialization failed
            // Exit with error code
            loop {
                core::hint::spin_loop();
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
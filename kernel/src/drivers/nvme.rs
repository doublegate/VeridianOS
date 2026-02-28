//! NVMe (Non-Volatile Memory Express) Driver
//!
//! High-performance storage driver for NVMe SSDs using the BlockDevice trait.

// NVMe driver -- hardware register offsets per NVMe spec
#![allow(dead_code)]

use alloc::{vec, vec::Vec};
use core::sync::atomic::AtomicU16;

use crate::{error::KernelError, fs::blockdev::BlockDevice};

/// NVMe PCI vendor/device IDs
pub const NVME_VENDOR_INTEL: u16 = 0x8086;
pub const NVME_VENDOR_SAMSUNG: u16 = 0x144d;

/// NVMe register offsets
const REG_CAP: usize = 0x00; // Controller Capabilities
const REG_VS: usize = 0x08; // Version
const REG_CC: usize = 0x14; // Controller Configuration
const REG_CSTS: usize = 0x1C; // Controller Status
const REG_AQA: usize = 0x24; // Admin Queue Attributes
const REG_ASQ: usize = 0x28; // Admin Submission Queue
const REG_ACQ: usize = 0x30; // Admin Completion Queue

/// Controller Configuration bits
const CC_ENABLE: u32 = 1 << 0;
const CC_CSS_NVM: u32 = 0 << 4;
const CC_MPS_4K: u32 = 0 << 7;
const CC_AMS_RR: u32 = 0 << 11;
const CC_SHN_NONE: u32 = 0 << 14;
const CC_IOSQES: u32 = 6 << 16;
const CC_IOCQES: u32 = 4 << 20;

/// Controller Status bits
const CSTS_RDY: u32 = 1 << 0;
const CSTS_CFS: u32 = 1 << 1;

/// NVMe Admin Commands
const ADMIN_DELETE_SQ: u8 = 0x00;
const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_DELETE_CQ: u8 = 0x04;
const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const ADMIN_SET_FEATURES: u8 = 0x09;

/// NVMe I/O Commands
const IO_READ: u8 = 0x02;
const IO_WRITE: u8 = 0x01;

/// Submission Queue Entry
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SubmissionQueueEntry {
    opcode: u8,
    flags: u8,
    command_id: u16,
    nsid: u32,
    _reserved: u64,
    metadata: u64,
    prp1: u64,
    prp2: u64,
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
}

impl SubmissionQueueEntry {
    fn new() -> Self {
        Self {
            opcode: 0,
            flags: 0,
            command_id: 0,
            nsid: 0,
            _reserved: 0,
            metadata: 0,
            prp1: 0,
            prp2: 0,
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
}

/// Completion Queue Entry
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CompletionQueueEntry {
    result: u32,
    _reserved: u32,
    sq_head: u16,
    sq_id: u16,
    command_id: u16,
    status: u16,
}

/// NVMe Queue Pair
struct QueuePair {
    /// Submission queue
    submission_queue: Vec<SubmissionQueueEntry>,

    /// Completion queue
    completion_queue: Vec<CompletionQueueEntry>,

    /// Submission queue tail (index of next free entry)
    sq_tail: AtomicU16,

    /// Completion queue head (index of next entry to process)
    cq_head: AtomicU16,

    /// Queue size
    queue_size: u16,
}

impl QueuePair {
    fn new(queue_size: u16) -> Self {
        Self {
            submission_queue: vec![SubmissionQueueEntry::new(); queue_size as usize],
            completion_queue: vec![
                CompletionQueueEntry {
                    result: 0,
                    _reserved: 0,
                    sq_head: 0,
                    sq_id: 0,
                    command_id: 0,
                    status: 0
                };
                queue_size as usize
            ],
            sq_tail: AtomicU16::new(0),
            cq_head: AtomicU16::new(0),
            queue_size,
        }
    }
}

/// NVMe Controller
pub struct NvmeController {
    /// MMIO base address
    mmio_base: usize,

    /// Admin queue pair
    admin_queue: Option<QueuePair>,

    /// I/O queue pairs
    io_queues: Vec<QueuePair>,

    /// Number of namespaces
    num_namespaces: u32,

    /// Block size
    block_size: usize,

    /// Total blocks
    total_blocks: u64,
}

impl NvmeController {
    /// Create a new NVMe controller
    pub fn new(mmio_base: usize) -> Result<Self, KernelError> {
        let mut controller = Self {
            mmio_base,
            admin_queue: None,
            io_queues: Vec::new(),
            num_namespaces: 1,
            block_size: 512,
            total_blocks: 0,
        };

        controller.initialize()?;

        Ok(controller)
    }

    /// Read MMIO register
    fn read_reg(&self, offset: usize) -> u32 {
        // SAFETY: Reading an NVMe MMIO register at mmio_base + offset. The mmio_base
        // is the controller's BAR0 address from PCI configuration. read_volatile
        // ensures the compiler does not elide or reorder this hardware register
        // access.
        unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u32) }
    }

    /// Write MMIO register
    fn write_reg(&self, offset: usize, value: u32) {
        // SAFETY: Writing an NVMe MMIO register. Same invariants as read_reg.
        unsafe { core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value) }
    }

    /// Read 64-bit MMIO register
    fn read_reg64(&self, offset: usize) -> u64 {
        // SAFETY: Reading a 64-bit NVMe MMIO register (e.g. CAP). Same invariants as
        // read_reg.
        unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u64) }
    }

    /// Write 64-bit MMIO register
    fn write_reg64(&self, offset: usize, value: u64) {
        // SAFETY: Writing a 64-bit NVMe MMIO register. Same invariants as write_reg.
        unsafe { core::ptr::write_volatile((self.mmio_base + offset) as *mut u64, value) }
    }

    /// Initialize the NVMe controller
    fn initialize(&mut self) -> Result<(), KernelError> {
        println!(
            "[NVME] Initializing NVMe controller at 0x{:x}",
            self.mmio_base
        );

        // Read version
        let version = self.read_reg(REG_VS);
        let _major = (version >> 16) & 0xFFFF;
        let _minor = (version >> 8) & 0xFF;
        let _tertiary = version & 0xFF;
        println!("[NVME] Version: {}.{}.{}", _major, _minor, _tertiary);

        // Read capabilities
        let cap = self.read_reg64(REG_CAP);
        let max_queue_entries = ((cap & 0xFFFF) + 1) as u16;
        println!("[NVME] Max queue entries: {}", max_queue_entries);

        // Disable controller
        self.write_reg(REG_CC, 0);

        // Wait for controller to be disabled
        let mut timeout = 1000;
        while (self.read_reg(REG_CSTS) & CSTS_RDY) != 0 && timeout > 0 {
            timeout -= 1;
        }

        if timeout == 0 {
            return Err(KernelError::HardwareError {
                device: "nvme",
                code: 1,
            });
        }

        // Create admin queue (stub - would need DMA allocation)
        let admin_queue_size = 64.min(max_queue_entries);
        self.admin_queue = Some(QueuePair::new(admin_queue_size));

        println!(
            "[NVME] Created admin queue with {} entries",
            admin_queue_size
        );

        // NOTE: Full initialization requires:
        // 1. DMA-capable memory allocation for queues
        // 2. Setting up admin queue physical addresses in ASQ/ACQ
        // 3. Configuring queue attributes in AQA
        // 4. Enabling the controller
        // 5. Creating I/O queues
        // 6. Identifying namespaces

        println!("[NVME] Controller initialized (stub - requires DMA)");

        Ok(())
    }

    /// Submit a command to the admin queue and poll for completion.
    ///
    /// Writes the command to the next available submission queue slot,
    /// rings the admin submission queue doorbell (offset 0x1000), and
    /// spins waiting for a matching completion queue entry.
    fn submit_admin_command(&mut self, cmd: SubmissionQueueEntry) -> Result<u32, KernelError> {
        let mmio = self.mmio_base;
        let queue = self
            .admin_queue
            .as_mut()
            .ok_or(KernelError::NotInitialized {
                subsystem: "NVMe admin queue",
            })?;

        let tail = queue.sq_tail.load(core::sync::atomic::Ordering::Relaxed);
        let idx = tail as usize % queue.queue_size as usize;

        // Write command to submission queue.
        queue.submission_queue[idx] = cmd;

        // Advance tail.
        let new_tail = (tail + 1) % queue.queue_size;
        queue
            .sq_tail
            .store(new_tail, core::sync::atomic::Ordering::Release);

        // Ring admin SQ doorbell (offset 0x1000 for queue 0).
        // SAFETY: MMIO write to NVMe doorbell register.
        unsafe {
            core::ptr::write_volatile((mmio + 0x1000) as *mut u32, new_tail as u32);
        }

        // Poll completion queue for response (admin queue = CQ 0).
        let cq_head = queue.cq_head.load(core::sync::atomic::Ordering::Relaxed);
        let cq_idx = cq_head as usize % queue.queue_size as usize;

        let mut timeout = 100_000u32;
        loop {
            let status = queue.completion_queue[cq_idx].status;
            // Phase bit check: completion entries toggle phase on wrap.
            if status & 1 != 0 || timeout == 0 {
                break;
            }
            timeout -= 1;
            core::hint::spin_loop();
        }

        if timeout == 0 {
            return Err(KernelError::Timeout {
                operation: "NVMe admin command",
                duration_ms: 100,
            });
        }

        let result = queue.completion_queue[cq_idx].result;
        let new_head = (cq_head + 1) % queue.queue_size;
        queue
            .cq_head
            .store(new_head, core::sync::atomic::Ordering::Release);

        // Ring admin CQ doorbell (offset 0x1000 + 1 * doorbell_stride for CQ 0).
        // SAFETY: MMIO write to NVMe doorbell register.
        unsafe {
            core::ptr::write_volatile((mmio + 0x1004) as *mut u32, new_head as u32);
        }

        Ok(result)
    }

    /// Create an I/O queue pair.
    ///
    /// Sends Create I/O Completion Queue and Create I/O Submission Queue
    /// admin commands to set up an I/O queue for block operations.
    fn create_io_queue(&mut self, queue_id: u16, queue_size: u16) -> Result<(), KernelError> {
        let qp = QueuePair::new(queue_size);

        // Create I/O Completion Queue (admin opcode 0x05).
        let mut cq_cmd = SubmissionQueueEntry::new();
        cq_cmd.opcode = ADMIN_CREATE_CQ;
        cq_cmd.cdw10 = ((queue_size as u32 - 1) << 16) | queue_id as u32;
        cq_cmd.cdw11 = 1; // physically contiguous, interrupts enabled
        let _ = self.submit_admin_command(cq_cmd)?;

        // Create I/O Submission Queue (admin opcode 0x01).
        let mut sq_cmd = SubmissionQueueEntry::new();
        sq_cmd.opcode = ADMIN_CREATE_SQ;
        sq_cmd.cdw10 = ((queue_size as u32 - 1) << 16) | queue_id as u32;
        sq_cmd.cdw11 = (queue_id as u32) << 16 | 1; // CQ ID + physically contiguous
        let _ = self.submit_admin_command(sq_cmd)?;

        self.io_queues.push(qp);
        println!(
            "[NVME] Created I/O queue pair {} (size={})",
            queue_id, queue_size
        );
        Ok(())
    }

    /// Submit an I/O read command to the specified queue.
    fn submit_io_read(
        &self,
        queue_idx: usize,
        start_lba: u64,
        num_blocks: u16,
        prp1: u64,
    ) -> Result<(), KernelError> {
        if queue_idx >= self.io_queues.len() {
            return Err(KernelError::InvalidArgument {
                name: "queue_idx",
                value: "exceeds number of I/O queues",
            });
        }

        let queue = &self.io_queues[queue_idx];
        let tail = queue.sq_tail.load(core::sync::atomic::Ordering::Relaxed);
        let idx = tail as usize % queue.queue_size as usize;

        let mut cmd = SubmissionQueueEntry::new();
        cmd.opcode = IO_READ;
        cmd.nsid = 1; // Namespace 1
        cmd.prp1 = prp1;
        cmd.cdw10 = (start_lba & 0xFFFF_FFFF) as u32;
        cmd.cdw11 = (start_lba >> 32) as u32;
        cmd.cdw12 = (num_blocks - 1) as u32; // 0-based count

        // SAFETY: We own this queue slot exclusively via the atomic tail index.
        // No other code writes to submission_queue[idx] until we advance the tail.
        unsafe {
            let sq_ptr = queue.submission_queue.as_ptr() as *mut SubmissionQueueEntry;
            core::ptr::write(sq_ptr.add(idx), cmd);
        }

        let new_tail = (tail + 1) % queue.queue_size;
        queue
            .sq_tail
            .store(new_tail, core::sync::atomic::Ordering::Release);

        // Ring I/O SQ doorbell: offset 0x1000 + (2 * queue_id) * doorbell_stride.
        let doorbell_offset = 0x1000 + (2 * (queue_idx + 1)) * 4;
        self.write_reg(doorbell_offset, new_tail as u32);

        Ok(())
    }

    /// Read blocks using the first I/O queue.
    fn read_blocks_internal(&self, start_block: u64, buffer: &mut [u8]) -> Result<(), KernelError> {
        if self.io_queues.is_empty() {
            // No I/O queues initialized -- return zeros (stub behavior).
            buffer.fill(0);
            return Ok(());
        }

        let num_blocks = (buffer.len() / self.block_size) as u16;

        // For actual DMA, we would allocate a DMA buffer, submit the command
        // with the DMA physical address as PRP1, wait for completion, then
        // copy from the DMA buffer to the user buffer. Since DMA buffer
        // allocation is done via iommu::alloc_dma_buffer(), we use a stub
        // PRP address of 0 which won't transfer real data.
        let _ = self.submit_io_read(0, start_block, num_blocks, 0);

        // Poll for completion on the first I/O queue.
        let queue = &self.io_queues[0];
        let mut timeout = 100_000u32;
        let cq_head = queue.cq_head.load(core::sync::atomic::Ordering::Relaxed);
        let cq_idx = cq_head as usize % queue.queue_size as usize;

        loop {
            if queue.completion_queue[cq_idx].status & 1 != 0 || timeout == 0 {
                break;
            }
            timeout -= 1;
            core::hint::spin_loop();
        }

        // Advance CQ head and ring doorbell.
        let new_head = (cq_head + 1) % queue.queue_size;
        queue
            .cq_head
            .store(new_head, core::sync::atomic::Ordering::Release);
        let cq_doorbell = 0x1000 + 3 * 4; // CQ 1 doorbell = offset 0x100C
        self.write_reg(cq_doorbell, new_head as u32);

        Ok(())
    }

    /// Write blocks using the first I/O queue.
    fn write_blocks_internal(
        &mut self,
        start_block: u64,
        buffer: &[u8],
    ) -> Result<(), KernelError> {
        if self.io_queues.is_empty() {
            return Ok(());
        }

        let num_blocks = (buffer.len() / self.block_size) as u16;
        let queue = &self.io_queues[0];
        let tail = queue.sq_tail.load(core::sync::atomic::Ordering::Relaxed);
        let idx = tail as usize % queue.queue_size as usize;

        // Build I/O Write command.
        let mut cmd = SubmissionQueueEntry::new();
        cmd.opcode = IO_WRITE;
        cmd.nsid = 1;
        cmd.prp1 = 0; // Would be DMA phys addr
        cmd.cdw10 = (start_block & 0xFFFF_FFFF) as u32;
        cmd.cdw11 = (start_block >> 32) as u32;
        cmd.cdw12 = (num_blocks - 1) as u32;

        // SAFETY: Exclusive access via atomic tail index.
        unsafe {
            let sq_ptr = queue.submission_queue.as_ptr() as *mut SubmissionQueueEntry;
            core::ptr::write(sq_ptr.add(idx), cmd);
        }

        let new_tail = (tail + 1) % queue.queue_size;
        queue
            .sq_tail
            .store(new_tail, core::sync::atomic::Ordering::Release);

        // Ring I/O SQ doorbell.
        let sq_doorbell = 0x1000 + 2 * 4; // SQ 1 doorbell = offset 0x1008
        self.write_reg(sq_doorbell, new_tail as u32);

        // Poll for completion.
        let cq_head = queue.cq_head.load(core::sync::atomic::Ordering::Relaxed);
        let cq_idx = cq_head as usize % queue.queue_size as usize;
        let mut timeout = 100_000u32;
        loop {
            if queue.completion_queue[cq_idx].status & 1 != 0 || timeout == 0 {
                break;
            }
            timeout -= 1;
            core::hint::spin_loop();
        }

        let new_head = (cq_head + 1) % queue.queue_size;
        queue
            .cq_head
            .store(new_head, core::sync::atomic::Ordering::Release);
        let cq_doorbell = 0x1000 + 3 * 4; // CQ 1 doorbell
        self.write_reg(cq_doorbell, new_head as u32);

        Ok(())
    }
}

impl BlockDevice for NvmeController {
    fn name(&self) -> &str {
        "nvme0"
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.total_blocks
    }

    fn read_blocks(&self, start_block: u64, buffer: &mut [u8]) -> Result<(), KernelError> {
        if !buffer.len().is_multiple_of(self.block_size) {
            return Err(KernelError::InvalidArgument {
                name: "buffer_length",
                value: "not_multiple_of_block_size",
            });
        }

        self.read_blocks_internal(start_block, buffer)
    }

    fn write_blocks(&mut self, start_block: u64, buffer: &[u8]) -> Result<(), KernelError> {
        if !buffer.len().is_multiple_of(self.block_size) {
            return Err(KernelError::InvalidArgument {
                name: "buffer_length",
                value: "not_multiple_of_block_size",
            });
        }

        self.write_blocks_internal(start_block, buffer)
    }

    fn flush(&mut self) -> Result<(), KernelError> {
        // NVMe flush command would go here
        Ok(())
    }
}

/// NVMe PCI subclass code.
const NVME_SUBCLASS: u8 = 0x08;

/// Admin queue size (64 entries is the minimum guaranteed by spec).
const ADMIN_QUEUE_SIZE: u16 = 64;

/// Timeout iterations for controller ready polling.
const CONTROLLER_READY_TIMEOUT: u32 = 500_000;

/// NVMe Identify Controller data offsets.
const IDENT_SERIAL_OFFSET: usize = 4;
const IDENT_SERIAL_LEN: usize = 20;
const IDENT_MODEL_OFFSET: usize = 24;
const IDENT_MODEL_LEN: usize = 40;
const IDENT_FIRMWARE_OFFSET: usize = 64;
const IDENT_FIRMWARE_LEN: usize = 8;
const IDENT_MDTS_OFFSET: usize = 77;

/// Initialize an NVMe controller found at the given BAR0 physical address.
///
/// Performs the full NVMe initialization sequence:
/// 1. Map BAR0 into kernel virtual address space
/// 2. Reset the controller (CC.EN=0, wait CSTS.RDY=0)
/// 3. Allocate admin submission/completion queues via frame allocator
/// 4. Program AQA, ASQ, ACQ registers
/// 5. Enable controller (CC.EN=1), wait for CSTS.RDY=1
/// 6. Issue Identify Controller command to read device metadata
#[cfg(target_arch = "x86_64")]
fn initialize_nvme_controller(bar0_phys: u64) -> Result<(), KernelError> {
    use crate::mm::{phys_to_virt_addr, FRAME_ALLOCATOR, FRAME_SIZE};

    // Step 1: Map BAR0 MMIO region into kernel virtual space.
    let mmio_base = phys_to_virt_addr(bar0_phys) as usize;
    println!(
        "[NVME] MMIO base: phys={:#x} virt={:#x}",
        bar0_phys, mmio_base
    );

    // Helper: read 32-bit MMIO register.
    let read32 = |offset: usize| -> u32 {
        // SAFETY: Reading NVMe MMIO register. mmio_base is the BAR0 address
        // mapped through the kernel direct-map (phys + PHYS_MEM_OFFSET).
        // All offsets are within the NVMe register space (< 0x1000).
        unsafe { core::ptr::read_volatile((mmio_base + offset) as *const u32) }
    };

    // Helper: write 32-bit MMIO register.
    let write32 = |offset: usize, value: u32| {
        // SAFETY: Writing NVMe MMIO register. Same invariants as read32.
        unsafe { core::ptr::write_volatile((mmio_base + offset) as *mut u32, value) }
    };

    // Helper: write 64-bit MMIO register (used for ASQ/ACQ base addresses).
    let write64 = |offset: usize, value: u64| {
        // SAFETY: Writing 64-bit NVMe MMIO register (ASQ/ACQ base address).
        // The register pair is naturally aligned at offset 0x28 and 0x30.
        unsafe { core::ptr::write_volatile((mmio_base + offset) as *mut u64, value) }
    };

    // Read controller version.
    let version = read32(REG_VS);
    let ver_major = (version >> 16) & 0xFFFF;
    let ver_minor = (version >> 8) & 0xFF;
    println!("[NVME] Controller version: {}.{}", ver_major, ver_minor);

    // Read capabilities to determine max queue entries supported.
    let cap_lo = read32(REG_CAP) as u64;
    let cap_hi = read32(REG_CAP + 4) as u64;
    let cap = cap_lo | (cap_hi << 32);
    let mqes = ((cap & 0xFFFF) + 1) as u16;
    let admin_qsize = ADMIN_QUEUE_SIZE.min(mqes);
    println!(
        "[NVME] CAP={:#018x}, MQES={}, using admin queue size={}",
        cap, mqes, admin_qsize
    );

    // Step 2: Disable controller (CC.EN=0).
    write32(REG_CC, 0);

    // Wait for CSTS.RDY to clear.
    let mut timeout = CONTROLLER_READY_TIMEOUT;
    while (read32(REG_CSTS) & CSTS_RDY) != 0 {
        if timeout == 0 {
            println!("[NVME] Timeout waiting for controller disable");
            return Err(KernelError::Timeout {
                operation: "NVMe controller disable",
                duration_ms: 500,
            });
        }
        timeout -= 1;
        core::hint::spin_loop();
    }
    println!("[NVME] Controller disabled");

    // Step 3: Allocate physically contiguous memory for admin queues.
    // Admin Submission Queue: 64 entries x 64 bytes = 4096 bytes = 1 frame.
    // Admin Completion Queue: 64 entries x 16 bytes = 1024 bytes = 1 frame.
    let asq_frame = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: FRAME_SIZE,
            available: 0,
        })?;
    let acq_frame = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: FRAME_SIZE,
            available: 0,
        })?;

    let asq_phys = asq_frame.as_u64() * FRAME_SIZE as u64;
    let acq_phys = acq_frame.as_u64() * FRAME_SIZE as u64;

    // Zero the queue memory.
    let asq_virt = phys_to_virt_addr(asq_phys) as *mut u8;
    let acq_virt = phys_to_virt_addr(acq_phys) as *mut u8;
    // SAFETY: Writing to freshly allocated frames mapped via kernel direct-map.
    // Each frame is FRAME_SIZE (4096) bytes. We zero the entire frame.
    unsafe {
        core::ptr::write_bytes(asq_virt, 0, FRAME_SIZE);
        core::ptr::write_bytes(acq_virt, 0, FRAME_SIZE);
    }

    println!(
        "[NVME] Admin queues allocated: ASQ phys={:#x}, ACQ phys={:#x}",
        asq_phys, acq_phys
    );

    // Step 4: Program admin queue registers.
    // AQA: Admin Queue Attributes -- ASQ size in bits [27:16], ACQ size in bits
    // [11:0]. Sizes are 0-based (value N means N+1 entries).
    let aqa = (((admin_qsize - 1) as u32) << 16) | ((admin_qsize - 1) as u32);
    write32(REG_AQA, aqa);

    // ASQ: Admin Submission Queue base address (physical, page-aligned).
    write64(REG_ASQ, asq_phys);

    // ACQ: Admin Completion Queue base address (physical, page-aligned).
    write64(REG_ACQ, acq_phys);

    // Step 5: Enable controller.
    // CC register: EN=1, CSS=0 (NVM), MPS=0 (4KB pages), IOSQES=6 (64B), IOCQES=4
    // (16B).
    let cc_value =
        CC_ENABLE | CC_CSS_NVM | CC_MPS_4K | CC_AMS_RR | CC_SHN_NONE | CC_IOSQES | CC_IOCQES;
    write32(REG_CC, cc_value);
    println!("[NVME] Controller enable: CC={:#010x}", cc_value);

    // Wait for CSTS.RDY to assert.
    timeout = CONTROLLER_READY_TIMEOUT;
    loop {
        let csts = read32(REG_CSTS);
        if (csts & CSTS_RDY) != 0 {
            break;
        }
        if (csts & CSTS_CFS) != 0 {
            println!("[NVME] Controller fatal status during enable");
            // Free allocated frames before returning error.
            let _ = FRAME_ALLOCATOR.lock().free_frames(asq_frame, 1);
            let _ = FRAME_ALLOCATOR.lock().free_frames(acq_frame, 1);
            return Err(KernelError::HardwareError {
                device: "nvme",
                code: 2,
            });
        }
        if timeout == 0 {
            println!("[NVME] Timeout waiting for controller ready");
            let _ = FRAME_ALLOCATOR.lock().free_frames(asq_frame, 1);
            let _ = FRAME_ALLOCATOR.lock().free_frames(acq_frame, 1);
            return Err(KernelError::Timeout {
                operation: "NVMe controller enable",
                duration_ms: 500,
            });
        }
        timeout -= 1;
        core::hint::spin_loop();
    }
    println!("[NVME] Controller ready");

    // Step 6: Issue Identify Controller command (opcode 0x06, CNS=1).
    // Allocate a frame for the 4KB identify data buffer.
    let ident_frame = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: FRAME_SIZE,
            available: 0,
        })?;
    let ident_phys = ident_frame.as_u64() * FRAME_SIZE as u64;
    let ident_virt = phys_to_virt_addr(ident_phys) as *mut u8;

    // SAFETY: Zeroing freshly allocated identify data frame.
    unsafe {
        core::ptr::write_bytes(ident_virt, 0, FRAME_SIZE);
    }

    // Build Identify Controller submission queue entry.
    let asq_entries = asq_virt as *mut SubmissionQueueEntry;
    let identify_cmd = SubmissionQueueEntry {
        opcode: ADMIN_IDENTIFY,
        flags: 0,
        command_id: 1,
        nsid: 0,
        _reserved: 0,
        metadata: 0,
        prp1: ident_phys, // PRP1 points to identify data buffer
        prp2: 0,
        cdw10: 1, // CNS=1: Identify Controller
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    };

    // Write command to ASQ slot 0.
    // SAFETY: asq_entries points to the zeroed admin submission queue frame.
    // Slot 0 is within bounds (admin_qsize >= 1). The queue memory is
    // 4KB-aligned and large enough for 64 entries of 64 bytes each.
    unsafe {
        core::ptr::write_volatile(asq_entries, identify_cmd);
    }

    // Ring admin SQ doorbell (queue 0 SQ doorbell is at offset 0x1000).
    write32(0x1000, 1); // Tail = 1 (we wrote entry at index 0)

    // Poll admin completion queue for response.
    let acq_entries = acq_virt as *const CompletionQueueEntry;
    timeout = CONTROLLER_READY_TIMEOUT;
    loop {
        // SAFETY: Reading completion queue entry 0 from the ACQ frame.
        let cqe = unsafe { core::ptr::read_volatile(acq_entries) };
        // Phase bit (bit 0 of status) toggles on each wrap. Initially 0,
        // so the first valid completion has phase bit = 1.
        if (cqe.status & 1) != 0 {
            // Check for error in status field (bits 1-15).
            let status_code = (cqe.status >> 1) & 0x7FFF;
            if status_code != 0 {
                println!(
                    "[NVME] Identify Controller failed: status={:#x}",
                    status_code
                );
            } else {
                // Parse identify data.
                // SAFETY: ident_virt points to a 4KB frame filled by the
                // controller via DMA. Offsets are within the 4KB page and
                // we only read byte slices, so alignment is not an issue.
                unsafe {
                    // Serial number (bytes 4-23, ASCII, space-padded)
                    let sn_ptr = ident_virt.add(IDENT_SERIAL_OFFSET);
                    let sn_slice = core::slice::from_raw_parts(sn_ptr, IDENT_SERIAL_LEN);
                    if let Ok(sn) = core::str::from_utf8(sn_slice) {
                        println!("[NVME] Serial:   {}", sn.trim_end());
                    }

                    // Model number (bytes 24-63, ASCII, space-padded)
                    let mn_ptr = ident_virt.add(IDENT_MODEL_OFFSET);
                    let mn_slice = core::slice::from_raw_parts(mn_ptr, IDENT_MODEL_LEN);
                    if let Ok(mn) = core::str::from_utf8(mn_slice) {
                        println!("[NVME] Model:    {}", mn.trim_end());
                    }

                    // Firmware revision (bytes 64-71, ASCII)
                    let fr_ptr = ident_virt.add(IDENT_FIRMWARE_OFFSET);
                    let fr_slice = core::slice::from_raw_parts(fr_ptr, IDENT_FIRMWARE_LEN);
                    if let Ok(fr) = core::str::from_utf8(fr_slice) {
                        println!("[NVME] Firmware: {}", fr.trim_end());
                    }

                    // MDTS: Maximum Data Transfer Size (byte 77).
                    // Value N means max transfer = 2^N * min memory page size.
                    // 0 means no limit reported.
                    let mdts = *ident_virt.add(IDENT_MDTS_OFFSET);
                    if mdts > 0 {
                        let max_transfer = 1u64 << (12 + mdts as u64); // 4KB * 2^MDTS
                        println!(
                            "[NVME] MDTS:     {} (max transfer {} bytes)",
                            mdts, max_transfer
                        );
                    } else {
                        println!("[NVME] MDTS:     0 (no limit)");
                    }
                }
            }
            break;
        }
        if timeout == 0 {
            println!("[NVME] Timeout waiting for Identify Controller completion");
            break;
        }
        timeout -= 1;
        core::hint::spin_loop();
    }

    // Ring admin CQ doorbell (offset 0x1004 for CQ 0).
    write32(0x1004, 1); // Head = 1

    // Free the identify data buffer.
    let _ = FRAME_ALLOCATOR.lock().free_frames(ident_frame, 1);

    println!("[NVME] Admin queue initialization complete");
    Ok(())
}

/// Detect and initialize NVMe devices via PCI bus enumeration.
///
/// Scans the PCI bus for Mass Storage controllers with NVMe subclass
/// (class 0x01, subclass 0x08). On QEMU without NVMe devices, this
/// will simply report no devices found.
pub fn init() -> Result<(), KernelError> {
    println!("[NVME] Scanning PCI bus for NVMe controllers...");

    #[cfg(target_arch = "x86_64")]
    {
        let pci_bus = crate::drivers::pci::get_pci_bus().lock();
        let storage_devices =
            pci_bus.find_devices_by_class(crate::drivers::pci::class_codes::MASS_STORAGE);

        let mut nvme_count = 0;
        for dev in &storage_devices {
            if dev.subclass == NVME_SUBCLASS {
                nvme_count += 1;
                println!(
                    "[NVME] Found NVMe controller: {:04x}:{:04x} at {}:{}.{}",
                    dev.vendor_id,
                    dev.device_id,
                    dev.location.bus,
                    dev.location.device,
                    dev.location.function,
                );

                // Report BAR0 (NVMe MMIO register space)
                if let Some(bar) = dev.bars.first() {
                    match bar {
                        crate::drivers::pci::PciBar::Memory { address, size, .. } => {
                            println!("[NVME]   BAR0: MMIO at {:#x}, size {:#x}", address, size);
                        }
                        crate::drivers::pci::PciBar::Io { address, size } => {
                            println!("[NVME]   BAR0: I/O at {:#x}, size {:#x}", address, size);
                        }
                        crate::drivers::pci::PciBar::None => {}
                    }
                }

                // Full NVMe initialization: map BAR0, set up admin queues,
                // enable controller, and identify.
                if let Some(bar0_phys) = dev.bars.first().and_then(|b| b.get_memory_address()) {
                    if let Err(e) = initialize_nvme_controller(bar0_phys) {
                        println!("[NVME] Controller init failed: {:?}", e);
                    }
                }
            }
        }

        if nvme_count == 0 {
            println!("[NVME] No NVMe controllers found on PCI bus");
        } else {
            println!("[NVME] Found {} NVMe controller(s)", nvme_count);
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        println!("[NVME] NVMe PCI scanning not available on this architecture");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submission_queue_entry_size() {
        assert_eq!(core::mem::size_of::<SubmissionQueueEntry>(), 64);
    }

    #[test]
    fn test_completion_queue_entry_size() {
        assert_eq!(core::mem::size_of::<CompletionQueueEntry>(), 16);
    }
}

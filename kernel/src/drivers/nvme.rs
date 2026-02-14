//! NVMe (Non-Volatile Memory Express) Driver
//!
//! High-performance storage driver for NVMe SSDs using the BlockDevice trait.

// Hardware register offsets, command opcodes, and queue structures are defined
// per the NVMe specification. Many are retained for completeness even if the
// current stub driver only uses a subset.
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

    /// Submit command to admin queue (stub)
    fn submit_admin_command(&mut self, _cmd: SubmissionQueueEntry) -> Result<(), KernelError> {
        // TODO(phase4): Implement NVMe admin command submission with doorbell ringing
        Ok(())
    }

    /// Read blocks (stub implementation)
    fn read_blocks_internal(
        &self,
        _start_block: u64,
        _buffer: &mut [u8],
    ) -> Result<(), KernelError> {
        // TODO(phase4): Implement NVMe read: create I/O command, submit, wait, copy
        // from DMA

        Ok(())
    }

    /// Write blocks (stub implementation)
    fn write_blocks_internal(
        &mut self,
        _start_block: u64,
        _buffer: &[u8],
    ) -> Result<(), KernelError> {
        // TODO(phase4): Implement NVMe write: copy to DMA, create I/O command, submit,
        // wait

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

/// Detect and initialize NVMe devices
pub fn init() -> Result<(), KernelError> {
    println!("[NVME] Scanning for NVMe devices...");

    // NOTE: Full implementation would:
    // 1. Scan PCI bus for NVMe controllers
    // 2. Initialize each controller
    // 3. Register block devices with VFS

    println!("[NVME] NVMe driver initialized (stub - requires PCI scanning)");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_submission_queue_entry_size() {
        assert_eq!(core::mem::size_of::<SubmissionQueueEntry>(), 64);
    }

    #[test_case]
    fn test_completion_queue_entry_size() {
        assert_eq!(core::mem::size_of::<CompletionQueueEntry>(), 16);
    }
}

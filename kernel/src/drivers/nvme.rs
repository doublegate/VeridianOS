//! NVMe (Non-Volatile Memory Express) driver.
//!
//! A polled, single-I/O-queue driver for NVMe controllers on PCI.
//!
//! DMA rules (DRV-INC-01, N-11): every queue, PRP list and data buffer is a
//! frame from the frame allocator ([`DmaFrame`]) and the controller is only
//! given physical addresses. Data moves through bounce frames, so callers'
//! buffers never need to be physically contiguous or pinned. Completions are
//! matched by the phase tag and command id, a status error or a timeout is
//! returned as `Err`, and after a timeout the controller is reset before any
//! frame it might still write is reused or freed. If the reset itself does
//! not complete, the frames are leaked rather than returned to the
//! allocator.
//!
//! Interrupts are not used (CQ IEN=0); completions are polled.

use alloc::{sync::Arc, vec::Vec};
use core::mem::ManuallyDrop;

use spin::Mutex;

use crate::{
    drivers::dma_frame::DmaFrame, error::KernelError, fs::blockdev::BlockDevice, mm::FRAME_SIZE,
};

/// NVMe PCI subclass code (class 0x01 mass storage).
#[cfg(target_arch = "x86_64")]
const NVME_SUBCLASS: u8 = 0x08;

/// Register offsets.
const REG_CAP: usize = 0x00;
const REG_VS: usize = 0x08;
const REG_CC: usize = 0x14;
const REG_CSTS: usize = 0x1C;
const REG_AQA: usize = 0x24;
const REG_ASQ: usize = 0x28;
const REG_ACQ: usize = 0x30;
const REG_DOORBELL_BASE: usize = 0x1000;

/// CC: enable, NVM command set, 4 KiB pages, 64-byte SQ and 16-byte CQ
/// entries.
const CC_ENABLE: u32 = 1 << 0;
const CC_IOSQES: u32 = 6 << 16;
const CC_IOCQES: u32 = 4 << 20;

const CSTS_RDY: u32 = 1 << 0;
const CSTS_CFS: u32 = 1 << 1;

/// Admin opcodes.
const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;

/// I/O opcodes.
const IO_FLUSH: u8 = 0x00;
const IO_WRITE: u8 = 0x01;
const IO_READ: u8 = 0x02;

/// Queue sizes. 64 SQ entries of 64 bytes fill one frame exactly.
const ADMIN_QUEUE_SIZE: u16 = 64;
const IO_QUEUE_SIZE: u16 = 64;
const IO_QUEUE_ID: u16 = 1;

/// Largest transfer per command, in pages (bounce frames).
const MAX_TRANSFER_PAGES: usize = 8;

/// Command timeout. Controllers report a worst-case enable time in CAP.TO;
/// commands get a fixed bound.
const COMMAND_TIMEOUT_MS: u64 = 5_000;

/// Submission queue entry (64 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
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

/// Completion queue entry (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CompletionQueueEntry {
    result: u32,
    _reserved: u32,
    sq_head: u16,
    sq_id: u16,
    command_id: u16,
    /// Bit 0: phase tag. Bits 1..15: status field.
    status: u16,
}

/// Completion-queue cursor: the slot to read next and the phase tag a new
/// entry in it carries. The controller inverts the phase each time it wraps,
/// so an entry is new only if its phase bit equals `phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CqCursor {
    head: u16,
    phase: bool,
    size: u16,
}

impl CqCursor {
    fn new(size: u16) -> Self {
        Self {
            head: 0,
            phase: true,
            size,
        }
    }

    fn is_new(&self, status: u16) -> bool {
        (status & 1 != 0) == self.phase
    }

    fn advance(&mut self) {
        self.head += 1;
        if self.head == self.size {
            self.head = 0;
            self.phase = !self.phase;
        }
    }
}

/// Bounds check for an I/O: `count` blocks from `start` within a namespace
/// of `total` blocks, and a non-zero count (NLB is 0-based on the wire, so a
/// zero count would underflow to the maximum).
fn lba_range_ok(start: u64, count: u64, total: u64) -> bool {
    count != 0 && start.checked_add(count).is_some_and(|end| end <= total)
}

/// One submission/completion queue pair in DMA memory.
struct Queue {
    size: u16,
    sq: DmaFrame,
    cq: DmaFrame,
    sq_tail: u16,
    cursor: CqCursor,
}

/// All DMA memory the controller can write. Kept together so it can be
/// leaked as a unit if the controller cannot be stopped.
struct DmaMemory {
    admin: Queue,
    io: Option<Queue>,
    /// PRP list for transfers of more than two pages.
    prp_list: DmaFrame,
    /// Bounce pages for data.
    bounce: Vec<DmaFrame>,
}

/// Controller state behind the device lock.
struct Inner {
    mmio_base: usize,
    doorbell_stride: usize,
    dma: ManuallyDrop<DmaMemory>,
    next_cid: u16,
    /// Set after a timeout or fatal status; all I/O then fails.
    failed: bool,
    /// Whether the DMA memory may be freed on drop (the controller was
    /// stopped).
    dma_releasable: bool,
    block_size: usize,
    total_blocks: u64,
    max_transfer_pages: usize,
}

impl Inner {
    fn read32(&self, off: usize) -> u32 {
        // SAFETY: mmio_base maps BAR0 (direct map) and `off` is a register
        // offset inside it.
        unsafe { core::ptr::read_volatile((self.mmio_base + off) as *const u32) }
    }

    fn write32(&self, off: usize, v: u32) {
        // SAFETY: as read32.
        unsafe { core::ptr::write_volatile((self.mmio_base + off) as *mut u32, v) }
    }

    fn read64(&self, off: usize) -> u64 {
        u64::from(self.read32(off)) | (u64::from(self.read32(off + 4)) << 32)
    }

    fn write64(&self, off: usize, v: u64) {
        self.write32(off, v as u32);
        self.write32(off + 4, (v >> 32) as u32);
    }

    fn sq_doorbell(&self, qid: u16) -> usize {
        REG_DOORBELL_BASE + (2 * qid as usize) * self.doorbell_stride
    }

    fn cq_doorbell(&self, qid: u16) -> usize {
        REG_DOORBELL_BASE + (2 * qid as usize + 1) * self.doorbell_stride
    }

    /// Wait until `CSTS.RDY == ready`. Fails on fatal status or timeout.
    fn wait_ready(&self, ready: bool, timeout_ms: u64) -> Result<(), KernelError> {
        let mut deadline = Deadline::new(timeout_ms);
        loop {
            let csts = self.read32(REG_CSTS);
            if csts & CSTS_CFS != 0 && ready {
                return Err(KernelError::HardwareError {
                    device: "nvme",
                    code: 2,
                });
            }
            if (csts & CSTS_RDY != 0) == ready {
                return Ok(());
            }
            if deadline.expired() {
                return Err(KernelError::Timeout {
                    operation: "NVMe controller ready",
                    duration_ms: timeout_ms,
                });
            }
            core::hint::spin_loop();
        }
    }

    /// Disable the controller and wait for it to stop. Once CSTS.RDY is 0
    /// the controller performs no more DMA.
    fn disable(&mut self, timeout_ms: u64) -> bool {
        self.write32(REG_CC, 0);
        let stopped = self.wait_ready(false, timeout_ms).is_ok();
        self.dma_releasable = stopped;
        stopped
    }

    fn queue(&mut self, qid: u16) -> &mut Queue {
        if qid == 0 {
            &mut self.dma.admin
        } else {
            self.dma.io.as_mut().expect("I/O queue exists once created")
        }
    }

    /// Submit `cmd` on queue `qid` and poll for its completion.
    fn execute(&mut self, qid: u16, mut cmd: SubmissionQueueEntry) -> Result<u32, KernelError> {
        if self.failed {
            return Err(KernelError::InvalidState {
                expected: "nvme operational",
                actual: "failed",
            });
        }
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1);
        cmd.command_id = cid;

        let tail = {
            let q = self.queue(qid);
            let slot = (q.sq.virt as *mut SubmissionQueueEntry).wrapping_add(q.sq_tail as usize);
            // SAFETY: sq_tail < size and the SQ frame holds `size` entries
            // (size * 64 <= FRAME_SIZE); the controller only reads slots
            // between its head and our tail, and this one is past the tail.
            unsafe { core::ptr::write_volatile(slot, cmd) };
            q.sq_tail = (q.sq_tail + 1) % q.size;
            q.sq_tail
        };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.write32(self.sq_doorbell(qid), u32::from(tail));

        let mut deadline = Deadline::new(COMMAND_TIMEOUT_MS);
        loop {
            let q = self.queue(qid);
            let entry_ptr =
                (q.cq.virt as *const CompletionQueueEntry).wrapping_add(q.cursor.head as usize);
            // SAFETY: cq.head < size and the CQ frame holds `size` entries.
            // The controller writes it, hence the volatile read.
            let entry = unsafe { core::ptr::read_volatile(entry_ptr) };
            if q.cursor.is_new(entry.status) {
                core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
                q.cursor.advance();
                let head = q.cursor.head;
                self.write32(self.cq_doorbell(qid), u32::from(head));
                if entry.command_id != cid {
                    // With one command in flight per queue a different id
                    // means the controller is misbehaving.
                    self.fail();
                    return Err(KernelError::HardwareError {
                        device: "nvme",
                        code: 3,
                    });
                }
                let status = entry.status >> 1;
                if status != 0 {
                    return Err(KernelError::HardwareError {
                        device: "nvme",
                        code: u32::from(status),
                    });
                }
                return Ok(entry.result);
            }
            if deadline.expired() {
                // The command may still complete and DMA into our frames:
                // stop the controller before anything is reused.
                self.fail();
                return Err(KernelError::Timeout {
                    operation: "NVMe command",
                    duration_ms: COMMAND_TIMEOUT_MS,
                });
            }
            core::hint::spin_loop();
        }
    }

    /// Mark the controller failed and stop it.
    fn fail(&mut self) {
        self.failed = true;
        if !self.disable(COMMAND_TIMEOUT_MS) {
            crate::println!("[NVME] controller did not stop; its DMA memory is quarantined");
        }
    }

    /// Point PRP1/PRP2 at the first `pages` bounce frames.
    fn set_prps(&mut self, cmd: &mut SubmissionQueueEntry, pages: usize) {
        let dma = &mut *self.dma;
        cmd.prp1 = dma.bounce[0].phys;
        cmd.prp2 = match pages {
            1 => 0,
            2 => dma.bounce[1].phys,
            _ => {
                let list = dma.prp_list.virt as *mut u64;
                for (i, frame) in dma.bounce[1..pages].iter().enumerate() {
                    // SAFETY: i < MAX_TRANSFER_PAGES - 1, far below the 512
                    // entries a PRP-list frame holds.
                    unsafe { core::ptr::write_volatile(list.add(i), frame.phys) };
                }
                dma.prp_list.phys
            }
        };
    }

    /// Transfer whole blocks between `buf` and the namespace.
    fn transfer(&mut self, write: bool, start: u64, buf: &mut [u8]) -> Result<(), KernelError> {
        let bs = self.block_size;
        if bs == 0 || !buf.len().is_multiple_of(bs) {
            return Err(KernelError::InvalidArgument {
                name: "buffer_length",
                value: "not_multiple_of_block_size",
            });
        }
        let blocks = (buf.len() / bs) as u64;
        if !lba_range_ok(start, blocks, self.total_blocks) {
            return Err(KernelError::InvalidArgument {
                name: "lba",
                value: "out of range or empty",
            });
        }

        let max_bytes = self.max_transfer_pages * FRAME_SIZE;
        let mut lba = start;
        for chunk in buf.chunks_mut(max_bytes) {
            let nblocks = chunk.len() / bs;
            let pages = chunk.len().div_ceil(FRAME_SIZE);
            for (i, part) in chunk.chunks(FRAME_SIZE).enumerate() {
                let page = &mut self.dma.bounce[i].bytes_mut()[..part.len()];
                if write {
                    page.copy_from_slice(part);
                } else {
                    // Stale data from an earlier transfer must not be
                    // returned if the device transfers less than asked.
                    page.fill(0);
                }
            }
            let mut cmd = SubmissionQueueEntry {
                opcode: if write { IO_WRITE } else { IO_READ },
                nsid: 1,
                cdw10: lba as u32,
                cdw11: (lba >> 32) as u32,
                cdw12: (nblocks - 1) as u32, // 0-based; nblocks >= 1
                ..Default::default()
            };
            self.set_prps(&mut cmd, pages);
            self.execute(IO_QUEUE_ID, cmd)?;
            if !write {
                for (i, part) in chunk.chunks_mut(FRAME_SIZE).enumerate() {
                    let n = part.len();
                    part.copy_from_slice(&self.dma.bounce[i].bytes()[..n]);
                }
            }
            lba += nblocks as u64;
        }
        Ok(())
    }

    fn identify(&mut self, cns: u32, nsid: u32) -> Result<(), KernelError> {
        let cmd = SubmissionQueueEntry {
            opcode: ADMIN_IDENTIFY,
            nsid,
            prp1: self.dma.bounce[0].phys,
            cdw10: cns,
            ..Default::default()
        };
        self.execute(0, cmd).map(|_| ())
    }

    fn create_io_queue(&mut self) -> Result<(), KernelError> {
        let io = Queue::new(IO_QUEUE_SIZE)?;
        let (sq, cq) = (io.sq.phys, io.cq.phys);
        // Install before the controller can use it, so a failure below still
        // leaves the frames owned by DmaMemory.
        self.dma.io = Some(io);
        let qsize = u32::from(IO_QUEUE_SIZE - 1) << 16;
        // CQ: physically contiguous, interrupts off.
        self.execute(
            0,
            SubmissionQueueEntry {
                opcode: ADMIN_CREATE_CQ,
                prp1: cq,
                cdw10: qsize | u32::from(IO_QUEUE_ID),
                cdw11: 1,
                ..Default::default()
            },
        )?;
        // SQ: physically contiguous, bound to the CQ above.
        self.execute(
            0,
            SubmissionQueueEntry {
                opcode: ADMIN_CREATE_SQ,
                prp1: sq,
                cdw10: qsize | u32::from(IO_QUEUE_ID),
                cdw11: (u32::from(IO_QUEUE_ID) << 16) | 1,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.dma_releasable && !self.disable(COMMAND_TIMEOUT_MS) {
            // The controller may still write these frames: leak them.
            return;
        }
        // SAFETY: dropped exactly once, here, and the controller is stopped.
        unsafe { ManuallyDrop::drop(&mut self.dma) };
    }
}

impl Queue {
    fn new(size: u16) -> Result<Self, KernelError> {
        debug_assert!(size as usize * 64 <= FRAME_SIZE);
        Ok(Self {
            size,
            sq: DmaFrame::alloc()?,
            cq: DmaFrame::alloc()?,
            sq_tail: 0,
            cursor: CqCursor::new(size),
        })
    }
}

/// A millisecond deadline with an iteration backstop, so a missing tick
/// source cannot turn a timeout into a hang.
struct Deadline {
    start_ms: u64,
    limit_ms: u64,
    spins: u64,
}

impl Deadline {
    const MAX_SPINS: u64 = 2_000_000_000;

    fn new(limit_ms: u64) -> Self {
        Self {
            start_ms: crate::arch::timer::get_timestamp_ms(),
            limit_ms,
            spins: 0,
        }
    }

    fn expired(&mut self) -> bool {
        self.spins += 1;
        if self.spins >= Self::MAX_SPINS {
            return true;
        }
        // Reading the clock is cheaper than an MMIO read but not free.
        if !self.spins.is_multiple_of(1024) {
            return false;
        }
        crate::arch::timer::get_timestamp_ms().saturating_sub(self.start_ms) >= self.limit_ms
    }
}

/// An initialized NVMe controller exposing namespace 1 as a block device.
pub struct NvmeController {
    inner: Mutex<Inner>,
    name: alloc::string::String,
    block_size: usize,
    total_blocks: u64,
}

// SAFETY: all controller state, including the raw MMIO base and DMA frames,
// is behind `inner`'s mutex.
unsafe impl Send for NvmeController {}
// SAFETY: as above.
unsafe impl Sync for NvmeController {}

impl NvmeController {
    /// Bring up the controller whose BAR0 (`bar_size` bytes) is mapped at
    /// `mmio_base`.
    pub fn new(mmio_base: usize, bar_size: usize, index: usize) -> Result<Self, KernelError> {
        let mut bounce = Vec::with_capacity(MAX_TRANSFER_PAGES);
        for _ in 0..MAX_TRANSFER_PAGES {
            bounce.push(DmaFrame::alloc()?);
        }
        let dma = DmaMemory {
            admin: Queue::new(ADMIN_QUEUE_SIZE)?,
            io: None,
            prp_list: DmaFrame::alloc()?,
            bounce,
        };
        let mut inner = Inner {
            mmio_base,
            doorbell_stride: 4,
            dma: ManuallyDrop::new(dma),
            next_cid: 1,
            failed: false,
            dma_releasable: false,
            block_size: 0,
            total_blocks: 0,
            max_transfer_pages: MAX_TRANSFER_PAGES,
        };

        let vs = inner.read32(REG_VS);
        let cap = inner.read64(REG_CAP);
        let mqes = (cap & 0xFFFF) as u16 + 1;
        // CAP.TO: worst-case ready time in 500 ms units.
        let ready_ms = (((cap >> 24) & 0xFF).max(1)) * 500;
        inner.doorbell_stride = 4 << ((cap >> 32) & 0xF);
        // The highest doorbell used (I/O CQ) must lie inside BAR0.
        if inner.cq_doorbell(IO_QUEUE_ID) + 4 > bar_size {
            return Err(KernelError::HardwareError {
                device: "nvme",
                code: 6,
            });
        }
        crate::println!(
            "[NVME] version {}.{}, MQES {}, ready timeout {} ms",
            vs >> 16,
            (vs >> 8) & 0xFF,
            mqes,
            ready_ms
        );
        if mqes < ADMIN_QUEUE_SIZE.max(IO_QUEUE_SIZE) {
            return Err(KernelError::HardwareError {
                device: "nvme",
                code: 4,
            });
        }

        // Reset, program the admin queue, enable.
        if !inner.disable(ready_ms) {
            return Err(KernelError::Timeout {
                operation: "NVMe controller disable",
                duration_ms: ready_ms,
            });
        }
        let aq = u32::from(ADMIN_QUEUE_SIZE - 1);
        inner.write32(REG_AQA, (aq << 16) | aq);
        inner.write64(REG_ASQ, inner.dma.admin.sq.phys);
        inner.write64(REG_ACQ, inner.dma.admin.cq.phys);
        inner.dma_releasable = false;
        inner.write32(REG_CC, CC_ENABLE | CC_IOSQES | CC_IOCQES);
        inner.wait_ready(true, ready_ms)?;

        // Identify controller: MDTS limits the transfer size.
        inner.identify(1, 0)?;
        let mdts = inner.dma.bounce[0].bytes()[77];
        if mdts != 0 {
            let limit = 1usize << mdts.min(16);
            inner.max_transfer_pages = MAX_TRANSFER_PAGES.min(limit);
        }

        // Identify namespace 1: size and LBA format.
        inner.identify(0, 1)?;
        let ns = inner.dma.bounce[0].bytes();
        let nsze = u64::from_le_bytes(ns[0..8].try_into().expect("8 bytes"));
        let flbas = (ns[26] & 0xF) as usize;
        let lbads = ns[128 + flbas * 4 + 2];
        if !(9..=12).contains(&lbads) {
            // Only block sizes 512..4096 fit the one-frame bounce layout.
            inner.fail();
            return Err(KernelError::HardwareError {
                device: "nvme",
                code: 5,
            });
        }
        inner.block_size = 1 << lbads;
        inner.total_blocks = nsze;

        inner.create_io_queue()?;

        let block_size = inner.block_size;
        let total_blocks = inner.total_blocks;
        crate::println!(
            "[NVME] nvme{}n1: {} blocks of {} bytes, {} KiB per command",
            index,
            total_blocks,
            block_size,
            inner.max_transfer_pages * FRAME_SIZE / 1024
        );
        Ok(Self {
            inner: Mutex::new(inner),
            name: alloc::format!("nvme{}n1", index),
            block_size,
            total_blocks,
        })
    }
}

impl BlockDevice for NvmeController {
    fn name(&self) -> &str {
        &self.name
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.total_blocks
    }

    fn read_blocks(&self, start_block: u64, buffer: &mut [u8]) -> Result<(), KernelError> {
        self.inner.lock().transfer(false, start_block, buffer)
    }

    fn write_blocks(&mut self, start_block: u64, buffer: &[u8]) -> Result<(), KernelError> {
        // transfer() takes a mutable slice for the read direction only; a
        // write never modifies it, so copy into a scratch buffer.
        let mut data = buffer.to_vec();
        self.inner.get_mut().transfer(true, start_block, &mut data)
    }

    fn flush(&mut self) -> Result<(), KernelError> {
        let cmd = SubmissionQueueEntry {
            opcode: IO_FLUSH,
            nsid: 1,
            ..Default::default()
        };
        self.inner.get_mut().execute(IO_QUEUE_ID, cmd).map(|_| ())
    }
}

/// Controllers brought up at boot.
static CONTROLLERS: Mutex<Vec<Arc<Mutex<NvmeController>>>> = Mutex::new(Vec::new());

/// Run `f` on controller `index`, if present.
pub fn with_controller<R>(index: usize, f: impl FnOnce(&mut NvmeController) -> R) -> Option<R> {
    let ctrl = CONTROLLERS.lock().get(index).cloned()?;
    let mut guard = ctrl.lock();
    Some(f(&mut guard))
}

/// Number of controllers brought up.
pub fn controller_count() -> usize {
    CONTROLLERS.lock().len()
}

/// Find NVMe controllers on PCI and bring them up.
pub fn init() -> Result<(), KernelError> {
    #[cfg(target_arch = "x86_64")]
    {
        let devices = {
            let bus = crate::drivers::pci::get_pci_bus().lock();
            bus.find_devices_by_class(crate::drivers::pci::class_codes::MASS_STORAGE)
        };
        for dev in devices.iter().filter(|d| d.subclass == NVME_SUBCLASS) {
            let Some(crate::drivers::pci::PciBar::Memory {
                address: bar0,
                size: bar_size,
                ..
            }) = dev.bars.first().cloned()
            else {
                continue;
            };
            crate::println!(
                "[NVME] {:04x}:{:04x} at {}:{}.{}, BAR0 {:#x}",
                dev.vendor_id,
                dev.device_id,
                dev.location.bus,
                dev.location.device,
                dev.location.function,
                bar0
            );
            crate::drivers::pci::get_pci_bus()
                .lock()
                .enable_bus_master(dev.location);
            // The doorbells for the admin and I/O queue must fit in BAR0.
            if (bar_size as usize) < REG_DOORBELL_BASE + 16 {
                crate::println!("[NVME] BAR0 too small ({:#x})", bar_size);
                continue;
            }
            let mmio = match crate::mm::vas::map_mmio(bar0, bar_size as usize) {
                Ok(v) => v,
                Err(e) => {
                    crate::println!("[NVME] cannot map BAR0: {:?}", e);
                    continue;
                }
            };
            let index = controller_count();
            match NvmeController::new(mmio, bar_size as usize, index) {
                Ok(ctrl) => CONTROLLERS.lock().push(Arc::new(Mutex::new(ctrl))),
                Err(e) => crate::println!("[NVME] controller init failed: {:?}", e),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_sizes_match_spec() {
        assert_eq!(core::mem::size_of::<SubmissionQueueEntry>(), 64);
        assert_eq!(core::mem::size_of::<CompletionQueueEntry>(), 16);
        assert!(ADMIN_QUEUE_SIZE as usize * 64 <= FRAME_SIZE);
        assert!(IO_QUEUE_SIZE as usize * 64 <= FRAME_SIZE);
    }

    #[test]
    fn cq_phase_flips_on_wrap() {
        let mut c = CqCursor::new(2);
        // Fresh queue memory is zero: phase 0 entries are not new.
        assert!(!c.is_new(0));
        assert!(c.is_new(1));
        c.advance();
        assert_eq!((c.head, c.phase), (1, true));
        c.advance();
        assert_eq!((c.head, c.phase), (0, false));
        // After the wrap, the stale phase-1 entry is no longer new.
        assert!(!c.is_new(1));
        assert!(c.is_new(0));
    }

    #[test]
    fn lba_range_rejects_empty_and_overflow() {
        assert!(lba_range_ok(0, 1, 10));
        assert!(lba_range_ok(9, 1, 10));
        assert!(!lba_range_ok(9, 2, 10));
        assert!(!lba_range_ok(0, 0, 10)); // NLB would underflow
        assert!(!lba_range_ok(u64::MAX, 1, u64::MAX));
    }
}

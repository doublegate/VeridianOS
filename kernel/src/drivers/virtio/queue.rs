//! Virtqueue implementation
//!
//! Implements the split virtqueue data structure used by legacy virtio devices.
//! A virtqueue consists of three physically contiguous regions:
//!
//! 1. **Descriptor table** -- array of `VirtqDesc` entries describing data
//!    buffers
//! 2. **Available ring** -- driver-to-device: ring of descriptor chain heads
//! 3. **Used ring** -- device-to-driver: ring of completed descriptor chain
//!    heads
//!
//! Memory layout follows the virtio specification: descriptors at the base,
//! available ring immediately after, used ring page-aligned after that.

#![allow(dead_code)]

use core::sync::atomic::{self, Ordering};

use crate::mm::{FrameNumber, FRAME_ALLOCATOR, FRAME_SIZE};

/// Default queue size (power of 2, must match QEMU's virtio-blk queue size).
/// Legacy virtio requires the driver to use the exact queue size reported by
/// the device. QEMU's virtio-blk reports 256.
pub const DEFAULT_QUEUE_SIZE: u16 = 256;

/// Descriptor flag: buffer continues via the `next` field
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Descriptor flag: buffer is device-writable (device writes, driver reads)
pub const VIRTQ_DESC_F_WRITE: u16 = 2;
/// Descriptor flag: buffer contains a list of buffer descriptors (indirect)
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;

/// Virtqueue descriptor table entry.
///
/// Each descriptor points to a physically contiguous buffer in guest memory.
/// Descriptors can be chained via the `next` field when `VIRTQ_DESC_F_NEXT`
/// is set in `flags`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqDesc {
    /// Physical address of the guest buffer
    pub addr: u64,
    /// Length of the guest buffer in bytes
    pub len: u32,
    /// Descriptor flags (NEXT, WRITE, INDIRECT)
    pub flags: u16,
    /// Index of the next descriptor in the chain (valid if NEXT flag is set)
    pub next: u16,
}

/// Available ring: driver writes descriptor chain heads here for the device to
/// consume.
#[repr(C)]
pub struct VirtqAvail {
    /// Flags (e.g., VIRTQ_AVAIL_F_NO_INTERRUPT to suppress used-buffer
    /// notifications)
    pub flags: u16,
    /// Index of the next entry the driver will write to in `ring[]`
    pub idx: u16,
    /// Ring of descriptor chain head indices
    pub ring: [u16; DEFAULT_QUEUE_SIZE as usize],
}

/// Element in the used ring, returned by the device after processing.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqUsedElem {
    /// Index of the start of the used descriptor chain
    pub id: u32,
    /// Total bytes written into the descriptor chain buffers by the device
    pub len: u32,
}

/// Used ring: device writes completed descriptor chain heads here.
#[repr(C)]
pub struct VirtqUsed {
    /// Flags (e.g., VIRTQ_USED_F_NO_NOTIFY to suppress available-buffer
    /// notifications)
    pub flags: u16,
    /// Index of the next entry the device will write to in `ring[]`
    pub idx: u16,
    /// Ring of completed descriptor chain elements
    pub ring: [VirtqUsedElem; DEFAULT_QUEUE_SIZE as usize],
}

/// A split virtqueue managing descriptors, available ring, and used ring.
///
/// Owns the physical memory backing all three structures. The physical page
/// frame number (PFN) is communicated to the device so it can DMA directly.
pub struct VirtQueue {
    /// Number of entries (descriptors) in this queue
    size: u16,

    /// Pointer to the descriptor table in identity-mapped kernel memory
    desc: *mut VirtqDesc,

    /// Pointer to the available ring
    avail: *mut VirtqAvail,

    /// Pointer to the used ring
    used: *mut VirtqUsed,

    /// Head of the free descriptor list
    free_head: u16,

    /// Number of free descriptors remaining
    num_free: u16,

    /// Last seen used ring index (for polling completion)
    last_used_idx: u16,

    /// Physical frame number of the queue memory (for QUEUE_ADDRESS register)
    queue_pfn: u32,

    /// Number of contiguous frames allocated for this queue
    num_frames: usize,

    /// First frame allocated (for freeing)
    first_frame: FrameNumber,

    /// Physical base address of the queue allocation
    phys_base: u64,

    /// Offsets of sub-structures from phys_base
    desc_offset: usize,
    avail_offset: usize,
    used_offset: usize,
}

impl VirtQueue {
    /// Allocate and initialize a new virtqueue.
    ///
    /// Allocates physically contiguous memory for the descriptor table,
    /// available ring, and used ring. The memory is zeroed and the free
    /// descriptor list is linked.
    ///
    /// `size` is typically read from the device via `QUEUE_SIZE` register
    /// and should be a power of 2. If `size` is 0 or exceeds our compiled
    /// maximum, we clamp to `DEFAULT_QUEUE_SIZE`.
    pub fn new(size: u16) -> Result<Self, crate::error::KernelError> {
        // Clamp to our compiled-in maximum
        let size = if size == 0 || size > DEFAULT_QUEUE_SIZE {
            DEFAULT_QUEUE_SIZE
        } else {
            size
        };

        // Calculate memory layout per virtio spec:
        //   descriptors: 16 bytes * queue_size
        //   available ring: 4 + 2 * queue_size bytes (+ padding)
        //   used ring: 4 + 8 * queue_size bytes
        //
        // Used ring must be page-aligned.
        let desc_size = 16 * size as usize;
        let avail_size = 4 + 2 * size as usize;
        let used_offset = align_up(desc_size + avail_size, FRAME_SIZE);
        let used_size = 4 + 8 * size as usize;
        let total_size = used_offset + used_size;
        let num_frames = total_size.div_ceil(FRAME_SIZE);

        // Allocate physically contiguous frames
        let first_frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(num_frames, None)
            .map_err(|_| crate::error::KernelError::OutOfMemory {
                requested: total_size,
                available: 0,
            })?;

        let phys_base = first_frame.as_u64() * FRAME_SIZE as u64;
        let virt_base = phys_to_kernel_virt(phys_base);

        // Zero the entire allocation
        // SAFETY: virt_base points to freshly allocated, identity-mapped memory
        // of size `num_frames * FRAME_SIZE` bytes. No other references exist.
        unsafe {
            core::ptr::write_bytes(virt_base as *mut u8, 0, num_frames * FRAME_SIZE);
        }

        let desc_ptr = virt_base as *mut VirtqDesc;
        let avail_ptr = (virt_base + desc_size) as *mut VirtqAvail;
        let used_ptr = (virt_base + used_offset) as *mut VirtqUsed;

        // Initialize the free descriptor chain: each descriptor's `next` field
        // points to the subsequent descriptor, forming a singly-linked free list.
        // SAFETY: desc_ptr points to zeroed memory of `size` VirtqDesc entries.
        // No other references to this memory exist.
        unsafe {
            for i in 0..size {
                let desc = &mut *desc_ptr.add(i as usize);
                desc.next = if i + 1 < size { i + 1 } else { 0 };
                desc.flags = 0;
                desc.addr = 0;
                desc.len = 0;
            }
        }

        Ok(Self {
            size,
            desc: desc_ptr,
            avail: avail_ptr,
            used: used_ptr,
            free_head: 0,
            num_free: size,
            last_used_idx: 0,
            queue_pfn: (phys_base / FRAME_SIZE as u64) as u32,
            num_frames,
            first_frame,
            phys_base,
            desc_offset: 0,
            avail_offset: desc_size,
            used_offset,
        })
    }

    /// Get the physical page frame number for the QUEUE_ADDRESS register.
    pub fn pfn(&self) -> u32 {
        self.queue_pfn
    }

    /// Physical addresses for mmio transports (virtio-mmio expects 64-bit phys)
    pub fn phys_desc(&self) -> u64 {
        self.phys_base + self.desc_offset as u64
    }

    pub fn phys_avail(&self) -> u64 {
        self.phys_base + self.avail_offset as u64
    }

    pub fn phys_used(&self) -> u64 {
        self.phys_base + self.used_offset as u64
    }

    /// Get the queue size.
    pub fn size(&self) -> u16 {
        self.size
    }

    /// Allocate a single free descriptor, returning its index.
    ///
    /// Returns `None` if all descriptors are in use.
    pub fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }

        let idx = self.free_head;
        // SAFETY: `idx` is within [0, size) because the free list is initialized
        // with valid indices and we only ever store indices < size.
        let desc = unsafe { &*self.desc.add(idx as usize) };
        self.free_head = desc.next;
        self.num_free -= 1;

        Some(idx)
    }

    /// Return a descriptor to the free list.
    pub fn free_desc(&mut self, idx: u16) {
        debug_assert!((idx as usize) < self.size as usize);

        // SAFETY: `idx` is within bounds (asserted above). We relink it into
        // the free list by updating its `next` field.
        unsafe {
            let desc = &mut *self.desc.add(idx as usize);
            desc.next = self.free_head;
            desc.flags = 0;
            desc.addr = 0;
            desc.len = 0;
        }
        self.free_head = idx;
        self.num_free += 1;
    }

    /// Free a chain of descriptors linked via NEXT flags, starting at `head`.
    pub fn free_chain(&mut self, head: u16) {
        let mut idx = head;
        loop {
            debug_assert!((idx as usize) < self.size as usize);
            // SAFETY: idx is in bounds (asserted). We read flags/next before freeing.
            let (flags, next) = unsafe {
                let desc = &*self.desc.add(idx as usize);
                (desc.flags, desc.next)
            };
            self.free_desc(idx);
            if flags & VIRTQ_DESC_F_NEXT == 0 {
                break;
            }
            idx = next;
        }
    }

    /// Write a descriptor's fields.
    ///
    /// # Safety
    ///
    /// `idx` must be a valid descriptor index (< queue size). `phys_addr` must
    /// point to a valid guest physical buffer of at least `len` bytes that will
    /// remain valid until the device returns the descriptor via the used ring.
    pub unsafe fn write_desc(&mut self, idx: u16, phys_addr: u64, len: u32, flags: u16, next: u16) {
        debug_assert!((idx as usize) < self.size as usize);
        // SAFETY: idx is in bounds (asserted). The caller guarantees phys_addr
        // and len are valid.
        let desc = unsafe { &mut *self.desc.add(idx as usize) };
        desc.addr = phys_addr;
        desc.len = len;
        desc.flags = flags;
        desc.next = next;
    }

    /// Push a descriptor chain head onto the available ring and advance the
    /// available index.
    ///
    /// The caller must call `kick()` (via the transport) after one or more
    /// `push_avail()` calls to notify the device.
    pub fn push_avail(&mut self, desc_head: u16) {
        // SAFETY: self.avail points to valid VirtqAvail memory we own.
        unsafe {
            let avail = &mut *self.avail;
            let ring_idx = avail.idx as usize % self.size as usize;
            avail.ring[ring_idx] = desc_head;

            // Write barrier: ensure the descriptor table writes and ring entry
            // write above are visible before we update the available index.
            atomic::fence(Ordering::Release);

            avail.idx = avail.idx.wrapping_add(1);
        }
    }

    /// Poll the used ring for a completed buffer.
    ///
    /// Returns `Some((chain_head_index, bytes_written))` if the device has
    /// returned a buffer, or `None` if no new completions are available.
    ///
    /// The caller should free the returned descriptor chain via `free_chain()`.
    pub fn poll_used(&mut self) -> Option<(u16, u32)> {
        // Read barrier: ensure we see the device's writes to the used ring
        // before we read the index.
        atomic::fence(Ordering::Acquire);

        // SAFETY: self.used points to valid VirtqUsed memory we own.
        let used_idx = unsafe { (*self.used).idx };

        if self.last_used_idx == used_idx {
            return None;
        }

        let ring_idx = self.last_used_idx as usize % self.size as usize;
        // SAFETY: ring_idx is modular-reduced to within [0, size).
        let elem = unsafe { (*self.used).ring[ring_idx] };

        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Some((elem.id as u16, elem.len))
    }

    /// Check if any completions are pending without consuming them.
    pub fn has_used(&self) -> bool {
        atomic::fence(Ordering::Acquire);
        // SAFETY: self.used is valid.
        let used_idx = unsafe { (*self.used).idx };
        self.last_used_idx != used_idx
    }
}

impl Drop for VirtQueue {
    fn drop(&mut self) {
        // Return physical frames to the allocator
        let _ = FRAME_ALLOCATOR
            .lock()
            .free_frames(self.first_frame, self.num_frames);
    }
}

// SAFETY: VirtQueue manages raw pointers to memory it owns exclusively.
// The physical DMA buffers are not shared with other Rust objects. Access
// is serialized by the caller (the blk driver holds VirtQueue behind a Mutex).
unsafe impl Send for VirtQueue {}
// SAFETY: VirtQueue is always accessed behind a Mutex (in the global
// VIRTIO_BLK OnceLock<Mutex<VirtioBlkDevice>>), so only one thread can
// access the raw pointers at a time. The pointers themselves are stable
// (allocated once, freed only on drop).
unsafe impl Sync for VirtQueue {}

/// Align `value` up to the next multiple of `align`.
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Convert a physical address to a kernel-accessible virtual address.
///
/// On x86_64 with the bootloader's physical memory mapping, physical addresses
/// are accessible at `phys + physical_memory_offset`. On other architectures
/// or in early boot, low physical addresses may be identity-mapped.
fn phys_to_kernel_virt(phys: u64) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // Try the bootloader's physical memory offset first.
        if let Some(virt) = crate::arch::x86_64::msr::phys_to_virt(phys as usize) {
            return virt;
        }
        // Fallback: assume identity mapping in the higher-half window
        (phys + 0xFFFF_8000_0000_0000) as usize
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        // AArch64 and RISC-V: physical addresses are identity-mapped in the
        // kernel's address space during early boot.
        phys as usize
    }
}

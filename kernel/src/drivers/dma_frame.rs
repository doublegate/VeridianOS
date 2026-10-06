//! One frame of device-visible DMA memory.
//!
//! Drivers give devices the physical address (`phys`) and touch the memory
//! through the kernel direct map (`virt`). The frame is zeroed on allocation
//! and returned to the frame allocator on drop, so a driver must stop the
//! device (reset it) before dropping a frame the device may still write.
//! When that cannot be guaranteed -- a command timed out and the device did
//! not stop -- the owner must leak the frame (e.g. via `ManuallyDrop`).

use crate::{
    error::KernelError,
    mm::{FrameNumber, FRAME_ALLOCATOR, FRAME_SIZE},
};

/// A zeroed, frame-allocator-owned page for DMA.
pub(crate) struct DmaFrame {
    frame: FrameNumber,
    /// Physical (bus) address of the frame.
    pub(crate) phys: u64,
    /// Kernel virtual address of the frame (direct map).
    pub(crate) virt: usize,
}

impl DmaFrame {
    /// Allocate and zero one frame.
    pub(crate) fn alloc() -> Result<Self, KernelError> {
        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: FRAME_SIZE,
                available: 0,
            })?;
        let phys = frame.as_u64() * FRAME_SIZE as u64;
        let virt = crate::mm::phys_to_virt_addr(phys) as usize;
        // SAFETY: the frame was just allocated and is mapped by the kernel's
        // direct map at `virt`; nothing else references it.
        unsafe { core::ptr::write_bytes(virt as *mut u8, 0, FRAME_SIZE) };
        Ok(Self { frame, phys, virt })
    }

    /// The whole frame as bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        // SAFETY: `virt` maps a whole frame owned by this value.
        unsafe { core::slice::from_raw_parts(self.virt as *const u8, FRAME_SIZE) }
    }

    /// The whole frame as mutable bytes.
    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above; `&mut self` gives exclusive CPU-side access.
        unsafe { core::slice::from_raw_parts_mut(self.virt as *mut u8, FRAME_SIZE) }
    }
}

impl Drop for DmaFrame {
    fn drop(&mut self) {
        let _ = FRAME_ALLOCATOR.lock().free_frames(self.frame, 1);
    }
}

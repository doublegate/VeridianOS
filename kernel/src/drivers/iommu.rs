//! IOMMU (I/O Memory Management Unit) Foundation
//!
//! Provides DMA address translation and device isolation via Intel VT-d
//! (or equivalent on other architectures). This module handles:
//! - DMAR table detection from ACPI
//! - IOMMU unit discovery and capability reading
//! - Identity mapping for known-safe devices
//! - DMA coherency flags and scatter-gather list support
//!
//! Full IOMMU page table management is deferred to a later phase; this
//! module provides the foundation for safe DMA with identity mapping.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

// ---------------------------------------------------------------------------
// DMAR Table Structures (Intel VT-d DMA Remapping)
// ---------------------------------------------------------------------------

/// DMAR Remapping Structure types (per Intel VT-d specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum DmarStructureType {
    /// DMA Remapping Hardware Unit Definition (DRHD).
    Drhd = 0,
    /// Reserved Memory Region Reporting (RMRR).
    Rmrr = 1,
    /// ACPI Namespace Device Declaration (ANDD).
    Andd = 3,
}

/// A DMA Remapping Hardware Unit discovered from the ACPI DMAR table.
#[derive(Debug, Clone)]
pub struct DrhdUnit {
    /// Segment number (PCI segment group).
    pub segment: u16,
    /// Register base address (MMIO).
    pub register_base: u64,
    /// Whether this unit covers all PCI devices in the segment.
    pub include_all: bool,
    /// Device scope entries (bus:dev.fn tuples).
    pub device_scope: Vec<DeviceScope>,
}

/// A Reserved Memory Region Reporting entry.
#[derive(Debug, Clone)]
pub struct RmrrRegion {
    /// Segment number.
    pub segment: u16,
    /// Base address of reserved region.
    pub base_address: u64,
    /// Limit address (inclusive) of reserved region.
    pub limit_address: u64,
    /// Device scope entries that require this reserved region.
    pub device_scope: Vec<DeviceScope>,
}

/// Device scope entry within DRHD or RMRR structures.
#[derive(Debug, Clone, Copy)]
pub struct DeviceScope {
    /// Scope type (1=PCI endpoint, 2=PCI sub-hierarchy, 3=IOAPIC, etc.).
    pub scope_type: u8,
    /// Enumeration ID (IOAPIC ID or HPET number).
    pub enumeration_id: u8,
    /// Start bus number.
    pub start_bus: u8,
    /// Path entries (dev:fn pairs).
    pub path: [(u8, u8); 4],
    /// Number of valid path entries.
    pub path_len: u8,
}

impl DeviceScope {
    #[allow(dead_code)]
    const fn empty() -> Self {
        Self {
            scope_type: 0,
            enumeration_id: 0,
            start_bus: 0,
            path: [(0, 0); 4],
            path_len: 0,
        }
    }
}

/// Parsed DMAR (DMA Remapping) table information.
#[derive(Debug, Clone)]
pub struct DmarInfo {
    /// Host address width (physical address bits - 1).
    pub host_address_width: u8,
    /// Global flags from the DMAR header.
    pub flags: u8,
    /// DMA Remapping Hardware Units.
    pub drhd_units: Vec<DrhdUnit>,
    /// Reserved Memory Region Reporting entries.
    pub rmrr_regions: Vec<RmrrRegion>,
}

// ---------------------------------------------------------------------------
// Scatter-Gather List for multi-buffer DMA
// ---------------------------------------------------------------------------

/// A single entry in a scatter-gather list.
#[derive(Debug, Clone, Copy)]
pub struct ScatterGatherEntry {
    /// Physical address of the buffer segment.
    pub phys_addr: u64,
    /// Length of the buffer segment in bytes.
    pub length: u32,
}

/// Scatter-gather list for multi-buffer DMA transfers.
///
/// Devices like NVMe and network NICs use scatter-gather to describe
/// non-contiguous physical memory regions for a single logical transfer.
#[derive(Debug, Clone)]
pub struct ScatterGatherList {
    /// Entries in the scatter-gather list.
    pub entries: Vec<ScatterGatherEntry>,
    /// Total byte length across all entries.
    pub total_length: u64,
}

impl ScatterGatherList {
    /// Create an empty scatter-gather list.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            total_length: 0,
        }
    }

    /// Add a physical buffer segment to the scatter-gather list.
    pub fn add_entry(&mut self, phys_addr: u64, length: u32) {
        self.entries.push(ScatterGatherEntry { phys_addr, length });
        self.total_length += length as u64;
    }

    /// Number of entries in the list.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

impl Default for ScatterGatherList {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DMA Coherency
// ---------------------------------------------------------------------------

/// DMA coherency policy for buffer allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaCoherency {
    /// Hardware maintains cache coherency (default on x86).
    Coherent,
    /// Software must explicitly manage cache flushes.
    NonCoherent,
    /// Write-combining: optimized for sequential writes (framebuffers).
    WriteCombining,
}

/// DMA direction hint for buffer mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    /// Device reads from memory (host -> device).
    ToDevice,
    /// Device writes to memory (device -> host).
    FromDevice,
    /// Bidirectional DMA.
    Bidirectional,
}

/// A DMA-mapped buffer with coherency and direction tracking.
#[derive(Debug)]
pub struct DmaMappedBuffer {
    /// Virtual address of the buffer.
    pub virt_addr: usize,
    /// Physical address for DMA (may differ from actual phys with IOMMU).
    pub dma_addr: u64,
    /// Buffer size in bytes.
    pub size: usize,
    /// Coherency policy.
    pub coherency: DmaCoherency,
    /// Transfer direction.
    pub direction: DmaDirection,
}

// ---------------------------------------------------------------------------
// Global IOMMU state
// ---------------------------------------------------------------------------

/// Whether IOMMU/DMAR has been detected and initialized.
static IOMMU_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Parsed DMAR information (None if no DMAR table found).
static DMAR_STATE: Mutex<Option<DmarInfo>> = Mutex::new(None);

/// Check whether IOMMU support has been initialized.
pub fn is_initialized() -> bool {
    IOMMU_INITIALIZED.load(Ordering::Acquire)
}

/// Access parsed DMAR information.
pub fn with_dmar_info<R, F: FnOnce(&DmarInfo) -> R>(f: F) -> Option<R> {
    let guard = DMAR_STATE.lock();
    guard.as_ref().map(f)
}

// ---------------------------------------------------------------------------
// DMAR Table Parsing
// ---------------------------------------------------------------------------

/// Initialize the IOMMU subsystem by parsing the ACPI DMAR table.
///
/// This function reads the DMAR table from ACPI (if present), discovers
/// IOMMU hardware units and reserved memory regions, and stores the
/// parsed information for use by device drivers.
///
/// Non-fatal: returns Ok(false) if no DMAR table is found (common on
/// older hardware or VMs without IOMMU emulation).
#[cfg(target_arch = "x86_64")]
pub fn init() -> KernelResult<bool> {
    if IOMMU_INITIALIZED.load(Ordering::Acquire) {
        return Ok(true);
    }

    // Check if ACPI parser found a DMAR table during boot.
    let dmar_result = crate::arch::x86_64::acpi::with_acpi_info(|info| {
        if !info.has_dmar || info.dmar_address == 0 {
            // No DMAR table present. Expected on QEMU without
            // `-device intel-iommu`.
            return None::<DmarInfo>;
        }

        println!(
            "[IOMMU] DMAR table at phys {:#x}, len {} bytes",
            info.dmar_address, info.dmar_length
        );

        // DMAR table found. Full parsing of DRHD/RMRR structures
        // requires walking the variable-length remapping entries.
        // For now, report discovery; full VT-d page table setup
        // is deferred to Phase 6 (requires MMIO register programming).
        // TODO(phase6): Parse DRHD entries for register base addresses,
        // set up context tables and second-level page tables.
        Some(DmarInfo {
            host_address_width: 0,
            flags: 0,
            drhd_units: alloc::vec::Vec::new(),
            rmrr_regions: alloc::vec::Vec::new(),
        })
    });

    match dmar_result {
        Some(Some(info)) => {
            let num_units = info.drhd_units.len();
            let num_rmrr = info.rmrr_regions.len();
            *DMAR_STATE.lock() = Some(info);
            IOMMU_INITIALIZED.store(true, Ordering::Release);
            println!(
                "[IOMMU] DMAR parsed: {} DRHD units, {} RMRR regions",
                num_units, num_rmrr
            );
            Ok(true)
        }
        _ => {
            println!("[IOMMU] No DMAR table found (IOMMU not available)");
            // Still mark as initialized (with no IOMMU) so callers can
            // distinguish "not checked" from "checked, not present".
            IOMMU_INITIALIZED.store(true, Ordering::Release);
            Ok(false)
        }
    }
}

/// Initialize IOMMU on non-x86 architectures (stub).
#[cfg(not(target_arch = "x86_64"))]
pub fn init() -> KernelResult<bool> {
    println!("[IOMMU] IOMMU not supported on this architecture");
    IOMMU_INITIALIZED.store(true, Ordering::Release);
    Ok(false)
}

// ---------------------------------------------------------------------------
// Identity Mapping (Phase 1 of IOMMU support)
// ---------------------------------------------------------------------------

/// Create an identity mapping for a DMA region.
///
/// In identity mapping mode, DMA addresses equal physical addresses.
/// This is the simplest IOMMU configuration and is used as a first step
/// before full page-table-based translation is implemented.
///
/// Returns the DMA address (equal to phys_addr in identity mapping mode).
pub fn identity_map_dma(phys_addr: u64, size: usize) -> KernelResult<u64> {
    // Without IOMMU hardware, DMA addresses are always physical addresses.
    // When IOMMU is present but in identity mapping mode, the mapping is
    // set up in the IOMMU page tables to translate DMA addr -> same phys addr.
    let _ = size; // Used when IOMMU page tables are implemented.
    Ok(phys_addr)
}

/// Remove a DMA identity mapping.
pub fn unmap_dma(dma_addr: u64, size: usize) -> KernelResult<()> {
    // No-op in identity mapping mode.
    let _ = (dma_addr, size);
    Ok(())
}

// ---------------------------------------------------------------------------
// DMA Buffer Allocation Helpers
// ---------------------------------------------------------------------------

/// Allocate a physically contiguous DMA buffer.
///
/// Returns a `DmaMappedBuffer` with both virtual and DMA addresses.
/// The buffer is cache-coherent by default on x86_64.
pub fn alloc_dma_buffer(size: usize, direction: DmaDirection) -> KernelResult<DmaMappedBuffer> {
    let num_frames = size.div_ceil(4096);

    // Allocate contiguous physical frames.
    let frame = crate::mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(num_frames, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: size,
            available: 0,
        })?;

    let phys_addr = frame.as_u64() * 4096;
    let virt_addr = crate::mm::phys_to_virt_addr(phys_addr) as usize;

    // Identity map for DMA (physical address = DMA address).
    let dma_addr = identity_map_dma(phys_addr, size)?;

    // Zero the buffer.
    // SAFETY: virt_addr points to freshly allocated physical memory mapped
    // into the kernel's virtual address space via the bootloader's physical
    // memory mapping. The buffer is num_frames * 4096 bytes.
    unsafe {
        core::ptr::write_bytes(virt_addr as *mut u8, 0, num_frames * 4096);
    }

    Ok(DmaMappedBuffer {
        virt_addr,
        dma_addr,
        size: num_frames * 4096,
        coherency: DmaCoherency::Coherent, // x86 is always coherent
        direction,
    })
}

/// Free a DMA-mapped buffer.
pub fn free_dma_buffer(buffer: DmaMappedBuffer) -> KernelResult<()> {
    let num_frames = buffer.size / 4096;
    let frame_number = crate::mm::FrameNumber::new(buffer.dma_addr / 4096);

    // Remove DMA mapping.
    unmap_dma(buffer.dma_addr, buffer.size)?;

    // Return frames to the allocator.
    let _ = crate::mm::FRAME_ALLOCATOR
        .lock()
        .free_frames(frame_number, num_frames);

    Ok(())
}

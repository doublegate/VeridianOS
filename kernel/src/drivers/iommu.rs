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
// DMAR/DRHD Table Parsing
// ---------------------------------------------------------------------------

/// DMAR remapping structure type codes.
const DMAR_TYPE_DRHD: u16 = 0;
const DMAR_TYPE_RMRR: u16 = 1;

/// DMAR table header size: 36-byte ACPI SDT header + 1 byte host_address_width
/// + 1 byte flags + 10 reserved bytes = 48 bytes.
const DMAR_HEADER_SIZE: usize = 48;

/// Intel VT-d register offsets.
const _VTD_CAP_REG: u64 = 0x08;
const _VTD_ECAP_REG: u64 = 0x10;

/// Represents a parsed IOMMU hardware unit with its capabilities.
#[derive(Debug, Clone)]
pub struct IommuUnit {
    /// Register base address (MMIO).
    pub register_base: u64,
    /// PCI segment group.
    pub segment: u16,
    /// Capability register value (offset 0x08).
    pub capability: u64,
    /// Extended capability register value (offset 0x10).
    pub extended_capability: u64,
    /// Whether this unit covers all devices.
    pub include_all: bool,
}

/// IOMMU context table (256 entries per PCI bus, page-aligned).
///
/// Each entry maps a PCI device:function pair to a DMA domain with its
/// own set of page tables.
#[derive(Debug)]
pub struct IommuContextTable {
    /// Physical address of the context table page.
    pub phys_addr: u64,
    /// PCI bus number this table covers.
    pub bus: u8,
}

/// Parse a device scope entry from raw DMAR data.
///
/// Device scope entries appear within DRHD and RMRR structures.
/// Format: type(1) + length(1) + reserved(2) + enum_id(1) + start_bus(1) +
/// path(variable)
fn parse_device_scope(data: &[u8]) -> Vec<DeviceScope> {
    let mut scopes = Vec::new();
    let mut offset = 0;

    while offset + 6 <= data.len() {
        let scope_type = data[offset];
        let scope_len = data[offset + 1] as usize;

        if scope_len < 6 || offset + scope_len > data.len() {
            break;
        }

        let enumeration_id = data[offset + 4];
        let start_bus = data[offset + 5];

        // Parse path entries (dev:fn pairs, 2 bytes each)
        let path_start = offset + 6;
        let path_bytes = scope_len - 6;
        let num_path_entries = path_bytes / 2;
        let mut path = [(0u8, 0u8); 4];
        let path_count = num_path_entries.min(4);

        for (i, slot) in path.iter_mut().enumerate().take(path_count) {
            let p = path_start + i * 2;
            if p + 1 < data.len() {
                *slot = (data[p], data[p + 1]);
            }
        }

        scopes.push(DeviceScope {
            scope_type,
            enumeration_id,
            start_bus,
            path,
            path_len: path_count as u8,
        });

        offset += scope_len;
    }

    scopes
}

/// Parse the ACPI DMAR table from raw bytes.
///
/// Walks the variable-length remapping structure entries and extracts
/// DRHD (hardware unit) and RMRR (reserved memory) entries.
pub fn parse_dmar(dmar_data: &[u8]) -> KernelResult<DmarInfo> {
    if dmar_data.len() < DMAR_HEADER_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "DMAR table",
            value: "too small",
        });
    }

    // Host address width at offset 36 (after 36-byte ACPI SDT header)
    let host_address_width = dmar_data[36];
    let flags = dmar_data[37];

    let mut drhd_units = Vec::new();
    let mut rmrr_regions = Vec::new();

    let mut offset = DMAR_HEADER_SIZE;

    while offset + 4 <= dmar_data.len() {
        // Remapping structure header: type(2) + length(2)
        let struct_type = u16::from_le_bytes([dmar_data[offset], dmar_data[offset + 1]]);
        let struct_len =
            u16::from_le_bytes([dmar_data[offset + 2], dmar_data[offset + 3]]) as usize;

        if struct_len < 4 || offset + struct_len > dmar_data.len() {
            break;
        }

        match struct_type {
            DMAR_TYPE_DRHD => {
                // DRHD structure: type(2) + len(2) + flags(1) + reserved(1)
                // + segment(2) + register_base(8) + device_scope(variable)
                if struct_len >= 16 {
                    let drhd_flags = dmar_data[offset + 4];
                    let segment =
                        u16::from_le_bytes([dmar_data[offset + 6], dmar_data[offset + 7]]);
                    let register_base = u64::from_le_bytes([
                        dmar_data[offset + 8],
                        dmar_data[offset + 9],
                        dmar_data[offset + 10],
                        dmar_data[offset + 11],
                        dmar_data[offset + 12],
                        dmar_data[offset + 13],
                        dmar_data[offset + 14],
                        dmar_data[offset + 15],
                    ]);

                    let include_all = (drhd_flags & 0x01) != 0;

                    // Parse device scope entries (after the 16-byte DRHD header)
                    let scope_data = if struct_len > 16 {
                        &dmar_data[offset + 16..offset + struct_len]
                    } else {
                        &[]
                    };
                    let device_scope = parse_device_scope(scope_data);

                    crate::println!(
                        "[IOMMU]   DRHD: seg={}, base={:#x}, include_all={}, scopes={}",
                        segment,
                        register_base,
                        include_all,
                        device_scope.len()
                    );

                    drhd_units.push(DrhdUnit {
                        segment,
                        register_base,
                        include_all,
                        device_scope,
                    });
                }
            }
            DMAR_TYPE_RMRR => {
                // RMRR structure: type(2) + len(2) + reserved(2) + segment(2)
                // + base_addr(8) + limit_addr(8) + device_scope(variable)
                if struct_len >= 24 {
                    let segment =
                        u16::from_le_bytes([dmar_data[offset + 6], dmar_data[offset + 7]]);
                    let base_address = u64::from_le_bytes([
                        dmar_data[offset + 8],
                        dmar_data[offset + 9],
                        dmar_data[offset + 10],
                        dmar_data[offset + 11],
                        dmar_data[offset + 12],
                        dmar_data[offset + 13],
                        dmar_data[offset + 14],
                        dmar_data[offset + 15],
                    ]);
                    let limit_address = u64::from_le_bytes([
                        dmar_data[offset + 16],
                        dmar_data[offset + 17],
                        dmar_data[offset + 18],
                        dmar_data[offset + 19],
                        dmar_data[offset + 20],
                        dmar_data[offset + 21],
                        dmar_data[offset + 22],
                        dmar_data[offset + 23],
                    ]);

                    let scope_data = if struct_len > 24 {
                        &dmar_data[offset + 24..offset + struct_len]
                    } else {
                        &[]
                    };
                    let device_scope = parse_device_scope(scope_data);

                    crate::println!(
                        "[IOMMU]   RMRR: seg={}, base={:#x}, limit={:#x}, scopes={}",
                        segment,
                        base_address,
                        limit_address,
                        device_scope.len()
                    );

                    rmrr_regions.push(RmrrRegion {
                        segment,
                        base_address,
                        limit_address,
                        device_scope,
                    });
                }
            }
            _ => {
                // Skip unknown structure types (ATSR=2, ANDD=3, SATC=4, etc.)
                crate::println!(
                    "[IOMMU]   Unknown DMAR structure type {} (len={})",
                    struct_type,
                    struct_len
                );
            }
        }

        offset += struct_len;
    }

    Ok(DmarInfo {
        host_address_width,
        flags,
        drhd_units,
        rmrr_regions,
    })
}

/// Initialize an IOMMU hardware unit from a parsed DRHD entry.
///
/// Reads capability registers from the MMIO region. Full translation
/// enablement (root table pointer, global command register) requires
/// MMIO mapping which is deferred until the VMM supports MMIO allocation.
pub fn init_iommu_unit(drhd: &DrhdUnit) -> KernelResult<IommuUnit> {
    // The register base needs to be MMIO-mapped to read capability registers.
    // For now, record the unit with zero capabilities (MMIO mapping is done
    // by the VMM when translation is actually enabled).
    crate::println!(
        "[IOMMU] Initializing IOMMU unit at {:#x} (segment {})",
        drhd.register_base,
        drhd.segment
    );

    Ok(IommuUnit {
        register_base: drhd.register_base,
        segment: drhd.segment,
        capability: 0,
        extended_capability: 0,
        include_all: drhd.include_all,
    })
}

/// Create an identity domain for DMA address translation.
///
/// In identity mapping mode, all DMA addresses translate to the same
/// physical address. This is the simplest IOMMU configuration and
/// allows devices to perform DMA without address translation overhead.
///
/// Returns the physical address of the identity-mapped root table page.
pub fn create_identity_domain() -> KernelResult<u64> {
    // Allocate a page for the root table (4096 bytes, 256 entries)
    let frame = crate::mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;

    let phys_addr = frame.as_u64() * 4096;
    let virt_addr = crate::mm::phys_to_virt_addr(phys_addr) as usize;

    // Zero the root table page
    // SAFETY: virt_addr points to a freshly allocated page mapped via
    // the bootloader's physical memory mapping. We zero it to create
    // an empty root table (all entries not-present).
    unsafe {
        core::ptr::write_bytes(virt_addr as *mut u8, 0, 4096);
    }

    crate::println!(
        "[IOMMU] Created identity domain root table at phys {:#x}",
        phys_addr
    );

    Ok(phys_addr)
}

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
    let dmar_data = crate::arch::x86_64::acpi::with_acpi_info(|info| {
        if !info.has_dmar || info.dmar_address == 0 {
            return None;
        }

        let addr = info.dmar_address as usize;
        let len = info.dmar_length as usize;

        crate::println!("[IOMMU] DMAR table at {:#x}, len {} bytes", addr, len);

        // SAFETY: dmar_address was captured from a valid ACPI table mapped
        // by the bootloader. The table remains in memory for the kernel's
        // lifetime. dmar_length was read from the table header.
        Some(unsafe { core::slice::from_raw_parts(addr as *const u8, len) })
    });

    let dmar_bytes = match dmar_data {
        Some(Some(bytes)) => bytes,
        _ => {
            crate::println!("[IOMMU] No DMAR table found (IOMMU not available)");
            IOMMU_INITIALIZED.store(true, Ordering::Release);
            return Ok(false);
        }
    };

    // Parse DRHD/RMRR structures from the DMAR table
    match parse_dmar(dmar_bytes) {
        Ok(info) => {
            let num_units = info.drhd_units.len();
            let num_rmrr = info.rmrr_regions.len();

            // Initialize each IOMMU unit (read capabilities)
            for drhd in &info.drhd_units {
                if let Err(e) = init_iommu_unit(drhd) {
                    crate::println!(
                        "[IOMMU] Warning: failed to init unit at {:#x}: {:?}",
                        drhd.register_base,
                        e
                    );
                }
            }

            *DMAR_STATE.lock() = Some(info);
            IOMMU_INITIALIZED.store(true, Ordering::Release);
            crate::println!(
                "[IOMMU] DMAR parsed: {} DRHD units, {} RMRR regions",
                num_units,
                num_rmrr
            );
            Ok(true)
        }
        Err(e) => {
            crate::println!("[IOMMU] DMAR parse error: {:?}", e);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dmar_too_small() {
        let data = [0u8; 10];
        assert!(parse_dmar(&data).is_err());
    }

    #[test]
    fn test_parse_dmar_empty() {
        // Minimal valid DMAR header (48 bytes) with no entries
        let mut data = [0u8; 48];
        // Signature "DMAR" at offset 0
        data[0] = b'D';
        data[1] = b'M';
        data[2] = b'A';
        data[3] = b'R';
        // Length = 48
        data[4] = 48;
        // Host address width at offset 36
        data[36] = 39; // 40-bit physical addresses
                       // Flags at offset 37
        data[37] = 0x01; // INTR_REMAP

        let info = parse_dmar(&data).unwrap();
        assert_eq!(info.host_address_width, 39);
        assert_eq!(info.flags, 0x01);
        assert!(info.drhd_units.is_empty());
        assert!(info.rmrr_regions.is_empty());
    }

    #[test]
    fn test_parse_dmar_with_drhd() {
        // DMAR header (48 bytes) + DRHD entry (16 bytes, no device scopes)
        let mut data = [0u8; 64];
        data[36] = 39; // host_address_width

        // DRHD at offset 48
        data[48] = 0; // type low
        data[49] = 0; // type high = 0 (DRHD)
        data[50] = 16; // length low
        data[51] = 0; // length high
        data[52] = 0x01; // flags: INCLUDE_PCI_ALL
                         // segment = 0
                         // register_base = 0xFED90000
        data[56] = 0x00;
        data[57] = 0x00;
        data[58] = 0xD9;
        data[59] = 0xFE;

        let info = parse_dmar(&data).unwrap();
        assert_eq!(info.drhd_units.len(), 1);
        assert!(info.drhd_units[0].include_all);
        assert_eq!(info.drhd_units[0].register_base, 0xFED9_0000);
        assert_eq!(info.drhd_units[0].segment, 0);
    }

    #[test]
    fn test_parse_dmar_with_rmrr() {
        // DMAR header (48 bytes) + RMRR entry (24 bytes, no device scopes)
        let mut data = [0u8; 72];
        data[36] = 39;

        // RMRR at offset 48
        data[48] = 1; // type low = RMRR
        data[49] = 0;
        data[50] = 24; // length
        data[51] = 0;
        // segment = 0 at offset 54-55
        // base_address = 0x000E0000 at offset 56-63
        data[56] = 0x00;
        data[57] = 0x00;
        data[58] = 0x0E;
        data[59] = 0x00;
        // limit_address = 0x000FFFFF at offset 64-71
        data[64] = 0xFF;
        data[65] = 0xFF;
        data[66] = 0x0F;
        data[67] = 0x00;

        let info = parse_dmar(&data).unwrap();
        assert_eq!(info.rmrr_regions.len(), 1);
        assert_eq!(info.rmrr_regions[0].base_address, 0x000E_0000);
        assert_eq!(info.rmrr_regions[0].limit_address, 0x000F_FFFF);
    }

    #[test]
    fn test_parse_device_scope() {
        // Device scope: type=1 (PCI endpoint), len=8, enum_id=0, bus=0, path=(2,0)
        let data = [1, 8, 0, 0, 0, 0, 2, 0];
        let scopes = parse_device_scope(&data);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].scope_type, 1);
        assert_eq!(scopes[0].start_bus, 0);
        assert_eq!(scopes[0].path[0], (2, 0));
        assert_eq!(scopes[0].path_len, 1);
    }

    #[test]
    fn test_parse_device_scope_empty() {
        let scopes = parse_device_scope(&[]);
        assert!(scopes.is_empty());
    }

    #[test]
    fn test_iommu_unit_init() {
        let drhd = DrhdUnit {
            segment: 0,
            register_base: 0xFED9_0000,
            include_all: true,
            device_scope: Vec::new(),
        };

        let unit = init_iommu_unit(&drhd).unwrap();
        assert_eq!(unit.register_base, 0xFED9_0000);
        assert_eq!(unit.segment, 0);
        assert!(unit.include_all);
    }

    #[test]
    fn test_scatter_gather_list() {
        let mut sgl = ScatterGatherList::new();
        assert_eq!(sgl.entry_count(), 0);
        assert_eq!(sgl.total_length, 0);

        sgl.add_entry(0x1000, 4096);
        sgl.add_entry(0x3000, 8192);

        assert_eq!(sgl.entry_count(), 2);
        assert_eq!(sgl.total_length, 4096 + 8192);
    }
}

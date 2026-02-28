//! ACPI table parser for x86_64.
//!
//! Parses ACPI tables from UEFI firmware to discover hardware topology:
//! - MADT (Multiple APIC Description Table): CPU enumeration, I/O APIC, ISA
//!   overrides
//! - MCFG (PCI Express config space base addresses)
//!
//! Not in scope: Full AML interpreter, ACPI namespace, runtime ACPI methods.

use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

// ---------------------------------------------------------------------------
// ACPI table signatures (4-byte ASCII)
// ---------------------------------------------------------------------------

const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";
const RSDT_SIGNATURE: &[u8; 4] = b"RSDT";
const XSDT_SIGNATURE: &[u8; 4] = b"XSDT";
const MADT_SIGNATURE: &[u8; 4] = b"APIC";
const MCFG_SIGNATURE: &[u8; 4] = b"MCFG";
const DMAR_SIGNATURE: &[u8; 4] = b"DMAR";
const SRAT_SIGNATURE: &[u8; 4] = b"SRAT";
const SLIT_SIGNATURE: &[u8; 4] = b"SLIT";

// ---------------------------------------------------------------------------
// MADT entry types
// ---------------------------------------------------------------------------

const MADT_LOCAL_APIC: u8 = 0;
const MADT_IO_APIC: u8 = 1;
const MADT_INTERRUPT_SOURCE_OVERRIDE: u8 = 2;
const MADT_LOCAL_APIC_NMI: u8 = 4;

// ---------------------------------------------------------------------------
// Maximum supported entries
// ---------------------------------------------------------------------------

const MAX_CPUS: usize = 16;
const MAX_IO_APICS: usize = 4;
const MAX_ISO: usize = 24;
const MAX_MCFG_ENTRIES: usize = 4;

// ---------------------------------------------------------------------------
// Parsed ACPI data structures
// ---------------------------------------------------------------------------

/// Local APIC entry from the MADT.
#[derive(Debug, Clone, Copy)]
pub struct MadtLocalApic {
    /// ACPI processor UID.
    pub acpi_processor_id: u8,
    /// Local APIC ID.
    pub apic_id: u8,
    /// Flags (bit 0: processor enabled, bit 1: online capable).
    pub flags: u32,
}

impl MadtLocalApic {
    /// Returns true if this processor is enabled or online-capable.
    pub fn is_usable(&self) -> bool {
        (self.flags & 0x01) != 0 || (self.flags & 0x02) != 0
    }
}

/// I/O APIC entry from the MADT.
#[derive(Debug, Clone, Copy)]
pub struct MadtIoApic {
    /// I/O APIC ID.
    pub id: u8,
    /// Physical base address of the I/O APIC MMIO registers.
    pub address: u32,
    /// Global System Interrupt base (first IRQ this I/O APIC handles).
    pub gsi_base: u32,
}

/// Interrupt Source Override entry from the MADT.
///
/// Maps ISA IRQ numbers to their actual GSI (Global System Interrupt)
/// numbers with polarity and trigger mode overrides.
#[derive(Debug, Clone, Copy)]
pub struct MadtIso {
    /// Bus source (always 0 = ISA).
    pub bus: u8,
    /// ISA IRQ number being overridden.
    pub irq_source: u8,
    /// Global System Interrupt number this IRQ maps to.
    pub gsi: u32,
    /// Flags: bits 1:0 = polarity, bits 3:2 = trigger mode.
    pub flags: u16,
}

impl MadtIso {
    /// Returns true if active-low polarity.
    pub fn is_active_low(&self) -> bool {
        (self.flags & 0x03) == 0x03
    }

    /// Returns true if level-triggered.
    pub fn is_level_triggered(&self) -> bool {
        ((self.flags >> 2) & 0x03) == 0x03
    }
}

/// PCIe Enhanced Configuration Mechanism entry from MCFG.
#[derive(Debug, Clone, Copy)]
pub struct McfgEntry {
    /// Base address of the PCIe ECAM region.
    pub base_address: u64,
    /// PCI segment group number.
    pub segment_group: u16,
    /// Start PCI bus number.
    pub start_bus: u8,
    /// End PCI bus number.
    pub end_bus: u8,
}

/// Parsed ACPI information, populated by `init()`.
#[derive(Debug)]
pub struct AcpiInfo {
    /// OEM ID from the RSDT/XSDT (6-byte ASCII, null-padded).
    pub oem_id: [u8; 6],
    /// Local APIC base address from the MADT.
    pub local_apic_address: u32,
    /// MADT flags (bit 0: dual 8259 PIC present).
    pub madt_flags: u32,
    /// Local APIC entries (one per CPU).
    pub local_apics: [Option<MadtLocalApic>; MAX_CPUS],
    /// Number of valid local APIC entries.
    pub local_apic_count: usize,
    /// I/O APIC entries.
    pub io_apics: [Option<MadtIoApic>; MAX_IO_APICS],
    /// Number of valid I/O APIC entries.
    pub io_apic_count: usize,
    /// Interrupt Source Override entries.
    pub isos: [Option<MadtIso>; MAX_ISO],
    /// Number of valid ISO entries.
    pub iso_count: usize,
    /// PCIe MCFG entries.
    pub mcfg_entries: [Option<McfgEntry>; MAX_MCFG_ENTRIES],
    /// Number of valid MCFG entries.
    pub mcfg_count: usize,
    /// Whether MADT was found and parsed.
    pub has_madt: bool,
    /// Whether MCFG was found and parsed.
    pub has_mcfg: bool,
    /// Whether DMAR (DMA Remapping) table was found.
    pub has_dmar: bool,
    /// Physical address of the DMAR table (for IOMMU driver to parse).
    pub dmar_address: u64,
    /// Length of the DMAR table in bytes.
    pub dmar_length: u32,
    /// ACPI revision (0 = ACPI 1.0, 2 = ACPI 2.0+).
    pub revision: u8,
    /// Whether SRAT (System Resource Affinity Table) was found.
    pub has_srat: bool,
    /// Virtual address of the SRAT table.
    pub srat_address: u64,
    /// Length of the SRAT table in bytes.
    pub srat_length: u32,
    /// Whether SLIT (System Locality Information Table) was found.
    pub has_slit: bool,
    /// Virtual address of the SLIT table.
    pub slit_address: u64,
    /// Length of the SLIT table in bytes.
    pub slit_length: u32,
}

impl AcpiInfo {
    const fn new() -> Self {
        Self {
            oem_id: [0; 6],
            local_apic_address: 0xFEE0_0000, // default
            madt_flags: 0,
            local_apics: [None; MAX_CPUS],
            local_apic_count: 0,
            io_apics: [None; MAX_IO_APICS],
            io_apic_count: 0,
            isos: [None; MAX_ISO],
            iso_count: 0,
            mcfg_entries: [None; MAX_MCFG_ENTRIES],
            mcfg_count: 0,
            has_madt: false,
            has_mcfg: false,
            has_dmar: false,
            dmar_address: 0,
            dmar_length: 0,
            revision: 0,
            has_srat: false,
            srat_address: 0,
            srat_length: 0,
            has_slit: false,
            slit_address: 0,
            slit_length: 0,
        }
    }

    /// Get the I/O APIC address (first one, or default 0xFEC0_0000).
    pub fn io_apic_address(&self) -> u32 {
        self.io_apics[0].map_or(0xFEC0_0000, |a| a.address)
    }

    /// Look up the GSI for a given ISA IRQ, applying interrupt source
    /// overrides.
    pub fn irq_to_gsi(&self, irq: u8) -> (u32, bool, bool) {
        for i in 0..self.iso_count {
            if let Some(ref iso) = self.isos[i] {
                if iso.irq_source == irq {
                    return (iso.gsi, iso.is_active_low(), iso.is_level_triggered());
                }
            }
        }
        // No override: identity map, edge-triggered, active-high
        (irq as u32, false, false)
    }

    /// Count usable CPUs.
    pub fn cpu_count(&self) -> usize {
        let mut count = 0;
        for i in 0..self.local_apic_count {
            if let Some(ref lapic) = self.local_apics[i] {
                if lapic.is_usable() {
                    count += 1;
                }
            }
        }
        count
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static ACPI_INITIALIZED: AtomicBool = AtomicBool::new(false);
static ACPI_INFO: Mutex<Option<AcpiInfo>> = Mutex::new(None);

/// Check whether ACPI tables have been parsed.
pub fn is_initialized() -> bool {
    ACPI_INITIALIZED.load(Ordering::Acquire)
}

/// Access the parsed ACPI info. Returns None if not initialized.
pub fn with_acpi_info<R, F: FnOnce(&AcpiInfo) -> R>(f: F) -> Option<R> {
    let guard = ACPI_INFO.lock();
    guard.as_ref().map(f)
}

// ---------------------------------------------------------------------------
// RSDP structures
// ---------------------------------------------------------------------------

/// RSDP (Root System Description Pointer) for ACPI 1.0.
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

/// Extended RSDP for ACPI 2.0+.
#[repr(C, packed)]
struct Rsdp2 {
    base: Rsdp,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    _reserved: [u8; 3],
}

/// Standard ACPI table header (present at the start of every ACPI table).
#[repr(C, packed)]
struct AcpiSdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

/// MADT header (follows standard header).
#[repr(C, packed)]
struct MadtHeader {
    sdt: AcpiSdtHeader,
    local_apic_address: u32,
    flags: u32,
}

/// MADT entry header (2 bytes: type + length).
#[repr(C, packed)]
struct MadtEntryHeader {
    entry_type: u8,
    length: u8,
}

/// MADT Local APIC entry (type 0).
#[repr(C, packed)]
struct MadtLocalApicEntry {
    header: MadtEntryHeader,
    acpi_processor_id: u8,
    apic_id: u8,
    flags: u32,
}

/// MADT I/O APIC entry (type 1).
#[repr(C, packed)]
struct MadtIoApicEntry {
    header: MadtEntryHeader,
    id: u8,
    _reserved: u8,
    address: u32,
    gsi_base: u32,
}

/// MADT Interrupt Source Override entry (type 2).
#[repr(C, packed)]
struct MadtIsoEntry {
    header: MadtEntryHeader,
    bus: u8,
    source: u8,
    gsi: u32,
    flags: u16,
}

/// MADT Local APIC NMI entry (type 4).
#[repr(C, packed)]
struct MadtLocalApicNmiEntry {
    header: MadtEntryHeader,
    acpi_processor_id: u8,
    flags: u16,
    lint: u8,
}

/// MCFG entry (one per PCI segment group).
#[repr(C, packed)]
struct McfgAllocation {
    base_address: u64,
    segment_group: u16,
    start_bus: u8,
    end_bus: u8,
    _reserved: u32,
}

// ---------------------------------------------------------------------------
// Checksum validation
// ---------------------------------------------------------------------------

/// Validate ACPI table checksum. All bytes must sum to zero (mod 256).
fn validate_checksum(addr: usize, len: usize) -> bool {
    let mut sum: u8 = 0;
    for i in 0..len {
        // SAFETY: addr..addr+len is within a valid ACPI table that was mapped
        // by the bootloader's physical memory mapping. We read individual bytes.
        sum = sum.wrapping_add(unsafe { *((addr + i) as *const u8) });
    }
    sum == 0
}

// ---------------------------------------------------------------------------
// Table parsing
// ---------------------------------------------------------------------------

/// Parse the MADT (Multiple APIC Description Table).
fn parse_madt(header_vaddr: usize, info: &mut AcpiInfo) {
    // SAFETY: header_vaddr points to a valid MADT table mapped by the
    // bootloader's physical memory offset. The packed struct layout matches
    // the ACPI specification.
    let madt = unsafe { &*(header_vaddr as *const MadtHeader) };
    let table_len = { madt.sdt.length } as usize;

    info.local_apic_address = madt.local_apic_address;
    info.madt_flags = madt.flags;
    info.has_madt = true;

    println!(
        "[ACPI] MADT: LAPIC addr={:#x}, flags={:#x}, len={}",
        info.local_apic_address, info.madt_flags, table_len
    );

    // Walk the variable-length entries after the fixed MADT header.
    let entries_start = header_vaddr + core::mem::size_of::<MadtHeader>();
    let entries_end = header_vaddr + table_len;
    let mut offset = entries_start;

    while offset + 2 <= entries_end {
        // SAFETY: offset points within the MADT table bounds. The entry
        // header is 2 bytes (type + length).
        let entry_header = unsafe { &*(offset as *const MadtEntryHeader) };
        let entry_len = entry_header.length as usize;

        if entry_len < 2 || offset + entry_len > entries_end {
            break;
        }

        match entry_header.entry_type {
            MADT_LOCAL_APIC => {
                if entry_len >= core::mem::size_of::<MadtLocalApicEntry>()
                    && info.local_apic_count < MAX_CPUS
                {
                    // SAFETY: Entry type 0 has the MadtLocalApicEntry layout
                    // and we verified the length is sufficient.
                    let entry = unsafe { &*(offset as *const MadtLocalApicEntry) };
                    let lapic = MadtLocalApic {
                        acpi_processor_id: entry.acpi_processor_id,
                        apic_id: entry.apic_id,
                        flags: { entry.flags },
                    };
                    println!(
                        "[ACPI]   CPU: proc_id={}, apic_id={}, flags={:#x}{}",
                        lapic.acpi_processor_id,
                        lapic.apic_id,
                        lapic.flags,
                        if lapic.is_usable() {
                            " [usable]"
                        } else {
                            " [disabled]"
                        }
                    );
                    info.local_apics[info.local_apic_count] = Some(lapic);
                    info.local_apic_count += 1;
                }
            }
            MADT_IO_APIC => {
                if entry_len >= core::mem::size_of::<MadtIoApicEntry>()
                    && info.io_apic_count < MAX_IO_APICS
                {
                    // SAFETY: Entry type 1 has the MadtIoApicEntry layout.
                    let entry = unsafe { &*(offset as *const MadtIoApicEntry) };
                    let ioapic = MadtIoApic {
                        id: entry.id,
                        address: { entry.address },
                        gsi_base: { entry.gsi_base },
                    };
                    println!(
                        "[ACPI]   I/O APIC: id={}, addr={:#x}, gsi_base={}",
                        ioapic.id, ioapic.address, ioapic.gsi_base
                    );
                    info.io_apics[info.io_apic_count] = Some(ioapic);
                    info.io_apic_count += 1;
                }
            }
            MADT_INTERRUPT_SOURCE_OVERRIDE => {
                if entry_len >= core::mem::size_of::<MadtIsoEntry>() && info.iso_count < MAX_ISO {
                    // SAFETY: Entry type 2 has the MadtIsoEntry layout.
                    let entry = unsafe { &*(offset as *const MadtIsoEntry) };
                    let iso = MadtIso {
                        bus: entry.bus,
                        irq_source: entry.source,
                        gsi: { entry.gsi },
                        flags: { entry.flags },
                    };
                    println!(
                        "[ACPI]   ISO: bus={}, irq={} -> gsi={}, flags={:#x}",
                        iso.bus, iso.irq_source, iso.gsi, iso.flags
                    );
                    info.isos[info.iso_count] = Some(iso);
                    info.iso_count += 1;
                }
            }
            MADT_LOCAL_APIC_NMI => {
                if entry_len >= core::mem::size_of::<MadtLocalApicNmiEntry>() {
                    // SAFETY: Entry type 4 has the MadtLocalApicNmiEntry layout.
                    let entry = unsafe { &*(offset as *const MadtLocalApicNmiEntry) };
                    println!(
                        "[ACPI]   LAPIC NMI: proc_id={}, flags={:#x}, lint={}",
                        entry.acpi_processor_id,
                        { entry.flags },
                        entry.lint
                    );
                }
            }
            other => {
                println!(
                    "[ACPI]   Unknown MADT entry type {} (len={})",
                    other, entry_len
                );
            }
        }

        offset += entry_len;
    }

    println!(
        "[ACPI] MADT summary: {} CPUs ({} usable), {} I/O APICs, {} ISOs",
        info.local_apic_count,
        info.cpu_count(),
        info.io_apic_count,
        info.iso_count
    );
}

/// Parse the MCFG (PCI Express Memory Mapped Configuration) table.
fn parse_mcfg(header_vaddr: usize, info: &mut AcpiInfo) {
    // SAFETY: header_vaddr points to a valid MCFG table.
    let sdt = unsafe { &*(header_vaddr as *const AcpiSdtHeader) };
    let table_len = { sdt.length } as usize;
    let header_size = core::mem::size_of::<AcpiSdtHeader>() + 8; // 8 reserved bytes

    if table_len <= header_size {
        println!("[ACPI] MCFG: no allocation entries");
        return;
    }

    info.has_mcfg = true;
    let entries_start = header_vaddr + header_size;
    let entry_size = core::mem::size_of::<McfgAllocation>();
    let num_entries = (table_len - header_size) / entry_size;

    println!("[ACPI] MCFG: {} allocation entries", num_entries);

    for i in 0..num_entries {
        if info.mcfg_count >= MAX_MCFG_ENTRIES {
            break;
        }
        let entry_addr = entries_start + i * entry_size;
        // SAFETY: entry_addr points within the MCFG table bounds.
        let entry = unsafe { &*(entry_addr as *const McfgAllocation) };
        let mcfg = McfgEntry {
            base_address: { entry.base_address },
            segment_group: { entry.segment_group },
            start_bus: entry.start_bus,
            end_bus: entry.end_bus,
        };
        println!(
            "[ACPI]   ECAM: base={:#x}, seg={}, bus={}..{}",
            mcfg.base_address, mcfg.segment_group, mcfg.start_bus, mcfg.end_bus
        );
        info.mcfg_entries[info.mcfg_count] = Some(mcfg);
        info.mcfg_count += 1;
    }
}

/// Parse a single ACPI table identified by its header.
fn parse_table(header_vaddr: usize, info: &mut AcpiInfo) {
    // SAFETY: header_vaddr points to a valid ACPI SDT header.
    let sdt = unsafe { &*(header_vaddr as *const AcpiSdtHeader) };
    let sig = { sdt.signature };
    let len = { sdt.length } as usize;

    // Validate checksum
    if !validate_checksum(header_vaddr, len) {
        println!(
            "[ACPI] WARNING: bad checksum for table {:?}",
            core::str::from_utf8(&sig).unwrap_or("????")
        );
        // Continue anyway -- some BIOS/firmware have incorrect checksums
    }

    if &sig == MADT_SIGNATURE {
        parse_madt(header_vaddr, info);
    } else if &sig == MCFG_SIGNATURE {
        parse_mcfg(header_vaddr, info);
    } else if &sig == DMAR_SIGNATURE {
        // Record DMAR presence and location for the IOMMU driver to parse.
        // The DMAR table has a complex structure (DRHD, RMRR, ATSR entries)
        // that the IOMMU driver handles directly.
        info.has_dmar = true;
        // Store the virtual address for the IOMMU driver to read directly.
        info.dmar_address = header_vaddr as u64;
        info.dmar_length = len as u32;
        println!("[ACPI]   DMAR table found (len={})", len);
    } else if &sig == SRAT_SIGNATURE {
        info.has_srat = true;
        info.srat_address = header_vaddr as u64;
        info.srat_length = len as u32;
        println!("[ACPI]   SRAT table found (len={})", len);
    } else if &sig == SLIT_SIGNATURE {
        info.has_slit = true;
        info.slit_address = header_vaddr as u64;
        info.slit_length = len as u32;
        println!("[ACPI]   SLIT table found (len={})", len);
    } else {
        // Log but skip other tables
        let sig_str = core::str::from_utf8(&sig).unwrap_or("????");
        println!("[ACPI]   Table '{}' (len={}) -- skipped", sig_str, len);
    }
}

/// Parse the RSDT (Root System Description Table, 32-bit pointers).
fn parse_rsdt(rsdt_vaddr: usize, info: &mut AcpiInfo) -> KernelResult<()> {
    // SAFETY: rsdt_vaddr points to the RSDT mapped via phys_to_virt.
    let sdt = unsafe { &*(rsdt_vaddr as *const AcpiSdtHeader) };
    let len = { sdt.length } as usize;

    if &{ sdt.signature } != RSDT_SIGNATURE {
        return Err(KernelError::InvalidArgument {
            name: "RSDT signature",
            value: "not RSDT",
        });
    }

    if !validate_checksum(rsdt_vaddr, len) {
        println!("[ACPI] WARNING: RSDT checksum invalid");
    }

    // Copy OEM ID
    info.oem_id = sdt.oem_id;

    let header_size = core::mem::size_of::<AcpiSdtHeader>();
    let num_entries = (len - header_size) / 4; // 32-bit pointers

    println!(
        "[ACPI] RSDT at {:#x}: {} child tables, OEM='{}'",
        rsdt_vaddr,
        num_entries,
        core::str::from_utf8(&info.oem_id).unwrap_or("??????")
    );

    for i in 0..num_entries {
        let ptr_addr = rsdt_vaddr + header_size + i * 4;
        // SAFETY: ptr_addr is within the RSDT bounds, reading a 32-bit physical
        // pointer.
        let phys_addr = unsafe { *(ptr_addr as *const u32) } as usize;
        if let Some(vaddr) = super::msr::phys_to_virt(phys_addr) {
            parse_table(vaddr, info);
        }
    }

    Ok(())
}

/// Parse the XSDT (Extended System Description Table, 64-bit pointers).
fn parse_xsdt(xsdt_vaddr: usize, info: &mut AcpiInfo) -> KernelResult<()> {
    // SAFETY: xsdt_vaddr points to the XSDT mapped via phys_to_virt.
    let sdt = unsafe { &*(xsdt_vaddr as *const AcpiSdtHeader) };
    let len = { sdt.length } as usize;

    if &{ sdt.signature } != XSDT_SIGNATURE {
        return Err(KernelError::InvalidArgument {
            name: "XSDT signature",
            value: "not XSDT",
        });
    }

    if !validate_checksum(xsdt_vaddr, len) {
        println!("[ACPI] WARNING: XSDT checksum invalid");
    }

    // Copy OEM ID
    info.oem_id = sdt.oem_id;

    let header_size = core::mem::size_of::<AcpiSdtHeader>();
    let num_entries = (len - header_size) / 8; // 64-bit pointers

    println!(
        "[ACPI] XSDT at {:#x}: {} child tables, OEM='{}'",
        xsdt_vaddr,
        num_entries,
        core::str::from_utf8(&info.oem_id).unwrap_or("??????")
    );

    for i in 0..num_entries {
        let ptr_addr = xsdt_vaddr + header_size + i * 8;
        // SAFETY: ptr_addr is within the XSDT bounds, reading a 64-bit physical
        // pointer.
        let phys_addr = unsafe { *(ptr_addr as *const u64) } as usize;
        if let Some(vaddr) = super::msr::phys_to_virt(phys_addr) {
            parse_table(vaddr, info);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the ACPI subsystem by parsing tables from the RSDP address
/// provided by the UEFI bootloader.
///
/// Must be called after memory management is initialized (physical memory
/// mapping available via `phys_to_virt`).
pub fn init() -> KernelResult<()> {
    if ACPI_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "ACPI",
            id: 0,
        });
    }

    println!("[ACPI] Initializing ACPI table parser...");

    // Get RSDP physical address from bootloader BootInfo.
    // SAFETY: BOOT_INFO is a static mut written once during early boot and
    // read-only afterwards. We are in single-threaded kernel init context.
    #[allow(static_mut_refs)]
    let rsdp_phys = unsafe {
        super::boot::BOOT_INFO
            .as_ref()
            .and_then(|bi| bi.rsdp_addr.into_option())
    };

    let rsdp_phys = match rsdp_phys {
        Some(addr) => addr as usize,
        None => {
            println!("[ACPI] No RSDP address from bootloader, ACPI unavailable");
            return Err(KernelError::NotInitialized {
                subsystem: "ACPI (no RSDP)",
            });
        }
    };

    println!("[ACPI] RSDP physical address: {:#x}", rsdp_phys);

    // Map RSDP to virtual address.
    let rsdp_vaddr = super::msr::phys_to_virt(rsdp_phys).ok_or(KernelError::NotInitialized {
        subsystem: "ACPI (phys_to_virt)",
    })?;

    // Validate RSDP signature.
    // SAFETY: rsdp_vaddr points to a valid RSDP structure mapped by the
    // bootloader's physical memory mapping.
    let rsdp = unsafe { &*(rsdp_vaddr as *const Rsdp) };
    if &rsdp.signature != RSDP_SIGNATURE {
        println!("[ACPI] Invalid RSDP signature");
        return Err(KernelError::InvalidArgument {
            name: "RSDP signature",
            value: "not 'RSD PTR '",
        });
    }

    // Validate RSDP checksum (first 20 bytes for ACPI 1.0).
    if !validate_checksum(rsdp_vaddr, 20) {
        println!("[ACPI] WARNING: RSDP checksum invalid");
    }

    let mut info = AcpiInfo::new();
    info.revision = rsdp.revision;

    println!(
        "[ACPI] RSDP: revision={}, OEM='{}'",
        rsdp.revision,
        core::str::from_utf8(&rsdp.oem_id).unwrap_or("??????")
    );

    // ACPI 2.0+ has XSDT (64-bit pointers); fall back to RSDT (32-bit).
    if rsdp.revision >= 2 {
        // SAFETY: ACPI 2.0 RSDP is at least 36 bytes. We read the extended
        // XSDT address field.
        let rsdp2 = unsafe { &*(rsdp_vaddr as *const Rsdp2) };
        let xsdt_phys = { rsdp2.xsdt_address } as usize;

        if xsdt_phys != 0 {
            if let Some(xsdt_vaddr) = super::msr::phys_to_virt(xsdt_phys) {
                println!("[ACPI] Using XSDT at phys={:#x}", xsdt_phys);
                parse_xsdt(xsdt_vaddr, &mut info)?;
            } else {
                println!("[ACPI] Cannot map XSDT, falling back to RSDT");
                let rsdt_phys = { rsdp.rsdt_address } as usize;
                if let Some(rsdt_vaddr) = super::msr::phys_to_virt(rsdt_phys) {
                    parse_rsdt(rsdt_vaddr, &mut info)?;
                }
            }
        } else {
            // XSDT address is zero -- use RSDT
            let rsdt_phys = { rsdp.rsdt_address } as usize;
            if let Some(rsdt_vaddr) = super::msr::phys_to_virt(rsdt_phys) {
                println!(
                    "[ACPI] XSDT addr is zero, using RSDT at phys={:#x}",
                    rsdt_phys
                );
                parse_rsdt(rsdt_vaddr, &mut info)?;
            }
        }
    } else {
        // ACPI 1.0 -- RSDT only
        let rsdt_phys = { rsdp.rsdt_address } as usize;
        if let Some(rsdt_vaddr) = super::msr::phys_to_virt(rsdt_phys) {
            println!("[ACPI] Using RSDT at phys={:#x}", rsdt_phys);
            parse_rsdt(rsdt_vaddr, &mut info)?;
        }
    }

    // Provide defaults if no MADT found (QEMU always provides one, but be safe).
    if !info.has_madt {
        println!("[ACPI] No MADT found, using defaults (1 CPU, LAPIC at 0xFEE00000)");
        info.local_apic_address = 0xFEE0_0000;
        info.local_apics[0] = Some(MadtLocalApic {
            acpi_processor_id: 0,
            apic_id: 0,
            flags: 1,
        });
        info.local_apic_count = 1;
    }

    println!(
        "[ACPI] Initialization complete: {} CPUs, {} I/O APICs, MADT={}, MCFG={}",
        info.cpu_count(),
        info.io_apic_count,
        info.has_madt,
        info.has_mcfg
    );

    *ACPI_INFO.lock() = Some(info);
    ACPI_INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// Dump parsed ACPI information to the console (for the `acpi` shell command).
pub fn dump() {
    let guard = ACPI_INFO.lock();
    let info = match guard.as_ref() {
        Some(info) => info,
        None => {
            println!("ACPI not initialized");
            return;
        }
    };

    println!("=== ACPI Information ===");
    println!(
        "  Revision: {} (ACPI {}.0)",
        info.revision,
        if info.revision >= 2 { "2" } else { "1" }
    );
    println!(
        "  OEM: '{}'",
        core::str::from_utf8(&info.oem_id).unwrap_or("??????")
    );
    println!("  Local APIC Address: {:#x}", info.local_apic_address);
    println!(
        "  MADT Flags: {:#x} (dual 8259: {})",
        info.madt_flags,
        if info.madt_flags & 1 != 0 {
            "yes"
        } else {
            "no"
        }
    );

    println!("\n--- CPUs ({}) ---", info.local_apic_count);
    for i in 0..info.local_apic_count {
        if let Some(ref lapic) = info.local_apics[i] {
            println!(
                "  CPU {}: APIC ID={}, proc_id={}, flags={:#x} {}",
                i,
                lapic.apic_id,
                lapic.acpi_processor_id,
                lapic.flags,
                if lapic.is_usable() {
                    "[usable]"
                } else {
                    "[disabled]"
                }
            );
        }
    }

    println!("\n--- I/O APICs ({}) ---", info.io_apic_count);
    for i in 0..info.io_apic_count {
        if let Some(ref ioapic) = info.io_apics[i] {
            println!(
                "  I/O APIC {}: ID={}, addr={:#x}, GSI base={}",
                i, ioapic.id, ioapic.address, ioapic.gsi_base
            );
        }
    }

    if info.iso_count > 0 {
        println!("\n--- Interrupt Source Overrides ({}) ---", info.iso_count);
        for i in 0..info.iso_count {
            if let Some(ref iso) = info.isos[i] {
                println!(
                    "  IRQ {} -> GSI {}, flags={:#x} (active_low={}, level={})",
                    iso.irq_source,
                    iso.gsi,
                    iso.flags,
                    iso.is_active_low(),
                    iso.is_level_triggered()
                );
            }
        }
    }

    if info.has_mcfg {
        println!("\n--- PCIe ECAM ({}) ---", info.mcfg_count);
        for i in 0..info.mcfg_count {
            if let Some(ref mcfg) = info.mcfg_entries[i] {
                println!(
                    "  Segment {}: base={:#x}, bus={}..{}",
                    mcfg.segment_group, mcfg.base_address, mcfg.start_bus, mcfg.end_bus
                );
            }
        }
    } else {
        println!("\n--- PCIe ECAM: not available ---");
    }

    println!(
        "\nSummary: {} usable CPUs, {} I/O APICs, {} ISOs, {} MCFG entries",
        info.cpu_count(),
        info.io_apic_count,
        info.iso_count,
        info.mcfg_count
    );
}

/// Find SRAT table data. Returns a slice of the raw SRAT table if present.
pub fn find_srat() -> Option<&'static [u8]> {
    with_acpi_info(|info| {
        if !info.has_srat || info.srat_address == 0 {
            return None;
        }
        let addr = info.srat_address as usize;
        let len = info.srat_length as usize;
        // SAFETY: srat_address was captured from a valid ACPI table mapped by
        // the bootloader's physical memory mapping. The table remains in memory
        // for the kernel's lifetime.
        Some(unsafe { core::slice::from_raw_parts(addr as *const u8, len) })
    })
    .flatten()
}

/// Find SLIT table data. Returns a slice of the raw SLIT table if present.
pub fn find_slit() -> Option<&'static [u8]> {
    with_acpi_info(|info| {
        if !info.has_slit || info.slit_address == 0 {
            return None;
        }
        let addr = info.slit_address as usize;
        let len = info.slit_length as usize;
        // SAFETY: slit_address was captured from a valid ACPI table mapped by
        // the bootloader's physical memory mapping. The table remains in memory
        // for the kernel's lifetime.
        Some(unsafe { core::slice::from_raw_parts(addr as *const u8, len) })
    })
    .flatten()
}

/// Find MADT CPU topology data.
///
/// Returns a vector of (apic_id, acpi_processor_id, is_usable) tuples.
pub fn find_madt_cpus() -> Option<alloc::vec::Vec<(u32, u32, bool)>> {
    with_acpi_info(|info| {
        if !info.has_madt {
            return None;
        }
        let mut cpus = alloc::vec::Vec::new();
        for i in 0..info.local_apic_count {
            if let Some(ref lapic) = info.local_apics[i] {
                cpus.push((
                    lapic.apic_id as u32,
                    lapic.acpi_processor_id as u32,
                    lapic.is_usable(),
                ));
            }
        }
        Some(cpus)
    })
    .flatten()
}

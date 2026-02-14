//! Bootloader memory map integration
//!
//! Handles parsing and processing of memory maps from various bootloaders
//! (GRUB, UEFI, custom bootloader, etc.)

#![allow(dead_code)]

use super::{
    frame_allocator::ReservedRegion, FrameNumber, MemoryRegion, FRAME_ALLOCATOR, FRAME_SIZE,
};
use crate::error::KernelError;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Memory region type from bootloader
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionType {
    /// Usable RAM
    Usable,
    /// Reserved by firmware/BIOS
    Reserved,
    /// ACPI data that can be reclaimed
    AcpiReclaimable,
    /// ACPI NVS memory
    AcpiNvs,
    /// Bad memory
    BadMemory,
    /// Kernel and modules
    KernelAndModules,
    /// Bootloader reclaimable
    BootloaderReclaimable,
    /// Framebuffer
    Framebuffer,
    /// Unknown type
    Unknown(u32),
}

/// Extended memory region with type information
#[derive(Debug, Clone, Copy)]
pub struct BootloaderMemoryRegion {
    pub start: u64,
    pub size: u64,
    pub region_type: MemoryRegionType,
}

impl BootloaderMemoryRegion {
    /// Create a new memory region
    pub const fn new(start: u64, size: u64, region_type: MemoryRegionType) -> Self {
        Self {
            start,
            size,
            region_type,
        }
    }

    /// Check if this region is usable memory
    pub const fn is_usable(&self) -> bool {
        matches!(self.region_type, MemoryRegionType::Usable)
    }

    /// Convert to simple memory region
    pub const fn to_memory_region(self) -> MemoryRegion {
        MemoryRegion {
            start: self.start,
            size: self.size,
            usable: self.is_usable(),
        }
    }
}

/// Parse E820 memory map (x86_64 BIOS)
#[cfg(target_arch = "x86_64")]
pub fn parse_e820_map(entries: &[(u64, u64, u32)]) -> Vec<BootloaderMemoryRegion> {
    let mut regions = Vec::with_capacity(entries.len());

    for &(base, length, typ) in entries {
        let region_type = match typ {
            1 => MemoryRegionType::Usable,
            2 => MemoryRegionType::Reserved,
            3 => MemoryRegionType::AcpiReclaimable,
            4 => MemoryRegionType::AcpiNvs,
            5 => MemoryRegionType::BadMemory,
            _ => MemoryRegionType::Unknown(typ),
        };

        regions.push(BootloaderMemoryRegion::new(base, length, region_type));
    }

    regions
}

/// Parse UEFI memory map
pub fn parse_uefi_map(descriptors: &[UefiMemoryDescriptor]) -> Vec<BootloaderMemoryRegion> {
    let mut regions = Vec::with_capacity(descriptors.len());

    for desc in descriptors {
        let region_type = match desc.typ {
            0..=2 => MemoryRegionType::Reserved,
            3 => MemoryRegionType::BootloaderReclaimable,
            4 => MemoryRegionType::BootloaderReclaimable,
            7 => MemoryRegionType::Usable,
            9 => MemoryRegionType::AcpiReclaimable,
            10 => MemoryRegionType::AcpiNvs,
            11 => MemoryRegionType::Reserved,
            _ => MemoryRegionType::Unknown(desc.typ),
        };

        regions.push(BootloaderMemoryRegion::new(
            desc.phys_start,
            desc.num_pages * 4096, // EFI page size
            region_type,
        ));
    }

    regions
}

/// UEFI memory descriptor
#[repr(C)]
pub struct UefiMemoryDescriptor {
    pub typ: u32,
    pub phys_start: u64,
    pub virt_start: u64,
    pub num_pages: u64,
    pub attr: u64,
}

/// Process bootloader memory map and initialize frame allocator
pub fn process_memory_map(regions: &[BootloaderMemoryRegion]) -> Result<(), KernelError> {
    println!("[BOOT] Processing bootloader memory map...");

    let mut total_memory = 0u64;
    let mut usable_memory = 0u64;
    let mut reserved_count = 0;

    // First pass: mark reserved regions
    for region in regions {
        total_memory += region.size;

        match region.region_type {
            MemoryRegionType::Reserved
            | MemoryRegionType::AcpiNvs
            | MemoryRegionType::BadMemory
            | MemoryRegionType::KernelAndModules
            | MemoryRegionType::Framebuffer => {
                // Mark as reserved in frame allocator
                let start_frame = region.start / FRAME_SIZE as u64;
                let end_frame = (region.start + region.size).div_ceil(FRAME_SIZE as u64);

                let description = match region.region_type {
                    MemoryRegionType::Reserved => "Reserved",
                    MemoryRegionType::AcpiNvs => "ACPI NVS",
                    MemoryRegionType::BadMemory => "Bad Memory",
                    MemoryRegionType::KernelAndModules => "Kernel/Modules",
                    MemoryRegionType::Framebuffer => "Framebuffer",
                    _ => "Reserved",
                };

                let reserved = ReservedRegion {
                    start: FrameNumber::new(start_frame),
                    end: FrameNumber::new(end_frame),
                    description,
                };

                FRAME_ALLOCATOR
                    .lock()
                    .add_reserved_region(reserved)
                    .map_err(|_| KernelError::ResourceExhausted {
                        resource: "reserved memory regions",
                    })?;

                reserved_count += 1;
            }
            _ => {}
        }
    }

    // Second pass: add usable memory
    let mut allocator = FRAME_ALLOCATOR.lock();
    let mut region_count = 0;

    for region in regions {
        if region.is_usable() {
            usable_memory += region.size;

            let start_frame = region.start / FRAME_SIZE as u64;
            let frame_count = region.size as usize / FRAME_SIZE;

            // Skip very small regions
            if frame_count < 16 {
                continue;
            }

            // Use region index as NUMA node (simplified)
            let numa_node = region_count % 8;

            if let Err(_e) =
                allocator.init_numa_node(numa_node, FrameNumber::new(start_frame), frame_count)
            {
                println!(
                    "[BOOT] Warning: Failed to add memory region at 0x{:x}",
                    region.start
                );
            } else {
                println!(
                    "[BOOT] Added {} MB at 0x{:x} (NUMA node {})",
                    region.size / (1024 * 1024),
                    region.start,
                    numa_node
                );
                region_count += 1;
            }
        }
    }

    drop(allocator);

    #[cfg(target_arch = "x86_64")]
    println!(
        "[BOOT] Memory map processed: {} MB total, {} MB usable, {} reserved regions",
        total_memory / (1024 * 1024),
        usable_memory / (1024 * 1024),
        reserved_count
    );

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = total_memory;
        let _ = usable_memory;
        let _ = reserved_count;
        println!("[BOOT] Memory map processed");
    }

    Ok(())
}

/// Standard x86 memory regions to reserve
pub fn reserve_standard_regions() -> Result<(), KernelError> {
    let allocator = FRAME_ALLOCATOR.lock();

    // Reserve first megabyte (BIOS, IVT, BDA, etc.)
    let bios_region = ReservedRegion {
        start: FrameNumber::new(0),
        end: FrameNumber::new(256),
        description: "BIOS/Real mode",
    };
    allocator
        .add_reserved_region(bios_region)
        .map_err(|_| KernelError::ResourceExhausted {
            resource: "reserved memory regions",
        })?;

    // Reserve common BIOS areas
    let video_region = ReservedRegion {
        start: FrameNumber::new(0xA0),
        end: FrameNumber::new(0x100),
        description: "Video memory",
    };
    allocator
        .add_reserved_region(video_region)
        .map_err(|_| KernelError::ResourceExhausted {
            resource: "reserved memory regions",
        })?;

    // Reserve local APIC region (typically at 0xFEE00000)
    #[cfg(target_arch = "x86_64")]
    {
        let apic_frame = 0xFEE00000 / FRAME_SIZE as u64;
        let apic_region = ReservedRegion {
            start: FrameNumber::new(apic_frame),
            end: FrameNumber::new(apic_frame + 1),
            description: "Local APIC",
        };
        allocator
            .add_reserved_region(apic_region)
            .map_err(|_| KernelError::ResourceExhausted {
                resource: "reserved memory regions",
            })?;
    }

    Ok(())
}

//! Intel i915/Xe GPU Driver Framework
//!
//! Provides PCI device detection and structured types for Intel integrated
//! and discrete GPUs. Actual register programming requires hardware access
//! not available in QEMU's virtio-gpu environment.

#![allow(dead_code)]

#[cfg(target_arch = "x86_64")]
use alloc::format;
use alloc::{string::String, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// PCI Device IDs
// ---------------------------------------------------------------------------

/// Intel GPU PCI device IDs (subset of common devices)
pub mod device_ids {
    // Intel HD Graphics / UHD Graphics / Iris
    /// Tiger Lake LP GT2 (11th gen mobile)
    pub const TIGERLAKE_LP_GT2: u16 = 0x9A49;
    /// Alder Lake S GT1 (12th gen desktop)
    pub const ALDERLAKE_S_GT1: u16 = 0x4680;
    /// Raptor Lake S GT1 (13th gen desktop)
    pub const RAPTORLAKE_S_GT1: u16 = 0xA780;
    /// Meteor Lake GT2 (14th gen mobile)
    pub const METEORLAKE_GT2: u16 = 0x7D55;

    // Intel Arc (Xe/DG2)
    /// Arc A770 (DG2-512EU)
    pub const ARC_A770: u16 = 0x56A0;
    /// Arc A750 (DG2-448EU)
    pub const ARC_A750: u16 = 0x56A1;
    /// Arc A380 (DG2-128EU)
    pub const ARC_A380: u16 = 0x56A5;
}

// ---------------------------------------------------------------------------
// GPU Generation Classification
// ---------------------------------------------------------------------------

/// Intel GPU generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntelGeneration {
    /// Skylake / Kaby Lake / Coffee Lake
    Gen9,
    /// Ice Lake
    Gen11,
    /// Tiger Lake
    Gen12,
    /// Alder Lake / Raptor Lake
    Gen12p5,
    /// Meteor Lake
    Gen12p7,
    /// Arc discrete GPUs (DG2)
    XeHpg,
    /// Unrecognised device ID
    Unknown,
}

impl core::fmt::Display for IntelGeneration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Gen9 => write!(f, "Gen9"),
            Self::Gen11 => write!(f, "Gen11"),
            Self::Gen12 => write!(f, "Gen12"),
            Self::Gen12p5 => write!(f, "Gen12.5"),
            Self::Gen12p7 => write!(f, "Gen12.7"),
            Self::XeHpg => write!(f, "Xe-HPG"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// MMIO Register Offsets
// ---------------------------------------------------------------------------

/// MMIO register block offsets (per i915 driver documentation)
pub mod regs {
    /// Render ring base
    pub const RENDER_RING_BASE: u64 = 0x02000;
    /// Blitter ring base
    pub const BLITTER_RING_BASE: u64 = 0x22000;
    /// Video decode ring base
    pub const VIDEO_RING_BASE: u64 = 0x12000;
    /// Display pipe A registers
    pub const DISPLAY_PIPE_A: u64 = 0x60000;
    /// Display pipe B registers
    pub const DISPLAY_PIPE_B: u64 = 0x61000;
    /// Graphics Translation Table base
    pub const GTT_BASE: u64 = 0x100000;
    /// Fence register base (tiling)
    pub const FENCE_REG_BASE: u64 = 0x02000;
    /// Instruction parser mode
    pub const INSTPM: u64 = 0x020C0;
    /// Forcewake register
    pub const FORCEWAKE: u64 = 0x0A188;
    /// Hardware status page address
    pub const HWS_PGA: u64 = 0x04080;
    /// Ring buffer control
    pub const RING_CTL: u64 = 0x0203C;
}

// ---------------------------------------------------------------------------
// Display Types
// ---------------------------------------------------------------------------

/// Display pipe configuration
#[derive(Debug, Clone)]
pub struct DisplayPipe {
    /// Pipe index (0 = A, 1 = B, etc.)
    pub pipe_id: u8,
    /// Horizontal resolution
    pub width: u32,
    /// Vertical resolution
    pub height: u32,
    /// Refresh rate in Hz
    pub refresh_rate: u32,
    /// Whether the pipe is currently active
    pub enabled: bool,
    /// Framebuffer pixel format
    pub pixel_format: DisplayPixelFormat,
}

/// Display framebuffer pixel format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayPixelFormat {
    /// 32-bit XRGB (8:8:8:8, X ignored)
    Xrgb8888,
    /// 32-bit ARGB (8:8:8:8, alpha blended)
    Argb8888,
    /// 30-bit deep colour (10:10:10:2)
    Xrgb2101010,
    /// 32-bit XBGR (reversed channel order)
    Xbgr8888,
}

// ---------------------------------------------------------------------------
// Memory Types
// ---------------------------------------------------------------------------

/// Graphics Translation Table (GTT) entry
#[derive(Debug, Clone, Copy)]
pub struct GttEntry {
    /// Physical address of the page
    pub physical_addr: u64,
    /// Entry is valid / present
    pub valid: bool,
    /// Page is writeable
    pub writeable: bool,
    /// Cache level for the mapping
    pub cache_level: CacheLevel,
}

/// Cache level for GTT mappings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLevel {
    /// Uncached (UC) -- MMIO regions
    Uncached,
    /// Write-combining (WC) -- framebuffer
    WriteCombining,
    /// Cached (LLC) -- general GPU memory
    Cached,
}

// ---------------------------------------------------------------------------
// Device Representation
// ---------------------------------------------------------------------------

/// Intel GPU device instance
pub struct I915Device {
    /// PCI vendor ID (always 0x8086 for Intel)
    pub vendor_id: u16,
    /// PCI device ID
    pub device_id: u16,
    /// Classified GPU generation
    pub generation: IntelGeneration,
    /// BAR0 MMIO base address
    pub mmio_base: u64,
    /// BAR0 MMIO region size
    pub mmio_size: u64,
    /// GTT aperture size (detected during init)
    pub gtt_size: u64,
    /// Dedicated VRAM size (0 for integrated)
    pub vram_size: u64,
    /// Configured display pipes
    pub display_pipes: Vec<DisplayPipe>,
    /// Human-readable device name
    pub name: String,
}

impl I915Device {
    /// Classify GPU generation from PCI device ID.
    ///
    /// Device ID ranges are derived from the upstream i915 and xe driver
    /// source tables.
    pub fn classify_generation(device_id: u16) -> IntelGeneration {
        match device_id {
            // Skylake / Kaby Lake / Coffee Lake
            0x1900..=0x19FF | 0x5900..=0x59FF | 0x3E90..=0x3EFF => IntelGeneration::Gen9,
            // Ice Lake
            0x8A50..=0x8AFF => IntelGeneration::Gen11,
            // Tiger Lake
            0x9A40..=0x9A7F => IntelGeneration::Gen12,
            // Alder Lake / Raptor Lake
            0x4680..=0x46FF | 0xA780..=0xA7FF => IntelGeneration::Gen12p5,
            // Meteor Lake
            0x7D40..=0x7DFF => IntelGeneration::Gen12p7,
            // Arc DG2 discrete
            0x56A0..=0x56BF => IntelGeneration::XeHpg,
            _ => IntelGeneration::Unknown,
        }
    }

    /// Return `true` if this is a discrete GPU (Arc).
    pub fn is_discrete(&self) -> bool {
        self.generation == IntelGeneration::XeHpg
    }
}

// ---------------------------------------------------------------------------
// PCI Probing
// ---------------------------------------------------------------------------

/// Probe the PCI bus for Intel GPU devices (x86_64 only).
///
/// Returns a list of detected Intel display-class devices with generation
/// classification and BAR0 information. No register programming is performed.
#[cfg(target_arch = "x86_64")]
pub fn probe() -> Vec<I915Device> {
    let mut devices = Vec::new();

    if !crate::drivers::pci::is_pci_initialized() {
        return devices;
    }

    let bus = crate::drivers::pci::get_pci_bus().lock();
    let pci_devices = bus.find_devices_by_class(crate::drivers::pci::class_codes::DISPLAY);

    for pci_dev in &pci_devices {
        if pci_dev.vendor_id != 0x8086 {
            continue;
        }

        let gen = I915Device::classify_generation(pci_dev.device_id);

        let (mmio_base, mmio_size) = pci_dev
            .bars
            .first()
            .map(|bar| match bar {
                crate::drivers::pci::PciBar::Memory { address, size, .. } => (*address, *size),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));

        devices.push(I915Device {
            vendor_id: pci_dev.vendor_id,
            device_id: pci_dev.device_id,
            generation: gen,
            mmio_base,
            mmio_size,
            gtt_size: 0,
            vram_size: 0,
            display_pipes: Vec::new(),
            name: format!("Intel GPU {:04x} ({})", pci_dev.device_id, gen),
        });
    }

    devices
}

/// Stub probe for non-x86_64 architectures (Intel GPUs are x86-only).
#[cfg(not(target_arch = "x86_64"))]
pub fn probe() -> Vec<I915Device> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the Intel i915 GPU driver (detection only).
///
/// Scans the PCI bus for Intel display-class devices and logs any found.
/// No register programming or mode setting is performed.
pub fn init() -> Result<(), KernelError> {
    let devices = probe();

    if devices.is_empty() {
        crate::println!("[i915] No Intel GPU devices found");
    } else {
        for dev in &devices {
            crate::println!(
                "[i915] Found: {} (BAR0: {:#x}, size: {:#x})",
                dev.name,
                dev.mmio_base,
                dev.mmio_size
            );
        }
    }

    Ok(())
}

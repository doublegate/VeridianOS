//! NVIDIA Nouveau GPU Driver Framework
//!
//! Provides PCI device detection and structured types for NVIDIA GPUs using
//! the open-source Nouveau driver model. Actual register programming requires
//! hardware access not available in QEMU's virtio-gpu environment.

#![allow(dead_code)]

#[cfg(target_arch = "x86_64")]
use alloc::format;
use alloc::{string::String, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// PCI Device IDs
// ---------------------------------------------------------------------------

/// NVIDIA GPU PCI device IDs (subset of common devices)
pub mod device_ids {
    // Pascal (GP10x)
    /// GeForce GTX 1080 (GP104)
    pub const GTX_1080: u16 = 0x1B80;
    /// GeForce GTX 1070 (GP104)
    pub const GTX_1070: u16 = 0x1B81;
    /// GeForce GTX 1060 (GP106)
    pub const GTX_1060: u16 = 0x1C03;

    // Turing (TU10x)
    /// GeForce RTX 2080 Ti (TU102)
    pub const RTX_2080TI: u16 = 0x1E04;
    /// GeForce RTX 2070 (TU106)
    pub const RTX_2070: u16 = 0x1F02;

    // Ampere (GA10x)
    /// GeForce RTX 3090 (GA102)
    pub const RTX_3090: u16 = 0x2204;
    /// GeForce RTX 3080 (GA102)
    pub const RTX_3080: u16 = 0x2206;
    /// GeForce RTX 3070 (GA104)
    pub const RTX_3070: u16 = 0x2484;

    // Ada Lovelace (AD10x)
    /// GeForce RTX 4090 (AD102)
    pub const RTX_4090: u16 = 0x2684;
    /// GeForce RTX 4080 (AD103)
    pub const RTX_4080: u16 = 0x2704;
}

// ---------------------------------------------------------------------------
// GPU Architecture Classification
// ---------------------------------------------------------------------------

/// NVIDIA GPU architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaArchitecture {
    /// Pascal (GTX 10-series, GP10x)
    Pascal,
    /// Turing (RTX 20-series, TU10x)
    Turing,
    /// Ampere (RTX 30-series, GA10x)
    Ampere,
    /// Ada Lovelace (RTX 40-series, AD10x)
    Ada,
    /// Unrecognised device ID
    Unknown,
}

impl core::fmt::Display for NvidiaArchitecture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Pascal => write!(f, "Pascal"),
            Self::Turing => write!(f, "Turing"),
            Self::Ampere => write!(f, "Ampere"),
            Self::Ada => write!(f, "Ada"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// MMIO Register Offsets
// ---------------------------------------------------------------------------

/// MMIO register block offsets (per Nouveau/envytools documentation)
pub mod regs {
    /// Master control registers (PMC)
    pub const NV_PMC: u64 = 0x000000;
    /// Bus interface control (PBUS)
    pub const NV_PBUS: u64 = 0x001000;
    /// Display engine (PDISP)
    pub const NV_PDISP: u64 = 0x610000;
    /// Graphics engine (PGRAPH)
    pub const NV_PGRAPH: u64 = 0x400000;
    /// FIFO engine (command submission)
    pub const NV_PFIFO: u64 = 0x002000;
    /// Frame buffer interface (PFB)
    pub const NV_PFB: u64 = 0x100000;
    /// Timer / clock control (PTIMER)
    pub const NV_PTIMER: u64 = 0x009000;
    /// GPIO / fan control (PGPIO)
    pub const NV_PGPIO: u64 = 0x00D000;
    /// Power management (PPWR / PMU Falcon)
    pub const NV_PPWR: u64 = 0x10A000;
    /// Boot / strap registers
    pub const NV_PBOOT: u64 = 0x000000;
}

// ---------------------------------------------------------------------------
// Falcon Microcontroller Types
// ---------------------------------------------------------------------------

/// NVIDIA Falcon security co-processor engine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalconEngine {
    /// Power Management Unit
    Pmu,
    /// Display engine
    Display,
    /// Graphics context switch
    GrCtxSw,
    /// Video decode (NVDEC)
    Nvdec,
    /// Video encode (NVENC)
    Nvenc,
    /// Security (SEC2)
    Sec2,
    /// GSP (GPU System Processor, Turing+)
    Gsp,
}

/// Falcon engine state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalconState {
    /// Engine is not initialised
    Reset,
    /// Firmware is loaded
    Loaded,
    /// Engine is running
    Running,
    /// Engine encountered an error
    Error,
}

// ---------------------------------------------------------------------------
// Device Representation
// ---------------------------------------------------------------------------

/// NVIDIA GPU device instance
pub struct NouveauDevice {
    /// PCI vendor ID (always 0x10DE for NVIDIA)
    pub vendor_id: u16,
    /// PCI device ID
    pub device_id: u16,
    /// Classified GPU architecture
    pub architecture: NvidiaArchitecture,
    /// BAR0 MMIO base address (registers)
    pub mmio_base: u64,
    /// BAR0 MMIO region size
    pub mmio_size: u64,
    /// VRAM aperture base (BAR1)
    pub vram_base: u64,
    /// VRAM aperture size
    pub vram_size: u64,
    /// Human-readable device name
    pub name: String,
}

impl NouveauDevice {
    /// Classify GPU architecture from PCI device ID.
    ///
    /// Device ID ranges are derived from the upstream Nouveau driver
    /// chipset identification and envytools database.
    pub fn classify_architecture(device_id: u16) -> NvidiaArchitecture {
        match device_id {
            // Pascal (GP10x): GTX 10-series
            0x1B00..=0x1D8F => NvidiaArchitecture::Pascal,
            // Turing (TU10x): RTX 20-series, GTX 16-series
            0x1E00..=0x1EFF | 0x1F00..=0x1FFF | 0x2180..=0x21FF => NvidiaArchitecture::Turing,
            // Ampere (GA10x): RTX 30-series
            0x2200..=0x22FF | 0x2480..=0x24FF | 0x2500..=0x257F => NvidiaArchitecture::Ampere,
            // Ada Lovelace (AD10x): RTX 40-series
            0x2680..=0x27FF => NvidiaArchitecture::Ada,
            _ => NvidiaArchitecture::Unknown,
        }
    }

    /// Return `true` if this GPU requires GSP firmware (Turing+).
    pub fn requires_gsp(&self) -> bool {
        matches!(
            self.architecture,
            NvidiaArchitecture::Turing | NvidiaArchitecture::Ampere | NvidiaArchitecture::Ada
        )
    }

    /// Return `true` if this GPU supports hardware raytracing (Turing+).
    pub fn has_raytracing(&self) -> bool {
        matches!(
            self.architecture,
            NvidiaArchitecture::Turing | NvidiaArchitecture::Ampere | NvidiaArchitecture::Ada
        )
    }
}

// ---------------------------------------------------------------------------
// PCI Probing
// ---------------------------------------------------------------------------

/// Probe the PCI bus for NVIDIA GPU devices (x86_64 only).
///
/// Returns a list of detected NVIDIA display-class devices with architecture
/// classification and BAR information. No register programming is performed.
#[cfg(target_arch = "x86_64")]
pub fn probe() -> Vec<NouveauDevice> {
    let mut devices = Vec::new();

    if !crate::drivers::pci::is_pci_initialized() {
        return devices;
    }

    let bus = crate::drivers::pci::get_pci_bus().lock();
    let pci_devices = bus.find_devices_by_class(crate::drivers::pci::class_codes::DISPLAY);

    for pci_dev in &pci_devices {
        if pci_dev.vendor_id != 0x10DE {
            continue;
        }

        let arch = NouveauDevice::classify_architecture(pci_dev.device_id);

        // BAR0 = MMIO registers
        let (mmio_base, mmio_size) = pci_dev
            .bars
            .first()
            .map(|bar| match bar {
                crate::drivers::pci::PciBar::Memory { address, size, .. } => (*address, *size),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));

        // BAR1 = VRAM aperture
        let (vram_base, vram_size) = pci_dev
            .bars
            .get(1)
            .map(|bar| match bar {
                crate::drivers::pci::PciBar::Memory { address, size, .. } => (*address, *size),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));

        devices.push(NouveauDevice {
            vendor_id: pci_dev.vendor_id,
            device_id: pci_dev.device_id,
            architecture: arch,
            mmio_base,
            mmio_size,
            vram_base,
            vram_size,
            name: format!("NVIDIA GPU {:04x} ({})", pci_dev.device_id, arch),
        });
    }

    devices
}

/// Stub probe for non-x86_64 architectures.
#[cfg(not(target_arch = "x86_64"))]
pub fn probe() -> Vec<NouveauDevice> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the Nouveau GPU driver (detection only).
///
/// Scans the PCI bus for NVIDIA display-class devices and logs any found.
/// No register programming, firmware loading, or mode setting is performed.
pub fn init() -> Result<(), KernelError> {
    let devices = probe();

    if devices.is_empty() {
        crate::println!("[nouveau] No NVIDIA GPU devices found");
    } else {
        for dev in &devices {
            crate::println!(
                "[nouveau] Found: {} (BAR0: {:#x}, VRAM: {:#x}{})",
                dev.name,
                dev.mmio_base,
                dev.vram_size,
                if dev.requires_gsp() {
                    ", GSP required"
                } else {
                    ""
                }
            );
        }
    }

    Ok(())
}

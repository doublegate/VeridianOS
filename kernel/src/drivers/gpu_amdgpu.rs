//! AMD GPU (amdgpu) Driver Framework
//!
//! Provides PCI device detection and structured types for AMD Radeon GPUs.
//! Actual register programming requires hardware access not available in
//! QEMU's virtio-gpu environment.

#![allow(dead_code)]

#[cfg(target_arch = "x86_64")]
use alloc::format;
use alloc::{string::String, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// PCI Device IDs
// ---------------------------------------------------------------------------

/// AMD GPU PCI device IDs (subset of common devices)
pub mod device_ids {
    // RDNA 1 (Navi 10)
    /// Radeon RX 5700 XT
    pub const NAVI10_RX5700XT: u16 = 0x731F;
    /// Radeon RX 5600 XT
    pub const NAVI10_RX5600XT: u16 = 0x7310;

    // RDNA 2 (Navi 21 / Navi 22 / Navi 23)
    /// Radeon RX 6900 XT (Navi 21)
    pub const NAVI21_RX6900XT: u16 = 0x73BF;
    /// Radeon RX 6700 XT (Navi 22)
    pub const NAVI22_RX6700XT: u16 = 0x73DF;
    /// Radeon RX 6600 XT (Navi 23)
    pub const NAVI23_RX6600XT: u16 = 0x73FF;

    // RDNA 3 (Navi 31 / Navi 33)
    /// Radeon RX 7900 XTX (Navi 31)
    pub const NAVI31_RX7900XTX: u16 = 0x744C;
    /// Radeon RX 7600 (Navi 33)
    pub const NAVI33_RX7600: u16 = 0x7480;
}

// ---------------------------------------------------------------------------
// GPU Generation Classification
// ---------------------------------------------------------------------------

/// AMD GPU architecture generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmdGeneration {
    /// Graphics Core Next 5th generation (Vega)
    Gcn5,
    /// RDNA 1st generation (Navi 10/12/14)
    Rdna1,
    /// RDNA 2nd generation (Navi 21/22/23/24)
    Rdna2,
    /// RDNA 3rd generation (Navi 31/32/33)
    Rdna3,
    /// Unrecognised device ID
    Unknown,
}

impl core::fmt::Display for AmdGeneration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Gcn5 => write!(f, "GCN5"),
            Self::Rdna1 => write!(f, "RDNA1"),
            Self::Rdna2 => write!(f, "RDNA2"),
            Self::Rdna3 => write!(f, "RDNA3"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// MMIO Register Offsets
// ---------------------------------------------------------------------------

/// MMIO register block offsets (per amdgpu driver documentation)
pub mod regs {
    /// Graphics Register Bus Manager status
    pub const GRBM_STATUS: u64 = 0x8010;
    /// System Register Bus Manager status
    pub const SRBM_STATUS: u64 = 0x0E50;
    /// Interrupt handler ring buffer base
    pub const IH_RB_BASE: u64 = 0x00040;
    /// Display Core Next engine base
    pub const DCN_BASE: u64 = 0x12000;
    /// Memory controller hub base
    pub const MC_HUB_BASE: u64 = 0x0B00;
    /// Power management base
    pub const SMU_BASE: u64 = 0x03B00000;
    /// GFX ring buffer base
    pub const GFX_RING_BASE: u64 = 0x08000;
    /// SDMA (System DMA) engine 0 base
    pub const SDMA0_BASE: u64 = 0x04E00;
    /// Video Core Next decode engine base
    pub const VCN_BASE: u64 = 0x01F400;
    /// Memory-mapped register index/data pair
    pub const MMIO_INDEX: u64 = 0x0000;
    /// Memory-mapped register data
    pub const MMIO_DATA: u64 = 0x0004;
}

// ---------------------------------------------------------------------------
// Display Types
// ---------------------------------------------------------------------------

/// Display Core Next (DCN) display output configuration
#[derive(Debug, Clone)]
pub struct DcnDisplay {
    /// Display engine index
    pub engine_id: u8,
    /// Connector type
    pub connector: ConnectorType,
    /// Horizontal resolution
    pub width: u32,
    /// Vertical resolution
    pub height: u32,
    /// Refresh rate in Hz
    pub refresh_rate: u32,
    /// Whether the display is currently active
    pub enabled: bool,
}

/// Display connector type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorType {
    /// DisplayPort
    DisplayPort,
    /// HDMI
    Hdmi,
    /// DVI-D
    DviD,
    /// USB-C with DisplayPort alt mode
    UsbC,
    /// eDP (embedded DisplayPort, laptop panels)
    Edp,
    /// VGA (legacy, via DAC)
    Vga,
}

// ---------------------------------------------------------------------------
// Power Management Types
// ---------------------------------------------------------------------------

/// GPU power state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// Full performance
    D0,
    /// Low power, display on
    D1,
    /// Lower power, display off
    D2,
    /// Suspended
    D3Hot,
    /// Powered off
    D3Cold,
}

/// Clock domain frequencies (MHz)
#[derive(Debug, Clone, Copy)]
pub struct ClockInfo {
    /// GPU core (shader) clock in MHz
    pub sclk_mhz: u32,
    /// Memory clock in MHz
    pub mclk_mhz: u32,
    /// Voltage in mV (0 if unknown)
    pub voltage_mv: u32,
}

// ---------------------------------------------------------------------------
// Device Representation
// ---------------------------------------------------------------------------

/// AMD GPU device instance
pub struct AmdGpuDevice {
    /// PCI vendor ID (always 0x1002 for AMD)
    pub vendor_id: u16,
    /// PCI device ID
    pub device_id: u16,
    /// Classified GPU generation
    pub generation: AmdGeneration,
    /// BAR0 MMIO base address
    pub mmio_base: u64,
    /// BAR0 MMIO region size
    pub mmio_size: u64,
    /// VRAM size in bytes (from BAR or discovery)
    pub vram_size: u64,
    /// VRAM BAR base address (BAR2 for large-BAR / resize-BAR)
    pub vram_base: u64,
    /// Configured display outputs
    pub displays: Vec<DcnDisplay>,
    /// Current power state
    pub power_state: PowerState,
    /// Human-readable device name
    pub name: String,
}

impl AmdGpuDevice {
    /// Classify GPU generation from PCI device ID.
    ///
    /// Device ID ranges are derived from the upstream amdgpu driver
    /// chip identification tables.
    pub fn classify_generation(device_id: u16) -> AmdGeneration {
        match device_id {
            // Vega (GCN5)
            0x6860..=0x687F | 0x69A0..=0x69AF => AmdGeneration::Gcn5,
            // Navi 10/12/14 (RDNA1)
            0x7310..=0x731F | 0x7340..=0x734F | 0x7360..=0x736F => AmdGeneration::Rdna1,
            // Navi 21/22/23/24 (RDNA2)
            0x73A0..=0x73FF | 0x7420..=0x743F => AmdGeneration::Rdna2,
            // Navi 31/32/33 (RDNA3)
            0x7440..=0x749F => AmdGeneration::Rdna3,
            _ => AmdGeneration::Unknown,
        }
    }

    /// Return `true` if this GPU supports hardware raytracing (RDNA2+).
    pub fn has_raytracing(&self) -> bool {
        matches!(self.generation, AmdGeneration::Rdna2 | AmdGeneration::Rdna3)
    }
}

// ---------------------------------------------------------------------------
// PCI Probing
// ---------------------------------------------------------------------------

/// Probe the PCI bus for AMD GPU devices (x86_64 only).
///
/// Returns a list of detected AMD display-class devices with generation
/// classification and BAR information. No register programming is performed.
#[cfg(target_arch = "x86_64")]
pub fn probe() -> Vec<AmdGpuDevice> {
    let mut devices = Vec::new();

    if !crate::drivers::pci::is_pci_initialized() {
        return devices;
    }

    let bus = crate::drivers::pci::get_pci_bus().lock();
    let pci_devices = bus.find_devices_by_class(crate::drivers::pci::class_codes::DISPLAY);

    for pci_dev in &pci_devices {
        if pci_dev.vendor_id != 0x1002 {
            continue;
        }

        let gen = AmdGpuDevice::classify_generation(pci_dev.device_id);

        let (mmio_base, mmio_size) = pci_dev
            .bars
            .first()
            .map(|bar| match bar {
                crate::drivers::pci::PciBar::Memory { address, size, .. } => (*address, *size),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));

        // BAR2 is typically the VRAM aperture on AMD GPUs
        let (vram_base, vram_size) = pci_dev
            .bars
            .get(2)
            .map(|bar| match bar {
                crate::drivers::pci::PciBar::Memory { address, size, .. } => (*address, *size),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));

        devices.push(AmdGpuDevice {
            vendor_id: pci_dev.vendor_id,
            device_id: pci_dev.device_id,
            generation: gen,
            mmio_base,
            mmio_size,
            vram_size,
            vram_base,
            displays: Vec::new(),
            power_state: PowerState::D0,
            name: format!("AMD GPU {:04x} ({})", pci_dev.device_id, gen),
        });
    }

    devices
}

/// Stub probe for non-x86_64 architectures.
#[cfg(not(target_arch = "x86_64"))]
pub fn probe() -> Vec<AmdGpuDevice> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the AMD GPU driver (detection only).
///
/// Scans the PCI bus for AMD display-class devices and logs any found.
/// No register programming or mode setting is performed.
pub fn init() -> Result<(), KernelError> {
    let devices = probe();

    if devices.is_empty() {
        crate::println!("[amdgpu] No AMD GPU devices found");
    } else {
        for dev in &devices {
            crate::println!(
                "[amdgpu] Found: {} (BAR0: {:#x}, VRAM: {:#x})",
                dev.name,
                dev.mmio_base,
                dev.vram_size
            );
        }
    }

    Ok(())
}

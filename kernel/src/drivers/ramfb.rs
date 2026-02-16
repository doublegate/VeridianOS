//! QEMU ramfb display device driver.
//!
//! Implements the `ramfb` virtual display for AArch64 and RISC-V guests
//! via QEMU's fw_cfg interface. This gives non-UEFI architectures a
//! graphical framebuffer output.
//!
//! Protocol: Write a `RamfbConfig` struct to the `etc/ramfb` fw_cfg
//! file selector. QEMU then displays the specified memory region as
//! a framebuffer.
//!
//! Requires `-device ramfb` on the QEMU command line.

use crate::error::KernelError;

/// DRM_FORMAT_XRGB8888 fourcc code (little-endian).
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // 'XR24'

/// fw_cfg MMIO base address for QEMU virt machine.
/// AArch64 and RISC-V virt machines use the same address.
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const FWCFG_BASE: usize = 0x0902_0000;

/// fw_cfg register offsets (MMIO, big-endian).
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const FWCFG_DATA: usize = 0x00;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const FWCFG_SELECTOR: usize = 0x08;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const FWCFG_DMA: usize = 0x10;

/// fw_cfg DMA control bits.
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const FWCFG_DMA_SELECT: u32 = 1 << 3;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const FWCFG_DMA_WRITE: u32 = 1 << 4;

/// Packed ramfb configuration structure (28 bytes).
/// All fields are big-endian as required by the fw_cfg protocol.
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
#[repr(C, packed)]
struct RamfbConfig {
    addr: u64,
    fourcc: u32,
    flags: u32,
    width: u32,
    height: u32,
    stride: u32,
}

/// fw_cfg DMA access descriptor (16 bytes).
#[repr(C, packed)]
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
struct FwCfgDmaAccess {
    control: u32,
    length: u32,
    address: u64,
}

/// Initialize the ramfb display device.
///
/// Allocates a framebuffer from the frame allocator and registers it
/// with QEMU via the fw_cfg `etc/ramfb` file selector.
///
/// Returns a pointer to the framebuffer memory on success.
///
/// # Arguments
/// * `width` - Display width in pixels
/// * `height` - Display height in pixels
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
pub fn init(width: u32, height: u32) -> Result<*mut u8, KernelError> {
    use crate::mm::{FRAME_ALLOCATOR, FRAME_SIZE};

    let stride = width * 4; // 4 bytes per pixel (XRGB8888)
    let fb_size = (stride * height) as usize;

    // Allocate contiguous physical frames for the framebuffer.
    let num_frames = fb_size.div_ceil(FRAME_SIZE);
    let frame_num = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(num_frames, None)
        .map_err(|_| KernelError::ResourceExhausted {
            resource: "physical frames for ramfb",
        })?;
    let fb_phys = frame_num.as_addr().as_u64() as usize;

    // On AArch64/RISC-V QEMU virt, physical memory is identity-mapped or
    // accessible at its physical address during early boot.
    let fb_ptr = fb_phys as *mut u8;

    // SAFETY: fb_ptr points to freshly allocated physical memory.
    // Writing zeros initializes the framebuffer to black.
    unsafe {
        core::ptr::write_bytes(fb_ptr, 0, fb_size);
    }

    // Build the ramfb config (all fields big-endian)
    let config = RamfbConfig {
        addr: (fb_phys as u64).to_be(),
        fourcc: DRM_FORMAT_XRGB8888.to_be(),
        flags: 0u32.to_be(),
        width: width.to_be(),
        height: height.to_be(),
        stride: stride.to_be(),
    };

    // Find the "etc/ramfb" fw_cfg selector by scanning the directory
    let selector = find_fwcfg_file("etc/ramfb").ok_or(KernelError::NotFound {
        resource: "fw_cfg etc/ramfb selector",
        id: 0,
    })?;

    // Write the config via fw_cfg DMA
    write_fwcfg_dma(selector, &config as *const RamfbConfig as *const u8, 28)?;

    Ok(fb_ptr)
}

/// Stub for architectures that don't support ramfb.
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
pub fn init(_width: u32, _height: u32) -> Result<*mut u8, KernelError> {
    Err(KernelError::OperationNotSupported {
        operation: "ramfb (x86_64 uses UEFI framebuffer)",
    })
}

/// Find a fw_cfg file by name and return its selector ID.
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn find_fwcfg_file(name: &str) -> Option<u16> {
    // Select the file directory (selector 0x0019)
    // SAFETY: MMIO writes to fw_cfg control registers.
    unsafe {
        let selector_reg = (FWCFG_BASE + FWCFG_SELECTOR) as *mut u16;
        selector_reg.write_volatile(0x0019u16.to_be());
    }

    // Read the file count (first 4 bytes, big-endian)
    let count: u32;
    // SAFETY: Reading from fw_cfg data register after selecting the directory.
    unsafe {
        let data_reg = (FWCFG_BASE + FWCFG_DATA) as *const u32;
        count = u32::from_be(data_reg.read_volatile());
    }

    // Each directory entry is 64 bytes: u32 size + u16 select + u16 reserved +
    // 56-byte name
    for _ in 0..count {
        let mut entry = [0u8; 64];
        // SAFETY: Reading sequential bytes from the fw_cfg data register.
        unsafe {
            let data_reg = FWCFG_BASE as *const u8;
            for byte in &mut entry {
                *byte = data_reg.read_volatile();
            }
        }
        let selector = u16::from_be_bytes([entry[4], entry[5]]);
        // Extract name (bytes 8..64, null-terminated)
        let name_bytes = &entry[8..64];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(56);
        let entry_name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("");
        if entry_name == name {
            return Some(selector);
        }
    }

    None
}

/// Write data to a fw_cfg file selector using DMA.
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn write_fwcfg_dma(selector: u16, data: *const u8, length: u32) -> Result<(), KernelError> {
    let dma_access = FwCfgDmaAccess {
        control: (FWCFG_DMA_SELECT | FWCFG_DMA_WRITE | ((selector as u32) << 16)).to_be(),
        length: length.to_be(),
        address: (data as u64).to_be(),
    };

    let dma_addr = &dma_access as *const FwCfgDmaAccess as u64;

    // Write the DMA descriptor address (big-endian, split into high/low 32-bit)
    // SAFETY: MMIO writes to the fw_cfg DMA register to initiate a write transfer.
    unsafe {
        let dma_reg_hi = (FWCFG_BASE + FWCFG_DMA) as *mut u32;
        let dma_reg_lo = (FWCFG_BASE + FWCFG_DMA + 4) as *mut u32;
        dma_reg_hi.write_volatile(((dma_addr >> 32) as u32).to_be());
        dma_reg_lo.write_volatile((dma_addr as u32).to_be());
    }

    // Poll for completion (control field becomes 0 when done).
    // Use raw pointer arithmetic to avoid referencing a packed field.
    let control_ptr = core::ptr::addr_of!(dma_access.control);
    // SAFETY: Reading the DMA control field to check completion. The
    // field is modified by the hypervisor (QEMU) when the DMA completes.
    // read_unaligned handles the packed struct alignment.
    for _ in 0..1_000_000 {
        let val = unsafe { core::ptr::read_unaligned(control_ptr) };
        if val == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }

    Err(KernelError::Timeout {
        operation: "fw_cfg DMA write",
        duration_ms: 0,
    })
}

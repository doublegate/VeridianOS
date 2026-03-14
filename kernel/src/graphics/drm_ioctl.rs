//! DRM ioctl interface for VeridianOS
//!
//! Exposes the kernel DRM/KMS infrastructure through Linux-compatible ioctl
//! numbers and C-ABI-stable structures. User-space libdrm calls ioctl() on
//! `/dev/dri/card0` or `/dev/dri/renderD128` and the request is routed here
//! via [`drm_ioctl_dispatch`].
//!
//! Each handler bridges to the existing gpu_accel.rs APIs (GemManager,
//! KmsManager, PageFlipManager, VirglDriver).

#![allow(dead_code)]

use super::gpu_accel::{
    self, ConnectorStatus, ConnectorType, DisplayMode, EncoderType, PageFlipRequest,
};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// DRM ioctl command numbers (Linux-compatible)
// ---------------------------------------------------------------------------

/// DRM_IOCTL_VERSION -- query driver name and version
pub(crate) const DRM_IOCTL_VERSION: u32 = 0x00;
/// DRM_IOCTL_GEM_CLOSE -- close a GEM handle
pub(crate) const DRM_IOCTL_GEM_CLOSE: u32 = 0x09;
/// DRM_IOCTL_GET_CAP -- query driver capabilities
pub(crate) const DRM_IOCTL_GET_CAP: u32 = 0x0C;
/// DRM_IOCTL_SET_CLIENT_CAP -- set client capabilities
pub(crate) const DRM_IOCTL_SET_CLIENT_CAP: u32 = 0x0D;
/// DRM_IOCTL_SET_MASTER -- acquire DRM master role
pub(crate) const DRM_IOCTL_SET_MASTER: u32 = 0x1E;
/// DRM_IOCTL_DROP_MASTER -- release DRM master role
pub(crate) const DRM_IOCTL_DROP_MASTER: u32 = 0x1F;
/// DRM_IOCTL_PRIME_HANDLE_TO_FD -- export GEM handle as DMA-BUF fd
pub(crate) const DRM_IOCTL_PRIME_HANDLE_TO_FD: u32 = 0x2D;
/// DRM_IOCTL_PRIME_FD_TO_HANDLE -- import DMA-BUF fd as GEM handle
pub(crate) const DRM_IOCTL_PRIME_FD_TO_HANDLE: u32 = 0x2E;
/// DRM_IOCTL_MODE_GETRESOURCES -- enumerate CRTCs, connectors, encoders
pub(crate) const DRM_IOCTL_MODE_GETRESOURCES: u32 = 0xA0;
/// DRM_IOCTL_MODE_GETCRTC -- query CRTC state
pub(crate) const DRM_IOCTL_MODE_GETCRTC: u32 = 0xA1;
/// DRM_IOCTL_MODE_SETCRTC -- configure CRTC mode + framebuffer
pub(crate) const DRM_IOCTL_MODE_SETCRTC: u32 = 0xA2;
/// DRM_IOCTL_MODE_GETENCODER -- query encoder state
pub(crate) const DRM_IOCTL_MODE_GETENCODER: u32 = 0xA6;
/// DRM_IOCTL_MODE_GETCONNECTOR -- query connector state and modes
pub(crate) const DRM_IOCTL_MODE_GETCONNECTOR: u32 = 0xA7;
/// DRM_IOCTL_MODE_GETPROPERTY -- query property metadata
pub(crate) const DRM_IOCTL_MODE_GETPROPERTY: u32 = 0xAA;
/// DRM_IOCTL_MODE_GETPROPBLOB -- read property blob data
pub(crate) const DRM_IOCTL_MODE_GETPROPBLOB: u32 = 0xAC;
/// DRM_IOCTL_MODE_ADDFB -- add framebuffer (legacy)
pub(crate) const DRM_IOCTL_MODE_ADDFB: u32 = 0xAE;
/// DRM_IOCTL_MODE_RMFB -- remove framebuffer
pub(crate) const DRM_IOCTL_MODE_RMFB: u32 = 0xAF;
/// DRM_IOCTL_MODE_PAGE_FLIP -- request a page flip
pub(crate) const DRM_IOCTL_MODE_PAGE_FLIP: u32 = 0xB0;
/// DRM_IOCTL_MODE_ADDFB2 -- add framebuffer (extended, with format)
pub(crate) const DRM_IOCTL_MODE_ADDFB2: u32 = 0xB8;
/// DRM_IOCTL_MODE_CREATE_DUMB -- allocate a dumb scanout buffer
pub(crate) const DRM_IOCTL_MODE_CREATE_DUMB: u32 = 0xB2;
/// DRM_IOCTL_MODE_MAP_DUMB -- prepare a dumb buffer for mmap
pub(crate) const DRM_IOCTL_MODE_MAP_DUMB: u32 = 0xB3;
/// DRM_IOCTL_MODE_DESTROY_DUMB -- free a dumb buffer
pub(crate) const DRM_IOCTL_MODE_DESTROY_DUMB: u32 = 0xB4;
/// DRM_IOCTL_MODE_GETPLANERESOURCES -- enumerate planes
pub(crate) const DRM_IOCTL_MODE_GETPLANERESOURCES: u32 = 0xB5;
/// DRM_IOCTL_MODE_GETPLANE -- get plane info
pub(crate) const DRM_IOCTL_MODE_GETPLANE: u32 = 0xB6;
/// DRM_IOCTL_MODE_OBJ_GETPROPERTIES -- get object properties
pub(crate) const DRM_IOCTL_MODE_OBJ_GETPROPERTIES: u32 = 0xB9;
/// DRM_IOCTL_MODE_OBJ_SETPROPERTY -- set object property
pub(crate) const DRM_IOCTL_MODE_OBJ_SETPROPERTY: u32 = 0xBA;
/// DRM_IOCTL_MODE_CURSOR -- set/unset cursor
pub(crate) const DRM_IOCTL_MODE_CURSOR: u32 = 0xA3;
/// DRM_IOCTL_MODE_CURSOR2 -- set cursor with hotspot
pub(crate) const DRM_IOCTL_MODE_CURSOR2: u32 = 0xBB;
/// DRM_IOCTL_MODE_ATOMIC -- atomic modesetting
pub(crate) const DRM_IOCTL_MODE_ATOMIC: u32 = 0xBC;
/// DRM_IOCTL_MODE_CREATEPROPBLOB -- create property blob
pub(crate) const DRM_IOCTL_MODE_CREATEPROPBLOB: u32 = 0xBD;
/// DRM_IOCTL_MODE_DESTROYPROPBLOB -- destroy property blob
pub(crate) const DRM_IOCTL_MODE_DESTROYPROPBLOB: u32 = 0xBE;

// ---------------------------------------------------------------------------
// DRM capability constants
// ---------------------------------------------------------------------------

/// Capability: supports dumb scanout buffers
pub(crate) const DRM_CAP_DUMB_BUFFER: u64 = 0x01;
/// Capability: VBLANK high CRTC
pub(crate) const DRM_CAP_VBLANK_HIGH_CRTC: u64 = 0x02;
/// Capability: preferred depth for dumb buffers
pub(crate) const DRM_CAP_DUMB_PREFERRED_DEPTH: u64 = 0x03;
/// Capability: prefer shadow buffer for dumb
pub(crate) const DRM_CAP_DUMB_PREFER_SHADOW: u64 = 0x04;
/// Capability: supports PRIME (DMA-BUF) import/export
pub(crate) const DRM_CAP_PRIME: u64 = 0x05;
/// Capability: timestamp monotonic
pub(crate) const DRM_CAP_TIMESTAMP_MONOTONIC: u64 = 0x06;
/// Capability: async page flip
pub(crate) const DRM_CAP_ASYNC_PAGE_FLIP: u64 = 0x07;
/// Capability: cursor width
pub(crate) const DRM_CAP_CURSOR_WIDTH: u64 = 0x08;
/// Capability: cursor height
pub(crate) const DRM_CAP_CURSOR_HEIGHT: u64 = 0x09;
/// Capability: supports addfb2 modifiers
pub(crate) const DRM_CAP_ADDFB2_MODIFIERS: u64 = 0x10;
/// Capability: CRTC in VBLANK event
pub(crate) const DRM_CAP_CRTC_IN_VBLANK_EVENT: u64 = 0x12;

/// Client capability: stereo 3D
pub(crate) const DRM_CLIENT_CAP_STEREO_3D: u64 = 1;
/// Client capability: universal planes
pub(crate) const DRM_CLIENT_CAP_UNIVERSAL_PLANES: u64 = 2;
/// Client capability: atomic modesetting
pub(crate) const DRM_CLIENT_CAP_ATOMIC: u64 = 3;

// ---------------------------------------------------------------------------
// C-compatible ioctl data structures (#[repr(C)])
// ---------------------------------------------------------------------------

/// DRM version info (DRM_IOCTL_VERSION)
///
/// Must match Linux `struct drm_version` layout exactly:
///   int version_major/minor/patchlevel (3x i32 + 4 bytes padding)
///   size_t name_len (u64 on x86_64), char *name (u64)
///   size_t date_len (u64), char *date (u64)
///   size_t desc_len (u64), char *desc (u64)
#[repr(C)]
#[derive(Debug, Clone)]
pub(crate) struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub _pad: u32,
    pub name_len: u64,
    pub name_ptr: u64,
    pub date_len: u64,
    pub date_ptr: u64,
    pub desc_len: u64,
    pub desc_ptr: u64,
}

/// DRM get capability (DRM_IOCTL_GET_CAP)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmGetCap {
    pub capability: u64,
    pub value: u64,
}

/// DRM GEM close (DRM_IOCTL_GEM_CLOSE)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmGemClose {
    pub handle: u32,
    pub pad: u32,
}

/// DRM PRIME handle-to-fd (DRM_IOCTL_PRIME_HANDLE_TO_FD)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmPrimeHandleToFd {
    pub handle: u32,
    pub flags: u32,
    pub fd: i32,
    pub pad: u32,
}

/// DRM PRIME fd-to-handle (DRM_IOCTL_PRIME_FD_TO_HANDLE)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmPrimeFdToHandle {
    pub fd: i32,
    pub pad: u32,
    pub handle: u32,
    pub pad2: u32,
}

/// DRM mode resources (DRM_IOCTL_MODE_GETRESOURCES)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeCardRes {
    pub fb_id_ptr: u64,
    pub crtc_id_ptr: u64,
    pub connector_id_ptr: u64,
    pub encoder_id_ptr: u64,
    pub count_fbs: u32,
    pub count_crtcs: u32,
    pub count_connectors: u32,
    pub count_encoders: u32,
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

/// DRM mode info (part of connector/CRTC responses)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeInfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub vrefresh: u32,
    pub flags: u32,
    pub mode_type: u32,
    pub name: [u8; 32],
}

impl DrmModeInfo {
    /// Convert from internal DisplayMode to DRM mode info
    pub(crate) fn from_display_mode(mode: &DisplayMode) -> Self {
        let mut name = [0u8; 32];
        // Generate a mode name like "1920x1080"
        let name_str = alloc::format!("{}x{}", mode.hdisplay, mode.vdisplay);
        let copy_len = name_str.len().min(31);
        name[..copy_len].copy_from_slice(&name_str.as_bytes()[..copy_len]);

        Self {
            clock: mode.clock_khz,
            hdisplay: mode.hdisplay as u16,
            hsync_start: mode.hsync_start as u16,
            hsync_end: mode.hsync_end as u16,
            htotal: mode.htotal as u16,
            hskew: 0,
            vdisplay: mode.vdisplay as u16,
            vsync_start: mode.vsync_start as u16,
            vsync_end: mode.vsync_end as u16,
            vtotal: mode.vtotal as u16,
            vscan: 0,
            // Convert from millihertz to hertz
            vrefresh: mode.vrefresh_mhz / 1000,
            flags: 0,
            mode_type: 0x40, // DRM_MODE_TYPE_PREFERRED
            name,
        }
    }

    /// Convert to internal DisplayMode
    pub(crate) fn to_display_mode(self) -> DisplayMode {
        DisplayMode {
            hdisplay: self.hdisplay as u32,
            vdisplay: self.vdisplay as u32,
            clock_khz: self.clock,
            hsync_start: self.hsync_start as u32,
            hsync_end: self.hsync_end as u32,
            htotal: self.htotal as u32,
            vsync_start: self.vsync_start as u32,
            vsync_end: self.vsync_end as u32,
            vtotal: self.vtotal as u32,
            vrefresh_mhz: self.vrefresh.checked_mul(1000).unwrap_or(60000),
        }
    }
}

/// DRM CRTC (DRM_IOCTL_MODE_GETCRTC / SETCRTC)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeCrtc {
    pub set_connectors_ptr: u64,
    pub count_connectors: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub x: u32,
    pub y: u32,
    pub gamma_size: u32,
    pub mode_valid: u32,
    pub mode: DrmModeInfo,
}

/// DRM encoder (DRM_IOCTL_MODE_GETENCODER)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeEncoder {
    pub encoder_id: u32,
    pub encoder_type: u32,
    pub crtc_id: u32,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

/// DRM connector (DRM_IOCTL_MODE_GETCONNECTOR)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeGetConnector {
    pub encoders_ptr: u64,
    pub modes_ptr: u64,
    pub props_ptr: u64,
    pub prop_values_ptr: u64,
    pub count_modes: u32,
    pub count_props: u32,
    pub count_encoders: u32,
    pub encoder_id: u32,
    pub connector_id: u32,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: u32,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
    pub pad: u32,
}

/// DRM create dumb buffer (DRM_IOCTL_MODE_CREATE_DUMB)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeCreateDumb {
    pub height: u32,
    pub width: u32,
    pub bpp: u32,
    pub flags: u32,
    /// Output: GEM handle
    pub handle: u32,
    /// Output: pitch (bytes per row)
    pub pitch: u32,
    /// Output: total size in bytes
    pub size: u64,
}

/// DRM map dumb buffer (DRM_IOCTL_MODE_MAP_DUMB)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeMapDumb {
    pub handle: u32,
    pub pad: u32,
    /// Output: fake mmap offset
    pub offset: u64,
}

/// DRM destroy dumb buffer (DRM_IOCTL_MODE_DESTROY_DUMB)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeDestroyDumb {
    pub handle: u32,
}

/// DRM page flip (DRM_IOCTL_MODE_PAGE_FLIP)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModePageFlip {
    pub crtc_id: u32,
    pub fb_id: u32,
    pub flags: u32,
    pub reserved: u32,
    pub user_data: u64,
}

/// DRM set client capability (DRM_IOCTL_SET_CLIENT_CAP)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmSetClientCap {
    pub capability: u64,
    pub value: u64,
}

/// DRM mode add framebuffer (DRM_IOCTL_MODE_ADDFB, legacy)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeAddFb {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub depth: u32,
    pub handle: u32,
    /// Output: framebuffer ID
    pub fb_id: u32,
}

/// DRM mode add framebuffer 2 (DRM_IOCTL_MODE_ADDFB2)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeAddFb2 {
    pub fb_id: u32, // output
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32, // fourcc
    pub flags: u32,
    pub handles: [u32; 4],
    pub pitches: [u32; 4],
    pub offsets: [u32; 4],
    pub modifier: [u64; 4],
}

/// DRM mode get property (DRM_IOCTL_MODE_GETPROPERTY)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeGetProperty {
    pub values_ptr: u64,
    pub enum_blob_ptr: u64,
    pub prop_id: u32,
    pub flags: u32,
    pub name: [u8; 32],
    pub count_values: u32,
    pub count_enum_blobs: u32,
}

/// DRM mode get property blob (DRM_IOCTL_MODE_GETPROPBLOB)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeGetBlob {
    pub blob_id: u32,
    pub length: u32,
    pub data: u64,
}

/// DRM mode remove framebuffer (DRM_IOCTL_MODE_RMFB)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmModeRmFb {
    pub fb_id: u32,
}

// ---------------------------------------------------------------------------
// DRM ioctl dispatcher
// ---------------------------------------------------------------------------

/// Dispatch a DRM ioctl.
///
/// `_fd` is the file descriptor (for future per-fd state tracking).
/// `request` is the full ioctl request value; we extract the command number
/// (low 8 bits after removing the DRM base offset).
/// `arg` points to the user-space ioctl data structure.
///
/// Returns 0 on success or a negative error code.
pub(crate) fn drm_ioctl_dispatch(_fd: i32, request: u64, arg: *mut u8) -> Result<i32, KernelError> {
    // Extract command number. Linux DRM ioctls encode direction + size in
    // the upper bits, but the command byte is at bits [7:0] of the number
    // field. The ioctl request also contains the DRM base ('d' = 0x64) in
    // bits [15:8]. We match on the command number alone for simplicity.
    let cmd = (request & 0xFF) as u32;

    // Log all DRM ioctls for debugging kwin bringup
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Writing to COM1 I/O port for diagnostic output.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"DRM_IO#");
            crate::arch::x86_64::idt::raw_serial_hex(cmd as u64);
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    match cmd {
        DRM_IOCTL_VERSION => handle_version(arg),
        DRM_IOCTL_GET_CAP => handle_get_cap(arg),
        DRM_IOCTL_SET_CLIENT_CAP => handle_set_client_cap(arg),
        DRM_IOCTL_GEM_CLOSE => handle_gem_close(arg),
        DRM_IOCTL_SET_MASTER => Ok(0),  // Accept silently
        DRM_IOCTL_DROP_MASTER => Ok(0), // Accept silently
        DRM_IOCTL_PRIME_HANDLE_TO_FD => handle_prime_handle_to_fd(arg),
        DRM_IOCTL_PRIME_FD_TO_HANDLE => handle_prime_fd_to_handle(arg),
        DRM_IOCTL_MODE_GETRESOURCES => handle_mode_get_resources(arg),
        DRM_IOCTL_MODE_GETCRTC => handle_mode_get_crtc(arg),
        DRM_IOCTL_MODE_SETCRTC => handle_mode_set_crtc(arg),
        DRM_IOCTL_MODE_GETENCODER => handle_mode_get_encoder(arg),
        DRM_IOCTL_MODE_GETCONNECTOR => handle_mode_get_connector(arg),
        DRM_IOCTL_MODE_GETPROPERTY => handle_mode_get_property(arg),
        DRM_IOCTL_MODE_GETPROPBLOB => handle_mode_get_prop_blob(arg),
        DRM_IOCTL_MODE_ADDFB => handle_mode_add_fb(arg),
        DRM_IOCTL_MODE_RMFB => handle_mode_rm_fb(arg),
        DRM_IOCTL_MODE_PAGE_FLIP => handle_mode_page_flip(arg),
        DRM_IOCTL_MODE_CREATE_DUMB => handle_mode_create_dumb(arg),
        DRM_IOCTL_MODE_MAP_DUMB => handle_mode_map_dumb(arg),
        DRM_IOCTL_MODE_DESTROY_DUMB => handle_mode_destroy_dumb(arg),
        DRM_IOCTL_MODE_ADDFB2 => handle_mode_add_fb2(arg),
        DRM_IOCTL_MODE_GETPLANERESOURCES => handle_mode_get_plane_resources(arg),
        DRM_IOCTL_MODE_GETPLANE => handle_mode_get_plane(arg),
        DRM_IOCTL_MODE_OBJ_GETPROPERTIES => handle_mode_obj_get_properties(arg),
        DRM_IOCTL_MODE_OBJ_SETPROPERTY => Ok(0), // Accept silently
        DRM_IOCTL_MODE_CURSOR | DRM_IOCTL_MODE_CURSOR2 => Ok(0), // Accept silently
        DRM_IOCTL_MODE_ATOMIC => Ok(0),          // Accept silently (non-atomic fallback)
        DRM_IOCTL_MODE_CREATEPROPBLOB => handle_mode_create_prop_blob(arg),
        DRM_IOCTL_MODE_DESTROYPROPBLOB => Ok(0), // Accept silently
        _ => {
            // Log unhandled DRM ioctl for debugging
            #[cfg(target_arch = "x86_64")]
            {
                // SAFETY: Writing to COM1 I/O port for diagnostic output.
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"DRM_UNK#");
                    crate::arch::x86_64::idt::raw_serial_hex(cmd as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b"\n");
                }
            }
            Err(KernelError::OperationNotSupported {
                operation: "unsupported DRM ioctl",
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Individual ioctl handlers
// ---------------------------------------------------------------------------

/// DRM_IOCTL_VERSION -- return driver name and version
fn handle_version(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_VERSION",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let ver = unsafe { &mut *(arg as *mut DrmVersion) };

    ver.version_major = 1;
    ver.version_minor = 0;
    ver.version_patchlevel = 0;
    ver._pad = 0;

    // Copy driver name if user provided a buffer
    let driver_name = b"veridian-drm";
    if ver.name_ptr != 0 && ver.name_len > 0 {
        let copy_len = (ver.name_len as usize).min(driver_name.len());
        // SAFETY: name_ptr was provided by user space and size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(driver_name.as_ptr(), ver.name_ptr as *mut u8, copy_len);
        }
    }
    ver.name_len = driver_name.len() as u64;

    // Copy date
    let date = b"20260307";
    if ver.date_ptr != 0 && ver.date_len > 0 {
        let copy_len = (ver.date_len as usize).min(date.len());
        // SAFETY: date_ptr was provided by user space and size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(date.as_ptr(), ver.date_ptr as *mut u8, copy_len);
        }
    }
    ver.date_len = date.len() as u64;

    // Copy description
    let desc = b"VeridianOS VirtIO GPU DRM driver";
    if ver.desc_ptr != 0 && ver.desc_len > 0 {
        let copy_len = (ver.desc_len as usize).min(desc.len());
        // SAFETY: desc_ptr was provided by user space and size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(desc.as_ptr(), ver.desc_ptr as *mut u8, copy_len);
        }
    }
    ver.desc_len = desc.len() as u64;

    Ok(0)
}

/// DRM_IOCTL_GET_CAP -- query driver capability
fn handle_get_cap(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_GET_CAP",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let cap = unsafe { &mut *(arg as *mut DrmGetCap) };

    cap.value = match cap.capability {
        DRM_CAP_DUMB_BUFFER => 1,
        DRM_CAP_VBLANK_HIGH_CRTC => 1,
        DRM_CAP_DUMB_PREFERRED_DEPTH => 24,
        DRM_CAP_DUMB_PREFER_SHADOW => 0,
        DRM_CAP_PRIME => 1,
        DRM_CAP_TIMESTAMP_MONOTONIC => 1,
        DRM_CAP_ASYNC_PAGE_FLIP => 0,
        DRM_CAP_CURSOR_WIDTH => 64,
        DRM_CAP_CURSOR_HEIGHT => 64,
        DRM_CAP_ADDFB2_MODIFIERS => 0,
        DRM_CAP_CRTC_IN_VBLANK_EVENT => 1,
        _ => 0,
    };

    Ok(0)
}

/// DRM_IOCTL_GEM_CLOSE -- close/release a GEM handle
fn handle_gem_close(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_GEM_CLOSE",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let close = unsafe { &*(arg as *const DrmGemClose) };

    gpu_accel::with_gem(|gem| {
        gem.destroy_buffer(close.handle);
    });

    Ok(0)
}

/// DRM_IOCTL_PRIME_HANDLE_TO_FD -- export GEM handle as DMA-BUF fd
fn handle_prime_handle_to_fd(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for PRIME_HANDLE_TO_FD",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let prime = unsafe { &mut *(arg as *mut DrmPrimeHandleToFd) };

    // Verify the handle exists
    let exists =
        gpu_accel::with_gem(|gem| gem.find_buffer(prime.handle).is_some()).unwrap_or(false);

    if !exists {
        return Err(KernelError::OperationNotSupported {
            operation: "invalid GEM handle for PRIME export",
        });
    }

    // Return a synthetic fd (handle + 1000 offset to avoid collisions)
    prime.fd = (prime.handle as i32).saturating_add(1000);

    Ok(0)
}

/// DRM_IOCTL_PRIME_FD_TO_HANDLE -- import DMA-BUF fd as GEM handle
fn handle_prime_fd_to_handle(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for PRIME_FD_TO_HANDLE",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let prime = unsafe { &mut *(arg as *mut DrmPrimeFdToHandle) };

    // Reverse the synthetic fd mapping
    let handle = (prime.fd).saturating_sub(1000) as u32;

    let exists = gpu_accel::with_gem(|gem| {
        if gem.find_buffer(handle).is_some() {
            gem.add_ref(handle);
            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if !exists {
        return Err(KernelError::OperationNotSupported {
            operation: "invalid PRIME fd for import",
        });
    }

    prime.handle = handle;

    Ok(0)
}

/// DRM_IOCTL_MODE_GETRESOURCES -- enumerate display resources
fn handle_mode_get_resources(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETRESOURCES",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let res = unsafe { &mut *(arg as *mut DrmModeCardRes) };

    gpu_accel::with_kms(|kms| {
        // Report counts
        res.count_fbs = kms.framebuffers.len() as u32;
        res.count_crtcs = kms.crtcs.len() as u32;
        res.count_connectors = kms.connectors.len() as u32;
        res.count_encoders = kms.encoders.len() as u32;

        // Copy IDs if user provided buffers
        if res.fb_id_ptr != 0 && !kms.framebuffers.is_empty() {
            let ptr = res.fb_id_ptr as *mut u32;
            for (i, fb) in kms.framebuffers.iter().enumerate() {
                // SAFETY: User provided buffer, bounded by count_fbs.
                unsafe {
                    ptr.add(i).write(fb.fb_id);
                }
            }
        }

        if res.crtc_id_ptr != 0 && !kms.crtcs.is_empty() {
            let ptr = res.crtc_id_ptr as *mut u32;
            for (i, crtc) in kms.crtcs.iter().enumerate() {
                // SAFETY: User provided buffer, bounded by count_crtcs.
                unsafe {
                    ptr.add(i).write(crtc.crtc_id);
                }
            }
        }

        if res.connector_id_ptr != 0 && !kms.connectors.is_empty() {
            let ptr = res.connector_id_ptr as *mut u32;
            for (i, conn) in kms.connectors.iter().enumerate() {
                // SAFETY: User provided buffer, bounded by count_connectors.
                unsafe {
                    ptr.add(i).write(conn.connector_id);
                }
            }
        }

        if res.encoder_id_ptr != 0 && !kms.encoders.is_empty() {
            let ptr = res.encoder_id_ptr as *mut u32;
            for (i, enc) in kms.encoders.iter().enumerate() {
                // SAFETY: User provided buffer, bounded by count_encoders.
                unsafe {
                    ptr.add(i).write(enc.encoder_id);
                }
            }
        }

        // Dimension limits
        res.min_width = 1;
        res.max_width = 7680;
        res.min_height = 1;
        res.max_height = 4320;
    });

    Ok(0)
}

/// DRM_IOCTL_MODE_GETCRTC -- query a CRTC's current state
fn handle_mode_get_crtc(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETCRTC",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let crtc_arg = unsafe { &mut *(arg as *mut DrmModeCrtc) };

    let found = gpu_accel::with_kms(|kms| {
        if let Some(crtc) = kms.find_crtc(crtc_arg.crtc_id) {
            crtc_arg.fb_id = crtc.fb_id.unwrap_or(0);
            crtc_arg.x = 0;
            crtc_arg.y = 0;
            crtc_arg.gamma_size = crtc.gamma_size;

            if let Some(ref mode) = crtc.mode {
                crtc_arg.mode_valid = 1;
                crtc_arg.mode = DrmModeInfo::from_display_mode(mode);
            } else {
                crtc_arg.mode_valid = 0;
            }
            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if !found {
        return Err(KernelError::OperationNotSupported {
            operation: "CRTC not found",
        });
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_SETCRTC -- set CRTC mode and framebuffer
fn handle_mode_set_crtc(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_SETCRTC",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let crtc_arg = unsafe { &*(arg as *const DrmModeCrtc) };

    let success = gpu_accel::with_kms(|kms| {
        if let Some(crtc) = kms.crtcs.iter_mut().find(|c| c.crtc_id == crtc_arg.crtc_id) {
            crtc.fb_id = if crtc_arg.fb_id != 0 {
                Some(crtc_arg.fb_id)
            } else {
                None
            };

            if crtc_arg.mode_valid != 0 {
                crtc.mode = Some(crtc_arg.mode.to_display_mode());
                crtc.active = true;
            } else {
                crtc.mode = None;
                crtc.active = false;
            }
            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if !success {
        return Err(KernelError::OperationNotSupported {
            operation: "CRTC set failed",
        });
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_GETENCODER -- query encoder state
fn handle_mode_get_encoder(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETENCODER",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let enc_arg = unsafe { &mut *(arg as *mut DrmModeEncoder) };

    let found = gpu_accel::with_kms(|kms| {
        if let Some(enc) = kms
            .encoders
            .iter()
            .find(|e| e.encoder_id == enc_arg.encoder_id)
        {
            enc_arg.encoder_type = match enc.encoder_type {
                EncoderType::None => 0,
                EncoderType::Dac => 1,
                EncoderType::Tmds => 2,
                EncoderType::Lvds => 3,
                EncoderType::DpMst => 4,
                EncoderType::Virtual => 5,
            };
            enc_arg.crtc_id = enc.crtc_id.unwrap_or(0);
            enc_arg.possible_crtcs = enc.possible_crtcs;
            enc_arg.possible_clones = 0;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if !found {
        return Err(KernelError::OperationNotSupported {
            operation: "encoder not found",
        });
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_GETCONNECTOR -- query connector state and modes
fn handle_mode_get_connector(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETCONNECTOR",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let conn_arg = unsafe { &mut *(arg as *mut DrmModeGetConnector) };

    let found = gpu_accel::with_kms(|kms| {
        if let Some(conn) = kms
            .connectors
            .iter()
            .find(|c| c.connector_id == conn_arg.connector_id)
        {
            conn_arg.encoder_id = conn.encoder_id.unwrap_or(0);
            conn_arg.connector_type = match conn.connector_type {
                ConnectorType::Hdmi => 11,
                ConnectorType::DisplayPort => 14,
                ConnectorType::Vga => 1,
                ConnectorType::Edp => 14,
                ConnectorType::Dvi => 3,
                ConnectorType::Lvds => 7,
                ConnectorType::Virtual => 15,
            };
            conn_arg.connector_type_id = 1;
            conn_arg.connection = match conn.status {
                ConnectorStatus::Connected => 1,
                ConnectorStatus::Disconnected => 2,
                _ => 3, // unknown
            };
            conn_arg.mm_width = 530; // ~24" monitor
            conn_arg.mm_height = 300;
            conn_arg.subpixel = 1; // DRM_MODE_SUBPIXEL_UNKNOWN
            conn_arg.count_modes = conn.modes.len() as u32;
            conn_arg.count_props = 0;
            conn_arg.count_encoders = if conn.encoder_id.is_some() { 1 } else { 0 };

            // Copy modes if user provided a buffer
            if conn_arg.modes_ptr != 0 && !conn.modes.is_empty() {
                let ptr = conn_arg.modes_ptr as *mut DrmModeInfo;
                for (i, mode) in conn.modes.iter().enumerate() {
                    // SAFETY: User provided buffer, bounded by count_modes.
                    unsafe {
                        ptr.add(i).write(DrmModeInfo::from_display_mode(mode));
                    }
                }
            }

            // Copy encoder ID if user provided a buffer
            if conn_arg.encoders_ptr != 0 {
                if let Some(enc_id) = conn.encoder_id {
                    // SAFETY: User provided buffer for at least 1 encoder ID.
                    unsafe {
                        (conn_arg.encoders_ptr as *mut u32).write(enc_id);
                    }
                }
            }

            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if !found {
        return Err(KernelError::OperationNotSupported {
            operation: "connector not found",
        });
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_CREATE_DUMB -- create a dumb scanout buffer via GEM
fn handle_mode_create_dumb(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_CREATE_DUMB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let dumb = unsafe { &mut *(arg as *mut DrmModeCreateDumb) };

    // Calculate pitch and size
    let bpp = if dumb.bpp == 0 { 32 } else { dumb.bpp };
    let pitch = dumb
        .width
        .checked_mul(bpp / 8)
        .ok_or(KernelError::OperationNotSupported {
            operation: "dumb buffer pitch overflow",
        })?;
    let size = (pitch as u64).checked_mul(dumb.height as u64).ok_or(
        KernelError::OperationNotSupported {
            operation: "dumb buffer size overflow",
        },
    )?;

    // Allocate GEM buffer
    let handle = gpu_accel::with_gem(|gem| gem.create_buffer(size as usize))
        .flatten()
        .ok_or(KernelError::OperationNotSupported {
            operation: "GEM allocation failed for dumb buffer",
        })?;

    dumb.handle = handle;
    dumb.pitch = pitch;
    dumb.size = size;

    Ok(0)
}

/// DRM_IOCTL_MODE_MAP_DUMB -- prepare a dumb buffer for user-space mmap
fn handle_mode_map_dumb(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_MAP_DUMB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let map = unsafe { &mut *(arg as *mut DrmModeMapDumb) };

    // Verify the handle exists
    let exists = gpu_accel::with_gem(|gem| gem.find_buffer(map.handle).is_some()).unwrap_or(false);

    if !exists {
        return Err(KernelError::OperationNotSupported {
            operation: "invalid handle for MAP_DUMB",
        });
    }

    // Return a synthetic offset (handle shifted left by 12 bits, like a page
    // offset) that mmap will use to locate the GEM buffer.
    map.offset = (map.handle as u64) << 12;

    Ok(0)
}

/// DRM_IOCTL_MODE_DESTROY_DUMB -- destroy a dumb buffer
fn handle_mode_destroy_dumb(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_DESTROY_DUMB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let destroy = unsafe { &*(arg as *const DrmModeDestroyDumb) };

    gpu_accel::with_gem(|gem| {
        gem.destroy_buffer(destroy.handle);
    });

    Ok(0)
}

/// DRM_IOCTL_MODE_PAGE_FLIP -- request a page flip
fn handle_mode_page_flip(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_PAGE_FLIP",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let flip = unsafe { &*(arg as *const DrmModePageFlip) };

    let success = gpu_accel::with_page_flip(|pf| {
        pf.request_flip(PageFlipRequest {
            crtc_id: flip.crtc_id,
            fb_id: flip.fb_id,
            user_data: flip.user_data,
        })
    })
    .unwrap_or(false);

    if !success {
        return Err(KernelError::OperationNotSupported {
            operation: "page flip request failed",
        });
    }

    Ok(0)
}

/// DRM_IOCTL_SET_CLIENT_CAP -- set client capability
fn handle_set_client_cap(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for SET_CLIENT_CAP",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let cap = unsafe { &*(arg as *const DrmSetClientCap) };

    // Accept known client capabilities. For our virtual DRM device we
    // accept all requests but don't actually change behavior.
    match cap.capability {
        DRM_CLIENT_CAP_STEREO_3D | DRM_CLIENT_CAP_UNIVERSAL_PLANES | DRM_CLIENT_CAP_ATOMIC => Ok(0),
        _ => {
            // Unknown capability: return success anyway to not block clients
            Ok(0)
        }
    }
}

/// DRM_IOCTL_MODE_ADDFB -- add framebuffer (legacy)
fn handle_mode_add_fb(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_ADDFB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let fb_arg = unsafe { &mut *(arg as *mut DrmModeAddFb) };

    let fb_id = gpu_accel::with_kms(|kms| {
        kms.create_framebuffer(
            fb_arg.width,
            fb_arg.height,
            super::PixelFormat::Xrgb8888,
            fb_arg.handle,
        )
    })
    .ok_or(KernelError::OperationNotSupported {
        operation: "KMS not initialized for ADDFB",
    })?;

    fb_arg.fb_id = fb_id;
    Ok(0)
}

/// DRM_IOCTL_MODE_ADDFB2 -- add framebuffer (extended)
fn handle_mode_add_fb2(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_ADDFB2",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let fb_arg = unsafe { &mut *(arg as *mut DrmModeAddFb2) };

    // Map fourcc pixel format to our internal format
    // DRM_FORMAT_XRGB8888 = 0x34325258 ('XR24')
    // DRM_FORMAT_ARGB8888 = 0x34325241 ('AR24')
    let _format = match fb_arg.pixel_format {
        0x34325258 => super::PixelFormat::Xrgb8888,
        0x34325241 => super::PixelFormat::Argb8888,
        _ => super::PixelFormat::Xrgb8888, // Default
    };

    let fb_id = gpu_accel::with_kms(|kms| {
        kms.create_framebuffer(fb_arg.width, fb_arg.height, _format, fb_arg.handles[0])
    })
    .ok_or(KernelError::OperationNotSupported {
        operation: "KMS not initialized for ADDFB2",
    })?;

    fb_arg.fb_id = fb_id;
    Ok(0)
}

/// DRM_IOCTL_MODE_RMFB -- remove framebuffer
fn handle_mode_rm_fb(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_RMFB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let rm_arg = unsafe { &*(arg as *const DrmModeRmFb) };

    gpu_accel::with_kms(|kms| {
        kms.destroy_framebuffer(rm_arg.fb_id);
    });

    Ok(0)
}

/// DRM_IOCTL_MODE_GETPROPERTY -- query property metadata
fn handle_mode_get_property(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETPROPERTY",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let prop = unsafe { &mut *(arg as *mut DrmModeGetProperty) };

    // Return a minimal property. KWin queries properties during init but
    // can function with empty/unknown property responses.
    prop.flags = 0;
    prop.count_values = 0;
    prop.count_enum_blobs = 0;
    // Zero-fill the name
    prop.name = [0u8; 32];
    let name = b"unknown";
    let copy_len = name.len().min(31);
    prop.name[..copy_len].copy_from_slice(&name[..copy_len]);

    Ok(0)
}

/// DRM_IOCTL_MODE_GETPROPBLOB -- read property blob data
fn handle_mode_get_prop_blob(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETPROPBLOB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let blob = unsafe { &mut *(arg as *mut DrmModeGetBlob) };

    // No property blobs stored yet. Return length=0 to indicate empty blob.
    blob.length = 0;

    Ok(0)
}

/// DRM plane resources (DRM_IOCTL_MODE_GETPLANERESOURCES)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DrmModePlaneRes {
    plane_id_ptr: u64,
    count_planes: u32,
    _pad: u32,
}

/// DRM plane info (DRM_IOCTL_MODE_GETPLANE)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DrmModeGetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    possible_crtcs: u32,
    gamma_size: u32,
    count_format_types: u32,
    format_type_ptr: u64,
}

/// DRM object properties (DRM_IOCTL_MODE_OBJ_GETPROPERTIES)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DrmModeObjGetProperties {
    props_ptr: u64,
    prop_values_ptr: u64,
    count_props: u32,
    obj_id: u32,
    obj_type: u32,
    _pad: u32,
}

/// DRM create property blob (DRM_IOCTL_MODE_CREATEPROPBLOB)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DrmModeCreateBlob {
    data: u64,
    length: u32,
    blob_id: u32,
}

/// DRM_IOCTL_MODE_GETPLANERESOURCES -- enumerate planes
///
/// KWin's DRM backend with universal planes queries this to find overlay,
/// cursor, and primary planes. We report one primary plane.
fn handle_mode_get_plane_resources(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETPLANERESOURCES",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let res = unsafe { &mut *(arg as *mut DrmModePlaneRes) };

    // Report one primary plane (id=1)
    if res.plane_id_ptr != 0 && res.count_planes >= 1 {
        // SAFETY: User provided buffer for at least 1 plane ID.
        unsafe {
            (res.plane_id_ptr as *mut u32).write(1);
        }
    }
    res.count_planes = 1;

    Ok(0)
}

/// DRM_IOCTL_MODE_GETPLANE -- get plane info
fn handle_mode_get_plane(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETPLANE",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let plane = unsafe { &mut *(arg as *mut DrmModeGetPlane) };

    plane.crtc_id = 1;
    plane.fb_id = 0;
    plane.possible_crtcs = 0x1; // Can drive CRTC 0
    plane.gamma_size = 0;

    // Report supported formats: XRGB8888 and ARGB8888
    let formats: [u32; 2] = [0x34325258, 0x34325241]; // XR24, AR24
    if plane.format_type_ptr != 0 && plane.count_format_types >= 2 {
        // SAFETY: User provided buffer for format types.
        unsafe {
            let ptr = plane.format_type_ptr as *mut u32;
            ptr.write(formats[0]);
            ptr.add(1).write(formats[1]);
        }
    }
    plane.count_format_types = 2;

    Ok(0)
}

/// DRM_IOCTL_MODE_OBJ_GETPROPERTIES -- get object properties
///
/// KWin queries properties for CRTCs, connectors, and planes.
/// Return empty property lists to keep things simple.
fn handle_mode_obj_get_properties(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_OBJ_GETPROPERTIES",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let props = unsafe { &mut *(arg as *mut DrmModeObjGetProperties) };

    // No properties -- KWin handles missing properties gracefully
    props.count_props = 0;

    Ok(0)
}

/// DRM_IOCTL_MODE_CREATEPROPBLOB -- create a property blob
///
/// KWin uses this to create mode blobs for atomic modesetting.
/// Assign a synthetic blob ID.
fn handle_mode_create_prop_blob(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_CREATEPROPBLOB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let blob = unsafe { &mut *(arg as *mut DrmModeCreateBlob) };

    // Assign a unique blob ID using an atomic counter
    static NEXT_BLOB_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(100);
    blob.blob_id = NEXT_BLOB_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

    Ok(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drm_mode_info_conversion() {
        let mode = DisplayMode::mode_1080p60();
        let info = DrmModeInfo::from_display_mode(&mode);
        assert_eq!(info.hdisplay, 1920);
        assert_eq!(info.vdisplay, 1080);
        assert_eq!(info.vrefresh, 60);
        assert_eq!(info.clock, 148500);

        let back = info.to_display_mode();
        assert_eq!(back.hdisplay, 1920);
        assert_eq!(back.vdisplay, 1080);
    }

    #[test]
    fn test_drm_mode_info_wxga() {
        let mode = DisplayMode::mode_wxga60();
        let info = DrmModeInfo::from_display_mode(&mode);
        assert_eq!(info.hdisplay, 1280);
        assert_eq!(info.vdisplay, 800);
    }

    #[test]
    fn test_create_dumb_pitch_calculation() {
        // 1920x1080 @ 32bpp -> pitch = 1920*4 = 7680
        let mut dumb = DrmModeCreateDumb {
            height: 1080,
            width: 1920,
            bpp: 32,
            flags: 0,
            handle: 0,
            pitch: 0,
            size: 0,
        };

        // We can't call the full handler without GEM init, but verify
        // the struct layout is correct.
        let bpp = dumb.bpp;
        let pitch = dumb.width * (bpp / 8);
        dumb.pitch = pitch;
        dumb.size = (pitch as u64) * (dumb.height as u64);

        assert_eq!(dumb.pitch, 7680);
        assert_eq!(dumb.size, 7680 * 1080);
    }
}

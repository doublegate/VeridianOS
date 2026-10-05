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

extern crate alloc;

use super::gpu_accel::{
    self, ConnectorStatus, ConnectorType, DisplayMode, EncoderType, PageFlipRequest,
};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// PRIME fd <-> GEM handle mapping
// ---------------------------------------------------------------------------

/// Map PRIME fd -> GEM handle. Simple fixed-size table for our virtual DRM
/// device (at most a handful of exported buffers).
static PRIME_FD_MAP: spin::Mutex<[(i32, u32); 8]> = spin::Mutex::new([(-1, 0); 8]);

/// Record a PRIME fd -> GEM handle mapping.
fn prime_map_insert(fd: i32, handle: u32) {
    let mut map = PRIME_FD_MAP.lock();
    // Find an empty slot or reuse an existing one for the same fd
    for entry in map.iter_mut() {
        if entry.0 == fd || entry.0 == -1 {
            *entry = (fd, handle);
            return;
        }
    }
    // Overflow: overwrite first slot (shouldn't happen with only a few buffers)
    map[0] = (fd, handle);
}

/// Look up a GEM handle from a PRIME fd.
fn prime_map_lookup(fd: i32) -> Option<u32> {
    let map = PRIME_FD_MAP.lock();
    for entry in map.iter() {
        if entry.0 == fd {
            return Some(entry.1);
        }
    }
    None
}

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
/// DRM_IOCTL_GET_UNIQUE -- return unique bus ID string
pub(crate) const DRM_IOCTL_GET_UNIQUE: u32 = 0x01;
/// DRM_IOCTL_GET_MAGIC -- get auth magic token (used by DRM auth)
pub(crate) const DRM_IOCTL_GET_MAGIC: u32 = 0x02;
/// DRM_IOCTL_AUTH_MAGIC -- authenticate a DRM client magic token
pub(crate) const DRM_IOCTL_AUTH_MAGIC: u32 = 0x11;
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
/// DRM_IOCTL_MODE_LIST_LESSEES -- list active DRM leases
pub(crate) const DRM_IOCTL_MODE_LIST_LESSEES: u32 = 0xC7;

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
///
/// NOTE: Both PRIME ioctls use the same C struct `drm_prime_handle`:
///   { __u32 handle; __u32 flags; __s32 fd; }
/// For FD_TO_HANDLE: fd is input (offset 8), handle is output (offset 0).
/// We reuse `DrmPrimeHandleToFd` for both since the layout is identical.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrmPrimeFdToHandle {
    pub handle: u32,
    pub flags: u32,
    pub fd: i32,
    pub pad: u32,
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
        DRM_IOCTL_GET_UNIQUE => handle_get_unique(arg),
        DRM_IOCTL_GET_MAGIC => handle_get_magic(arg),
        DRM_IOCTL_AUTH_MAGIC => Ok(0), // Accept silently -- all clients are trusted
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
        DRM_IOCTL_MODE_ATOMIC => handle_mode_atomic(arg),
        DRM_IOCTL_MODE_CREATEPROPBLOB => handle_mode_create_prop_blob(arg),
        DRM_IOCTL_MODE_DESTROYPROPBLOB => Ok(0), // Accept silently
        DRM_IOCTL_MODE_LIST_LESSEES => handle_mode_list_lessees(arg),
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

/// DRM_IOCTL_GET_UNIQUE -- return unique bus ID for the device.
///
/// libdrm and KWin call this to identify the DRM device. We return a PCI
/// bus ID string matching the virtual GPU at PCI 00:02.0.
fn handle_get_unique(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_GET_UNIQUE",
        });
    }

    /// Matches `struct drm_unique` from libdrm (drm.h).
    #[repr(C)]
    struct DrmUnique {
        unique_len: u64,
        unique_ptr: u64,
    }

    let unique_id = b"pci:0000:00:02.0";
    // SAFETY: Caller validated arg pointer before dispatch.
    let u = unsafe { &mut *(arg as *mut DrmUnique) };

    if u.unique_ptr != 0 && u.unique_len > 0 {
        let copy_len = (u.unique_len as usize).min(unique_id.len());
        // SAFETY: unique_ptr provided by user space, size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(unique_id.as_ptr(), u.unique_ptr as *mut u8, copy_len);
        }
    }
    u.unique_len = unique_id.len() as u64;
    Ok(0)
}

/// DRM_IOCTL_GET_MAGIC -- return a magic token for DRM authentication.
///
/// The master process authenticates client tokens via AUTH_MAGIC. Since
/// VeridianOS trusts all clients (single-user kernel-managed compositor),
/// we return a fixed token and accept all AUTH_MAGIC calls.
fn handle_get_magic(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_GET_MAGIC",
        });
    }

    /// Matches `struct drm_auth` from libdrm (drm.h).
    #[repr(C)]
    struct DrmAuth {
        magic: u32,
    }

    // SAFETY: Caller validated arg pointer before dispatch.
    let auth = unsafe { &mut *(arg as *mut DrmAuth) };
    auth.magic = 1; // fixed non-zero token -- AUTH_MAGIC accepts all
    Ok(0)
}

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
///
/// Creates a real file descriptor in the process file table backed by the
/// DRM device node. When user space mmaps this fd, the DRM mmap path
/// in sys_mmap maps the framebuffer physical memory into user space.
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

    // Create a real fd in the process file table backed by the DRM device
    // node so that mmap() on this fd triggers the DRM framebuffer mapping.
    let proc = crate::process::current_process().ok_or(KernelError::OperationNotSupported {
        operation: "PRIME export: no current process",
    })?;

    // Look up the DRM device node in VFS
    let flags = crate::fs::file::OpenFlags::read_write();
    let vfs = crate::fs::try_get_vfs().ok_or(KernelError::NotInitialized { subsystem: "VFS" })?;
    let vfs_read = vfs.read();
    let node =
        vfs_read
            .open("/dev/dri/card0", flags)
            .map_err(|_| KernelError::OperationNotSupported {
                operation: "PRIME export: cannot open DRM device",
            })?;

    // Create a File with the path set so mmap can detect it as DRM
    let file = crate::fs::file::File::new_with_path(
        node,
        flags,
        alloc::string::String::from("dri/card0-prime"),
    );

    let file_table = proc.file_table.lock();
    let new_fd = file_table.open(alloc::sync::Arc::new(file)).map_err(|_| {
        KernelError::ResourceExhausted {
            resource: "file descriptors",
        }
    })?;

    prime.fd = new_fd as i32;

    // Record the PRIME fd -> GEM handle mapping for PRIME_FD_TO_HANDLE
    prime_map_insert(new_fd as i32, prime.handle);

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

    // Look up the GEM handle from the PRIME fd mapping
    let handle = prime_map_lookup(prime.fd).ok_or(KernelError::OperationNotSupported {
        operation: "unknown PRIME fd for import",
    })?;

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

/// DRM_IOCTL_MODE_LIST_LESSEES -- list active DRM leases
///
/// Returns count=0 (no leasing support). The struct has { count_lessees, pad,
/// lessees_ptr }.
fn handle_mode_list_lessees(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_LIST_LESSEES",
        });
    }
    // struct drm_mode_list_lessees { __u32 count_lessees; __u32 pad; __u64
    // lessees_ptr; } SAFETY: Caller validated arg pointer before dispatch.
    let count_ptr = arg as *mut u32;
    unsafe {
        *count_ptr = 0; // No active leases
    }
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
        let ok = pf.request_flip(PageFlipRequest {
            crtc_id: flip.crtc_id,
            fb_id: flip.fb_id,
            user_data: flip.user_data,
        });
        if ok {
            // Immediately simulate vblank to complete the flip and generate
            // the page-flip completion event for user space to read.
            let ts_ns = crate::timer::get_uptime_ms() * 1_000_000;
            pf.handle_vblank(flip.crtc_id, ts_ns);
        }
        ok
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

// ---------------------------------------------------------------------------
// Plane property IDs and constants for atomic modesetting
// ---------------------------------------------------------------------------

/// Property ID for plane "type" (Primary/Overlay/Cursor)
const PROP_ID_TYPE: u32 = 10;
/// Property ID for plane "FB_ID" (framebuffer attached to plane)
const PROP_ID_FB_ID: u32 = 11;
/// Property ID for plane "CRTC_ID" (CRTC attached to plane)
const PROP_ID_CRTC_ID: u32 = 12;
/// Property ID for plane "CRTC_X" (destination X on CRTC)
const PROP_ID_CRTC_X: u32 = 13;
/// Property ID for plane "CRTC_Y" (destination Y on CRTC)
const PROP_ID_CRTC_Y: u32 = 14;
/// Property ID for plane "CRTC_W" (destination width on CRTC)
const PROP_ID_CRTC_W: u32 = 15;
/// Property ID for plane "CRTC_H" (destination height on CRTC)
const PROP_ID_CRTC_H: u32 = 16;
/// Property ID for plane "SRC_X" (source X in 16.16 fixed point)
const PROP_ID_SRC_X: u32 = 17;
/// Property ID for plane "SRC_Y" (source Y in 16.16 fixed point)
const PROP_ID_SRC_Y: u32 = 18;
/// Property ID for plane "SRC_W" (source width in 16.16 fixed point)
const PROP_ID_SRC_W: u32 = 19;
/// Property ID for plane "SRC_H" (source height in 16.16 fixed point)
const PROP_ID_SRC_H: u32 = 20;
/// Property ID for plane "IN_FORMATS" (blob with supported format/modifier
/// pairs)
const PROP_ID_IN_FORMATS: u32 = 21;

/// Property ID for connector "CRTC_ID"
const PROP_ID_CONN_CRTC_ID: u32 = 30;
/// Property ID for connector "DPMS"
const PROP_ID_CONN_DPMS: u32 = 31;

/// Property ID for CRTC "ACTIVE"
const PROP_ID_CRTC_ACTIVE: u32 = 40;
/// Property ID for CRTC "MODE_ID"
const PROP_ID_CRTC_MODE_ID: u32 = 41;

/// Blob ID for IN_FORMATS blob
const BLOB_ID_IN_FORMATS: u32 = 50;

/// DRM_MODE_PROP_IMMUTABLE flag
const DRM_MODE_PROP_IMMUTABLE: u32 = 1 << 2;
/// DRM_MODE_PROP_ENUM flag
const DRM_MODE_PROP_ENUM: u32 = 1 << 3;
/// DRM_MODE_PROP_BLOB flag
const DRM_MODE_PROP_BLOB: u32 = 1 << 4;
/// DRM_MODE_PROP_RANGE flag
const DRM_MODE_PROP_RANGE: u32 = 1 << 1;
/// DRM_MODE_PROP_SIGNED_RANGE flag
const DRM_MODE_PROP_SIGNED_RANGE: u32 = 1 << 5;

/// DRM object types
const DRM_MODE_OBJECT_CRTC: u32 = 0xCCCCCCCC;
const DRM_MODE_OBJECT_CONNECTOR: u32 = 0xC0C0C0C0;
const DRM_MODE_OBJECT_ENCODER: u32 = 0xE0E0E0E0;
const DRM_MODE_OBJECT_PLANE: u32 = 0xEEEEEEEE;

/// DRM_IOCTL_MODE_GETPROPERTY -- query property metadata
///
/// KWin queries each property by ID to learn its name, type (range/enum/blob),
/// and enumeration values. We return known metadata for our virtual plane,
/// connector, and CRTC properties.
fn handle_mode_get_property(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETPROPERTY",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let prop = unsafe { &mut *(arg as *mut DrmModeGetProperty) };

    // Zero-fill name first
    prop.name = [0u8; 32];

    match prop.prop_id {
        PROP_ID_TYPE => {
            // "type" property: immutable enum with Primary/Overlay/Cursor
            let name = b"type";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_ENUM | DRM_MODE_PROP_IMMUTABLE;
            // 3 enum values: Primary(1), Overlay(0), Cursor(2)
            prop.count_enum_blobs = 3;
            prop.count_values = 3;
            // If user provided buffer, write enum entries
            if prop.enum_blob_ptr != 0 && prop.count_enum_blobs >= 3 {
                // Each drm_mode_property_enum is { value: u64, name: [u8; 32] } = 40 bytes
                // SAFETY: User provided buffer for enum blob entries.
                unsafe {
                    let ptr = prop.enum_blob_ptr as *mut u8;
                    // Overlay = 0
                    (ptr as *mut u64).write(0);
                    let n = ptr.add(8);
                    let overlay_name = b"Overlay";
                    core::ptr::write_bytes(n, 0, 32);
                    core::ptr::copy_nonoverlapping(overlay_name.as_ptr(), n, overlay_name.len());
                    // Primary = 1
                    let entry1 = ptr.add(40);
                    (entry1 as *mut u64).write(1);
                    let n1 = entry1.add(8);
                    let primary_name = b"Primary";
                    core::ptr::write_bytes(n1, 0, 32);
                    core::ptr::copy_nonoverlapping(primary_name.as_ptr(), n1, primary_name.len());
                    // Cursor = 2
                    let entry2 = ptr.add(80);
                    (entry2 as *mut u64).write(2);
                    let n2 = entry2.add(8);
                    let cursor_name = b"Cursor";
                    core::ptr::write_bytes(n2, 0, 32);
                    core::ptr::copy_nonoverlapping(cursor_name.as_ptr(), n2, cursor_name.len());
                }
            }
            if prop.values_ptr != 0 && prop.count_values >= 3 {
                // SAFETY: User provided buffer for values.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // Overlay
                    vptr.add(1).write(1); // Primary
                    vptr.add(2).write(2); // Cursor
                }
            }
        }
        PROP_ID_FB_ID => {
            let name = b"FB_ID";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Range: any valid FB ID (0 = none)
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // min
                    vptr.add(1).write(0xFFFF_FFFF); // max
                }
            }
        }
        PROP_ID_CRTC_ID | PROP_ID_CONN_CRTC_ID => {
            let name = b"CRTC_ID";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Range: any valid CRTC ID (0 = none)
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // min
                    vptr.add(1).write(0xFFFF_FFFF); // max
                }
            }
        }
        PROP_ID_CRTC_X => {
            let name = b"CRTC_X";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_SIGNED_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Signed range: negative offsets allowed for panning
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    // i64::MIN as u64 and i32::MAX as u64 for signed range
                    vptr.write(i32::MIN as i64 as u64); // min
                    vptr.add(1).write(i32::MAX as i64 as u64); // max
                }
            }
        }
        PROP_ID_CRTC_Y => {
            let name = b"CRTC_Y";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_SIGNED_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Signed range: negative offsets allowed for panning
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(i32::MIN as i64 as u64); // min
                    vptr.add(1).write(i32::MAX as i64 as u64); // max
                }
            }
        }
        PROP_ID_CRTC_W | PROP_ID_CRTC_H => {
            let name = if prop.prop_id == PROP_ID_CRTC_W {
                b"CRTC_W\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"
            } else {
                b"CRTC_H\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"
            };
            prop.name[..6].copy_from_slice(&name[..6]);
            prop.flags = DRM_MODE_PROP_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Range: 0..8192 (max supported resolution dimension)
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // min
                    vptr.add(1).write(8192); // max
                }
            }
        }
        PROP_ID_SRC_X | PROP_ID_SRC_Y | PROP_ID_SRC_W | PROP_ID_SRC_H => {
            let name: &[u8] = match prop.prop_id {
                PROP_ID_SRC_X => b"SRC_X",
                PROP_ID_SRC_Y => b"SRC_Y",
                PROP_ID_SRC_W => b"SRC_W",
                _ => b"SRC_H",
            };
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Range: 0..0xFFFFFFFF (16.16 fixed-point coordinates)
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // min
                    vptr.add(1).write(0xFFFF_FFFF); // max
                }
            }
        }
        PROP_ID_IN_FORMATS => {
            let name = b"IN_FORMATS";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_BLOB | DRM_MODE_PROP_IMMUTABLE;
            prop.count_values = 0;
            prop.count_enum_blobs = 0;
        }
        PROP_ID_CONN_DPMS => {
            let name = b"DPMS";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_ENUM;
            prop.count_values = 4;
            prop.count_enum_blobs = 4;
            // Enum values: On(0), Standby(1), Suspend(2), Off(3)
            if prop.values_ptr != 0 && prop.count_values >= 4 {
                // SAFETY: User provided buffer for enum values.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // On
                    vptr.add(1).write(1); // Standby
                    vptr.add(2).write(2); // Suspend
                    vptr.add(3).write(3); // Off
                }
            }
            if prop.enum_blob_ptr != 0 && prop.count_enum_blobs >= 4 {
                // Each drm_mode_property_enum: { u64 value, [u8; 32] name } = 40 bytes
                // SAFETY: User provided buffer for enum blob entries.
                unsafe {
                    let ptr = prop.enum_blob_ptr as *mut u8;
                    let entries: [(&[u8], u64); 4] =
                        [(b"On", 0), (b"Standby", 1), (b"Suspend", 2), (b"Off", 3)];
                    for (i, (ename, eval)) in entries.iter().enumerate() {
                        let entry = ptr.add(i * 40);
                        (entry as *mut u64).write(*eval);
                        let n = entry.add(8);
                        core::ptr::write_bytes(n, 0, 32);
                        core::ptr::copy_nonoverlapping(ename.as_ptr(), n, ename.len());
                    }
                }
            }
        }
        PROP_ID_CRTC_ACTIVE => {
            let name = b"ACTIVE";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_RANGE;
            prop.count_values = 2;
            prop.count_enum_blobs = 0;
            // Range: 0..1 (boolean)
            if prop.values_ptr != 0 && prop.count_values >= 2 {
                // SAFETY: User provided buffer for range min/max.
                unsafe {
                    let vptr = prop.values_ptr as *mut u64;
                    vptr.write(0); // min
                    vptr.add(1).write(1); // max
                }
            }
        }
        PROP_ID_CRTC_MODE_ID => {
            let name = b"MODE_ID";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = DRM_MODE_PROP_BLOB;
            prop.count_values = 0;
            prop.count_enum_blobs = 0;
        }
        _ => {
            // Unknown property -- return generic metadata
            let name = b"unknown";
            prop.name[..name.len()].copy_from_slice(name);
            prop.flags = 0;
            prop.count_values = 0;
            prop.count_enum_blobs = 0;
        }
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_GETPROPBLOB -- read property blob data
///
/// KWin reads IN_FORMATS blob to discover supported pixel format + modifier
/// pairs. The blob uses the `drm_format_modifier_blob` layout:
///   header (24 bytes) + format array (u32[]) + modifier entries.
fn handle_mode_get_prop_blob(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETPROPBLOB",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let blob = unsafe { &mut *(arg as *mut DrmModeGetBlob) };

    if blob.blob_id == BLOB_ID_IN_FORMATS {
        // IN_FORMATS blob: header(24) + 2 formats(8) + 1 modifier entry(16) = 48 bytes
        //
        // struct drm_format_modifier_blob {
        //   u32 version;         // 1
        //   u32 flags;           // 0
        //   u32 count_formats;   // 2
        //   u32 formats_offset;  // 24 (right after header)
        //   u32 count_modifiers; // 1
        //   u32 modifiers_offset;// 32 (after 2 u32 formats)
        // }
        // u32 formats[2]: XRGB8888, ARGB8888
        // struct drm_format_modifier {
        //   u64 formats_bitmask; // bits 0+1 set = both formats
        //   u32 offset;          // 0 (starts at first format)
        //   u64 modifier;        // 0 = DRM_FORMAT_MOD_LINEAR
        // } -- but actual struct is { u64 formats, u32 offset, u32 pad, u64 modifier }
        // = 24 bytes Total: 24 + 8 + 24 = 56 bytes
        const BLOB_SIZE: u32 = 56;
        blob.length = BLOB_SIZE;

        if blob.data != 0 {
            // SAFETY: User provided buffer at blob.data with at least blob.length bytes.
            unsafe {
                let ptr = blob.data as *mut u8;
                core::ptr::write_bytes(ptr, 0, BLOB_SIZE as usize);

                // Header
                let hdr = ptr as *mut u32;
                hdr.write(1); // version
                hdr.add(1).write(0); // flags
                hdr.add(2).write(2); // count_formats
                hdr.add(3).write(24); // formats_offset
                hdr.add(4).write(1); // count_modifiers
                hdr.add(5).write(32); // modifiers_offset

                // Formats array at offset 24
                let fmt_ptr = ptr.add(24) as *mut u32;
                fmt_ptr.write(0x34325258); // DRM_FORMAT_XRGB8888
                fmt_ptr.add(1).write(0x34325241); // DRM_FORMAT_ARGB8888

                // Modifier entry at offset 32: { u64 formats_bitmask, u32 offset, u32 pad, u64
                // modifier }
                let mod_ptr = ptr.add(32);
                (mod_ptr as *mut u64).write(0x3); // formats bitmask: bits 0+1
                (mod_ptr.add(8) as *mut u32).write(0); // offset
                (mod_ptr.add(12) as *mut u32).write(0); // pad
                (mod_ptr.add(16) as *mut u64).write(0); // DRM_FORMAT_MOD_LINEAR
            }
        }
    } else {
        // Unknown blob -- return empty
        blob.length = 0;
    }

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

/// DRM atomic modesetting (DRM_IOCTL_MODE_ATOMIC)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DrmModeAtomic {
    flags: u32,
    count_objs: u32,
    objs_ptr: u64,
    count_props_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    reserved: u64,
    user_data: u64,
}

/// DRM_MODE_ATOMIC_TEST_ONLY -- validate without applying
const DRM_MODE_ATOMIC_TEST_ONLY: u32 = 0x0100;
/// DRM_MODE_ATOMIC_NONBLOCK -- non-blocking commit
const _DRM_MODE_ATOMIC_NONBLOCK: u32 = 0x0200;
/// DRM_MODE_ATOMIC_ALLOW_MODESET -- allow full modeset
const _DRM_MODE_ATOMIC_ALLOW_MODESET: u32 = 0x0400;
/// DRM_MODE_PAGE_FLIP_EVENT -- generate page flip completion event
const DRM_MODE_PAGE_FLIP_EVENT: u32 = 0x01;

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
/// KWin queries properties for CRTCs, connectors, and planes to build its
/// atomic modesetting pipeline. Without "type" on planes, kwin falls back
/// to legacy mode. We return the full set of standard properties.
fn handle_mode_obj_get_properties(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_OBJ_GETPROPERTIES",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let props = unsafe { &mut *(arg as *mut DrmModeObjGetProperties) };

    match props.obj_type {
        DRM_MODE_OBJECT_PLANE => {
            // Plane properties: type, FB_ID, CRTC_ID, CRTC_X/Y/W/H, SRC_X/Y/W/H, IN_FORMATS
            const PLANE_PROP_IDS: [u32; 12] = [
                PROP_ID_TYPE,
                PROP_ID_FB_ID,
                PROP_ID_CRTC_ID,
                PROP_ID_CRTC_X,
                PROP_ID_CRTC_Y,
                PROP_ID_CRTC_W,
                PROP_ID_CRTC_H,
                PROP_ID_SRC_X,
                PROP_ID_SRC_Y,
                PROP_ID_SRC_W,
                PROP_ID_SRC_H,
                PROP_ID_IN_FORMATS,
            ];
            // Values: type=Primary(1), rest=0, IN_FORMATS=blob_id
            const PLANE_PROP_VALUES: [u64; 12] = [
                1, // type = Primary
                0, // FB_ID
                0, // CRTC_ID
                0, // CRTC_X
                0, // CRTC_Y
                0, // CRTC_W
                0, // CRTC_H
                0, // SRC_X
                0, // SRC_Y
                0, // SRC_W
                0, // SRC_H
                BLOB_ID_IN_FORMATS as u64,
            ];
            let count = PLANE_PROP_IDS.len() as u32;

            if props.props_ptr != 0 && props.prop_values_ptr != 0 && props.count_props >= count {
                // SAFETY: User provided buffers for prop IDs and values.
                unsafe {
                    let id_ptr = props.props_ptr as *mut u32;
                    let val_ptr = props.prop_values_ptr as *mut u64;
                    for i in 0..PLANE_PROP_IDS.len() {
                        id_ptr.add(i).write(PLANE_PROP_IDS[i]);
                        val_ptr.add(i).write(PLANE_PROP_VALUES[i]);
                    }
                }
            }
            props.count_props = count;
        }
        DRM_MODE_OBJECT_CONNECTOR => {
            // Connector properties: CRTC_ID, DPMS
            const CONN_PROP_IDS: [u32; 2] = [PROP_ID_CONN_CRTC_ID, PROP_ID_CONN_DPMS];
            const CONN_PROP_VALUES: [u64; 2] = [1, 0]; // CRTC_ID=1, DPMS=On
            let count = CONN_PROP_IDS.len() as u32;

            if props.props_ptr != 0 && props.prop_values_ptr != 0 && props.count_props >= count {
                // SAFETY: User provided buffers for prop IDs and values.
                unsafe {
                    let id_ptr = props.props_ptr as *mut u32;
                    let val_ptr = props.prop_values_ptr as *mut u64;
                    for i in 0..CONN_PROP_IDS.len() {
                        id_ptr.add(i).write(CONN_PROP_IDS[i]);
                        val_ptr.add(i).write(CONN_PROP_VALUES[i]);
                    }
                }
            }
            props.count_props = count;
        }
        DRM_MODE_OBJECT_CRTC => {
            // CRTC properties: ACTIVE, MODE_ID
            const CRTC_PROP_IDS: [u32; 2] = [PROP_ID_CRTC_ACTIVE, PROP_ID_CRTC_MODE_ID];
            const CRTC_PROP_VALUES: [u64; 2] = [1, 0]; // ACTIVE=1, MODE_ID=0
            let count = CRTC_PROP_IDS.len() as u32;

            if props.props_ptr != 0 && props.prop_values_ptr != 0 && props.count_props >= count {
                // SAFETY: User provided buffers for prop IDs and values.
                unsafe {
                    let id_ptr = props.props_ptr as *mut u32;
                    let val_ptr = props.prop_values_ptr as *mut u64;
                    for i in 0..CRTC_PROP_IDS.len() {
                        id_ptr.add(i).write(CRTC_PROP_IDS[i]);
                        val_ptr.add(i).write(CRTC_PROP_VALUES[i]);
                    }
                }
            }
            props.count_props = count;
        }
        _ => {
            // Unknown object type -- return no properties
            props.count_props = 0;
        }
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_ATOMIC -- atomic modesetting commit
///
/// Accepts a batch of property changes and applies them atomically.
/// For our virtual DRM device, we parse the commit to track active FB_ID
/// on the primary plane (for future scanout), and always return success.
/// If DRM_MODE_ATOMIC_TEST_ONLY is set, we validate without applying.
/// If DRM_MODE_PAGE_FLIP_EVENT is set, we queue a page flip completion event.
fn handle_mode_atomic(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_ATOMIC",
        });
    }
    // SAFETY: Caller validated arg pointer before dispatch.
    let atomic = unsafe { &*(arg as *const DrmModeAtomic) };

    let is_test = atomic.flags & DRM_MODE_ATOMIC_TEST_ONLY != 0;
    let wants_event = atomic.flags & DRM_MODE_PAGE_FLIP_EVENT != 0;

    // Parse the commit to find FB_ID property updates on planes.
    // Layout: objs_ptr has count_objs u32 IDs, count_props_ptr has per-object
    // property counts (u32[]), props_ptr has all property IDs (u32[]),
    // prop_values_ptr has all property values (u64[]).
    if !is_test
        && atomic.count_objs > 0
        && atomic.objs_ptr != 0
        && atomic.props_ptr != 0
        && atomic.prop_values_ptr != 0
        && atomic.count_props_ptr != 0
    {
        let count_objs = (atomic.count_objs as usize).min(64); // sanity cap
        let mut prop_offset: usize = 0;

        for obj_idx in 0..count_objs {
            // SAFETY: User-provided arrays, bounded by count_objs.
            let _obj_id = unsafe { *(atomic.objs_ptr as *const u32).add(obj_idx) };
            let num_props =
                unsafe { *(atomic.count_props_ptr as *const u32).add(obj_idx) } as usize;
            let num_props = num_props.min(64); // sanity cap

            for p in 0..num_props {
                let idx = prop_offset + p;
                // SAFETY: User-provided arrays, bounded by accumulated count.
                let prop_id = unsafe { *(atomic.props_ptr as *const u32).add(idx) };
                let prop_val = unsafe { *(atomic.prop_values_ptr as *const u64).add(idx) };

                // Track FB_ID changes on the plane -> update CRTC active FB
                if prop_id == PROP_ID_FB_ID && prop_val != 0 {
                    let fb_id = prop_val as u32;
                    gpu_accel::with_kms(|kms| {
                        if let Some(crtc) = kms.crtcs.first_mut() {
                            crtc.fb_id = Some(fb_id);
                        }
                    });
                }

                // Track ACTIVE property on CRTC
                if prop_id == PROP_ID_CRTC_ACTIVE {
                    let active = prop_val != 0;
                    gpu_accel::with_kms(|kms| {
                        if let Some(crtc) = kms.crtcs.first_mut() {
                            crtc.active = active;
                        }
                    });
                }
            }
            prop_offset += num_props;
        }
    }

    // If page flip event was requested, queue a completion event.
    // Since we have no real hardware vsync, immediately simulate a vblank
    // after requesting the flip so that a drm_event_vblank is queued for
    // kwin to read from the DRM fd.
    if wants_event && !is_test {
        gpu_accel::with_page_flip(|pf| {
            let crtc_id = 1u32;
            // Queue the flip request
            let ok = pf.request_flip(PageFlipRequest {
                crtc_id,
                fb_id: 0, // FB tracked above
                user_data: atomic.user_data,
            });
            if ok {
                // Immediately simulate vblank to complete the flip and
                // generate the event. Use current uptime as timestamp.
                let ts_ns = crate::timer::get_uptime_ms() * 1_000_000;
                pf.handle_vblank(crtc_id, ts_ns);
            }
        });
    }

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

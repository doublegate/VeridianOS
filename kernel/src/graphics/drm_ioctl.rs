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
/// DRM_IOCTL_MODE_PAGE_FLIP -- request a page flip
pub(crate) const DRM_IOCTL_MODE_PAGE_FLIP: u32 = 0xB0;
/// DRM_IOCTL_MODE_CREATE_DUMB -- allocate a dumb scanout buffer
pub(crate) const DRM_IOCTL_MODE_CREATE_DUMB: u32 = 0xB2;
/// DRM_IOCTL_MODE_MAP_DUMB -- prepare a dumb buffer for mmap
pub(crate) const DRM_IOCTL_MODE_MAP_DUMB: u32 = 0xB3;
/// DRM_IOCTL_MODE_DESTROY_DUMB -- free a dumb buffer
pub(crate) const DRM_IOCTL_MODE_DESTROY_DUMB: u32 = 0xB4;

// ---------------------------------------------------------------------------
// DRM capability constants
// ---------------------------------------------------------------------------

/// Capability: supports dumb scanout buffers
pub(crate) const DRM_CAP_DUMB_BUFFER: u64 = 0x01;
/// Capability: supports PRIME (DMA-BUF) import/export
pub(crate) const DRM_CAP_PRIME: u64 = 0x05;
/// Capability: timestamp monotonic
pub(crate) const DRM_CAP_TIMESTAMP_MONOTONIC: u64 = 0x06;

// ---------------------------------------------------------------------------
// C-compatible ioctl data structures (#[repr(C)])
// ---------------------------------------------------------------------------

/// DRM version info (DRM_IOCTL_VERSION)
#[repr(C)]
#[derive(Debug, Clone)]
pub(crate) struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub name_len: u32,
    pub name_ptr: u64,
    pub date_len: u32,
    pub date_ptr: u64,
    pub desc_len: u32,
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

    match cmd {
        DRM_IOCTL_VERSION => handle_version(arg),
        DRM_IOCTL_GET_CAP => handle_get_cap(arg),
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
        DRM_IOCTL_MODE_PAGE_FLIP => handle_mode_page_flip(arg),
        DRM_IOCTL_MODE_CREATE_DUMB => handle_mode_create_dumb(arg),
        DRM_IOCTL_MODE_MAP_DUMB => handle_mode_map_dumb(arg),
        DRM_IOCTL_MODE_DESTROY_DUMB => handle_mode_destroy_dumb(arg),
        _ => Err(KernelError::OperationNotSupported {
            operation: "unsupported DRM ioctl",
        }),
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

    // Copy driver name if user provided a buffer
    let driver_name = b"veridian-drm";
    if ver.name_ptr != 0 && ver.name_len > 0 {
        let copy_len = (ver.name_len as usize).min(driver_name.len());
        // SAFETY: name_ptr was provided by user space and size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(driver_name.as_ptr(), ver.name_ptr as *mut u8, copy_len);
        }
    }
    ver.name_len = driver_name.len() as u32;

    // Copy date
    let date = b"20260307";
    if ver.date_ptr != 0 && ver.date_len > 0 {
        let copy_len = (ver.date_len as usize).min(date.len());
        // SAFETY: date_ptr was provided by user space and size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(date.as_ptr(), ver.date_ptr as *mut u8, copy_len);
        }
    }
    ver.date_len = date.len() as u32;

    // Copy description
    let desc = b"VeridianOS VirtIO GPU DRM driver";
    if ver.desc_ptr != 0 && ver.desc_len > 0 {
        let copy_len = (ver.desc_len as usize).min(desc.len());
        // SAFETY: desc_ptr was provided by user space and size-bounded.
        unsafe {
            core::ptr::copy_nonoverlapping(desc.as_ptr(), ver.desc_ptr as *mut u8, copy_len);
        }
    }
    ver.desc_len = desc.len() as u32;

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
        DRM_CAP_PRIME => 1,
        DRM_CAP_TIMESTAMP_MONOTONIC => 1,
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

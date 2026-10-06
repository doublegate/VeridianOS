//! DRM ioctl interface for VeridianOS
//!
//! Exposes the kernel DRM/KMS infrastructure through Linux-compatible ioctl
//! numbers and C-ABI-stable structures. User-space libdrm calls ioctl() on
//! `/dev/dri/card0` or `/dev/dri/renderD128` and the request is routed here
//! via [`drm_ioctl_dispatch`].
//!
//! Each handler bridges to the existing gpu_accel.rs APIs (GemManager,
//! KmsManager, PageFlipManager, VirglDriver).
//!
//! # User memory
//!
//! `sys_ioctl` runs every DRM ioctl through
//! [`crate::syscall::userspace::ioctl_bounce`], so the `arg` a handler
//! receives points at a kernel copy of the top-level struct, never at user
//! memory. Pointers *embedded* in those structs (`*_ptr`, `data`, ...) are
//! still user pointers: handlers only touch them through the validated
//! accessors (`write_user_slice`, `read_user_index`, ...), bound every copy by
//! the capacity the caller supplied (read before it is overwritten with the
//! required count), and gather data under the KMS lock but write it to user
//! memory only after releasing it.
//!
//! # Access control
//!
//! GEM handles are only usable by processes that created or imported them,
//! PRIME exports are keyed by (process, fd), and modesetting requires being
//! the DRM master: the first process to modeset or call SET_MASTER, until it
//! drops master or exits.

#![allow(dead_code)]

extern crate alloc;

use alloc::{collections::BTreeSet, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use super::gpu_accel::{
    self, ConnectorStatus, ConnectorType, DisplayMode, EncoderType, PageFlipRequest,
};
use crate::{
    error::KernelError,
    syscall::{
        userspace::{read_user_index, write_user, write_user_bytes, write_user_slice},
        SyscallError,
    },
};

/// Map a rejected user pointer to the error DRM handlers return.
fn bad_user_ptr(_: SyscallError) -> KernelError {
    KernelError::InvalidArgument {
        name: "drm_user_pointer",
        value: "invalid",
    }
}

/// Number of elements to copy into a user array that has room for
/// `capacity`: never more than the caller said it can hold (W-4).
fn bounded(capacity: u32, available: usize) -> usize {
    (capacity as usize).min(available)
}

/// Copy a (not NUL-terminated) string to a user buffer of `user_len` bytes,
/// truncating to fit, as libdrm's two-call length/data protocol expects.
fn copy_string_out(user_ptr: u64, user_len: u64, value: &[u8]) -> Result<(), KernelError> {
    if user_ptr == 0 || user_len == 0 {
        return Ok(());
    }
    let n = (user_len as usize).min(value.len());
    write_user_bytes(user_ptr as usize, &value[..n]).map_err(bad_user_ptr)
}

// ---------------------------------------------------------------------------
// Access control: GEM handle ownership, PRIME exports, DRM master
// ---------------------------------------------------------------------------

/// PID of the calling process, or 0 when there is none.
fn caller_pid() -> u64 {
    crate::process::current_process().map_or(0, |p| p.pid.0)
}

/// Whether `pid` names a process that has not exited.
fn process_alive(pid: u64) -> bool {
    crate::process::get_process(crate::process::ProcessId(pid)).is_some_and(|p| {
        !matches!(
            p.get_state(),
            crate::process::ProcessState::Zombie | crate::process::ProcessState::Dead
        )
    })
}

/// (GEM handle, pid) pairs: which processes may use which handles. GEM
/// handle numbers are global, so without this any process could close,
/// map or export another process's buffers (W-11).
static GEM_ACCESS: spin::Mutex<BTreeSet<(u32, u64)>> = spin::Mutex::new(BTreeSet::new());

fn gem_grant(handle: u32, pid: u64) {
    GEM_ACCESS.lock().insert((handle, pid));
}

/// Check the caller may use `handle`.
fn gem_check(handle: u32, pid: u64) -> Result<(), KernelError> {
    if GEM_ACCESS.lock().contains(&(handle, pid)) {
        Ok(())
    } else {
        Err(KernelError::PermissionDenied {
            operation: "GEM handle not owned by caller",
        })
    }
}

/// Drop the caller's access to `handle`; returns whether it had any.
fn gem_revoke(handle: u32, pid: u64) -> bool {
    GEM_ACCESS.lock().remove(&(handle, pid))
}

/// (framebuffer id, pid) pairs: the process that created each framebuffer,
/// the only one allowed to remove it.
static FB_OWNERS: spin::Mutex<BTreeSet<(u32, u64)>> = spin::Mutex::new(BTreeSet::new());

/// PRIME exports: (exporting pid, fd) -> GEM handle. Keyed by process so a
/// raw fd number names nothing in another process (W-11).
static PRIME_EXPORTS: spin::Mutex<alloc::collections::BTreeMap<(u64, i32), u32>> =
    spin::Mutex::new(alloc::collections::BTreeMap::new());

/// Forget a PRIME export when its fd is closed.
pub(crate) fn prime_fd_closed(pid: u64, fd: i32) {
    PRIME_EXPORTS.lock().remove(&(pid, fd));
}

/// Whether `pid` may mmap `length` bytes at `offset` of its DRM fd `fd`.
///
/// Only the DRM master may map at all: every dumb buffer of this virtual
/// device aliases the one scanout framebuffer, so mapping any of them gives
/// read/write access to what is on screen.
///
/// Dumb buffers are mapped at the offset MAP_DUMB returned (`handle << 12`),
/// which must name a handle the caller holds; a PRIME export is mapped at
/// offset 0 of the caller's own export fd. Every dumb buffer of this virtual
/// device is backed by the one scanout framebuffer, so the length can never
/// exceed it -- otherwise the mapping would expose the physical memory that
/// follows the framebuffer (W-6).
pub(crate) fn may_mmap(pid: u64, fd: i32, offset: usize, length: usize) -> bool {
    if !is_master(pid) {
        return false;
    }
    let Some(fb) = crate::graphics::framebuffer::get_fb_info() else {
        return false;
    };
    let fb_len = (fb.size as usize).next_multiple_of(4096);
    if length == 0 || length > fb_len {
        return false;
    }
    if offset == 0 {
        PRIME_EXPORTS.lock().contains_key(&(pid, fd))
    } else {
        offset.is_multiple_of(4096)
            && u32::try_from(offset >> 12).is_ok_and(|handle| gem_check(handle, pid).is_ok())
    }
}

/// PID of the DRM master, 0 if none.
static DRM_MASTER: AtomicU64 = AtomicU64::new(0);

/// Make the caller the DRM master if there is none (or the master has
/// exited), and fail if another live process holds it. Mirrors Linux, where
/// the first process to open the primary node becomes master (W-9).
fn ensure_master(pid: u64) -> Result<(), KernelError> {
    loop {
        let current = DRM_MASTER.load(Ordering::Acquire);
        if current == pid && pid != 0 {
            return Ok(());
        }
        if current != 0 && process_alive(current) {
            return Err(KernelError::PermissionDenied {
                operation: "DRM modeset requires master",
            });
        }
        if DRM_MASTER
            .compare_exchange(current, pid, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }
}

fn is_master(pid: u64) -> bool {
    pid != 0 && DRM_MASTER.load(Ordering::Acquire) == pid
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
/// `_fd` is the caller's file descriptor (unused; PRIME keys on new fds).
/// `request` is the full ioctl request value; we extract the command number
/// (low 8 bits after removing the DRM base offset).
/// `arg` points to a *kernel* copy of the ioctl struct (see the module docs);
/// it is at least 512 bytes and 8-byte aligned.
///
/// Returns 0 on success or a negative error code.
pub(crate) fn drm_ioctl_dispatch(_fd: i32, request: u64, arg: *mut u8) -> Result<i32, KernelError> {
    // Extract command number. Linux DRM ioctls encode direction + size in
    // the upper bits, but the command byte is at bits [7:0] of the number
    // field. The ioctl request also contains the DRM base ('d' = 0x64) in
    // bits [15:8]. We match on the command number alone for simplicity.
    let cmd = (request & 0xFF) as u32;

    // Log all DRM ioctls for debugging kwin bringup
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // SAFETY: Writing to COM1 I/O port for diagnostic output.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"DRM_IO#");
            crate::arch::x86_64::idt::raw_serial_hex(cmd as u64);
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    let pid = caller_pid();
    // Every check below is keyed by PID; without a calling process there is
    // nothing to authorize against, and PID 0 must never act as master or
    // share GEM handles.
    if pid == 0 {
        return Err(KernelError::PermissionDenied {
            operation: "DRM ioctl without a calling process",
        });
    }

    // Modesetting changes what is on screen for everyone, so it requires
    // the DRM master (claimed by the first process to modeset).
    if matches!(
        cmd,
        DRM_IOCTL_MODE_SETCRTC
            | DRM_IOCTL_MODE_PAGE_FLIP
            | DRM_IOCTL_MODE_OBJ_SETPROPERTY
            | DRM_IOCTL_MODE_CURSOR
            | DRM_IOCTL_MODE_CURSOR2
            | DRM_IOCTL_MODE_ATOMIC
    ) {
        ensure_master(pid)?;
    }

    match cmd {
        DRM_IOCTL_VERSION => handle_version(arg),
        DRM_IOCTL_GET_UNIQUE => handle_get_unique(arg),
        DRM_IOCTL_GET_MAGIC => handle_get_magic(arg),
        // Only the master may authenticate other clients.
        DRM_IOCTL_AUTH_MAGIC => {
            if is_master(pid) {
                Ok(0)
            } else {
                Err(KernelError::PermissionDenied {
                    operation: "DRM AUTH_MAGIC requires master",
                })
            }
        }
        DRM_IOCTL_GET_CAP => handle_get_cap(arg),
        DRM_IOCTL_SET_CLIENT_CAP => handle_set_client_cap(arg),
        DRM_IOCTL_GEM_CLOSE => handle_gem_close(arg, pid),
        DRM_IOCTL_SET_MASTER => ensure_master(pid).map(|()| 0),
        DRM_IOCTL_DROP_MASTER => {
            let _ = DRM_MASTER.compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Acquire);
            Ok(0)
        }
        DRM_IOCTL_PRIME_HANDLE_TO_FD => handle_prime_handle_to_fd(arg, pid),
        DRM_IOCTL_PRIME_FD_TO_HANDLE => handle_prime_fd_to_handle(arg, pid),
        DRM_IOCTL_MODE_GETRESOURCES => handle_mode_get_resources(arg),
        DRM_IOCTL_MODE_GETCRTC => handle_mode_get_crtc(arg),
        DRM_IOCTL_MODE_SETCRTC => handle_mode_set_crtc(arg),
        DRM_IOCTL_MODE_GETENCODER => handle_mode_get_encoder(arg),
        DRM_IOCTL_MODE_GETCONNECTOR => handle_mode_get_connector(arg),
        DRM_IOCTL_MODE_GETPROPERTY => handle_mode_get_property(arg),
        DRM_IOCTL_MODE_GETPROPBLOB => handle_mode_get_prop_blob(arg),
        DRM_IOCTL_MODE_ADDFB => handle_mode_add_fb(arg, pid),
        DRM_IOCTL_MODE_RMFB => handle_mode_rm_fb(arg, pid),
        DRM_IOCTL_MODE_PAGE_FLIP => handle_mode_page_flip(arg, pid),
        DRM_IOCTL_MODE_CREATE_DUMB => handle_mode_create_dumb(arg, pid),
        DRM_IOCTL_MODE_MAP_DUMB => handle_mode_map_dumb(arg, pid),
        DRM_IOCTL_MODE_DESTROY_DUMB => handle_mode_destroy_dumb(arg, pid),
        DRM_IOCTL_MODE_ADDFB2 => handle_mode_add_fb2(arg, pid),
        DRM_IOCTL_MODE_GETPLANERESOURCES => handle_mode_get_plane_resources(arg),
        DRM_IOCTL_MODE_GETPLANE => handle_mode_get_plane(arg),
        DRM_IOCTL_MODE_OBJ_GETPROPERTIES => handle_mode_obj_get_properties(arg),
        DRM_IOCTL_MODE_OBJ_SETPROPERTY => Ok(0), // Accept (master checked above)
        DRM_IOCTL_MODE_CURSOR | DRM_IOCTL_MODE_CURSOR2 => Ok(0), // Accept (master checked above)
        DRM_IOCTL_MODE_ATOMIC => handle_mode_atomic(arg, pid),
        DRM_IOCTL_MODE_CREATEPROPBLOB => handle_mode_create_prop_blob(arg),
        DRM_IOCTL_MODE_DESTROYPROPBLOB => Ok(0), // Accept silently
        DRM_IOCTL_MODE_LIST_LESSEES => handle_mode_list_lessees(arg),
        _ => {
            // Log unhandled DRM ioctl for debugging
            #[cfg(all(target_arch = "x86_64", target_os = "none"))]
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer,
    // large enough for any DRM ioctl struct.
    let u = unsafe { &mut *(arg as *mut DrmUnique) };

    copy_string_out(u.unique_ptr, u.unique_len, unique_id)?;
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

    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let auth = unsafe { &mut *(arg as *mut DrmAuth) };
    auth.magic = 1; // fixed non-zero token; only the master may AUTH_MAGIC
    Ok(0)
}

/// DRM_IOCTL_VERSION -- return driver name and version
fn handle_version(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_VERSION",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let ver = unsafe { &mut *(arg as *mut DrmVersion) };

    ver.version_major = 1;
    ver.version_minor = 0;
    ver.version_patchlevel = 0;
    ver._pad = 0;

    let driver_name = b"veridian-drm";
    copy_string_out(ver.name_ptr, ver.name_len, driver_name)?;
    ver.name_len = driver_name.len() as u64;

    let date = b"20260307";
    copy_string_out(ver.date_ptr, ver.date_len, date)?;
    ver.date_len = date.len() as u64;

    let desc = b"VeridianOS VirtIO GPU DRM driver";
    copy_string_out(ver.desc_ptr, ver.desc_len, desc)?;
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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
fn handle_gem_close(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for DRM_IOCTL_GEM_CLOSE",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let close = unsafe { &*(arg as *const DrmGemClose) };
    release_gem_handle(close.handle, pid)?;
    Ok(0)
}

/// Drop the caller's reference to a GEM handle it owns or imported.
fn release_gem_handle(handle: u32, pid: u64) -> Result<(), KernelError> {
    if !gem_revoke(handle, pid) {
        return Err(KernelError::PermissionDenied {
            operation: "GEM handle not owned by caller",
        });
    }
    gpu_accel::with_gem(|gem| {
        gem.destroy_buffer(handle);
    });
    Ok(())
}

/// DRM_IOCTL_PRIME_HANDLE_TO_FD -- export GEM handle as DMA-BUF fd
///
/// Creates a real file descriptor in the process file table backed by the
/// DRM device node. When user space mmaps this fd, the DRM mmap path
/// in sys_mmap maps the framebuffer physical memory into user space.
fn handle_prime_handle_to_fd(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for PRIME_HANDLE_TO_FD",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let prime = unsafe { &mut *(arg as *mut DrmPrimeHandleToFd) };

    // Only a handle the caller holds may be exported.
    gem_check(prime.handle, pid)?;

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
    let vfs_read = vfs;
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

    // Record the export for PRIME_FD_TO_HANDLE, keyed by this process.
    PRIME_EXPORTS
        .lock()
        .insert((pid, new_fd as i32), prime.handle);

    Ok(0)
}

/// DRM_IOCTL_PRIME_FD_TO_HANDLE -- import DMA-BUF fd as GEM handle
fn handle_prime_fd_to_handle(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for PRIME_FD_TO_HANDLE",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let prime = unsafe { &mut *(arg as *mut DrmPrimeFdToHandle) };

    // The fd is resolved in the caller's own export table: a raw fd number
    // names nothing in another process. (DMA-BUF passing between processes
    // over Unix sockets would need the handle stored on the file itself.)
    let handle = PRIME_EXPORTS.lock().get(&(pid, prime.fd)).copied().ok_or(
        KernelError::OperationNotSupported {
            operation: "unknown PRIME fd for import",
        },
    )?;

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

    gem_grant(handle, pid);
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let res = unsafe { &mut *(arg as *mut DrmModeCardRes) };

    // Gather under the KMS lock; write to user memory after releasing it.
    let ids = gpu_accel::with_kms(|kms| {
        (
            kms.framebuffers
                .iter()
                .map(|f| f.fb_id)
                .collect::<Vec<u32>>(),
            kms.crtcs.iter().map(|c| c.crtc_id).collect::<Vec<u32>>(),
            kms.connectors
                .iter()
                .map(|c| c.connector_id)
                .collect::<Vec<u32>>(),
            kms.encoders
                .iter()
                .map(|e| e.encoder_id)
                .collect::<Vec<u32>>(),
        )
    });
    let Some((fbs, crtcs, connectors, encoders)) = ids else {
        return Err(KernelError::NotInitialized { subsystem: "KMS" });
    };

    // Each array gets at most as many IDs as the caller has room for.
    for (ptr, capacity, values) in [
        (res.fb_id_ptr, res.count_fbs, &fbs),
        (res.crtc_id_ptr, res.count_crtcs, &crtcs),
        (res.connector_id_ptr, res.count_connectors, &connectors),
        (res.encoder_id_ptr, res.count_encoders, &encoders),
    ] {
        if ptr != 0 {
            write_user_slice(ptr as usize, &values[..bounded(capacity, values.len())])
                .map_err(bad_user_ptr)?;
        }
    }

    res.count_fbs = fbs.len() as u32;
    res.count_crtcs = crtcs.len() as u32;
    res.count_connectors = connectors.len() as u32;
    res.count_encoders = encoders.len() as u32;
    res.min_width = 1;
    res.max_width = 7680;
    res.min_height = 1;
    res.max_height = 4320;

    Ok(0)
}

/// DRM_IOCTL_MODE_GETCRTC -- query a CRTC's current state
fn handle_mode_get_crtc(arg: *mut u8) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_GETCRTC",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let conn_arg = unsafe { &mut *(arg as *mut DrmModeGetConnector) };

    // Capacities the caller supplied, read before they are overwritten with
    // the counts the kernel reports.
    let modes_capacity = conn_arg.count_modes;
    let encoders_capacity = conn_arg.count_encoders;
    let mut modes: Vec<DrmModeInfo> = Vec::new();
    let mut encoder: Option<u32> = None;

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

            modes = conn
                .modes
                .iter()
                .map(DrmModeInfo::from_display_mode)
                .collect();
            encoder = conn.encoder_id;
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

    // Copy out after the KMS lock is released, bounded by the caller's room.
    if conn_arg.modes_ptr != 0 {
        let n = bounded(modes_capacity, modes.len());
        write_user_slice(conn_arg.modes_ptr as usize, &modes[..n]).map_err(bad_user_ptr)?;
    }
    if let Some(enc_id) = encoder {
        if conn_arg.encoders_ptr != 0 && encoders_capacity >= 1 {
            write_user(conn_arg.encoders_ptr as usize, enc_id).map_err(bad_user_ptr)?;
        }
    }

    Ok(0)
}

/// DRM_IOCTL_MODE_CREATE_DUMB -- create a dumb scanout buffer via GEM
fn handle_mode_create_dumb(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_CREATE_DUMB",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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

    gem_grant(handle, pid);
    dumb.handle = handle;
    dumb.pitch = pitch;
    dumb.size = size;

    Ok(0)
}

/// DRM_IOCTL_MODE_MAP_DUMB -- prepare a dumb buffer for user-space mmap
fn handle_mode_map_dumb(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_MAP_DUMB",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let map = unsafe { &mut *(arg as *mut DrmModeMapDumb) };
    gem_check(map.handle, pid)?;

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
fn handle_mode_destroy_dumb(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_DESTROY_DUMB",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let destroy = unsafe { &*(arg as *const DrmModeDestroyDumb) };
    release_gem_handle(destroy.handle, pid)?;
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
    // lessees_ptr; }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    unsafe {
        *(arg as *mut u32) = 0; // No active leases
    }
    Ok(0)
}

/// DRM_IOCTL_MODE_PAGE_FLIP -- request a page flip
fn handle_mode_page_flip(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_PAGE_FLIP",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let flip = unsafe { &*(arg as *const DrmModePageFlip) };

    let success = gpu_accel::with_page_flip(|pf| {
        let ok = pf.request_flip(PageFlipRequest {
            crtc_id: flip.crtc_id,
            fb_id: flip.fb_id,
            user_data: flip.user_data,
            owner_pid: pid,
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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
fn handle_mode_add_fb(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_ADDFB",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let fb_arg = unsafe { &mut *(arg as *mut DrmModeAddFb) };
    gem_check(fb_arg.handle, pid)?;

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

    FB_OWNERS.lock().insert((fb_id, pid));
    fb_arg.fb_id = fb_id;
    Ok(0)
}

/// DRM_IOCTL_MODE_ADDFB2 -- add framebuffer (extended)
fn handle_mode_add_fb2(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_ADDFB2",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let fb_arg = unsafe { &mut *(arg as *mut DrmModeAddFb2) };
    gem_check(fb_arg.handles[0], pid)?;

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

    FB_OWNERS.lock().insert((fb_id, pid));
    fb_arg.fb_id = fb_id;
    Ok(0)
}

/// DRM_IOCTL_MODE_RMFB -- remove framebuffer
fn handle_mode_rm_fb(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_RMFB",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let rm_arg = unsafe { &*(arg as *const DrmModeRmFb) };

    // Only the process that created a framebuffer may remove it.
    if !FB_OWNERS.lock().remove(&(rm_arg.fb_id, pid)) {
        return Err(KernelError::PermissionDenied {
            operation: "framebuffer not owned by caller",
        });
    }
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let prop = unsafe { &mut *(arg as *mut DrmModeGetProperty) };

    // Capacities supplied by the caller, read before they are overwritten.
    let values_capacity = prop.count_values;
    let enums_capacity = prop.count_enum_blobs;

    let (name, flags, values, enums) = property_info(prop.prop_id);

    prop.name = [0u8; 32];
    prop.name[..name.len()].copy_from_slice(name);
    prop.flags = flags;

    if prop.values_ptr != 0 {
        let n = bounded(values_capacity, values.len());
        write_user_slice(prop.values_ptr as usize, &values[..n]).map_err(bad_user_ptr)?;
    }
    if prop.enum_blob_ptr != 0 {
        // Each struct drm_mode_property_enum is { u64 value; char name[32] }.
        let n = bounded(enums_capacity, enums.len());
        let records: Vec<DrmModePropertyEnum> = enums[..n]
            .iter()
            .map(|(ename, value)| DrmModePropertyEnum::new(*value, ename))
            .collect();
        write_user_slice(prop.enum_blob_ptr as usize, &records).map_err(bad_user_ptr)?;
    }
    prop.count_values = values.len() as u32;
    prop.count_enum_blobs = enums.len() as u32;

    Ok(0)
}

/// `struct drm_mode_property_enum` (40 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DrmModePropertyEnum {
    value: u64,
    name: [u8; 32],
}

impl DrmModePropertyEnum {
    fn new(value: u64, name: &[u8]) -> Self {
        let mut buf = [0u8; 32];
        let n = name.len().min(31);
        buf[..n].copy_from_slice(&name[..n]);
        Self { value, name: buf }
    }
}

/// Static description of a property: name, flags, values, enum entries.
type PropertyInfo = (
    &'static [u8],
    u32,
    &'static [u64],
    &'static [(&'static [u8], u64)],
);

/// Name, flags, values and enum entries for a property ID. Range properties
/// report `[min, max]` as their values; enum properties report each enum
/// value as a value and as a named entry.
fn property_info(prop_id: u32) -> PropertyInfo {
    const PLANE_TYPES: [(&[u8], u64); 3] = [(b"Overlay", 0), (b"Primary", 1), (b"Cursor", 2)];
    const DPMS: [(&[u8], u64); 4] = [(b"On", 0), (b"Standby", 1), (b"Suspend", 2), (b"Off", 3)];
    const ID_RANGE: [u64; 2] = [0, 0xFFFF_FFFF];
    const SIGNED_RANGE: [u64; 2] = [i32::MIN as i64 as u64, i32::MAX as i64 as u64];
    const SIZE_RANGE: [u64; 2] = [0, 8192];
    const BOOL_RANGE: [u64; 2] = [0, 1];

    match prop_id {
        PROP_ID_TYPE => (
            b"type",
            DRM_MODE_PROP_ENUM | DRM_MODE_PROP_IMMUTABLE,
            &[0, 1, 2],
            &PLANE_TYPES,
        ),
        PROP_ID_FB_ID => (b"FB_ID", DRM_MODE_PROP_RANGE, &ID_RANGE, &[]),
        PROP_ID_CRTC_ID | PROP_ID_CONN_CRTC_ID => (b"CRTC_ID", DRM_MODE_PROP_RANGE, &ID_RANGE, &[]),
        PROP_ID_CRTC_X => (b"CRTC_X", DRM_MODE_PROP_SIGNED_RANGE, &SIGNED_RANGE, &[]),
        PROP_ID_CRTC_Y => (b"CRTC_Y", DRM_MODE_PROP_SIGNED_RANGE, &SIGNED_RANGE, &[]),
        PROP_ID_CRTC_W => (b"CRTC_W", DRM_MODE_PROP_RANGE, &SIZE_RANGE, &[]),
        PROP_ID_CRTC_H => (b"CRTC_H", DRM_MODE_PROP_RANGE, &SIZE_RANGE, &[]),
        PROP_ID_SRC_X => (b"SRC_X", DRM_MODE_PROP_RANGE, &ID_RANGE, &[]),
        PROP_ID_SRC_Y => (b"SRC_Y", DRM_MODE_PROP_RANGE, &ID_RANGE, &[]),
        PROP_ID_SRC_W => (b"SRC_W", DRM_MODE_PROP_RANGE, &ID_RANGE, &[]),
        PROP_ID_SRC_H => (b"SRC_H", DRM_MODE_PROP_RANGE, &ID_RANGE, &[]),
        PROP_ID_IN_FORMATS => (
            b"IN_FORMATS",
            DRM_MODE_PROP_BLOB | DRM_MODE_PROP_IMMUTABLE,
            &[],
            &[],
        ),
        PROP_ID_CONN_DPMS => (b"DPMS", DRM_MODE_PROP_ENUM, &[0, 1, 2, 3], &DPMS),
        PROP_ID_CRTC_ACTIVE => (b"ACTIVE", DRM_MODE_PROP_RANGE, &BOOL_RANGE, &[]),
        PROP_ID_CRTC_MODE_ID => (b"MODE_ID", DRM_MODE_PROP_BLOB, &[], &[]),
        _ => (b"unknown", 0, &[], &[]),
    }
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let blob = unsafe { &mut *(arg as *mut DrmModeGetBlob) };

    // Room the caller has for the data, read before `length` is overwritten.
    let capacity = blob.length as usize;
    let data = if blob.blob_id == BLOB_ID_IN_FORMATS {
        in_formats_blob()
    } else {
        Vec::new() // unknown blob: empty
    };

    // As in Linux, data is copied only when the whole blob fits; otherwise
    // the caller learns the size from `length` and retries.
    if blob.data != 0 && !data.is_empty() && capacity >= data.len() {
        write_user_bytes(blob.data as usize, &data).map_err(bad_user_ptr)?;
    }
    blob.length = data.len() as u32;

    Ok(0)
}

/// The IN_FORMATS blob (`struct drm_format_modifier_blob`, 56 bytes):
/// 24-byte header, two u32 formats (XRGB8888, ARGB8888), and one 24-byte
/// `struct drm_format_modifier` { u64 formats; u32 offset; u32 pad;
/// u64 modifier } covering both formats with DRM_FORMAT_MOD_LINEAR.
fn in_formats_blob() -> Vec<u8> {
    let mut blob = Vec::with_capacity(56);
    for word in [1u32, 0, 2, 24, 1, 32] {
        // version, flags, count_formats, formats_offset, count_modifiers,
        // modifiers_offset
        blob.extend_from_slice(&word.to_ne_bytes());
    }
    blob.extend_from_slice(&0x3432_5258u32.to_ne_bytes()); // DRM_FORMAT_XRGB8888
    blob.extend_from_slice(&0x3432_5241u32.to_ne_bytes()); // DRM_FORMAT_ARGB8888
    blob.extend_from_slice(&0x3u64.to_ne_bytes()); // formats bitmask: both
    blob.extend_from_slice(&0u32.to_ne_bytes()); // offset
    blob.extend_from_slice(&0u32.to_ne_bytes()); // pad
    blob.extend_from_slice(&0u64.to_ne_bytes()); // DRM_FORMAT_MOD_LINEAR
    blob
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let res = unsafe { &mut *(arg as *mut DrmModePlaneRes) };

    // Report one primary plane (id=1)
    if res.plane_id_ptr != 0 && res.count_planes >= 1 {
        write_user(res.plane_id_ptr as usize, 1u32).map_err(bad_user_ptr)?;
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let plane = unsafe { &mut *(arg as *mut DrmModeGetPlane) };

    plane.crtc_id = 1;
    plane.fb_id = 0;
    plane.possible_crtcs = 0x1; // Can drive CRTC 0
    plane.gamma_size = 0;

    // Report supported formats: XRGB8888 and ARGB8888
    let formats: [u32; 2] = [0x34325258, 0x34325241]; // XR24, AR24
    if plane.format_type_ptr != 0 {
        let n = bounded(plane.count_format_types, formats.len());
        write_user_slice(plane.format_type_ptr as usize, &formats[..n]).map_err(bad_user_ptr)?;
    }
    plane.count_format_types = formats.len() as u32;

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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let props = unsafe { &mut *(arg as *mut DrmModeObjGetProperties) };

    // Plane: type=Primary(1), FB_ID .. SRC_H = 0, IN_FORMATS = blob id.
    const PLANE: [(u32, u64); 12] = [
        (PROP_ID_TYPE, 1),
        (PROP_ID_FB_ID, 0),
        (PROP_ID_CRTC_ID, 0),
        (PROP_ID_CRTC_X, 0),
        (PROP_ID_CRTC_Y, 0),
        (PROP_ID_CRTC_W, 0),
        (PROP_ID_CRTC_H, 0),
        (PROP_ID_SRC_X, 0),
        (PROP_ID_SRC_Y, 0),
        (PROP_ID_SRC_W, 0),
        (PROP_ID_SRC_H, 0),
        (PROP_ID_IN_FORMATS, BLOB_ID_IN_FORMATS as u64),
    ];
    // Connector: CRTC_ID=1, DPMS=On. CRTC: ACTIVE=1, MODE_ID=0.
    const CONNECTOR: [(u32, u64); 2] = [(PROP_ID_CONN_CRTC_ID, 1), (PROP_ID_CONN_DPMS, 0)];
    const CRTC: [(u32, u64); 2] = [(PROP_ID_CRTC_ACTIVE, 1), (PROP_ID_CRTC_MODE_ID, 0)];

    let table: &[(u32, u64)] = match props.obj_type {
        DRM_MODE_OBJECT_PLANE => &PLANE,
        DRM_MODE_OBJECT_CONNECTOR => &CONNECTOR,
        DRM_MODE_OBJECT_CRTC => &CRTC,
        _ => &[],
    };

    // Copied only when the caller has room for all of them (libdrm asks for
    // the count first, then calls again with buffers of that size).
    if props.props_ptr != 0
        && props.prop_values_ptr != 0
        && !table.is_empty()
        && props.count_props as usize >= table.len()
    {
        let ids: Vec<u32> = table.iter().map(|(id, _)| *id).collect();
        let values: Vec<u64> = table.iter().map(|(_, v)| *v).collect();
        write_user_slice(props.props_ptr as usize, &ids).map_err(bad_user_ptr)?;
        write_user_slice(props.prop_values_ptr as usize, &values).map_err(bad_user_ptr)?;
    }
    props.count_props = table.len() as u32;

    Ok(0)
}

/// DRM_IOCTL_MODE_ATOMIC -- atomic modesetting commit
///
/// Accepts a batch of property changes and applies them atomically.
/// For our virtual DRM device, we parse the commit to track active FB_ID
/// on the primary plane (for future scanout), and always return success.
/// If DRM_MODE_ATOMIC_TEST_ONLY is set, we validate without applying.
/// If DRM_MODE_PAGE_FLIP_EVENT is set, we queue a page flip completion event.
fn handle_mode_atomic(arg: *mut u8, pid: u64) -> Result<i32, KernelError> {
    if arg.is_null() {
        return Err(KernelError::OperationNotSupported {
            operation: "null arg for MODE_ATOMIC",
        });
    }
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
    let atomic = unsafe { &*(arg as *const DrmModeAtomic) };

    /// Upper bounds on one commit; KWin's commits are far smaller.
    const MAX_OBJS: usize = 64;
    const MAX_PROPS_TOTAL: usize = 1024;

    let is_test = atomic.flags & DRM_MODE_ATOMIC_TEST_ONLY != 0;
    let wants_event = atomic.flags & DRM_MODE_PAGE_FLIP_EVENT != 0;

    // Parse the commit to find FB_ID / ACTIVE property updates. Layout:
    // objs_ptr has count_objs u32 IDs, count_props_ptr has per-object
    // property counts (u32[]), props_ptr has all property IDs (u32[]),
    // prop_values_ptr has all property values (u64[]). All four are user
    // pointers, read element by element through validated accessors (W-5).
    if !is_test
        && atomic.count_objs > 0
        && atomic.objs_ptr != 0
        && atomic.props_ptr != 0
        && atomic.prop_values_ptr != 0
        && atomic.count_props_ptr != 0
    {
        if atomic.count_objs as usize > MAX_OBJS {
            return Err(KernelError::InvalidArgument {
                name: "atomic_count_objs",
                value: "too_large",
            });
        }
        let mut updates: Vec<(u32, u64)> = Vec::new();
        let mut prop_offset: usize = 0;

        for obj_idx in 0..atomic.count_objs as usize {
            let _obj_id: u32 =
                read_user_index(atomic.objs_ptr as usize, obj_idx).map_err(bad_user_ptr)?;
            let num_props: u32 =
                read_user_index(atomic.count_props_ptr as usize, obj_idx).map_err(bad_user_ptr)?;
            let end = prop_offset
                .checked_add(num_props as usize)
                .filter(|&end| end <= MAX_PROPS_TOTAL)
                .ok_or(KernelError::InvalidArgument {
                    name: "atomic_count_props",
                    value: "too_large",
                })?;

            for idx in prop_offset..end {
                let prop_id: u32 =
                    read_user_index(atomic.props_ptr as usize, idx).map_err(bad_user_ptr)?;
                let prop_val: u64 =
                    read_user_index(atomic.prop_values_ptr as usize, idx).map_err(bad_user_ptr)?;
                updates.push((prop_id, prop_val));
            }
            prop_offset = end;
        }

        // Apply only after the whole commit has been read successfully.
        gpu_accel::with_kms(|kms| {
            if let Some(crtc) = kms.crtcs.first_mut() {
                for (prop_id, prop_val) in &updates {
                    match *prop_id {
                        // Track FB_ID changes on the plane -> update CRTC active FB
                        PROP_ID_FB_ID if *prop_val != 0 => crtc.fb_id = Some(*prop_val as u32),
                        // Track ACTIVE property on CRTC
                        PROP_ID_CRTC_ACTIVE => crtc.active = *prop_val != 0,
                        _ => {}
                    }
                }
            }
        });
    }

    // If page flip event was requested, queue a completion event.
    // Since we have no real hardware vsync, immediately simulate a vblank
    // after requesting the flip so that a drm_event_vblank is queued for
    // the caller to read from the DRM fd.
    if wants_event && !is_test {
        gpu_accel::with_page_flip(|pf| {
            let crtc_id = 1u32;
            let ok = pf.request_flip(PageFlipRequest {
                crtc_id,
                fb_id: 0, // FB tracked above
                user_data: atomic.user_data,
                owner_pid: pid,
            });
            if ok {
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
    // SAFETY: `arg` is the dispatcher's 8-aligned kernel bounce buffer.
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

    const KERNEL_ADDR: u64 = 0xFFFF_8000_0000_1000;

    /// An 8-aligned stand-in for the dispatcher's kernel bounce buffer.
    fn bounce<T: Copy>(value: T) -> [u64; 64] {
        let mut buf = [0u64; 64];
        // SAFETY: T is a small repr(C) ioctl struct (< 512 bytes) and buf is
        // 8-aligned.
        unsafe { core::ptr::write(buf.as_mut_ptr() as *mut T, value) };
        buf
    }

    fn view<T: Copy>(buf: &[u64; 64]) -> T {
        // SAFETY: as in `bounce`.
        unsafe { core::ptr::read(buf.as_ptr() as *const T) }
    }

    /// W-4: values are copied only up to the capacity the caller gave, even
    /// though the property has more, and the full count is reported back.
    #[test]
    fn get_property_respects_value_capacity() {
        let mut values = [0xEEu64; 3];
        let mut buf = bounce(DrmModeGetProperty {
            values_ptr: values.as_mut_ptr() as u64,
            enum_blob_ptr: 0,
            prop_id: PROP_ID_CONN_DPMS,
            flags: 0,
            name: [0; 32],
            count_values: 2,
            count_enum_blobs: 0,
        });
        handle_mode_get_property(buf.as_mut_ptr() as *mut u8).unwrap();
        let prop: DrmModeGetProperty = view(&buf);
        assert_eq!(values, [0, 1, 0xEE]);
        assert_eq!(prop.count_values, 4);
        assert_eq!(prop.count_enum_blobs, 4);
        assert_eq!(&prop.name[..4], b"DPMS");
    }

    #[test]
    fn get_property_writes_enum_records() {
        let mut enums = [DrmModePropertyEnum::new(99, b"x"); 3];
        let mut buf = bounce(DrmModeGetProperty {
            values_ptr: 0,
            enum_blob_ptr: enums.as_mut_ptr() as u64,
            prop_id: PROP_ID_TYPE,
            flags: 0,
            name: [0; 32],
            count_values: 0,
            count_enum_blobs: 3,
        });
        handle_mode_get_property(buf.as_mut_ptr() as *mut u8).unwrap();
        assert_eq!(enums[1].value, 1);
        assert_eq!(&enums[1].name[..8], b"Primary\0");
    }

    #[test]
    fn get_property_rejects_kernel_pointer() {
        let mut buf = bounce(DrmModeGetProperty {
            values_ptr: KERNEL_ADDR,
            enum_blob_ptr: 0,
            prop_id: PROP_ID_FB_ID,
            flags: 0,
            name: [0; 32],
            count_values: 2,
            count_enum_blobs: 0,
        });
        assert!(handle_mode_get_property(buf.as_mut_ptr() as *mut u8).is_err());
    }

    /// The IN_FORMATS blob is copied only when it fits whole.
    #[test]
    fn get_prop_blob_requires_full_capacity() {
        let mut data = [0xEEu8; 56];
        let mut buf = bounce(DrmModeGetBlob {
            blob_id: BLOB_ID_IN_FORMATS,
            length: 55,
            data: data.as_mut_ptr() as u64,
        });
        handle_mode_get_prop_blob(buf.as_mut_ptr() as *mut u8).unwrap();
        assert_eq!(view::<DrmModeGetBlob>(&buf).length, 56);
        assert_eq!(data, [0xEE; 56]);

        let mut buf = bounce(DrmModeGetBlob {
            blob_id: BLOB_ID_IN_FORMATS,
            length: 56,
            data: data.as_mut_ptr() as u64,
        });
        handle_mode_get_prop_blob(buf.as_mut_ptr() as *mut u8).unwrap();
        assert_eq!(&data[..], &in_formats_blob()[..]);
        assert_eq!(
            u32::from_ne_bytes([data[8], data[9], data[10], data[11]]),
            2
        );
    }

    #[test]
    fn obj_get_properties_needs_room_for_all() {
        let mut ids = [0u32; 12];
        let mut vals = [0u64; 12];
        let mut buf = bounce(DrmModeObjGetProperties {
            props_ptr: ids.as_mut_ptr() as u64,
            prop_values_ptr: vals.as_mut_ptr() as u64,
            count_props: 11,
            obj_id: 1,
            obj_type: DRM_MODE_OBJECT_PLANE,
            _pad: 0,
        });
        handle_mode_obj_get_properties(buf.as_mut_ptr() as *mut u8).unwrap();
        assert_eq!(ids, [0; 12]);
        assert_eq!(view::<DrmModeObjGetProperties>(&buf).count_props, 12);
    }

    /// W-5: ATOMIC reads its arrays through validated accessors, so a kernel
    /// address in prop_values_ptr is rejected rather than read.
    #[test]
    fn atomic_rejects_kernel_array_pointers() {
        let objs = [1u32];
        let counts = [1u32];
        let props = [PROP_ID_FB_ID];
        let mut buf = bounce(DrmModeAtomic {
            flags: 0,
            count_objs: 1,
            objs_ptr: objs.as_ptr() as u64,
            count_props_ptr: counts.as_ptr() as u64,
            props_ptr: props.as_ptr() as u64,
            prop_values_ptr: KERNEL_ADDR,
            reserved: 0,
            user_data: 0,
        });
        assert!(handle_mode_atomic(buf.as_mut_ptr() as *mut u8, 0).is_err());
    }

    #[test]
    fn atomic_caps_total_properties() {
        let objs = [1u32];
        let counts = [1025u32];
        let props = [0u32; 1];
        let values = [0u64; 1];
        let mut buf = bounce(DrmModeAtomic {
            flags: 0,
            count_objs: 1,
            objs_ptr: objs.as_ptr() as u64,
            count_props_ptr: counts.as_ptr() as u64,
            props_ptr: props.as_ptr() as u64,
            prop_values_ptr: values.as_ptr() as u64,
            reserved: 0,
            user_data: 0,
        });
        // 1025 properties exceed the per-commit cap, so the commit is
        // rejected before any property is read.
        assert!(handle_mode_atomic(buf.as_mut_ptr() as *mut u8, 0).is_err());
    }

    /// W-11: GEM handles are usable only by processes granted them.
    #[test]
    fn gem_access_is_per_process() {
        let handle = 0xDEAD_0001;
        gem_grant(handle, 41);
        assert!(gem_check(handle, 41).is_ok());
        assert!(gem_check(handle, 42).is_err());
        assert!(!gem_revoke(handle, 42));
        assert!(gem_revoke(handle, 41));
        assert!(gem_check(handle, 41).is_err());
    }

    #[test]
    fn copy_string_out_truncates_to_user_length() {
        let mut out = [0u8; 4];
        copy_string_out(out.as_mut_ptr() as u64, 3, b"veridian").unwrap();
        assert_eq!(&out, b"ver\0");
        assert!(copy_string_out(KERNEL_ADDR, 8, b"veridian").is_err());
        assert!(copy_string_out(0, 8, b"veridian").is_ok());
    }

    /// W-6: without a framebuffer nothing is mappable, and a handle the
    /// caller does not hold never is.
    #[test]
    fn may_mmap_requires_framebuffer_and_ownership() {
        assert!(!may_mmap(41, 5, 0x5000, 4096));
    }

    /// Dumb buffers alias the scanout framebuffer, so a non-master holding
    /// its own handle still may not map it.
    #[test]
    fn may_mmap_requires_master() {
        let handle = 0xDEAD_0002;
        gem_grant(handle, 4242);
        assert!(!is_master(4242));
        assert!(!may_mmap(4242, 5, (handle as usize) << 12, 4096));
        gem_revoke(handle, 4242);
    }

    /// No calling process: refused outright, never treated as master.
    #[test]
    fn dispatch_without_caller_is_refused() {
        let mut buf = [0u64; 64];
        // On the host test runner there is no current process (pid 0).
        assert!(
            drm_ioctl_dispatch(3, DRM_IOCTL_SET_MASTER as u64, buf.as_mut_ptr() as *mut u8)
                .is_err()
        );
        assert!(!is_master(0));
    }

    #[test]
    fn rm_fb_requires_creator() {
        FB_OWNERS.lock().insert((0xF00D, 51));
        let mut buf = bounce(DrmModeRmFb { fb_id: 0xF00D });
        assert!(handle_mode_rm_fb(buf.as_mut_ptr() as *mut u8, 52).is_err());
        assert!(handle_mode_rm_fb(buf.as_mut_ptr() as *mut u8, 51).is_ok());
        assert!(handle_mode_rm_fb(buf.as_mut_ptr() as *mut u8, 51).is_err());
    }

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

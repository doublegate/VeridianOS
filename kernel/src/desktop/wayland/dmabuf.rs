//! DMA-BUF Protocol (zwp_linux_dmabuf_v1)
//!
//! Provides zero-copy buffer sharing between GPU and compositor.
//!
//! On VeridianOS, DMA-BUF handles are virtio-gpu resource IDs rather than
//! Linux file descriptors. The protocol framework remains compatible with
//! the Wayland zwp_linux_dmabuf_v1 specification, but the underlying
//! buffer import mechanism uses virtio-gpu resource handles instead of
//! dmabuf fds. This allows GPU-rendered content to be composited without
//! copying pixel data through the CPU.
//!
//! ## Buffer lifecycle
//!
//! 1. Client calls `create_params()` to get a params builder
//! 2. Client calls `add_plane()` for each plane (most formats need 1)
//! 3. Client calls `create_buffer()` to finalize and import the buffer
//! 4. The compositor can then sample from the imported buffer directly

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

/// Wayland global interface name
pub const ZWP_LINUX_DMABUF_V1: &str = "zwp_linux_dmabuf_v1";

/// Protocol version
pub const ZWP_LINUX_DMABUF_V1_VERSION: u32 = 4;

// zwp_linux_dmabuf_v1 request opcodes
/// destroy
pub const ZWP_LINUX_DMABUF_V1_DESTROY: u16 = 0;
/// create_params(id: new_id) -> zwp_linux_buffer_params_v1
pub const ZWP_LINUX_DMABUF_V1_CREATE_PARAMS: u16 = 1;
/// get_default_feedback(id: new_id) -- since version 4
pub const ZWP_LINUX_DMABUF_V1_GET_DEFAULT_FEEDBACK: u16 = 2;
/// get_surface_feedback(id: new_id, surface: object) -- since version 4
pub const ZWP_LINUX_DMABUF_V1_GET_SURFACE_FEEDBACK: u16 = 3;

// zwp_linux_dmabuf_v1 event opcodes
/// format(format: uint) -- deprecated since version 4
pub const ZWP_LINUX_DMABUF_V1_FORMAT: u16 = 0;
/// modifier(format: uint, modifier_hi: uint, modifier_lo: uint) -- since
/// version 3
pub const ZWP_LINUX_DMABUF_V1_MODIFIER: u16 = 1;

// zwp_linux_buffer_params_v1 request opcodes
/// destroy
pub const ZWP_LINUX_BUFFER_PARAMS_V1_DESTROY: u16 = 0;
/// add(fd, plane_idx, offset, stride, modifier_hi, modifier_lo)
pub const ZWP_LINUX_BUFFER_PARAMS_V1_ADD: u16 = 1;
/// create(width, height, format, flags)
pub const ZWP_LINUX_BUFFER_PARAMS_V1_CREATE: u16 = 2;
/// create_immed(buffer_id, width, height, format, flags) -- since version 2
pub const ZWP_LINUX_BUFFER_PARAMS_V1_CREATE_IMMED: u16 = 3;

// zwp_linux_buffer_params_v1 event opcodes
/// created(buffer: new_id)
pub const ZWP_LINUX_BUFFER_PARAMS_V1_CREATED: u16 = 0;
/// failed
pub const ZWP_LINUX_BUFFER_PARAMS_V1_FAILED: u16 = 1;

// Buffer params flags
/// Bottom-first (y-inverted) buffer
pub const ZWP_LINUX_BUFFER_PARAMS_V1_FLAGS_Y_INVERT: u32 = 1;
/// Buffer content is interlaced
pub const ZWP_LINUX_BUFFER_PARAMS_V1_FLAGS_INTERLACED: u32 = 2;
/// Buffer content has bottom field first
pub const ZWP_LINUX_BUFFER_PARAMS_V1_FLAGS_BOTTOM_FIRST: u32 = 4;

/// Maximum number of planes per buffer
pub const MAX_PLANES: usize = 4;

// ---------------------------------------------------------------------------
// DRM fourcc format codes
// ---------------------------------------------------------------------------

/// Common DRM fourcc pixel format codes.
///
/// These are the standard format identifiers used by the Linux DRM
/// subsystem and the Wayland DMA-BUF protocol.
pub mod fourcc {
    /// 32-bit ARGB (8:8:8:8) -- the most common compositing format
    pub const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // AR24
    /// 32-bit XRGB (8:8:8:8) -- opaque RGB with unused alpha byte
    pub const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // XR24
    /// 32-bit ABGR (8:8:8:8) -- ARGB with reversed channel order
    pub const DRM_FORMAT_ABGR8888: u32 = 0x34324241; // AB24
    /// 32-bit XBGR (8:8:8:8) -- XRGB with reversed channel order
    pub const DRM_FORMAT_XBGR8888: u32 = 0x34324258; // XB24
    /// 24-bit RGB (8:8:8) -- packed, no padding
    pub const DRM_FORMAT_RGB888: u32 = 0x34324752; // RG24
    /// 24-bit BGR (8:8:8) -- packed, no padding
    pub const DRM_FORMAT_BGR888: u32 = 0x34324742; // BG24
    /// NV12 semi-planar YUV 4:2:0 -- common video format
    pub const DRM_FORMAT_NV12: u32 = 0x3231564E; // NV12
    /// YUYV packed YUV 4:2:2 -- common camera/video format
    pub const DRM_FORMAT_YUYV: u32 = 0x56595559; // YUYV
    /// 16-bit RGB 5:6:5 -- legacy embedded/mobile format
    pub const DRM_FORMAT_RGB565: u32 = 0x36314752; // RG16
}

// ---------------------------------------------------------------------------
// DRM format modifier constants
// ---------------------------------------------------------------------------

/// DRM format modifiers describe GPU-specific memory tiling layouts.
pub mod modifiers {
    /// Invalid modifier (unspecified layout)
    pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00FF_FFFF_FFFF_FFFF;
    /// Linear (row-major, no tiling) -- universally supported
    pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;
    /// Intel X-tiling (legacy)
    pub const I915_FORMAT_MOD_X_TILED: u64 = (1u64 << 56) | 1;
    /// Intel Y-tiling
    pub const I915_FORMAT_MOD_Y_TILED: u64 = (1u64 << 56) | 2;
    /// Intel Tile4 (Xe/DG2+)
    pub const I915_FORMAT_MOD_4_TILED: u64 = (1u64 << 56) | 9;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// DMA-BUF format descriptor (fourcc + modifier pair).
///
/// Each supported format is advertised as a (fourcc, modifier) combination.
/// The modifier describes the GPU-specific memory layout (e.g., linear,
/// tiled). A single fourcc may appear with multiple modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaBufFormat {
    /// DRM fourcc format code
    pub fourcc: u32,
    /// Upper 32 bits of the 64-bit modifier
    pub modifier_hi: u32,
    /// Lower 32 bits of the 64-bit modifier
    pub modifier_lo: u32,
}

impl DmaBufFormat {
    /// Create a new format descriptor.
    pub fn new(fourcc: u32, modifier: u64) -> Self {
        Self {
            fourcc,
            modifier_hi: (modifier >> 32) as u32,
            modifier_lo: modifier as u32,
        }
    }

    /// Reconstruct the full 64-bit modifier.
    pub fn modifier(&self) -> u64 {
        ((self.modifier_hi as u64) << 32) | (self.modifier_lo as u64)
    }

    /// Check whether this format uses the linear modifier.
    pub fn is_linear(&self) -> bool {
        self.modifier() == modifiers::DRM_FORMAT_MOD_LINEAR
    }
}

/// DMA-BUF parameter builder (corresponds to zwp_linux_buffer_params_v1).
///
/// Accumulates plane descriptions before buffer creation.
#[derive(Debug, Clone)]
pub struct DmaBufParams {
    /// Params object ID
    pub id: u32,
    /// Requested buffer width in pixels
    pub width: u32,
    /// Requested buffer height in pixels
    pub height: u32,
    /// DRM fourcc format code
    pub format: u32,
    /// Buffer creation flags (Y_INVERT, INTERLACED, BOTTOM_FIRST)
    pub flags: u32,
    /// Planes added so far (up to MAX_PLANES)
    pub planes: Vec<DmaBufPlane>,
}

impl DmaBufParams {
    /// Create empty params for accumulating planes.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            width: 0,
            height: 0,
            format: 0,
            flags: 0,
            planes: Vec::new(),
        }
    }
}

/// Single plane of a DMA-BUF.
///
/// Multi-planar formats (e.g., NV12) have one plane per component.
/// Most RGB formats use a single plane.
#[derive(Debug, Clone, Copy)]
pub struct DmaBufPlane {
    /// Virtio-GPU resource ID (VeridianOS DMA-BUF handle).
    ///
    /// On Linux this would be a file descriptor; on VeridianOS we use
    /// the virtio-gpu resource ID directly.
    pub resource_id: u32,
    /// Byte offset into the resource where this plane starts
    pub offset: u32,
    /// Byte stride (distance between rows) for this plane
    pub stride: u32,
    /// Upper 32 bits of the 64-bit format modifier
    pub modifier_hi: u32,
    /// Lower 32 bits of the 64-bit format modifier
    pub modifier_lo: u32,
}

impl DmaBufPlane {
    /// Reconstruct the full 64-bit modifier for this plane.
    pub fn modifier(&self) -> u64 {
        ((self.modifier_hi as u64) << 32) | (self.modifier_lo as u64)
    }
}

/// Imported DMA-BUF buffer ready for compositor use.
///
/// Created from validated DmaBufParams. The compositor references this
/// buffer by its `id` when compositing.
pub struct DmaBufBuffer {
    /// Buffer ID within the DMA-BUF manager
    pub id: u32,
    /// The validated params that created this buffer
    pub params: DmaBufParams,
    /// Associated wl_buffer object ID (for Wayland object mapping)
    pub wl_buffer_id: u32,
}

// ---------------------------------------------------------------------------
// DMA-BUF manager
// ---------------------------------------------------------------------------

/// DMA-BUF manager.
///
/// Tracks supported formats, in-progress params builders, and imported
/// buffers. Provides the server-side implementation of zwp_linux_dmabuf_v1.
pub struct DmaBufManager {
    /// Supported format + modifier combinations advertised to clients
    supported_formats: Vec<DmaBufFormat>,
    /// In-progress params builders keyed by params object ID
    params: BTreeMap<u32, DmaBufParams>,
    /// Imported buffers keyed by buffer ID
    buffers: BTreeMap<u32, DmaBufBuffer>,
    /// Next params object ID
    next_params_id: u32,
    /// Next buffer ID
    next_buffer_id: u32,
}

impl DmaBufManager {
    /// Create a new DMA-BUF manager with default supported formats.
    ///
    /// By default, supports ARGB8888 and XRGB8888 with the LINEAR modifier.
    /// These are universally supported by software renderers and virtio-gpu.
    pub fn new() -> Self {
        let supported_formats = vec![
            DmaBufFormat::new(
                fourcc::DRM_FORMAT_ARGB8888,
                modifiers::DRM_FORMAT_MOD_LINEAR,
            ),
            DmaBufFormat::new(
                fourcc::DRM_FORMAT_XRGB8888,
                modifiers::DRM_FORMAT_MOD_LINEAR,
            ),
        ];

        Self {
            supported_formats,
            params: BTreeMap::new(),
            buffers: BTreeMap::new(),
            next_params_id: 1,
            next_buffer_id: 1,
        }
    }

    /// Get the list of supported format + modifier combinations.
    pub fn get_supported_formats(&self) -> &[DmaBufFormat] {
        &self.supported_formats
    }

    /// Add a supported format + modifier combination.
    pub fn add_supported_format(&mut self, format: DmaBufFormat) {
        if !self.supported_formats.contains(&format) {
            self.supported_formats.push(format);
        }
    }

    /// Check whether a specific fourcc + modifier combination is supported.
    pub fn is_format_supported(&self, fourcc: u32, modifier: u64) -> bool {
        self.supported_formats
            .iter()
            .any(|f| f.fourcc == fourcc && f.modifier() == modifier)
    }

    /// Create a new params builder. Returns the params object ID.
    ///
    /// The client should subsequently call `add_plane()` for each buffer
    /// plane, then `create_buffer()` to finalize.
    pub fn create_params(&mut self) -> u32 {
        let id = self.next_params_id;
        self.next_params_id += 1;

        self.params.insert(id, DmaBufParams::new(id));
        id
    }

    /// Add a plane to an in-progress params builder.
    pub fn add_plane(&mut self, params_id: u32, plane: DmaBufPlane) -> Result<(), KernelError> {
        let params = self
            .params
            .get_mut(&params_id)
            .ok_or(KernelError::NotFound {
                resource: "dmabuf_params",
                id: params_id as u64,
            })?;

        if params.planes.len() >= MAX_PLANES {
            return Err(KernelError::InvalidArgument {
                name: "plane_count",
                value: "exceeds_max_planes",
            });
        }

        params.planes.push(plane);
        Ok(())
    }

    /// Finalize a params builder and create an imported DMA-BUF buffer.
    ///
    /// Validates that at least one plane was added and that the format is
    /// supported. Returns the buffer ID on success.
    pub fn create_buffer(
        &mut self,
        params_id: u32,
        width: u32,
        height: u32,
        format: u32,
        flags: u32,
    ) -> Result<u32, KernelError> {
        let mut params = self
            .params
            .remove(&params_id)
            .ok_or(KernelError::NotFound {
                resource: "dmabuf_params",
                id: params_id as u64,
            })?;

        // Validate: at least one plane required
        if params.planes.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "planes",
                value: "no_planes_added",
            });
        }

        // Validate: dimensions must be non-zero
        if width == 0 || height == 0 {
            return Err(KernelError::InvalidArgument {
                name: "dimensions",
                value: "zero_width_or_height",
            });
        }

        // Validate: all planes must use the same modifier
        let first_modifier = params.planes[0].modifier();
        for plane in &params.planes[1..] {
            if plane.modifier() != first_modifier {
                return Err(KernelError::InvalidArgument {
                    name: "modifier",
                    value: "planes_have_different_modifiers",
                });
            }
        }

        // Validate: format + modifier combination must be supported
        if !self.is_format_supported(format, first_modifier) {
            return Err(KernelError::InvalidArgument {
                name: "format",
                value: "unsupported_format_modifier",
            });
        }

        // Finalize the params
        params.width = width;
        params.height = height;
        params.format = format;
        params.flags = flags;

        // Create the imported buffer
        let buffer_id = self.next_buffer_id;
        self.next_buffer_id += 1;

        let buffer = DmaBufBuffer {
            id: buffer_id,
            params,
            wl_buffer_id: 0, // Set when bound to a wl_buffer object
        };

        self.buffers.insert(buffer_id, buffer);
        Ok(buffer_id)
    }

    /// Destroy an imported buffer.
    pub fn destroy_buffer(&mut self, buffer_id: u32) -> Result<(), KernelError> {
        self.buffers
            .remove(&buffer_id)
            .ok_or(KernelError::NotFound {
                resource: "dmabuf_buffer",
                id: buffer_id as u64,
            })?;
        Ok(())
    }

    /// Convenience wrapper: import a single-plane buffer from a virtio-gpu
    /// resource with the LINEAR modifier.
    ///
    /// This is the common case for software-rendered content shared via
    /// virtio-gpu. The stride is computed as `width * bpp`.
    pub fn import_from_virtio_gpu(
        &mut self,
        resource_id: u32,
        width: u32,
        height: u32,
        format: u32,
    ) -> Result<u32, KernelError> {
        let bpp: u32 = match format {
            fourcc::DRM_FORMAT_ARGB8888
            | fourcc::DRM_FORMAT_XRGB8888
            | fourcc::DRM_FORMAT_ABGR8888
            | fourcc::DRM_FORMAT_XBGR8888 => 4,
            fourcc::DRM_FORMAT_RGB888 | fourcc::DRM_FORMAT_BGR888 => 3,
            fourcc::DRM_FORMAT_RGB565 => 2,
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "format",
                    value: "unknown_format_bpp",
                });
            }
        };

        let stride = width * bpp;
        let params_id = self.create_params();

        let plane = DmaBufPlane {
            resource_id,
            offset: 0,
            stride,
            modifier_hi: 0,
            modifier_lo: 0, // LINEAR
        };

        self.add_plane(params_id, plane)?;
        self.create_buffer(params_id, width, height, format, 0)
    }

    /// Get a reference to an imported buffer.
    pub fn get_buffer(&self, buffer_id: u32) -> Option<&DmaBufBuffer> {
        self.buffers.get(&buffer_id)
    }

    /// Get a mutable reference to an imported buffer.
    pub fn get_buffer_mut(&mut self, buffer_id: u32) -> Option<&mut DmaBufBuffer> {
        self.buffers.get_mut(&buffer_id)
    }

    /// Cancel an in-progress params builder without creating a buffer.
    pub fn destroy_params(&mut self, params_id: u32) -> Result<(), KernelError> {
        self.params
            .remove(&params_id)
            .ok_or(KernelError::NotFound {
                resource: "dmabuf_params",
                id: params_id as u64,
            })?;
        Ok(())
    }

    /// Number of imported buffers.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Number of in-progress params builders.
    pub fn params_count(&self) -> usize {
        self.params.len()
    }
}

impl Default for DmaBufManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_formats() {
        let mgr = DmaBufManager::new();
        let formats = mgr.get_supported_formats();
        assert_eq!(formats.len(), 2);
        assert!(mgr.is_format_supported(
            fourcc::DRM_FORMAT_ARGB8888,
            modifiers::DRM_FORMAT_MOD_LINEAR
        ));
        assert!(mgr.is_format_supported(
            fourcc::DRM_FORMAT_XRGB8888,
            modifiers::DRM_FORMAT_MOD_LINEAR
        ));
        assert!(!mgr.is_format_supported(fourcc::DRM_FORMAT_NV12, modifiers::DRM_FORMAT_MOD_LINEAR));
    }

    #[test]
    fn test_create_buffer_single_plane() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();

        let plane = DmaBufPlane {
            resource_id: 42,
            offset: 0,
            stride: 1280 * 4,
            modifier_hi: 0,
            modifier_lo: 0,
        };

        mgr.add_plane(params_id, plane).unwrap();
        let buf_id = mgr
            .create_buffer(params_id, 1280, 720, fourcc::DRM_FORMAT_ARGB8888, 0)
            .unwrap();

        let buf = mgr.get_buffer(buf_id).unwrap();
        assert_eq!(buf.params.width, 1280);
        assert_eq!(buf.params.height, 720);
        assert_eq!(buf.params.planes.len(), 1);
        assert_eq!(buf.params.planes[0].resource_id, 42);
    }

    #[test]
    fn test_create_buffer_no_planes_fails() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();

        let result = mgr.create_buffer(params_id, 100, 100, fourcc::DRM_FORMAT_ARGB8888, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_buffer_zero_dimensions_fails() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();

        let plane = DmaBufPlane {
            resource_id: 1,
            offset: 0,
            stride: 0,
            modifier_hi: 0,
            modifier_lo: 0,
        };
        mgr.add_plane(params_id, plane).unwrap();

        let result = mgr.create_buffer(params_id, 0, 100, fourcc::DRM_FORMAT_ARGB8888, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_buffer_unsupported_format_fails() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();

        let plane = DmaBufPlane {
            resource_id: 1,
            offset: 0,
            stride: 100,
            modifier_hi: 0,
            modifier_lo: 0,
        };
        mgr.add_plane(params_id, plane).unwrap();

        // NV12 not in default supported formats
        let result = mgr.create_buffer(params_id, 100, 100, fourcc::DRM_FORMAT_NV12, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_mismatched_modifiers_fails() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();

        let plane1 = DmaBufPlane {
            resource_id: 1,
            offset: 0,
            stride: 100,
            modifier_hi: 0,
            modifier_lo: 0, // LINEAR
        };
        let plane2 = DmaBufPlane {
            resource_id: 2,
            offset: 0,
            stride: 50,
            modifier_hi: 1,
            modifier_lo: 1, // different
        };

        mgr.add_plane(params_id, plane1).unwrap();
        mgr.add_plane(params_id, plane2).unwrap();

        let result = mgr.create_buffer(params_id, 100, 100, fourcc::DRM_FORMAT_ARGB8888, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_too_many_planes_fails() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();

        for i in 0..MAX_PLANES {
            let plane = DmaBufPlane {
                resource_id: i as u32,
                offset: 0,
                stride: 100,
                modifier_hi: 0,
                modifier_lo: 0,
            };
            mgr.add_plane(params_id, plane).unwrap();
        }

        // One more should fail
        let extra_plane = DmaBufPlane {
            resource_id: 99,
            offset: 0,
            stride: 100,
            modifier_hi: 0,
            modifier_lo: 0,
        };
        assert!(mgr.add_plane(params_id, extra_plane).is_err());
    }

    #[test]
    fn test_destroy_buffer() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();
        let plane = DmaBufPlane {
            resource_id: 1,
            offset: 0,
            stride: 400,
            modifier_hi: 0,
            modifier_lo: 0,
        };
        mgr.add_plane(params_id, plane).unwrap();
        let buf_id = mgr
            .create_buffer(params_id, 100, 100, fourcc::DRM_FORMAT_ARGB8888, 0)
            .unwrap();

        assert_eq!(mgr.buffer_count(), 1);
        mgr.destroy_buffer(buf_id).unwrap();
        assert_eq!(mgr.buffer_count(), 0);
    }

    #[test]
    fn test_destroy_nonexistent_buffer_fails() {
        let mut mgr = DmaBufManager::new();
        assert!(mgr.destroy_buffer(999).is_err());
    }

    #[test]
    fn test_import_from_virtio_gpu() {
        let mut mgr = DmaBufManager::new();
        let buf_id = mgr
            .import_from_virtio_gpu(7, 640, 480, fourcc::DRM_FORMAT_ARGB8888)
            .unwrap();

        let buf = mgr.get_buffer(buf_id).unwrap();
        assert_eq!(buf.params.width, 640);
        assert_eq!(buf.params.height, 480);
        assert_eq!(buf.params.format, fourcc::DRM_FORMAT_ARGB8888);
        assert_eq!(buf.params.planes.len(), 1);
        assert_eq!(buf.params.planes[0].resource_id, 7);
        assert_eq!(buf.params.planes[0].stride, 640 * 4);
    }

    #[test]
    fn test_import_unknown_format_fails() {
        let mut mgr = DmaBufManager::new();
        let result = mgr.import_from_virtio_gpu(1, 100, 100, 0xDEADBEEF);
        assert!(result.is_err());
    }

    #[test]
    fn test_destroy_params() {
        let mut mgr = DmaBufManager::new();
        let params_id = mgr.create_params();
        assert_eq!(mgr.params_count(), 1);
        mgr.destroy_params(params_id).unwrap();
        assert_eq!(mgr.params_count(), 0);
    }

    #[test]
    fn test_add_supported_format() {
        let mut mgr = DmaBufManager::new();
        assert_eq!(mgr.get_supported_formats().len(), 2);

        mgr.add_supported_format(DmaBufFormat::new(
            fourcc::DRM_FORMAT_NV12,
            modifiers::DRM_FORMAT_MOD_LINEAR,
        ));
        assert_eq!(mgr.get_supported_formats().len(), 3);

        // Adding duplicate should not increase count
        mgr.add_supported_format(DmaBufFormat::new(
            fourcc::DRM_FORMAT_NV12,
            modifiers::DRM_FORMAT_MOD_LINEAR,
        ));
        assert_eq!(mgr.get_supported_formats().len(), 3);
    }

    #[test]
    fn test_format_is_linear() {
        let linear = DmaBufFormat::new(
            fourcc::DRM_FORMAT_ARGB8888,
            modifiers::DRM_FORMAT_MOD_LINEAR,
        );
        assert!(linear.is_linear());

        let tiled = DmaBufFormat::new(
            fourcc::DRM_FORMAT_ARGB8888,
            modifiers::I915_FORMAT_MOD_X_TILED,
        );
        assert!(!tiled.is_linear());
    }

    #[test]
    fn test_format_modifier_roundtrip() {
        let f = DmaBufFormat::new(
            fourcc::DRM_FORMAT_ARGB8888,
            modifiers::I915_FORMAT_MOD_Y_TILED,
        );
        assert_eq!(f.modifier(), modifiers::I915_FORMAT_MOD_Y_TILED);
    }

    #[test]
    fn test_plane_modifier_roundtrip() {
        let p = DmaBufPlane {
            resource_id: 0,
            offset: 0,
            stride: 0,
            modifier_hi: 0x0001_0000,
            modifier_lo: 0x0000_0002,
        };
        assert_eq!(p.modifier(), 0x0001_0000_0000_0002);
    }
}

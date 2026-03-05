//! GPU Acceleration Framework
//!
//! Provides comprehensive GPU acceleration including:
//! - VirtIO GPU 3D (virgl) protocol support
//! - OpenGL ES 2.0 software rasterizer (integer/fixed-point math only)
//! - GEM/TTM buffer object management
//! - DRM Kernel Mode Setting (KMS)
//! - Vsync and page-flip scheduling
//! - Hardware cursor plane
//!
//! All arithmetic uses integer or 16.16 fixed-point math (no FPU required).

#![allow(dead_code)]

use alloc::{vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use spin::Mutex;

// ---------------------------------------------------------------------------
// Fixed-point math utilities (16.16 format)
// ---------------------------------------------------------------------------

/// Number of fractional bits in 16.16 fixed-point
const FP_SHIFT: u32 = 16;

/// 1.0 in 16.16 fixed-point
const FP_ONE: i32 = 1 << FP_SHIFT;

/// Convert integer to 16.16 fixed-point
#[inline]
fn fp_from_int(v: i32) -> i32 {
    v << FP_SHIFT as i32
}

/// Convert 16.16 fixed-point to integer (truncating)
#[inline]
fn fp_to_int(v: i32) -> i32 {
    v >> FP_SHIFT as i32
}

/// Multiply two 16.16 fixed-point values with saturation
#[inline]
fn fp_mul(a: i32, b: i32) -> i32 {
    let product = (a as i64).checked_mul(b as i64).unwrap_or(i64::MAX);
    let shifted = product >> FP_SHIFT;
    shifted.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Divide two 16.16 fixed-point values
#[inline]
fn fp_div(a: i32, b: i32) -> i32 {
    if b == 0 {
        return if a >= 0 { i32::MAX } else { i32::MIN };
    }
    let numerator = (a as i64) << FP_SHIFT;
    (numerator / (b as i64)).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Linear interpolation: a + t * (b - a), where t is 16.16 fixed-point in [0,
/// FP_ONE]
#[inline]
fn fp_lerp(a: i32, b: i32, t: i32) -> i32 {
    a + fp_mul(t, b - a)
}

// ===========================================================================
// 1. VirtIO GPU 3D (virgl) Protocol
// ===========================================================================

/// Virgl 3D resource types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Virgl3dResourceType {
    /// Standard 2D texture
    Texture2D = 1,
    /// 3D volume texture
    Texture3D = 2,
    /// Cube map texture
    TextureCube = 3,
    /// Vertex/index buffer
    Buffer = 4,
    /// Renderbuffer (off-screen target)
    Renderbuffer = 5,
    /// Texture array
    TextureArray = 6,
}

/// Virgl resource format (subset of Gallium pipe_format)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum VirglFormat {
    B8G8R8A8Unorm = 1,
    B8G8R8X8Unorm = 2,
    R8G8B8A8Unorm = 3,
    R8Unorm = 4,
    R16G16B16A16Float = 5,
    Z24UnormS8Uint = 6,
    Z32Float = 7,
    R32Uint = 8,
}

/// Virgl 3D resource descriptor
#[derive(Debug, Clone)]
pub struct Virgl3dResource {
    pub resource_id: u32,
    pub resource_type: Virgl3dResourceType,
    pub format: VirglFormat,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
    pub last_level: u32,
    pub nr_samples: u32,
    pub bind_flags: u32,
}

/// Virgl rendering context
#[derive(Debug)]
pub struct VirglContext {
    pub ctx_id: u32,
    pub name: [u8; 64],
    pub name_len: usize,
    pub resources: Vec<u32>,
    pub active: bool,
}

impl VirglContext {
    pub fn new(ctx_id: u32, name: &[u8]) -> Self {
        let mut name_buf = [0u8; 64];
        let len = name.len().min(64);
        name_buf[..len].copy_from_slice(&name[..len]);
        Self {
            ctx_id,
            name: name_buf,
            name_len: len,
            resources: Vec::new(),
            active: true,
        }
    }

    /// Attach a resource to this context
    pub fn attach_resource(&mut self, resource_id: u32) {
        if !self.resources.contains(&resource_id) {
            self.resources.push(resource_id);
        }
    }

    /// Detach a resource from this context
    pub fn detach_resource(&mut self, resource_id: u32) {
        self.resources.retain(|&r| r != resource_id);
    }
}

/// Virgl command types sent via command buffer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirglCommand {
    /// Create a 3D resource on the host renderer
    CreateResource3d {
        resource_id: u32,
        resource_type: Virgl3dResourceType,
        format: VirglFormat,
        width: u32,
        height: u32,
        depth: u32,
        array_size: u32,
        last_level: u32,
        nr_samples: u32,
        bind_flags: u32,
    },
    /// Transfer data between guest and host for a 3D resource
    Transfer3d {
        resource_id: u32,
        level: u32,
        x: u32,
        y: u32,
        z: u32,
        width: u32,
        height: u32,
        depth: u32,
        stride: u32,
        layer_stride: u32,
        direction: TransferDirection,
    },
    /// Create a new rendering context
    CtxCreate { ctx_id: u32, name_len: u32 },
    /// Destroy a rendering context
    CtxDestroy { ctx_id: u32 },
    /// Submit a command buffer for execution
    SubmitCommandBuffer { ctx_id: u32, data_len: u32 },
    /// Create a fence for synchronization
    CreateFence { fence_id: u64, ctx_id: u32 },
}

/// Transfer direction for 3D resource data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    /// Guest to host
    ToHost,
    /// Host to guest
    FromHost,
}

/// Virgl fence for GPU/CPU synchronization
#[derive(Debug)]
pub struct VirglFence {
    pub fence_id: u64,
    pub ctx_id: u32,
    pub signaled: AtomicBool,
}

impl VirglFence {
    pub fn new(fence_id: u64, ctx_id: u32) -> Self {
        Self {
            fence_id,
            ctx_id,
            signaled: AtomicBool::new(false),
        }
    }

    pub fn signal(&self) {
        self.signaled.store(true, Ordering::Release);
    }

    pub fn is_signaled(&self) -> bool {
        self.signaled.load(Ordering::Acquire)
    }
}

/// VirtIO GPU 3D (virgl) driver state
pub struct VirglDriver {
    pub contexts: Vec<VirglContext>,
    pub resources: Vec<Virgl3dResource>,
    pub fences: Vec<VirglFence>,
    pub command_queue: Vec<VirglCommand>,
    next_ctx_id: u32,
    next_resource_id: u32,
    next_fence_id: u64,
}

impl VirglDriver {
    pub fn new() -> Self {
        Self {
            contexts: Vec::new(),
            resources: Vec::new(),
            fences: Vec::new(),
            command_queue: Vec::new(),
            next_ctx_id: 1,
            next_resource_id: 1,
            next_fence_id: 1,
        }
    }

    /// Create a new rendering context
    pub fn create_context(&mut self, name: &[u8]) -> u32 {
        let ctx_id = self.next_ctx_id;
        self.next_ctx_id += 1;
        let ctx = VirglContext::new(ctx_id, name);
        self.command_queue.push(VirglCommand::CtxCreate {
            ctx_id,
            name_len: ctx.name_len as u32,
        });
        self.contexts.push(ctx);
        ctx_id
    }

    /// Destroy a rendering context
    pub fn destroy_context(&mut self, ctx_id: u32) -> bool {
        if let Some(pos) = self.contexts.iter().position(|c| c.ctx_id == ctx_id) {
            self.contexts[pos].active = false;
            self.command_queue.push(VirglCommand::CtxDestroy { ctx_id });
            self.contexts.remove(pos);
            true
        } else {
            false
        }
    }

    /// Create a 3D resource
    #[allow(clippy::too_many_arguments)]
    pub fn create_resource_3d(
        &mut self,
        resource_type: Virgl3dResourceType,
        format: VirglFormat,
        width: u32,
        height: u32,
        depth: u32,
        array_size: u32,
        last_level: u32,
        nr_samples: u32,
        bind_flags: u32,
    ) -> u32 {
        let resource_id = self.next_resource_id;
        self.next_resource_id += 1;

        let resource = Virgl3dResource {
            resource_id,
            resource_type,
            format,
            width,
            height,
            depth,
            array_size,
            last_level,
            nr_samples,
            bind_flags,
        };

        self.command_queue.push(VirglCommand::CreateResource3d {
            resource_id,
            resource_type,
            format,
            width,
            height,
            depth,
            array_size,
            last_level,
            nr_samples,
            bind_flags,
        });

        self.resources.push(resource);
        resource_id
    }

    /// Transfer data to/from a 3D resource
    #[allow(clippy::too_many_arguments)]
    pub fn transfer_3d(
        &mut self,
        resource_id: u32,
        level: u32,
        x: u32,
        y: u32,
        z: u32,
        width: u32,
        height: u32,
        depth: u32,
        stride: u32,
        layer_stride: u32,
        direction: TransferDirection,
    ) -> bool {
        if !self.resources.iter().any(|r| r.resource_id == resource_id) {
            return false;
        }

        self.command_queue.push(VirglCommand::Transfer3d {
            resource_id,
            level,
            x,
            y,
            z,
            width,
            height,
            depth,
            stride,
            layer_stride,
            direction,
        });
        true
    }

    /// Submit a command buffer for a given context
    pub fn submit_command_buffer(&mut self, ctx_id: u32, data_len: u32) -> bool {
        if !self.contexts.iter().any(|c| c.ctx_id == ctx_id && c.active) {
            return false;
        }
        self.command_queue
            .push(VirglCommand::SubmitCommandBuffer { ctx_id, data_len });
        true
    }

    /// Create a fence for synchronization
    pub fn create_fence(&mut self, ctx_id: u32) -> u64 {
        let fence_id = self.next_fence_id;
        self.next_fence_id += 1;

        self.command_queue
            .push(VirglCommand::CreateFence { fence_id, ctx_id });
        self.fences.push(VirglFence::new(fence_id, ctx_id));
        fence_id
    }

    /// Check if a fence has been signaled
    pub fn is_fence_signaled(&self, fence_id: u64) -> Option<bool> {
        self.fences
            .iter()
            .find(|f| f.fence_id == fence_id)
            .map(|f| f.is_signaled())
    }

    /// Signal a fence (called by interrupt handler or poll)
    pub fn signal_fence(&self, fence_id: u64) -> bool {
        if let Some(fence) = self.fences.iter().find(|f| f.fence_id == fence_id) {
            fence.signal();
            true
        } else {
            false
        }
    }

    /// Flush all pending commands (returns count of commands submitted)
    pub fn flush(&mut self) -> usize {
        let count = self.command_queue.len();
        self.command_queue.clear();
        count
    }

    /// Find a resource by ID
    pub fn find_resource(&self, resource_id: u32) -> Option<&Virgl3dResource> {
        self.resources.iter().find(|r| r.resource_id == resource_id)
    }

    /// Destroy a resource
    pub fn destroy_resource(&mut self, resource_id: u32) -> bool {
        // Remove from all contexts
        for ctx in &mut self.contexts {
            ctx.detach_resource(resource_id);
        }
        if let Some(pos) = self
            .resources
            .iter()
            .position(|r| r.resource_id == resource_id)
        {
            self.resources.remove(pos);
            true
        } else {
            false
        }
    }
}

impl Default for VirglDriver {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// 2. OpenGL ES 2.0 Software Rasterizer
// ===========================================================================

/// Maximum number of vertex attributes
const MAX_VERTEX_ATTRIBS: usize = 8;

/// Maximum number of shader uniforms (vec4 equivalents)
const MAX_UNIFORMS: usize = 32;

/// Maximum texture dimension
const MAX_TEXTURE_SIZE: u32 = 4096;

/// Primitive type for drawing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    Triangles,
    TriangleStrip,
    TriangleFan,
    Lines,
    LineStrip,
    Points,
}

/// Blend mode for fragment output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    None,
    Alpha,
    Additive,
    Multiply,
}

/// Depth test function
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthFunc {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

/// Texture filter mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFilter {
    Nearest,
    Bilinear,
}

/// A 4-component integer vector (used as vertex attribute or uniform in 16.16
/// FP)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vec4 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
}

impl Vec4 {
    pub const ZERO: Self = Self {
        x: 0,
        y: 0,
        z: 0,
        w: 0,
    };

    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self {
        Self { x, y, z, w }
    }

    /// Create from integer values (converts to 16.16 FP)
    pub fn from_ints(x: i32, y: i32, z: i32, w: i32) -> Self {
        Self {
            x: fp_from_int(x),
            y: fp_from_int(y),
            z: fp_from_int(z),
            w: fp_from_int(w),
        }
    }

    /// Linear interpolation between two Vec4 values
    pub fn lerp(a: &Vec4, b: &Vec4, t: i32) -> Vec4 {
        Vec4 {
            x: fp_lerp(a.x, b.x, t),
            y: fp_lerp(a.y, b.y, t),
            z: fp_lerp(a.z, b.z, t),
            w: fp_lerp(a.w, b.w, t),
        }
    }
}

/// Vertex data for software rasterizer
#[derive(Debug, Clone, Copy, Default)]
pub struct Vertex {
    /// Position (x, y, z, w) in 16.16 FP after vertex transformation
    pub position: Vec4,
    /// Color (r, g, b, a) in 16.16 FP [0, FP_ONE]
    pub color: Vec4,
    /// Texture coordinates (u, v) in 16.16 FP [0, FP_ONE]
    pub texcoord_u: i32,
    pub texcoord_v: i32,
}

/// Software texture (ARGB8888)
pub struct SoftTexture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    pub filter: TextureFilter,
}

impl SoftTexture {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width as usize).checked_mul(height as usize).unwrap_or(0);
        Self {
            width,
            height,
            pixels: vec![0u32; size],
            filter: TextureFilter::Nearest,
        }
    }

    /// Sample the texture at (u, v) in 16.16 fixed-point [0, FP_ONE]
    pub fn sample(&self, u: i32, v: i32) -> u32 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        match self.filter {
            TextureFilter::Nearest => self.sample_nearest(u, v),
            TextureFilter::Bilinear => self.sample_bilinear(u, v),
        }
    }

    fn sample_nearest(&self, u: i32, v: i32) -> u32 {
        // Wrap coordinates to [0, FP_ONE)
        let u_wrapped = ((u % FP_ONE) + FP_ONE) % FP_ONE;
        let v_wrapped = ((v % FP_ONE) + FP_ONE) % FP_ONE;

        let x = (fp_mul(u_wrapped, fp_from_int(self.width as i32)) >> FP_SHIFT) as u32;
        let y = (fp_mul(v_wrapped, fp_from_int(self.height as i32)) >> FP_SHIFT) as u32;
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));

        let idx = (y as usize)
            .checked_mul(self.width as usize)
            .and_then(|v| v.checked_add(x as usize))
            .unwrap_or(0);

        self.pixels.get(idx).copied().unwrap_or(0)
    }

    fn sample_bilinear(&self, u: i32, v: i32) -> u32 {
        if self.width < 2 || self.height < 2 {
            return self.sample_nearest(u, v);
        }

        let u_wrapped = ((u % FP_ONE) + FP_ONE) % FP_ONE;
        let v_wrapped = ((v % FP_ONE) + FP_ONE) % FP_ONE;

        // Scale to texture pixel coordinates in 16.16
        let tx = fp_mul(u_wrapped, fp_from_int(self.width as i32)) - (FP_ONE / 2);
        let ty = fp_mul(v_wrapped, fp_from_int(self.height as i32)) - (FP_ONE / 2);

        let x0 = (fp_to_int(tx).max(0) as u32).min(self.width - 2);
        let y0 = (fp_to_int(ty).max(0) as u32).min(self.height - 2);
        let x1 = x0 + 1;
        let y1 = y0 + 1;

        // Fractional parts
        let fx = tx & (FP_ONE - 1); // lower 16 bits
        let fy = ty & (FP_ONE - 1);

        let w = self.width as usize;
        let p00 = self
            .pixels
            .get(y0 as usize * w + x0 as usize)
            .copied()
            .unwrap_or(0);
        let p10 = self
            .pixels
            .get(y0 as usize * w + x1 as usize)
            .copied()
            .unwrap_or(0);
        let p01 = self
            .pixels
            .get(y1 as usize * w + x0 as usize)
            .copied()
            .unwrap_or(0);
        let p11 = self
            .pixels
            .get(y1 as usize * w + x1 as usize)
            .copied()
            .unwrap_or(0);

        bilinear_pixel(p00, p10, p01, p11, fx, fy)
    }
}

/// Bilinear interpolation of ARGB pixel values using 16.16 FP weights
fn bilinear_pixel(p00: u32, p10: u32, p01: u32, p11: u32, fx: i32, fy: i32) -> u32 {
    let inv_fx = FP_ONE - fx;
    let inv_fy = FP_ONE - fy;

    let mut result = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let c00 = ((p00 >> shift) & 0xFF) as i32;
        let c10 = ((p10 >> shift) & 0xFF) as i32;
        let c01 = ((p01 >> shift) & 0xFF) as i32;
        let c11 = ((p11 >> shift) & 0xFF) as i32;

        // top = c00 * inv_fx + c10 * fx (all in 16.16)
        let top = fp_mul(c00 << FP_SHIFT, inv_fx) + fp_mul(c10 << FP_SHIFT, fx);
        let bot = fp_mul(c01 << FP_SHIFT, inv_fx) + fp_mul(c11 << FP_SHIFT, fx);
        let val = fp_mul(top, inv_fy) + fp_mul(bot, fy);
        let byte = fp_to_int(val).clamp(0, 255) as u32;
        result |= byte << shift;
    }
    result
}

/// Scissor rectangle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScissorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Viewport transform parameters
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Near depth in 16.16 FP
    pub near: i32,
    /// Far depth in 16.16 FP
    pub far: i32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
            near: 0,
            far: FP_ONE,
        }
    }
}

/// Shader state (uniforms + enabled attributes)
#[derive(Debug, Clone)]
pub struct ShaderState {
    /// Model-view-projection matrix rows (4 x Vec4, each component 16.16 FP)
    pub mvp: [Vec4; 4],
    /// Uniform vec4 values
    pub uniforms: Vec<Vec4>,
    /// Vertex color attribute enabled
    pub color_enabled: bool,
    /// Texture coordinate attribute enabled
    pub texcoord_enabled: bool,
}

impl Default for ShaderState {
    fn default() -> Self {
        Self {
            mvp: [
                Vec4::new(FP_ONE, 0, 0, 0),
                Vec4::new(0, FP_ONE, 0, 0),
                Vec4::new(0, 0, FP_ONE, 0),
                Vec4::new(0, 0, 0, FP_ONE),
            ],
            uniforms: Vec::new(),
            color_enabled: true,
            texcoord_enabled: false,
        }
    }
}

/// Software rasterizer state
pub struct SoftwareRasterizer {
    /// Output color buffer (ARGB8888)
    pub color_buffer: Vec<u32>,
    /// Depth buffer (16.16 FP Z values)
    pub depth_buffer: Vec<i32>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Viewport
    pub viewport: Viewport,
    /// Scissor rectangle (None = disabled)
    pub scissor: Option<ScissorRect>,
    /// Depth test function
    pub depth_func: DepthFunc,
    /// Depth write enabled
    pub depth_write: bool,
    /// Depth test enabled
    pub depth_test: bool,
    /// Blend mode
    pub blend_mode: BlendMode,
    /// Bound texture (index 0)
    pub texture: Option<SoftTexture>,
    /// Shader state
    pub shader: ShaderState,
    /// Clear color (ARGB8888)
    pub clear_color: u32,
    /// Clear depth (16.16 FP)
    pub clear_depth: i32,
}

impl SoftwareRasterizer {
    pub fn new(width: u32, height: u32) -> Self {
        let pixel_count = (width as usize).checked_mul(height as usize).unwrap_or(0);
        Self {
            color_buffer: vec![0u32; pixel_count],
            depth_buffer: vec![i32::MAX; pixel_count],
            width,
            height,
            viewport: Viewport {
                x: 0,
                y: 0,
                width,
                height,
                near: 0,
                far: FP_ONE,
            },
            scissor: None,
            depth_func: DepthFunc::Less,
            depth_write: true,
            depth_test: true,
            blend_mode: BlendMode::None,
            texture: None,
            shader: ShaderState::default(),
            clear_color: 0xFF000000,
            clear_depth: i32::MAX,
        }
    }

    /// Clear color buffer
    pub fn clear_color_buffer(&mut self) {
        for px in &mut self.color_buffer {
            *px = self.clear_color;
        }
    }

    /// Clear depth buffer
    pub fn clear_depth_buffer(&mut self) {
        for d in &mut self.depth_buffer {
            *d = self.clear_depth;
        }
    }

    /// Apply vertex shader (MVP transform) to a vertex position
    fn transform_vertex(&self, pos: &Vec4) -> Vec4 {
        let m = &self.shader.mvp;
        Vec4 {
            x: fp_mul(m[0].x, pos.x)
                + fp_mul(m[0].y, pos.y)
                + fp_mul(m[0].z, pos.z)
                + fp_mul(m[0].w, pos.w),
            y: fp_mul(m[1].x, pos.x)
                + fp_mul(m[1].y, pos.y)
                + fp_mul(m[1].z, pos.z)
                + fp_mul(m[1].w, pos.w),
            z: fp_mul(m[2].x, pos.x)
                + fp_mul(m[2].y, pos.y)
                + fp_mul(m[2].z, pos.z)
                + fp_mul(m[2].w, pos.w),
            w: fp_mul(m[3].x, pos.x)
                + fp_mul(m[3].y, pos.y)
                + fp_mul(m[3].z, pos.z)
                + fp_mul(m[3].w, pos.w),
        }
    }

    /// Viewport transform: clip-space [-1,1] -> screen coordinates
    fn viewport_transform(&self, ndc: &Vec4) -> Vec4 {
        let vp = &self.viewport;
        let half_w = fp_from_int(vp.width as i32) / 2;
        let half_h = fp_from_int(vp.height as i32) / 2;

        Vec4 {
            x: fp_mul(ndc.x, half_w) + fp_from_int(vp.x) + half_w,
            y: fp_mul(ndc.y, half_h) + fp_from_int(vp.y) + half_h,
            z: fp_lerp(vp.near, vp.far, (ndc.z + FP_ONE) / 2),
            w: ndc.w,
        }
    }

    /// Depth test comparison
    fn depth_test_pass(&self, new_z: i32, old_z: i32) -> bool {
        match self.depth_func {
            DepthFunc::Never => false,
            DepthFunc::Less => new_z < old_z,
            DepthFunc::Equal => new_z == old_z,
            DepthFunc::LessEqual => new_z <= old_z,
            DepthFunc::Greater => new_z > old_z,
            DepthFunc::NotEqual => new_z != old_z,
            DepthFunc::GreaterEqual => new_z >= old_z,
            DepthFunc::Always => true,
        }
    }

    /// Scissor test: returns true if pixel (x, y) passes
    fn scissor_test(&self, x: i32, y: i32) -> bool {
        match &self.scissor {
            None => true,
            Some(s) => {
                x >= s.x && x < s.x + s.width as i32 && y >= s.y && y < s.y + s.height as i32
            }
        }
    }

    /// Alpha blend: blend src over dst (integer math)
    fn alpha_blend(&self, src: u32, dst: u32) -> u32 {
        match self.blend_mode {
            BlendMode::None => src,
            BlendMode::Alpha => {
                let sa = (src >> 24) & 0xFF;
                if sa == 255 {
                    return src;
                }
                if sa == 0 {
                    return dst;
                }
                let inv_sa = 255 - sa;
                let mut result = 0u32;
                for shift in [0u32, 8, 16] {
                    let sc = (src >> shift) & 0xFF;
                    let dc = (dst >> shift) & 0xFF;
                    // (sc * sa + dc * inv_sa + 127) / 255
                    let blended = (sc * sa + dc * inv_sa + 127) / 255;
                    result |= blended.min(255) << shift;
                }
                // Output alpha = sa + da * inv_sa / 255
                let da = (dst >> 24) & 0xFF;
                let out_a = (sa + (da * inv_sa + 127) / 255).min(255);
                result |= out_a << 24;
                result
            }
            BlendMode::Additive => {
                let mut result = 0u32;
                for shift in [0u32, 8, 16, 24] {
                    let sc = (src >> shift) & 0xFF;
                    let dc = (dst >> shift) & 0xFF;
                    result |= (sc + dc).min(255) << shift;
                }
                result
            }
            BlendMode::Multiply => {
                let mut result = 0u32;
                for shift in [0u32, 8, 16] {
                    let sc = (src >> shift) & 0xFF;
                    let dc = (dst >> shift) & 0xFF;
                    result |= ((sc * dc + 127) / 255) << shift;
                }
                // Keep max alpha
                let sa = (src >> 24) & 0xFF;
                let da = (dst >> 24) & 0xFF;
                result |= sa.max(da) << 24;
                result
            }
        }
    }

    /// Write a fragment (pixel) with depth test, scissor test, and blending
    fn write_fragment(&mut self, x: i32, y: i32, z: i32, color: u32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        if !self.scissor_test(x, y) {
            return;
        }

        let idx = y as usize * self.width as usize + x as usize;
        if idx >= self.color_buffer.len() {
            return;
        }

        if self.depth_test {
            if !self.depth_test_pass(z, self.depth_buffer[idx]) {
                return;
            }
            if self.depth_write {
                self.depth_buffer[idx] = z;
            }
        }

        let final_color = if self.blend_mode != BlendMode::None {
            self.alpha_blend(color, self.color_buffer[idx])
        } else {
            color
        };
        self.color_buffer[idx] = final_color;
    }

    /// Convert Vec4 color (16.16 FP, 0..FP_ONE) to ARGB8888
    fn vec4_to_argb(color: &Vec4) -> u32 {
        let r = (fp_to_int(color.x).clamp(0, 255) as u32) & 0xFF;
        let g = (fp_to_int(color.y).clamp(0, 255) as u32) & 0xFF;
        let b = (fp_to_int(color.z).clamp(0, 255) as u32) & 0xFF;
        let a = (fp_to_int(color.w).clamp(0, 255) as u32) & 0xFF;
        (a << 24) | (r << 16) | (g << 8) | b
    }

    /// Rasterize a single triangle using edge functions (fixed-point)
    pub fn rasterize_triangle(&mut self, v0: &Vertex, v1: &Vertex, v2: &Vertex) {
        // Transform vertices through MVP
        let pos0 = self.transform_vertex(&v0.position);
        let pos1 = self.transform_vertex(&v1.position);
        let pos2 = self.transform_vertex(&v2.position);

        // Perspective divide (skip if w is zero or identity)
        let ndc0 = if pos0.w != 0 && pos0.w != FP_ONE {
            Vec4::new(
                fp_div(pos0.x, pos0.w),
                fp_div(pos0.y, pos0.w),
                fp_div(pos0.z, pos0.w),
                FP_ONE,
            )
        } else {
            pos0
        };
        let ndc1 = if pos1.w != 0 && pos1.w != FP_ONE {
            Vec4::new(
                fp_div(pos1.x, pos1.w),
                fp_div(pos1.y, pos1.w),
                fp_div(pos1.z, pos1.w),
                FP_ONE,
            )
        } else {
            pos1
        };
        let ndc2 = if pos2.w != 0 && pos2.w != FP_ONE {
            Vec4::new(
                fp_div(pos2.x, pos2.w),
                fp_div(pos2.y, pos2.w),
                fp_div(pos2.z, pos2.w),
                FP_ONE,
            )
        } else {
            pos2
        };

        // Viewport transform
        let s0 = self.viewport_transform(&ndc0);
        let s1 = self.viewport_transform(&ndc1);
        let s2 = self.viewport_transform(&ndc2);

        // Screen-space integer coordinates (pixel centers)
        let x0 = fp_to_int(s0.x);
        let y0 = fp_to_int(s0.y);
        let x1 = fp_to_int(s1.x);
        let y1 = fp_to_int(s1.y);
        let x2 = fp_to_int(s2.x);
        let y2 = fp_to_int(s2.y);

        // Bounding box
        let min_x = x0.min(x1).min(x2).max(0);
        let max_x = x0.max(x1).max(x2).min(self.width as i32 - 1);
        let min_y = y0.min(y1).min(y2).max(0);
        let max_y = y0.max(y1).max(y2).min(self.height as i32 - 1);

        if min_x > max_x || min_y > max_y {
            return;
        }

        // Edge function: area of parallelogram formed by edges
        // Twice the signed area of the triangle
        let area = (x1 - x0) as i64 * (y2 - y0) as i64 - (x2 - x0) as i64 * (y1 - y0) as i64;
        if area == 0 {
            return; // Degenerate triangle
        }

        // Scan bounding box
        let mut py = min_y;
        while py <= max_y {
            let mut px = min_x;
            while px <= max_x {
                // Edge functions (barycentric coordinates * 2*area)
                let w0 = (px - x1) as i64 * (y2 - y1) as i64 - (py - y1) as i64 * (x2 - x1) as i64;
                let w1 = (px - x2) as i64 * (y0 - y2) as i64 - (py - y2) as i64 * (x0 - x2) as i64;
                let w2 = (px - x0) as i64 * (y1 - y0) as i64 - (py - y0) as i64 * (x1 - x0) as i64;

                // Top-left rule check (positive area = CCW)
                let inside = if area > 0 {
                    w0 >= 0 && w1 >= 0 && w2 >= 0
                } else {
                    w0 <= 0 && w1 <= 0 && w2 <= 0
                };

                if inside {
                    // Barycentric interpolation weights in 16.16 FP
                    let bary0 = ((w0 << FP_SHIFT) / area) as i32;
                    let bary1 = ((w1 << FP_SHIFT) / area) as i32;
                    let bary2 = FP_ONE - bary0 - bary1;

                    // Interpolate depth
                    let z = fp_mul(bary0, s0.z) + fp_mul(bary1, s1.z) + fp_mul(bary2, s2.z);

                    // Interpolate color
                    let color = Vec4 {
                        x: fp_mul(bary0, v0.color.x)
                            + fp_mul(bary1, v1.color.x)
                            + fp_mul(bary2, v2.color.x),
                        y: fp_mul(bary0, v0.color.y)
                            + fp_mul(bary1, v1.color.y)
                            + fp_mul(bary2, v2.color.y),
                        z: fp_mul(bary0, v0.color.z)
                            + fp_mul(bary1, v1.color.z)
                            + fp_mul(bary2, v2.color.z),
                        w: fp_mul(bary0, v0.color.w)
                            + fp_mul(bary1, v1.color.w)
                            + fp_mul(bary2, v2.color.w),
                    };

                    let mut frag_color = Self::vec4_to_argb(&color);

                    // Texture sampling
                    if self.shader.texcoord_enabled {
                        if let Some(ref tex) = self.texture {
                            let u = fp_mul(bary0, v0.texcoord_u)
                                + fp_mul(bary1, v1.texcoord_u)
                                + fp_mul(bary2, v2.texcoord_u);
                            let v = fp_mul(bary0, v0.texcoord_v)
                                + fp_mul(bary1, v1.texcoord_v)
                                + fp_mul(bary2, v2.texcoord_v);
                            frag_color = tex.sample(u, v);
                        }
                    }

                    self.write_fragment(px, py, z, frag_color);
                }
                px += 1;
            }
            py += 1;
        }
    }

    /// Draw an array of triangles
    pub fn draw_triangles(&mut self, vertices: &[Vertex]) {
        let count = vertices.len() / 3;
        let mut i = 0;
        while i < count {
            let base = i * 3;
            self.rasterize_triangle(&vertices[base], &vertices[base + 1], &vertices[base + 2]);
            i += 1;
        }
    }

    /// Draw indexed triangles
    pub fn draw_indexed_triangles(&mut self, vertices: &[Vertex], indices: &[u32]) {
        let count = indices.len() / 3;
        let mut i = 0;
        while i < count {
            let i0 = indices[i * 3] as usize;
            let i1 = indices[i * 3 + 1] as usize;
            let i2 = indices[i * 3 + 2] as usize;
            if i0 < vertices.len() && i1 < vertices.len() && i2 < vertices.len() {
                self.rasterize_triangle(&vertices[i0], &vertices[i1], &vertices[i2]);
            }
            i += 1;
        }
    }
}

// ===========================================================================
// 3. GEM/TTM Buffer Object Management
// ===========================================================================

/// Memory domain for buffer objects
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryDomain {
    /// CPU-accessible system memory
    Cpu,
    /// GPU-accessible VRAM
    Gpu,
    /// GPU-accessible through GTT (Graphics Translation Table)
    Gtt,
    /// Shared (CPU and GPU coherent)
    Shared,
}

/// Cache coherency mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Normal cached (WB)
    Cached,
    /// Write-combined (for streaming writes)
    WriteCombine,
    /// Uncached (for MMIO-like access)
    Uncached,
}

/// GEM buffer object
pub struct GemBufferObject {
    /// Unique handle
    pub handle: u32,
    /// Size in bytes
    pub size: usize,
    /// Current memory domain
    pub domain: MemoryDomain,
    /// Cache mode
    pub cache_mode: CacheMode,
    /// Reference count
    ref_count: AtomicU32,
    /// Whether the buffer is pinned (cannot be evicted)
    pub pinned: bool,
    /// Backing storage
    pub data: Vec<u8>,
    /// Name (for debugging)
    pub name: [u8; 32],
    pub name_len: usize,
}

impl GemBufferObject {
    pub fn new(handle: u32, size: usize) -> Self {
        Self {
            handle,
            size,
            domain: MemoryDomain::Cpu,
            cache_mode: CacheMode::Cached,
            ref_count: AtomicU32::new(1),
            pinned: false,
            data: vec![0u8; size],
            name: [0u8; 32],
            name_len: 0,
        }
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let len = name.len().min(32);
        self.name[..len].copy_from_slice(&name[..len]);
        self.name_len = len;
    }

    pub fn ref_count(&self) -> u32 {
        self.ref_count.load(Ordering::Acquire)
    }

    pub fn add_ref(&self) -> u32 {
        self.ref_count.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn release(&self) -> u32 {
        self.ref_count.fetch_sub(1, Ordering::AcqRel) - 1
    }
}

/// GEM buffer manager
pub struct GemManager {
    pub buffers: Vec<GemBufferObject>,
    next_handle: u32,
    /// Total allocated bytes
    pub total_allocated: usize,
    /// Maximum allocation limit
    pub max_allocation: usize,
}

impl GemManager {
    pub fn new(max_allocation: usize) -> Self {
        Self {
            buffers: Vec::new(),
            next_handle: 1,
            total_allocated: 0,
            max_allocation,
        }
    }

    /// Create a new buffer object
    pub fn create_buffer(&mut self, size: usize) -> Option<u32> {
        if size == 0 || self.total_allocated + size > self.max_allocation {
            return None;
        }

        let handle = self.next_handle;
        self.next_handle += 1;
        let bo = GemBufferObject::new(handle, size);
        self.total_allocated += size;
        self.buffers.push(bo);
        Some(handle)
    }

    /// Destroy a buffer object (only if ref_count reaches 0)
    pub fn destroy_buffer(&mut self, handle: u32) -> bool {
        if let Some(pos) = self.buffers.iter().position(|b| b.handle == handle) {
            let remaining = self.buffers[pos].release();
            if remaining == 0 {
                let bo = self.buffers.remove(pos);
                self.total_allocated = self.total_allocated.saturating_sub(bo.size);
                return true;
            }
        }
        false
    }

    /// Find a buffer by handle
    pub fn find_buffer(&self, handle: u32) -> Option<&GemBufferObject> {
        self.buffers.iter().find(|b| b.handle == handle)
    }

    /// Find a buffer by handle (mutable)
    pub fn find_buffer_mut(&mut self, handle: u32) -> Option<&mut GemBufferObject> {
        self.buffers.iter_mut().find(|b| b.handle == handle)
    }

    /// Pin a buffer (prevent eviction)
    pub fn pin_buffer(&mut self, handle: u32) -> bool {
        if let Some(bo) = self.find_buffer_mut(handle) {
            bo.pinned = true;
            true
        } else {
            false
        }
    }

    /// Unpin a buffer (allow eviction)
    pub fn unpin_buffer(&mut self, handle: u32) -> bool {
        if let Some(bo) = self.find_buffer_mut(handle) {
            bo.pinned = false;
            true
        } else {
            false
        }
    }

    /// Move buffer to a different memory domain
    pub fn set_domain(&mut self, handle: u32, domain: MemoryDomain) -> bool {
        if let Some(bo) = self.find_buffer_mut(handle) {
            bo.domain = domain;
            true
        } else {
            false
        }
    }

    /// Set cache coherency mode for a buffer
    pub fn set_cache_mode(&mut self, handle: u32, mode: CacheMode) -> bool {
        if let Some(bo) = self.find_buffer_mut(handle) {
            bo.cache_mode = mode;
            true
        } else {
            false
        }
    }

    /// Add a reference to a buffer
    pub fn add_ref(&self, handle: u32) -> bool {
        if let Some(bo) = self.buffers.iter().find(|b| b.handle == handle) {
            bo.add_ref();
            true
        } else {
            false
        }
    }

    /// Get number of buffers
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }
}

impl Default for GemManager {
    fn default() -> Self {
        Self::new(256 * 1024 * 1024) // 256 MB default limit
    }
}

// ===========================================================================
// 4. DRM Kernel Mode Setting (KMS)
// ===========================================================================

/// Connector type (physical display output)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorType {
    Hdmi,
    DisplayPort,
    Vga,
    Edp,
    Dvi,
    Lvds,
    Virtual,
}

/// Connector status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorStatus {
    Connected,
    Disconnected,
    Unknown,
}

/// Pixel format for framebuffers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888,
    Rgb565,
    Xbgr8888,
    Abgr8888,
}

impl PixelFormat {
    /// Bytes per pixel
    pub fn bpp(&self) -> u32 {
        match self {
            PixelFormat::Xrgb8888
            | PixelFormat::Argb8888
            | PixelFormat::Xbgr8888
            | PixelFormat::Abgr8888 => 4,
            PixelFormat::Rgb565 => 2,
        }
    }
}

/// Display mode (resolution + timing)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMode {
    /// Horizontal active pixels
    pub hdisplay: u32,
    /// Vertical active lines
    pub vdisplay: u32,
    /// Pixel clock in kHz
    pub clock_khz: u32,
    /// Horizontal sync start
    pub hsync_start: u32,
    /// Horizontal sync end
    pub hsync_end: u32,
    /// Horizontal total
    pub htotal: u32,
    /// Vertical sync start
    pub vsync_start: u32,
    /// Vertical sync end
    pub vsync_end: u32,
    /// Vertical total
    pub vtotal: u32,
    /// Refresh rate in mHz (millihertz) for integer precision
    pub vrefresh_mhz: u32,
}

impl DisplayMode {
    /// Common 1920x1080 @ 60 Hz mode
    pub fn mode_1080p60() -> Self {
        Self {
            hdisplay: 1920,
            vdisplay: 1080,
            clock_khz: 148500,
            hsync_start: 2008,
            hsync_end: 2052,
            htotal: 2200,
            vsync_start: 1084,
            vsync_end: 1089,
            vtotal: 1125,
            vrefresh_mhz: 60000,
        }
    }

    /// Common 1280x720 @ 60 Hz mode
    pub fn mode_720p60() -> Self {
        Self {
            hdisplay: 1280,
            vdisplay: 720,
            clock_khz: 74250,
            hsync_start: 1390,
            hsync_end: 1430,
            htotal: 1650,
            vsync_start: 725,
            vsync_end: 730,
            vtotal: 750,
            vrefresh_mhz: 60000,
        }
    }

    /// Common 1280x800 @ 60 Hz mode (VeridianOS default)
    pub fn mode_wxga60() -> Self {
        Self {
            hdisplay: 1280,
            vdisplay: 800,
            clock_khz: 83500,
            hsync_start: 1352,
            hsync_end: 1480,
            htotal: 1680,
            vsync_start: 803,
            vsync_end: 809,
            vtotal: 831,
            vrefresh_mhz: 60000,
        }
    }

    /// Validate mode timing parameters
    pub fn validate(&self) -> bool {
        // Basic sanity checks
        if self.hdisplay == 0 || self.vdisplay == 0 {
            return false;
        }
        if self.htotal == 0 || self.vtotal == 0 {
            return false;
        }
        // hsync must be within htotal
        if self.hsync_start > self.hsync_end || self.hsync_end > self.htotal {
            return false;
        }
        // vsync must be within vtotal
        if self.vsync_start > self.vsync_end || self.vsync_end > self.vtotal {
            return false;
        }
        // Active area must fit in total
        if self.hdisplay > self.htotal || self.vdisplay > self.vtotal {
            return false;
        }
        // Clock must be positive
        if self.clock_khz == 0 {
            return false;
        }
        // Max practical limits
        if self.hdisplay > 7680 || self.vdisplay > 4320 {
            return false;
        }
        true
    }

    /// Calculate actual refresh rate in millihertz from timing parameters
    pub fn calculated_refresh_mhz(&self) -> u32 {
        if self.htotal == 0 || self.vtotal == 0 {
            return 0;
        }
        // refresh = clock / (htotal * vtotal)
        // clock is in kHz, we want mHz output
        // mHz = (clock_khz * 1_000_000) / (htotal * vtotal)
        let total_pixels = (self.htotal as u64)
            .checked_mul(self.vtotal as u64)
            .unwrap_or(1);
        let numerator = (self.clock_khz as u64).checked_mul(1_000_000).unwrap_or(0);
        (numerator / total_pixels) as u32
    }
}

/// DRM framebuffer object
#[derive(Debug, Clone)]
pub struct DrmFramebuffer {
    pub fb_id: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Pitch (bytes per row)
    pub pitch: u32,
    /// Byte offset into the buffer
    pub offset: u32,
    /// Pixel format
    pub format: PixelFormat,
    /// GEM handle for backing buffer
    pub gem_handle: u32,
}

impl DrmFramebuffer {
    pub fn new(fb_id: u32, width: u32, height: u32, format: PixelFormat, gem_handle: u32) -> Self {
        let pitch = width * format.bpp();
        Self {
            fb_id,
            width,
            height,
            pitch,
            offset: 0,
            format,
            gem_handle,
        }
    }

    /// Calculate total size in bytes
    pub fn size_bytes(&self) -> usize {
        (self.pitch as usize)
            .checked_mul(self.height as usize)
            .unwrap_or(0)
    }
}

/// Encoder type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderType {
    None,
    Dac,
    Tmds,
    Lvds,
    DpMst,
    Virtual,
}

/// DRM encoder (links CRTC to connector)
#[derive(Debug, Clone)]
pub struct DrmEncoder {
    pub encoder_id: u32,
    pub encoder_type: EncoderType,
    /// Which CRTC this encoder is bound to (None if unbound)
    pub crtc_id: Option<u32>,
    /// Bitmask of possible CRTCs
    pub possible_crtcs: u32,
}

/// DRM connector
#[derive(Debug, Clone)]
pub struct DrmConnector {
    pub connector_id: u32,
    pub connector_type: ConnectorType,
    pub status: ConnectorStatus,
    /// Bound encoder ID
    pub encoder_id: Option<u32>,
    /// Supported display modes
    pub modes: Vec<DisplayMode>,
}

impl DrmConnector {
    pub fn new(connector_id: u32, connector_type: ConnectorType) -> Self {
        Self {
            connector_id,
            connector_type,
            status: ConnectorStatus::Disconnected,
            encoder_id: None,
            modes: Vec::new(),
        }
    }
}

/// CRTC (CRT Controller) -- drives a scanout engine
#[derive(Debug, Clone)]
pub struct DrmCrtc {
    pub crtc_id: u32,
    /// Currently active framebuffer
    pub fb_id: Option<u32>,
    /// Current display mode
    pub mode: Option<DisplayMode>,
    /// Whether this CRTC is active
    pub active: bool,
    /// Gamma table size
    pub gamma_size: u32,
}

impl DrmCrtc {
    pub fn new(crtc_id: u32) -> Self {
        Self {
            crtc_id,
            fb_id: None,
            mode: None,
            active: false,
            gamma_size: 256,
        }
    }
}

/// Atomic mode-setting commit request
#[derive(Debug, Clone)]
pub struct AtomicCommit {
    /// CRTC ID to configure
    pub crtc_id: u32,
    /// Connector ID to use
    pub connector_id: u32,
    /// Framebuffer to scanout
    pub fb_id: u32,
    /// Display mode to set
    pub mode: DisplayMode,
    /// Whether this is a test-only commit (validate without applying)
    pub test_only: bool,
}

/// DRM KMS state manager
pub struct KmsManager {
    pub crtcs: Vec<DrmCrtc>,
    pub connectors: Vec<DrmConnector>,
    pub encoders: Vec<DrmEncoder>,
    pub framebuffers: Vec<DrmFramebuffer>,
    next_crtc_id: u32,
    next_connector_id: u32,
    next_encoder_id: u32,
    next_fb_id: u32,
}

impl KmsManager {
    pub fn new() -> Self {
        Self {
            crtcs: Vec::new(),
            connectors: Vec::new(),
            encoders: Vec::new(),
            framebuffers: Vec::new(),
            next_crtc_id: 1,
            next_connector_id: 1,
            next_encoder_id: 1,
            next_fb_id: 1,
        }
    }

    /// Add a CRTC
    pub fn add_crtc(&mut self) -> u32 {
        let id = self.next_crtc_id;
        self.next_crtc_id += 1;
        self.crtcs.push(DrmCrtc::new(id));
        id
    }

    /// Add a connector
    pub fn add_connector(&mut self, connector_type: ConnectorType) -> u32 {
        let id = self.next_connector_id;
        self.next_connector_id += 1;
        self.connectors.push(DrmConnector::new(id, connector_type));
        id
    }

    /// Add an encoder
    pub fn add_encoder(&mut self, encoder_type: EncoderType, possible_crtcs: u32) -> u32 {
        let id = self.next_encoder_id;
        self.next_encoder_id += 1;
        self.encoders.push(DrmEncoder {
            encoder_id: id,
            encoder_type,
            crtc_id: None,
            possible_crtcs,
        });
        id
    }

    /// Create a framebuffer object
    pub fn create_framebuffer(
        &mut self,
        width: u32,
        height: u32,
        format: PixelFormat,
        gem_handle: u32,
    ) -> u32 {
        let id = self.next_fb_id;
        self.next_fb_id += 1;
        self.framebuffers
            .push(DrmFramebuffer::new(id, width, height, format, gem_handle));
        id
    }

    /// Destroy a framebuffer object
    pub fn destroy_framebuffer(&mut self, fb_id: u32) -> bool {
        if let Some(pos) = self.framebuffers.iter().position(|f| f.fb_id == fb_id) {
            self.framebuffers.remove(pos);
            true
        } else {
            false
        }
    }

    /// Bind an encoder to a CRTC
    pub fn bind_encoder(&mut self, encoder_id: u32, crtc_id: u32) -> bool {
        // Check CRTC exists
        if !self.crtcs.iter().any(|c| c.crtc_id == crtc_id) {
            return false;
        }
        if let Some(enc) = self
            .encoders
            .iter_mut()
            .find(|e| e.encoder_id == encoder_id)
        {
            // Check if CRTC is in possible_crtcs bitmask
            let crtc_idx = self
                .crtcs
                .iter()
                .position(|c| c.crtc_id == crtc_id)
                .unwrap_or(0);
            if (enc.possible_crtcs >> crtc_idx) & 1 == 0 {
                return false;
            }
            enc.crtc_id = Some(crtc_id);
            true
        } else {
            false
        }
    }

    /// Connect a connector to an encoder
    pub fn connect_encoder(&mut self, connector_id: u32, encoder_id: u32) -> bool {
        if !self.encoders.iter().any(|e| e.encoder_id == encoder_id) {
            return false;
        }
        if let Some(conn) = self
            .connectors
            .iter_mut()
            .find(|c| c.connector_id == connector_id)
        {
            conn.encoder_id = Some(encoder_id);
            true
        } else {
            false
        }
    }

    /// Set connector status and available modes
    pub fn set_connector_status(
        &mut self,
        connector_id: u32,
        status: ConnectorStatus,
        modes: Vec<DisplayMode>,
    ) -> bool {
        if let Some(conn) = self
            .connectors
            .iter_mut()
            .find(|c| c.connector_id == connector_id)
        {
            conn.status = status;
            conn.modes = modes;
            true
        } else {
            false
        }
    }

    /// Perform an atomic mode-setting commit
    pub fn atomic_commit(&mut self, commit: &AtomicCommit) -> bool {
        // Validate mode
        if !commit.mode.validate() {
            return false;
        }

        // Check that framebuffer exists
        if !self.framebuffers.iter().any(|f| f.fb_id == commit.fb_id) {
            return false;
        }

        // Check that connector exists
        if !self
            .connectors
            .iter()
            .any(|c| c.connector_id == commit.connector_id)
        {
            return false;
        }

        if commit.test_only {
            return true; // Validation passed
        }

        // Apply: set CRTC mode and framebuffer
        if let Some(crtc) = self.crtcs.iter_mut().find(|c| c.crtc_id == commit.crtc_id) {
            crtc.fb_id = Some(commit.fb_id);
            crtc.mode = Some(commit.mode);
            crtc.active = true;
            true
        } else {
            false
        }
    }

    /// Find a framebuffer by ID
    pub fn find_framebuffer(&self, fb_id: u32) -> Option<&DrmFramebuffer> {
        self.framebuffers.iter().find(|f| f.fb_id == fb_id)
    }

    /// Find a CRTC by ID
    pub fn find_crtc(&self, crtc_id: u32) -> Option<&DrmCrtc> {
        self.crtcs.iter().find(|c| c.crtc_id == crtc_id)
    }
}

impl Default for KmsManager {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// 5. Vsync / Page Flip
// ===========================================================================

/// Page flip state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipState {
    /// No flip pending
    Idle,
    /// Flip requested, waiting for vblank
    Pending,
    /// Flip completed
    Completed,
}

/// Vblank event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VblankEvent {
    /// Vblank counter
    pub sequence: u64,
    /// Timestamp in nanoseconds (monotonic)
    pub timestamp_ns: u64,
    /// CRTC that generated this event
    pub crtc_id: u32,
}

/// Page flip request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageFlipRequest {
    /// CRTC to flip
    pub crtc_id: u32,
    /// New framebuffer to display
    pub fb_id: u32,
    /// User data for completion callback
    pub user_data: u64,
}

/// Double-buffered page flip manager
pub struct PageFlipManager {
    /// Front buffer (currently displayed) per CRTC
    pub front_buffers: Vec<(u32, u32)>, // (crtc_id, fb_id)
    /// Back buffer (being rendered to) per CRTC
    pub back_buffers: Vec<(u32, u32)>,
    /// Pending flip requests
    pub pending_flips: Vec<PageFlipRequest>,
    /// Flip state per CRTC
    pub flip_states: Vec<(u32, FlipState)>,
    /// Vblank counter per CRTC
    pub vblank_counters: Vec<(u32, AtomicU64)>,
    /// Last vblank timestamp per CRTC (nanoseconds)
    pub vblank_timestamps: Vec<(u32, AtomicU64)>,
    /// Completed vblank events waiting to be consumed
    pub vblank_events: Vec<VblankEvent>,
}

impl PageFlipManager {
    pub fn new() -> Self {
        Self {
            front_buffers: Vec::new(),
            back_buffers: Vec::new(),
            pending_flips: Vec::new(),
            flip_states: Vec::new(),
            vblank_counters: Vec::new(),
            vblank_timestamps: Vec::new(),
            vblank_events: Vec::new(),
        }
    }

    /// Register a CRTC for page flipping
    pub fn register_crtc(&mut self, crtc_id: u32, initial_fb: u32) {
        // Avoid duplicates
        if self.front_buffers.iter().any(|(id, _)| *id == crtc_id) {
            return;
        }
        self.front_buffers.push((crtc_id, initial_fb));
        self.back_buffers.push((crtc_id, 0));
        self.flip_states.push((crtc_id, FlipState::Idle));
        self.vblank_counters.push((crtc_id, AtomicU64::new(0)));
        self.vblank_timestamps.push((crtc_id, AtomicU64::new(0)));
    }

    /// Request a page flip (swap back buffer to front on next vblank)
    pub fn request_flip(&mut self, request: PageFlipRequest) -> bool {
        // Check that CRTC is registered
        let state_idx = self
            .flip_states
            .iter()
            .position(|(id, _)| *id == request.crtc_id);
        let state_idx = match state_idx {
            Some(i) => i,
            None => return false,
        };

        // Reject if a flip is already pending
        if self.flip_states[state_idx].1 == FlipState::Pending {
            return false;
        }

        // Set back buffer and mark pending
        if let Some(back) = self
            .back_buffers
            .iter_mut()
            .find(|(id, _)| *id == request.crtc_id)
        {
            back.1 = request.fb_id;
        }
        self.flip_states[state_idx].1 = FlipState::Pending;
        self.pending_flips.push(request);
        true
    }

    /// Process a vblank interrupt for a CRTC (called from interrupt handler)
    pub fn handle_vblank(&mut self, crtc_id: u32, timestamp_ns: u64) {
        // Increment vblank counter
        if let Some((_, counter)) = self.vblank_counters.iter().find(|(id, _)| *id == crtc_id) {
            counter.fetch_add(1, Ordering::Relaxed);
        }

        // Update timestamp
        if let Some((_, ts)) = self.vblank_timestamps.iter().find(|(id, _)| *id == crtc_id) {
            ts.store(timestamp_ns, Ordering::Relaxed);
        }

        // Process pending flips for this CRTC
        let mut completed_indices = Vec::new();
        for (i, flip) in self.pending_flips.iter().enumerate() {
            if flip.crtc_id == crtc_id {
                // Swap front and back buffers
                if let Some(front) = self.front_buffers.iter_mut().find(|(id, _)| *id == crtc_id) {
                    if let Some(back) = self.back_buffers.iter().find(|(id, _)| *id == crtc_id) {
                        front.1 = back.1;
                    }
                }

                // Mark completed
                if let Some(state) = self.flip_states.iter_mut().find(|(id, _)| *id == crtc_id) {
                    state.1 = FlipState::Completed;
                }

                // Generate vblank event
                let seq = self
                    .vblank_counters
                    .iter()
                    .find(|(id, _)| *id == crtc_id)
                    .map(|(_, c)| c.load(Ordering::Relaxed))
                    .unwrap_or(0);

                self.vblank_events.push(VblankEvent {
                    sequence: seq,
                    timestamp_ns,
                    crtc_id,
                });

                completed_indices.push(i);
            }
        }

        // Remove completed flips (in reverse order to preserve indices)
        for &idx in completed_indices.iter().rev() {
            if idx < self.pending_flips.len() {
                self.pending_flips.remove(idx);
            }
        }

        // Reset flip state to idle after completion
        if let Some(state) = self.flip_states.iter_mut().find(|(id, _)| *id == crtc_id) {
            if state.1 == FlipState::Completed {
                state.1 = FlipState::Idle;
            }
        }
    }

    /// Get the current front buffer for a CRTC
    pub fn front_buffer(&self, crtc_id: u32) -> Option<u32> {
        self.front_buffers
            .iter()
            .find(|(id, _)| *id == crtc_id)
            .map(|(_, fb)| *fb)
    }

    /// Get the current vblank count for a CRTC
    pub fn vblank_count(&self, crtc_id: u32) -> Option<u64> {
        self.vblank_counters
            .iter()
            .find(|(id, _)| *id == crtc_id)
            .map(|(_, c)| c.load(Ordering::Relaxed))
    }

    /// Get the last vblank timestamp for a CRTC
    pub fn vblank_timestamp(&self, crtc_id: u32) -> Option<u64> {
        self.vblank_timestamps
            .iter()
            .find(|(id, _)| *id == crtc_id)
            .map(|(_, ts)| ts.load(Ordering::Relaxed))
    }

    /// Drain all pending vblank events
    pub fn drain_events(&mut self) -> Vec<VblankEvent> {
        core::mem::take(&mut self.vblank_events)
    }

    /// Check if a flip is pending for a given CRTC
    pub fn is_flip_pending(&self, crtc_id: u32) -> bool {
        self.flip_states
            .iter()
            .find(|(id, _)| *id == crtc_id)
            .map(|(_, s)| *s == FlipState::Pending)
            .unwrap_or(false)
    }
}

impl Default for PageFlipManager {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// 6. Hardware Cursor Plane
// ===========================================================================

/// Maximum cursor dimension
const MAX_CURSOR_SIZE: u32 = 64;

/// Cursor image format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorFormat {
    /// 32x32 ARGB8888
    Argb32x32,
    /// 64x64 ARGB8888
    Argb64x64,
}

impl CursorFormat {
    pub fn width(&self) -> u32 {
        match self {
            CursorFormat::Argb32x32 => 32,
            CursorFormat::Argb64x64 => 64,
        }
    }

    pub fn height(&self) -> u32 {
        self.width() // Square cursors
    }

    pub fn pixel_count(&self) -> usize {
        let w = self.width() as usize;
        w.checked_mul(w).unwrap_or(0)
    }
}

/// Hardware cursor plane state
pub struct HardwareCursor {
    /// Cursor image pixels (ARGB8888)
    pub image: Vec<u32>,
    /// Cursor format
    pub format: CursorFormat,
    /// Screen position X
    pub x: i32,
    /// Screen position Y
    pub y: i32,
    /// Hotspot offset X (within cursor image)
    pub hotspot_x: u32,
    /// Hotspot offset Y (within cursor image)
    pub hotspot_y: u32,
    /// Whether cursor is visible
    pub enabled: bool,
    /// Dirty flag (needs re-upload to hardware)
    pub dirty: bool,
}

impl HardwareCursor {
    pub fn new(format: CursorFormat) -> Self {
        let pixel_count = format.pixel_count();
        Self {
            image: vec![0u32; pixel_count],
            format,
            x: 0,
            y: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            enabled: false,
            dirty: true,
        }
    }

    /// Set cursor image data (ARGB8888 pixels)
    pub fn set_image(&mut self, pixels: &[u32]) -> bool {
        let expected = self.format.pixel_count();
        if pixels.len() != expected {
            return false;
        }
        self.image.copy_from_slice(pixels);
        self.dirty = true;
        true
    }

    /// Set cursor position on screen
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.dirty = true;
    }

    /// Set hotspot offset
    pub fn set_hotspot(&mut self, x: u32, y: u32) {
        self.hotspot_x = x.min(self.format.width().saturating_sub(1));
        self.hotspot_y = y.min(self.format.height().saturating_sub(1));
        self.dirty = true;
    }

    /// Enable cursor display
    pub fn enable(&mut self) {
        self.enabled = true;
        self.dirty = true;
    }

    /// Disable cursor display
    pub fn disable(&mut self) {
        self.enabled = false;
        self.dirty = true;
    }

    /// Composite cursor onto a scanout buffer
    ///
    /// Blends cursor ARGB pixels onto the target buffer at the cursor's
    /// position. Uses pre-multiplied alpha blending with integer math.
    pub fn composite_onto(&self, target: &mut [u32], target_width: u32, target_height: u32) {
        if !self.enabled {
            return;
        }

        let cursor_w = self.format.width() as i32;
        let cursor_h = self.format.height() as i32;
        let draw_x = self.x - self.hotspot_x as i32;
        let draw_y = self.y - self.hotspot_y as i32;

        let mut cy = 0i32;
        while cy < cursor_h {
            let screen_y = draw_y + cy;
            if screen_y >= 0 && screen_y < target_height as i32 {
                let mut cx = 0i32;
                while cx < cursor_w {
                    let screen_x = draw_x + cx;
                    if screen_x >= 0 && screen_x < target_width as i32 {
                        let cursor_idx = cy as usize * cursor_w as usize + cx as usize;
                        let target_idx =
                            screen_y as usize * target_width as usize + screen_x as usize;

                        if cursor_idx < self.image.len() && target_idx < target.len() {
                            let src = self.image[cursor_idx];
                            let sa = (src >> 24) & 0xFF;

                            if sa == 255 {
                                target[target_idx] = src;
                            } else if sa > 0 {
                                let dst = target[target_idx];
                                let inv_sa = 255 - sa;
                                let mut result = 0u32;
                                // Blend RGB channels
                                let mut shift = 0u32;
                                while shift <= 16 {
                                    let sc = (src >> shift) & 0xFF;
                                    let dc = (dst >> shift) & 0xFF;
                                    let blended = (sc * sa + dc * inv_sa + 127) / 255;
                                    result |= blended.min(255) << shift;
                                    shift += 8;
                                }
                                // Alpha channel
                                let da = (dst >> 24) & 0xFF;
                                let out_a = (sa + (da * inv_sa + 127) / 255).min(255);
                                result |= out_a << 24;
                                target[target_idx] = result;
                            }
                        }
                    }
                    cx += 1;
                }
            }
            cy += 1;
        }
    }

    /// Generate a default arrow cursor image (simple pointer)
    pub fn load_default_arrow(&mut self) {
        // Simple 16x16 arrow pattern, scaled to format size
        let w = self.format.width() as usize;
        let _h = self.format.height() as usize;

        // Clear to transparent
        for px in &mut self.image {
            *px = 0x00000000;
        }

        // Arrow shape (1-pixel-per-row widening triangle)
        let arrow_height = 16.min(w);
        let mut row = 0usize;
        while row < arrow_height {
            let row_width = row + 1;
            let mut col = 0usize;
            while col < row_width && col < w {
                let idx = row * w + col;
                if idx < self.image.len() {
                    // White interior with black outline
                    if col == 0 || col == row_width - 1 || row == arrow_height - 1 {
                        self.image[idx] = 0xFF000000; // Black
                    } else {
                        self.image[idx] = 0xFFFFFFFF; // White
                    }
                }
                col += 1;
            }
            row += 1;
        }

        self.hotspot_x = 0;
        self.hotspot_y = 0;
        self.dirty = true;
    }
}

impl Default for HardwareCursor {
    fn default() -> Self {
        Self::new(CursorFormat::Argb32x32)
    }
}

// ===========================================================================
// Global State
// ===========================================================================

static VIRGL_DRIVER: Mutex<Option<VirglDriver>> = Mutex::new(None);
static GEM_MANAGER: Mutex<Option<GemManager>> = Mutex::new(None);
static KMS_MANAGER: Mutex<Option<KmsManager>> = Mutex::new(None);
static PAGE_FLIP_MANAGER: Mutex<Option<PageFlipManager>> = Mutex::new(None);
static HARDWARE_CURSOR: Mutex<Option<HardwareCursor>> = Mutex::new(None);

/// Initialize the GPU acceleration subsystem
pub fn init() {
    *VIRGL_DRIVER.lock() = Some(VirglDriver::new());
    *GEM_MANAGER.lock() = Some(GemManager::default());
    *KMS_MANAGER.lock() = Some(KmsManager::new());
    *PAGE_FLIP_MANAGER.lock() = Some(PageFlipManager::new());
    *HARDWARE_CURSOR.lock() = Some(HardwareCursor::default());
}

/// Access the virgl driver
pub fn with_virgl<R, F: FnOnce(&mut VirglDriver) -> R>(f: F) -> Option<R> {
    VIRGL_DRIVER.lock().as_mut().map(f)
}

/// Access the GEM buffer manager
pub fn with_gem<R, F: FnOnce(&mut GemManager) -> R>(f: F) -> Option<R> {
    GEM_MANAGER.lock().as_mut().map(f)
}

/// Access the KMS manager
pub fn with_kms<R, F: FnOnce(&mut KmsManager) -> R>(f: F) -> Option<R> {
    KMS_MANAGER.lock().as_mut().map(f)
}

/// Access the page flip manager
pub fn with_page_flip<R, F: FnOnce(&mut PageFlipManager) -> R>(f: F) -> Option<R> {
    PAGE_FLIP_MANAGER.lock().as_mut().map(f)
}

/// Access the hardware cursor
pub fn with_cursor<R, F: FnOnce(&mut HardwareCursor) -> R>(f: F) -> Option<R> {
    HARDWARE_CURSOR.lock().as_mut().map(f)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -- Fixed-point math tests --

    #[test]
    fn test_fp_from_int_and_back() {
        assert_eq!(fp_to_int(fp_from_int(42)), 42);
        assert_eq!(fp_to_int(fp_from_int(-7)), -7);
        assert_eq!(fp_to_int(fp_from_int(0)), 0);
    }

    #[test]
    fn test_fp_mul_basic() {
        // 2.0 * 3.0 = 6.0
        let two = fp_from_int(2);
        let three = fp_from_int(3);
        let result = fp_mul(two, three);
        assert_eq!(fp_to_int(result), 6);
    }

    #[test]
    fn test_fp_mul_fractional() {
        // 0.5 * 0.5 = 0.25
        let half = FP_ONE / 2;
        let quarter = fp_mul(half, half);
        assert_eq!(quarter, FP_ONE / 4);
    }

    #[test]
    fn test_fp_div_basic() {
        // 6.0 / 3.0 = 2.0
        let six = fp_from_int(6);
        let three = fp_from_int(3);
        assert_eq!(fp_to_int(fp_div(six, three)), 2);
    }

    #[test]
    fn test_fp_div_by_zero() {
        let result = fp_div(fp_from_int(1), 0);
        assert_eq!(result, i32::MAX);
    }

    #[test]
    fn test_fp_lerp() {
        let a = fp_from_int(0);
        let b = fp_from_int(10);
        let mid = fp_lerp(a, b, FP_ONE / 2);
        assert_eq!(fp_to_int(mid), 5);
    }

    // -- Virgl driver tests --

    #[test]
    fn test_virgl_create_context() {
        let mut driver = VirglDriver::new();
        let ctx_id = driver.create_context(b"test_ctx");
        assert_eq!(ctx_id, 1);
        assert_eq!(driver.contexts.len(), 1);
        assert!(driver.contexts[0].active);
    }

    #[test]
    fn test_virgl_destroy_context() {
        let mut driver = VirglDriver::new();
        let ctx_id = driver.create_context(b"test");
        assert!(driver.destroy_context(ctx_id));
        assert_eq!(driver.contexts.len(), 0);
        assert!(!driver.destroy_context(ctx_id)); // Already destroyed
    }

    #[test]
    fn test_virgl_create_resource_3d() {
        let mut driver = VirglDriver::new();
        let rid = driver.create_resource_3d(
            Virgl3dResourceType::Texture2D,
            VirglFormat::R8G8B8A8Unorm,
            256,
            256,
            1,
            1,
            0,
            1,
            0,
        );
        assert_eq!(rid, 1);
        let res = driver.find_resource(rid).unwrap();
        assert_eq!(res.width, 256);
        assert_eq!(res.resource_type, Virgl3dResourceType::Texture2D);
    }

    #[test]
    fn test_virgl_transfer_3d() {
        let mut driver = VirglDriver::new();
        let rid = driver.create_resource_3d(
            Virgl3dResourceType::Buffer,
            VirglFormat::R32Uint,
            1024,
            1,
            1,
            1,
            0,
            1,
            0,
        );
        assert!(driver.transfer_3d(
            rid,
            0,
            0,
            0,
            0,
            512,
            1,
            1,
            4096,
            0,
            TransferDirection::ToHost
        ));
        assert!(!driver.transfer_3d(999, 0, 0, 0, 0, 1, 1, 1, 0, 0, TransferDirection::ToHost));
    }

    #[test]
    fn test_virgl_fence() {
        let mut driver = VirglDriver::new();
        let ctx_id = driver.create_context(b"fence_ctx");
        let fence_id = driver.create_fence(ctx_id);
        assert_eq!(driver.is_fence_signaled(fence_id), Some(false));
        assert!(driver.signal_fence(fence_id));
        assert_eq!(driver.is_fence_signaled(fence_id), Some(true));
    }

    #[test]
    fn test_virgl_flush() {
        let mut driver = VirglDriver::new();
        driver.create_context(b"a");
        driver.create_resource_3d(
            Virgl3dResourceType::Buffer,
            VirglFormat::R32Uint,
            64,
            1,
            1,
            1,
            0,
            1,
            0,
        );
        let count = driver.flush();
        assert!(count >= 2); // at least CtxCreate + CreateResource3d
        assert_eq!(driver.command_queue.len(), 0);
    }

    // -- Software rasterizer tests --

    #[test]
    fn test_rasterizer_clear() {
        let mut rast = SoftwareRasterizer::new(4, 4);
        rast.clear_color = 0xFFFF0000;
        rast.clear_color_buffer();
        assert_eq!(rast.color_buffer[0], 0xFFFF0000);
        assert_eq!(rast.color_buffer[15], 0xFFFF0000);
    }

    #[test]
    fn test_rasterizer_depth_test() {
        let rast = SoftwareRasterizer::new(4, 4);
        assert!(rast.depth_test_pass(5, 10)); // Less: 5 < 10
        assert!(!rast.depth_test_pass(10, 5)); // Less: 10 < 5 = false
    }

    #[test]
    fn test_rasterizer_scissor() {
        let mut rast = SoftwareRasterizer::new(100, 100);
        rast.scissor = Some(ScissorRect {
            x: 10,
            y: 10,
            width: 20,
            height: 20,
        });
        assert!(rast.scissor_test(15, 15));
        assert!(!rast.scissor_test(5, 5));
        assert!(!rast.scissor_test(35, 35));
    }

    #[test]
    fn test_rasterizer_alpha_blend() {
        let mut rast = SoftwareRasterizer::new(1, 1);
        rast.blend_mode = BlendMode::Alpha;
        // Fully opaque src over anything = src
        let result = rast.alpha_blend(0xFFFF0000, 0xFF00FF00);
        assert_eq!(result, 0xFFFF0000);
        // Fully transparent src = dst
        let result = rast.alpha_blend(0x00FF0000, 0xFF00FF00);
        assert_eq!(result, 0xFF00FF00);
    }

    #[test]
    fn test_texture_nearest_sampling() {
        let mut tex = SoftTexture::new(2, 2);
        tex.pixels[0] = 0xFFFF0000; // top-left red
        tex.pixels[1] = 0xFF00FF00; // top-right green
        tex.pixels[2] = 0xFF0000FF; // bottom-left blue
        tex.pixels[3] = 0xFFFFFFFF; // bottom-right white
        tex.filter = TextureFilter::Nearest;
        // Sample top-left corner
        let c = tex.sample(0, 0);
        assert_eq!(c, 0xFFFF0000);
    }

    #[test]
    fn test_vec4_lerp() {
        let a = Vec4::from_ints(0, 0, 0, 0);
        let b = Vec4::from_ints(10, 20, 30, 40);
        let mid = Vec4::lerp(&a, &b, FP_ONE / 2);
        assert_eq!(fp_to_int(mid.x), 5);
        assert_eq!(fp_to_int(mid.y), 10);
    }

    // -- GEM buffer management tests --

    #[test]
    fn test_gem_create_destroy() {
        let mut gem = GemManager::new(1024);
        let h = gem.create_buffer(256).unwrap();
        assert_eq!(gem.buffer_count(), 1);
        assert_eq!(gem.total_allocated, 256);
        assert!(gem.destroy_buffer(h));
        assert_eq!(gem.buffer_count(), 0);
        assert_eq!(gem.total_allocated, 0);
    }

    #[test]
    fn test_gem_over_allocation() {
        let mut gem = GemManager::new(100);
        assert!(gem.create_buffer(50).is_some());
        assert!(gem.create_buffer(50).is_some());
        assert!(gem.create_buffer(1).is_none()); // Over limit
    }

    #[test]
    fn test_gem_pin_unpin() {
        let mut gem = GemManager::new(1024);
        let h = gem.create_buffer(64).unwrap();
        assert!(gem.pin_buffer(h));
        assert!(gem.find_buffer(h).unwrap().pinned);
        assert!(gem.unpin_buffer(h));
        assert!(!gem.find_buffer(h).unwrap().pinned);
    }

    #[test]
    fn test_gem_domain_change() {
        let mut gem = GemManager::new(1024);
        let h = gem.create_buffer(64).unwrap();
        assert_eq!(gem.find_buffer(h).unwrap().domain, MemoryDomain::Cpu);
        assert!(gem.set_domain(h, MemoryDomain::Gpu));
        assert_eq!(gem.find_buffer(h).unwrap().domain, MemoryDomain::Gpu);
    }

    #[test]
    fn test_gem_ref_counting() {
        let mut gem = GemManager::new(1024);
        let h = gem.create_buffer(64).unwrap();
        assert_eq!(gem.find_buffer(h).unwrap().ref_count(), 1);
        gem.add_ref(h);
        assert_eq!(gem.find_buffer(h).unwrap().ref_count(), 2);
        // First destroy just decrements ref_count
        assert!(!gem.destroy_buffer(h));
        assert_eq!(gem.buffer_count(), 1); // Still alive
                                           // Second destroy actually frees
        assert!(gem.destroy_buffer(h));
        assert_eq!(gem.buffer_count(), 0);
    }

    // -- KMS tests --

    #[test]
    fn test_display_mode_validation() {
        let mode = DisplayMode::mode_1080p60();
        assert!(mode.validate());

        let bad_mode = DisplayMode {
            hdisplay: 0,
            vdisplay: 0,
            clock_khz: 0,
            hsync_start: 0,
            hsync_end: 0,
            htotal: 0,
            vsync_start: 0,
            vsync_end: 0,
            vtotal: 0,
            vrefresh_mhz: 0,
        };
        assert!(!bad_mode.validate());
    }

    #[test]
    fn test_kms_atomic_commit() {
        let mut kms = KmsManager::new();
        let crtc_id = kms.add_crtc();
        let conn_id = kms.add_connector(ConnectorType::Hdmi);
        let fb_id = kms.create_framebuffer(1920, 1080, PixelFormat::Xrgb8888, 1);

        let commit = AtomicCommit {
            crtc_id,
            connector_id: conn_id,
            fb_id,
            mode: DisplayMode::mode_1080p60(),
            test_only: false,
        };
        assert!(kms.atomic_commit(&commit));
        let crtc = kms.find_crtc(crtc_id).unwrap();
        assert!(crtc.active);
        assert_eq!(crtc.fb_id, Some(fb_id));
    }

    #[test]
    fn test_kms_test_only_commit() {
        let mut kms = KmsManager::new();
        let crtc_id = kms.add_crtc();
        let conn_id = kms.add_connector(ConnectorType::DisplayPort);
        let fb_id = kms.create_framebuffer(1280, 720, PixelFormat::Xrgb8888, 1);

        let commit = AtomicCommit {
            crtc_id,
            connector_id: conn_id,
            fb_id,
            mode: DisplayMode::mode_720p60(),
            test_only: true,
        };
        assert!(kms.atomic_commit(&commit));
        // CRTC should NOT be modified
        let crtc = kms.find_crtc(crtc_id).unwrap();
        assert!(!crtc.active);
    }

    // -- Page flip tests --

    #[test]
    fn test_page_flip_basic() {
        let mut pfm = PageFlipManager::new();
        pfm.register_crtc(1, 10); // CRTC 1, initial fb 10

        let req = PageFlipRequest {
            crtc_id: 1,
            fb_id: 20,
            user_data: 0,
        };
        assert!(pfm.request_flip(req));
        assert!(pfm.is_flip_pending(1));

        // Simulate vblank
        pfm.handle_vblank(1, 16_666_666);
        assert!(!pfm.is_flip_pending(1));
        assert_eq!(pfm.front_buffer(1), Some(20));
        assert_eq!(pfm.vblank_count(1), Some(1));
    }

    #[test]
    fn test_page_flip_reject_double() {
        let mut pfm = PageFlipManager::new();
        pfm.register_crtc(1, 10);

        let req = PageFlipRequest {
            crtc_id: 1,
            fb_id: 20,
            user_data: 0,
        };
        assert!(pfm.request_flip(req));
        // Second flip should fail while first is pending
        let req2 = PageFlipRequest {
            crtc_id: 1,
            fb_id: 30,
            user_data: 0,
        };
        assert!(!pfm.request_flip(req2));
    }

    // -- Hardware cursor tests --

    #[test]
    fn test_cursor_set_image() {
        let mut cursor = HardwareCursor::new(CursorFormat::Argb32x32);
        let pixels = vec![0xFFFFFFFF; 32 * 32];
        assert!(cursor.set_image(&pixels));
        // Wrong size should fail
        let bad = vec![0u32; 10];
        assert!(!cursor.set_image(&bad));
    }

    #[test]
    fn test_cursor_composite() {
        let mut cursor = HardwareCursor::new(CursorFormat::Argb32x32);
        cursor.enable();
        cursor.set_position(0, 0);

        // Set a single opaque white pixel at (0,0)
        let mut pixels = vec![0x00000000u32; 32 * 32];
        pixels[0] = 0xFFFFFFFF;
        cursor.set_image(&pixels);

        let mut target = vec![0xFF000000u32; 64 * 64]; // 64x64 black
        cursor.composite_onto(&mut target, 64, 64);
        assert_eq!(target[0], 0xFFFFFFFF); // Cursor pixel composited
        assert_eq!(target[1], 0xFF000000); // Unaffected
    }

    #[test]
    fn test_cursor_default_arrow() {
        let mut cursor = HardwareCursor::new(CursorFormat::Argb32x32);
        cursor.load_default_arrow();
        // First pixel of arrow should be black (outline)
        assert_eq!(cursor.image[0], 0xFF000000);
        assert_eq!(cursor.hotspot_x, 0);
        assert_eq!(cursor.hotspot_y, 0);
    }

    #[test]
    fn test_cursor_hotspot() {
        let mut cursor = HardwareCursor::new(CursorFormat::Argb64x64);
        cursor.set_hotspot(16, 16);
        assert_eq!(cursor.hotspot_x, 16);
        assert_eq!(cursor.hotspot_y, 16);
        // Clamp to max
        cursor.set_hotspot(100, 100);
        assert_eq!(cursor.hotspot_x, 63);
        assert_eq!(cursor.hotspot_y, 63);
    }

    #[test]
    fn test_display_mode_refresh_calc() {
        let mode = DisplayMode::mode_1080p60();
        let refresh = mode.calculated_refresh_mhz();
        // Should be approximately 60000 mHz (60 Hz)
        // 148500 * 1_000_000 / (2200 * 1125) = 60000
        assert_eq!(refresh, 60000);
    }

    #[test]
    fn test_framebuffer_size() {
        let fb = DrmFramebuffer::new(1, 1920, 1080, PixelFormat::Xrgb8888, 1);
        assert_eq!(fb.pitch, 1920 * 4);
        assert_eq!(fb.size_bytes(), 1920 * 4 * 1080);
    }

    #[test]
    fn test_pixel_format_bpp() {
        assert_eq!(PixelFormat::Xrgb8888.bpp(), 4);
        assert_eq!(PixelFormat::Rgb565.bpp(), 2);
        assert_eq!(PixelFormat::Argb8888.bpp(), 4);
    }
}

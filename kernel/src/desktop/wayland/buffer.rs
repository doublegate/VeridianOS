//! Wayland Buffer and SHM Pool Management
//!
//! Provides pixel buffer allocation backed by kernel-heap memory pools.
//! Each `WlShmPool` owns a contiguous byte allocation from which individual
//! `WlBuffer` objects are sub-allocated at arbitrary offsets. This mirrors
//! the real Wayland wl_shm / wl_shm_pool / wl_buffer protocol objects.

use alloc::{collections::BTreeMap, vec, vec::Vec};

use spin::Mutex;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Pixel formats
// ---------------------------------------------------------------------------

/// Supported pixel formats (subset of wl_shm.format).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit ARGB with alpha channel
    Argb8888,
    /// 32-bit XRGB (alpha ignored, treated as 0xFF)
    Xrgb8888,
    /// 16-bit RGB 5:6:5
    Rgb565,
}

impl PixelFormat {
    /// Bytes per pixel for this format.
    pub fn bpp(self) -> u32 {
        match self {
            PixelFormat::Argb8888 | PixelFormat::Xrgb8888 => 4,
            PixelFormat::Rgb565 => 2,
        }
    }

    /// Convert a Wayland wl_shm format code to our enum.
    pub fn from_wl_format(code: u32) -> Option<Self> {
        match code {
            0 => Some(PixelFormat::Argb8888), // WL_SHM_FORMAT_ARGB8888
            1 => Some(PixelFormat::Xrgb8888), // WL_SHM_FORMAT_XRGB8888
            _ => None,
        }
    }

    /// Convert to Wayland wl_shm format code.
    #[allow(dead_code)] // Phase 6: used when announcing supported formats
    pub fn to_wl_format(self) -> u32 {
        match self {
            PixelFormat::Argb8888 => 0,
            PixelFormat::Xrgb8888 => 1,
            PixelFormat::Rgb565 => 0x20363154, // WL_SHM_FORMAT_RGB565
        }
    }
}

// ---------------------------------------------------------------------------
// WlShmPool -- shared memory pool
// ---------------------------------------------------------------------------

/// A shared memory pool that backs one or more buffers.
///
/// In a real Wayland compositor the pool would reference a client-provided
/// file descriptor pointing to an mmap'd region. Here we allocate from the
/// kernel heap as a stand-in, since user-space shared memory is not yet
/// wired for graphics.
pub struct WlShmPool {
    /// Pool object ID (Wayland protocol ID)
    pub id: u32,
    /// Owning client ID
    pub client_id: u32,
    /// Backing byte storage
    data: Vec<u8>,
    /// Total size in bytes
    pub size: usize,
    /// Buffers sub-allocated from this pool (buffer_id -> WlBuffer)
    buffers: BTreeMap<u32, WlBuffer>,
    /// Next buffer ID within this pool
    next_buffer_id: u32,
}

impl WlShmPool {
    /// Create a new pool with `size` bytes of zeroed backing memory.
    pub fn new(id: u32, client_id: u32, size: usize) -> Self {
        Self {
            id,
            client_id,
            data: vec![0u8; size],
            size,
            buffers: BTreeMap::new(),
            next_buffer_id: 1,
        }
    }

    /// Create a buffer that references a region within this pool.
    ///
    /// Arguments mirror wl_shm_pool.create_buffer:
    ///   offset -- byte offset into the pool
    ///   width, height -- dimensions in pixels
    ///   stride -- bytes per row
    ///   format -- pixel format code
    pub fn create_buffer(
        &mut self,
        offset: u32,
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Result<u32, KernelError> {
        // Validate that the described region fits inside the pool.
        let end = offset as usize + (stride as usize) * (height as usize);
        if end > self.size {
            return Err(KernelError::InvalidArgument {
                name: "buffer region",
                value: "exceeds pool size",
            });
        }
        if stride < width * format.bpp() {
            return Err(KernelError::InvalidArgument {
                name: "stride",
                value: "smaller than row width",
            });
        }

        let buf_id = self.next_buffer_id;
        self.next_buffer_id += 1;

        let buffer = WlBuffer {
            id: buf_id,
            pool_id: self.id,
            offset,
            width,
            height,
            stride,
            format,
            released: true, // initially available for client writes
        };
        self.buffers.insert(buf_id, buffer);
        Ok(buf_id)
    }

    /// Get an immutable reference to a buffer.
    pub fn get_buffer(&self, buffer_id: u32) -> Option<&WlBuffer> {
        self.buffers.get(&buffer_id)
    }

    /// Remove a buffer from this pool.
    #[allow(dead_code)] // Phase 6: buffer destruction path
    pub fn destroy_buffer(&mut self, buffer_id: u32) -> bool {
        self.buffers.remove(&buffer_id).is_some()
    }

    /// Read pixel data for a buffer from the pool backing store.
    ///
    /// Returns a slice of the pool data corresponding to the buffer's region.
    pub fn read_buffer_pixels(&self, buffer_id: u32) -> Option<&[u8]> {
        let buf = self.buffers.get(&buffer_id)?;
        let start = buf.offset as usize;
        let len = buf.stride as usize * buf.height as usize;
        if start + len > self.data.len() {
            return None;
        }
        Some(&self.data[start..start + len])
    }

    /// Write pixel data into the pool backing store at the buffer's offset.
    ///
    /// Used for testing and by kernel-side rendering that needs to fill a
    /// buffer (e.g. a cursor image).
    pub fn write_buffer_pixels(&mut self, buffer_id: u32, pixels: &[u8]) -> bool {
        let buf = match self.buffers.get(&buffer_id) {
            Some(b) => b,
            None => return false,
        };
        let start = buf.offset as usize;
        let len = buf.stride as usize * buf.height as usize;
        if start + len > self.data.len() || pixels.len() < len {
            return false;
        }
        self.data[start..start + len].copy_from_slice(&pixels[..len]);
        true
    }

    /// Get raw access to the pool's backing memory (for direct pixel reads
    /// by the compositor).
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Write raw bytes into the pool at a given offset.
    ///
    /// Used by the desktop renderer to populate background/window pixel data.
    pub fn write_data(&mut self, offset: usize, data: &[u8]) {
        let end = (offset + data.len()).min(self.data.len());
        let src_len = end - offset;
        self.data[offset..end].copy_from_slice(&data[..src_len]);
    }

    /// Resize the pool (wl_shm_pool.resize). Only growing is allowed.
    #[allow(dead_code)] // Phase 6: pool resize protocol support
    pub fn resize(&mut self, new_size: usize) -> Result<(), KernelError> {
        if new_size < self.size {
            return Err(KernelError::InvalidArgument {
                name: "pool size",
                value: "cannot shrink",
            });
        }
        self.data.resize(new_size, 0);
        self.size = new_size;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WlBuffer -- sub-region of a pool
// ---------------------------------------------------------------------------

/// A buffer object referencing pixel data within a `WlShmPool`.
#[derive(Debug, Clone)]
pub struct WlBuffer {
    /// Buffer object ID
    pub id: u32,
    /// Owning pool ID
    pub pool_id: u32,
    /// Byte offset into the pool
    pub offset: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Bytes per row
    pub stride: u32,
    /// Pixel format
    pub format: PixelFormat,
    /// Whether the compositor has released this buffer back to the client
    pub released: bool,
}

impl WlBuffer {
    /// Total byte size of pixel data for this buffer.
    pub fn byte_size(&self) -> usize {
        self.stride as usize * self.height as usize
    }
}

// ---------------------------------------------------------------------------
// Global SHM pool registry
// ---------------------------------------------------------------------------

/// Registry of all SHM pools, keyed by pool object ID.
///
/// Accessed from both the Wayland protocol dispatcher and the compositor
/// rendering path, so we use a spin Mutex.
static SHM_POOLS: Mutex<Option<BTreeMap<u32, WlShmPool>>> = Mutex::new(None);

/// Initialize the SHM pool registry (called once during Wayland init).
pub fn init_shm_pools() {
    let mut pools = SHM_POOLS.lock();
    if pools.is_none() {
        *pools = Some(BTreeMap::new());
    }
}

/// Register a new SHM pool.
pub fn register_pool(pool: WlShmPool) -> u32 {
    let id = pool.id;
    let mut guard = SHM_POOLS.lock();
    if let Some(ref mut pools) = *guard {
        pools.insert(id, pool);
    }
    id
}

/// Execute a closure with mutable access to a specific pool.
pub fn with_pool_mut<R, F: FnOnce(&mut WlShmPool) -> R>(pool_id: u32, f: F) -> Option<R> {
    let mut guard = SHM_POOLS.lock();
    guard
        .as_mut()
        .and_then(|pools| pools.get_mut(&pool_id).map(f))
}

/// Execute a closure with read-only access to a specific pool.
pub fn with_pool<R, F: FnOnce(&WlShmPool) -> R>(pool_id: u32, f: F) -> Option<R> {
    let guard = SHM_POOLS.lock();
    guard.as_ref().and_then(|pools| pools.get(&pool_id).map(f))
}

/// Remove a pool from the registry.
#[allow(dead_code)] // Phase 6: pool destruction
pub fn unregister_pool(pool_id: u32) -> Option<WlShmPool> {
    let mut guard = SHM_POOLS.lock();
    guard.as_mut().and_then(|pools| pools.remove(&pool_id))
}

// ---------------------------------------------------------------------------
// Legacy compat: Buffer type alias used by surface.rs
// ---------------------------------------------------------------------------

/// Legacy buffer struct retained for surface.rs compatibility.
/// New code should use `WlBuffer` + pool references.
#[derive(Debug, Clone)]
pub struct Buffer {
    /// Buffer ID
    pub id: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Stride (bytes per row)
    pub stride: u32,
    /// Pixel format
    pub format: PixelFormat,
    /// Pool ID that owns this buffer's pixel data
    pub pool_id: u32,
    /// Buffer ID within the pool
    pub pool_buffer_id: u32,
}

impl Buffer {
    /// Create a new buffer descriptor.
    pub fn new(id: u32, width: u32, height: u32, format: PixelFormat) -> Self {
        let stride = width * format.bpp();
        Self {
            id,
            width,
            height,
            stride,
            format,
            pool_id: 0,
            pool_buffer_id: 0,
        }
    }

    /// Create a buffer linked to a pool.
    pub fn from_pool(
        id: u32,
        pool_id: u32,
        pool_buffer_id: u32,
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Self {
        Self {
            id,
            width,
            height,
            stride,
            format,
            pool_id,
            pool_buffer_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_format_bpp() {
        assert_eq!(PixelFormat::Argb8888.bpp(), 4);
        assert_eq!(PixelFormat::Xrgb8888.bpp(), 4);
        assert_eq!(PixelFormat::Rgb565.bpp(), 2);
    }

    #[test]
    fn test_pool_create_buffer() {
        let mut pool = WlShmPool::new(1, 1, 1024 * 768 * 4);
        let buf_id = pool
            .create_buffer(0, 1024, 768, 1024 * 4, PixelFormat::Xrgb8888)
            .unwrap();
        assert_eq!(buf_id, 1);
        let buf = pool.get_buffer(buf_id).unwrap();
        assert_eq!(buf.width, 1024);
        assert_eq!(buf.height, 768);
    }

    #[test]
    fn test_pool_buffer_out_of_bounds() {
        let mut pool = WlShmPool::new(1, 1, 100);
        // Request buffer larger than pool
        let result = pool.create_buffer(0, 100, 100, 400, PixelFormat::Argb8888);
        assert!(result.is_err());
    }

    #[test]
    fn test_pool_write_read_pixels() {
        let mut pool = WlShmPool::new(1, 1, 16);
        let buf_id = pool
            .create_buffer(0, 2, 2, 8, PixelFormat::Xrgb8888)
            .unwrap();
        let pixels = [0xFFu8; 16];
        assert!(pool.write_buffer_pixels(buf_id, &pixels));
        let read = pool.read_buffer_pixels(buf_id).unwrap();
        assert_eq!(read, &pixels[..]);
    }

    #[test]
    fn test_format_from_wl() {
        assert_eq!(PixelFormat::from_wl_format(0), Some(PixelFormat::Argb8888));
        assert_eq!(PixelFormat::from_wl_format(1), Some(PixelFormat::Xrgb8888));
        assert_eq!(PixelFormat::from_wl_format(999), None);
    }
}

//! POSIX Shared Memory (shm_open / shm_unlink)
//!
//! Provides named shared memory objects accessible via `/dev/shm` semantics.
//! Used by Wayland compositors and other IPC-heavy applications for zero-copy
//! buffer sharing between processes.
//!
//! This module implements the kernel-side of the POSIX shared memory API:
//! - `shm_open()`: Create or open a named shared memory object
//! - `shm_unlink()`: Remove a named shared memory object
//! - `ftruncate()`: Set the size of the shared memory object
//! - `mmap(MAP_SHARED)`: Map the shared memory into a process address space
//!
//! Named objects are stored in a global registry keyed by name. Each object
//! tracks its physical backing frames and per-process virtual mappings.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use spin::Mutex;

use crate::{
    error::{KernelError, KernelResult},
    process::ProcessId,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum name length for a shared memory object.
pub const SHM_NAME_MAX: usize = 255;

/// Maximum number of concurrent shared memory objects.
pub const SHM_MAX_OBJECTS: usize = 256;

/// Maximum size of a single shared memory object (256 MB).
pub const SHM_MAX_SIZE: usize = 256 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Data Structures
// ---------------------------------------------------------------------------

/// Open flags for shm_open (mirror POSIX O_CREAT, O_EXCL, O_RDONLY, O_RDWR).
#[derive(Debug, Clone, Copy)]
pub struct ShmOpenFlags {
    /// Create the object if it does not exist.
    pub create: bool,
    /// Fail if object already exists (used with create).
    pub exclusive: bool,
    /// Read-only access.
    pub read_only: bool,
}

impl ShmOpenFlags {
    /// O_RDWR | O_CREAT
    pub const CREATE_RDWR: Self = Self {
        create: true,
        exclusive: false,
        read_only: false,
    };

    /// O_RDONLY
    pub const RDONLY: Self = Self {
        create: false,
        exclusive: false,
        read_only: true,
    };
}

/// A per-process mapping of a shared memory object.
#[derive(Debug, Clone)]
pub struct ShmMapping {
    /// Virtual address in the process's address space.
    pub virt_addr: u64,
    /// Mapping size (may be less than the object size).
    pub size: usize,
    /// Read-only mapping.
    pub read_only: bool,
}

/// A named shared memory object.
pub struct ShmObject {
    /// Object name (without leading slash).
    pub name: String,
    /// Unique object ID.
    pub id: u64,
    /// Size in bytes (set by ftruncate).
    pub size: usize,
    /// Physical frame number of the backing memory (contiguous).
    pub phys_frame: usize,
    /// Number of physical frames allocated.
    pub num_frames: usize,
    /// Reference count (number of open descriptors).
    pub ref_count: AtomicU32,
    /// Per-process virtual mappings.
    pub mappings: Mutex<BTreeMap<u64, ShmMapping>>, // key = ProcessId.0
    /// Creator process ID.
    pub owner: ProcessId,
    /// Whether the object has been unlinked (will be destroyed when ref_count
    /// reaches 0).
    pub unlinked: bool,
}

// ---------------------------------------------------------------------------
// Global Registry
// ---------------------------------------------------------------------------

/// Global counter for unique object IDs.
static NEXT_SHM_ID: AtomicU64 = AtomicU64::new(1);

/// Global registry of named shared memory objects.
static SHM_REGISTRY: Mutex<BTreeMap<String, ShmObject>> = Mutex::new(BTreeMap::new());

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

/// Create or open a named shared memory object.
///
/// Returns the object ID on success. The object starts with size 0;
/// use `shm_truncate()` to set its size before mapping.
pub fn shm_open(name: &str, flags: ShmOpenFlags, owner: ProcessId) -> KernelResult<u64> {
    if name.is_empty() || name.len() > SHM_NAME_MAX {
        return Err(KernelError::InvalidArgument {
            name: "name",
            value: "empty or exceeds SHM_NAME_MAX",
        });
    }

    let mut registry = SHM_REGISTRY.lock();

    if let Some(existing) = registry.get(name) {
        if flags.exclusive {
            return Err(KernelError::AlreadyExists {
                resource: "shm_object",
                id: existing.id,
            });
        }
        existing.ref_count.fetch_add(1, Ordering::Relaxed);
        return Ok(existing.id);
    }

    // Object does not exist.
    if !flags.create {
        return Err(KernelError::NotFound {
            resource: "shm_object",
            id: 0,
        });
    }

    if registry.len() >= SHM_MAX_OBJECTS {
        return Err(KernelError::ResourceExhausted {
            resource: "shm_objects",
        });
    }

    let id = NEXT_SHM_ID.fetch_add(1, Ordering::Relaxed);
    let obj = ShmObject {
        name: name.to_string(),
        id,
        size: 0,
        phys_frame: 0,
        num_frames: 0,
        ref_count: AtomicU32::new(1),
        mappings: Mutex::new(BTreeMap::new()),
        owner,
        unlinked: false,
    };

    registry.insert(name.to_string(), obj);

    println!("[SHM] Created shared memory object '{}' (id={})", name, id);
    Ok(id)
}

/// Remove a named shared memory object.
///
/// The object's name is removed from the registry immediately, but the
/// backing memory is not freed until all references are closed (ref_count
/// reaches 0).
pub fn shm_unlink(name: &str) -> KernelResult<()> {
    let mut registry = SHM_REGISTRY.lock();

    if let Some(obj) = registry.get_mut(name) {
        obj.unlinked = true;
        let refs = obj.ref_count.load(Ordering::Relaxed);
        if refs == 0 {
            // Free physical frames if allocated.
            if obj.num_frames > 0 {
                let frame = crate::mm::FrameNumber::new(obj.phys_frame as u64);
                let _ = crate::mm::FRAME_ALLOCATOR
                    .lock()
                    .free_frames(frame, obj.num_frames);
            }
            registry.remove(name);
            println!("[SHM] Unlinked and destroyed '{}'", name);
        } else {
            println!(
                "[SHM] Unlinked '{}' (deferred destroy, {} refs remaining)",
                name, refs
            );
        }
        Ok(())
    } else {
        Err(KernelError::NotFound {
            resource: "shm_object",
            id: 0,
        })
    }
}

/// Whether `obj` may change size now (N-88): not while any process maps
/// it. Its frames move on a resize, so a mapping would keep the old,
/// freed frames -- a use-after-free once they are reused.
fn check_resizable(obj: &ShmObject) -> KernelResult<()> {
    if obj.mappings.lock().is_empty() {
        Ok(())
    } else {
        Err(KernelError::FsError(crate::error::FsError::Busy))
    }
}

/// Set the size of a shared memory object (analogous to ftruncate).
///
/// Refused (`Busy`) while the object is mapped. The contents up to the
/// smaller of the old and new size are kept, and new bytes read as zero,
/// as POSIX `ftruncate` does; they used to be discarded.
pub fn shm_truncate(name: &str, size: usize) -> KernelResult<()> {
    if size > SHM_MAX_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "size",
            value: "exceeds SHM_MAX_SIZE",
        });
    }

    let mut registry = SHM_REGISTRY.lock();
    let obj = registry.get_mut(name).ok_or(KernelError::NotFound {
        resource: "shm_object",
        id: 0,
    })?;
    check_resizable(obj)?;

    // New backing first (zeroed), so a failed allocation changes nothing.
    let num_frames = size.div_ceil(4096);
    let new_frame = if num_frames > 0 {
        let frame = crate::mm::FRAME_ALLOCATOR
            .lock()
            .allocate_frames(num_frames, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: size,
                available: 0,
            })?;
        let virt = crate::mm::phys_to_virt_addr(frame.as_u64() * 4096);
        // SAFETY: `virt` is the physical-memory-map address of the
        // `num_frames` frames just allocated, owned here and unmapped
        // elsewhere.
        unsafe { core::ptr::write_bytes(virt as *mut u8, 0, num_frames * 4096) };
        Some(frame)
    } else {
        None
    };

    // Carry the old contents over, then release the old frames.
    if obj.num_frames > 0 {
        if let Some(frame) = new_frame {
            let keep = obj.size.min(size);
            let from = crate::mm::phys_to_virt_addr(obj.phys_frame as u64 * 4096);
            let to = crate::mm::phys_to_virt_addr(frame.as_u64() * 4096);
            // SAFETY: both ranges are this object's frames (old and new,
            // disjoint allocations) through the physical-memory map, and
            // `keep` is within both sizes.
            unsafe { core::ptr::copy_nonoverlapping(from as *const u8, to as *mut u8, keep) };
        }
        let old = crate::mm::FrameNumber::new(obj.phys_frame as u64);
        let _ = crate::mm::FRAME_ALLOCATOR
            .lock()
            .free_frames(old, obj.num_frames);
    }

    obj.phys_frame = new_frame.map_or(0, |f| f.as_u64() as usize);
    obj.num_frames = num_frames;
    obj.size = size;

    println!(
        "[SHM] Truncated '{}' to {} bytes ({} frames)",
        name, size, num_frames
    );
    Ok(())
}

/// Close a reference to a shared memory object.
///
/// Decrements the reference count. If the object was unlinked and this
/// was the last reference, the backing memory is freed.
pub fn shm_close(name: &str) -> KernelResult<()> {
    let mut registry = SHM_REGISTRY.lock();
    let should_destroy = if let Some(obj) = registry.get(name) {
        let prev = obj.ref_count.fetch_sub(1, Ordering::Release);
        prev == 1 && obj.unlinked
    } else {
        return Ok(());
    };

    if should_destroy {
        if let Some(obj) = registry.remove(name) {
            if obj.num_frames > 0 {
                let frame = crate::mm::FrameNumber::new(obj.phys_frame as u64);
                let _ = crate::mm::FRAME_ALLOCATOR
                    .lock()
                    .free_frames(frame, obj.num_frames);
            }
            println!("[SHM] Destroyed '{}' (last reference closed)", name);
        }
    }
    Ok(())
}

/// Get information about a shared memory object.
pub fn shm_stat(name: &str) -> KernelResult<(u64, usize, u32)> {
    let registry = SHM_REGISTRY.lock();
    let obj = registry.get(name).ok_or(KernelError::NotFound {
        resource: "shm_object",
        id: 0,
    })?;
    Ok((obj.id, obj.size, obj.ref_count.load(Ordering::Relaxed)))
}

/// Get the number of active shared memory objects.
pub fn shm_count() -> usize {
    SHM_REGISTRY.lock().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object() -> ShmObject {
        ShmObject {
            name: String::from("t"),
            id: 1,
            size: 0,
            phys_frame: 0,
            num_frames: 0,
            ref_count: AtomicU32::new(1),
            mappings: Mutex::new(BTreeMap::new()),
            owner: ProcessId(1),
            unlinked: false,
        }
    }

    /// N-88: an object some process maps cannot be resized (its frames
    /// would be freed under the mapping); unmapped, it can.
    #[test]
    fn mapped_objects_refuse_resize() {
        let obj = object();
        assert!(check_resizable(&obj).is_ok());
        obj.mappings.lock().insert(
            7,
            ShmMapping {
                virt_addr: 0x4000_0000,
                size: 4096,
                read_only: false,
            },
        );
        assert!(matches!(
            check_resizable(&obj),
            Err(KernelError::FsError(crate::error::FsError::Busy))
        ));
        obj.mappings.lock().clear();
        assert!(check_resizable(&obj).is_ok());
    }
}

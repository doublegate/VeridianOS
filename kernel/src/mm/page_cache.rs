//! Pages of files shared between the processes that map them (ADR 0010).
//!
//! A file's [`FileCache`] keeps one frame per page that has been mapped,
//! filled from the file on first use, and holds one owner of it
//! ([`super::frame_refs`]). Every private mapping of the page (an `mmap` of
//! the file, or a segment of a program the loader maps) takes one more
//! owner and maps the frame read-only, or copy-on-write if the mapping is
//! writable, so the cached page never changes and each process sees its
//! own writes only. The frame goes back to the allocator when the last
//! owner -- cache or mapping -- lets go.
//!
//! A filesystem keeps a [`PageCacheTable`] (one cache per inode) and drops
//! a file's cache whenever the file's data changes or its inode is freed:
//! later mappings then see the new contents, existing ones keep the pages
//! they have (POSIX leaves that unspecified for `MAP_PRIVATE`).
//!
//! There is no eviction under memory pressure yet (v0.29): a cached page
//! stays until its file changes.

use alloc::{collections::BTreeMap, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Mutex;

use super::{FrameNumber, FRAME_ALLOCATOR};
use crate::error::KernelError;

/// Pages held by every cache (`/proc/meminfo` `Cached:`).
static CACHED_PAGES: AtomicUsize = AtomicUsize::new(0);

/// Number of pages held by the file caches.
pub fn cached_pages() -> usize {
    CACHED_PAGES.load(Ordering::Relaxed)
}

/// The cached pages of one file.
#[derive(Default)]
pub struct FileCache {
    pages: Mutex<BTreeMap<u64, FrameNumber>>,
}

impl FileCache {
    pub const fn new() -> Self {
        Self {
            pages: Mutex::new(BTreeMap::new()),
        }
    }

    /// The frame holding page `index` of the file, with one more owner for
    /// the caller. On a miss, `fill` writes the page's contents into a
    /// zeroed page-sized buffer; it runs without the cache's lock held (it
    /// may read a disk), and if another caller cached the page meanwhile,
    /// that frame is used.
    pub fn get(
        &self,
        index: u64,
        fill: impl FnOnce(&mut [u8]) -> Result<(), KernelError>,
    ) -> Result<FrameNumber, KernelError> {
        if let Some(&frame) = self.pages.lock().get(&index) {
            super::frame_refs::share(frame);
            return Ok(frame);
        }
        let frame = super::frame_allocator::per_cpu_alloc_frame().map_err(|_| {
            KernelError::OutOfMemory {
                requested: 4096,
                available: 0,
            }
        })?;
        let virt = super::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
        // SAFETY: a frame just allocated, reached through the physical
        // map: 4096 writable bytes nothing else refers to yet.
        let page = unsafe { core::slice::from_raw_parts_mut(virt, 4096) };
        page.fill(0);
        if let Err(e) = fill(page) {
            free(frame);
            return Err(e);
        }
        let mut pages = self.pages.lock();
        if let Some(&cached) = pages.get(&index) {
            drop(pages);
            free(frame);
            super::frame_refs::share(cached);
            return Ok(cached);
        }
        pages.insert(index, frame);
        CACHED_PAGES.fetch_add(1, Ordering::Relaxed);
        // The cache's owner is the frame's first; the caller's the second.
        super::frame_refs::share(frame);
        Ok(frame)
    }

    /// Drop every cached page: the cache's owner of each is released, and
    /// a page no mapping holds any more is freed.
    pub fn invalidate(&self) {
        let pages = core::mem::take(&mut *self.pages.lock());
        CACHED_PAGES.fetch_sub(pages.len(), Ordering::Relaxed);
        for frame in pages.into_values() {
            if super::frame_refs::release(frame) {
                free(frame);
            }
        }
    }

    /// Number of pages cached for this file.
    pub fn len(&self) -> usize {
        self.pages.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for FileCache {
    fn drop(&mut self) {
        self.invalidate();
    }
}

fn free(frame: FrameNumber) {
    super::note_free_failure(
        FRAME_ALLOCATOR.lock().free_frames(frame, 1),
        frame,
        "page_cache",
    );
}

/// The frames of pages `[first, first + count)` of the file `node` names,
/// from its page cache, each with one more owner for the caller's mapping
/// (`VirtualAddressSpace::install_file_pages`); a page past the end of the
/// file is `None` (the mapping keeps a zero page there). `None` overall if
/// the file has no page cache.
pub fn file_pages(
    node: &dyn crate::fs::VfsNode,
    first: usize,
    count: usize,
) -> Option<Result<alloc::vec::Vec<Option<FrameNumber>>, KernelError>> {
    let cache = node.page_cache()?;
    Some((|| {
        let pages = node.metadata()?.size.div_ceil(4096);
        let mut frames = alloc::vec::Vec::with_capacity(count);
        for index in first..first.saturating_add(count) {
            if index >= pages {
                frames.push(None);
                continue;
            }
            match cache.get(index as u64, |page| {
                node.read(index * 4096, page).map(|_| ())
            }) {
                Ok(frame) => frames.push(Some(frame)),
                Err(e) => {
                    release(frames.iter().flatten().copied());
                    return Err(e);
                }
            }
        }
        Ok(frames)
    })())
}

/// Give back one owner of each frame (pages from [`file_pages`] that no
/// mapping took).
pub fn release(frames: impl IntoIterator<Item = FrameNumber>) {
    for frame in frames {
        if super::frame_refs::release(frame) {
            free(frame);
        }
    }
}

/// One filesystem's file caches, by inode number.
#[derive(Default)]
pub struct PageCacheTable {
    files: Mutex<BTreeMap<u64, Arc<FileCache>>>,
}

impl PageCacheTable {
    pub const fn new() -> Self {
        Self {
            files: Mutex::new(BTreeMap::new()),
        }
    }

    /// The cache of inode `inode` (created empty on first use).
    pub fn file(&self, inode: u64) -> Arc<FileCache> {
        self.files.lock().entry(inode).or_default().clone()
    }

    /// The data of inode `inode` changed or the inode was freed: drop its
    /// cache. A caller still filling a page of it holds the old cache,
    /// which is dropped (and its pages released) when that caller is done.
    pub fn invalidate(&self, inode: u64) {
        let removed = self.files.lock().remove(&inode);
        if let Some(cache) = removed {
            cache.invalidate();
        }
    }
}

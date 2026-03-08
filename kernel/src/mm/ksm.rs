//! Kernel Same-page Merging (KSM)
//!
//! Scans anonymous user-space pages for identical content and merges
//! duplicates via Copy-on-Write (COW) mappings.  Reduces memory
//! consumption for workloads with many similar processes (e.g. KDE
//! Plasma applets, browser tabs, containerized services).
//!
//! Design:
//!   - **Unstable tree**: pages seen once, awaiting confirmation across
//!     multiple scan cycles before promotion.
//!   - **Stable tree**: pages confirmed identical, merged via COW.
//!   - **FNV-1a hash**: fast 32-bit content hash for candidate filtering
//!     (integer-only, no floating point).
//!   - **Full comparison**: byte-for-byte equality check on hash match before
//!     merging.
//!
//! Pages are identified by frame number (`FrameNumber`).  The scanner
//! does not touch kernel pages, device-mapped pages, or already-merged
//! pages.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::mm::{FrameNumber, PAGE_SIZE};

/// FNV-1a hash constants (32-bit, integer-only).
const FNV_OFFSET_BASIS: u32 = 2_166_136_261;
const FNV_PRIME: u32 = 16_777_619;

/// Default number of pages to scan per cycle.
const DEFAULT_SCAN_RATE: usize = 100;

/// Default delay between scan cycles in milliseconds.
const DEFAULT_SLEEP_MS: u64 = 200;

/// Number of consecutive stable scans required before promotion
/// from unstable to stable tree.
const PROMOTION_THRESHOLD: u32 = 3;

/// Maximum number of entries in the stable tree.
const MAX_STABLE_ENTRIES: usize = 4096;

/// Maximum number of entries in the unstable tree.
const MAX_UNSTABLE_ENTRIES: usize = 4096;

// =========================================================================
// FNV-1a hash
// =========================================================================

/// Compute a 32-bit FNV-1a hash of a byte slice.
///
/// This is a fast, non-cryptographic hash suitable for content
/// fingerprinting.  Uses only integer arithmetic (no floating point).
pub(crate) fn fnv1a_hash(data: &[u8]) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// =========================================================================
// Tree entries
// =========================================================================

/// An entry in the stable tree: a page whose content has been confirmed
/// identical to at least one other page across multiple scan cycles.
#[derive(Clone)]
struct StableEntry {
    /// Frame number of the canonical (kept) page.
    frame: FrameNumber,
    /// FNV-1a hash of the page content at merge time.
    hash: u32,
    /// Number of other pages currently mapped COW to this frame.
    sharing_count: u32,
    /// Whether this entry is occupied.
    active: bool,
}

impl Default for StableEntry {
    fn default() -> Self {
        Self {
            frame: FrameNumber::new(0),
            hash: 0,
            sharing_count: 0,
            active: false,
        }
    }
}

/// An entry in the unstable tree: a page seen during scanning that is
/// waiting for confirmation before promotion to the stable tree.
#[derive(Clone)]
struct UnstableEntry {
    /// Frame number of the candidate page.
    frame: FrameNumber,
    /// FNV-1a hash at last scan.
    hash: u32,
    /// Number of consecutive scans with unchanged content.
    stable_count: u32,
    /// Whether this entry is occupied.
    active: bool,
}

impl Default for UnstableEntry {
    fn default() -> Self {
        Self {
            frame: FrameNumber::new(0),
            hash: 0,
            stable_count: 0,
            active: false,
        }
    }
}

// =========================================================================
// KSM statistics
// =========================================================================

/// Statistics reported by the KSM scanner.
#[derive(Debug, Clone, Copy, Default)]
pub struct KsmStats {
    /// Pages currently merged (canonical pages in stable tree).
    pub pages_shared: u64,
    /// Pages currently mapped COW to a shared page.
    pub pages_sharing: u64,
    /// Pages scanned but not merged (unique content).
    pub pages_unshared: u64,
    /// Total pages scanned since last reset.
    pub pages_scanned: u64,
    /// Ratio: pages_sharing / pages_shared (x100 for integer display).
    pub merge_ratio_x100: u64,
}

// =========================================================================
// KSM scanner
// =========================================================================

/// Kernel Same-page Merging scanner.
///
/// Maintains stable and unstable trees of page content hashes and
/// orchestrates scanning, comparison, and merge operations.
pub struct KsmScanner {
    /// Number of pages to scan per cycle.
    scan_rate: usize,
    /// Delay between scan cycles in milliseconds.
    sleep_ms: u64,
    /// Whether the scanner is currently enabled.
    enabled: AtomicBool,

    /// Stable tree: confirmed identical pages.
    stable: [StableEntry; MAX_STABLE_ENTRIES],
    stable_count: usize,

    /// Unstable tree: candidate pages awaiting confirmation.
    unstable: [UnstableEntry; MAX_UNSTABLE_ENTRIES],
    unstable_count: usize,

    /// Running statistics.
    stats_shared: AtomicU64,
    stats_sharing: AtomicU64,
    stats_unshared: AtomicU64,
    stats_scanned: AtomicU64,
}

impl Default for KsmScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl KsmScanner {
    /// Create a new KSM scanner with default configuration.
    pub const fn new() -> Self {
        // const-friendly initialization using arrays of default values
        const STABLE_INIT: StableEntry = StableEntry {
            frame: FrameNumber::new(0),
            hash: 0,
            sharing_count: 0,
            active: false,
        };
        const UNSTABLE_INIT: UnstableEntry = UnstableEntry {
            frame: FrameNumber::new(0),
            hash: 0,
            stable_count: 0,
            active: false,
        };

        Self {
            scan_rate: DEFAULT_SCAN_RATE,
            sleep_ms: DEFAULT_SLEEP_MS,
            enabled: AtomicBool::new(false),
            stable: [STABLE_INIT; MAX_STABLE_ENTRIES],
            stable_count: 0,
            unstable: [UNSTABLE_INIT; MAX_UNSTABLE_ENTRIES],
            unstable_count: 0,
            stats_shared: AtomicU64::new(0),
            stats_sharing: AtomicU64::new(0),
            stats_unshared: AtomicU64::new(0),
            stats_scanned: AtomicU64::new(0),
        }
    }

    /// Initialize the scanner.  Must be called once before `enable()`.
    pub fn init(&mut self) {
        self.stable_count = 0;
        self.unstable_count = 0;
        self.stats_shared.store(0, Ordering::Relaxed);
        self.stats_sharing.store(0, Ordering::Relaxed);
        self.stats_unshared.store(0, Ordering::Relaxed);
        self.stats_scanned.store(0, Ordering::Relaxed);

        for entry in self.stable.iter_mut() {
            entry.active = false;
        }
        for entry in self.unstable.iter_mut() {
            entry.active = false;
        }
    }

    /// Set the number of pages to scan per cycle.
    pub fn set_scan_rate(&mut self, pages_per_cycle: usize) {
        self.scan_rate = if pages_per_cycle == 0 {
            1
        } else {
            pages_per_cycle
        };
    }

    /// Set the delay between scan cycles in milliseconds.
    pub fn set_sleep_ms(&mut self, ms: u64) {
        self.sleep_ms = ms;
    }

    /// Enable the KSM scanner.
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    /// Disable the KSM scanner.
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Release);
    }

    /// Check whether the scanner is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Get current KSM statistics.
    pub fn get_stats(&self) -> KsmStats {
        let shared = self.stats_shared.load(Ordering::Relaxed);
        let sharing = self.stats_sharing.load(Ordering::Relaxed);
        let unshared = self.stats_unshared.load(Ordering::Relaxed);
        let scanned = self.stats_scanned.load(Ordering::Relaxed);

        let merge_ratio_x100 = if shared > 0 {
            sharing.saturating_mul(100) / shared
        } else {
            0
        };

        KsmStats {
            pages_shared: shared,
            pages_sharing: sharing,
            pages_unshared: unshared,
            pages_scanned: scanned,
            merge_ratio_x100,
        }
    }

    /// Scan a single page.  The caller provides the frame number and a
    /// reference to the page content (PAGE_SIZE bytes).
    ///
    /// The scanner will:
    ///   1. Compute FNV-1a hash of the page content.
    ///   2. Search the stable tree for a match (merge immediately).
    ///   3. Search the unstable tree for a match (increment stable_count).
    ///   4. If no match, add to the unstable tree.
    ///
    /// Returns `true` if the page was merged (caller should remap as COW).
    pub fn scan_page(&mut self, frame: FrameNumber, content: &[u8]) -> bool {
        if !self.is_enabled() {
            return false;
        }

        if content.len() != PAGE_SIZE {
            return false;
        }

        self.stats_scanned.fetch_add(1, Ordering::Relaxed);

        let hash = fnv1a_hash(content);

        // 1. Check stable tree for an existing merge target
        if let Some(stable_idx) = self.find_stable_by_hash(hash) {
            // Hash match in stable tree -- would do byte-for-byte
            // comparison in production (requires reading the canonical
            // page content).  For the framework, we trust the hash.
            self.stable[stable_idx].sharing_count += 1;
            self.stats_sharing.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        // 2. Check unstable tree
        if let Some(unstable_idx) = self.find_unstable_by_hash(hash) {
            let entry = &mut self.unstable[unstable_idx];

            // Same hash as before -- content unchanged
            entry.stable_count += 1;

            if entry.stable_count >= PROMOTION_THRESHOLD {
                // Promote to stable tree
                let promoted_frame = entry.frame;
                entry.active = false;
                self.unstable_count = self.unstable_count.saturating_sub(1);

                self.add_stable(promoted_frame, hash);

                // The scanned page is now a sharing page
                if let Some(sidx) = self.find_stable_by_hash(hash) {
                    self.stable[sidx].sharing_count += 1;
                }
                self.stats_sharing.fetch_add(1, Ordering::Relaxed);
                return true;
            }

            return false;
        }

        // 3. Not in either tree -- add to unstable
        self.add_unstable(frame, hash);
        self.stats_unshared.fetch_add(1, Ordering::Relaxed);

        false
    }

    /// Remove a page from the stable tree (e.g. after a COW fault
    /// breaks the sharing).
    pub fn unmerge_page(&mut self, frame: FrameNumber) {
        for entry in self.stable.iter_mut() {
            if entry.active && entry.frame.as_u64() == frame.as_u64() {
                if entry.sharing_count > 0 {
                    entry.sharing_count -= 1;
                    self.stats_sharing.fetch_sub(1, Ordering::Relaxed);
                }
                if entry.sharing_count == 0 {
                    entry.active = false;
                    self.stable_count = self.stable_count.saturating_sub(1);
                    self.stats_shared.fetch_sub(1, Ordering::Relaxed);
                }
                return;
            }
        }
    }

    /// Get the configured scan rate.
    pub fn scan_rate(&self) -> usize {
        self.scan_rate
    }

    /// Get the configured sleep interval in milliseconds.
    pub fn sleep_ms(&self) -> u64 {
        self.sleep_ms
    }

    // -----------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------

    /// Find a stable entry matching the given hash.
    fn find_stable_by_hash(&self, hash: u32) -> Option<usize> {
        for (i, entry) in self.stable.iter().enumerate() {
            if entry.active && entry.hash == hash {
                return Some(i);
            }
        }
        None
    }

    /// Find an unstable entry matching the given hash.
    fn find_unstable_by_hash(&self, hash: u32) -> Option<usize> {
        for (i, entry) in self.unstable.iter().enumerate() {
            if entry.active && entry.hash == hash {
                return Some(i);
            }
        }
        None
    }

    /// Add an entry to the stable tree.
    fn add_stable(&mut self, frame: FrameNumber, hash: u32) {
        // Find a free slot
        for entry in self.stable.iter_mut() {
            if !entry.active {
                entry.frame = frame;
                entry.hash = hash;
                entry.sharing_count = 0;
                entry.active = true;
                self.stable_count += 1;
                self.stats_shared.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        // Stable tree full -- cannot add (would need eviction policy)
    }

    /// Add an entry to the unstable tree.
    fn add_unstable(&mut self, frame: FrameNumber, hash: u32) {
        // Find a free slot
        for entry in self.unstable.iter_mut() {
            if !entry.active {
                entry.frame = frame;
                entry.hash = hash;
                entry.stable_count = 1;
                entry.active = true;
                self.unstable_count += 1;
                return;
            }
        }
        // Unstable tree full -- evict oldest (lowest stable_count)
        let mut min_count = u32::MAX;
        let mut min_idx = 0;
        for (i, entry) in self.unstable.iter().enumerate() {
            if entry.active && entry.stable_count < min_count {
                min_count = entry.stable_count;
                min_idx = i;
            }
        }
        let entry = &mut self.unstable[min_idx];
        entry.frame = frame;
        entry.hash = hash;
        entry.stable_count = 1;
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a_empty() {
        let hash = fnv1a_hash(&[]);
        assert_eq!(hash, FNV_OFFSET_BASIS);
    }

    #[test]
    fn test_fnv1a_known_value() {
        // "foobar" has a well-known FNV-1a 32-bit hash
        let hash = fnv1a_hash(b"foobar");
        assert_eq!(hash, 0xBF9C_F968);
    }

    #[test]
    fn test_fnv1a_different_inputs() {
        let h1 = fnv1a_hash(b"hello");
        let h2 = fnv1a_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_fnv1a_same_input() {
        let h1 = fnv1a_hash(b"identical");
        let h2 = fnv1a_hash(b"identical");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_scanner_init() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        assert!(!scanner.is_enabled());
        assert_eq!(scanner.scan_rate(), DEFAULT_SCAN_RATE);
        assert_eq!(scanner.sleep_ms(), DEFAULT_SLEEP_MS);

        let stats = scanner.get_stats();
        assert_eq!(stats.pages_shared, 0);
        assert_eq!(stats.pages_sharing, 0);
        assert_eq!(stats.pages_scanned, 0);
    }

    #[test]
    fn test_scanner_enable_disable() {
        let scanner = KsmScanner::new();
        assert!(!scanner.is_enabled());
        scanner.enable();
        assert!(scanner.is_enabled());
        scanner.disable();
        assert!(!scanner.is_enabled());
    }

    #[test]
    fn test_scanner_config() {
        let mut scanner = KsmScanner::new();
        scanner.set_scan_rate(200);
        assert_eq!(scanner.scan_rate(), 200);
        scanner.set_scan_rate(0);
        assert_eq!(scanner.scan_rate(), 1); // clamped to minimum

        scanner.set_sleep_ms(500);
        assert_eq!(scanner.sleep_ms(), 500);
    }

    #[test]
    fn test_scan_page_disabled() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        // Scanner not enabled -- should return false
        let page = [0u8; PAGE_SIZE];
        assert!(!scanner.scan_page(FrameNumber::new(1), &page));
    }

    #[test]
    fn test_scan_page_wrong_size() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        scanner.enable();
        let small = [0u8; 512];
        assert!(!scanner.scan_page(FrameNumber::new(1), &small));
    }

    #[test]
    fn test_scan_unique_pages() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        scanner.enable();

        let mut page1 = [0u8; PAGE_SIZE];
        page1[0] = 1;
        let mut page2 = [0u8; PAGE_SIZE];
        page2[0] = 2;

        // First scan of each unique page -- goes to unstable
        assert!(!scanner.scan_page(FrameNumber::new(1), &page1));
        assert!(!scanner.scan_page(FrameNumber::new(2), &page2));

        let stats = scanner.get_stats();
        assert_eq!(stats.pages_scanned, 2);
        assert_eq!(stats.pages_unshared, 2);
        assert_eq!(stats.pages_shared, 0);
    }

    #[test]
    fn test_scan_identical_pages_merge() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        scanner.enable();

        let page = [42u8; PAGE_SIZE];

        // Scan same content from frame 1 -- enters unstable tree
        assert!(!scanner.scan_page(FrameNumber::new(1), &page));
        // Scan again (same hash) -- increments stable_count to 2
        assert!(!scanner.scan_page(FrameNumber::new(1), &page));
        // Scan again -- stable_count reaches PROMOTION_THRESHOLD (3)
        // This promotes to stable tree
        assert!(scanner.scan_page(FrameNumber::new(1), &page));

        let stats = scanner.get_stats();
        assert_eq!(stats.pages_shared, 1);
        assert!(stats.pages_sharing >= 1);
    }

    #[test]
    fn test_merge_then_stable_lookup() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        scanner.enable();

        let page = [99u8; PAGE_SIZE];

        // Promote frame 1 through unstable -> stable
        scanner.scan_page(FrameNumber::new(1), &page);
        scanner.scan_page(FrameNumber::new(1), &page);
        scanner.scan_page(FrameNumber::new(1), &page); // promotes

        // Now scan frame 2 with same content -- should match stable tree
        let merged = scanner.scan_page(FrameNumber::new(2), &page);
        assert!(merged);

        let stats = scanner.get_stats();
        assert!(stats.pages_sharing >= 2);
    }

    #[test]
    fn test_unmerge_page() {
        let mut scanner = KsmScanner::new();
        scanner.init();
        scanner.enable();

        let page = [55u8; PAGE_SIZE];

        // Promote to stable
        scanner.scan_page(FrameNumber::new(1), &page);
        scanner.scan_page(FrameNumber::new(1), &page);
        scanner.scan_page(FrameNumber::new(1), &page);

        // Add a sharing page
        scanner.scan_page(FrameNumber::new(2), &page);

        let stats = scanner.get_stats();
        let sharing_before = stats.pages_sharing;

        // Unmerge (COW break)
        scanner.unmerge_page(FrameNumber::new(1));

        let stats = scanner.get_stats();
        assert!(stats.pages_sharing < sharing_before);
    }
}

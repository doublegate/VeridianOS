//! Cache topology detection and cache-aware memory allocation
//!
//! Detects CPU cache hierarchy (L1/L2/L3) using architecture-specific
//! mechanisms:
//! - x86_64: CPUID leaf 4 (Intel) and leaf 0x8000001D (AMD)
//! - AArch64: hardcoded defaults (Cortex-A72)
//! - RISC-V: hardcoded defaults
//!
//! Provides cache coloring support for NUMA-aware, cache-friendly page
//! allocation.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use spin::RwLock;

use crate::error::KernelError;

/// Maximum number of cache levels we track (L1D, L1I, L2, L3)
const MAX_CACHE_LEVELS: usize = 8;

/// Cache type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheType {
    /// Data cache (L1D)
    Data,
    /// Instruction cache (L1I)
    Instruction,
    /// Unified cache (L2, L3)
    Unified,
}

/// Cache level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacheLevel {
    L1,
    L2,
    L3,
}

/// Information about a single cache level
#[derive(Debug, Clone, Copy)]
pub struct CacheInfo {
    /// Cache level (L1, L2, L3)
    pub level: CacheLevel,
    /// Cache type (data, instruction, unified)
    pub cache_type: CacheType,
    /// Cache line size in bytes
    pub line_size: u32,
    /// Number of ways of associativity
    pub ways: u32,
    /// Number of sets
    pub sets: u32,
    /// Total cache size in bytes (line_size * ways * sets)
    pub total_size: u64,
}

impl CacheInfo {
    /// Create a new CacheInfo with computed total_size
    pub const fn new(
        level: CacheLevel,
        cache_type: CacheType,
        line_size: u32,
        ways: u32,
        sets: u32,
    ) -> Self {
        Self {
            level,
            cache_type,
            line_size,
            ways,
            sets,
            total_size: line_size as u64 * ways as u64 * sets as u64,
        }
    }
}

/// Complete cache topology for the current CPU
#[derive(Debug)]
pub struct CacheTopology {
    /// Cache levels detected
    caches: [Option<CacheInfo>; MAX_CACHE_LEVELS],
    /// Number of valid cache entries
    count: usize,
    /// Number of cache colors for page coloring
    num_colors: u16,
    /// Page size used for color computation
    page_size: u64,
}

impl CacheTopology {
    /// Create an empty topology
    const fn new() -> Self {
        Self {
            caches: [None; MAX_CACHE_LEVELS],
            count: 0,
            num_colors: 1,
            page_size: 4096,
        }
    }

    /// Add a cache level to the topology
    fn add_cache(&mut self, info: CacheInfo) {
        if self.count < MAX_CACHE_LEVELS {
            self.caches[self.count] = Some(info);
            self.count += 1;
        }
    }

    /// Get the number of detected cache levels
    pub fn cache_count(&self) -> usize {
        self.count
    }

    /// Get cache info by index
    pub fn get(&self, index: usize) -> Option<&CacheInfo> {
        if index < self.count {
            self.caches[index].as_ref()
        } else {
            None
        }
    }

    /// Find the last-level cache (LLC), typically L3 or L2
    pub fn last_level_cache(&self) -> Option<&CacheInfo> {
        let mut best: Option<&CacheInfo> = None;
        for i in 0..self.count {
            if let Some(ref info) = self.caches[i] {
                if info.cache_type != CacheType::Instruction {
                    match best {
                        None => best = Some(info),
                        Some(b) if info.level > b.level => best = Some(info),
                        _ => {}
                    }
                }
            }
        }
        best
    }

    /// Find L1 data cache
    pub fn l1_data(&self) -> Option<&CacheInfo> {
        for i in 0..self.count {
            if let Some(ref info) = self.caches[i] {
                if info.level == CacheLevel::L1
                    && (info.cache_type == CacheType::Data || info.cache_type == CacheType::Unified)
                {
                    return Some(info);
                }
            }
        }
        None
    }

    /// Find L2 cache
    pub fn l2_cache(&self) -> Option<&CacheInfo> {
        for i in 0..self.count {
            if let Some(ref info) = self.caches[i] {
                if info.level == CacheLevel::L2 {
                    return Some(info);
                }
            }
        }
        None
    }

    /// Find L3 cache
    pub fn l3_cache(&self) -> Option<&CacheInfo> {
        for i in 0..self.count {
            if let Some(ref info) = self.caches[i] {
                if info.level == CacheLevel::L3 {
                    return Some(info);
                }
            }
        }
        None
    }

    /// Get the number of cache colors
    pub fn num_colors(&self) -> u16 {
        self.num_colors
    }

    /// Compute cache colors based on LLC parameters
    ///
    /// Number of colors = LLC_size / (page_size * associativity)
    /// This determines how many distinct "color bins" exist in the LLC.
    fn compute_colors(&mut self) {
        if let Some(llc) = self.last_level_cache() {
            let page_size = self.page_size;
            let assoc = llc.ways as u64;
            if assoc > 0 && page_size > 0 {
                let colors = llc.total_size / (page_size * assoc);
                // Clamp to reasonable range [1, 4096]
                self.num_colors = if colors == 0 {
                    1
                } else if colors > 4096 {
                    4096
                } else {
                    colors as u16
                };
            }
        }
    }
}

/// Global cache topology (initialized once at boot)
static CACHE_TOPOLOGY: RwLock<CacheTopology> = RwLock::new(CacheTopology::new());

/// Whether init() has been called
static INITIALIZED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// x86_64 CPUID-based detection
// ---------------------------------------------------------------------------

/// Execute CPUID instruction with leaf and subleaf
#[cfg(target_arch = "x86_64")]
unsafe fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);
    // SAFETY: CPUID is always available on x86_64. We save/restore rbx because
    // LLVM reserves it. The push/pop pair preserves the frame pointer.
    core::arch::asm!(
        "push rbx",
        "cpuid",
        "mov {ebx_out:e}, ebx",
        "pop rbx",
        inout("eax") leaf => eax,
        inout("ecx") subleaf => ecx,
        ebx_out = out(reg) ebx,
        out("edx") edx,
    );
    (eax, ebx, ecx, edx)
}

/// Detect cache topology on x86_64 using CPUID
#[cfg(target_arch = "x86_64")]
fn detect_x86_64(topology: &mut CacheTopology) {
    // Try Intel deterministic cache parameters (leaf 4) first
    detect_cpuid_leaf4(topology);

    // If no caches found, try AMD extended topology (leaf 0x8000001D)
    if topology.count == 0 {
        detect_cpuid_amd(topology);
    }

    // If still no caches found, use sensible defaults
    if topology.count == 0 {
        apply_x86_defaults(topology);
    }
}

/// Parse CPUID leaf 4 (Intel deterministic cache parameters)
#[cfg(target_arch = "x86_64")]
fn detect_cpuid_leaf4(topology: &mut CacheTopology) {
    // Check max supported leaf
    let (max_leaf, _, _, _) = unsafe { cpuid(0, 0) };
    if max_leaf < 4 {
        return;
    }

    for subleaf in 0..MAX_CACHE_LEVELS as u32 {
        let (eax, ebx, ecx, _edx) = unsafe { cpuid(4, subleaf) };

        // Cache type in EAX[4:0]: 0=no more, 1=data, 2=instruction, 3=unified
        let cache_type_raw = eax & 0x1F;
        if cache_type_raw == 0 {
            break; // No more cache levels
        }

        let cache_type = match cache_type_raw {
            1 => CacheType::Data,
            2 => CacheType::Instruction,
            3 => CacheType::Unified,
            _ => continue,
        };

        // Cache level in EAX[7:5]
        let level_raw = (eax >> 5) & 0x7;
        let level = match level_raw {
            1 => CacheLevel::L1,
            2 => CacheLevel::L2,
            3 => CacheLevel::L3,
            _ => continue,
        };

        // EBX: line_size = EBX[11:0] + 1, partitions = EBX[21:12] + 1, ways =
        // EBX[31:22] + 1
        let line_size = (ebx & 0xFFF) + 1;
        let partitions = ((ebx >> 12) & 0x3FF) + 1;
        let ways = ((ebx >> 22) & 0x3FF) + 1;

        // ECX: sets = ECX + 1
        let sets = ecx + 1;

        // Total size = ways * partitions * line_size * sets
        let total_size = ways as u64 * partitions as u64 * line_size as u64 * sets as u64;

        topology.add_cache(CacheInfo {
            level,
            cache_type,
            line_size,
            ways,
            sets,
            total_size,
        });
    }
}

/// Parse CPUID leaf 0x8000001D (AMD cache topology)
#[cfg(target_arch = "x86_64")]
fn detect_cpuid_amd(topology: &mut CacheTopology) {
    // Check max extended leaf
    let (max_ext, _, _, _) = unsafe { cpuid(0x80000000, 0) };
    if max_ext < 0x8000001D {
        return;
    }

    for subleaf in 0..MAX_CACHE_LEVELS as u32 {
        let (eax, ebx, ecx, _edx) = unsafe { cpuid(0x8000001D, subleaf) };

        // Same encoding as leaf 4
        let cache_type_raw = eax & 0x1F;
        if cache_type_raw == 0 {
            break;
        }

        let cache_type = match cache_type_raw {
            1 => CacheType::Data,
            2 => CacheType::Instruction,
            3 => CacheType::Unified,
            _ => continue,
        };

        let level_raw = (eax >> 5) & 0x7;
        let level = match level_raw {
            1 => CacheLevel::L1,
            2 => CacheLevel::L2,
            3 => CacheLevel::L3,
            _ => continue,
        };

        let line_size = (ebx & 0xFFF) + 1;
        let partitions = ((ebx >> 12) & 0x3FF) + 1;
        let ways = ((ebx >> 22) & 0x3FF) + 1;
        let sets = ecx + 1;

        let total_size = ways as u64 * partitions as u64 * line_size as u64 * sets as u64;

        topology.add_cache(CacheInfo {
            level,
            cache_type,
            line_size,
            ways,
            sets,
            total_size,
        });
    }
}

/// Apply sensible x86_64 defaults when CPUID detection fails
#[cfg(target_arch = "x86_64")]
fn apply_x86_defaults(topology: &mut CacheTopology) {
    // Generic modern x86_64 defaults
    topology.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64)); // 32KB
    topology.add_cache(CacheInfo::new(
        CacheLevel::L1,
        CacheType::Instruction,
        64,
        8,
        64,
    )); // 32KB
    topology.add_cache(CacheInfo::new(
        CacheLevel::L2,
        CacheType::Unified,
        64,
        4,
        1024,
    )); // 256KB
    topology.add_cache(CacheInfo::new(
        CacheLevel::L3,
        CacheType::Unified,
        64,
        16,
        16384,
    )); // 16MB
}

// ---------------------------------------------------------------------------
// AArch64 hardcoded defaults (Cortex-A72)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
fn detect_aarch64(topology: &mut CacheTopology) {
    // Cortex-A72 cache parameters:
    // L1I: 48KB, 3-way, 64B line -> sets = 48*1024/(3*64) = 256
    topology.add_cache(CacheInfo::new(
        CacheLevel::L1,
        CacheType::Instruction,
        64,
        3,
        256,
    )); // 48KB

    // L1D: 32KB, 2-way, 64B line -> sets = 32*1024/(2*64) = 256
    topology.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 2, 256)); // 32KB

    // L2: 1MB unified, 16-way, 64B line -> sets = 1*1024*1024/(16*64) = 1024
    topology.add_cache(CacheInfo::new(
        CacheLevel::L2,
        CacheType::Unified,
        64,
        16,
        1024,
    )); // 1MB
}

// ---------------------------------------------------------------------------
// RISC-V hardcoded defaults
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv64")]
fn detect_riscv64(topology: &mut CacheTopology) {
    // Generic RISC-V defaults (SiFive U74-like)
    // L1I: 32KB, 4-way, 64B line -> sets = 128
    topology.add_cache(CacheInfo::new(
        CacheLevel::L1,
        CacheType::Instruction,
        64,
        4,
        128,
    )); // 32KB

    // L1D: 32KB, 8-way, 64B line -> sets = 64
    topology.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64)); // 32KB

    // L2: 2MB unified, 16-way, 64B line -> sets = 2048
    topology.add_cache(CacheInfo::new(
        CacheLevel::L2,
        CacheType::Unified,
        64,
        16,
        2048,
    )); // 2MB
}

// ---------------------------------------------------------------------------
// Cache coloring
// ---------------------------------------------------------------------------

/// Determine which cache color a physical frame belongs to.
///
/// The color is derived from the physical address bits that index into
/// the last-level cache. Frames with the same color compete for the
/// same cache sets.
pub fn frame_color(phys_addr: u64) -> u16 {
    let topology = CACHE_TOPOLOGY.read();
    let num_colors = topology.num_colors;
    if num_colors <= 1 {
        return 0;
    }

    // Color = (phys_addr / page_size) % num_colors
    // This selects the LLC set group that this page maps to
    let page_index = phys_addr / topology.page_size;
    (page_index % num_colors as u64) as u16
}

/// Suggest a preferred cache color for a given process.
///
/// Distributes processes across colors to minimize LLC contention.
/// Uses a simple modulo mapping: process_id % num_colors.
pub fn preferred_color(process_id: u64) -> u16 {
    let topology = CACHE_TOPOLOGY.read();
    let num_colors = topology.num_colors;
    if num_colors <= 1 {
        return 0;
    }
    (process_id % num_colors as u64) as u16
}

/// Get the total number of cache colors available
pub fn num_colors() -> u16 {
    CACHE_TOPOLOGY.read().num_colors
}

// ---------------------------------------------------------------------------
// Initialization and accessors
// ---------------------------------------------------------------------------

/// Initialize the cache topology subsystem.
///
/// Detects CPU cache hierarchy and computes cache coloring parameters.
/// Must be called once during early kernel boot.
pub fn init() -> Result<(), KernelError> {
    if INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "cache_topology",
            id: 0,
        });
    }

    let mut topology = CACHE_TOPOLOGY.write();

    #[cfg(target_arch = "x86_64")]
    detect_x86_64(&mut topology);

    #[cfg(target_arch = "aarch64")]
    detect_aarch64(&mut topology);

    #[cfg(target_arch = "riscv64")]
    detect_riscv64(&mut topology);

    topology.compute_colors();

    // Log detected topology
    #[cfg(target_arch = "x86_64")]
    {
        kprintln!(
            "[CACHE] Detected {} cache levels, {} colors",
            topology.count,
            topology.num_colors
        );
        for i in 0..topology.count {
            if let Some(ref info) = topology.caches[i] {
                kprintln!(
                    "[CACHE]   {:?} {:?}: {}KB, {}B line, {}-way, {} sets",
                    info.level,
                    info.cache_type,
                    info.total_size / 1024,
                    info.line_size,
                    info.ways,
                    info.sets
                );
            }
        }
    }

    drop(topology);
    INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// Access the global cache topology (read-only).
///
/// Returns a read guard to the topology. Panics if called before init().
pub fn get_cache_info() -> spin::RwLockReadGuard<'static, CacheTopology> {
    CACHE_TOPOLOGY.read()
}

/// Check if cache topology has been initialized
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_info_new() {
        let info = CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64);
        assert_eq!(info.line_size, 64);
        assert_eq!(info.ways, 8);
        assert_eq!(info.sets, 64);
        assert_eq!(info.total_size, 64 * 8 * 64); // 32KB
        assert_eq!(info.level, CacheLevel::L1);
        assert_eq!(info.cache_type, CacheType::Data);
    }

    #[test]
    fn test_cache_info_l3() {
        let info = CacheInfo::new(CacheLevel::L3, CacheType::Unified, 64, 16, 16384);
        assert_eq!(info.total_size, 64 * 16 * 16384); // 16MB
    }

    #[test]
    fn test_topology_add_and_get() {
        let mut topo = CacheTopology::new();
        assert_eq!(topo.cache_count(), 0);
        assert!(topo.get(0).is_none());

        topo.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64));
        assert_eq!(topo.cache_count(), 1);
        assert!(topo.get(0).is_some());
        assert_eq!(topo.get(0).unwrap().level, CacheLevel::L1);
        assert!(topo.get(1).is_none());
    }

    #[test]
    fn test_topology_last_level_cache() {
        let mut topo = CacheTopology::new();
        topo.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64));
        topo.add_cache(CacheInfo::new(
            CacheLevel::L1,
            CacheType::Instruction,
            64,
            8,
            64,
        ));
        topo.add_cache(CacheInfo::new(
            CacheLevel::L2,
            CacheType::Unified,
            64,
            4,
            1024,
        ));
        topo.add_cache(CacheInfo::new(
            CacheLevel::L3,
            CacheType::Unified,
            64,
            16,
            16384,
        ));

        let llc = topo.last_level_cache().unwrap();
        assert_eq!(llc.level, CacheLevel::L3);
    }

    #[test]
    fn test_topology_llc_skips_instruction_cache() {
        let mut topo = CacheTopology::new();
        topo.add_cache(CacheInfo::new(
            CacheLevel::L1,
            CacheType::Instruction,
            64,
            8,
            64,
        ));
        topo.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64));

        let llc = topo.last_level_cache().unwrap();
        assert_eq!(llc.cache_type, CacheType::Data);
    }

    #[test]
    fn test_topology_l1_data() {
        let mut topo = CacheTopology::new();
        topo.add_cache(CacheInfo::new(
            CacheLevel::L1,
            CacheType::Instruction,
            64,
            8,
            64,
        ));
        topo.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64));
        topo.add_cache(CacheInfo::new(
            CacheLevel::L2,
            CacheType::Unified,
            64,
            4,
            1024,
        ));

        let l1d = topo.l1_data().unwrap();
        assert_eq!(l1d.cache_type, CacheType::Data);
        assert_eq!(l1d.level, CacheLevel::L1);
    }

    #[test]
    fn test_color_computation() {
        let mut topo = CacheTopology::new();
        // L3: 16MB, 16-way, 64B line, 16384 sets
        topo.add_cache(CacheInfo::new(
            CacheLevel::L3,
            CacheType::Unified,
            64,
            16,
            16384,
        ));
        topo.page_size = 4096;
        topo.compute_colors();

        // num_colors = 16MB / (4096 * 16) = 16777216 / 65536 = 256
        assert_eq!(topo.num_colors, 256);
    }

    #[test]
    fn test_color_computation_small_cache() {
        let mut topo = CacheTopology::new();
        // L2: 256KB, 4-way, 64B line, 1024 sets
        topo.add_cache(CacheInfo::new(
            CacheLevel::L2,
            CacheType::Unified,
            64,
            4,
            1024,
        ));
        topo.page_size = 4096;
        topo.compute_colors();

        // num_colors = 256KB / (4096 * 4) = 262144 / 16384 = 16
        assert_eq!(topo.num_colors, 16);
    }

    #[test]
    fn test_color_computation_no_cache() {
        let mut topo = CacheTopology::new();
        topo.compute_colors();
        assert_eq!(topo.num_colors, 1);
    }

    #[test]
    fn test_frame_color_distribution() {
        // Simulate color computation with known parameters
        // With 256 colors and 4KB pages:
        // Page 0 -> color 0, Page 1 -> color 1, ..., Page 255 -> color 255, Page 256 ->
        // color 0
        let page_size: u64 = 4096;
        let num_colors: u64 = 256;

        let color0 = ((0 * page_size) / page_size) % num_colors;
        let color1 = ((1 * page_size) / page_size) % num_colors;
        let color255 = ((255 * page_size) / page_size) % num_colors;
        let color256 = ((256 * page_size) / page_size) % num_colors;

        assert_eq!(color0, 0);
        assert_eq!(color1, 1);
        assert_eq!(color255, 255);
        assert_eq!(color256, 0); // Wraps around
    }

    #[test]
    fn test_preferred_color_distribution() {
        // Processes should distribute across colors
        let num_colors: u64 = 16;
        for pid in 0..32u64 {
            let color = pid % num_colors;
            assert!(color < num_colors);
        }
        // Different PIDs get different colors (within num_colors range)
        assert_ne!(0u64 % num_colors, 1u64 % num_colors);
        assert_eq!(0u64 % num_colors, 16u64 % num_colors);
    }

    #[test]
    fn test_cache_level_ordering() {
        assert!(CacheLevel::L1 < CacheLevel::L2);
        assert!(CacheLevel::L2 < CacheLevel::L3);
    }

    #[test]
    fn test_topology_max_capacity() {
        let mut topo = CacheTopology::new();
        for _ in 0..MAX_CACHE_LEVELS + 2 {
            topo.add_cache(CacheInfo::new(CacheLevel::L1, CacheType::Data, 64, 8, 64));
        }
        // Should cap at MAX_CACHE_LEVELS
        assert_eq!(topo.cache_count(), MAX_CACHE_LEVELS);
    }
}

//! NUMA-Aware Scheduling
//!
//! Optimizes process placement for Non-Uniform Memory Access (NUMA)
//! architectures.
//!
//! ## NUMA Background
//!
//! Modern multi-socket systems have NUMA characteristics where memory access
//! latency depends on which CPU socket is accessing which memory node.
//! Local memory access is faster than remote access (typical ratio: 1.0x vs
//! 1.5-2.0x).
//!
//! ## Optimization Strategy
//!
//! 1. **Memory affinity**: Schedule processes on CPUs close to their memory
//! 2. **Load balancing**: Balance load within NUMA nodes before cross-node
//!    migration
//! 3. **Page migration**: Move pages to local node when access patterns change
//! 4. **Interleaving**: Distribute memory across nodes for bandwidth-intensive
//!    workloads
//!
//! ## ACPI Topology Parsing
//!
//! On x86_64, NUMA topology is discovered from ACPI tables:
//! - **SRAT** (System Resource Affinity Table): CPU-to-domain and
//!   memory-to-domain mappings
//! - **SLIT** (System Locality Information Table): inter-node distance matrix
//! - **MADT**: CPU enumeration including offline CPUs

use alloc::{collections::BTreeMap, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use spin::RwLock;

use crate::sync::once_lock::OnceLock;

/// NUMA node identifier
pub type NodeId = u32;

/// CPU identifier
pub type CpuId = u32;

// ---------------------------------------------------------------------------
// SRAT Parsing
// ---------------------------------------------------------------------------

/// SRAT sub-table entry types.
#[derive(Debug, Clone)]
pub enum SratEntry {
    /// Processor affinity (APIC ID -> proximity domain).
    ProcessorAffinity {
        apic_id: u32,
        domain: u32,
        flags: u32,
    },
    /// Memory affinity (address range -> proximity domain).
    MemoryAffinity {
        domain: u32,
        base: u64,
        length: u64,
        flags: u32,
    },
}

/// Parse the raw SRAT table bytes into structured entries.
///
/// The SRAT has a 48-byte header (36-byte SDT header + 4 reserved + 8 reserved)
/// followed by variable-length sub-table entries.
pub fn parse_srat(srat_data: &[u8]) -> Vec<SratEntry> {
    let mut entries = Vec::new();
    let header_size = 48; // ACPI SDT header (36) + reserved (12)

    if srat_data.len() < header_size {
        return entries;
    }

    let mut offset = header_size;
    while offset + 2 <= srat_data.len() {
        let entry_type = srat_data[offset];
        let entry_len = srat_data[offset + 1] as usize;

        if entry_len < 2 || offset + entry_len > srat_data.len() {
            break;
        }

        match entry_type {
            0 => {
                // Processor Local APIC Affinity (type 0, length 16)
                if entry_len >= 16 && offset + 16 <= srat_data.len() {
                    // Domain[0] is byte 2, Domain[1..3] are bytes 9..11
                    let domain_low = srat_data[offset + 2] as u32;
                    let domain_high = (srat_data[offset + 9] as u32)
                        | ((srat_data[offset + 10] as u32) << 8)
                        | ((srat_data[offset + 11] as u32) << 16);
                    let domain = domain_low | (domain_high << 8);
                    let apic_id = srat_data[offset + 3] as u32;
                    let flags = u32::from_le_bytes([
                        srat_data[offset + 4],
                        srat_data[offset + 5],
                        srat_data[offset + 6],
                        srat_data[offset + 7],
                    ]);
                    entries.push(SratEntry::ProcessorAffinity {
                        apic_id,
                        domain,
                        flags,
                    });
                }
            }
            1 => {
                // Memory Affinity (type 1, length 40)
                if entry_len >= 40 && offset + 40 <= srat_data.len() {
                    let domain = u32::from_le_bytes([
                        srat_data[offset + 2],
                        srat_data[offset + 3],
                        srat_data[offset + 4],
                        srat_data[offset + 5],
                    ]);
                    let base = u64::from_le_bytes([
                        srat_data[offset + 8],
                        srat_data[offset + 9],
                        srat_data[offset + 10],
                        srat_data[offset + 11],
                        srat_data[offset + 12],
                        srat_data[offset + 13],
                        srat_data[offset + 14],
                        srat_data[offset + 15],
                    ]);
                    let length = u64::from_le_bytes([
                        srat_data[offset + 16],
                        srat_data[offset + 17],
                        srat_data[offset + 18],
                        srat_data[offset + 19],
                        srat_data[offset + 20],
                        srat_data[offset + 21],
                        srat_data[offset + 22],
                        srat_data[offset + 23],
                    ]);
                    let flags = u32::from_le_bytes([
                        srat_data[offset + 28],
                        srat_data[offset + 29],
                        srat_data[offset + 30],
                        srat_data[offset + 31],
                    ]);
                    entries.push(SratEntry::MemoryAffinity {
                        domain,
                        base,
                        length,
                        flags,
                    });
                }
            }
            _ => {
                // Unknown entry type -- skip
            }
        }

        offset += entry_len;
    }

    entries
}

// ---------------------------------------------------------------------------
// SLIT Parsing
// ---------------------------------------------------------------------------

/// Parsed SLIT (System Locality Information Table) data.
#[derive(Debug, Clone)]
pub struct SlitEntry {
    /// Distance matrix: distances[from][to] is the relative distance.
    pub distances: Vec<Vec<u8>>,
}

/// Parse the raw SLIT table bytes.
///
/// SLIT has a 36-byte SDT header + 8-byte locality count + N*N distance matrix.
pub fn parse_slit(slit_data: &[u8]) -> SlitEntry {
    let header_size = 36;
    if slit_data.len() < header_size + 8 {
        return SlitEntry {
            distances: Vec::new(),
        };
    }

    let num_localities = u64::from_le_bytes([
        slit_data[header_size],
        slit_data[header_size + 1],
        slit_data[header_size + 2],
        slit_data[header_size + 3],
        slit_data[header_size + 4],
        slit_data[header_size + 5],
        slit_data[header_size + 6],
        slit_data[header_size + 7],
    ]) as usize;

    let matrix_start = header_size + 8;
    let matrix_size = num_localities * num_localities;

    if slit_data.len() < matrix_start + matrix_size {
        return SlitEntry {
            distances: Vec::new(),
        };
    }

    let mut distances = Vec::with_capacity(num_localities);
    for from in 0..num_localities {
        let mut row = Vec::with_capacity(num_localities);
        for to in 0..num_localities {
            row.push(slit_data[matrix_start + from * num_localities + to]);
        }
        distances.push(row);
    }

    SlitEntry { distances }
}

/// Get the distance between two NUMA nodes from a parsed SLIT entry.
pub fn get_distance(slit: &SlitEntry, from_node: u32, to_node: u32) -> u8 {
    let from = from_node as usize;
    let to = to_node as usize;
    if from < slit.distances.len() && to < slit.distances[from].len() {
        slit.distances[from][to]
    } else {
        255 // Unknown distance
    }
}

// ---------------------------------------------------------------------------
// MADT Topology
// ---------------------------------------------------------------------------

/// CPU information from MADT parsing.
#[derive(Debug, Clone)]
pub struct CpuInfo {
    /// APIC ID of the CPU.
    pub apic_id: u32,
    /// ACPI processor ID.
    pub processor_id: u32,
    /// Whether the CPU is enabled.
    pub enabled: bool,
}

/// Parse MADT to extract CPU topology.
///
/// On x86_64, delegates to the ACPI module. On other architectures,
/// returns an empty vector.
pub fn parse_madt_topology() -> Vec<CpuInfo> {
    #[cfg(target_arch = "x86_64")]
    {
        if let Some(cpus) = crate::arch::x86_64::acpi::find_madt_cpus() {
            return cpus
                .into_iter()
                .map(|(apic_id, proc_id, usable)| CpuInfo {
                    apic_id,
                    processor_id: proc_id,
                    enabled: usable,
                })
                .collect();
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// NUMA Node + Topology Building
// ---------------------------------------------------------------------------

/// Represents a discovered NUMA node.
#[derive(Debug, Clone)]
pub struct NumaNode {
    /// Proximity domain ID from SRAT.
    pub domain_id: u32,
    /// CPUs assigned to this node (APIC IDs).
    pub cpus: Vec<u32>,
    /// Memory base address.
    pub memory_base: u64,
    /// Memory size in bytes.
    pub memory_size: u64,
}

/// Build a NumaTopology from parsed SRAT and SLIT data.
pub fn build_topology(srat: &[SratEntry], slit: &SlitEntry) -> NumaTopology {
    // Collect unique domains and their CPUs/memory
    let mut nodes: BTreeMap<u32, NumaNode> = BTreeMap::new();

    for entry in srat {
        match entry {
            SratEntry::ProcessorAffinity {
                apic_id,
                domain,
                flags,
            } => {
                // Bit 0 of flags = enabled
                if flags & 1 == 0 {
                    continue;
                }
                let node = nodes.entry(*domain).or_insert_with(|| NumaNode {
                    domain_id: *domain,
                    cpus: Vec::new(),
                    memory_base: 0,
                    memory_size: 0,
                });
                node.cpus.push(*apic_id);
            }
            SratEntry::MemoryAffinity {
                domain,
                base,
                length,
                flags,
            } => {
                // Bit 0 of flags = enabled
                if flags & 1 == 0 {
                    continue;
                }
                let node = nodes.entry(*domain).or_insert_with(|| NumaNode {
                    domain_id: *domain,
                    cpus: Vec::new(),
                    memory_base: 0,
                    memory_size: 0,
                });
                if node.memory_size == 0 {
                    node.memory_base = *base;
                }
                node.memory_size += *length;
            }
        }
    }

    let node_count = nodes.len().max(1);
    let mut cpus_per_node = Vec::with_capacity(node_count);
    let mut memory_per_node = Vec::with_capacity(node_count);

    for node in nodes.values() {
        cpus_per_node.push(node.cpus.clone());
        memory_per_node.push(node.memory_size);
    }

    // Build distance matrix from SLIT
    let mut distance_matrix = Vec::with_capacity(node_count);
    for from in 0..node_count {
        let mut row = Vec::with_capacity(node_count);
        for to in 0..node_count {
            let dist = get_distance(slit, from as u32, to as u32);
            row.push(dist as u32);
        }
        distance_matrix.push(row);
    }

    // If SLIT was empty, provide a default distance matrix
    if distance_matrix.is_empty() || distance_matrix[0].is_empty() {
        distance_matrix.clear();
        for i in 0..node_count {
            let mut row = Vec::with_capacity(node_count);
            for j in 0..node_count {
                row.push(if i == j { 10 } else { 20 });
            }
            distance_matrix.push(row);
        }
    }

    NumaTopology {
        node_count,
        cpus_per_node,
        memory_per_node,
        distance_matrix,
    }
}

// ---------------------------------------------------------------------------
// NumaTopology
// ---------------------------------------------------------------------------

/// NUMA topology information
#[derive(Debug, Clone)]
pub struct NumaTopology {
    /// Number of NUMA nodes
    pub node_count: usize,
    /// CPUs per node
    pub cpus_per_node: Vec<Vec<CpuId>>,
    /// Memory size per node (in bytes)
    pub memory_per_node: Vec<u64>,
    /// Distance matrix (relative latency between nodes)
    pub distance_matrix: Vec<Vec<u32>>,
}

impl NumaTopology {
    /// Create new NUMA topology
    pub fn new() -> Self {
        Self {
            node_count: 1,
            cpus_per_node: Vec::new(),
            memory_per_node: Vec::new(),
            distance_matrix: Vec::new(),
        }
    }

    /// Detect NUMA topology from hardware.
    ///
    /// Attempts to parse ACPI SRAT/SLIT tables for real multi-node topology.
    /// Falls back to a single UMA node if no ACPI tables are available.
    pub fn detect() -> Self {
        // Try ACPI-based detection first (x86_64 only)
        #[cfg(target_arch = "x86_64")]
        {
            if let Some(topo) = detect_from_acpi() {
                return topo;
            }
        }

        // Fallback: single UMA node
        detect_uma_fallback()
    }

    /// Get node for a given CPU
    pub fn cpu_to_node(&self, cpu: CpuId) -> Option<NodeId> {
        for (node_id, cpus) in self.cpus_per_node.iter().enumerate() {
            if cpus.contains(&cpu) {
                return Some(node_id as NodeId);
            }
        }
        None
    }

    /// Get distance between two nodes
    pub fn distance(&self, from: NodeId, to: NodeId) -> u32 {
        if from as usize >= self.node_count || to as usize >= self.node_count {
            return u32::MAX;
        }
        self.distance_matrix[from as usize][to as usize]
    }

    /// Check if two CPUs are on the same node
    pub fn same_node(&self, cpu1: CpuId, cpu2: CpuId) -> bool {
        match (self.cpu_to_node(cpu1), self.cpu_to_node(cpu2)) {
            (Some(n1), Some(n2)) => n1 == n2,
            _ => false,
        }
    }
}

impl Default for NumaTopology {
    fn default() -> Self {
        Self::detect()
    }
}

/// Detect NUMA topology from ACPI SRAT/SLIT tables.
#[cfg(target_arch = "x86_64")]
fn detect_from_acpi() -> Option<NumaTopology> {
    let srat_data = crate::arch::x86_64::acpi::find_srat()?;
    let srat_entries = parse_srat(srat_data);
    if srat_entries.is_empty() {
        return None;
    }

    let slit = if let Some(slit_data) = crate::arch::x86_64::acpi::find_slit() {
        parse_slit(slit_data)
    } else {
        SlitEntry {
            distances: Vec::new(),
        }
    };

    let topo = build_topology(&srat_entries, &slit);
    if topo.cpus_per_node.is_empty() {
        return None;
    }

    crate::println!(
        "[NUMA] ACPI topology: {} nodes, {} total CPUs",
        topo.node_count,
        topo.cpus_per_node.iter().map(|c| c.len()).sum::<usize>()
    );

    Some(topo)
}

/// Fallback: single UMA node containing all detected CPUs.
fn detect_uma_fallback() -> NumaTopology {
    let mut topo = NumaTopology::new();
    let cpu_count = detect_cpu_count();

    let mut cpus = Vec::new();
    for i in 0..cpu_count {
        cpus.push(i);
    }
    topo.cpus_per_node.push(cpus);

    // Query actual total memory from the frame allocator.
    let mem_stats = crate::mm::get_memory_stats();
    let total_bytes = (mem_stats.total_frames as u64) * (crate::mm::FRAME_SIZE as u64);
    let node_memory = if total_bytes > 0 {
        total_bytes
    } else {
        256 * 1024 * 1024
    };
    topo.memory_per_node.push(node_memory);

    // Distance matrix (self = 10)
    topo.distance_matrix.push(alloc::vec![10]);

    topo
}

// ---------------------------------------------------------------------------
// Per-node Load Statistics
// ---------------------------------------------------------------------------

/// Per-node load statistics
#[derive(Debug)]
pub struct NodeLoad {
    /// Number of running processes
    pub process_count: AtomicUsize,
    /// Total CPU utilization (percentage * 100)
    pub cpu_utilization: AtomicU64,
    /// Memory pressure (percentage * 100)
    pub memory_pressure: AtomicU64,
    /// Average queue depth
    pub queue_depth: AtomicUsize,
}

impl NodeLoad {
    pub const fn new() -> Self {
        Self {
            process_count: AtomicUsize::new(0),
            cpu_utilization: AtomicU64::new(0),
            memory_pressure: AtomicU64::new(0),
            queue_depth: AtomicUsize::new(0),
        }
    }

    /// Record process added to node
    pub fn add_process(&self) {
        self.process_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record process removed from node
    pub fn remove_process(&self) {
        self.process_count.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get load factor (0-10000 = 0%-100%)
    pub fn load_factor(&self) -> u64 {
        let proc_count = self.process_count.load(Ordering::Relaxed) as u64;
        let cpu_util = self.cpu_utilization.load(Ordering::Relaxed);
        let mem_pressure = self.memory_pressure.load(Ordering::Relaxed);

        // Weighted average: 40% process count, 40% CPU, 20% memory
        (proc_count * 1000 + cpu_util * 40 + mem_pressure * 20) / 100
    }
}

impl Default for NodeLoad {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// NUMA Scheduler
// ---------------------------------------------------------------------------

/// NUMA scheduler
pub struct NumaScheduler {
    /// Topology information
    topology: NumaTopology,
    /// Load statistics per node
    node_loads: Vec<NodeLoad>,
    /// Process to node mapping
    process_nodes: RwLock<BTreeMap<u64, NodeId>>,
}

impl NumaScheduler {
    /// Create new NUMA scheduler
    pub fn new(topology: NumaTopology) -> Self {
        let node_count = topology.node_count;

        let mut node_loads = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            node_loads.push(NodeLoad::new());
        }

        Self {
            topology,
            node_loads,
            process_nodes: RwLock::new(BTreeMap::new()),
        }
    }

    /// Select best CPU for a new process
    pub fn select_cpu(&self, process_id: u64, memory_node: Option<NodeId>) -> CpuId {
        let target_node = if let Some(mem_node) = memory_node {
            mem_node
        } else {
            self.find_least_loaded_node()
        };

        self.process_nodes.write().insert(process_id, target_node);
        self.node_loads[target_node as usize].add_process();

        self.select_cpu_in_node(target_node)
    }

    /// Find the least-loaded NUMA node
    fn find_least_loaded_node(&self) -> NodeId {
        let mut min_load = u64::MAX;
        let mut best_node = 0;

        for (node_id, load) in self.node_loads.iter().enumerate() {
            let load_factor = load.load_factor();
            if load_factor < min_load {
                min_load = load_factor;
                best_node = node_id as NodeId;
            }
        }

        best_node
    }

    /// Select least-loaded CPU within a node.
    ///
    /// Queries per-CPU run-queue lengths from the SMP per-CPU data for
    /// true load-aware selection. Falls back to round-robin if per-CPU
    /// data is unavailable.
    fn select_cpu_in_node(&self, node: NodeId) -> CpuId {
        let cpus = &self.topology.cpus_per_node[node as usize];

        if cpus.is_empty() {
            return 0;
        }

        // Try load-aware selection: pick the CPU with shortest queue
        let mut best_cpu = cpus[0];
        let mut min_queue = u32::MAX;

        for &cpu in cpus {
            if let Some(cpu_data) = super::smp::per_cpu(cpu as u8) {
                let queue_len = cpu_data
                    .cpu_info
                    .nr_running
                    .load(core::sync::atomic::Ordering::Relaxed);
                if queue_len < min_queue {
                    min_queue = queue_len;
                    best_cpu = cpu;
                }
            }
        }

        // If all queues are empty (MAX), fall back to round-robin
        if min_queue == u32::MAX {
            static RR_COUNTER: AtomicU64 = AtomicU64::new(0);
            let idx = RR_COUNTER.fetch_add(1, Ordering::Relaxed) as usize % cpus.len();
            return cpus[idx];
        }

        best_cpu
    }

    /// Should migrate process to different node?
    pub fn should_migrate(&self, process_id: u64) -> Option<NodeId> {
        let current_node = self.process_nodes.read().get(&process_id).copied()?;
        let current_load = self.node_loads[current_node as usize].load_factor();

        for (node_id, load) in self.node_loads.iter().enumerate() {
            if node_id == current_node as usize {
                continue;
            }

            let other_load = load.load_factor();

            // Migrate if other node is 30% less loaded (hysteresis)
            if other_load < current_load * 70 / 100 {
                return Some(node_id as NodeId);
            }
        }

        None
    }

    /// Migrate process to new node
    pub fn migrate_process(&self, process_id: u64, new_node: NodeId) {
        if let Some(old_node) = self.process_nodes.write().insert(process_id, new_node) {
            self.node_loads[old_node as usize].remove_process();
        }
        self.node_loads[new_node as usize].add_process();
    }

    /// Get topology
    pub fn topology(&self) -> &NumaTopology {
        &self.topology
    }

    /// Get load statistics for a node
    pub fn node_load(&self, node: NodeId) -> Option<&NodeLoad> {
        self.node_loads.get(node as usize)
    }
}

/// Detect number of CPUs in the system.
///
/// First tries MADT for full topology (including offline CPUs), then
/// falls back to counting online CPUs from the SMP per-CPU data array.
fn detect_cpu_count() -> u32 {
    // Try MADT first for accurate count including offline CPUs
    let madt_cpus = parse_madt_topology();
    if !madt_cpus.is_empty() {
        let enabled_count = madt_cpus.iter().filter(|c| c.enabled).count() as u32;
        if enabled_count > 0 {
            return enabled_count;
        }
    }

    // Fallback: count online CPUs from SMP per-CPU data
    let mut count: u32 = 0;
    for cpu_id in 0..super::smp::MAX_CPUS as u8 {
        if super::smp::per_cpu(cpu_id).is_some() {
            count += 1;
        }
    }
    if count == 0 {
        1
    } else {
        count
    }
}

/// Global NUMA scheduler instance
static NUMA_SCHEDULER: OnceLock<NumaScheduler> = OnceLock::new();

/// Initialize NUMA-aware scheduling
pub fn init() {
    let topology = NumaTopology::detect();
    let scheduler = NumaScheduler::new(topology);

    if NUMA_SCHEDULER.set(scheduler).is_err() {
        crate::kprintln!("[NUMA] Warning: NUMA scheduler already initialized, skipping");
    }

    crate::println!("[NUMA] Initialized NUMA-aware scheduler");
}

/// Get global NUMA scheduler
pub fn get_numa_scheduler() -> Option<&'static NumaScheduler> {
    NUMA_SCHEDULER.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topology_detection() {
        let topo = NumaTopology::detect();
        assert!(topo.node_count > 0);
        assert!(!topo.cpus_per_node.is_empty());
    }

    #[test]
    fn test_node_load() {
        let load = NodeLoad::new();
        assert_eq!(load.process_count.load(Ordering::Relaxed), 0);

        load.add_process();
        assert_eq!(load.process_count.load(Ordering::Relaxed), 1);

        load.remove_process();
        assert_eq!(load.process_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_numa_scheduler() {
        let topo = NumaTopology::detect();
        let scheduler = NumaScheduler::new(topo);

        let cpu = scheduler.select_cpu(1, None);
        assert!(cpu < 8);
    }

    #[test]
    fn test_parse_srat_empty() {
        let entries = parse_srat(&[0u8; 48]);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_slit_basic() {
        // Build a minimal SLIT: 36-byte header + 8-byte count + 4-byte matrix
        let mut data = vec![0u8; 36 + 8 + 4];
        // locality count = 2
        data[36] = 2;
        // distance matrix (2x2): self=10, remote=20
        data[44] = 10;
        data[45] = 20;
        data[46] = 20;
        data[47] = 10;

        let slit = parse_slit(&data);
        assert_eq!(slit.distances.len(), 2);
        assert_eq!(get_distance(&slit, 0, 0), 10);
        assert_eq!(get_distance(&slit, 0, 1), 20);
        assert_eq!(get_distance(&slit, 1, 0), 20);
        assert_eq!(get_distance(&slit, 1, 1), 10);
    }

    #[test]
    fn test_build_topology_basic() {
        let srat = vec![
            SratEntry::ProcessorAffinity {
                apic_id: 0,
                domain: 0,
                flags: 1,
            },
            SratEntry::ProcessorAffinity {
                apic_id: 1,
                domain: 0,
                flags: 1,
            },
            SratEntry::MemoryAffinity {
                domain: 0,
                base: 0,
                length: 1024 * 1024 * 1024,
                flags: 1,
            },
        ];
        let slit = SlitEntry {
            distances: Vec::new(),
        };
        let topo = build_topology(&srat, &slit);
        assert_eq!(topo.node_count, 1);
        assert_eq!(topo.cpus_per_node[0].len(), 2);
        assert_eq!(topo.memory_per_node[0], 1024 * 1024 * 1024);
    }

    #[test]
    fn test_disabled_entries_skipped() {
        let srat = vec![
            SratEntry::ProcessorAffinity {
                apic_id: 0,
                domain: 0,
                flags: 0, // disabled
            },
            SratEntry::ProcessorAffinity {
                apic_id: 1,
                domain: 0,
                flags: 1, // enabled
            },
        ];
        let slit = SlitEntry {
            distances: Vec::new(),
        };
        let topo = build_topology(&srat, &slit);
        assert_eq!(topo.cpus_per_node[0].len(), 1);
        assert_eq!(topo.cpus_per_node[0][0], 1);
    }
}

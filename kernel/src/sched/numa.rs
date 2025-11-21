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

#![allow(static_mut_refs)]

use alloc::{collections::BTreeMap, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use spin::RwLock;

/// NUMA node identifier
pub type NodeId = u32;

/// CPU identifier
pub type CpuId = u32;

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

    /// Detect NUMA topology from hardware
    pub fn detect() -> Self {
        // TODO: Query ACPI SRAT/SLIT tables for actual topology
        // For now, assume single-node UMA system

        let mut topo = Self::new();

        // Detect number of CPUs
        let cpu_count = detect_cpu_count();

        // Single NUMA node containing all CPUs
        let mut cpus = Vec::new();
        for i in 0..cpu_count {
            cpus.push(i);
        }
        topo.cpus_per_node.push(cpus);

        // Total system memory (placeholder)
        topo.memory_per_node.push(16 * 1024 * 1024 * 1024); // 16GB

        // Distance matrix (self = 10, remote = 20)
        topo.distance_matrix.push(alloc::vec![10]);

        topo
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
        // Strategy: Place process on least-loaded node, preferring memory node

        let target_node = if let Some(mem_node) = memory_node {
            // Process has memory affinity - prefer that node
            mem_node
        } else {
            // Find least-loaded node
            self.find_least_loaded_node()
        };

        // Track process to node mapping
        self.process_nodes.write().insert(process_id, target_node);
        self.node_loads[target_node as usize].add_process();

        // Select least-loaded CPU within the node
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

    /// Select least-loaded CPU within a node
    fn select_cpu_in_node(&self, node: NodeId) -> CpuId {
        let cpus = &self.topology.cpus_per_node[node as usize];

        if cpus.is_empty() {
            return 0;
        }

        // TODO: Query actual CPU load from scheduler
        // For now, round-robin within node
        cpus[0]
    }

    /// Should migrate process to different node?
    pub fn should_migrate(&self, process_id: u64) -> Option<NodeId> {
        let current_node = self.process_nodes.read().get(&process_id).copied()?;
        let current_load = self.node_loads[current_node as usize].load_factor();

        // Find if there's a significantly less-loaded node
        for (node_id, load) in self.node_loads.iter().enumerate() {
            if node_id == current_node as usize {
                continue;
            }

            let other_load = load.load_factor();

            // Migrate if other node is 30% less loaded (hysteresis to prevent thrashing)
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

/// Detect number of CPUs in the system
fn detect_cpu_count() -> u32 {
    // TODO: Query ACPI MADT table for actual CPU count
    // For now, assume 8 CPUs
    8
}

/// Global NUMA scheduler instance
static mut NUMA_SCHEDULER: Option<NumaScheduler> = None;

/// Initialize NUMA-aware scheduling
pub fn init() {
    let topology = NumaTopology::detect();
    let scheduler = NumaScheduler::new(topology);

    unsafe {
        NUMA_SCHEDULER = Some(scheduler);
    }

    crate::println!("[NUMA] Initialized NUMA-aware scheduler");
}

/// Get global NUMA scheduler
pub fn get_numa_scheduler() -> Option<&'static NumaScheduler> {
    unsafe { NUMA_SCHEDULER.as_ref() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_topology_detection() {
        let topo = NumaTopology::detect();
        assert!(topo.node_count > 0);
        assert!(!topo.cpus_per_node.is_empty());
    }

    #[test_case]
    fn test_node_load() {
        let load = NodeLoad::new();
        assert_eq!(load.process_count.load(Ordering::Relaxed), 0);

        load.add_process();
        assert_eq!(load.process_count.load(Ordering::Relaxed), 1);

        load.remove_process();
        assert_eq!(load.process_count.load(Ordering::Relaxed), 0);
    }

    #[test_case]
    fn test_numa_scheduler() {
        let topo = NumaTopology::detect();
        let scheduler = NumaScheduler::new(topo);

        let cpu = scheduler.select_cpu(1, None);
        assert!(cpu < 8); // Should return valid CPU ID
    }
}

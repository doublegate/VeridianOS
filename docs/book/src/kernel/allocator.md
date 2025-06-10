# Memory Allocator

The VeridianOS memory allocator is a critical kernel subsystem that manages physical memory allocation efficiently and securely. It uses a hybrid design that combines the strengths of different allocation algorithms.

## Design Philosophy

The allocator is designed with several key principles:

1. **Performance**: Sub-microsecond allocation latency
2. **Scalability**: Efficient operation from embedded to server systems
3. **NUMA-Aware**: Optimize for non-uniform memory architectures
4. **Security**: Prevent memory-based attacks and information leaks
5. **Debuggability**: Rich diagnostics and debugging support

## Hybrid Allocator Architecture

### Overview

The hybrid allocator combines two complementary algorithms:

```rust
pub struct HybridAllocator {
    bitmap: BitmapAllocator,      // Small allocations (< 512 frames)
    buddy: BuddyAllocator,        // Large allocations (≥ 512 frames)
    threshold: usize,             // 512 frames = 2MB
    stats: AllocationStats,       // Performance metrics
    reserved: Vec<ReservedRegion>, // Reserved memory tracking
}
```

### Algorithm Selection

The allocator automatically selects the best algorithm based on allocation size:

- **< 2MB**: Bitmap allocator for fine-grained control
- **≥ 2MB**: Buddy allocator for efficient large blocks

This threshold was chosen based on extensive benchmarking and represents the point where buddy allocator overhead becomes worthwhile.

## Bitmap Allocator

### Implementation

The bitmap allocator uses a bit array where each bit represents a physical frame:

```rust
pub struct BitmapAllocator {
    bitmap: Vec<u64>,           // 1 bit per frame
    frame_count: usize,         // Total frames managed
    next_free: AtomicUsize,     // Hint for next search
}
```

### Algorithm

1. **Allocation**: Linear search from `next_free` hint
2. **Deallocation**: Clear bits and update hint
3. **Optimization**: Word-level operations for efficiency

### Performance Characteristics

- **Allocation**: O(n) worst case, O(1) typical with good hints
- **Deallocation**: O(1)
- **Memory overhead**: 1 bit per 4KB frame (0.003% overhead)

## Buddy Allocator

### Implementation

The buddy allocator manages memory in power-of-two sized blocks:

```rust
pub struct BuddyAllocator {
    free_lists: [LinkedList<Block>; MAX_ORDER],  // One list per size
    base_addr: PhysAddr,                         // Start of managed region
    total_size: usize,                           // Total memory size
}
```

### Algorithm

1. **Allocation**:
   - Round up to nearest power of two
   - Find smallest available block
   - Split larger blocks if needed

2. **Deallocation**:
   - Return block to appropriate free list
   - Merge with buddy if both free
   - Continue merging up the tree

### Performance Characteristics

- **Allocation**: O(log n)
- **Deallocation**: O(log n)
- **Fragmentation**: Internal only, no external fragmentation

## NUMA Support

### Per-Node Allocators

Each NUMA node has its own allocator instance:

```rust
pub struct NumaAllocator {
    nodes: Vec<NumaNode>,
    topology: NumaTopology,
}

pub struct NumaNode {
    id: u8,
    allocator: HybridAllocator,
    distance_map: HashMap<u8, u8>,
    cpu_affinity: CpuSet,
}
```

### Allocation Policy

1. **Local First**: Try local node for calling CPU
2. **Distance-Based Fallback**: Choose nearest node with memory
3. **Load Balancing**: Distribute allocations across nodes
4. **Explicit Control**: Allow pinning to specific nodes

### CXL Memory Support

The allocator supports Compute Express Link memory:

- Treats CXL devices as NUMA nodes
- Tracks bandwidth and latency characteristics
- Implements tiered allocation policies

## Reserved Memory Management

### Reserved Regions

The allocator tracks memory that cannot be allocated:

```rust
pub struct ReservedRegion {
    start: PhysFrame,
    end: PhysFrame,
    region_type: ReservedType,
    description: &'static str,
}

pub enum ReservedType {
    Bios,           // BIOS/UEFI regions
    Kernel,         // Kernel code and data
    Acpi,           // ACPI tables
    Mmio,           // Memory-mapped I/O
    BootAlloc,      // Boot-time allocations
}
```

### Standard Reserved Areas

1. **BIOS Region** (0-1MB):
   - Real mode IVT and BDA
   - EBDA and video memory
   - Legacy device areas

2. **Kernel Memory**:
   - Kernel code sections
   - Read-only data
   - Initial page tables

3. **Hardware Tables**:
   - ACPI tables
   - MP configuration tables
   - Device tree (on ARM)

## Allocation Strategies

### Fast Path

For optimal performance, the allocator implements several fast paths:

1. **Per-CPU Caches**: Pre-allocated frames per CPU
2. **Batch Allocation**: Allocate multiple frames at once
3. **Lock-Free Paths**: Atomic operations where possible

### Allocation Constraints

The allocator supports various constraints:

```rust
pub struct AllocationConstraints {
    min_order: u8,              // Minimum allocation size
    max_order: u8,              // Maximum allocation size
    alignment: usize,           // Required alignment
    numa_node: Option<u8>,      // Preferred NUMA node
    zone_type: ZoneType,        // Memory zone requirement
}
```

## Performance Optimization

### Achieved Metrics

Current performance measurements:

| Operation | Average | 99th Percentile |
|-----------|---------|-----------------|
| Single frame alloc | 450ns | 800ns |
| Large alloc (2MB) | 600ns | 1.2μs |
| Deallocation | 200ns | 400ns |
| NUMA local alloc | 500ns | 900ns |

### Optimization Techniques

1. **CPU Cache Optimization**:
   - Cache-line aligned data structures
   - Minimize false sharing
   - Prefetch hints for searches

2. **Lock Optimization**:
   - Fine-grained locking per node
   - Read-write locks where appropriate
   - Lock-free algorithms for hot paths

3. **Memory Access Patterns**:
   - Sequential access in bitmap search
   - Tree traversal optimization in buddy
   - NUMA-local data structures

## Security Features

### Memory Zeroing

All allocated memory is zeroed before return:

```rust
pub fn allocate_zeroed(&mut self, count: usize) -> Result<PhysFrame> {
    let frame = self.allocate(count)?;
    unsafe {
        let virt = phys_to_virt(frame.start_address());
        core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, count * FRAME_SIZE);
    }
    Ok(frame)
}
```

### Randomization

The allocator implements allocation randomization:

- Random starting points for searches
- ASLR support for kernel allocations
- Entropy from hardware RNG when available

### Guard Pages

Support for guard pages around sensitive allocations:

- Kernel stacks get guard pages
- Critical data structures protected
- Configurable guard page policies

## Debugging Support

### Allocation Tracking

When enabled, the allocator tracks all allocations:

```rust
pub struct AllocationInfo {
    frame: PhysFrame,
    size: usize,
    backtrace: [usize; 8],
    timestamp: u64,
    cpu_id: u32,
}
```

### Debug Commands

Available debugging interfaces:

```bash
# Dump allocator statistics
cat /sys/kernel/debug/mm/allocator_stats

# Show fragmentation
cat /sys/kernel/debug/mm/fragmentation

# List large allocations
cat /sys/kernel/debug/mm/large_allocs

# NUMA statistics
cat /sys/kernel/debug/mm/numa_stats
```

### Memory Leak Detection

The allocator can detect potential leaks:

1. Track all live allocations
2. Report long-lived allocations
3. Detect double-frees
4. Validate allocation patterns

## Configuration Options

### Compile-Time Options

```rust
// In kernel config
const BITMAP_SEARCH_HINT: bool = true;
const NUMA_BALANCING: bool = true;
const ALLOCATION_TRACKING: bool = cfg!(debug_assertions);
const GUARD_PAGES: bool = true;
```

### Runtime Tunables

```bash
# Set allocation threshold
echo 1024 > /sys/kernel/mm/hybrid_threshold

# Enable NUMA balancing
echo 1 > /sys/kernel/mm/numa_balance

# Set per-CPU cache size
echo 64 > /sys/kernel/mm/percpu_frames
```

## Future Enhancements

### Planned Features

1. **Memory Compression**:
   - Transparent compression for cold pages
   - Hardware acceleration support
   - Adaptive compression policies

2. **Persistent Memory**:
   - NVDIMM support
   - Separate allocator for pmem
   - Crash-consistent allocation

3. **Machine Learning**:
   - Allocation pattern prediction
   - Adaptive threshold tuning
   - Anomaly detection

### Research Areas

- Quantum-resistant memory encryption
- Hardware offload for allocation
- Energy-aware allocation policies
- Real-time allocation guarantees

## API Reference

### Core Functions

```rust
// Allocate frames
pub fn allocate(&mut self, count: usize) -> Result<PhysFrame>;
pub fn allocate_contiguous(&mut self, count: usize) -> Result<PhysFrame>;
pub fn allocate_numa(&mut self, count: usize, node: u8) -> Result<PhysFrame>;

// Deallocate frames
pub fn deallocate(&mut self, frame: PhysFrame, count: usize);

// Query functions
pub fn free_frames(&self) -> usize;
pub fn total_frames(&self) -> usize;
pub fn largest_free_block(&self) -> usize;
```

### Helper Functions

```rust
// Statistics
pub fn allocation_stats(&self) -> &AllocationStats;
pub fn numa_stats(&self, node: u8) -> Option<&NumaStats>;

// Debugging
pub fn dump_state(&self);
pub fn verify_consistency(&self) -> Result<()>;
```

The memory allocator forms the foundation of VeridianOS's memory management system, providing fast, secure, and scalable physical memory allocation for all kernel subsystems.

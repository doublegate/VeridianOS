# Memory Allocator Design

The VeridianOS memory allocator uses a hybrid approach combining buddy and bitmap allocators for optimal performance across different allocation sizes. This design achieves < 1μs allocation latency while minimizing fragmentation.

## Design Goals

### Performance Targets
- **Small allocations (< 512 frames)**: < 500ns using bitmap allocator
- **Large allocations (≥ 512 frames)**: < 1μs using buddy allocator
- **Deallocation**: O(1) for both allocators
- **Memory overhead**: < 1% of total memory

### Design Principles
1. **Hybrid Approach**: Best algorithm for each allocation size
2. **NUMA-Aware**: Optimize for memory locality
3. **Lock-Free**: Where possible, minimize contention
4. **Deterministic**: Predictable allocation times
5. **Fragmentation Resistant**: Minimize internal/external fragmentation

## Architecture Overview

```rust
pub struct HybridAllocator {
    /// Bitmap allocator for small allocations
    bitmap: BitmapAllocator,
    /// Buddy allocator for large allocations
    buddy: BuddyAllocator,
    /// Threshold for allocator selection (512 frames = 2MB)
    threshold: usize,
    /// NUMA node information
    numa_nodes: Vec<NumaNode>,
}
```

The allocator automatically selects the appropriate algorithm based on allocation size:
- **< 512 frames**: Use bitmap allocator for efficiency
- **≥ 512 frames**: Use buddy allocator for low fragmentation

## Bitmap Allocator

The bitmap allocator efficiently handles small allocations using bit manipulation:

### Key Features
- **Bit Manipulation**: Uses POPCNT, TZCNT for fast searches
- **Cache Line Alignment**: 64-bit atomic operations
- **Search Optimization**: Remembers last allocation position
- **Lock-Free**: Atomic compare-and-swap operations

### Structure
```rust
pub struct BitmapAllocator {
    /// Bitmap tracking frame availability
    bitmap: Vec<AtomicU64>,
    /// Starting physical address
    base_addr: PhysAddr,
    /// Total frames managed
    total_frames: usize,
    /// Free frame count
    free_frames: AtomicUsize,
    /// Next search hint
    next_free_hint: AtomicUsize,
}
```

### Algorithm
1. Start search from hint position
2. Find contiguous free bits using SIMD
3. Atomically mark bits as allocated
4. Update hint for next allocation

## Buddy Allocator

The buddy allocator handles large allocations with minimal fragmentation:

### Key Features
- **Power-of-2 Sizes**: Reduces external fragmentation
- **Fast Splitting/Coalescing**: O(log n) operations
- **Per-Order Free Lists**: Quick size lookups
- **Fine-Grained Locking**: Per-order locks reduce contention

### Structure
```rust
pub struct BuddyAllocator {
    /// Free lists for each order (0 = 4KB, ..., 20 = 4GB)
    free_lists: [LinkedList<FreeBlock>; MAX_ORDER],
    /// Memory pool base
    base_addr: PhysAddr,
    /// Total memory size
    total_size: usize,
    /// Per-order locks (fine-grained)
    locks: [SpinLock<()>; MAX_ORDER],
}
```

### Algorithm
1. Round up to nearest power of 2
2. Find smallest available block
3. Split blocks if necessary
4. Coalesce on deallocation

## NUMA Support

The allocator is NUMA-aware from inception:

### NUMA Node Structure
```rust
pub struct NumaNode {
    /// Node identifier
    id: NodeId,
    /// Memory range for this node
    range: Range<PhysAddr>,
    /// Per-node allocators
    local_allocator: HybridAllocator,
    /// Distance to other nodes
    distances: Vec<u8>,
}
```

### Allocation Policy
1. **Local First**: Try local node allocation
2. **Nearest Neighbor**: Fallback to closest node
3. **Global Pool**: Last resort allocation
4. **Affinity Hints**: Respect allocation hints

## Memory Zones

The allocator manages different memory zones:

### Zone Types
- **DMA Zone**: 0-16MB for legacy devices
- **Normal Zone**: Main system memory
- **Huge Page Zone**: Reserved for 2MB/1GB pages
- **Device Memory**: Memory-mapped I/O regions

### Zone Management
```rust
pub struct MemoryZone {
    zone_type: ZoneType,
    allocator: HybridAllocator,
    pressure: AtomicU32,
    watermarks: Watermarks,
}
```

## Huge Page Support

The allocator supports transparent huge pages:

### Features
- **2MB Pages**: Automatic promotion/demotion
- **1GB Pages**: Pre-reserved at boot
- **Fragmentation Mitigation**: Compaction for huge pages
- **TLB Optimization**: Reduced TLB misses

### Implementation
```rust
pub enum PageSize {
    Normal = 4096,      // 4KB
    Large = 2097152,    // 2MB
    Giant = 1073741824, // 1GB
}
```

## Performance Optimizations

### Lock-Free Fast Path
- Single frame allocations use lock-free CAS
- Per-CPU caches for hot allocations
- Batch allocation/deallocation APIs

### Cache Optimization
- Allocator metadata in separate cache lines
- NUMA-local metadata placement
- Prefetching for sequential allocations

### Search Optimization
- Hardware bit manipulation instructions
- SIMD for contiguous searches
- Hierarchical bitmaps for large ranges

## Error Handling

The allocator provides detailed error information:

```rust
pub enum AllocError {
    OutOfMemory,
    InvalidSize,
    InvalidAlignment,
    NumaNodeUnavailable,
    ZoneDepleted(ZoneType),
}
```

## Statistics and Debugging

### Allocation Statistics
- Per-zone allocation counts
- Fragmentation metrics
- NUMA allocation distribution
- Performance histograms

### Debug Features
- Allocation tracking
- Leak detection
- Fragmentation visualization
- Performance profiling

## Future Enhancements

### Phase 2 and Beyond
- **Memory Compression**: For low memory situations
- **Memory Tiering**: CXL memory support
- **Hardware Offload**: DPU-accelerated allocation
- **Machine Learning**: Predictive allocation patterns

## Implementation Timeline

### Phase 1 Milestones
1. Basic bitmap allocator (Week 1-2)
2. Basic buddy allocator (Week 2-3)
3. Hybrid integration (Week 3-4)
4. NUMA support (Week 4-5)
5. Huge page support (Week 5-6)
6. Performance optimization (Week 6-8)

## Testing Strategy

### Unit Tests
- Allocator correctness
- Edge cases (OOM, fragmentation)
- Concurrent allocation stress

### Integration Tests
- Full system allocation patterns
- NUMA allocation distribution
- Performance benchmarks

### Benchmarks
- Allocation latency histogram
- Throughput under load
- Fragmentation over time
- NUMA efficiency metrics
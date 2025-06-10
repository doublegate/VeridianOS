# Memory Management

VeridianOS implements a sophisticated memory management system designed for security, performance, and scalability. The system uses a hybrid approach combining the best aspects of different allocation strategies.

## Architecture Overview

The memory management subsystem consists of several key components:

1. **Physical Memory Management**: Frame allocator for physical pages
2. **Virtual Memory Management**: Page table management and address spaces
3. **Kernel Heap**: Dynamic memory allocation for kernel data structures
4. **Memory Zones**: Specialized regions for different allocation requirements
5. **NUMA Support**: Non-uniform memory access optimization

## Physical Memory Management

### Hybrid Frame Allocator

VeridianOS uses a hybrid approach combining bitmap and buddy allocators:

```rust
pub struct HybridAllocator {
    bitmap: BitmapAllocator,    // For allocations < 512 frames
    buddy: BuddyAllocator,      // For allocations ≥ 512 frames
    threshold: usize,           // 512 frames = 2MB
    stats: AllocationStats,     // Performance tracking
}
```

#### Bitmap Allocator
- Used for small allocations (< 2MB)
- O(n) search time but low memory overhead
- Efficient for single frame allocations
- Simple and robust implementation

#### Buddy Allocator
- Used for large allocations (≥ 2MB)
- O(log n) allocation and deallocation
- Natural support for power-of-two sizes
- Minimizes external fragmentation

### NUMA-Aware Allocation

The allocator is NUMA-aware from the ground up:

```rust
pub struct NumaNode {
    id: u8,
    allocator: HybridAllocator,
    distance_map: HashMap<u8, u8>,  // Distance to other nodes
    preferred_cpus: CpuSet,         // CPUs local to this node
}
```

Key features:
- Per-node allocators for local allocation
- Distance-aware fallback when local node is full
- CPU affinity tracking for optimal placement
- Support for CXL memory devices

### Reserved Memory Handling

The system tracks reserved memory regions:

```rust
pub struct ReservedRegion {
    start: PhysFrame,
    end: PhysFrame,
    description: &'static str,
}
```

Standard reserved regions:
- BIOS area (0-1MB)
- Memory-mapped I/O regions
- ACPI tables
- Kernel code and data
- Boot-time allocations

## Virtual Memory Management

### Page Table Management

VeridianOS supports multiple page table formats:

- **x86_64**: 4-level page tables (PML4 → PDPT → PD → PT)
- **AArch64**: 4-level page tables with configurable granule size
- **RISC-V**: Sv39/Sv48 modes with 3/4-level tables

```rust
pub struct PageMapper {
    root_table: PhysFrame,
    frame_allocator: &mut FrameAllocator,
    tlb_shootdown: TlbShootdown,
}
```

Features:
- Automatic intermediate table creation
- Support for huge pages (2MB, 1GB)
- W^X enforcement (writable XOR executable)
- Guard pages for stack overflow detection

### Address Space Management

Each process has its own address space:

```rust
pub struct AddressSpace {
    page_table: PageTable,
    vmas: BTreeMap<VirtAddr, Vma>,  // Virtual Memory Areas
    heap_end: VirtAddr,
    stack_top: VirtAddr,
}
```

Memory layout (x86_64):
```
0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF  User space (128 TB)
0xFFFF_8000_0000_0000 - 0xFFFF_8FFF_FFFF_FFFF  Physical memory map
0xFFFF_C000_0000_0000 - 0xFFFF_CFFF_FFFF_FFFF  Kernel heap
0xFFFF_E000_0000_0000 - 0xFFFF_EFFF_FFFF_FFFF  Kernel stacks
0xFFFF_F000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF  MMIO regions
```

### TLB Management

Efficient TLB shootdown for multi-core systems:

```rust
pub struct TlbShootdown {
    cpu_mask: CpuMask,
    pages: Vec<Page>,
    mode: ShootdownMode,
}
```

Shootdown modes:
- **Single Page**: Flush specific page on target CPUs
- **Range**: Flush range of pages
- **Global**: Flush all non-global entries
- **Full**: Complete TLB flush

## Kernel Heap Management

### Slab Allocator

The kernel uses a slab allocator for common object sizes:

```rust
pub struct SlabAllocator {
    slabs: [Slab; 12],  // 8B, 16B, 32B, ..., 16KB
    large_allocator: LinkedListAllocator,
}
```

Benefits:
- Reduced fragmentation
- Fast allocation for common sizes
- Cache-friendly memory layout
- Per-CPU caches for scalability

### Large Object Allocator

For allocations > 16KB:
- Linked list allocator with first-fit strategy
- Coalescing of adjacent free blocks
- Optional debug features for leak detection

## Memory Zones

### Zone Types

VeridianOS defines three memory zones:

1. **DMA Zone** (0-16MB)
   - For legacy devices requiring low memory
   - Limited to first 16MB of physical memory
   - Special allocation constraints

2. **Normal Zone** (16MB-4GB on 32-bit, all memory on 64-bit)
   - Standard allocations
   - Most kernel and user allocations
   - Default zone for most operations

3. **High Zone** (32-bit only, >4GB)
   - Memory above 4GB on 32-bit systems
   - Requires special mapping
   - Not present on 64-bit systems

### Zone Balancing

The allocator implements zone balancing:

```rust
pub struct ZoneAllocator {
    zones: [Zone; MAX_ZONES],
    fallback_order: [[ZoneType; MAX_ZONES]; MAX_ZONES],
}
```

Allocation strategy:
1. Try preferred zone
2. Fall back to other zones if allowed
3. Reclaim memory if necessary
4. Return error if all zones exhausted

## Page Fault Handling

### Fault Types

The page fault handler recognizes:

- **Demand Paging**: First access to allocated page
- **Copy-on-Write**: Write to shared page
- **Stack Growth**: Access below stack pointer
- **Invalid Access**: Segmentation fault

### Fault Resolution

```rust
pub fn handle_page_fault(addr: VirtAddr, error_code: PageFaultError) -> Result<()> {
    let vma = find_vma(addr)?;
    
    match vma.fault_type(addr, error_code) {
        FaultType::DemandPage => allocate_and_map(addr, vma),
        FaultType::CopyOnWrite => copy_and_remap(addr, vma),
        FaultType::StackGrowth => extend_stack(addr, vma),
        FaultType::Invalid => Err(Error::SegmentationFault),
    }
}
```

## Performance Optimizations

### Allocation Performance

Achieved performance metrics:
- Frame allocation: ~500ns average
- Page mapping: ~1.5μs including TLB flush
- Heap allocation: ~350ns for slab sizes
- TLB shootdown: ~4.2μs per CPU

### Optimization Techniques

1. **Per-CPU Caches**: Reduce lock contention
2. **Batch Operations**: Allocate multiple frames at once
3. **Lazy TLB Flushing**: Defer flushes when possible
4. **NUMA Locality**: Prefer local memory allocation
5. **Huge Pages**: Reduce TLB pressure

## Security Features

### Memory Protection

- **W^X Enforcement**: Pages cannot be writable and executable
- **ASLR**: Address space layout randomization
- **Guard Pages**: Detect buffer overflows
- **Zeroing**: Clear pages before reuse

### Hardware Features

Support for modern hardware security:
- Intel CET (Control-flow Enforcement Technology)
- ARM Pointer Authentication
- Memory tagging (MTE/LAM)
- Encrypted memory (TDX/SEV)

## Future Enhancements

### Planned Features

1. **Memory Compression**: Transparent page compression
2. **Memory Deduplication**: Share identical pages
3. **Persistent Memory**: Support for NVDIMM devices
4. **Memory Hot-Plug**: Dynamic memory addition
5. **CXL Support**: Compute Express Link memory

### Research Areas

- Machine learning for allocation prediction
- Quantum-resistant memory encryption
- Hardware-accelerated memory operations
- Energy-aware memory management

## API Examples

### Kernel API

```rust
// Allocate physical frame
let frame = FRAME_ALLOCATOR.lock().allocate()?;

// Map page with specific permissions
page_mapper.map_page(
    Page::containing_address(virt_addr),
    frame,
    PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER,
)?;

// Allocate from specific zone
let dma_frame = zone_allocator.allocate_from_zone(
    ZoneType::DMA,
    order,
)?;
```

### User Space API

```rust
// Memory mapping
let addr = mmap(
    None,                    // Any address
    4096,                    // Size
    PROT_READ | PROT_WRITE,  // Permissions
    MAP_PRIVATE | MAP_ANON,  // Flags
)?;

// Memory protection
mprotect(addr, 4096, PROT_READ)?;

// Memory unmapping
munmap(addr, 4096)?;
```

## Debugging Support

### Memory Debugging Tools

1. **Allocation Tracking**: Track all allocations with backtraces
2. **Leak Detection**: Find unreleased memory
3. **Corruption Detection**: Guard bytes and checksums
4. **Statistics**: Detailed allocation statistics

### Debug Commands

```bash
# Show memory statistics
echo mem > /sys/kernel/debug/memory

# Dump page tables
echo "dump_pt 0x1000" > /sys/kernel/debug/memory

# Show NUMA topology
cat /sys/devices/system/node/node*/meminfo
```

The memory management system is designed to be robust, efficient, and secure, providing a solid foundation for the rest of the VeridianOS kernel.

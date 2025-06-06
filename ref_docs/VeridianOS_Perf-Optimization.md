# Veridian OS Performance Optimization Guide

## Table of Contents

1. [Performance Philosophy](#performance-philosophy)
1. [CPU Optimization](#cpu-optimization)
1. [Memory Performance](#memory-performance)
1. [I/O Optimization](#io-optimization)
1. [Lock-Free Programming](#lock-free-programming)
1. [Cache Optimization](#cache-optimization)
1. [Compiler Optimizations](#compiler-optimizations)
1. [Profiling and Analysis](#profiling-and-analysis)
1. [Benchmarking](#benchmarking)
1. [Performance Patterns](#performance-patterns)

## Performance Philosophy

### Core Principles

1. **Measure, Don’t Guess**: Always profile before optimizing
1. **Algorithmic Efficiency First**: Better algorithms beat micro-optimizations
1. **Cache is King**: Optimize for cache locality
1. **Minimize Contention**: Lock-free when possible, fine-grained when not
1. **Zero-Copy by Design**: Avoid unnecessary data movement

### Performance Targets

|Metric              |Target|Stretch Goal|
|--------------------|------|------------|
|Context Switch      |<500ns|<300ns      |
|System Call Overhead|<100ns|<50ns       |
|IPC Latency         |<1μs  |<500ns      |
|Memory Allocation   |<200ns|<100ns      |
|Page Fault          |<10μs |<5μs        |

## CPU Optimization

### Modern CPU Architecture Considerations

```rust
/// CPU feature detection and optimization
pub struct CpuOptimizer {
    /// Available SIMD instruction sets
    simd_features: SimdFeatures,
    /// CPU microarchitecture
    microarch: MicroArchitecture,
    /// Cache hierarchy
    cache_info: CacheHierarchy,
    /// Heterogeneous core info
    core_topology: CoreTopology,
}

impl CpuOptimizer {
    pub fn new() -> Self {
        let features = detect_cpu_features();
        
        Self {
            simd_features: features.simd,
            microarch: features.microarch,
            cache_info: features.caches,
            core_topology: features.topology,
        }
    }
    
    /// Select optimal implementation based on CPU features
    pub fn select_implementation<T>(&self, 
        implementations: &[Implementation<T>]
    ) -> &Implementation<T> {
        implementations.iter()
            .filter(|impl| self.supports_features(impl.required_features))
            .max_by_key(|impl| impl.performance_score)
            .unwrap_or(&implementations[0])  // Fallback to generic
    }
}
```

### SIMD Optimization

```rust
use core::arch::x86_64::*;

/// Optimized memory copy using AVX-512
#[target_feature(enable = "avx512f")]
unsafe fn memcpy_avx512(dst: *mut u8, src: *const u8, len: usize) {
    let mut offset = 0;
    
    // Copy 64-byte chunks with AVX-512
    while offset + 64 <= len {
        let data = _mm512_loadu_si512(src.add(offset) as *const __m512i);
        _mm512_storeu_si512(dst.add(offset) as *mut __m512i, data);
        offset += 64;
    }
    
    // Copy 32-byte chunks with AVX
    while offset + 32 <= len {
        let data = _mm256_loadu_si256(src.add(offset) as *const __m256i);
        _mm256_storeu_si256(dst.add(offset) as *mut __m256i, data);
        offset += 32;
    }
    
    // Copy remaining bytes
    for i in offset..len {
        *dst.add(i) = *src.add(i);
    }
}

/// Runtime CPU feature detection
pub fn optimized_memcpy(dst: &mut [u8], src: &[u8]) {
    assert_eq!(dst.len(), src.len());
    
    unsafe {
        if is_x86_feature_detected!("avx512f") {
            memcpy_avx512(dst.as_mut_ptr(), src.as_ptr(), src.len());
        } else if is_x86_feature_detected!("avx2") {
            memcpy_avx2(dst.as_mut_ptr(), src.as_ptr(), src.len());
        } else {
            dst.copy_from_slice(src);
        }
    }
}
```

### Branch Prediction Optimization

```rust
/// Branch prediction hints
#[inline(always)]
pub fn likely(b: bool) -> bool {
    if cfg!(target_arch = "x86_64") {
        unsafe { core::intrinsics::likely(b) }
    } else {
        b
    }
}

#[inline(always)]
pub fn unlikely(b: bool) -> bool {
    if cfg!(target_arch = "x86_64") {
        unsafe { core::intrinsics::unlikely(b) }
    } else {
        b
    }
}

/// Example usage in hot path
pub fn process_packet(packet: &Packet) -> Result<(), Error> {
    // Fast path for common case
    if likely(packet.is_valid()) {
        // Process normal packet
        process_valid_packet(packet)
    } else if unlikely(packet.is_malformed()) {
        // Slow path for error handling
        Err(Error::MalformedPacket)
    } else {
        // Handle other cases
        process_special_packet(packet)
    }
}
```

### CPU Affinity and NUMA

```rust
/// NUMA-aware thread placement
pub struct NumaScheduler {
    topology: NumaTopology,
    node_loads: Vec<AtomicU64>,
}

impl NumaScheduler {
    pub fn assign_thread(&self, thread: &Thread) -> NumaNode {
        // Find node with lowest load
        let best_node = self.node_loads
            .iter()
            .enumerate()
            .min_by_key(|(_, load)| load.load(Ordering::Relaxed))
            .map(|(idx, _)| idx)
            .unwrap();
        
        // Update load
        self.node_loads[best_node].fetch_add(1, Ordering::Relaxed);
        
        // Set CPU affinity
        let cpus = self.topology.node_cpus(best_node);
        thread.set_affinity(cpus);
        
        NumaNode(best_node)
    }
    
    pub fn allocate_memory(&self, size: usize, node: NumaNode) -> *mut u8 {
        unsafe {
            // Allocate memory on specific NUMA node
            libc::numa_alloc_onnode(size, node.0 as i32) as *mut u8
        }
    }
}
```

## Memory Performance

### Memory Allocation Strategies

```rust
/// Size-class based allocator for optimal performance
pub struct SizeClassAllocator {
    /// Small allocations (8-256 bytes)
    small: [SlabAllocator; 6],
    /// Medium allocations (512-8192 bytes)
    medium: [SlabAllocator; 5],
    /// Large allocations (>8192 bytes)
    large: BuddyAllocator,
    /// Thread-local caches
    thread_caches: ThreadLocal<ThreadCache>,
}

impl SizeClassAllocator {
    pub fn allocate(&self, size: usize) -> *mut u8 {
        // Try thread-local cache first
        if let Some(ptr) = self.thread_caches.get().allocate(size) {
            return ptr;
        }
        
        // Select appropriate allocator
        match size {
            0..=256 => {
                let class = size_to_class_small(size);
                self.small[class].allocate()
            }
            257..=8192 => {
                let class = size_to_class_medium(size);
                self.medium[class].allocate()
            }
            _ => self.large.allocate(size),
        }
    }
}

/// Thread-local allocation cache
struct ThreadCache {
    /// Per-size-class free lists
    free_lists: [Option<FreeList>; 16],
    /// Allocation statistics
    stats: AllocationStats,
}

impl ThreadCache {
    fn allocate(&mut self, size: usize) -> Option<*mut u8> {
        let class = size_to_class(size);
        
        if let Some(ref mut list) = self.free_lists[class] {
            if let Some(ptr) = list.pop() {
                self.stats.hits += 1;
                return Some(ptr);
            }
        }
        
        self.stats.misses += 1;
        None
    }
}
```

### Memory Prefetching

```rust
use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};

/// Prefetch memory for upcoming access
#[inline(always)]
pub fn prefetch_read<T>(ptr: *const T) {
    unsafe {
        _mm_prefetch(ptr as *const i8, _MM_HINT_T0);
    }
}

/// Example: Prefetching in linked list traversal
pub fn sum_linked_list(head: *const Node) -> u64 {
    let mut sum = 0;
    let mut current = head;
    
    while !current.is_null() {
        let node = unsafe { &*current };
        
        // Prefetch next node while processing current
        if !node.next.is_null() {
            prefetch_read(node.next);
        }
        
        sum += node.value;
        current = node.next;
    }
    
    sum
}
```

### Huge Pages

```rust
/// Transparent huge page management
pub struct HugePageManager {
    /// 2MB huge pages
    huge_2mb: HugePageAllocator,
    /// 1GB huge pages
    huge_1gb: HugePageAllocator,
    /// Promotion/demotion engine
    thp_engine: TransparentHugePages,
}

impl HugePageManager {
    pub fn allocate_huge(&mut self, size: usize) -> Result<HugePage, Error> {
        // Try 1GB pages for very large allocations
        if size >= 1024 * 1024 * 1024 && self.huge_1gb.available() > 0 {
            return self.huge_1gb.allocate();
        }
        
        // Use 2MB pages for medium allocations
        if size >= 2 * 1024 * 1024 && self.huge_2mb.available() > 0 {
            return self.huge_2mb.allocate();
        }
        
        // Fall back to regular pages
        Err(Error::NoHugePagesAvailable)
    }
    
    pub fn promote_to_huge_page(&mut self, addr: VirtAddr) -> Result<(), Error> {
        // Check if region is eligible for promotion
        if !self.thp_engine.is_eligible(addr) {
            return Err(Error::NotEligible);
        }
        
        // Allocate huge page
        let huge_page = self.huge_2mb.allocate()?;
        
        // Copy data from small pages
        self.thp_engine.promote(addr, huge_page)?;
        
        Ok(())
    }
}
```

## I/O Optimization

### Zero-Copy I/O

```rust
/// Zero-copy networking with io_uring
pub struct ZeroCopyNetwork {
    io_uring: IoUring,
    buffer_pool: BufferPool,
    registered_fds: HashMap<RawFd, u32>,
}

impl ZeroCopyNetwork {
    pub async fn send_file(
        &mut self,
        socket_fd: RawFd,
        file_fd: RawFd,
        offset: u64,
        len: usize,
    ) -> io::Result<usize> {
        // Register file descriptors if not already registered
        let socket_idx = self.register_fd(socket_fd)?;
        let file_idx = self.register_fd(file_fd)?;
        
        // Submit splice operation
        let sqe = opcode::Splice::new(file_idx, socket_idx)
            .offset(offset)
            .len(len as u32)
            .build();
        
        let result = self.io_uring.submit_and_wait(sqe).await?;
        Ok(result as usize)
    }
    
    pub async fn zero_copy_receive(
        &mut self,
        socket_fd: RawFd,
    ) -> io::Result<ZeroCopyBuffer> {
        // Get buffer from pool
        let buffer = self.buffer_pool.get_buffer().await?;
        
        // Submit receive operation
        let sqe = opcode::Recv::new(Fd(socket_fd), buffer.as_mut_ptr(), buffer.len() as u32)
            .build()
            .flags(RecvFlags::ZERO_COPY);
        
        let bytes_received = self.io_uring.submit_and_wait(sqe).await?;
        
        Ok(ZeroCopyBuffer {
            data: buffer,
            len: bytes_received as usize,
            pool: self.buffer_pool.clone(),
        })
    }
}
```

### Batched I/O Operations

```rust
/// Batched system calls for improved throughput
pub struct BatchedIo {
    operations: Vec<IoOperation>,
    results: Vec<IoResult>,
}

impl BatchedIo {
    pub fn add_read(&mut self, fd: RawFd, buf: &mut [u8], offset: u64) -> OperationId {
        let id = self.operations.len();
        self.operations.push(IoOperation::Read {
            fd,
            buf: buf.as_mut_ptr(),
            len: buf.len(),
            offset,
        });
        OperationId(id)
    }
    
    pub fn add_write(&mut self, fd: RawFd, buf: &[u8], offset: u64) -> OperationId {
        let id = self.operations.len();
        self.operations.push(IoOperation::Write {
            fd,
            buf: buf.as_ptr(),
            len: buf.len(),
            offset,
        });
        OperationId(id)
    }
    
    pub async fn submit(&mut self) -> Result<(), Error> {
        // Submit all operations in a single syscall
        let mut ring = IO_URING.lock().await;
        
        for op in &self.operations {
            let sqe = match op {
                IoOperation::Read { fd, buf, len, offset } => {
                    opcode::Read::new(Fd(*fd), *buf, *len as u32)
                        .offset(*offset)
                        .build()
                }
                IoOperation::Write { fd, buf, len, offset } => {
                    opcode::Write::new(Fd(*fd), *buf, *len as u32)
                        .offset(*offset)
                        .build()
                }
            };
            
            unsafe { ring.submission().push(&sqe)? };
        }
        
        ring.submit_and_wait(self.operations.len())?;
        
        // Collect results
        let cqes: Vec<_> = ring.completion().collect();
        self.results = cqes.into_iter()
            .map(|cqe| IoResult {
                result: cqe.result(),
                flags: cqe.flags(),
            })
            .collect();
        
        Ok(())
    }
}
```

## Lock-Free Programming

### Lock-Free Data Structures

```rust
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};

/// Lock-free stack implementation
pub struct LockFreeStack<T> {
    head: Atomic<Node<T>>,
}

struct Node<T> {
    data: T,
    next: Atomic<Node<T>>,
}

impl<T> LockFreeStack<T> {
    pub fn new() -> Self {
        Self {
            head: Atomic::null(),
        }
    }
    
    pub fn push(&self, data: T) {
        let node = Owned::new(Node {
            data,
            next: Atomic::null(),
        });
        
        let guard = &epoch::pin();
        
        loop {
            let head = self.head.load(epoch::Ordering::Acquire, guard);
            node.next.store(head, epoch::Ordering::Relaxed);
            
            match self.head.compare_exchange(
                head,
                node,
                epoch::Ordering::Release,
                epoch::Ordering::Acquire,
                guard,
            ) {
                Ok(_) => break,
                Err(e) => node = e.new,
            }
        }
    }
    
    pub fn pop(&self) -> Option<T> {
        let guard = &epoch::pin();
        
        loop {
            let head = self.head.load(epoch::Ordering::Acquire, guard);
            
            match unsafe { head.as_ref() } {
                None => return None,
                Some(h) => {
                    let next = h.next.load(epoch::Ordering::Acquire, guard);
                    
                    if self.head.compare_exchange(
                        head,
                        next,
                        epoch::Ordering::Release,
                        epoch::Ordering::Acquire,
                        guard,
                    ).is_ok() {
                        unsafe {
                            guard.defer_destroy(head);
                            return Some(ptr::read(&h.data));
                        }
                    }
                }
            }
        }
    }
}
```

### Atomic Operations

```rust
use core::sync::atomic::{AtomicU64, AtomicPtr, Ordering};

/// High-performance counter using fetch_add
pub struct Counter {
    /// Distributed counter to avoid contention
    counters: Vec<CachePadded<AtomicU64>>,
}

#[repr(align(64))]
struct CachePadded<T>(T);

impl Counter {
    pub fn new(num_cpus: usize) -> Self {
        Self {
            counters: (0..num_cpus)
                .map(|_| CachePadded(AtomicU64::new(0)))
                .collect(),
        }
    }
    
    pub fn increment(&self) {
        let cpu = current_cpu();
        self.counters[cpu].0.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn get(&self) -> u64 {
        self.counters.iter()
            .map(|c| c.0.load(Ordering::Relaxed))
            .sum()
    }
}

/// Seqlock for high-performance reads
pub struct SeqLock<T> {
    sequence: AtomicU64,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SeqLock<T> {}

impl<T: Copy> SeqLock<T> {
    pub fn read(&self) -> T {
        loop {
            let seq1 = self.sequence.load(Ordering::Acquire);
            
            // Retry if write in progress (odd sequence number)
            if seq1 & 1 != 0 {
                core::hint::spin_loop();
                continue;
            }
            
            let data = unsafe { *self.data.get() };
            
            let seq2 = self.sequence.load(Ordering::Acquire);
            
            // Retry if sequence changed (write occurred)
            if seq1 == seq2 {
                return data;
            }
        }
    }
    
    pub fn write(&self, value: T) {
        // Increment sequence (now odd - write in progress)
        self.sequence.fetch_add(1, Ordering::Acquire);
        
        unsafe {
            *self.data.get() = value;
        }
        
        // Increment sequence again (now even - write complete)
        self.sequence.fetch_add(1, Ordering::Release);
    }
}
```

## Cache Optimization

### Cache-Friendly Data Structures

```rust
/// Cache-conscious B-tree optimized for 64-byte cache lines
pub struct CacheBTree<K, V> {
    root: Option<Box<Node<K, V>>>,
    node_size: usize,
}

#[repr(align(64))]  // Align to cache line
struct Node<K, V> {
    /// Keys packed together for cache locality
    keys: [Option<K>; 7],  // 7 keys fit in cache line with metadata
    /// Child pointers or values
    children: NodeChildren<K, V>,
    /// Number of keys in this node
    len: u8,
}

enum NodeChildren<K, V> {
    Internal([Option<Box<Node<K, V>>>; 8]),
    Leaf([Option<V>; 7]),
}

impl<K: Ord + Copy, V: Copy> CacheBTree<K, V> {
    pub fn insert(&mut self, key: K, value: V) {
        // Prefetch root node
        if let Some(ref root) = self.root {
            prefetch_read(root.as_ref());
        }
        
        // Standard B-tree insertion with cache optimizations
        self.insert_internal(key, value);
    }
    
    fn search_node(&self, node: &Node<K, V>, key: &K) -> Option<V> {
        // Binary search within node (all keys in cache)
        let pos = node.keys[..node.len as usize]
            .binary_search_by(|k| k.as_ref().unwrap().cmp(key));
        
        match pos {
            Ok(i) => {
                if let NodeChildren::Leaf(ref values) = node.children {
                    values[i]
                } else {
                    None
                }
            }
            Err(i) => {
                if let NodeChildren::Internal(ref children) = node.children {
                    if let Some(ref child) = children[i] {
                        // Prefetch child node
                        prefetch_read(child.as_ref());
                        self.search_node(child, key)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }
}
```

### False Sharing Prevention

```rust
/// Padding to prevent false sharing
#[repr(C)]
pub struct PerCpuData {
    /// Actual data
    data: CpuLocalData,
    /// Padding to fill cache line
    _pad: [u8; 64 - size_of::<CpuLocalData>()],
}

/// Example: Per-CPU run queues
pub struct PerCpuRunQueues {
    queues: Vec<CachePadded<RunQueue>>,
}

impl PerCpuRunQueues {
    pub fn new(num_cpus: usize) -> Self {
        Self {
            queues: (0..num_cpus)
                .map(|_| CachePadded(RunQueue::new()))
                .collect(),
        }
    }
    
    pub fn enqueue(&self, cpu: usize, task: Task) {
        self.queues[cpu].0.push(task);
    }
    
    pub fn dequeue(&self, cpu: usize) -> Option<Task> {
        // Try local queue first
        if let Some(task) = self.queues[cpu].0.pop() {
            return Some(task);
        }
        
        // Work stealing with cache awareness
        self.steal_work(cpu)
    }
    
    fn steal_work(&self, thief_cpu: usize) -> Option<Task> {
        // Steal from nearby CPUs first (likely same NUMA node)
        let nearby_cpus = get_nearby_cpus(thief_cpu);
        
        for victim_cpu in nearby_cpus {
            if let Some(task) = self.queues[victim_cpu].0.steal() {
                return Some(task);
            }
        }
        
        None
    }
}
```

## Compiler Optimizations

### Profile-Guided Optimization

```toml
# Cargo.toml PGO configuration
[profile.release-pgo-generate]
inherits = "release"
lto = false
profile-generate = true

[profile.release-pgo-use]
inherits = "release"
lto = "fat"
profile-use = "profiles"
codegen-units = 1
```

### Build Script for PGO

```bash
#!/bin/bash
# build-pgo.sh

# Step 1: Build with profiling
cargo build --profile release-pgo-generate

# Step 2: Run representative workloads
./target/release-pgo-generate/veridian-bench --all
./target/release-pgo-generate/veridian-test --stress

# Step 3: Merge profile data
llvm-profdata merge -output=profiles/pgo.profdata profiles/*.profraw

# Step 4: Build with profile data
cargo build --profile release-pgo-use
```

### Inline Annotations

```rust
/// Force inline for hot paths
#[inline(always)]
pub fn fast_path_function(x: u32) -> u32 {
    x * 2
}

/// Prevent inlining for cold paths
#[inline(never)]
pub fn error_handler(err: Error) -> ! {
    eprintln!("Fatal error: {}", err);
    panic!("{}", err);
}

/// Let compiler decide
#[inline]
pub fn normal_function(x: u32) -> u32 {
    fast_path_function(x) + 1
}
```

## Profiling and Analysis

### Integrated Profiling

```rust
/// Built-in profiler for production use
pub struct Profiler {
    samples: RingBuffer<Sample>,
    overhead_tracker: OverheadTracker,
}

impl Profiler {
    pub fn sample(&mut self) {
        if self.overhead_tracker.should_sample() {
            let sample = Sample {
                timestamp: rdtsc(),
                cpu: current_cpu(),
                instruction_pointer: get_rip(),
                stack_trace: capture_stack_trace(),
                thread_id: current_thread_id(),
            };
            
            self.samples.push(sample);
            self.overhead_tracker.record_sample();
        }
    }
    
    pub fn generate_flamegraph(&self) -> FlameGraph {
        let mut stacks = HashMap::new();
        
        for sample in &self.samples {
            let stack = symbolize_stack(&sample.stack_trace);
            *stacks.entry(stack).or_insert(0) += 1;
        }
        
        FlameGraph::from_stacks(stacks)
    }
}
```

### Performance Counters

```rust
/// Hardware performance counter abstraction
pub struct PerfCounters {
    counters: [PerfCounter; 4],
    group_fd: RawFd,
}

impl PerfCounters {
    pub fn new() -> io::Result<Self> {
        let mut counters = [
            PerfCounter::new(PerfEvent::CpuCycles)?,
            PerfCounter::new(PerfEvent::Instructions)?,
            PerfCounter::new(PerfEvent::CacheMisses)?,
            PerfCounter::new(PerfEvent::BranchMisses)?,
        ];
        
        // Group counters for atomic read
        let group_fd = counters[0].fd;
        for counter in &mut counters[1..] {
            counter.set_group(group_fd);
        }
        
        Ok(Self { counters, group_fd })
    }
    
    pub fn read(&self) -> PerfStats {
        let mut buffer = [0u64; 5];  // format + 4 counters
        
        unsafe {
            libc::read(
                self.group_fd,
                buffer.as_mut_ptr() as *mut _,
                size_of_val(&buffer),
            );
        }
        
        PerfStats {
            cycles: buffer[1],
            instructions: buffer[2],
            cache_misses: buffer[3],
            branch_misses: buffer[4],
        }
    }
}
```

## Benchmarking

### Microbenchmarks

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

fn benchmark_allocator(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocator");
    
    // Configure for consistent results
    group.sample_size(1000);
    group.warm_up_time(Duration::from_secs(3));
    group.measurement_time(Duration::from_secs(10));
    
    for size in [8, 64, 256, 1024, 4096, 16384].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        
        group.bench_with_input(
            BenchmarkId::new("alloc", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let layout = Layout::from_size_align(size, 8).unwrap();
                    let ptr = unsafe { ALLOCATOR.alloc(layout) };
                    black_box(ptr);
                    unsafe { ALLOCATOR.dealloc(ptr, layout) };
                });
            },
        );
    }
    
    group.finish();
}
```

### System Benchmarks

```rust
/// End-to-end system benchmark
pub struct SystemBenchmark {
    config: BenchmarkConfig,
    results: BenchmarkResults,
}

impl SystemBenchmark {
    pub async fn run_all(&mut self) {
        self.benchmark_boot_time().await;
        self.benchmark_context_switches().await;
        self.benchmark_syscall_overhead().await;
        self.benchmark_ipc_latency().await;
        self.benchmark_file_io().await;
        self.benchmark_network_throughput().await;
    }
    
    async fn benchmark_context_switches(&mut self) {
        let num_threads = self.config.thread_count;
        let iterations = self.config.iterations;
        
        // Create threads
        let threads: Vec<_> = (0..num_threads)
            .map(|_| Thread::create(context_switch_worker))
            .collect();
        
        // Measure time
        let start = Instant::now();
        
        // Trigger context switches
        for _ in 0..iterations {
            for (i, thread) in threads.iter().enumerate() {
                thread.set_affinity(CpuSet::single(i % num_cpus()));
                thread.yield_now();
            }
        }
        
        let elapsed = start.elapsed();
        
        self.results.context_switch_latency = 
            elapsed.as_nanos() as f64 / (iterations * num_threads) as f64;
    }
}
```

## Performance Patterns

### Fast Path Optimization

```rust
/// Optimize for the common case
pub fn process_message(msg: &Message) -> Result<Response, Error> {
    // Fast path: small, valid messages (90% of cases)
    if likely(msg.len() < 256 && msg.is_valid_fast()) {
        return Ok(process_small_message_fast(msg));
    }
    
    // Medium path: larger but still common messages
    if msg.len() < 4096 {
        return process_medium_message(msg);
    }
    
    // Slow path: large or special messages
    process_large_message(msg)
}

#[inline(always)]
fn process_small_message_fast(msg: &Message) -> Response {
    // Stack allocation for small messages
    let mut buffer = [0u8; 256];
    let len = msg.serialize_into(&mut buffer);
    
    Response::from_bytes(&buffer[..len])
}
```

### Batching for Throughput

```rust
/// Batch operations to amortize overhead
pub struct BatchProcessor<T> {
    batch: Vec<T>,
    batch_size: usize,
    processor: Box<dyn Fn(&[T])>,
}

impl<T> BatchProcessor<T> {
    pub fn submit(&mut self, item: T) {
        self.batch.push(item);
        
        if self.batch.len() >= self.batch_size {
            self.flush();
        }
    }
    
    pub fn flush(&mut self) {
        if !self.batch.is_empty() {
            (self.processor)(&self.batch);
            self.batch.clear();
        }
    }
}
```

### Memory Pooling

```rust
/// Object pool to avoid allocation overhead
pub struct ObjectPool<T> {
    available: Mutex<Vec<T>>,
    creator: fn() -> T,
    resetter: fn(&mut T),
}

impl<T> ObjectPool<T> {
    pub fn acquire(&self) -> PoolGuard<T> {
        let obj = self.available.lock().unwrap().pop()
            .unwrap_or_else(|| (self.creator)());
        
        PoolGuard {
            object: Some(obj),
            pool: self,
        }
    }
    
    fn release(&self, mut obj: T) {
        (self.resetter)(&mut obj);
        self.available.lock().unwrap().push(obj);
    }
}

pub struct PoolGuard<'a, T> {
    object: Option<T>,
    pool: &'a ObjectPool<T>,
}

impl<T> Drop for PoolGuard<'_, T> {
    fn drop(&mut self) {
        if let Some(obj) = self.object.take() {
            self.pool.release(obj);
        }
    }
}
```

## Performance Checklist

### Before Optimization

- [ ] Profile to identify bottlenecks
- [ ] Measure baseline performance
- [ ] Understand the workload characteristics
- [ ] Review algorithmic complexity

### During Optimization

- [ ] Focus on hot paths first
- [ ] Minimize memory allocations
- [ ] Optimize for cache locality
- [ ] Use appropriate concurrency primitives
- [ ] Leverage SIMD where applicable

### After Optimization

- [ ] Measure improvement
- [ ] Run regression tests
- [ ] Document performance characteristics
- [ ] Monitor for performance regressions

## Conclusion

Performance optimization in Veridian OS is a continuous process that requires careful measurement, analysis, and implementation. Key principles:

1. **Measure First**: Never optimize without profiling
1. **System Thinking**: Consider the whole system, not just microbenchmarks
1. **Hardware Awareness**: Modern CPUs have complex performance characteristics
1. **Algorithmic Efficiency**: Better algorithms beat micro-optimizations
1. **Continuous Monitoring**: Performance can regress quickly without vigilance

By following these guidelines and leveraging Rust’s zero-cost abstractions, Veridian OS can achieve performance competitive with or exceeding traditional systems programming languages while maintaining memory safety and reliability.
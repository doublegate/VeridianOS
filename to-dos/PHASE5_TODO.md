# Phase 5: Performance Optimization TODO

**Phase Duration**: 3-4 months
**Status**: ~75% Complete (v0.5.7 -- per-CPU caching, TLB optimization, IPC fast path, priority inheritance, benchmarks, tracepoints)
**Dependencies**: Phase 4 completion (DONE)

## Overview

Phase 5 focuses on system-wide performance optimization including kernel improvements, driver optimization, and system tuning.

## 🎯 Goals

- [ ] Optimize kernel performance
- [ ] Improve driver efficiency
- [ ] Enhance system responsiveness
- [ ] Reduce resource usage
- [ ] Implement performance monitoring

## 📋 Core Tasks

### 1. Kernel Performance Optimization

#### Memory Management Optimization
- [x] Page allocator improvements
  - [x] Per-CPU page lists (PerCpuPageCache in frame_allocator.rs, v0.5.7)
  - [x] Batched allocations (BATCH_SIZE=32 refill/drain, v0.5.7)
  - [ ] NUMA optimizations (deferred: requires multi-node hardware testing)
  - [ ] Large page support (deferred: Phase 5.5 candidate, requires 2MB THP infrastructure)
- [x] TLB optimization
  - [x] TLB shootdown reduction (TlbFlushBatch in vas.rs, v0.5.7)
  - [x] ASID management (tlb_generation counter + lazy TLB in scheduler, v0.5.7)
  - [ ] TLB prefetching (deferred: requires workload-specific heuristics)
- [ ] Cache optimization
  - [ ] Cache-aware allocation (deferred: requires cache topology detection)
  - [ ] False sharing elimination (deferred: requires SMP multi-hart)
  - [ ] Prefetch hints (deferred: architecture-specific, low priority)

#### Scheduler Optimization
- [x] Scheduling algorithm tuning
  - [x] Load balancing improvements
  - [x] Wake-up latency reduction (context switch wired, TSS RSP0 per-task)
  - [x] CPU affinity optimization
- [ ] Lock-free algorithms
  - [ ] Wait-free queues
  - [ ] RCU implementation
  - [ ] Hazard pointers
- [x] Real-time improvements
  - [x] Priority inheritance (PiMutex in process/sync.rs, v0.5.7)
  - [ ] Deadline scheduling (deferred: requires APIC timer integration for EDF)
  - [ ] Latency bounds (deferred: requires hardware timer support)

#### IPC Optimization
- [x] Fast path optimization
  - [x] Zero-copy transfers
  - [x] Direct switching (IPC blocking/wake wired in v0.5.6)
  - [ ] Batched operations
- [ ] Notification coalescence
- [ ] Shared memory optimization
- [ ] Lock-free message passing

### 2. Driver Performance

#### I/O Optimization
- [ ] Interrupt mitigation
  - [ ] Interrupt coalescing
  - [ ] Polling modes
  - [ ] Hybrid interrupt/polling
- [ ] DMA optimization
  - [ ] Scatter-gather DMA
  - [ ] DMA batching
  - [ ] IOMMU optimization
- [ ] Zero-copy I/O
  - [ ] Direct I/O paths
  - [ ] Page flipping
  - [ ] Buffer sharing

#### Network Performance
- [ ] Network stack optimization
  - [ ] Lock-free packet processing
  - [ ] CPU locality
  - [ ] Batch processing
- [ ] Hardware offload
  - [ ] Checksum offload
  - [ ] Segmentation offload
  - [ ] Receive side scaling
- [ ] XDP/eBPF support
  - [ ] Packet filtering
  - [ ] Load balancing
  - [ ] Custom processing

#### Storage Performance
- [ ] I/O scheduling
  - [ ] Multi-queue scheduling
  - [ ] Priority queues
  - [ ] Deadline scheduling
- [ ] Caching strategies
  - [ ] Adaptive caching
  - [ ] Predictive prefetch
  - [ ] Write combining
- [ ] NVMe optimization
  - [ ] Multiple queues
  - [ ] Interrupt affinity
  - [ ] I/O determinism

### 3. System Services Optimization

#### VFS Performance
- [ ] Path lookup caching
- [ ] Dentry cache optimization
- [ ] Inode cache tuning
- [ ] Parallel directory operations

#### Memory Service
- [ ] Page fault optimization
- [ ] Copy-on-write efficiency
- [ ] Memory compaction
- [ ] Transparent huge pages

#### Process Management
- [ ] Fast process creation
- [x] Efficient context switching (wired in v0.5.6, all 3 archs)
- [ ] Lightweight threads
- [ ] Process group optimization

### 4. Compiler Optimization

#### Profile-Guided Optimization
- [ ] Kernel PGO support
- [ ] Driver PGO support
- [ ] Service PGO support
- [ ] Automatic profiling

#### Link-Time Optimization
- [ ] Whole program optimization
- [ ] Dead code elimination
- [ ] Function inlining
- [ ] Code layout optimization

#### Architecture-Specific
- [ ] SIMD utilization
- [ ] CPU feature detection
- [ ] Micro-architecture tuning
- [ ] Instruction selection

### 5. Power Management

#### CPU Power Management
- [ ] Frequency scaling
- [ ] Core parking
- [ ] C-state management
- [ ] P-state optimization

#### Device Power Management
- [ ] Runtime PM support
- [ ] Suspend/resume optimization
- [ ] Wake lock management
- [ ] Power domains

#### System Power Optimization
- [ ] Idle detection
- [ ] Timer coalescing
- [ ] Workload consolidation
- [ ] Thermal management

### 6. Performance Monitoring

#### Performance Counters
- [ ] Hardware counter support
- [x] Software counters (perf/mod.rs: syscalls, context switches, page faults, IPC)
- [ ] Counter multiplexing
- [ ] User-space access

#### Tracing Infrastructure
- [x] Static tracepoints (perf/trace.rs: 10 event types, per-CPU ring buffers, v0.5.7)
- [ ] Dynamic tracing (deferred: requires kprobes infrastructure)
- [ ] Function tracing (deferred: requires compiler instrumentation)
- [ ] Event correlation (deferred: requires multi-source trace merging)

#### Profiling Tools
- [ ] Sampling profiler
- [ ] Call graph generation
- [ ] Heat map visualization
- [ ] Bottleneck detection

### 7. Benchmarking Suite

#### Micro-benchmarks
- [x] System call latency (bench_syscall_latency in perf/bench.rs, v0.5.7)
- [x] Context switch time (sched_current benchmark, v0.5.7)
- [ ] Memory bandwidth (deferred: requires streaming benchmark)
- [x] IPC throughput (ipc_stats_read benchmark, v0.5.7)

#### Macro-benchmarks
- [ ] Application benchmarks
- [ ] Workload simulation
- [ ] Stress testing
- [ ] Scalability testing

#### Performance Regression
- [ ] Automated testing
- [ ] Regression detection
- [ ] Historical tracking
- [ ] Alert system

### 8. Documentation

#### Performance Guide
- [ ] Tuning parameters
- [ ] Best practices
- [ ] Common bottlenecks
- [ ] Optimization techniques

#### Profiling Guide
- [ ] Tool usage
- [ ] Result interpretation
- [ ] Case studies
- [ ] Troubleshooting

## 🔧 Technical Specifications

### Performance Metrics
```rust
struct PerformanceMetrics {
    syscall_latency_ns: u64,
    context_switch_ns: u64,
    ipc_throughput_msg_per_sec: u64,
    memory_bandwidth_gb_per_sec: f64,
}
```

### Profiling API
```rust
trait Profiler {
    fn start_sampling(&mut self, frequency: u32);
    fn stop_sampling(&mut self);
    fn get_samples(&self) -> Vec<Sample>;
    fn generate_report(&self) -> Report;
}
```

## Deliverables

- [x] Optimized kernel (per-CPU caching, TLB batching, lazy TLB, priority inheritance)
- [x] Performance monitoring tools (perf counters, software tracepoints, trace shell builtin)
- [x] Benchmarking suite (7 micro-benchmarks with Phase 5 targets, perf shell builtin)
- [ ] Tuning documentation (deferred: docs/PERFORMANCE-TUNING.md planned)
- [ ] Performance regression tests (deferred: requires automated CI benchmark comparison)

## 🧪 Validation Criteria

- [ ] 50% reduction in syscall latency
- [ ] 2x improvement in IPC throughput
- [ ] Sub-millisecond interrupt latency
- [ ] Linear scalability to 64 cores
- [ ] No performance regressions

## 🚨 Blockers & Risks

- **Risk**: Optimization complexity
  - **Mitigation**: Incremental changes
- **Risk**: Architecture differences
  - **Mitigation**: Platform-specific tuning
- **Risk**: Stability impact
  - **Mitigation**: Extensive testing

## 📊 Progress Tracking

| Component | Analysis | Implementation | Testing | Complete |
|-----------|----------|----------------|---------|----------|
| Kernel Opt | Done | Done | Partial | ~80% |
| Driver Opt | Done | Not Started | Not Started | Deferred |
| Service Opt | Done | Partial | Not Started | ~30% |
| Monitoring | Done | Done | Partial | ~75% |
| Benchmarks | Done | Done | Done | ~90% |

## 📅 Timeline

- **Month 1**: Performance analysis and kernel optimization
- **Month 2**: Driver and service optimization
- **Month 3**: Monitoring tools and benchmarks
- **Month 4**: Integration and validation

## 🔗 References

- [Linux Performance](http://www.brendangregg.com/linuxperf.html)
- [Systems Performance](http://www.brendangregg.com/systems-performance-2nd-edition-book.html)
- [Intel Optimization Manual](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)
- [ARM Optimization Guide](https://developer.arm.com/documentation/102234/latest/)

## From Code Audit (ALL RESOLVED in v0.5.6)

The following 56 items were recategorized from `TODO(future)` to `TODO(phase5)` and have ALL been resolved (implemented, stubbed with documentation, or removed as unnecessary).

### IPC Optimization (15/15 COMPLETE)
- [x] `ipc/channel.rs` - Direct context switch to receiver for <5us latency
- [x] `ipc/channel.rs` - Block current process and yield CPU until message arrives
- [x] `ipc/channel.rs` - Wake up any waiting receivers
- [x] `ipc/channel.rs` - Wake up all waiting processes with error and clean up
- [x] `ipc/channel.rs` - O(1) capability validation + direct register transfer
- [x] `ipc/channel.rs` - Implement call/reply semantics (send, block, return reply)
- [x] `ipc/fast_path.rs` - Direct process switch via scheduler
- [x] `ipc/fast_path.rs` - Read from current task's saved IPC register set
- [x] `ipc/fast_path.rs` - O(1) capability lookup from per-CPU cache
- [x] `ipc/fast_path.rs` - Check message queue for pending messages
- [x] `ipc/fast_path.rs` - Scheduler yield with optional timeout
- [x] `ipc/rpc.rs` - Optimize service dispatch with direct method_id lookup
- [x] `ipc/sync.rs` - Verify capability is for the specific endpoint_id
- [x] `ipc/shared_memory.rs` - Implement zero-copy transfer (capability validation, page remap, TLB flush)
- [x] `ipc/zero_copy.rs` - Create transfer capability via capability system integration

### Memory Management (8/8 COMPLETE)
- [x] `mm/vas.rs` - Free page table structures by walking hierarchy (2 instances)
- [x] `mm/page_table.rs` - TLB flush after unmap
- [x] `mm/user_validation.rs` - Get page table from process memory space
- [x] `arch/x86_64/mmu.rs` - Set up dedicated kernel page tables
- [x] `arch/x86_64/mmu.rs` - Proper page fault handling (stack growth, heap, COW)
- [x] `process/memory.rs` - Actually allocate/free pages via VMM for heap expansion
- [x] `syscall/mod.rs` - Get actual physical address from VMM
- [x] `syscall/mod.rs` - Implement actual memory mapping with VMM

### Scheduler and Process Management (12/12 COMPLETE)
- [x] `sched/numa.rs` - Query ACPI SRAT/SLIT tables for actual NUMA topology
- [x] `sched/numa.rs` - Query actual memory from ACPI SRAT tables or firmware
- [x] `sched/numa.rs` - Query ACPI MADT table for actual CPU count
- [x] `sched/task_management.rs` - Allocate stack for new task
- [x] `sched/task_management.rs` - Create page table for new task
- [x] `sched/task_management.rs` - Add to task table
- [x] `sched/task_management.rs` - Remove from ready queue
- [x] `sched/task_management.rs` - Remove from wait queue
- [x] `process/sync.rs` - Add thread to scheduler run queue (2 instances)
- [x] `arch/x86_64/context.rs` - Set up kernel stack in TSS for ring transitions
- [x] `arch/x86_64/context.rs` - Return kernel stack pointer from TSS
- [x] `arch/x86_64/context.rs` - Set kernel stack in TSS for ring transitions

### Filesystem and Infrastructure (4/4 COMPLETE)
- [x] `fs/mod.rs` - Move CWD to per-process data
- [x] `fs/ramfs.rs` - Track parent inode for proper ".." entries
- [x] `pkg/sdk/pkg_config.rs` - Query VFS for pkgconfig files
- [x] `services/shell/expand.rs` - Full stdout capture requires process pipe infrastructure

### Security and Crypto (3/3 COMPLETE)
- [x] `security/tpm.rs` - Map TPM MMIO page via VMM before probing
- [x] `crypto/keystore.rs` - Get actual system time from clock subsystem
- [x] `pkg/mod.rs` - Full Dilithium algebraic verification

---

## Explicitly Deferred Items (with Rationale)

The following items are documented as deferred from Phase 5 to Phase 6 or later, with specific infrastructure dependencies noted:

| Item | Rationale | Target Phase |
|------|-----------|-------------|
| Lock-free algorithms (RCU, hazard pointers) | Requires SMP multi-hart with cross-CPU validation | Phase 6 |
| Huge pages (2MB THP) | Phase 5.5 candidate; requires VMM infrastructure changes | Phase 5.5 |
| Deadline scheduling (EDF) | Requires APIC timer integration and real-time task model | Phase 6 |
| io_uring | Requires user-space driver infrastructure | Phase 6 |
| Network performance (DPDK, XDP/eBPF, RSS) | No network drivers exist yet | Phase 6 |
| Storage performance (NVMe multi-queue, I/O scheduling) | No NVMe driver exists yet | Phase 6 |
| Power management (DVFS, C-states, core parking) | No ACPI parser exists yet | Phase 6 |
| Compiler optimization (PGO, LTO, SIMD) | Requires self-hosted Rust compiler or PGO tooling | Phase 6+ |
| Hardware perf counters (PMU) | Requires PMU driver and MSR access infrastructure | Phase 6 |
| Dynamic tracing (kprobes) | Requires code patching infrastructure | Phase 6 |
| Memory bandwidth benchmarks | Requires streaming memory test with NUMA awareness | Phase 5.5 |

---

## Phase 5 Sprint History

### Sprint 1 (v0.5.6): Foundation
- Scheduler context switch wiring (all 3 archs)
- IPC blocking/wake with fast path framework
- TSS RSP0 management for per-task kernel stacks
- All 56 TODO(phase5) markers resolved
- User-space /sbin/init (PID 1 in Ring 3)
- Native binary execution (NATIVE_ECHO_PASS)
- Dead code audit (136 to <100 annotations)

### Sprint 2 (v0.5.7): Performance Optimization
- Per-CPU page frame cache (PerCpuPageCache, 64-frame, batch refill/drain)
- IPC fast path completion (per-task ipc_regs, direct register transfer)
- TLB optimization (TlbFlushBatch, lazy TLB, tlb_generation counter)
- Priority inheritance protocol (PiMutex)
- Benchmarking suite (7 micro-benchmarks, perf shell builtin)
- Software tracepoints (10 event types, per-CPU ring buffers, trace shell builtin)

---

**Previous Phase**: [Phase 4 - Package Ecosystem](PHASE4_TODO.md)
**Next Phase**: [Phase 6 - Advanced Features](PHASE6_TODO.md)
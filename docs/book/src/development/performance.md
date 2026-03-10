# Performance

## Benchmark Results (v0.21.0)

Measured with 7 in-kernel micro-benchmarks on QEMU x86_64 with KVM (i9-10850K):

| Benchmark | Result | Target | Status |
|-----------|--------|--------|--------|
| syscall_getpid | 79ns | <500ns | Exceeded |
| cap_validate | 57ns | <100ns | Exceeded |
| atomic_counter | 34ns | -- | Baseline |
| ipc_stats_read | 44ns | -- | Baseline |
| sched_current | 77ns | -- | Baseline |
| frame_alloc_global | 1,525ns | <2,000ns | Met |
| frame_alloc_1 (per-CPU) | 2,215ns | <2,000ns | Marginal |

6/7 benchmarks meet or exceed Phase 5 targets.

## Performance Targets (All Achieved)

| Metric | Target | Achieved |
|--------|--------|----------|
| IPC Latency | <5us | <1us |
| Context Switch | <10us | <10us |
| Memory Allocation | <1us | <1us |
| Capability Lookup | O(1) | O(1) |
| Concurrent Processes | 1000+ | 1000+ |

## Running Benchmarks

In-kernel benchmarks are accessible via the `perf` shell command in QEMU:

```
root@veridian:/# perf
```

This runs all 7 micro-benchmarks and prints TSC-based timing results.

## Performance Design

Key performance features implemented:
- **Per-CPU page frame cache** (64-frame) minimizes allocator lock contention
- **TLB shootdown reduction** via `TlbFlushBatch` and ASID management
- **Fast-path IPC** with register-based transfer for small messages (<64 bytes)
- **Direct IPC context switching** with priority inheritance (`PiMutex`)
- **CFS scheduler** with per-CPU run queues and work-stealing
- **Cache-aware allocation** to prevent false sharing
- **Write-combining PAT** for framebuffer (1200+ MB/s vs 200 MB/s UC)

See [docs/PERFORMANCE-REPORT.md](https://github.com/doublegate/VeridianOS/blob/main/docs/PERFORMANCE-REPORT.md) for the full benchmark report.

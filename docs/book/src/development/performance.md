# Performance

## Benchmark Results (v0.27.0)

Measured with the in-kernel `perf` command on QEMU x86_64 with KVM (i9-10850K), one vCPU,
calibrated TSC, 1,000 iterations (2026-10-15):

| Benchmark | Release kernel | Dev kernel | Target |
|-----------|---------------:|-----------:|-------:|
| syscall_getpid | 33 ns | 86 ns | 500 ns |
| frame_alloc_1 (per-CPU cache) | 74 ns | 2,115 ns | 2,000 ns |
| frame_alloc_global | 85 ns | 297 ns | 4,000 ns |
| cap_lookup | 29 ns | 97 ns | 100 ns |
| atomic_counter | 13 ns | 14 ns | 50 ns |
| ipc_stats_read | 8 ns | 20 ns | 100 ns |
| sched_current | 32 ns | 81 ns | 200 ns |
| page_clear (4 KiB) | 31 ns | 36 ns | -- |

Dev kernels build `core` and `compiler_builtins` unoptimised, so anything that clears or copies
memory looks many times slower than it is (N-281): a dev `frame_alloc_1` includes such a page
clear. Decisions are taken on release-kernel numbers.

## Performance Targets

| Metric | Target | Measured |
|--------|--------|----------|
| IPC Latency | <1us | not measured: messages are queued on the endpoint, and no round trip is benchmarked (the IPC redesign is v0.28 work) |
| Context Switch | <10us | not measured |
| Memory Allocation | <1us | 74 ns per-CPU cache, 85 ns global allocator (release kernel) |
| Capability Lookup | O(1), <100 ns | 29 ns (release kernel) |
| Concurrent Processes | 1000+ | not measured |

## Running Benchmarks

In-kernel benchmarks are accessible via the `perf` shell command in QEMU:

```
root@veridian:/# perf
```

This runs all 8 micro-benchmarks and prints timing results from the calibrated clock source.

## Performance Design

Key performance features implemented:
- **Per-CPU page frame cache** minimizes allocator lock contention
- **TLB shootdown reduction** via `TlbFlushBatch` and ASID management
- **Dispatcher** (ADR 0006) with EEVDF fair, real-time and deadline classes (ADR 0007), and
  per-CPU run queues with wake placement and load balancing on every CPU (ADR 0012)
- No IPC fast path or priority inheritance yet: both were removed with the old scheduler in v0.27
  (IPC hand-off returns with the v0.28 IPC redesign; priority inheritance is SCHED-INC-01)
- **Cache-aware allocation** to prevent false sharing
- **Write-combining PAT** for framebuffer (1200+ MB/s vs 200 MB/s UC)

See [docs/PERFORMANCE-REPORT.md](https://github.com/doublegate/VeridianOS/blob/main/docs/PERFORMANCE-REPORT.md) for the full benchmark report.

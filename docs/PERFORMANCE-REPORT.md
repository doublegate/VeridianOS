# VeridianOS v0.21.0 Performance Report

**Date**: 2026-03-07
**Version**: v0.21.0

## Test Environment

| Parameter | Value |
|-----------|-------|
| CPU | Intel Core i9-10850K @ 3.60GHz |
| QEMU | 10.2.0 |
| KVM | Enabled |
| Host Kernel | 6.19.5-3-cachyos |
| Rust Toolchain | rustc 1.93.0-nightly (2025-11-14) |
| Build Mode | Debug (dev) |
| Guest RAM | 2GB |

## Boot Verification

- Boot stages: 6/6 complete (Hardware, Memory, Process, Services, Scheduler, User Space)
- In-kernel tests: **29/29 passed**
- Boot status: **BOOTOK**

## Host-Target Tests (`cargo test`)

Host-target test compilation is blocked by duplicate `#[panic_handler]` lang items when
`build-std` interacts with the standard test harness. This is a known limitation of
bare-metal `#![no_std]` kernels compiled for host targets. CI uses `cargo llvm-cov`
with `build-std` stripped from `.cargo/config.toml` (with `continue-on-error: true`).

The "4,095 tests" count reflects `#[test]` functions that compile and pass under CI's
modified configuration. The kernel's correctness is validated by the 29/29 in-kernel
boot tests, clippy (0 warnings across 4 targets), and successful 3-architecture builds.

## In-Kernel Micro-Benchmarks (QEMU x86_64 + KVM)

Benchmarks executed via the `perf` shell command inside QEMU. Each benchmark runs
1,000 iterations with 10 warmup iterations. Timing via TSC (Time Stamp Counter).

### Run 1

| Benchmark | Min (ns) | Avg (ns) | Max (ns) | Target (ns) | Result |
|-----------|----------|----------|----------|-------------|--------|
| syscall_getpid | 75 | 79 | 96 | 500 | **PASS** |
| frame_alloc_1 | 2,059 | 2,215 | 8,544 | 2,000 | FAIL |
| frame_alloc_global | 1,497 | 1,525 | 6,616 | 4,000 | **PASS** |
| cap_validate | 52 | 57 | 61 | 100 | **PASS** |
| atomic_counter | 28 | 34 | 54 | 50 | **PASS** |
| ipc_stats_read | 41 | 44 | 63 | 100 | **PASS** |
| sched_current | 73 | 77 | 97 | 200 | **PASS** |

**Result: 6/7 benchmarks meet Phase 5 targets**

### Run 2 (via `perf stats`)

| Benchmark | Min (ns) | Avg (ns) | Max (ns) | Target (ns) | Result |
|-----------|----------|----------|----------|-------------|--------|
| syscall_getpid | 76 | 80 | 87 | 500 | **PASS** |
| frame_alloc_1 | 2,060 | 2,424 | 155,725 | 2,000 | FAIL |
| frame_alloc_global | 1,493 | 1,518 | 8,625 | 4,000 | **PASS** |
| cap_validate | 53 | 57 | 62 | 100 | **PASS** |
| atomic_counter | 27 | 34 | 36 | 50 | **PASS** |
| ipc_stats_read | 42 | 44 | 63 | 100 | **PASS** |
| sched_current | 74 | 77 | 110 | 200 | **PASS** |

**Result: 6/7 benchmarks meet Phase 5 targets**

### Analysis

- **syscall_getpid**: 79ns avg vs 500ns target -- 6.3x under target. Excellent.
- **frame_alloc_1** (per-CPU): 2,215ns avg vs 2,000ns target -- 10.7% over target.
  The per-CPU frame allocator path involves spinlock acquisition. The min (2,059ns) is
  close to target, suggesting contention or cache effects inflate the average. Run 2
  shows a 155us max outlier, likely a VM exit or host interrupt.
- **frame_alloc_global**: 1,525ns avg vs 4,000ns target -- 2.6x under target.
  Interestingly faster than per-CPU path, suggesting the global allocator's lock is
  uncontended in single-core QEMU and benefits from simpler code path.
- **cap_validate**: 57ns avg vs 100ns target -- 1.8x under target. Simple range check.
- **atomic_counter**: 34ns avg vs 50ns target -- 1.5x under target. AtomicU64 baseline.
- **ipc_stats_read**: 44ns avg vs 100ns target -- 2.3x under target.
- **sched_current**: 77ns avg vs 200ns target -- 2.6x under target.

### IPC Statistics

| Metric | Value |
|--------|-------|
| Fast path calls | 0 |
| Fast path avg cycles | 0 |
| Slow path fallbacks | 0 |

Note: IPC fast path calls are zero because no user-space IPC has been exercised
during this boot session. The benchmarks measure individual operation latencies,
not end-to-end IPC.

## Comparison with Phase 0/1 Baselines (June 2025)

| Operation | Phase 0/1 Baseline | v0.21.0 Measured | Change |
|-----------|--------------------|------------------|--------|
| Syscall latency | Not measured | 79ns | New |
| Frame alloc (single) | <500ns | 2,215ns (per-CPU) / 1,525ns (global) | Different methodology |
| Cap validation | Not measured | 57ns | New |
| Context switch | ~500ns (simulated) | Not directly measured | -- |
| IPC small msg | <1us (register) | 44ns (stats read) | Different measurement |

Note: Phase 0/1 baselines measured different operations (simulated context switches,
allocator unit tests). Direct comparison is limited. The v0.21.0 benchmarks measure
actual in-kernel operations under QEMU/KVM.

## Standalone Bench Binaries

Three `[[bench]]` targets exist in `kernel/benches/`:
- `ipc_latency`
- `context_switch`
- `memory_allocation`

These are `#![no_std]` bare-metal binaries requiring custom QEMU boot with specific
entry points. They cannot run via `cargo bench`. The existing `scripts/benchmark.sh`
uses `bootimage` (incompatible with current UEFI build system) and `timeout`
(incompatible with QEMU 10.2). These benchmarks require manual QEMU boot configuration
to execute.

## Summary

- **6/7** in-kernel micro-benchmarks meet Phase 5 performance targets
- **29/29** boot-time kernel tests pass
- Only `frame_alloc_1` (per-CPU path) slightly exceeds its 2,000ns target at 2,215ns avg
- All other operations significantly under target (1.5x-6.3x margin)
- Results are consistent across two consecutive runs

---

## v0.26.0 Audit Remediation -- Before/After

Tracks the in-kernel effect of the fixes listed in
`docs/audit/AUDIT-VERIFICATION-2026-10-05.md`. The audit matrix's own figures come from
host-side models (`tools/audit-models`), not from the kernel; the numbers here are the
kernel's, from the `perf` shell command (1,000 iterations, 10 warmup, TSC timing).

| Parameter | Value |
|-----------|-------|
| CPU | Intel Core i9-10850K @ 3.60GHz, guest `-cpu host`, 1 vCPU (only the BSP runs) |
| QEMU | 11.1.1, KVM |
| Host Kernel | 7.2.9-1-cachyos |
| Rust Toolchain | nightly-2025-11-15 |
| Build Mode | Debug (dev) |
| Guest RAM | 2GB |

**Baseline** (2026-10-05, commit `f007c76`; the v0.25.2 kernel plus the KDE Ring-3 work):

| Benchmark | Min (ns) | Avg (ns) | Max (ns) | Target | After (avg ns) |
|-----------|---------:|---------:|---------:|-------:|---------------:|
| syscall_getpid | 46 | 49 | 68 | 500 | |
| frame_alloc_1 (per-CPU) | 1,586 | 1,609 | 6,201 | 2,000 | |
| frame_alloc_global | 790 | 806 | 859 | 4,000 | |
| cap_validate | 31 | 33 | 36 | 100 | |
| atomic_counter | 24 | 27 | 30 | 50 | |
| ipc_stats_read | 29 | 30 | 34 | 100 | |
| sched_current | 46 | 49 | 52 | 200 | |

On a single CPU, with no contention possible, the per-CPU path is already 2.0x slower than
the global allocator it is meant to speed up (1,609 vs 806 ns): every call takes the one
mutex around all 16 caches, and a refill takes the global allocator lock while still holding
it (MEM-PERF-01). The v0.21.0 run above showed the same inversion (2,215 vs 1,525 ns).

Host-target unit tests at baseline: **4,284 passed, 0 failed**
(`cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu`).
The "Host-Target Tests" section above is out of date: the host build compiles, and
`[INIT] Results: 29/29 passed` plus BOOTOK hold on all three architectures.

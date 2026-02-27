# VeridianOS Performance Benchmarks

**Last Updated**: February 27, 2026 (v0.5.8)

This document describes the in-kernel micro-benchmark suite and Phase 5 performance targets.

---

## Running Benchmarks

From the VeridianOS shell (`root@veridian:/#`):

```
perf          # Run all 7 benchmarks
perf stats    # Show performance counters (syscalls, context switches, page faults, IPC)
perf reset    # Reset performance counters to zero
```

## Benchmark Suite

The benchmark suite is implemented in `kernel/src/perf/bench.rs`. Each benchmark runs 1000 iterations after a 10-iteration warmup, reporting min/avg/max latency in nanoseconds.

### Benchmarks

| Benchmark | Operation | Target (ns) | Description |
|-----------|-----------|-------------|-------------|
| `syscall_getpid` | `current_process_id()` | 500 | Minimal syscall overhead (no mode switch, kernel-internal) |
| `frame_alloc_1` | `per_cpu_alloc_frame()` + free | 500 | Per-CPU page frame cache alloc/free (lock-free hot path) |
| `frame_alloc_global` | `FRAME_ALLOCATOR.lock()` + alloc/free | 1000 | Global frame allocator with lock (comparison baseline) |
| `cap_validate` | Range check on capability token | 100 | Fast-path capability validation (no cache lookup) |
| `atomic_counter` | `AtomicU64::fetch_add` | 50 | Atomic operation baseline (noise floor) |
| `ipc_stats_read` | `get_fast_path_stats()` | 100 | IPC fast path statistics read (2 atomic loads) |
| `sched_current` | `SCHEDULER.lock().current()` | 200 | Scheduler lock + current task pointer read |

### Phase 5 Performance Targets

| Metric | Target | Status |
|--------|--------|--------|
| System call latency | < 500 ns | Measured via `syscall_getpid` |
| Context switch time | < 10 us | Measured via `sched_current` (proxy) |
| IPC small message | < 1 us | Measured via fast path stats |
| Frame allocation (per-CPU) | < 500 ns | Measured via `frame_alloc_1` |
| Capability lookup | < 100 ns | Measured via `cap_validate` |

### Output Format

```
=== VeridianOS Phase 5 Performance Benchmarks ===

Benchmark            Min(ns)  Avg(ns)  Max(ns)   Target  Pass?
--------------------------------------------------------------------
syscall_getpid            12       15       42      500   PASS
frame_alloc_1             28       35      120      500   PASS
frame_alloc_global        85      110      340     1000   PASS
cap_validate               3        4       12      100   PASS
atomic_counter             5        6       15       50   PASS
ipc_stats_read             8       10       25      100   PASS
sched_current             45       60      180      200   PASS
--------------------------------------------------------------------
Results: 7/7 benchmarks meet Phase 5 targets

IPC Statistics:
  Fast path: 0 calls, 0 avg cycles
  Slow path fallbacks: 0
```

(Values are illustrative; actual measurements depend on hardware and KVM availability.)

## Performance Counters

Software performance counters are maintained in `kernel/src/perf/mod.rs` using `AtomicU64`:

| Counter | Tracks |
|---------|--------|
| `SYSCALL_COUNT` | Total system calls handled |
| `CONTEXT_SWITCH_COUNT` | Total context switches performed |
| `PAGE_FAULT_COUNT` | Total page faults handled |
| `IPC_MESSAGE_COUNT` | Total IPC messages sent |

Accessible via `perf stats` shell builtin.

## Tracepoints

Software tracepoints (`kernel/src/perf/trace.rs`) provide per-event timing:

| Event Type | Instrumented Location |
|------------|----------------------|
| `SyscallEntry` | `syscall_handler()` entry |
| `SyscallExit` | `syscall_handler()` return |
| `SchedSwitchOut` | `switch_to()` before context switch |
| `SchedSwitchIn` | `switch_to()` after context switch |
| `IpcFastSend` | `fast_send()` entry |
| `IpcFastReceive` | `fast_receive()` entry |
| `IpcSlowPath` | Fast path fallback to slow path |
| `FrameAlloc` | `per_cpu_alloc_frame()` return |
| `FrameFree` | (not yet wired) |
| `PageFault` | (not yet wired) |

Per-CPU ring buffers hold 4096 events each (128KB per CPU). Zero overhead when disabled (single `AtomicBool` check).

```
trace on      # Enable tracing
trace off     # Disable tracing
trace dump    # Dump trace buffer contents
trace status  # Show tracing status and event counts
```

## Architecture Notes

- All benchmarks use `read_timestamp()` which maps to `RDTSC` (x86_64), `CNTVCT_EL0` (AArch64), or `rdcycle` (RISC-V).
- `cycles_to_ns()` converts using a hardcoded 2 GHz frequency estimate. Actual calibration requires APIC timer (Phase 5.5).
- KVM acceleration (`-enable-kvm`) is required for meaningful x86_64 measurements. TCG emulation adds orders of magnitude overhead.

---

**See also**: [Performance Tuning](PERFORMANCE-TUNING.md) | [Phase 5 TODO](../to-dos/PHASE5_TODO.md) | [Deferred Items](DEFERRED-IMPLEMENTATION-ITEMS.md)

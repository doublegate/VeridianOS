# VeridianOS Performance Tuning Guide

**Last Updated**: February 27, 2026 (v0.5.8)

This document describes the kernel's performance optimization subsystems, their architecture, and tuning parameters.

---

## 1. Per-CPU Page Frame Cache

**File**: `kernel/src/mm/frame_allocator.rs`

### Architecture

The per-CPU page cache eliminates global `FRAME_ALLOCATOR` lock contention for single-frame allocations, which dominate the page fault and `map_page()` hot paths.

```
map_page() --> per_cpu_alloc_frame() --> [Per-CPU Cache]
                                              |
                                         (cache empty?)
                                              |
                                    [Global FRAME_ALLOCATOR]
                                         (batch refill)
```

Each CPU maintains a `PerCpuPageCache` with up to 64 frames. When the cache is empty, it refills 32 frames at once from the global allocator. When full, it drains 32 frames back.

### Parameters

| Parameter | Value | Location | Purpose |
|-----------|-------|----------|---------|
| `MAX_FRAMES` | 64 | `PerCpuPageCache` | Maximum frames per CPU cache |
| `LOW_WATERMARK` | 16 | `PerCpuPageCache` | Trigger batch refill below this |
| `HIGH_WATERMARK` | 48 | `PerCpuPageCache` | Trigger batch drain above this |
| `BATCH_SIZE` | 32 | `PerCpuPageCache` | Frames transferred per refill/drain |

### Hot Path Integration (v0.5.8)

- `map_page()` in `vas.rs` uses `per_cpu_alloc_frame()` instead of `FRAME_ALLOCATOR.lock()`.
- Falls back to global allocator transparently when per-CPU cache is empty and refill fails.

### Performance Impact

- Eliminates spin lock contention on multi-CPU systems.
- Amortizes lock acquisition cost: 1 lock per 32 frames instead of 1 per frame.
- Expected speedup: 2-5x for frame allocation on multi-core workloads.

---

## 2. TLB Flush Batching

**File**: `kernel/src/mm/vas.rs`

### Architecture

`TlbFlushBatch` accumulates virtual addresses for TLB invalidation instead of issuing individual `invlpg` instructions. When the batch exceeds 16 entries, it falls back to a full TLB flush (more efficient for large unmaps).

```
unmap_region() --> TlbFlushBatch::add(vaddr) --> [Buffer: up to 16 addrs]
                                                       |
                                                  (> 16 addrs?)
                                                    /        \
                                              invlpg x16   full TLB flush
```

### Parameters

| Parameter | Value | Location | Purpose |
|-----------|-------|----------|---------|
| `MAX_BATCH` | 16 | `TlbFlushBatch` | Threshold for individual vs full TLB flush |

### Hot Path Integration (v0.5.8)

Three locations wired:
1. `map_region()` -- post-mapping TLB flush loop
2. `unmap_region()` -- region unmapping TLB flush loop
3. `unmap()` (partial munmap) -- sub-range unmapping

Single-page flushes (e.g., individual page table updates) are left as individual `tlb_flush_address()` calls since batching provides no benefit for 1 address.

### Lazy TLB Optimization

`switch_to()` in the scheduler skips CR3 reload when switching to kernel threads (`has_user_mappings == false`), avoiding unnecessary TLB flushes. A `tlb_generation` counter on each VAS tracks modifications; the scheduler compares generations to detect stale TLBs.

---

## 3. IPC Fast Path

**File**: `kernel/src/ipc/fast_path.rs`

### Architecture

The IPC fast path provides sub-microsecond message delivery for small messages (up to 7 registers) by bypassing the general-purpose channel infrastructure.

```
fast_send(target_pid, msg)
    |
    +--> validate_capability_fast(cap)   # CapabilityCache check, then range check
    |         |
    |    [16-entry direct-mapped cache]
    |
    +--> get_task_ptr(target_pid)         # O(log n) BTreeMap lookup
    |         |
    |    [Global TASK_REGISTRY]
    |
    +--> write ipc_regs[0..6]            # Direct register transfer
    |
    +--> scheduler::wake_task()           # Wake receiver
```

### Components

#### CapabilityCache (v0.5.8)

A 16-entry direct-mapped cache for IPC capability validation. Hash function: `id & 0xF`. On cache hit, `validate_capability_fast()` returns immediately without full capability space lookup.

- Populated on successful IPC completion in `fast_send()`.
- Cache miss falls through to existing range check.
- Uses `try_lock()` to avoid blocking the IPC hot path.

#### PID-to-Task Registry (v0.5.8)

A global `BTreeMap<u64, SendTaskPtr>` providing O(log n) PID-to-Task lookup, replacing the previous O(n) linear scan through the scheduler's task list.

- `register_task()` called from `create_task()` and `create_task_from_thread()`.
- `unregister_task()` called from `exit_task()`.
- Lock scope minimized: pointer cloned and lock released before message copy.

#### Per-Task IPC Registers

Each task has `ipc_regs: [u64; 7]` for direct register-based message transfer. `fast_send()` writes directly to the target task's registers; `fast_receive()` reads from the current task's registers on wakeup.

### Trace Events

| Event | When | Data |
|-------|------|------|
| `IpcFastSend` | Entry to `fast_send()` | target PID, capability |
| `IpcFastReceive` | Entry to `fast_receive()` | calling PID, 0 |
| `IpcSlowPath` | Fast path falls back | calling PID, reason code |

---

## 4. Priority Inheritance Protocol

**File**: `kernel/src/process/sync.rs`

### Architecture

`PiMutex` prevents unbounded priority inversion by temporarily boosting the lock owner's priority to match the highest-priority waiter.

```
High-priority Task A
    |
    +--> PiMutex::lock()
    |       |
    |   (owner = Task B, priority 10)
    |       |
    |   boost Task B: priority 10 --> priority A's priority
    |       |
    |   Task A blocks (added to wait queue)
    |
    ... Task B runs at boosted priority ...
    |
Task B: PiMutex::unlock()
    |
    +--> restore original priority
    +--> wake highest-priority waiter
```

### Implementation

- `Task::priority_boost: Option<u8>` -- active boost value, checked by `effective_priority()`.
- On `lock()`: if mutex is held, boost owner's priority to max(owner, waiter).
- On `unlock()`: restore owner's original priority, wake highest-priority waiter from queue.
- Transitive: if Task C blocks on a PiMutex held by Task B (which is boosted by Task A), Task B keeps the highest boost.

---

## 5. Software Tracepoints

**File**: `kernel/src/perf/trace.rs`

### Architecture

Per-CPU ring buffers (4096 events each, 128KB per CPU) with zero-overhead disable path. The `trace!()` macro compiles to a single `AtomicBool` load when tracing is disabled.

### 10 Event Types

| Type | Instrumented | v0.5.8 Status |
|------|-------------|---------------|
| `SyscallEntry` | `syscall_handler()` entry | Wired |
| `SyscallExit` | `syscall_handler()` return | Wired |
| `SchedSwitchOut` | `switch_to()` before switch | Wired |
| `SchedSwitchIn` | `switch_to()` after switch | Wired |
| `IpcFastSend` | `fast_send()` | Wired (v0.5.8) |
| `IpcFastReceive` | `fast_receive()` | Wired (v0.5.8) |
| `IpcSlowPath` | Fast path fallback | Wired (v0.5.8) |
| `FrameAlloc` | `per_cpu_alloc_frame()` | Wired (v0.5.8) |
| `FrameFree` | `per_cpu_free_frame()` | Wired |
| `PageFault` | `page_fault_handler()` | Wired |

### Usage

```
trace on       # Enable (sets TRACING_ENABLED AtomicBool)
trace dump     # Print last N events from current CPU's ring buffer
trace status   # Show total event count and enabled/disabled state
trace off      # Disable
```

### Overhead

- Disabled: 1 atomic load per trace point (~1 ns).
- Enabled: timestamp read + ring buffer write (~20-50 ns per event).
- Ring buffer is fixed-size (no allocation): events overwrite oldest on wrap.

---

## 6. General Tuning Recommendations

### QEMU Testing

- Always use `-enable-kvm` for x86_64 benchmarks. TCG emulation adds 100x+ overhead.
- Use `-m 2048M` for workloads involving native compilation (512MB kernel heap).
- Benchmark results under TCG (AArch64, RISC-V on x86_64 host) are not representative of bare-metal performance.

### Memory

- Per-CPU cache sizing: 64 frames (256KB per CPU) balances cache hit rate against memory overhead. Increase `MAX_FRAMES` for allocation-heavy workloads.
- Batch size: 32 frames amortizes global lock cost. Larger batches reduce lock frequency but increase per-refill latency.

### IPC

- Fast path is most effective for small, frequent messages between known process pairs.
- CapabilityCache benefits workloads with repeated IPC to the same endpoints.
- For bulk data transfer, use shared memory regions instead of register-based IPC.

### Scheduling

- Lazy TLB avoids unnecessary CR3 reloads for kernel threads. Most beneficial when kernel threads are frequently scheduled between user tasks.
- Priority inheritance is only needed when high-priority tasks share mutexes with low-priority tasks. Prefer lock-free designs where possible.

---

---

## 7. Cache Topology Detection (v0.11.1)

**File**: `kernel/src/mm/cache_topology.rs`

### Architecture

Detects CPU cache hierarchy (L1/L2/L3) using architecture-specific mechanisms:
- x86_64: CPUID leaf 4 (Intel) and leaf 0x8000001D (AMD)
- AArch64: hardcoded defaults (Cortex-A72: 48KB L1D, 1MB L2)
- RISC-V: hardcoded defaults (32KB L1D, 256KB L2)

### Cache Coloring

Assigns "colors" to physical frames based on L3 set index bits, reducing cache
conflict misses for adjacent allocations on NUMA systems. Advisory preference
in frame allocator -- not a hard constraint.

---

## 8. False Sharing Elimination (v0.11.1)

**File**: `kernel/src/mm/cache_aligned.rs`

### CacheAligned<T> Wrapper

`CacheAligned<T>` wraps a value with `#[repr(C, align(64))]` to ensure each
value occupies its own cache line, preventing false sharing between CPU cores.

Used for:
- Performance counters (`perf/mod.rs`): 5 atomic counters
- Per-CPU page caches (`mm/frame_allocator.rs`): `PerCpuPageCache`
- Ready queues (`sched/queue.rs`): `ReadyQueue`

---

## 9. Deadline Scheduling -- EDF (v0.11.1)

**File**: `kernel/src/sched/deadline.rs`

### SCHED_DEADLINE Policy

Implements Earliest Deadline First alongside CFS. Deadline tasks always preempt
CFS tasks. Among deadline tasks, the one with the earliest absolute deadline wins.

| Parameter | Description |
|-----------|-------------|
| runtime_ns | Worst-case execution time per period |
| deadline_ns | Relative deadline from period start |
| period_ns | Activation period |

Admission control ensures total utilization (sum of runtime/period) <= 100%.
Uses fixed-point permille arithmetic (1000 = 100%).

---

## 10. Power Management (v0.11.1)

**File**: `kernel/src/power/mod.rs`

### C-States

| State | x86_64 | AArch64/RISC-V | Wake Latency |
|-------|--------|----------------|-------------|
| C0 | Active | Active | 0 |
| C1 | HLT | WFI | ~1 us |
| C2 | MWAIT | WFI | ~10 us |
| C3 | MWAIT C3 | WFI | ~100 us |

### P-States (OnDemand Governor)

- Monitors CPU utilization over 10ms intervals
- Scales up when utilization > 80%, down when < 30%
- x86_64: `IA32_PERF_CTL` MSR for frequency control
- 8 performance levels (P0-P7)

---

## 11. Profile-Guided Optimization (v0.11.1)

### PGO Build Steps

```bash
# Step 1: Instrumented build
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" \
  cargo build --target targets/x86_64-veridian.json \
  -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc --release

# Step 2: Run workload in QEMU
qemu-system-x86_64 -enable-kvm \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
  -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/release/veridian-uefi.img \
  -device ide-hd,drive=disk0 \
  -serial stdio -display none -m 256M </dev/null > /tmp/pgo-boot.log 2>&1 &
PGO_PID=$!; sleep 30; kill $PGO_PID 2>/dev/null

# Step 3: Merge profiles
llvm-profdata merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data/

# Step 4: Optimized build
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata" \
  cargo build --target targets/x86_64-veridian.json \
  -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc --release
```

Expected: 5-15% improvement for hot code paths (IPC, scheduler, page faults).

---

**See also**: [Performance Benchmarks](PERFORMANCE-BENCHMARKS.md) | [Phase 5 TODO](../to-dos/PHASE5_TODO.md) | [Deferred Items](DEFERRED-IMPLEMENTATION-ITEMS.md)

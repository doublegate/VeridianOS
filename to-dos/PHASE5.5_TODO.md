# Phase 5.5: Infrastructure Bridge TODO

**Phase Duration**: 2-3 months
**Status**: 0% (Planning Complete)
**Dependencies**: Phase 5 completion (~90%, v0.5.8)

## Overview

Phase 5.5 bridges the gap between Phase 5 (Performance Optimization) and Phase 6 (Advanced Features/GUI). Phase 6 requires hardware infrastructure (ACPI, APIC timer, PCI MSI/MSI-X, IOMMU) and OS primitives (Unix domain sockets, shared memory, lock-free data structures) that do not yet exist. This phase builds those prerequisites.

## Rationale

Phase 6 targets include a Wayland compositor, desktop environment, and advanced networking. These require:
- **ACPI**: Hardware topology discovery (CPU count, IRQ routing, PCIe config space)
- **APIC Timer**: Preemptive scheduling (currently scheduler only runs when explicitly called)
- **PCI MSI/MSI-X**: Modern interrupt delivery for virtio-gpu, USB, NVMe
- **Shared Memory + Unix Sockets**: Wayland client-compositor IPC
- **Lock-Free Paths**: Sub-microsecond IPC without spin lock contention
- **DMA/IOMMU**: Secure DMA for real GPU and network drivers

---

## Dependency Graph

```
B-1 (ACPI) ──┬──> B-2 (APIC Timer)
              │         │
              │         ├──> B-3 (IPI/SMP)  ──> B-7 (Lock-Free)
              │         │
              │         ├──> B-4 (PCI/PCIe) ──┬──> B-8 (NVMe)
              │         │                      │
              │         │                      ├──> B-9 (Networking)
              │         │                      │
              ├──> B-5 (DMA/IOMMU) ────────────┘
              │
              └──> B-10 (Profiling/PMU)

B-6 (Shared Mem + Unix Sockets) -- independent, can parallel with any sprint
B-11 (Huge Pages) -- stretch goal
B-12 (Dynamic Linker) -- stretch goal
```

---

## Sprint B-1: ACPI Table Parser (v0.5.9)

**Goal**: Parse ACPI tables from UEFI firmware to discover hardware topology.
**Depends on**: None
**Blocked by**: None

- [ ] Discover RSDP from UEFI system table
- [ ] Parse RSDT/XSDT to enumerate child tables
- [ ] Parse MADT (Multiple APIC Description Table)
  - [ ] Local APIC entries (CPU enumeration)
  - [ ] I/O APIC entries (interrupt routing)
  - [ ] Interrupt source overrides
- [ ] Parse MCFG (PCI Express config space base addresses)
- [ ] Store parsed data in static structures
- [ ] `acpi` shell command to dump parsed tables

**Key files**:
- CREATE: `kernel/src/arch/x86_64/acpi.rs` (~500-700 lines)
- MODIFY: `kernel/src/arch/x86_64/mod.rs` (pub mod acpi)
- MODIFY: `kernel/src/arch/x86_64/boot.rs` (call acpi::init())

**Not in scope**: Full AML interpreter, ACPI namespace, runtime ACPI methods.

**Verification**: `acpi` shell command dumps MADT/MCFG; tri-arch build clean.

---

## Sprint B-2: APIC Timer + Interrupt Wiring (v0.5.9)

**Goal**: Wire APIC timer to IDT for preemptive scheduling.
**Depends on**: B-1 (MADT provides APIC base address)
**Blocked by**: B-1

- [ ] Register timer interrupt handler at IDT vector 32
- [ ] Calibrate APIC timer frequency (PIT or TSC reference)
- [ ] Configure periodic mode at ~1000 Hz (1ms tick)
- [ ] Timer handler calls `scheduler::timer_tick()`
- [ ] Time slice expiry handling in scheduler
- [ ] EOI (End Of Interrupt) after handler

**Key files**:
- MODIFY: `kernel/src/arch/x86_64/apic.rs` (calibrate, start wiring)
- MODIFY: `kernel/src/arch/x86_64/idt.rs` (vector 32 handler)
- MODIFY: `kernel/src/sched/scheduler.rs` (timer_tick() entry point)

**Verification**: Timer tick counter increments ~1000/sec; preemption observed.

---

## Sprint B-3: IPI + SMP Foundation (v0.5.10)

**Goal**: Enable inter-processor interrupts for cross-CPU coordination.
**Depends on**: B-1 (MADT), B-2 (APIC initialized)
**Blocked by**: B-2

- [ ] Wire `send_ipi()` (exists in apic.rs as dead_code) to scheduler
- [ ] TLB shootdown IPI (signal other CPUs to flush page table changes)
- [ ] Scheduler IPI (wake remote CPU when placing task on its run queue)
- [ ] AP (Application Processor) startup sequence (INIT-SIPI-SIPI)
- [ ] Per-CPU initialization for APs

**Key files**:
- MODIFY: `kernel/src/arch/x86_64/apic.rs` (send_ipi, IPI handlers)
- MODIFY: `kernel/src/sched/smp.rs` (AP boot, cross-CPU wake)
- MODIFY: `kernel/src/mm/vas.rs` (TLB shootdown via IPI)

**Verification**: IPI delivery confirmed via per-CPU counter.

---

## Sprint B-4: PCI/PCIe Completion (v0.5.10)

**Goal**: Complete PCI subsystem for modern device support.
**Depends on**: B-1 (MCFG table), B-2 (APIC IRQ routing)
**Blocked by**: B-2

- [ ] PCI bridge enumeration (scan secondary buses)
- [ ] MSI (Message Signaled Interrupts) capability configuration
- [ ] MSI-X support (table and PBA configuration)
- [ ] PCIe MCFG-based memory-mapped config access
- [ ] Wire interrupt allocation to I/O APIC or MSI

**Key files**:
- MODIFY: `kernel/src/drivers/pci.rs` (bridge scan, MSI/MSI-X, MCFG)
- MODIFY: `kernel/src/arch/x86_64/apic.rs` (MSI vector allocation)

**Verification**: PCI enumeration finds devices behind bridges; MSI vector allocated.

---

## Sprint B-5: DMA + IOMMU Foundation (v0.5.11)

**Goal**: Hardware-safe DMA with address translation.
**Depends on**: B-1 (DMAR table from ACPI)
**Blocked by**: B-1

- [ ] Intel VT-d detection via ACPI DMAR table
- [ ] Basic IOMMU page table setup (identity map for safe devices)
- [ ] DMA coherency flags in DmaBuffer (dma_pool.rs)
- [ ] Scatter-gather list support for multi-buffer DMA

**Key files**:
- CREATE: `kernel/src/drivers/iommu.rs` (~300 lines)
- MODIFY: `kernel/src/net/dma_pool.rs` (scatter-gather, coherency flags)

**Verification**: DMA buffer allocated with IOMMU identity mapping.

---

## Sprint B-6: Shared Memory + Unix Domain Sockets (v0.5.11)

**Goal**: POSIX shared memory and Unix sockets for GUI IPC.
**Depends on**: None (independent)
**Blocked by**: None

- [ ] POSIX shm
  - [ ] `shm_open()` / `shm_unlink()` syscalls
  - [ ] `/dev/shm` tmpfs mount
  - [ ] `MAP_SHARED` mmap flag support
- [ ] Unix domain sockets
  - [ ] AF_UNIX socket type with path-based binding
  - [ ] Stream and datagram modes
  - [ ] `socketpair()` syscall
  - [ ] SCM_RIGHTS (fd passing) for Wayland buffer handles

**Key files**:
- CREATE: `kernel/src/ipc/posix_shm.rs` (~200 lines)
- CREATE: `kernel/src/net/unix_socket.rs` (~400 lines)
- MODIFY: `kernel/src/syscall/mod.rs` (shm_open, shm_unlink, socketpair)
- MODIFY: `kernel/src/net/socket.rs` (AF_UNIX dispatch)

**Verification**: Unix socket echo test; shm_open/mmap/shm_unlink cycle.

---

## Sprint B-7: Lock-Free Kernel Paths (v0.5.11)

**Goal**: Replace spin::Mutex on critical paths with lock-free structures.
**Depends on**: B-3 (IPI needed for RCU grace period detection)
**Blocked by**: B-3

- [ ] RCU (Read-Copy-Update)
  - [ ] `read_lock()` / `read_unlock()` (reader-side)
  - [ ] `synchronize_rcu()` (writer-side grace period)
  - [ ] `call_rcu()` (deferred callback)
- [ ] Wait-free ready queue
  - [ ] Lock-free MPSC queue (AtomicPtr-based linked list)
  - [ ] Integrate into scheduler ready queue
- [ ] Hazard pointers
  - [ ] Per-CPU hazard pointer slots
  - [ ] Safe memory reclamation
- [ ] Lock-free IPC registry
  - [ ] Atomic hash map (open addressing, linear probing)

**Key files**:
- CREATE: `kernel/src/sync/rcu.rs` (~300 lines)
- CREATE: `kernel/src/sync/hazard.rs` (~200 lines)
- CREATE: `kernel/src/sync/lockfree_queue.rs` (~150 lines)
- MODIFY: `kernel/src/sched/scheduler.rs` (lock-free ready queue)
- MODIFY: `kernel/src/ipc/registry.rs` (lock-free channel lookup)

**Verification**: RCU read-side benchmark shows zero-lock overhead; lock-free queue throughput.

---

## Sprint B-8: NVMe Driver Completion + Multi-Queue (v0.5.12)

**Goal**: Complete NVMe driver with multi-queue I/O.
**Depends on**: B-4 (MSI/MSI-X), B-5 (DMA infrastructure)
**Blocked by**: B-4, B-5

- [ ] Admin queue initialization (doorbell, command submission, completion polling)
- [ ] I/O queue pair creation (1 per CPU)
- [ ] Read/write command submission with DMA buffers
- [ ] Completion interrupt handling (MSI/MSI-X)
- [ ] Multi-queue scheduling (per-CPU I/O queues with interrupt affinity)
- [ ] Priority-aware queue selection

**Key files**:
- MODIFY: `kernel/src/drivers/nvme.rs` (~500 lines added)
- MODIFY: `kernel/src/net/dma_pool.rs` (NVMe-compatible DMA buffers)

**Verification**: NVMe identify controller returns valid serial/model; read/write 4K block.

---

## Sprint B-9: DPDK-Style Networking + Network Driver Completion (v0.5.12)

**Goal**: Complete network drivers and kernel-bypass infrastructure.
**Depends on**: B-4 (MSI/MSI-X), B-5 (DMA)
**Blocked by**: B-4, B-5

- [ ] VirtIO-Net driver completion
  - [ ] Functional TX/RX via virtqueues
  - [ ] Interrupt-driven receive
  - [ ] DMA buffer management
- [ ] E1000 driver completion
  - [ ] Basic TX/RX for Intel gigabit
- [ ] Receive Side Scaling (RSS)
  - [ ] Multi-queue receive with hash-based flow distribution
- [ ] Kernel bypass framework
  - [ ] User-space NIC queue access via mmap'd ring buffers
- [ ] XDP/eBPF hooks
  - [ ] Packet filter attachment points at driver level
  - [ ] Simple bytecode verifier + JIT for x86_64
- [ ] Hardware offload
  - [ ] TCP/UDP checksum offload
  - [ ] TSO/LRO for large transfers
- [ ] Lock-free packet processing
  - [ ] Per-CPU RX/TX queues
  - [ ] NAPI-style batch processing

**Key files**:
- MODIFY: `kernel/src/drivers/virtio_net.rs` (~400 lines added)
- MODIFY: `kernel/src/drivers/e1000.rs` (~300 lines added)
- CREATE: `kernel/src/net/bypass.rs` (~300 lines)
- CREATE: `kernel/src/net/xdp.rs` (~400 lines)
- MODIFY: `kernel/src/net/device.rs` (RSS, offload)

**Verification**: VirtIO-Net ping test; packet counter increments.

---

## Sprint B-10: Profiling Tools + Hardware PMU (v0.5.12)

**Goal**: Hardware performance counters and sampling profiler.
**Depends on**: B-2 (APIC timer for sampling interrupt)
**Blocked by**: B-2

- [ ] Hardware PMU driver (x86_64)
  - [ ] Configure IA32_PERFEVTSELx and IA32_PMCx MSRs
  - [ ] Events: instructions retired, cache misses, branch mispredicts, TLB misses
  - [ ] RDPMC for user-space counter reads
  - [ ] Counter multiplexing
- [ ] Sampling profiler
  - [ ] Timer-interrupt-driven IP sampling (100Hz-10kHz)
  - [ ] Per-CPU sample buffers
  - [ ] `perf record` / `perf report` shell builtins
- [ ] Call graph generation
  - [ ] Frame pointer-based stack walking
  - [ ] Top functions by sample count
- [ ] AArch64 PMU (PMCR_EL0, PMCNTENSET_EL0)
- [ ] RISC-V HPM (mcycle, minstret, mhpmcounter)

**Key files**:
- CREATE: `kernel/src/perf/pmu.rs` (~400 lines)
- CREATE: `kernel/src/perf/sampler.rs` (~250 lines)
- MODIFY: `kernel/src/perf/mod.rs` (pub mod pmu, sampler)
- MODIFY: `kernel/src/services/shell/commands.rs` (perf record, perf report)

**Verification**: `perf record` captures IP samples; `perf report` shows top functions.

---

## Sprint B-11: Huge Pages (Stretch Goal, v0.5.13)

**Goal**: 2MB huge page support for reduced TLB pressure.
**Depends on**: None
**Blocked by**: None

- [ ] L2 page table direct mapping (2MB pages)
- [ ] Transparent huge page (THP) promotion for anonymous memory
- [ ] Huge page frame allocator (buddy system 2MB alignment)
- [ ] TLB miss reduction measurement

**Key files**:
- MODIFY: `kernel/src/mm/vas.rs` (2MB page table entries)
- MODIFY: `kernel/src/mm/frame_allocator.rs` (2MB-aligned allocation)

**Verification**: 2MB page allocation; TLB miss reduction measured.

---

## Sprint B-12: Dynamic Linker (Stretch Goal, v0.5.13)

**Goal**: Basic `ld.so` for shared library support.
**Depends on**: None
**Blocked by**: None

- [ ] PT_INTERP handling in ELF loader
- [ ] Runtime relocation processing
- [ ] Lazy PLT binding
- [ ] Library search path (`/lib`, `/usr/lib`)
- [ ] `dlopen()` / `dlsym()` / `dlclose()` API

**Key files**:
- CREATE: `userland/ld-veridian/` (~800 lines)
- MODIFY: `kernel/src/process/elf.rs` (PT_INTERP delegation)

**Verification**: Dynamic ELF with PT_INTERP loads and runs via ld.so.

---

## Items Remaining in Phase 6

These items are NOT planned for Phase 5.5:

| Item | Rationale |
|------|-----------|
| Full AML interpreter | Only basic table parsing needed; AML is complex runtime |
| Wayland compositor | Requires B-6 (shared memory + Unix sockets) first |
| Desktop environment | Requires Wayland compositor |
| Container runtime | Requires namespace isolation, cgroups |
| Self-hosted Rust compiler | Requires complete POSIX layer + dynamic linker |
| Power management (DVFS, C-states) | Requires ACPI runtime methods |

---

## Progress Tracking

| Sprint | Component | Analysis | Implementation | Testing | Complete |
|--------|-----------|----------|---------------|---------|----------|
| B-1 | ACPI Parser | Not Started | Not Started | Not Started | 0% |
| B-2 | APIC Timer | Not Started | Not Started | Not Started | 0% |
| B-3 | IPI/SMP | Not Started | Not Started | Not Started | 0% |
| B-4 | PCI/PCIe | Not Started | Not Started | Not Started | 0% |
| B-5 | DMA/IOMMU | Not Started | Not Started | Not Started | 0% |
| B-6 | Shared Mem/Unix Sockets | Not Started | Not Started | Not Started | 0% |
| B-7 | Lock-Free Paths | Not Started | Not Started | Not Started | 0% |
| B-8 | NVMe Driver | Not Started | Not Started | Not Started | 0% |
| B-9 | Network Drivers | Not Started | Not Started | Not Started | 0% |
| B-10 | Profiling/PMU | Not Started | Not Started | Not Started | 0% |
| B-11 | Huge Pages | Not Started | Not Started | Not Started | 0% |
| B-12 | Dynamic Linker | Not Started | Not Started | Not Started | 0% |

## Timeline

- **Wave 1** (v0.5.9): B-1 (ACPI) + B-2 (APIC Timer) -- hardware foundation
- **Wave 2** (v0.5.10): B-3 (IPI/SMP) + B-4 (PCI/PCIe) -- multi-core + device infra
- **Wave 3** (v0.5.11): B-5 (DMA) + B-6 (Shared Mem) + B-7 (Lock-Free) -- I/O + IPC
- **Wave 4** (v0.5.12): B-8 (NVMe) + B-9 (Networking) + B-10 (Profiling) -- drivers + tools
- **Wave 5** (v0.5.13): B-11 (Huge Pages) + B-12 (Dynamic Linker) -- stretch goals

---

**Previous Phase**: [Phase 5 - Performance Optimization](PHASE5_TODO.md)
**Next Phase**: [Phase 6 - Advanced Features](PHASE6_TODO.md)

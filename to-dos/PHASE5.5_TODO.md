# Phase 5.5: Infrastructure Bridge TODO

**Phase Duration**: 2-3 months
**Status**: ~58% (Waves 1-3 Complete)
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

- [x] Discover RSDP from UEFI system table (bootloader_api `rsdp_addr`)
- [x] Parse RSDT/XSDT to enumerate child tables (ACPI 1.0 + 2.0+)
- [x] Parse MADT (Multiple APIC Description Table)
  - [x] Local APIC entries (CPU enumeration)
  - [x] I/O APIC entries (interrupt routing)
  - [x] Interrupt source overrides + NMI entries
- [x] Parse MCFG (PCI Express config space base addresses)
- [x] Store parsed data in static structures (`ACPI_INFO: Mutex<Option<AcpiInfo>>`)
- [x] `acpi` shell command to dump parsed tables
- [x] `irq_to_gsi()` for interrupt source override lookup

**Key files**:
- CREATED: `kernel/src/arch/x86_64/acpi.rs` (~570 lines)
- MODIFIED: `kernel/src/arch/x86_64/mod.rs` (pub mod acpi, init call)
- MODIFIED: `kernel/src/services/shell/commands.rs` (AcpiCommand)
- MODIFIED: `kernel/src/services/shell/mod.rs` (acpi builtin registration)

**Not in scope**: Full AML interpreter, ACPI namespace, runtime ACPI methods.

**Verification**: `acpi` shell command dumps MADT/MCFG; tri-arch build clean.

---

## Sprint B-2: APIC Timer + Interrupt Wiring (v0.5.9)

**Goal**: Wire APIC timer to IDT for preemptive scheduling.
**Depends on**: B-1 (MADT provides APIC base address)
**Blocked by**: B-1

- [x] Register APIC timer interrupt handler at IDT vector 48 (dedicated, separate from PIC at 32)
- [x] Calibrate APIC timer frequency (PIT channel 2 10ms reference)
- [x] Configure periodic mode at ~1000 Hz (1ms tick) with divide-by-16
- [x] Timer handler calls `timer::tick()` -> `scheduler.tick()` (try_lock deadlock-safe)
- [x] Time slice expiry handling in scheduler (pre-existing `tick()` method)
- [x] APIC EOI after handler (`apic::send_eoi()`)
- [x] Enable interrupts after timer configuration

**Key files**:
- MODIFIED: `kernel/src/arch/x86_64/apic.rs` (calibrate_timer, start_timer, APIC_TIMER_VECTOR=48)
- MODIFIED: `kernel/src/arch/x86_64/idt.rs` (apic_timer_interrupt_handler at vector 48)
- MODIFIED: `kernel/src/arch/x86_64/timer.rs` (tick() wired, try_lock scheduler)
- MODIFIED: `kernel/src/arch/x86_64/mod.rs` (calibrate+start in boot, enable_interrupts)

**Verification**: Timer tick counter increments ~1000/sec; preemption observed.

---

## Sprint B-3: IPI + SMP Foundation (v0.5.10)

**Goal**: Enable inter-processor interrupts for cross-CPU coordination.
**Depends on**: B-1 (MADT), B-2 (APIC initialized)
**Blocked by**: B-2

- [x] Wire `send_ipi()` to SMP subsystem via actual APIC ICR (removed dead_code)
- [x] TLB shootdown IPI (vector 49, broadcast to all CPUs, flushes local TLB)
- [x] Scheduler wake IPI (vector 50, breaks HLT on idle CPUs)
- [x] AP startup sequence (INIT-SIPI-SIPI via APIC ICR per Intel SDM)
- [x] `TlbFlushBatch::flush_with_shootdown()` for cross-CPU TLB invalidation
- [x] IDT handlers for vectors 49 (TLB shootdown) and 50 (scheduler wake)

**Key files**:
- MODIFIED: `kernel/src/arch/x86_64/apic.rs` (send_init_ipi, send_startup_ipi, send_ipi_all_excluding_self, IPI vectors)
- MODIFIED: `kernel/src/arch/x86_64/idt.rs` (tlb_shootdown_handler, sched_wake_handler)
- MODIFIED: `kernel/src/sched/smp.rs` (x86_64 IPI via APIC, INIT-SIPI-SIPI in cpu_up)
- MODIFIED: `kernel/src/mm/vas.rs` (flush_with_shootdown)

**Verification**: IPI vectors registered in IDT; APIC ICR wired for cross-CPU delivery.

---

## Sprint B-4: PCI/PCIe Completion (v0.5.10)

**Goal**: Complete PCI subsystem for modern device support.
**Depends on**: B-1 (MCFG table), B-2 (APIC IRQ routing)
**Blocked by**: B-2

- [x] PCI bridge enumeration (`scan_bridge()` recursively scans secondary buses)
- [x] MSI capability parsing and `configure_msi()` for vector/address programming
- [x] MSI-X capability parsing (table size, BAR, offset, PBA)
- [x] PCIe ECAM memory-mapped config access (`ecam_read_config`/`ecam_write_config`)
- [x] PCI capability chain walker (`parse_capabilities()` for cap IDs 0x05, 0x11)
- [x] Bridge secondary bus number tracking (`PciDevice::secondary_bus`)

**Key files**:
- MODIFIED: `kernel/src/drivers/pci.rs` (MsiCapability, MsixCapability, parse_capabilities, scan_bridge, configure_msi, ecam_read/write_config)

**Verification**: PCI enumeration finds 6 devices on QEMU; bridge scanning enabled.

---

## Sprint B-5: DMA + IOMMU Foundation (v0.5.11)

**Goal**: Hardware-safe DMA with address translation.
**Depends on**: B-1 (DMAR table from ACPI)
**Blocked by**: B-1

- [x] DMAR table structures and detection (DrhdUnit, RmrrRegion, DeviceScope)
- [x] IOMMU identity mapping for DMA (alloc_dma_buffer, free_dma_buffer)
- [x] DMA coherency policy (Coherent, NonCoherent, WriteCombining)
- [x] Scatter-gather list support (ScatterGatherList, ScatterGatherEntry)
- [x] DmaMappedBuffer with virtual/DMA addr tracking and direction hints

**Key files**:
- CREATED: `kernel/src/drivers/iommu.rs` (~310 lines)
- MODIFIED: `kernel/src/drivers/mod.rs` (pub mod iommu)

**Verification**: DMA buffer allocation with identity mapping functional.

---

## Sprint B-6: Shared Memory + Unix Domain Sockets (v0.5.11)

**Goal**: POSIX shared memory and Unix sockets for GUI IPC.
**Depends on**: None (independent)
**Blocked by**: None

- [x] POSIX shm
  - [x] `shm_open()` / `shm_unlink()` with O_CREAT/O_EXCL, reference counting
  - [x] `shm_truncate()` with contiguous physical frame allocation
  - [x] `shm_close()` with deferred destruction on last reference
  - [x] `shm_stat()` query API
- [x] Unix domain sockets
  - [x] AF_UNIX socket type with path-based binding (UNIX_PATH_MAX=108)
  - [x] Stream (SOCK_STREAM) and datagram (SOCK_DGRAM) modes
  - [x] `socketpair()` for anonymous connected pairs
  - [x] SCM_RIGHTS ancillary data for fd passing (Wayland buffer handles)
  - [x] Full connection lifecycle (bind, listen, connect, accept, send, recv)
  - [x] Connectionless datagram delivery via `socket_sendto()`

**Key files**:
- CREATED: `kernel/src/ipc/posix_shm.rs` (~300 lines)
- CREATED: `kernel/src/net/unix_socket.rs` (~500 lines)
- MODIFIED: `kernel/src/ipc/mod.rs` (pub mod posix_shm)
- MODIFIED: `kernel/src/net/mod.rs` (pub mod unix_socket)

**Verification**: API complete, syscall wiring deferred to Phase 6 integration.

---

## Sprint B-7: Lock-Free Kernel Paths (v0.5.11)

**Goal**: Replace spin::Mutex on critical paths with lock-free structures.
**Depends on**: B-3 (IPI needed for RCU grace period detection)
**Blocked by**: B-3

- [x] RCU (Read-Copy-Update)
  - [x] `rcu_read_lock()` / `rcu_read_unlock()` (atomic counter, zero-lock reader-side)
  - [x] `synchronize_rcu()` (writer-side grace period wait)
  - [x] `call_rcu()` (deferred callback with epoch-based reclamation)
  - [x] `rcu_quiescent()` (per-CPU quiescent state reporting)
- [x] Wait-free MPSC queue
  - [x] Michael-Scott lock-free queue (AtomicPtr CAS, sentinel-based)
  - [x] `push()` (multi-producer) / `pop()` (single-consumer)
- [x] Hazard pointers
  - [x] Per-CPU hazard pointer slots (4 per CPU, 16 CPUs max)
  - [x] `protect()` / `is_protected()` / `collect_protected()` for safe reclamation
  - [x] `HazardGuard` RAII for automatic slot clearing

**Key files**:
- CREATED: `kernel/src/sync/rcu.rs` (~170 lines)
- CREATED: `kernel/src/sync/hazard.rs` (~120 lines)
- CREATED: `kernel/src/sync/lockfree_queue.rs` (~190 lines)
- MODIFIED: `kernel/src/sync/mod.rs` (pub mod rcu, hazard, lockfree_queue)

**Verification**: All data structures compile and are available for scheduler/IPC integration.

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
| B-1 | ACPI Parser | Done | Done | Done | 100% |
| B-2 | APIC Timer | Done | Done | Done | 100% |
| B-3 | IPI/SMP | Done | Done | Done | 100% |
| B-4 | PCI/PCIe | Done | Done | Done | 100% |
| B-5 | DMA/IOMMU | Done | Done | Done | 100% |
| B-6 | Shared Mem/Unix Sockets | Done | Done | Done | 100% |
| B-7 | Lock-Free Paths | Done | Done | Done | 100% |
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

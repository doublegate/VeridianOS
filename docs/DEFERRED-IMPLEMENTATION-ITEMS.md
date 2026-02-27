# Deferred Implementation Items

**Last Updated**: February 27, 2026 (v0.5.8)

This document tracks items explicitly deferred from their original implementation phase, with rationale and target phase for each.

For Phase 0-4 deferred items and pre-Phase 2 fixes, see the detailed documents in [`docs/deferred/`](deferred/).

---

## Phase 5 Deferred Items

Items identified during Phase 5 (Performance Optimization) that require infrastructure not yet available:

| Item | Original Phase | Rationale | Target Phase |
|------|---------------|-----------|--------------|
| Lock-free algorithms (RCU, hazard pointers) | Phase 5 | Requires SMP multi-hart with cross-CPU validation | Phase 5.5 (B-7) |
| Huge pages (2MB THP) | Phase 5 | Requires VMM infrastructure changes for L2 direct mapping | Phase 5.5 (B-11) |
| Deadline scheduling (EDF) | Phase 5 | Requires APIC timer integration and real-time task model | Phase 6 |
| NUMA optimizations | Phase 5 | Requires multi-node hardware testing | Phase 6 |
| TLB prefetching | Phase 5 | Requires workload-specific heuristics | Phase 6 |
| Cache-aware allocation | Phase 5 | Requires cache topology detection | Phase 6 |
| False sharing elimination | Phase 5 | Requires SMP multi-hart | Phase 6 |
| Dynamic tracing (kprobes) | Phase 5 | Requires code patching infrastructure | Phase 6 |
| Hardware perf counters (PMU) | Phase 5 | Requires PMU driver and MSR access infrastructure | Phase 5.5 (B-10) |
| Memory bandwidth benchmarks | Phase 5 | Requires streaming memory test with NUMA awareness | Phase 5.5 |
| Performance regression tests | Phase 5 | Requires automated CI benchmark comparison | Phase 6 |

## Phase 5.5 Bridge Items

Items that serve as prerequisites for Phase 6 (Advanced Features/GUI):

| Item | Rationale | Sprint |
|------|-----------|--------|
| ACPI table parser | CPU topology, IRQ routing, PCIe config space discovery | B-1 |
| APIC timer + interrupt wiring | Timer-driven preemption for responsive GUI | B-2 |
| IPI + SMP foundation | Cross-CPU coordination (TLB shootdown, scheduler wake) | B-3 |
| PCI/PCIe completion (MSI/MSI-X) | Required by virtio-gpu, USB, NVMe | B-4 |
| DMA + IOMMU foundation | Hardware-safe DMA for GPU and network drivers | B-5 |
| Shared memory + Unix domain sockets | Wayland compositor IPC (buffer passing, fd passing) | B-6 |
| Lock-free kernel paths | Sub-microsecond IPC, high-throughput scheduling | B-7 |
| NVMe driver + multi-queue | Standard storage interface for modern hardware | B-8 |
| Network driver completion | Functional TX/RX for virtio-net and e1000 | B-9 |
| Profiling tools + hardware PMU | Cache miss, branch mispredict measurement | B-10 |

## Items Staying in Phase 6+

These items are NOT planned for Phase 5.5 and remain in Phase 6 or later:

| Item | Rationale |
|------|-----------|
| Full AML interpreter | Only basic ACPI table parsing needed; AML requires complex runtime |
| Wayland compositor | Requires shared memory + Unix sockets (Phase 5.5 B-6) first |
| Container runtime | Requires namespace isolation and cgroup infrastructure |
| Self-hosted Rust compiler | Requires complete POSIX layer and dynamic linker |
| io_uring | Requires user-space driver infrastructure |
| Power management (DVFS, C-states) | Requires ACPI runtime methods |

## Pre-Phase 2 Deferred Items (Historical)

The `docs/deferred/` directory contains 7 category files documenting items deferred during the Phases 0-4 audit:

- [`00-INDEX.md`](deferred/00-INDEX.md) -- Master index
- [`01-CRITICAL-ARCHITECTURE-ISSUES.md`](deferred/01-CRITICAL-ARCHITECTURE-ISSUES.md) -- Architecture-level fixes
- [`02-SCHEDULER-PROCESS-MANAGEMENT.md`](deferred/02-SCHEDULER-PROCESS-MANAGEMENT.md) -- Scheduler gaps
- [`03-MEMORY-MANAGEMENT.md`](deferred/03-MEMORY-MANAGEMENT.md) -- VMM/allocator gaps
- [`04-IPC-CAPABILITY-SYSTEM.md`](deferred/04-IPC-CAPABILITY-SYSTEM.md) -- IPC integration gaps
- [`05-BUILD-TEST-INFRASTRUCTURE.md`](deferred/05-BUILD-TEST-INFRASTRUCTURE.md) -- Testing limitations
- [`06-CODE-QUALITY-CLEANUP.md`](deferred/06-CODE-QUALITY-CLEANUP.md) -- Tech debt items
- [`07-FUTURE-FEATURES.md`](deferred/07-FUTURE-FEATURES.md) -- Future feature ideas

For Phases 0-4 gap tracking, see also [`to-dos/REMEDIATION_TODO.md`](../to-dos/REMEDIATION_TODO.md).

---

**See also**: [Phase 5 TODO](../to-dos/PHASE5_TODO.md) | [Phase 5.5 TODO](../to-dos/PHASE5.5_TODO.md) | [Phase 6 TODO](../to-dos/PHASE6_TODO.md)

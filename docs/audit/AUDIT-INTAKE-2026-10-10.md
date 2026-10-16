# Audit intake, 2026-10-10 matrix (supplement)

On 2026-10-09/10 another agent framework (Antigravity, an unmerged session) produced a
"performance and quality audit matrix" against `feat/v0.27`, plus an uncommitted remediation diff
and a handoff claiming 17 fixes. Neither was merged. The matrix, the diff and the handoff are
archived outside the repository (`/opt/veridian/archive/2026-10-10-antigravity/`), not committed:
the matrix's benchmark figures contradict its own harness (its reviewers loosened the assertions
until they passed; for example a claimed 13.92x TLB speed-up measured 0.84x), several IDs mean
something else than in the 2026-10-05 matrix (`docs/PERFORMANCE_AND_QUALITY_AUDIT_MATRIX.md`), and
its N-65/N-76 labels are swapped.

Every finding was re-checked against the tree on 2026-10-14 by three read-only reviews (claim
status, code verification, reachability). This file is the tracking list for the IDs it adds
(**N-261 to N-282**) and records where each matrix finding is tracked. **C** = confirmed in the
code, **L** = latent (unreachable today), as in the 2026-10-07 re-evaluation.

## The remediation that was not merged

| Claim | Status on 2026-10-14 |
|---|---|
| MEM-PERF-01/02, MEM-PERF-03/MEM-INC-01 (COW) | already in the tree before the session (f15a1442, 6fb55daa) |
| PB-MEM-04 targeted TLB shootdown | broken: `active_cpus` never set, so remote flushes were skipped; IPIs sent to logical CPU numbers, not APIC IDs. Reverted; tracked as N-262 |
| heap 1 GiB -> 32 MiB | breaks the rootfs, cc1 and KDE. Reverted; the low-memory boot is N-278 |
| BM-FS-01 / BM-DRV-06 (BlockFS) | broken: deadlocked on the first cache miss, wrote dirty metadata early (ADR 0003), gutted the tests. Reverted; done properly in 1ce99be9 (ADR 0011) |
| N-69 `knetd` | broken: `expect` panicked aarch64/riscv64 boot; never scheduled on x86_64. Reverted; still N-69 |
| N-80, N-83, N-87, N-88, N-89, CRIT-SEC-01, BM-FS-05, BM-DRV-02, SYS-PERF-01 | not implemented (files unchanged). Done since: see below |
| `tools/audit-models` 21 -> 36 models | thresholds loosened, comparisons rigged; not merged. The 36-model source was lost before the archive was taken (only compiled binaries survived) |

## New IDs

| ID | Sev | Finding | Matrix ID | Status / Target |
|---|---|---|---|---|
| N-261 | Medium L | `per_cpu_free_frame` has one caller (`perf/bench.rs`); every unmap, munmap and teardown free takes the global `FRAME_ALLOCATOR` lock | PB-MEM-01 | **open** (v0.27, D5): needs a per-CPU free that is checked. Measured on a release kernel (the first measurement, -6%/+5%, ran on a dev kernel where zeroing was ~95% of each mmap, N-281). Allocating in mmap and freeing in unmap through the per-CPU cache was much faster with four processes on four CPUs (16-page mmap 11-12 -> 4 us, 256 pages 1.1-1.7 ms -> 142 us), but the cache does not check a frame on free as the global allocator does (already free, not RAM), so a double free on these paths would hand one frame to two owners; it was reverted (commit review). Allocating from the cache while freeing globally was slower than either (16 pages, four processes: 70-113 us), as the cache refilled with frames nobody had used recently (cold in a VM). Next: per-frame allocation state that the cache can check without the global lock |
| N-262 | Medium L | TLB shootdown interrupts every CPU; a per-address-space active-CPU mask (set at the dispatcher's address-space switch) and logical-to-APIC-ID mapping would target only CPUs that ran it | PB-MEM-04 | **fixed** (v0.27, D5): each CPU records the root it loads (`tlb::note_root`, before the load); user-space flushes go to the CPUs on that root or with no record, kernel-half flushes to all; IPIs by APIC ID |
| N-263 | Medium C | Dirty BlockFS blocks were pinned until sync, so a large write grew the cache by its whole size | BM-FS-01 | **fixed** 1ce99be9 (ADR 0011) |
| N-264 | Low C | `mm/cache_topology.rs` (771 lines) has no callers, and `docs/PERFORMANCE-TUNING.md` says the frame allocator uses cache colouring | INC-MEM-02 | open; G (wire it into the allocator or remove it, and correct the doc) |
| N-265 | Low L | `translate_address` dereferenced page-table physical addresses, which are low user addresses since N-134 | INC-MEM-03 | **fixed** b0644391 (no live caller) |
| N-266 | Low C | heap.rs described RISC-V as using a bump allocator | INC-MEM-04 | **fixed** b0644391 |
| N-267 | Medium L | The demand-paging branch of the fault handler drops the address-space lock and, in `map_page`, inserts a one-page mapping overlapping the existing one (breaking the no-overlap rule `find_mapping` relies on) and pushes the frame at the wrong index; unreachable while every mapping is filled eagerly | INC-MEM-05 | open; v0.29 (lazy anonymous memory) |
| N-268 | Low C | Empty intermediate page tables are kept until exit or exec (bounded by the range a process touched) | ARCH-MEM-01 | open; v0.29 |
| N-269 | Low C | `thread_join` (native 1066) spins on `yield_cpu` with no wait queue and no EINTR | BM-SYS-03 | **fixed** (a dispatched joiner sleeps on `PROCESS_EVENTS` until the thread's task is reaped; EINTR on a signal) |
| N-270 | Medium C | VirtIO: no barrier between the avail-index store and the doorbell, and the ring fence was CPU-only | CRIT-SEC-01 | **fixed** 20b90360 (`dma_wmb`/`dma_rmb`/`io_wmb`) |
| N-271 | Low C | Barrier asm marked `nomem`, so it was no compiler barrier | CRIT-PERF-01 (as a performance finding: false) | **fixed** 20b90360 |
| N-272 | Low C | `SyscallError` defined three times (`error.rs` unused, `syscall/mod.rs` the ABI, `pkg/sdk`, plus `userland/rust-std`) | ARCH-ERR-01 | open; G |
| N-273 | Low C | The kernel shell's `mv` copied and unlinked | BM-FS-05 | **fixed** c7de04d9 |
| N-274 | Low C | `mm/ksm.rs` (KSM) has no callers (flat 4096-entry tables) | INC-MEM-01, PB-MEM-06 | open; G (wire it as a scanner or remove it) |
| N-275 | Low C | The kernel no longer builds without the `alloc` feature (unconditional `alloc` uses in procfs, cap/space, page-cache references), and no CI job builds that way | (review note) | open; G (make `alloc` unconditional) |
| N-276 | Low C | The reviewer workflow called the fork-approval setting "belt-and-suspenders" while it is the only fork gate (a fork's PR runs its own copy of the workflow) | (review note) | **fixed** with this file |
| N-277 | Low C | kernel stacks need physically contiguous frames although x86_64 maps them into a slot | ARCH-MEM-04 | duplicate of N-144 (G); no new work item |
| N-278 | Medium C | The kernel heap is a 1 GiB static array, so a guest with little memory cannot boot; the heap should be sized from the boot memory map | (handoff "QEMU OOM fix"); also HX-02 | **fixed** (frames sized from usable RAM, 32 MiB..1 GiB) |
| N-279 | Low L | Two futex tables: dispatched threads queue in `DFUTEX`, others in `FUTEX_TABLE`; a waiter and a waker on different tables miss each other (only possible for non-dispatched threads) | POC-SYS-02 | **fixed** (v0.27, D4c): the old table and the nested boot dispatch are deleted; every futex operation is the dispatcher's |
| N-280 | Low C | None of the bare-metal test programs in `kernel/tests/` compiles (stale signatures, a missing `test_main`, unresolved imports), and no CI job builds them, so `kernel/tests/README.md` describes tests that cannot run; the host unit tests, the in-kernel boot tests and the rootfs runtime suites are the real coverage (found 2026-10-15 while deleting `scheduler_tests.rs`, which tested only the old scheduler) | (review note) | open; G (port the useful checks to host or boot tests, then delete the directory, or fix and add a CI job) |
| N-281 | Medium L | Anonymous mmap was slow: 16 pages took ~570 us on one CPU (KVM). Recorded as "a page fault costs ~35 us", which was wrong: anonymous mappings are filled when made, and touching them takes no fault (0.2-0.3 us a page). Profiled with TSC probes in `sys_mmap`: ~95% was zeroing the frames with `write_bytes`, whose `memset` (from `compiler_builtins`, built by `-Zbuild-std` at the kernel's opt-level) runs ~30 cycles a byte in a dev kernel. Four processes on four CPUs took the same time as one: no contention | (D5 measurement) | **fixed** (v0.27, D5): `mm::page_ops::clear_page`/`copy_page` (`rep stosq`/`rep movsq` on x86_64, as Linux) at every whole-frame zero and copy. 16-page mmap 588 -> 31 us (dev), 19 -> 3 us (release); `perf` benchmark `page_clear` 32,889 -> 36 ns (dev), 868 -> 31 ns (release). Remaining variation for large mappings is the host backing guest RAM on first touch |
| N-282 | Low L | With four processes on four CPUs a 1-page anonymous mmap and munmap each take ~5 us against ~1 us for one process (release kernel, KVM), and the per-CPU frame cache (N-261) does not change it: some other lock or structure every mmap takes is shared (the heap, the process table, the syscall path) | (D5 measurement) | open; profile before changing anything |

## Matrix findings already tracked elsewhere

| Matrix ID | Verdict | Tracked as |
|---|---|---|
| PB-MEM-02 (eager mmap under the lock), PB-MEM-03 (`EXTRA_OWNERS` under `FRAME_ALLOCATOR`) | partly real (the outer per-process lock is the stall; the order is documented) | N-143, N-138 (v0.29) |
| PB-MEM-05 (linear VMA scan on COW faults), PB-MEM-07 (per-page bound check) | real | N-143; both **fixed** b0644391 |
| ARCH-MEM-02 (`Box` under the buddy free-list lock) | overstated: no AB-BA deadlock (the x86 heap never calls the frame allocator) | MEM-SEC-03 follow-up |
| ARCH-MEM-03 (no fault-tolerant usercopy on AArch64/RISC-V) | real, documented | E; RISC-V **fixed** (v0.27): copies with a fixup table, SUM only inside them (ADR 0014); compare-and-exchange and AArch64 open |
| ARCH-MEM-05 (page cache never evicts) | real | KNOWN-LIMITATIONS, v0.29 |
| IPC-SEC-01..03, IPC-SYNC-01..03, IPC-ARCH-01, IPC-PERF-01 | real | N-83 (**fixed** 9ec919ec), N-88 (partly fixed), N-80 (partly fixed), N-82, N-86, N-84, N-81, N-91 |
| CAP-SEC-01, CAP-SEC-02, ARCH-CAP-01 | real (ARCH-CAP-01's call sites are dead) | N-89, N-87 (both partly fixed 9ec919ec) |
| CAP-PERF-01 (one `RwLock` per capability space) | overstated: per process, the deliberate earlier fix | earlier CAP-PERF-01/02 |
| SCHED-INC-01 (priority inheritance is a stub) | real | SCHED-INC-01 (D) |
| POC-SMP-01 (`exit_task` uses CPU 0's scheduler), BM-SCHED-01, POC-SCHED-02b, SCHED-SEC-01 (`find_process`) | real but on the old scheduler path, unreachable once the dispatcher runs | D4 (retire the old scheduler), N-156 |
| SYS-PERF-01 (one global rate-limiter cache line) | real, minor | **fixed** (v0.27, D5): one cache-aligned bucket per CPU holding its share of the global budget |
| BM-DRV-06 (cache lock across disk I/O) | real | N-118: the BlockFS part **fixed** 1ce99be9 |
| SEC-DRV-01 (virtio-blk spins, no FLUSH) | real | N-52, N-121 (F5) |
| N-69, N-64, N-75 and the swapped N-65/N-76 | real | their own IDs (F) |
| AARCH-SEC-01 (PL011 TX FIFO not polled) | real | HA-05 (E/H) |
| ARCH-C6 (drivers in ring 0) | real | C6 (v0.28+) |

## False

| Matrix ID | Why |
|---|---|
| POC-SCHED-04 (rate-limiter underflow) | fixed earlier: `fetch_update` with `checked_sub`, covered by tests |
| BM-DRV-02 (8 MB back-buffer clone per frame) | presentation uses `with_back_buffer`; the cloning accessor had only a test caller (now `#[cfg(test)]`, c7de04d9) |
| CRIT-PERF-01 (`dsb sy` too heavy) | `memory_fence` has no callers and `data_sync_barrier` is used only on cold virtio-mmio setup paths (the `nomem` problem found while checking it is N-271) |

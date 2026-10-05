# Audit Verification Supplement -- 2026-10-05

Supplements `docs/PERFORMANCE_AND_QUALITY_AUDIT_MATRIX.md` (`AUDIT-MATRIX-VERIDIAN-2026.1`).
That matrix is kept unchanged as an immutable reference; corrections live here. This file is
also the remediation checklist for v0.26.0 and v0.27.0: the **Target** column says which
release closes each item, and **Status** is updated in the same commit as the fix.

## How this was verified

Every finding was re-read against the working tree on 2026-10-05, after commit `52e2e6a` plus
the uncommitted KDE Ring-3 bring-up work. Line numbers in the matrix had drifted; the
**Location** column below is current at the time of writing.

Verdicts:

- **CONFIRMED** -- the defect exists as described.
- **PARTIAL** -- the defect exists, but the described impact, location or mechanism is wrong
  in a way that changes the fix or its priority.
- **FALSE** -- the claimed defect does not exist as described.
- **LATENT** -- the code is wrong, but no current execution path reaches it.

## Two corrections that apply to the whole matrix

1. **Only CPU 0 runs, on every architecture.** `sched::smp::init` (`sched/smp.rs:303`) is
   BSP-only; `wake_up_aps` has no callers; the x86_64 INIT-SIPI trampoline at 0x8000 is not
   implemented (`smp.rs:543`); per-CPU data is never initialised, even for CPU 0, so
   `per_cpu(n)` is always `None`; the `smp` feature is off by default. Every SMP-contention,
   TLB-shootdown and data-race finding is therefore a real code defect but currently
   **LATENT**. They are fixed in the code paths now and become live tests once AP bring-up
   lands in v0.27.0.
2. **The empirical figures measure models, not the kernel.** `tests/audit_benchmarks.rs`
   (now `tools/audit-models/`) is a self-contained `std` program that imports nothing from
   `veridian-kernel`; it re-implements each structure and times the re-implementation. The
   figures are useful as order-of-magnitude illustrations of an algorithmic choice, not as
   measurements of VeridianOS. Two (BM-DRV-02, BM-IPC-06) time code paths that never execute
   in the kernel. Before/after kernel measurements for this remediation come from the
   in-kernel `perf` command and are recorded in `docs/PERFORMANCE-REPORT.md`.

## Findings

Paths are relative to `kernel/src/` unless they start with `userland/`.

### Memory management

| ID | Verdict | Location | Notes | Target | Status |
|---|---|---|---|---|---|
| MEM-PERF-01 | CONFIRMED, LATENT | `mm/frame_allocator.rs:1118-1155` | One `Mutex` wraps all 16 caches; refill/drain take `FRAME_ALLOCATOR` while holding it. Frees bypass the per-CPU path entirely (`vas.rs:1029,1757`). `MAX_CPUS` is defined five times with two values. | v0.26.0 | open |
| MEM-PERF-02 | CONFIRMED | `mm/frame_allocator.rs:250-285, 703-707` | Bit-by-bit scan from word 0; `stats.lock()` per allocation; also a per-allocation `reserved_regions.lock()` the matrix missed. | v0.26.0 | open |
| MEM-PERF-03 / MEM-INC-01 | CONFIRMED (worse) | `process/fork.rs:66-79`, `mm/vas.rs:596-727`, `mm/page_fault.rs:179` | Fork deep-copies every page *and* registers the parent's frames in a global CoW table keyed by frame number (the table is documented as keyed by virtual address). Nothing marks pages read-only and nothing removes the entries, so they accumulate. No `PageFlags::COW` bit and no per-frame refcount exist. | v0.27.0 | open |
| MEM-INC-02 | CONFIRMED | `mm/heap.rs:91-311`, `mm/vmm.rs` | Slab allocator builds slabs and discards them; every call goes to the fallback. `vmm.rs` is declared but nothing references it. | v0.27.0 | open |
| MEM-ARCH-01 | CONFIRMED, dead code | `mm/vas.rs:1791-1843`, `mm/page_table.rs:381` | `map_page` always installs an L1 entry. Correction: offsets past 4 KB are demand-paged, not faulted. `VirtualAddressSpace::map_huge_page` has no callers. | v0.27.0 | open |
| MEM-ARCH-02 | CONFIRMED | `mm/demand_paging.rs:174,324` | One global lazy-mapping table keyed only by virtual address; `unregister_lazy` and `handle_page_fault` have no callers; the real fault path never consults it. Entries leak and collide across processes. | v0.27.0 | open |
| MEM-ARCH-03 | PARTIAL, LATENT | `arch/aarch64/mod.rs:114-133`, `arch/riscv64/mod.rs:134-150` | Non-shareable `tlbi vae1`/`vmalle1` and local `sfence.vma`. `mm/page_table.rs:722` already uses the broadcast `tlbi vaae1is` form, so the code is inconsistent. SBI RFENCE is probed but `remote_sfence_vma` is not implemented. | v0.27.0 | open |
| MEM-SEC-01 | CONFIRMED (page walk); PARTIAL (impact) | `mm/user_validation.rs:32-85`, `syscall/userspace.rs:51-146` | Walks raw physical addresses as pointers. Live caller: `net/zero_copy.rs:337`. Correction: a kernel-mode fault on a user address does **not** unconditionally halt -- both HEAD and the WIP recover via `boot_return_to_kernel`; neither returns `EFAULT`. | v0.26.0 | open |
| MEM-SEC-02 | CONFIRMED, LATENT | `mm/vas.rs:425-441, 891, 947, 1019-1030, 1125-1137, ~1465` | `flush_with_shootdown` has zero callers and does not wait for acknowledgement. | v0.27.0 | open |
| MEM-SEC-03 | CONFIRMED (aarch64/riscv64 only) | `simple_alloc_unsafe.rs:149-174`, `lib.rs:25-39` | x86_64 uses `LockedHeap` and is unaffected. The race needs concurrent allocators (latent until SMP or interrupt-context allocation). Additional issue: a second bump allocator (`LOCKED_ALLOCATOR`) is initialised over the same region. | v0.26.0 (CAS + align), v0.27.0 (real heap) | open |
| MEM-SEC-04 | CONFIRMED, dead code | `mm/ksm.rs:296-327` | Hash-only merge in both trees. `KsmScanner` has no callers outside tests. | v0.26.0 | open |

### IPC and capabilities

| ID | Verdict | Location | Notes | Target | Status |
|---|---|---|---|---|---|
| IPC-INC-01 | CONFIRMED (worse) | `ipc/fast_path.rs:147-150, 236-254`, `syscall/mod.rs:2384-2447` | As described, plus: `sys_ipc_send` validates the syscall argument but `fast_send` checks the raw `msg.capability`; the cache has no process key, so one process's forged token is valid for all; `sync_send(message, capability)` passes the capability as `target_pid`. Correction: a type-0 (Memory) token with generation 0 and no flags is below 2^32 and passes the range check. | v0.26.0 | open |
| IPC-INC-02 | CONFIRMED (worse), dead code | `ipc/zero_copy.rs:278, 353-406` | Capability check ignores the region. `map` allocates a fresh zeroed frame, so "share" shares nothing and "move" loses data; `update_flags` never changes flags. No callers. | v0.26.0 | open |
| IPC-PERF-01 | PARTIAL | `ipc/fast_path.rs:46-50` | Single global `Mutex` despite the "per-CPU" comment: confirmed. **FALSE**: lock contention does not cause the bypass -- a cache miss returns `true` with or without contention (see IPC-INC-01). | v0.26.0 | open |
| IPC-PERF-02 | PARTIAL, dead code | `ipc/zero_copy.rs:312-332, 429-439` | `tlb_flush_all` is real, but no small message ever takes this path; the benchmark times code that does not run. | v0.26.0 | open |
| IPC-ARCH-01 | CONFIRMED | `ipc/registry.rs:25-58` | One global `Mutex` over three `BTreeMap`s, taken on the fast-path receive. | v0.26.0 | open |
| IPC-ARCH-02 | CONFIRMED | `ipc/` | No copy tier between 64-byte register messages and page remapping. | v0.26.0 | open |
| IPC-SYNC-01 | CONFIRMED (worse) | `ipc/channel.rs:161-172`, `sched/ipc_blocking.rs:18-235` | Lost wakeup as described; on SMP the same window can also double-enqueue a running task. No wait primitive with a pending-wakeup flag exists; `process/sync.rs` `WaitQueue::wait` has the same race. | v0.26.0 | open |
| IPC-SYNC-02 | CONFIRMED | `ipc/fast_path.rs:124-139` | Unlocked state check; never verifies the target is blocked on *this* endpoint, so a futex/sleep waiter can be woken with clobbered registers; no refcount on the task pointer. | v0.26.0 | open |
| IPC-SYNC-03 | CONFIRMED, test-only | `ipc/async_channel.rs:250-285, 368` | Unsound MPMC ring (and `capacity == 0` divides by zero). Only constructed in tests, but publicly re-exported. | v0.26.0 | open |
| IPC-SEC-01 | CONFIRMED (worse) | `ipc/registry.rs:408-427` | Endpoints are stored by value in a `BTreeMap`, which moves values on node split/merge, so *any* insert can dangle the `&'static`. Live caller: `syscall/mod.rs:2686`. Endpoint IDs come from two different counters. | v0.26.0 | open |
| CAP-INC-01 | PARTIAL, LATENT | `cap/revocation.rs:80-90` | Logic bug confirmed, but `cleanup` has no callers and production checks use `cap_manager().is_valid()`, not `REVOCATION_LIST`. | v0.26.0 | open |
| CAP-INC-02 | CONFIRMED, dead code | `cap/revocation.rs:288-342`, `cap/manager.rs:213-268, 295` | Derivation tree never populated; cascade rebuilds tokens without generation/type/flags; revocation is broadcast twice. | v0.27.0 | open |
| CAP-PERF-01 | CONFIRMED | `cap/space.rs:146-266`, `cap/manager.rs:52-95` | As described. | v0.26.0 | open |
| CAP-PERF-02 | CONFIRMED | `cap/space.rs:91, 223` | As described; also `IdAllocator::allocate` takes a write lock on every allocation. | v0.26.0 | open |

### Scheduler, process and syscalls

| ID | Verdict | Location | Notes | Target | Status |
|---|---|---|---|---|---|
| SCHED-PERF-01 | PARTIAL, LATENT | `sched/queue.rs:432`, `sched/percpu_queue.rs` | Global queue confirmed; `percpu_queue.rs` is declared but unreferenced. No contention exists while one CPU runs. | v0.27.0 | open |
| SCHED-PERF-02 | PARTIAL, LATENT | `sched/scheduler.rs:234-273, 428-439, 455-488` | Unbounded requeue in CFS, RR and hybrid pickers. Unreachable today (default algorithm is Priority; affinity never changes). | v0.26.0 | open |
| SCHED-PERF-03 | CONFIRMED | `sched/queue.rs:84-116, 422-458`, `sched/smp.rs:78` | ~72 KB `ReadyQueue` built on the stack; `smp` builds add 4.6 MB of BSS; `remove` is O(N) with a 2 KB copy. | v0.26.0 | open |
| SCHED-INC-01 | CONFIRMED | `process/sync.rs:672-680` | `Task::priority_boost` exists and is honoured for preemption, but is never set; the ready queue indexes by base priority. `PiMutex` has no users and `sync.rs` has no tests. | v0.27.0 | open |
| SCHED-INC-02 | CONFIRMED, dead code | `sched/deadline.rs` | 33 tests, zero callers. | v0.27.0 | open |
| SMP-PERF-01 | CONFIRMED | `sched/smp.rs`, `process/mod.rs` | See "Only CPU 0 runs" above. `current_process()` takes the global scheduler lock on every syscall. | v0.27.0 | open |
| PROC-ARCH-01 | CONFIRMED | `mm/vas.rs:596-727` | Same root cause as MEM-PERF-03. | v0.27.0 | open |
| SYS-PERF-01 | PARTIAL | `syscall/mod.rs:139-160` | Race confirmed. Impact overstated: the refill uses raw cycle counts, so the bucket refills to max on almost every syscall and is effectively never limiting. One global limiter, not per process. | v0.26.0 | fixed (371aa62) |
| SYS-PERF-02 | CONFIRMED, LATENT | `syscall/futex.rs:71` | Single global table. | v0.26.0 | open |
| SYS-CONC-01 | CONFIRMED, LATENT | `syscall/futex.rs:425-439` | Non-atomic RMW; also ignores `FUTEX_OP_OPARG_SHIFT`, never wakes `uaddr2` waiters, and runs without the table lock. Syscalls run with interrupts off, so on one CPU nothing interleaves today. | v0.26.0 | open |
| SYS-INC-01 | CONFIRMED, dead code | `syscall/linux_compat.rs:31-59` | 64-PID limit confirmed, but `set_linux_abi` has no callers (`userspace/loader.rs:118` says not to call it), so the bitmap is always empty. | v0.27.0 | open |
| SYS-SEC-01 | PARTIAL | `syscall/userspace.rs:51-146`, `arch/x86_64/idt.rs` | No `EFAULT` path and no fixup table: confirmed. The kernel does not halt on a user-address fault (see MEM-SEC-01). | v0.26.0 | open |
| PROC-SEC-01 | CONFIRMED | `process/table.rs:172-217` | `&'static` / `&'static mut` into boxed entries that `remove_process` drops. | v0.26.0 | open |
| PROC-SEC-02 | CONFIRMED (worse) | `process/mod.rs:234-262`, `process/exit.rs:660-750` | Frees the running kernel stack, and also drops the `Thread` and then reads `thread.clear_tid`. | v0.26.0 | open |

### Filesystems, drivers, network, desktop, virtualisation

| ID | Verdict | Location | Notes | Target | Status |
|---|---|---|---|---|---|
| FS-PERF-01 | PARTIAL | `fs/blockfs.rs:551, 586-589, 635-640, 991-1013` | Root cause misdescribed: BlockFS is RAM-resident with write-back, and `load_existing` reads *every* allocated block at mount (not only accessed blocks). `free_block` keeps the 4 KB `Vec`. | v0.26.0 | open |
| FS-PERF-02 | CONFIRMED | `fs/mod.rs:726-737` | Mostly read-lock cache-line traffic rather than a convoy (writers are rare). | v0.26.0 | open |
| FS-PERF-03 | CONFIRMED | `syscall/filesystem.rs:1699-1715` | Also non-atomic, drops metadata, fails for directories. `VfsNode` has no rename. | v0.26.0 | open |
| FS-ARCH-01 | PARTIAL | `fs/file.rs:293-354` | Linear free-slot scan, bounded at 1024 -- minor. | v0.26.0 | open |
| FS-SEC-01 | CONFIRMED (worse) | `fs/mod.rs:486-628` | Prefix hijack confirmed; also no `..` normalisation before mount lookup, relative symlinks resolve from `/`, and MAC checks the unresolved path. | v0.26.0 | open |
| FS-SEC-02 | CONFIRMED | `syscall/filesystem.rs:1724-1735, 2219-2247, 2737-2760` | No checks on chmod/fchmod/unlink/rename; chown/fchown are no-op successes. | v0.26.0 | open |
| DRV-PERF-01 | PARTIAL, dead code | `services/desktop_ipc.rs:213-221` | Only a struct definition; nothing handles `UpdateWindowContent`. The copies that do happen are DESK-ARCH-01. | v0.26.0 | open |
| DRV-PERF-02 | CONFIRMED | `fs/blockfs.rs:143-150`, `drivers/virtio/blk.rs:395, 484-496` | 8 requests per 4 KB block, each allocating and zeroing a frame and spinning up to 10M iterations. | v0.26.0 | open |
| DRV-INC-01 | CONFIRMED (dangerous) | `drivers/nvme.rs:426, 472` | With a real controller, reads DMA to physical address 0. | v0.26.0 | open |
| DRV-SEC-01 | CONFIRMED | `drivers/virtio_net.rs:603-621` | Descriptor freed before the device is notified; no barrier; non-volatile ring indices. | v0.26.0 | open |
| DRV-SEC-02 | CONFIRMED (worse for e1000) | `drivers/virtio_net.rs:410-520`, `drivers/e1000.rs:88-212` | Virtual addresses used as DMA addresses. e1000 programs addresses of fields on a stack frame that has since been returned from. | v0.26.0 | open |
| NET-PERF-01 | CONFIRMED | `net/http.rs:539, 555, 572, 608, 609, 641, 647, 664, 670` | Nine sites; the feed buffer is also unbounded. | v0.26.0 | open |
| NET-INC-01 | CONFIRMED | `net/tcp.rs:419-520` | Also: segments carry checksum 0 (see N-08). | v0.27.0 | open |
| NET-INC-02 | CONFIRMED | `net/wireguard.rs`, `services/shell/commands/network.rs:770` | Not bound to the stack; the `wg` command uses an all-zero key seed. | v0.27.0 | open |
| NET-ARCH-01 | CONFIRMED (worse) | `net/ip.rs:~249-263` | virtio-net registers as `eth1`, so on a virtio-net-only machine every IPv4 transmit is silently dropped. Routing table and gateway are ignored. | v0.26.0 | open |
| NET-SEC-01 | CONFIRMED, different line | `net/udp.rs:303-311` | `from_bytes` is safe; the panic is in `process_packet`. | v0.26.0 | fixed (399929b) |
| NET-SEC-02 | CONFIRMED | `syscall/network_ext_syscalls.rs:29-36, 150-152` | | v0.26.0 | fixed (399929b) |
| DESK-ARCH-01 | CONFIRMED | `desktop/wayland/buffer.rs:25-154`, `desktop/wayland/mod.rs:424-452` | Also: pool size is client-controlled and unbounded; `write_data` panics on a bad offset (see N-09). | v0.26.0 | open |
| VIRT-INC-01 | CONFIRMED | `virt/container.rs:117-121`, `virt/containers/oci.rs:429-439` | | v0.27.0 | open |

### Userland libc

| ID | Verdict | Location | Notes | Target | Status |
|---|---|---|---|---|---|
| LIBC-SEC-01 | CONFIRMED | `userland/libc/src/stdlib.c:82-191` | `pthread_create` exists, so multithreaded programs can race the allocator. | v0.26.0 | open |
| LIBC-SEC-02 | PARTIAL, LATENT | `userland/libc/src/stdio.c:510` | Comment is false, but current callers use bases 8/10/16; octal `u64::MAX` is exactly 22 digits. | v0.26.0 | open |
| LIBC-SEC-03 | CONFIRMED (worse) | `userland/libc/src/stdio.c:1036-1121` | Field widths are not parsed at all, so `%31s` misparses as well as overflowing. | v0.26.0 | open |

## Findings not in the matrix

| ID | Location | Defect | Target | Status |
|---|---|---|---|---|
| N-01 | `mm/frame_allocator.rs:229-334` | Bitmap is initialised all-free for 131072 frames regardless of node size, and `allocate` never checks `total_frames`: on nodes under 512 MB it returns frames past the end of RAM (hits aarch64/riscv64 at the 128 MB default). `free()` can underflow and partially frees before detecting a double free. | v0.26.0 | fixed (3f72251) |
| N-02 | `sched/scheduler.rs:~1018` | `schedule_on_cpu` silently drops the task when `per_cpu(cpu)` is `None` (always, today). | v0.26.0 | open |
| N-03 | `sched/task_management.rs:150-158` | `CLEANUP_QUEUE` is a function-local static nothing drains; dead tasks leak. | v0.26.0 | open |
| N-04 | `cap/space.rs:163, 217, 251-266` | L2 index `(cap_id >> 8) as u16` truncates and aliases IDs >= 2^24; `remove()` takes the slot before comparing tokens, wiping a different capability. | v0.26.0 | open |
| N-05 | `cap/manager.rs:52`, `cap/token.rs:241-279`, `cap/space.rs:409` | Three independent capability-ID allocators, all starting at 1. | v0.26.0 | open |
| N-06 | `fs/file.rs:293-305` | `FileTable::new` leaves fds 0-2 as free slots, so the first `open` returns fd 0. | v0.26.0 | open |
| N-07 | `mm/vas.rs:1419, 1478`, `mm/demand_paging.rs:265` | Unreachable CoW code paths (`vas.fork`, `vas.handle_page_fault`, `handle_cow_fault`). | v0.27.0 | open |
| N-08 | `net/tcp.rs:226-247` | TCP segments are built with checksum 0. | v0.27.0 | open |
| N-09 | `desktop/wayland/mod.rs:424-452`, `desktop/wayland/buffer.rs:150-154` | Unbounded client-controlled `wl_shm` pool size; `write_data` panics when `offset > len`. | v0.26.0 | open |
| N-10 | `drivers/virtio/blk.rs:488-495` | On timeout the request frame is freed while the device may still DMA into it. | v0.26.0 | open |
| N-11 | `drivers/nvme.rs` | No completion phase tracking; timeouts return `Ok`; `num_blocks - 1` underflows. | v0.26.0 | open |
| N-13 | riscv64 boot, after BOOTOK | Boot silently restarted from `_start` in Stage 6, so the second pass found singletons already initialised or zeroed (the various panics seen). Root cause: `current_cpu_id()` read the M-mode CSR `mhartid` from S-mode (illegal instruction) and no `stvec` was installed, so the trap jumped to the kernel entry. Separately, the hardcoded frame-pool start (0x80E00000) lay inside the grown kernel image (ends 0x81148000), so frames aliased the kernel heap and boot stack. BOOTOK and 29/29 print before Stage 6, which is why the boot check passed. | v0.26.0 | fixed (3067856) |
| N-14 | `sched/smp.rs` `current_cpu_id` (riscv64) | The logical CPU ID is read from `tp`, which is the user TLS pointer in U-mode. The future U-mode trap entry must restore the kernel `tp` from `sscratch` before any code calls `current_cpu_id`, or user code chooses its CPU identity. Latent: riscv64 has no U-mode entry yet. Raised by review of `3067856`. | v0.27.0 (with SMP/U-mode bring-up) | open |
| N-12 | test suite, CI | The host unit-test build did not compile (missing `alloc` imports in five test modules, `ScriptError` assertions stale since v0.17.1), and two eventfd/signalfd tests crashed the test binary by reaching the real scheduler from a blocking read. CI hid this because the coverage job -- the only job that runs host tests -- is `continue-on-error`. | v0.26.0 | fixed |

## Issues in the uncommitted KDE Ring-3 work (pre-commit review)

| ID | Location | Issue | Resolution |
|---|---|---|---|
| W-1 | `bootstrap.rs` | Hardcoded `/home/parobek/Code/VeridianOS/target/veridian-sysroot` directory chain. | Now one `option_env!("VERIDIAN_SYSROOT")` constant (default: the path the shipped rootfs binaries were built with). Removed entirely once `tools/cross` builds with `--prefix=/usr`. |
| W-2 | `syscall/filesystem.rs` `sys_close` | Silently refuses to close DRM fds (leak) to work around Mesa closing them after reading an empty `/proc/self/maps`. | `/proc/self/maps` now reports the real address space (`fs/procfs.rs`). The `sys_close` special case is removed (DRM hardening commit). **Pending:** a KDE boot to confirm kwin keeps its DRM fds now that `/proc/self/maps` is populated -- the cross-compiled sysroot and KDE rootfs are not present on the build machine and must be rebuilt with `tools/cross/build-all-kde.sh`. |
| W-3 | `arch/x86_64/idt.rs` | Kernel-mode faults on user addresses no longer try demand paging, so a syscall touching a not-yet-faulted user page aborts. | Fixed with the user-copy fault fixup (MEM-SEC-01 / SYS-SEC-01). |

### Security review of commit `a738914`

A dedicated review of the committed KDE work found the following. Items marked *pre-existing*
were already present at `52e2e6a`; the KDE work added more reachable surface to them. All are
reachable from an unprivileged process and are fixed first in v0.26.0 Sprint A.

| ID | Severity | Location | Issue | Status |
|---|---|---|---|---|
| W-4 | Critical | `syscall/filesystem.rs` `sys_ioctl`, `graphics/drm_ioctl.rs` | **Arbitrary kernel write.** `arg` is never validated before `drm_ioctl_dispatch`; handlers write through it and through nested user-supplied pointers (`unique_ptr`, `values_ptr`, `enum_blob_ptr`, `blob.data`, `props_ptr`, `*_id_ptr`, ...). Count checks run after the count is overwritten, so they never limit anything. *Pre-existing gate; new primitives.* | fixed (DRM hardening) |
| W-5 | Critical | `graphics/drm_ioctl.rs` `handle_mode_atomic` | **Arbitrary kernel read.** ATOMIC reads `count_props_ptr`/`props_ptr`/`prop_values_ptr` raw; a kernel address in `prop_values_ptr` is stored as `crtc.fb_id` and read back through `GETCRTC`. `PAGE_FLIP` gives a second, constrained read via the event's `user_data`. | fixed (DRM hardening) |
| W-6 | Critical | `syscall/memory.rs` DRM `mmap` | **Maps arbitrary physical memory, user-writable.** `length` is not bounded by the framebuffer, so a large mmap on a DRM fd exposes all RAM after it. *Pre-existing; PRIME fds and the real `fb_phys` make it more reachable.* | fixed (DRM hardening) |
| W-7 | High | `syscall/filesystem.rs`, `syscall/memory.rs` | DRM handling is selected by `path.contains("dri/")`, so any file under a directory named `dri` reaches the DRM paths. *Pre-existing.* | fixed (DRM hardening) |
| W-8 | High | `process/mod.rs` `current_process` | `BOOT_CURRENT_PID` (one global) now takes priority over the scheduler, so any task scheduled while it is set acts with the boot process's file table, address space and uid. `current_thread` still prefers the scheduler, so thread and process can disagree. | fixed (boot PID scoped to the dispatching task) |
| W-9 | High | `graphics/drm_ioctl.rs`, `graphics/gpu_accel.rs`, `fs/devfs.rs` | No DRM master: `AUTH_MAGIC`/`SET_MASTER` always succeed. One global vblank event queue: any process can inject events (kwin dereferences `user_data`) or drain kwin's events; `read` drops events that do not fit. | fixed (DRM hardening) |
| W-10 | Medium | `graphics/gpu_accel.rs` | Vblank event queue is unbounded (kernel heap exhaustion by looping `PAGE_FLIP`). | fixed (DRM hardening) |
| W-11 | Medium | `graphics/drm_ioctl.rs` PRIME | Global 8-entry fd-to-handle table keyed by raw fd number across processes (overflow overwrites another process's entry); `FD_TO_HANDLE` never checks the fd belongs to the caller; PRIME fds can never be closed (W-2's `contains("dri/card0")` matches `dri/card0-prime`). | fixed (DRM hardening) |
| W-12 | Medium | `arch/x86_64/idt.rs` | The `KERN_PF_USER_ADDR` path unwinds to the boot context while holding spinlocks (epoll registry, KMS, file table), no longer marks the task zombie, covers `cr2 < 0x1000` (hides kernel NULL dereferences), and runs `swapgs` unconditionally. | open |
| W-13 | Medium | `sti; hlt; cli` in epoll, poll, nanosleep, timerfd, futex | Enables interrupts mid-syscall, so the timer IRQ can `schedule()` on the syscall stack (feeds W-8). `timerfd_read` ignores the boot cooperative mode and can stall boot for 30 s per call. | open |
| W-14 | Low-Medium | `net/epoll.rs`, `syscall/filesystem.rs` poll | The Unix-socket readiness fallback treats any fd number as a global socket ID, ignoring ownership: cross-process readiness side channel. | open |
| W-15 | Medium | `syscall/linux_compat.rs` | `faccessat2` always returns success. `LINUX_FCHOWNAT` was 269 -- faccessat's number -- so every `faccessat()` was executed as `fchownat` (a no-op only because chown was unimplemented). | fixed (faccessat/faccessat2 enforce access; fchownat is 260) |
| W-16 | Low | `syscall/mod.rs` epoll_wait | `max_events * size_of::<EpollEvent>()` is unchecked; in release builds it wraps to a small validated length while the slice keeps the huge count. *Pre-existing; now also reachable via the 263/281 heuristics.* | open |
| W-17 | Low | `bootstrap.rs` | `fontconfig` cache directories are 0777 without the sticky bit (cache poisoning of files kwin/Qt parse). | open |
| W-19 | High | `drivers/evdev.rs` via `sys_ioctl` | evdev ioctls wrote through the raw user `arg` (e.g. `EVIOCGNAME` writes 64 bytes regardless of the size the command encodes). *Pre-existing.* | fixed (DRM hardening) |
| W-18 | Low | `fs/blockfs.rs` `link` | The same-filesystem check is an inode-number range test, so a hard link to a node from another filesystem with a colliding inode number links an arbitrary BlockFS inode. *Pre-existing.* | open |

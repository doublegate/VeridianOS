# VeridianOS Master TODO List

**Last Updated**: 2026-10-18 (v0.27.0 released: process model, SMP, user mode on all three architectures)

## Project Overview Status

- [x] **Phase 0: Foundation and Tooling** - COMPLETE (100%) v0.1.0 (June 7, 2025)
- [x] **Phase 1: Microkernel Core** - COMPLETE (100%) v0.2.0 (June 12, 2025)
- [x] **Phase 2: User Space Foundation** - COMPLETE (100%) v0.3.2 (February 14, 2026)
- [x] **Phase 3: Security Hardening** - COMPLETE (100%) v0.3.2 (February 14, 2026)
- [x] **Phase 4: Package Ecosystem** - COMPLETE (100%) v0.4.0 (February 15, 2026)
- [x] **Phase 5: Performance Optimization** - COMPLETE (100%) v0.16.2 (March 2026)
- [x] **Phase 6: Advanced Features & GUI** - ~100% (Wayland compositor, desktop renderer, input, TCP/IP, desktop apps) v0.6.4
- [x] **Phase 6.5: Rust Compiler + Bash-in-Rust Shell** - COMPLETE (100%) v0.7.0 (February 27, 2026)
- [x] **Phase 7: Production Readiness** - ~100% (All 6 Waves complete: GPU, Wayland, desktop, networking, multimedia, virtualization, security, performance) v0.7.1-v0.10.0
- [x] **Phase 8: Next-Generation Features** - COMPLETE (100%) v0.16.3 (March 2026)
- [x] **Phase 9: KDE Plasma 6 Porting** - COMPLETE (100%) v0.22.0 (March 2026)
- [x] **Phase 10: KDE Known Limitations Remediation** - COMPLETE (100%) v0.23.0 (March 2026)
- [x] **Phase 11: KDE Plasma 6 Default Desktop Integration** - COMPLETE (100%) v0.24.0 (March 2026)
- [x] **Phase 12: KDE Plasma 6 Cross-Compilation** - COMPLETE (100%) v0.25.0 (March 2026)

## Current Version: v0.27.0 (October 2026)

### v0.27.0: process model, SMP and user mode on all three architectures

Released 2026-10-18, cut early at the user's decision: sprints A to E, X1, credentials, dynamic
linking (part) and the first system calls of the Linux program are in it (CHANGELOG `[v0.27.0]`);
the items still open below are carried to v0.28 (list under v0.28.0).

### v0.26.0: audit remediation (P0 + P1)

Tracking checklist: `docs/audit/AUDIT-VERIFICATION-2026-10-05.md`.

- **Fixed:** 86 findings.
- **Deferred:** 2.
- **Open:** 24, all targeted at v0.27 or later.

What does not work yet: `docs/KNOWN-LIMITATIONS.md`.

### Roadmap after v0.26.0 (v0.27.0 released 2026-10-18)

- [x] **v0.27.0: process model and SMP (C5, X0), released 2026-10-18.** Items still open below
  are carried to v0.28 (see v0.28.0). Sprints, in dependency order; IDs are in
  `docs/audit/AUDIT-VERIFICATION-2026-10-05.md`:
  - [x] **A, foundations and hygiene:** done (N-07 moves to C with copy-on-write).
    - N-56 workspace lints and the clippy allow-list (C2).
    - N-55 FIPS 204 ML-DSA with NIST known-answer tests.
    - N-43 raw socket buffer reads; N-45 ramfs cross-fs link.
    - Dead code: N-07, `mm/vmm.rs`, MEM-INC-02 slab, CAP-INC-02 derivation, SYS-INC-01.
    - Coverage helpers (plan Addendum 3); C1 runtime-suite CI job.
  - [x] **B, per-CPU data and SMP (stage S1, ADR 0004):**
    - [x] BSP per-CPU data on every architecture; N-35 per-CPU syscall frame.
    - [x] AP bring-up behind `smp`: x86 INIT-SIPI-SIPI, AArch64 PSCI, RISC-V SBI HSM; 4/4 online
      on every arch in QEMU and in CI.
    - [x] TLB shootdown: MEM-SEC-02, MEM-ARCH-03.
    - Moved to D: per-CPU run queues and work stealing (SCHED-PERF-01, SMP-PERF-01; stage S2),
      N-51, N-52 and N-14 (with RISC-V U-mode in E). S2 needs scheduler-dispatched context
      switching, which D reworks. Today `schedule()` switches while its caller holds the
      scheduler lock, so a task that starts fresh never releases it. The two races become
      reachable only once tasks run concurrently.
  - [x] **C, memory** (guard pages x86_64 only until E):
    - Copy-on-write with frame refcounts: MEM-ARCH-02, PROC-ARCH-01, N-07.
    - Huge pages: MEM-ARCH-01.
    - Real AArch64/RISC-V heaps: MEM-SEC-03.
    - Kernel stack guard pages: N-26.
  - [x] **D0, urgent fixes reachable today** (re-evaluation, `docs/audit/AUDIT-REEVALUATION-2026-10-07.md`;
    do first, each with a regression test). Done 2026-10-07: runtime suites native 41/41 and musl
    6/6 (`musl_runtime_test`, CI `REQUIRE_MUSL=1`); remainders of partly fixed items (N-99, N-105,
    N-120 blocking, N-143) moved to D:
    - Gemini issue-triage workflow on `main` (N-158); CI release/summary gates and three-target clippy
      (N-159, N-160).
    - Memory: `prot` honoured by mmap/brk, PROT_NONE, W^X, `max_prot` for mprotect, bounded mmap length
      and cursor, no USER at kernel slots, physmap out of the user range, stack-growth limit, brk/map
      leaks (N-132 to N-137).
    - Process: fork credentials/cwd/handlers (N-93), exec capability filter (N-94), `kill` to real
      processes (N-92), `arch_prctl` canonical check (N-100), `sigaction` layout and `sigprocmask`
      order (N-95, N-97), musl `tkill`/`tgkill`/`waitid` intercepts (N-103), `prctl` fail-closed
      (N-151).
    - Files: syscall 73 heuristic (N-120), offset/size DoS (N-122), O_EXCL/O_TRUNC/O_NOFOLLOW/
      O_DIRECTORY (N-116 flags).
    - Package header overflow (N-147), PCI transmute (N-155), per-fault and per-open serial output
      (N-176, N-130).
    - Truthful claims: KNOWN-LIMITATIONS "hardening not active" (KPTI, KASLR, canaries, retpoline,
      Spectre, SMEP/SMAP), TLS, MAC, seccomp, Kani/TLA+, file mmap; README/book/overview corrections
      (N-145, N-146, N-150, N-152, N-163, N-165).
  - [ ] **DL, dynamic linking (ADR 0010; X3 loader pulled forward from v0.29, 2026-10-08; open
    items carried to v0.28).** KDE
    switches from static archives to shared libraries loaded by musl's dynamic linker:
    - [x] Program loader: every PT_LOAD validated before exec commits (malformed files fail, the
      kernel no longer panics on them), page plan with shared-page merging, PIE programs at
      `0x5555_5555_4000`, the interpreter in the mmap area; one path for exec and the initial
      loader. Full Linux auxv for every program (AT_RANDOM, AT_HWCAP/HWCAP2, AT_CLKTCK, AT_UID..
      AT_EGID, AT_SECURE, AT_EXECFN, AT_PLATFORM, AT_MINSIGSTKSZ). Runtime: `musl_auxv`,
      `musl_pie_loaded` (the musl suite runs as a static PIE).
    - [x] Shared file pages: a page cache per BlockFS file (`mm::page_cache`) for private file
      mappings and loaded program segments, read-only or copy-on-write; dropped when the file
      changes or its inode is freed. Runtime: `musl_page_cache`, `musl_program_pages_shared`.
    - [x] Toolchain: shared musl (`/lib/ld-musl-x86_64.so.1`), GCC with shared libstdc++ and
      libgcc_s, dynamic linking by default; verified on the build host through musl's loader,
      including `dlopen` of a C++ shared object. CI builds and caches it.
    - [x] Dynamic programs in VeridianOS: musl's loader and the shared C++ runtime in the root
      filesystem; `musl_dynamic_program` runs a dynamically linked C++ program (exceptions,
      threads, thread_local, `dlopen` of a C++ library).
    - [ ] KDE pipeline with shared libraries; static workarounds removed; rootfs with `/lib`.
    - [ ] Media backend for the KDE stack: Qt Multimedia is built for Prison, Qt Speech and QML
      but has no backend; add FFmpeg (shared) and an audio path (ALSA now, PipeWire later),
      then Qt Multimedia's FFmpeg plugin; cameras need kernel V4L2
      (`userland/integration/KNOWN-LIMITATIONS.md`).
    - Later (sprint G / v0.29): ASLR, vDSO, eviction of cached pages, writable MAP_SHARED of files.
  - [ ] **D, process model and scheduler (C5; ADR 0006 + ADR 0007):**
    - [x] Scheduler policy core: EEVDF fair, FIFO/RR with bandwidth limit, SCHED_DEADLINE (CBS),
      PELT-style load, SMP placement/balancing (`sched/policy/`, 36 host tests).
    - [x] x86 entry layer first: assembly stubs with symmetric swapgs + lfence, one TrapFrame for
      syscalls and traps, IST only for #DF/NMI/#MC, SYSRET eligibility with IRET fallback,
      exit-to-user hook, user-register sanitiser (N-166, N-169 to N-171, N-175; ADR 0008).
      Per-thread frames (N-167) and the no-launch-context fault path (N-168) finish with D2.
    - [x] D1: switch primitive (`switch_stacks`), boot task + idle, kernel threads on guarded
      stacks, wait queues (prepare-to-wait), reaping after `on_cpu` clears (`sched/dispatch.rs`,
      2 boot tests).
    - [x] D2 dispatcher: user tasks
      dispatched by the scheduler on their own kernel stacks; launch from boot/shell/KDE as
      spawn + wait; exit vs exit_group, process exit by the last thread, teardown after every
      thread is off-CPU, child wait queue (N-106 to N-111); fork/wait; XSAVE switching (N-41).
      Done 2026-10-07: every launcher (boot, shell, KDE session) uses `process::run_and_wait`;
      musl pthreads run concurrently. Open: N-111 (Zombie->Dead CAS) and N-112 with SMP (D5);
      waits poll through `dispatch::wait_in_syscall` until the blocking step.
    - [ ] Blocking (open item carried to v0.28 with F1): wait-queue primitive, sleep queue, no spinlock held across I/O, user copies
      or waits; file table lock dropped after fd lookup; sleeping address-space lock (N-118,
      N-119, N-138); pipes with reader/writer counts and PIPE_BUF atomicity; futex queues with
      timespec timeouts (N-104). Done: `wait_event` + timer sleepers, sleeping nanosleep/
      clock_nanosleep/sigsuspend/timerfd/futex/pipe/eventfd/poll/epoll, futex requeue and shared
      keys, N-104, file-table lock after lookup and stream position lock (N-118 part), sleeping
      address-space lock (N-138), whole blocking writes and PIPE_BUF atomicity, BlockFS reads
      with the cache unlocked (N-118 BlockFS part), pty readiness wakeups and blocking pty reads
      (Unix sockets done before), console reads that sleep on a shared input buffer (N-118
      console part). Open: INET socket readiness goes with the F1 socket layer.
    - [ ] D3 (N-115 and N-260 carried to v0.28): timer preemption of ring 3 (done: `dispatch::preempt_user` on trap and syscall
      exit; kill acted on in every dispatched wait); device IRQs on the kernel stack (done, ADR 0008);
      signal delivery with rt_sigframe/restorer, per-thread mask/pending, ignored dropped, Linux
      set layout (done: N-96, N-98, N-109 part, N-113; stop/continue done; siginfo, sigqueue and sigtimedwait
      done, N-259; no SA_RESTART restart of sigsuspend/sleeps/polls, N-260); signal delivery on
      return to user with restorer frames (N-113); per-thread signal state (N-109); ignored
      signals dropped at generation (N-98); sigset layout (N-96); wait status and errno (N-99 done; WNOWAIT and rusage done, N-213; N-127 done); exec permission and
      fatal-after-clear, exec from a multithreaded process ending the other threads first (done,
      N-101; the exec'ing thread keeps its tid); relative paths per thread (N-115); MAP_SHARED anon (done, N-140) and
      MAP_FIXED semantics (done, N-141); pty fixes (done, N-128); thread placement (N-114 partly done: no unused
      clone stacks; main tid == pid and keyed futex queues done, N-114 fixed).
    - [x] Threads: clone through the dispatcher, native libc clone stub, TCB join, TLS errno
      (N-102); N-46, N-50, N-54; `thread_join` sleeps on an exit queue instead of yielding
      (N-269). Done: N-102 (`clone_<arch>.S`, errno in the TCB, native pthreads tested by
      `audit_runtime_test`), N-269.
    - [x] D4: nested boot dispatch and `BOOT_RETURN_*` removed; old scheduler, `queue.rs`,
      `percpu_queue.rs`, `deadline.rs`, `process_compat`, old `context_switch` retired (N-84,
      N-85, N-156, N-181); the old `FUTEX_TABLE` path goes with it (N-279). Done: D4a (no
      old-scheduler tasks, failed-fork rollback N-46), D4b (IPC on dispatcher wait queues; N-47,
      N-81 part, N-84, N-85), D4c (nested boot model, `boot_futex_spin` and `FUTEX_TABLE` deleted;
      N-54, N-279), D4d (old scheduler modules, `Task`, `TaskContext`, `context_switch`,
      `process::sync`/`wait` deleted; N-50, N-156, N-181).
    - [ ] D5, SMP S2/S3 (leftovers N-52 frees, N-261, N-282 carried to v0.28): per-CPU run queues and current task, idle and balancing on every CPU,
      `cpu_init_local()` on APs (N-172), IPI fixes (N-173), interrupts enabled in syscalls with
      deferred frees and per-mm TLB masks (N-139, N-143 flushes), smp_boot stop + guarded AP
      stacks (N-180), timer advance by elapsed time (N-179), `smp` on by default; N-51, N-52;
      targeted TLB shootdown (per-address-space CPU mask, logical-to-APIC-ID map; N-262) and
      frees through the per-CPU cache after it (N-261); per-CPU syscall rate limiter
      (SYS-PERF-01). Done: D5a (AP setup N-172, IPIs N-173, bring-up N-180), D5b (timers
      N-179), D5c (every CPU dispatches its own run queue), D5d (interrupts on in syscalls,
      N-139 part; blocking Unix accept), D5e (placement, idle pull and periodic balancing,
      affinity migration, getcpu, per-task user GS base, interrupt-safe heap, N-139 fixed:
      interrupts on in page faults and the return to user mode, reaper thread; ADR 0012;
      `gate.sh --smp`), N-262 targeted shootdown, remote signal kick. Left: per-CPU frees after
      N-52 (with F5 interrupt-driven storage), N-282 (1-page mmap 5x slower with four
      processes; profile first), N-261 (per-CPU frees pay, but need checking: per-frame state
      the cache can test without the global lock). Done since: SYS-PERF-01 (per-CPU rate-limit buckets), `smp` on by default, N-51 (directory checks on the resolved node), audit cost and cap, BlockFS
      directory slot reuse, deadline tasks placed and pulled by deadline (`pull_dl`), new
      threads inherit the creator's CPU mask, deadline threads must allow every CPU, N-281
      (mmap cost was dev-build `memset` zeroing, not faults: `clear_page`/`copy_page`).
    - [x] Syscalls `sched_setattr/getattr`, `sched_setscheduler`, `nice`/`setpriority`, real
      `sched_setaffinity`/`getaffinity`, `sched_yield` on the new policy (SCHED-INC-02 resolved
      by the deadline class).
  - [ ] **E, user mode beyond x86_64** (open: RISC-V HR-02, HR-04 to HR-06, HR-11, carried to
    v0.28; also the boot-critical hardware items, since they coincide
    with turning the MMU on; `ref-docs/HARDWARE-AARCH64.md`, `ref-docs/HARDWARE-RISCV64.md`):
    - AArch64: done: DTB from x0, Linux-style EL2/SCTLR setup on boot and secondary paths (HA-01,
      HA-14, HA-15 part, N-177); arm64 Image header and flat binary, booted in the gate (HA-03);
      exception vectors installed in boot.S before any Rust code (bring-up step 1);
      position-independent early boot with the kernel at a high TTBR1 VA, MMU and caches on
      before Rust, TCR.IPS from PARange, 48-bit TTBR0, any 2 MiB-aligned load base (ADR 0015;
      HA-04, HA-16, N-28 part); Arm page-table format (`mm::pte::arm`); MMIO through the
      physical map, identity map confined to the switch; `bare_lock`, the bump
      allocator and the reduced secondary stage removed (N-28 fixed); the dispatcher on every
      CPU (switch primitive, `dispatch` cfg, SGI kicks, timer tick; E-A5/E-A5c); process
      address spaces hold only the user half (TTBR0); EL0 entry/exit, SVC system calls with arm64
      numbers, user page faults, fault-tolerant user copies, ELF programs and the embedded init
      (ADR 0016, E-A6); user FP/SIMD state switched with the task, signal handlers with Linux's
      rt_sigframe, fault signals to handlers, fault-tolerant user cmpxchg, static TLS from PT_TLS
      (all also RISC-V);
      an ASID per address space, released with `tlbi aside1is` when its tables are freed (HA-26);
      cache maintenance for code the kernel writes (HA-25 part: `dc cvau` without CTR_EL0.IDC,
      `ic ialluis` without DIC; loader, mmap and mprotect with PROT_EXEC);
      memory map from `/memory` less the reservations and the blob, heap sized from it (HA-17);
      musl, Linux 7.2 UAPI headers and a GCC 16 cross toolchain (`tools/cross/build-musl-arch.sh`,
      also RISC-V); fork/clone/exec, generic stat/epoll layouts, PL031/Goldfish RTC and a musl
      BusyBox root running the BusyBox self-test and both runtime suites
      (`scripts/build-musl-rootfs.sh`, `ARCH=... scripts/run-rootfs-tests.sh`; also RISC-V);
      those runs in the gate on one and four CPUs; PSTATE sanitised on every return to EL0 and
      by sigreturn, with a test that panics the kernel without it (N-170); done: DMA coherency
      (HA-25 rest; ADR 0019: `mm::dma`, coherency per device from the device tree, AArch64
      `dc cvac/civac`, RISC-V Zicbom, the virtio drivers and ramfb); done: AT_HWCAP2 (instructions needing no kernel support), arm64
      MINSIGSTKSZ 5120, BTI for user programs (PROT_BTI, GP pages, loader property notes,
      SCTLR_EL1.BT0, HWCAP2_BTI; boot-tested on `-cpu max` in the gate).
    - RISC-V: done: zero BSS, hart lottery, `sstatus.FS` Off in the kernel (HR-01, HR-19, HR-20
      part; the M-mode CSR reads go with FP switching). Relocatable or MMU-early boot (HR-02);
      done: the RISC-V Image header and flat Image, boot-tested (HR-03); done: `satp` mode probe (Sv48, then Sv39)
      and 3/4-level tables (HR-08, HR-09; ADR 0018 stage A, the kernel and user layout inside
      Sv39's halves, and stage B, the probe and the 3-level walk; gate runs `sv48=off`);
      RISC-V PTE encoder with A/D
      (HR-10, done: `mm::pte`), Svpbmt/XTheadMae (HR-11); done: paging on and the kernel in the
      upper half, Sv48 (E-RV3, ADR 0013); identity map only in the trampoline, MMIO through the
      physical map, user address spaces share the kernel half (E-RV4); guarded kernel stacks,
      switch primitive and dispatcher on the boot hart (E-RV5); U-mode entry and exit, ecall
      system calls by the generic table, fault-tolerant user copies (E-RV6, ADR 0014; N-14);
      ELF programs through the loader (E-RV6c); every hart dispatches (E-RV5c); the musl BusyBox
      root (done: musl and toolchain, TLS, user FP, signals, user cmpxchg, fork/exec); next: remove raw 0x1000_0000 stores and QEMU-only probes (HR-04
      to HR-06); done: memory map from the DT (HR-07); external interrupts (scause 9) handled, not fatal
      (HR-31); IPIs by hart id (HR-33); U-mode with `tp` via `sscratch` (N-14); done: `sstatus`
      never taken from user state, sigreturn refuses a kernel pc (N-170).
    - Both: kernel stack guard pages (N-26; done on both, which still need an overflow stack
      for the exception). Done: W^X for the kernel image, mapped in 4 KiB pages with its
      sections' permissions, and a non-executable physical map (`arch::*::kmap`, boot test
      `kernel_image_w_xor_x`, also guarding x86_64's bootloader mapping). Done: soft-float kernels (N-178,
      `aarch64-unknown-none-softfloat`, `riscv64imac-unknown-none-elf`); E0 boot tests report
      what cannot run as skipped.
  - [ ] **F, networking, storage and protocols (carried to v0.28):**
    - F1 socket layer: sockaddr decoding, UDP send/receive/demux/checksum, errno and readiness
      (N-64 to N-68, N-72).
    - F2 network thread with NIC interrupts and per-socket wait queues (N-69, N-75); generic
      device-interrupt vectors, x2APIC and MADT-driven I/O APIC (N-174).
    - F3 TCP: decide smoltcp (no_std, 0BSD) vs in-house, then NET-INC-01, N-08, N-20, N-70,
      N-71, N-76, DNS (N-77).
    - F4 virtqueues: device-sized rings, EVENT_IDX, batched kicks, DMA barriers, fewer copies
      (N-73, N-78, N-58).
    - F5 storage: interrupt-driven NVMe and virtio-blk, FLUSH from BlockFS sync, durable fsync
      (N-52, N-74, N-121 durability), then NVMe queues.
    - WireGuard (NET-INC-02) after F1/F2; priority inheritance (SCHED-INC-01, done in D: PI
      futexes through the policy boost); namespaces (VIRT-INC-01).
  - [ ] **G, hardening and correctness (carried to v0.28):** enable retpoline, stack protector, eIBRS/AutoIBRS, IBPB,
    UMIP, SMEP/SMAP (N-15) and decide KPTI (N-145, N-146); entropy sources and fast key erasure
    (N-149); crypto KATs and RustCrypto primitives (N-154); package verifier cleanup (N-148);
    passwords (N-153); POSIX shm access control (N-88); on-disk validation and fuzzing, `mkfs`
    (N-123); BlockFS open-unlinked inodes (N-117 interim); mount checks (N-129); frame allocator
    regions (N-142); `write_user` padding and IRQ-safe allocator locks (N-144); RTC fn pointer
    (N-157); CI flags, blocking coverage, real Kani harnesses, supply chain (N-161 to N-164);
    from the 2026-10 intake: dead `cache_topology` and KSM wired or removed and
    `PERFORMANCE-TUNING.md` corrected (N-264, N-274), one `SyscallError` (N-272), `alloc`
    unconditional with a CI build that proves it (N-275); `kernel/tests/` either ported to host
    or boot tests and deleted, or fixed and built in CI (N-280).
    - Kernel stack overflow reported on AArch64 and RISC-V (N-26 rest, moved from E on 2026-10-16):
      check at the EL1/S-mode exception entry (AArch64: `sp - frame` probed with `at s1e1w`, x0
      stashed in SP_EL0, which the return to EL0 reloads; RISC-V: `sscratch`), then a per-CPU
      overflow stack and a fatal report, as x86_64's IST does; tested by a feature-gated boot
      that overflows on purpose (none of the three architectures tests its report yet).
  - [ ] **H, real hardware (carried to v0.28)** (`ref-docs/HARDWARE-{X86_64,AARCH64,RISCV64}.md`: HX-01..34,
    HA-01..30, HR-01..40; items already in E are not repeated):
    - Shared infrastructure: one FDT parser (cells, ranges, `dma-ranges`, interrupt cells,
      `/chosen`, `reserved-memory`; fuzzed) and ACPI table parsers (MADT, FADT, MCFG, SPCR, DBG2,
      HPET, SRAT, DMAR, GTDT, PPTT, IORT); a console framework (SPCR/DBG2/`stdout-path`, PL011,
      ns16550/DW with `reg-shift`/`reg-io-width`, SBI DBCN, GOP fallback, probes that never read
      absent ports); a DMA API (coherent and non-coherent, Zicbom/T-Head/SiFive and Arm cache
      maintenance, bus-address translation); generic PCIe ECAM (MCFG / `pci-host-ecam-generic`)
      with correct BAR sizing and MSI/MSI-X; QEMU-only devices (virtio-mmio, fw_cfg/ramfb) probed
      only from firmware tables.
    - x86_64: PIT-less timer calibration (CPUID 15h/16h, MSR 0xCE, HPET, PM timer) and LAPIC
      against TSC (HX-01, HX-21); runtime-sized heap (HX-02); console selection (HX-03); ACPI
      before APIC, x2APIC, MADT-driven I/O APIC (HX-04, N-174); 32-bit APIC IDs and dynamic CPU
      count (HX-05); trampoline reservation and 4 KiB low mapping (HX-06); full memory map and
      SRAT nodes (HX-07); MAXPHYADDR, kernel-owned page tables with UC MMIO and one WC
      framebuffer, PAT sequence (HX-08 to HX-10); PCI segments and BARs (HX-11 to HX-13); FADT
      power/reset/century, AML interpreter choice (HX-14 to HX-16, HX-28); no legacy VGA
      (HX-17); xHCI and AHCI BIOS handoff, USB HID, NVMe shutdown (HX-18, HX-29, HX-30); IOMMU
      state and RMRRs (HX-19); NMI/#MC, microcode, mitigations, idle/P-states, thermal (HX-22 to
      HX-27); supported-hardware list and NIC drivers (HX-20); EFI system table, SMBIOS, crash
      capture (HX-31 to HX-34).
    - AArch64: earlycon and console drivers (HA-05, HA-06); GICv2 from the DT and GICv3/ITS
      (HA-07 to HA-11, HA-28); timer frequency and INTID from the DT (HA-12, HA-13, HA-27); full
      MPIDR ids, PSCI features/off/reset, spin-table (HA-21, HA-22); dma-ranges (HA-29);
      Spectre/SSBD (HA-30); errata and CPU-feature survey (PAN, LSE, PAuth, BTI); big.LITTLE
      capacity for scheduler placement; UEFI + ACPI entry for SystemReady servers (HA-24);
      PCIe and SMMUv3 (HA-20). First board: Raspberry Pi 4, then Pi 5, RK3588, a server.
    - RISC-V: PLIC from the DT with contexts from `interrupts-extended`, APLIC/IMSIC (HR-12 to
      HR-14); timebase and Sstc from the boot hart (HR-15, HR-16); SBI hygiene and HSM with
      physical addresses (HR-17, HR-18, HR-27); no M-mode CSRs (HR-21); entropy (HR-22); PCIe
      (HR-34); SiFive errata, ASIDs, vector state, SRST shutdown, idle suspend, IOMMU (HR-29 to
      HR-40); cache topology from the DT (HR-26). First board: VisionFive 2, then Unmatched, then a
      T-Head board.
    - CI: the QEMU validation matrices from the three documents (flat-Image loads at two
      addresses, `gic-version=3`, `virtualization=on`, `-M sifive_u`, `sv48=off`, `aia=`, no
      PIT/HPET, no COM1, `pxb-pcie`, x2APIC above 255 CPUs) as boot jobs.
  - [x] **X1, Linux system call numbers (pulled forward from v0.28; ADR 0009):** generated from
    `abi/syscalls.map`, VeridianOS calls from 1024 (fixes N-33), stock musl, every caller and
    the kernel's embedded code moved; rootfs suite and tri-arch boot pass. Handler review:
    N-182 to N-250 (`docs/audit/ABI-REVIEW-2026-10-07.md`), N-182 to N-189 fixed.
  - [x] **Credentials and paths (N-248 to N-250, pulled forward from v0.30 at the user's
    request):** setresuid/setresgid/getres*id, setreuid/setregid, setgroups/getgroups, permission
    checks on effective ids and groups; fchdir; chroot through the path resolver; utimensat.
    Native libc and musl use them.
  - [ ] **ABI review, security and D/F items (carried to v0.28):** N-190 to N-193 (permission checks) with D;
    N-207, N-209 to N-211, N-217, N-221, N-222, N-225 with D3/threads/scheduling syscalls;
    N-231, N-235 to N-238 with F1.
  - [ ] **Consolidation and the 2026-10 audit intake (open item N-69 carried to v0.28; 2026-10-14;
    `docs/audit/AUDIT-INTAKE-2026-10-10.md`).** Done: another agent's unmerged work audited,
    archived and reverted, its private files kept out of git; frame-allocator zones; BlockFS
    early data write-back and lock-free misses (N-263, N-118 BlockFS part; ADR 0011); IPC and
    capability authority (N-80, N-83, N-87, N-88, N-89 parts); VirtIO barriers (N-270, N-271);
    mm items (N-265, N-266, PB-MEM-05/07); shell `mv` (N-273); reviewer gate comment (N-276);
    `scripts/dev/` helpers; the x86_64 heap sized from the memory map, so small guests boot
    (N-278, HX-02). Open: `knetd` done properly (N-69, with F2); the rest is in the sprint lists
    above and v0.29.
  - [ ] **Every Linux system call (2026-10-16 directive; the rest carried to v0.28, decided
    2026-10-18).** Each call implemented with Linux semantics on all three architectures, numbers from
    Linux's tables (`abi/syscalls.map`, `tools/compat/gen_syscalls.py`; the AArch64/RISC-V
    decoders are generated from `abi/linux/syscall.tbl` and pinned by
    `generic_numbers_match_the_linux_table`). Kernel-modifying and raw-hardware calls
    (`init_module`/`finit_module`/`delete_module`, `kexec_*`, `iopl`/`ioperm`, `bpf` and
    similar) are refused by policy with EPERM after Linux's argument checks. Steps: done: per-arch
    coverage reports in `docs/compat/LINUX-SYSCALL-COVERAGE{,-AARCH64,-RISCV64}.md`, checked for
    staleness by the gate and CI; then group by group (process, signals, time, files, memory, ipc, events, network, sched,
    mount, system) with fail-first tests; LTP and musl libc-test in the rootfs suite; rewrite
    `docs/PORTING-GUIDE.md` (it still lists the pre-ADR-0009 native numbers). Done: W^X
    per-program opt-in (ELF note) and prctl PR_SET_MDWE/PR_GET_MDWE (ADR 0017); mknod and
    mknodat (FIFOs with Linux's open/read/write/poll rules, device nodes opening their device by
    number, socket files; special files on RamFS, tmpfs and BlockFS and in mkfs-blockfs images;
    st_rdev in stat; N-201); bind() of a Unix socket makes its socket file and connect() finds
    the socket through it; `tools/veridian-elfctl` adds or clears the W^X note in an existing
    binary. Process group: done: setfsuid and setfsgid (filesystem IDs used by every file
    permission check and for new files' owners; `/proc/<pid>/status` Uid/Gid/Groups lines);
    next: capget/capset, personality, kcmp, execveat, clone3, the pidfd calls,
    process_vm_readv/writev, rseq, unshare/setns.
  - [x] **Release:** tri-arch boot with `-smp 4`, runtime suite, CHANGELOG, tag (2026-10-18).
- [ ] **v0.28.0:** first the items carried over from v0.27.0, which was cut early (2026-10-18):
  - [ ] The rest of the Linux system calls, group by group, with LTP and musl libc-test in the
    rootfs suite and the `docs/PORTING-GUIDE.md` rewrite. Decisions of 2026-10-18: capget/capset
    on a full Linux capability model (permitted, effective, inheritable, bounding, ambient; file
    capabilities at exec; every privilege check by capability; N-131 pulled forward, ADR first);
    personality's READ_IMPLIES_EXEC honoured only for programs with the W^X opt-in (ADR 0017);
    unshare/setns with every Linux namespace type (PID, mount, UTS, IPC, network, user, cgroup,
    time; `/proc/<pid>/ns`).
  - [ ] Sprints F, G and H as listed under v0.27.0 above; INET socket readiness with F1;
    `knetd` (N-69) with F2.
  - [ ] DL: KDE with shared libraries and its media backend (FFmpeg, ALSA, then PipeWire).
  - [ ] D leftovers: per-thread relative paths (N-115), SA_RESTART restarts (N-260), per-CPU
    frees (N-52, N-261), the 1-page mmap cost with four processes (N-282).
  - [ ] RISC-V: relocatable or MMU-early boot (HR-02), no raw 0x1000_0000 stores or QEMU-only
    probes (HR-04 to HR-06), Svpbmt/XTheadMae (HR-11).
  - [ ] ABI-review items N-190 to N-193, N-207, N-209 to N-211, N-217, N-221, N-222, N-225,
    N-231, N-235 to N-238.

  Then the process, signal, time and file syscall tiers (X2 groups, with the open
  ABI-review findings N-194 to N-247 that belong to them).
  - [ ] **IPC redesign** (makes IPC reachable and correct): one endpoint object held by capabilities,
    typed rights, per-endpoint sender/receiver wait queues, one-shot reply capabilities and a direct
    switch with time-slice donation on call, notifications, accounting and rollback, the derivation
    tree as sole authority (N-79 to N-91, IPC-INC-01 remainder, N-47).
  - [ ] **Package signing keys (N-55 follow-up).** Today no post-quantum key is provisioned, so a
    policy requiring the PQ signature refuses every package.
    - Key hierarchy: an offline maintainer root key (ML-DSA-65 seed, plus Ed25519) that only
      certifies and revokes, and online signing keys (Ed25519 + ML-DSA-65 pairs) that sign
      packages, certified by the root and usable from CI.
    - `tools/pkg-keygen`: an offline tool on the `ml-dsa` crate to generate the root seed, and to
      generate and certify signing keys and revocations.
    - Kernel: embed the root public key; verify a signing-key certificate (and revocation list)
      against it instead of trusting one embedded key; replace `trusted_mldsa_public_key()`.
    - Users may add their own trusted keys for their own repositories (an extra keyring); the base
      system trusts the project root.
    - ADR for the key ceremony: offline generation, seed backup, rotation, revocation, and what
      happens if the root is lost or compromised.
- [ ] **v0.29.0 - v0.31.0:** the rest of the Linux/POSIX syscall surface, loader/vDSO/TTY/procfs,
  and LTP and open_posix in CI. See `docs/compat/COMPATIBILITY-PLAN.md`.
  - [ ] **v0.29 filesystem architecture:** node-based path API with lookup-or-create (N-116, N-51),
    inode cache with open counts and orphan list (N-117), crash consistency ADR (journal or shadow
    superblock; N-121), page cache and shared mmap (N-125), dentry cache and hashed directories
    (N-130), stable getdents cookies (N-124), atomic O_APPEND (N-126).
  - [ ] **v0.29 memory architecture** (also N-267 demand-paging mapping overlap, N-268 empty
    page tables kept): lazy anonymous memory, range-indexed mappings, per-frame
    refcount array, ranged per-mm TLB flushes, per-VMA locks (N-143).
  - [ ] **v0.30 credentials remainder:** filesystem ids, Linux capabilities, setuid exec
    (N-131; the id triples and groups move to v0.27 as N-248).
- [ ] **C6 (staged from v0.28):** move drivers, protocols and codecs out of the kernel.
- [ ] **C7:** serve a native user-space Wayland client; frame-backed `wl_shm` (DRV-PERF-01,
  DESK-ARCH-01).

### Build Status
- **x86_64**: 0 errors, 0 warnings, Stage 6 BOOTOK, 34/34 tests
- **AArch64**: 0 errors, 0 warnings, Stage 6 BOOTOK, 34/34 tests
- **RISC-V**: 0 errors, 0 warnings, Stage 6 BOOTOK, 34/34 tests
- **Host-target unit tests**: 4,475 passing (Codecov integrated)
- **In-guest runtime suite**: `audit_runtime_test` 27/27 and BusyBox 27/27 on the BlockFS root
- **CI pipeline**: 11/11 jobs passing

### Code Quality Metrics
- static mut: 7 justified instances (early boot, per-CPU, heap backing)
- Err("...") string literals: 0
- Result<T, &str>: 1 justified (parser return)
- #[allow(dead_code)]: ~125 instances across 52 files (mostly justified: hardware API completeness, Phase 6 stubs, arch-specific)
- SAFETY comment coverage: >100% (410/389 unsafe blocks)
- Soundness bugs: 0

### Self-Hosting Status
- **Tiers 0-5**: ALL COMPLETE (v0.4.9)
- **Tier 6 (T6-0 through T6-5)**: COMPLETE -- merged from test-codex, audited, tri-arch BOOTOK
- **Tier 7 (T7-1 through T7-5)**: COMPLETE (v0.5.0)
- See `docs/SELF-HOSTING-STATUS.md` for detailed status

## Detailed Feature Status

### Phase 0: Foundation (100% COMPLETE)
- [x] Rust nightly toolchain with cross-compilation
- [x] Cargo workspace with build scripts
- [x] Custom target specifications (x86_64, AArch64, RISC-V)
- [x] QEMU development environment for all architectures
- [x] GDB debugging infrastructure
- [x] CI/CD pipeline (GitHub Actions, 100% pass rate)
- [x] Documentation framework (mdBook, rustdoc, GitHub Pages)
- [x] Git hooks and PR templates

### Phase 1: Microkernel Core (100% COMPLETE)
- [x] Hybrid bitmap+buddy frame allocator with NUMA awareness
- [x] 4-level page tables (x86_64/AArch64), Sv48 (RISC-V)
- [x] Kernel heap with slab allocator
- [x] IPC: sync/async channels, zero-copy, fast path <1us
- [x] Process management: PCB/TCB, context switching all architectures
- [x] CFS scheduler with SMP support, load balancing, CPU hotplug
- [x] Capability system: 64-bit tokens, two-level O(1) lookup, revocation
- [x] System call interface (x86_64 SYSCALL/SYSRET)

### Phase 2: User Space Foundation (100% COMPLETE)
- [x] VFS: RamFS, DevFS, ProcFS, BlockFS with ext2-style directories
- [x] ELF loader with dynamic linking and relocations (the dynamic part could not run a program until v0.27, ADR 0010)
- [x] Driver framework: PCI/USB bus, network, storage, console, GPU
- [x] Init system with service management
- [x] Shell with 20+ built-in commands
- [x] Process server, driver framework service
- [x] Signal handling, PTY support
- [x] Userland bridge: Ring 3 entry, embedded init binary

### Phase 3: Security Hardening (100% COMPLETE)
- [x] Crypto: ChaCha20-Poly1305, Ed25519, X25519, SHA-256, CSPRNG
- [x] Post-quantum: ML-DSA (Dilithium), ML-KEM (Kyber)
- [x] MAC: policy parser, RBAC, MLS enforcement
- [x] Audit system with structured event logging
- [x] Memory protection: ASLR, DEP/NX, W^X, guard pages, Spectre barriers, KPTI
- [x] Auth: PBKDF2 password hashing
- [x] TPM 2.0 integration (command structures)
- [x] Secure boot verification framework
- [x] Syscall fuzzing infrastructure

### Phase 4: Package Ecosystem (100% COMPLETE)
- [x] DPLL SAT dependency resolver
- [x] Package manager: install, remove, upgrade, search with transactions
- [x] Repository: index generation, mirrors, HTTP client
- [x] Delta updates (binary diff/patch)
- [x] Configuration tracking, orphan detection
- [x] Ports system: TOML parser, build environment, port collection
- [x] SDK: toolchain registry, cross-compiler config, syscall API
- [x] Security scanning, license compliance, statistics
- [x] Ecosystem: core packages, essential apps, driver packages

### Phase 4.5: Interactive Shell (100% COMPLETE)
- [x] Wire shell to boot, ANSI escape parser, line editor
- [x] Pipe infrastructure, I/O redirection, new syscalls (dup, dup2, pipe, getcwd, chdir, ioctl, kill)
- [x] Variable expansion ($VAR, ${VAR:-default}, $?, $$, tilde), glob matching, tab completion
- [x] Job control (&, fg, bg, jobs), signal handling (SIGTSTP, SIGCONT, SIGPIPE)
- [x] Control flow (if/elif/else/fi, while, for, case), functions, aliases, advanced operators
- [x] 24+ builtins, console/PTY integration, boot tests (29/29)

### Self-Hosting Roadmap: Tiers 0-6 COMPLETE, Tier 7 in progress
- [x] Tier 0: Critical kernel bug fixes (page fault, fork, exec, timer, mmap)
- [x] Tier 1: 79+ syscalls for GCC toolchain
- [x] Tier 2: Complete libc (17 source files, 6,547 LOC, 25+ headers)
- [x] Tier 3: TAR rootfs loader, virtio-blk PCI, PATH resolution, /tmp
- [x] Tier 4: User-space shell, libm, wait queues, SIGCHLD
- [x] Tier 5: GCC cross-compiler (binutils 2.43 + GCC 14.2), sysroot, rootfs
- [x] **Tier 6: Platform completeness** (merged from test-codex, audited, tri-arch BOOTOK)
  - [x] T6-0: ELF multi-LOAD handling (multi-segment binaries like /bin/sh)
  - [x] T6-1: readlink() full VFS implementation (BlockFS + RamFS symlinks)
  - [x] T6-2: AArch64/RISC-V signal delivery (full signal frame save/restore)
  - [x] T6-3: Virtio-MMIO disk driver for AArch64/RISC-V
  - [x] T6-4: LLVM triple patch (veridian OS enum)
  - [x] T6-5: Thread support -- clone()/futex()/pthread (1,145 lines kernel + 556 lines libc)
- [x] **Tier 7: Full self-hosting loop** (COMPLETE v0.5.0)
  - [x] T7-1: Rust user-space target JSON (x86_64, aarch64, riscv64)
  - [x] T7-2: Rust std port (platform layer for VeridianOS syscalls)
  - [x] T7-3: Native GCC on VeridianOS (static cross-build, GCC 14.2 + binutils 2.43)
  - [x] T7-4: make/ninja cross-compiled (GNU Make 4.4.1 + Ninja 1.12.1)
  - [x] T7-5: vpkg user-space migration (176KB static ELF)

### Phase 5: Performance Optimization (~90% actual)
- [x] NUMA-aware scheduling data structures (sched/numa.rs)
- [x] Zero-copy networking framework (net/zero_copy.rs)
- [x] Performance counters (perf/mod.rs)
- [x] Scheduler context switch wiring (all 3 architectures)
- [x] IPC blocking/wake with fast path framework
- [x] TSS RSP0 management for per-task kernel stacks
- [x] All 56 TODO(phase5) markers resolved
- [x] User-space /sbin/init (PID 1 in Ring 3)
- [x] Native binary execution (NATIVE_ECHO_PASS)
- [x] Dead code audit (136 to <100 annotations)
- [x] Per-CPU page free lists (PerCpuPageCache, batch refill/drain, v0.5.7)
- [x] IPC fast path completion (per-task ipc_regs, direct register transfer, v0.5.7)
- [x] TLB optimization (TlbFlushBatch, lazy TLB, tlb_generation, v0.5.7)
- [x] Priority inheritance protocol (PiMutex, v0.5.7) -- never used; deleted with the old scheduler in v0.27, redo is SCHED-INC-01
- [x] Benchmarking suite (7 micro-benchmarks, perf shell builtin, v0.5.7)
- [x] Software tracepoints (10 event types, per-CPU ring buffers, trace shell builtin, v0.5.7)
- [x] TlbFlushBatch wired into unmap/map hot paths (v0.5.8)
- [x] per_cpu_alloc_frame wired into map_page() (v0.5.8)
- [x] CapabilityCache wired into IPC fast path (v0.5.8)
- [x] O(log n) PID-to-Task registry for IPC fast path (v0.5.8)
- [x] Trace events wired: IpcFastSend, IpcFastReceive, IpcSlowPath, FrameAlloc (v0.5.8)
- [ ] Lock-free algorithms (RCU, wait-free queues) -- deferred, requires SMP
- [ ] Power management -- deferred, requires ACPI parser
- [ ] Profile-guided optimization -- deferred, requires self-hosted Rust

### Phase 6: Advanced Features & GUI (~40% actual -- core graphical path)
- [x] Wayland compositor: wire protocol parser, SHM buffers, surface compositing, XDG shell (v0.6.1)
- [x] Desktop renderer: gradient background, demo windows, compositor render loop (v0.6.1)
- [x] PS/2 mouse driver, unified input events (EV_KEY/EV_REL), hardware cursor (v0.6.1)
- [x] TCP/IP network stack: VirtIO-Net TX/RX, Ethernet, ARP cache, TCP state machine, DHCP (v0.6.1)
- [x] Shell commands: ifconfig, dhcp, netstat, arp, startgui (v0.6.1)
- [x] 19 new syscalls (FbGetInfo..FbSwap, WlConnect..WlGetEvents, NetSendTo..NetGetSockOpt) (v0.6.1)
- [x] AF_INET socket creation wired to net::socket (v0.6.2)
- [x] VirtIO-Net/E1000 device registry integration (v0.6.2)
- [x] UDP recv_from wired to socket buffer layer (v0.6.2)
- [x] All TODO(phase6) markers resolved or reclassified to Phase 7 (v0.6.2)
- [ ] GPU drivers, advanced Wayland, multimedia, virtualization -- see Phase 7

## Progress Tracking

| Component | Planning | Development | Testing | Complete |
|-----------|----------|-------------|---------|----------|
| Build System | Done | Done | Done | Done |
| CI/CD Pipeline | Done | Done | Done | Done |
| Boot (all archs) | Done | Done | Done | Done |
| Memory Manager | Done | Done | Done | Done |
| Process Manager | Done | Done | Done | Done |
| IPC System | Done | Done | Done | Done |
| Scheduler | Done | Done | Done | Done |
| Capability System | Done | Done | Done | Done |
| VFS / Filesystem | Done | Done | Done | Done |
| Driver Framework | Done | Done | Partial | Partial |
| Network Stack | Done | Done | Done | Done |
| Package Manager | Done | Done | Partial | Done |
| Crypto / Security | Done | Done | Partial | Done |
| NUMA Scheduling | Done | Done | Partial | Partial |
| Wayland/Compositor | Done | Done | Partial | Partial |
| Desktop/Input | Done | Done | Partial | Partial |
| TCP/IP Stack | Done | Done | Partial | Partial |
| GPU Acceleration | Done | Done | Partial | Partial |

## Known Issues

Currently tracking **0 critical issues**. All architectures boot cleanly with zero warnings.

Tier 6 merge (T6-0) resolved the `/bin/sh` multi-LOAD-segment GP fault. The `test-codex` branch has been merged, audited (8 critical bugs fixed), and deleted.

See [ISSUES_TODO.md](ISSUES_TODO.md) for full issue history (18 resolved, 0 open).

## Remediation

See [REMEDIATION_TODO.md](REMEDIATION_TODO.md) for 37 identified gaps from Phases 0-4 audit:
- 4 Critical (interrupt controllers, UEFI boot)
- 11 High (pointer validation, timers, driver SDK)
- 14 Medium (sandboxing, file integrity, async I/O)
- 8 Low (documentation, stale TODO files)

## Quick Links

- [Phase 0 TODO](PHASE0_TODO.md) - COMPLETE
- [Phase 1 TODO](PHASE1_TODO.md) - COMPLETE
- [Phase 2 TODO](PHASE2_TODO.md) - COMPLETE
- [Phase 3 TODO](PHASE3_TODO.md) - COMPLETE
- [Phase 4 TODO](PHASE4_TODO.md) - COMPLETE
- [Phase 5 TODO](PHASE5_TODO.md) - ~90%
- [Phase 5.5 TODO](PHASE5.5_TODO.md) - 100% COMPLETE (all 12 sprints, v0.5.13)
- [Phase 6 TODO](PHASE6_TODO.md) - ~40% (core graphical path complete)
- [Phase 7 TODO](PHASE7_TODO.md) - ~100% (All 6 Waves complete)
- [Phase 7.5 TODO](PHASE7.5_TODO.md) - **COMPLETE** (all 8 waves, 80/80, v0.16.0)
- [Phase 8 TODO](PHASE8_TODO.md) - COMPLETE (next-generation features)
- [Phase 9 TODO](PHASE9_TODO.md) - COMPLETE (KDE Plasma 6 porting)
- [Phase 10 TODO](PHASE10_TODO.md) - **COMPLETE** (KDE Known Limitations Remediation, v0.23.0)
- [Remediation TODO](REMEDIATION_TODO.md) - Gaps from Phases 0-4
- [Issues TODO](ISSUES_TODO.md) - Issue history
- [Testing TODO](TESTING_TODO.md) - Testing status
- [Release TODO](RELEASE_TODO.md) - Release history

## Release History

| Version | Date | Summary |
|---------|------|---------|
| v0.26.0 | Oct 6, 2026 | Audit remediation P0+P1: security fixes, real clocks/timers/interrupts on all arches, per-CPU allocator, lazy BlockFS cache, capability hash table, routing, IPC data-path fixes, documentation corrected against the code |
| v0.10.0 | Feb 28, 2026 | Phase 7 Wave 6: Virtualization (VMX/VMCS, EPT, containers), security (KPTI, demand paging, COW fork, TPM, Dilithium), performance (NUMA SRAT/SLIT, per-CPU queues, IPC batching, IOMMU) |
| v0.9.0 | Feb 28, 2026 | Phase 7 Wave 5: Audio subsystem (mixer, VirtIO-Sound, WAV playback), video framework (TGA/QOI, scaling, media player) |
| v0.8.0 | Feb 28, 2026 | Phase 7 Wave 4: zero-copy DMA networking, hardware NIC TX/RX rings, IPv6 dual-stack, command substitution, NVMe admin queue |
| v0.7.1 | Feb 28, 2026 | Phase 7 Waves 1-3: GPU drivers (virtio-gpu, i915/amdgpu/nouveau), advanced Wayland, desktop completion |
| v0.7.0 | Feb 27, 2026 | Phase 6.5: Rust compiler port (std::sys::veridian, LLVM 19) + vsh Bash-in-Rust shell (49 builtins) |
| v0.6.4 | Feb 27, 2026 | Desktop interaction: PS/2 input pipeline, compositor zero-copy (~30fps), click-to-focus, window dragging |
| v0.6.3 | Feb 27, 2026 | Desktop completion: functional GUI apps, app-to-surface bridge, font8x16, window decorations |
| v0.6.2 | Feb 27, 2026 | Phase 6 completion: documentation sync, integration wiring, TODO(phase6) resolved, Phase 7 TODO |
| v0.6.1 | Feb 27, 2026 | Phase 6 graphical desktop: Wayland compositor, PS/2 mouse, TCP/IP (VirtIO-Net/Ethernet/ARP/TCP/DHCP), startgui |
| v0.6.0 | Feb 27, 2026 | Pre-Phase 6 tech debt: 12 new syscalls (POSIX shm + Unix sockets), PMU, RCU, NVMe PCI, IOMMU DMAR |
| v0.5.13 | Feb 27, 2026 | Phase 5.5 COMPLETE: 2MB huge pages, dynamic linker (ld-veridian), all 12 sprints done |
| v0.5.12 | Feb 27, 2026 | Phase 5.5 Wave 4: NVMe driver, VirtIO-Net TX/RX, hardware PMU |
| v0.5.11 | Feb 27, 2026 | Phase 5.5 Wave 3: DMA/IOMMU, shared memory, Unix sockets, lock-free paths |
| v0.5.10 | Feb 27, 2026 | Phase 5.5 Wave 2: IPI/SMP foundation, PCI/PCIe completion |
| v0.5.9 | Feb 27, 2026 | Phase 5.5 Wave 1: ACPI table parser, APIC timer 1000Hz preemptive scheduling |
| v0.5.8 | Feb 27, 2026 | Phase 5 completion: hot path wiring, CapabilityCache, O(log n) IPC PID lookup, trace instrumentation |
| v0.5.7 | Feb 26, 2026 | Phase 5 sprint 2: per-CPU page caching, TLB batching, IPC fast path, priority inheritance, benchmarks, tracepoints |
| v0.5.6 | Feb 25, 2026 | Phase 5 sprint 1: scheduler context switch, IPC blocking/wake, /sbin/init, dead_code audit, native execution |
| v0.5.5 | Feb 25, 2026 | POSIX partial munmap, consolidated brk(), native BusyBox 208/208 PASS |
| v0.5.4 | Feb 25, 2026 | Critical memory leak fixes: GP fault wrmsr, page table subtree leak, thread stack lifecycle |
| v0.5.3 | Feb 24, 2026 | BusyBox ash compat, process lifecycle hardening, ARG_MAX, strftime/popen |
| v0.5.2 | Feb 24, 2026 | BusyBox B-5 through B-17: EPIPE, float printf, sbrk hardening, POSIX regex, CI fix |
| v0.5.1 | Feb 23, 2026 | 6 coreutils, pipe fd fix, tri-arch clippy clean |
| v0.5.0 | Feb 21, 2026 | Self-hosting T7 complete, user-space foundation (exec/fork/fd/shell), dead_code audit |
| v0.4.9 | Feb 18, 2026 | Self-hosting Tiers 0-5, complete libc, virtio-blk, 30+ syscalls, user-space exec |
| v0.4.8 | Feb 16, 2026 | Fbcon scroll fix, KVM acceleration, version sync |
| v0.4.7 | Feb 16, 2026 | Fbcon glyph cache, pixel ring buffer, write-combining (PAT) |
| v0.4.6 | Feb 16, 2026 | Fbcon performance: back-buffer, text cell ring, dirty row tracking |
| v0.4.5 | Feb 16, 2026 | Framebuffer display, PS/2 keyboard, input multiplexer, ramfb, 29/29 tests |
| v0.4.4 | Feb 16, 2026 | Shell usability: CWD prompt, VFS population, RISC-V ELF fix |
| v0.4.3 | Feb 15, 2026 | Phase 4.5: Interactive Shell (vsh) -- 18 sprints, tri-arch shell prompt |
| v0.4.2 | Feb 15, 2026 | Hardware abstraction: interrupt controllers, IRQ framework, timer management |
| v0.4.1 | Feb 15, 2026 | Tech debt: bootstrap refactor, dead_code consolidation |
| v0.4.0 | Feb 15, 2026 | Phase 4 complete: toolchain, testing, compliance, ecosystem |
| v0.3.9 | Feb 15, 2026 | Phase 4 100% + Userland Bridge: Ring 3 entry, SYSCALL/SYSRET |
| v0.3.8 | Feb 15, 2026 | Phase 4 Groups 3+4 |
| v0.3.7 | Feb 15, 2026 | Phase 4 Group 2 |
| v0.3.6 | Feb 15, 2026 | Phase 4 Group 1 |
| v0.3.5 | Feb 15, 2026 | Critical boot fixes (CSPRNG, RISC-V memory, stack) |
| v0.3.4 | Feb 15, 2026 | Phase 1-3 integration + Phase 4 ~75% |
| v0.3.3 | Feb 14, 2026 | Technical debt: 0 Err("..."), soundness fixes |
| v0.3.2 | Feb 14, 2026 | Phase 2+3 completion (full crypto suite) |
| v0.3.1 | Feb 14, 2026 | Tech debt: OnceLock fix, 48 static mut eliminated |
| v0.3.0 | 2025 | Architecture cleanup and security hardening |
| v0.2.5 | 2025 | RISC-V crash fix |
| v0.2.1 | Jun 17, 2025 | Boot fixes, AArch64 workaround |
| v0.2.0 | Jun 12, 2025 | Phase 1 complete |
| v0.1.0 | Jun 7, 2025 | Foundation and tooling |

---

**Note**: This document is the source of truth for project status. Update after each release.

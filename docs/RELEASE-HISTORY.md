# VeridianOS Release History

Detailed release notes for all VeridianOS versions, in reverse-chronological order.

For current project status, see the [README](../README.md). For task tracking, see the [Master TODO](../to-dos/MASTER_TODO.md).

---

## Table of Contents

- [v0.6.2 -- Phase 6 Completion + Phase 7 TODO](#v062----phase-6-completion--phase-7-todo)
- [v0.6.1 -- Phase 6 Graphical Desktop, Wayland Compositor, Network Stack](#v061----phase-6-graphical-desktop-wayland-compositor-network-stack)
- [v0.6.0 -- Pre-Phase 6 Tech Debt Remediation](#v060----pre-phase-6-tech-debt-remediation)
- [v0.5.13 -- Phase 5.5 Wave 5: Huge Pages + Dynamic Linker (COMPLETE)](#v0513----phase-55-wave-5-huge-pages--dynamic-linker-complete)
- [v0.5.12 -- Phase 5.5 Wave 4: NVMe + Networking + PMU](#v0512----phase-55-wave-4-nvme--networking--pmu)
- [v0.5.11 -- Phase 5.5 Wave 3: DMA/IOMMU + Shared Mem + Lock-Free](#v0511----phase-55-wave-3-dmaiommu--shared-mem--lock-free)
- [v0.5.10 -- Phase 5.5 Wave 2: IPI/SMP + PCI/PCIe](#v0510----phase-55-wave-2-ipismp--pcipcie)
- [v0.5.9 -- Phase 5.5 Wave 1: ACPI + APIC Timer](#v059----phase-55-wave-1-acpi--apic-timer)
- [v0.5.8 -- Phase 5 Completion: Hot Path Wiring](#v058----phase-5-completion-hot-path-wiring)
- [v0.5.7 -- Phase 5 Performance Optimization](#v057----phase-5-performance-optimization)
- [v0.5.6 -- Phase 5 Scheduler, IPC, Init](#v056----phase-5-scheduler-ipc-init)
- [v0.5.5 -- POSIX Partial Munmap + Native BusyBox 208/208](#v055----posix-partial-munmap--native-busybox-208208)
- [v0.5.4 -- Critical Memory Leak Fixes](#v054----critical-memory-leak-fixes)
- [v0.5.3 -- BusyBox Ash Compatibility + Process Hardening](#v053----busybox-ash-compatibility--process-hardening)
- [v0.5.2 -- BusyBox EPIPE, Float Printf, POSIX Regex](#v052----busybox-epipe-float-printf-posix-regex)
- [v0.5.1 -- Coreutils + Pipe Fix](#v051----coreutils--pipe-fix)
- [v0.5.0 -- Self-Hosting Tier 7 Complete](#v050----self-hosting-tier-7-complete)
- [v0.4.9 -- Self-Hosting Foundation](#v049----self-hosting-foundation)
- [v0.4.8 -- Fbcon Scroll Fix + KVM](#v048----fbcon-scroll-fix--kvm)
- [v0.4.7 -- Fbcon Glyph Cache + Pixel Ring Buffer](#v047----fbcon-glyph-cache--pixel-ring-buffer)
- [v0.4.6 -- Fbcon Back-Buffer + Text Cell Ring](#v046----fbcon-back-buffer--text-cell-ring)
- [v0.4.5 -- Framebuffer Display + PS/2 Keyboard](#v045----framebuffer-display--ps2-keyboard)
- [v0.4.4 -- CWD Prompt + VFS Population](#v044----cwd-prompt--vfs-population)
- [v0.4.3 -- Interactive Shell (vsh)](#v043----interactive-shell-vsh)
- [v0.4.2 -- IRQ Framework + Timer Management](#v042----irq-framework--timer-management)
- [v0.4.1 -- Technical Debt Remediation](#v041----technical-debt-remediation)
- [v0.4.0 -- Phase 4 Milestone](#v040----phase-4-milestone)
- [v0.3.9 -- Phase 4 Completion + Userland Bridge](#v039----phase-4-completion--userland-bridge)
- [v0.3.8 -- Phase 4 Groups 3+4: Toolchain, Testing, Compliance, Ecosystem](#v038----phase-4-groups-34-toolchain-testing-compliance-ecosystem)
- [v0.3.7 -- Phase 4 Group 2: Ports Build, Reproducible Builds, Security](#v037----phase-4-group-2-ports-build-reproducible-builds-security)
- [v0.3.6 -- Phase 4 Group 1 + Build Fixes](#v036----phase-4-group-1--build-fixes)
- [v0.3.5 -- Critical Architecture Boot Fixes](#v035----critical-architecture-boot-fixes)
- [v0.3.4 -- Phase 1-3 Integration + Phase 4 Package Ecosystem](#v034----phase-1-3-integration--phase-4-package-ecosystem)
- [v0.3.3 -- Technical Debt Remediation](#v033----technical-debt-remediation)
- [v0.3.2 -- Phase 2 and Phase 3 Completion](#v032----phase-2-and-phase-3-completion)
- [v0.3.1 -- Technical Debt Remediation](#v031----technical-debt-remediation)
- [v0.3.0 -- Phase 3 Security Hardening](#v030----phase-3-security-hardening)
- [v0.2.5 -- RISC-V Crash Fix and Architecture Parity](#v025----risc-v-crash-fix-and-architecture-parity)
- [v0.2.4 -- Technical Debt Remediation](#v024----technical-debt-remediation)
- [v0.2.3 -- Phase 2 User Space Foundation](#v023----phase-2-user-space-foundation)
- [v0.2.1 -- Phase 1 Maintenance Release](#v021----phase-1-maintenance-release)
- [v0.2.0 -- Phase 1 Microkernel Core](#v020----phase-1-microkernel-core)
- [v0.1.0 -- Phase 0 Foundation and Tooling](#v010----phase-0-foundation-and-tooling)
- [DEEP-RECOMMENDATIONS](#deep-recommendations)

---

## v0.6.2 -- Phase 6 Completion + Phase 7 TODO

**Date**: February 27, 2026

Phase 6 documentation sync, integration wiring, and TODO marker resolution:

- **Documentation Sync** -- All Phase 6 references updated from "~5%" to "~40%"; MASTER_TODO, PHASE6_TODO, design doc, README, RELEASE_TODO, DEFERRED-IMPLEMENTATION-ITEMS all updated
- **AF_INET Socket Creation** -- `sys_socket_create()` now wires AF_INET to `net::socket::create_socket()` for IPv4 TCP/UDP sockets
- **Device Registry Integration** -- E1000 and VirtIO-Net drivers registered with `net::device::register_device()` on init
- **UDP recv_from Wiring** -- `UdpSocket` now tracks `socket_id`, registers with UDP socket buffer on `bind()`, and delegates `recv_from()` to the socket buffer layer
- **TODO(phase6) Resolution** -- All 43 markers resolved: 4 wired to implementations, 39 reclassified to `TODO(phase7)` with justification
- **Phase 7 TODO** -- Comprehensive roadmap generated (`to-dos/PHASE7_TODO.md`) with 15 categories and ~85 items

---

## v0.6.1 -- Phase 6 Graphical Desktop, Wayland Compositor, Network Stack

**Date**: February 27, 2026

First graphical desktop release. Transforms VeridianOS from a text-mode microkernel into a graphical desktop OS with networking.

### Wave 1: Display and Input Foundation (Sprints 1-4, syscalls 230-234)
- `FbInfo` struct + physical address tracking in `graphics/framebuffer.rs`
- `map_physical_region()` / `map_physical_region_user()` in `mm/vas.rs`
- PS/2 mouse driver (`drivers/mouse.rs`): 3-byte packets, absolute cursor, ring buffer
- Unified input event system (`drivers/input_event.rs`): EV_KEY/EV_REL, 256-entry queue
- Hardware cursor sprite (`graphics/cursor.rs`): 16x16 arrow with outline

### Wave 2: Wayland Compositor Core (Sprints 5-8, syscalls 240-247)
- Wire protocol parser (`desktop/wayland/protocol.rs`): 8 argument types, message framing
- SHM buffer management (`desktop/wayland/buffer.rs`): WlShmPool, sub-allocation, write_data()
- Software compositor (`desktop/wayland/compositor.rs`): back-buffer, Z-order, alpha blend, atomic dimensions
- Double-buffered surfaces (`desktop/wayland/surface.rs`): pending/committed state swap
- XDG shell (`desktop/wayland/shell.rs`): ping/pong, configure, toplevel lifecycle
- Display server (`desktop/wayland/mod.rs`): 9 interface handlers, per-client objects

### Wave 3: Desktop Environment (Sprints 9-12)
- Terminal ANSI (`desktop/terminal.rs`): CSI parser, SGR colors, cursor movement
- Desktop panel (`desktop/panel.rs`): window list, clock, click-to-focus
- Desktop renderer (`desktop/renderer.rs`): start_desktop(), render loop, framebuffer blit

### Wave 4: Network Stack (Sprints 13-16, syscalls 250-255)
- VirtIO-Net (`drivers/virtio_net.rs`): full VIRTIO negotiation, virtqueue TX/RX
- Ethernet (`net/ethernet.rs`): IEEE 802.3 parse/construct, EtherType dispatch
- ARP (`net/arp.rs`): cache with timeout, request/reply, broadcast
- TCP (`net/tcp.rs`): 3-way handshake, data transfer MSS=1460, FIN/ACK close
- DHCP (`net/dhcp.rs`): discover/offer/request/ack, option parsing, IP config
- IP layer (`net/ip.rs`): InterfaceConfig, IPv4 headers, ARP resolve
- Socket extensions (`net/socket.rs`): sendto/recvfrom/getsockname/getpeername/setsockopt/getsockopt
- Shell commands: ifconfig, dhcp, netstat, arp

### Wave 5: Integration (Sprints 17-20)
- Desktop init wires `wayland::init()` into subsystem init
- `startgui` shell command launches graphical desktop
- Compositor render loop: composite -> blit -> cursor -> input poll -> repeat

**Stats**: 37 files changed (10 new), +6,862/-267 lines, 19 new syscalls.

---

## v0.6.0 -- Pre-Phase 6 Tech Debt Remediation

**Date**: February 27, 2026

Comprehensive tech debt audit bridging Phase 5.5 to Phase 6 readiness:

- **12 New Syscalls** -- POSIX shared memory (ShmOpen=210, ShmUnlink=211, ShmTruncate=212); Unix domain sockets (SocketCreate=220 through SocketPair=228)
- **PMU Bootstrap** -- `perf::pmu::init()` called during Stage 3 after ACPI/APIC
- **RCU Integration** -- `rcu_quiescent()` called after every scheduler context switch
- **NVMe PCI Scan** -- init() scans PCI bus for class 0x01/subclass 0x08
- **IOMMU DMAR** -- AcpiInfo extended with `has_dmar`, `dmar_address`, `dmar_length`
- **Dynamic Linker Fix** -- `load_interpreter()` copies real ELF segment data

**Stats**: 18 files changed, +542/-53 lines.

---

## v0.5.13 -- Phase 5.5 Wave 5: Huge Pages + Dynamic Linker (COMPLETE)

**Date**: February 27, 2026

Final Phase 5.5 release. All 12 sprints complete.

### Sprint B-11: 2MB Huge Pages
- VAS::map_huge_page() with L2 HUGE flag, 512-frame contiguous allocation
- 2MB alignment validation

### Sprint B-12: Dynamic Linker
- ld-veridian.c: PT_INTERP dynamic linker (~330 lines)
- RELA relocation processing (RELATIVE, GLOB_DAT, JUMP_SLOT, 64)
- PT_DYNAMIC parser, library search paths, dlopen/dlsym/dlclose stubs

**Phase 5.5 Status**: 12/12 sprints COMPLETE

---

## v0.5.12 -- Phase 5.5 Wave 4: NVMe + Networking + PMU

**Date**: February 27, 2026

Phase 5.5 Wave 4 completing driver I/O paths and hardware profiling.

### Sprint B-8: NVMe Driver Completion
- Admin command submission with doorbell ringing and polling completion
- I/O queue pair creation via Create I/O CQ/SQ admin commands
- Block read/write via I/O queue with NVMe doorbell protocol

### Sprint B-9: Network Driver Completion
- VirtIO-Net: TX via virtqueue descriptor allocation + available ring + MMIO kick
- VirtIO-Net: RX via used buffer retrieval + descriptor recycling
- E1000: Already complete (transmit_raw/receive_raw functional since prior release)

### Sprint B-10: Hardware PMU Driver
- x86_64: CPUID 0x0A detection, IA32_PERFEVTSELx/PMCx MSR programming
- AArch64: PMCR_EL0 counter detection, PMCCNTR_EL0 cycle reads
- RISC-V: mcycle/minstret CSR access
- 8 hardware events: cycles, instructions, cache misses, branches, TLB
- SampleBuffer for per-CPU IP sampling (4096 samples)

**Stats**: 5 files changed, ~500 insertions

---

## v0.5.11 -- Phase 5.5 Wave 3: DMA/IOMMU + Shared Mem + Lock-Free

**Date**: February 27, 2026

Phase 5.5 Wave 3 release implementing I/O infrastructure, POSIX IPC, and lock-free synchronization.

### Sprint B-5: DMA + IOMMU Foundation
- DMAR table structures for Intel VT-d detection (DrhdUnit, RmrrRegion, DeviceScope)
- ScatterGatherList for multi-buffer DMA transfers
- DmaCoherency/DmaDirection enums for buffer allocation policy
- DmaMappedBuffer with alloc_dma_buffer()/free_dma_buffer() using identity mapping
- IOMMU init with non-fatal DMAR absence detection

### Sprint B-6: Shared Memory + Unix Domain Sockets
- POSIX shm: shm_open/shm_unlink/shm_truncate/shm_close with reference counting
- Unix sockets: AF_UNIX stream/datagram, bind/listen/connect/accept/send/recv
- socketpair() for anonymous connected pairs
- SCM_RIGHTS ancillary data for Wayland fd passing

### Sprint B-7: Lock-Free Kernel Paths
- RCU: epoch-based read-copy-update (rcu_read_lock, synchronize_rcu, call_rcu)
- Hazard pointers: per-CPU slots for safe lock-free memory reclamation
- Lock-free MPSC queue: Michael-Scott CAS-based queue for scheduler ready queues

**Stats**: 11 files changed, ~1600 insertions

---

## v0.5.10 -- Phase 5.5 Wave 2: IPI/SMP + PCI/PCIe

**Date**: February 27, 2026

Phase 5.5 Wave 2 release implementing multi-core coordination and modern PCI device infrastructure.

### Sprint B-3: IPI + SMP Foundation
- IPI vector constants: TLB_SHOOTDOWN_VECTOR=49, SCHED_WAKE_VECTOR=50
- INIT-SIPI-SIPI AP startup sequence via APIC ICR (per Intel SDM)
- TLB shootdown handler at IDT[49]: flushes local TLB on remote page table modifications
- Scheduler wake handler at IDT[50]: breaks HLT on idle CPUs for new tasks
- `TlbFlushBatch::flush_with_shootdown()`: local flush + broadcast IPI to all other CPUs
- `smp::send_ipi()` x86_64 path wired to actual APIC (replaced println stub)
- `smp::cpu_up()` uses proper INIT -> 10ms -> SIPI -> 200us -> SIPI retry

### Sprint B-4: PCI/PCIe Completion
- `MsiCapability`/`MsixCapability` structs for parsed PCI capability chain data
- `parse_capabilities()`: walks PCI capability linked list (cap IDs 0x05=MSI, 0x11=MSI-X)
- `scan_bridge()`: recursive PCI-to-PCI bridge enumeration via secondary bus numbers
- `configure_msi()`: MSI message address/data programming for interrupt vector delivery
- `ecam_read_config()`/`ecam_write_config()`: PCIe ECAM memory-mapped config access (x86_64)
- Bridge secondary bus tracking in PciDevice for recursive scanning

**Stats**: 8 files changed, ~450 insertions

---

## v0.5.9 -- Phase 5.5 Wave 1: ACPI + APIC Timer

**Date**: February 27, 2026

First Phase 5.5 Infrastructure Bridge release. Implements the hardware foundation (Wave 1) required for future multi-core, PCI, and driver support.

### Sprint B-1: ACPI Table Parser

Created `kernel/src/arch/x86_64/acpi.rs` (~570 lines):
- RSDP discovery from bootloader_api `BootInfo.rsdp_addr` (UEFI system table)
- RSDT/XSDT parsing with both ACPI 1.0 (32-bit pointers) and ACPI 2.0+ (64-bit pointers)
- MADT parsing: Local APIC entries (CPU enumeration), I/O APIC entries (interrupt routing), Interrupt Source Overrides, LAPIC NMI entries
- MCFG parsing: PCIe Enhanced Configuration Access Mechanism (ECAM) entries
- Static storage via `ACPI_INFO: Mutex<Option<AcpiInfo>>` with `with_acpi_info()` accessor
- `irq_to_gsi()` for ISA IRQ to Global System Interrupt remapping
- `acpi` shell command for runtime table inspection

### Sprint B-2: APIC Timer + Interrupt Wiring

- APIC timer calibration using PIT channel 2 as 10ms reference (~62731 ticks/ms on QEMU, ~1003MHz bus with divide-by-16)
- Periodic timer at 1000Hz (1ms tick) for preemptive scheduling via dedicated IDT vector 48
- `apic_timer_interrupt_handler`: increments tick counter, calls scheduler `tick()` with `try_lock()` deadlock prevention, sends APIC EOI
- Interrupts enabled at end of arch init after timer configuration
- Timer tick() in timer.rs wired to scheduler (removed dead_code annotation)

### Boot test (QEMU)

x86_64: 29/29 tests, 2x BOOTOK, APIC timer 1000Hz, ACPI parsed (1 CPU, 1 I/O APIC, 5 ISOs)
AArch64: 29/29 tests, 2x BOOTOK
RISC-V: 29/29 tests, BOOTOK

---

## v0.5.8 -- Phase 5 Completion: Hot Path Wiring

**Released**: February 27, 2026

Wires Phase 5 performance data structures into production hot paths, bringing Phase 5 from ~75% to ~90%:

- **TlbFlushBatch Integration** -- Replaces individual `tlb_flush_address()` calls in `map_region()`, `unmap_region()`, and `unmap()` (partial munmap) with batched TLB invalidation (up to 16 addresses, then full flush)
- **Per-CPU Frame Allocation** -- `map_page()` now uses `per_cpu_alloc_frame()` instead of global `FRAME_ALLOCATOR.lock()`, eliminating lock contention for single-frame allocations
- **CapabilityCache** -- 16-entry direct-mapped cache integrated into `validate_capability_fast()` for O(1) fast-accept path on IPC capability validation
- **Global PID-to-Task Registry** -- `BTreeMap<u64, SendTaskPtr>` provides O(log n) IPC fast path lookup, replacing broken `find_task_by_pid()` linear scan
- **Trace Instrumentation** -- IpcFastSend, IpcFastReceive, IpcSlowPath, and FrameAlloc events wired (8/10 event types now active)
- **Documentation** -- Created DEFERRED-IMPLEMENTATION-ITEMS.md, PERFORMANCE-BENCHMARKS.md, PERFORMANCE-TUNING.md; rewrote TESTING-STATUS.md; updated RELEASE-HISTORY.md

All 3 architectures: Stage 6 BOOTOK, 29/29 tests, zero warnings.

---

## v0.5.7 -- Phase 5 Performance Optimization

**Released**: February 26, 2026

6 implementation sprints bringing Phase 5 from ~30% to ~75%:

- **Per-CPU Page Frame Cache** -- `PerCpuPageCache` (64-frame, batch refill/drain 32), `per_cpu_alloc_frame()`/`per_cpu_free_frame()`
- **IPC Fast Path Completion** -- Per-task `ipc_regs: [u64; 7]`, `fast_send()` writes directly to target task registers, `fast_receive()` reads from current task
- **TLB Optimization** -- `TlbFlushBatch` (16 addresses), lazy TLB in scheduler (skip CR3 for kernel threads), `tlb_generation` counter
- **Priority Inheritance** -- `PiMutex` with owner tracking, priority boosting, original priority restoration
- **Benchmarking Suite** -- 7 micro-benchmarks with Phase 5 targets, `perf` shell builtin
- **Software Tracepoints** -- 10 event types, per-CPU ring buffers (4096 events/CPU), `trace` shell builtin

All 3 architectures: Stage 6 BOOTOK, 29/29 tests, zero warnings.

---

## v0.5.6 -- Phase 5 Scheduler, IPC, Init

**Released**: February 25, 2026

First Phase 5 sprint. 8 implementation sprints:

- **Scheduler Context Switch** -- `switch_to()` wired to `context_switch()` (all 3 architectures), TSS RSP0 per-task kernel stacks
- **IPC Blocking/Wake** -- `send_sync()` directly switches to waiting receiver, fast path framework
- **User-Space /sbin/init** -- PID 1 running in Ring 3
- **Dead Code Audit** -- 136 to less than 100 `#[allow(dead_code)]` annotations
- **Native Execution** -- NATIVE_ECHO_PASS (compile + link + execute on VeridianOS)
- **TODO Resolution** -- All 56 `TODO(phase5)` markers across 31 files resolved

91 files changed, +1399/-343 lines. All 3 architectures: Stage 6 BOOTOK, 29/29 tests, zero warnings.

---

## v0.5.5 -- POSIX Partial Munmap + Native BusyBox 208/208

**Released**: February 25, 2026

- **POSIX Partial Munmap** -- 5-case munmap (exact, front trim, back trim, hole punch, sub-range); fixes GCC ggc garbage collector segfaults
- **Consolidated brk()** -- Single BTreeMap entry instead of per-sbrk(), O(1) instead of O(n^2)
- **Native BusyBox** -- 208/208 sources compiled and linked by native GCC 14.2 on VeridianOS
- **12 libc stubs** -- gethostbyname, getpeername, getsockname, inet_aton, inet_ntoa, sendto, initgroups, endgrent, chroot, fchdir, settimeofday

17 files changed, +436/-75 lines.

---

## v0.5.4 -- Critical Memory Leak Fixes

**Released**: February 25, 2026

Three critical memory leak fixes totaling ~272MB over 630 process executions:

- **GP Fault wrmsr** -- Explicit EAX/EDX register constraints for MSR writes (release-only register allocation conflict)
- **Page Table Subtree Leak** -- `VAS::clear()` now frees L3/L2/L1 page table frames during exec() (~75MB leaked)
- **Thread Stack Leak** -- Fixed double-allocation and added kernel stack frame cleanup in `cleanup_process()` (~197MB leaked)

5 files changed, +230/-97 lines.

---

## v0.5.3 -- BusyBox Ash Compatibility + Process Hardening

**Released**: February 24, 2026

- **Ash Shell Compatibility** -- ENOTTY for non-terminal fds, sysconf, exec family, fnmatch/glob rewrite, tcgetpgrp fix
- **Process Lifecycle Hardening** -- MAX_PROCESSES=1024 enforcement, boot-context zombie reaping (213+ sequential execs), fd leak detection
- **ARG_MAX Enforcement** -- 128KB cumulative limit across argv+envp, MAX_ARGS=32768
- **strftime + popen** -- 28 format specifiers, fork/exec/pipe-based popen (16 concurrent streams)

35 files changed, +3475/-206 lines.

---

## v0.5.2 -- BusyBox EPIPE, Float Printf, POSIX Regex

**Released**: February 24, 2026

- **EPIPE/BrokenPipe** -- `sys_write()` returns EPIPE on broken pipe (critical for pipelines)
- **Float printf** -- `__format_double()` (~170 lines) for `%f/%g/%e` format specifiers
- **384MB Kernel Heap** -- Scaled from 128MB for native compilation workloads
- **sbrk Hardening** -- 64KB chunk pre-allocation, page-aligned breaks, 512MB per-process limit
- **POSIX Regex** -- 1291-line BRE/ERE NFA engine (regcomp/regexec/regfree/regerror, 12 character classes)
- **30+ libc headers** -- byteswap.h, endian.h, regex.h, sched.h, syslog.h, and more

69 files changed, +6953/-262 lines.

---

## v0.5.1 -- Coreutils + Pipe Fix

**Released**: February 23, 2026

- **6 Coreutils** -- echo, cat, wc, ls, sort, pipeline_test (~884 lines C) in `userland/coreutils/`
- **Critical Pipe Fix** -- `sys_pipe2()` wrote fd values as `usize` (8 bytes) instead of `i32` (4 bytes), corrupting second fd
- **Clippy Clean** -- cfg interleaving restructured to top-level `#[cfg]` blocks

83 files changed, +10926/-699 lines.

---

## v0.5.0 -- Self-Hosting Tier 7 Complete

**Released**: February 21, 2026

Self-hosting Tier 7 complete across 10 sprints:

- **ELF/fork/exec/wait** -- Full process lifecycle, console blocking read, fd 0/1/2 wiring
- **User-Space Shell Bootstrap** -- Interactive shell from Ring 3
- **Dead Code Audit** -- 159 to 136 annotations
- **TODO Categorization** -- 79 items split to phase5/phase6
- **T7-3 GCC Toolchain** -- GCC 14.2.0 + binutils 2.43 (Canadian cross)

130 files changed, +952/-254 lines.

---

## v0.4.9 -- Self-Hosting Foundation

**Released**: February 18, 2026

Major milestone: self-hosting tiers T0-T5, complete libc, virtio-blk driver, TAR rootfs:

- **30+ Syscalls** -- Complete system call interface for POSIX compatibility
- **Complete libc** -- Full C library for cross-compilation
- **VirtIO-Blk Driver** -- Block device for rootfs access
- **TAR Rootfs** -- Boot from TAR archive containing toolchain
- **CR3 Switching Removed** -- ~500-2000 cycles saved per syscall

209 files changed, +24573/-8618 lines.

---

## v0.4.8 -- Fbcon Scroll Fix + KVM

**Released**: February 16, 2026

Fbcon scroll fix, KVM acceleration documentation, free panic fix. 7 files changed.

---

## v0.4.7 -- Fbcon Glyph Cache + Pixel Ring Buffer

**Released**: February 16, 2026

128KB glyph cache, pixel ring buffer (O(1) scroll), write-combining PAT (1200+ MB/s). 8 files changed.

---

## v0.4.6 -- Fbcon Back-Buffer + Text Cell Ring

**Released**: February 16, 2026

RAM back-buffer, text cell ring, dirty row tracking for framebuffer console.

---

## v0.4.5 -- Framebuffer Display + PS/2 Keyboard

**Released**: February 16, 2026

UEFI GOP framebuffer (1280x800 BGR), ramfb driver (AArch64/RISC-V), PS/2 keyboard polling, 29/29 tests.

---

## v0.4.4 -- CWD Prompt + VFS Population

**Released**: February 16, 2026

CWD prompt in shell, VFS population with /proc /dev /etc, RISC-V ELF fix.

---

## v0.4.3 -- Interactive Shell (vsh)

**Released**: February 15, 2026

18 sprints implementing a full interactive shell: ANSI parser, line editor, pipes, redirects, variables, glob, tab completion, job control, scripting, 24 builtins. AArch64 DirectUartWriter fix. 13 new files, +8630 lines.

---

## v0.4.2 -- IRQ Framework + Timer Management

**Released**: February 15, 2026

IRQ framework, timer management, syscall hardening.

---

## v0.4.1 -- Technical Debt Remediation

**Released**: February 15, 2026

Cross-cutting remediation across 58 kernel source files:

- **Bootstrap Refactoring** -- `kernel_init_main()` refactored from 370-line monolith to 24-line dispatcher with 6 focused helpers; guarded `unwrap()` on `BOOT_ALLOCATOR` lock replaced with contextual `expect()`
- **Error Handling Audit** -- 22 `let _ =` patterns in security-critical subsystems upgraded to log warnings (auth RNG, SIGCHLD delivery, frame leaks, capability inheritance, network registration, database persistence)
- **Dead Code Consolidation** -- 157 per-item `#[allow(dead_code)]` in `pkg/` replaced with 11 module-level `#![allow(dead_code)]` directives
- **String Error Elimination** -- 7 remaining `Err("...")` in `arch/x86_64/usermode.rs` converted to typed `KernelError` variants
- **TODO Reclassification** -- 35 `TODO(phase4)` reclassified to `TODO(future)`, 12 untagged TODOs given phase markers

58 files changed (+407/-352 lines). All 3 architectures: Stage 6 BOOTOK, 27/27 tests, zero warnings.

---

## v0.4.0 -- Phase 4 Milestone

**Released**: February 15, 2026

Formal Phase 4 milestone with comprehensive syscall API documentation (19 wrappers fully documented with examples, errors, and arguments) and 5 new Phase 4 boot tests bringing the total to 27/27. Version bump to 0.4.0 marks Phase 4 as complete.

8 files changed (+1,294/-103 lines). All 3 architectures: Stage 6 BOOTOK, 27/27 tests, zero warnings.

---

## v0.3.9 -- Phase 4 Completion + Userland Bridge

**Released**: February 15, 2026

Completes Phase 4 (Package Ecosystem) to 100% and implements the Userland Bridge for Ring 0 to Ring 3 transitions.

**Userland Bridge (5 sprints):**

- **GDT User Segments** -- Ring 3 code/data segments (0x30/0x28), SYSCALL/SYSRET MSR configuration (EFER, LSTAR, STAR, SFMASK, KernelGsBase)
- **Embedded Init Binary** -- x86_64 machine code init process (57 bytes) using SYSCALL for sys_write + sys_exit, with ELF header generation
- **Ring 3 Entry via iretq** -- `enter_usermode()` pushing SS/RSP/RFLAGS/CS/RIP frame; page table walker with safe frame allocation (skips bootloader page table pages)
- **Syscall Backends** -- sys_write serial fallback for fd 1/2, sys_read serial input for fd 0, sys_exit process termination
- **Integration** -- Full Ring 0 -> Ring 3 -> SYSCALL -> Ring 0 path verified; init binary prints "VeridianOS init started" via serial

**Phase 4 Finalization:**

- SDK Generator, Plugin System, Async Runtime type definitions
- PHASE4_TODO.md updated to 100% complete

22 files changed, 5 new files. All 3 architectures: Stage 6 BOOTOK, 27/27 tests (5 new Phase 4 tests), zero warnings.

---

## v0.3.8 -- Phase 4 Groups 3+4: Toolchain, Testing, Compliance, Ecosystem

**Released**: February 15, 2026

Three parallel implementation sprints advancing Phase 4 to ~95%:

- **Toolchain Manager** -- Toolchain registry, cross-compiler config, linker script generation, CMake toolchain files
- **Testing + Compliance** -- Package test framework, security scanner (9 patterns), license detection and compatibility checking, dependency graph analysis with cycle detection
- **Statistics + Ecosystem** -- Package stats collector, update notifications, CVE advisory checking, core package ecosystem definitions (base-system, dev-tools, drivers, apps)

5 new files (+2,350 lines). All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.7 -- Phase 4 Group 2: Ports Build, Reproducible Builds, Security

**Released**: February 15, 2026

Three parallel implementation sprints advancing Phase 4 to ~85%:

- **Ports Build Execution** -- Real SHA-256 checksum verification, `execute_command()` framework for build steps, VFS-first port collection scanning
- **Reproducible Builds** -- `BuildSnapshot`/`BuildManifest` types, environment normalization (zeroed timestamps, canonical paths), manifest comparison and serialization
- **Repository Security** -- Access control with Ed25519 upload verification, malware pattern scanning (10 default patterns), CVE vulnerability database

5 files changed (+1,385/-49 lines), 1 new file. All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.6 -- Phase 4 Group 1 + Build Fixes

**Released**: February 15, 2026

Four parallel implementation sprints advancing Phase 4:

- **Repository Infrastructure** -- Repository index with Ed25519 verification, mirror manager with failover, multi-repo configuration
- **Package Removal** -- Config file preservation on remove/upgrade, orphan package detection and batch removal
- **Binary Delta Updates** -- Block-matching delta computation/application with SHA-256 verification for incremental downloads
- **Config File Tracking** -- FileType classification (Binary/Config/Documentation/Asset) with path-based inference
- **RISC-V Build Fix** -- Changed `jal` to `call` in boot.S (kernel grew past JAL's 1MB range)

7 files changed (+717/-392 lines), 1 new file. All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.5 -- Critical Architecture Boot Fixes

**Released**: February 15, 2026

Resolves 3 architecture-specific boot issues:

- **x86_64 CSPRNG Double Fault** -- Added CPUID check for RDRAND support before use; prevents `#UD` -> double fault on CPU models without RDRAND (e.g., QEMU `qemu64`)
- **RISC-V Frame Allocator** -- Fixed memory start address from `0x88000000` (end of RAM) to `0x80E00000` (after kernel image); frame allocations now reference valid physical memory
- **RISC-V Stack Canary Guard** -- Restricted RNG usage during process creation to x86_64 only; prevents unhandled faults on RISC-V (no `stvec` trap handler during creation)
- **x86_64 Boot Stack Overflow** -- Increased boot stack from 64KB to 256KB; prevents silent overflow from `CapabilitySpace` array construction (~20KB) in debug builds

4 files changed (+67/-21 lines). All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.4 -- Phase 1-3 Integration + Phase 4 Package Ecosystem

**Released**: February 15, 2026

Two-track release closing Phase 1-3 integration gaps and advancing Phase 4 to ~75% across 14 implementation sprints.

**Phase 1-3 Integration Gaps Closed (7 sprints):**

- **IPC-Scheduler Bridge** -- IPC sync_send/sync_receive now block via scheduler instead of returning ChannelFull/NoMessage; sync_reply wakes blocked senders; async channels wake endpoint waiters after enqueue
- **VMM-Page Table Integration** -- map_region/unmap_region write to real architecture page tables via PageMapper; VAS operations allocate/free physical frames via frame allocator
- **Capability Validation** -- IPC capability validation performs two-level check against process capability space; fast path process lookup uses real process table
- **FPU Context Switching** -- NEON Q0-Q31 save/restore on AArch64; F/D extension f0-f31 save/restore on RISC-V
- **Thread Memory** -- Thread creation allocates real stack frames with guard pages; TLS allocation uses real frame allocation with architecture-specific register setup (FS_BASE/TPIDR_EL0/tp)
- **Shared Memory** -- Regions allocate/free physical frames and flush TLB; transfer_ownership validates target process; unmap properly frees frames
- **Zero-Copy IPC** -- Uses real ProcessPageTable with VAS delegation; allocate_virtual_range uses VAS mmap instead of hardcoded address

**Phase 4 Package Ecosystem (7 sprints, ~75% complete):**

- **Transaction System** -- Package manager with atomic install/remove/upgrade operations and rollback support
- **DPLL SAT Resolver** -- Dependency resolver with version ranges, virtual packages, conflict detection, and backtracking
- **Ports Framework** -- 6 build types (Autotools, CMake, Meson, Cargo, Make, Custom); port collection management with 6 standard categories
- **SDK Types** -- ToolchainInfo, BuildTarget, SdkConfig; typed syscall API wrappers for 6 subsystems; package configuration with .pc file generation
- **Shell Commands** -- install, remove, update, upgrade, list, search, info, verify
- **Package Syscalls** -- SYS_PKG_INSTALL (90) through SYS_PKG_UPDATE (94)
- **Crypto Hardening** -- Real Ed25519 signature verification with trust policies for packages

**Phase 4 Prerequisites:**

- Page fault handler framework with demand paging and stack growth
- ELF dynamic linker support with auxiliary vector and PT_INTERP parsing
- Process waitpid infrastructure with WNOHANG and POSIX wstatus encoding
- Per-process working directory with path normalization

42 files changed (+7,581/-424 lines), 15 new files. AArch64 and RISC-V boot to Stage 6 BOOTOK with 22/22 tests passing.

---

## v0.3.3 -- Technical Debt Remediation

**Released**: February 14, 2026

Comprehensive technical debt remediation across 4 parallel work streams:

- **Soundness and Safety** -- Fixed RiscvScheduler soundness issue (UnsafeCell to spin::Mutex), deleted 353-line dead `security::crypto` module, fixed 5 clippy suppressions, deduplicated x86_64 I/O port functions
- **Error Type Migration** -- Eliminated all remaining `Err("...")` string literals (96 to 0) and `Result<T, &str>` signatures (91 to 1 justified); 11 primary files + ~33 cascade files converted to typed `KernelError`
- **Code Organization** -- Split 3 files exceeding 1,500 lines: `crypto/post_quantum.rs` into directory (kyber/dilithium/hybrid), `security/mac.rs` into directory (parser extracted), `elf/types.rs` extracted; created `arch/entropy.rs` abstraction
- **Comment and Annotation Cleanup** -- 55 `TODO(phase3)` items triaged to zero (9 eliminated, 1 removed as already implemented, 45 reclassified), 15 unnecessary `#[allow(unused_imports)]` removed, `process_compat::Process` renamed to `TaskProcessAdapter`
- **Net result**: 80 files changed, +1,024/-5,069 lines (net -4,045 lines), zero `Result<T, &str>` remaining, zero soundness bugs

---

## v0.3.2 -- Phase 2 and Phase 3 Completion

**Released**: February 14, 2026

Comprehensive completion of both Phase 2 (User Space Foundation: 80% to 100%) and Phase 3 (Security Hardening: 65% to 100%) across 15 implementation sprints.

**Phase 2 Sprints (6):**

- **Clock/Timestamp Infrastructure** -- `get_timestamp_secs()`/`get_timestamp_ms()` wrappers; RamFS/ProcFS/DevFS timestamp integration; VFS `list_mounts()`; init system and shell uptime using real timers
- **BlockFS Directory Operations** -- ext2-style `DiskDirEntry` parsing; `readdir()`, `lookup_in_dir()`, `create_file()`, `create_directory()` with `.`/`..`, `unlink_from_dir()`, `truncate_inode()` block freeing
- **Signal Handling + Shell Input** -- PTY signal delivery (SIGINT, SIGWINCH); architecture-conditional serial input (x86_64 port I/O, AArch64 UART MMIO, RISC-V SBI getchar); touch command implementation
- **ELF Relocation Processing** -- `process_relocations()` with AArch64 (R_AARCH64_RELATIVE/GLOB_DAT/JUMP_SLOT/ABS64) and RISC-V (R_RISCV_RELATIVE/64/JUMP_SLOT) types; PIE binary support; dynamic linker bootstrap delegation
- **Driver Hot-Plug Event System** -- `DeviceEvent` enum (Added/Removed/StateChanged); `DeviceEventListener` trait; publish-subscribe notification; auto-probe on device addition
- **Init System Hardening** -- Service wait timeout with SIGKILL; exponential backoff restart (base_delay * 2^min(count,5)); architecture-specific reboot (x86_64 keyboard controller 0xFE, AArch64 PSCI, RISC-V SBI reset); timer-based sleep replacing spin loops

**Phase 3 Sprints (9):**

- **Cryptographic Algorithms** -- ChaCha20-Poly1305 AEAD (RFC 8439); Ed25519 sign/verify (RFC 8032); X25519 key exchange (RFC 7748); ML-DSA/Dilithium sign/verify (FIPS 204); ML-KEM/Kyber encapsulate/decapsulate (FIPS 203); ChaCha20-based CSPRNG with hardware entropy seeding
- **Secure Boot Verification** -- Kernel image SHA-256 hashing via linker symbols; Ed25519 signature verification; measured boot with measurement log; TPM PCR extension; certificate chain validation
- **TPM Integration** -- MMIO-based TPM 2.0 communication; locality management; command marshaling (TPM2_Startup, PCR_Extend, PCR_Read, GetRandom); `seal_key()`/`unseal_key()` for TPM-backed storage
- **MAC Policy System** -- Text-based policy language parser (`allow source target { perms };`); domain transitions; RBAC layer (users to roles to types); MLS support (sensitivity + categories + dominance); `SecurityLabel` struct replacing `&'static str` labels
- **Audit System Completion** -- Event filtering by type; structured format (timestamp, PID, UID, action, target, result); VFS-backed persistent storage; binary serialization; wired into syscall dispatch, capability ops, MAC decisions; real-time alert hooks
- **Memory Protection Hardening** -- ChaCha20 CSPRNG-based ASLR entropy; DEP/NX enforcement via page table NX bits; guard page integration with VMM; W^X enforcement; stack guard pages; Spectre v1 barriers (LFENCE/CSDB); KPTI (separate kernel/user page tables on x86_64)
- **Authentication Hardening** -- Real timestamps for MFA; PBKDF2-HMAC-SHA256 password hashing; password complexity enforcement; password history (prevent reuse); account expiration
- **Capability System Phase 3** -- ObjectRef::Endpoint in IPC integration; PRESERVE_EXEC filtering; default IPC/memory capabilities; process notification on revocation; permission checks; IPC broadcast for revocation
- **Syscall Security + Fuzzing** -- MAC checks before capability checks in syscall handlers; audit logging in syscall entry/exit; argument validation (pointer bounds, size limits); `FuzzTarget` trait with mutation-based fuzzer; ELF/IPC/FS/capability fuzz targets; crash detection via panic handler hooks

---

## v0.3.1 -- Technical Debt Remediation

**Released**: February 14, 2026

Comprehensive 5-sprint remediation covering safety, soundness, and architecture:

- **Critical Safety** -- Fixed OnceLock::set() use-after-free soundness bug, fixed process_compat memory leak, added `#[must_use]` to KernelError
- **Static Mut Elimination** -- Converted 48 of 55 `static mut` declarations to safe patterns (OnceLock, Mutex, Atomics); 7 retained with documented SAFETY justifications (pre-heap boot, per-CPU data)
- **Panic-Free Syscalls** -- Removed 8 production panic paths from syscall/VFS handlers via error propagation
- **Error Type Migration** -- Converted 150+ functions across 18 files from `&'static str` errors to typed `KernelError` (legacy ratio reduced from ~65% to ~37%)
- **Architecture Abstractions** -- PlatformTimer trait with 3 arch implementations, memory barrier abstractions (memory_fence, data_sync_barrier, instruction_sync_barrier)
- **Dead Code Cleanup** -- Removed 25 incorrect `#[allow(dead_code)]` annotations plus 1 dead function

---

## v0.3.0 -- Phase 3 Security Hardening

**Released**: February 14, 2026

Architecture leakage reduction and comprehensive security hardening:

- **Architecture Leakage Reduction** -- `kprintln!`/`kprint!` macro family, `IpcRegisterSet` trait, heap/scheduler consolidation; `cfg(target_arch)` outside `arch/` reduced from 379 to 204 (46% reduction)
- **Test Expansion** -- Kernel-mode init tests expanded from 12 to 22, all passing on all architectures
- **Capability System Hardening** -- Root capability bootstrap, per-process resource quotas (256 cap limit), syscall enforcement (fork/exec/kill require Process cap)
- **MAC + Audit** -- MAC convenience functions wired into VFS `open()`/`mkdir()`, audit events for capability and process lifecycle
- **Memory Hardening** -- Speculation barriers (LFENCE/CSDB/FENCE.I) at syscall entry, guard pages in VMM, stack canary integration
- **Crypto Validation** -- SHA-256 NIST FIPS 180-4 test vector validation

---

## v0.2.5 -- RISC-V Crash Fix and Architecture Parity

**Released**: February 13, 2026

Full multi-architecture boot parity achieved with RISC-V post-BOOTOK crash fix, heap sizing corrections, and 30-second stability tests passing on all architectures.

---

## v0.2.4 -- Technical Debt Remediation

**Released**: February 13, 2026

Comprehensive codebase quality improvement:

- **550 `// SAFETY:` comments** added across 122 files (0.9% to 84.5% coverage)
- **180 new unit tests** across 7 modules (70 to 250 total)
- **5 god objects split** into focused submodules (0 files >1000 LOC remaining)
- **201 TODO/FIXME/HACK** comments triaged with phase tags
- **204 files** with module-level documentation (up from ~60)
- **39 files** cleaned of `#[allow(dead_code)]` with proper feature gating
- **161 files changed** total

---

## v0.2.3 -- Phase 2 User Space Foundation

**Released**: August 16, 2025 (architecturally complete). Runtime activation verified February 13, 2026.

Implementation achievements:

- **Virtual Filesystem (VFS) Layer** -- Mount points, ramfs, devfs (`/dev`), procfs (`/proc`)
- **File Descriptors and Operations** -- POSIX-style operations with full syscall suite (open, read, write, close, seek, mkdir, etc.)
- **Live System Information** -- `/proc` with real process and memory stats
- **Device Abstraction** -- `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/console`
- **Process Server** -- Complete process management with resource handling
- **ELF Loader** -- Dynamic linking support for user-space applications
- **Thread Management** -- Complete APIs with TLS and scheduling policies
- **Standard Library** -- C-compatible foundation for user-space
- **Init System** -- Service management with dependencies and runlevels
- **Shell Implementation** -- 20+ built-in commands with environment management
- **Driver Suite** -- PCI/USB bus drivers, network drivers (Ethernet + loopback with TCP/IP stack), storage drivers (ATA/IDE), console drivers (VGA + serial)
- **Runtime Init Tests** -- 22 kernel-mode tests (6 VFS + 6 shell + 10 security/capability/crypto) verifying subsystem functionality at boot

---

## v0.2.1 -- Phase 1 Maintenance Release

**Released**: June 17, 2025

Maintenance release with boot fixes, AArch64 LLVM workaround, and all three architectures booting to Stage 6 BOOTOK.

---

## v0.2.0 -- Phase 1 Microkernel Core

**Released**: June 12, 2025 (Phase 1 completed in 5 days, started June 8, 2025)

Core subsystems implemented:

- **IPC System** -- Synchronous/asynchronous channels, registry, performance tracking, rate limiting, capability integration
- **Memory Management** -- Frame allocator, virtual memory, page tables, bootloader integration, VAS cleanup
- **Process Management** -- PCB, threads, context switching, synchronization primitives, syscalls
- **Scheduler** -- CFS, SMP support, load balancing, CPU hotplug, task management
- **Capability System** -- Tokens, rights, space management, inheritance, revocation, per-CPU cache
- **Test Framework** -- `no_std` test framework with benchmarks, IPC/scheduler/process tests

---

## v0.1.0 -- Phase 0 Foundation and Tooling

**Released**: June 7, 2025

Initial release establishing the development foundation:

- Rust nightly toolchain with custom target specifications
- Multi-architecture build system (x86_64, AArch64, RISC-V)
- CI/CD pipeline with GitHub Actions
- QEMU testing infrastructure
- GDB debugging support
- Documentation framework

---

## DEEP-RECOMMENDATIONS

All 9 of 9 recommendations complete:

1. Bootstrap circular dependency fix
2. AArch64 calling convention
3. Atomic operations
4. Capability overflow
5. User pointer validation
6. Custom test framework
7. Error type migration
8. RAII patterns
9. Phase 2 readiness

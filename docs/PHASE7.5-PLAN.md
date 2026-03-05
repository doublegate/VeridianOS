# Phase 7.5: Follow-On Enhancements -- Comprehensive Development Plan

**Version**: 1.0.0
**Created**: March 5, 2026
**Status**: Planned
**Dependencies**: Phase 7 complete (v0.10.6), all 6 waves done, CI 11/11 green, 998 host-target tests
**Estimated Duration**: 6-9 months (8 waves, ~80 items across 13 categories)
**Starting Codebase**: 349 kernel source files, ~179K lines, tri-arch BOOTOK 29/29

---

## Executive Summary

Phase 7.5 bridges VeridianOS from "production readiness" (Phase 7) to "fully self-sufficient OS" (Phase 8). It delivers 80 enhancements across networking, multimedia, GPU, hypervisor, containers, security, performance, desktop, userland, filesystem, protocols, and hardware drivers. Each item builds on Phase 7's foundation and is organized into 8 dependency-ordered waves for incremental delivery with continuous verification.

---

## Wave Architecture

```
Wave 1: Filesystem & Core Security       (ext4, FAT32, tmpfs, KASLR, stack canaries, SMEP/SMAP)
    |
Wave 2: Performance & Scheduling          (EDF deadline, cache-aware alloc, false sharing, power mgmt)
    |
Wave 3: Hardware Drivers & RTC            (USB xHCI, mass storage, HID, AHCI/SATA, Bluetooth HCI)
    |
Wave 4: Networking Foundations            (TCP Reno/Cubic, SACK, DNS resolver, NTP, VLAN, multicast)
    |
Wave 5: Crypto & Protocol Layer           (TLS 1.3, SSH server, QUIC, WireGuard, mDNS, HTTP/1.1)
    |
Wave 6: Desktop & Userland               (clipboard, drag-drop, fonts, io_uring, ptrace, users/groups)
    |
Wave 7: Audio/Video & GPU                (ALSA, USB audio, PNG/JPEG/GIF, VirtIO GPU 3D, OpenGL ES 2.0)
    |
Wave 8: Hypervisor & Containers          (nested virt, OCI, cgroups, overlay FS, seccomp, veth)
```

---

## Wave 1: Filesystem & Core Security

**Duration**: 4-6 weeks | **Items**: 12 | **Priority**: Critical foundation
**Rationale**: Real filesystems (ext4/FAT32) and security hardening (KASLR/SMEP/SMAP) are prerequisites for nearly all subsequent waves. tmpfs enables efficient in-memory operations. inotify enables file monitoring. These are the most foundational enhancements.

### Sprint 1.1: tmpfs (Memory-Backed Filesystem)

**Category**: Filesystem | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/fs/tmpfs.rs`
- Implement `Filesystem` trait for tmpfs with in-memory inode table
- Support configurable size limits via mount options (`size=128M`)
- Store file data in page-aligned frame allocations (reuse `mm::frame_allocator`)
- Directory entries in `BTreeMap<String, InodeNumber>`
- Wire into VFS mount table (`fs/mod.rs`) with `mount -t tmpfs tmpfs /tmp`
- Support all VFS operations: create, read, write, unlink, mkdir, rmdir, readdir, stat, truncate
- Automatic cleanup on unmount (free all backing frames)

**Key Design Decisions**:
- Use frame allocator directly (not heap) for file data -- avoids heap fragmentation for large files
- Page-granular allocation (4KB minimum) with last-page partial tracking
- Inode numbers from AtomicU64 counter (simple, no reuse needed for tmpfs)

**Verification**: Mount tmpfs, create/read/write/delete files, verify size limit enforcement, unmount and verify frame reclamation.

### Sprint 1.2: FAT32 Read/Write Support

**Category**: Filesystem | **Estimated Lines**: ~800

**Implementation**:
- New file: `kernel/src/fs/fat32.rs`
- Parse BPB (BIOS Parameter Block) and FSInfo sector from block device
- FAT table navigation: cluster chain traversal, free cluster allocation
- Directory parsing: 8.3 short names + VFAT long file name (LFN) entries
- Read path: resolve path -> find directory entry -> follow cluster chain -> copy data
- Write path: allocate clusters -> update FAT entries -> write data -> update directory entry
- Support: create file, write, append, truncate, delete, mkdir, rmdir
- Integration with VirtIO-blk and NVMe block device abstraction (`drivers/storage.rs`)

**Key Design Decisions**:
- Cache FAT table in memory (typically 128KB-4MB depending on volume size)
- Lazy directory loading (read directory clusters on demand)
- Write-through for FAT table updates (safety over performance)
- Use existing `BlockDevice` trait from `drivers/storage.rs`

**Dependencies**: VirtIO-blk driver (exists), block device trait (exists)

**Verification**: Format image with `mkfs.fat -F 32`, mount in kernel, read/write files, verify with host tools.

### Sprint 1.3: ext4 Read-Only Support

**Category**: Filesystem | **Estimated Lines**: ~900

**Implementation**:
- New file: `kernel/src/fs/ext4.rs`
- Parse superblock (offset 1024, magic 0xEF53) and block group descriptors
- Inode table traversal with extent tree support (ext4 extents vs classic block map)
- Directory traversal: linear and `dir_index` (HTree) hash directories
- Journal replay: scan journal (inode 8) for committed transactions, replay metadata blocks
- Read operations only: open, read, readdir, stat, readlink
- Extent tree: internal nodes (idx) and leaf nodes (extent) with `ee_block`, `ee_start`, `ee_len`
- 64-bit block numbers, large file support (>2GB)

**Key Design Decisions**:
- Read-only simplifies implementation significantly (no journal writes, no allocation)
- Journal replay at mount time ensures consistent state after unclean shutdown
- Support both extent-based and legacy block-map inodes (check `EXT4_EXTENTS_FL` flag)
- Cache superblock and block group descriptors; inode reads are on-demand

**Dependencies**: Block device abstraction (exists)

**Verification**: Create ext4 image on host, populate files, mount read-only in kernel, verify all contents readable.

### Sprint 1.4: inotify File Event Monitoring

**Category**: Filesystem | **Estimated Lines**: ~350

**Implementation**:
- New file: `kernel/src/fs/inotify.rs`
- `inotify_init()` syscall: returns file descriptor for inotify instance
- `inotify_add_watch(fd, path, mask)`: add watch on path with event mask
- `inotify_rm_watch(fd, wd)`: remove watch descriptor
- Event types: `IN_CREATE`, `IN_DELETE`, `IN_MODIFY`, `IN_MOVED_FROM`, `IN_MOVED_TO`, `IN_ATTRIB`, `IN_CLOSE_WRITE`
- Event queue: per-instance circular buffer (256 events)
- VFS integration: emit events from `fs/mod.rs` create/write/unlink/rename operations
- Readable via `read()` syscall on the inotify fd (returns `inotify_event` structs)

**Key Design Decisions**:
- Separate inotify instance per fd (not global) for isolation
- Fixed event buffer (no heap allocation per event)
- Path-based watches (resolve to inode at add time, track inode number)

**Verification**: Create inotify watch on directory, create/modify/delete files, read events, verify correct event types and names.

### Sprint 1.5: File Locking (flock/fcntl)

**Category**: Filesystem | **Estimated Lines**: ~300

**Implementation**:
- New file: `kernel/src/fs/locking.rs`
- POSIX advisory locks via `fcntl(fd, F_SETLK/F_SETLKW/F_GETLK, struct flock)`
- BSD-style whole-file locks via `flock(fd, operation)` (LOCK_SH, LOCK_EX, LOCK_UN, LOCK_NB)
- Lock table: per-inode lock list with `(pid, type, start, len)` tuples
- Deadlock detection for `F_SETLKW` blocking locks (cycle detection in wait graph)
- Auto-release on fd close and process exit
- Integration with VFS open/close paths

**Verification**: Two processes acquire shared lock, one tries exclusive (blocks), verify POSIX semantics.

### Sprint 1.6: Extended Attributes (xattr)

**Category**: Filesystem | **Estimated Lines**: ~250

**Implementation**:
- New file: `kernel/src/fs/xattr.rs`
- Syscalls: `getxattr()`, `setxattr()`, `listxattr()`, `removexattr()` + `l`/`f` variants
- Namespace support: `user.*`, `system.*` (security namespace for MAC integration)
- Storage: per-inode `BTreeMap<String, Vec<u8>>` (in-memory for RamFS/tmpfs)
- FAT32: store in hidden files or reject (FAT32 has no native xattr)
- ext4: read from inode's extra space and external xattr blocks
- Size limits: 64KB per attribute value, 256 attributes per inode

**Verification**: Set/get/list/remove xattr on files, verify namespace enforcement, test with security labels.

### Sprint 1.7: KASLR (Kernel Address Space Layout Randomization)

**Category**: Security | **Estimated Lines**: ~350

**Implementation**:
- Modify `kernel/src/arch/x86_64/boot.rs` and linker script
- Randomize kernel base address within 2GB region (0xFFFFFFFF00000000 - 0xFFFFFFFF80100000)
- Entropy source: RDRAND (x86_64), boot-time timer jitter (fallback)
- Slide granularity: 2MB (huge page aligned) for TLB efficiency
- Apply slide to: kernel text, rodata, data, BSS, heap base, stack base
- Update all absolute references via position-independent relocation
- Randomize kernel heap base offset and per-CPU data area locations
- Physical-to-virtual mapping offset adjustment

**Key Design Decisions**:
- 2MB granularity limits entropy to ~10 bits within 2GB range -- acceptable for QEMU, sufficient for hardware
- Must patch page table entries at boot before MMU enable (or use identity mapping during slide)
- AArch64/RISC-V: analogous implementation with architecture-specific randomization ranges

**Verification**: Boot multiple times, verify kernel base differs, verify all symbols accessible, run full test suite.

### Sprint 1.8: Stack Canaries

**Category**: Security | **Estimated Lines**: ~200

**Implementation**:
- Modify `kernel/src/arch/x86_64/gdt.rs` (or new `security/stack_canary.rs`)
- Initialize per-CPU stack canary value from CSPRNG at boot
- Store canary in `gs:0x28` (x86_64 standard location) via GS base MSR
- Enable `-Z stack-protector=strong` or equivalent via custom target JSON `stack-protector` field
- Panic handler for canary check failure: print "stack smashing detected", halt
- Per-thread canary rotation on context switch (optional, for defense-in-depth)

**Key Design Decisions**:
- Use GS segment for canary (standard Linux convention, compiler expects it)
- Randomize canary value per-CPU, not globally, to prevent cross-CPU leak
- Compiler generates the prologue/epilogue automatically; we just need the runtime support

**Dependencies**: CSPRNG (exists in `crypto/random.rs`), GDT/TSS infrastructure (exists)

**Verification**: Deliberately overflow a buffer in test, verify panic instead of corruption.

### Sprint 1.9: SMEP/SMAP Enforcement

**Category**: Security | **Estimated Lines**: ~150

**Implementation**:
- Modify `kernel/src/arch/x86_64/boot.rs` or `security/memory_protection.rs`
- SMEP (Supervisor Mode Execution Prevention): set CR4 bit 20 at boot
  - Prevents kernel from executing code in user-space pages
- SMAP (Supervisor Mode Access Prevention): set CR4 bit 21 at boot
  - Prevents kernel from reading/writing user-space pages unless EFLAGS.AC=1
- `stac()`/`clac()` wrapper functions for controlled user-space access in syscall handlers
- CPUID check (leaf 7, EBX bit 7 for SMEP, bit 20 for SMAP) before enabling
- Graceful fallback if CPU doesn't support SMEP/SMAP (QEMU `-cpu host` does)

**Key Design Decisions**:
- Enable at the earliest possible point after CR4 is accessible
- All user-space pointer access in syscall handlers must be wrapped in `stac()`/`clac()` blocks
- KPTI (already implemented) + SMEP/SMAP = comprehensive kernel isolation

**Verification**: Attempt kernel access to user-space page without STAC, verify fault. Normal syscall paths work.

### Sprint 1.10: Spectre Retpoline Mitigation

**Category**: Security | **Estimated Lines**: ~100

**Implementation**:
- Add `retpoline` flag to custom target JSON (`x86_64-veridian.json`)
- Compiler automatically generates retpoline thunks for indirect branches
- Manual retpoline for any inline assembly indirect calls: `jmp *%rax` -> `call retpoline_rax_trampoline`
- Verify with `objdump` that no unmitigated indirect branches remain in kernel binary
- IBRS/IBPB MSR writes at context switch boundaries (if CPUID reports support)

**Key Design Decisions**:
- Retpoline is the primary mitigation (works on all x86_64)
- IBRS/IBPB is supplementary for CPUs that support it
- Performance impact is minimal for kernel workloads (mostly direct calls)

**Verification**: Disassemble kernel, grep for unmitigated `jmp *%r` / `call *%r`, verify all converted.

### Sprint 1.11: Capability Revocation Propagation

**Category**: Security | **Estimated Lines**: ~250

**Implementation**:
- Modify `kernel/src/cap/revocation.rs`
- Transitive revocation tree walk: when a capability is revoked, recursively revoke all derived capabilities
- Maintain derivation tree: each capability tracks its parent capability ID
- Revocation algorithm: BFS/DFS walk from revoked capability through derivation tree
- Batch revocation: collect all affected capability IDs, then revoke atomically
- Notification: wake any tasks blocked on revoked capabilities with `CapabilityRevoked` error

**Dependencies**: Capability system (exists, `cap/` module)

**Verification**: Create capability chain A->B->C, revoke A, verify B and C also revoked.

### Sprint 1.12: Audit Log Persistence

**Category**: Security | **Estimated Lines**: ~300

**Implementation**:
- Modify `kernel/src/security/audit.rs`
- Write-ahead log (WAL) to BlockFS: append audit events to `/var/log/audit.log`
- Binary format: 64-byte fixed records (timestamp, event_type, pid, uid, result, path_hash)
- Rotation: when log exceeds 1MB, rename to `.1`, create new file
- Buffer in memory (256 events) and flush to disk periodically or on critical events
- Recovery: on boot, verify WAL integrity, truncate partial records

**Dependencies**: BlockFS (exists), VFS write path (exists)

**Verification**: Generate audit events, reboot, verify events persisted on disk, test rotation.

---

## Wave 2: Performance & Scheduling

**Duration**: 3-4 weeks | **Items**: 5 | **Priority**: High
**Rationale**: Deadline scheduling and cache-aware allocation improve all subsequent workloads. Power management enables real hardware deployment. PGO enables build optimization.

### Sprint 2.1: Deadline Scheduling (EDF)

**Category**: Performance | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/sched/deadline.rs`
- Earliest Deadline First (EDF) policy alongside existing CFS
- Task parameters: `runtime`, `deadline`, `period` (all in nanoseconds)
- Admission control: reject task if total utilization exceeds CPU capacity (U <= 1.0 check)
- APIC timer integration: program one-shot timer for nearest deadline
- Bandwidth reservation: guarantee CPU time for deadline tasks before CFS tasks
- SCHED_DEADLINE syscall: `sched_setattr(pid, policy=SCHED_DEADLINE, runtime, deadline, period)`
- Priority: deadline tasks always preempt CFS tasks; among deadline tasks, earliest deadline wins

**Key Design Decisions**:
- Use existing APIC timer (1000Hz from Phase 5.5) for deadline enforcement
- Keep CFS as default; deadline is opt-in per-task
- Global EDF (not partitioned) for simplicity -- sufficient for audio/real-time use cases
- Bandwidth accounting via `struct DeadlineEntity { runtime_remaining, deadline_abs, period }`

**Dependencies**: APIC timer (exists), scheduler framework (exists)

**Verification**: Create deadline task with 10ms runtime / 20ms period, verify it meets deadlines under CFS load. Audio playback stress test.

### Sprint 2.2: Cache-Aware Memory Allocation

**Category**: Performance | **Estimated Lines**: ~350

**Implementation**:
- Modify `kernel/src/mm/frame_allocator.rs` and new `mm/cache_topology.rs`
- Detect cache topology via CPUID (leaf 4 for Intel, leaf 0x8000001D for AMD)
- L1/L2/L3 cache line sizes, set associativity, total size
- Cache coloring: assign frames to "colors" based on physical address bits corresponding to cache set index
- Allocator preference: when possible, allocate frames with same color for a process's working set (improve cache hit rate)
- Per-NUMA-node color tracking: `colors_available[MAX_COLORS]` bitmap per node

**Key Design Decisions**:
- Cache coloring is advisory, not mandatory -- falls back to normal allocation when colored frames exhausted
- Number of colors = L3_size / page_size / associativity (typically 64-256 for modern CPUs)
- Only apply to user-space allocations (kernel uses direct mapping, less benefit)

**Verification**: Benchmark memory-intensive workload with and without cache coloring, measure L3 miss rate change.

### Sprint 2.3: False Sharing Elimination

**Category**: Performance | **Estimated Lines**: ~200

**Implementation**:
- Audit all per-CPU data structures for cache line alignment
- Add `#[repr(align(64))]` to per-CPU structs: `PerCpuPageCache`, `ReadyQueue`, `PerCpuData`
- Pad shared atomics to cache line boundaries: `perf::SYSCALL_COUNT`, `perf::CONTEXT_SWITCH_COUNT`, etc.
- New `CacheAligned<T>` wrapper type: `#[repr(C, align(64))] struct CacheAligned<T>(T)`
- Apply to scheduler per-CPU run queues, IPC per-task registers, TLB generation counters

**Key Design Decisions**:
- 64-byte alignment (Intel cache line size; ARM is also 64B for Cortex-A72)
- Only align truly per-CPU or frequently-contended data -- don't waste memory padding everything

**Verification**: Multi-CPU benchmark (when SMP is active), measure reduction in cross-core cache invalidations.

### Sprint 2.4: Power Management (C-states/P-states)

**Category**: Performance | **Estimated Lines**: ~450

**Implementation**:
- New file: `kernel/src/power/mod.rs`
- ACPI `_CST` method parsing for C-state definitions (C0=active, C1=halt, C2=stop-clock, C3=sleep)
- `_PSS` method parsing for P-state (frequency/voltage) levels
- C-state transitions: idle loop calls `mwait` (Intel) or `wfi` (ARM) with appropriate hint
- P-state transitions: write to `IA32_PERF_CTL` MSR (Intel) or ACPI _PCT method
- Governor: `ondemand` policy -- scale frequency based on CPU utilization over 100ms window
- Integration with scheduler idle path: enter deeper C-state as idle duration increases

**Key Design Decisions**:
- Conservative governor at first (don't enter C3+ which has high exit latency)
- Track per-CPU utilization in scheduler `MetricsSummary` for governor decisions
- QEMU doesn't meaningfully support P-states, but C1 (halt) is respected

**Dependencies**: ACPI parser (exists in `arch/x86_64/acpi.rs`)

**Verification**: Monitor CPU frequency scaling under varying load, verify C-state transitions in idle.

### Sprint 2.5: Profile-Guided Optimization (PGO)

**Category**: Performance | **Estimated Lines**: ~200 (mostly build system)

**Implementation**:
- Modify `build-kernel.sh` and `Cargo.toml`
- Phase 1: Instrument build with `-Cprofile-generate=/tmp/pgo-data`
- Phase 2: Run instrumented kernel in QEMU with representative workload (boot + shell commands + startgui)
- Phase 3: Merge profiles with `llvm-profdata merge`
- Phase 4: Rebuild with `-Cprofile-use=/tmp/pgo-data/merged.profdata`
- CI integration: nightly PGO build as separate workflow (not blocking regular CI)
- Document PGO build process in `docs/PERFORMANCE-TUNING.md`

**Key Design Decisions**:
- PGO is opt-in release optimization, not default build
- Representative workload: full boot + 10 shell commands + GUI startup + shutdown
- Expect 5-15% improvement in hot paths (IPC, syscall, scheduler)

**Dependencies**: Self-hosted Rust compiler (exists, Phase 6.5), LLVM tools

**Verification**: Compare benchmark results (perf shell command) between PGO and non-PGO builds.

---

## Wave 3: Hardware Drivers

**Duration**: 4-5 weeks | **Items**: 6 | **Priority**: High
**Rationale**: USB xHCI unlocks mass storage, HID, and audio USB class drivers. AHCI enables SATA disk access. Bluetooth HCI enables wireless peripherals. RTC is partially complete.

### Sprint 3.1: USB xHCI Host Controller

**Category**: Hardware Drivers | **Estimated Lines**: ~1200

**Implementation**:
- New file: `kernel/src/drivers/usb/xhci.rs`
- PCI enumeration: class 0x0C, subclass 0x03, progif 0x30 (xHCI)
- MMIO register access: capability, operational, runtime, doorbell registers
- Ring buffers: Command Ring (host->controller), Event Ring (controller->host), Transfer Ring (per-endpoint)
- TRB (Transfer Request Block) types: Normal, Setup Stage, Data Stage, Status Stage, Link, No-Op
- Device slot allocation: `Enable Slot` command, `Address Device` command
- Device enumeration: USB descriptor parsing (Device, Configuration, Interface, Endpoint)
- Port management: port status change detection, reset, enable
- Interrupt handling: MSI-X primary event ring completion

**Key Design Decisions**:
- Support xHCI 1.0+ (USB 3.0 host controllers also handle USB 2.0/1.1 devices)
- DMA: use physically-contiguous buffers from `net/dma_pool.rs` (reuse existing DMA infrastructure)
- Max 16 device slots initially (expandable)
- Scratchpad buffer allocation per xHCI spec requirements

**Dependencies**: PCI enumeration (exists), DMA pool (exists), MSI-X interrupt routing (exists in `irq/`)

**Verification**: Boot with USB device in QEMU (`-device qemu-xhci -device usb-kbd`), enumerate device, read descriptors.

### Sprint 3.2: USB Mass Storage (Bulk-Only Transport)

**Category**: Hardware Drivers | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/drivers/usb/mass_storage.rs`
- USB Mass Storage class (class 0x08, subclass 0x06=SCSI, protocol 0x50=BBB)
- Bulk-Only Transport (BOT): CBW (Command Block Wrapper), CSW (Command Status Wrapper)
- SCSI commands: INQUIRY, READ_CAPACITY, READ(10), WRITE(10), TEST_UNIT_READY, REQUEST_SENSE
- Block device interface: implement `BlockDevice` trait for USB storage
- Auto-detection on USB device enumeration (match class/subclass/protocol)
- Integration with VFS mount system for USB drive access

**Dependencies**: xHCI driver (Sprint 3.1), BlockDevice trait (exists)

**Verification**: QEMU `usb-storage` device, enumerate, read blocks, mount filesystem.

### Sprint 3.3: USB HID (Keyboard/Mouse)

**Category**: Hardware Drivers | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/drivers/usb/hid.rs`
- USB HID class (class 0x03): keyboard (protocol 1), mouse (protocol 2)
- HID report descriptor parsing: usage pages, usage IDs, report sizes
- Boot protocol support: simplified 8-byte keyboard reports, 3-byte mouse reports
- Report protocol: parse full HID report descriptors for generic HID devices
- Input event generation: convert USB HID reports to `InputEvent` (EV_KEY/EV_REL)
- Integration with existing input subsystem (`drivers/input.rs`, `drivers/input_event.rs`)
- Interrupt transfer polling via xHCI interrupt endpoint

**Dependencies**: xHCI driver (Sprint 3.1), input subsystem (exists)

**Verification**: QEMU `usb-kbd` and `usb-mouse`, type keys, move mouse, verify events reach desktop.

### Sprint 3.4: AHCI/SATA Controller

**Category**: Hardware Drivers | **Estimated Lines**: ~700

**Implementation**:
- Modify/expand `kernel/src/drivers/storage.rs` or new `drivers/ahci.rs`
- PCI enumeration: class 0x01 (mass storage), subclass 0x06 (SATA), progif 0x01 (AHCI)
- AHCI HBA (Host Bus Adapter) initialization: GHC register, port enumeration
- Port initialization: detect device presence (SSTS.DET), issue COMRESET, wait for PHY ready
- Command list: 32 command slots per port, each with CFIS (Command FIS) + PRDT (Physical Region Descriptor Table)
- FIS-based I/O: H2D Register FIS (ATA commands), D2H Register FIS (status), PIO Setup FIS, DMA Setup FIS
- ATA commands: IDENTIFY DEVICE, READ DMA EXT, WRITE DMA EXT, FLUSH CACHE EXT
- NCQ (Native Command Queuing): 32 outstanding commands per port
- BlockDevice trait implementation

**Key Design Decisions**:
- AHCI is memory-mapped (no PIO), all communication via DMA
- Reuse DMA pool for command tables and PRDTs
- Support ATAPI (CD-ROM) devices via PACKET command (secondary priority)

**Dependencies**: PCI enumeration (exists), DMA infrastructure (exists)

**Verification**: QEMU `-device ahci,id=ahci -drive id=disk1,file=test.img,if=none -device ide-hd,drive=disk1,bus=ahci.0`, detect device, read sectors.

### Sprint 3.5: Bluetooth HCI

**Category**: Hardware Drivers | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/drivers/bluetooth/hci.rs`
- HCI (Host Controller Interface) transport: USB (bulk/interrupt endpoints)
- HCI command/event protocol: command packets (OGF/OCF), event packets, ACL data packets
- Initialization: HCI_Reset, Read_Local_Version, Read_BD_ADDR, Set_Event_Mask
- Inquiry/scanning: HCI_Inquiry, HCI_Inquiry_Cancel, Remote_Name_Request
- Connection management: HCI_Create_Connection, HCI_Disconnect, HCI_Accept_Connection
- L2CAP (Logical Link Control): signaling channel (CID 0x0001), connection-oriented channels
- SDP (Service Discovery Protocol): service search, attribute request

**Key Design Decisions**:
- USB transport only initially (most common for QEMU/development)
- L2CAP provides the foundation for all higher-level Bluetooth profiles
- No Bluetooth LE (BLE) in Phase 7.5 -- that's Phase 8 scope

**Dependencies**: xHCI driver (Sprint 3.1)

**Verification**: QEMU with USB Bluetooth adapter emulation, initialize HCI, scan for devices.

### Sprint 3.6: RTC Enhancement

**Category**: Hardware Drivers | **Estimated Lines**: ~100 (mostly extending existing)

**Implementation**:
- Modify `kernel/src/arch/x86_64/rtc.rs` (133 lines exist)
- Add alarm functionality: set alarm registers (0x01, 0x03, 0x05) for hours:minutes:seconds
- IRQ 8 handling for alarm interrupt (enable via CMOS register B, bit 5)
- Timezone support: configurable UTC offset stored in kernel parameter
- NTP integration point: `set_system_time(epoch_secs)` function for Sprint 5.4 NTP client
- Expose `/dev/rtc` device node in DevFS for user-space access

**Dependencies**: Existing RTC driver (exists), IRQ framework (exists)

**Verification**: Set alarm, verify interrupt fires at correct time. Read/write via /dev/rtc.

---

## Wave 4: Networking Foundations

**Duration**: 3-4 weeks | **Items**: 7 | **Priority**: High
**Rationale**: TCP improvements (congestion control, SACK) are essential for reliable networking. DNS resolver enables hostname resolution. These are prerequisites for Wave 5's TLS, SSH, and QUIC.

### Sprint 4.1: TCP Congestion Control (Reno)

**Category**: Networking | **Estimated Lines**: ~400

**Implementation**:
- Modify `kernel/src/net/tcp.rs`
- TCP Reno state machine: Slow Start, Congestion Avoidance, Fast Retransmit, Fast Recovery
- Variables: `cwnd` (congestion window), `ssthresh` (slow start threshold), `rtt_estimate`, `rto` (retransmission timeout)
- Slow Start: cwnd += MSS for each ACK (exponential growth until ssthresh)
- Congestion Avoidance: cwnd += MSS*MSS/cwnd for each ACK (linear growth)
- Fast Retransmit: on 3 duplicate ACKs, retransmit lost segment, ssthresh = cwnd/2, cwnd = ssthresh + 3*MSS
- RTO calculation: Jacobson's algorithm (SRTT, RTTVAR, RTO = SRTT + 4*RTTVAR)
- Timer: use kernel timer wheel (exists) for retransmission timeouts

**Verification**: TCP bulk transfer with packet loss simulation (QEMU network delay/loss), verify throughput recovery.

### Sprint 4.2: TCP Congestion Control (Cubic)

**Category**: Networking | **Estimated Lines**: ~300

**Implementation**:
- Extend TCP congestion module from Sprint 4.1
- CUBIC algorithm: cwnd = C * (t - K)^3 + W_max (cubic function of time since last loss)
- Parameters: C = 0.4, beta = 0.7 (multiplicative decrease factor)
- K = cubic_root(W_max * (1-beta) / C)
- TCP-friendly region: fall back to Reno behavior when Reno would be faster
- Hystart++: delayed ACK-based slow start exit for faster convergence
- Configurable: kernel parameter to select Reno or Cubic (default: Cubic)

**Key Design Decisions**:
- Fixed-point arithmetic for cubic root (no floating point in kernel)
- Cubic is the modern default (Linux default since 2.6.19)

**Verification**: Compare throughput between Reno and Cubic under varying loss rates.

### Sprint 4.3: TCP Selective Acknowledgment (SACK)

**Category**: Networking | **Estimated Lines**: ~350

**Implementation**:
- Modify `kernel/src/net/tcp.rs`
- SACK option negotiation during handshake (TCP option kind 4 for SACK-permitted, kind 5 for SACK blocks)
- SACK block tracking: up to 4 SACK blocks per ACK (RFC 2018)
- Sender-side: scoreboard tracking which segments have been SACKed
- Selective retransmission: only retransmit segments not covered by SACK blocks
- Integration with congestion control: SACK-based loss detection (RFC 6675)

**Verification**: TCP transfer with selective packet drops, verify only lost segments retransmitted (not entire window).

### Sprint 4.4: DNS Resolver

**Category**: Networking | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/net/dns.rs`
- DNS message format: header, question, answer, authority, additional sections
- Query types: A (IPv4), AAAA (IPv6), CNAME, MX, TXT, PTR, SRV
- Recursive query: send to configured nameserver (from `/etc/resolv.conf` or DHCP)
- Response parsing: decompress DNS name labels (pointer compression)
- Caching: `BTreeMap<(String, QueryType), (DnsRecord, u64_expiry)>` with TTL-based expiration
- `/etc/resolv.conf` parsing: `nameserver`, `domain`, `search` directives
- `/etc/hosts` file support: static hostname-to-IP mapping (checked before DNS query)
- UDP transport on port 53, with TCP fallback for responses > 512 bytes

**Key Design Decisions**:
- Cache up to 256 entries (fixed-size, LRU eviction when full)
- Timeout: 5 seconds per query, up to 3 retries
- Thread-safe: `GlobalState<Mutex<DnsCache>>`

**Verification**: Resolve `example.com`, verify A record response. Test caching (second query returns from cache).

### Sprint 4.5: VLAN (802.1Q) Tagging

**Category**: Networking | **Estimated Lines**: ~250

**Implementation**:
- Modify `kernel/src/net/ethernet.rs` (or new `net/vlan.rs`)
- 802.1Q tag insertion: 4-byte VLAN tag (TPID=0x8100, PCP, DEI, VID) after Ethernet src MAC
- VLAN interface creation: `vlan add eth0 100` (create eth0.100 with VLAN ID 100)
- Tag stripping on receive: detect 0x8100 EtherType, extract VID, deliver to correct VLAN interface
- Tag insertion on transmit: add VLAN tag before sending on physical interface
- Trunk port: accept multiple VLAN IDs on one physical interface
- Access port: single VLAN, auto-tag untagged frames

**Verification**: Create VLAN interface, send tagged frames, verify VLAN isolation.

### Sprint 4.6: Multicast Group Management (IGMP/MLD)

**Category**: Networking | **Estimated Lines**: ~300

**Implementation**:
- New file: `kernel/src/net/multicast.rs`
- IGMPv2 (IPv4): Membership Report, Leave Group, Query messages
- MLDv2 (IPv6): equivalent protocol for IPv6 multicast
- Group membership tracking: per-interface set of joined multicast groups
- Periodic membership reports (every 125 seconds default)
- Integration with socket API: `setsockopt(IP_ADD_MEMBERSHIP)`, `setsockopt(IP_DROP_MEMBERSHIP)`
- Ethernet multicast address mapping: IP multicast -> Ethernet multicast MAC (01:00:5E:xx:xx:xx)

**Verification**: Join multicast group, send/receive multicast packets, verify leave behavior.

### Sprint 4.7: NIC Bonding / Link Aggregation

**Category**: Networking | **Estimated Lines**: ~350

**Implementation**:
- New file: `kernel/src/net/bonding.rs`
- Bond interface creation: `bond create bond0 eth0 eth1`
- Modes: Active-Backup (mode 1), Round-Robin (mode 0)
- Active-Backup: monitor link status, failover on primary link failure
- Round-Robin: distribute packets across all active slaves
- ARP monitoring: periodic ARP probe to detect link failures (beyond link state)
- MAC address management: bond inherits first slave's MAC

**Verification**: Create bond with two VirtIO-Net interfaces, verify failover behavior.

---

## Wave 5: Crypto & Protocol Layer

**Duration**: 5-7 weeks | **Items**: 6 | **Priority**: High
**Rationale**: TLS 1.3 is the foundation for all secure networking (SSH, QUIC, HTTPS). SSH enables remote management. HTTP client enables package downloads. These are the most complex items in Phase 7.5.

### Sprint 5.1: TLS 1.3 Implementation

**Category**: Networking Protocols | **Estimated Lines**: ~1500

**Implementation**:
- New directory: `kernel/src/net/tls/`
- Files: `mod.rs`, `handshake.rs`, `record.rs`, `crypto.rs`, `certificate.rs`
- Record layer: TLS record framing (type, version, length, fragment), max 16KB payload
- Handshake state machine: ClientHello -> ServerHello -> EncryptedExtensions -> Certificate -> CertificateVerify -> Finished
- Key exchange: X25519 ECDHE (exists in `crypto/asymmetric.rs`)
- AEAD encryption: ChaCha20-Poly1305 (exists) and AES-128-GCM (new, ~200 lines)
- Key derivation: HKDF-SHA256 (HKDF-Extract + HKDF-Expand)
- Certificate validation: X.509 parsing (DER/ASN.1), signature verification (Ed25519, RSA)
- Session resumption: PSK-based 0-RTT (optional, for performance)
- Alert protocol: close_notify, unexpected_message, bad_record_mac, etc.

**Key Design Decisions**:
- TLS 1.3 only (no TLS 1.2 backward compat -- simplifies implementation significantly)
- ChaCha20-Poly1305 as primary cipher suite (already implemented, fast in software)
- AES-GCM as secondary (needed for interoperability; implement with carry-less multiply if AES-NI available)
- No client certificate support initially
- Self-signed certificates accepted with explicit trust (no full PKI chain validation in Phase 7.5)

**Dependencies**: X25519 (exists), ChaCha20-Poly1305 (exists), SHA-256 (exists)

**Verification**: TLS handshake with test server, encrypted data exchange, verify with Wireshark.

### Sprint 5.2: SSH Server

**Category**: Networking Protocols | **Estimated Lines**: ~1200

**Implementation**:
- New directory: `kernel/src/net/ssh/`
- Files: `mod.rs`, `transport.rs`, `auth.rs`, `channel.rs`, `session.rs`
- Transport layer: binary packet protocol, key exchange (curve25519-sha256)
- Host key: Ed25519 (exists), generated at boot or loaded from `/etc/ssh/`
- Authentication: password (via `security/auth.rs` PBKDF2) and publickey (Ed25519 verify)
- Channel multiplexing: session channels, direct-tcpip, forwarded-tcpip
- Shell session: allocate PTY (exists), spawn shell process, forward I/O
- TCP listener on port 22
- Supported algorithms: chacha20-poly1305@openssh.com, curve25519-sha256, ssh-ed25519

**Key Design Decisions**:
- Minimal SSH implementation: one cipher suite, one key exchange, one host key type
- No SFTP subsystem in Phase 7.5 (deferred)
- No SSH agent forwarding
- Connection limit: max 4 concurrent sessions

**Dependencies**: TLS crypto primitives (Sprint 5.1 shares infrastructure), PTY (exists), Ed25519 (exists)

**Verification**: Connect with `ssh` client, authenticate, execute commands, verify encrypted transport.

### Sprint 5.3: HTTP/1.1 Client Library

**Category**: Networking | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/net/http.rs`
- Request construction: method, URL parsing, headers, body
- Response parsing: status line, headers, body with Content-Length or chunked transfer encoding
- Chunked transfer decoding: parse chunk sizes, assemble body
- Keep-alive: reuse TCP connection for multiple requests (Connection: keep-alive)
- Redirect following: 301, 302, 307, 308 with max 5 redirects
- HTTPS support: wrap TCP socket in TLS (Sprint 5.1)
- Basic authentication: `Authorization: Basic base64(user:pass)`

**Dependencies**: TCP (exists), DNS (Sprint 4.4), TLS (Sprint 5.1)

**Verification**: HTTP GET to known endpoint, parse response. HTTPS connection. Chunked transfer.

### Sprint 5.4: NTP Client

**Category**: Networking Protocols | **Estimated Lines**: ~300

**Implementation**:
- New file: `kernel/src/net/ntp.rs`
- NTPv4 packet format: 48-byte UDP packets on port 123
- Timestamp format: 64-bit NTP timestamp (seconds since 1900-01-01 + 32-bit fraction)
- Client mode: send request, receive response, calculate offset and delay
- Clock discipline: Marzullo's algorithm for multiple server consensus
- Offset calculation: `offset = ((T2-T1) + (T3-T4)) / 2`, `delay = (T4-T1) - (T3-T2)`
- Integration with RTC: `rtc::set_system_time()` for initial sync, gradual adjtime for drift
- Periodic sync: every 1024 seconds (default NTP poll interval)
- Stratum tracking: report local stratum = server_stratum + 1

**Dependencies**: UDP socket (exists), DNS (Sprint 4.4), RTC (Sprint 3.6)

**Verification**: Query public NTP server (or QEMU user-net NTP), verify time synchronized.

### Sprint 5.5: QUIC Protocol

**Category**: Networking Protocols | **Estimated Lines**: ~1000

**Implementation**:
- New directory: `kernel/src/net/quic/`
- Files: `mod.rs`, `connection.rs`, `stream.rs`, `frame.rs`, `crypto.rs`
- UDP-based transport with TLS 1.3 integrated handshake
- Connection establishment: Initial packet, Handshake packet, 1-RTT packet types
- Stream multiplexing: bidirectional and unidirectional streams, stream IDs
- Frame types: STREAM, ACK, CRYPTO, PADDING, PING, CONNECTION_CLOSE, MAX_DATA, MAX_STREAM_DATA
- Flow control: per-stream and connection-level credit-based
- Loss detection: packet number-based (not sequence number), ACK-based
- Congestion control: reuse Cubic from Sprint 4.2 (QUIC uses same algorithms)
- Connection migration: allow source IP/port change with connection ID continuity

**Key Design Decisions**:
- QUIC v1 (RFC 9000) only
- Reuse TLS 1.3 crypto from Sprint 5.1 (QUIC integrates TLS differently but uses same primitives)
- No HTTP/3 in Phase 7.5 (QUIC transport layer only, HTTP/3 is Phase 8)
- Max 16 concurrent streams per connection

**Dependencies**: UDP (exists), TLS 1.3 crypto (Sprint 5.1)

**Verification**: QUIC handshake, stream creation, bidirectional data transfer, connection close.

### Sprint 5.6: WireGuard VPN

**Category**: Networking Protocols | **Estimated Lines**: ~700

**Implementation**:
- New file: `kernel/src/net/wireguard.rs`
- Noise IK handshake protocol: initiator/responder with X25519 DH
- Transport encryption: ChaCha20-Poly1305 with rotating nonce counter
- Peer management: static public key, optional pre-shared key, allowed IPs
- Virtual interface: `wg0` network interface with IP address
- Packet encapsulation: IP packet -> WireGuard header -> UDP -> outer IP
- Timer-based handshake renewal (every 2 minutes) and keepalive (every 25 seconds)
- Configuration: `wg set wg0 private-key /etc/wireguard/private.key peer <pubkey> endpoint <ip>:<port> allowed-ips 0.0.0.0/0`

**Key Design Decisions**:
- WireGuard is elegantly simple (~4000 lines in Linux) -- well-suited for kernel implementation
- Reuse X25519 (exists), ChaCha20-Poly1305 (exists), BLAKE2s (implement ~200 lines)
- CookieReply for DDoS mitigation (Mac1/Mac2 in message headers)

**Dependencies**: X25519 (exists), ChaCha20-Poly1305 (exists), UDP (exists)

**Verification**: Configure WireGuard tunnel between two QEMU instances, verify encrypted traffic, test keepalive.

### Sprint 5.7: mDNS/DNS-SD

**Category**: Networking Protocols | **Estimated Lines**: ~350

**Implementation**:
- New file: `kernel/src/net/mdns.rs`
- Multicast DNS: send/receive on 224.0.0.251:5353 (IPv4) / ff02::fb:5353 (IPv6)
- .local domain resolution: query for `hostname.local` -> respond with local IP
- DNS-SD (Service Discovery): `_services._dns-sd._udp.local` browsing
- Service registration: `_http._tcp.local`, `_ssh._tcp.local` with SRV and TXT records
- Conflict resolution: probe (3 queries) before claiming name
- Cache: remember discovered services with TTL expiry

**Dependencies**: UDP multicast (Sprint 4.6), DNS message format (Sprint 4.4)

**Verification**: Register service, discover from another instance, verify .local resolution.

---

## Wave 6: Desktop & Userland

**Duration**: 4-5 weeks | **Items**: 12 | **Priority**: Medium-High
**Rationale**: Desktop enhancements (clipboard, fonts, themes) improve daily usability. Userland features (io_uring, ptrace, user management) enable real application support.

### Sprint 6.1: Clipboard Protocol

**Category**: Desktop | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/desktop/clipboard.rs`
- Wayland `wl_data_device` protocol: selection source, offer, receive
- MIME type negotiation: `text/plain`, `text/plain;charset=utf-8`, `text/html`, `image/png`
- Clipboard storage: kernel-side buffer (max 1MB) for current selection
- Copy: app writes data to compositor via `wl_data_source`
- Paste: app reads data from compositor via `wl_data_offer`
- Primary selection (X11 middle-click paste) via `zwp_primary_selection_v1`
- Integration with text editor and terminal (`Ctrl+C`/`Ctrl+V` handling)

**Verification**: Copy text in terminal, paste in text editor, verify MIME type negotiation.

### Sprint 6.2: Drag-and-Drop

**Category**: Desktop | **Estimated Lines**: ~350

**Implementation**:
- Extend `kernel/src/desktop/clipboard.rs` (or new `dnd.rs`)
- Wayland `wl_data_device` DnD protocol: start_drag, enter, motion, leave, drop
- Source app: initiate drag with button press + motion, provide data offer
- Target app: receive enter/motion/leave events, accept/reject drop
- Visual feedback: cursor change to drag icon, highlight drop target
- MIME negotiation: same as clipboard (text, files, images)
- File manager integration: drag files between directories

**Dependencies**: Clipboard protocol (Sprint 6.1)

**Verification**: Drag file in file manager, drop to new location, verify move/copy.

### Sprint 6.3: Font Rendering (TrueType/OpenType)

**Category**: Desktop | **Estimated Lines**: ~1200

**Implementation**:
- New directory: `kernel/src/desktop/font_render/`
- Files: `mod.rs`, `ttf_parser.rs`, `rasterizer.rs`, `cache.rs`
- TrueType parsing: `cmap` (character mapping), `glyf` (glyph outlines), `head`, `hhea`, `hmtx`, `loca` tables
- Quadratic Bezier rasterization: outline -> bitmap at requested size
- Hinting: basic grid-fitting for small sizes (interpret TrueType instructions or use auto-hinting)
- Subpixel rendering: RGB subpixel order for LCD displays (3x horizontal resolution)
- Glyph cache: `BTreeMap<(GlyphID, Size), Bitmap>` with LRU eviction (256 entries)
- Font file loading: from BlockFS `/usr/share/fonts/`
- Fallback: keep existing `font8x16` bitmap font for system/emergency rendering

**Key Design Decisions**:
- Start with TrueType outlines only (no CFF/PostScript -- simpler parser)
- Auto-hinting preferred over full TrueType bytecode interpreter (complex to implement correctly)
- Subpixel rendering optional (configurable in settings)

**Verification**: Render TrueType font at multiple sizes, compare visual quality with bitmap font.

### Sprint 6.4: CJK Unicode Support

**Category**: Desktop | **Estimated Lines**: ~500

**Implementation**:
- Modify `kernel/src/desktop/font_render/` and text rendering paths
- Wide character handling: Unicode codepoint width detection (East Asian Width property)
- Double-width cell rendering: CJK characters occupy 2 columns in terminal and text editor
- Character map: support Unicode BMP (U+0000 to U+FFFF) including CJK Unified Ideographs (U+4E00-U+9FFF)
- Input method framework: composing buffer, candidate window, commit (basic framework, not full IM)
- Bidirectional text: basic LTR rendering (full BiDi algorithm deferred to Phase 8)

**Dependencies**: Font rendering (Sprint 6.3)

**Verification**: Display CJK characters in terminal, verify double-width rendering, input method compose.

### Sprint 6.5: Theme Engine

**Category**: Desktop | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/desktop/theme.rs`
- Theme definition: color scheme (background, foreground, accent, window, panel, button states)
- Built-in themes: "Dark" (current), "Light", "Nord", "Solarized"
- Settings integration: theme selection in Settings app panel
- Runtime theme switching: notify all desktop modules of theme change
- Configuration persistence: save selected theme to `/etc/veridian/theme.conf`
- Widget rendering: apply theme colors to window decorations, panel, buttons, menus
- Icon theme support: `/usr/share/icons/<theme>/` directory structure

**Verification**: Switch between themes in Settings, verify all UI elements update.

### Sprint 6.6: Global Keyboard Shortcuts

**Category**: Desktop | **Estimated Lines**: ~300

**Implementation**:
- New file: `kernel/src/desktop/shortcuts.rs`
- Shortcut registry: `BTreeMap<KeyCombo, Action>` where `KeyCombo = (modifiers, keycode)`
- Default shortcuts: Alt+Tab (app switch), Ctrl+Alt+Del (logout), Super (launcher), Print (screenshot)
- Configurable: `/etc/veridian/shortcuts.conf` file parsing
- Settings panel: keyboard shortcuts configuration UI
- Conflict detection: warn when new shortcut conflicts with existing
- Per-app shortcut bypass: allow apps to receive shortcuts they need (e.g., terminal needs Ctrl+C)

**Dependencies**: Keyboard modifier tracking (exists)

**Verification**: Configure custom shortcut, verify it triggers correct action, test conflict detection.

### Sprint 6.7: io_uring Async I/O

**Category**: Shell/Userland | **Estimated Lines**: ~800

**Implementation**:
- New file: `kernel/src/io/uring.rs`
- Submission Queue (SQ) and Completion Queue (CQ): shared memory ring buffers between kernel and user-space
- SQE (Submission Queue Entry): opcode, fd, offset, addr, len, flags
- CQE (Completion Queue Entry): user_data, res, flags
- Supported operations: READV, WRITEV, FSYNC, POLL_ADD, ACCEPT, CONNECT, SEND, RECV, OPENAT, CLOSE
- Syscalls: `io_uring_setup(entries, params)`, `io_uring_enter(fd, to_submit, min_complete, flags)`, `io_uring_register(fd, opcode, arg)`
- Batched submission: process multiple SQEs in single syscall entry
- Completion polling: both interrupt-driven and polling modes

**Key Design Decisions**:
- Fixed-size ring (256 entries initially, configurable)
- Kernel-side SQE processing in syscall context (not IRQ-driven)
- SQ/CQ in shared memory page (user-space maps via mmap)
- Start with subset of operations (READV, WRITEV, FSYNC, POLL_ADD)

**Verification**: User-space program submits I/O via io_uring, verify completions, benchmark vs synchronous I/O.

### Sprint 6.8: ptrace System Call

**Category**: Shell/Userland | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/syscall/ptrace.rs`
- `ptrace(request, pid, addr, data)` syscall
- Requests: PTRACE_ATTACH, PTRACE_DETACH, PTRACE_PEEKDATA, PTRACE_POKEDATA, PTRACE_PEEKUSER, PTRACE_POKEUSER
- Single-step: PTRACE_SINGLESTEP (set TF flag in EFLAGS for x86_64)
- Breakpoints: PTRACE_CONT (continue), PTRACE_SYSCALL (stop at syscall entry/exit)
- Register access: PTRACE_GETREGS, PTRACE_SETREGS (read/write tracee's register state)
- Signal delivery: tracer can intercept and modify signals to tracee
- Process state: tracee stops on attach, signal delivery, syscall entry/exit

**Key Design Decisions**:
- Minimal ptrace for debugger support (GDB remote stub uses this)
- Only x86_64 initially (AArch64/RISC-V deferred)
- Security: tracer must be parent or have CAP_SYS_PTRACE

**Dependencies**: Signal infrastructure (exists), process management (exists)

**Verification**: Attach to running process, read memory, set breakpoint, single-step, detach.

### Sprint 6.9: Core Dump Generation

**Category**: Shell/Userland | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/process/coredump.rs`
- ELF core file format: ET_CORE type, PT_NOTE + PT_LOAD segments
- NT_PRSTATUS note: register state at crash time
- NT_PRPSINFO note: process name, PID, parent PID
- Memory segments: dump all mapped user-space pages as PT_LOAD segments
- Signal handler: on SIGSEGV/SIGBUS/SIGFPE, generate core dump before termination
- Core file location: `/tmp/core.<pid>` (configurable via `/proc/sys/kernel/core_pattern`)
- Size limit: configurable max core file size (default 1MB)

**Dependencies**: ELF format knowledge (exists in bootstrap.rs), signal handling (exists)

**Verification**: Trigger segfault in user program, verify core dump generated, parse with `readelf -a core.pid`.

### Sprint 6.10: User and Group Management

**Category**: Shell/Userland | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/security/users.rs`
- `/etc/passwd` parsing: `username:x:uid:gid:gecos:home:shell`
- `/etc/group` parsing: `groupname:x:gid:member1,member2`
- `/etc/shadow` parsing: `username:password_hash:lastchanged:...`
- Syscalls: `getuid()`, `getgid()`, `setuid()`, `setgid()`, `getpwnam()`, `getgrnam()`
- Shell commands: `useradd`, `userdel`, `groupadd`, `groupdel`, `passwd`, `id`, `whoami`
- Per-process UID/GID tracking in PCB
- Permission checking: file access based on owner UID/GID and permission bits

**Dependencies**: VFS (exists), process management (exists)

**Verification**: Create user, login, verify file permissions enforced per UID/GID.

### Sprint 6.11: sudo/su Privilege Elevation

**Category**: Shell/Userland | **Estimated Lines**: ~350

**Implementation**:
- Modify `kernel/src/security/auth.rs` and new `security/privilege.rs`
- `su` command: switch user identity (setuid/setgid after password verification)
- `sudo` command: execute single command as root with password verification
- `/etc/sudoers` parsing: `username ALL=(ALL) ALL` format (simplified)
- PAM-style authentication flow: prompt for password, verify against `/etc/shadow`
- Audit logging: log all privilege elevation attempts
- Timeout: cached sudo authentication for 5 minutes

**Dependencies**: User management (Sprint 6.10), auth (exists)

**Verification**: Create non-root user, `su root` with password, `sudo ls /root`, verify audit log entries.

### Sprint 6.12: Crontab Scheduler

**Category**: Shell/Userland | **Estimated Lines**: ~350

**Implementation**:
- New file: `kernel/src/services/cron.rs`
- Crontab format: `minute hour day month weekday command`
- Per-user crontab files: `/var/spool/cron/<username>`
- System crontab: `/etc/crontab`
- Cron daemon: kernel service that checks schedule every 60 seconds
- Job execution: fork + exec with user's UID/GID
- Special strings: `@reboot`, `@hourly`, `@daily`, `@weekly`, `@monthly`
- Shell commands: `crontab -e` (edit), `crontab -l` (list), `crontab -r` (remove)

**Dependencies**: User management (Sprint 6.10), timer (exists), fork/exec (exists)

**Verification**: Install crontab entry, verify command executes at scheduled time.

---

## Wave 7: Audio/Video & GPU

**Duration**: 5-7 weeks | **Items**: 13 | **Priority**: Medium
**Rationale**: ALSA compatibility enables audio applications. Image decoders (PNG/JPEG/GIF) enable real image viewing. VirtIO GPU 3D and OpenGL ES 2.0 enable 3D rendering.

### Sprint 7.1: ALSA-Compatible User-Space API

**Category**: Audio | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/audio/alsa_compat.rs`
- PCM device interface: `/dev/snd/pcmC0D0p` (playback), `/dev/snd/pcmC0D0c` (capture)
- Operations: `snd_pcm_open()`, `snd_pcm_hw_params_set_*()`, `snd_pcm_writei()`, `snd_pcm_readi()`, `snd_pcm_close()`
- Hardware parameters: sample rate, channels, format (S16_LE, S32_LE, FLOAT_LE), buffer size, period size
- Mixer interface: `/dev/snd/controlC0` for volume control
- ioctl commands: SNDRV_PCM_IOCTL_HW_PARAMS, SNDRV_PCM_IOCTL_PREPARE, SNDRV_PCM_IOCTL_START
- Ring buffer: map shared memory between user-space and audio subsystem
- Integration with existing audio mixer (exists in `audio/mixer.rs`)

**Dependencies**: Audio subsystem (exists), device nodes (exists)

**Verification**: User-space program opens PCM device, configures parameters, writes audio data, verify playback.

### Sprint 7.2: Audio Recording/Capture Pipeline

**Category**: Audio | **Estimated Lines**: ~350

**Implementation**:
- Modify `kernel/src/audio/pipeline.rs` and `audio/virtio_sound.rs`
- Capture stream: VirtIO-Sound PCM capture configuration
- Ring buffer for capture: DMA buffer for incoming audio data
- Capture API: `AudioClient::start_capture()`, `read_capture_data(buffer)`
- Format conversion: input sample format -> requested format (S16_LE normalization)
- Loopback device: route playback output to capture input (for testing/monitoring)

**Dependencies**: Audio subsystem (exists), VirtIO-Sound (exists)

**Verification**: Start capture, play audio, verify captured data matches output.

### Sprint 7.3: USB Audio Class Driver

**Category**: Audio | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/drivers/usb/audio.rs`
- USB Audio Class 1.0/2.0: class 0x01 (Audio), subclass 0x01 (AudioControl), 0x02 (AudioStreaming)
- Audio control interface: terminal descriptors, mixer unit, feature unit (volume, mute)
- Audio streaming interface: alternate settings for different sample rates/formats
- Isochronous transfers: periodic audio data transfer via xHCI isochronous endpoints
- PCM device registration: create `/dev/snd/pcmC1D0p` for USB audio device
- Hot-plug support: detect USB audio device insertion/removal

**Dependencies**: xHCI driver (Sprint 3.1), audio subsystem (exists)

**Verification**: QEMU with USB audio device, play audio through USB device, verify output.

### Sprint 7.4: Real-Time Audio Scheduling

**Category**: Audio | **Estimated Lines**: ~200

**Implementation**:
- Integrate deadline scheduling (Sprint 2.1) with audio pipeline
- Audio thread: SCHED_DEADLINE with runtime=2ms, deadline=10ms, period=10ms
- Priority boost: audio completion threads get deadline priority automatically
- Buffer underrun prevention: pre-buffer 2 periods before starting playback
- Latency target: <20ms round-trip (record -> process -> playback)

**Dependencies**: Deadline scheduling (Sprint 2.1), audio pipeline (exists)

**Verification**: Play audio under heavy CPU load, verify no underruns with deadline scheduling enabled.

### Sprint 7.5: OGG Vorbis Decoder

**Category**: Audio | **Estimated Lines**: ~800

**Implementation**:
- New file: `kernel/src/audio/vorbis.rs`
- OGG container: page parsing (sync pattern, stream serial, page sequence)
- Vorbis codec: identification header, comment header, setup header
- MDCT (Modified Discrete Cosine Transform): inverse transform for frequency->time domain
- Floor/residue decoding: floor type 1 (common), residue type 0/1/2
- Windowing: overlap-add with Vorbis window function
- Output: PCM samples at native sample rate
- Fixed-point implementation: avoid floating-point (use 16.16 fixed-point for MDCT)

**Key Design Decisions**:
- OGG Vorbis chosen because it's patent-free and moderately complex
- Fixed-point MDCT is challenging but feasible (~400 lines)
- Decode quality: sufficient for 44.1kHz/16-bit stereo

**Verification**: Decode OGG Vorbis file, play through audio pipeline, verify audible output.

### Sprint 7.6: MP3 Decoder

**Category**: Audio | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/audio/mp3.rs`
- Frame sync: find 0xFFE0 sync word, parse header (bitrate, sample rate, padding, channel mode)
- Huffman decoding: decode quantized spectral data using MP3 Huffman tables
- Dequantization: apply scale factors and global gain
- Stereo processing: joint stereo (MS and intensity stereo)
- IMDCT: 36-point and 12-point inverse MDCT
- Synthesis polyphase filterbank: 32-band -> PCM output
- Layer III only (most common MP3 format)

**Key Design Decisions**:
- MP3 patents have expired (as of 2017), so no licensing concerns
- Fixed-point arithmetic throughout (same approach as Vorbis)
- Support MPEG-1 Layer III only (no Layer I/II)

**Verification**: Decode MP3 file, play through audio pipeline, verify correct output.

### Sprint 7.7: HDMI Audio Output

**Category**: Audio | **Estimated Lines**: ~300

**Implementation**:
- Modify `kernel/src/drivers/gpu.rs` and `audio/mod.rs`
- HDMI audio: ELD (EDID-Like Data) parsing from GPU driver for audio capabilities
- Audio infoframe: configure HDMI audio parameters (channels, sample rate, bit depth)
- HDA (High Definition Audio) codec interface: configure HDMI audio widget
- VirtIO GPU + audio bridge: route audio data to GPU's HDMI output
- PCM device: register as separate audio device `/dev/snd/pcmC2D0p`

**Dependencies**: GPU driver (exists), audio pipeline (exists)

**Verification**: Enable HDMI audio output, play audio, verify output on HDMI display.

### Sprint 7.8: PNG Decoder

**Category**: Video | **Estimated Lines**: ~700

**Implementation**:
- New file: `kernel/src/desktop/png.rs`
- PNG structure: signature (8 bytes), IHDR, IDAT, IEND chunks
- DEFLATE decompression: inflate algorithm (Huffman + LZ77 back-references)
- Filter reconstruction: None, Sub, Up, Average, Paeth filters per scanline
- Color types: grayscale (0), truecolor (2), indexed (3), grayscale+alpha (4), truecolor+alpha (6)
- Interlacing: Adam7 interlace support (7 passes with different start/step)
- Output: RGBA8888 pixel buffer for compositor
- Integration with image viewer and desktop (icons, wallpapers)

**Key Design Decisions**:
- DEFLATE decompression is ~400 lines (the bulk of the work)
- Support all standard PNG color types and bit depths
- No animated PNG (APNG) in Phase 7.5

**Verification**: Decode PNG test images (all color types), display in image viewer, compare with reference.

### Sprint 7.9: JPEG Decoder

**Category**: Video | **Estimated Lines**: ~800

**Implementation**:
- New file: `kernel/src/desktop/jpeg.rs`
- JPEG structure: SOI, APP0 (JFIF), DQT, DHT, SOF0, SOS, EOI markers
- Baseline DCT: 8x8 block-based discrete cosine transform
- Huffman decoding: decode DC and AC coefficients using DHT tables
- Dequantization: apply DQT tables to DCT coefficients
- IDCT: 8x8 inverse DCT (can use AAN algorithm for speed, or simple matrix multiply)
- Color space conversion: YCbCr -> RGB (BT.601 coefficients, already implemented for video)
- Chroma subsampling: 4:4:4, 4:2:2, 4:2:0
- Output: RGB888 pixel buffer

**Key Design Decisions**:
- Baseline JPEG only (no progressive, no arithmetic coding)
- Fixed-point IDCT (8-bit precision sufficient for JPEG quality)
- Reuse BT.601 YUV->RGB from existing video framework

**Verification**: Decode JPEG test images, display in image viewer, verify visual quality.

### Sprint 7.10: GIF Decoder

**Category**: Video | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/desktop/gif.rs`
- GIF87a/GIF89a: header, logical screen descriptor, global color table
- LZW decompression: variable-length code dictionary (initial size from min code size)
- Image descriptor: per-frame position, size, local color table, interlacing
- Animation: frame delay (Graphic Control Extension), disposal method
- Transparency: transparent color index from Graphic Control Extension
- Frame sequencing: decode frames in order, apply disposal, composite
- Output: RGBA8888 per frame

**Verification**: Decode static and animated GIFs, display in image viewer, verify animation playback.

### Sprint 7.11: AVI Container Parser

**Category**: Video | **Estimated Lines**: ~300

**Implementation**:
- New file: `kernel/src/audio/avi.rs`
- RIFF/AVI structure: RIFF header, hdrl (header list), movi (data), idx1 (index)
- Stream parsing: vids (video stream header), auds (audio stream header)
- Codec identification: FourCC codes (e.g., MJPG, H264, DIVX for video; PCM for audio)
- Demuxing: separate audio and video chunks from interleaved movi data
- Index: use idx1 for random access / seeking
- Integration with media player: feed video frames to decoder, audio to mixer

**Dependencies**: Media player (exists in `desktop/`)

**Verification**: Parse AVI file with MJPEG video + PCM audio, demux streams.

### Sprint 7.12: VirtIO GPU 3D (Virgl)

**Category**: GPU | **Estimated Lines**: ~800

**Implementation**:
- Modify `kernel/src/drivers/gpu.rs` (VirtIO GPU driver)
- VirtIO GPU 3D mode: `VIRTIO_GPU_CMD_CTX_CREATE`, `VIRTIO_GPU_CMD_CTX_DESTROY`
- Virgl protocol: create 3D context, submit command buffers, create 3D resources
- Resource types: PIPE_TEXTURE_2D, PIPE_BUFFER (vertex/index buffers)
- Command submission: VIRGL_CCMD_CREATE_OBJECT, VIRGL_CCMD_DRAW_VBO, VIRGL_CCMD_SET_FRAMEBUFFER_STATE
- Framebuffer object: render-to-texture for offscreen rendering
- Fence synchronization: wait for GPU command completion

**Dependencies**: VirtIO GPU 2D (exists)

**Verification**: Create 3D context, render triangle, read back framebuffer, verify pixel data.

### Sprint 7.13: OpenGL ES 2.0 Software Rasterizer

**Category**: GPU | **Estimated Lines**: ~1500

**Implementation**:
- New directory: `kernel/src/graphics/gles2/`
- Files: `mod.rs`, `context.rs`, `shader.rs`, `rasterizer.rs`, `texture.rs`, `framebuffer.rs`
- Vertex processing: model-view-projection transform, attribute interpolation
- Fragment processing: texture sampling, per-fragment operations
- Rasterization: triangle rasterization with edge equations (scanline or tile-based)
- Texture sampling: nearest and bilinear filtering, wrap modes
- Blending: standard blend equations (SRC_ALPHA, ONE_MINUS_SRC_ALPHA, etc.)
- Depth buffer: 16-bit or 24-bit depth testing
- GLSL ES shader compilation: simplified shader parser (vertex/fragment shaders as fixed functions)

**Key Design Decisions**:
- Software rasterizer (no hardware 3D acceleration in Phase 7.5)
- Subset of GLES 2.0: enough for simple 3D rendering (triangles, textures, transforms)
- Fixed-function pipeline initially, programmable shaders as stretch goal
- Target: 10-30 FPS for simple scenes on KVM-accelerated QEMU

**Dependencies**: VirtIO GPU 3D (Sprint 7.12) for hardware path, software fallback otherwise

**Verification**: Render rotating textured cube, verify correct perspective and texture mapping.

### Sprint 7.14: Additional GPU Features

**Category**: GPU | **Estimated Lines**: ~400

**Implementation**:
- **VSync/Page Flip**: Double buffering with vblank synchronization, `drmModePageFlip()` equivalent
- **GEM/TTM Buffer Management**: GPU memory object allocation, mapping, cache management
- **Hardware Cursor**: GPU cursor plane for zero-copy cursor rendering, position updates via MMIO
- **DRM KMS Interface**: Mode setting, CRTC/encoder/connector abstraction for user-space display servers

**Verification**: Enable VSync, verify no tearing. Move hardware cursor, verify smooth tracking.

---

## Wave 8: Hypervisor & Containers

**Duration**: 4-6 weeks | **Items**: 13 | **Priority**: Medium
**Rationale**: Nested virtualization and container enhancements enable enterprise deployment. OCI compliance enables Docker compatibility. Cgroups enable resource management.

### Sprint 8.1: Nested Virtualization (L2 VMCS Shadowing)

**Category**: Hypervisor | **Estimated Lines**: ~800

**Implementation**:
- Modify `kernel/src/virt/vmx.rs`
- L2 VMCS: shadow VMCS for guest hypervisor's virtual machines
- VMCS shadowing: intercept VMREAD/VMWRITE from L1 guest, redirect to shadow VMCS
- VM-exit merging: L2 exit -> decide if L0 or L1 handles it
- Page table composition: L0 EPT maps L1 guest-physical -> host-physical; L1 EPT maps L2 -> L1
- VPID management: separate VPID namespace for L2 guests
- VMCS field forwarding: which fields are passed through vs intercepted

**Key Design Decisions**:
- Start with VMCS shadowing (simpler than full VMCS emulation)
- Require EPT (exists) for nested page table composition
- Limited to 2 levels of nesting (L0 -> L1 -> L2)

**Dependencies**: VMX/VMCS (exists), EPT (exists)

**Verification**: Run VeridianOS inside VeridianOS (L1), verify L2 guest boots.

### Sprint 8.2: Guest SMP Support (Multi-vCPU)

**Category**: Hypervisor | **Estimated Lines**: ~500

**Implementation**:
- Modify `kernel/src/virt/vmx.rs` and new `virt/vcpu.rs`
- Multiple VMCS instances per VM: one per vCPU
- vCPU scheduling: map vCPUs to physical CPUs, schedule via kernel scheduler
- INIT/SIPI emulation: vCPU startup protocol for x86 SMP
- IPI delivery: inter-vCPU interrupts via virtual LAPIC

**Dependencies**: VMX (exists), scheduler (exists)

**Verification**: Boot SMP guest (2 vCPUs), verify both vCPUs execute.

### Sprint 8.3: Virtual LAPIC Emulation

**Category**: Hypervisor | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/virt/lapic.rs`
- MMIO trap: intercept guest accesses to 0xFEE00000 LAPIC page
- Register emulation: APIC ID, TPR, EOI, SVR, ISR, TMR, IRR, ICR, LVT, timer
- Timer emulation: periodic and one-shot modes via host APIC timer
- IPI delivery: virtual ICR write triggers vCPU interrupt injection
- EOI broadcast: clear in-service bit, check for pending interrupts

**Dependencies**: VMX (exists), multi-vCPU (Sprint 8.2)

**Verification**: Guest uses LAPIC timer for scheduling, verify timer interrupts delivered.

### Sprint 8.4: VirtIO Device Passthrough

**Category**: Hypervisor | **Estimated Lines**: ~500

**Implementation**:
- Modify `kernel/src/virt/mod.rs`
- VirtIO MMIO device presentation: expose VirtIO devices to guest at configured MMIO addresses
- Virtqueue forwarding: guest writes to virtqueue -> host processes request -> inject completion
- Device types: virtio-blk (block device), virtio-net (network)
- Guest memory translation: GPA -> HPA via EPT for DMA operations

**Dependencies**: VMX (exists), EPT (exists), VirtIO drivers (exist)

**Verification**: Guest boots with VirtIO block device, reads/writes data.

### Sprint 8.5: VM Snapshots

**Category**: Hypervisor | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/virt/snapshot.rs`
- VMCS serialization: save all VMCS fields to binary buffer
- Memory snapshot: save guest physical memory pages
- Device state: save virtual device registers and buffers
- Restore: load VMCS + memory + device state, resume execution
- Storage format: custom binary format to BlockFS file

**Dependencies**: VMX (exists), EPT (exists)

**Verification**: Run guest, take snapshot, modify state, restore, verify execution resumes from snapshot point.

### Sprint 8.6: Live Migration (Stretch Goal)

**Category**: Hypervisor | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/virt/migration.rs`
- Pre-copy: iteratively send dirty memory pages while VM runs
- Stop-and-copy: pause VM, send remaining dirty pages, VMCS state, device state
- Network transport: TCP stream to destination host
- Convergence: track dirty page rate vs network bandwidth, switch to stop-and-copy when convergent
- Downtime target: <100ms

**Dependencies**: VM snapshots (Sprint 8.5), TCP (exists)

**Verification**: Migrate running VM between two VeridianOS instances, verify minimal downtime.

### Sprint 8.7: OCI Runtime Specification

**Category**: Containers | **Estimated Lines**: ~600

**Implementation**:
- New file: `kernel/src/virt/oci.rs`
- `config.json` parsing: OCI runtime spec v1.0 (process, root, mounts, linux namespaces, hooks)
- Container lifecycle: create, start, kill, delete states
- Rootfs setup: bind-mount container root filesystem
- Lifecycle hooks: prestart, poststart, poststop hook execution
- `veridian-runtime` CLI interface: `create <id> <bundle>`, `start <id>`, `kill <id>`, `delete <id>`

**Dependencies**: Namespaces (exist), process management (exists)

**Verification**: Create OCI bundle, run container, verify isolated execution.

### Sprint 8.8: Container Image Format

**Category**: Containers | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/virt/image.rs`
- OCI image format: manifest, config, layer tarballs
- Layer extraction: untar layers in order, apply to rootfs
- Layer caching: content-addressable storage by SHA256 digest
- Overlay composition: combine layers using overlay filesystem (Sprint 8.11)

**Dependencies**: OCI runtime (Sprint 8.7), overlay FS (Sprint 8.11)

**Verification**: Pull and extract OCI image, run container from extracted image.

### Sprint 8.9: Cgroup Memory Controller

**Category**: Containers | **Estimated Lines**: ~400

**Implementation**:
- New file: `kernel/src/virt/cgroup_mem.rs`
- Memory cgroup: per-group memory limit, usage tracking
- OOM notification: callback when group exceeds limit
- Accounting: track RSS, cache, swap per cgroup
- Hierarchy: parent cgroup limits children's total usage
- `/sys/fs/cgroup/memory/` interface: `memory.limit_in_bytes`, `memory.usage_in_bytes`

**Dependencies**: Process management (exists), frame allocator (exists)

**Verification**: Set memory limit, allocate beyond limit, verify OOM handling.

### Sprint 8.10: Cgroup CPU Controller

**Category**: Containers | **Estimated Lines**: ~350

**Implementation**:
- New file: `kernel/src/virt/cgroup_cpu.rs`
- CPU shares: proportional CPU time allocation
- CPU quota: maximum CPU time per period (quota/period)
- Bandwidth control: enforce quota via scheduler integration
- `/sys/fs/cgroup/cpu/` interface: `cpu.shares`, `cpu.cfs_quota_us`, `cpu.cfs_period_us`

**Dependencies**: CFS scheduler (exists), process management (exists)

**Verification**: Set CPU quota to 50%, run CPU-intensive workload, verify CPU usage capped.

### Sprint 8.11: Overlay Filesystem

**Category**: Containers | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/fs/overlayfs.rs`
- Three-layer design: lower (read-only), upper (read-write), work (temporary)
- Read: check upper first, fall back to lower
- Write: copy-up from lower to upper, then modify in upper
- Delete: whiteout files in upper (special device node or `.wh.<name>` file)
- Directory merge: combine entries from both layers
- Mount: `mount -t overlay overlay -o lowerdir=...,upperdir=...,workdir=... /merged`

**Dependencies**: VFS (exists), tmpfs (Sprint 1.1)

**Verification**: Create overlay mount, verify copy-up on write, verify whiteout on delete.

### Sprint 8.12: Veth Networking

**Category**: Containers | **Estimated Lines**: ~350

**Implementation**:
- New file: `kernel/src/net/veth.rs`
- Virtual Ethernet pair: create two linked virtual interfaces
- Packet forwarding: send on one veth -> receive on the other
- Bridge: connect multiple veth pairs to a software bridge
- NAT: source NAT for container outbound traffic (masquerade)
- Container network namespace: assign veth endpoint to container's network namespace

**Dependencies**: Network namespaces (exist), IP stack (exists)

**Verification**: Create veth pair, assign to container, verify network connectivity.

### Sprint 8.13: Seccomp BPF

**Category**: Containers | **Estimated Lines**: ~500

**Implementation**:
- New file: `kernel/src/security/seccomp.rs`
- Seccomp mode: `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, prog)`
- BPF program: simple bytecode interpreter for syscall filtering
- Filter actions: SECCOMP_RET_ALLOW, SECCOMP_RET_KILL, SECCOMP_RET_ERRNO, SECCOMP_RET_TRACE
- Filter format: array of `struct sock_filter` (BPF instructions)
- Syscall argument inspection: filter based on syscall number and arguments
- Inheritance: seccomp filters inherited across fork/exec

**Dependencies**: Syscall infrastructure (exists), process management (exists)

**Verification**: Install seccomp filter blocking `write()`, verify process killed on write attempt.

---

## Cross-Wave Activities

### Documentation (Continuous)

- Update `docs/PHASE7.5-PLAN.md` status after each sprint
- New architecture docs for major subsystems (TLS, QUIC, ext4, xHCI)
- Update `to-dos/PHASE7.5_TODO.md` checkboxes as items complete
- Update `to-dos/MASTER_TODO.md` with Phase 7.5 status
- mdBook updates: new chapters for filesystem, networking protocols, security
- API documentation: rustdoc comments on all new public interfaces

### Testing (Per Sprint)

- Host-target unit tests for all new modules (maintain 998+ test count)
- QEMU boot testing for all three architectures after each wave
- CI pipeline green after each commit (11/11 jobs)
- Clippy zero warnings on all targets
- `cargo fmt --all -- --check` clean

### Version Bumps

- One version bump per wave completion
- Wave 1: v0.11.0 (Filesystem & Security)
- Wave 2: v0.11.1 (Performance)
- Wave 3: v0.12.0 (Hardware Drivers)
- Wave 4: v0.12.1 (Networking)
- Wave 5: v0.13.0 (Crypto & Protocols)
- Wave 6: v0.14.0 (Desktop & Userland)
- Wave 7: v0.15.0 (Audio/Video & GPU)
- Wave 8: v0.16.0 (Hypervisor & Containers)

---

## Estimated Effort Summary

| Wave | Sprints | New Lines | New Files | Modified Files | Duration |
|------|---------|-----------|-----------|----------------|----------|
| 1: Filesystem & Security | 12 | ~4,050 | 8 | 6 | 4-6 weeks |
| 2: Performance | 5 | ~1,700 | 3 | 4 | 3-4 weeks |
| 3: Hardware Drivers | 6 | ~3,500 | 6 | 2 | 4-5 weeks |
| 4: Networking | 7 | ~2,450 | 5 | 2 | 3-4 weeks |
| 5: Crypto & Protocols | 7 | ~5,550 | 10+ | 3 | 5-7 weeks |
| 6: Desktop & Userland | 12 | ~5,650 | 10 | 5 | 4-5 weeks |
| 7: Audio/Video & GPU | 14 | ~8,250 | 12 | 5 | 5-7 weeks |
| 8: Hypervisor & Containers | 13 | ~5,600 | 12 | 4 | 4-6 weeks |
| **Total** | **~76** | **~36,750** | **~66** | **~31** | **~32-44 weeks** |

---

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| TLS 1.3 complexity | High | Implement ChaCha20 suite first (crypto exists), defer AES-GCM if needed |
| DEFLATE decompression (PNG) | Medium | Well-documented algorithm, reference implementations available |
| xHCI complexity | High | Start with bulk transfers only, add isochronous later |
| Fixed-point MDCT (Vorbis/MP3) | Medium | Use 32.32 fixed-point for intermediate, 16.16 for output |
| OpenGL ES 2.0 scope | High | Implement fixed-function subset first, programmable shaders as stretch |
| Nested virtualization | High | Start with VMCS shadowing, simplest approach |
| ext4 extent tree | Medium | Read-only simplifies significantly; no journal writes needed |

---

## Success Criteria

Phase 7.5 is complete when:

1. All 80 items in `to-dos/PHASE7.5_TODO.md` are checked
2. CI passes 11/11 jobs with zero warnings on all architectures
3. Host-target test count exceeds 1200 (from current 998)
4. All new subsystems have at least basic documentation
5. QEMU x86_64 release build demonstrates:
   - ext4/FAT32 disk image mount and file access
   - SSH connection from host to guest
   - DNS resolution and HTTP GET to external server
   - Audio playback (WAV through ALSA-compatible API)
   - PNG/JPEG image display in image viewer
   - Container creation with OCI bundle
   - WireGuard tunnel establishment

---

## References

- [Phase 7.5 TODO](../to-dos/PHASE7.5_TODO.md) -- Item checklist
- [Phase 8 TODO](../to-dos/PHASE8_TODO.md) -- Deferred items for next phase
- [Master TODO](../to-dos/MASTER_TODO.md) -- Overall project status
- [Phase 7 TODO](../to-dos/PHASE7_TODO.md) -- Completed Phase 7 reference
- [Remediation TODO](../to-dos/REMEDIATION_TODO.md) -- Resolved gap tracking
- [RFC 9000 (QUIC)](https://www.rfc-editor.org/rfc/rfc9000) -- QUIC v1 specification
- [RFC 8446 (TLS 1.3)](https://www.rfc-editor.org/rfc/rfc8446) -- TLS 1.3 specification
- [RFC 7539 (ChaCha20-Poly1305)](https://www.rfc-editor.org/rfc/rfc7539) -- AEAD cipher
- [WireGuard Protocol](https://www.wireguard.com/protocol/) -- Noise IK framework
- [OCI Runtime Spec](https://github.com/opencontainers/runtime-spec) -- Container runtime
- [ext4 Documentation](https://ext4.wiki.kernel.org/index.php/Ext4_Disk_Layout) -- Filesystem layout
- [xHCI Specification](https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/extensible-host-controler-interface-usb-xhci.pdf) -- USB 3.0 host controller
- [ALSA API](https://www.alsa-project.org/alsa-doc/alsa-lib/) -- Audio interface
- [PNG Specification](https://www.w3.org/TR/PNG/) -- Image format
- [JPEG Standard (ITU-T T.81)](https://www.w3.org/Graphics/JPEG/itu-t81.pdf) -- Image compression

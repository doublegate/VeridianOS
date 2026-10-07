# Known Limitations

What VeridianOS does not do yet, or does only partly, as of the v0.27 development branch
(updated 2026-10-07). Each entry gives its tracking ID and when it is planned:

- N-01 to N-63 and the original audit IDs (MEM-SEC-01 and so on) are tracked in
  [`docs/audit/AUDIT-VERIFICATION-2026-10-05.md`](audit/AUDIT-VERIFICATION-2026-10-05.md).
- N-64 to N-181 are tracked in
  [`docs/audit/AUDIT-REEVALUATION-2026-10-07.md`](audit/AUDIT-REEVALUATION-2026-10-07.md).
- HX-, HA- and HR- IDs (real hardware) are in `ref-docs/HARDWARE-X86_64.md`,
  `ref-docs/HARDWARE-AARCH64.md` and `ref-docs/HARDWARE-RISCV64.md`.
- Sprint letters (D0, D, E, F, G, H) refer to the v0.27.0 plan in
  [`to-dos/MASTER_TODO.md`](../to-dos/MASTER_TODO.md).

Linux system-call coverage, call by call, is in
[`docs/compat/LINUX-SYSCALL-COVERAGE.md`](compat/LINUX-SYSCALL-COVERAGE.md). For the KDE Plasma
port, see [`userland/integration/KNOWN-LIMITATIONS.md`](../userland/integration/KNOWN-LIMITATIONS.md).

A claim elsewhere in the documentation that contradicts this page is wrong. Please report it.

## Kernel limitations

### Drivers and services run in the kernel (C6, planned v0.28+)

The microkernel design places drivers, filesystems, network protocols and the desktop in user-space
processes. Today all of them run in ring 0. Moving them out, in risk order, is critique item C6 of
the audit plan.

### IPC is not reachable from user programs (N-33, planned v0.28)

The native IPC syscalls 0-7 are send, receive, call, reply, create endpoint, bind, share memory and
map memory. The native dispatcher routes all eight numbers to the Linux compatibility layer, where
they mean read, write, open, close, stat, fstat, lstat and poll. This is because Qt and libstdc++
issue raw Linux numbers. So no user program can use VeridianOS IPC today. Inside the kernel, the synchronous send path also looks endpoints up in a registry that
nothing fills (N-47), so it cannot reach any endpoint either.

- **Verification is host-only.** The v0.26 IPC work (shared regions that really share their frames,
  the 16 KiB buffered message tier, capability checks with rights attenuation, and the receive
  buffer fix N-34) is covered by host unit tests and review only.
- **Latency figures are in-kernel.** Any IPC latency figure measures in-kernel helpers, not a
  process-to-process round trip.
- **The fix is v0.28 (X1):** a single Linux ABI, with the VeridianOS IPC calls in their own number
  range.
- **The IPC design itself is rebuilt in v0.28 (N-79 to N-91).** One endpoint is represented by
  several unrelated objects, receive and reply use tokens and pids as endpoint ids, SEND is the
  generic WRITE right that heap and stack capabilities also carry, delivered messages can be lost,
  the fast-path receive never blocks, and kernel memory used by IPC is not accounted. All of this
  is unreachable from user space today.

### User processes are not scheduled (C5, ADR 0006; planned v0.27 sprint D)

User programs (x86_64 only) do not run as scheduler tasks. Each one runs nested inside the kernel
shell's boot context: `fork` runs the child to completion, or until it waits, before the parent
continues, and the scheduler itself is never started. The new dispatcher (ADR 0006: per-thread
kernel stacks, `switch_stacks`, an idle task) and the scheduling policy (ADR 0007: EEVDF fair
class, FIFO/RR real-time, SCHED_DEADLINE, implemented and host-tested in `kernel/src/sched/policy/`)
replace this in sprint D. Until then:

- **No timer preemption of user code, and none inside system calls (W-13).** On x86_64 a system
  call that waits (poll, epoll, nanosleep, futex, timerfd) halts with interrupts enabled, but the
  tick does not schedule other work during that wait. A long wait delays every other process.
- **No real blocking (N-119).** A blocking pipe read with no data returns EAGAIN, a full pipe
  short-writes, PIPE_BUF writes are not atomic, eventfd/signalfd/timerfd waits spin for up to
  30 s, an empty pty read returns end-of-file, and `flock` without `LOCK_NB` fails with
  EWOULDBLOCK instead of waiting (N-120).
- **No user threads (N-102, N-46, N-50, N-54).** `clone(CLONE_THREAD)` and `pthread_create`
  cannot run threads, the native libc's `clone` returns through a C epilogue, and `errno` is one
  global. Two fixes are therefore tested by unit tests only: LIBC-SEC-01 (the allocator lock in
  `stdlib.c`; runtime test `audit_runtime_test threads`) and PROC-SEC-02 (deferred reaping of an
  exiting thread's stack).
- **Per-thread state is per process (N-109):** signal mask and pending set, `clear_child_tid`,
  the robust-list head and the FS base.
- **Exit and wait (N-106 to N-112)** are correct only because everything is nested; the
  non-nested paths lose wakeups, can reap a child twice and free running threads' stacks.

### x86_64 entry layer remainders (N-167, N-168; sprint D2)

The entry layer was rebuilt in v0.27 (ADR 0008): one register frame for every entry, symmetric
`swapgs`, IST only for #DF, NMI and #MC, checked `sysretq`, sanitised user registers, and
handlers for every exception. What remains depends on the dispatcher:

- The syscall frame pointer and user RSP scratch are per CPU, not per thread (N-167, N-35).
- A fault in user code is handled by returning to the context that launched the program; without
  one (none exists today outside that model) it stops the CPU (N-168).
- A fatal kernel fault stops only the faulting CPU; the others keep running until SMP stage S2.

### Signals (N-96, N-98, N-99, N-104, N-105, N-113, N-170; sprint D)

- **Signal-set bit layout (N-96).** The kernel and native libc use bit *n* for signal *n*, musl
  uses bit *n-1*. A musl program's `sigprocmask` therefore blocks the neighbouring signal.
- **Ignored signals stay pending (N-98).** SIGCHLD and other default-ignored signals stay in the
  pending set, and futex, wait and sigsuspend treat any pending bit as EINTR.
- **Signal frames (N-113).** The return trampoline is written to the (non-executable) user stack,
  the frame skips the red zone and is not 16-byte aligned.
- **sigreturn on AArch64 and RISC-V does not sanitise registers (N-170).** PSTATE and `sstatus`
  are restored as given, latent until those architectures have user mode. On x86_64 RFLAGS and
  RIP are sanitised since v0.27 (ADR 0008).
- **Real-time signals 32-64** cannot be installed and `sigaltstack` reports success without effect
  (N-105). Futex timeouts are read as raw ticks and absolute `FUTEX_WAIT_BITSET` timeouts are
  treated as relative (N-104).
- **wait (N-99):** no process-group waits, WUNTRACED/WCONTINUED are wrong, `waitid(WNOWAIT)`
  returns EINVAL, and file errors are mostly reported as ENOENT (N-127).

### Processes, exec and credentials

- **One working directory for everything (N-115).** Relative paths resolve against a global
  directory set by the kernel shell, not the calling process's, and `*at()` directory fds resolve
  through a path string.
- **exec (N-101):** a failure after the old address space is cleared returns to an empty address
  space instead of killing the process, there is no execute-permission check, and other threads are
  not stopped first. Fork children are runnable before setup completes (N-110).
- **Credentials (N-131, planned v0.30):** one uid and gid per process; no effective, saved or
  filesystem ids, no supplementary groups, no Linux capabilities. `chdir` does not check search
  permission.

### One CPU runs work (SMP stages S2/S3; planned v0.27 sprint D5)

With the `smp` feature (off by default), the other CPUs are started on every architecture (x86
INIT-SIPI-SIPI, AArch64 PSCI, RISC-V SBI HSM) and come online, but they only idle: all work runs on
the boot CPU. Per-CPU run queues, balancing and TLB shootdown that waits for acknowledgement are
sprint D5. Known defects that become reachable then: APs skip the syscall MSR setup (N-172), IPIs
use logical ids as APIC destinations and vector 0 (N-173), a TLB shootdown can deadlock against a
CPU spinning with interrupts off (N-139), `smp_boot` continues after a failed start (N-180), the
timer wheel assumes no lost ticks (N-179), and directory permission checks resolve the path
separately from the operation (N-51). Lock-free and per-CPU structures are single-CPU-safe but have
not run on more than one CPU (N-59).

### Only x86_64 runs user programs (N-28, N-14; planned v0.27 sprint E)

- **AArch64 runs with the MMU and caches off (N-28, HA-16).** All memory is treated as Device
  memory, so the filesystems use `fs::bare_lock`, which does not actually lock. This blocks SMP
  work and EL0 (user mode) on AArch64. The EL2 entry leaves several EL2 registers unset and writes
  SCTLR_EL1 = 0 over its RES1 bits (N-177, HA-14).
- **RISC-V has no user mode (N-14).** U-mode traps are fatal; page tables use the x86 entry format
  (HR-10).
- **The AArch64 and RISC-V kernels use FP/SIMD registers** in kernel code, so every interrupt saves
  a 720-byte frame (N-178). The x86_64 kernel is built soft-float since v0.27.

### FPU and vector registers are not saved across switches (N-41, planned v0.27 sprint D)

No x87, SSE or AVX state is saved or restored when the CPU switches between processes, although
AVX and AVX-512 are enabled. Two processes that both use vector registers can corrupt, and read,
each other's values. Today this happens only between a parent and a nested child. The x86_64 kernel
no longer touches these registers itself (soft-float target); saving user state with XSAVE, sized
from CPUID, comes with the dispatcher.

### AArch64 and RISC-V kernel stacks have no guard pages (N-26, sprint E)

x86_64 kernel stacks have unmapped guard pages. On AArch64 and RISC-V, which run the kernel
without paging today, a kernel stack overflow still silently corrupts the neighbouring frame.

### IPC copy-on-write transfers are refused

Fork is copy-on-write (ADR 0005), but IPC copy-on-write transfers are still refused rather
than faked.

### BlockFS writes only at sync and is not crash-consistent (ADR 0003, N-121)

Changed blocks stay in memory until `sync`, so the disk normally holds the last-synced state. There
is no journal, and there is no periodic sync yet, so memory for unsynced writes grows until the next
sync. Clean blocks are cached within a bound (16 MiB on x86_64, 1 MiB elsewhere).

`sync` itself is not durable or atomic: no FLUSH command is sent to the disk, data is written in
place before metadata, and freed blocks can be reused before the change that freed them reaches the
disk. A crash during `sync` can leave the filesystem inconsistent. `fsync` syncs the whole
filesystem.

### Filesystem semantics (N-117, N-123, N-124, N-126, N-128 to N-130; sprints D, G, v0.29)

- **Unlinked open files (N-117).** BlockFS frees an inode as soon as it is unlinked, even while a
  descriptor is still open, so that descriptor can read the next file that reuses the inode.
- **Directory listing (N-124).** `getdents64` re-reads the directory and seeks by index, so
  removing entries while listing (`rm -r`) skips entries; listing is O(n^2).
- **`O_APPEND` (N-126)** is not atomic across separate opens of a file.
- **Directories and paths (N-130).** No dentry cache (`..` and symlinks re-walk from the root),
  BlockFS directories are scanned linearly and limited to 12 blocks.
- **On-disk structures are not validated (N-123).** BlockFS checks only the magic number; the ext4
  and FAT32 readers have unchecked arithmetic and cannot be mounted; the shell `mkfs` does nothing.
  Only mount images you trust.
- **pty (N-128):** output processing drops `\n` but reports it written; signals are sent while
  holding the input lock.
- **mount (N-129)** is allowed to any process holding a memory capability, and type `blockfs`
  mounts an empty RAM filesystem.

### Shared IPC regions are never reclaimed

A region's frames are freed only when no process maps it. Mappings that a child inherits through
`fork` are not counted, so registered regions are kept for the life of the system. This leaks
memory, but it cannot free memory that is still in use.

### KDE binaries in existing images predate the musl and shim fixes (N-37, N-42)

v0.26.0 fixes the musl syscall-number patch (`faccessat` was delivered as `fchownat`, so a root
access check changed the file's owner; errors were translated twice) and the ctype table of the
glibc compatibility shim. These are build inputs: the kwin_wayland, plasmashell and dbus-daemon
binaries in a KDE rootfs built before v0.26.0 still contain both defects. Rebuild them with the
`tools/cross/` pipeline (`build-musl.sh` first) before using the KDE session. The BusyBox rootfs is
not affected, because it uses the native libc.

v0.27 adds native calls 355-359 for `prctl`, `flock`, `tkill`, `tgkill` and `waitid` to the musl
patch (N-103). Binaries built with an older patch still get `prctl` (the kernel recognises it) and
`flock`/`fsync` (told apart by argument), but their `tkill`, `tgkill` and `waitid` reach unrelated
native calls, so `raise()` and `abort()` do not work in them until they are rebuilt.

### Some socket calls still read user buffers directly (N-43)

`send`, `recv`, `sendto`, `recvfrom` and `sendmsg` read the caller's buffer through a raw slice after
a range check. An unmapped page inside that range kills the process instead of returning EFAULT.
Socket addresses, lengths and control messages already use the fault-tolerant copies.

### Unix socket options and flags (N-48)

Only the first control message is parsed, SOCK_CLOEXEC and SOCK_NONBLOCK are ignored at creation,
`setsockopt` on a Unix socket is accepted and ignored, and peers are reported unnamed.

### No post-quantum package signing key is provisioned (N-55)

Packages are verified with Ed25519. The kernel has a real FIPS 204 ML-DSA-65 verifier (checked
against NIST ACVP vectors), but no ML-DSA package-signing key has been generated and embedded, so a
policy that requires a post-quantum signature rejects every package. Up to v0.26.0 the
"post-quantum" verifier was not a lattice verifier and accepted forged signatures, and its trusted
key was a placeholder. The default policy never required it, so installs were gated by Ed25519
alone.

### The Kyber KEM is not FIPS 203 ML-KEM (N-57)

`crypto/post_quantum/kyber.rs` expands seeds with SHA-256 rather than SHAKE and does not implement
FIPS 203. It does not interoperate with ML-KEM and should not be relied on for confidentiality.
Nothing in the kernel uses it for real traffic.

### Unix sockets cannot pass Unix sockets

Passing a Unix-socket file descriptor in SCM_RIGHTS is refused with EINVAL. Passing one would need a
garbage collector for reference cycles. Other file types pass normally.

### Frame-backed Wayland buffers (DRV-PERF-01, DESK-ARCH-01; with C7)

`wl_shm` pools live in kernel heap memory and are not shared with a client. No user-space Wayland
client exists yet to share them with.

### Capability lookup in debug builds

`cap_lookup` measures 93 ns in a dev build: within the 100 ns target, but only just. Release-build
numbers have not been recorded.

### Hardening features are not active (N-145, N-146, N-15; planned v0.27 sprint G)

Present as code but not in effect, despite older documentation:

- **KPTI:** a shadow page table is built, but CR3 is never switched on entry or exit, and the
  shadow maps the kernel image user-accessible. On CPUs affected by Meltdown, a process can read
  kernel memory, including the physical map. The shell's `kpti` command reports it inactive.
- **KASLR:** the kernel is linked and loaded at a fixed address.
- **Stack canaries:** no stack-protector instrumentation is compiled in.
- **Retpoline, IBRS/eIBRS, IBPB, RSB filling, MDS clearing, UMIP:** not enabled (HX-23). There is
  no early microcode loading either (HX-24), so fixes shipped as microcode depend on the firmware.
- **SMEP/SMAP:** not enabled (N-15).
- **AArch64:** no branch-predictor or BHB hardening on entry from EL0 and no SSBD (HA-30); moot
  until AArch64 has user mode.

### Network sockets do not work from user programs (N-64 to N-69; planned v0.27 sprint F)

`bind` and `connect` misread `struct sockaddr_in`, UDP datagrams are neither sent nor received
through the socket layer, TCP `connect` reports success without a handshake, and received packets
are processed only while the kernel shell waits for input. Loopback and Unix sockets are not
affected. Below the socket layer:

- **TCP is incomplete (NET-INC-01, N-08, N-20, N-76).** No retransmission or close states,
  segments carry checksum 0, `accept` returns a fresh socket instead of the connection, in-window
  sequence numbers are not checked (RFC 5961 injection), and the initial sequence number is a
  global counter (RFC 6528). Whether to adopt smoltcp is decided at the start of sprint F3.
- **UDP (N-66, N-67):** datagrams are delivered last-in first-out, a port-0 socket captures every
  datagram, checksums are not verified and the IPv6 pseudo-header is missing.
- **IPv4 and ARP (N-70 to N-72):** an ARP miss sends the packet to the broadcast address, the ARP
  cache accepts any sender and never expires, received IPv4 headers are not checksummed or checked
  for destination and fragments, ICMP is dropped, and over-MTU sends vanish silently.
- **DNS (N-77):** `resolve` never sends a query.
- **Errors (N-68):** socket errors are all reported as EIO (`EAGAIN` included), `setsockopt`
  accepts and ignores every option, and INET sockets always poll writable and never readable.
- **WireGuard (NET-INC-02)** is not bound to the network stack, and the `wg` shell command uses an
  all-zero key.

### File mmap is a private copy (N-125)

A file mapping is copied in full at `mmap` time. `MAP_SHARED` writes never reach the file, and
two processes mapping one file do not see each other's changes.

### Memory mapping semantics (N-140 to N-143, N-88; sprints D and G)

Page protections now follow `prot` (fixed in v0.27, N-132 to N-137). What remains:

- **`MAP_SHARED` anonymous memory** becomes copy-on-write at `fork`, so parent and child stop
  sharing it (N-140).
- **`MAP_FIXED` over an existing mapping** fails with ENOMEM instead of replacing it, and `munmap`
  across several mappings or holes fails with EINVAL (N-141). `mprotect` of part of a mapping does
  not split it (N-135 remainder).
- **Memory is allocated eagerly** at `mmap` time, not on first touch (v0.29 memory work).
- **RAM above the eighth memory region is lost (N-142, HX-07).** Only the eight largest usable
  regions are used; UEFI firmware on real machines reports 50 to 150.
- **POSIX shared memory (N-88)** has no access control, `ftruncate` can free frames that are still
  mapped, and unlinked names can still be opened.

### Security policy features are partial (N-150 to N-152, N-88)

- The TLS 1.3 client does not verify certificate signatures or the server's CertificateVerify.
- MAC labels every file and user process alike; its capability step only logs, once per access,
  which floods the audit ring.
- `prctl` implements only the name and timer-slack options; everything else, including
  `PR_SET_NO_NEW_PRIVS` and `PR_SET_SECCOMP`, fails with EINVAL (fixed in v0.27, N-151). The
  seccomp filter itself is never consulted.
- POSIX shared memory objects have no access control (N-88).

### Default credentials and password storage (N-153; sprint G)

- The kernel creates the account `root` with the password `veridian` at boot.
- The graphical display manager accepts `root` with **any** non-empty password.
- Password hashes use 10 PBKDF2 iterations in dev builds and 10,000 in release builds, the count is
  not stored with the hash, the comparison is not constant-time, and TOTP does not follow RFC 6238.

Do not expose a VeridianOS system to untrusted users or networks.

### Cryptography is not constant-time and has no known-answer tests (N-154, N-148; sprint G)

Ed25519 scalar multiplication branches on secret data, AES is table-based and the GF multiply
branches, so keys can leak through timing. Apart from ML-DSA-65 (checked against NIST ACVP vectors),
no primitive is tested against published vectors, and the TLS code carries duplicate AEADs. The repository server's upload
check (`pkg/repo_server.rs`, through `pkg/build_package.rs`) accepts a 4-byte marker instead of a
signature; package installation verifies with the real Ed25519 verifier in `pkg/mod.rs`.

### No DMA isolation (N-77, HX-19)

The IOMMU tables are parsed but translation is never enabled; devices can DMA anywhere. On
AArch64 and RISC-V the IOMMU code is a stub.

### Drivers poll and some still mishandle DMA (N-52, N-58, N-73 to N-75; sprints F4, F5)

- **Storage completes by polling.** virtio-blk spins for up to ten million iterations while holding
  the device lock, and NVMe polls for up to 5 s holding the controller lock (N-52, N-74). FLUSH is
  negotiated with virtio-blk but never sent.
- **virtio-gpu** hands the device heap virtual addresses as DMA addresses, and its fixed 256-entry
  ring structures overlap for smaller queues (N-58).
- **Legacy virtio-pci queues** are clamped to 256 entries while the device keeps its own larger
  size, so the device can write past the ring (N-73).
- **e1000e (8086:10D3)** is driven as an 8254x, so its MAC address reads as zero (N-75). Only the
  QEMU NIC models are supported: e1000 82540EM and virtio-net (HX-20).
- **Network receive is not interrupt-driven (N-69):** the interrupt path fills a queue nothing
  reads.

### Formal verification covers models, not the kernel (N-163)

The Kani harnesses prove properties of standalone models (whose capability token layout differs
from `cap/token.rs`), and neither Kani nor TLC runs in CI.

### Build, CI and supply chain (N-161, N-162, N-164; sprint G)

- CI's global `RUSTFLAGS` replaces the target rustflags from `.cargo/config.toml`, so CI's AArch64
  and x86_64 kernels are built with different flags from local ones (N-161).
- The coverage job can never fail the build (N-162).
- BusyBox, musl and the `tools/cross` sources are downloaded without checksum verification,
  `cargo audit` covers one of five lockfiles, and the workflows have no top-level `permissions:`
  and pin actions by tag rather than commit (N-164).

Since v0.27, releases and the CI summary require the host, boot and rootfs test jobs, the release
profile is booted on every architecture, and clippy runs on all three bare-metal targets with no
allow-list (N-159, N-160).

### Entropy on AArch64 and RISC-V (N-149)

The CSPRNG is seeded only from timer jitter on AArch64, RISC-V and x86 CPUs without RDRAND, which
is close to deterministic under QEMU TCG.

### Runs only under QEMU (planned v0.27 sprints E and H)

Every architecture depends on QEMU-specific addresses and devices, and AArch64 and RISC-V can
only be loaded as ELF files at one fixed address. Real-hardware requirements and the bring-up
order are in `ref-docs/HARDWARE-X86_64.md`, `ref-docs/HARDWARE-AARCH64.md` and
`ref-docs/HARDWARE-RISCV64.md`. In short:

- **x86_64:**
  - **Boot devices.** Needs a 16550 at 0x3F8 and an 8254 PIT; the console reads endless 0xFF
    bytes without the UART, and timer calibration can hang (HX-01, HX-03).
  - **Memory and CPUs.** At least ~1.5 GiB of RAM for the 1 GiB static heap (HX-02), xAPIC
    handover with the I/O APIC at 0xFEC00000, 8-bit APIC IDs and at most 16 CPUs
    (HX-04, HX-05, N-174).
  - **Buses and ACPI.** PCI configuration space through port I/O only (no ECAM), and BAR sizing
    mishandles 64-bit BARs (HX-11, HX-12). There is no AML interpreter, so interrupt routing,
    sleep states and power buttons are unavailable (HX-28).
  - **Power and controllers.** Power-off uses QEMU's debug-exit port (HX-15). There is no
    USB-legacy or AHCI firmware handoff (HX-18, HX-29).
  - **Device coverage.** Display is the UEFI framebuffer or virtio-gpu only (HX-31).
- **AArch64:**
  - **Boot protocol.** Discards the device-tree pointer, has no Image header, and runs only at
    0x40200000 with the MMU off (HA-01 to HA-04).
  - **Platform devices.** Hard-codes the QEMU PL011 UART, GICv2 addresses and a 128 MiB RAM layout
    (HA-05, HA-07, HA-17), and supports no GICv3 or PCIe (HA-20).
  - **Caches and CPU start.** Has no cache maintenance anywhere (HA-25), and can start secondary
    CPUs only through PSCI, not the Raspberry Pi 4's spin-table (HA-22).
- **RISC-V:**
  - **Boot.** Never zeroes BSS (relies on the ELF loader) and has no Image header (HR-01, HR-03).
  - **Fixed addresses.** Writes to 0x1000_0000, a clock controller on the HiFive Unmatched (HR-04).
  - **Paging.** Supports only Sv48, while the common boards are Sv39-only, and uses x86-format
    page-table entries (HR-09, HR-10).
  - **Interrupts.** Computes the PLIC context wrongly on boards whose hart 0 is a monitor core
    (HR-12) and treats device interrupts as fatal (HR-31).
  - **Privileged registers.** Reads M-mode registers from S-mode (HR-20, HR-21).
  - **Missing support.** No PCIe, no non-coherent DMA and no shutdown or reboot path
    (HR-24, HR-34, HR-38).

Sprint E fixes the boot-critical AArch64 and RISC-V items together with turning the MMU on;
sprint H covers the rest, in the hardware order given in the plan.

## Measurement caveat

Before v0.26.0, every in-kernel benchmark divided TSC ticks by 2 (an assumed 2 GHz clock). On the
3.6 GHz reference machine, every older benchmark figure in this repository overstates time by 1.8x.
`docs/PERFORMANCE-REPORT.md` explains the conversion.

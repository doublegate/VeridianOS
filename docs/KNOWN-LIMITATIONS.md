# Known Limitations

What VeridianOS does not do yet, or does only partly, as of the v0.26 audit remediation. Each
entry gives its tracking ID in [`docs/audit/AUDIT-VERIFICATION-2026-10-05.md`](audit/AUDIT-VERIFICATION-2026-10-05.md)
and when it is planned. For the KDE Plasma port, see
[`userland/integration/KNOWN-LIMITATIONS.md`](../userland/integration/KNOWN-LIMITATIONS.md).

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

### No user threads (C5, planned v0.27)

The scheduler has no ring-3 entry path for threads, so `clone(CLONE_THREAD)` and `pthread_create`
cannot run threads. Two fixes are therefore tested by unit tests only:

- LIBC-SEC-01: the allocator lock in `stdlib.c`. Its runtime test is `audit_runtime_test threads`.
- PROC-SEC-02: the deferred reaping of an exiting thread's stack.

### User processes are not preempted inside system calls (W-13, C5)

On x86_64 a system call that waits (poll, epoll, nanosleep, futex, timerfd) halts with interrupts
enabled, but the tick does not schedule other work during that wait. A voluntary yield broke the
nested boot-time process context. A long wait therefore delays other processes on the same CPU.

### One CPU (planned v0.27)

Only the boot CPU runs on every architecture. SMP bring-up, per-CPU run queues and TLB shootdown are
v0.27 work. Lock-free and per-CPU structures are single-CPU-safe but have not run on more than one
CPU. Some state that must be per-CPU is still global: the x86_64 saved syscall
frame (`SYSCALL_FRAME_PTR`, N-35) is one example, and directory permission checks that resolve the
path separately from the operation (N-51) are another.

### AArch64 runs with the MMU and caches off (N-28, planned v0.27)

All memory is treated as Device memory, so the filesystems use `fs::bare_lock`, which does not
actually lock. This blocks SMP and EL0 (user mode) on AArch64. RISC-V has no user mode yet either.

### FPU and vector registers are not saved across switches (N-41, planned v0.27)

No x87, SSE or AVX state is saved or restored when the CPU switches between processes, although
AVX and AVX-512 are enabled. Two processes that both use vector registers can corrupt, and read,
each other's values. The fix (XSAVE, with the area sized from CPUID) is part of the v0.27 process
model work (C5).

### AArch64 and RISC-V kernel stacks have no guard pages (N-26, sprint E)

x86_64 kernel stacks have unmapped guard pages. On AArch64 and RISC-V, which run the kernel
without paging today, a kernel stack overflow still silently corrupts the neighbouring frame.

### IPC copy-on-write transfers are refused

Fork is copy-on-write (ADR 0005), but IPC copy-on-write transfers are still refused rather
than faked.

### BlockFS writes only at sync (ADR 0003)

Changed blocks stay in memory until `sync`, so the disk always holds the last-synced state. There is
no journal, and there is no periodic sync yet, so memory for unsynced writes grows until the next
sync. Clean blocks are cached within a bound (16 MiB on x86_64, 1 MiB elsewhere).

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
  kernel memory, including the physical map.
- **KASLR:** the kernel is linked and loaded at a fixed address.
- **Stack canaries:** no stack-protector instrumentation is compiled in.
- **Retpoline, IBRS/eIBRS, IBPB, RSB filling, MDS clearing, UMIP:** not enabled.
- **SMEP/SMAP:** not enabled (N-15).

### Network sockets do not work from user programs (N-64 to N-69; planned v0.27 sprint F)

`bind` and `connect` misread `struct sockaddr_in`, UDP datagrams are neither sent nor received
through the socket layer, TCP `connect` reports success without a handshake, and received packets
are processed only while the kernel shell waits for input. Loopback and Unix sockets are not
affected.

### File mmap is a private copy (N-125)

A file mapping is copied in full at `mmap` time. `MAP_SHARED` writes never reach the file, and
two processes mapping one file do not see each other's changes.

### Memory protection flags (N-132, N-135; planned v0.27 sprint D0)

`mmap` and `brk` ignore `prot`: anonymous, shared and heap pages are mapped writable and
executable, and `PROT_NONE` does not remove access. `mprotect` cannot revoke access.

### Security policy features are partial (N-150 to N-152, N-88)

- The TLS 1.3 client does not verify certificate signatures or the server's CertificateVerify.
- MAC labels every file and user process alike; its capability step only logs.
- `prctl` reports success for every option, so `PR_SET_NO_NEW_PRIVS` and `PR_SET_SECCOMP` have no
  effect, and the seccomp filter is never consulted.
- POSIX shared memory objects have no access control.

### No DMA isolation (N-77)

The IOMMU tables are parsed but translation is never enabled; devices can DMA anywhere.

### Formal verification covers models, not the kernel (N-163)

The Kani harnesses prove properties of standalone models (whose capability token layout differs
from `cap/token.rs`), and neither Kani nor TLC runs in CI.

### Entropy on AArch64 and RISC-V (N-149)

The CSPRNG is seeded only from timer jitter on AArch64, RISC-V and x86 CPUs without RDRAND, which
is close to deterministic under QEMU TCG.

### Runs only under QEMU (planned v0.27 sprints E and H)

Every architecture depends on QEMU-specific addresses and devices, and AArch64 and RISC-V can
only be loaded as ELF files at one fixed address. Real-hardware requirements and the bring-up
order are in `ref-docs/HARDWARE-X86_64.md`, `ref-docs/HARDWARE-AARCH64.md` and
`ref-docs/HARDWARE-RISCV64.md`. In short:

- **x86_64:** needs a 16550 at 0x3F8 and an 8254 PIT (the console reads endless 0xFF bytes
  without one, and timer calibration can hang), at least ~1.5 GiB of RAM, xAPIC handover and at
  most 16 CPUs.
- **AArch64:** discards the device-tree pointer, has no Image header, runs only at 0x40200000
  with the MMU off, and hard-codes the QEMU UART, GIC and RAM layout.
- **RISC-V:** never zeroes BSS (relies on the ELF loader), has no Image header, writes to
  0x1000_0000 (a clock controller on the HiFive Unmatched), uses x86-format page-table
  entries, and treats device interrupts as fatal.

## Measurement caveat

Before v0.26.0, every in-kernel benchmark divided TSC ticks by 2 (an assumed 2 GHz clock). On the
3.6 GHz reference machine, every older benchmark figure in this repository overstates time by 1.8x.
`docs/PERFORMANCE-REPORT.md` explains the conversion.

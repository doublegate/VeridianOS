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
issue raw Linux numbers. So no user program can use VeridianOS IPC today.

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
frame (`SYSCALL_FRAME_PTR`, N-35) is one example.

### AArch64 runs with the MMU and caches off (N-28, planned v0.27)

All memory is treated as Device memory, so the filesystems use `fs::bare_lock`, which does not
actually lock. This blocks SMP and EL0 (user mode) on AArch64. RISC-V has no user mode yet either.

### Kernel stacks have no guard pages (N-26, planned v0.27)

A kernel stack overflow silently corrupts the neighbouring frame.

### No copy-on-write (planned v0.27)

Fork copies every page. IPC copy-on-write transfers are refused rather than faked.

### BlockFS writes only at sync (ADR 0003)

Changed blocks stay in memory until `sync`, so the disk always holds the last-synced state. There is
no journal, and there is no periodic sync yet, so memory for unsynced writes grows until the next
sync. Clean blocks are cached within a bound (16 MiB on x86_64, 1 MiB elsewhere).

### Shared IPC regions are never reclaimed

A region's frames are freed only when no process maps it. Mappings that a child inherits through
`fork` are not counted, so registered regions are kept for the life of the system. This leaks
memory, but it cannot free memory that is still in use.

### Unix sockets cannot pass Unix sockets

Passing a Unix-socket file descriptor in SCM_RIGHTS is refused with EINVAL. Passing one would need a
garbage collector for reference cycles. Other file types pass normally.

### Frame-backed Wayland buffers (DRV-PERF-01, DESK-ARCH-01; with C7)

`wl_shm` pools live in kernel heap memory and are not shared with a client. No user-space Wayland
client exists yet to share them with.

### Capability lookup in debug builds

`cap_lookup` measures 93 ns in a dev build: within the 100 ns target, but only just. Release-build
numbers have not been recorded.

## Measurement caveat

Before v0.26.0, every in-kernel benchmark divided TSC ticks by 2 (an assumed 2 GHz clock). On the
3.6 GHz reference machine, every older benchmark figure in this repository overstates time by 1.8x.
`docs/PERFORMANCE-REPORT.md` explains the conversion.

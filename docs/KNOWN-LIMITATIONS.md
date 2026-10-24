# Known Limitations

What VeridianOS does not do yet, or does only partly, as of v0.27.0
(2026-10-18). Each entry gives its tracking ID and when it is planned:

- N-01 to N-63 and the original audit IDs (MEM-SEC-01 and so on) are tracked in
  [`docs/audit/AUDIT-VERIFICATION-2026-10-05.md`](audit/AUDIT-VERIFICATION-2026-10-05.md).
- N-64 to N-181 are tracked in
  [`docs/audit/AUDIT-REEVALUATION-2026-10-07.md`](audit/AUDIT-REEVALUATION-2026-10-07.md).
- N-182 to N-260 are tracked in
  [`docs/audit/ABI-REVIEW-2026-10-07.md`](audit/ABI-REVIEW-2026-10-07.md), and N-261 to N-279
  (the 2026-10-10 matrix intake) in
  [`docs/audit/AUDIT-INTAKE-2026-10-10.md`](audit/AUDIT-INTAKE-2026-10-10.md).
- HX-, HA- and HR- IDs (real hardware) are in `ref-docs/HARDWARE-X86_64.md`,
  `ref-docs/HARDWARE-AARCH64.md` and `ref-docs/HARDWARE-RISCV64.md`.
- Sprint letters (D0, D, E, F, G, H) refer to the v0.27.0 plan in
  [`to-dos/MASTER_TODO.md`](../to-dos/MASTER_TODO.md). v0.27.0 was cut before F, G and H and
  before the last D and E items; those are planned for v0.28.

Linux system-call coverage, call by call, is in
[`docs/compat/LINUX-SYSCALL-COVERAGE.md`](compat/LINUX-SYSCALL-COVERAGE.md) (x86_64),
[`-AARCH64.md`](compat/LINUX-SYSCALL-COVERAGE-AARCH64.md) and
[`-RISCV64.md`](compat/LINUX-SYSCALL-COVERAGE-RISCV64.md). For the KDE Plasma
port, see [`userland/integration/KNOWN-LIMITATIONS.md`](../userland/integration/KNOWN-LIMITATIONS.md).

A claim elsewhere in the documentation that contradicts this page is wrong. Please report it.

## Kernel limitations

### Linux system call semantics (N-190 to N-247; sprints D, F and X2)

Since ADR 0009 a Linux system call number promises Linux behaviour. A review of every handler
behind one found about 80 differences. The security-relevant ones and those the renumbering
exposed were fixed (N-182 to N-189); the rest are listed, with their targets, in
[`docs/audit/ABI-REVIEW-2026-10-07.md`](audit/ABI-REVIEW-2026-10-07.md). The most visible:

- **Blocking.** recvfrom, accept, sendmsg and recvmsg never block on a blocking socket
  (N-235).
- **MAP_SHARED of regular files** is a private copy: a write through the mapping does not reach
  the file or other mappings. Memfds and tmpfs files, including everything in `/dev/shm`, share
  their pages (N-230); the rest needs the v0.29 page cache.
- **clone across processes.** A `clone` without CLONE_THREAD makes a process (fork, vfork,
  `posix_spawn`); with CLONE_VM it shares the parent's memory page by page as it is at the
  clone, so a mapping either side adds or removes afterwards is its own (enough for vfork and
  `posix_spawn`, whose child only execs or exits). CLONE_FILES and CLONE_SIGHAND between
  processes (a shared fd or signal-handler table) are refused (N-210).
- **Time.** Every Linux clock exists and CLOCK_REALTIME counts from 1970 (N-218, N-219), with
  these differences: a sleep or futex wait until an absolute CLOCK_REALTIME time does not move
  when the clock is set afterwards (timerfds do); sleeping on a process CPU clock is ENOTSUP;
  there is no leap-second offset, so CLOCK_TAI reads as UTC; BOOTTIME equals MONOTONIC, as there
  is no suspend; and on AArch64 and RISC-V the wall clock comes from a device-tree PL031 or
  Goldfish RTC, read once at boot (without one it counts from the epoch). adjtimex and
  clock_adjtime do not exist.
- **Resource usage.** getrusage, times and wait4 report CPU time (split into user and system
  time by tick samples), peak resident set and context switches; page-fault, block I/O, IPC and
  signal counts are zero.
- **Resource limits** (N-224). All sixteen limits are kept per process, inherited on fork and
  kept across exec, with Linux's prlimit64 rules. NOFILE, FSIZE, NPROC, AS, DATA, STACK (stack
  growth), CPU, NICE, RTPRIO and SIGPENDING are enforced as Linux enforces them. CORE, MEMLOCK
  and MSGQUEUE limit facilities that do not exist yet (core files, mlock, POSIX message queues).
  LOCKS and RSS are ignored, as Linux ignores them. RTTIME is not enforced: a real-time thread
  that never blocks is not stopped. The stack limit applies to stack growth only; the 256 KiB
  first stack is mapped whatever the limit, and exec does not size argument space from it.
  Queued real-time signals stay charged to the real user the first one was charged to, even if
  the process changes its real user before they are delivered.
- **Memory calls.** mremap, madvise, munmap, mprotect and brk follow Linux (N-240 to N-246),
  except: MADV_FREE keeps the pages (memory is never reclaimed under pressure, which Linux
  permits); MADV_COLLAPSE and the guard-page advice (Linux 6.13) are EINVAL; MADV_REMOVE on a
  MAP_SHARED regular file other than a memfd or tmpfs file zeroes only this process's copy (such
  a mapping is a private copy until the v0.29 page cache); MREMAP_DONTUNMAP takes only private anonymous memory (Linux 5.7's rule);
  moving the heap with mremap leaves the program break where it was; and mappings that Linux
  would merge (two adjacent mmaps alike in every way) stay separate, so an mremap spanning both
  is EFAULT.
- **W^X.** By default a mapping is never writable and executable at once: mmap and mprotect
  with PROT_WRITE|PROT_EXEC fail with EACCES, where Linux allows it (N-241, kept by design; ADR
  0017).
  - A program that needs it (a JIT) opts in with the VeridianOS ELF note
    (`<veridian/wxneeded.h>`) and then behaves as on Linux.
  - An unmodified Linux binary carries no note. `tools/veridian-elfctl set wxneeded FILE` adds
    one without relinking (a new program header table at the end of the file; run it after
    `strip`), and `clear` takes the opt-in away again.
  - Without the note, a JIT must write code through a writable mapping and run it from an
    executable one (a second mapping of a memfd, or mprotect between the two).
  - PR_SET_MDWE follows Linux. It is reported only when set: the default W^X is not MDWE,
    because it allows memory to become executable.
- **Futexes.** FUTEX_LOCK_PI, LOCK_PI2, TRYLOCK_PI and UNLOCK_PI work (priority inheritance,
  SCHED-INC-01), but FUTEX_WAIT_REQUEUE_PI and CMP_REQUEUE_PI (glibc's PI condition variables) are
  ENOSYS; a deadline task waiting for a PI lock lends real-time priority 99, not its deadline
  parameters; and a waiter of a PI lock in memory shared between processes whose holder exits gets
  ESRCH unless the lock is robust.
- **Errnos.** Many errors come back as EINVAL where Linux is specific (N-202, N-212 to N-216,
  N-234, N-238).

### Conformance suites: what most failures come from (tests/compat)

LTP's `syscalls`, the Open POSIX Test Suite and musl libc-test run on all three architectures, and
`tests/compat/expected-<arch>.tsv` records every test's result with the reason it gave (counts in
[`docs/compat/COMPATIBILITY-PLAN.md`](compat/COMPATIBILITY-PLAN.md)). Most LTP tests never reach
their check, for a few shared reasons:

- **statfs is ENOSYS.** LTP's library asks for the filesystem type of its temporary directory, so
  about 630 of the 1,552 tests are TBROK before they start.
- **The release `uname` reports.** LTP warns that the kernel is "older than minimal supported 4.4"
  (TWARN) in every new-API test: VeridianOS reports its own version. About 205 of them otherwise
  passed; in the rest the warning comes with the real outcome (a skip, a failure).
- **No kernel configuration.** Tests that check a `CONFIG_*` option (about 95) find neither
  `/proc/config.gz` nor `/boot/config-<release>` and are TBROK.
- **Missing calls.** alarm and the interval and POSIX timers, the System V IPC calls, message
  queues, mlock, msync, sched_getparam and the other calls the coverage tables list as missing;
  most of `/proc/sys` (it has `kernel/pid_max`, `kernel/cap_last_cap` and
  `kernel/core_pattern`), `/proc/self/mounts` and parts of `/proc/<pid>/stat` and `status`.
- **Memory exhaustion.** libc-test's `malloc-oom`, `malloc-brk-fail` and `pthread_create-oom`
  stop the whole guest on RISC-V. On x86_64 and AArch64 they ran until the time limit after the
  full LTP and Open POSIX runs in the same guest, but reached ENOMEM within 10 to 16 s when
  libc-test ran on its own (x86_64): how they end depends on what ran before.
- **A new process can be missing from /proc for a moment.** Open POSIX tests that wait for a
  child's state in `/proc/<pid>/stat` (its `include/proc.h`) read it right after fork and now and
  then find no such file (`sem_timedwait/9-1`, once in four x86_64 runs; it is marked FLAKY).
- **Stress tests are slow.** Open POSIX's `shm_open/23-1` (1,000 processes) does not finish within
  the limit; tearing down that many processes takes longer than a minute.
- **wordexp.** libc-test's `functional/wordexp` runs until the time limit on all three
  architectures. On x86_64 the first error it reports is the root filesystem's, not the kernel's:
  BusyBox ash there is built without arithmetic (`tools/busybox/veridian_defconfig`), and the test
  expands `$((3*4))`.

### Drivers and services run in the kernel (C6, planned v0.28+)

The microkernel design places drivers, filesystems, network protocols and the desktop in user-space
processes. Today all of them run in ring 0. Moving them out, in risk order, is critique item C6 of
the audit plan.

### Native IPC works, but is not the planned design yet (N-79 to N-91; planned v0.28)

The native IPC calls (send, receive, call, reply, create endpoint, bind, share memory, map memory)
have private numbers 1024-1031 since the switch to Linux system call numbers (ADR 0009). Since
v0.27 a program can create an endpoint, send to it and receive from it: the capability names the
registered endpoint (N-81 part), send and receive use the registry endpoints are created in (N-47),
and a receive sleeps on the endpoint until a message arrives, with EINTR on a signal (N-84). The
runtime test `native_ipc_send_receive_blocks` covers this.

- **Verification is partly host-only.** Shared regions that really share their frames, the 16 KiB
  buffered message tier and the receive buffer fix N-34 are covered by host unit tests and review
  only.
- **Latency figures are in-kernel.** Any IPC latency figure measures in-kernel helpers, not a
  process-to-process round trip.
- **The IPC design itself is rebuilt in v0.28 (N-79 to N-91).** Every message goes through an
  endpoint queue (the register fast path went with the old scheduler); reply uses thread ids;
  SEND is the generic WRITE right that heap and stack capabilities also carry; a capability passed
  in a message is not transferred to the receiver; and kernel memory used by IPC is not accounted.
  No program uses native IPC yet.

### Process model gaps (C5, ADR 0006; sprint D, the rest planned v0.28)

Since stage D2, every user thread (x86_64) runs as a dispatcher task of its own, on its own
kernel stack and with its own saved vector state. Programs, forked children and `clone` threads
run alongside each other, the kernel shell waits for the programs it starts, and musl's pthreads
work. User code is preempted when its slice ends (D3). What is still missing:

- **The kernel is not preemptible.** A task is switched only on its way back to user mode or
  when it waits, so a long system call keeps the CPU until it returns or waits.
- **Some waits still re-check instead of being woken (blocking step of sprint D).** Sleeps,
  futexes, pipes, ptys, the console, eventfd, timerfd, signalfd, IPC endpoints, `sigsuspend`,
  `wait` and job-control stops sleep until woken. `poll`, `select`, `epoll_wait` and blocking
  reads of INET sockets and other descriptors that do not report readiness changes yet sleep too
  but re-check every 10 ms (INET sockets go with the F1 socket layer). Without per-object
  wake-ups, an edge-triggered epoll registration may report again after unrelated I/O (users of
  EPOLLET read until EAGAIN, so this costs a wake-up, not correctness).
- **exec by a thread other than the main one keeps its thread id.** exec from a multithreaded
  process ends the other threads first, as on Linux (N-101), but Linux also gives the exec'ing
  thread the process id as its thread id; here it keeps its own.
- **Some per-thread state is per process (N-109):** the robust-list head and the FS base
  registered through `arch_prctl` (the signal mask and pending set are per thread since D3).
- **Old binaries.** KDE binaries built before the v0.27 musl patch map `pthread_exit` to a
  process exit.

### x86_64 entry layer remainders (N-167, N-168; sprint D2)

The entry layer was rebuilt in v0.27 (ADR 0008): one register frame for every entry, symmetric
`swapgs`, IST only for #DF, NMI and #MC, checked `sysretq`, sanitised user registers, and
handlers for every exception. What remains depends on the dispatcher:

- The syscall frame pointer and user RSP scratch are per CPU; the dispatcher saves and restores
  them with each task rather than keeping them on the task's own stack (N-167, N-35).
- A fatal kernel fault stops only the faulting CPU; the others keep running until SMP stage S2.

### Signals (N-99, N-104, N-105, N-170; sprint D)

Since sprint D3, dispatched threads on x86_64 take signals as Linux does:
- **Sets and state.** Sets use the Linux layout. Each thread has its own mask and pending set.
  Ignored signals are dropped when sent.
- **Delivery.** Handlers run on a Linux `rt_sigframe` with SA_RESTORER, and SA_RESTART,
  SA_NODEFER and SA_RESETHAND are honoured. `sigsuspend` and `sigpending` work, and a fault
  reaches an installed handler (N-96, N-98, N-113).
- **Job control.** Stop signals stop every thread of the process, SIGCONT continues it, and the
  parent sees both through SIGCHLD and `wait` (WUNTRACED, WCONTINUED, process-group waits).

What remains:

- **Job control is partial.** A stopped thread parks at its next return to user mode, so one
  blocked in a system call stays there until it returns; the parent is told at once rather than
  after every thread has stopped. Orphaned process groups are not exempt from SIGTSTP, SIGTTIN
  and SIGTTOU, and background reads and writes on a terminal raise neither (no line discipline
  yet).
- **Handlers save x87/SSE state only.** The signal frame holds an FXSAVE image, so the upper
  halves of AVX registers are not preserved across a handler that uses them.
- **No timer or I/O siginfo.** POSIX timers (timer_create) and SIGIO (F_SETOWN, O_ASYNC) do
  not exist yet, so no signal arrives with SI_TIMER or POLL_* information; kill, tkill,
  sigqueue, SIGCHLD and fault signals carry Linux's siginfo (N-259).

### Processes, exec and credentials

- **Directory fds are paths (N-115 remainder).** `*at()` calls and `fchdir` reach their
  directory fd through the canonical path recorded at open, not the open directory. If the
  directory is renamed afterwards they fail with ENOENT (where Linux would still use it); they
  never act on a different directory that now has the old name.
- **exec (N-101 remainder):** fork children are runnable before setup completes (N-110). A
  multithreaded process can exec: its other threads end at the point of no return, once the
  program has been found and checked, as Linux's de_thread.
- **ptrace is partial:** PTRACE_TRACEME and PTRACE_ATTACH make the caller the tracer for
  PEEKDATA and POKEDATA (and deny the tracee set-ID privilege at exec), but the tracee is not
  stopped, the tracer's `wait` does not report it, and the register and resume requests (GETREGS,
  SETREGS, CONT, SINGLESTEP) fail. A debugger cannot drive a process yet (planned v0.28).
- **Credentials (N-131 remainder, planned v0.30):** real, effective and saved IDs,
  filesystem IDs (setfsuid, setfsgid), supplementary groups (N-248) and set-user-ID/set-group-ID
  exec exist. Each thread has Linux capability sets (ADR 0020), which `capget`, `capset` and the
  capability prctl options read and change and `/proc/<pid>/status` shows, and which the set*id
  calls check (CAP_SETUID, CAP_SETGID) and fix up as Linux does, as do file permission checks
  (CAP_DAC_OVERRIDE, CAP_DAC_READ_SEARCH). The rest do not use them yet: file ownership checks
  (chmod, chown, utimes, sticky directories, mknod) are still filesystem uid 0, and other
  privilege effective uid 0. A directory's
  set-group-ID bit is kept but not acted on: new entries take the creator's group, not the
  directory's.
- **Process ids never wrap.** `/proc/sys/kernel/pid_max` reports 4194304, Linux's limit on
  64-bit systems, but the kernel does not keep ids below it: process and thread ids count up from
  1 and are never reused, where Linux wraps at `pid_max` and reuses freed ids. A system that
  creates more than 4194303 processes and threads gets ids above `pid_max`, and an id past
  2147483647 no longer fits in `pid_t`. Programs that read `pid_max` to size a table, or that use
  it as an id no process has (LTP's `tst_get_unused_pid`), work until then.
- **chroot (N-250):** a working directory outside the new root is moved to the root, where Linux
  would leave it outside.

### SMP: what is not done yet (sprint D5 leftovers, planned v0.28)

The `smp` feature is on by default since v0.27: the other CPUs are started on every architecture (x86
INIT-SIPI-SIPI, AArch64 PSCI, RISC-V SBI HSM), and all three architectures run tasks on them
(ADR 0012), user tasks included: tasks are placed on idle CPUs, balanced every 4 ms and moved by
sched_setaffinity. Since D5a every CPU gets the boot CPU's
syscall MSRs and XCR0 (N-172), IPIs go to APIC IDs with the ICR written under one interrupt-off
sequence (N-173), and bring-up stops at the first CPU that fails, with guarded stacks (N-180). Since
D5b the timer wheel advances by elapsed clock time and an x86_64 CPU whose TSC is not synchronised
with the boot CPU's stays offline (N-179); a TSC that is synchronised but not invariant is reported,
not refused. System calls, page faults and the return to user mode run with interrupts enabled,
so TLB shootdowns are always answered (N-139, ADR 0012 rule 6).

Still open:
- Lock-free structures (`sync/`, N-59) have not run on more than one CPU.
- Secondaries copy the boot CPU's CR4, so they match it, including SMEP and SMAP being off (N-15,
  below).

### AArch64 and RISC-V user programs (sprint E)

- **AArch64 runs musl programs and a BusyBox root (ADR 0015, ADR 0016).** Since v0.27
  the kernel runs in the upper half with the MMU and caches on, so exclusive loads and stores work,
  and the N-28 workarounds are gone (`fs::bare_lock`, the bump allocator, the reduced secondary
  stage). Every CPU dispatches, and programs from the VFS run at EL0 as dispatcher tasks with
  system calls, page faults, fault-tolerant user copies, user FP/SIMD state and signal handlers
  (Linux's arm64 `rt_sigframe`), and fault-tolerant
  user compare-and-exchange for futexes, static TLS, and an ASID per address space (HA-26). fork,
  clone and exec run (the child resumes from the parent's live registers, and exec enters the
  new image), and `scripts/build-musl-rootfs.sh aarch64` builds a BusyBox root with musl
  (`tools/cross/build-musl-arch.sh`) that `ARCH=aarch64 scripts/run-rootfs-tests.sh` runs:
  the BusyBox self-test, `audit_runtime_test` and `musl_runtime_test` all pass, on one and four
  CPUs, and `scripts/dev/gate.sh --rootfs` runs them. On a CPU with FEAT_BTI, guarded pages
  (PROT_BTI, and code of programs whose GNU property note asks for it) are enforced as on Linux;
  the kernel's own code is not built with BTI landing pads. `AT_HWCAP2` reports BTI there and
  otherwise only instructions that need no kernel support (not MOPS, SVE, SME, MTE). (The boot also sets SCTLR_EL1's RES1 bits and
  the EL2 state the kernel relies on, N-177, HA-14, and uses the device tree firmware passes,
  HA-01.)
- **RISC-V runs musl programs and a BusyBox root (ADR 0013, ADR 0014).** Paging is
  on, the kernel runs in the upper half, and programs from the VFS run in U-mode as dispatcher
  tasks with system calls, page faults, fault-tolerant user copies, user FP state and signal
  handlers (Linux's RISC-V `rt_sigframe`; handlers return through a trampoline page the kernel maps,
  where Linux uses its vDSO), and fault-tolerant user
  compare-and-exchange for futexes, and static TLS (`tp` at the PT_TLS block). fork, clone and
  exec run as on AArch64, and `scripts/build-musl-rootfs.sh riscv64` builds the same BusyBox root,
  which `ARCH=riscv64 scripts/run-rootfs-tests.sh` runs (in the gate, on one and four harts).

### FPU and vector registers: no SVE or RISC-V V state (N-41 part)

Every architecture switches user FP state with the task. x86_64 saves and restores its
x87/SSE/AVX state with XSAVE sized from CPUID (stage D2). Since sprint E, AArch64 saves v0-v31,
FPCR and FPSR, and RISC-V f0-f31 and fcsr, eagerly at each switch. All three kernels are
soft-float (N-178), so these registers hold only user state. RISC-V keeps `sstatus.FS` off
while the kernel runs and detects F/D from the device tree (`misa` is an M-mode CSR). Not
handled: the AArch64 SVE/SME registers and the RISC-V vector extension (V). A program using them
gets SIGILL; QEMU's default CPUs do not offer them.

### AArch64 and RISC-V kernel stack overflows hang instead of reporting (N-26 rest, sprint E)

Kernel stacks have unmapped guard pages on every architecture (AArch64 and RISC-V since v0.27),
so an overflow no longer corrupts a neighbouring frame. On AArch64 and RISC-V it faults on the
guard, but the exception is taken on the same stack, so it faults again instead of reporting:
there is no separate overflow stack yet (x86_64 takes the fault on an IST stack).

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
- **mount (N-129)** is allowed to any process holding a memory capability, and type `blockfs`
  mounts an empty RAM filesystem.

### Shared IPC regions are never reclaimed

A region's frames are freed only when it is unregistered, which is refused while any process maps
it. Every mapping is counted (a fork child's inherited ones too, and an exiting process leaves the
count since v0.27), but nothing unregisters a region once its count reaches zero, so registered
regions are kept for the life of the system. This leaks memory, but it cannot free memory that is
still in use.

### Binaries built before ADR 0009 do not run (rebuild required)

The kernel speaks Linux system call numbers since v0.27 (ADR 0009). Programs built before the
switch, against the native numbering or the patched musl, call the wrong system calls: rebuild the
BusyBox rootfs (`scripts/build-busybox-rootfs.sh headers`, `all`, `blockfs`) and the KDE stack
(`tools/cross/build-all-kde.sh`, which starts with `build-musl.sh`). The KDE binaries in the
v0.26 images also predate the shim fixes N-37 and N-42.

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

### Unix socket names

bind() makes the socket file and connect() finds the socket through it, as on Linux. Still missing:

- abstract names (a `sun_path` starting with a NUL byte) and autobind (an empty name) are EINVAL;
- sendto() on a Unix datagram socket with an address is ENOSYS (connected sends work).

### Device nodes and FIFOs made by mknod

- A device node opens one of the character devices `/dev` has (`devfs::DEVICES`: null, zero,
  random, urandom, console, tty0, the DRM and input nodes); any other number, and every block
  device number, is ENXIO, as on Linux for a number no driver registered.

### Unix sockets cannot pass Unix sockets

Passing a Unix-socket file descriptor in SCM_RIGHTS is refused with EINVAL. Passing one would need a
garbage collector for reference cycles. Other file types pass normally.

### Frame-backed Wayland buffers (DRV-PERF-01, DESK-ARCH-01; with C7)

`wl_shm` pools live in kernel heap memory and are not shared with a client, so no client can draw
through the built-in compositor. `userland/libwayland/wayland-client.c` is an early prototype of
such a client, built by no script: it creates kernel surfaces with the attached buffer's size and
pool at commit, but its pool memory is its own anonymous mapping, never the kernel pool, so
nothing it draws reaches the screen until pools are shared. KDE uses upstream libwayland over Unix
sockets instead.

### Capability lookup in debug builds

`cap_lookup` measures 97 ns in a dev build: within the 100 ns target, but only just. A release
kernel measures 29 ns (2026-10-15, `docs/PERFORMANCE-REPORT.md`, N-281).

### Hardening features are not active (N-145, N-146, N-15; planned v0.28, sprint G)

Present as code but not in effect, despite older documentation:

- **KPTI:** a shadow page table is built, but CR3 is never switched on entry or exit, and the
  shadow maps the kernel image user-accessible. On CPUs affected by Meltdown, a process can read
  kernel memory, including the physical map. The shell's `kpti` command reports it inactive.
- **KASLR:** the kernel is linked and loaded at a fixed address.
- **Stack canaries:** no stack-protector instrumentation is compiled in.
- **Retpoline, IBRS/eIBRS, IBPB, RSB filling, MDS clearing, UMIP:** not enabled (HX-23). There is
  no early microcode loading either (HX-24), so fixes shipped as microcode depend on the firmware.
- **SMEP/SMAP:** not enabled (N-15).
- **AArch64:** no branch-predictor or BHB hardening on entry from EL0 and no SSBD (HA-30).

### Network sockets do not work from user programs (N-64 to N-69; planned v0.28, sprint F)

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
two processes mapping one file do not see each other's changes. Memfds and tmpfs files (so
everything in `/dev/shm`, where musl's `shm_open` and LTP keep shared memory) are the exception:
their pages are the file, and every mapping maps them (N-230).

### Program loading (ADR 0010)

- **No address-space randomisation:** PIE programs load at `0x5555_5555_4000`, the interpreter and
  mmap regions at the bottom of the mmap area, stacks at the top (sprint G).
- **No vDSO:** `clock_gettime` and friends are real system calls; the auxiliary vector has no
  AT_SYSINFO_EHDR.
- **Image size:** a program (with its BSS) larger than 1 GiB is refused with ENOMEM: every page is
  given a frame at exec, as BSS is not mapped lazily yet.
- **Dynamically linked programs** run with musl's loader, which the BusyBox root filesystem ships
  with the shared C++ runtime (`libstdc++.so.6`, `libgcc_s.so.1`). Programs built against glibc
  do not run: there is no glibc and no `ld-linux-x86-64.so.2`.
- **Shared file pages** exist for BlockFS files only; private mappings of files on other
  filesystems are copies. Cached pages are never evicted under memory pressure; a file's pages stay
  cached until it is written, truncated or deleted (v0.29).

### Memory mapping semantics (N-140 to N-143, N-88; sprints D and G)

Page protections now follow `prot` (fixed in v0.27, N-132 to N-137). What remains:

- **`MAP_SHARED` with huge pages** (`MAP_HUGETLB`) is still copied at `fork`; ordinary
  `MAP_SHARED` memory stays shared (fixed in v0.27, N-140). File-backed `MAP_SHARED` shares the
  pages between processes but is not written back to the file (no page cache, v0.29).
- **Huge pages are not split:** `munmap` or `MAP_FIXED` over part of a 2 MiB mapping fails with
  EINVAL (`MAP_FIXED` replacement and `munmap` across mappings work since v0.27, N-141).
  `mprotect` of part of a mapping does not split it (N-135 remainder).
- **Memory is allocated eagerly** at `mmap` time, not on first touch (v0.29 memory work).
- **A memfd or tmpfs file cannot shrink under a shared mapping:** `ftruncate` that would drop a page still
  mapped `MAP_SHARED` fails with EBUSY. Linux shrinks it and unmaps those pages (access raises
  SIGBUS); mappings cannot be invalidated here yet (v0.29 memory work).
- **A shared mapping of a memfd or tmpfs file must lie within the file:** `mmap(MAP_SHARED)`
  of pages past the file's last page fails with EINVAL. Linux maps them and raises SIGBUS when a
  page wholly beyond the end is touched; that needs the same mapping invalidation. (Before
  v0.28 such a mapping was a private copy, so it never shared memory either.)
- **RAM above the eighth memory region is lost (N-142, HX-07).** Only the eight largest usable
  regions are used; UEFI firmware on real machines reports 50 to 150.
- **POSIX shared memory (N-88)** has no access control, and unlinked names can still be opened.
  (`ftruncate` of a mapped object is now refused rather than freeing frames under the mapping.)

### Security policy features are partial (N-150 to N-152, N-88)

- The TLS 1.3 client does not verify certificate signatures or the server's CertificateVerify.
- MAC labels every file and user process alike; its capability step only logs, once per access,
  which floods the audit ring.
- `prctl` implements only the name and timer-slack options; everything else, including
  `PR_SET_NO_NEW_PRIVS` and `PR_SET_SECCOMP`, fails with EINVAL (fixed in v0.27, N-151). The
  seccomp filter itself is never consulted.
- POSIX shared memory objects have no access control (N-88).

### Accounts and passwords (N-131, N-153)

Accounts live in `/etc/passwd` and `/etc/shadow` (SHA-512-crypt, the form musl's crypt reads),
loaded at boot and rewritten on every change (`security::accounts`). There is no built-in password:
until one is set -- `passwd` on the kernel console, or `VERIDIAN_ROOT_PASSWORD` when building the
KDE root filesystem -- nobody can log in with a password, the screen lock included. What remains:

- **The kernel console has no login.** Whoever can type at the serial or VGA console runs the
  kernel shell, with full control; `su` and `sudo` there are stubs.
- The failed-attempt lockout is kept in memory, so a reboot clears it.
- `useradd` gives a user a group ID equal to its user ID but writes no `/etc/group` entry, and the
  supplementary groups of `/etc/group` are not applied at login.
- TOTP does not follow RFC 6238, and pam_veridian cannot check a second factor: an account with
  MFA enabled fails PAM authentication.
- A line of `/etc/veridian/mfa` with no `:` names no account, so it is only counted as malformed
  (the boot log says so and the account files are not saved); the account it was meant for keeps
  password-only login. A line naming an account with a bad secret, or an unreadable file, fails
  closed.
- New hashes use 1000 rounds in dev builds (5000, the standard count, in release builds); the count
  is stored with each hash.

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

### Runs only under QEMU (planned v0.28: sprint H and the open E items)

Every architecture depends on QEMU-specific addresses and devices, and AArch64 and RISC-V can
only be loaded as ELF files at one fixed address. Real-hardware requirements and the bring-up
order are in `ref-docs/HARDWARE-X86_64.md`, `ref-docs/HARDWARE-AARCH64.md` and
`ref-docs/HARDWARE-RISCV64.md`. In short:

- **x86_64:**
  - **Boot devices.** Needs a 16550 at 0x3F8 and an 8254 PIT; the console reads endless 0xFF
    bytes without the UART, and timer calibration can hang (HX-01, HX-03).
  - **Memory and CPUs.** The heap is sized from RAM since N-278 (32 MiB at least, so 256 MiB
    boots; the BlockFS runtime suite uses 2 GiB), but there is still an xAPIC
    handover with the I/O APIC at 0xFEC00000, 8-bit APIC IDs and at most 16 CPUs
    (HX-04, HX-05, N-174).
  - **Buses and ACPI.** PCI configuration space through port I/O only (no ECAM), and BAR sizing
    mishandles 64-bit BARs (HX-11, HX-12). There is no AML interpreter, so interrupt routing,
    sleep states and power buttons are unavailable (HX-28).
  - **Power and controllers.** Power-off uses QEMU's debug-exit port (HX-15). There is no
    USB-legacy or AHCI firmware handoff (HX-18, HX-29).
  - **Device coverage.** Display is the UEFI framebuffer or virtio-gpu only (HX-31).
- **AArch64:**
  - **Boot protocol.** Boots as an ELF or as an arm64 Image with the device tree in x0 (HA-01,
    HA-03), from any 2 MiB-aligned load base (HA-04, checked at a second address in the gate), but
    the early map treats the first GiB as MMIO and the rest as RAM, which only QEMU `virt` satisfies
    (ADR 0015, HA-17), and it has no PE/COFF header for UEFI.
  - **Platform devices.** Hard-codes the QEMU PL011 UART and GICv2 addresses (HA-05, HA-07), and
    supports no GICv3 or PCIe (HA-20). RAM comes from the device tree's memory nodes, less its
    reservations, since v0.27 (HA-17); without a device tree 128 MiB is assumed.
  - **Caches and CPU start.** Maintains the caches for its boot page tables, for code it writes
    for user programs (cleaned to the point of unification unless CTR_EL0.IDC, instruction caches
    invalidated unless CTR_EL0.DIC), and since v0.27 for DMA buffers of devices the device tree
    does not mark `dma-coherent` (ADR 0019; virtio, ramfb). QEMU models no caches, so none of it
    is tested against stale data. It can start secondary
    CPUs only through PSCI, not the Raspberry Pi 4's spin-table (HA-22).
- **RISC-V:**
  - **Boot.** Runs only at its link address, physical 0x8020_0000, so boards whose RAM starts
    elsewhere (VisionFive 2: 0x4000_0000) cannot boot it yet (HR-02). It has an Image header for
    U-Boot `booti` since v0.27 (HR-03), but no PE/COFF header for EFI. (BSS is zeroed and a hart
    lottery runs since v0.27, HR-01/HR-19.)
  - **Fixed addresses.** Writes to 0x1000_0000, a clock controller on the HiFive Unmatched (HR-04).
    (RAM comes from the device tree since v0.27, HR-07.)
  - **Paging.** On since v0.27, in Sv48 when the hart has it and Sv39 otherwise (ADR 0018;
    tested on QEMU with `sv48=off`, not yet on a board). Sv57 is not used. What the kernel places
    is below 256 GiB on every hart, programs can map above it on Sv48 as on Linux, and RAM above
    128 GiB is not used. All harts are assumed to have the boot hart's mode.
  - **Interrupts.** Computes the PLIC context wrongly on boards whose hart 0 is a monitor core
    (HR-12) and treats device interrupts as fatal (HR-31).
  - **Privileged registers.** The unused FP save/restore code reads `misa` and the FP
    initialisation writes `mstatus`, M-mode registers that trap in S-mode (HR-20, HR-21); fixed
    with FP state switching in sprint E. `sstatus.FS` is Off in the kernel since v0.27.
  - **Missing support.** No PCIe, no DMA maintenance for non-coherent devices on harts without
    Zicbom (the T-Head vendor instructions), and no shutdown or reboot path
    (HR-24, HR-34, HR-38).

Sprint E fixes the boot-critical AArch64 and RISC-V items together with turning the MMU on;
sprint H covers the rest, in the hardware order given in the plan.

## Measurement caveat

Before v0.26.0, every in-kernel benchmark divided TSC ticks by 2 (an assumed 2 GHz clock). On the
3.6 GHz reference machine, every older benchmark figure in this repository overstates time by 1.8x.
`docs/PERFORMANCE-REPORT.md` explains the conversion.

# Porting Software to VeridianOS

VeridianOS runs Linux programs built against musl. Its only user ABI is Linux's: the system
call numbers come from Linux's tables (ADR 0009; `abi/linux/syscall_64.tbl` on x86_64,
`abi/linux/syscall.tbl` on AArch64 and RISC-V), the structures and errno values are Linux's,
and the kernel's behaviour is checked against Linux 7.3 with LTP, the Open POSIX Test Suite and
musl's libc-test. Porting a program is therefore mostly cross-compiling it with the musl
toolchain and knowing where VeridianOS still differs from Linux.

Where to look for detail:

- per-call coverage: `docs/compat/LINUX-SYSCALL-COVERAGE.md` (x86_64), `-AARCH64.md`, `-RISCV64.md`;
- everything that differs from Linux: `docs/KNOWN-LIMITATIONS.md` (the authority; this guide
  summarizes the parts that matter most when porting);
- conformance results: `tests/compat/expected-<arch>.tsv` and `tests/compat/README.md`.

LTP's `syscalls` suite passes 741 (x86_64), 730 (AArch64) and 726 (RISC-V) of 1,552 tests, Open
POSIX 1,640, 1,637 and 1,635 of 1,965, and libc-test 527, 529 and 529 of 560 (full runs of
2026-11-05, `CHANGELOG.md` and `docs/compat/COMPATIBILITY-PLAN.md`).

## Toolchains

Use the musl toolchains. `tools/cross/veridian-paths.sh` defines one per architecture (every
path can be overridden from the environment; `VERIDIAN_PREFIX` defaults to `/opt/veridian`):

| Architecture | Triple | Toolchain | Sysroot |
|--------------|--------|-----------|---------|
| x86_64    | `x86_64-veridian-linux-musl`  | `/opt/veridian/musl-toolchain`         | `/opt/veridian/musl-sysroot`         |
| AArch64   | `aarch64-veridian-linux-musl` | `/opt/veridian/musl-toolchain-aarch64` | `/opt/veridian/musl-sysroot-aarch64` |
| RISC-V 64 | `riscv64-veridian-linux-musl` | `/opt/veridian/musl-toolchain-riscv64` | `/opt/veridian/musl-sysroot-riscv64` |

They hold stock musl 1.2.6 (no system call patch since ADR 0009), Linux 7.2 UAPI headers,
binutils 2.47 and GCC 16.2 for C and C++ with the shared runtimes (`libgcc_s`, `libstdc++`).
The compiler is `<triple>-gcc` (for example `aarch64-veridian-linux-musl-gcc`). Build them with:

```bash
tools/cross/build-musl.sh && tools/cross/build-musl-toolchain.sh   # x86_64
tools/cross/build-musl-arch.sh aarch64                             # or riscv64 (needs qemu-user)
```

Programs link dynamically by default against `/lib/ld-musl-<arch>.so.1`; `-static` and
`-static-pie` work too. The toolchain script's `verify` step builds and runs static and dynamic
C and C++ programs and a `dlopen`.

There is also a toolchain for the native libc (`scripts/build-cross-toolchain.sh`, triple
`<arch>-veridian`, prefix `/opt/veridian/toolchain`). Only the x86_64 BusyBox root uses it; do
not port software with it. The CMake and Meson files under `toolchain/` are for that toolchain,
not for musl (see below).

## Build systems

**autotools.** Pass both the host and the build machine:

```bash
./configure --host=x86_64-veridian-linux-musl --build="$(cc -dumpmachine)" --prefix=/usr
```

With `--host` alone, configure decides whether it is cross compiling by running a test program,
and a build host with musl's loader installed runs it against the host's libraries, so later
answers are the host's (`tools/cross/lib/cross-env.sh`). `scripts/build-compat-rootfs.sh` (LTP's
configure) is a working example. On VeridianOS itself `config.guess` needs nothing:
`/usr/share/config.site`, named by `CONFIG_SITE`, which init, the kernel shell and
`/etc/profile` set, gives the build system as `<machine>-unknown-linux-musl`. Under `env -i`, or
with a `--prefix` other than `/usr` or `/usr/local`, pass `--build`.

**CMake.** `tools/cross/cmake-toolchain-veridian-deps.cmake` is the musl toolchain file
(`CMAKE_SYSTEM_NAME Linux`, the musl sysroot, `pkg-config` confined to it, and
`veridian-run-target` as `CMAKE_CROSSCOMPILING_EMULATOR`). It is written for x86_64
(`CMAKE_SYSTEM_PROCESSOR` and the paths); copy and adjust it for the other architectures.
`toolchain/cmake/veridian-<arch>.cmake` use the native-libc toolchain.

**Meson.** `tools/cross/meson-cross-veridian.txt` is a template that `tools/cross/lib/cross-env.sh`
fills in (`system = 'linux'`, `veridian-run-target` as `exe_wrapper`); its `[host_machine]` is
x86_64. `toolchain/meson/*.ini` use the native-libc toolchain.

**pkg-config.** There is no wrapper: set `PKG_CONFIG_LIBDIR` to the sysroot's
`usr/lib/pkgconfig:usr/share/pkgconfig` and `PKG_CONFIG_SYSROOT_DIR` to the sysroot, and unset
`PKG_CONFIG_PATH`, as `cross-env.sh` does, so no host library leaks in.

`veridian-run-target` (written by `cross-env.sh`) runs a VeridianOS program on the build host
through musl's loader with only the sysroot's libraries; it works for x86_64 programs only.

## Getting a program onto a disk

The root filesystem images are BlockFS disks the kernel mounts as `/`:

- **AArch64 and RISC-V:** `scripts/build-musl-rootfs.sh aarch64|riscv64` builds BusyBox, init
  and the test programs into `target/rootfs-musl-<arch>/` and the image
  `target/rootfs-musl-<arch>.img`. Add a compile line to its `build_programs()`, or copy a
  binary into the directory and run the script again.
- **x86_64:** `scripts/build-busybox-rootfs.sh all` then `blockfs` builds
  `target/rootfs-busybox-build/` and `target/rootfs-blockfs.img`; musl programs are built in its
  `if [ -x "$musl_cc" ]` section. (`headers` first after a change to the native libc.)
- **Conformance suites:** `scripts/build-compat-rootfs.sh <arch>` adds LTP and libc-test to
  either root as `target/rootfs-compat-<arch>.img`.

Boot commands are in `CLAUDE.md`/`AGENTS.md`; `scripts/dev/guest.py` boots an AArch64 or RISC-V
image and runs commands at its shell (`IMAGE=` picks another image).

## What differs from Linux

The full list, with tracking IDs, is `docs/KNOWN-LIMITATIONS.md`. The differences a port most
often meets:

**Binaries.** glibc programs do not run (there is no `ld-linux-x86-64.so.2`); rebuild them
against musl. Binaries built for VeridianOS before ADR 0009 (v0.27) must be rebuilt. There is no
ASLR and no vDSO (no `AT_SYSINFO_EHDR`), and programs over 1 GiB are refused. W^X is enforced: a
program that needs writable and executable memory (a JIT) carries the `wxneeded` note
(`<veridian/wxneeded.h>`, or `veridian-elfctl set wxneeded FILE`; ADR 0017).

**Threads and signals.** musl's pthreads, TLS, futexes, `sigaction`, `sigaltstack` and
real-time signals work on all three architectures. Signal handlers save only the x87/SSE state
on x86_64 (AVX upper halves are lost across a handler), there is no SIGIO/`O_ASYNC`, and
AArch64 SVE/SME and the RISC-V vector extension are not supported (SIGILL). rseq is ENOSYS, as
without CONFIG_RSEQ.

**Files.** The root is BlockFS: writes stay in memory until `sync`, there is no journal, and
directories hold at most 12 blocks of entries. `/tmp` is a directory on the root; `/dev/shm` is
a tmpfs. `mount(2)` can make tmpfs, ramfs, proc and devtmpfs only; ext4 and FAT cannot be
mounted. `/sys` is a few stub files, not sysfs. A `MAP_SHARED` mapping of a regular file is a
private copy except on tmpfs, memfds and `/dev/shm`. Timestamps have one-second resolution
(`tv_nsec` is 0); `/dev/fd` and `/proc/self/fd` do not exist; `O_TMPFILE` fails on BlockFS.

**Terminals.** PTYs answer only the window-size ioctls, so `tcgetattr` on a PTY fails; job
control is partial.

**Networking.** AF_UNIX stream and datagram sockets with path names work (no SOCK_SEQPACKET,
no abstract names, no Unix sockets in SCM_RIGHTS). AF_INET is not usable yet: UDP datagrams do
not reach the socket layer and TCP has no real handshake, retransmission or close (sprint F).
AF_INET6 and AF_NETLINK are EAFNOSUPPORT. A program that needs IP networking cannot be ported
until sprint F.

**Namespaces and containers.** UTS, IPC, cgroup, time, mount, PID, user and network namespaces
exist (ADR 0022); a new network namespace has only a loopback interface that is down and cannot
be brought up, and creating a user namespace needs CAP_SYS_ADMIN unless root sets
`/proc/sys/kernel/unprivileged_userns_clone` to 1.

## Credentials and privilege

There are two kinds of capability:

1. **Credentials and Linux capabilities (ADR 0020).** A process has Linux's real, effective,
   saved and filesystem user and group IDs and supplementary groups, and each thread Linux's
   capability sets (effective, permitted, inheritable, bounding, ambient, securebits). Every
   privilege check consults them as Linux does, in the user namespace that owns the object
   (file permissions, ownership, signals, the set*id calls, and so on). The set*id fixups,
   `capget`/`capset`, the capability `prctl` options, exec's rules and file capabilities
   (`security.capability`) follow Linux, so libcap, `setcap`-ed programs and services that drop
   privilege work unmodified.
2. **Object capabilities.** The native IPC and memory calls are authorised by unforgeable
   capability tokens (`kernel/src/cap/`), inherited on `fork()` and filtered on `exec()`. Linux
   privilege checks also need one for the object they act on (ADR 0024): `mount` takes
   CAP_SYS_ADMIN and MODIFY on the mount namespace; the namespace calls, setting the clocks, and
   signalling, tracing, rescheduling or limiting another process need theirs likewise (EPERM
   without), and changing the machine as a whole -- writing a sysctl (EACCES), the package
   calls, going past RLIMIT_NPROC as root or with CAP_SYS_RESOURCE (EAGAIN) -- needs MODIFY on
   the `System` object. File access needs READ on the mount namespace, and WRITE as well to change anything
   (EACCES without; EPERM for chmod, chown and the other owner checks, the owner included); a
   device node from `mknod` needs CREATE (EPERM). A descriptor opened before a drop keeps
   working, as one opened before a chmod does, so a launcher that wants a program to have no
   file access drops before opening what the program inherits, or closes it (or sets
   close-on-exec). A program the kernel starts holds all of them, so a Linux program sees no
   difference unless its launcher dropped some or some of their rights (`CapabilityDrop`;
   `veridian_capability_drop()` in `<veridian/capability.h>`, ADR 0026). For example, MODIFY
   taken off the mount namespace and the user namespaces above it, and CREATE off the user
   namespace (so it cannot make a user namespace of its own to mount in), gives root that cannot
   mount but still uses files; WRITE and CREATE off as well give root that can only read files.

`setuid`/`setgid`, `access()` and `stat().st_mode` behave as on Linux, and a program that needs
one privilege can run as an ordinary user with a file or ambient capability instead of as root.
The kernel console's shell runs with every capability.

## The system name (uname)

uname reports the release `7.3.0-veridian` to every program, and the system name `VeridianOS`
unless the program runs in Linux mode, where it is `Linux` (ADR 0021). Fork keeps the mode.
Exec keeps it when the process chose it (the launcher or the prctl); a mode that came from a
marked program or a glibc binary ends when that program execs another, which decides again.

- **autoconf `./configure`** needs nothing (`config.site`, above).
- **Tools that branch on the name at run time** (CMake, Python's `platform` and `sysconfig`,
  Node, the JDK) need Linux mode. Run them under the launcher (`linux-uname cmake ..`,
  `linux-uname sh` for a whole session), or mark the program once:
  - build it with `#include <veridian/linux-uname.h>` in one source file (from musl builds, add
    `-idirafter userland/libc/include`);
  - or add the note to a built binary: `veridian-elfctl set linux-uname FILE` (run it after
    `strip`; `tools/veridian-elfctl`).
  A marked program does not pass the mode to what it execs; use the launcher for a session.
- **At run time**: `prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, PR_VERIDIAN_UNAME_LINUX, 0,
  0)` enters the mode and `PR_VERIDIAN_UNAME_GET` reads it (Linux answers EINVAL).
  `linux-uname` without arguments prints the current mode.

## Ports

`ports/<name>/Portfile.toml` describes a package: `[port]` (name, version, description,
homepage, license, category, `build_type`), `[sources]` (URLs and SHA-256 checksums, or a
`local_path`), `[dependencies]` (build and runtime), `[build]` (shell steps using `$PKG_DIR`,
`$SRC_DIR`, `$BUILD_DIR` and `$VERIDIAN_TARGET`, and patches) and optionally `[native_build]`.
`ports/README.md` describes the fields. The ports are binutils, gcc, cmake, gdb, llvm, make,
meson, ninja, pkg-config and three in-tree programs (edit, libcurses, sysinfo).

Nothing builds a port from its Portfile yet: the kernel's `pkg` module parses Portfiles, but its
step runner does not execute commands. Build software with the toolchains and scripts above; the
Portfiles record the versions, sources and steps.

## Common problems

- **A configure test runs on the host.** Pass `--build="$(cc -dumpmachine)"` with `--host`.
- **The build finds host libraries.** pkg-config is not confined to the sysroot: set
  `PKG_CONFIG_LIBDIR` and `PKG_CONFIG_SYSROOT_DIR` and unset `PKG_CONFIG_PATH`.
- **`config.sub` does not know the system.** Use the musl triple (`*-linux-musl`), which every
  `config.sub` knows, not `*-veridian`.
- **A program fails with ENOSYS or EINVAL on a call Linux supports.** Look the call up in the
  coverage table and in KNOWN-LIMITATIONS; most gaps are listed there with their plan.
- **CMake, Python or Node takes VeridianOS for an unknown system.** Run it in Linux mode (uname,
  above).
- **A JIT dies on its first generated code.** Mark it `wxneeded` (W^X, above).
- **An old binary crashes at once.** It was built with the pre-ADR-0009 numbers; rebuild it.
- **Data is gone after a reset.** BlockFS keeps writes in memory until `sync`.

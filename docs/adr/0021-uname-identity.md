# ADR 0021: what uname reports, and when the system calls itself Linux

- Status: accepted
- Date: 2026-10-27
- Tracking: v0.28.0 queue item 2 (first commits of the statfs/fstatfs group). The maintainer
  chose on 2026-10-27 that the release is `7.3.0-veridian`, and that `sysname` says `Linux` only
  where build scripts and runtimes need it, staying `VeridianOS` otherwise, with `nodename`
  staying `veridian`.

## Context

uname(2) reports `VeridianOS`, `veridian`, the crate version (`0.27.0`), `#1 SMP` and the machine.
`/proc/version` still says `VeridianOS version 0.5.0 (gcc 16.2.0)`, and `/proc/sys/kernel/ostype`,
`osrelease` and `version` do not exist.

Two kinds of programs read these at run time:

- **The release.** LTP compares it with its minimum kernel (4.4); `0.27.0` makes every test print
  `TWARN: Kernel is older than minimal supported 4.4`, which turns the result into WARN even when
  every check passed (231 of 232 WARNs on x86_64, 2026-10-27). Programs that gate features on the
  kernel version read it the same way.
- **The system name.** Build systems and language runtimes branch on it, and do so at run time,
  not at build time:
  - autoconf's `config.guess` runs `uname -s` and matches `*:Linux:*:*`; an unknown name ends in
    "unable to guess system type";
  - CPython's `sysconfig.get_platform()` lowercases `os.uname().sysname`, so the platform becomes
    `veridianos-x86_64` and pip's `packaging.tags` offers no manylinux wheels;
    `platform.system()` reports it too;
  - OpenJDK sets `os.name` from `uname(&name); name.sysname`;
  - CMake sets `CMAKE_HOST_SYSTEM_NAME` from `uname -s` ("System is unknown to cmake");
  - Node's `os.type()` calls uname.

  Programs that fix the name at build time (Python's `sys.platform`, Go's `GOOS`, Rust's
  `std::env::consts::OS`, Perl's `$^O`) are not affected.

Other systems that run Linux programs settle the name like this:

- **FreeBSD's Linux emulation** answers `Linux` (sysctl `compat.linux.osname`, per jail) only to
  processes running a binary branded as Linux; FreeBSD programs see `FreeBSD`.
- **Linux** itself changes what uname reports per process with a personality flag
  (`UNAME26`, set by `setarch --uname-2.6`), kept across fork and exec.
- **gVisor and WSL 1** always report `Linux`, with a fixed old release (`4.4.0`).

VeridianOS runs Linux-ABI programs only, but many of them are its own (BusyBox, the shell, the
system tools), and those should name the system they run on.

## Decision

1. **The release is `7.3.0-veridian`** for every process: the Linux version whose sources are the
   specification the kernel is checked against. The utsname `version` field carries the VeridianOS
   version: `#1 SMP VeridianOS 0.27.0` (from `CARGO_PKG_VERSION`). The release is not part of the
   per-process choice below: LTP's checks and feature gates must see the same kernel everywhere.
2. **`nodename` stays `veridian`** (the host name; changed only by sethostname, and later per UTS
   namespace). It is not part of the choice either.
3. **`sysname` is per process: `VeridianOS` by default, `Linux` for a process in Linux mode.**
   - The mode is a process attribute, copied by fork and clone and kept across exec, as Linux keeps
     its personality flags. A build driver or a login shell in Linux mode therefore covers every
     `uname -s`, `config.guess`, `cmake` and interpreter it starts.
   - It changes only what the process is told about the system. It grants nothing, so any process
     may set or clear it, without a capability.
4. **A process enters Linux mode at exec when its main program asks for it**, in one of two ways:
   - a VeridianOS feature-control note (ADR 0017) with the new bit
     `VERIDIAN_FCTL_LINUX_UNAME` (`0x2`). `<veridian/linux-uname.h>` emits it;
     `veridian-elfctl set linux-uname FILE` adds it to an existing binary, for a runtime built
     elsewhere. The feature bits of every feature-control note in the program are combined, so a
     program can include both headers;
   - a GNU ABI tag note (`NT_GNU_ABI_TAG`, owner `GNU`) naming Linux (OS word 0): the program was
     built for GNU/Linux, so it is a Linux program by its own declaration (glibc's `crt1.o` emits
     it). This is FreeBSD's branding rule. musl does not emit the note, so VeridianOS's own
     programs are unaffected.

   Only the main program counts, as for the W^X note; an interpreter (`PT_INTERP`) or a library
   cannot set the mode. Exec never clears an inherited mode.
5. **A private prctl sets and reads the mode at run time**: `PR_VERIDIAN_UNAME` = `0x56455249`
   (`'VERI'`, following Linux's ASCII-tagged private options such as `PR_SET_VMA` and
   `PR_SET_PTRACER`).
   - `prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_GET, 0, 0, 0)` returns 1 in Linux mode, 0
     otherwise;
   - `prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, mode, 0, 0)` with `mode`
     `PR_VERIDIAN_UNAME_NATIVE` (0) or `PR_VERIDIAN_UNAME_LINUX` (1);
   - EINVAL for any other subcommand or mode, or nonzero unused arguments.

   Linux returns EINVAL for the option, so a program can probe it.
6. **`linux-uname`, a launcher in the base system**, sets the mode and execs its arguments, as
   `setarch` does: `linux-uname ./configure`, `linux-uname make`, `linux-uname sh` for a whole
   session; `linux-uname -n CMD` runs CMD in the native mode.
7. **procfs follows the caller's view.** `/proc/sys/kernel/ostype` is the caller's `sysname`,
   `osrelease` the release, `version` the version field; `/proc/version` is
   `<sysname> version <release> (veridian@veridian) #1 SMP VeridianOS <version>`.
8. **The kernel shell's `uname` builtin keeps reporting `VeridianOS`**: it is not a Linux program.

## Consequences

- LTP results count passes again, and LTP runs the tests it skipped for an "old" kernel; many fail
  until their system calls exist, which is the remaining work made visible.
- A Linux distribution's binaries (glibc-built) and any program marked with the note or run under
  `linux-uname` see `Linux` and take their Linux paths; everything else sees `VeridianOS`. A
  musl-built third-party tool that branches on the name must be marked or launched in Linux mode:
  the rootfs scripts mark the runtimes VeridianOS ships for that purpose.
- The answer to "what system is this" depends on how a program was started. That is deliberate,
  and the same trade-off FreeBSD and Linux's personality make; `prctl(PR_VERIDIAN_UNAME,
  PR_VERIDIAN_UNAME_GET)` and `/proc/self` tell a program which view it has.
- Rejected:
  - `Linux` for everyone (gVisor, WSL 1): the maintainer wants the system's own name wherever a
    program does not need Linux's.
  - Recognising callers by name or path (`config.guess`, `python3`): fragile, and a script cannot
    be told from any other shell process.
  - An environment variable read by the kernel at exec: hidden state that crosses `sudo`/`env -i`
    unpredictably and that the kernel would have to parse from every exec.
  - Reusing Linux's personality values: personality(2) is Linux ABI (and its own queue item); a
    VeridianOS meaning in it would break programs and LTP's personality tests.
- Tests: host tests for the note reader (both bits, several notes, the GNU tag, malformed notes),
  the mode's fork/exec rules and the prctl; runtime tests on all three architectures for the
  default, the prctl, inheritance across fork and exec, a marked program, the launcher and
  procfs.

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
   - The mode is a process attribute, copied by fork and clone. A mode the process *chose* (the
     prctl, and so the launcher) is kept across exec, as Linux keeps its personality flags: a
     build driver or a shell started under `linux-uname` covers every `uname -s`, `config.guess`,
     `cmake` and interpreter it starts.
   - A mode the program image asked for (item 4) holds for that image and its forks only; each exec
     decides again from the program it loads. Otherwise a glibc login shell, in Linux mode by its
     GNU ABI tag, would carry `Linux` into every native program of the session.
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
   cannot set the mode. Exec never clears a chosen mode, and always replaces one that came from the
   previous image.
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
9. **autoconf configure scripts are told the build system without Linux mode**, by a site file:
   autoconf loads `$CONFIG_SITE` (else `$prefix/share/config.site` and `$prefix/etc/config.site`)
   before it guesses the build system, and uses `build_alias` when that is set. The base system
   installs `/usr/share/config.site` setting `build_alias=$(uname -m)-unknown-linux-musl` unless
   it is already set, and a copy in `/usr/local/share` for configure's default prefix. Because
   the prefix copies are found only for those two prefixes, `CONFIG_SITE=/usr/share/config.site`
   is in the environment of every session: init's, the kernel shell's, and `/etc/profile`'s for
   login shells. `--build` and a `CONFIG_SITE` of the user's own still win. This covers the
   commonest case (`./configure && make` of a package) with `uname -s` still saying
   `VeridianOS`; Linux mode remains for tools that read the name themselves (CMake, Python,
   Node, OpenJDK).

## Consequences

- LTP results count passes again, and LTP runs the tests it skipped for an "old" kernel; many fail
  until their system calls exist, which is the remaining work made visible.
- A Linux distribution's binaries (glibc-built) and any program marked with the note or run under
  `linux-uname` see `Linux` and take their Linux paths; everything else sees `VeridianOS`. A
  musl-built third-party tool that branches on the name must be marked (`veridian-elfctl set
  linux-uname`) or launched in Linux mode. The roots built today ship no such runtime; one that
  is added later is marked when it is installed.
- `./configure` works at the prompt without Linux mode (decision 9), and with `env -i` or
  another `--prefix` only through `--build` or `linux-uname`.
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
  the prctl and `veridian-elfctl`'s second feature; the runtime test `musl_uname_linux_mode` on
  all three architectures for the default, the prctl, inheritance of a chosen mode across fork and exec,
  an image's mode ending at the next exec, programs
  marked by the header, a GNU ABI tag and `veridian-elfctl`, the launcher, procfs, and the site
  file and `CONFIG_SITE`.

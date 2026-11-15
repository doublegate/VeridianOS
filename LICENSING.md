# Licensing

VeridianOS is licensed by component (ADR 0028), as of the commit that adds this file, ahead of v0.28.0. Earlier commits and releases (through v0.27.0) remain available under MIT OR Apache-2.0, the terms they were published under.

`REUSE.toml` is the authoritative map from every path to its licence, and the licence texts are in `LICENSES/`. `reuse lint` checks both in CI.

## The split

| Component | Licence | Paths |
|---|---|---|
| The kernel, the system servers, the Linux runtime | **GPL-3.0-or-later** with the [VeridianOS user-space exception](LICENSES/LicenseRef-VeridianOS-syscall-exception.txt) | `kernel/`, `userland/init/`; as they are created, `servers/`, `drivers/`, `userland/servers/`, `userland/linuxd/`, `userland/vinit/`, and `crates/personality-linux/` (the Linux semantics the kernel and `linuxd` run) |
| The driver and server framework: the server and driver libraries, the driver SDK, the ring helpers, the server side of the interface definitions | **GPL-3.0-or-later**, without the exception | `crates/server-framework/`, `crates/veridian-driver/`, `crates/veridian-ring/`, `crates/*-server/`, `userland/server-framework/`, as they are created; server stubs the IDL compiler generates carry this licence |
| Everything programs and other projects build against as clients, and everything outside the trusted base | **MIT OR Apache-2.0**, at your option | everything else, notably `abi/`, `userland/libc`, `libm`, `ld-veridian`, `libcurses`, `libwayland`, `rust-std`, `crates/`, the user programs, the tools, the tests, the build system and the documentation |

**What the exception means.** A program that uses the kernel or servers only as a client, through their system calls (native or Linux), as a client of their IPC protocols, or through the published user-space headers and client libraries, is not a work based on them. Its licence is its own. That holds for an ordinary application and for an unmodified Linux program. The GPL does apply in full to:
- changes to the kernel or the servers;
- code linked or loaded into them;
- drivers and system servers built on the project's driver and server framework, which is GPL without the exception.

**Drivers and services.** The exception defines its terms objectively, so that whether a program is covered does not turn on what "client" means:
- **A System Component** is outside the exception whatever interfaces it uses. That is a program that does any of:
  - holds a device capability (registers, I/O ports, interrupts, DMA, IOMMU);
  - answers requests under a VeridianOS protocol in the serving role;
  - provides a service the system's own servers provide (storage, network, display, input, audio, the Linux runtime and so on);
  - holds authority over the system or other programs beyond an application's (the System object, other programs' processes or capabilities, others' namespaces, granting capabilities);
  - is built with the framework.
- Dividing a component into parts, or putting a proxy in front of it, does not change that.
- **Linux programs** written for Linux and run unmodified are always covered, whatever they do: that is what the Linux personality is for.

A System Component built with the project's driver and server framework is a work based on that framework, which is GPL-3.0-or-later. Shipping it closed needs a [commercial licence](COMMERCIAL.md).

One written entirely without the framework, copying nothing from VeridianOS, gets no permission from the exception, and no statement that it is or is not a derivative work. Whether the GPL reaches a program that copies none of VeridianOS's code is a question for copyright law, which no licence text can settle. A vendor shipping such a component closed relies on their own legal judgement rather than on VeridianOS's permission; the safe routes are the GPL or a commercial licence.

**Why the split.**
- Copyleft on the kernel and servers stops closed, modified versions being shipped. GPL-3.0's terms add a patent licence, and require that devices let their owners install modified versions.
- Copyleft on the driver and server framework means closed drivers and services built with the project's work need a commercial licence.
- The interfaces, the client libraries and the other crates stay permissive, so applications and other projects can use them freely.

**Root files:**
- `COPYING` is the GPL-3.0 text, for the copyleft parts.
- `LICENSE-MIT` and `LICENSE-APACHE` are for the permissive parts.

## Contributions and commercial licences

Every contribution is made under the [Contributor Licence Agreement](CLA.md). Contributors keep their copyright and grant the maintainer a licence that includes relicensing. That lets the maintainer offer the GPL parts under [commercial licences](COMMERCIAL.md) to those who cannot accept the GPL. The CLA check runs on every pull request.

## Third-party files in this repository

| Path | What | Licence |
|---|---|---|
| `abi/linux/syscall_64.tbl`, `abi/linux/syscall.tbl` | Linux's system call tables, vendored from Linux 7.3 (`abi/linux/VERSION`) | `GPL-2.0-only WITH Linux-syscall-note` |
| `ports/*/patches/`, `tools/busybox/patches/`, `tools/cross/*-patches/` | Patches to GCC, binutils, LLVM, BusyBox, musl, Qt, KDE Frameworks, Plasma, KWin and their dependencies | [`LicenseRef-Upstream-Patch`](LICENSES/LicenseRef-Upstream-Patch.txt): each under the licence of the files it changes |

The generated files `kernel/src/syscall/numbers.rs`, `userland/libc/include/veridian/sysno.h`, `userland/abi/syscall_numbers.rs` and `docs/compat/*` take only the names and numbers of the system calls from the two Linux tables. These are the interface programs use, which the Linux-syscall-note keeps outside the GPL. The generated files are under their directory's licence. No other Linux kernel source is copied into the tree.

Programs built for the root filesystems are fetched and built from their upstream sources, and each keeps its own licence:
- BusyBox, GPL-2.0;
- musl, MIT;
- the conformance suites: LTP (GPL-2.0), Open POSIX (GPL-2.0), libc-test (MIT);
- Qt, KDE Frameworks and the other desktop components (LGPL and GPL).

## Linux as a specification

The Linux personality follows Linux 7.3's behaviour exactly, and the Linux sources are how that behaviour is learned. They are read as a specification, never translated:
- Write down the behaviour, then implement it from that description. Use the commit message, an ADR, a test, or a comment naming the Linux file and function (for example "as Linux's `do_statmount`").
- Do not port Linux functions line by line, and do not copy Linux code, comments or data tables into the tree. The vendored tables above are the exception, and code is generated from them, not copied.
- Tests are written against observable behaviour (errno values, flags, the order of checks), which is what compatibility means anyway.

The same applies to any other GPL or LGPL system used as a reference: BusyBox, glibc, the Linux tools. This matters more now that VeridianOS's own kernel is GPL-3.0: GPL-2.0-only code cannot be combined into it at all.

## The Linux Driver Container (planned)

ADR 0030 (proposed) runs unmodified Linux drivers for GPUs, Wi-Fi, audio and laptops in a Linux Driver Container. This is a runtime built from a Linux LTS kernel, running as its own user-space program. Linux is GPL-2.0-only, which GPL-3.0 cannot be combined with, so the container must stay a separate program:
- It lives in its own directory (`drivers-linux/`) or repository, with its own GPL-2.0 `COPYING` and SPDX headers on every file. Its source is published with every binary.
- It is never linked into the kernel or any other component, and talks to them only through IPC and the shared-memory rings that every driver server uses.
- The protocol between them is VeridianOS's own: block, network, input and display messages, documented in their ADRs. It is not Linux's internal structures passed across. Separate programs that exchange "complex internal data structures" may be one program in the GPL's sense, and keeping the protocol generic and documented keeps them two.
- Code may flow into the container from the MIT OR Apache-2.0 parts only: the protocol and interface definitions, the ring layouts as they define them, and the client libraries. The GPL-3.0 parts, including the ring helpers in `crates/veridian-ring` and the rest of the driver and server framework, cannot be combined with GPL-2.0-only code, so the container implements its side of the rings from those definitions. Code never flows from the container into anything else.

## Not legal advice

This is the project's reading of the licences. A licensing lawyer reviews the exception, the CLA and the commercial terms before the first release under them.

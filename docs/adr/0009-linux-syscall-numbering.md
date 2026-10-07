# ADR 0009: Linux system call numbers as the only user ABI

- Status: accepted
- Date: 2026-10-07
- Tracking: X1 of the compatibility plan (`docs/compat/COMPATIBILITY-PLAN.md`), pulled forward
  from v0.28 into v0.27; N-33, N-103, N-120; new findings N-182 to N-250
  (`docs/audit/ABI-REVIEW-2026-10-07.md`).

## Context

The kernel numbered its system calls its own way (write = 53, exit = 11) and reached Linux
programs three ways at once:

- **Native numbers** for the native libc, rust-std and vsh, each with its own hand-written copy
  of the numbers (the dynamic loader, libwayland, several test programs and two libc files had
  further private copies, some already wrong).
- **A patched musl** whose `__veridian_remap_syscall` rewrote Linux numbers to native ones
  before the `syscall` instruction. The remap table had many wrong entries: dup3 went to pipe2,
  epoll_create1 to dup3, getppid to getpgid, sigaltstack to setsid, truncate and ftruncate were
  swapped, and more (N-182). Code that issued raw Linux numbers (libstdc++, Qt) bypassed it.
- **A per-process Linux-ABI flag** with its own translation table, which nothing ever set.

Because the remapped numbers, native numbers and raw Linux numbers overlap, the kernel guessed
which one it was looking at from argument values (a pointer-sized first argument meant
`fstatat`, a small one `epoll_ctl`). The native IPC calls 0-7 were unreachable because those
numbers had to mean read/write/open/close for Linux code (N-33). Each layer had its own
argument conventions: `mmap` packed fd and offset into one register natively, while musl passed
them in r8 and r9, so musl file mappings read fd 0 (N-183).

The compatibility plan's goal is Linux binary compatibility, and every compatible system (gVisor,
WSL1, the FreeBSD Linuxulator) speaks Linux's numbers.

## Decision

- **Linux numbers.** A call Linux also has uses its x86_64 number and name from
  `abi/linux/syscall_64.tbl` (vendored, Linux v7.2), and must implement Linux's semantics.
- **Private range.** A call only VeridianOS has (IPC, capabilities, packages, framebuffer,
  Wayland, audio, PTY helpers) gets a private number: 1024 + its pre-v0.27 native number. That
  is far above Linux's range (471 in v7.2, which grows by a few numbers a year) and clear of the
  x32 range (512-547).
- **One source.** `abi/syscalls.map` lists every call. `tools/compat/gen_syscalls.py` generates
  the kernel enum and decoder (`kernel/src/syscall/numbers.rs`), the C header
  (`<veridian/sysno.h>`), the Rust constants (`userland/abi/syscall_numbers.rs`) and the table
  (`docs/compat/SYSCALL-NUMBERS.md`). CI runs it with `--check`.
- **One name per call.** The kernel variant is the Linux name in CamelCase (`RtSigaction`); the
  C and Rust constant is `SYS_<linux name>` (`SYS_rt_sigaction`). A VeridianOS call has one
  name (`IpcSend`, `SYS_IPC_SEND`). Duplicate calls that served the same handler under a
  second number were removed.
- **One dispatcher.** `syscall_handler` decodes the number with `Syscall::try_from` and nothing
  else; an unknown number is ENOSYS. Errors are Linux errno values for every caller.
- **Stock musl.** The remap patch is gone; musl carries only unrelated patches.
- **Other architectures.** AArch64 and RISC-V use the same numbers until they get user mode
  (sprint E). Linux uses its generic table there (`write` is 64), so at that point the generator
  gains a per-architecture table.

## Consequences

- **Bug class removed.** Remap mistakes and argument-pattern guessing cannot happen. Every
  number has exactly one meaning, and the table is mechanically checked against Linux's.
- **Semantics are now a promise.** A Linux number tells callers to expect Linux behaviour. A
  review of every handler behind a Linux number found about 80 differences, from wrong errnos to
  missing permission checks. The security-relevant and convention ones were fixed with the
  renumbering (N-182 to N-189). The rest are tracked as N-190 to N-247 and scheduled into D, F
  and X2.
- **Coverage counted correctly.** The coverage table counts 133 implemented calls (118
  before), because calls that were only reachable through the remap now count.
- **IPC reachable.** The native IPC calls are reachable from user space (N-33). The endpoint
  split (N-47, N-81) still stops the synchronous path inside the kernel.
- **Rebuild required.** Every binary built before this change issues native numbers and must be
  rebuilt: the BusyBox rootfs, the test programs and the whole KDE stack. Existing images do not
  run on the new kernel.
- **Embedded machine code.** The kernel's embedded programs and signal trampolines take their
  numbers from the generated enum. The x86_64 ones were re-encoded for `write` (1) and
  `exit_group` (231).

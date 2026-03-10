# Phase 4 Completion Analysis (as of Feb 19, 2026)

## Current State
- Tiers 0–5 of self-hosting roadmap: **complete** (v0.4.9). Cross-compiler, sysroot, ports, virtio-blk (PCI), TAR loader, libc/libm, shell, PATH/env, blocking syscalls all in place.
- Phase 4A goal (“cross-compiled static C binary runs”): met for `/bin/minimal`; **/bin/sh still fails** (GP fault on iretq, multi-LOAD ELF).
- Phase 4B/4C/4D: blocked on Tier 6–7 items below.

## Highest-Priority Gap (Phase 4A)
1) **T6-0 /bin/sh ELF loading (M)**  
   - Symptom: GP fault before first syscall on 2-LOAD-segment ELF (~41 KB).  
   - Likely causes: second LOAD mapping, user PT completeness, stack/ABI alignment.  
   - Suggested steps: instrument `load_elf_binary()` to log all LOAD vaddrs/sizes; verify PTEs for both segments; check stack setup for argc/envp alignment; confirm user CR3 maps both segments.  
   - Exit criteria: `/bin/sh -c 'echo ok'` prints and exits cleanly on x86_64 QEMU.

## Tier 6 Work (Platform Completeness)
2) **T6-1 readlink() (S)** — implement VFS readlink + RamFS/BlockFS, wire syscall 152.  
3) **T6-2 Signals on AArch64/RISC-V (L)** — real signal frames/trampolines; save/restore regs/PSTATE or mstatus; handler + sigreturn path.  
4) **T6-3 Virtio-MMIO for AArch64/RISC-V (L)** — add virtio-mmio transport (`0x0a000000` on QEMU virt); reuse blk logic; boot with `-device virtio-blk-device`.  
5) **T6-4 LLVM triple patch (S)** — add `veridian` OS to Triple.cpp; patch in `ports/llvm/patches/`.  
6) **T6-5 Threads: clone/futex/pthread (XL)** — implement `clone(CLONE_VM|CLONE_FILES|CLONE_SIGHAND)`, futex wait/wake, TLS (`arch_prctl`), `libpthread.a` wrappers.

## Tier 7 Work (Self-Hosting Loop)
7) **T7-1 User-space Rust targets (S)** — add `*-veridian-user.json` for std-enabled userland.  
8) **T7-2 Rust std port (XL)** — implement `std::sys` bindings over Veridian syscalls (fs/io/process/time/thread). Depends on T6-5.  
9) **T7-3 Native GCC on VeridianOS (XL)** — build static GCC using existing cross toolchain; needs working `/bin/sh` and pthreads.  
10) **T7-4 make/ninja (L)** — cross-build static binaries; package into rootfs.  
11) **T7-5 User-space vpkg (L)** — move package manager to userland using pkg syscalls; file-backed DB in `/var/db/vpkg`.

## Recommended Execution Order
1) T6-0 (/bin/sh) — Phase 4A blocker.  
2) T6-1 (readlink) — small, unblocks compat tests.  
3) T6-3 (virtio-mmio) — enables disk I/O on AArch64/RISC-V for later tiers.  
4) T6-2 (signals AArch64/RISC-V) — correctness + parity.  
5) T6-4 (LLVM triple) — quick win; unblocks Rust target/tooling.  
6) T6-5 (threads/futex/pthread) — long pole; prerequisite for GCC-in-guest and Rust std.  
7) T7-3 → T7-4 → T7-1/2 → T7-5 (in that order).

## Verification Checklist
- `/bin/sh -c 'echo ok'` passes on x86_64 (T6-0).  
- `readlink /proc/self` returns target path (T6-1).  
- AArch64/RISC-V: signal handler runs and returns; virtio-blk MMIO loads rootfs and executes `/bin/minimal` (T6-2, T6-3).  
- `pthread_create`/join sample succeeds; futex wake/wait tests pass (T6-5).  
- Inside VeridianOS: `gcc hello.c -o hello && ./hello` works (T7-3); `make`/`ninja` usable (T7-4); `rustc --target x86_64-veridian-user` produces a runnable binary (T7-1/2).  
- `vpkg` user-space CLI installs a package via pkg syscalls (T7-5).

## Notes & Risks
- ELF loader fix (T6-0) is prerequisite for most downstream work (native toolchains, package manager).  
- Threading (T6-5) is the longest item and gates GCC-in-guest and Rust std. Plan buffer time for futex edge cases and TLS per arch.  
- Virtio-mmio (T6-3) affects AArch64/RISC-V testing throughput; prioritize to avoid x86-only blind spots.  
- Keep invariants/unsafe policy updated when touching loader, futex, and MMIO code paths.

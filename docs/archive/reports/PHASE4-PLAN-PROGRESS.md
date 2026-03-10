# Phase 4 Plan Checkpoint (Feb 20, 2026)

Source plans: `docs/phase4-status-analysis.md`, `~/.claude/plans/atomic-riding-breeze.md`.

## Status vs Tier Items
- **T6-0 (/bin/sh multi-LOAD ELF)**: Fixed by remapping main-thread stack after VAS reset; /bin/sh should now enter userland. ✔ (`64ec422`)
- **T6-1 (readlink)**: Still pending; syscall stub remains.
- **T6-2 (signals AArch64/RISC-V)**: Implemented full delivery + sigreturn restore paths. ✔ (`ff8fdce`)
- **T6-3 (virtio-MMIO blk)**: MMIO transport hooked for AArch64/RISC-V probing at QEMU virt bases. ✔ (`3301a81`)
- **T6-4 (LLVM triple)**: Added `veridian` OS triple patch. ✔ (`926ceb4`)
- **T6-5 (threads/futex/pthread)**: Clone/futex/TLS landed; libc pthreads added. Needs remaining edge checks (error paths, shared FS semantics, futex validation) and test coverage. ◑
- **T7-3/4/1/2/5 (toolchain & user-space)**: Not started in this branch; blocked on validating T6-5 and /bin/sh runtime regression tests.

## Gaps / Next Actions (recommended order)
1) **Validate /bin/sh**: QEMU boot all arches; run `/bin/sh -c 'echo ok'` to confirm T6-0. Add regression boot test.
2) **readlink()**: Implement VFS + RamFS/BlockFS support; wire syscall 152.
3) **Futex/clone hardening**: EINVAL paths (unmapped uaddr, length mismatch), proper CLONE_FS semantics, signal mask inheritance, join/detach tests.
4) **Toolchain loop**: Native GCC (T7-3) → make/ninja (T7-4) → Rust user target + std (T7-1/2) → vpkg user CLI (T7-5).

## Risk Notes
- Futex correctness is critical for pthread stability and native GCC builds; add unit + stress tests.
- Virtio-MMIO needs runtime verification on AArch64/RISC-V; ensure blk device detected and rootfs loads.
- Ensure `docs/unsafe-policy.md` stays aligned when touching loader/signal/futex code.

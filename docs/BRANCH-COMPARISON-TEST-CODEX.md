# Branch Comparison: `main` vs `test-codex`

**Generated**: February 20, 2026
**Purpose**: Document differences between the `main` and `test-codex` branches, mapping `test-codex` changes to pending Phase 4/4.5 plan items (Tiers 6-7) for future development reference.

---

## Branch Overview

| Branch | Base | Unique Commits | Files Changed | Lines |
|--------|------|---------------|---------------|-------|
| `main` | v0.4.9 (`2bb711b`) | 7 commits | ~20 files | +792/-424 |
| `test-codex` | v0.4.9 (`2bb711b`) | 1 squashed commit (`f066fe9`) | 118 files | +3,443/-428 |

Both branches share the same base: v0.4.9 (commit `2bb711b`, February 18, 2026).

---

## Commits Unique to `main` (Not in `test-codex`)

These commits represent CI/CD fixes, shell correctness, and code quality work done after the `test-codex` branch diverged:

| Commit | Type | Summary |
|--------|------|---------|
| `53863c0` | chore | Sync working tree snapshot before test-codex merge |
| `11facbe` | fix(ci) | Coverage "target was empty" -- use explicit `--target` |
| `cae214c` | fix(ci) | Codecov 0% -- correct no_std host-target coverage |
| `b1036f0` | fix(ci) | GitHub Pages OIDC 401 -- `contents: read` + `environment:` |
| `4478523` | fix(shell) | Glob `*` must not match `/`; parameter expansion pattern matching |
| `a313d9d` | chore | cargo fmt (10 files), README QEMU updates, CHANGELOG [Unreleased] |
| `1ac66f4` | fix(ci) | Remove `--bins` clippy from Quick Checks (duplicate `panic_impl`) |
| `c41da93` | fix(ci) | Guard `.cargo/config.toml` in coverage step (file is gitignored) |

**Key outcomes on `main`**:
- CI pipeline: 10/10 jobs passing
- Host-target tests: 646/646
- Codecov integration: operational
- Shell POSIX compliance: glob and parameter expansion fixes

---

## Commit Unique to `test-codex` (Not in `main`)

### `f066fe9` -- feat(kernel): squash post-origin/main work

Single squashed commit containing Tier 6 implementations: 118 files changed, +3,443 insertions, -428 deletions.

---

## Detailed Change Analysis: `test-codex` Content

### Category 1: Tier 6 Plan Items (New Kernel Features)

These are the high-value changes that directly address pending plan items T6-0 through T6-5:

#### T6-0: ELF Multi-LOAD Handling (Phase 4A Blocker)

| File | Delta | Description |
|------|-------|-------------|
| `kernel/src/elf/mod.rs` | +49 | Multi-LOAD segment handling that preserves stack mapping; test for `calculate_memory_layout` with multiple PT_LOAD segments |
| `kernel/src/process/creation.rs` | +22 | Stack preservation during multi-segment ELF loading |

**Plan impact**: Directly addresses the `/bin/sh` GP fault. If validated via QEMU, this unblocks Phase 4A completion for multi-segment binaries.

#### T6-1: readlink() Full Implementation

| File | Delta | Description |
|------|-------|-------------|
| `kernel/src/fs/blockfs.rs` | +90 | `create_symlink()` with inode-based target storage; `read_symlink()` returning `String`; `is_symlink()` type check; `readlink()` on VfsNode |
| `kernel/src/fs/ramfs.rs` | +80 | RamFS symlink node creation and readlink support |
| `kernel/src/fs/mod.rs` | +29 | VFS trait: added `readlink()` method returning `Result<String, KernelError>` |
| `kernel/src/syscall/filesystem.rs` | +105/- | `sys_readlink()` wired to VFS; improved UTF-8 error mapping throughout |

**Plan impact**: Replaces the `InvalidSyscall` stub on `main`. Full VFS-level implementation across both RamFS and BlockFS.

#### T6-2: AArch64/RISC-V Signal Delivery

| File | Delta | Description |
|------|-------|-------------|
| `kernel/src/process/signal_delivery.rs` | +387 | File grew from ~625 to 1012 lines |

**AArch64 implementation**:
- `deliver_signal_aarch64()`: saves x0-x30, SP, PC, PSTATE to `Aarch64SignalFrame` on user stack
- Sets `pc = handler`, `lr = sigreturn_trampoline_addr`
- `restore_signal_frame_aarch64()`: reads frame from user stack, restores full register context

**RISC-V implementation**:
- `deliver_signal_riscv()`: saves x1-x31, PC, mstatus to `RiscvSignalFrame` on user stack
- Sets `a0 = signum`, `ra = sigreturn_trampoline_addr`
- `restore_signal_frame_riscv()`: reads frame from user stack, restores full register context

**Plan impact**: Replaces the `Ok(false)` stubs on `main`. Both architectures now have full signal frame construction and restoration.

#### T6-3: Virtio-MMIO Disk Driver

| File | Delta | Description |
|------|-------|-------------|
| `kernel/src/drivers/virtio/mmio.rs` | NEW (246 lines) | Virtio-MMIO transport: legacy register set, magic value verification, feature negotiation with guard, split virtqueue setup, status register management |
| `kernel/src/drivers/virtio/queue.rs` | NEW (25 lines) | Shared virtqueue types: VirtqDesc, VirtqAvail, VirtqUsed ring structures |
| `kernel/src/drivers/virtio/mod.rs` | NEW (94 lines) | Virtio subsystem entry point: arch dispatch (PCI on x86_64, MMIO on AArch64/RISC-V), device enumeration at hardcoded bases |
| `kernel/src/drivers/virtio/blk.rs` | +63 | MMIO device integration: probe path for MMIO transport alongside existing PCI path |

**MMIO base addresses** (QEMU virt machine): `0x0a000000`, `0x0a002000`, `0x0a004000`, `0x0a006000`

**Plan impact**: Enables disk I/O on AArch64 and RISC-V. Required for loading rootfs.tar containing cross-compiled binaries on non-x86_64 architectures.

#### T6-4: LLVM Triple Patch

| File | Delta | Description |
|------|-------|-------------|
| `ports/llvm/patches/0001-add-veridian-triple.patch` | NEW (81 lines) | Adds `veridian` OS enum and string parsing to LLVM `Triple.cpp` |

**Plan impact**: Build-time only. Required for LLVM-based toolchains to recognize VeridianOS as a valid target OS. No QEMU validation needed.

#### T6-5: Thread Support (clone/futex/pthread)

| File | Delta | Description |
|------|-------|-------------|
| `kernel/src/syscall/futex.rs` | NEW (399 lines) | FUTEX_WAIT, FUTEX_WAKE, FUTEX_REQUEUE, FUTEX_WAIT_BITSET, FUTEX_WAKE_OP; per-process wait queues keyed by `(pid, uaddr)`; 32-bit alignment enforcement |
| `kernel/src/syscall/thread_clone.rs` | NEW (173 lines) | `sys_thread_clone()`: CLONE_VM, CLONE_FILES, CLONE_SIGHAND, CLONE_THREAD, CLONE_SETTLS, CLONE_CHILD_CLEARTID; creates thread sharing parent address space |
| `kernel/src/syscall/arch_prctl.rs` | NEW (70 lines) | ARCH_SET_FS/GET_FS/SET_GS/GET_GS; wired on x86_64, AArch64, RISC-V |
| `kernel/src/process/thread.rs` | +101 | `ThreadBuilder` for ergonomic thread creation; `ThreadFs` struct for per-thread cwd/umask; child-cleartid futex wake on thread exit |
| `kernel/src/arch/x86_64/context.rs` | +29 | TLS (FS base) field in x86_64 ThreadContext |
| `kernel/src/arch/aarch64/context.rs` | +21 | TLS (TPIDR_EL0) field in AArch64 ThreadContext |
| `kernel/src/arch/riscv/context.rs` | +18 | TLS (TP register) field in RISC-V ThreadContext |
| `kernel/src/sched/task.rs` | +3 | Thread ID field in scheduler task |
| `kernel/src/syscall/mod.rs` | +20 | Syscall dispatch entries for futex, clone, arch_prctl |
| `userland/libc/include/pthread.h` | NEW (83 lines) | POSIX pthread types: pthread_t, pthread_mutex_t, pthread_cond_t, pthread_key_t; function declarations |
| `userland/libc/src/pthread.c` | NEW (473 lines) | pthread_create, pthread_join, pthread_mutex_init/lock/unlock/destroy, pthread_cond_init/wait/signal/broadcast/destroy, pthread_key_create/delete, pthread_getspecific/setspecific |
| `userland/libc/src/syscall.c` | +66 | Syscall wrappers: `veridian_clone()`, `veridian_futex()`, `veridian_arch_prctl()` |
| `toolchain/sysroot/include/veridian/syscall.h` | +33 | Syscall numbers for SYS_clone, SYS_futex, SYS_arch_prctl |

**Plan impact**: This is the largest Tier 6 item (XL). Complete kernel-level thread infrastructure plus user-space pthread library. Required for GCC's build system (T7-3).

---

### Category 2: Code Quality and Formatting Changes

The `test-codex` commit includes `cargo fmt` and `cargo check` passes that touch many files with minor formatting adjustments. These overlap partially with `main`'s `a313d9d` commit (cargo fmt on 10 files). **Merge conflicts are expected in these files.**

Files with formatting-only changes (partial list):
- `kernel/src/crypto/*.rs` (7 files) -- import grouping, line wrapping
- `kernel/src/desktop/*.rs` (5 files) -- import grouping
- `kernel/src/drivers/*.rs` (4 files) -- import grouping
- `kernel/src/graphics/*.rs` (4 files) -- import grouping
- `kernel/src/ipc/*.rs` (6 files) -- import consolidation, line wrapping
- `kernel/src/net/*.rs` (9 files) -- import grouping
- `kernel/src/security/*.rs` (8 files) -- import grouping
- `kernel/src/pkg/format/*.rs` (2 files) -- import consolidation

---

### Category 3: Documentation and Configuration

| File | Delta | Description |
|------|-------|-------------|
| `AGENTS.md` | NEW (75 lines) | Agent configuration for Codex/multi-agent development |
| `GEMINI.md` | +145/- | Updated Gemini agent instructions |
| `CHANGELOG.md` | +18 | Changelog entries for Tier 6 work |
| `README.md` | +6 | Updated feature list |
| `.github/workflows/ci.yml` | +28/- | CI adjustments (conflicts with main's CI fixes) |
| `.gitignore` | +4/- | Additional ignore patterns |
| `docs/phase4-completion-report.md` | NEW (39 lines) | Phase 4 completion summary |
| `docs/phase4-plan-progress.md` | NEW (23 lines) | Phase 4 plan progress notes |
| `docs/phase4-status-analysis.md` | NEW (50 lines) | Phase 4 status analysis |
| `ref_docs/redox-capability-fd-bridge.md` | NEW (38 lines) | Research: Redox capability-fd bridging pattern |

---

### Category 4: Minor Bug Fixes and Improvements

| File | Delta | Description |
|------|-------|-------------|
| `kernel/src/intrinsics.rs` | +10/- | Improved compiler intrinsic implementations |
| `kernel/src/print.rs` | +21/- | Print macro improvements |
| `kernel/src/sync/once_lock.rs` | +8/- | OnceLock refinements |
| `kernel/src/integration_tests.rs` | +36/- | Updated integration test assertions |
| `kernel/src/lib.rs` | +30/- | Module declarations for new syscall files |
| `kernel/src/error.rs` | +2 | New error variant for thread operations |
| `kernel/src/elf/types.rs` | +1 | ELF type addition |
| `kernel/src/elf/dynamic.rs` | +2 | Dynamic linking constant |
| `kernel/src/services/shell/completion.rs` | +2 | Shell completion additions |
| `kernel/src/services/shell/jobs.rs` | +2 | Job control additions |
| `userland/libc/include/unistd.h` | +3 | clone/futex function declarations |

---

## Merge Strategy

### Expected Conflicts

The following files are modified on both branches and will likely conflict during merge:

| File | main change | test-codex change | Conflict type |
|------|-------------|-------------------|---------------|
| `.github/workflows/ci.yml` | CI fix (Quick Checks, coverage guards) | CI adjustments | Medium -- different sections likely |
| `.gitignore` | Cleanup (duplicates, lcov.info) | Additional patterns | Low -- additive |
| `CHANGELOG.md` | [Unreleased] section | Tier 6 entries | Low -- different sections |
| `README.md` | QEMU command updates | Feature list update | Low -- different sections |
| `kernel/src/crypto/asymmetric.rs` | cargo fmt | cargo fmt (same changes likely) | Trivial |
| `kernel/src/fs/ramfs.rs` | cargo fmt | +80 lines (symlink/readlink) + fmt | Medium |
| `kernel/src/ipc/rate_limit.rs` | cargo fmt | cargo fmt | Trivial |
| `kernel/src/ipc/shared_memory.rs` | cargo fmt | cargo fmt | Trivial |
| `kernel/src/ipc/tests.rs` | cargo fmt | cargo fmt | Trivial |
| `kernel/src/mm/vas.rs` | cargo fmt | +42 lines (modifications) | Medium |
| `kernel/src/pkg/format/compression.rs` | cargo fmt | cargo fmt | Trivial |
| `kernel/src/pkg/format/mod.rs` | cargo fmt | cargo fmt | Trivial |
| `kernel/src/services/shell/redirect.rs` | cargo fmt | import change | Trivial |

### Recommended Merge Approach

1. **Create a merge branch**: `git checkout -b merge-test-codex main`
2. **Merge**: `git merge test-codex` -- resolve conflicts
3. **For formatting conflicts**: Accept either side (both applied `cargo fmt`), then re-run `cargo fmt --all`
4. **For CI conflicts**: Keep `main`'s CI fixes (they are more recent and tested)
5. **For content conflicts**: Manual merge -- `main`'s shell fixes + `test-codex`'s new features
6. **Validate**: `cargo build` all 3 targets, `cargo clippy`, `cargo fmt --check`
7. **QEMU boot test**: All 3 architectures to Stage 6 BOOTOK

---

## Applicability Matrix: test-codex to Pending Plan Items

| Plan Item | test-codex Coverage | Merge Priority | QEMU Validation Required |
|-----------|-------------------|----------------|--------------------------|
| **T6-0**: ELF multi-LOAD | Full implementation | HIGH (Phase 4A blocker) | Yes -- /bin/sh boot test |
| **T6-1**: readlink() | Full implementation | Medium | Yes -- syscall test |
| **T6-2**: Signal delivery | Full implementation | Medium | Yes -- AArch64/RISC-V signal test |
| **T6-3**: Virtio-MMIO | Full implementation | Medium | Yes -- AArch64/RISC-V disk test |
| **T6-4**: LLVM triple | Full implementation | Low (build-time only) | No |
| **T6-5**: Threads | Full implementation | HIGH (T7-3 dependency) | Yes -- pthread test |
| **T7-1**: Rust target JSON | Not addressed | -- | -- |
| **T7-2**: Rust std port | Not addressed | -- | -- |
| **T7-3**: Native GCC | Not addressed | -- | -- |
| **T7-4**: make/ninja | Not addressed | -- | -- |
| **T7-5**: vpkg migration | Not addressed | -- | -- |

---

## Summary

The `test-codex` branch contains complete implementations for **all 6 Tier 6 plan items** (T6-0 through T6-5), totaling approximately 2,500 lines of new feature code across 12 new files and 30+ modified files. The `main` branch has CI/CD and shell correctness fixes not present in `test-codex`. A merge is the critical next step: once `test-codex` is merged and QEMU-validated, Tier 6 will be complete and development can proceed to Tier 7 (full self-hosting loop).

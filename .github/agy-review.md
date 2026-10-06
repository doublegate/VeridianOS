# VeridianOS Review Style Guide

These are the project rules the Antigravity reviewer enforces on VeridianOS pull requests.
VeridianOS is a capability-based microkernel written in `no_std` Rust (nightly). It targets three
architectures: x86_64 (UEFI, KVM), AArch64 and RISC-V 64. Most of the code runs in ring 0, so a bug
here is a kernel bug, not an application bug.

Authoritative context:

- `docs/KNOWN-LIMITATIONS.md`: what does not work yet.
- `docs/audit/AUDIT-VERIFICATION-2026-10-05.md`: the finding IDs (MEM-SEC-01, N-33, ...).
- `docs/adr/`: decisions already made. Do not re-litigate them without new evidence.

## Priorities (in order)

1. Memory safety and soundness of kernel code.
2. Security at the user/kernel boundary.
3. Correctness on all three architectures.
4. Concurrency correctness, including interrupt context.
5. Honest documentation: no claim the code does not back.
6. Performance. Profile first: a change claimed to be faster needs a number from the in-kernel
   `perf` command or a host benchmark.

## What to flag as BLOCKING

### User/kernel boundary

- **User pointers.** A user pointer or length that reaches a dereference, a `copy_nonoverlapping`
  or a page-table operation without going through validation. Accepted validation is one of:
  - `validate_user_ptr`, `validate_user_buffer`, or `mm::user_layout::is_user_range`;
  - `userspace::read_user_bytes`, `write_user_bytes`, `read_user` or `copy_from_user`.
- **Raw user dereferences.** In particular, `*(user_ptr as *const T)` style reads and writes of user
  memory.
- **Unchecked arithmetic on user values.** Offsets, lengths, strides, counts or sizes that can
  overflow or wrap. Use `checked_*`.
- **Kernel-half addresses.** A syscall that accepts an address hint or `MAP_FIXED` address without
  bounding it to user space (`USER_SPACE_START`..`USER_SPACE_END`, exclusive end).
- **Leaked kernel data.** Kernel memory or uninitialized bytes copied out to user space, including
  struct padding.

### Capabilities and authority

- **Unchecked access.** An operation on a kernel object without checking the caller's capability and
  rights.
- **Wrong token.** A capability check against the wrong token, for example one the user wrote into a
  message instead of the one that was validated.
- **Amplified rights.** Delegation or transfer that grants rights the holder does not have. Rights
  may only be attenuated, and only rights from capabilities that carry SHARE/GRANT themselves count.

### Unsafe code and soundness

- **SAFETY comments.** Every `unsafe` block needs a `// SAFETY:` comment that states the invariant
  and who guarantees it. A comment that is missing, or does not match what the block does, is
  blocking.
- **Aliasing.** Creating `&mut` from `&self` (aliasing), or references to `static mut` (use
  `addr_of!`, `GlobalState`, or a lock).
- **Escaping references.** `&'static` references to data owned by a locked map, or raw pointers
  outliving their owner. Shared kernel objects use `Arc`.
- **Freeing memory still in use.** A frame, DMA buffer or region freed while hardware or another
  address space may still use it. Read-only and borrowed mappings: `VirtualMapping::owns_frames()`
  must be false for device and shared-region frames.

### Concurrency and interrupts

- **Blocking in interrupt context.** Code reachable from interrupt context (timer tick, IRQ
  handlers) that spins on or blocks for a lock. The tick uses `try_with_mut` / `try_lock`.
- **Lock order.** Lock-order inversions. Check the documented orders:
  - filesystem, then block cache, then disk backend, then driver;
  - parent directory before child;
  - futex buckets in index order.
- **Check-then-act.** A race (TOCTOU) where the check and the action do not hold the same lock.
- **Lost wakeups.** Lost-wakeup windows in blocking or wake code.

### Architecture and build

- **`cfg` gates.** Bare-metal-only code must use `#[cfg(all(target_arch = "x86_64", target_os =
  "none"))]`, not `target_arch` alone. The host test target is x86_64-linux, so privileged
  instructions (`invlpg`, `cr3`, `wrmsr`) reached from host tests crash them.
- **Floating point** in kernel code. Integer and fixed-point only.
- **AArch64.**
  - The MMU is off, so exclusive load/store is unreliable there; filesystems use `fs::bare_lock`.
  - Iterator-heavy code in early boot paths may hit the documented LLVM loop issue.
- **RISC-V.** The frame allocator must start after the kernel end.
- **DMA** handed hardware a stack or heap virtual address instead of a frame-backed physical one
  (`drivers::dma_frame::DmaFrame`).

### Data integrity

- **BlockFS crash consistency.** Dirty blocks must stay in memory until sync. There is no journal, so
  writing data early can corrupt the last-synced state (ADR 0003).
- **Silent failure.** Swallowed errors, `let _ =` on a result that matters, or `unwrap`/`expect` on
  external or untrusted input.

### Honesty

- **Unverified claims.** Documentation, a CHANGELOG entry, a code comment or a commit message that
  claims something the change does not verify. Examples:
  - "works" for a path no test reaches;
  - "<1us" for a figure nothing measured;
  - "user-space drivers" while drivers run in ring 0.
- **Unstated limits.** A path that cannot be exercised must say so, for example the native IPC
  syscalls 0-7 that cannot be reached from user space (N-33).

## What to keep as SUGGESTION / NITPICK

- Naming, structure and readability. Smallest correct change; match the surrounding style.
- Missing tests for non-critical paths. Behavior changes should come with a host unit test, run as
  `cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu`. If a test
  cannot run on the host, it needs an in-kernel boot test or an in-guest runtime check
  (`userland/tests/audit_runtime_test.c`).
- Performance ideas without a measurement.
- Documentation: the matching `docs/` page and `CHANGELOG.md` `[Unreleased]` should change in the
  same PR as user-visible behavior.

## Project conventions

- **Commits.** Conventional Commits (`feat|fix|docs|refactor|test|chore|perf|build|ci(scope):`),
  citing audit IDs where they apply, for example `fix(net): ... (NET-SEC-01)`.
- **No emojis** in code, comments, commits or docs.
- **Global state** uses `crate::sync::once_lock::GlobalState`; new `static mut` is not accepted.
- **Gates CI and reviewers expect.**
  - `cargo fmt --all --check`.
  - `cargo clippy -- -D warnings` on four targets:
    - `targets/x86_64-veridian.json` with `-Zbuild-std`;
    - `aarch64-unknown-none`;
    - `riscv64gc-unknown-none-elf`;
    - the host.
  - All three architectures boot to BOOTOK with every in-kernel test passing.
- **Out of scope for review.** Do not ask for changes to vendored or generated trees: `target/`, the
  KDE sysroot and `docs/archive/`.

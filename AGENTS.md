# Repository Guidelines

## Project Structure & Module Organization
- `kernel/`: core microkernel crates; unsafe must be rare and documented.  
- `libs/`: shared Rust crates (capability model, IPC, drivers); keep crates small and single-purpose.  
- `drivers/`, `services/`: user-space drivers and system services; enforce capability boundaries.  
- `userland/`: user-space apps (vsh, minimal) mirroring kernel APIs.  
- `tools/`, `scripts/`: build/debug helpers (QEMU, benchmarks, hooks) — prefer reuse over new scripts.  
- `targets/`: arch JSON specs (x86_64/aarch64/riscv64); align with build matrix.  
- `tests/`: cross-arch boot/integration tests; crate-level unit tests live beside source.  
- `docs/`, `ref_docs/`: invariants, design, and status docs — update when changing interfaces or invariants.  
- Generated outputs live in `images/`, `artifacts/`, `release-artifacts/`; avoid manual edits.

## Build, Test, and Development Commands
- Preferred: `./build-kernel.sh all dev` (or `x86_64|aarch64|riscv64 [dev|release]`) for full builds.  
- `just build` / `just build-<arch>`: release kernel for a target (uses target JSON).  
- `just run` / `just run-iso`: boot in QEMU (x86_64 defaults to ISO flow).  
- `just debug-<arch>`: GDB-ready QEMU session for the target architecture.  
- `just test`, `just test-<arch>`, `just test-all`: run integration/boot tests via scripts.  
- `just fmt`, `just clippy`, `just ci-checks`: format, lint (warnings as errors), and run CI bundle.  
- `just build-iso`: produce `build/veridian.iso`; `just clean` removes build artifacts.  
- First-time setup: `just setup` then `just install-hooks`; toolchain pinned in `rust-toolchain.toml` (nightly-2025-11-15 override).

## Coding Style & Naming Conventions
- Rust 2021 edition; `rustfmt.toml` enforces 100-col, 4 spaces, reordered imports.  
- Name patterns: `snake_case` (modules/functions), `CamelCase` (types), `SCREAMING_SNAKE_CASE` (consts).  
- Unsafe policy: every unsafe block needs `// SAFETY:` explaining invariants and preconditions; prefer the `GlobalState` pattern over `static mut` (only 7 justified remain). See `docs/unsafe-policy.md` for the binding rules.  
- Public APIs document invariants and capability expectations.  
- Run `just fmt` and `just clippy` before pushing; CI treats warnings as failures.

## Testing Guidelines
- Fast unit tests live beside source; cross-arch/boot tests are in `tests/` plus `scripts/test-*.sh`.  
- `just test-verbose` runs `cargo test --all -- --nocapture`; note that some no_std automation is limited — rely on QEMU boot tests.  
- Expectation: all three architectures reach Stage 6 with 29/29 boot tests (includes `fbcon_initialized` and `keyboard_driver_ready`).  
- When touching arch-specific paths, run the matching `just test-<arch>` and report coverage (Codecov wired).  
- Add regression tests for kernel invariants, IPC, and syscall surfaces when fixing bugs.

## Commit & Pull Request Guidelines
- Commit messages follow conventional commits with scopes, e.g., `fix(ci): handle empty coverage target — explain` (see recent history).  
- Install git hooks via `just install-hooks`; `just check-commit-msg "<msg>"` mirrors the commit-msg hook.  
- PRs should include: short summary, linked issue, architectures tested (x86_64/aarch64/riscv64), commands run, and any breaking changes or migration notes.  
- Keep diffs small and focused; update `docs/` or `ref_docs/` when changing invariants or interfaces.

## QEMU & Boot Notes
- QEMU 10.2+: never wrap with `timeout`; background + kill pattern avoids drive conflicts.  
- x86_64 must boot from the UEFI disk image (`target/x86_64-veridian/debug/veridian-uefi.img`) with OVMF and `-enable-kvm`; do **not** use `-kernel`.  
- AArch64/RISC-V boot with `-kernel`; add `-device ramfb` for display (TCG only on x86 hosts).  
- Kill stale QEMU processes before reruns (`pkill -9 -f qemu-system; sleep 3`); avoid `-cdrom` alongside pflash/disk drives.

**Known-good QEMU commands (v10.2):**

```bash
# x86_64 UEFI (serial-only)
qemu-system-x86_64 -enable-kvm \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
  -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
  -device ide-hd,drive=disk0 \
  -serial stdio -display none -m 256M

# AArch64 (serial-only)
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
  -kernel target/aarch64-unknown-none/debug/veridian-kernel \
  -serial stdio -display none

# RISC-V (serial-only, OpenSBI default)
qemu-system-riscv64 -M virt -m 256M -bios default \
  -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
  -serial stdio -display none
```

## Security & Configuration Tips
- Do not commit secrets; prefer env vars or script templates.  
- Verify toolchain with `just info`; refresh dependencies with `just audit` and `just outdated` before release changes.  
- Treat `build/`, `artifacts/`, and release outputs as disposable; regenerate instead of editing.  
- For KVM, only on trusted hosts; avoid root when running QEMU.

# Project Status

## Current Status: All Phases Complete (0-12)

**Latest Release**: v0.27.0 (October 18, 2026): process model, SMP and user mode on all three architectures
**All 13 Phases**: COMPLETE; v0.28 next (the remaining Linux system calls, then networking, hardening and real hardware)
**Tests**: 4,770 host unit tests; in-kernel boot tests and in-guest runtime suites on every architecture (below)
**CI Pipeline**: builds, clippy, host tests with coverage, boot tests on one and four CPUs, x86_64 runtime suite

What does not work yet is listed in
[Known Limitations](https://github.com/doublegate/VeridianOS/blob/main/docs/KNOWN-LIMITATIONS.md).

## Phase Completion Summary

| Phase | Description | Version | Date | Status |
|-------|-------------|---------|------|--------|
| 0 | Foundation & Tooling | v0.1.0 | Jun 2025 | COMPLETE |
| 1 | Microkernel Core | v0.2.0 | Jun 2025 | COMPLETE |
| 2 | User Space Foundation | v0.3.2 | Feb 2026 | COMPLETE |
| 3 | Security Hardening | v0.3.2 | Feb 2026 | COMPLETE |
| 4 | Package Ecosystem | v0.4.0 | Feb 2026 | COMPLETE |
| 5 | Performance Optimization | v0.16.2 | Mar 2026 | COMPLETE |
| 5.5 | Infrastructure Bridge | v0.5.13 | Feb 2026 | COMPLETE |
| 6 | Advanced Features & GUI | v0.6.4 | Feb 2026 | COMPLETE |
| 6.5 | Rust Compiler + vsh Shell | v0.7.0 | Feb 2026 | COMPLETE |
| 7 | Production Readiness (6 Waves) | v0.10.0 | Mar 2026 | COMPLETE |
| 7.5 | Follow-On Features (8 Waves) | v0.16.0 | Mar 2026 | COMPLETE |
| 8 | Next-Generation (8 Waves) | v0.16.3 | Mar 2026 | COMPLETE |
| 9 | KDE Plasma 6 Porting | v0.22.0 | Mar 2026 | COMPLETE |
| 10 | KDE Limitations Remediation | v0.23.0 | Mar 2026 | COMPLETE |
| 11 | KDE Default Desktop Integration | v0.24.0 | Mar 2026 | COMPLETE |
| 12 | KDE Cross-Compilation | v0.25.0 | Mar 2026 | COMPLETE |

## Architecture Boot Status

All three architectures boot under QEMU to Stage 6 BOOTOK and pass every in-kernel test, on one
CPU and on four. None has run on real hardware yet.

| | x86_64 | AArch64 | RISC-V |
|---|---|---|---|
| Boot | UEFI (OVMF) | direct kernel or arm64 Image | OpenSBI, direct kernel or Image |
| In-kernel tests, 1 CPU | 43/43 | 64/64 | 57/57 |
| In-kernel tests, 4 CPUs | 47/47 | 68/68 | 61/61 |
| User mode | Ring 3, native libc | EL0, musl | U-mode, musl (Sv48 or Sv39) |
| BusyBox self-test | pass | pass | pass |
| `audit_runtime_test` | 91/91 | 91/91 | 89/89 |
| `musl_runtime_test` | 80/80 | 78/78 | 78/78 |

The runtime suites run on the BlockFS root, on one and four CPUs, and on RISC-V also under Sv39.
`scripts/dev/gate.sh --boot --rootfs` runs all of it.

## Code Quality Metrics

| Metric | Value |
|--------|-------|
| Host-target tests | 4,770 passing |
| Host-test line coverage | 56.2% (Codecov) |
| Clippy warnings | 0 (`-D warnings` on all four targets) |
| `static mut` | 3 justified (early serial, boot info, per-CPU data) |
| SAFETY comments | on every `unsafe` block |
| Linux system calls implemented (x86_64) | 180 of 385 ([coverage report](https://github.com/doublegate/VeridianOS/blob/main/docs/compat/LINUX-SYSCALL-COVERAGE.md)) |
| Shell builtins | 153 |
| Desktop apps | 9 |
| Settings panels | 8 |

## Performance Benchmarks

Measured with the in-kernel `perf` command on QEMU x86_64 with KVM (i9-10850K), dev kernel,
calibrated TSC (v0.26.0 run):

| Benchmark | Average | Target |
|-----------|--------:|-------:|
| syscall_getpid | 46 ns | 500 ns |
| frame_alloc_1 (per-CPU cache) | 62 ns | 2,000 ns |
| frame_alloc_global | 272 ns | 4,000 ns |
| cap_lookup | 93 ns | 100 ns |
| ipc_stats_read | 19 ns | 100 ns |
| sched_current | 27 ns | 200 ns |

IPC latency and context-switch time are not measured yet. Dev kernels overstate anything that
copies or clears memory (N-281); method and every table:
[Performance Report](https://github.com/doublegate/VeridianOS/blob/main/docs/PERFORMANCE-REPORT.md).

## Self-Hosting Status

All self-hosting tiers (0-7) complete as of v0.5.0:
- Cross toolchain GCC 16.2 / binutils 2.47 (since 2026-10-07); native GCC 14.2 / binutils 2.43 as last built, make, ninja
- vpkg package manager
- BusyBox 208/208 tests passing
- Native compilation on VeridianOS

## KDE Plasma 6 Status

Cross-compiled from source with a musl-based pipeline for x86_64 (kwin_wayland, plasmashell,
dbus-daemon and the libraries under them). The shared-library build and a media backend are
v0.28 work; AArch64 and RISC-V builds are future work. Details:
[KDE known limitations](https://github.com/doublegate/VeridianOS/blob/main/userland/integration/KNOWN-LIMITATIONS.md).

## Verification Infrastructure

- 38 Kani proof harnesses and 6 TLA+ specifications (boot chain, IPC, memory, capabilities),
  with TLC configurations and the `scripts/verify.sh` runner. They check standalone models, not
  the kernel's own functions, and CI runs neither (N-163).

## Next Steps

- **v0.28**: the remaining Linux system call groups with LTP and musl libc-test; the full
  capability model and every namespace type; the KDE shared-library pipeline and media backend
- Then networking and storage (sprint F), hardening (G) and real hardware (H), as planned in
  [`to-dos/MASTER_TODO.md`](https://github.com/doublegate/VeridianOS/blob/main/to-dos/MASTER_TODO.md)

## Project Resources

- **GitHub**: [github.com/doublegate/VeridianOS](https://github.com/doublegate/VeridianOS)
- **GitHub Pages**: [doublegate.github.io/VeridianOS](https://doublegate.github.io/VeridianOS)
- **CHANGELOG**: [CHANGELOG.md](https://github.com/doublegate/VeridianOS/blob/main/CHANGELOG.md)
- **Discord**: [discord.gg/WGcgrnuVHt](https://discord.gg/WGcgrnuVHt)

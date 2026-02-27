# VeridianOS Testing Status

**Last Updated**: February 27, 2026
**Latest Release**: v0.5.8 -- Phase 5 Performance Optimization (~90%)

## Summary

| Category | Status |
|----------|--------|
| Kernel Compilation | All 3 architectures, zero warnings |
| Kernel Boot | 29/29 tests, Stage 6 BOOTOK (all 3 architectures) |
| Clippy | Zero warnings with `-D warnings` (all 3 targets) |
| Cargo fmt | Clean |
| CI Pipeline | 10/10 jobs passing (GitHub Actions) |
| Host-Target Unit Tests | 646/646 passing |
| Automated Integration Tests | Blocked (Rust toolchain `core` duplication) |

## Boot Testing Results

### Architecture Status (v0.5.8)

| Architecture | Build | Boot | Tests | Stage 6 | Stable Idle | Display | Notes |
|-------------|-------|------|-------|---------|-------------|---------|-------|
| x86_64 | Zero warnings | BOOTOK | 29/29 | PASS | 30s PASS | 1280x800 BGR UEFI GOP | UEFI boot via OVMF, `-enable-kvm`, `-m 2048M` |
| AArch64 | Zero warnings | BOOTOK | 29/29 | PASS | 30s PASS | ramfb | Direct kernel boot, DirectUartWriter |
| RISC-V 64 | Zero warnings | BOOTOK | 29/29 | PASS | 30s PASS | ramfb | OpenSBI boot |

All architectures reach `root@veridian:/#` interactive shell prompt.

### Boot Test Inventory (29 tests)

Tests are run during kernel bootstrap (Stage 1-6) and verified via serial output:

- Stage 1: Serial initialization, early memory
- Stage 2: Memory management (frame allocator, page tables, heap)
- Stage 3: IPC system, capability system, process management
- Stage 4: Scheduler, driver framework, security subsystems
- Stage 5: VFS, shell, package manager, performance subsystems
- Stage 6: User-space transition, init system, fbcon, keyboard driver

Tests 28-29 (`fbcon_initialized`, `keyboard_driver_ready`) added in v0.4.5.

### x86_64 Extended Testing

| Feature | Status |
|---------|--------|
| User-space entry (Ring 3) | PASS -- SYSCALL/SYSRET path verified |
| /sbin/init (PID 1) | PASS -- Runs in Ring 3 |
| Native compilation | 208/208 BusyBox sources compiled+linked (NATIVE_COMPILE_PASS) |
| Native execution | Compile + link + execute on-target (NATIVE_ECHO_PASS) |
| BusyBox 1.36.1 | 95 applets, ash shell |
| Coreutils | 6/6 (echo, cat, wc, ls, sort, pipeline_test) |
| BlockFS | 512MB persistent storage with sync/fsync |
| POSIX regex | BRE/ERE engine (1291 lines) |
| PS/2 keyboard | Polling-based input (ports 0x64/0x60) |

## Automated Testing Limitation

### Root Cause

Rust's `-Zbuild-std` with bare metal targets creates duplicate `core` library instances, causing `error[E0152]: duplicate lang item in crate 'core': 'sized'`. This prevents `cargo test` from working with the kernel crate.

### Impact

- No automated unit test execution via `cargo test`
- No CI-integrated test coverage reporting
- All verification done via QEMU boot tests and manual inspection

### Workaround

The in-kernel test framework (`kernel/src/test_framework.rs`) runs 29 boot tests during kernel initialization. These tests exercise all major subsystems and are verified by checking serial output for `BOOTOK` and the test count.

### Future Solutions

- Monitor Rust RFC developments for no_std testing improvements
- User-space test applications via the native compilation toolchain
- Hardware PMU-based performance regression testing (Phase 5.5)

## Build Verification Commands

```bash
# Build all 3 architectures
./build-kernel.sh all dev

# Clippy (all 3 targets)
cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel -- -D warnings
cargo clippy --target aarch64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target riscv64gc-unknown-none-elf -p veridian-kernel -- -D warnings

# Format check
cargo fmt --all -- --check
```

## Performance Benchmarks

In-kernel micro-benchmark suite (`perf` shell builtin) with 7 benchmarks and Phase 5 targets. See [PERFORMANCE-BENCHMARKS.md](PERFORMANCE-BENCHMARKS.md) for details.

## Tracepoint System

Software tracepoints (`trace` shell builtin) with 10 event types and per-CPU ring buffers. 8 of 10 events wired as of v0.5.8. See [PERFORMANCE-TUNING.md](PERFORMANCE-TUNING.md) for details.

---

**See also**: [Performance Benchmarks](PERFORMANCE-BENCHMARKS.md) | [Performance Tuning](PERFORMANCE-TUNING.md) | [Phase 5 TODO](../to-dos/PHASE5_TODO.md)

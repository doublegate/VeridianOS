# Testing

VeridianOS has 4,770 host-target unit tests, in-kernel boot tests on every architecture (x86_64 43, AArch64 64, RISC-V 57; more with four CPUs) and in-guest runtime suites on the BlockFS root.

## Test Commands

```bash
# Host-target unit tests (`--lib` matters: plain `cargo test` also builds the bare-metal
# binary and test targets for the host and fails on a duplicate lang item)
cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu

# Format check
cargo fmt --all --check

# Lint all bare-metal targets (x86_64 uses the custom target JSON, as CI does)
BS="-Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem"
cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel $BS -- -D warnings
cargo clippy --target aarch64-unknown-none-softfloat -p veridian-kernel $BS -- -D warnings
cargo clippy --target riscv64imac-unknown-none-elf -p veridian-kernel $BS -- -D warnings
cargo clippy --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu -- -D warnings
cargo clippy --lib --all-features -- -D warnings   # feature-gated code such as `trace`

# Everything: the above plus builds, QEMU boot tests and the runtime suites
scripts/dev/gate.sh --boot --rootfs
```

## Testing Strategy

### Unit Tests

Host-target tests (4,770) run with
`cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu` and cover
the kernel subsystems: memory management, IPC, scheduling, capabilities, processes, filesystems,
cryptography, the desktop and more. Code that only runs on bare metal is gated with
`#[cfg(all(target_arch = "x86_64", target_os = "none"))]`.

### Boot Tests

Every architecture boots in QEMU to Stage 6 BOOTOK and runs the in-kernel tests: x86_64 43/43,
AArch64 64/64, RISC-V 57/57, and 47, 68 and 61 with four CPUs. `scripts/boot-test.sh <arch>` runs
one; CI and the gate run all of them, on release kernels too.

### Runtime Suites

On the BlockFS root of each architecture, `scripts/run-rootfs-tests.sh` runs the BusyBox
self-test, `audit_runtime_test` (91/91 on x86_64 and AArch64, 89/89 on RISC-V) and
`musl_runtime_test` (80/80 on x86_64, 78/78 on AArch64 and RISC-V), on one and four CPUs.

### CI Pipeline

GitHub Actions (`.github/workflows/ci.yml`) runs:

- Quick checks: `cargo fmt`, clippy with `-D warnings` on the host (all features) and the three
  bare-metal targets, the system call number and coverage checks, the native libc host tests
- Builds of all three architectures
- Host unit tests, and coverage uploaded to Codecov
- Boot tests on every architecture: dev, release and four CPUs
- The x86_64 rootfs runtime suite
- `cargo audit` and the documentation build

`scripts/dev/gate.sh --boot --rootfs` runs before every commit: fmt, clippy on every target, the
system call checks, the native libc and host tests, the builds, the boot tests (with more variants
than CI) and the runtime suites on all three architectures. It does not run `cargo audit`, build
the documentation or measure coverage; those run in CI only.

## Writing Tests

Tests should follow standard Rust conventions:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;  // Required for vec! macro in no_std test modules

    #[test]
    fn test_example() {
        // Test implementation
    }
}
```

Key patterns:
- Use `#[cfg(all(target_arch = "x86_64", target_os = "none"))]` for bare-metal-only functions
- Add `use alloc::vec;` in test modules that need `vec!`
- No floating point in kernel tests -- use integer/fixed-point only

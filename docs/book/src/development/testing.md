# Testing

VeridianOS has 4,770 host-target unit tests, in-kernel boot tests on every architecture (x86_64 43, AArch64 64, RISC-V 57; more with four CPUs) and in-guest runtime suites on the BlockFS root.

## Test Commands

```bash
# Host-target unit tests (plain `cargo test` targets bare metal and does not work)
cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu

# Format check
cargo fmt --all --check

# Lint all bare-metal targets (x86_64 uses the custom target JSON, as CI does)
BS="-Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem"
cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel $BS -- -D warnings
cargo clippy --target aarch64-unknown-none-softfloat -p veridian-kernel $BS -- -D warnings
cargo clippy --target riscv64imac-unknown-none-elf -p veridian-kernel $BS -- -D warnings

# Everything: the above plus builds, QEMU boot tests and the runtime suites
scripts/dev/gate.sh --boot --rootfs
```

## Testing Strategy

### Unit Tests
Host-target tests run with `cargo test` and cover all kernel subsystems: memory management, IPC, scheduling, capabilities, processes, filesystem, cryptography, desktop, and more.

### Boot Tests
All 3 architectures must boot to Stage 6 BOOTOK with 29/29 kernel tests passing in QEMU. This verifies the full boot chain, hardware initialization, and subsystem integration.

### CI Pipeline
The GitHub Actions CI runs 11 jobs:
- Format check (`cargo fmt`)
- Clippy on 3 bare-metal targets + host
- Build verification for all 3 architectures
- Host-target test suite
- Security audit (`cargo audit`)

### Known Limitations
Automated bare-metal test execution is blocked by a Rust toolchain `lang_items` limitation. Kernel functionality is validated via QEMU boot verification.

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

# Testing

VeridianOS has 4,095+ tests passing across host-target unit tests and kernel boot tests.

## Test Commands

```bash
# Host-target unit tests (4,095+ passing)
cargo test

# Format check
cargo fmt --all --check

# Lint all bare-metal targets
cargo clippy --target x86_64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target aarch64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target riscv64gc-unknown-none-elf -p veridian-kernel -- -D warnings
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

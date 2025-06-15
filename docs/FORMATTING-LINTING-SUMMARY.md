# Formatting and Linting Summary

## Purpose

This document preserves essential code quality standards and linting practices established for VeridianOS. Historical formatting fix details are archived in `docs/archive/format/`.

## Code Quality Standards

### Formatting Rules (cargo fmt)
- **Indentation**: 4 spaces (Rust standard)
- **Line Length**: 100 characters preferred
- **Import Grouping**: Standard library → External crates → Internal modules
- **Trailing Commas**: Required in multi-line constructs
- **Spacing**: Consistent around operators and blocks

### Linting Rules (cargo clippy)
- **Warning Level**: `-D warnings` (deny all warnings)
- **All Targets**: Use `--all-targets --all-features` for comprehensive coverage
- **Zero Warnings Policy**: All clippy warnings must be resolved

## Common Issues and Solutions

### Dead Code
- Use `#[allow(dead_code)]` for planned functionality
- Remove genuinely unused code
- Document why code is temporarily unused

### Global Allocator
```rust
use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();
```

### Test Framework Exports
```rust
// Re-export for tests and benchmarks
pub use test_framework::{exit_qemu, test_panic_handler, test_runner, QemuExitCode, Testable};
```

### Architecture-Specific Code
- Use conditional compilation: `#[cfg(target_arch = "x86_64")]`
- Provide implementations for all architectures
- Handle architecture differences in centralized modules

### Assembly Constraints
- Avoid using reserved registers (e.g., `rbx` on x86_64)
- Use appropriate clobber lists
- Document register usage

## Quality Assurance Commands

### Pre-Commit Checks
```bash
# Format check
cargo fmt --all --check

# Lint check (all architectures)
cargo clippy --target x86_64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target aarch64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target riscv64gc-unknown-none-elf -p veridian-kernel -- -D warnings

# Build check
cargo check --all-targets --all-features
```

### Fixing Issues
```bash
# Auto-format
cargo fmt --all

# Show clippy suggestions
cargo clippy --all-targets --all-features -- -D warnings
```

## Best Practices

### Module Organization
- Make modules public only when needed for external access
- Use `pub(crate)` for internal visibility
- Consolidate duplicate code in common modules

### Error Handling
- Use proper return types (`-> !` for diverging functions)
- Add `core::hint::spin_loop()` in infinite loops
- Ensure panic handlers return `!`

### Feature Flags
```toml
[features]
default = ["alloc"]
alloc = []
testing = []  # For test-specific code
```

### Documentation
- Document why `#[allow(dead_code)]` is used
- Explain architecture-specific implementations
- Note planned functionality for future phases

## Maintained Standards

These standards are actively maintained and enforced:
- Zero warnings policy across all architectures
- Consistent formatting via cargo fmt
- All clippy lints must be addressed
- Clean compilation for all targets
- Proper feature flag usage

For historical formatting and linting fixes, see `docs/archive/format/`.
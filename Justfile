# VeridianOS build commands
#
# Every recipe delegates to the scripts the gate and CI use (build-kernel.sh,
# scripts/boot-test.sh, scripts/dev/gate.sh), so `just` cannot drift from them.
# There is deliberately no `clean` recipe: target/ also holds the cross-built
# sysroot and rootfs trees, which `cargo clean` would delete.

# Bare-metal builds need the standard library built for the target.
bs := "-Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem"

# Default target
default: build

# Setup development environment
setup:
    @echo "Setting up development environment..."
    @bash scripts/setup-dev.sh

# Install git hooks
install-hooks:
    @echo "Installing git hooks..."
    @bash scripts/install-hooks.sh

# Run pre-commit checks manually
pre-commit:
    @echo "Running pre-commit checks..."
    @bash .githooks/pre-commit

# Validate commit message
check-commit-msg MSG:
    @echo "{{ MSG }}" > .tmp-commit-msg
    @bash .githooks/commit-msg .tmp-commit-msg || true
    @rm -f .tmp-commit-msg

# Build the kernel for every architecture (x86_64 also gets its UEFI disk image)
build:
    ./build-kernel.sh all dev

# Release builds for every architecture
build-release:
    ./build-kernel.sh all release

# Build one architecture: just build-arch aarch64 [release]
build-arch ARCH PROFILE="dev":
    ./build-kernel.sh {{ ARCH }} {{ PROFILE }}

# Build for x86_64
build-x86_64:
    ./build-kernel.sh x86_64 dev

# Build for aarch64
build-aarch64:
    ./build-kernel.sh aarch64 dev

# Build for riscv64
build-riscv64:
    ./build-kernel.sh riscv64 dev

# Run x86_64 in a QEMU window (UEFI boot; stops other QEMU instances first)
run:
    ./scripts/run-veridian.sh --display

# Run x86_64 with the persistent BlockFS root (build it with scripts/build-busybox-rootfs.sh)
run-blockfs:
    ./scripts/run-veridian.sh --blockfs --display

# Run AArch64 (serial console)
run-aarch64: build-aarch64
    qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
        -kernel target/aarch64-unknown-none-softfloat/debug/veridian-kernel \
        -serial stdio -display none

# Run RISC-V (serial console)
run-riscv64: build-riscv64
    qemu-system-riscv64 -M virt -m 256M -bios default \
        -kernel target/riscv64imac-unknown-none-elf/debug/veridian-kernel \
        -serial stdio -display none

# Debug x86_64 kernel
debug-x86_64:
    @echo "Starting x86_64 debug session..."
    ./scripts/debug-x86_64.sh

# Debug AArch64 kernel
debug-aarch64:
    @echo "Starting AArch64 debug session..."
    ./scripts/debug-aarch64.sh

# Debug RISC-V kernel
debug-riscv64:
    @echo "Starting RISC-V debug session..."
    ./scripts/debug-riscv64.sh

# Generic debug command (defaults to x86_64)
debug: debug-x86_64

# Host unit tests (plain `cargo test` also builds the bare-metal targets and fails)
test:
    cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu

# Host unit tests with output
test-verbose:
    cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu -- --nocapture

# Boot one architecture in QEMU and check the in-kernel tests (needs a build)
test-x86_64:
    ./scripts/boot-test.sh x86_64

test-aarch64:
    ./scripts/boot-test.sh aarch64

test-riscv64:
    ./scripts/boot-test.sh riscv64

# Build and boot-test every architecture
test-all: build test-x86_64 test-aarch64 test-riscv64
    @echo "All architecture boot tests passed"

# Format code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Run clippy on every target, as the gate and CI do
clippy:
    cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel {{ bs }} -- -D warnings
    cargo clippy --target aarch64-unknown-none-softfloat -p veridian-kernel {{ bs }} -- -D warnings
    cargo clippy --target riscv64imac-unknown-none-elf -p veridian-kernel {{ bs }} -- -D warnings
    cargo clippy --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu -- -D warnings

# The pre-commit gate without QEMU: fmt, clippy, syscall checks, host tests
ci-checks:
    scripts/dev/gate.sh

# The full pre-commit gate: also builds, boot tests and the runtime suites
gate:
    scripts/dev/gate.sh --boot --rootfs

# Build the mdBook guide into docs/book/book
doc:
    cd docs/book && mdbook build

# Update dependencies
update:
    cargo update

# Audit dependencies for security issues
audit:
    cargo audit

# Check for outdated dependencies
outdated:
    cargo outdated

# Install development tools (the toolchain is pinned by rust-toolchain.toml)
install-tools:
    rustup toolchain install nightly-2025-11-15 --component rust-src,llvm-tools-preview,rustfmt,clippy
    cargo install cargo-audit
    cargo install mdbook --version 0.4.37 --locked

# Print system info
info:
    @echo "Rust version:"
    @rustc --version
    @echo "Cargo version:"
    @cargo --version
    @echo "QEMU versions:"
    @qemu-system-x86_64 --version | head -n1
    @qemu-system-aarch64 --version | head -n1
    @qemu-system-riscv64 --version | head -n1

# Help command
help:
    @echo "VeridianOS build commands:"
    @echo ""
    @echo "  just build          - Build every architecture (dev)"
    @echo "  just build-release  - Build every architecture (release)"
    @echo "  just run            - Run x86_64 in a QEMU window"
    @echo "  just run-blockfs    - Run x86_64 with the BlockFS root"
    @echo "  just run-aarch64    - Run AArch64 (serial)"
    @echo "  just run-riscv64    - Run RISC-V (serial)"
    @echo "  just debug          - Debug with GDB (x86_64)"
    @echo "  just debug-aarch64  - Debug AArch64 kernel"
    @echo "  just debug-riscv64  - Debug RISC-V kernel"
    @echo "  just test           - Host unit tests"
    @echo "  just test-all       - Build and boot-test every architecture"
    @echo "  just fmt            - Format code"
    @echo "  just clippy         - Lint every target"
    @echo "  just ci-checks      - Gate without QEMU"
    @echo "  just gate           - Full pre-commit gate"
    @echo "  just doc            - Build the mdBook guide"
    @echo "  just help           - Show this help"
    @echo ""
    @echo "Benchmarks run inside the kernel: the shell's perf command."
    @echo "See Justfile for more commands"

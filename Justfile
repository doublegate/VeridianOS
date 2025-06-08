# VeridianOS build commands

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
    @echo "{{MSG}}" > .tmp-commit-msg
    @bash .githooks/commit-msg .tmp-commit-msg || true
    @rm -f .tmp-commit-msg

# Build the kernel
build:
    @echo "Building VeridianOS..."
    cargo build --release --target targets/x86_64-veridian.json -p veridian-kernel

# Build for specific architecture
build-arch ARCH:
    @echo "Building for {{ARCH}}..."
    cargo build --release --target targets/{{ARCH}}-veridian.json -p veridian-kernel

# Build for x86_64
build-x86_64:
    cargo build --release --target targets/x86_64-veridian.json -p veridian-kernel

# Build for aarch64
build-aarch64:
    cargo build --release --target targets/aarch64-veridian.json -p veridian-kernel

# Build for riscv64
build-riscv64:
    cargo build --release --target targets/riscv64gc-veridian.json -p veridian-kernel

# Run in QEMU
run: build
    @echo "Running VeridianOS in QEMU..."
    qemu-system-x86_64 \
        -enable-kvm \
        -m 2G \
        -smp 4 \
        -serial stdio \
        -kernel target/x86_64-unknown-none/release/veridian_kernel

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

# Run tests for x86_64
test-x86_64:
    @echo "Running x86_64 tests..."
    ./scripts/test-x86_64.sh

# Run tests for AArch64
test-aarch64:
    @echo "Running AArch64 tests..."
    ./scripts/test-aarch64.sh

# Run tests for RISC-V
test-riscv64:
    @echo "Running RISC-V tests..."
    ./scripts/test-riscv64.sh

# Run tests for all architectures
test-all: test-x86_64 test-aarch64 test-riscv64
    @echo "All architecture tests complete!"

# Run tests (defaults to x86_64)
test: test-x86_64

# Run tests with output
test-verbose:
    cargo test --all -- --nocapture

# Run benchmarks for x86_64
bench-x86_64:
    @echo "Running x86_64 benchmarks..."
    ./scripts/benchmark.sh -a x86_64

# Run benchmarks for AArch64
bench-aarch64:
    @echo "Running AArch64 benchmarks..."
    ./scripts/benchmark.sh -a aarch64

# Run benchmarks for RISC-V
bench-riscv64:
    @echo "Running RISC-V benchmarks..."
    ./scripts/benchmark.sh -a riscv64

# Run benchmarks for all architectures
bench-all: bench-x86_64 bench-aarch64 bench-riscv64
    @echo "All benchmarks complete!"

# Run benchmarks (defaults to x86_64)
bench: bench-x86_64

# Run specific benchmark
bench-ipc:
    ./scripts/benchmark.sh -b ipc

bench-context:
    ./scripts/benchmark.sh -b context

bench-memory:
    ./scripts/benchmark.sh -b memory

# Format code
fmt:
    @echo "Formatting code..."
    cargo fmt --all

# Check formatting
fmt-check:
    @echo "Checking code formatting..."
    cargo fmt --all -- --check

# Run clippy
clippy:
    @echo "Running clippy..."
    cargo clippy --all-targets --all-features -- -D warnings

# Run all CI checks
ci-checks: fmt-check clippy test
    @echo "All CI checks passed!"

# Clean build artifacts
clean:
    @echo "Cleaning build artifacts..."
    cargo clean
    rm -rf build/

# Build documentation
doc:
    @echo "Building documentation..."
    cargo doc --no-deps --open

# Build ISO image
build-iso: build
    @echo "Building ISO image..."
    mkdir -p build/iso/boot/grub
    cp target/x86_64-unknown-none/release/veridian_kernel build/iso/boot/
    echo 'set timeout=0' > build/iso/boot/grub/grub.cfg
    echo 'set default=0' >> build/iso/boot/grub/grub.cfg
    echo '' >> build/iso/boot/grub/grub.cfg
    echo 'menuentry "VeridianOS" {' >> build/iso/boot/grub/grub.cfg
    echo '    multiboot2 /boot/veridian_kernel' >> build/iso/boot/grub/grub.cfg
    echo '    boot' >> build/iso/boot/grub/grub.cfg
    echo '}' >> build/iso/boot/grub/grub.cfg
    grub-mkrescue -o build/veridian.iso build/iso

# Run from ISO
run-iso: build-iso
    @echo "Running ISO in QEMU..."
    qemu-system-x86_64 \
        -enable-kvm \
        -m 2G \
        -smp 4 \
        -serial stdio \
        -cdrom build/veridian.iso

# Watch for changes and rebuild
watch:
    @echo "Watching for changes..."
    cargo watch -x build

# Update dependencies
update:
    @echo "Updating dependencies..."
    cargo update

# Audit dependencies for security issues
audit:
    @echo "Auditing dependencies..."
    cargo audit

# Check for outdated dependencies
outdated:
    @echo "Checking for outdated dependencies..."
    cargo outdated

# Generate target specifications
gen-targets:
    @echo "Generating target specifications..."
    mkdir -p targets/
    @echo "Target files would be generated here"

# Install development tools
install-tools:
    @echo "Installing development tools..."
    rustup toolchain install nightly-2025-01-15
    rustup component add rust-src llvm-tools-preview rustfmt clippy --toolchain nightly-2025-01-15
    rustup override set nightly-2025-01-15
    cargo install bootimage cargo-xbuild cargo-binutils cargo-watch cargo-expand cargo-audit cargo-outdated

# Print system info
info:
    @echo "System information:"
    @echo "Rust version:"
    @rustc --version
    @echo "Cargo version:"
    @cargo --version
    @echo "QEMU version:"
    @qemu-system-x86_64 --version | head -n1

# Help command
help:
    @echo "VeridianOS build commands:"
    @echo ""
    @echo "  just build          - Build the kernel"
    @echo "  just run            - Run in QEMU"
    @echo "  just debug          - Debug with GDB (x86_64)"
    @echo "  just debug-x86_64   - Debug x86_64 kernel"
    @echo "  just debug-aarch64  - Debug AArch64 kernel"
    @echo "  just debug-riscv64  - Debug RISC-V kernel"
    @echo "  just test           - Run tests"
    @echo "  just fmt            - Format code"
    @echo "  just clippy         - Run linter"
    @echo "  just ci-checks      - Run all CI checks"
    @echo "  just doc            - Build documentation"
    @echo "  just clean          - Clean build artifacts"
    @echo "  just help           - Show this help"
    @echo ""
    @echo "See Justfile for more commands"
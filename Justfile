# VeridianOS build commands

# Default target
default: build

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

# Run with debugging
debug: build
    @echo "Running with GDB debugging..."
    qemu-system-x86_64 \
        -enable-kvm \
        -m 2G \
        -smp 4 \
        -serial stdio \
        -s -S \
        -kernel target/x86_64-unknown-none/release/veridian_kernel &
    gdb target/x86_64-unknown-none/release/veridian_kernel \
        -ex "target remote :1234" \
        -ex "break kernel_main" \
        -ex "continue"

# Run with GDB
gdb: build
    @echo "Starting GDB session..."
    rust-gdb target/x86_64-unknown-none/release/veridian_kernel

# Run tests
test:
    @echo "Running tests..."
    cargo test --all

# Run tests with output
test-verbose:
    cargo test --all -- --nocapture

# Run benchmarks
bench:
    @echo "Running benchmarks..."
    cargo bench --all

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
    @echo "  just debug          - Run with GDB debugging"
    @echo "  just test           - Run tests"
    @echo "  just fmt            - Format code"
    @echo "  just clippy         - Run linter"
    @echo "  just ci-checks      - Run all CI checks"
    @echo "  just doc            - Build documentation"
    @echo "  just clean          - Clean build artifacts"
    @echo "  just help           - Show this help"
    @echo ""
    @echo "See Justfile for more commands"
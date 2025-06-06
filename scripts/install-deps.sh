#!/bin/bash
# Install development dependencies for VeridianOS

set -e

echo "Installing VeridianOS development dependencies..."

# Detect OS
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    # Linux
    if command -v apt-get &> /dev/null; then
        # Debian/Ubuntu
        echo "Detected Debian/Ubuntu system"
        sudo apt-get update
        sudo apt-get install -y \
            build-essential \
            curl \
            git \
            qemu-system-x86 \
            qemu-system-aarch64 \
            qemu-system-misc \
            nasm \
            mtools \
            xorriso \
            grub-pc-bin \
            grub-efi-amd64-bin \
            ovmf \
            llvm \
            clang \
            lld \
            gdb \
            tmux \
            ripgrep \
            fd-find
    elif command -v dnf &> /dev/null; then
        # Fedora
        echo "Detected Fedora system"
        sudo dnf install -y \
            @development-tools \
            curl \
            git \
            qemu-system-x86 \
            qemu-system-aarch64 \
            qemu-system-riscv \
            nasm \
            mtools \
            xorriso \
            grub2-pc \
            grub2-efi-x64 \
            edk2-ovmf \
            llvm \
            clang \
            lld \
            gdb \
            tmux \
            ripgrep \
            fd-find
    elif command -v pacman &> /dev/null; then
        # Arch Linux
        echo "Detected Arch Linux system"
        sudo pacman -Syu --needed \
            base-devel \
            curl \
            git \
            qemu-full \
            nasm \
            mtools \
            xorriso \
            grub \
            edk2-ovmf \
            llvm \
            clang \
            lld \
            gdb \
            tmux \
            ripgrep \
            fd
    else
        echo "Unsupported Linux distribution"
        exit 1
    fi
elif [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    echo "Detected macOS system"
    if ! command -v brew &> /dev/null; then
        echo "Homebrew not found. Please install Homebrew first."
        exit 1
    fi
    brew install \
        qemu \
        nasm \
        mtools \
        xorriso \
        llvm \
        gdb \
        tmux \
        ripgrep \
        fd
else
    echo "Unsupported operating system: $OSTYPE"
    exit 1
fi

# Install Rust if not already installed
if ! command -v rustc &> /dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo "Rust is already installed"
fi

# Install required Rust components
echo "Installing Rust nightly and components..."
rustup toolchain install nightly-2025-01-15
rustup default nightly-2025-01-15
rustup component add rust-src llvm-tools-preview rustfmt clippy
rustup target add x86_64-unknown-none aarch64-unknown-none riscv64gc-unknown-none-elf

# Install cargo tools
echo "Installing cargo tools..."
cargo install --locked \
    cargo-binutils \
    cargo-xbuild \
    cargo-watch \
    cargo-expand \
    cargo-audit \
    cargo-outdated \
    cargo-tree

# Install just command runner
if ! command -v just &> /dev/null; then
    echo "Installing just..."
    cargo install --locked just
else
    echo "just is already installed"
fi

# Create build directories
echo "Creating build directories..."
mkdir -p build/iso
mkdir -p build/efi

echo "✅ Development dependencies installed successfully!"
echo ""
echo "Next steps:"
echo "1. Run 'just build' to build the kernel"
echo "2. Run 'just run' to run in QEMU"
echo "3. See docs/DEVELOPMENT-GUIDE.md for more information"
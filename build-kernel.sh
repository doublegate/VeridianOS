#!/bin/bash
# VeridianOS Kernel Build Script
# Builds the kernel for all architectures with proper configurations

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${GREEN}VeridianOS Kernel Build Script${NC}"
echo "================================"

# Parse command line arguments
ARCH=${1:-all}
BUILD_TYPE=${2:-dev}

# Check if we're using nightly Rust (required for build-std)
if ! rustc --version | grep -q nightly; then
    echo -e "${RED}Error: Nightly Rust is required for building the kernel${NC}"
    echo "Please run: rustup override set nightly"
    exit 1
fi

# Ensure rust-src component is installed (required for build-std)
echo -e "${YELLOW}Checking for rust-src component...${NC}"
if ! rustup component list | grep -q "rust-src (installed)"; then
    echo "Installing rust-src component..."
    rustup component add rust-src
fi

# Function to build for a specific architecture
build_arch() {
    local arch=$1
    local target=$2
    local custom_target=$3
    
    echo -e "${BLUE}Building $arch kernel...${NC}"
    
    if [ "$BUILD_TYPE" == "release" ]; then
        RELEASE_FLAG="--release"
    else
        RELEASE_FLAG=""
    fi
    
    if [ "$custom_target" == "true" ]; then
        # Build with custom target JSON (for x86_64 with kernel code model)
        if cargo build $RELEASE_FLAG --target "$target" -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc; then
            echo -e "${GREEN}$arch build successful!${NC}"
        else
            echo -e "${RED}$arch build failed!${NC}"
            exit 1
        fi
    else
        # Build with standard bare metal target
        if cargo build $RELEASE_FLAG --target "$target" -p veridian-kernel; then
            echo -e "${GREEN}$arch build successful!${NC}"
        else
            echo -e "${RED}$arch build failed!${NC}"
            exit 1
        fi
    fi
}

# Build based on architecture selection
case $ARCH in
    x86_64)
        build_arch "x86_64" "targets/x86_64-veridian.json" "true"
        ;;
    aarch64)
        build_arch "AArch64" "aarch64-unknown-none" "false"
        ;;
    riscv64)
        build_arch "RISC-V" "riscv64gc-unknown-none-elf" "false"
        ;;
    all)
        echo -e "${YELLOW}Building all architectures...${NC}"
        build_arch "x86_64" "targets/x86_64-veridian.json" "true"
        build_arch "AArch64" "aarch64-unknown-none" "false"
        build_arch "RISC-V" "riscv64gc-unknown-none-elf" "false"
        ;;
    *)
        echo -e "${RED}Unknown architecture: $ARCH${NC}"
        echo "Usage: $0 [x86_64|aarch64|riscv64|all] [dev|release]"
        exit 1
        ;;
esac

echo -e "${GREEN}All builds completed successfully!${NC}"

# Show build artifacts
echo -e "${YELLOW}Build artifacts:${NC}"
if [ "$BUILD_TYPE" == "release" ]; then
    BUILD_DIR="release"
else
    BUILD_DIR="debug"
fi

if [ "$ARCH" == "all" ] || [ "$ARCH" == "x86_64" ]; then
    echo "  x86_64: target/x86_64-veridian/$BUILD_DIR/veridian-kernel"
fi
if [ "$ARCH" == "all" ] || [ "$ARCH" == "aarch64" ]; then
    echo "  AArch64: target/aarch64-unknown-none/$BUILD_DIR/veridian-kernel"
fi
if [ "$ARCH" == "all" ] || [ "$ARCH" == "riscv64" ]; then
    echo "  RISC-V: target/riscv64gc-unknown-none-elf/$BUILD_DIR/veridian-kernel"
fi
#!/bin/bash
# VeridianOS Kernel Build Script
# This script builds the kernel with the proper code model to avoid relocation errors

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}VeridianOS Kernel Build Script${NC}"
echo "================================"

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

# Clean previous builds (optional, comment out for incremental builds)
# cargo clean

# Method 1: Build with kernel code model (recommended)
echo -e "${YELLOW}Building kernel with kernel code model...${NC}"
echo "Kernel will be linked at 0xFFFFFFFF80000000"

# The RUSTFLAGS are set in .cargo/config.toml, but we can override here if needed
# RUSTFLAGS="-C code-model=kernel -C relocation-model=static" \

cargo build --target targets/x86_64-veridian.json

# Alternative Method 2: Build with large code model
# Uncomment this section if you need to link at addresses beyond ±2GB
# echo -e "${YELLOW}Building kernel with large code model...${NC}"
# echo "Use this for kernels linked beyond 0xFFFFFFFF80000000"
# 
# RUSTFLAGS="-C code-model=large -C relocation-model=static -C link-arg=-Tkernel/src/arch/x86_64/link.ld" \
# cargo build --target targets/x86_64-veridian-large.json

# Check if build succeeded
if [ $? -eq 0 ]; then
    echo -e "${GREEN}Build successful!${NC}"
    
    # Create bootable image
    echo -e "${YELLOW}Creating bootable image...${NC}"
    cargo bootimage --target targets/x86_64-veridian.json
    
    echo -e "${GREEN}Bootable image created successfully!${NC}"
else
    echo -e "${RED}Build failed! Check the error messages above.${NC}"
    exit 1
fi
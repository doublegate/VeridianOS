#!/usr/bin/env bash
# VeridianOS Rust Toolchain Rootfs Packager
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Packages the cross-compiled Rust toolchain (rustc + cargo + std) into
# a VeridianOS BlockFS root filesystem image for self-hosting verification.
#
# Usage:
#   ./scripts/build-rust-rootfs.sh [OPTIONS]
#
# Options:
#   --rust-prefix DIR  Rust toolchain prefix (default: /opt/veridian/rust)
#   --output FILE      Output rootfs image (default: target/rootfs-rust.img)
#   --size MB          Image size in MB (default: 1024)
#   --help             Show this help

set -euo pipefail

# ---------------------------------------------------------------------------
# Color helpers
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()    { printf "${CYAN}[INFO]${NC}  %s\n" "$*"; }
success() { printf "${GREEN}[OK]${NC}    %s\n" "$*"; }
warn()    { printf "${YELLOW}[WARN]${NC}  %s\n" "$*"; }
error()   { printf "${RED}[ERROR]${NC} %s\n" "$*" >&2; }
step()    { printf "\n${BOLD}==> %s${NC}\n" "$*"; }

die() {
    error "$@"
    exit 1
}

# ---------------------------------------------------------------------------
# Resolve project root
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
RUST_PREFIX="/opt/veridian/rust"
OUTPUT="${PROJECT_ROOT}/target/rootfs-rust.img"
SIZE_MB=1024

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rust-prefix) RUST_PREFIX="$2"; shift 2 ;;
        --output)      OUTPUT="$2"; shift 2 ;;
        --size)        SIZE_MB="$2"; shift 2 ;;
        --help|-h)
            head -18 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
            exit 0
            ;;
        *) die "Unknown option: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Build mkfs-blockfs tool
# ---------------------------------------------------------------------------
step "Building mkfs-blockfs tool"
MKFS="${PROJECT_ROOT}/tools/mkfs-blockfs/target/release/mkfs-blockfs"
if [[ ! -x "$MKFS" ]]; then
    cd "${PROJECT_ROOT}/tools/mkfs-blockfs"
    cargo build --release
fi
success "mkfs-blockfs ready"

# ---------------------------------------------------------------------------
# Create staging directory
# ---------------------------------------------------------------------------
step "Creating staging directory"
STAGING="${PROJECT_ROOT}/build/rust-rootfs-staging"
rm -rf "$STAGING"
mkdir -p "$STAGING"/{bin,lib,usr/lib,usr/bin,tmp,etc,home/user}

# ---------------------------------------------------------------------------
# Copy Rust toolchain binaries
# ---------------------------------------------------------------------------
step "Copying Rust toolchain"

for bin in rustc cargo; do
    SRC="${RUST_PREFIX}/bin/${bin}"
    if [[ -x "$SRC" ]]; then
        cp "$SRC" "$STAGING/usr/bin/"
        info "Copied: $bin ($(du -h "$SRC" | cut -f1))"
    else
        warn "Not found: ${SRC}"
    fi
done

# Copy Rust standard library
if [[ -d "${RUST_PREFIX}/lib" ]]; then
    find "${RUST_PREFIX}/lib" -name "*.rlib" -o -name "*.so" | while read -r lib; do
        DEST="$STAGING/usr/lib/$(basename "$lib")"
        cp "$lib" "$DEST"
    done
    LIB_COUNT=$(find "$STAGING/usr/lib" -name "*.rlib" -o -name "*.so" 2>/dev/null | wc -l)
    success "Copied ${LIB_COUNT} library files"
fi

# ---------------------------------------------------------------------------
# Create test files
# ---------------------------------------------------------------------------
step "Creating test files"

cat > "$STAGING/home/user/hello.rs" << 'HELLO'
fn main() {
    println!("Hello from VeridianOS Rust!");
    let x: Vec<i32> = (1..=10).collect();
    println!("Sum of 1..10 = {}", x.iter().sum::<i32>());
}
HELLO

cat > "$STAGING/home/user/test_std.rs" << 'TEST'
use std::fs;
use std::path::Path;

fn main() {
    // Test file I/O
    fs::write("/tmp/test.txt", "Hello from Rust std!\n").unwrap();
    let content = fs::read_to_string("/tmp/test.txt").unwrap();
    println!("File content: {}", content.trim());

    // Test path manipulation
    let p = Path::new("/home/user/hello.rs");
    println!("File name: {:?}", p.file_name());
    println!("Extension: {:?}", p.extension());
    println!("Parent: {:?}", p.parent());

    // Test env
    println!("PID: {}", std::process::id());

    println!("All tests passed!");
}
TEST

cat > "$STAGING/home/user/self-host-test.sh" << 'SELFHOST'
#!/bin/ash
echo "=== VeridianOS Rust Self-Hosting Verification ==="
echo ""

echo "Step 1: Check rustc version"
rustc --version
if [ $? -ne 0 ]; then
    echo "FAIL: rustc not found"
    exit 1
fi

echo ""
echo "Step 2: Compile hello.rs"
rustc -o /tmp/hello /home/user/hello.rs
if [ $? -ne 0 ]; then
    echo "FAIL: rustc compilation failed"
    exit 1
fi

echo ""
echo "Step 3: Run hello"
/tmp/hello
if [ $? -ne 0 ]; then
    echo "FAIL: hello execution failed"
    exit 1
fi

echo ""
echo "Step 4: Compile test_std.rs"
rustc -o /tmp/test_std /home/user/test_std.rs
if [ $? -ne 0 ]; then
    echo "FAIL: test_std compilation failed"
    exit 1
fi

echo ""
echo "Step 5: Run test_std"
/tmp/test_std
if [ $? -ne 0 ]; then
    echo "FAIL: test_std execution failed"
    exit 1
fi

echo ""
echo "=== ALL SELF-HOSTING TESTS PASSED ==="
SELFHOST
chmod +x "$STAGING/home/user/self-host-test.sh"

success "Test files created"

# ---------------------------------------------------------------------------
# Create BlockFS image
# ---------------------------------------------------------------------------
step "Creating BlockFS image (${SIZE_MB}MB)"

"$MKFS" "$STAGING" "$OUTPUT" --size "${SIZE_MB}M" 2>&1 || {
    # Fallback to TAR-based rootfs
    warn "mkfs-blockfs failed, creating TAR rootfs instead"
    OUTPUT="${OUTPUT%.img}.tar"
    tar cf "$OUTPUT" -C "$STAGING" .
}

success "Rootfs image created: ${OUTPUT} ($(du -h "$OUTPUT" | cut -f1))"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
success "Rust rootfs packaging complete!"
echo ""
info "Image: ${OUTPUT}"
info ""
info "To boot with this rootfs:"
info "  qemu-system-x86_64 -enable-kvm \\"
info "    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
info "    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
info "    -device ide-hd,drive=disk0 \\"
info "    -drive file=${OUTPUT},if=none,id=vd0,format=raw \\"
info "    -device virtio-blk-pci,drive=vd0 \\"
info "    -serial stdio -m 8192M"
info ""
info "Then run: /home/user/self-host-test.sh"

# Cleanup
rm -rf "$STAGING"

#!/usr/bin/env bash
# VeridianOS Rust Compiler Cross-Build Script
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compiles rustc (Rust compiler) + cargo for VeridianOS x86_64.
# Produces a complete Rust toolchain that runs ON VeridianOS.
#
# Build stages:
#   Stage 0: Cross-compile std for x86_64-unknown-veridian (using host rustc)
#   Stage 1: Cross-compile rustc for x86_64-unknown-veridian (using host rustc + Stage 0 std)
#   Stage 2: Cross-compile cargo for x86_64-unknown-veridian
#   Stage 3: (Optional) Self-host verification -- build rustc ON VeridianOS
#
# Prerequisites:
#   - LLVM 19 cross-compiled for VeridianOS (via build-llvm-veridian.sh)
#   - x86_64-veridian-gcc cross-compiler (via build-cross-toolchain.sh)
#   - Host Rust toolchain (rustup nightly)
#   - ~20GB disk space
#
# Usage:
#   ./scripts/build-rustc-veridian.sh [OPTIONS]
#
# Options:
#   --rust-src DIR     Rust source directory (default: downloads from GitHub)
#   --llvm-root DIR    LLVM installation (default: /opt/veridian/llvm)
#   --prefix DIR       Installation prefix (default: /opt/veridian/rust)
#   --stage N          Build up to stage N (0-3, default: 2)
#   --jobs N           Parallel jobs (default: nproc/2, rustc is memory-heavy)
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
RUST_VERSION="1.83.0"  # Stable version close to the target
RUST_SRC=""
LLVM_ROOT="/opt/veridian/llvm"
RUST_PREFIX="/opt/veridian/rust"
TOOLCHAIN_PREFIX="${VERIDIAN_TOOLCHAIN_PREFIX:-/opt/veridian/toolchain}"
MAX_STAGE=2
JOBS="$(( $(nproc) / 2 ))"
[[ $JOBS -lt 1 ]] && JOBS=1
BUILD_DIR="${PROJECT_ROOT}/build/rustc-veridian"
VERIDIAN_STD="${PROJECT_ROOT}/userland/rust-std"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --rust-src) RUST_SRC="$2"; shift 2 ;;
        --llvm-root) LLVM_ROOT="$2"; shift 2 ;;
        --prefix) RUST_PREFIX="$2"; shift 2 ;;
        --stage) MAX_STAGE="$2"; shift 2 ;;
        --jobs) JOBS="$2"; shift 2 ;;
        --help|-h)
            head -30 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
            exit 0
            ;;
        *) die "Unknown option: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Step 1: Validate
# ---------------------------------------------------------------------------
step "Validating prerequisites"

CROSS_GCC="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
[[ -x "$CROSS_GCC" ]] || die "Cross-compiler not found: ${CROSS_GCC}"
success "Cross-compiler: ${CROSS_GCC}"

SYSROOT="${TOOLCHAIN_PREFIX}/x86_64-veridian/sysroot"
[[ -d "$SYSROOT" ]] || die "Sysroot not found: ${SYSROOT}"
success "Sysroot: ${SYSROOT}"

if [[ -d "$LLVM_ROOT/lib" ]]; then
    LIB_COUNT=$(find "${LLVM_ROOT}/lib" -name "libLLVM*.a" 2>/dev/null | wc -l)
    if [[ $LIB_COUNT -eq 0 ]]; then
        die "No LLVM static libraries found in ${LLVM_ROOT}/lib"
    fi
    success "LLVM: ${LLVM_ROOT} (${LIB_COUNT} static libs)"
else
    die "LLVM installation not found at ${LLVM_ROOT}
    Run: ./scripts/build-llvm-veridian.sh first"
fi

command -v rustc &>/dev/null || die "Host rustc not found (install via rustup)"
command -v cargo &>/dev/null || die "Host cargo not found (install via rustup)"
success "Host Rust: $(rustc --version)"

info "Rust version:    ${RUST_VERSION}"
info "LLVM root:       ${LLVM_ROOT}"
info "Install prefix:  ${RUST_PREFIX}"
info "Max stage:       ${MAX_STAGE}"
info "Build jobs:      ${JOBS}"

# ---------------------------------------------------------------------------
# Step 2: Get Rust source
# ---------------------------------------------------------------------------
step "Preparing Rust source"

if [[ -z "$RUST_SRC" ]]; then
    RUST_SRC="${PROJECT_ROOT}/build/rust-${RUST_VERSION}"
    TARBALL="${PROJECT_ROOT}/build/rustc-${RUST_VERSION}-src.tar.xz"

    if [[ ! -d "$RUST_SRC" ]]; then
        mkdir -p "${PROJECT_ROOT}/build"
        if [[ ! -f "$TARBALL" ]]; then
            info "Downloading Rust ${RUST_VERSION} source..."
            curl -L -o "$TARBALL" \
                "https://static.rust-lang.org/dist/rustc-${RUST_VERSION}-src.tar.xz"
        fi
        info "Extracting..."
        tar xf "$TARBALL" -C "${PROJECT_ROOT}/build/"
        mv "${PROJECT_ROOT}/build/rustc-${RUST_VERSION}-src" "$RUST_SRC"
        success "Rust source extracted to ${RUST_SRC}"
    fi
fi

[[ -f "$RUST_SRC/x.py" ]] || die "Rust source not found at ${RUST_SRC} (missing x.py)"
success "Rust source: ${RUST_SRC}"

# ---------------------------------------------------------------------------
# Step 3: Create VeridianOS target specification
# ---------------------------------------------------------------------------
step "Registering x86_64-unknown-veridian target"

TARGET_DIR="${RUST_SRC}/compiler/rustc_target/src/spec/targets"
TARGET_FILE="${TARGET_DIR}/x86_64_unknown_veridian.rs"

if [[ ! -f "$TARGET_FILE" ]]; then
    info "Creating target spec: ${TARGET_FILE}"
    cat > "$TARGET_FILE" << 'TARGETEOF'
use crate::spec::{base, Cc, LinkerFlavor, StackProbeType, Target, TargetOptions};

pub fn target() -> Target {
    let mut base = base::veridian::opts();
    base.cpu = "x86-64".into();
    base.plt_by_default = false;
    base.max_atomic_width = Some(64);
    base.stack_probes = StackProbeType::Inline;

    Target {
        llvm_target: "x86_64-unknown-none".into(),
        metadata: crate::spec::TargetMetadata {
            description: Some("VeridianOS (x86_64)".into()),
            tier: Some(3),
            host_tools: Some(true),
            std: Some(true),
        },
        pointer_width: 64,
        data_layout: "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128".into(),
        arch: "x86_64".into(),
        options: base,
    }
}
TARGETEOF

    # Create base/veridian.rs if it doesn't exist
    BASE_DIR="${RUST_SRC}/compiler/rustc_target/src/spec/base"
    BASE_FILE="${BASE_DIR}/veridian.rs"
    if [[ ! -f "$BASE_FILE" ]]; then
        info "Creating base platform spec: ${BASE_FILE}"
        cat > "$BASE_FILE" << 'BASEEOF'
use crate::spec::{cvs, Cc, LinkerFlavor, RelroLevel, TargetOptions};

pub fn opts() -> TargetOptions {
    TargetOptions {
        os: "veridian".into(),
        env: "".into(),
        vendor: "unknown".into(),
        linker_flavor: LinkerFlavor::Gnu(Cc::Yes, crate::spec::Lld::No),
        linker: Some("x86_64-veridian-gcc".into()),
        dynamic_linking: true,
        executables: true,
        has_rpath: false,
        position_independent_executables: true,
        static_position_independent_executables: false,
        has_thread_local: true,
        crt_static_default: true,
        crt_static_respected: true,
        relro_level: RelroLevel::Full,
        families: cvs!["unix"],
        ..Default::default()
    }
}
BASEEOF
    fi
    success "Target spec registered"
else
    success "Target spec already exists"
fi

# ---------------------------------------------------------------------------
# Step 4: Configure x.py
# ---------------------------------------------------------------------------
step "Generating config.toml for Rust build"

CONFIG_FILE="${RUST_SRC}/config.toml"
cat > "$CONFIG_FILE" << CONFIGEOF
# VeridianOS Cross-Build Configuration
# Generated by build-rustc-veridian.sh

[llvm]
link-shared = false
static-libstdcpp = true
targets = "X86"
# Use pre-built LLVM
download-ci-llvm = false

[build]
target = ["x86_64-unknown-veridian"]
host = ["x86_64-unknown-linux-gnu"]
build = "x86_64-unknown-linux-gnu"
docs = false
extended = true
tools = ["cargo"]
cargo = "$(command -v cargo)"
rustc = "$(command -v rustc)"
python = "$(command -v python3)"
# Vendor all dependencies for offline builds
vendor = true

[install]
prefix = "${RUST_PREFIX}"

[target.x86_64-unknown-veridian]
llvm-config = "${LLVM_ROOT}/bin/llvm-config"
cc = "${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
cxx = "${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-g++"
ar = "${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-ar"
ranlib = "${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-ranlib"
linker = "${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"

[target.x86_64-unknown-linux-gnu]
llvm-config = "$(command -v llvm-config 2>/dev/null || command -v llvm-config-19 2>/dev/null || echo /usr/bin/llvm-config)"

[rust]
optimize = true
debug = false
codegen-units = 1
lto = "thin"
CONFIGEOF

success "config.toml written"

# ---------------------------------------------------------------------------
# Step 5: Copy VeridianOS std platform layer
# ---------------------------------------------------------------------------
step "Integrating VeridianOS std platform layer"

STD_SYS_DIR="${RUST_SRC}/library/std/src/sys"
VERIDIAN_SYS_DIR="${STD_SYS_DIR}/veridian"

if [[ -d "$VERIDIAN_STD/src/sys/veridian" ]]; then
    mkdir -p "$VERIDIAN_SYS_DIR"
    cp -r "$VERIDIAN_STD/src/sys/veridian/"* "$VERIDIAN_SYS_DIR/"
    success "Platform layer copied to ${VERIDIAN_SYS_DIR}"
else
    warn "VeridianOS std platform layer not found at ${VERIDIAN_STD}/src/sys/veridian"
    warn "std will build with stubs only -- programs may not link correctly"
fi

# ---------------------------------------------------------------------------
# Step 6: Build
# ---------------------------------------------------------------------------
mkdir -p "$BUILD_DIR"

# Stage 0: Build std for veridian target
if [[ $MAX_STAGE -ge 0 ]]; then
    step "Stage 0: Building std for x86_64-unknown-veridian"
    cd "$RUST_SRC"
    python3 x.py build library/std --target x86_64-unknown-veridian -j "$JOBS" 2>&1 | tail -20
    success "Stage 0 (std) complete"
fi

# Stage 1: Build rustc for veridian target
if [[ $MAX_STAGE -ge 1 ]]; then
    step "Stage 1: Building rustc for x86_64-unknown-veridian"
    cd "$RUST_SRC"
    python3 x.py build compiler/rustc --target x86_64-unknown-veridian -j "$JOBS" 2>&1 | tail -20
    success "Stage 1 (rustc) complete"
fi

# Stage 2: Build cargo for veridian target
if [[ $MAX_STAGE -ge 2 ]]; then
    step "Stage 2: Building cargo for x86_64-unknown-veridian"
    cd "$RUST_SRC"
    python3 x.py build src/tools/cargo --target x86_64-unknown-veridian -j "$JOBS" 2>&1 | tail -20
    success "Stage 2 (cargo) complete"
fi

# Stage 3: Self-hosting verification (requires booting VeridianOS)
if [[ $MAX_STAGE -ge 3 ]]; then
    step "Stage 3: Self-hosting verification"
    warn "Stage 3 requires booting VeridianOS with the cross-compiled toolchain."
    warn "Package the toolchain into the rootfs and run the self-host test:"
    info "  1. ./scripts/build-rust-rootfs.sh  (packages rustc+cargo into BlockFS)"
    info "  2. Boot VeridianOS in QEMU"
    info "  3. Run: rustc --version && rustc -o hello hello.rs && ./hello"
    info "  4. Run: cargo new test_project && cd test_project && cargo build"
fi

# ---------------------------------------------------------------------------
# Step 7: Install
# ---------------------------------------------------------------------------
step "Installing Rust toolchain to ${RUST_PREFIX}"
cd "$RUST_SRC"
python3 x.py install -j "$JOBS" 2>&1 | tail -10

success "Rust toolchain installed to ${RUST_PREFIX}"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
success "Rust toolchain cross-build for VeridianOS complete!"
echo ""
info "Installation: ${RUST_PREFIX}"
info "rustc:        ${RUST_PREFIX}/bin/rustc"
info "cargo:        ${RUST_PREFIX}/bin/cargo"
info ""
info "To package for VeridianOS rootfs:"
info "  ./scripts/build-rust-rootfs.sh --rust-prefix ${RUST_PREFIX}"

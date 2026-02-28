#!/usr/bin/env bash
# VeridianOS LLVM 19 Cross-Compilation Script
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compiles LLVM 19 from Linux for the VeridianOS x86_64 target.
# Produces static LLVM libraries needed by the Rust compiler (rustc).
#
# Prerequisites:
#   - x86_64-veridian-gcc cross-compiler (via build-cross-toolchain.sh)
#   - CMake 3.20+, Ninja, Python 3
#   - Host LLVM 19 (for llvm-tblgen, clang-tblgen)
#   - ~10GB disk space for build artifacts
#
# Usage:
#   ./scripts/build-llvm-veridian.sh [OPTIONS]
#
# Options:
#   --llvm-src DIR     LLVM source directory (default: downloads 19.1.0)
#   --prefix DIR       Installation prefix (default: /opt/veridian/llvm)
#   --jobs N           Parallel jobs (default: nproc)
#   --skip-download    Skip downloading LLVM source
#   --clean            Clean build directory first
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
LLVM_VERSION="19.1.0"
LLVM_SRC=""
LLVM_PREFIX="/opt/veridian/llvm"
TOOLCHAIN_PREFIX="${VERIDIAN_TOOLCHAIN_PREFIX:-/opt/veridian/toolchain}"
JOBS="$(nproc)"
SKIP_DOWNLOAD=0
CLEAN=0
BUILD_DIR="${PROJECT_ROOT}/build/llvm-veridian"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --llvm-src)      LLVM_SRC="$2"; shift 2 ;;
        --prefix)        LLVM_PREFIX="$2"; shift 2 ;;
        --jobs)          JOBS="$2"; shift 2 ;;
        --skip-download) SKIP_DOWNLOAD=1; shift ;;
        --clean)         CLEAN=1; shift ;;
        --help|-h)
            head -28 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
            exit 0
            ;;
        *)               die "Unknown option: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Step 1: Validate prerequisites
# ---------------------------------------------------------------------------
step "Checking prerequisites"

CROSS_GCC="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
if [[ ! -x "$CROSS_GCC" ]]; then
    die "Cross-compiler not found: ${CROSS_GCC}
    Run: ./scripts/build-cross-toolchain.sh first"
fi
success "Cross-compiler: ${CROSS_GCC}"

for tool in cmake ninja python3; do
    if ! command -v "$tool" &>/dev/null; then
        die "$tool is required but not found in PATH"
    fi
done
success "Build tools: cmake, ninja, python3"

# Find host LLVM tablegen tools
HOST_LLVM_TBLGEN=""
HOST_CLANG_TBLGEN=""
for candidate in llvm-tblgen llvm-tblgen-19; do
    if command -v "$candidate" &>/dev/null; then
        HOST_LLVM_TBLGEN="$(command -v "$candidate")"
        break
    fi
done
for candidate in clang-tblgen clang-tblgen-19; do
    if command -v "$candidate" &>/dev/null; then
        HOST_CLANG_TBLGEN="$(command -v "$candidate")"
        break
    fi
done

if [[ -z "$HOST_LLVM_TBLGEN" ]]; then
    warn "llvm-tblgen not found -- will build native tablegen first (slower)"
fi

info "LLVM version:  ${LLVM_VERSION}"
info "Install to:    ${LLVM_PREFIX}"
info "Build jobs:    ${JOBS}"
info "Toolchain:     ${TOOLCHAIN_PREFIX}"

# ---------------------------------------------------------------------------
# Step 2: Get LLVM source
# ---------------------------------------------------------------------------
step "Preparing LLVM source"

if [[ -z "$LLVM_SRC" ]]; then
    LLVM_SRC="${PROJECT_ROOT}/build/llvm-project-${LLVM_VERSION}.src"
    TARBALL="${PROJECT_ROOT}/build/llvm-project-${LLVM_VERSION}.src.tar.xz"

    if [[ $SKIP_DOWNLOAD -eq 0 && ! -d "$LLVM_SRC" ]]; then
        mkdir -p "${PROJECT_ROOT}/build"
        if [[ ! -f "$TARBALL" ]]; then
            info "Downloading LLVM ${LLVM_VERSION} source..."
            curl -L -o "$TARBALL" \
                "https://github.com/llvm/llvm-project/releases/download/llvmorg-${LLVM_VERSION}/llvm-project-${LLVM_VERSION}.src.tar.xz"
        fi
        info "Extracting..."
        tar xf "$TARBALL" -C "${PROJECT_ROOT}/build/"
        success "LLVM source extracted to ${LLVM_SRC}"
    fi
fi

if [[ ! -d "$LLVM_SRC/llvm" ]]; then
    die "LLVM source directory not found at ${LLVM_SRC}/llvm"
fi
success "LLVM source: ${LLVM_SRC}"

# ---------------------------------------------------------------------------
# Step 3: Build native tablegen (if needed)
# ---------------------------------------------------------------------------
if [[ -z "$HOST_LLVM_TBLGEN" ]]; then
    step "Building native tablegen tools"
    NATIVE_BUILD="${PROJECT_ROOT}/build/llvm-native-tblgen"
    mkdir -p "$NATIVE_BUILD"
    cd "$NATIVE_BUILD"

    cmake -G Ninja "${LLVM_SRC}/llvm" \
        -DCMAKE_BUILD_TYPE=Release \
        -DLLVM_TARGETS_TO_BUILD="X86" \
        -DLLVM_ENABLE_PROJECTS="clang" \
        -DLLVM_BUILD_TOOLS=OFF \
        -DLLVM_BUILD_UTILS=OFF \
        -DLLVM_INCLUDE_TESTS=OFF \
        -DLLVM_INCLUDE_BENCHMARKS=OFF \
        -DLLVM_INCLUDE_EXAMPLES=OFF

    cmake --build . --target llvm-tblgen clang-tblgen -j "$JOBS"

    HOST_LLVM_TBLGEN="${NATIVE_BUILD}/bin/llvm-tblgen"
    HOST_CLANG_TBLGEN="${NATIVE_BUILD}/bin/clang-tblgen"
    success "Native tablegen built"
fi

# ---------------------------------------------------------------------------
# Step 4: Cross-compile LLVM
# ---------------------------------------------------------------------------
step "Cross-compiling LLVM ${LLVM_VERSION} for VeridianOS"

if [[ $CLEAN -eq 1 && -d "$BUILD_DIR" ]]; then
    info "Cleaning previous build..."
    rm -rf "$BUILD_DIR"
fi

mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

SYSROOT="${TOOLCHAIN_PREFIX}/x86_64-veridian/sysroot"
TOOLCHAIN_FILE="${SCRIPT_DIR}/cmake/veridian-x86_64-toolchain.cmake"

CMAKE_ARGS=(
    -G Ninja
    -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN_FILE}"
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="${LLVM_PREFIX}"

    # LLVM configuration
    -DLLVM_TARGETS_TO_BUILD="X86"
    -DLLVM_DEFAULT_TARGET_TRIPLE="x86_64-unknown-veridian"
    -DLLVM_HOST_TRIPLE="x86_64-unknown-veridian"
    -DLLVM_TARGET_ARCH="X86"

    # Projects to build (minimal for rustc)
    -DLLVM_ENABLE_PROJECTS="lld"
    -DLLVM_ENABLE_RUNTIMES=""

    # Build type
    -DLLVM_BUILD_STATIC=ON
    -DBUILD_SHARED_LIBS=OFF
    -DLLVM_LINK_LLVM_DYLIB=OFF

    # Disable optional dependencies
    -DLLVM_ENABLE_ZLIB=OFF
    -DLLVM_ENABLE_ZSTD=OFF
    -DLLVM_ENABLE_LIBXML2=OFF
    -DLLVM_ENABLE_TERMINFO=OFF
    -DLLVM_ENABLE_LIBEDIT=OFF
    -DLLVM_ENABLE_LIBPFM=OFF
    -DLLVM_ENABLE_THREADS=OFF
    -DLLVM_ENABLE_PIC=OFF

    # Reduce build scope
    -DLLVM_BUILD_TOOLS=ON
    -DLLVM_BUILD_UTILS=ON
    -DLLVM_INCLUDE_BENCHMARKS=OFF
    -DLLVM_INCLUDE_EXAMPLES=OFF
    -DLLVM_INCLUDE_TESTS=OFF
    -DLLVM_INCLUDE_DOCS=OFF

    # Host tablegen tools
    -DLLVM_TABLEGEN="${HOST_LLVM_TBLGEN}"

    # Optimization
    -DCMAKE_C_FLAGS_RELEASE="-O2 -DNDEBUG"
    -DCMAKE_CXX_FLAGS_RELEASE="-O2 -DNDEBUG -fno-exceptions -fno-rtti"
)

if [[ -n "$HOST_CLANG_TBLGEN" ]]; then
    CMAKE_ARGS+=(-DCLANG_TABLEGEN="${HOST_CLANG_TBLGEN}")
fi

info "Running cmake configure..."
cmake "${LLVM_SRC}/llvm" "${CMAKE_ARGS[@]}"
success "CMake configure complete"

step "Building LLVM (${JOBS} parallel jobs)"
info "This will take 30-60 minutes..."
cmake --build . -j "$JOBS"
success "LLVM build complete"

# ---------------------------------------------------------------------------
# Step 5: Install
# ---------------------------------------------------------------------------
step "Installing LLVM to ${LLVM_PREFIX}"
cmake --install .
success "LLVM installed"

# ---------------------------------------------------------------------------
# Step 6: Verify
# ---------------------------------------------------------------------------
step "Verifying LLVM installation"

LLVM_CONFIG="${LLVM_PREFIX}/bin/llvm-config"
if [[ -f "$LLVM_CONFIG" ]]; then
    info "llvm-config: ${LLVM_CONFIG}"
    info "LLVM version: $("${LLVM_CONFIG}" --version 2>/dev/null || echo 'N/A (cross-compiled)')"
    info "LLVM libs: $("${LLVM_CONFIG}" --libs 2>/dev/null | wc -w || echo 'N/A') libraries"
else
    warn "llvm-config not found (expected for cross-compiled LLVM)"
fi

# Check that static libs exist
LIB_COUNT=$(find "${LLVM_PREFIX}/lib" -name "libLLVM*.a" 2>/dev/null | wc -l)
if [[ $LIB_COUNT -gt 0 ]]; then
    success "Found ${LIB_COUNT} LLVM static libraries"
else
    warn "No LLVM static libraries found -- build may have failed"
fi

echo ""
success "LLVM ${LLVM_VERSION} cross-compilation for VeridianOS complete!"
echo ""
info "Installation: ${LLVM_PREFIX}"
info "Libraries:    ${LLVM_PREFIX}/lib/"
info "Headers:      ${LLVM_PREFIX}/include/"
info ""
info "To use with rustc build:"
info "  export LLVM_CONFIG=${LLVM_PREFIX}/bin/llvm-config"
info "  or pass --llvm-root=${LLVM_PREFIX} to x.py configure"

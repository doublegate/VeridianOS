#!/usr/bin/env bash
# VeridianOS CMake Cross-Compilation Helper
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Provides a streamlined interface for cross-compiling CMake-based projects
# (especially LLVM) for VeridianOS. Sets up the toolchain, sysroot, and
# common CMake variables for a successful cross-build.
#
# Usage:
#   ./scripts/build-cmake-veridian.sh [OPTIONS] <source-dir>
#
# Options:
#   --arch ARCH            Target architecture (x86_64, default)
#   --prefix PREFIX        Toolchain prefix (default: /opt/veridian/toolchain)
#   --build-dir DIR        Build directory (default: build-veridian)
#   --install-dir DIR      Install prefix for built project
#   --llvm                 Enable LLVM-specific CMake flags
#   --jobs N               Parallel jobs (default: nproc)
#   --cmake-args "ARGS"    Additional CMake arguments
#   --configure-only       Only run cmake configure, don't build
#   --help                 Show this help

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
# Default configuration
# ---------------------------------------------------------------------------
ARCH="x86_64"
TOOLCHAIN_PREFIX="${VERIDIAN_TOOLCHAIN_PREFIX:-/opt/veridian/toolchain}"
BUILD_DIR="build-veridian"
INSTALL_DIR=""
SOURCE_DIR=""
LLVM_MODE=0
JOBS="$(nproc)"
EXTRA_CMAKE_ARGS=""
CONFIGURE_ONLY=0

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
usage() {
    head -25 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)        ARCH="$2"; shift 2 ;;
        --prefix)      TOOLCHAIN_PREFIX="$2"; shift 2 ;;
        --build-dir)   BUILD_DIR="$2"; shift 2 ;;
        --install-dir) INSTALL_DIR="$2"; shift 2 ;;
        --llvm)        LLVM_MODE=1; shift ;;
        --jobs)        JOBS="$2"; shift 2 ;;
        --cmake-args)  EXTRA_CMAKE_ARGS="$2"; shift 2 ;;
        --configure-only) CONFIGURE_ONLY=1; shift ;;
        --help|-h)     usage ;;
        -*)            die "Unknown option: $1" ;;
        *)             SOURCE_DIR="$1"; shift ;;
    esac
done

if [[ -z "$SOURCE_DIR" ]]; then
    die "No source directory specified. Usage: $0 [OPTIONS] <source-dir>"
fi

SOURCE_DIR="$(cd "$SOURCE_DIR" && pwd)"

# ---------------------------------------------------------------------------
# Validate environment
# ---------------------------------------------------------------------------
step "Validating cross-compilation environment"

TOOLCHAIN_FILE="${SCRIPT_DIR}/cmake/veridian-${ARCH}-toolchain.cmake"
if [[ ! -f "$TOOLCHAIN_FILE" ]]; then
    die "Toolchain file not found: ${TOOLCHAIN_FILE}"
fi

CROSS_GCC="${TOOLCHAIN_PREFIX}/bin/${ARCH}-veridian-gcc"
if [[ ! -x "$CROSS_GCC" ]]; then
    die "Cross-compiler not found: ${CROSS_GCC}
    Run: ./scripts/build-cross-toolchain.sh --arch ${ARCH}"
fi

SYSROOT="${VERIDIAN_SYSROOT:-${TOOLCHAIN_PREFIX}/${ARCH}-veridian/sysroot}"
if [[ ! -d "$SYSROOT" ]]; then
    die "Sysroot not found: ${SYSROOT}
    Run: ./scripts/build-sysroot.sh --arch ${ARCH}"
fi

info "Toolchain:  ${TOOLCHAIN_PREFIX}"
info "Sysroot:    ${SYSROOT}"
info "Source:     ${SOURCE_DIR}"
info "Build:      ${BUILD_DIR}"
info "Arch:       ${ARCH}"
info "Jobs:       ${JOBS}"

success "Environment validated"

# ---------------------------------------------------------------------------
# Prepare build directory
# ---------------------------------------------------------------------------
step "Configuring CMake build"

mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

# ---------------------------------------------------------------------------
# Build CMake arguments
# ---------------------------------------------------------------------------
CMAKE_ARGS=(
    -G Ninja
    -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN_FILE}"
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="${INSTALL_DIR:-${SYSROOT}/usr}"
)

# LLVM-specific configuration
if [[ $LLVM_MODE -eq 1 ]]; then
    info "Enabling LLVM cross-compilation flags"
    CMAKE_ARGS+=(
        # Core LLVM settings for cross-compilation
        -DLLVM_TARGETS_TO_BUILD="X86"
        -DLLVM_DEFAULT_TARGET_TRIPLE="${ARCH}-unknown-veridian"
        -DLLVM_HOST_TRIPLE="${ARCH}-unknown-veridian"
        -DLLVM_TARGET_ARCH="${ARCH}"

        # Build settings
        -DLLVM_ENABLE_PROJECTS="clang;lld"
        -DLLVM_ENABLE_RUNTIMES=""
        -DLLVM_BUILD_TOOLS=ON
        -DLLVM_BUILD_UTILS=ON
        -DLLVM_INCLUDE_BENCHMARKS=OFF
        -DLLVM_INCLUDE_EXAMPLES=OFF
        -DLLVM_INCLUDE_TESTS=OFF
        -DLLVM_INCLUDE_DOCS=OFF

        # Static linking (no shared libs on VeridianOS yet)
        -DLLVM_BUILD_STATIC=ON
        -DLLVM_LINK_LLVM_DYLIB=OFF
        -DBUILD_SHARED_LIBS=OFF

        # Minimize size and dependencies
        -DLLVM_ENABLE_ZLIB=OFF
        -DLLVM_ENABLE_ZSTD=OFF
        -DLLVM_ENABLE_LIBXML2=OFF
        -DLLVM_ENABLE_TERMINFO=OFF
        -DLLVM_ENABLE_LIBEDIT=OFF
        -DLLVM_ENABLE_LIBPFM=OFF

        # Cross-compile tablegen
        -DLLVM_TABLEGEN="$(command -v llvm-tblgen 2>/dev/null || echo /usr/bin/llvm-tblgen)"
        -DCLANG_TABLEGEN="$(command -v clang-tblgen 2>/dev/null || echo /usr/bin/clang-tblgen)"

        # Thread model (VeridianOS pthreads are minimal)
        -DLLVM_ENABLE_THREADS=OFF
        -DLLVM_ENABLE_PIC=OFF

        # Optimize for size
        -DCMAKE_C_FLAGS_RELEASE="-O2 -DNDEBUG"
        -DCMAKE_CXX_FLAGS_RELEASE="-O2 -DNDEBUG"
    )
fi

# Add install prefix
if [[ -n "$INSTALL_DIR" ]]; then
    CMAKE_ARGS+=(-DCMAKE_INSTALL_PREFIX="${INSTALL_DIR}")
fi

# Append extra args
if [[ -n "$EXTRA_CMAKE_ARGS" ]]; then
    # shellcheck disable=SC2206
    CMAKE_ARGS+=(${EXTRA_CMAKE_ARGS})
fi

# Run cmake configure
info "Running cmake configure..."
cmake "${SOURCE_DIR}" "${CMAKE_ARGS[@]}"

success "CMake configure complete"

if [[ $CONFIGURE_ONLY -eq 1 ]]; then
    info "Configure-only mode; stopping before build."
    exit 0
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
step "Building (${JOBS} jobs)"

cmake --build . -j "${JOBS}"

success "Build complete!"

# ---------------------------------------------------------------------------
# Install (if prefix specified)
# ---------------------------------------------------------------------------
if [[ -n "$INSTALL_DIR" ]]; then
    step "Installing to ${INSTALL_DIR}"
    cmake --install .
    success "Installation complete"
fi

echo ""
success "Cross-compilation for VeridianOS (${ARCH}) finished"
info "Build directory: ${BUILD_DIR}"
[[ -n "$INSTALL_DIR" ]] && info "Install directory: ${INSTALL_DIR}"

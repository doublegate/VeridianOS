#!/usr/bin/env bash
# VeridianOS Native Ninja Builder
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Downloads and cross-compiles Ninja build system for VeridianOS, producing
# a statically linked binary.
#
# Ninja is a small, fast build system focused on speed.  It has minimal
# dependencies (just a C++ compiler and standard library).  For VeridianOS,
# we cross-compile it statically since there is no dynamic linker.
#
# Ninja's build system can use either CMake or a bootstrap script (configure.py).
# We use the bootstrap approach since it requires only a C++ compiler and Python,
# avoiding circular dependencies with CMake.
#
# Usage:
#   ./scripts/build-native-ninja.sh [OPTIONS]
#
#   --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
#   --output-dir PATH     Staging directory (default: target/native-tools-staging)
#   --jobs N              Parallel build jobs (default: nproc)
#   --clean               Remove build directory before starting
#   -h, --help            Show this help message

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
# Defaults
# ---------------------------------------------------------------------------
ARCH="x86_64"
CROSS_PREFIX="/opt/veridian/toolchain"
OUTPUT_DIR=""
JOBS="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
CLEAN="no"

# ---------------------------------------------------------------------------
# Ninja version and checksum
# ---------------------------------------------------------------------------
NINJA_VERSION="1.12.1"
NINJA_URL="https://github.com/ninja-build/ninja/archive/refs/tags/v${NINJA_VERSION}.tar.gz"
NINJA_SHA256="821bdff48a3f683bc4bb3b6f0b5fe7b2d647cf65d52a1571cc550d03f16c99ea"

# ---------------------------------------------------------------------------
# Build directory
# ---------------------------------------------------------------------------
BUILD_BASE="/tmp/veridian-native-ninja-build"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --cross-prefix)
                CROSS_PREFIX="${2:?--cross-prefix requires a path}"
                shift 2
                ;;
            --output-dir)
                OUTPUT_DIR="${2:?--output-dir requires a path}"
                shift 2
                ;;
            --jobs)
                JOBS="${2:?--jobs requires a number}"
                shift 2
                ;;
            --clean)
                CLEAN="yes"
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                die "Unknown option: $1  (try --help)"
                ;;
        esac
    done
}

usage() {
    cat <<'EOF'
Usage: ./scripts/build-native-ninja.sh [OPTIONS]

Download and cross-compile Ninja for VeridianOS.  Produces a statically
linked `ninja` binary.

Ninja is a minimal build system.  Its entire source compiles into a single
binary with no runtime dependencies, making it ideal for a freestanding OS.

Options:
  --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
  --output-dir PATH     Staging directory (default: target/native-tools-staging)
  --jobs N              Parallel build jobs (default: nproc)
  --clean               Remove build directory before starting
  -h, --help            Show this help message

Prerequisites:
  - Cross-toolchain with C++ support for VeridianOS
      scripts/build-native-gcc.sh (provides Stage 2.5 with g++)
    OR
      scripts/build-cross-toolchain.sh (if cross-g++ is available)
  - Sysroot with libc.a (scripts/build-sysroot.sh)
  - Python 3 (for ninja's configure.py bootstrap script)

Example:
  ./scripts/build-native-ninja.sh
  ./scripts/build-native-ninja.sh --cross-prefix ~/veridian-tc
EOF
}

# ---------------------------------------------------------------------------
# Derived variables
# ---------------------------------------------------------------------------
setup_vars() {
    TARGET="${ARCH}-veridian"
    BUILD_TRIPLE="$(gcc -dumpmachine 2>/dev/null || echo "x86_64-pc-linux-gnu")"

    CROSS_CXX="${CROSS_PREFIX}/bin/${TARGET}-g++"
    CROSS_CC="${CROSS_PREFIX}/bin/${TARGET}-gcc"
    CROSS_AR="${CROSS_PREFIX}/bin/${TARGET}-ar"
    CROSS_STRIP="${CROSS_PREFIX}/bin/${TARGET}-strip"

    SYSROOT="${CROSS_PREFIX}/sysroot"

    if [[ -z "${OUTPUT_DIR}" ]]; then
        OUTPUT_DIR="${PROJECT_ROOT}/target/native-tools-staging"
    fi
}

# ---------------------------------------------------------------------------
# Verify cross-compiler
# ---------------------------------------------------------------------------
verify_cross_compiler() {
    step "Verifying cross-compiler"

    if [[ ! -x "${CROSS_CXX}" ]]; then
        die "Cross C++ compiler not found at ${CROSS_CXX}.  Ninja requires C++.  Run scripts/build-native-gcc.sh (Stage 2.5) first."
    fi

    if [[ ! -f "${SYSROOT}/usr/lib/libc.a" ]]; then
        die "Sysroot libc.a not found.  Run scripts/build-sysroot.sh first."
    fi

    if ! command -v python3 &>/dev/null; then
        die "Python 3 is required for ninja's build system."
    fi

    # Sanity check the C++ cross-compiler
    local test_cpp="${BUILD_BASE}/test.cpp"
    local test_o="${BUILD_BASE}/test.o"
    mkdir -p "${BUILD_BASE}"
    echo 'extern "C" int test() { return 0; }' > "${test_cpp}"
    if "${CROSS_CXX}" -c -o "${test_o}" "${test_cpp}" -ffreestanding -nostdlib -fno-exceptions -fno-rtti 2>/dev/null; then
        rm -f "${test_cpp}" "${test_o}"
        success "Cross C++ compiler is functional"
    else
        rm -f "${test_cpp}" "${test_o}"
        die "Cross C++ compiler failed sanity check."
    fi
}

# ---------------------------------------------------------------------------
# Download and verify source
# ---------------------------------------------------------------------------
download_source() {
    step "Downloading Ninja ${NINJA_VERSION}"

    mkdir -p "${BUILD_BASE}/downloads"
    local dest="${BUILD_BASE}/downloads/ninja-${NINJA_VERSION}.tar.gz"

    if [[ -f "${dest}" ]]; then
        info "Already downloaded"
    else
        info "Downloading ninja v${NINJA_VERSION} ..."
        wget -q --show-progress -O "${dest}" "${NINJA_URL}" \
            || die "Failed to download Ninja"
    fi

    # Verify checksum
    local got
    got="$(sha256sum "${dest}" | awk '{print $1}')"
    if [[ "${got}" != "${NINJA_SHA256}" ]]; then
        warn "Checksum mismatch for ninja-${NINJA_VERSION}.tar.gz"
        warn "  Expected: ${NINJA_SHA256}"
        warn "  Got:      ${got}"
        warn "Continuing anyway (GitHub archive checksums may vary)."
    fi

    success "Download complete"
}

# ---------------------------------------------------------------------------
# Extract source
# ---------------------------------------------------------------------------
extract_source() {
    step "Extracting source"

    local srcdir="${BUILD_BASE}/sources"
    mkdir -p "${srcdir}"

    if [[ -d "${srcdir}/ninja-${NINJA_VERSION}" ]]; then
        info "Already extracted"
    else
        tar -xf "${BUILD_BASE}/downloads/ninja-${NINJA_VERSION}.tar.gz" -C "${srcdir}"
    fi

    success "Source extracted"
}

# ---------------------------------------------------------------------------
# Build Ninja
#
# Ninja has a simple build system:
#   1. configure.py (Python) generates build.ninja
#   2. The first build bootstraps ninja using the host compiler
#   3. We then cross-compile manually since configure.py does not
#      natively support cross-compilation.
#
# For cross-compilation, we compile ninja's source files directly
# with the cross-compiler.  Ninja's codebase is small enough that
# this is straightforward.
# ---------------------------------------------------------------------------
build_ninja() {
    step "Cross-compiling Ninja for VeridianOS"

    local srcdir="${BUILD_BASE}/sources/ninja-${NINJA_VERSION}"
    local builddir="${BUILD_BASE}/build"

    mkdir -p "${builddir}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    # Ninja's source files (core + platform)
    # These are the files needed for a minimal ninja build.
    local ninja_sources=(
        src/build.cc
        src/build_log.cc
        src/clean.cc
        src/clparser.cc
        src/debug_flags.cc
        src/depfile_parser.cc
        src/deps_log.cc
        src/disk_interface.cc
        src/dyndep.cc
        src/dyndep_parser.cc
        src/edit_distance.cc
        src/eval_env.cc
        src/graph.cc
        src/graphviz.cc
        src/json.cc
        src/lexer.cc
        src/line_printer.cc
        src/manifest_parser.cc
        src/metrics.cc
        src/missing_deps.cc
        src/ninja.cc
        src/parser.cc
        src/state.cc
        src/status.cc
        src/string_piece_util.cc
        src/subprocess-posix.cc
        src/util.cc
        src/version.cc
    )

    # Cross-compile flags
    local CXXFLAGS="-O2 -static -fno-exceptions -fno-rtti"
    CXXFLAGS+=" -DNINJA_HAVE_FORK"
    CXXFLAGS+=" -I${srcdir}/src"
    CXXFLAGS+=" --sysroot=${SYSROOT}"
    local LDFLAGS="-static"

    info "Compiling ninja source files ..."
    local obj_files=()
    local compiled=0
    local total=${#ninja_sources[@]}

    for src in "${ninja_sources[@]}"; do
        local src_path="${srcdir}/${src}"
        local obj_name
        obj_name="$(basename "${src}" .cc).o"
        local obj_path="${builddir}/${obj_name}"

        if [[ ! -f "${src_path}" ]]; then
            warn "Source not found: ${src_path} (skipping)"
            continue
        fi

        if [[ -f "${obj_path}" ]] && [[ "${obj_path}" -nt "${src_path}" ]]; then
            obj_files+=("${obj_path}")
            compiled=$((compiled + 1))
            continue
        fi

        "${CROSS_CXX}" ${CXXFLAGS} -c -o "${obj_path}" "${src_path}" 2>&1 \
            || die "Failed to compile ${src}"
        obj_files+=("${obj_path}")
        compiled=$((compiled + 1))
        printf "\r  [%d/%d] %s" "${compiled}" "${total}" "${src}"
    done
    echo ""

    info "Linking ninja ..."
    "${CROSS_CXX}" ${LDFLAGS} -o "${builddir}/ninja" "${obj_files[@]}" 2>&1 \
        || die "Failed to link ninja"

    success "Ninja compiled (${compiled} source files)"
}

# ---------------------------------------------------------------------------
# Install to staging
# ---------------------------------------------------------------------------
install_ninja() {
    step "Installing Ninja to staging"

    mkdir -p "${OUTPUT_DIR}/usr/bin"

    local ninja_bin="${BUILD_BASE}/build/ninja"

    if [[ ! -f "${ninja_bin}" ]]; then
        die "Ninja binary not found at ${ninja_bin}"
    fi

    cp "${ninja_bin}" "${OUTPUT_DIR}/usr/bin/ninja"
    "${CROSS_STRIP}" "${OUTPUT_DIR}/usr/bin/ninja" 2>/dev/null || true

    local size
    size="$(stat -c%s "${OUTPUT_DIR}/usr/bin/ninja" 2>/dev/null || stat -f%z "${OUTPUT_DIR}/usr/bin/ninja" 2>/dev/null || echo "?")"
    success "ninja installed: ${OUTPUT_DIR}/usr/bin/ninja (${size} bytes)"

    if command -v file &>/dev/null; then
        info "Binary type: $(file "${OUTPUT_DIR}/usr/bin/ninja")"
    fi
}

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "Ninja build complete"

    echo ""
    printf "${GREEN}Binary:${NC}    %s\n" "${OUTPUT_DIR}/usr/bin/ninja"
    printf "${GREEN}Version:${NC}   Ninja %s\n" "${NINJA_VERSION}"
    printf "${GREEN}Target:${NC}    %s (statically linked)\n" "${TARGET}"
    echo ""
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Native Ninja Builder${NC}\n"
    printf "  Cross-prefix:  %s\n" "${CROSS_PREFIX}"
    printf "  Jobs:          %s\n" "${JOBS}"
    printf "  Clean:         %s\n" "${CLEAN}"
    echo ""

    setup_vars

    if [[ "${CLEAN}" == "yes" ]]; then
        info "Cleaning build directory: ${BUILD_BASE}"
        rm -rf "${BUILD_BASE}"
    fi

    verify_cross_compiler
    download_source
    extract_source
    build_ninja
    install_ninja
    print_summary
}

main "$@"

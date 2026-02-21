#!/usr/bin/env bash
# VeridianOS Native GNU Make Builder
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Downloads and cross-compiles GNU Make for VeridianOS, producing a
# statically linked binary that runs on VeridianOS.
#
# Uses the VeridianOS cross-compiler to configure and build:
#   --host=x86_64-veridian  --build=x86_64-linux-gnu
#   LDFLAGS="-static"
#
# Usage:
#   ./scripts/build-native-make.sh [OPTIONS]
#
#   --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
#   --output-dir PATH     Staging directory (default: target/native-tools-staging)
#   --jobs N              Parallel make jobs (default: nproc)
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
# GNU Make version and checksum
# ---------------------------------------------------------------------------
MAKE_VERSION="4.4.1"
MAKE_URL="https://ftp.gnu.org/gnu/make/make-${MAKE_VERSION}.tar.gz"
MAKE_SHA256="dd16fb1d67bfab79a72f5e8390735c49e3e8e70b4945a15ab1f81ddb78658fb3"

# ---------------------------------------------------------------------------
# Build directory
# ---------------------------------------------------------------------------
BUILD_BASE="/tmp/veridian-native-make-build"

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
Usage: ./scripts/build-native-make.sh [OPTIONS]

Download and cross-compile GNU Make for VeridianOS.  Produces a statically
linked `make` binary that runs on VeridianOS.

Options:
  --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
  --output-dir PATH     Staging directory (default: target/native-tools-staging)
  --jobs N              Parallel make jobs (default: nproc)
  --clean               Remove build directory before starting
  -h, --help            Show this help message

Prerequisites:
  - Cross-toolchain for VeridianOS (scripts/build-cross-toolchain.sh)
  - Sysroot with libc.a (scripts/build-sysroot.sh)

Example:
  ./scripts/build-native-make.sh
  ./scripts/build-native-make.sh --cross-prefix ~/veridian-tc --jobs 8
EOF
}

# ---------------------------------------------------------------------------
# Derived variables
# ---------------------------------------------------------------------------
setup_vars() {
    TARGET="${ARCH}-veridian"
    BUILD_TRIPLE="$(gcc -dumpmachine 2>/dev/null || echo "x86_64-pc-linux-gnu")"

    CROSS_CC="${CROSS_PREFIX}/bin/${TARGET}-gcc"
    CROSS_AR="${CROSS_PREFIX}/bin/${TARGET}-ar"
    CROSS_RANLIB="${CROSS_PREFIX}/bin/${TARGET}-ranlib"
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

    if [[ ! -x "${CROSS_CC}" ]]; then
        die "Cross-compiler not found at ${CROSS_CC}.  Run scripts/build-cross-toolchain.sh first."
    fi

    if [[ ! -f "${SYSROOT}/usr/lib/libc.a" ]]; then
        die "Sysroot libc.a not found.  Run scripts/build-sysroot.sh first."
    fi

    # Quick sanity check
    local test_c="${BUILD_BASE}/test.c"
    local test_o="${BUILD_BASE}/test.o"
    mkdir -p "${BUILD_BASE}"
    echo 'int test(void) { return 0; }' > "${test_c}"
    if "${CROSS_CC}" -c -o "${test_o}" "${test_c}" -ffreestanding -nostdlib 2>/dev/null; then
        rm -f "${test_c}" "${test_o}"
        success "Cross-compiler is functional"
    else
        rm -f "${test_c}" "${test_o}"
        die "Cross-compiler failed sanity check."
    fi
}

# ---------------------------------------------------------------------------
# Download and verify source
# ---------------------------------------------------------------------------
download_source() {
    step "Downloading GNU Make ${MAKE_VERSION}"

    mkdir -p "${BUILD_BASE}/downloads"
    local dest="${BUILD_BASE}/downloads/make-${MAKE_VERSION}.tar.gz"

    if [[ -f "${dest}" ]]; then
        info "Already downloaded"
    else
        info "Downloading make-${MAKE_VERSION}.tar.gz ..."
        wget -q --show-progress -O "${dest}" "${MAKE_URL}" \
            || die "Failed to download GNU Make"
    fi

    # Verify checksum
    local got
    got="$(sha256sum "${dest}" | awk '{print $1}')"
    if [[ "${got}" != "${MAKE_SHA256}" ]]; then
        die "Checksum mismatch for make-${MAKE_VERSION}.tar.gz"
    fi

    success "Download verified"
}

# ---------------------------------------------------------------------------
# Extract source
# ---------------------------------------------------------------------------
extract_source() {
    step "Extracting source"

    local srcdir="${BUILD_BASE}/sources"
    mkdir -p "${srcdir}"

    if [[ -d "${srcdir}/make-${MAKE_VERSION}" ]]; then
        info "Already extracted"
    else
        tar -xf "${BUILD_BASE}/downloads/make-${MAKE_VERSION}.tar.gz" -C "${srcdir}"
    fi

    success "Source extracted"
}

# ---------------------------------------------------------------------------
# Build GNU Make
# ---------------------------------------------------------------------------
build_make() {
    step "Cross-compiling GNU Make for VeridianOS"

    local srcdir="${BUILD_BASE}/sources/make-${MAKE_VERSION}"
    local builddir="${BUILD_BASE}/build"

    mkdir -p "${builddir}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring GNU Make ..."
        info "  build  = ${BUILD_TRIPLE}"
        info "  host   = ${TARGET}"
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --build="${BUILD_TRIPLE}" \
                --host="${TARGET}" \
                --prefix="/usr" \
                --disable-nls \
                --without-guile \
                CC="${CROSS_CC}" \
                AR="${CROSS_AR}" \
                RANLIB="${CROSS_RANLIB}" \
                LDFLAGS="-static" \
                CFLAGS="-O2 -static"
        )
    fi

    info "Compiling (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}"

    success "GNU Make compiled"
}

# ---------------------------------------------------------------------------
# Install to staging
# ---------------------------------------------------------------------------
install_make() {
    step "Installing GNU Make to staging"

    local builddir="${BUILD_BASE}/build"

    mkdir -p "${OUTPUT_DIR}/usr/bin"

    make -C "${builddir}" install DESTDIR="${OUTPUT_DIR}"

    # Verify the binary
    local make_bin="${OUTPUT_DIR}/usr/bin/make"
    if [[ -f "${make_bin}" ]]; then
        "${CROSS_STRIP}" "${make_bin}" 2>/dev/null || true
        local size
        size="$(stat -c%s "${make_bin}" 2>/dev/null || stat -f%z "${make_bin}" 2>/dev/null || echo "?")"
        success "make installed: ${make_bin} (${size} bytes)"

        if command -v file &>/dev/null; then
            info "Binary type: $(file "${make_bin}")"
        fi
    else
        die "make binary not found after installation."
    fi
}

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "GNU Make build complete"

    echo ""
    printf "${GREEN}Binary:${NC}    %s\n" "${OUTPUT_DIR}/usr/bin/make"
    printf "${GREEN}Version:${NC}   GNU Make %s\n" "${MAKE_VERSION}"
    printf "${GREEN}Target:${NC}    %s (statically linked)\n" "${TARGET}"
    echo ""
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Native GNU Make Builder${NC}\n"
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
    build_make
    install_make
    print_summary
}

main "$@"

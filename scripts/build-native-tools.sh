#!/usr/bin/env bash
# VeridianOS Native Build Tools Orchestrator
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Master script that orchestrates building all native user-space tools
# needed for self-hosting VeridianOS.  Calls individual build scripts
# and packages everything into a rootfs TAR.
#
# Components built:
#   1. GNU Make      -- build automation (autotools, Makefiles)
#   2. Ninja         -- fast parallel build system (CMake backend)
#   3. Coreutils     -- essential shell utilities (optional)
#
# Prerequisites:
#   - Cross-toolchain: scripts/build-cross-toolchain.sh
#   - C++ cross-compiler: scripts/build-native-gcc.sh (Stage 2.5)
#   - Sysroot: scripts/build-sysroot.sh
#
# Usage:
#   ./scripts/build-native-tools.sh [OPTIONS]
#
#   --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
#   --output PATH         Output TAR (default: target/native-tools.tar)
#   --with-coreutils      Also build coreutils essentials
#   --only TOOL           Build only this tool: make, ninja, coreutils
#   --jobs N              Parallel make jobs (default: nproc)
#   --clean               Clean before building
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
OUTPUT_TAR=""
WITH_COREUTILS="no"
ONLY_TOOL=""
JOBS="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
CLEAN="no"

# Coreutils configuration
COREUTILS_VERSION="9.5"
COREUTILS_URL="https://ftp.gnu.org/gnu/coreutils/coreutils-${COREUTILS_VERSION}.tar.xz"
COREUTILS_SHA256="cd328edeac92f6a665de9f323c93b712af1571f2571ac906b3c56f2a41c52e81"

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
            --output)
                OUTPUT_TAR="${2:?--output requires a path}"
                shift 2
                ;;
            --with-coreutils)
                WITH_COREUTILS="yes"
                shift
                ;;
            --only)
                ONLY_TOOL="${2:?--only requires: make, ninja, or coreutils}"
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

    if [[ -n "${ONLY_TOOL}" ]]; then
        case "${ONLY_TOOL}" in
            make|ninja|coreutils) ;;
            *) die "Unknown tool: ${ONLY_TOOL}.  Use: make, ninja, or coreutils" ;;
        esac
    fi
}

usage() {
    cat <<'EOF'
Usage: ./scripts/build-native-tools.sh [OPTIONS]

Orchestrate building all native user-space build tools for VeridianOS.

Components:
  make          GNU Make (always built)
  ninja         Ninja build system (always built)
  coreutils     Essential shell utilities (optional, --with-coreutils)

Options:
  --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
  --output PATH         Output TAR (default: target/native-tools.tar)
  --with-coreutils      Also build coreutils essentials (cat, cp, mv, rm, mkdir, ls)
  --only TOOL           Build only: make, ninja, or coreutils
  --jobs N              Parallel make jobs (default: nproc)
  --clean               Clean build directories before building
  -h, --help            Show this help message

Prerequisites:
  - Cross-toolchain: ./scripts/build-cross-toolchain.sh
  - C++ support:     ./scripts/build-native-gcc.sh (Stage 2.5, for ninja)
  - Sysroot:         ./scripts/build-sysroot.sh

Examples:
  # Build make and ninja
  ./scripts/build-native-tools.sh

  # Build everything including coreutils
  ./scripts/build-native-tools.sh --with-coreutils

  # Build only ninja
  ./scripts/build-native-tools.sh --only ninja

  # Custom output, clean build
  ./scripts/build-native-tools.sh --clean --output ~/veridian-tools.tar
EOF
}

# ---------------------------------------------------------------------------
# Derived variables
# ---------------------------------------------------------------------------
setup_vars() {
    TARGET="${ARCH}-veridian"
    STAGING="${PROJECT_ROOT}/target/native-tools-staging"

    CROSS_CC="${CROSS_PREFIX}/bin/${TARGET}-gcc"
    CROSS_CXX="${CROSS_PREFIX}/bin/${TARGET}-g++"
    CROSS_STRIP="${CROSS_PREFIX}/bin/${TARGET}-strip"
    SYSROOT="${CROSS_PREFIX}/sysroot"

    if [[ -z "${OUTPUT_TAR}" ]]; then
        OUTPUT_TAR="${PROJECT_ROOT}/target/native-tools.tar"
    fi
}

# ---------------------------------------------------------------------------
# Verify prerequisites
# ---------------------------------------------------------------------------
verify_prerequisites() {
    step "Verifying prerequisites"

    if [[ ! -x "${CROSS_CC}" ]]; then
        die "Cross-compiler not found at ${CROSS_CC}."
    fi

    if [[ ! -f "${SYSROOT}/usr/lib/libc.a" ]]; then
        die "Sysroot libc.a not found.  Run scripts/build-sysroot.sh first."
    fi

    success "Prerequisites verified"
}

# ---------------------------------------------------------------------------
# Build GNU Make
# ---------------------------------------------------------------------------
build_make() {
    step "Building GNU Make"

    local args=(
        --cross-prefix "${CROSS_PREFIX}"
        --output-dir "${STAGING}"
        --jobs "${JOBS}"
    )

    if [[ "${CLEAN}" == "yes" ]]; then
        args+=(--clean)
    fi

    "${SCRIPT_DIR}/build-native-make.sh" "${args[@]}"

    if [[ -f "${STAGING}/usr/bin/make" ]]; then
        success "GNU Make built successfully"
    else
        die "GNU Make build failed."
    fi
}

# ---------------------------------------------------------------------------
# Build Ninja
# ---------------------------------------------------------------------------
build_ninja() {
    step "Building Ninja"

    # Ninja requires C++ cross-compiler
    if [[ ! -x "${CROSS_CXX}" ]]; then
        warn "Cross C++ compiler not found at ${CROSS_CXX}."
        warn "Ninja requires C++.  Skipping."
        warn "Run scripts/build-native-gcc.sh (Stage 2.5) to enable."
        return 0
    fi

    local args=(
        --cross-prefix "${CROSS_PREFIX}"
        --output-dir "${STAGING}"
        --jobs "${JOBS}"
    )

    if [[ "${CLEAN}" == "yes" ]]; then
        args+=(--clean)
    fi

    "${SCRIPT_DIR}/build-native-ninja.sh" "${args[@]}"

    if [[ -f "${STAGING}/usr/bin/ninja" ]]; then
        success "Ninja built successfully"
    else
        warn "Ninja build may have failed."
    fi
}

# ---------------------------------------------------------------------------
# Build coreutils essentials
#
# Cross-compiles a subset of GNU coreutils for VeridianOS.
# Only the most essential utilities are built to keep the rootfs small.
# ---------------------------------------------------------------------------
build_coreutils() {
    step "Building coreutils essentials"

    local build_base="/tmp/veridian-native-coreutils-build"

    if [[ "${CLEAN}" == "yes" ]]; then
        rm -rf "${build_base}"
    fi

    mkdir -p "${build_base}/downloads"
    mkdir -p "${build_base}/sources"

    # Download
    local tarball="${build_base}/downloads/coreutils-${COREUTILS_VERSION}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        info "Downloading coreutils ${COREUTILS_VERSION} ..."
        wget -q --show-progress -O "${tarball}" "${COREUTILS_URL}" \
            || die "Failed to download coreutils"
    fi

    # Verify checksum
    local got
    got="$(sha256sum "${tarball}" | awk '{print $1}')"
    if [[ "${got}" != "${COREUTILS_SHA256}" ]]; then
        warn "Checksum mismatch for coreutils (expected ${COREUTILS_SHA256}, got ${got})"
        warn "Continuing anyway (tarball may have been updated)."
    fi

    # Extract
    if [[ ! -d "${build_base}/sources/coreutils-${COREUTILS_VERSION}" ]]; then
        info "Extracting coreutils ..."
        tar -xf "${tarball}" -C "${build_base}/sources"
    fi

    local srcdir="${build_base}/sources/coreutils-${COREUTILS_VERSION}"
    local builddir="${build_base}/build"
    mkdir -p "${builddir}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    local build_triple
    build_triple="$(gcc -dumpmachine 2>/dev/null || echo "x86_64-pc-linux-gnu")"

    # Configure
    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring coreutils ..."
        (
            cd "${builddir}"
            # Many coreutils features require runtime tests that fail during
            # cross-compilation.  We disable problematic features and force
            # certain cache variables.
            "${srcdir}/configure" \
                --build="${build_triple}" \
                --host="${TARGET}" \
                --prefix="/usr" \
                --disable-nls \
                --without-gmp \
                --without-openssl \
                --without-selinux \
                --disable-acl \
                --disable-xattr \
                --disable-libcap \
                CC="${CROSS_CC}" \
                LDFLAGS="-static" \
                CFLAGS="-O2 -static" \
                FORCE_UNSAFE_CONFIGURE=1 \
                gl_cv_func_working_mktime=yes \
                gl_cv_func_working_utimes=yes \
                fu_cv_sys_stat_statfs2_bsize=yes \
                ac_cv_func_renameat2=no
        )
    fi

    info "Compiling coreutils (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}" 2>&1 || true

    # Install only the tools we want
    local essential_tools=(cat cp mv rm mkdir ls echo chmod chown head tail wc)
    local installed=0

    mkdir -p "${STAGING}/usr/bin"

    for tool in "${essential_tools[@]}"; do
        local tool_bin="${builddir}/src/${tool}"
        if [[ -f "${tool_bin}" ]]; then
            cp "${tool_bin}" "${STAGING}/usr/bin/${tool}"
            "${CROSS_STRIP}" "${STAGING}/usr/bin/${tool}" 2>/dev/null || true
            local size
            size="$(stat -c%s "${STAGING}/usr/bin/${tool}" 2>/dev/null || echo "?")"
            info "  + ${tool} (${size} bytes)"
            installed=$((installed + 1))
        else
            warn "  - ${tool} (build failed or not found)"
        fi
    done

    if [[ ${installed} -gt 0 ]]; then
        success "Coreutils: ${installed}/${#essential_tools[@]} tools installed"
    else
        warn "No coreutils tools were successfully compiled."
        warn "This is expected if the libc is incomplete."
    fi
}

# ---------------------------------------------------------------------------
# Package all tools into TAR
# ---------------------------------------------------------------------------
package_tools() {
    step "Packaging tools into TAR"

    if [[ ! -d "${STAGING}" ]]; then
        die "Staging directory not found: ${STAGING}"
    fi

    mkdir -p "$(dirname "${OUTPUT_TAR}")"

    # Count what we have
    local bin_count=0
    if [[ -d "${STAGING}/usr/bin" ]]; then
        bin_count="$(find "${STAGING}/usr/bin" -type f -executable 2>/dev/null | wc -l)"
    fi

    if [[ ${bin_count} -eq 0 ]]; then
        die "No tools were built.  Nothing to package."
    fi

    (
        cd "${STAGING}"
        tar cf "${OUTPUT_TAR}" .
    )

    local size
    size="$(stat -c%s "${OUTPUT_TAR}" 2>/dev/null || stat -f%z "${OUTPUT_TAR}" 2>/dev/null || echo "?")"

    info "Archive: ${OUTPUT_TAR}"
    info "Size: ${size} bytes ($((${size:-0} / 1024)) KB)"
    info "Binaries: ${bin_count}"

    success "Tools packaged"
}

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "Build Summary"

    echo ""
    printf "${GREEN}Output:${NC}  %s\n" "${OUTPUT_TAR}"
    echo ""

    echo "Installed tools:"
    if [[ -d "${STAGING}/usr/bin" ]]; then
        for f in "${STAGING}/usr/bin/"*; do
            [[ -f "${f}" ]] || continue
            local name
            name="$(basename "${f}")"
            local size
            size="$(stat -c%s "${f}" 2>/dev/null || stat -f%z "${f}" 2>/dev/null || echo "?")"
            printf "  %-16s %s KB\n" "${name}" "$((${size:-0} / 1024))"
        done
    fi
    echo ""

    echo "To use in QEMU (as additional drive):"
    echo "  -drive file=${OUTPUT_TAR},if=none,id=vd1,format=raw \\"
    echo "  -device virtio-blk-pci,drive=vd1"
    echo ""

    echo "To merge with rootfs:"
    echo "  ./scripts/package-native-toolchain.sh --merge target/rootfs.tar \\"
    echo "      --input-dir ${STAGING}"
    echo ""
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Native Build Tools Orchestrator${NC}\n"
    printf "  Cross-prefix:     %s\n" "${CROSS_PREFIX}"
    printf "  Jobs:             %s\n" "${JOBS}"
    printf "  With coreutils:   %s\n" "${WITH_COREUTILS}"
    if [[ -n "${ONLY_TOOL}" ]]; then
        printf "  Only:             %s\n" "${ONLY_TOOL}"
    fi
    printf "  Clean:            %s\n" "${CLEAN}"
    echo ""

    setup_vars

    if [[ "${CLEAN}" == "yes" ]] && [[ -d "${STAGING}" ]]; then
        info "Cleaning staging directory: ${STAGING}"
        rm -rf "${STAGING}"
    fi

    verify_prerequisites

    if [[ -n "${ONLY_TOOL}" ]]; then
        case "${ONLY_TOOL}" in
            make)       build_make ;;
            ninja)      build_ninja ;;
            coreutils)  build_coreutils ;;
        esac
    else
        build_make
        build_ninja
        if [[ "${WITH_COREUTILS}" == "yes" ]]; then
            build_coreutils
        fi
    fi

    package_tools
    print_summary
}

main "$@"

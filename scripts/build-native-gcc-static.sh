#!/usr/bin/env bash
# VeridianOS Static Native GCC Builder
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds a STATIC native GCC toolchain that runs ON VeridianOS.
# This is a streamlined script that assumes the cross-compiler already
# exists (built by build-native-gcc.sh or build-cross-toolchain.sh).
#
# Unlike build-native-gcc.sh (which performs the full Canadian cross from
# scratch including Stage 2.5), this script focuses on producing statically
# linked binaries suitable for inclusion in a VeridianOS root filesystem.
#
# Build phases:
#   Phase 1: Verify cross-compiler and sysroot exist
#   Phase 2: Build static binutils (as, ld, ar, ranlib, nm, objcopy, strip)
#   Phase 3: Build static libgcc
#   Phase 4: Build static GCC (cc1 + gcc driver)
#   Phase 5: Package into rootfs directory structure
#
# Prerequisites:
#   - Cross-toolchain with C++ support (Stage 2.5):
#       scripts/build-native-gcc.sh (builds the full Canadian cross pipeline)
#     OR manually:
#       scripts/build-cross-toolchain.sh (Stage 1+2)
#       then rebuild with --enable-languages=c,c++ for Stage 2.5
#   - Sysroot with headers, CRT objects, and libc.a:
#       scripts/build-sysroot.sh
#
# Usage:
#   ./scripts/build-native-gcc-static.sh [OPTIONS]
#
#   --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
#   --output-dir PATH     Output directory for static binaries (default: target/native-gcc-static)
#   --jobs N              Parallel make jobs (default: nproc)
#   --clean               Remove build directory before starting
#   -h, --help            Show this help message

set -euo pipefail

# ---------------------------------------------------------------------------
# Color helpers (consistent with other VeridianOS scripts)
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
# Version table (must match build-native-gcc.sh / build-cross-toolchain.sh)
# ---------------------------------------------------------------------------
BINUTILS_VERSION="2.43"
GCC_VERSION="14.2.0"
GMP_VERSION="6.3.0"
MPFR_VERSION="4.2.1"
MPC_VERSION="1.3.1"

BINUTILS_URL="https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
GCC_URL="https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
GMP_URL="https://ftp.gnu.org/gnu/gmp/gmp-${GMP_VERSION}.tar.xz"
MPFR_URL="https://ftp.gnu.org/gnu/mpfr/mpfr-${MPFR_VERSION}.tar.xz"
MPC_URL="https://ftp.gnu.org/gnu/mpc/mpc-${MPC_VERSION}.tar.gz"

BINUTILS_SHA256="b53606f443ac8f01d1d5fc9c39497f2af322d99e14cea5c0b4b124d630379365"
GCC_SHA256="a7b39bc69cbf9e25826c5a60ab26477001f7c08d85cec04bc0e29cabed6f3cc9"
GMP_SHA256="a3c2b80201b89e68616f4ad30bc66aee4927c3ce50e33929ca819d5c43538898"
MPFR_SHA256="277807353a6726978996945af13e52829e3abd7a9a5b7fb2793894e18f1fcbb2"
MPC_SHA256="ab642492f5cf882b74aa0cb730cd410a81edcdbec895183ce930e706c1c759b8"

# ---------------------------------------------------------------------------
# Build directory
# ---------------------------------------------------------------------------
BUILD_BASE="/tmp/veridian-native-gcc-static"

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
Usage: ./scripts/build-native-gcc-static.sh [OPTIONS]

Build a statically-linked native GCC toolchain for VeridianOS.  The resulting
binaries run ON VeridianOS and compile C programs FOR VeridianOS.

This script uses the existing cross-compiler to perform a Canadian
cross-compilation, producing static ELF binaries:
  gcc, cc1, as, ld, ar, ranlib, nm, objcopy, strip

Phases:
  1. Verify cross-compiler exists and works
  2. Build static binutils (as, ld, ar, ranlib, nm, objcopy, strip)
  3. Build static libgcc
  4. Build static GCC (cc1 + gcc driver)
  5. Package into rootfs directory structure

Options:
  --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
  --output-dir PATH     Output directory (default: target/native-gcc-static)
  --jobs N              Parallel make jobs (default: nproc)
  --clean               Remove build directory before starting
  -h, --help            Show this help message

Prerequisites:
  - Cross-toolchain with C++ support (Stage 2.5):
      ./scripts/build-native-gcc.sh  (or build-cross-toolchain.sh + Stage 2.5)
  - Sysroot with libc.a, crt0.o, headers:
      ./scripts/build-sysroot.sh

Example:
  # Build with default settings
  ./scripts/build-native-gcc-static.sh

  # Custom cross-toolchain location, 8 parallel jobs
  ./scripts/build-native-gcc-static.sh --cross-prefix ~/veridian-tc --jobs 8

  # Clean rebuild
  ./scripts/build-native-gcc-static.sh --clean

Output:
  target/native-gcc-static/
    usr/
      bin/      gcc, as, ld, ar, ranlib, nm, objcopy, strip
      lib/      libgcc.a, crt*.o
      libexec/  cc1
      include/  C headers
EOF
}

# ---------------------------------------------------------------------------
# Derived variables
# ---------------------------------------------------------------------------
setup_vars() {
    TARGET="${ARCH}-veridian"

    BUILD_TRIPLE="$(gcc -dumpmachine 2>/dev/null || echo "x86_64-pc-linux-gnu")"

    # Cross-toolchain tools
    CROSS_CC="${CROSS_PREFIX}/bin/${TARGET}-gcc"
    CROSS_CXX="${CROSS_PREFIX}/bin/${TARGET}-g++"
    CROSS_AR="${CROSS_PREFIX}/bin/${TARGET}-ar"
    CROSS_AS="${CROSS_PREFIX}/bin/${TARGET}-as"
    CROSS_LD="${CROSS_PREFIX}/bin/${TARGET}-ld"
    CROSS_RANLIB="${CROSS_PREFIX}/bin/${TARGET}-ranlib"
    CROSS_STRIP="${CROSS_PREFIX}/bin/${TARGET}-strip"

    SYSROOT="${CROSS_PREFIX}/sysroot"

    # Output directory
    if [[ -z "${OUTPUT_DIR}" ]]; then
        OUTPUT_DIR="${PROJECT_ROOT}/target/native-gcc-static"
    fi

    # Staging directory (DESTDIR for make install)
    STAGING="${BUILD_BASE}/staging"

    # Native prefix (where GCC expects to find itself on VeridianOS)
    NATIVE_PREFIX="/usr"

    # Log file
    LOG_DIR="${BUILD_BASE}/logs"
}

# ---------------------------------------------------------------------------
# Phase 1: Verify cross-compiler exists and works
# ---------------------------------------------------------------------------
phase1_verify_cross_compiler() {
    step "Phase 1: Verifying cross-compiler"

    # 1a. Host build tools
    local missing=()
    for cmd in gcc g++ make tar wget sha256sum patch bison flex; do
        if ! command -v "${cmd}" &>/dev/null; then
            missing+=("${cmd}")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        die "Missing host tools: ${missing[*]}"
    fi
    success "Host build tools found"

    # 1b. Cross-compiler binaries
    if [[ ! -x "${CROSS_CC}" ]]; then
        die "Cross-compiler not found at ${CROSS_CC}.  Run scripts/build-cross-toolchain.sh first."
    fi

    if [[ ! -x "${CROSS_CXX}" ]]; then
        die "Cross C++ compiler not found at ${CROSS_CXX}.  Run scripts/build-native-gcc.sh first (Stage 2.5 needed)."
    fi

    if [[ ! -x "${CROSS_AS}" ]]; then
        die "Cross-assembler not found at ${CROSS_AS}."
    fi

    if [[ ! -x "${CROSS_LD}" ]]; then
        die "Cross-linker not found at ${CROSS_LD}."
    fi

    success "Cross-toolchain found at ${CROSS_PREFIX}"

    # 1c. Sysroot
    if [[ ! -f "${SYSROOT}/usr/lib/libc.a" ]]; then
        die "Sysroot libc.a not found at ${SYSROOT}/usr/lib/libc.a.  Run scripts/build-sysroot.sh first."
    fi

    if [[ ! -f "${SYSROOT}/usr/lib/crt0.o" ]]; then
        die "Sysroot crt0.o not found at ${SYSROOT}/usr/lib/crt0.o.  Run scripts/build-sysroot.sh first."
    fi

    success "Sysroot verified at ${SYSROOT}"

    # 1d. Sanity test
    local test_c="${BUILD_BASE}/test_cross.c"
    local test_o="${BUILD_BASE}/test_cross.o"
    mkdir -p "${BUILD_BASE}"
    echo 'int veridian_test(void) { return 42; }' > "${test_c}"
    if "${CROSS_CC}" -c -o "${test_o}" "${test_c}" -ffreestanding -nostdlib 2>/dev/null; then
        rm -f "${test_c}" "${test_o}"
        success "Cross-compiler produces valid object files"
    else
        rm -f "${test_c}" "${test_o}"
        die "Cross-compiler failed sanity check."
    fi

    # 1e. C++ cross-compiler sanity test
    local test_cpp="${BUILD_BASE}/test_cxx.cpp"
    local test_o2="${BUILD_BASE}/test_cxx.o"
    echo 'extern "C" int test() { return 1; }' > "${test_cpp}"
    if "${CROSS_CXX}" -c -o "${test_o2}" "${test_cpp}" -ffreestanding -nostdlib -fno-exceptions -fno-rtti 2>/dev/null; then
        rm -f "${test_cpp}" "${test_o2}"
        success "Cross C++ compiler is functional"
    else
        rm -f "${test_cpp}" "${test_o2}"
        die "Cross C++ compiler failed sanity check."
    fi
}

# ---------------------------------------------------------------------------
# Download, verify, extract sources (reuses cached downloads)
# ---------------------------------------------------------------------------
prepare_sources() {
    step "Preparing source tarballs"

    mkdir -p "${BUILD_BASE}/downloads"
    mkdir -p "${BUILD_BASE}/sources"
    mkdir -p "${LOG_DIR}"

    # Try to reuse downloads from cross-toolchain or native-gcc builds
    local other_dl_dirs=(
        "/tmp/veridian-toolchain-build/downloads"
        "/tmp/veridian-native-gcc-build/downloads"
    )

    local urls=(
        "${BINUTILS_URL}"
        "${GCC_URL}"
        "${GMP_URL}"
        "${MPFR_URL}"
        "${MPC_URL}"
    )

    for url in "${urls[@]}"; do
        local filename
        filename="$(basename "${url}")"
        local dest="${BUILD_BASE}/downloads/${filename}"

        if [[ -f "${dest}" ]]; then
            info "Already downloaded: ${filename}"
            continue
        fi

        # Try to reuse from other build directories
        local found="no"
        for dl_dir in "${other_dl_dirs[@]}"; do
            if [[ -f "${dl_dir}/${filename}" ]]; then
                info "Reusing from ${dl_dir}: ${filename}"
                ln -sf "${dl_dir}/${filename}" "${dest}"
                found="yes"
                break
            fi
        done

        if [[ "${found}" == "no" ]]; then
            info "Downloading ${filename} ..."
            wget -q --show-progress -O "${dest}" "${url}" \
                || die "Failed to download ${url}"
        fi
    done

    # Verify checksums
    local -A expected=(
        ["binutils-${BINUTILS_VERSION}.tar.xz"]="${BINUTILS_SHA256}"
        ["gcc-${GCC_VERSION}.tar.xz"]="${GCC_SHA256}"
        ["gmp-${GMP_VERSION}.tar.xz"]="${GMP_SHA256}"
        ["mpfr-${MPFR_VERSION}.tar.xz"]="${MPFR_SHA256}"
        ["mpc-${MPC_VERSION}.tar.gz"]="${MPC_SHA256}"
    )

    for filename in "${!expected[@]}"; do
        local file="${BUILD_BASE}/downloads/${filename}"
        local want="${expected[${filename}]}"
        local got
        got="$(sha256sum "$(readlink -f "${file}")" | awk '{print $1}')"

        if [[ "${got}" != "${want}" ]]; then
            die "Checksum mismatch for ${filename}!"
        fi
        info "Checksum OK: ${filename}"
    done

    # Extract
    local srcdir="${BUILD_BASE}/sources"
    for archive in "${BUILD_BASE}/downloads"/*.tar.*; do
        local real_archive
        real_archive="$(readlink -f "${archive}")"
        local name
        name="$(basename "${archive}")"
        local dirname="${name%.tar.*}"

        if [[ -d "${srcdir}/${dirname}" ]]; then
            info "Already extracted: ${dirname}"
        else
            info "Extracting ${name} ..."
            tar -xf "${real_archive}" -C "${srcdir}"
        fi
    done

    success "All sources prepared"
}

# ---------------------------------------------------------------------------
# Apply VeridianOS patches
# ---------------------------------------------------------------------------
apply_patches() {
    step "Applying VeridianOS patches"

    local srcdir="${BUILD_BASE}/sources"
    local patches_dir

    # Binutils patches
    patches_dir="${PROJECT_ROOT}/ports/binutils/patches"
    if [[ -d "${patches_dir}" ]]; then
        for patch_file in "${patches_dir}"/*.patch; do
            [[ -f "${patch_file}" ]] || continue
            local pname
            pname="$(basename "${patch_file}")"
            local stamp="${srcdir}/binutils-${BINUTILS_VERSION}/.applied-${pname}"
            if [[ -f "${stamp}" ]]; then
                info "Patch already applied: binutils/${pname}"
            else
                info "Applying: binutils/${pname}"
                patch -d "${srcdir}/binutils-${BINUTILS_VERSION}" -p1 < "${patch_file}" \
                    || die "Failed to apply patch: ${patch_file}"
                touch "${stamp}"
            fi
        done
    fi

    # GCC patches
    patches_dir="${PROJECT_ROOT}/ports/gcc/patches"
    if [[ -d "${patches_dir}" ]]; then
        for patch_file in "${patches_dir}"/*.patch; do
            [[ -f "${patch_file}" ]] || continue
            local pname
            pname="$(basename "${patch_file}")"
            local stamp="${srcdir}/gcc-${GCC_VERSION}/.applied-${pname}"
            if [[ -f "${stamp}" ]]; then
                info "Patch already applied: gcc/${pname}"
            else
                info "Applying: gcc/${pname}"
                patch -d "${srcdir}/gcc-${GCC_VERSION}" -p1 < "${patch_file}" \
                    || die "Failed to apply patch: ${patch_file}"
                touch "${stamp}"
            fi
        done
    fi

    # Symlink GMP/MPFR/MPC into GCC source tree
    local gcc_src="${srcdir}/gcc-${GCC_VERSION}"
    ln -sfn "${srcdir}/gmp-${GMP_VERSION}"  "${gcc_src}/gmp"
    ln -sfn "${srcdir}/mpfr-${MPFR_VERSION}" "${gcc_src}/mpfr"
    ln -sfn "${srcdir}/mpc-${MPC_VERSION}"   "${gcc_src}/mpc"

    success "Patches applied"
}

# ---------------------------------------------------------------------------
# Phase 2: Build static binutils
# ---------------------------------------------------------------------------
phase2_build_static_binutils() {
    step "Phase 2: Building static binutils (Canadian cross, host=veridian)"

    local builddir="${BUILD_BASE}/build-binutils"
    local srcdir="${BUILD_BASE}/sources/binutils-${BINUTILS_VERSION}"

    mkdir -p "${builddir}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring binutils ..."
        info "  build  = ${BUILD_TRIPLE}"
        info "  host   = ${TARGET}"
        info "  target = ${TARGET}"
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --build="${BUILD_TRIPLE}" \
                --host="${TARGET}" \
                --target="${TARGET}" \
                --prefix="${NATIVE_PREFIX}" \
                --with-sysroot="/" \
                --disable-nls \
                --disable-werror \
                --disable-gdb \
                --disable-gdbserver \
                --disable-sim \
                --disable-readline \
                --disable-shared \
                --enable-static \
                --enable-deterministic-archives \
                --enable-new-dtags \
                --enable-default-hash-style=gnu \
                CC="${CROSS_CC}" \
                CXX="${CROSS_CXX}" \
                AR="${CROSS_AR}" \
                AS="${CROSS_AS}" \
                LD="${CROSS_LD}" \
                RANLIB="${CROSS_RANLIB}" \
                STRIP="${CROSS_STRIP}" \
                LDFLAGS="-static" \
                CFLAGS="-O2 -static" \
                CXXFLAGS="-O2 -static" \
                2>&1 | tee "${LOG_DIR}/binutils-configure.log"
        )
    fi

    info "Compiling static binutils (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}" 2>&1 | tee "${LOG_DIR}/binutils-build.log"

    info "Installing static binutils to staging ..."
    make -C "${builddir}" install DESTDIR="${STAGING}" 2>&1 | tee "${LOG_DIR}/binutils-install.log"

    # Verify key tools
    local tools_found=0
    for tool in as ld ar ranlib nm objcopy strip; do
        local tool_path="${STAGING}${NATIVE_PREFIX}/bin/${tool}"
        local tool_path_prefixed="${STAGING}${NATIVE_PREFIX}/bin/${TARGET}-${tool}"
        if [[ -f "${tool_path}" ]] || [[ -f "${tool_path_prefixed}" ]]; then
            tools_found=$((tools_found + 1))
        fi
    done

    if [[ ${tools_found} -eq 0 ]]; then
        # Check in target subdirectory
        local target_bin="${STAGING}${NATIVE_PREFIX}/${TARGET}/bin"
        if [[ -d "${target_bin}" ]]; then
            tools_found="$(find "${target_bin}" -type f -executable 2>/dev/null | wc -l)"
        fi
    fi

    if [[ ${tools_found} -gt 0 ]]; then
        success "Static binutils installed (${tools_found} tools)"
    else
        warn "Binutils installed but tools not found at expected paths."
        warn "Listing staging bin/:"
        ls -la "${STAGING}${NATIVE_PREFIX}/bin/" 2>/dev/null || true
    fi

    # Verify static linking
    local sample_bin
    sample_bin="$(find "${STAGING}${NATIVE_PREFIX}" -name 'as' -o -name "${TARGET}-as" 2>/dev/null | head -1)"
    if [[ -n "${sample_bin}" ]] && command -v file &>/dev/null; then
        local file_type
        file_type="$(file "${sample_bin}")"
        info "Sample: ${file_type}"
        if echo "${file_type}" | grep -q "statically linked"; then
            success "Binaries are statically linked"
        fi
    fi
}

# ---------------------------------------------------------------------------
# Phase 3: Build static libgcc
# ---------------------------------------------------------------------------
phase3_build_static_libgcc() {
    step "Phase 3: Building static libgcc"

    local builddir="${BUILD_BASE}/build-gcc"
    local srcdir="${BUILD_BASE}/sources/gcc-${GCC_VERSION}"

    mkdir -p "${builddir}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring GCC (for libgcc + gcc driver + cc1) ..."
        info "  build  = ${BUILD_TRIPLE}"
        info "  host   = ${TARGET}"
        info "  target = ${TARGET}"
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --build="${BUILD_TRIPLE}" \
                --host="${TARGET}" \
                --target="${TARGET}" \
                --prefix="${NATIVE_PREFIX}" \
                --with-sysroot="/" \
                --with-native-system-header-dir="/usr/include" \
                --enable-languages=c \
                --disable-bootstrap \
                --disable-shared \
                --enable-static \
                --disable-threads \
                --disable-multilib \
                --disable-libssp \
                --disable-libquadmath \
                --disable-libgomp \
                --disable-libatomic \
                --disable-libstdcxx \
                --disable-nls \
                --disable-plugin \
                --disable-libcc1 \
                --disable-decimal-float \
                --disable-libffi \
                --disable-libitm \
                --disable-libsanitizer \
                --disable-libvtv \
                --with-newlib \
                CC="${CROSS_CC}" \
                CXX="${CROSS_CXX}" \
                AR="${CROSS_AR}" \
                AS="${CROSS_AS}" \
                LD="${CROSS_LD}" \
                RANLIB="${CROSS_RANLIB}" \
                STRIP="${CROSS_STRIP}" \
                CC_FOR_BUILD="gcc" \
                CXX_FOR_BUILD="g++" \
                AR_FOR_TARGET="${CROSS_AR}" \
                AS_FOR_TARGET="${CROSS_AS}" \
                LD_FOR_TARGET="${CROSS_LD}" \
                RANLIB_FOR_TARGET="${CROSS_RANLIB}" \
                NM_FOR_TARGET="${CROSS_PREFIX}/bin/${TARGET}-nm" \
                OBJCOPY_FOR_TARGET="${CROSS_PREFIX}/bin/${TARGET}-objcopy" \
                OBJDUMP_FOR_TARGET="${CROSS_PREFIX}/bin/${TARGET}-objdump" \
                STRIP_FOR_TARGET="${CROSS_STRIP}" \
                LDFLAGS="-static" \
                CFLAGS="-O2 -static" \
                CXXFLAGS="-O2 -static -fno-exceptions -fno-rtti" \
                CFLAGS_FOR_BUILD="-O2" \
                CXXFLAGS_FOR_BUILD="-O2" \
                2>&1 | tee "${LOG_DIR}/gcc-configure.log"
        )
    fi

    info "Building libgcc (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}" all-target-libgcc \
        2>&1 | tee "${LOG_DIR}/libgcc-build.log"

    info "Installing libgcc to staging ..."
    make -C "${builddir}" install-target-libgcc DESTDIR="${STAGING}" \
        2>&1 | tee "${LOG_DIR}/libgcc-install.log"

    # Verify libgcc.a
    local libgcc_path
    libgcc_path="$(find "${STAGING}" -name 'libgcc.a' 2>/dev/null | head -1)"
    if [[ -n "${libgcc_path}" ]]; then
        success "libgcc.a installed at ${libgcc_path}"
    else
        warn "libgcc.a not found in staging.  Build may have failed."
    fi
}

# ---------------------------------------------------------------------------
# Phase 4: Build static GCC (cc1 + gcc driver)
# ---------------------------------------------------------------------------
phase4_build_static_gcc() {
    step "Phase 4: Building static GCC (cc1 + gcc driver)"

    local builddir="${BUILD_BASE}/build-gcc"

    # GCC was already configured in Phase 3.  Now build the compiler itself.
    info "Building GCC compiler (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}" all-gcc \
        2>&1 | tee "${LOG_DIR}/gcc-build.log"

    info "Installing GCC to staging ..."
    make -C "${builddir}" install-gcc DESTDIR="${STAGING}" \
        2>&1 | tee "${LOG_DIR}/gcc-install.log"

    # Verify gcc binary
    local gcc_bin="${STAGING}${NATIVE_PREFIX}/bin/gcc"
    local gcc_prefixed="${STAGING}${NATIVE_PREFIX}/bin/${TARGET}-gcc"
    if [[ -f "${gcc_bin}" ]] || [[ -f "${gcc_prefixed}" ]]; then
        success "GCC driver installed"
    else
        warn "GCC driver not found at expected paths."
        warn "Listing staging bin/:"
        ls -la "${STAGING}${NATIVE_PREFIX}/bin/" 2>/dev/null || true
    fi

    # Verify cc1
    local cc1_bin
    cc1_bin="$(find "${STAGING}" -name 'cc1' 2>/dev/null | head -1)"
    if [[ -n "${cc1_bin}" ]]; then
        success "cc1 installed at ${cc1_bin}"
        if command -v file &>/dev/null; then
            local file_type
            file_type="$(file "${cc1_bin}")"
            info "cc1: ${file_type}"
            if echo "${file_type}" | grep -q "statically linked"; then
                success "cc1 is statically linked"
            fi
        fi
    else
        warn "cc1 not found in staging."
    fi
}

# ---------------------------------------------------------------------------
# Phase 5: Package into rootfs directory structure
# ---------------------------------------------------------------------------
phase5_package() {
    step "Phase 5: Packaging into rootfs directory structure"

    local rootfs_dir="${OUTPUT_DIR}"
    rm -rf "${rootfs_dir}"
    mkdir -p "${rootfs_dir}/usr/bin"
    mkdir -p "${rootfs_dir}/usr/lib"
    mkdir -p "${rootfs_dir}/usr/libexec"
    mkdir -p "${rootfs_dir}/usr/include"

    # 5a. Copy binutils tools
    info "Copying binutils tools ..."
    local binutils_tools=(as ld ar ranlib nm objcopy strip objdump readelf size strings)
    for tool in "${binutils_tools[@]}"; do
        local src
        # Check unprefixed first, then prefixed
        src="${STAGING}${NATIVE_PREFIX}/bin/${tool}"
        if [[ ! -f "${src}" ]]; then
            src="${STAGING}${NATIVE_PREFIX}/bin/${TARGET}-${tool}"
        fi
        if [[ ! -f "${src}" ]]; then
            # Check target-specific bin directory
            src="${STAGING}${NATIVE_PREFIX}/${TARGET}/bin/${tool}"
        fi
        if [[ -f "${src}" ]]; then
            cp "${src}" "${rootfs_dir}/usr/bin/${tool}"
            info "  + ${tool}"
        else
            warn "  - ${tool} (not found)"
        fi
    done

    # 5b. Copy GCC driver and cc1
    info "Copying GCC compiler ..."
    local gcc_bin="${STAGING}${NATIVE_PREFIX}/bin/gcc"
    if [[ ! -f "${gcc_bin}" ]]; then
        gcc_bin="${STAGING}${NATIVE_PREFIX}/bin/${TARGET}-gcc"
    fi
    if [[ -f "${gcc_bin}" ]]; then
        cp "${gcc_bin}" "${rootfs_dir}/usr/bin/gcc"
        # Create cc symlink
        ln -sf gcc "${rootfs_dir}/usr/bin/cc"
        info "  + gcc (+ cc symlink)"
    else
        warn "  - gcc (not found)"
    fi

    # cc1 lives under libexec/gcc/<target>/<version>/
    local cc1_src
    cc1_src="$(find "${STAGING}" -name 'cc1' 2>/dev/null | head -1)"
    if [[ -n "${cc1_src}" ]]; then
        local cc1_rel_dir="gcc/${TARGET}/${GCC_VERSION}"
        mkdir -p "${rootfs_dir}/usr/libexec/${cc1_rel_dir}"
        cp "${cc1_src}" "${rootfs_dir}/usr/libexec/${cc1_rel_dir}/cc1"
        info "  + cc1"

        # Also copy collect2 and other GCC internal tools if present
        local cc1_dir
        cc1_dir="$(dirname "${cc1_src}")"
        for internal_tool in collect2 lto1 lto-wrapper; do
            if [[ -f "${cc1_dir}/${internal_tool}" ]]; then
                cp "${cc1_dir}/${internal_tool}" "${rootfs_dir}/usr/libexec/${cc1_rel_dir}/"
                info "  + ${internal_tool}"
            fi
        done
    else
        warn "  - cc1 (not found)"
    fi

    # 5c. Copy libraries
    info "Copying libraries ..."

    # libgcc.a
    local libgcc_src
    libgcc_src="$(find "${STAGING}" -name 'libgcc.a' 2>/dev/null | head -1)"
    if [[ -n "${libgcc_src}" ]]; then
        local gcc_lib_dir="${rootfs_dir}/usr/lib/gcc/${TARGET}/${GCC_VERSION}"
        mkdir -p "${gcc_lib_dir}"
        cp "${libgcc_src}" "${gcc_lib_dir}/libgcc.a"
        info "  + libgcc.a"

        # Copy crtbegin.o and crtend.o if present
        local libgcc_dir
        libgcc_dir="$(dirname "${libgcc_src}")"
        for crt in crtbegin.o crtend.o; do
            if [[ -f "${libgcc_dir}/${crt}" ]]; then
                cp "${libgcc_dir}/${crt}" "${gcc_lib_dir}/"
                info "  + ${crt}"
            fi
        done
    fi

    # Copy sysroot CRT objects and libc
    for f in crt0.o crti.o crtn.o libc.a; do
        if [[ -f "${SYSROOT}/usr/lib/${f}" ]]; then
            cp "${SYSROOT}/usr/lib/${f}" "${rootfs_dir}/usr/lib/"
            info "  + ${f}"
        fi
    done

    # 5d. Copy headers
    info "Copying headers ..."
    if [[ -d "${SYSROOT}/usr/include" ]]; then
        cp -r "${SYSROOT}/usr/include/"* "${rootfs_dir}/usr/include/" 2>/dev/null || true
        local header_count
        header_count="$(find "${rootfs_dir}/usr/include" -name '*.h' 2>/dev/null | wc -l)"
        info "  + ${header_count} headers"
    fi

    # Copy GCC-internal headers (stdarg.h, stddef.h, etc.)
    local gcc_include_dir
    gcc_include_dir="$(find "${STAGING}" -path "*/lib/gcc/${TARGET}/${GCC_VERSION}/include" -type d 2>/dev/null | head -1)"
    if [[ -n "${gcc_include_dir}" ]]; then
        local dest_gcc_inc="${rootfs_dir}/usr/lib/gcc/${TARGET}/${GCC_VERSION}/include"
        mkdir -p "${dest_gcc_inc}"
        cp -r "${gcc_include_dir}/"* "${dest_gcc_inc}/" 2>/dev/null || true
        local gcc_header_count
        gcc_header_count="$(find "${dest_gcc_inc}" -name '*.h' 2>/dev/null | wc -l)"
        info "  + ${gcc_header_count} GCC internal headers (stdarg.h, stddef.h, etc.)"
    fi

    # 5e. Strip binaries for size reduction
    info "Stripping binaries for size reduction ..."
    local total_before=0
    local total_after=0
    for bin_file in "${rootfs_dir}/usr/bin/"* "${rootfs_dir}/usr/libexec/gcc/${TARGET}/${GCC_VERSION}/"*; do
        [[ -f "${bin_file}" ]] || continue
        local before
        before="$(stat -c%s "${bin_file}" 2>/dev/null || stat -f%z "${bin_file}" 2>/dev/null || echo 0)"
        total_before=$((total_before + before))
        "${CROSS_STRIP}" "${bin_file}" 2>/dev/null || true
        local after
        after="$(stat -c%s "${bin_file}" 2>/dev/null || stat -f%z "${bin_file}" 2>/dev/null || echo 0)"
        total_after=$((total_after + after))
    done
    if [[ ${total_before} -gt 0 ]]; then
        local saved=$((total_before - total_after))
        info "Stripped: ${total_before} -> ${total_after} bytes (saved $((saved / 1024)) KB)"
    fi

    success "Rootfs directory packaged at ${rootfs_dir}"
}

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "Static Native GCC Build Complete"

    echo ""
    printf "${GREEN}Build type:${NC}          Canadian cross (static)\n"
    printf "${GREEN}Build machine:${NC}       %s\n" "${BUILD_TRIPLE}"
    printf "${GREEN}Host (runs on):${NC}      %s\n" "${TARGET}"
    printf "${GREEN}Target (compiles for):${NC} %s\n" "${TARGET}"
    printf "${GREEN}Output directory:${NC}    %s\n" "${OUTPUT_DIR}"
    echo ""

    echo "Installed binaries:"
    if [[ -d "${OUTPUT_DIR}/usr/bin" ]]; then
        for f in "${OUTPUT_DIR}/usr/bin/"*; do
            [[ -f "${f}" ]] || continue
            local name
            name="$(basename "${f}")"
            local size
            size="$(stat -c%s "${f}" 2>/dev/null || stat -f%z "${f}" 2>/dev/null || echo "?")"
            printf "  %-16s %s KB\n" "${name}" "$((size / 1024))"
        done
    fi
    echo ""

    echo "Installed libraries:"
    for f in "${OUTPUT_DIR}/usr/lib/"*.a "${OUTPUT_DIR}/usr/lib/"*.o "${OUTPUT_DIR}/usr/lib/gcc/${TARGET}/${GCC_VERSION}/"*.a "${OUTPUT_DIR}/usr/lib/gcc/${TARGET}/${GCC_VERSION}/"*.o; do
        [[ -f "${f}" ]] || continue
        local name
        name="$(basename "${f}")"
        local size
        size="$(stat -c%s "${f}" 2>/dev/null || stat -f%z "${f}" 2>/dev/null || echo "?")"
        printf "  %-16s %s bytes\n" "${name}" "${size}"
    done
    echo ""

    echo "To package into a VeridianOS rootfs TAR:"
    echo "  ./scripts/package-native-toolchain.sh"
    echo ""
    echo "To merge with the existing rootfs:"
    echo "  cd ${OUTPUT_DIR} && tar cf native-toolchain.tar ."
    echo "  # Then use as an additional virtio-blk drive in QEMU"
    echo ""

    printf "${YELLOW}NOTE:${NC} The native binaries are statically linked ELF executables\n"
    echo "for VeridianOS.  They require a functional VeridianOS user-space"
    echo "environment with multi-LOAD ELF support and sufficient memory."
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Static Native GCC Builder${NC}\n"
    printf "  Architecture:       %s\n" "${ARCH}"
    printf "  Cross-prefix:       %s\n" "${CROSS_PREFIX}"
    printf "  Parallel jobs:      %s\n" "${JOBS}"
    printf "  Build dir:          %s\n" "${BUILD_BASE}"
    printf "  Clean build:        %s\n" "${CLEAN}"
    echo ""

    setup_vars

    if [[ "${CLEAN}" == "yes" ]]; then
        info "Cleaning build directory: ${BUILD_BASE}"
        rm -rf "${BUILD_BASE}"
        info "Cleaning output directory: ${OUTPUT_DIR}"
        rm -rf "${OUTPUT_DIR}"
    fi

    phase1_verify_cross_compiler
    prepare_sources
    apply_patches
    phase2_build_static_binutils
    phase3_build_static_libgcc
    phase4_build_static_gcc
    phase5_package
    print_summary
}

main "$@"

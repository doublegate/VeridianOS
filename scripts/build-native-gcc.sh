#!/usr/bin/env bash
# VeridianOS Native GCC Builder (Canadian Cross-Compilation)
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds a native GCC toolchain that runs ON VeridianOS (host=veridian,
# target=veridian) using Canadian cross-compilation from a Linux build
# machine.
#
# The build proceeds through these stages:
#
#   1. Prerequisite check: verify cross-toolchain exists and works
#   2. Stage 2.5: rebuild cross-GCC with C++ support (needed because
#      GCC 14.2 is written in C++ and the Canadian cross requires
#      x86_64-veridian-g++ as the host C++ compiler)
#   3. Native binutils: Canadian cross-compile binutils
#      (build=linux, host=veridian, target=veridian)
#   4. Native GCC: Canadian cross-compile GCC (C only)
#      (build=linux, host=veridian, target=veridian)
#   5. Package: install into staging directory and create rootfs TAR
#
# Usage:
#   ./scripts/build-native-gcc.sh [OPTIONS]
#
#   --arch ARCH           Target architecture: x86_64 (default)
#   --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
#   --jobs N              Parallel make jobs (default: nproc)
#   --skip-stage25        Skip Stage 2.5 if x86_64-veridian-g++ already exists
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
NC='\033[0m' # No Color

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
# Resolve project root (works from any working directory)
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
ARCH="x86_64"
CROSS_PREFIX="/opt/veridian/toolchain"
JOBS="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
SKIP_STAGE25="no"

# ---------------------------------------------------------------------------
# Version and checksum table (must match build-cross-toolchain.sh)
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
BUILD_BASE="/tmp/veridian-native-gcc-build"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --arch)
                ARCH="${2:?--arch requires a value (x86_64)}"
                shift 2
                ;;
            --cross-prefix)
                CROSS_PREFIX="${2:?--cross-prefix requires a path}"
                shift 2
                ;;
            --jobs)
                JOBS="${2:?--jobs requires a number}"
                shift 2
                ;;
            --skip-stage25)
                SKIP_STAGE25="yes"
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

    case "${ARCH}" in
        x86_64) ;;
        aarch64|riscv64)
            warn "Architecture ${ARCH} is experimental for native builds."
            warn "The Canadian cross may work but has not been verified."
            ;;
        *)
            die "Unsupported architecture: ${ARCH}.  Currently only x86_64 is supported."
            ;;
    esac
}

usage() {
    cat <<'EOF'
Usage: ./scripts/build-native-gcc.sh [OPTIONS]

Build a native GCC toolchain for VeridianOS using Canadian cross-compilation.
The resulting binaries run ON VeridianOS and compile FOR VeridianOS.

Stages:
  1. Prerequisite check  -- verify cross-toolchain exists
  2. Stage 2.5           -- rebuild cross-GCC with C++ support
  3. Native binutils     -- Canadian cross-compile binutils
  4. Native GCC          -- Canadian cross-compile GCC (C only)
  5. Package             -- create rootfs TAR with native toolchain

Options:
  --arch ARCH           Target architecture: x86_64 (default)
  --cross-prefix PATH   Cross-toolchain prefix (default: /opt/veridian/toolchain)
  --jobs N              Parallel make jobs (default: nproc)
  --skip-stage25        Skip Stage 2.5 if x86_64-veridian-g++ already exists
  -h, --help            Show this help message

Prerequisites:
  - Cross-toolchain built via scripts/build-cross-toolchain.sh
  - Sysroot populated via scripts/build-sysroot.sh (called by cross-toolchain)
  - Host build tools: gcc, g++, make, tar, wget, patch, bison, flex, texinfo

Example:
  # Full build (including Stage 2.5 C++ cross-compiler)
  ./scripts/build-native-gcc.sh

  # Skip Stage 2.5 if you already have x86_64-veridian-g++
  ./scripts/build-native-gcc.sh --skip-stage25

  # Use a custom cross-toolchain location
  ./scripts/build-native-gcc.sh --cross-prefix ~/veridian-tc
EOF
}

# ---------------------------------------------------------------------------
# Derived variables (set after argument parsing)
# ---------------------------------------------------------------------------
setup_vars() {
    TARGET="${ARCH}-veridian"

    # Determine the build system triple (linux host)
    BUILD_TRIPLE="$(gcc -dumpmachine 2>/dev/null || echo "x86_64-pc-linux-gnu")"

    # Cross-toolchain tools (these compile FOR veridian, running ON linux)
    CROSS_CC="${CROSS_PREFIX}/bin/${TARGET}-gcc"
    CROSS_CXX="${CROSS_PREFIX}/bin/${TARGET}-g++"
    CROSS_AR="${CROSS_PREFIX}/bin/${TARGET}-ar"
    CROSS_AS="${CROSS_PREFIX}/bin/${TARGET}-as"
    CROSS_LD="${CROSS_PREFIX}/bin/${TARGET}-ld"
    CROSS_RANLIB="${CROSS_PREFIX}/bin/${TARGET}-ranlib"
    CROSS_STRIP="${CROSS_PREFIX}/bin/${TARGET}-strip"

    SYSROOT="${CROSS_PREFIX}/sysroot"

    # Native toolchain staging directory
    STAGING="${BUILD_BASE}/staging"
    NATIVE_PREFIX="/usr"

    # Output archive
    OUTPUT_TAR="${PROJECT_ROOT}/target/native-toolchain.tar"
}

# ---------------------------------------------------------------------------
# Target triple helper
# ---------------------------------------------------------------------------
target_triple() {
    echo "${TARGET}"
}

# ---------------------------------------------------------------------------
# Step 1: Check prerequisites
# ---------------------------------------------------------------------------
check_prerequisites() {
    step "Checking prerequisites"

    # 1a. Host build tools
    local missing=()

    for cmd in gcc g++ make tar wget sha256sum patch bison flex \
               makeinfo xz gzip; do
        if ! command -v "${cmd}" &>/dev/null; then
            missing+=("${cmd}")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        error "Missing host tools: ${missing[*]}"
        echo ""
        echo "Install them with your system package manager.  For example:"
        echo ""
        echo "  # Arch / CachyOS"
        echo "  sudo pacman -S base-devel texinfo wget"
        echo ""
        echo "  # Debian / Ubuntu"
        echo "  sudo apt install build-essential texinfo bison flex wget"
        echo ""
        die "Please install the missing prerequisites and try again."
    fi

    success "Host build tools found"

    # 1b. Cross-toolchain
    if [[ ! -x "${CROSS_CC}" ]]; then
        die "Cross-compiler not found at ${CROSS_CC}.  Run scripts/build-cross-toolchain.sh first."
    fi

    if [[ ! -x "${CROSS_AS}" ]]; then
        die "Cross-assembler not found at ${CROSS_AS}.  Run scripts/build-cross-toolchain.sh first."
    fi

    if [[ ! -x "${CROSS_LD}" ]]; then
        die "Cross-linker not found at ${CROSS_LD}.  Run scripts/build-cross-toolchain.sh first."
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

    # 1d. Quick sanity test: can the cross-compiler produce an object file?
    local test_c="${BUILD_BASE}/test_cross.c"
    local test_o="${BUILD_BASE}/test_cross.o"
    mkdir -p "${BUILD_BASE}"
    echo 'int veridian_test(void) { return 42; }' > "${test_c}"
    if "${CROSS_CC}" -c -o "${test_o}" "${test_c}" -ffreestanding -nostdlib 2>/dev/null; then
        rm -f "${test_c}" "${test_o}"
        success "Cross-compiler produces valid object files"
    else
        rm -f "${test_c}" "${test_o}"
        die "Cross-compiler failed to compile a test file.  The toolchain may be broken."
    fi
}

# ---------------------------------------------------------------------------
# Step 2: Create build directories
# ---------------------------------------------------------------------------
create_directories() {
    step "Creating build directories"

    mkdir -p "${BUILD_BASE}/downloads"
    mkdir -p "${BUILD_BASE}/sources"
    mkdir -p "${BUILD_BASE}/build-stage25"
    mkdir -p "${BUILD_BASE}/build-native-binutils"
    mkdir -p "${BUILD_BASE}/build-native-gcc"
    mkdir -p "${STAGING}"

    info "Build base: ${BUILD_BASE}"
    success "Directories created"
}

# ---------------------------------------------------------------------------
# Step 3: Download source tarballs
# ---------------------------------------------------------------------------
download_sources() {
    step "Downloading source tarballs"

    local cross_dl="/tmp/veridian-toolchain-build/downloads"
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
        elif [[ -f "${cross_dl}/${filename}" ]]; then
            # Reuse downloads from the cross-toolchain build
            info "Reusing from cross-toolchain build: ${filename}"
            ln -sf "${cross_dl}/${filename}" "${dest}"
        else
            info "Downloading ${filename} ..."
            wget -q --show-progress -O "${dest}" "${url}" \
                || die "Failed to download ${url}"
        fi
    done

    success "All source tarballs available"
}

# ---------------------------------------------------------------------------
# Step 4: Verify SHA-256 checksums
# ---------------------------------------------------------------------------
verify_checksums() {
    step "Verifying SHA-256 checksums"

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
        # Resolve symlinks for sha256sum
        got="$(sha256sum "$(readlink -f "${file}")" | awk '{print $1}')"

        if [[ "${got}" != "${want}" ]]; then
            error "Checksum mismatch for ${filename}!"
            error "  Expected: ${want}"
            error "  Got:      ${got}"
            die "Aborting.  Remove ${file} and re-download."
        fi
        info "OK: ${filename}"
    done

    success "All checksums verified"
}

# ---------------------------------------------------------------------------
# Step 5: Extract tarballs
# ---------------------------------------------------------------------------
extract_sources() {
    step "Extracting source tarballs"

    local srcdir="${BUILD_BASE}/sources"

    for archive in "${BUILD_BASE}/downloads"/*.tar.*; do
        # Resolve symlinks so tar can read the file
        local real_archive
        real_archive="$(readlink -f "${archive}")"
        local name
        name="$(basename "${archive}")"
        # Derive expected directory name (strip .tar.*)
        local dirname="${name%.tar.*}"

        if [[ -d "${srcdir}/${dirname}" ]]; then
            info "Already extracted: ${dirname}"
        else
            info "Extracting ${name} ..."
            tar -xf "${real_archive}" -C "${srcdir}"
        fi
    done

    success "All sources extracted"
}

# ---------------------------------------------------------------------------
# Step 6: Apply patches
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
    else
        warn "No binutils patches directory found at ${patches_dir}"
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
    else
        warn "No GCC patches directory found at ${patches_dir}"
    fi

    success "Patches applied"
}

# ---------------------------------------------------------------------------
# Step 7: Symlink GMP/MPFR/MPC into GCC source tree
# ---------------------------------------------------------------------------
symlink_gcc_deps() {
    step "Symlinking GMP, MPFR, MPC into GCC source tree"

    local gcc_src="${BUILD_BASE}/sources/gcc-${GCC_VERSION}"

    ln -sfn "${BUILD_BASE}/sources/gmp-${GMP_VERSION}"  "${gcc_src}/gmp"
    ln -sfn "${BUILD_BASE}/sources/mpfr-${MPFR_VERSION}" "${gcc_src}/mpfr"
    ln -sfn "${BUILD_BASE}/sources/mpc-${MPC_VERSION}"   "${gcc_src}/mpc"

    success "Symlinks created"
}

# ---------------------------------------------------------------------------
# Step 8: Build Stage 2.5 -- Cross-GCC with C++ support
#
# GCC 14.2 is written in C++.  For the Canadian cross (native GCC build
# where host=veridian), we need x86_64-veridian-g++ to compile GCC's C++
# source code into veridian-hosted binaries.
#
# Stage 2 (from build-cross-toolchain.sh) only builds --enable-languages=c.
# Stage 2.5 rebuilds GCC with --enable-languages=c,c++ but DISABLES the
# hosted libstdc++ (which needs a working OS) -- we only need enough C++
# compiler support to build GCC itself, not a full standard library.
# ---------------------------------------------------------------------------
build_stage25() {
    step "Building Stage 2.5: Cross-GCC with C++ support"

    if [[ "${SKIP_STAGE25}" == "yes" ]]; then
        if [[ -x "${CROSS_CXX}" ]]; then
            info "Skipping Stage 2.5 (--skip-stage25 set, ${CROSS_CXX} exists)"
            success "Stage 2.5 skipped"
            return 0
        else
            warn "--skip-stage25 set but ${CROSS_CXX} not found."
            warn "Building Stage 2.5 anyway."
        fi
    fi

    if [[ -x "${CROSS_CXX}" ]]; then
        # Verify it actually works
        local test_cpp="${BUILD_BASE}/test_cxx.cpp"
        local test_o="${BUILD_BASE}/test_cxx.o"
        echo 'extern "C" int test() { return 1; }' > "${test_cpp}"
        if "${CROSS_CXX}" -c -o "${test_o}" "${test_cpp}" -ffreestanding -nostdlib -fno-exceptions -fno-rtti 2>/dev/null; then
            rm -f "${test_cpp}" "${test_o}"
            info "Existing ${CROSS_CXX} appears functional, skipping rebuild."
            success "Stage 2.5 already available"
            return 0
        fi
        rm -f "${test_cpp}" "${test_o}"
        warn "Existing ${CROSS_CXX} failed sanity check, rebuilding."
    fi

    local builddir="${BUILD_BASE}/build-stage25"
    local srcdir="${BUILD_BASE}/sources/gcc-${GCC_VERSION}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring Stage 2.5 (C + C++ cross-compiler) ..."
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --target="$(target_triple)" \
                --prefix="${CROSS_PREFIX}" \
                --with-sysroot="${SYSROOT}" \
                --enable-languages=c,c++ \
                --disable-shared \
                --disable-threads \
                --disable-multilib \
                --disable-libssp \
                --disable-libquadmath \
                --disable-libgomp \
                --disable-libatomic \
                --disable-hosted-libstdcxx \
                --disable-nls \
                --with-newlib
        )
    fi

    info "Compiling Stage 2.5 (${JOBS} jobs) ..."
    info "This rebuilds GCC with C++ cross-compilation support."
    make -C "${builddir}" -j"${JOBS}" all-gcc all-target-libgcc

    info "Installing Stage 2.5 ..."
    make -C "${builddir}" install-gcc install-target-libgcc

    # Verify the C++ compiler was produced
    if [[ ! -x "${CROSS_CXX}" ]]; then
        die "Stage 2.5 build completed but ${CROSS_CXX} not found.  Build may have failed silently."
    fi

    # Sanity test
    local test_cpp="${BUILD_BASE}/test_cxx.cpp"
    local test_o="${BUILD_BASE}/test_cxx.o"
    echo 'extern "C" int test() { return 1; }' > "${test_cpp}"
    if "${CROSS_CXX}" -c -o "${test_o}" "${test_cpp}" -ffreestanding -nostdlib -fno-exceptions -fno-rtti 2>/dev/null; then
        rm -f "${test_cpp}" "${test_o}"
        success "Stage 2.5 installed -- ${CROSS_CXX} is functional"
    else
        rm -f "${test_cpp}" "${test_o}"
        die "Stage 2.5 ${CROSS_CXX} failed to compile a test file."
    fi
}

# ---------------------------------------------------------------------------
# Step 9: Canadian cross-compile native binutils
#
#   build  = x86_64-pc-linux-gnu  (runs on the build machine)
#   host   = x86_64-veridian      (the resulting binary runs on VeridianOS)
#   target = x86_64-veridian      (the tools process veridian ELF files)
#
# We use LDFLAGS="-static" because VeridianOS has no dynamic linker.
# ---------------------------------------------------------------------------
build_native_binutils() {
    step "Building native binutils (Canadian cross: host=veridian, target=veridian)"

    local builddir="${BUILD_BASE}/build-native-binutils"
    local srcdir="${BUILD_BASE}/sources/binutils-${BINUTILS_VERSION}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring native binutils ..."
        info "  build  = ${BUILD_TRIPLE}"
        info "  host   = $(target_triple)"
        info "  target = $(target_triple)"
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --build="${BUILD_TRIPLE}" \
                --host="$(target_triple)" \
                --target="$(target_triple)" \
                --prefix="${NATIVE_PREFIX}" \
                --with-sysroot="/" \
                --disable-nls \
                --disable-werror \
                --disable-gdb \
                --disable-gdbserver \
                --disable-sim \
                --disable-readline \
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
                CXXFLAGS="-O2 -static"
        )
    fi

    info "Compiling native binutils (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}"

    info "Installing native binutils to staging area ..."
    make -C "${builddir}" install DESTDIR="${STAGING}"

    # Verify key tools were produced
    local as_bin="${STAGING}${NATIVE_PREFIX}/bin/as"
    local ld_bin="${STAGING}${NATIVE_PREFIX}/bin/ld"
    if [[ -f "${as_bin}" ]] || [[ -f "${STAGING}${NATIVE_PREFIX}/${TARGET}/bin/as" ]]; then
        success "Native binutils installed to ${STAGING}${NATIVE_PREFIX}"
    else
        # binutils may install as ARCH-veridian-as when target matches host
        local alt_as="${STAGING}${NATIVE_PREFIX}/bin/${TARGET}-as"
        if [[ -f "${alt_as}" ]]; then
            success "Native binutils installed to ${STAGING}${NATIVE_PREFIX} (prefixed)"
        else
            warn "Native binutils installed but key tools not found at expected paths."
            warn "Listing ${STAGING}${NATIVE_PREFIX}/bin/:"
            ls -la "${STAGING}${NATIVE_PREFIX}/bin/" 2>/dev/null || true
        fi
    fi

    # Verify the binaries are veridian ELF, not linux ELF
    local sample_bin
    sample_bin="$(find "${STAGING}${NATIVE_PREFIX}" -name 'as' -o -name "${TARGET}-as" 2>/dev/null | head -1)"
    if [[ -n "${sample_bin}" ]] && command -v file &>/dev/null; then
        local file_type
        file_type="$(file "${sample_bin}")"
        info "Sample binary: ${file_type}"
        if echo "${file_type}" | grep -q "statically linked"; then
            success "Binary is statically linked (correct for VeridianOS)"
        fi
    fi
}

# ---------------------------------------------------------------------------
# Step 10: Canadian cross-compile native GCC
#
#   build  = x86_64-pc-linux-gnu  (runs on the build machine)
#   host   = x86_64-veridian      (the resulting gcc runs on VeridianOS)
#   target = x86_64-veridian      (compiles code for VeridianOS)
#
# Key considerations:
#   - --disable-bootstrap: we cannot run veridian binaries on linux
#   - --enable-languages=c: C only for now (C++ requires libstdc++)
#   - --disable-libstdcxx: no standard C++ library on veridian yet
#   - GMP/MPFR/MPC built in-tree (symlinked into GCC source)
#   - LDFLAGS="-static": VeridianOS has no dynamic linker
# ---------------------------------------------------------------------------
build_native_gcc() {
    step "Building native GCC (Canadian cross: host=veridian, target=veridian)"

    local builddir="${BUILD_BASE}/build-native-gcc"
    local srcdir="${BUILD_BASE}/sources/gcc-${GCC_VERSION}"

    export PATH="${CROSS_PREFIX}/bin:${PATH}"

    # The native binutils must be findable for the GCC build.  When GCC
    # configures itself, it looks for ${target}-as, ${target}-ld, etc.
    # The cross-toolchain already has these, so they should be on PATH.

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring native GCC ..."
        info "  build  = ${BUILD_TRIPLE}"
        info "  host   = $(target_triple)"
        info "  target = $(target_triple)"
        (
            cd "${builddir}"

            # Configure the Canadian cross-compilation.
            #
            # CC_FOR_BUILD / CXX_FOR_BUILD: host (linux) compilers used to
            #   build the build-time tools (like gen* programs that run
            #   during the GCC build process on the build machine).
            #
            # CC / CXX: cross-compilers that produce veridian binaries.
            #   These compile the actual gcc/cc1/collect2 executables that
            #   will run on VeridianOS.
            #
            # AR_FOR_TARGET / AS_FOR_TARGET / etc.: tools for the target
            #   system.  Since host==target, these are the cross-tools.
            "${srcdir}/configure" \
                --build="${BUILD_TRIPLE}" \
                --host="$(target_triple)" \
                --target="$(target_triple)" \
                --prefix="${NATIVE_PREFIX}" \
                --with-sysroot="/" \
                --with-native-system-header-dir="/usr/include" \
                --enable-languages=c \
                --disable-bootstrap \
                --disable-shared \
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
                CXXFLAGS_FOR_BUILD="-O2"
        )
    fi

    info "Compiling native GCC (${JOBS} jobs) ..."
    info "Building all-gcc and all-target-libgcc ..."
    make -C "${builddir}" -j"${JOBS}" all-gcc all-target-libgcc

    info "Installing native GCC to staging area ..."
    make -C "${builddir}" install-gcc install-target-libgcc DESTDIR="${STAGING}"

    # Verify the gcc binary was produced
    local gcc_bin="${STAGING}${NATIVE_PREFIX}/bin/gcc"
    local gcc_prefixed="${STAGING}${NATIVE_PREFIX}/bin/${TARGET}-gcc"
    if [[ -f "${gcc_bin}" ]] || [[ -f "${gcc_prefixed}" ]]; then
        success "Native GCC installed to ${STAGING}${NATIVE_PREFIX}"
    else
        warn "Native GCC installed but binary not found at expected paths."
        warn "Listing ${STAGING}${NATIVE_PREFIX}/bin/:"
        ls -la "${STAGING}${NATIVE_PREFIX}/bin/" 2>/dev/null || true
    fi

    # Verify the binary is veridian ELF, not linux ELF
    local sample_bin
    sample_bin="$(find "${STAGING}${NATIVE_PREFIX}" -name 'gcc' -o -name "${TARGET}-gcc" -o -name 'cc1' 2>/dev/null | head -1)"
    if [[ -n "${sample_bin}" ]] && command -v file &>/dev/null; then
        local file_type
        file_type="$(file "${sample_bin}")"
        info "Sample binary: ${file_type}"
        if echo "${file_type}" | grep -q "statically linked"; then
            success "Binary is statically linked (correct for VeridianOS)"
        fi
    fi
}

# ---------------------------------------------------------------------------
# Step 11: Install sysroot into staging
#
# The native GCC needs headers and libraries in its sysroot.  Since we
# set --with-sysroot="/" for the native compiler, it will look for
# headers at /usr/include and libraries at /usr/lib on the target system.
# We copy our sysroot contents into the staging area at those paths.
# ---------------------------------------------------------------------------
install_sysroot_to_staging() {
    step "Installing sysroot into staging directory"

    local staging_inc="${STAGING}/usr/include"
    local staging_lib="${STAGING}/usr/lib"

    mkdir -p "${staging_inc}"
    mkdir -p "${staging_lib}"

    # Copy headers
    if [[ -d "${SYSROOT}/usr/include" ]]; then
        cp -r "${SYSROOT}/usr/include/"* "${staging_inc}/" 2>/dev/null || true
        local header_count
        header_count="$(find "${staging_inc}" -name '*.h' 2>/dev/null | wc -l)"
        info "Installed ${header_count} headers"
    fi

    # Copy CRT objects and libc
    for f in crt0.o crti.o crtn.o libc.a; do
        if [[ -f "${SYSROOT}/usr/lib/${f}" ]]; then
            cp "${SYSROOT}/usr/lib/${f}" "${staging_lib}/"
            info "Installed ${f}"
        fi
    done

    # Copy libgcc from cross-toolchain
    local libgcc_dir
    libgcc_dir="$(find "${CROSS_PREFIX}/lib/gcc/${TARGET}" -name 'libgcc.a' -printf '%h\n' 2>/dev/null | head -1)"
    if [[ -n "${libgcc_dir}" ]]; then
        local gcc_lib_dest="${STAGING}/usr/lib/gcc/${TARGET}/${GCC_VERSION}"
        mkdir -p "${gcc_lib_dest}"
        cp "${libgcc_dir}/libgcc.a" "${gcc_lib_dest}/" 2>/dev/null || true
        # Also copy crtbegin.o and crtend.o if they exist
        for crt in crtbegin.o crtend.o; do
            if [[ -f "${libgcc_dir}/${crt}" ]]; then
                cp "${libgcc_dir}/${crt}" "${gcc_lib_dest}/"
            fi
        done
        info "Installed libgcc and CRT objects from cross-toolchain"
    fi

    success "Sysroot installed to staging"
}

# ---------------------------------------------------------------------------
# Step 12: Create output TAR
# ---------------------------------------------------------------------------
package_rootfs() {
    step "Packaging native toolchain into TAR archive"

    mkdir -p "$(dirname "${OUTPUT_TAR}")"

    # Create a TAR from the staging directory.
    # The archive contains paths like usr/bin/gcc, usr/lib/libc.a, etc.
    (
        cd "${STAGING}"
        tar cf "${OUTPUT_TAR}" .
    )

    local size
    size="$(stat -c%s "${OUTPUT_TAR}" 2>/dev/null || stat -f%z "${OUTPUT_TAR}" 2>/dev/null || echo "?")"

    info "Archive: ${OUTPUT_TAR}"
    info "Size: ${size} bytes"

    # Show a summary of what is in the archive
    echo ""
    info "Archive contents (top-level):"
    tar tf "${OUTPUT_TAR}" | head -50
    local total_files
    total_files="$(tar tf "${OUTPUT_TAR}" | wc -l)"
    if [[ "${total_files}" -gt 50 ]]; then
        info "  ... and $((total_files - 50)) more files"
    fi
    info "Total files: ${total_files}"

    success "Native toolchain packaged at ${OUTPUT_TAR}"
}

# ---------------------------------------------------------------------------
# Step 13: Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "Native GCC build complete!"

    echo ""
    printf "${GREEN}Build type:${NC}          Canadian cross-compilation\n"
    printf "${GREEN}Build machine:${NC}       %s\n" "${BUILD_TRIPLE}"
    printf "${GREEN}Host (runs on):${NC}      %s\n" "$(target_triple)"
    printf "${GREEN}Target (compiles for):${NC} %s\n" "$(target_triple)"
    printf "${GREEN}Staging directory:${NC}   %s\n" "${STAGING}"
    printf "${GREEN}Output archive:${NC}      %s\n" "${OUTPUT_TAR}"
    echo ""
    echo "The native toolchain is packaged at:"
    echo "  ${OUTPUT_TAR}"
    echo ""
    echo "This archive contains a GCC toolchain that runs ON VeridianOS."
    echo "To use it, extract into the VeridianOS root filesystem:"
    echo ""
    echo "  tar xf native-toolchain.tar -C /  (on VeridianOS)"
    echo ""
    echo "The native compiler will then be available as:"
    echo "  /usr/bin/${TARGET}-gcc    (or /usr/bin/gcc)"
    echo "  /usr/bin/${TARGET}-as"
    echo "  /usr/bin/${TARGET}-ld"
    echo ""
    echo "To include in a QEMU rootfs image, use the archive as an"
    echo "additional virtio-blk drive or merge it with the rootfs.tar"
    echo "from scripts/build-rootfs.sh."
    echo ""

    # Note about current limitations
    printf "${YELLOW}NOTE:${NC} The native binaries are statically linked ELF executables\n"
    echo "for VeridianOS.  They may not be runnable yet if the VeridianOS"
    echo "user-space execution environment has not been fully brought up"
    echo "(multi-LOAD ELF support, large binary memory mapping, etc.)."
    echo "This script establishes the BUILD PROCESS for the native toolchain."
    echo ""
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Native GCC Builder (Canadian Cross)${NC}\n"
    printf "  Architecture:       %s\n" "${ARCH}"
    printf "  Cross-prefix:       %s\n" "${CROSS_PREFIX}"
    printf "  Parallel jobs:      %s\n" "${JOBS}"
    printf "  Build dir:          %s\n" "${BUILD_BASE}"
    printf "  Skip Stage 2.5:     %s\n" "${SKIP_STAGE25}"
    echo ""

    setup_vars

    check_prerequisites
    create_directories
    download_sources
    verify_checksums
    extract_sources
    apply_patches
    symlink_gcc_deps
    build_stage25
    build_native_binutils
    build_native_gcc
    install_sysroot_to_staging
    package_rootfs
    print_summary
}

main "$@"

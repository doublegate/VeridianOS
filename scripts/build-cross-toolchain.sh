#!/usr/bin/env bash
# VeridianOS Cross-Compilation Toolchain Builder
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds a complete GCC cross-compiler toolchain for VeridianOS targeting
# x86_64, aarch64, or riscv64.  The toolchain consists of:
#
#   1. GNU Binutils (assembler, linker, object utilities)
#   2. GCC Stage 1 (C compiler, no libc headers)
#   3. Sysroot population (kernel headers, CRT objects, libc)
#   4. GCC Stage 2 (C compiler with sysroot and libgcc)
#
# Usage:
#   ./scripts/build-cross-toolchain.sh [--arch ARCH] [--prefix PREFIX] [--jobs N]
#
#   ARCH:   x86_64 (default), aarch64, riscv64
#   PREFIX: /opt/veridian/toolchain (default)
#   JOBS:   $(nproc) (default)

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
PREFIX="/opt/veridian/toolchain"
JOBS="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"

# ---------------------------------------------------------------------------
# Version and checksum table
# ---------------------------------------------------------------------------
BINUTILS_VERSION="2.47"
GCC_VERSION="16.2.0"
GMP_VERSION="6.3.0"
MPFR_VERSION="4.2.2"
MPC_VERSION="1.4.1"

BINUTILS_URL="https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
GCC_URL="https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
GMP_URL="https://ftp.gnu.org/gnu/gmp/gmp-${GMP_VERSION}.tar.xz"
MPFR_URL="https://ftp.gnu.org/gnu/mpfr/mpfr-${MPFR_VERSION}.tar.xz"
MPC_URL="https://ftp.gnu.org/gnu/mpc/mpc-${MPC_VERSION}.tar.xz"

BINUTILS_SHA256="154ab23b60070e8f27013c22977f1129425d67d1e8acd6e13010e617811e4cff"
GCC_SHA256="e6738e29597f733270731aa90600f37ffdc045079dfc27ec7e8192cc81085c3e"
GMP_SHA256="a3c2b80201b89e68616f4ad30bc66aee4927c3ce50e33929ca819d5c43538898"
MPFR_SHA256="b67ba0383ef7e8a8563734e2e889ef5ec3c3b898a01d00fa0a6869ad81c6ce01"
MPC_SHA256="91204cd32f164bd3b7c992d4a6a8ce6519511aadab30f78b6982d0bf8d73e931"

# ---------------------------------------------------------------------------
# Build directory
# ---------------------------------------------------------------------------
# Overridable: a GCC build needs several GB, more than a small tmpfs /tmp has.
BUILD_BASE="${BUILD_BASE:-/tmp/veridian-toolchain-build}"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --arch)
                ARCH="${2:?--arch requires a value (x86_64, aarch64, riscv64)}"
                shift 2
                ;;
            --prefix)
                PREFIX="${2:?--prefix requires a path}"
                shift 2
                ;;
            --jobs)
                JOBS="${2:?--jobs requires a number}"
                shift 2
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
        x86_64|aarch64|riscv64) ;;
        *) die "Unsupported architecture: ${ARCH}.  Use x86_64, aarch64, or riscv64." ;;
    esac
}

usage() {
    cat <<'EOF'
Usage: ./scripts/build-cross-toolchain.sh [OPTIONS]

Build a GCC cross-compiler toolchain for VeridianOS.

Options:
  --arch ARCH       Target architecture: x86_64 (default), aarch64, riscv64
  --prefix PREFIX   Installation prefix (default: /opt/veridian/toolchain)
  --jobs N          Parallel make jobs (default: nproc)
  -h, --help        Show this help message

The toolchain is installed under PREFIX and targets ARCH-veridian.  The
sysroot (kernel headers, libc, CRT objects) lives at PREFIX/sysroot.

Example:
  ./scripts/build-cross-toolchain.sh --arch aarch64 --prefix ~/veridian-tc --jobs 8
EOF
}

# ---------------------------------------------------------------------------
# Target triple
# ---------------------------------------------------------------------------
target_triple() {
    echo "${ARCH}-veridian"
}

# ---------------------------------------------------------------------------
# Step 1: Check host prerequisites
# ---------------------------------------------------------------------------
check_prerequisites() {
    step "Checking host prerequisites"

    local missing=()

    for cmd in gcc g++ make tar curl sha256sum patch bison flex \
               makeinfo xz gzip; do
        if ! command -v "${cmd}" &>/dev/null; then
            missing+=("${cmd}")
        fi
    done

    # makeinfo is part of texinfo; some distros split it
    if ! command -v makeinfo &>/dev/null && ! command -v texi2any &>/dev/null; then
        # Already captured above; just a note
        true
    fi

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
        echo "  # Fedora"
        echo "  sudo dnf install gcc gcc-c++ make texinfo bison flex wget xz"
        echo ""
        die "Please install the missing prerequisites and try again."
    fi

    success "All host prerequisites found"
}

# ---------------------------------------------------------------------------
# Step 2: Create build directories
# ---------------------------------------------------------------------------
create_directories() {
    step "Creating build directories"

    mkdir -p "${BUILD_BASE}/downloads"
    mkdir -p "${BUILD_BASE}/sources"
    mkdir -p "${BUILD_BASE}/build-binutils"
    mkdir -p "${BUILD_BASE}/build-gcc-stage1"
    mkdir -p "${BUILD_BASE}/build-gcc-stage2"

    info "Build base: ${BUILD_BASE}"
    success "Directories created"
}

# ---------------------------------------------------------------------------
# Step 3: Download source tarballs
# ---------------------------------------------------------------------------
# Apply a patch to an extracted source tree once. The stamp records the
# patch's SHA-256: a tree patched with an older version of it cannot be
# patched again, and is reported instead of being silently built as it is.
apply_patch_once() {
    local tree="$1" patch_file="$2" label="$3"
    local pname sum stamp
    pname="$(basename "${patch_file}")"
    sum="$(sha256sum "${patch_file}" | cut -d' ' -f1)"
    stamp="${tree}/.applied-${pname}"
    if [[ -f "${stamp}" ]]; then
        [[ "$(cat "${stamp}")" == "${sum}" ]] ||
            die "${tree} has an older ${label}/${pname}; remove the tree to re-extract it"
        info "Patch already applied: ${label}/${pname}"
        return 0
    fi
    info "Applying: ${label}/${pname}"
    patch -d "${tree}" -p1 < "${patch_file}" || die "Failed to apply patch: ${patch_file}"
    echo "${sum}" > "${stamp}"
}

download_sources() {
    step "Downloading source tarballs"

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
        else
            info "Downloading ${filename} ..."
            # curl first: wget may be a sandbox wrapper (firejail) that
            # cannot write outside the home directory.
            if command -v curl &>/dev/null; then
                curl -fL --progress-bar -o "${dest}" "${url}" \
                    || { rm -f "${dest}"; die "Failed to download ${url}"; }
            else
                wget -q --show-progress -O "${dest}" "${url}" \
                    || { rm -f "${dest}"; die "Failed to download ${url}"; }
            fi
        fi
    done

    success "All source tarballs downloaded"
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
        ["mpc-${MPC_VERSION}.tar.xz"]="${MPC_SHA256}"
    )

    for filename in "${!expected[@]}"; do
        local file="${BUILD_BASE}/downloads/${filename}"
        local want="${expected[${filename}]}"
        local got
        got="$(sha256sum "${file}" | awk '{print $1}')"

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
        local name
        name="$(basename "${archive}")"
        # Derive expected directory name (strip .tar.*)
        local dirname="${name%.tar.*}"

        if [[ -d "${srcdir}/${dirname}" ]]; then
            info "Already extracted: ${dirname}"
        else
            info "Extracting ${name} ..."
            tar -xf "${archive}" -C "${srcdir}"
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
            apply_patch_once "${srcdir}/binutils-${BINUTILS_VERSION}" "${patch_file}" binutils
        done
    else
        warn "No binutils patches directory found at ${patches_dir}"
    fi

    # GCC patches
    patches_dir="${PROJECT_ROOT}/ports/gcc/patches"
    if [[ -d "${patches_dir}" ]]; then
        for patch_file in "${patches_dir}"/*.patch; do
            [[ -f "${patch_file}" ]] || continue
            apply_patch_once "${srcdir}/gcc-${GCC_VERSION}" "${patch_file}" gcc
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
# Step 8: Build binutils
# ---------------------------------------------------------------------------
build_binutils() {
    step "Building binutils ${BINUTILS_VERSION} for $(target_triple)"

    local builddir="${BUILD_BASE}/build-binutils"
    local srcdir="${BUILD_BASE}/sources/binutils-${BINUTILS_VERSION}"

    # Clean previous build if reconfiguring
    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring binutils ..."
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --target="$(target_triple)" \
                --prefix="${PREFIX}" \
                --with-sysroot="${PREFIX}/sysroot" \
                --disable-nls \
                --disable-werror \
                --enable-deterministic-archives \
                --enable-new-dtags \
                --enable-default-hash-style=gnu
        )
    fi

    info "Compiling binutils (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}"

    info "Installing binutils ..."
    make -C "${builddir}" install

    success "Binutils installed to ${PREFIX}"
}

# ---------------------------------------------------------------------------
# Step 9: Build GCC Stage 1 (freestanding, no libc)
# ---------------------------------------------------------------------------
build_gcc_stage1() {
    step "Building GCC Stage 1 (C only, freestanding) for $(target_triple)"

    local builddir="${BUILD_BASE}/build-gcc-stage1"
    local srcdir="${BUILD_BASE}/sources/gcc-${GCC_VERSION}"

    # Ensure the just-built binutils are on PATH
    export PATH="${PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring GCC Stage 1 ..."
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --target="$(target_triple)" \
                --prefix="${PREFIX}" \
                --with-sysroot="${PREFIX}/sysroot" \
                --enable-languages=c \
                --disable-shared \
                --disable-threads \
                --disable-multilib \
                --disable-libssp \
                --disable-libquadmath \
                --disable-libgomp \
                --disable-libatomic \
                --disable-libstdcxx \
                --disable-nls \
                --with-newlib \
                --without-headers
        )
    fi

    info "Compiling GCC Stage 1 (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}" all-gcc

    info "Installing GCC Stage 1 ..."
    make -C "${builddir}" install-gcc

    success "GCC Stage 1 installed to ${PREFIX}"
}

# ---------------------------------------------------------------------------
# Step 10: Build sysroot (headers, CRT objects, libc)
# ---------------------------------------------------------------------------
build_sysroot() {
    step "Building sysroot (kernel headers, CRT, libc)"

    "${SCRIPT_DIR}/build-sysroot.sh" \
        --arch "${ARCH}" \
        --prefix "${PREFIX}"

    success "Sysroot populated at ${PREFIX}/sysroot"
}

# ---------------------------------------------------------------------------
# Step 11: Build GCC Stage 2 (with sysroot and libgcc)
# ---------------------------------------------------------------------------
build_gcc_stage2() {
    step "Building GCC Stage 2 (with sysroot and libgcc) for $(target_triple)"

    local builddir="${BUILD_BASE}/build-gcc-stage2"
    local srcdir="${BUILD_BASE}/sources/gcc-${GCC_VERSION}"

    export PATH="${PREFIX}/bin:${PATH}"

    if [[ ! -f "${builddir}/Makefile" ]]; then
        info "Configuring GCC Stage 2 ..."
        (
            cd "${builddir}"
            "${srcdir}/configure" \
                --target="$(target_triple)" \
                --prefix="${PREFIX}" \
                --with-sysroot="${PREFIX}/sysroot" \
                --enable-languages=c,c++ \
                --disable-shared \
                --disable-threads \
                --disable-multilib \
                --disable-libssp \
                --disable-libquadmath \
                --disable-libgomp \
                --disable-libatomic \
                --disable-libstdcxx \
                --disable-nls \
                --with-newlib
        )
    fi

    info "Compiling GCC Stage 2 (${JOBS} jobs) ..."
    make -C "${builddir}" -j"${JOBS}" all-gcc all-target-libgcc

    info "Installing GCC Stage 2 ..."
    make -C "${builddir}" install-gcc install-target-libgcc

    success "GCC Stage 2 installed to ${PREFIX}"
}

# ---------------------------------------------------------------------------
# Step 12: Print summary
# ---------------------------------------------------------------------------
print_summary() {
    local triple
    triple="$(target_triple)"

    step "Toolchain build complete!"

    echo ""
    printf "${GREEN}Toolchain location:${NC}  %s\n" "${PREFIX}"
    printf "${GREEN}Target triple:${NC}       %s\n" "${triple}"
    printf "${GREEN}Sysroot:${NC}             %s\n" "${PREFIX}/sysroot"
    echo ""
    echo "Installed tools:"
    echo "  ${PREFIX}/bin/${triple}-gcc"
    echo "  ${PREFIX}/bin/${triple}-as"
    echo "  ${PREFIX}/bin/${triple}-ld"
    echo "  ${PREFIX}/bin/${triple}-ar"
    echo "  ${PREFIX}/bin/${triple}-objdump"
    echo "  ${PREFIX}/bin/${triple}-objcopy"
    echo "  ${PREFIX}/bin/${triple}-strip"
    echo "  ${PREFIX}/bin/${triple}-nm"
    echo "  ${PREFIX}/bin/${triple}-ranlib"
    echo ""
    echo "Add the toolchain to your PATH:"
    echo ""
    echo "  export PATH=\"${PREFIX}/bin:\${PATH}\""
    echo ""
    echo "Compile a VeridianOS program:"
    echo ""
    echo "  ${triple}-gcc -o hello hello.c"
    echo ""
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Cross-Compilation Toolchain Builder${NC}\n"
    printf "  Architecture:  %s\n" "${ARCH}"
    printf "  Prefix:        %s\n" "${PREFIX}"
    printf "  Parallel jobs: %s\n" "${JOBS}"
    printf "  Build dir:     %s\n" "${BUILD_BASE}"
    echo ""

    check_prerequisites
    create_directories
    download_sources
    verify_checksums
    extract_sources
    apply_patches
    symlink_gcc_deps
    build_binutils
    build_gcc_stage1
    build_sysroot
    build_gcc_stage2
    print_summary
}

main "$@"

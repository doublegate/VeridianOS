#!/usr/bin/env bash
# VeridianOS Sysroot Builder
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Populates the cross-compilation sysroot with:
#   1. Kernel syscall headers  (toolchain/sysroot/include/veridian/)
#   2. libc headers            (userland/libc/include/)
#   3. CRT startup objects     (crt0.o, crti.o, crtn.o)
#   4. Static libc             (libc.a)
#
# This script is normally called by build-cross-toolchain.sh after
# GCC Stage 1 is installed.  It can also be run standalone to rebuild
# only the sysroot.
#
# Usage:
#   ./scripts/build-sysroot.sh [--arch ARCH] [--prefix PREFIX]
#
#   ARCH:   x86_64 (default), aarch64, riscv64
#   PREFIX: /opt/veridian/toolchain (default)

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
PREFIX="/opt/veridian/toolchain"

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
        *) die "Unsupported architecture: ${ARCH}" ;;
    esac
}

usage() {
    cat <<'EOF'
Usage: ./scripts/build-sysroot.sh [OPTIONS]

Populate the VeridianOS cross-compilation sysroot with headers, CRT
objects, and a static libc.

Options:
  --arch ARCH       Target architecture: x86_64 (default), aarch64, riscv64
  --prefix PREFIX   Toolchain prefix (default: /opt/veridian/toolchain)
  -h, --help        Show this help message

The sysroot is created at PREFIX/sysroot with the layout:
  PREFIX/sysroot/
    usr/
      include/
        veridian/     (kernel syscall headers)
        stdio.h ...   (libc headers)
        sys/          (libc sys/ headers)
      lib/
        crt0.o        (C runtime startup)
        crti.o        (init prologue)
        crtn.o        (init epilogue)
        libc.a        (static C library)
EOF
}

# ---------------------------------------------------------------------------
# Derived variables
# ---------------------------------------------------------------------------
setup_vars() {
    TARGET="${ARCH}-veridian"

    CC="${PREFIX}/bin/${TARGET}-gcc"
    AS="${PREFIX}/bin/${TARGET}-as"
    AR="${PREFIX}/bin/${TARGET}-ar"
    RANLIB="${PREFIX}/bin/${TARGET}-ranlib"

    SYSROOT="${PREFIX}/sysroot"

    # Source locations in the project tree
    KERNEL_HEADERS="${PROJECT_ROOT}/toolchain/sysroot/include/veridian"
    LIBC_INCLUDE="${PROJECT_ROOT}/userland/libc/include"
    LIBC_DIR="${PROJECT_ROOT}/userland/libc"
    CRT_DIR="${PROJECT_ROOT}/toolchain/sysroot/crt"
}

# ---------------------------------------------------------------------------
# Step 1: Verify cross-tools exist
# ---------------------------------------------------------------------------
verify_cross_tools() {
    step "Verifying cross-compilation tools"

    # At minimum we need the assembler and archiver from binutils.
    # GCC Stage 1 may or may not be ready yet; we only need it for libc.
    for tool in "${AS}" "${AR}" "${RANLIB}"; do
        if [[ ! -x "${tool}" ]]; then
            die "Cross-tool not found: ${tool}  (build binutils first)"
        fi
    done

    if [[ -x "${CC}" ]]; then
        info "Cross-compiler found: ${CC}"
    else
        warn "Cross-compiler not found at ${CC}"
        warn "CRT objects will be assembled but libc cannot be built yet."
        warn "Re-run after GCC Stage 1 is installed to build libc."
    fi

    success "Cross-tools verified"
}

# ---------------------------------------------------------------------------
# Step 2: Create sysroot directory structure
# ---------------------------------------------------------------------------
create_sysroot_dirs() {
    step "Creating sysroot directory structure"

    mkdir -p "${SYSROOT}/usr/include/veridian"
    mkdir -p "${SYSROOT}/usr/include/sys"
    mkdir -p "${SYSROOT}/usr/lib"

    success "Sysroot directories created at ${SYSROOT}"
}

# ---------------------------------------------------------------------------
# Step 3: Install kernel syscall headers
# ---------------------------------------------------------------------------
install_kernel_headers() {
    step "Installing kernel syscall headers"

    if [[ ! -d "${KERNEL_HEADERS}" ]]; then
        die "Kernel headers not found at ${KERNEL_HEADERS}"
    fi

    local count=0
    for header in "${KERNEL_HEADERS}"/*.h; do
        [[ -f "${header}" ]] || continue
        cp "${header}" "${SYSROOT}/usr/include/veridian/"
        count=$((count + 1))
    done

    info "Installed ${count} kernel headers to ${SYSROOT}/usr/include/veridian/"
    success "Kernel headers installed"
}

# ---------------------------------------------------------------------------
# Step 4: Install libc headers
# ---------------------------------------------------------------------------
install_libc_headers() {
    step "Installing libc headers"

    if [[ ! -d "${LIBC_INCLUDE}" ]]; then
        die "libc headers not found at ${LIBC_INCLUDE}"
    fi

    # Top-level headers (stdio.h, stdlib.h, etc.)
    local count=0
    for header in "${LIBC_INCLUDE}"/*.h; do
        [[ -f "${header}" ]] || continue
        cp "${header}" "${SYSROOT}/usr/include/"
        count=$((count + 1))
    done
    info "Installed ${count} top-level libc headers"

    # sys/ subdirectory headers
    if [[ -d "${LIBC_INCLUDE}/sys" ]]; then
        local sys_count=0
        for header in "${LIBC_INCLUDE}/sys"/*.h; do
            [[ -f "${header}" ]] || continue
            cp "${header}" "${SYSROOT}/usr/include/sys/"
            sys_count=$((sys_count + 1))
        done
        info "Installed ${sys_count} sys/ libc headers"
    fi

    success "libc headers installed"
}

# ---------------------------------------------------------------------------
# Step 5: Assemble CRT objects
# ---------------------------------------------------------------------------
assemble_crt_objects() {
    step "Assembling CRT objects for ${ARCH}"

    local destdir="${SYSROOT}/usr/lib"

    # Determine arch-specific preprocessor define for the cross-assembler
    local arch_define
    case "${ARCH}" in
        x86_64)  arch_define="__x86_64__" ;;
        aarch64) arch_define="__aarch64__" ;;
        riscv64) arch_define="__riscv"     ;;
    esac

    # crt0.o -- architecture-specific entry point
    local crt0_src="${CRT_DIR}/${ARCH}/crt0.S"
    if [[ ! -f "${crt0_src}" ]]; then
        die "crt0.S not found for ${ARCH} at ${crt0_src}"
    fi
    info "Assembling crt0.o from ${crt0_src}"
    "${AS}" -o "${destdir}/crt0.o" "${crt0_src}"

    # crti.o -- uses C preprocessor ifdefs for multi-arch
    local crti_src="${CRT_DIR}/crti.S"
    if [[ ! -f "${crti_src}" ]]; then
        die "crti.S not found at ${crti_src}"
    fi

    # crtn.o -- uses C preprocessor ifdefs for multi-arch
    local crtn_src="${CRT_DIR}/crtn.S"
    if [[ ! -f "${crtn_src}" ]]; then
        die "crtn.S not found at ${crtn_src}"
    fi

    # crti.S and crtn.S use #if defined(...) so we need the C preprocessor.
    # If GCC is available, use it.  Otherwise, fall back to the assembler
    # with a manual -D flag (which works for gas but not all assemblers).
    if [[ -x "${CC}" ]]; then
        info "Assembling crti.o (via gcc -c)"
        "${CC}" -c -o "${destdir}/crti.o" "${crti_src}" \
            -nostdlib -nostdinc -ffreestanding

        info "Assembling crtn.o (via gcc -c)"
        "${CC}" -c -o "${destdir}/crtn.o" "${crtn_src}" \
            -nostdlib -nostdinc -ffreestanding
    else
        # Fallback: use the assembler with preprocessor defines.
        # GNU as supports --defsym but not C preprocessor macros.
        # We run cpp manually first if available.
        if command -v cpp &>/dev/null; then
            info "Assembling crti.o (via cpp + as)"
            cpp -D"${arch_define}" -P "${crti_src}" | "${AS}" -o "${destdir}/crti.o" -

            info "Assembling crtn.o (via cpp + as)"
            cpp -D"${arch_define}" -P "${crtn_src}" | "${AS}" -o "${destdir}/crtn.o" -
        else
            warn "Neither cross-gcc nor host cpp found."
            warn "Skipping crti.o and crtn.o -- they require C preprocessing."
            warn "Re-run after GCC Stage 1 is installed."
        fi
    fi

    # List what we installed
    echo ""
    info "CRT objects in ${destdir}:"
    for obj in crt0.o crti.o crtn.o; do
        if [[ -f "${destdir}/${obj}" ]]; then
            printf "  ${GREEN}+${NC} %s  (%s bytes)\n" "${obj}" "$(wc -c < "${destdir}/${obj}")"
        else
            printf "  ${YELLOW}-${NC} %s  (not built)\n" "${obj}"
        fi
    done

    success "CRT objects assembled"
}

# ---------------------------------------------------------------------------
# Step 6: Build libc
# ---------------------------------------------------------------------------
build_libc() {
    step "Building libc for ${ARCH}"

    if [[ ! -x "${CC}" ]]; then
        warn "Cross-compiler not available at ${CC}"
        warn "Skipping libc build.  Re-run after GCC Stage 1 is installed."
        return 0
    fi

    if [[ ! -f "${LIBC_DIR}/Makefile" ]]; then
        die "libc Makefile not found at ${LIBC_DIR}/Makefile"
    fi

    # The Makefile uses CROSS_PREFIX to derive CC, AR, RANLIB.
    # We set CROSS_PREFIX to point at our toolchain.
    local cross_prefix="${PREFIX}/bin/${ARCH}-veridian-"

    info "Building libc with CROSS_PREFIX=${cross_prefix}"
    make -C "${LIBC_DIR}" \
        ARCH="${ARCH}" \
        CROSS_PREFIX="${cross_prefix}" \
        CC="${CC}" \
        AR="${AR}" \
        RANLIB="${RANLIB}" \
        clean

    make -C "${LIBC_DIR}" \
        ARCH="${ARCH}" \
        CROSS_PREFIX="${cross_prefix}" \
        CC="${CC}" \
        AR="${AR}" \
        RANLIB="${RANLIB}"

    success "libc built"
}

# ---------------------------------------------------------------------------
# Step 7: Install libc
# ---------------------------------------------------------------------------
install_libc() {
    step "Installing libc into sysroot"

    local libc_archive="${LIBC_DIR}/build/${ARCH}/libc.a"

    if [[ ! -f "${libc_archive}" ]]; then
        warn "libc.a not found at ${libc_archive}"
        warn "Skipping libc installation (build may have been skipped)."
        return 0
    fi

    cp "${libc_archive}" "${SYSROOT}/usr/lib/libc.a"
    info "Installed libc.a ($(wc -c < "${libc_archive}") bytes)"

    success "libc installed to ${SYSROOT}/usr/lib/libc.a"
}

# ---------------------------------------------------------------------------
# Step 8: Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "Sysroot summary"

    echo ""
    printf "${GREEN}Sysroot:${NC} %s\n" "${SYSROOT}"
    echo ""
    echo "Installed files:"

    # Headers
    local header_count
    header_count=$(find "${SYSROOT}/usr/include" -name '*.h' 2>/dev/null | wc -l)
    echo "  Headers:     ${header_count} files in ${SYSROOT}/usr/include/"

    # CRT objects
    for obj in crt0.o crti.o crtn.o; do
        if [[ -f "${SYSROOT}/usr/lib/${obj}" ]]; then
            printf "  %-12s %s bytes\n" "${obj}:" "$(wc -c < "${SYSROOT}/usr/lib/${obj}")"
        fi
    done

    # libc
    if [[ -f "${SYSROOT}/usr/lib/libc.a" ]]; then
        printf "  %-12s %s bytes\n" "libc.a:" "$(wc -c < "${SYSROOT}/usr/lib/libc.a")"
    else
        printf "  %-12s ${YELLOW}(not built)${NC}\n" "libc.a:"
    fi

    echo ""
    success "Sysroot ready"
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Sysroot Builder${NC}\n"
    printf "  Architecture:  %s\n" "${ARCH}"
    printf "  Prefix:        %s\n" "${PREFIX}"
    echo ""

    setup_vars
    verify_cross_tools
    create_sysroot_dirs
    install_kernel_headers
    install_libc_headers
    assemble_crt_objects
    build_libc
    install_libc
    print_summary
}

main "$@"

#!/usr/bin/env bash
# VeridianOS End-to-End Cross-Compilation Test Script
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compiles test programs in userland/tests/ against the VeridianOS
# sysroot and optionally boots them in QEMU to verify execution.
#
# Usage:
#   ./scripts/cross-compile-test.sh [OPTIONS]
#
# Options:
#   --arch ARCH       Target architecture: x86_64 (default), aarch64, riscv64
#   --toolchain DIR   Cross-compiler root (default: /opt/veridian/toolchain)
#   --run             Boot QEMU after compilation to run the test
#   --help            Show this help message
#
# Prerequisites:
#   - Cross-compiler built and installed at the toolchain path
#   - libc.a built and installed into sysroot:
#       cd userland/libc && make ARCH=<arch> install
#   - For --run: kernel built for the target architecture:
#       ./build-kernel.sh <arch> dev

set -euo pipefail

# ========================================================================= #
# Configuration defaults                                                    #
# ========================================================================= #

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

ARCH="x86_64"
TOOLCHAIN_PREFIX="/opt/veridian/toolchain"
RUN_QEMU=false

# ========================================================================= #
# Argument parsing                                                          #
# ========================================================================= #

usage() {
    cat <<'USAGE'
Usage: ./scripts/cross-compile-test.sh [OPTIONS]

Cross-compile and optionally run VeridianOS E2E test programs.

Options:
  --arch ARCH       Target architecture: x86_64 (default), aarch64, riscv64
  --toolchain DIR   Cross-compiler root (default: /opt/veridian/toolchain)
  --run             Boot QEMU after compilation to run the test
  --help            Show this help message

Examples:
  # Compile tests for x86_64
  ./scripts/cross-compile-test.sh

  # Compile for AArch64 and run in QEMU
  ./scripts/cross-compile-test.sh --arch aarch64 --run

  # Use a custom toolchain location
  ./scripts/cross-compile-test.sh --toolchain /usr/local/veridian

Prerequisites:
  1. Build the cross-compiler (see docs/CROSS-COMPILATION.md)
  2. Build libc: cd userland/libc && make ARCH=<arch> install
  3. For --run: build the kernel: ./build-kernel.sh <arch> dev
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --toolchain)
            TOOLCHAIN_PREFIX="$2"
            shift 2
            ;;
        --run)
            RUN_QEMU=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "Error: unknown option: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

# Validate architecture
case "${ARCH}" in
    x86_64|aarch64|riscv64) ;;
    *)
        echo "Error: unsupported architecture '${ARCH}'" >&2
        echo "Supported: x86_64, aarch64, riscv64" >&2
        exit 1
        ;;
esac

# ========================================================================= #
# Paths                                                                     #
# ========================================================================= #

TESTS_DIR="${PROJECT_ROOT}/userland/tests"
SYSROOT="${PROJECT_ROOT}/toolchain/sysroot"
LIBC_DIR="${PROJECT_ROOT}/userland/libc"
BUILD_DIR="${TESTS_DIR}/build/${ARCH}"
LIBC_INCDIR="${LIBC_DIR}/include"
SYS_INCDIR="${SYSROOT}/include"
LIBC_LIBDIR="${SYSROOT}/lib/${ARCH}"
CRT0="${LIBC_LIBDIR}/crt0.o"
LOG_DIR="/tmp/VeridianOS"

# Cross-compiler tools
CC="${TOOLCHAIN_PREFIX}/bin/${ARCH}-veridian-gcc"
READELF_CMD="${TOOLCHAIN_PREFIX}/bin/${ARCH}-veridian-readelf"
SIZE_CMD="${TOOLCHAIN_PREFIX}/bin/${ARCH}-veridian-size"

# ========================================================================= #
# Preflight checks                                                          #
# ========================================================================= #

echo "=== VeridianOS E2E Cross-Compilation Test ==="
echo "  Architecture:  ${ARCH}"
echo "  Toolchain:     ${TOOLCHAIN_PREFIX}"
echo "  Sysroot:       ${SYSROOT}"
echo "  Tests dir:     ${TESTS_DIR}"
echo "  Output dir:    ${BUILD_DIR}"
echo ""

# Check cross-compiler
if [[ ! -x "${CC}" ]]; then
    echo "Error: cross-compiler not found at ${CC}" >&2
    echo "" >&2
    echo "The VeridianOS cross-compiler must be built first." >&2
    echo "See docs/CROSS-COMPILATION.md for instructions." >&2
    echo "" >&2
    echo "Expected toolchain location: ${TOOLCHAIN_PREFIX}/bin/${ARCH}-veridian-gcc" >&2
    exit 1
fi

echo "[OK] Cross-compiler: ${CC}"

# Check libc.a
HAVE_LIBC=false
if [[ -f "${LIBC_LIBDIR}/libc.a" ]]; then
    echo "[OK] libc.a: ${LIBC_LIBDIR}/libc.a"
    HAVE_LIBC=true
else
    echo "Warning: libc.a not found at ${LIBC_LIBDIR}/libc.a" >&2
    echo "  The 'hello' test requires libc. Build it with:" >&2
    echo "    cd ${LIBC_DIR} && make ARCH=${ARCH} install" >&2
    echo "" >&2
fi

# Check crt0.o
HAVE_CRT=false
if [[ -f "${CRT0}" ]]; then
    echo "[OK] crt0.o: ${CRT0}"
    HAVE_CRT=true
else
    echo "Warning: crt0.o not found at ${CRT0}" >&2
    echo "  The 'hello' test requires crt0.o. Assemble it from:" >&2
    echo "    ${SYSROOT}/crt/${ARCH}/crt0.S" >&2
    echo "" >&2
fi

echo ""

# ========================================================================= #
# Build output directory                                                    #
# ========================================================================= #

mkdir -p "${BUILD_DIR}"
mkdir -p "${LOG_DIR}"

# ========================================================================= #
# Compile: minimal (no-libc) test                                          #
# ========================================================================= #

echo "--- Compiling: minimal (no libc, raw syscalls) ---"

# Architecture-specific flags
ARCH_FLAGS=""
case "${ARCH}" in
    x86_64)  ARCH_FLAGS="-mno-red-zone -mcmodel=small" ;;
    aarch64) ARCH_FLAGS="-mgeneral-regs-only" ;;
    riscv64) ARCH_FLAGS="-march=rv64gc -mabi=lp64d" ;;
esac

set -x
"${CC}" \
    -nostdlib -nostdinc -ffreestanding -static \
    -Wall -Wextra -O2 -g \
    ${ARCH_FLAGS} \
    -o "${BUILD_DIR}/minimal" \
    "${TESTS_DIR}/minimal.c"
set +x

echo "[OK] Built: ${BUILD_DIR}/minimal"
echo ""

# Show ELF info for minimal
echo "  ELF info (minimal):"
file "${BUILD_DIR}/minimal" 2>/dev/null || true
if [[ -x "${READELF_CMD}" ]]; then
    "${READELF_CMD}" -h "${BUILD_DIR}/minimal" 2>/dev/null | grep -E 'Class|Machine|Entry' || true
else
    readelf -h "${BUILD_DIR}/minimal" 2>/dev/null | grep -E 'Class|Machine|Entry' || true
fi
if [[ -x "${SIZE_CMD}" ]]; then
    "${SIZE_CMD}" "${BUILD_DIR}/minimal" 2>/dev/null || true
else
    size "${BUILD_DIR}/minimal" 2>/dev/null || true
fi
echo ""

# ========================================================================= #
# Compile: hello (libc) test                                                #
# ========================================================================= #

if [[ "${HAVE_LIBC}" == true && "${HAVE_CRT}" == true ]]; then
    echo "--- Compiling: hello (with libc) ---"

    set -x
    "${CC}" \
        -std=c11 -nostdinc -ffreestanding -static \
        -isystem "${LIBC_INCDIR}" \
        -isystem "${SYS_INCDIR}" \
        -fno-stack-protector -fno-builtin \
        -Wall -Wextra -O2 -g \
        ${ARCH_FLAGS} \
        -nostdlib -L"${LIBC_LIBDIR}" \
        -o "${BUILD_DIR}/hello" \
        "${CRT0}" "${TESTS_DIR}/hello.c" \
        -lc
    set +x

    echo "[OK] Built: ${BUILD_DIR}/hello"
    echo ""

    # Show ELF info for hello
    echo "  ELF info (hello):"
    file "${BUILD_DIR}/hello" 2>/dev/null || true
    if [[ -x "${READELF_CMD}" ]]; then
        "${READELF_CMD}" -h "${BUILD_DIR}/hello" 2>/dev/null | grep -E 'Class|Machine|Entry' || true
    else
        readelf -h "${BUILD_DIR}/hello" 2>/dev/null | grep -E 'Class|Machine|Entry' || true
    fi
    if [[ -x "${SIZE_CMD}" ]]; then
        "${SIZE_CMD}" "${BUILD_DIR}/hello" 2>/dev/null || true
    else
        size "${BUILD_DIR}/hello" 2>/dev/null || true
    fi
    echo ""
else
    echo "--- Skipping: hello (libc or crt0 not available) ---"
    echo ""
fi

# ========================================================================= #
# Summary                                                                   #
# ========================================================================= #

echo "=== Build Summary ==="
echo "  Architecture: ${ARCH}"
echo "  Output:       ${BUILD_DIR}/"
ls -lh "${BUILD_DIR}/" 2>/dev/null
echo ""

# ========================================================================= #
# Optional: run in QEMU                                                     #
# ========================================================================= #

if [[ "${RUN_QEMU}" == true ]]; then
    echo "=== Running in QEMU ==="
    echo ""
    echo "Note: Running user-space test programs in QEMU requires the kernel"
    echo "to be able to load and execute ELF binaries from a disk image or"
    echo "embedded initramfs. This is a placeholder for the full integration."
    echo ""
    echo "Current status:"
    echo "  - The kernel boots to an interactive shell (vsh)"
    echo "  - User-mode transition works (Ring 3 via SYSCALL/SYSRET on x86_64)"
    echo "  - exec() syscall is implemented but needs a filesystem to load from"
    echo ""
    echo "To test manually:"
    echo "  1. Build the kernel: ./build-kernel.sh ${ARCH} dev"
    echo ""

    case "${ARCH}" in
        x86_64)
            echo "  2. Run QEMU (serial only):"
            echo "     qemu-system-x86_64 -enable-kvm \\"
            echo "       -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
            echo "       -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
            echo "       -device ide-hd,drive=disk0 \\"
            echo "       -serial stdio -display none -m 256M"
            ;;
        aarch64)
            echo "  2. Run QEMU:"
            echo "     qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \\"
            echo "       -kernel target/aarch64-unknown-none/debug/veridian-kernel \\"
            echo "       -serial stdio -display none"
            ;;
        riscv64)
            echo "  2. Run QEMU:"
            echo "     qemu-system-riscv64 -M virt -m 256M -bios default \\"
            echo "       -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \\"
            echo "       -serial stdio -display none"
            ;;
    esac

    echo ""
    echo "  3. Once virtio-blk or initramfs support is available, test programs"
    echo "     can be loaded into a disk image and executed from the shell."
    echo ""
    echo "Full QEMU integration will be added when the kernel can load"
    echo "user-space ELF binaries from a block device."
fi

echo "=== Done ==="

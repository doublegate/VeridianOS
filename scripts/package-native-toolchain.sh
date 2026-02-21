#!/usr/bin/env bash
# VeridianOS Native Toolchain Packager
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Packages the static native GCC toolchain (built by build-native-gcc-static.sh)
# into the TAR rootfs format that VeridianOS loads at boot via the virtio-blk
# TAR loader.
#
# The TAR archive contains paths like:
#   bin/gcc
#   bin/as
#   bin/ld
#   usr/lib/libc.a
#   usr/lib/gcc/x86_64-veridian/14.2.0/libgcc.a
#   usr/include/stdio.h
#   ...
#
# This can be used as a standalone virtio-blk drive alongside the regular
# rootfs.tar, or merged with it for a single-drive boot image.
#
# Usage:
#   ./scripts/package-native-toolchain.sh [OPTIONS]
#
#   --input-dir PATH    Input directory (default: target/native-gcc-static)
#   --output PATH       Output TAR path (default: target/native-toolchain.tar)
#   --merge PATH        Merge with existing rootfs TAR (optional)
#   -h, --help          Show this help message

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
INPUT_DIR=""
OUTPUT_TAR=""
MERGE_TAR=""

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --input-dir)
                INPUT_DIR="${2:?--input-dir requires a path}"
                shift 2
                ;;
            --output)
                OUTPUT_TAR="${2:?--output requires a path}"
                shift 2
                ;;
            --merge)
                MERGE_TAR="${2:?--merge requires a path}"
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

    # Apply defaults
    if [[ -z "${INPUT_DIR}" ]]; then
        INPUT_DIR="${PROJECT_ROOT}/target/native-gcc-static"
    fi

    if [[ -z "${OUTPUT_TAR}" ]]; then
        OUTPUT_TAR="${PROJECT_ROOT}/target/native-toolchain.tar"
    fi
}

usage() {
    cat <<'EOF'
Usage: ./scripts/package-native-toolchain.sh [OPTIONS]

Package the static native GCC toolchain into a TAR archive suitable for
booting VeridianOS with the virtio-blk TAR loader.

Options:
  --input-dir PATH    Input directory (default: target/native-gcc-static)
  --output PATH       Output TAR path (default: target/native-toolchain.tar)
  --merge PATH        Merge with existing rootfs TAR (combines both into one)
  -h, --help          Show this help message

The input directory should contain the rootfs layout (usr/bin/gcc, etc.)
as produced by build-native-gcc-static.sh.

Examples:
  # Package the toolchain
  ./scripts/package-native-toolchain.sh

  # Merge with existing rootfs
  ./scripts/package-native-toolchain.sh --merge target/rootfs.tar

  # Custom output location
  ./scripts/package-native-toolchain.sh --output ~/veridian-toolchain.tar

QEMU usage (standalone drive):
  qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive file=target/rootfs.tar,if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -drive file=target/native-toolchain.tar,if=none,id=vd1,format=raw \
    -device virtio-blk-pci,drive=vd1 \
    -serial stdio -display none -m 512M
EOF
}

# ---------------------------------------------------------------------------
# Verify input
# ---------------------------------------------------------------------------
verify_input() {
    step "Verifying input directory"

    if [[ ! -d "${INPUT_DIR}" ]]; then
        die "Input directory not found: ${INPUT_DIR}"
    fi

    # Check for at least some expected files
    local found_files=0
    for expected in usr/bin/gcc usr/bin/as usr/bin/ld usr/lib/libc.a; do
        if [[ -f "${INPUT_DIR}/${expected}" ]]; then
            found_files=$((found_files + 1))
        fi
    done

    if [[ ${found_files} -eq 0 ]]; then
        die "Input directory appears empty or invalid: ${INPUT_DIR}"
    fi

    info "Input directory: ${INPUT_DIR}"
    info "Found ${found_files} expected files"
    success "Input verified"
}

# ---------------------------------------------------------------------------
# Create TAR archive
# ---------------------------------------------------------------------------
create_tar() {
    step "Creating TAR archive"

    mkdir -p "$(dirname "${OUTPUT_TAR}")"

    if [[ -n "${MERGE_TAR}" ]]; then
        # Merge mode: extract existing rootfs, overlay toolchain, re-tar
        if [[ ! -f "${MERGE_TAR}" ]]; then
            die "Merge source TAR not found: ${MERGE_TAR}"
        fi

        local merge_dir="/tmp/veridian-toolchain-merge"
        rm -rf "${merge_dir}"
        mkdir -p "${merge_dir}"

        info "Extracting base rootfs from ${MERGE_TAR} ..."
        tar xf "${MERGE_TAR}" -C "${merge_dir}"

        info "Overlaying native toolchain ..."
        cp -r "${INPUT_DIR}/"* "${merge_dir}/"

        info "Creating merged TAR ..."
        (
            cd "${merge_dir}"
            tar cf "${OUTPUT_TAR}" .
        )

        rm -rf "${merge_dir}"
        success "Merged TAR created"
    else
        # Standalone mode: tar the input directory directly
        info "Creating standalone TAR ..."
        (
            cd "${INPUT_DIR}"
            tar cf "${OUTPUT_TAR}" .
        )
        success "TAR created"
    fi
}

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------
print_summary() {
    step "Package summary"

    local size
    size="$(stat -c%s "${OUTPUT_TAR}" 2>/dev/null || stat -f%z "${OUTPUT_TAR}" 2>/dev/null || echo "?")"

    echo ""
    printf "${GREEN}Archive:${NC}  %s\n" "${OUTPUT_TAR}"
    printf "${GREEN}Size:${NC}     %s bytes (%s KB)\n" "${size}" "$((${size:-0} / 1024))"
    echo ""

    info "Archive contents (first 30 entries):"
    tar tf "${OUTPUT_TAR}" | head -30
    local total_files
    total_files="$(tar tf "${OUTPUT_TAR}" | wc -l)"
    if [[ "${total_files}" -gt 30 ]]; then
        info "  ... and $((total_files - 30)) more entries"
    fi
    echo ""
    info "Total entries: ${total_files}"
    echo ""

    if [[ -n "${MERGE_TAR}" ]]; then
        echo "This archive contains the base rootfs + native toolchain."
        echo "Use it as the sole virtio-blk drive."
    else
        echo "This archive contains only the native toolchain."
        echo "Use it as an additional virtio-blk drive alongside rootfs.tar."
    fi
    echo ""
    echo "QEMU usage:"
    echo "  qemu-system-x86_64 -enable-kvm \\"
    echo "    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
    echo "    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
    echo "    -device ide-hd,drive=disk0 \\"
    if [[ -n "${MERGE_TAR}" ]]; then
        echo "    -drive file=${OUTPUT_TAR},if=none,id=vd0,format=raw \\"
        echo "    -device virtio-blk-pci,drive=vd0 \\"
    else
        echo "    -drive file=target/rootfs.tar,if=none,id=vd0,format=raw \\"
        echo "    -device virtio-blk-pci,drive=vd0 \\"
        echo "    -drive file=${OUTPUT_TAR},if=none,id=vd1,format=raw \\"
        echo "    -device virtio-blk-pci,drive=vd1 \\"
    fi
    echo "    -serial stdio -display none -m 512M"
    echo ""
}

# ===========================================================================
# Main
# ===========================================================================
main() {
    parse_args "$@"

    echo ""
    printf "${BOLD}VeridianOS Native Toolchain Packager${NC}\n"
    printf "  Input:   %s\n" "${INPUT_DIR}"
    printf "  Output:  %s\n" "${OUTPUT_TAR}"
    if [[ -n "${MERGE_TAR}" ]]; then
        printf "  Merge:   %s\n" "${MERGE_TAR}"
    fi
    echo ""

    verify_input
    create_tar
    print_summary
}

main "$@"

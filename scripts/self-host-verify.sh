#!/usr/bin/env bash
# VeridianOS Rust Self-Hosting Verification Script
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Verifies that the Rust compiler runs correctly ON VeridianOS by:
#   1. Booting VeridianOS in QEMU with the Rust toolchain rootfs
#   2. Running rustc --version
#   3. Compiling and running a hello world program
#   4. Verifying basic std library functionality
#
# Prerequisites:
#   - Built kernel image (via build-kernel.sh x86_64 dev)
#   - Rust rootfs image (via build-rust-rootfs.sh)
#
# Usage:
#   ./scripts/self-host-verify.sh [OPTIONS]
#
# Options:
#   --rootfs FILE    Rust rootfs image (default: target/rootfs-rust.img)
#   --timeout N      QEMU timeout in seconds (default: 120)
#   --help           Show this help

set -euo pipefail

# ---------------------------------------------------------------------------
# Color helpers + config
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

die() { error "$@"; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

ROOTFS="${PROJECT_ROOT}/target/rootfs-rust.img"
TIMEOUT=120
LOG="/tmp/VeridianOS/self-host-verify.log"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rootfs)  ROOTFS="$2"; shift 2 ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        --help|-h)
            head -24 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
            exit 0
            ;;
        *) die "Unknown option: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Validate
# ---------------------------------------------------------------------------
step "Validating environment"

KERNEL_IMG="${PROJECT_ROOT}/target/x86_64-veridian/debug/veridian-uefi.img"
[[ -f "$KERNEL_IMG" ]] || die "Kernel image not found: ${KERNEL_IMG}
    Run: ./build-kernel.sh x86_64 dev"

[[ -f "$ROOTFS" ]] || die "Rust rootfs not found: ${ROOTFS}
    Run: ./scripts/build-rust-rootfs.sh"

OVMF="/usr/share/edk2/x64/OVMF.4m.fd"
[[ -f "$OVMF" ]] || die "OVMF firmware not found: ${OVMF}"

mkdir -p "$(dirname "$LOG")"
success "All prerequisites found"

# ---------------------------------------------------------------------------
# Kill any existing QEMU
# ---------------------------------------------------------------------------
pkill -9 -f qemu-system-x86_64 2>/dev/null || true
sleep 2

# ---------------------------------------------------------------------------
# Boot and run self-host test
# ---------------------------------------------------------------------------
step "Booting VeridianOS with Rust toolchain (timeout: ${TIMEOUT}s)"

qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF" \
    -drive id=disk0,if=none,format=raw,file="$KERNEL_IMG" \
    -device ide-hd,drive=disk0 \
    -drive file="$ROOTFS",if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -serial stdio -display none -m 8192M \
    </dev/null > "$LOG" 2>&1 &
QEMU_PID=$!

info "QEMU PID: ${QEMU_PID}"
info "Log file: ${LOG}"

# Wait for boot + self-host test
sleep "$TIMEOUT"

# Kill QEMU
kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

# ---------------------------------------------------------------------------
# Analyze results
# ---------------------------------------------------------------------------
step "Analyzing results"

PASS=0
FAIL=0

check() {
    local pattern="$1"
    local desc="$2"
    if grep -q "$pattern" "$LOG" 2>/dev/null; then
        success "$desc"
        PASS=$((PASS + 1))
    else
        error "FAIL: $desc"
        FAIL=$((FAIL + 1))
    fi
}

check "BOOTOK" "Kernel boot"
check "rustc" "rustc available"
check "Hello from VeridianOS Rust" "Hello world compilation and execution"
check "All tests passed" "std library tests"
check "ALL SELF-HOSTING TESTS PASSED" "Full self-hosting verification"

echo ""
echo "====================================="
printf "Results: ${GREEN}%d passed${NC}, ${RED}%d failed${NC} / %d total\n" \
    "$PASS" "$FAIL" "$((PASS + FAIL))"
echo "====================================="

if [[ $FAIL -gt 0 ]]; then
    echo ""
    warn "Self-hosting verification had failures."
    warn "Check log: ${LOG}"
    echo ""
    echo "Last 30 lines of log:"
    tail -30 "$LOG"
    exit 1
else
    echo ""
    success "Rust self-hosting on VeridianOS VERIFIED!"
    exit 0
fi

#!/bin/sh
# VeridianOS -- qemu-kde.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# QEMU launch script for KDE Plasma 6 testing on VeridianOS.
#
# Launches QEMU with VirtIO GPU (virgl 3D acceleration), 2+ GB RAM,
# SMP 4 cores, and the KDE rootfs image.
#
# Usage:
#   ./qemu-kde.sh [options]
#
# Options:
#   --software     Use llvmpipe software rendering (no virgl)
#   --gdb          Enable GDB server on :1234 (start paused)
#   --serial-only  Disable graphical display (serial console only)
#   --ram <MB>     Override RAM size (default: 2048)
#   --cpus <N>     Override CPU count (default: 4)
#   --rootfs <img> Override KDE rootfs image path

set -e

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

KERNEL_IMAGE="${PROJECT_ROOT}/target/x86_64-veridian/debug/veridian-uefi.img"
KDE_ROOTFS="${PROJECT_ROOT}/target/rootfs-kde.img"
OVMF_FW="/usr/share/edk2/x64/OVMF.4m.fd"

RAM_MB=2048
CPUS=4
USE_VIRGL=true
GDB_ENABLE=false
SERIAL_ONLY=false

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --software)
            USE_VIRGL=false
            shift
            ;;
        --gdb)
            GDB_ENABLE=true
            shift
            ;;
        --serial-only)
            SERIAL_ONLY=true
            shift
            ;;
        --ram)
            RAM_MB="$2"
            shift 2
            ;;
        --cpus)
            CPUS="$2"
            shift 2
            ;;
        --rootfs)
            KDE_ROOTFS="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--software] [--gdb] [--serial-only]"
            echo "         [--ram MB] [--cpus N] [--rootfs image]"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# =========================================================================
# Validation
# =========================================================================

if [ ! -f "${KERNEL_IMAGE}" ]; then
    echo "ERROR: Kernel image not found: ${KERNEL_IMAGE}"
    echo "  Run: ./build-kernel.sh x86_64 dev"
    exit 1
fi

if [ ! -f "${KDE_ROOTFS}" ]; then
    echo "ERROR: KDE rootfs not found: ${KDE_ROOTFS}"
    echo "  Run: ./userland/integration/build-kde-rootfs.sh"
    exit 1
fi

if [ ! -f "${OVMF_FW}" ]; then
    echo "ERROR: OVMF firmware not found: ${OVMF_FW}"
    echo "  Install: edk2-ovmf package"
    exit 1
fi

# =========================================================================
# Build QEMU command
# =========================================================================

echo "========================================"
echo "  VeridianOS KDE Plasma QEMU Launcher"
echo "========================================"
echo "  Kernel:  ${KERNEL_IMAGE}"
echo "  Rootfs:  ${KDE_ROOTFS}"
echo "  RAM:     ${RAM_MB} MB"
echo "  CPUs:    ${CPUS}"
echo "  GPU:     $([ "${USE_VIRGL}" = true ] && echo 'VirtIO virgl (3D)' || echo 'VirtIO (llvmpipe)')"
echo "  GDB:     $([ "${GDB_ENABLE}" = true ] && echo 'enabled (:1234)' || echo 'disabled')"
echo "  Display: $([ "${SERIAL_ONLY}" = true ] && echo 'serial only' || echo 'graphical')"
echo "========================================"
echo ""

# Base QEMU arguments
QEMU_ARGS=""
QEMU_ARGS="${QEMU_ARGS} -enable-kvm"
QEMU_ARGS="${QEMU_ARGS} -m ${RAM_MB}M"
QEMU_ARGS="${QEMU_ARGS} -smp ${CPUS}"
QEMU_ARGS="${QEMU_ARGS} -cpu host"

# UEFI firmware
QEMU_ARGS="${QEMU_ARGS} -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}"

# Kernel boot disk
QEMU_ARGS="${QEMU_ARGS} -drive id=disk0,if=none,format=raw,file=${KERNEL_IMAGE}"
QEMU_ARGS="${QEMU_ARGS} -device ide-hd,drive=disk0"

# KDE rootfs as VirtIO block device
QEMU_ARGS="${QEMU_ARGS} -drive id=vd0,if=none,format=raw,file=${KDE_ROOTFS}"
QEMU_ARGS="${QEMU_ARGS} -device virtio-blk-pci,drive=vd0"

# GPU
if [ "${USE_VIRGL}" = true ]; then
    QEMU_ARGS="${QEMU_ARGS} -device virtio-gpu-gl-pci,id=gpu0"
else
    QEMU_ARGS="${QEMU_ARGS} -device virtio-gpu-pci,id=gpu0"
fi

# Display
if [ "${SERIAL_ONLY}" = true ]; then
    QEMU_ARGS="${QEMU_ARGS} -display none"
else
    if [ "${USE_VIRGL}" = true ]; then
        QEMU_ARGS="${QEMU_ARGS} -display gtk,gl=on"
    else
        QEMU_ARGS="${QEMU_ARGS} -display gtk"
    fi
fi

# Serial console
QEMU_ARGS="${QEMU_ARGS} -serial stdio"

# Input devices
QEMU_ARGS="${QEMU_ARGS} -device virtio-keyboard-pci"
QEMU_ARGS="${QEMU_ARGS} -device virtio-mouse-pci"

# Network (for future use)
QEMU_ARGS="${QEMU_ARGS} -nic user,model=virtio-net-pci"

# QMP monitor (for screenshots)
QEMU_ARGS="${QEMU_ARGS} -qmp unix:/tmp/qmp-kde.sock,server,nowait"

# GDB server
if [ "${GDB_ENABLE}" = true ]; then
    QEMU_ARGS="${QEMU_ARGS} -s -S"
    echo "  GDB server listening on :1234 (paused)"
    echo "  Connect with: gdb-multiarch -ex 'target remote :1234'"
fi

# Debug exit device
QEMU_ARGS="${QEMU_ARGS} -device isa-debug-exit,iobase=0xf4,iosize=0x04"

# =========================================================================
# Launch
# =========================================================================

echo "Launching QEMU..."
echo "  Command: qemu-system-x86_64 ${QEMU_ARGS}"
echo ""

# shellcheck disable=SC2086
exec qemu-system-x86_64 ${QEMU_ARGS}

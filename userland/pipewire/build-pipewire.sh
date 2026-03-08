#!/bin/sh
# VeridianOS -- build-pipewire.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compilation build script for the PipeWire audio daemon and
# PulseAudio compatibility layer targeting VeridianOS.
#
# Prerequisites:
#   1. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   2. VeridianOS libc in sysroot
#   3. D-Bus headers in sysroot (Sprint 9.5)
#
# Usage:
#   ./build-pipewire.sh
#
# Environment variables:
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   BUILD_TYPE       - Release or Debug (default: Release)
#   JOBS             - Parallel build jobs (default: $(nproc))

set -e

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
JOBS="${JOBS:-$(nproc)}"
BUILD_DIR="${SCRIPT_DIR}/build"

CC="${CC:-x86_64-veridian-gcc}"
CXX="${CXX:-x86_64-veridian-g++}"

CFLAGS_COMMON="-I${VERIDIAN_SYSROOT}/usr/include --sysroot=${VERIDIAN_SYSROOT}"
LDFLAGS_COMMON="-L${VERIDIAN_SYSROOT}/usr/lib --sysroot=${VERIDIAN_SYSROOT}"

if [ "${BUILD_TYPE}" = "Debug" ]; then
    CFLAGS_COMMON="${CFLAGS_COMMON} -g -O0 -DDEBUG"
else
    CFLAGS_COMMON="${CFLAGS_COMMON} -O2 -DNDEBUG"
fi

echo "========================================"
echo "  PipeWire Build for VeridianOS"
echo "========================================"
echo "  Source:     ${SCRIPT_DIR}"
echo "  Build dir:  ${BUILD_DIR}"
echo "  Sysroot:    ${VERIDIAN_SYSROOT}"
echo "  CC:         ${CC}"
echo "  CXX:        ${CXX}"
echo "  Build type: ${BUILD_TYPE}"
echo "========================================"

mkdir -p "${BUILD_DIR}"

# =========================================================================
# Step 1: Build ALSA bridge (C++)
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 1: Building ALSA bridge"
echo "================================================================"

${CXX} ${CFLAGS_COMMON} -Wall -Wextra -std=c++17 \
    -c "${SCRIPT_DIR}/pw-alsa-bridge.cpp" \
    -o "${BUILD_DIR}/pw-alsa-bridge.o"

echo "  Built pw-alsa-bridge.o"

# =========================================================================
# Step 2: Build PipeWire daemon library (C++)
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 2: Building PipeWire daemon"
echo "================================================================"

${CXX} ${CFLAGS_COMMON} -Wall -Wextra -std=c++17 \
    -c "${SCRIPT_DIR}/pipewire-veridian.cpp" \
    -o "${BUILD_DIR}/pipewire-veridian.o"

echo "  Built pipewire-veridian.o"

# =========================================================================
# Step 3: Build PulseAudio compatibility layer (C++)
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 3: Building PulseAudio compatibility layer"
echo "================================================================"

${CXX} ${CFLAGS_COMMON} -Wall -Wextra -std=c++17 \
    -c "${SCRIPT_DIR}/pulseaudio-compat.cpp" \
    -o "${BUILD_DIR}/pulseaudio-compat.o"

echo "  Built pulseaudio-compat.o"

# =========================================================================
# Step 4: Create static libraries
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 4: Creating libraries"
echo "================================================================"

ar rcs "${BUILD_DIR}/libpipewire-veridian.a" \
    "${BUILD_DIR}/pipewire-veridian.o" \
    "${BUILD_DIR}/pw-alsa-bridge.o"

ar rcs "${BUILD_DIR}/libpulse-veridian.a" \
    "${BUILD_DIR}/pulseaudio-compat.o"

echo "  Created libpipewire-veridian.a"
echo "  Created libpulse-veridian.a"

# =========================================================================
# Step 5: Build standalone daemon (optional)
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Building standalone PipeWire daemon"
echo "================================================================"

${CXX} ${CFLAGS_COMMON} ${LDFLAGS_COMMON} -Wall -Wextra -std=c++17 \
    -DPW_STANDALONE_DAEMON \
    "${SCRIPT_DIR}/pipewire-veridian.cpp" \
    "${BUILD_DIR}/pw-alsa-bridge.o" \
    -o "${BUILD_DIR}/pipewire-daemon"

echo "  Built pipewire-daemon"

# =========================================================================
# Step 6: Install to sysroot
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 6: Installing to sysroot"
echo "================================================================"

INSTALL_LIB="${VERIDIAN_SYSROOT}/usr/lib"
INSTALL_INC="${VERIDIAN_SYSROOT}/usr/include/pipewire"
INSTALL_BIN="${VERIDIAN_SYSROOT}/usr/bin"

mkdir -p "${INSTALL_LIB}" "${INSTALL_INC}" "${INSTALL_BIN}"

cp "${BUILD_DIR}/libpipewire-veridian.a" "${INSTALL_LIB}/"
cp "${BUILD_DIR}/libpulse-veridian.a"    "${INSTALL_LIB}/"
cp "${BUILD_DIR}/pipewire-daemon"        "${INSTALL_BIN}/"

cp "${SCRIPT_DIR}/pipewire-veridian.h"   "${INSTALL_INC}/"
cp "${SCRIPT_DIR}/pw-alsa-bridge.h"      "${INSTALL_INC}/"
cp "${SCRIPT_DIR}/pulseaudio-compat.h"   "${INSTALL_INC}/"

echo "  Installed libraries to ${INSTALL_LIB}"
echo "  Installed headers to ${INSTALL_INC}"
echo "  Installed daemon to ${INSTALL_BIN}"

echo ""
echo "========================================"
echo "  PipeWire build complete"
echo "========================================"

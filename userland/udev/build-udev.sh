#!/bin/sh
# VeridianOS -- build-udev.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compilation build script for the udev shim.
#
# Compiles the udev daemon and libudev shim as shared libraries
# for the VeridianOS sysroot.
#
# Prerequisites:
#   1. VeridianOS sysroot with Qt 6, D-Bus
#   2. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   3. pkg-config configured for sysroot
#
# Usage:
#   ./build-udev.sh [source-dir]
#
# Environment:
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   BUILD_TYPE       - release or debug (default: release)
#   JOBS             - Parallel build jobs (default: $(nproc))

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
BUILD_TYPE="${BUILD_TYPE:-release}"
JOBS="${JOBS:-$(nproc)}"

SOURCE_DIR="${1:-${SCRIPT_DIR}}"
BUILD_DIR="${SCRIPT_DIR}/build"

CXX="${CXX:-x86_64-veridian-g++}"
CC="${CC:-x86_64-veridian-gcc}"
PKG_CONFIG="${PKG_CONFIG:-x86_64-veridian-pkg-config}"

echo "========================================"
echo "  udev Shim Build for VeridianOS"
echo "========================================"
echo "  Source:     ${SOURCE_DIR}"
echo "  Build:      ${BUILD_DIR}"
echo "  Sysroot:    ${VERIDIAN_SYSROOT}"
echo "  Build type: ${BUILD_TYPE}"
echo "  Compiler:   ${CXX}"
echo "  Jobs:       ${JOBS}"
echo "========================================"

# =========================================================================
# Step 1: Set up build directory
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 1: Setting up build directory"
echo "================================================================"

mkdir -p "${BUILD_DIR}"

# =========================================================================
# Step 2: Determine compiler flags
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 2: Configuring compiler flags"
echo "================================================================"

CXXFLAGS="--sysroot=${VERIDIAN_SYSROOT} -I${VERIDIAN_SYSROOT}/usr/include"
CXXFLAGS="${CXXFLAGS} -fPIC -std=c++17"

if [ "${BUILD_TYPE}" = "debug" ]; then
    CXXFLAGS="${CXXFLAGS} -g -O0 -DDEBUG"
else
    CXXFLAGS="${CXXFLAGS} -O2 -DNDEBUG"
fi

LDFLAGS="--sysroot=${VERIDIAN_SYSROOT} -L${VERIDIAN_SYSROOT}/usr/lib"

# Qt 6 flags
QT_CFLAGS="$(${PKG_CONFIG} --cflags Qt6Core Qt6DBus 2>/dev/null || echo "-I${VERIDIAN_SYSROOT}/usr/include/qt6")"
QT_LIBS="$(${PKG_CONFIG} --libs Qt6Core Qt6DBus 2>/dev/null || echo "-lQt6Core -lQt6DBus")"

echo "  CXXFLAGS: ${CXXFLAGS}"
echo "  LDFLAGS:  ${LDFLAGS}"
echo "  Qt flags: ${QT_CFLAGS}"

# =========================================================================
# Step 3: Compile udev daemon
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 3: Compiling udev daemon"
echo "================================================================"

${CXX} ${CXXFLAGS} ${QT_CFLAGS} -c "${SOURCE_DIR}/udev-veridian.cpp" \
    -o "${BUILD_DIR}/udev-veridian.o"
echo "  Compiled udev-veridian.cpp"

# =========================================================================
# Step 4: Compile libudev shim
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 4: Compiling libudev shim"
echo "================================================================"

${CXX} ${CXXFLAGS} ${QT_CFLAGS} -c "${SOURCE_DIR}/libudev-veridian.cpp" \
    -o "${BUILD_DIR}/libudev-veridian.o"
echo "  Compiled libudev-veridian.cpp"

# =========================================================================
# Step 5: Link shared libraries
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Linking shared libraries"
echo "================================================================"

# udev daemon library
${CXX} -shared -o "${BUILD_DIR}/libudev-daemon-veridian.so" \
    "${BUILD_DIR}/udev-veridian.o" ${LDFLAGS} ${QT_LIBS} -ldbus-1
echo "  Library: ${BUILD_DIR}/libudev-daemon-veridian.so"

# libudev shim (links against daemon lib)
${CXX} -shared -o "${BUILD_DIR}/libudev-veridian.so" \
    "${BUILD_DIR}/libudev-veridian.o" "${BUILD_DIR}/udev-veridian.o" \
    ${LDFLAGS} ${QT_LIBS} -ldbus-1
echo "  Library: ${BUILD_DIR}/libudev-veridian.so"

# =========================================================================
# Step 6: Install to sysroot
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 6: Installing to sysroot"
echo "================================================================"

# Libraries
install -d "${VERIDIAN_SYSROOT}/usr/lib"
install -m 644 "${BUILD_DIR}/libudev-daemon-veridian.so" "${VERIDIAN_SYSROOT}/usr/lib/"
install -m 644 "${BUILD_DIR}/libudev-veridian.so" "${VERIDIAN_SYSROOT}/usr/lib/"

# Symlink for libudev compatibility
ln -sf libudev-veridian.so "${VERIDIAN_SYSROOT}/usr/lib/libudev.so"
ln -sf libudev-veridian.so "${VERIDIAN_SYSROOT}/usr/lib/libudev.so.1"

# Headers
install -d "${VERIDIAN_SYSROOT}/usr/include"
install -m 644 "${SOURCE_DIR}/libudev-veridian.h" "${VERIDIAN_SYSROOT}/usr/include/"
install -m 644 "${SOURCE_DIR}/udev-veridian.h" "${VERIDIAN_SYSROOT}/usr/include/"

# Symlink for libudev.h compatibility
ln -sf libudev-veridian.h "${VERIDIAN_SYSROOT}/usr/include/libudev.h"

# udev rules directory
install -d "${VERIDIAN_SYSROOT}/etc/udev/rules.d"

# D-Bus service file
install -d "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services"
cat > "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services/org.freedesktop.UDev.service" << EOF
[D-BUS Service]
Name=org.freedesktop.UDev
Exec=/usr/lib/udev-veridian-daemon
User=root
EOF

echo "  Installed libraries, headers, and D-Bus service"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  udev Shim Build Complete"
echo "========================================"
echo ""
echo "  Libraries:"
echo "    ${VERIDIAN_SYSROOT}/usr/lib/libudev-daemon-veridian.so"
echo "    ${VERIDIAN_SYSROOT}/usr/lib/libudev-veridian.so"
echo "    ${VERIDIAN_SYSROOT}/usr/lib/libudev.so -> libudev-veridian.so"
echo ""
echo "  Headers:"
echo "    ${VERIDIAN_SYSROOT}/usr/include/udev-veridian.h"
echo "    ${VERIDIAN_SYSROOT}/usr/include/libudev-veridian.h"
echo "    ${VERIDIAN_SYSROOT}/usr/include/libudev.h -> libudev-veridian.h"
echo ""
echo "  The shim provides:"
echo "    - udev daemon (device monitoring, rule engine, D-Bus)"
echo "    - libudev API (for PipeWire, libinput, Mesa, etc.)"
echo "    - Solid integration via D-Bus DeviceAdded/DeviceRemoved"
echo ""
echo "  Known limitations:"
echo "    - Rule actions (RUN, SYMLINK) are stubs"
echo "    - devtype filtering not yet implemented"
echo "    - Parent device tracking simplified"

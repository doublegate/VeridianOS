#!/bin/sh
# VeridianOS -- build-nm.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compilation build script for the NetworkManager shim.
#
# Compiles the NM daemon, Wi-Fi backend, Ethernet backend, and DNS
# backend as a single shared library (libnm-veridian.so) plus the
# nm-veridian daemon binary.
#
# Prerequisites:
#   1. VeridianOS sysroot with Qt 6, D-Bus
#   2. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   3. pkg-config configured for sysroot
#
# Usage:
#   ./build-nm.sh [source-dir]
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
echo "  NetworkManager Shim Build for VeridianOS"
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
# Step 3: Compile object files
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 3: Compiling source files"
echo "================================================================"

SOURCES="nm-veridian.cpp nm-wifi.cpp nm-ethernet.cpp nm-dns.cpp"

for src in ${SOURCES}; do
    obj="${BUILD_DIR}/$(echo ${src} | sed 's/\.cpp$/.o/')"
    echo "  Compiling ${src}..."
    ${CXX} ${CXXFLAGS} ${QT_CFLAGS} -c "${SOURCE_DIR}/${src}" -o "${obj}"
done

echo "  All objects compiled"

# =========================================================================
# Step 4: Link shared library
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 4: Linking libnm-veridian.so"
echo "================================================================"

OBJECTS=""
for src in ${SOURCES}; do
    OBJECTS="${OBJECTS} ${BUILD_DIR}/$(echo ${src} | sed 's/\.cpp$/.o/')"
done

${CXX} -shared -o "${BUILD_DIR}/libnm-veridian.so" \
    ${OBJECTS} ${LDFLAGS} ${QT_LIBS} -ldbus-1

echo "  Library: ${BUILD_DIR}/libnm-veridian.so"

# =========================================================================
# Step 5: Install to sysroot
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Installing to sysroot"
echo "================================================================"

# Library
install -d "${VERIDIAN_SYSROOT}/usr/lib"
install -m 644 "${BUILD_DIR}/libnm-veridian.so" "${VERIDIAN_SYSROOT}/usr/lib/"

# Headers
install -d "${VERIDIAN_SYSROOT}/usr/include/networkmanager"
install -m 644 "${SOURCE_DIR}/nm-veridian.h" "${VERIDIAN_SYSROOT}/usr/include/networkmanager/"
install -m 644 "${SOURCE_DIR}/nm-wifi.h" "${VERIDIAN_SYSROOT}/usr/include/networkmanager/"
install -m 644 "${SOURCE_DIR}/nm-ethernet.h" "${VERIDIAN_SYSROOT}/usr/include/networkmanager/"
install -m 644 "${SOURCE_DIR}/nm-dns.h" "${VERIDIAN_SYSROOT}/usr/include/networkmanager/"

# Connection profile directory
install -d "${VERIDIAN_SYSROOT}/etc/veridian/connections"

# D-Bus service file
install -d "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services"
cat > "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services/org.freedesktop.NetworkManager.service" << EOF
[D-BUS Service]
Name=org.freedesktop.NetworkManager
Exec=/usr/lib/nm-veridian-daemon
User=root
SystemdService=NetworkManager.service
EOF

echo "  Installed library, headers, and D-Bus service file"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  NetworkManager Shim Build Complete"
echo "========================================"
echo ""
echo "  Library:  ${VERIDIAN_SYSROOT}/usr/lib/libnm-veridian.so"
echo "  Headers:  ${VERIDIAN_SYSROOT}/usr/include/networkmanager/"
echo "  Profiles: ${VERIDIAN_SYSROOT}/etc/veridian/connections/"
echo ""
echo "  The shim provides org.freedesktop.NetworkManager on D-Bus"
echo "  for Plasma's network management applet (plasma-nm)."
echo ""
echo "  Known limitations:"
echo "    - VPN not yet implemented"
echo "    - IPv6 SLAAC not yet wired"
echo "    - WPA3-SAE handshake delegated to kernel (stub)"
echo "    - No captive portal detection"

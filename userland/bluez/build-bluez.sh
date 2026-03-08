#!/bin/sh
# VeridianOS -- build-bluez.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Cross-compilation build script for the BlueZ shim.
#
# Compiles the BlueZ daemon, HCI bridge, and pairing agent as a single
# shared library (libbluez-veridian.so) for the VeridianOS sysroot.
#
# Prerequisites:
#   1. VeridianOS sysroot with Qt 6, D-Bus
#   2. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   3. pkg-config configured for sysroot
#
# Usage:
#   ./build-bluez.sh [source-dir]
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
echo "  BlueZ Shim Build for VeridianOS"
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

SOURCES="bluez-veridian.cpp bluez-hci-bridge.cpp bluez-pair.cpp"

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
echo "  Step 4: Linking libbluez-veridian.so"
echo "================================================================"

OBJECTS=""
for src in ${SOURCES}; do
    OBJECTS="${OBJECTS} ${BUILD_DIR}/$(echo ${src} | sed 's/\.cpp$/.o/')"
done

${CXX} -shared -o "${BUILD_DIR}/libbluez-veridian.so" \
    ${OBJECTS} ${LDFLAGS} ${QT_LIBS} -ldbus-1

echo "  Library: ${BUILD_DIR}/libbluez-veridian.so"

# =========================================================================
# Step 5: Install to sysroot
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Installing to sysroot"
echo "================================================================"

# Library
install -d "${VERIDIAN_SYSROOT}/usr/lib"
install -m 644 "${BUILD_DIR}/libbluez-veridian.so" "${VERIDIAN_SYSROOT}/usr/lib/"

# Headers
install -d "${VERIDIAN_SYSROOT}/usr/include/bluez"
install -m 644 "${SOURCE_DIR}/bluez-veridian.h" "${VERIDIAN_SYSROOT}/usr/include/bluez/"
install -m 644 "${SOURCE_DIR}/bluez-hci-bridge.h" "${VERIDIAN_SYSROOT}/usr/include/bluez/"
install -m 644 "${SOURCE_DIR}/bluez-pair.h" "${VERIDIAN_SYSROOT}/usr/include/bluez/"

# Bluetooth key storage directory
install -d "${VERIDIAN_SYSROOT}/etc/veridian/bluetooth"

# D-Bus service file
install -d "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services"
cat > "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services/org.bluez.service" << EOF
[D-BUS Service]
Name=org.bluez
Exec=/usr/lib/bluez-veridian-daemon
User=root
SystemdService=bluetooth.service
EOF

# D-Bus configuration for BlueZ
install -d "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system.d"
cat > "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system.d/bluetooth.conf" << EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <policy user="root">
    <allow own="org.bluez"/>
    <allow send_destination="org.bluez"/>
  </policy>
  <policy context="default">
    <allow send_destination="org.bluez"/>
  </policy>
</busconfig>
EOF

echo "  Installed library, headers, and D-Bus service files"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  BlueZ Shim Build Complete"
echo "========================================"
echo ""
echo "  Library:  ${VERIDIAN_SYSROOT}/usr/lib/libbluez-veridian.so"
echo "  Headers:  ${VERIDIAN_SYSROOT}/usr/include/bluez/"
echo "  Keys:     ${VERIDIAN_SYSROOT}/etc/veridian/bluetooth/"
echo ""
echo "  The shim provides org.bluez on D-Bus for Plasma's"
echo "  Bluetooth applet (Bluedevil) and other BlueZ clients."
echo ""
echo "  Known limitations:"
echo "    - BLE GATT application registration is a stub"
echo "    - Audio profiles (A2DP/HFP) not yet implemented"
echo "    - LE scanning uses classic inquiry (no LE-specific scan)"
echo "    - Connection handle tracking simplified"

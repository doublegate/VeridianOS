#!/bin/sh
# VeridianOS -- build-xwayland.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for XWayland server targeting VeridianOS.
#
# Clones the xserver source, configures with --enable-xwayland only
# (no Xorg, Xnest, Xvfb), and cross-compiles for the VeridianOS
# sysroot.
#
# XWayland provides X11 application compatibility under KWin.  Most
# KDE applications are Wayland-native, but legacy X11 apps (xterm,
# Firefox older builds, etc.) require XWayland.
#
# Prerequisites:
#   1. VeridianOS sysroot with: libdrm, libgbm, EGL, GLES2, pixman,
#      xkbcommon, libepoxy, dbus-1
#   2. X11 protocol headers (xcb, xproto) in sysroot
#   3. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   4. meson + ninja (host)
#   5. git
#
# Usage:
#   ./build-xwayland.sh [source-dir]
#
# Environment:
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   BUILD_TYPE       - release or debug (default: release)
#   JOBS             - Parallel build jobs (default: $(nproc))
#   XWAYLAND_VERSION - Git tag (default: xwayland-24.1.0)
#   XWAYLAND_GIT_URL - Git repo URL

set -e

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
BUILD_TYPE="${BUILD_TYPE:-release}"
JOBS="${JOBS:-$(nproc)}"
XWAYLAND_VERSION="${XWAYLAND_VERSION:-xwayland-24.1.0}"
XWAYLAND_GIT_URL="${XWAYLAND_GIT_URL:-https://gitlab.freedesktop.org/xorg/xserver.git}"

XWAYLAND_SOURCE="${1:-${SCRIPT_DIR}/xwayland-src}"
BUILD_DIR="${SCRIPT_DIR}/xwayland-build"
CROSS_FILE="${SCRIPT_DIR}/xwayland-veridian-cross.ini"

echo "========================================"
echo "  XWayland Build for VeridianOS"
echo "========================================"
echo "  Version:   ${XWAYLAND_VERSION}"
echo "  Source:     ${XWAYLAND_SOURCE}"
echo "  Build:      ${BUILD_DIR}"
echo "  Sysroot:    ${VERIDIAN_SYSROOT}"
echo "  Build type: ${BUILD_TYPE}"
echo "  Jobs:       ${JOBS}"
echo "========================================"

# =========================================================================
# Step 1: Generate Meson cross file
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 1: Generating Meson cross-compilation file"
echo "================================================================"

cat > "${CROSS_FILE}" << CROSSEOF
[binaries]
c = 'x86_64-veridian-gcc'
cpp = 'x86_64-veridian-g++'
ar = 'x86_64-veridian-ar'
strip = 'x86_64-veridian-strip'
pkgconfig = 'x86_64-veridian-pkg-config'

[properties]
sys_root = '${VERIDIAN_SYSROOT}'
pkg_config_libdir = '${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig'

[host_machine]
system = 'veridian'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
CROSSEOF

echo "  Cross file: ${CROSS_FILE}"

# =========================================================================
# Step 2: Clone or update source
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 2: Obtaining XWayland source"
echo "================================================================"

if [ -d "${XWAYLAND_SOURCE}/.git" ]; then
    echo "  Source tree exists, updating..."
    cd "${XWAYLAND_SOURCE}"
    git fetch origin
    git checkout "${XWAYLAND_VERSION}" 2>/dev/null || \
        git checkout -b "veridian-${XWAYLAND_VERSION}" "${XWAYLAND_VERSION}"
    cd "${SCRIPT_DIR}"
else
    echo "  Cloning xserver ${XWAYLAND_VERSION}..."
    git clone --depth=1 --branch="${XWAYLAND_VERSION}" \
        "${XWAYLAND_GIT_URL}" "${XWAYLAND_SOURCE}"
fi

echo "  Source ready at ${XWAYLAND_SOURCE}"

# =========================================================================
# Step 3: Configure with Meson
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 3: Meson configure"
echo "================================================================"

# Remove stale build directory
if [ -d "${BUILD_DIR}" ]; then
    rm -rf "${BUILD_DIR}"
fi

meson setup "${BUILD_DIR}" "${XWAYLAND_SOURCE}" \
    --cross-file="${CROSS_FILE}" \
    --prefix=/usr \
    --buildtype="${BUILD_TYPE}" \
    -Dxwayland=true \
    -Dxorg=false \
    -Dxnest=false \
    -Dxvfb=false \
    -Dxephyr=false \
    -Ddmx=false \
    -Dxwin=false \
    -Ddri3=true \
    -Dglamor=true \
    -Dglx=false \
    -Dxkb_dir="${VERIDIAN_SYSROOT}/usr/share/X11/xkb" \
    -Ddefault_font_path="${VERIDIAN_SYSROOT}/usr/share/fonts" \
    -Dbuiltin_fonts=false \
    -Dsecure-rpc=false \
    -Dipv6=false \
    -Dinput_thread=false \
    -Ddocs=false \
    -Ddevel-docs=false

echo "  Meson configure complete"

# =========================================================================
# Step 4: Build
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 4: Building XWayland"
echo "================================================================"

ninja -C "${BUILD_DIR}" -j "${JOBS}"

echo "  Build complete"

# =========================================================================
# Step 5: Install to sysroot
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Installing to sysroot"
echo "================================================================"

DESTDIR="${VERIDIAN_SYSROOT}" ninja -C "${BUILD_DIR}" install

# Verify the binary was installed
if [ -f "${VERIDIAN_SYSROOT}/usr/bin/Xwayland" ]; then
    echo "  Xwayland installed to ${VERIDIAN_SYSROOT}/usr/bin/Xwayland"
else
    echo "  WARNING: Xwayland binary not found after install"
fi

# =========================================================================
# Step 6: Install X11 support files
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 6: Installing X11 support files"
echo "================================================================"

# Create X11 socket directory in sysroot
mkdir -p "${VERIDIAN_SYSROOT}/tmp/.X11-unix"
chmod 01777 "${VERIDIAN_SYSROOT}/tmp/.X11-unix"

# Install minimal xkb data if not present
if [ ! -d "${VERIDIAN_SYSROOT}/usr/share/X11/xkb" ]; then
    mkdir -p "${VERIDIAN_SYSROOT}/usr/share/X11/xkb"
    echo "  NOTE: xkb data not found -- keyboard layouts may not work"
    echo "  Install xkeyboard-config package to ${VERIDIAN_SYSROOT}/usr/share/X11/xkb"
fi

echo "  X11 support files installed"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  XWayland Build Complete"
echo "========================================"
echo ""
echo "  Binary:  ${VERIDIAN_SYSROOT}/usr/bin/Xwayland"
echo "  Version: ${XWAYLAND_VERSION}"
echo ""
echo "  XWayland is launched by KWin on demand when an X11"
echo "  application is started.  No manual configuration needed."
echo ""
echo "  To test manually:"
echo "    Xwayland :0 -rootless -wm <fd> -listenfd <fd>"
echo ""
echo "  Known limitations:"
echo "    - No GLX (use EGL/GLES2 via glamor)"
echo "    - No Xorg DDX (Wayland-only)"
echo "    - Input methods not fully wired"

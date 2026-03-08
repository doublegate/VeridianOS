#!/bin/sh
# VeridianOS -- qt6-configure.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Configuration script for building Qt 6 for VeridianOS.
#
# Prerequisites:
#   1. A native Linux Qt 6 build (host tools: moc, rcc, uic, qsb, qlalr)
#   2. VeridianOS cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   3. VeridianOS sysroot with installed libraries:
#      - libc, libstdc++, libpthread
#      - Mesa (libEGL, libGLESv2, libgbm)
#      - FreeType, HarfBuzz, Fontconfig
#      - libwayland-client, libwayland-server, libwayland-cursor
#      - libxkbcommon
#      - libdbus-1
#      - OpenSSL (libssl, libcrypto)
#      - zlib, libpng, libjpeg
#
# Usage:
#   cd /path/to/qt6-build
#   /path/to/qt6-configure.sh /path/to/qt6-source
#
# Environment variables:
#   QT_HOST_PATH    - Path to native Qt 6 installation (required)
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   BUILD_TYPE       - Release or Debug (default: Release)

set -e

# -------------------------------------------------------------------------
# Configuration
# -------------------------------------------------------------------------

QT_SOURCE="${1:?Usage: $0 <qt6-source-dir>}"
QT_HOST_PATH="${QT_HOST_PATH:?Set QT_HOST_PATH to your native Qt 6 install}"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TOOLCHAIN_FILE="${SCRIPT_DIR}/qt6-toolchain.cmake"

echo "========================================"
echo "  Qt 6 Configuration for VeridianOS"
echo "========================================"
echo "  Qt source:     ${QT_SOURCE}"
echo "  Host Qt path:  ${QT_HOST_PATH}"
echo "  Sysroot:       ${VERIDIAN_SYSROOT}"
echo "  Build type:    ${BUILD_TYPE}"
echo "  Toolchain:     ${TOOLCHAIN_FILE}"
echo "========================================"

# -------------------------------------------------------------------------
# Step 1: Configure Qt 6 with CMake
# -------------------------------------------------------------------------
# This is the cmake invocation that configures Qt 6 for cross-compilation
# targeting VeridianOS.

cmake \
    -G Ninja \
    -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
    -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN_FILE}" \
    -DCMAKE_INSTALL_PREFIX="${VERIDIAN_SYSROOT}/usr" \
    -DQT_HOST_PATH="${QT_HOST_PATH}" \
    \
    -DBUILD_SHARED_LIBS=ON \
    \
    `# ---- Modules to build ----` \
    -DBUILD_qtbase=ON \
    -DBUILD_qtdeclarative=ON \
    -DBUILD_qtshadertools=ON \
    -DBUILD_qtwayland=ON \
    -DBUILD_qtsvg=ON \
    -DBUILD_qt5compat=ON \
    \
    `# ---- Modules to skip (not available on VeridianOS) ----` \
    -DBUILD_qtx11extras=OFF \
    -DBUILD_qtmultimedia=OFF \
    -DBUILD_qtwebengine=OFF \
    -DBUILD_qtwebview=OFF \
    -DBUILD_qtpositioning=OFF \
    -DBUILD_qtlocation=OFF \
    -DBUILD_qtsensors=OFF \
    -DBUILD_qtserialport=OFF \
    -DBUILD_qtserialbus=OFF \
    -DBUILD_qtconnectivity=OFF \
    \
    `# ---- QtCore features ----` \
    -DFEATURE_thread=ON \
    -DFEATURE_future=ON \
    -DFEATURE_concurrent=ON \
    -DFEATURE_dbus=ON \
    -DFEATURE_network=ON \
    -DFEATURE_sql=OFF \
    -DFEATURE_xml=ON \
    -DFEATURE_regularexpression=ON \
    -DFEATURE_iconv=ON \
    -DFEATURE_icu=OFF \
    \
    `# ---- QtGui features ----` \
    -DFEATURE_opengl=ON \
    -DFEATURE_opengl_es2=ON \
    -DFEATURE_opengles2=ON \
    -DFEATURE_egl=ON \
    -DFEATURE_freetype=ON \
    -DFEATURE_fontconfig=ON \
    -DFEATURE_harfbuzz=ON \
    -DFEATURE_xkbcommon=ON \
    -DFEATURE_wayland=ON \
    -DFEATURE_png=ON \
    -DFEATURE_jpeg=ON \
    \
    `# ---- Features NOT available on VeridianOS ----` \
    -DFEATURE_xcb=OFF \
    -DFEATURE_xcb_xlib=OFF \
    -DFEATURE_xlib=OFF \
    -DFEATURE_xkb=OFF \
    -DFEATURE_glx=OFF \
    -DFEATURE_vulkan=OFF \
    -DFEATURE_linuxfb=OFF \
    -DFEATURE_directfb=OFF \
    -DFEATURE_kms=ON \
    -DFEATURE_drm_atomic=ON \
    -DFEATURE_gbm=ON \
    \
    `# ---- QtNetwork features ----` \
    -DFEATURE_ssl=ON \
    -DFEATURE_openssl=ON \
    -DFEATURE_opensslv30=ON \
    \
    `# ---- QtQml / QtQuick features ----` \
    -DFEATURE_qml_jit=OFF \
    -DFEATURE_qml_interpreter=ON \
    -DFEATURE_qml_network=ON \
    -DFEATURE_quick_shadereffect=ON \
    \
    `# ---- QtWayland features ----` \
    -DFEATURE_wayland_client=ON \
    -DFEATURE_wayland_server=OFF \
    -DFEATURE_xdg_shell=ON \
    \
    `# ---- Platform plugin ----` \
    -DQT_QPA_DEFAULT_PLATFORM=veridian \
    \
    "${QT_SOURCE}"

echo ""
echo "Configuration complete.  Build with:"
echo "  cmake --build . --parallel"
echo ""
echo "Install with:"
echo "  cmake --install . --prefix ${VERIDIAN_SYSROOT}/usr"

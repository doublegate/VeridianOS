#!/bin/sh
# VeridianOS -- build-tier2.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for KDE Frameworks 6 Tier 2 libraries.
#
# Tier 2 frameworks depend on Tier 1 frameworks (and Qt 6).
# All Tier 1 frameworks must be installed in the sysroot before
# running this script.
#
# Prerequisites:
#   1. Tier 1 frameworks installed (build-tier1.sh)
#   2. Qt 6 installed in sysroot (Sprint 9.6)
#   3. ECM installed in sysroot
#   4. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#
# Usage:
#   cd /path/to/build-dir
#   /path/to/build-tier2.sh /path/to/kf6-source-root
#
# Environment variables:
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   QT_HOST_PATH     - Path to native Qt 6 installation (required)
#   BUILD_TYPE        - Release or Debug (default: Release)
#   JOBS              - Parallel build jobs (default: $(nproc))

set -e

# =========================================================================
# Configuration
# =========================================================================

KF6_SOURCE="${1:?Usage: $0 <kf6-source-root>}"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
QT_HOST_PATH="${QT_HOST_PATH:?Set QT_HOST_PATH to your native Qt 6 install}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
JOBS="${JOBS:-$(nproc)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TOOLCHAIN_FILE="${SCRIPT_DIR}/../qt6/qt6-toolchain.cmake"
ECM_DIR="${VERIDIAN_SYSROOT}/usr/share/ECM/cmake"

echo "========================================"
echo "  KDE Frameworks 6 -- Tier 2 Build"
echo "========================================"
echo "  KF6 source:    ${KF6_SOURCE}"
echo "  Sysroot:       ${VERIDIAN_SYSROOT}"
echo "  Host Qt path:  ${QT_HOST_PATH}"
echo "  Build type:    ${BUILD_TYPE}"
echo "  Jobs:          ${JOBS}"
echo "========================================"

# Common cmake arguments
CMAKE_COMMON_ARGS="\
    -G Ninja \
    -DCMAKE_BUILD_TYPE=${BUILD_TYPE} \
    -DCMAKE_TOOLCHAIN_FILE=${TOOLCHAIN_FILE} \
    -DCMAKE_INSTALL_PREFIX=${VERIDIAN_SYSROOT}/usr \
    -DQT_HOST_PATH=${QT_HOST_PATH} \
    -DECM_DIR=${ECM_DIR} \
    -DBUILD_TESTING=OFF \
    -DBUILD_QCH=OFF \
    -DQT_MAJOR_VERSION=6 \
    -DKDE_INSTALL_USE_QT_SYS_PATHS=ON"

# Helper: build a single framework
build_framework() {
    local name="$1"
    shift
    local extra_args="$*"

    echo ""
    echo "================================================================"
    echo "  Building: ${name}"
    echo "================================================================"

    local src="${KF6_SOURCE}/${name}"
    local build="build-${name}"

    if [ ! -d "${src}" ]; then
        echo "  WARNING: Source directory ${src} not found -- skipping"
        return 0
    fi

    mkdir -p "${build}"
    cd "${build}"

    cmake ${CMAKE_COMMON_ARGS} ${extra_args} "${src}"
    cmake --build . --parallel "${JOBS}"
    cmake --install .

    cd ..
    echo "  ${name}: DONE"
}

# =========================================================================
# Tier 2 Frameworks (depend on Tier 1)
#
# Build order determined by inter-dependencies:
#
#   1. KAuth           (depends on KCoreAddons; uses Polkit)
#   2. KCrash          (depends on KCoreAddons)
#   3. KJobWidgets     (depends on KCoreAddons, KWidgetsAddons)
#   4. KNotifications  (depends on KConfig, KCoreAddons; D-Bus)
#   5. KIconThemes     (depends on KArchive, KI18n, KCoreAddons,
#                        KWidgetsAddons, KConfigWidgets -- see note)
#   6. KConfigWidgets  (depends on KConfig, KColorScheme, KCoreAddons,
#                        KGuiAddons, KI18n, KWidgetsAddons, KCodecs)
#   7. KXmlGui         (depends on KConfig, KConfigWidgets, KCoreAddons,
#                        KGuiAddons, KI18n, KIconThemes, KWidgetsAddons)
#   8. KGlobalAccel    (depends on KConfig, KCoreAddons; D-Bus service)
#   9. KBookmarks      (depends on KConfig, KConfigWidgets, KCoreAddons,
#                        KIconThemes, KWidgetsAddons, KXmlGui)
#
# Note: KIconThemes and KConfigWidgets have a circular dependency.
# Build KConfigWidgets first (without icon theme support), then
# KIconThemes, then rebuild KConfigWidgets if needed.
# =========================================================================

# --- Group 1: Minimal Tier 1 dependencies ---

build_framework "kauth" \
    "-DKAUTH_BACKEND_NAME=PolkitQt6-1"

build_framework "kcrash"

build_framework "kjobwidgets"

build_framework "knotifications" \
    "-DWITH_SNORETOAST=OFF"

# --- Group 2: KConfigWidgets (before KIconThemes) ---

build_framework "kconfigwidgets"

# --- Group 3: KIconThemes (depends on KConfigWidgets) ---

build_framework "kiconthemes" \
    "-DWITH_SVGZ=ON"

# --- Group 4: KGlobalAccel (D-Bus global shortcut service) ---

build_framework "kglobalaccel" \
    "-DKGLOBALACCEL_RUNTIME_DIR=${VERIDIAN_SYSROOT}/usr/lib/libexec"

# --- Group 5: KXmlGui (depends on KConfigWidgets + KIconThemes) ---

build_framework "kxmlgui"

# --- Group 6: KBookmarks (depends on most Tier 2) ---

build_framework "kbookmarks"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  Tier 2 Build Complete"
echo "========================================"
echo ""
echo "  Frameworks built:"
echo "    KAuth, KCrash, KJobWidgets, KNotifications,"
echo "    KConfigWidgets, KIconThemes, KGlobalAccel,"
echo "    KXmlGui, KBookmarks"
echo ""
echo "  Installed to: ${VERIDIAN_SYSROOT}/usr"
echo ""
echo "  Next: Run build-tier3.sh"

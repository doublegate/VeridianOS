#!/bin/sh
# VeridianOS -- build-tier3.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for KDE Frameworks 6 Tier 3 libraries.
#
# Tier 3 frameworks depend on Tier 1 + Tier 2 frameworks.
# All lower-tier frameworks must be installed before running this.
#
# Prerequisites:
#   1. Tier 1 frameworks installed (build-tier1.sh)
#   2. Tier 2 frameworks installed (build-tier2.sh)
#   3. Qt 6 installed in sysroot (Sprint 9.6)
#   4. ECM installed in sysroot
#   5. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#
# Usage:
#   cd /path/to/build-dir
#   /path/to/build-tier3.sh /path/to/kf6-source-root
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
echo "  KDE Frameworks 6 -- Tier 3 Build"
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
# Tier 3 Frameworks (depend on Tier 1 + Tier 2)
#
# Build order determined by inter-dependencies:
#
#   1. KService         (depends on KConfig, KCoreAddons, KI18n,
#                         KDBusAddons -- .desktop file parsing)
#   2. KTextWidgets     (depends on KConfig, KConfigWidgets, KI18n,
#                         KWidgetsAddons, KCompletion, Sonnet)
#   3. KWallet          (depends on KConfig, KCoreAddons, KDBusAddons,
#                         KWidgetsAddons, KNotifications)
#   4. KWindowSystem    (depends on Qt6 + Wayland protocols)
#   5. KIO              (depends on KConfig, KCoreAddons, KDBusAddons,
#                         KI18n, KService, KArchive, KWidgetsAddons,
#                         KJobWidgets, KBookmarks, KAuth, KWallet,
#                         KWindowSystem, Solid, KCrash, KCompletion,
#                         KIconThemes, KNotifications, KConfigWidgets)
#   6. KDeclarative     (depends on KConfig, KI18n, KWidgetsAddons,
#                         KGuiAddons, KIconThemes, KPackage)
#   7. KPackage         (depends on KArchive, KCoreAddons, KI18n)
#   8. KParts           (depends on KConfig, KCoreAddons, KI18n,
#                         KIO, KService, KWidgetsAddons, KXmlGui,
#                         KJobWidgets, KIconThemes)
#   9. KNewStuff        (depends on KI18n, KArchive, KPackage,
#                         KAttica, KCoreAddons, KConfig, KWidgetsAddons)
#  10. KActivities      (depends on KConfig, KCoreAddons -- D-Bus)
#  11. Plasma Framework (depends on KConfig, KCoreAddons, KI18n,
#                         KArchive, KPackage, KService, KDeclarative,
#                         KWindowSystem, KIconThemes, KNotifications,
#                         KActivities, KConfigWidgets, KGuiAddons)
# =========================================================================

# --- Group 1: Foundational Tier 3 ---

build_framework "kservice"

build_framework "ktextwidgets"

# --- KWallet: with VeridianOS backend ---

echo ""
echo "================================================================"
echo "  Building: kwallet (with VeridianOS backend)"
echo "================================================================"

KWALLET_SRC="${KF6_SOURCE}/kwallet"
if [ -d "${KWALLET_SRC}" ]; then
    # Copy VeridianOS backend files into KWallet source tree
    KWALLET_BACKENDS="${KWALLET_SRC}/src/runtime/kwalletd/backend"
    if [ -d "${KWALLET_BACKENDS}" ]; then
        cp -f "${SCRIPT_DIR}/kwallet-veridian-backend.h" \
              "${KWALLET_BACKENDS}/veridianbackend.h"
        cp -f "${SCRIPT_DIR}/kwallet-veridian-backend.cpp" \
              "${KWALLET_BACKENDS}/veridianbackend.cpp"
    fi

    mkdir -p build-kwallet
    cd build-kwallet
    cmake ${CMAKE_COMMON_ARGS} \
        -DHAVE_GPGMEPP=OFF \
        -DHAVE_GCRYPT=OFF \
        "${KWALLET_SRC}"
    cmake --build . --parallel "${JOBS}"
    cmake --install .
    cd ..
    echo "  kwallet: DONE"
else
    echo "  WARNING: KWallet source not found -- skipping"
fi

# --- KWindowSystem: with VeridianOS Wayland backend ---

echo ""
echo "================================================================"
echo "  Building: kwindowsystem (Wayland-only for VeridianOS)"
echo "================================================================"

KWINDOWSYSTEM_SRC="${KF6_SOURCE}/kwindowsystem"
if [ -d "${KWINDOWSYSTEM_SRC}" ]; then
    # Copy VeridianOS Wayland backend files
    KWS_PLUGINS="${KWINDOWSYSTEM_SRC}/src/platforms/wayland"
    if [ -d "${KWS_PLUGINS}" ]; then
        cp -f "${SCRIPT_DIR}/kwindowsystem-veridian.h" \
              "${KWS_PLUGINS}/veridianintegration.h"
        cp -f "${SCRIPT_DIR}/kwindowsystem-veridian.cpp" \
              "${KWS_PLUGINS}/veridianintegration.cpp"
    fi

    mkdir -p build-kwindowsystem
    cd build-kwindowsystem
    cmake ${CMAKE_COMMON_ARGS} \
        -DKWINDOWSYSTEM_WAYLAND=ON \
        -DKWINDOWSYSTEM_X11=OFF \
        "${KWINDOWSYSTEM_SRC}"
    cmake --build . --parallel "${JOBS}"
    cmake --install .
    cd ..
    echo "  kwindowsystem: DONE"
else
    echo "  WARNING: KWindowSystem source not found -- skipping"
fi

# --- Group 2: KIO (largest Tier 3 framework) ---

echo ""
echo "================================================================"
echo "  Building: kio (with VeridianOS file worker)"
echo "================================================================"

KIO_SRC="${KF6_SOURCE}/kio"
if [ -d "${KIO_SRC}" ]; then
    # Copy VeridianOS KIO worker files
    KIO_WORKERS="${KIO_SRC}/src/kioworkers/file"
    if [ -d "${KIO_WORKERS}" ]; then
        cp -f "${SCRIPT_DIR}/kio-veridian-worker.h" \
              "${KIO_WORKERS}/veridianfileworker.h"
        cp -f "${SCRIPT_DIR}/kio-veridian-worker.cpp" \
              "${KIO_WORKERS}/veridianfileworker.cpp"
    fi

    mkdir -p build-kio
    cd build-kio
    cmake ${CMAKE_COMMON_ARGS} \
        -DKIO_FORK_SLAVES=ON \
        -DWITH_ACL=OFF \
        "${KIO_SRC}"
    cmake --build . --parallel "${JOBS}"
    cmake --install .
    cd ..
    echo "  kio: DONE"
else
    echo "  WARNING: KIO source not found -- skipping"
fi

# --- Group 3: QML integration, packaging, parts ---

build_framework "kpackage"

build_framework "kdeclarative"

build_framework "kparts"

build_framework "knewstuff" \
    "-DWITH_KNEWSTUFFCORE=ON \
     -DWITH_KNEWSTUFFWIDGETS=ON"

build_framework "kactivities"

# --- Plasma Framework (top of the dependency chain) ---

build_framework "plasma-framework" \
    "-DPLASMA_NO_KIO=OFF"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  Tier 3 Build Complete"
echo "========================================"
echo ""
echo "  Frameworks built:"
echo "    KService, KTextWidgets, KWallet (VeridianOS backend),"
echo "    KWindowSystem (Wayland backend), KIO (VeridianOS worker),"
echo "    KPackage, KDeclarative, KParts, KNewStuff,"
echo "    KActivities, Plasma Framework"
echo ""
echo "  Installed to: ${VERIDIAN_SYSROOT}/usr"
echo ""
echo "  All KDE Frameworks 6 tiers complete!"
echo "  Next: Build KWin (Sprint 9.8)"

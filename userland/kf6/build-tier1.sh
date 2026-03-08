#!/bin/sh
# VeridianOS -- build-tier1.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for KDE Frameworks 6 Tier 1 libraries.
#
# Tier 1 frameworks have no KDE dependencies -- they depend only on Qt 6
# and standard system libraries.  Build order follows dependency chain.
#
# Prerequisites:
#   1. Qt 6 installed in sysroot (Sprint 9.6)
#   2. ECM installed in sysroot (build-ecm step below)
#   3. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   4. VeridianOS sysroot at /opt/veridian-sysroot
#
# Usage:
#   cd /path/to/build-dir
#   /path/to/build-tier1.sh /path/to/kf6-source-root
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
echo "  KDE Frameworks 6 -- Tier 1 Build"
echo "========================================"
echo "  KF6 source:    ${KF6_SOURCE}"
echo "  Sysroot:       ${VERIDIAN_SYSROOT}"
echo "  Host Qt path:  ${QT_HOST_PATH}"
echo "  Build type:    ${BUILD_TYPE}"
echo "  Jobs:          ${JOBS}"
echo "========================================"

# Common cmake arguments for all Tier 1 frameworks
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
# Step 0: Build and install ECM (Extra CMake Modules)
# =========================================================================

echo ""
echo "================================================================"
echo "  Building: extra-cmake-modules (ECM)"
echo "================================================================"

ECM_SRC="${KF6_SOURCE}/extra-cmake-modules"
if [ -d "${ECM_SRC}" ]; then
    mkdir -p build-ecm
    cd build-ecm
    cmake -G Ninja \
        -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
        -DCMAKE_INSTALL_PREFIX="${VERIDIAN_SYSROOT}/usr" \
        -DBUILD_TESTING=OFF \
        -DBUILD_HTML_DOCS=OFF \
        -DBUILD_MAN_DOCS=OFF \
        "${ECM_SRC}"
    cmake --build . --parallel "${JOBS}"
    cmake --install .
    cd ..
    echo "  extra-cmake-modules: DONE"
else
    echo "  WARNING: ECM source not found at ${ECM_SRC}"
    echo "  Assuming ECM already installed at ${ECM_DIR}"
fi

# Install VeridianOS platform files into ECM
if [ -d "${ECM_DIR}" ]; then
    echo "  Installing VeridianOS ECM platform files..."
    cp -f "${SCRIPT_DIR}/ecm/VeridianOSPlatform.cmake" \
          "${ECM_DIR}/VeridianOSPlatform.cmake" 2>/dev/null || true
    cp -f "${SCRIPT_DIR}/ecm/ECMVeridianOSConfig.cmake" \
          "${ECM_DIR}/ECMVeridianOSConfig.cmake" 2>/dev/null || true
fi

# =========================================================================
# Tier 1 Frameworks (no KDE dependencies)
#
# Build order determined by inter-dependencies:
#   1. KArchive           (no Tier 1 deps)
#   2. KCodecs            (no Tier 1 deps)
#   3. KConfig            (no Tier 1 deps)
#   4. KCoreAddons        (no Tier 1 deps)
#   5. KI18n              (no Tier 1 deps -- gettext)
#   6. KGuiAddons         (no Tier 1 deps)
#   7. KWidgetsAddons     (no Tier 1 deps)
#   8. KItemViews         (no Tier 1 deps)
#   9. KItemModels        (no Tier 1 deps)
#  10. KDBusAddons        (no Tier 1 deps -- D-Bus)
#  11. KColorScheme       (depends on KConfig)
#  12. ThreadWeaver       (no Tier 1 deps)
#  13. KCompletion        (depends on KConfig, KWidgetsAddons)
#  14. Sonnet             (no Tier 1 deps -- hunspell optional)
#  15. Solid              (depends on Qt6 -- needs VeridianOS backend)
# =========================================================================

# --- Group 1: No inter-dependencies ---

build_framework "karchive" \
    "-DWITH_LIBZSTD=OFF \
     -DWITH_LIBLZMA=OFF \
     -DWITH_BZIP2=OFF"

build_framework "kcodecs"

build_framework "kconfig" \
    "-DKCONFIG_USE_GUI=ON \
     -DKCONFIG_USE_QML=ON"

build_framework "kcoreaddons"

build_framework "ki18n" \
    "-DKI18N_USE_GETTEXT=ON"

build_framework "kguiaddons" \
    "-DWITH_WAYLAND=ON \
     -DWITH_X11=OFF"

build_framework "kwidgetsaddons"

build_framework "kitemviews"

build_framework "kitemmodels"

build_framework "kdbusaddons"

build_framework "threadweaver"

build_framework "sonnet" \
    "-DSONNET_USE_WIDGETS=ON \
     -DSONNET_USE_QML=OFF"

# --- Group 2: Depends on earlier Tier 1 ---

build_framework "kcolorscheme"

build_framework "kcompletion"

# --- Solid: needs VeridianOS backend ---

echo ""
echo "================================================================"
echo "  Building: solid (with VeridianOS backend)"
echo "================================================================"

SOLID_SRC="${KF6_SOURCE}/solid"
if [ -d "${SOLID_SRC}" ]; then
    # Copy VeridianOS backend files into Solid source tree
    SOLID_BACKENDS="${SOLID_SRC}/src/solid/devices/backends/veridian"
    mkdir -p "${SOLID_BACKENDS}"
    cp -f "${SCRIPT_DIR}/solid-veridian-backend.h" \
          "${SOLID_BACKENDS}/veridiandevice.h"
    cp -f "${SCRIPT_DIR}/solid-veridian-backend.cpp" \
          "${SOLID_BACKENDS}/veridiandevice.cpp"

    mkdir -p build-solid
    cd build-solid
    cmake ${CMAKE_COMMON_ARGS} \
        -DWITH_NEW_SOLID_JOB=ON \
        -DWITH_NEW_POWER_ASYNC_API=ON \
        "${SOLID_SRC}"
    cmake --build . --parallel "${JOBS}"
    cmake --install .
    cd ..
    echo "  solid: DONE"
else
    echo "  WARNING: Solid source not found at ${SOLID_SRC} -- skipping"
fi

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  Tier 1 Build Complete"
echo "========================================"
echo ""
echo "  Frameworks built:"
echo "    KArchive, KCodecs, KConfig, KCoreAddons, KI18n,"
echo "    KGuiAddons, KWidgetsAddons, KItemViews, KItemModels,"
echo "    KDBusAddons, KColorScheme, ThreadWeaver, KCompletion,"
echo "    Sonnet, Solid (with VeridianOS backend)"
echo ""
echo "  Installed to: ${VERIDIAN_SYSROOT}/usr"
echo ""
echo "  Next: Run build-tier2.sh"

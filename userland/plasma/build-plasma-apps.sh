#!/bin/sh
# VeridianOS -- build-plasma-apps.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for KDE Plasma workspace, Breeze theme, and core KDE
# applications targeting VeridianOS.
#
# Builds the following components in dependency order:
#   1. kdecoration (window decoration framework)
#   2. Breeze (style + decoration + icons + cursors)
#   3. plasma-integration (Qt platform theme)
#   4. plasma-workspace (startkde, session management, lock screen)
#   5. plasma-desktop (desktop containment, folder view)
#   6. KScreen (display configuration KCM)
#   7. PowerDevil (power management KCM)
#   8. System Settings (systemsettings6)
#   9. Dolphin (file manager)
#  10. Konsole (terminal emulator)
#  11. Kate (text editor)
#  12. Spectacle (screenshot utility)
#
# Prerequisites:
#   1. Qt 6 installed in sysroot (Sprint 9.6)
#   2. KDE Frameworks 6 installed in sysroot (Sprint 9.7)
#   3. KWin installed in sysroot (Sprint 9.8)
#   4. All system libraries from Sprints 9.0-9.5
#   5. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   6. Native Qt 6 host tools (moc, rcc, uic)
#   7. git, cmake, ninja
#
# Usage:
#   ./build-plasma-apps.sh [component]
#
#   component: all | kdecoration | breeze | plasma-integration |
#              plasma-workspace | plasma-desktop | kscreen |
#              powerdevil | systemsettings | dolphin | konsole |
#              kate | spectacle
#
# Environment variables:
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   QT_HOST_PATH     - Path to native Qt 6 installation (required)
#   BUILD_TYPE        - Release or Debug (default: Release)
#   JOBS              - Parallel build jobs (default: $(nproc))
#   KDE_GIT_BASE      - Base URL for KDE git repos

set -e

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
QT_HOST_PATH="${QT_HOST_PATH:?Set QT_HOST_PATH to your native Qt 6 install}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
JOBS="${JOBS:-$(nproc)}"
KDE_GIT_BASE="${KDE_GIT_BASE:-https://invent.kde.org}"
TOOLCHAIN_FILE="${SCRIPT_DIR}/../kwin/kwin-veridian-toolchain.cmake"
SOURCE_BASE="${SCRIPT_DIR}/src"
BUILD_BASE="${SCRIPT_DIR}/build"
COMPONENT="${1:-all}"

echo "========================================"
echo "  Plasma Apps Build for VeridianOS"
echo "========================================"
echo "  Component:      ${COMPONENT}"
echo "  Sysroot:        ${VERIDIAN_SYSROOT}"
echo "  Host Qt path:   ${QT_HOST_PATH}"
echo "  Build type:     ${BUILD_TYPE}"
echo "  Jobs:           ${JOBS}"
echo "========================================"

# =========================================================================
# Helper functions
# =========================================================================

clone_or_update() {
    REPO_NAME="$1"
    REPO_GROUP="$2"
    REPO_TAG="${3:-master}"

    REPO_DIR="${SOURCE_BASE}/${REPO_NAME}"
    REPO_URL="${KDE_GIT_BASE}/${REPO_GROUP}/${REPO_NAME}.git"

    if [ -d "${REPO_DIR}/.git" ]; then
        echo "  Updating ${REPO_NAME}..."
        cd "${REPO_DIR}"
        git fetch origin
        git checkout "${REPO_TAG}" 2>/dev/null || \
            git checkout -b "veridian-${REPO_TAG}" "${REPO_TAG}" 2>/dev/null || true
        cd "${SCRIPT_DIR}"
    else
        echo "  Cloning ${REPO_NAME} (${REPO_TAG})..."
        mkdir -p "${SOURCE_BASE}"
        git clone --depth=1 --branch="${REPO_TAG}" "${REPO_URL}" "${REPO_DIR}" || \
            git clone --depth=1 "${REPO_URL}" "${REPO_DIR}"
    fi
}

build_component() {
    COMP_NAME="$1"
    shift
    CMAKE_EXTRA_ARGS="$@"

    COMP_SOURCE="${SOURCE_BASE}/${COMP_NAME}"
    COMP_BUILD="${BUILD_BASE}/${COMP_NAME}"

    echo ""
    echo "================================================================"
    echo "  Building: ${COMP_NAME}"
    echo "================================================================"

    if [ ! -d "${COMP_SOURCE}" ]; then
        echo "  ERROR: Source directory ${COMP_SOURCE} not found"
        return 1
    fi

    mkdir -p "${COMP_BUILD}"
    cd "${COMP_BUILD}"

    cmake -G Ninja \
        -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
        -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN_FILE}" \
        -DCMAKE_INSTALL_PREFIX="${VERIDIAN_SYSROOT}/usr" \
        -DQT_HOST_PATH="${QT_HOST_PATH}" \
        -DQT_MAJOR_VERSION=6 \
        -DBUILD_TESTING=OFF \
        -DBUILD_QCH=OFF \
        -DKDE_INSTALL_USE_QT_SYS_PATHS=ON \
        ${CMAKE_EXTRA_ARGS} \
        "${COMP_SOURCE}"

    cmake --build . --parallel "${JOBS}"
    cmake --install .

    cd "${SCRIPT_DIR}"
    echo "  ${COMP_NAME} installed to ${VERIDIAN_SYSROOT}/usr"
}

should_build() {
    COMP="$1"
    [ "${COMPONENT}" = "all" ] || [ "${COMPONENT}" = "${COMP}" ]
}

# =========================================================================
# 1. kdecoration -- Window decoration framework
# =========================================================================

if should_build "kdecoration"; then
    clone_or_update "kdecoration" "plasma" "v6.2.0"
    build_component "kdecoration"
fi

# =========================================================================
# 2. Breeze -- Style + Decoration + Icons + Cursors
# =========================================================================

if should_build "breeze"; then
    clone_or_update "breeze" "plasma" "v6.2.0"

    # Copy VeridianOS style shim into source tree
    BREEZE_VERIDIAN_DIR="${SOURCE_BASE}/breeze/kdestyle/veridian"
    mkdir -p "${BREEZE_VERIDIAN_DIR}"
    cp -f "${SCRIPT_DIR}/breeze-veridian-style.h" "${BREEZE_VERIDIAN_DIR}/"
    cp -f "${SCRIPT_DIR}/breeze-veridian-style.cpp" "${BREEZE_VERIDIAN_DIR}/"

    # Copy decoration shim
    BREEZE_DECO_DIR="${SOURCE_BASE}/breeze/kdecoration/veridian"
    mkdir -p "${BREEZE_DECO_DIR}"
    cp -f "${SCRIPT_DIR}/breeze-veridian-decoration.h" "${BREEZE_DECO_DIR}/"
    cp -f "${SCRIPT_DIR}/breeze-veridian-decoration.cpp" "${BREEZE_DECO_DIR}/"

    build_component "breeze" \
        -DBUILD_KSTYLE=ON \
        -DBUILD_KDECORATION=ON \
        -DBUILD_ICONS=ON \
        -DBUILD_CURSORS=ON \
        -DBUILD_WALLPAPERS=ON
fi

# =========================================================================
# 3. plasma-integration -- Qt platform theme
# =========================================================================

if should_build "plasma-integration"; then
    clone_or_update "plasma-integration" "plasma" "v6.2.0"

    # Copy VeridianOS integration plugin into source tree
    INTEGRATION_DIR="${SOURCE_BASE}/plasma-integration/veridian"
    mkdir -p "${INTEGRATION_DIR}"
    cp -f "${SCRIPT_DIR}/plasma-veridian-integration.h" "${INTEGRATION_DIR}/"
    cp -f "${SCRIPT_DIR}/plasma-veridian-integration.cpp" "${INTEGRATION_DIR}/"

    build_component "plasma-integration"
fi

# =========================================================================
# 4. plasma-workspace -- Session management, lock screen
# =========================================================================

if should_build "plasma-workspace"; then
    clone_or_update "plasma-workspace" "plasma" "v6.2.0"

    # Copy lock screen plugin
    LOCKSCREEN_DIR="${SOURCE_BASE}/plasma-workspace/ksmserver/veridian"
    mkdir -p "${LOCKSCREEN_DIR}"
    cp -f "${SCRIPT_DIR}/plasma-veridian-lockscreen.h" "${LOCKSCREEN_DIR}/"
    cp -f "${SCRIPT_DIR}/plasma-veridian-lockscreen.cpp" "${LOCKSCREEN_DIR}/"

    build_component "plasma-workspace" \
        -DPLASMA_X11=OFF \
        -DPLASMA_WAYLAND=ON
fi

# =========================================================================
# 5. plasma-desktop -- Desktop containment, folder view
# =========================================================================

if should_build "plasma-desktop"; then
    clone_or_update "plasma-desktop" "plasma" "v6.2.0"
    build_component "plasma-desktop" \
        -DPLASMA_X11=OFF
fi

# =========================================================================
# 6. KScreen -- Display configuration
# =========================================================================

if should_build "kscreen"; then
    clone_or_update "kscreen" "plasma" "v6.2.0"

    # Copy VeridianOS backend
    KSCREEN_DIR="${SOURCE_BASE}/kscreen/backends/veridian"
    mkdir -p "${KSCREEN_DIR}"
    cp -f "${SCRIPT_DIR}/kscreen-veridian-backend.h" "${KSCREEN_DIR}/"
    cp -f "${SCRIPT_DIR}/kscreen-veridian-backend.cpp" "${KSCREEN_DIR}/"

    build_component "kscreen"
fi

# =========================================================================
# 7. PowerDevil -- Power management
# =========================================================================

if should_build "powerdevil"; then
    clone_or_update "powerdevil" "plasma" "v6.2.0"

    # Copy VeridianOS backend
    POWERDEVIL_DIR="${SOURCE_BASE}/powerdevil/daemon/backends/veridian"
    mkdir -p "${POWERDEVIL_DIR}"
    cp -f "${SCRIPT_DIR}/powerdevil-veridian-backend.h" "${POWERDEVIL_DIR}/"
    cp -f "${SCRIPT_DIR}/powerdevil-veridian-backend.cpp" "${POWERDEVIL_DIR}/"

    build_component "powerdevil"
fi

# =========================================================================
# 8. System Settings
# =========================================================================

if should_build "systemsettings"; then
    clone_or_update "systemsettings" "plasma" "v6.2.0"
    build_component "systemsettings"
fi

# =========================================================================
# 9. Dolphin -- File manager
# =========================================================================

if should_build "dolphin"; then
    clone_or_update "dolphin" "system" "v24.08.0"
    build_component "dolphin" \
        -DBUILD_TERMINAL=OFF
fi

# =========================================================================
# 10. Konsole -- Terminal emulator
# =========================================================================

if should_build "konsole"; then
    clone_or_update "konsole" "utilities" "v24.08.0"
    build_component "konsole"
fi

# =========================================================================
# 11. Kate -- Text editor
# =========================================================================

if should_build "kate"; then
    clone_or_update "kate" "utilities" "v24.08.0"
    build_component "kate" \
        -DBUILD_KWRITE=ON \
        -DBUILD_KATE=ON \
        -DBUILD_ADDONS=ON
fi

# =========================================================================
# 12. Spectacle -- Screenshot utility
# =========================================================================

if should_build "spectacle"; then
    clone_or_update "spectacle" "graphics" "v24.08.0"
    build_component "spectacle" \
        -DSPECTACLE_X11=OFF
fi

# =========================================================================
# Install assets
# =========================================================================

if [ "${COMPONENT}" = "all" ]; then
    echo ""
    echo "================================================================"
    echo "  Installing Breeze assets"
    echo "================================================================"
    "${SCRIPT_DIR}/install-breeze-assets.sh" "${VERIDIAN_SYSROOT}"

    # Install session startup script
    mkdir -p "${VERIDIAN_SYSROOT}/usr/bin"
    cp -f "${SCRIPT_DIR}/plasma-veridian-session.sh" \
          "${VERIDIAN_SYSROOT}/usr/bin/plasma-veridian-session"
    chmod +x "${VERIDIAN_SYSROOT}/usr/bin/plasma-veridian-session"
    echo "  Session script installed to ${VERIDIAN_SYSROOT}/usr/bin/"
fi

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  Plasma Apps Build Complete"
echo "========================================"
echo ""
echo "  Components built:"
if should_build "kdecoration";       then echo "    - kdecoration (window decoration framework)"; fi
if should_build "breeze";            then echo "    - Breeze (style + decoration + icons + cursors)"; fi
if should_build "plasma-integration"; then echo "    - plasma-integration (Qt platform theme)"; fi
if should_build "plasma-workspace";  then echo "    - plasma-workspace (session, lock screen)"; fi
if should_build "plasma-desktop";    then echo "    - plasma-desktop (desktop containment, folder view)"; fi
if should_build "kscreen";           then echo "    - KScreen (display configuration KCM)"; fi
if should_build "powerdevil";        then echo "    - PowerDevil (power management KCM)"; fi
if should_build "systemsettings";    then echo "    - System Settings (systemsettings6)"; fi
if should_build "dolphin";           then echo "    - Dolphin (file manager)"; fi
if should_build "konsole";           then echo "    - Konsole (terminal emulator)"; fi
if should_build "kate";              then echo "    - Kate (text editor)"; fi
if should_build "spectacle";         then echo "    - Spectacle (screenshot utility)"; fi
echo ""
echo "  Installed to: ${VERIDIAN_SYSROOT}/usr"
echo ""
echo "  To start a Plasma session on VeridianOS:"
echo "    plasma-veridian-session"
echo ""
echo "  Next: Run Sprint 9.10 (Integration + Polish)"

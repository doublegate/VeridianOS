#!/bin/sh
# VeridianOS -- build-kwin.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for KWin 6.x compositor targeting VeridianOS.
#
# Clones the KWin source tree, applies VeridianOS patches (disable X11,
# configure backends, add platform plugin), then cross-compiles using
# the KWin toolchain file.
#
# Prerequisites:
#   1. Qt 6 installed in sysroot (Sprint 9.6)
#   2. KDE Frameworks 6 installed in sysroot (Sprint 9.7)
#   3. libdrm, libgbm, libinput, xkbcommon, dbus-1 in sysroot
#   4. EGL + GLES 2.0 in sysroot (Sprint 9.3)
#   5. Cross-compiler toolchain (x86_64-veridian-gcc/g++)
#   6. Native Qt 6 host tools (moc, rcc, uic, etc.)
#   7. git (for cloning KWin source)
#
# Usage:
#   ./build-kwin.sh [source-dir]
#
# Environment variables:
#   VERIDIAN_SYSROOT - Path to sysroot (default: /opt/veridian-sysroot)
#   QT_HOST_PATH     - Path to native Qt 6 installation (required)
#   BUILD_TYPE        - Release or Debug (default: Release)
#   JOBS              - Parallel build jobs (default: $(nproc))
#   KWIN_VERSION      - KWin version tag (default: v6.2.0)
#   KWIN_GIT_URL      - KWin git repository URL

set -e

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
QT_HOST_PATH="${QT_HOST_PATH:?Set QT_HOST_PATH to your native Qt 6 install}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
JOBS="${JOBS:-$(nproc)}"
KWIN_VERSION="${KWIN_VERSION:-v6.2.0}"
KWIN_GIT_URL="${KWIN_GIT_URL:-https://invent.kde.org/plasma/kwin.git}"
TOOLCHAIN_FILE="${SCRIPT_DIR}/kwin-veridian-toolchain.cmake"

# Source directory: argument or default
KWIN_SOURCE="${1:-${SCRIPT_DIR}/kwin-src}"
BUILD_DIR="${SCRIPT_DIR}/kwin-build"

echo "========================================"
echo "  KWin Compositor Build for VeridianOS"
echo "========================================"
echo "  KWin version:  ${KWIN_VERSION}"
echo "  Source:         ${KWIN_SOURCE}"
echo "  Build dir:      ${BUILD_DIR}"
echo "  Sysroot:        ${VERIDIAN_SYSROOT}"
echo "  Host Qt path:   ${QT_HOST_PATH}"
echo "  Build type:     ${BUILD_TYPE}"
echo "  Jobs:           ${JOBS}"
echo "  Toolchain:      ${TOOLCHAIN_FILE}"
echo "========================================"

# =========================================================================
# Step 1: Clone or update KWin source
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 1: Obtaining KWin source"
echo "================================================================"

if [ -d "${KWIN_SOURCE}/.git" ]; then
    echo "  Source tree exists, updating..."
    cd "${KWIN_SOURCE}"
    git fetch origin
    git checkout "${KWIN_VERSION}" 2>/dev/null || git checkout -b "veridian-${KWIN_VERSION}" "${KWIN_VERSION}"
    cd "${SCRIPT_DIR}"
else
    echo "  Cloning KWin ${KWIN_VERSION}..."
    git clone --depth=1 --branch="${KWIN_VERSION}" "${KWIN_GIT_URL}" "${KWIN_SOURCE}"
fi

echo "  KWin source ready at ${KWIN_SOURCE}"

# =========================================================================
# Step 2: Apply VeridianOS patches
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 2: Applying VeridianOS patches"
echo "================================================================"

# Copy VeridianOS platform plugin files into KWin source tree
PLATFORM_DIR="${KWIN_SOURCE}/src/backends/veridian"
mkdir -p "${PLATFORM_DIR}"

echo "  Copying platform backend files..."
cp -f "${SCRIPT_DIR}/kwin-veridian-platform.h"    "${PLATFORM_DIR}/veridian_platform.h"
cp -f "${SCRIPT_DIR}/kwin-veridian-platform.cpp"   "${PLATFORM_DIR}/veridian_platform.cpp"
cp -f "${SCRIPT_DIR}/kwin-veridian-input.h"        "${PLATFORM_DIR}/veridian_input.h"
cp -f "${SCRIPT_DIR}/kwin-veridian-input.cpp"      "${PLATFORM_DIR}/veridian_input.cpp"
cp -f "${SCRIPT_DIR}/kwin-veridian-effects.cpp"    "${PLATFORM_DIR}/veridian_effects.cpp"
cp -f "${SCRIPT_DIR}/kwin-veridian-protocols.h"    "${PLATFORM_DIR}/veridian_protocols.h"
cp -f "${SCRIPT_DIR}/kwin-veridian-protocols.cpp"  "${PLATFORM_DIR}/veridian_protocols.cpp"
cp -f "${SCRIPT_DIR}/kwin-veridian-session.cpp"    "${PLATFORM_DIR}/veridian_session.cpp"

# Create CMakeLists.txt for the VeridianOS backend
cat > "${PLATFORM_DIR}/CMakeLists.txt" << 'VERIDIAN_CMAKE'
# VeridianOS DRM/KMS backend for KWin
add_library(kwin_veridian_backend MODULE
    veridian_platform.cpp
    veridian_input.cpp
    veridian_effects.cpp
    veridian_protocols.cpp
    veridian_session.cpp
)

target_link_libraries(kwin_veridian_backend PRIVATE
    kwin
    Qt6::Core
    Qt6::DBus
    Qt6::Gui
    ${LIBDRM_LIBRARIES}
    ${GBM_LIBRARIES}
    ${LIBINPUT_LIBRARIES}
    EGL
    GLESv2
    xkbcommon
    wayland-server
    dbus-1
)

target_include_directories(kwin_veridian_backend PRIVATE
    ${CMAKE_SOURCE_DIR}/src
    ${LIBDRM_INCLUDE_DIRS}
    ${GBM_INCLUDE_DIRS}
    ${LIBINPUT_INCLUDE_DIRS}
)

install(TARGETS kwin_veridian_backend
    DESTINATION ${KDE_INSTALL_PLUGINDIR}/org.kde.kwin.platforms)
VERIDIAN_CMAKE

echo "  Platform backend installed to ${PLATFORM_DIR}"

# Patch KWin's top-level CMakeLists.txt to include the VeridianOS backend
KWIN_BACKENDS_CMAKE="${KWIN_SOURCE}/src/backends/CMakeLists.txt"
if [ -f "${KWIN_BACKENDS_CMAKE}" ]; then
    if ! grep -q "veridian" "${KWIN_BACKENDS_CMAKE}"; then
        echo "" >> "${KWIN_BACKENDS_CMAKE}"
        echo "# VeridianOS DRM/KMS backend" >> "${KWIN_BACKENDS_CMAKE}"
        echo 'if(CMAKE_SYSTEM_NAME STREQUAL "VeridianOS")' >> "${KWIN_BACKENDS_CMAKE}"
        echo "    add_subdirectory(veridian)" >> "${KWIN_BACKENDS_CMAKE}"
        echo "endif()" >> "${KWIN_BACKENDS_CMAKE}"
        echo "  Patched ${KWIN_BACKENDS_CMAKE}"
    else
        echo "  VeridianOS backend already in CMakeLists.txt"
    fi
fi

echo "  Patches applied"

# =========================================================================
# Step 3: Configure with CMake
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 3: CMake configure"
echo "================================================================"

mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

cmake -G Ninja \
    -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
    -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN_FILE}" \
    -DCMAKE_INSTALL_PREFIX="${VERIDIAN_SYSROOT}/usr" \
    -DQT_HOST_PATH="${QT_HOST_PATH}" \
    -DQT_MAJOR_VERSION=6 \
    -DBUILD_TESTING=OFF \
    -DBUILD_QCH=OFF \
    -DKDE_INSTALL_USE_QT_SYS_PATHS=ON \
    -DKWIN_BUILD_X11=OFF \
    -DKWIN_BUILD_XWAYLAND=OFF \
    -DKWIN_BUILD_WAYLAND=ON \
    -DKWIN_BUILD_DRM=ON \
    -DKWIN_BUILD_LIBINPUT=ON \
    -DKWIN_BUILD_EGL=ON \
    -DKWIN_BUILD_SCREENLOCKER=OFF \
    -DKWIN_BUILD_ACTIVITIES=OFF \
    -DKWIN_BUILD_RUNNERS=OFF \
    "${KWIN_SOURCE}"

echo "  CMake configure complete"

# =========================================================================
# Step 4: Build
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 4: Building KWin"
echo "================================================================"

cmake --build . --parallel "${JOBS}"

echo "  Build complete"

# =========================================================================
# Step 5: Install to sysroot
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Installing to sysroot"
echo "================================================================"

cmake --install .

echo "  Installed to ${VERIDIAN_SYSROOT}/usr"

# =========================================================================
# Step 6: Install KWin configuration defaults
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 6: Installing VeridianOS configuration defaults"
echo "================================================================"

# Default kwinrc for VeridianOS
KWIN_DEFAULTS_DIR="${VERIDIAN_SYSROOT}/etc/xdg"
mkdir -p "${KWIN_DEFAULTS_DIR}"

cat > "${KWIN_DEFAULTS_DIR}/kwinrc" << 'KWINRC'
[Compositing]
Backend=OpenGL
GLCore=false
GLPreferBufferSwap=a
GLVSync=true
AnimationSpeed=3

[Wayland]
InputMethod=
VirtualKeyboardEnabled=false

[Windows]
FocusPolicy=ClickToFocus
AutoRaise=false
AutoRaiseInterval=750

[Desktops]
Number=2
Rows=1

[Effect-kwin4_effect_blur]
BlurStrength=12
NoiseStrength=0

[Effect-kwin4_effect_fade]
FadeDuration=200

[Effect-kwin4_effect_slide]
Duration=300

[TabBox]
HighlightWindows=true
LayoutName=thumbnail_grid
ShowDesktopMode=0
KWINRC

echo "  Default kwinrc installed to ${KWIN_DEFAULTS_DIR}/kwinrc"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  KWin Build Complete"
echo "========================================"
echo ""
echo "  Components built:"
echo "    - KWin Wayland compositor"
echo "    - VeridianOS DRM/KMS platform backend"
echo "    - VeridianOS input backend (libinput)"
echo "    - VeridianOS session launcher"
echo "    - KDE Wayland protocol handlers"
echo "    - Effect configuration (GPU auto-detect)"
echo ""
echo "  Installed to: ${VERIDIAN_SYSROOT}/usr"
echo "  Config:       ${KWIN_DEFAULTS_DIR}/kwinrc"
echo ""
echo "  To start KWin on VeridianOS:"
echo "    export XDG_RUNTIME_DIR=/run/user/\$(id -u)"
echo "    export WAYLAND_DISPLAY=wayland-0"
echo "    kwin_wayland --drm-device /dev/dri/card0"
echo ""
echo "  Next: Run Sprint 9.9 (Plasma Desktop)"

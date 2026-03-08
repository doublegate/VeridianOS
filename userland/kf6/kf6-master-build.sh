#!/bin/sh
# VeridianOS -- kf6-master-build.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Master build script for KDE Frameworks 6 on VeridianOS.
# Orchestrates all three tiers in dependency order.
#
# This script sets up the cross-compilation environment and runs
# build-tier1.sh, build-tier2.sh, and build-tier3.sh in sequence.
#
# Prerequisites:
#   1. Qt 6 installed in sysroot (Sprint 9.6)
#   2. VeridianOS cross-compiler toolchain installed
#   3. KDE Frameworks 6 source tree (all repos cloned)
#
# Usage:
#   /path/to/kf6-master-build.sh /path/to/kf6-source-root
#
# Environment variables:
#   VERIDIAN_SYSROOT  - Path to sysroot (default: /opt/veridian-sysroot)
#   QT_HOST_PATH      - Path to native Qt 6 installation (required)
#   BUILD_TYPE         - Release or Debug (default: Release)
#   JOBS               - Parallel build jobs (default: $(nproc))
#   BUILD_DIR          - Build directory (default: ./kf6-build)

set -e

# =========================================================================
# Configuration
# =========================================================================

KF6_SOURCE="${1:?Usage: $0 <kf6-source-root>}"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
QT_HOST_PATH="${QT_HOST_PATH:?Set QT_HOST_PATH to your native Qt 6 install}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
JOBS="${JOBS:-$(nproc)}"
BUILD_DIR="${BUILD_DIR:-./kf6-build}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Export for sub-scripts
export VERIDIAN_SYSROOT
export QT_HOST_PATH
export BUILD_TYPE
export JOBS

echo "========================================================"
echo "  KDE Frameworks 6 -- Master Build for VeridianOS"
echo "========================================================"
echo "  KF6 source:    ${KF6_SOURCE}"
echo "  Sysroot:       ${VERIDIAN_SYSROOT}"
echo "  Host Qt path:  ${QT_HOST_PATH}"
echo "  Build type:    ${BUILD_TYPE}"
echo "  Jobs:          ${JOBS}"
echo "  Build dir:     ${BUILD_DIR}"
echo "========================================================"
echo ""

# =========================================================================
# Step 0: Environment setup
# =========================================================================

echo "Setting up cross-compilation environment..."

# Ensure sysroot directories exist
mkdir -p "${VERIDIAN_SYSROOT}/usr/lib"
mkdir -p "${VERIDIAN_SYSROOT}/usr/lib/cmake"
mkdir -p "${VERIDIAN_SYSROOT}/usr/lib/pkgconfig"
mkdir -p "${VERIDIAN_SYSROOT}/usr/lib/plugins"
mkdir -p "${VERIDIAN_SYSROOT}/usr/include"
mkdir -p "${VERIDIAN_SYSROOT}/usr/share"
mkdir -p "${VERIDIAN_SYSROOT}/usr/share/ECM/cmake"

# Configure pkg-config for cross-compilation
export PKG_CONFIG_PATH=""
export PKG_CONFIG_LIBDIR="${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${VERIDIAN_SYSROOT}"

# Ensure CMake can find Qt6 in sysroot
export CMAKE_PREFIX_PATH="${VERIDIAN_SYSROOT}/usr/lib/cmake:${VERIDIAN_SYSROOT}/usr/share/ECM/cmake"

# Add host Qt tools to PATH
if [ -d "${QT_HOST_PATH}/bin" ]; then
    export PATH="${QT_HOST_PATH}/bin:${PATH}"
fi

echo "  PKG_CONFIG_LIBDIR: ${PKG_CONFIG_LIBDIR}"
echo "  CMAKE_PREFIX_PATH: ${CMAKE_PREFIX_PATH}"
echo ""

# =========================================================================
# Step 1: Create build directory
# =========================================================================

mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

# =========================================================================
# Step 2: Build Tier 1 (no KDE dependencies)
# =========================================================================

echo ""
echo "========================================================"
echo "  PHASE 1/3: Tier 1 Frameworks"
echo "========================================================"
echo ""

TIER1_START=$(date +%s)
"${SCRIPT_DIR}/build-tier1.sh" "${KF6_SOURCE}"
TIER1_END=$(date +%s)
TIER1_TIME=$((TIER1_END - TIER1_START))

echo ""
echo "  Tier 1 completed in ${TIER1_TIME} seconds"

# =========================================================================
# Step 3: Build Tier 2 (depends on Tier 1)
# =========================================================================

echo ""
echo "========================================================"
echo "  PHASE 2/3: Tier 2 Frameworks"
echo "========================================================"
echo ""

TIER2_START=$(date +%s)
"${SCRIPT_DIR}/build-tier2.sh" "${KF6_SOURCE}"
TIER2_END=$(date +%s)
TIER2_TIME=$((TIER2_END - TIER2_START))

echo ""
echo "  Tier 2 completed in ${TIER2_TIME} seconds"

# =========================================================================
# Step 4: Build Tier 3 (depends on Tier 1 + Tier 2)
# =========================================================================

echo ""
echo "========================================================"
echo "  PHASE 3/3: Tier 3 Frameworks"
echo "========================================================"
echo ""

TIER3_START=$(date +%s)
"${SCRIPT_DIR}/build-tier3.sh" "${KF6_SOURCE}"
TIER3_END=$(date +%s)
TIER3_TIME=$((TIER3_END - TIER3_START))

echo ""
echo "  Tier 3 completed in ${TIER3_TIME} seconds"

# =========================================================================
# Summary
# =========================================================================

TOTAL_TIME=$((TIER1_TIME + TIER2_TIME + TIER3_TIME))

echo ""
echo "========================================================"
echo "  KDE Frameworks 6 -- Build Complete!"
echo "========================================================"
echo ""
echo "  Tier 1 (15 frameworks): ${TIER1_TIME}s"
echo "    KArchive, KCodecs, KConfig, KCoreAddons, KI18n,"
echo "    KGuiAddons, KWidgetsAddons, KItemViews, KItemModels,"
echo "    KDBusAddons, KColorScheme, ThreadWeaver, KCompletion,"
echo "    Sonnet, Solid"
echo ""
echo "  Tier 2 (9 frameworks):  ${TIER2_TIME}s"
echo "    KAuth, KCrash, KJobWidgets, KNotifications,"
echo "    KConfigWidgets, KIconThemes, KGlobalAccel,"
echo "    KXmlGui, KBookmarks"
echo ""
echo "  Tier 3 (11 frameworks): ${TIER3_TIME}s"
echo "    KService, KTextWidgets, KWallet, KWindowSystem,"
echo "    KIO, KPackage, KDeclarative, KParts, KNewStuff,"
echo "    KActivities, Plasma Framework"
echo ""
echo "  Total: 35 frameworks in ${TOTAL_TIME}s"
echo "  Installed to: ${VERIDIAN_SYSROOT}/usr"
echo ""
echo "  VeridianOS-specific backends:"
echo "    - Solid: Device enumeration via /dev + /proc"
echo "    - KIO: Local file worker via POSIX APIs"
echo "    - KWindowSystem: Wayland via KDE plasma protocols"
echo "    - KWallet: File-based credential storage"
echo ""
echo "  Next steps:"
echo "    1. Build KWin compositor (Sprint 9.8)"
echo "    2. Build Plasma Desktop (Sprint 9.9)"
echo "    3. Integration testing (Sprint 9.10)"

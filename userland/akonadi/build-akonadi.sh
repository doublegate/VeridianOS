#!/bin/sh
# build-akonadi.sh -- Build Akonadi PIM data store for VeridianOS
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds the Akonadi data store as both a shared library and a
# standalone daemon binary.
#
# Usage:
#   ./build-akonadi.sh [clean]

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"
INSTALL_PREFIX="${VERIDIAN_SYSROOT:-/usr}"

CXX="${CXX:-g++}"
CXXFLAGS="${CXXFLAGS:--std=c++17 -Wall -Wextra -O2}"
INCLUDES="-I${SCRIPT_DIR}"

echo "[build-akonadi] Building Akonadi PIM data store for VeridianOS"

# Clean target
if [ "${1}" = "clean" ]; then
    echo "[build-akonadi] Cleaning build artifacts"
    rm -rf "${BUILD_DIR}"
    exit 0
fi

mkdir -p "${BUILD_DIR}"

# Build shared library
echo "[build-akonadi] Compiling libakonadi-veridian.so"
${CXX} ${CXXFLAGS} ${INCLUDES} \
    -shared -fPIC \
    -o "${BUILD_DIR}/libakonadi-veridian.so" \
    "${SCRIPT_DIR}/akonadi-veridian.cpp"

# Build standalone daemon
echo "[build-akonadi] Compiling akonadi-veridian (daemon)"
${CXX} ${CXXFLAGS} ${INCLUDES} \
    -DAKONADI_STANDALONE \
    -o "${BUILD_DIR}/akonadi-veridian" \
    "${SCRIPT_DIR}/akonadi-veridian.cpp"

echo "[build-akonadi] Build complete:"
echo "  Library: ${BUILD_DIR}/libakonadi-veridian.so"
echo "  Daemon:  ${BUILD_DIR}/akonadi-veridian"
echo ""
echo "Install targets:"
echo "  ${INSTALL_PREFIX}/lib/libakonadi-veridian.so"
echo "  ${INSTALL_PREFIX}/bin/akonadi-veridian"
echo "  ${INSTALL_PREFIX}/include/akonadi-veridian.h"

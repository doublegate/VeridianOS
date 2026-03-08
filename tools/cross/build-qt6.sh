#!/usr/bin/env bash
# Build Qt 6 (static) for VeridianOS
#
# Minimal static build: QtCore + QtGui + QtWidgets + QtWayland + QtDBus.
# Integrates the VeridianOS QPA plugin from userland/qt6/qpa/.
#
# This is the hardest phase. Qt 6 is ~25M LOC; even a minimal static
# build is a significant cross-compilation effort.
#
# Prerequisites:
#   - musl libc + all C dependencies + Mesa + Wayland + font stack + D-Bus

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/qt6"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
JOBS="${JOBS:-$(nproc)}"

QT_VER="6.8.3"
QT_MAJOR="6.8"
QT_BASE_URL="https://download.qt.io/official_releases/qt/${QT_MAJOR}/${QT_VER}/submodules"

log() { echo "[build-qt6] $*"; }
die() { echo "[build-qt6] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${BUILD_DIR}/${name}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        curl -fsSL -o "${tarball}" "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── 1. Build host Qt tools (moc, rcc, uic, etc.) ─────────────────────
# Qt cross-compilation requires native tools to generate code.
build_host_qt() {
    local host_prefix="${BUILD_DIR}/host-qt"
    if [[ -f "${host_prefix}/bin/moc" ]]; then
        log "Host Qt tools: already built."
        return 0
    fi
    fetch "qtbase-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtbase-everywhere-src-${QT_VER}.tar.xz" \
        "qtbase-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtbase-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/host-qt-build"
    log "Building host Qt tools (moc, rcc, uic)..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${src}/configure" \
            -prefix "${host_prefix}" \
            -release \
            -nomake examples \
            -nomake tests \
            -no-gui \
            -no-widgets \
            -dbus-linked \
            -no-opengl && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "Host Qt tools: done."
}

# ── 2. Install VeridianOS QPA plugin into Qt source tree ──────────────
install_qpa_plugin() {
    local src="${BUILD_DIR}/qtbase-everywhere-src-${QT_VER}"
    local qpa_dir="${src}/src/plugins/platforms/veridian"
    if [[ -d "${qpa_dir}" ]]; then
        log "QPA plugin: already installed in Qt source."
        return 0
    fi

    log "Installing VeridianOS QPA plugin into Qt source..."
    mkdir -p "${qpa_dir}"
    cp "${PROJECT_ROOT}/userland/qt6/qpa/"*.cpp "${qpa_dir}/" 2>/dev/null || true
    cp "${PROJECT_ROOT}/userland/qt6/qpa/"*.h "${qpa_dir}/" 2>/dev/null || true

    # Create CMakeLists.txt for the QPA plugin
    cat > "${qpa_dir}/CMakeLists.txt" << 'CMAKE'
qt_internal_add_plugin(QVeridianIntegrationPlugin
    OUTPUT_NAME qveridian
    PLUGIN_TYPE platforms
    DEFAULT_IF "veridian" IN_LIST QT_QPA_PLATFORMS
    SOURCES
        veridian_integration.cpp veridian_integration.h
        veridian_window.cpp veridian_window.h
        veridian_screen.cpp veridian_screen.h
        veridian_backingstore.cpp veridian_backingstore.h
        veridian_egl.cpp veridian_egl.h
    LIBRARIES
        Qt::Core
        Qt::CorePrivate
        Qt::Gui
        Qt::GuiPrivate
)
CMAKE

    log "QPA plugin: installed."
}

# ── 3. Cross-compile Qt 6 (static) ───────────────────────────────────
build_qt_cross() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6Core.a" ]]; then
        log "Qt 6 cross-build: already installed."
        return 0
    fi

    local src="${BUILD_DIR}/qtbase-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/cross-qt-build"
    local host_prefix="${BUILD_DIR}/host-qt"

    log "Cross-compiling Qt 6 ${QT_VER} (static) for VeridianOS..."
    rm -rf "${bld}"
    mkdir -p "${bld}"

    # Apply VeridianOS patches if present
    local patch_dir="${SCRIPT_DIR}/qt6-patches"
    if [[ -d "${patch_dir}" ]]; then
        local marker="${src}/.veridian_patched"
        if [[ ! -f "${marker}" ]]; then
            for patch in "${patch_dir}"/*.patch; do
                [[ -f "$patch" ]] || continue
                log "Applying $(basename "$patch")..."
                (cd "${src}" && patch -p1 < "$patch")
            done
            touch "${marker}"
        fi
    fi

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        "${src}/configure" \
            -prefix "${SYSROOT}/usr" \
            -static \
            -release \
            -opensource -confirm-license \
            -qt-host-path "${host_prefix}" \
            -platform linux-g++ \
            -xplatform linux-g++ \
            -opengl es2 \
            -egl \
            -no-openssl \
            -no-feature-sql \
            -no-feature-testlib \
            -no-feature-network \
            -no-feature-system-doubleconversion \
            -no-zstd \
            -no-feature-system-libb2 \
            -no-feature-textmarkdownreader \
            -no-feature-textmarkdownwriter \
            -no-feature-accessibility-atspi-bridge \
            -no-feature-mtdev \
            -no-feature-tslib \
            -no-feature-libinput \
            -system-zlib \
            -system-freetype \
            -system-harfbuzz \
            -system-pcre \
            -system-libpng \
            -system-libjpeg \
            -fontconfig \
            -dbus-linked \
            -nomake examples \
            -nomake tests \
            -- \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_IGNORE_PREFIX_PATH="/home/linuxbrew/.linuxbrew" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "Qt 6 cross-build: done."
}

# ── 4a. Build host QtWayland scanner ──────────────────────────────────
# The host Qt was built without GUI, so we can't use cmake to build the
# full QtWayland module natively. Instead, compile qtwaylandscanner
# manually against host QtCore and install cmake package configs.
build_host_qt_wayland() {
    local host_prefix="${BUILD_DIR}/host-qt"
    if [[ -f "${host_prefix}/libexec/qtwaylandscanner" ]]; then
        log "Host QtWayland scanner: already built."
        return 0
    fi
    fetch "qtwayland-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtwayland-everywhere-src-${QT_VER}.tar.xz" \
        "qtwayland-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtwayland-everywhere-src-${QT_VER}/src/qtwaylandscanner/qtwaylandscanner.cpp"
    log "Building host qtwaylandscanner..."
    mkdir -p "${host_prefix}/libexec"
    g++ -std=c++17 -O2 \
        -I"${host_prefix}/include" \
        -I"${host_prefix}/include/QtCore" \
        "${src}" \
        -L"${host_prefix}/lib" \
        -Wl,-rpath,"${host_prefix}/lib" \
        -lQt6Core \
        -lpthread -ldl \
        -o "${host_prefix}/libexec/qtwaylandscanner"

    # Create cmake package config for cross-compilation to find the scanner
    local cmake_dir="${host_prefix}/lib/cmake/Qt6WaylandScannerTools"
    mkdir -p "${cmake_dir}"
    cat > "${cmake_dir}/Qt6WaylandScannerToolsTargets.cmake" << EOF
if(NOT TARGET Qt6::qtwaylandscanner)
    add_executable(Qt6::qtwaylandscanner IMPORTED GLOBAL)
    set_target_properties(Qt6::qtwaylandscanner PROPERTIES
        IMPORTED_LOCATION "${host_prefix}/libexec/qtwaylandscanner"
    )
endif()
EOF
    cat > "${cmake_dir}/Qt6WaylandScannerToolsConfig.cmake" << 'CMAKEEOF'
if(NOT DEFINED QT_DEFAULT_MAJOR_VERSION)
    set(QT_DEFAULT_MAJOR_VERSION 6)
endif()
set(Qt6WaylandScannerTools_FOUND TRUE)
get_filename_component(_qt6_wst_dir "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
include("${_qt6_wst_dir}/Qt6WaylandScannerToolsTargets.cmake")
unset(_qt6_wst_dir)
CMAKEEOF
    cat > "${cmake_dir}/Qt6WaylandScannerToolsConfigVersion.cmake" << VEREOF
set(PACKAGE_VERSION "${QT_VER}")
set(PACKAGE_VERSION_EXACT FALSE)
set(PACKAGE_VERSION_COMPATIBLE TRUE)
if("\${PACKAGE_FIND_VERSION}" VERSION_EQUAL "${QT_VER}")
    set(PACKAGE_VERSION_EXACT TRUE)
endif()
VEREOF
    log "Host QtWayland scanner: done."
}

# ── 4b. Build QtWayland (cross) ──────────────────────────────────────
build_qt_wayland() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6WaylandClient.a" ]]; then
        log "QtWayland: already installed."
        return 0
    fi

    local src="${BUILD_DIR}/qtwayland-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/qtwayland-build"
    local host_prefix="${BUILD_DIR}/host-qt"
    log "Building QtWayland ${QT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DQT_HOST_PATH="${host_prefix}" \
            -DBUILD_SHARED_LIBS=OFF \
            -DCMAKE_IGNORE_PREFIX_PATH="/home/linuxbrew/.linuxbrew" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "QtWayland: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Qt 6 installation..."
    local errors=0
    for lib in libQt6Core.a libQt6Gui.a libQt6Widgets.a libQt6DBus.a libQt6WaylandClient.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            local size
            size=$(stat -c%s "${SYSROOT}/usr/lib/${lib}" 2>/dev/null || echo "?")
            log "  OK: ${lib} (${size} bytes)"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    for tool in moc rcc uic; do
        if [[ -f "${BUILD_DIR}/host-qt/bin/${tool}" ]]; then
            log "  OK: host ${tool}"
        else
            log "  MISSING: host ${tool}"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} items missing!"
    fi
    log "Qt 6 static build ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building Qt 6 ${QT_VER} (static) for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"

    [[ -f "${SYSROOT}/usr/lib/libc.a" ]] || die "musl libc not found. Run build-musl.sh first."
    [[ -f "${SYSROOT}/usr/lib/libz.a" ]] || die "zlib not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/libfreetype.a" ]] || die "FreeType not found. Run build-fonts.sh first."
    [[ -f "${SYSROOT}/usr/lib/libdbus-1.a" ]] || die "D-Bus not found. Run build-dbus.sh first."
    [[ -f "${SYSROOT}/usr/lib/libwayland-client.a" ]] || die "Wayland not found. Run build-wayland.sh first."

    build_host_qt
    install_qpa_plugin
    build_qt_cross
    build_host_qt_wayland
    build_qt_wayland
    verify
    log "=== Qt 6 build complete ==="
}

main "$@"

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

# ── 1. Build host Qt (full, with GUI/Widgets/DBus) ───────────────────
# Qt cross-compilation requires native tools (moc, rcc, uic) and full
# host Qt libraries for building submodule host tools (qsb, qmlcachegen).
build_host_qt() {
    local host_prefix="${BUILD_DIR}/host-qt"
    if [[ -f "${host_prefix}/libexec/moc" ]]; then
        log "Host Qt: already built."
        return 0
    fi
    fetch "qtbase-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtbase-everywhere-src-${QT_VER}.tar.xz" \
        "qtbase-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtbase-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/host-qt-build"
    log "Building host Qt (full)..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${src}/configure" \
            -prefix "${host_prefix}" \
            -release \
            -nomake examples \
            -nomake tests \
            -dbus-linked \
            -gui \
            -widgets && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "Host Qt: done."
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
            -openssl-linked \
            -feature-sql \
            -sql-sqlite \
            -no-feature-testlib \
            -no-feature-system-doubleconversion \
            -no-zstd \
            -no-feature-system-libb2 \
            -no-feature-textmarkdownreader \
            -no-feature-textmarkdownwriter \
            -no-feature-accessibility-atspi-bridge \
            -no-feature-mtdev \
            -no-feature-tslib \
            -feature-libinput \
            -no-feature-brotli \
            -system-zlib \
            -system-freetype \
            -system-harfbuzz \
            -qt-pcre \
            -system-libpng \
            -system-libjpeg \
            -fontconfig \
            -dbus-linked \
            -nomake examples \
            -nomake tests \
            -- \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" && \
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
            -DQT_HOST_PATH:PATH="${host_prefix}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${host_prefix}/lib/cmake" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DQT_BUILD_EXAMPLES=OFF \
            -DQT_FORCE_BUILD_TOOLS=OFF \
            -DQT_FEATURE_wayland_server=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "QtWayland: done."
}

# ── 5. Build QtShaderTools ────────────────────────────────────────────
# Required by QtQuick for runtime shader compilation.
build_qt_shadertools() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6ShaderTools.a" ]]; then
        log "QtShaderTools: already installed."
        return 0
    fi
    fetch "qtshadertools-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtshadertools-everywhere-src-${QT_VER}.tar.xz" \
        "qtshadertools-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtshadertools-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/qtshadertools-build"
    local host_prefix="${BUILD_DIR}/host-qt"

    # First build qsb tool for host (code generator)
    if [[ ! -f "${host_prefix}/bin/qsb" ]] && \
       [[ ! -f "${host_prefix}/libexec/qsb" ]]; then
        log "Building host QtShaderTools (qsb tool)..."
        local host_bld="${BUILD_DIR}/host-qtshadertools-build"
        rm -rf "${host_bld}"
        mkdir -p "${host_bld}"
        (cd "${host_bld}" && \
            cmake "${src}" \
                -DCMAKE_PREFIX_PATH="${host_prefix}" \
                -DCMAKE_INSTALL_PREFIX="${host_prefix}" \
                -DBUILD_SHARED_LIBS=ON \
                -DBUILD_TESTING=OFF \
                -DQT_BUILD_TESTS=OFF && \
            cmake --build . --parallel "${JOBS}" && \
            cmake --install .)
        log "Host QtShaderTools: done."
    fi

    log "Building QtShaderTools ${QT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DQT_HOST_PATH:PATH="${host_prefix}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${host_prefix}/lib/cmake" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DQT_BUILD_EXAMPLES=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "QtShaderTools: done."
}

# ── 6. Build QtDeclarative (QML + Quick) ─────────────────────────────
# Provides QtQml and QtQuick, required by KDE Plasma shell.
build_qt_declarative() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6Qml.a" ]]; then
        log "QtDeclarative: already installed."
        return 0
    fi
    fetch "qtdeclarative-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtdeclarative-everywhere-src-${QT_VER}.tar.xz" \
        "qtdeclarative-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtdeclarative-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/qtdeclarative-build"
    local host_prefix="${BUILD_DIR}/host-qt"

    # Build host QML tools (qmlcachegen, qmltyperegistrar, etc.)
    if [[ ! -f "${host_prefix}/bin/qmlcachegen" ]] && \
       [[ ! -f "${host_prefix}/libexec/qmlcachegen" ]]; then
        log "Building host QtDeclarative tools..."
        local host_bld="${BUILD_DIR}/host-qtdeclarative-build"
        rm -rf "${host_bld}"
        mkdir -p "${host_bld}"
        (cd "${host_bld}" && \
            cmake "${src}" \
                -DCMAKE_PREFIX_PATH="${host_prefix}" \
                -DCMAKE_INSTALL_PREFIX="${host_prefix}" \
                -DBUILD_SHARED_LIBS=ON \
                -DBUILD_TESTING=OFF \
                -DQT_BUILD_TESTS=OFF \
                -DQT_BUILD_EXAMPLES=OFF && \
            cmake --build . --parallel "${JOBS}" && \
            cmake --install .)
        log "Host QtDeclarative tools: done."
    fi

    log "Building QtDeclarative ${QT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DQT_HOST_PATH:PATH="${host_prefix}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${host_prefix}/lib/cmake" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DQT_BUILD_EXAMPLES=OFF \
            -DQT_FORCE_BUILD_TOOLS=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" && \
        cmake --build . --parallel "${JOBS}" -- -k || true && \
        cmake --install . 2>/dev/null || true)
    # Tool binaries (qml, qmlpreview, etc.) may fail to link when cross-
    # compiling due to static Mesa link complexity. The libraries themselves
    # build successfully and are installed. Host tools (from host-qt) are
    # used for code generation instead.
    if [[ ! -f "${SYSROOT}/usr/lib/libQt6Qml.a" ]]; then
        die "QtDeclarative build failed: libQt6Qml.a not produced"
    fi
    log "QtDeclarative: done."
}

# ── 7. Build QtSvg ───────────────────────────────────────────────────
# SVG support used by KDE icons and themes.
build_qt_svg() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6Svg.a" ]]; then
        log "QtSvg: already installed."
        return 0
    fi
    fetch "qtsvg-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtsvg-everywhere-src-${QT_VER}.tar.xz" \
        "qtsvg-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtsvg-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/qtsvg-build"
    local host_prefix="${BUILD_DIR}/host-qt"
    log "Building QtSvg ${QT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DQT_HOST_PATH:PATH="${host_prefix}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${host_prefix}/lib/cmake" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DQT_BUILD_EXAMPLES=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "QtSvg: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Qt 6 installation..."
    local errors=0
    for lib in libQt6Core.a libQt6Gui.a libQt6Widgets.a libQt6DBus.a libQt6WaylandClient.a libQt6Qml.a libQt6Quick.a libQt6ShaderTools.a libQt6Svg.a; do
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
    build_qt_shadertools
    build_qt_declarative
    build_qt_svg
    build_host_qt_wayland
    build_qt_wayland
    verify
    log "=== Qt 6 build complete ==="
}

main "$@"

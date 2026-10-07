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
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/qt6"
SYSROOT="${VERIDIAN_SYSROOT}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
JOBS="${JOBS:-$(nproc)}"

QT_VER="6.12.0"
QT_MAJOR="6.12"
# archive/ is permanent; official_releases/ drops a version once it is superseded.
QT_BASE_URL="https://download.qt.io/archive/qt/${QT_MAJOR}/${QT_VER}/submodules"

log() { echo "[build-qt6] $*"; }
die() { echo "[build-qt6] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${VERIDIAN_SOURCES}/${name}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        { curl -fsSL -o "${tarball}.part" "${url}" || wget -q -O "${tarball}.part" "${url}"; } && [[ -s "${tarball}.part" ]] && mv "${tarball}.part" "${tarball}" || { rm -f "${tarball}.part"; echo "download failed: ${url}" >&2; exit 1; }
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
            -widgets \
            -- -DFEATURE_system_textmarkdownreader=OFF && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "Host Qt: done."
}

# ── 2. Install VeridianOS QPA plugin into Qt source tree ──────────────
install_qpa_plugin() {
    local src="${BUILD_DIR}/qtbase-everywhere-src-${QT_VER}"
    local platforms="${src}/src/plugins/platforms"
    local qpa_dir="${platforms}/veridian"

    # Refreshed on every run from userland/qt6/qpa (sources, metadata and its
    # qtbase-internal CMakeLists.txt), so the tree never keeps a stale copy.
    # It used to be copied once, with a generated CMakeLists.txt naming
    # veridian_*.cpp files that do not exist, and was never added to the
    # platforms list, so the plugin was never built.
    log "Installing VeridianOS QPA plugin into Qt source..."
    rm -rf "${qpa_dir}"
    mkdir -p "${qpa_dir}"
    cp "${PROJECT_ROOT}/userland/qt6/qpa/"*.cpp "${PROJECT_ROOT}/userland/qt6/qpa/"*.h \
       "${PROJECT_ROOT}/userland/qt6/qpa/veridian.json" \
       "${PROJECT_ROOT}/userland/qt6/qpa/CMakeLists.txt" "${qpa_dir}/"

    # Build it with qtbase: it needs the Wayland client and EGL that qtbase
    # already uses for its own Wayland plugin.
    if ! grep -q "add_subdirectory(veridian)" "${platforms}/CMakeLists.txt"; then
        cat >> "${platforms}/CMakeLists.txt" << 'CMAKE'
if(QT_FEATURE_wayland AND QT_FEATURE_egl)
    add_subdirectory(veridian) # VeridianOS platform (tools/cross/build-qt6.sh)
endif()
CMAKE
    fi
    grep -q "add_subdirectory(veridian)" "${platforms}/CMakeLists.txt" \
        || die "could not add the veridian platform to ${platforms}/CMakeLists.txt"

    log "QPA plugin: installed."
}

# ── 3. Cross-compile Qt 6 (static) ───────────────────────────────────
build_qt_cross() {
    # qtbase is complete only with the VeridianOS platform plugin it builds.
    if [[ -f "${SYSROOT}/usr/lib/libQt6Core.a" && -f "${SYSROOT}/usr/plugins/platforms/libqveridian.a" ]]; then
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
            -feature-wayland-client \
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
    # Since Qt 6.10 the scanner, WaylandClient and the Wayland QPA plugin
    # live in qtbase (fetched by build_host_qt); qtwayland keeps only the
    # compositor and a few extra plugins. A host qtbase that found the
    # host's wayland-scanner has built the tool already (checked above).
    local src="${BUILD_DIR}/qtbase-everywhere-src-${QT_VER}/src/tools/qtwaylandscanner/qtwaylandscanner.cpp"
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
    # libQt6WaylandClient.a now comes from qtbase, so a stamp marks this
    # module (the remaining qtwayland plugins) as done.
    if [[ -f "${BUILD_DIR}/qtwayland.done" ]]; then
        log "QtWayland: already installed."
        return 0
    fi
    fetch "qtwayland-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtwayland-everywhere-src-${QT_VER}.tar.xz" \
        "qtwayland-everywhere-src-${QT_VER}"

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
    touch "${BUILD_DIR}/qtwayland.done"
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
        # Host build: drop the cross pkg-config search path exported by the
        # cross steps above, or sysroot (musl) headers leak into host code.
        (unset PKG_CONFIG_PATH PKG_CONFIG_LIBDIR PKG_CONFIG_SYSROOT_DIR && \
            cd "${host_bld}" && \
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
        cmake --install . --prefix "${SYSROOT}/usr")
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
        # Host build: drop the cross pkg-config search path exported by the
        # cross steps above, or sysroot (musl) headers leak into host code.
        (unset PKG_CONFIG_PATH PKG_CONFIG_LIBDIR PKG_CONFIG_SYSROOT_DIR && \
            cd "${host_bld}" && \
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

    # Skip the target-side apps in tools/ (qml, qmleasing, qmlscene,
    # svgtoqml, ...) when cross-compiling. They fail to link against static
    # Mesa/udev, and their install rules sit in tools/cmake_install.cmake
    # ahead of later CMake package configs (Qt6QmlModels etc.), so one
    # failed link aborts the install and KF6 cannot find Qt6Quick. The
    # build-time tools (qmltyperegistrar, qmlcachegen, ...) are outside this
    # block and come from host-qt.
    # The apps block is `if(NOT (ANDROID OR WASM OR IOS ...))`; the platform
    # list grows between releases (6.12 added OHOS), so it is matched by
    # pattern and must occur exactly once.
    local tools_cml="${src}/tools/CMakeLists.txt"
    python3 - "${tools_cml}" <<'PYEOF' || die "unexpected ${tools_cml}: cannot gate target apps"
import re, sys
p = sys.argv[1]
s = open(p).read()
if "OR CMAKE_CROSSCOMPILING))" in s:
    sys.exit(0)
pat = re.compile(r"^if\(NOT \((ANDROID OR WASM OR IOS[^()]*)\)\)$", re.M)
if len(pat.findall(s)) != 1:
    sys.exit(1)
open(p, "w").write(pat.sub(r"if(NOT (\1 OR CMAKE_CROSSCOMPILING))", s))
PYEOF
    grep -q "OR CMAKE_CROSSCOMPILING))" "${tools_cml}" || die "failed to patch ${tools_cml}"

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
        cmake --install . --prefix "${SYSROOT}/usr" 2>/dev/null || true)
    # Tool binaries (qml, qmlpreview, etc.) may fail to link when cross-
    # compiling due to static Mesa link complexity. The libraries themselves
    # build successfully and are installed. Host tools (from host-qt) are
    # used for code generation instead.

    # Copy any .a libraries the install step missed (due to failed tool binaries)
    for lib in "${bld}"/lib/libQt6*.a; do
        [[ -f "$lib" ]] || continue
        local base
        base=$(basename "$lib")
        if [[ ! -f "${SYSROOT}/usr/lib/${base}" ]]; then
            log "  Manually copying ${base}..."
            cp "$lib" "${SYSROOT}/usr/lib/"
        fi
    done

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

# Qt 5 compatibility module (QTextCodec, QRegExp, ...), required by KWin.
build_qt_5compat() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6Core5Compat.a" ]]; then
        log "Qt5Compat: already installed."
        return 0
    fi
    fetch "qt5compat-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qt5compat-everywhere-src-${QT_VER}.tar.xz" \
        "qt5compat-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qt5compat-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/qt5compat-build"
    local host_prefix="${BUILD_DIR}/host-qt"
    log "Building Qt5Compat ${QT_VER}..."
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
    log "Qt5Compat: done."
}

# Qt Sensors (orientation sensor), required by KWin core.
build_qt_sensors() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6Sensors.a" ]]; then
        log "QtSensors: already installed."
        return 0
    fi
    fetch "qtsensors-everywhere-src-${QT_VER}" \
        "${QT_BASE_URL}/qtsensors-everywhere-src-${QT_VER}.tar.xz" \
        "qtsensors-everywhere-src-${QT_VER}"

    local src="${BUILD_DIR}/qtsensors-everywhere-src-${QT_VER}"
    local bld="${BUILD_DIR}/qtsensors-build"
    local host_prefix="${BUILD_DIR}/host-qt"
    log "Building QtSensors ${QT_VER}..."
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
    log "QtSensors: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Qt 6 installation..."
    local errors=0
    for lib in libQt6Core.a libQt6Gui.a libQt6Widgets.a libQt6DBus.a libQt6WaylandClient.a libQt6Qml.a libQt6Quick.a libQt6QmlModels.a libQt6ShaderTools.a libQt6Svg.a libQt6SvgWidgets.a libQt6Core5Compat.a libQt6Sensors.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            local size
            size=$(stat -c%s "${SYSROOT}/usr/lib/${lib}" 2>/dev/null || echo "?")
            log "  OK: ${lib} (${size} bytes)"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    # The VeridianOS platform plugin (userland/qt6/qpa), built with qtbase.
    if [[ -f "${SYSROOT}/usr/plugins/platforms/libqveridian.a" ]]; then
        log "  OK: platforms/libqveridian.a"
    else
        log "  MISSING: platforms/libqveridian.a"
        errors=$((errors + 1))
    fi
    for tool in moc rcc uic; do
        # Qt 6 installs these in libexec/ (bin/ only holds user-facing tools).
        if [[ -f "${BUILD_DIR}/host-qt/libexec/${tool}" || -f "${BUILD_DIR}/host-qt/bin/${tool}" ]]; then
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
    # The host scanner must exist before the cross qtbase: since Qt 6.10
    # qtbase builds WaylandClient and needs it.
    build_host_qt_wayland
    install_qpa_plugin
    build_qt_cross
    build_qt_shadertools
    build_qt_declarative
    build_qt_svg
    build_qt_5compat
    build_qt_sensors
    build_qt_wayland
    verify
    log "=== Qt 6 build complete ==="
}

main "$@"

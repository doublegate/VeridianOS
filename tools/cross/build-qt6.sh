#!/usr/bin/env bash
# Build Qt 6 for VeridianOS
#
# A host Qt (${VERIDIAN_HOST_TOOLS}/qt6: moc, rcc, uic, qtwaylandscanner,
# qsb, qmlcachegen, ...) runs Qt's code generators; the target Qt is
# shared (ADR 0010), configured for /usr (Qt's directories laid out as distributions
# do, under /usr/lib/qt6) and staged into the sysroot (-extprefix).
# qtbase also builds the VeridianOS QPA plugin from userland/qt6/qpa/.
#
# Modules: qtbase, qtshadertools, qtdeclarative, qtsvg, qt5compat,
# qtsensors, qtwayland, qtpositioning, qtlocation, qttools (UiTools for
# the target, Linguist's tools on the host). Tarball checksums are the
# ones Qt publishes.
#
# Prerequisites: build-deps.sh, build-mesa.sh, build-wayland.sh,
# build-fonts.sh, build-dbus.sh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/qt6"
JOBS="${JOBS:-$(nproc)}"

QT_VER="6.12.0"
QT_MAJOR="6.12"
# archive/ is permanent; official_releases/ drops a version once it is superseded.
QT_BASE_URL="https://download.qt.io/archive/qt/${QT_MAJOR}/${QT_VER}/submodules"
declare -A QT_SHA256=(
    [qtbase]="a951bd163c7b80fc6b8c88d7668fb56abf91c152373e13c10666763238131307"
    [qtshadertools]="c7d84f436e1aaef39fdcadebf2bd71bdf24dc69497e13f2e0198873cbbfea2ab"
    [qtdeclarative]="311f3a2603e1973bb59baef9dfa740a376de713157d4782ee043681e889c9260"
    [qtsvg]="e4ab39534ec97987b1b9b60ef7f4d3253d5912a69f8c713a01753174a4029331"
    [qt5compat]="78cf1c283795312a7fa31c73eb7f7f556b48374adc484f10ad5a3c0bcb59264e"
    [qtsensors]="861df58be8808cef777a1b531d7bec804d25733ec34e8ebbdafbe27040319090"
    [qtwayland]="ae14d9bcf1bb3c300a3ab2e6a53f50284e5c6c966a4ac4cc5aedcb1bc74e4d02"
    [qttools]="8dab8f3611496486a470ad5f115ceea584f36bc22a2b8b6f6ebdbafbb8160693"
    [qtpositioning]="b060d410fac7413b05a0e7938bbf3325f7ad08d7df14af3956270f0696e76763"
    [qtlocation]="218696d57c9eb95e8756e9fda62a8eabf94badc507db5f6e8ab9a640c4b0f50d"
    [qtwebsockets]="21690a365cd5fbfa8dea051473e98968df9f4eca0121fed7481d1215b7455a31"
    [qtspeech]="9a60ce5bee54a9343740feab2582bffb996812f43d6c5585aade26dbd04933db"
    [qtmultimedia]="3143f53b64257ba2a0685c940f5dce476c75871ff86b516de45cfd54ea93a62d"
)
QT_HOST="${VERIDIAN_HOST_TOOLS}/qt6"

log() { echo "[build-qt6] $*"; }
die() { echo "[build-qt6] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

# qt_src MODULE: a fresh, checked source tree of MODULE (its path is echoed).
qt_src() {
    local name="$1-everywhere-src-${QT_VER}"
    fetch "${name}.tar.xz" "${QT_BASE_URL}/${name}.tar.xz" "${name}" "${QT_SHA256[$1]}" >&2
    echo "${BUILD_DIR}/${name}"
}

# A native build: none of the target environment.
host_env() {
    env -u CC -u CXX -u AR -u RANLIB -u NM -u STRIP -u CFLAGS -u CXXFLAGS \
        -u PKG_CONFIG_LIBDIR -u PKG_CONFIG_SYSROOT_DIR "$@"
}

# ── 1. Host Qt ────────────────────────────────────────────────────────
# Qt cross-compilation requires native tools (moc, rcc, uic,
# qtwaylandscanner) and host Qt libraries for building the other modules'
# host tools (qsb, qmlcachegen).
build_host_qt() {
    if [[ -x "${QT_HOST}/libexec/moc" && -x "${QT_HOST}/libexec/qtwaylandscanner" ]]; then
        log "Host Qt: already built."
        return 0
    fi
    local src
    src="$(qt_src qtbase)"
    local bld="${BUILD_DIR}/host-qt-build"
    log "Building host Qt..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && host_env "${src}/configure" \
            -prefix "${QT_HOST}" \
            -release \
            -nomake examples \
            -nomake tests \
            -dbus-linked \
            -gui \
            -widgets \
            -feature-qtwaylandscanner \
            -no-feature-system-textmarkdownreader && \
        host_env cmake --build . --parallel "${JOBS}" && \
        host_env cmake --install .)
    [[ -x "${QT_HOST}/libexec/qtwaylandscanner" ]] || die "host Qt has no qtwaylandscanner"
}

# host_qt_module MODULE TOOL [CMAKE OPTIONS...]: MODULE's host tools into
# the host Qt.
host_qt_module() {
    local module="$1" tool="$2"
    shift 2
    if [[ -x "${QT_HOST}/bin/${tool}" || -x "${QT_HOST}/libexec/${tool}" ]]; then
        log "Host ${module}: already built."
        return 0
    fi
    local src
    src="$(qt_src "${module}")"
    local bld="${BUILD_DIR}/host-${module}-build"
    log "Building host ${module}..."
    rm -rf "${bld}"
    host_env cmake -S "${src}" -B "${bld}" \
        -DCMAKE_PREFIX_PATH="${QT_HOST}" \
        -DCMAKE_INSTALL_PREFIX="${QT_HOST}" \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_SHARED_LIBS=ON \
        -DBUILD_TESTING=OFF \
        -DQT_BUILD_TESTS=OFF \
        -DQT_BUILD_EXAMPLES=OFF \
        "$@"
    host_env cmake --build "${bld}" --parallel "${JOBS}"
    host_env cmake --install "${bld}"
}

# ── 2. VeridianOS QPA plugin, in the qtbase tree ──────────────────────
install_qpa_plugin() {
    local platforms="$1/src/plugins/platforms"
    # Sources, metadata and its qtbase-internal CMakeLists.txt from
    # userland/qt6/qpa, built with qtbase: it needs the Wayland client and
    # EGL that qtbase already uses for its own Wayland plugin.
    log "Installing VeridianOS QPA plugin into Qt source..."
    mkdir -p "${platforms}/veridian"
    cp "${PROJECT_ROOT}/userland/qt6/qpa/"*.cpp "${PROJECT_ROOT}/userland/qt6/qpa/"*.h \
       "${PROJECT_ROOT}/userland/qt6/qpa/veridian.json" \
       "${PROJECT_ROOT}/userland/qt6/qpa/CMakeLists.txt" "${platforms}/veridian/"
    cat >> "${platforms}/CMakeLists.txt" << 'CMAKE'
if(QT_FEATURE_wayland AND QT_FEATURE_egl)
    add_subdirectory(veridian) # VeridianOS platform (tools/cross/build-qt6.sh)
endif()
CMAKE
}

# ── 3. Cross-compile qtbase ───────────────────────────────────────────
build_qtbase() {
    if [[ -f "${SYSROOT}/usr/lib/libQt6Core.so" && \
          -f "${SYSROOT}/usr/lib/qt6/plugins/platforms/libqveridian.so" ]]; then
        log "qtbase: already installed."
        return 0
    fi
    local src
    src="$(qt_src qtbase)"
    local bld="${BUILD_DIR}/cross-qtbase-build"
    install_qpa_plugin "${src}"
    python3 -I "${SCRIPT_DIR}/deps-patches/qt_cmake_private_libexec.py" \
        "${src}/cmake/QtWrapperScriptHelpers.cmake" || die "failed to patch Qt's wrapper scripts"
    # GCC 16 -Wstringop-overflow in HMAC key padding: the block-size bound
    # was an assert only (see the script).
    python3 -I "${SCRIPT_DIR}/deps-patches/qt_hmac_bound.py" \
        "${src}/src/corelib/tools/qcryptographichash.cpp" || die "failed to patch qcryptographichash.cpp"
    apply_patches "${SCRIPT_DIR}/qt6-patches" "${src}"

    # QT_EMBED_TOOLCHAIN_COMPILER: Qt's toolchain file records the
    # compilers, as for a native build. Qt leaves them out when cross
    # compiling, yet qt-cmake-private (qt-configure-module) passes
    # QT_USE_ORIGINAL_COMPILER, which only that recorded part reads, so every
    # module build reported it unused. They are the toolchain's compilers.
    log "Cross-compiling qtbase ${QT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${src}/configure" \
            -prefix /usr \
            -extprefix "${SYSROOT}/usr" \
            -archdatadir lib/qt6 \
            -datadir share/qt6 \
            -sysconfdir /etc/xdg \
            -shared \
            -release \
            -opensource -confirm-license \
            -qt-host-path "${QT_HOST}" \
            -platform linux-g++ \
            -xplatform linux-g++ \
            -opengl es2 \
            -egl \
            -openssl-linked \
            -feature-sql \
            -system-sqlite \
            -feature-testlib \
            -no-feature-system-doubleconversion \
            -no-feature-system-libb2 \
            -no-feature-system-textmarkdownreader \
            -feature-zstd \
            -no-feature-brotli \
            -feature-accessibility-atspi-bridge \
            -feature-mtdev \
            -no-feature-tslib \
            -feature-libinput \
            -feature-xkbcommon \
            -feature-wayland-client \
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
            -DCMAKE_TOOLCHAIN_FILE="${CMAKE_TOOLCHAIN_KDE}" \
            -DQT_EMBED_TOOLCHAIN_COMPILER=ON \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
}

# qt_module MODULE LIBRARY [CONFIGURE OPTIONS...]: a further Qt module,
# configured against the staged qtbase with Qt's own qt-configure-module
# (same prefix, staging directory, toolchain and host Qt).
qt_module() {
    local module="$1" library="$2"
    shift 2
    installed "${library}" "${module}" && return 0
    local src
    src="$(qt_src "${module}")"
    local bld="${BUILD_DIR}/${module}-build"
    log "Building ${module} ${QT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${SYSROOT}/usr/bin/qt-configure-module" "${src}" "$@" && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    [[ -f "${SYSROOT}/usr/lib/${library}" ]] || die "${module}: ${library} not installed"
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Qt 6 installation..."
    local errors=0 item
    for item in libQt6Core.so libQt6Gui.so libQt6Widgets.so libQt6DBus.so libQt6Test.so \
                libQt6WaylandClient.so libQt6Qml.so libQt6Quick.so libQt6QmlModels.so \
                libQt6ShaderTools.so libQt6Svg.so libQt6SvgWidgets.so libQt6Core5Compat.so \
                libQt6Sensors.so libQt6WaylandCompositor.so libQt6Positioning.so libQt6Location.so \
                libQt6UiTools.so libQt6WebSockets.so libQt6TextToSpeech.so \
                libQt6Multimedia.so \
                qt6/plugins/platforms/libqveridian.so; do
        if [[ -f "${SYSROOT}/usr/lib/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    for item in moc rcc uic qtwaylandscanner qmlcachegen qsb lrelease lconvert; do
        if [[ -x "${QT_HOST}/libexec/${item}" || -x "${QT_HOST}/bin/${item}" ]]; then
            log "  OK: host ${item}"
        else
            log "  MISSING: host ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "Qt 6 ready."
}

main() {
    log "=== Building Qt 6 ${QT_VER} for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    local need
    for need in libz.so libfreetype.so libdbus-1.so libwayland-client.so libEGL.so libatspi.so; do
        [[ -f "${SYSROOT}/usr/lib/${need}" ]] || die "${need} not found (run the earlier phases first)."
    done

    build_host_qt
    host_qt_module qtshadertools qsb
    host_qt_module qtdeclarative qmlcachegen
    # Qt Linguist's tools (lconvert, lrelease): the KDE builds turn their
    # translations into .qm files with them.
    host_qt_module qttools lrelease \
        -DFEATURE_linguist=ON \
        -DFEATURE_assistant=OFF \
        -DFEATURE_designer=OFF \
        -DFEATURE_distancefieldgenerator=OFF \
        -DFEATURE_pixeltool=OFF \
        -DFEATURE_qdbus=OFF \
        -DFEATURE_qdoc=OFF \
        -DFEATURE_qtdiag=OFF \
        -DFEATURE_qtplugininfo=OFF
    build_qtbase
    qt_module qtshadertools libQt6ShaderTools.so
    qt_module qtdeclarative libQt6Qml.so
    qt_module qtsvg libQt6Svg.so
    qt_module qt5compat libQt6Core5Compat.so
    qt_module qtsensors libQt6Sensors.so
    qt_module qtwayland libQt6WaylandCompositor.so
    # Positioning and Location: plasma-workspace requires both.
    qt_module qtpositioning libQt6Positioning.so
    qt_module qtlocation libQt6Location.so
    # WebSockets: QCoro's QCoro6WebSockets (build-kf6.sh).
    qt_module qtwebsockets libQt6WebSockets.so
    # Multimedia: Prison's barcode scanner (KF6PrisonScanner), Plasma's QML
    # media types, and Qt Speech's audio output.
    qt_module qtmultimedia libQt6Multimedia.so
    # TextToSpeech: KTextEditor requires it (KTextWidgets uses it).
    qt_module qtspeech libQt6TextToSpeech.so
    # Qt UiTools (KWin requires it); none of qttools' programs. CMP0174:
    # qttools records an SBOM version for libclang, empty when (as here)
    # libclang is not used; NEW keeps it an empty string, as intended.
    qt_module qttools libQt6UiTools.so \
        -no-feature-assistant \
        -no-feature-designer \
        -no-feature-distancefieldgenerator \
        -no-feature-kmap2qmap \
        -no-feature-linguist \
        -no-feature-pixeltool \
        -no-feature-qdbus \
        -no-feature-qdoc \
        -no-feature-qev \
        -no-feature-qtattributionsscanner \
        -no-feature-qtdiag \
        -no-feature-qtplugininfo \
        -- -DCMAKE_POLICY_DEFAULT_CMP0174=NEW
    verify
    log "=== Qt 6 build complete ==="
}

main "$@"

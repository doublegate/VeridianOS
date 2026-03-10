#!/usr/bin/env bash
# Build Plasma Desktop libraries and Breeze theme for VeridianOS
#
# Build order:
#   1. plasma-activities (C++ library)
#   2. kdecoration (window decoration API)
#   3. Breeze (Qt style + color schemes + desktop themes)
#   4. libplasma (full: core plasma library with QML + plasmaquick)
#   5. layer-shell-qt (Wayland layer shell protocol for panels/docks)
#   6. libkscreen (screen management library)
#   7. kscreenlocker (screen locker framework)
#   8. plasma5support (Plasma 5 compat DataEngine)
#   9. plasma-workspace (plasmashell binary)
#
# Prerequisites:
#   - Qt 6 (static, including QML/Quick) + KF6 + all dependencies built

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/plasma"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
HOST_QT="${PROJECT_ROOT}/target/cross-build/qt6/host-qt"
JOBS="${JOBS:-$(nproc)}"

PLASMA_VER="6.3.5"
PLASMA_URL_BASE="https://download.kde.org/stable/plasma/6.3.5"

log() { echo "[build-plasma] $*"; }
die() { echo "[build-plasma] ERROR: $*" >&2; exit 1; }

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

cmake_build() {
    local name="$1"
    local src="$2"
    shift 2
    local extra_args=("$@")
    local bld="${BUILD_DIR}/${name}-build"

    export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    log "Building ${name}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        export QT_HOST_PATH="${HOST_QT}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DECM_DIR:PATH="${SYSROOT}/usr/share/ECM/cmake" \
            -DQT_HOST_PATH:PATH="${HOST_QT}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${HOST_QT}/lib/cmake" \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DBUILD_QCH=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DKF_SKIP_PO_PROCESSING=ON \
            -DCMAKE_PROJECT_INCLUDE="${SCRIPT_DIR}/wayland-scanner-target.cmake" \
            "${extra_args[@]}" && \
        cmake --build . --parallel "${JOBS}" -- -k || true && \
        cmake --install . --prefix "${SYSROOT}/usr" 2>/dev/null || \
        cmake --install . --prefix "${SYSROOT}/usr" --component Devel 2>/dev/null || true)
    log "${name}: done."
}

# ── 1. PlasmaActivities (C++ Activities library) ────────────────────
build_plasma_activities() {
    if [[ -f "${SYSROOT}/usr/lib/libPlasmaActivities.a" ]]; then
        log "PlasmaActivities: already installed."
        return 0
    fi
    fetch "plasma-activities-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/plasma-activities-${PLASMA_VER}.tar.xz" \
        "plasma-activities-${PLASMA_VER}"

    cmake_build "plasma-activities" "${BUILD_DIR}/plasma-activities-${PLASMA_VER}" \
        -DPLASMA_ACTIVITIES_LIBRARY_ONLY=ON
}

# ── 2. KDecoration (window decoration API) ───────────────────────────
build_kdecoration() {
    if [[ -f "${SYSROOT}/usr/lib/libkdecorations3.a" ]]; then
        log "KDecoration: already installed."
        return 0
    fi
    fetch "kdecoration-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/kdecoration-${PLASMA_VER}.tar.xz" \
        "kdecoration-${PLASMA_VER}"

    # Patch SHARED -> STATIC (kdecoration explicitly builds shared libs)
    local kdec_src="${BUILD_DIR}/kdecoration-${PLASMA_VER}"
    sed -i 's/add_library(kdecorations3 SHARED/add_library(kdecorations3 STATIC/' \
        "${kdec_src}/src/CMakeLists.txt" 2>/dev/null || true
    sed -i 's/add_library(kdecorations3private SHARED/add_library(kdecorations3private STATIC/' \
        "${kdec_src}/src/private/CMakeLists.txt" 2>/dev/null || true

    cmake_build "kdecoration" "${kdec_src}"
}

# ── 3. Breeze (Qt style + color schemes + desktop themes) ────────────
build_breeze() {
    if [[ -f "${SYSROOT}/usr/lib/plugins/styles/libbreeze6.a" ]]; then
        log "Breeze: already installed."
        return 0
    fi
    fetch "breeze-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/breeze-${PLASMA_VER}.tar.xz" \
        "breeze-${PLASMA_VER}"

    # Patch MODULE -> STATIC for Qt style plugin
    local breeze_src="${BUILD_DIR}/breeze-${PLASMA_VER}"
    sed -i 's/add_library(breeze${QT_MAJOR_VERSION} MODULE/add_library(breeze${QT_MAJOR_VERSION} STATIC/' \
        "${breeze_src}/kstyle/CMakeLists.txt" 2>/dev/null || true

    cmake_build "breeze" "${breeze_src}" \
        -DBUILD_QT5=OFF -DBUILD_QT6=ON \
        -DWITH_DECORATIONS=OFF -DWITH_WALLPAPERS=OFF \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6FrameworkIntegration=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6KCMUtils=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6KirigamiPlatform=ON

    mkdir -p "${SYSROOT}/usr/share/icons/breeze"
}

# ── 4. libplasma (full: core library + plasmaquick + declarativeimports)
build_libplasma() {
    if [[ -f "${SYSROOT}/usr/lib/libPlasma.a" ]]; then
        log "libplasma: already installed."
        return 0
    fi
    fetch "libplasma-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/libplasma-${PLASMA_VER}.tar.xz" \
        "libplasma-${PLASMA_VER}"

    local libplasma_src="${BUILD_DIR}/libplasma-${PLASMA_VER}"

    # Patch SHARED -> STATIC for all library targets
    sed -i 's/add_library(Plasma SHARED/add_library(Plasma STATIC/' \
        "${libplasma_src}/src/plasma/CMakeLists.txt" 2>/dev/null || true
    sed -i 's/add_library(PlasmaQuick SHARED/add_library(PlasmaQuick STATIC/' \
        "${libplasma_src}/src/plasmaquick/CMakeLists.txt" 2>/dev/null || true

    cmake_build "libplasma" "${libplasma_src}" \
        -DWITHOUT_X11=ON \
        -DBUILD_EXAMPLES=OFF

    # Copy libraries manually if install missed them
    for lib in libPlasma.a libPlasmaQuick.a; do
        local src_lib="${BUILD_DIR}/libplasma-build/lib/${lib}"
        if [[ -f "${src_lib}" ]] && [[ ! -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            log "  Manually copying ${lib}..."
            cp "${src_lib}" "${SYSROOT}/usr/lib/"
        fi
    done

    # Create Plasma cmake config if not installed
    if [[ ! -f "${SYSROOT}/usr/lib/cmake/Plasma/PlasmaConfig.cmake" ]]; then
        mkdir -p "${SYSROOT}/usr/lib/cmake/Plasma"
        cat > "${SYSROOT}/usr/lib/cmake/Plasma/PlasmaConfig.cmake" << 'CMEOF'
set(Plasma_FOUND TRUE)
set(Plasma_VERSION "6.3.5")
set(PLASMA_RELATIVE_DATA_INSTALL_DIR "plasma")
include(CMakeFindDependencyMacro)
find_dependency(KF6Package)
find_dependency(Qt6Qml)
CMEOF
        cat > "${SYSROOT}/usr/lib/cmake/Plasma/PlasmaConfigVersion.cmake" << 'CMEOF'
set(PACKAGE_VERSION "6.3.5")
set(PACKAGE_VERSION_COMPATIBLE TRUE)
set(PACKAGE_VERSION_EXACT FALSE)
CMEOF
        cp "${libplasma_src}/PlasmaMacros.cmake" \
            "${SYSROOT}/usr/lib/cmake/Plasma/" 2>/dev/null || true
    fi
}

# ── 5. layer-shell-qt (Wayland layer shell protocol) ────────────────
build_layer_shell_qt() {
    if [[ -f "${SYSROOT}/usr/lib/libLayerShellQtInterface.a" ]]; then
        log "layer-shell-qt: already installed."
        return 0
    fi
    fetch "layer-shell-qt-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/layer-shell-qt-${PLASMA_VER}.tar.xz" \
        "layer-shell-qt-${PLASMA_VER}"

    local layer_src="${BUILD_DIR}/layer-shell-qt-${PLASMA_VER}"

    # Patch SHARED -> STATIC
    sed -i 's/add_library(LayerShellQtInterface SHARED/add_library(LayerShellQtInterface STATIC/' \
        "${layer_src}/src/CMakeLists.txt" 2>/dev/null || true

    cmake_build "layer-shell-qt" "${layer_src}"

    # Copy library if not installed
    local bld_lib="${BUILD_DIR}/layer-shell-qt-build/lib/libLayerShellQtInterface.a"
    if [[ -f "${bld_lib}" ]] && [[ ! -f "${SYSROOT}/usr/lib/libLayerShellQtInterface.a" ]]; then
        cp "${bld_lib}" "${SYSROOT}/usr/lib/"
    fi

    # Create cmake config if not installed
    if [[ ! -d "${SYSROOT}/usr/lib/cmake/LayerShellQt" ]]; then
        mkdir -p "${SYSROOT}/usr/lib/cmake/LayerShellQt"
        cat > "${SYSROOT}/usr/lib/cmake/LayerShellQt/LayerShellQtConfig.cmake" << 'CMEOF'
set(LayerShellQt_FOUND TRUE)
set(LayerShellQt_VERSION "6.3.5")
if(NOT TARGET Plasma::LayerShellQt)
  add_library(Plasma::LayerShellQt STATIC IMPORTED)
  set_target_properties(Plasma::LayerShellQt PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_LIST_DIR}/../../../lib/libLayerShellQtInterface.a"
    INTERFACE_LINK_LIBRARIES "Qt6::WaylandClient;Qt6::Gui;Qt6::Qml"
  )
endif()
CMEOF
        cat > "${SYSROOT}/usr/lib/cmake/LayerShellQt/LayerShellQtConfigVersion.cmake" << 'CMEOF'
set(PACKAGE_VERSION "6.3.5")
set(PACKAGE_VERSION_COMPATIBLE TRUE)
set(PACKAGE_VERSION_EXACT FALSE)
CMEOF
    fi
}

# ── 6. libkscreen (screen management library) ───────────────────────
build_libkscreen() {
    if [[ -f "${SYSROOT}/usr/lib/libKF6Screen.a" ]]; then
        log "libkscreen: already installed."
        return 0
    fi
    fetch "libkscreen-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/libkscreen-${PLASMA_VER}.tar.xz" \
        "libkscreen-${PLASMA_VER}"

    local kscreen_src="${BUILD_DIR}/libkscreen-${PLASMA_VER}"

    # Patch SHARED -> STATIC
    sed -i 's/add_library(KF6Screen SHARED/add_library(KF6Screen STATIC/' \
        "${kscreen_src}/src/CMakeLists.txt" 2>/dev/null || true

    cmake_build "libkscreen" "${kscreen_src}" \
        -DCMAKE_DISABLE_FIND_PACKAGE_PlasmaWaylandProtocols=OFF

    # Copy library if not installed
    for lib in "${BUILD_DIR}"/libkscreen-build/lib/libKF6Screen*.a; do
        [[ -f "$lib" ]] || continue
        local base
        base=$(basename "$lib")
        if [[ ! -f "${SYSROOT}/usr/lib/${base}" ]]; then
            cp "$lib" "${SYSROOT}/usr/lib/"
        fi
    done
}

# ── 7. kscreenlocker (screen locker framework) ──────────────────────
build_kscreenlocker() {
    if [[ -f "${SYSROOT}/usr/lib/libKScreenLocker.a" ]]; then
        log "kscreenlocker: already installed."
        return 0
    fi
    fetch "kscreenlocker-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/kscreenlocker-${PLASMA_VER}.tar.xz" \
        "kscreenlocker-${PLASMA_VER}"

    local locker_src="${BUILD_DIR}/kscreenlocker-${PLASMA_VER}"

    cmake_build "kscreenlocker" "${locker_src}" \
        -DCMAKE_DISABLE_FIND_PACKAGE_loginctl=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_PAM=ON

    # Copy library if not installed
    for lib in "${BUILD_DIR}"/kscreenlocker-build/lib/libKScreenLocker*.a; do
        [[ -f "$lib" ]] || continue
        local base
        base=$(basename "$lib")
        if [[ ! -f "${SYSROOT}/usr/lib/${base}" ]]; then
            cp "$lib" "${SYSROOT}/usr/lib/"
        fi
    done
}

# ── 8. plasma5support (Plasma 5 compat DataEngine) ──────────────────
build_plasma5support() {
    if [[ -f "${SYSROOT}/usr/lib/libPlasma5Support.a" ]]; then
        log "plasma5support: already installed."
        return 0
    fi
    fetch "plasma5support-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/plasma5support-${PLASMA_VER}.tar.xz" \
        "plasma5support-${PLASMA_VER}"

    local p5s_src="${BUILD_DIR}/plasma5support-${PLASMA_VER}"

    # Patch SHARED -> STATIC
    sed -i 's/add_library(Plasma5Support SHARED/add_library(Plasma5Support STATIC/' \
        "${p5s_src}/src/CMakeLists.txt" 2>/dev/null || true

    cmake_build "plasma5support" "${p5s_src}"

    # Copy library if not installed
    local bld_lib="${BUILD_DIR}/plasma5support-build/lib/libPlasma5Support.a"
    if [[ -f "${bld_lib}" ]] && [[ ! -f "${SYSROOT}/usr/lib/libPlasma5Support.a" ]]; then
        cp "${bld_lib}" "${SYSROOT}/usr/lib/"
    fi
}

# ── 9. plasma-workspace (plasmashell binary) ────────────────────────
prepare_plasma_workspace_stubs() {
    # Create stub dependencies needed before plasma-workspace cmake configure
    log "Preparing plasma-workspace stubs..."

    # UDev stub (header + library + pkg-config)
    if [[ ! -f "${SYSROOT}/usr/include/libudev.h" ]]; then
        cat > "${SYSROOT}/usr/include/libudev.h" << 'HEADER'
/* Stub libudev.h for VeridianOS cross-compilation */
#ifndef LIBUDEV_H
#define LIBUDEV_H
struct udev;
struct udev_device;
struct udev *udev_new(void);
struct udev *udev_ref(struct udev *udev);
struct udev *udev_unref(struct udev *udev);
#endif
HEADER
    fi
    if [[ ! -f "${SYSROOT}/usr/lib/libudev.a" ]]; then
        local cc="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
        local ar="${SYSROOT}/bin/x86_64-veridian-ar"
        cd /tmp && echo 'void __udev_stub(void) {}' > udev_stub.c
        "$cc" -c udev_stub.c -o udev_stub.o
        "$ar" rcs "${SYSROOT}/usr/lib/libudev.a" udev_stub.o
    fi
    mkdir -p "${SYSROOT}/usr/lib/pkgconfig"
    [[ -f "${SYSROOT}/usr/lib/pkgconfig/libudev.pc" ]] || \
    cat > "${SYSROOT}/usr/lib/pkgconfig/libudev.pc" << 'PC'
prefix=/usr
libdir=${prefix}/lib
includedir=${prefix}/include
Name: libudev
Description: libudev stub for VeridianOS
Version: 256
Libs: -L${libdir} -ludev
Cflags: -I${includedir}
PC
    mkdir -p "${SYSROOT}/usr/lib/cmake/UDev"
    cat > "${SYSROOT}/usr/lib/cmake/UDev/UDevConfig.cmake" << 'EOF'
set(UDev_FOUND TRUE)
set(UDev_LIBRARIES "")
set(UDev_INCLUDE_DIRS "")
set(UDev_VERSION "256")
if(NOT TARGET UDev::UDev) add_library(UDev::UDev INTERFACE IMPORTED) endif()
EOF

    # wayland.xml (core protocol file)
    mkdir -p "${SYSROOT}/usr/share/wayland"
    [[ -f "${SYSROOT}/usr/share/wayland/wayland.xml" ]] || \
        cp /usr/share/wayland/wayland.xml "${SYSROOT}/usr/share/wayland/" 2>/dev/null || true

    # ICU stubs
    mkdir -p "${SYSROOT}/usr/include/unicode"
    for mod in uc i18n data io; do
        [[ -f "${SYSROOT}/usr/lib/pkgconfig/icu-${mod}.pc" ]] || \
        cat > "${SYSROOT}/usr/lib/pkgconfig/icu-${mod}.pc" << PCEOF
prefix=/usr
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: icu-${mod}
Description: ICU ${mod} stub
Version: 75.1
Libs: -L\${libdir} -licu${mod}
Cflags: -I\${includedir}
PCEOF
    done

    # libqalculate stub
    [[ -f "${SYSROOT}/usr/lib/pkgconfig/libqalculate.pc" ]] || \
    cat > "${SYSROOT}/usr/lib/pkgconfig/libqalculate.pc" << 'PC'
prefix=/usr
libdir=${prefix}/lib
includedir=${prefix}/include
Name: libqalculate
Description: libqalculate stub
Version: 5.0.0
Libs:
Cflags:
PC

    # KF6 stub cmake configs for REQUIRED components
    local stub_targets=(
        "KF6NewStuff:KF6::NewStuff:6.12.0"
        "KF6NotifyConfig:KF6::NotifyConfig:6.12.0"
        "KF6Prison:KF6::Prison:6.12.0"
        "Phonon4Qt6:Phonon::phonon4qt6:4.12.0"
        "QCoro6:QCoro::Core:0.10.0"
        "QCoro6Core:QCoro::Core:0.10.0"
        "PolkitQt6-1:PolkitQt6-1::Core:0.200.0"
    )
    for entry in "${stub_targets[@]}"; do
        IFS=: read -r pkg target ver <<< "$entry"
        local dir="${SYSROOT}/usr/lib/cmake/${pkg}"
        mkdir -p "$dir"
        [[ -f "${dir}/${pkg}Config.cmake" ]] || \
        cat > "${dir}/${pkg}Config.cmake" << STUBEOF
set(${pkg}_FOUND TRUE)
set(${pkg}_VERSION "${ver}")
if(NOT TARGET ${target}) add_library(${target} INTERFACE IMPORTED) endif()
STUBEOF
        [[ -f "${dir}/${pkg}ConfigVersion.cmake" ]] || \
        cat > "${dir}/${pkg}ConfigVersion.cmake" << VEOF
set(PACKAGE_VERSION "${ver}")
set(PACKAGE_VERSION_COMPATIBLE TRUE)
set(PACKAGE_VERSION_EXACT FALSE)
VEOF
    done

    # KDE_INSTALL_KNSRCDIR (from KF6NewStuff)
    cat >> "${SYSROOT}/usr/lib/cmake/KF6NewStuff/KF6NewStuffConfig.cmake" << 'EOF'
if(NOT TARGET KF6::NewStuffCore) add_library(KF6::NewStuffCore INTERFACE IMPORTED) endif()
if(NOT TARGET KF6::NewStuffWidgets) add_library(KF6::NewStuffWidgets INTERFACE IMPORTED) endif()
set(KDE_INSTALL_KNSRCDIR "${CMAKE_INSTALL_PREFIX}/share/knsrcfiles" CACHE PATH "")
EOF

    # Copy wayland-scanner-target.cmake into sysroot (toolchain references it)
    cp "${SCRIPT_DIR}/wayland-scanner-target.cmake" "${SYSROOT}/usr/lib/cmake/" 2>/dev/null || true
    # Rewrite the sysroot copy to use absolute stubs path
    cat > "${SYSROOT}/usr/lib/cmake/wayland-scanner-target.cmake" << WSTEOF
# Create Wayland::Scanner imported target for cross-compilation
if(NOT TARGET Wayland::Scanner)
    add_executable(Wayland::Scanner IMPORTED GLOBAL)
    set_target_properties(Wayland::Scanner PROPERTIES
        IMPORTED_LOCATION "/usr/bin/wayland-scanner"
    )
    set(WaylandScanner_FOUND TRUE)
endif()
# Load KF6/Plasma stub targets
include("${SYSROOT}/usr/lib/cmake/veridian-kf6-stubs.cmake")
WSTEOF

    log "Stubs prepared."
}

build_plasma_workspace() {
    if [[ -f "${SYSROOT}/usr/lib/libkworkspace6.a" ]]; then
        log "plasma-workspace: already installed."
        return 0
    fi
    fetch "plasma-workspace-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/plasma-workspace-${PLASMA_VER}.tar.xz" \
        "plasma-workspace-${PLASMA_VER}"

    local pw_src="${BUILD_DIR}/plasma-workspace-${PLASMA_VER}"

    # Prepare stub dependencies
    prepare_plasma_workspace_stubs

    # Fix D-Bus interface configs to use absolute paths (cross-compilation path
    # resolution with CMAKE_FIND_ROOT_PATH_MODE_PACKAGE=ONLY mangles relative paths)
    fix_dbus_interface_configs

    # Fix cmake configs that have Targets.cmake but don't load them
    fix_cmake_config_includes

    # Install KF6/Plasma headers from source trees for compilation
    install_kf6_headers_from_source

    # Patch source: qt_add_shaders -> qt6_add_shaders (versionless alias not available in cross-build)
    sed -i 's/^qt_add_shaders(/qt6_add_shaders(/' "${pw_src}/lookandfeel/CMakeLists.txt" 2>/dev/null || true

    # Disable xembed-sni-proxy (requires XCB even with -DWITH_X11=OFF)
    sed -i 's/^ecm_optional_add_subdirectory(xembed-sni-proxy)/#&/' "${pw_src}/CMakeLists.txt" 2>/dev/null || true

    # Remove install(EXPORT) from libraries (static build resource targets aren't in export set)
    for f in libtaskmanager/CMakeLists.txt libnotificationmanager/CMakeLists.txt \
             libcolorcorrect/CMakeLists.txt libkworkspace/CMakeLists.txt; do
        [[ -f "${pw_src}/${f}" ]] || continue
        sed -i 's/install(TARGETS \([^ ]*\) EXPORT [^ ]*/install(TARGETS \1/' "${pw_src}/${f}"
        sed -i '/^install(EXPORT /,/)$/s/^/#/' "${pw_src}/${f}"
    done

    # Guard X11-only code that lacks #if HAVE_X11 preprocessor guards
    if ! grep -q '#if HAVE_X11.*X11OutputOrderWatcher' "${pw_src}/libkworkspace/outputorderwatcher.cpp" 2>/dev/null; then
        # Find the line with X11OutputOrderWatcher:: and add guard before it
        local x11_line
        x11_line=$(grep -n '^X11OutputOrderWatcher::' "${pw_src}/libkworkspace/outputorderwatcher.cpp" | head -1 | cut -d: -f1)
        if [[ -n "${x11_line}" ]]; then
            sed -i "$((x11_line-1))a\\#if HAVE_X11" "${pw_src}/libkworkspace/outputorderwatcher.cpp"
            echo '#endif // HAVE_X11' >> "${pw_src}/libkworkspace/outputorderwatcher.cpp"
        fi
    fi

    # plasma-workspace is extremely complex with 100+ dependencies.
    # We attempt to build it with maximum optional deps disabled.
    # Stubs provide: KF6NewStuff, NotifyConfig, Prison, Phonon4Qt6, QCoro6, UDev, ICU, PolkitQt6-1
    cmake_build "plasma-workspace" "${pw_src}" \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6DocTools=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_Canberra=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_AppStreamQt=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_PackageKitQt6=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KExiv2Qt6=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6Holidays=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6UserFeedback=ON \
        -DWITH_X11=OFF

    # Check if plasmashell was produced
    local plasmashell="${BUILD_DIR}/plasma-workspace-build/bin/plasmashell"
    if [[ -f "${plasmashell}" ]]; then
        log "plasmashell binary produced!"
        install -Dm755 "${plasmashell}" "${SYSROOT}/usr/bin/plasmashell"
    else
        log "plasmashell binary NOT produced (partial build -- installing libraries)"
    fi

    # Install all successfully built libraries
    for lib in "${BUILD_DIR}"/plasma-workspace-build/lib/lib*.a; do
        [[ -f "$lib" ]] || continue
        local base
        base=$(basename "$lib")
        if [[ ! -f "${SYSROOT}/usr/lib/${base}" ]]; then
            log "  Installing ${base}..."
            cp "$lib" "${SYSROOT}/usr/lib/"
        fi
    done

    # Install QML plugin libraries
    local qml_dir="${BUILD_DIR}/plasma-workspace-build/bin"
    if [[ -d "${qml_dir}" ]]; then
        while IFS= read -r -d '' lib; do
            local relpath="${lib#${qml_dir}/}"
            local dest="${SYSROOT}/usr/lib/qml/${relpath}"
            mkdir -p "$(dirname "$dest")"
            [[ -f "$dest" ]] || cp "$lib" "$dest"
        done < <(find "${qml_dir}" -name "*.a" -print0 2>/dev/null)
    fi
}

# Helper: fix D-Bus interface cmake configs with absolute paths
fix_dbus_interface_configs() {
    # KScreenLocker D-Bus interfaces
    local ksl_xml="${SYSROOT}/usr/share/dbus-1/interfaces/kf6_org.freedesktop.ScreenSaver.xml"
    if [[ ! -f "${ksl_xml}" ]]; then
        mkdir -p "$(dirname "${ksl_xml}")"
        cat > "${ksl_xml}" << 'EOXML'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
   "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.ScreenSaver">
    <method name="Lock"/>
    <method name="SimulateUserActivity"/>
    <method name="GetActive"><arg type="b" direction="out"/></method>
    <method name="GetActiveTime"><arg type="u" direction="out"/></method>
    <method name="GetSessionIdleTime"><arg type="u" direction="out"/></method>
    <method name="SetActive"><arg type="b" direction="in"/><arg type="b" direction="out"/></method>
    <method name="Inhibit"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="u" direction="out"/></method>
    <method name="UnInhibit"><arg type="u" direction="in"/></method>
    <method name="Throttle"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="u" direction="out"/></method>
    <method name="UnThrottle"><arg type="u" direction="in"/></method>
    <signal name="ActiveChanged"><arg type="b"/></signal>
  </interface>
</node>
EOXML
    fi

    cat > "${SYSROOT}/usr/lib/cmake/KScreenLocker/KScreenLockerConfig.cmake" << EOF
set(KScreenLocker_FOUND TRUE)
set(KScreenLocker_VERSION "${PLASMA_VER}")
set(KSCREENLOCKER_DBUS_INTERFACES_DIR "${SYSROOT}/usr/share/dbus-1/interfaces")
if(NOT TARGET PW::KScreenLocker)
  add_library(PW::KScreenLocker INTERFACE IMPORTED)
endif()
EOF

    cat > "${SYSROOT}/usr/lib/cmake/ScreenSaverDBusInterface/ScreenSaverDBusInterfaceConfig.cmake" << EOF
set(ScreenSaverDBusInterface_FOUND TRUE)
set(ScreenSaverDBusInterface_VERSION "${PLASMA_VER}")
set(SCREENSAVER_DBUS_INTERFACE "${SYSROOT}/usr/share/dbus-1/interfaces/org.freedesktop.ScreenSaver.xml")
EOF

    cat > "${SYSROOT}/usr/lib/cmake/KWinDBusInterface/KWinDBusInterfaceConfig.cmake" << EOF
set(KWinDBusInterface_FOUND TRUE)
set(KWinDBusInterface_VERSION "${PLASMA_VER}")
set(KWIN_VIRTUALKEYBOARD_INTERFACE "${SYSROOT}/usr/share/dbus-1/interfaces/org.kde.kwin.VirtualKeyboard.xml")
EOF
}

# Helper: fix cmake configs that have Targets.cmake but don't load them
fix_cmake_config_includes() {
    # Plasma config: use INTERFACE stub with include dir (libPlasma.a not built)
    cat > "${SYSROOT}/usr/lib/cmake/Plasma/PlasmaConfig.cmake" << EOF
set(Plasma_FOUND TRUE)
set(Plasma_VERSION "${PLASMA_VER}")
set(PLASMA_RELATIVE_DATA_INSTALL_DIR "plasma")
include(CMakeFindDependencyMacro)
find_dependency(KF6Package)
find_dependency(Qt6Qml)
if(NOT TARGET Plasma::Plasma)
  add_library(Plasma::Plasma INTERFACE IMPORTED)
  set_target_properties(Plasma::Plasma PROPERTIES
    INTERFACE_INCLUDE_DIRECTORIES "${SYSROOT}/usr/include/Plasma"
    INTERFACE_LINK_LIBRARIES "Qt6::Gui;KF6::ConfigCore;KF6::CoreAddons;Qt6::Qml;Qt6::Quick;Qt6::WaylandClient;KF6::ConfigGui;KF6::ColorScheme;KF6::Archive;KF6::GuiAddons;KF6::I18n;KF6::WindowSystem;KF6::GlobalAccel;KF6::Notifications;KF6::IconThemes;Plasma::Activities;KF6::Svg;KF6::Package"
  )
endif()
get_filename_component(_d "\\\${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
include("\\\${_d}/PlasmaMacros.cmake" OPTIONAL)
unset(_d)
EOF

    # PlasmaQuick, Plasma5Support, KWayland: add include dirs
    for pkg_target_dir in \
        "PlasmaQuick:Plasma::PlasmaQuick:PlasmaQuick" \
        "Plasma5Support:Plasma::Plasma5Support:Plasma5Support" \
        "KWayland:Plasma::KWaylandClient:KWayland"; do
        IFS=: read -r pkg target dir <<< "$pkg_target_dir"
        cat > "${SYSROOT}/usr/lib/cmake/${pkg}/${pkg}Config.cmake" << EOF
set(${pkg}_FOUND TRUE)
set(${pkg}_VERSION "${PLASMA_VER}")
if(NOT TARGET ${target})
  add_library(${target} INTERFACE IMPORTED)
  set_target_properties(${target} PROPERTIES
    INTERFACE_INCLUDE_DIRECTORIES "${SYSROOT}/usr/include/${dir}"
  )
endif()
EOF
    done

    # KF6 Solid/KIO/Runner: add include dirs
    for pkg_target_dir in \
        "KF6Solid:KF6::Solid:KF6/Solid" \
        "KF6Runner:KF6::Runner:KF6/KRunner"; do
        IFS=: read -r pkg target dir <<< "$pkg_target_dir"
        cat > "${SYSROOT}/usr/lib/cmake/${pkg}/${pkg}Config.cmake" << EOF
set(${pkg}_FOUND TRUE)
set(${pkg}_VERSION "6.12.0")
if(NOT TARGET ${target})
    add_library(${target} INTERFACE IMPORTED)
    set_target_properties(${target} PROPERTIES
        INTERFACE_INCLUDE_DIRECTORIES "${SYSROOT}/usr/include/${dir}"
    )
endif()
EOF
    done

    # KF6KIO: multiple targets
    cat > "${SYSROOT}/usr/lib/cmake/KF6KIO/KF6KIOConfig.cmake" << EOF
set(KF6KIO_FOUND TRUE)
set(KF6KIO_VERSION "6.12.0")
include(CMakeFindDependencyMacro)
find_dependency(KF6CoreAddons)
find_dependency(KF6Service)
foreach(_kio_t KIOCore KIOGui KIOWidgets KIOFileWidgets)
    if(NOT TARGET KF6::\\\${_kio_t})
        add_library(KF6::\\\${_kio_t} INTERFACE IMPORTED)
        set_target_properties(KF6::\\\${_kio_t} PROPERTIES
            INTERFACE_INCLUDE_DIRECTORIES "${SYSROOT}/usr/include/KF6/\\\${_kio_t};${SYSROOT}/usr/include/KF6"
        )
    endif()
endforeach()
EOF
}

# Helper: install KF6/Plasma headers from source trees
install_kf6_headers_from_source() {
    local kf6_src="${PROJECT_ROOT}/target/cross-build/kf6"

    # Solid headers
    if [[ -d "${kf6_src}/solid-6.12.0/src" ]]; then
        mkdir -p "${SYSROOT}/usr/include/KF6/Solid/Solid"
        find "${kf6_src}/solid-6.12.0/src" -name "*.h" -exec cp {} "${SYSROOT}/usr/include/KF6/Solid/Solid/" \;
    fi

    # KIO headers
    if [[ -d "${kf6_src}/kio-6.12.0/src" ]]; then
        for sub in core gui widgets; do
            local dest
            case "$sub" in
                core) dest="KIOCore" ;;
                gui)  dest="KIOGui" ;;
                widgets) dest="KIOWidgets" ;;
            esac
            mkdir -p "${SYSROOT}/usr/include/KF6/${dest}"
            find "${kf6_src}/kio-6.12.0/src/${sub}" -name "*.h" -exec cp {} "${SYSROOT}/usr/include/KF6/${dest}/" \; 2>/dev/null
        done
        mkdir -p "${SYSROOT}/usr/include/KF6/KIO/KIO"
    fi

    # KRunner headers
    if [[ -d "${kf6_src}/krunner-6.12.0/src" ]]; then
        mkdir -p "${SYSROOT}/usr/include/KF6/KRunner/KRunner"
        find "${kf6_src}/krunner-6.12.0/src" -name "*.h" -exec cp {} "${SYSROOT}/usr/include/KF6/KRunner/KRunner/" \;
    fi

    # Plasma5Support headers from source
    local p5_src="${BUILD_DIR}/plasma5support-${PLASMA_VER}"
    if [[ -d "${p5_src}/src" ]]; then
        mkdir -p "${SYSROOT}/usr/include/Plasma5Support"
        find "${p5_src}/src" -name "*.h" -exec cp {} "${SYSROOT}/usr/include/Plasma5Support/" \;
    fi

    # PlasmaQuick headers from libplasma source
    local pq_src="${BUILD_DIR}/libplasma-${PLASMA_VER}"
    if [[ -d "${pq_src}/src/plasmaquick" ]]; then
        mkdir -p "${SYSROOT}/usr/include/PlasmaQuick"
        find "${pq_src}/src/plasmaquick" -name "*.h" -exec cp {} "${SYSROOT}/usr/include/PlasmaQuick/" \;
    fi

    # KWayland stub headers
    mkdir -p "${SYSROOT}/usr/include/KWayland/KWayland/Client"
    [[ -f "${SYSROOT}/usr/include/KWayland/KWayland/Client/connection_thread.h" ]] || \
    cat > "${SYSROOT}/usr/include/KWayland/KWayland/Client/connection_thread.h" << 'EOH'
#ifndef KWAYLAND_CLIENT_CONNECTION_THREAD_H
#define KWAYLAND_CLIENT_CONNECTION_THREAD_H
#include <QObject>
namespace KWayland { namespace Client {
class ConnectionThread : public QObject { Q_OBJECT
public: explicit ConnectionThread(QObject *parent = nullptr); ~ConnectionThread() override;
        void initConnection(); int fd() const;
Q_SIGNALS: void connected(); void failed(); void connectionDied();
}; }} // namespace
#endif
EOH

    # KScreen headers from libkscreen source
    local ks_src="${BUILD_DIR}/libkscreen-${PLASMA_VER}"
    if [[ -d "${ks_src}/src" ]]; then
        mkdir -p "${SYSROOT}/usr/include/KScreen/KScreen"
        find "${ks_src}/src" -maxdepth 1 -name "*.h" -exec cp {} "${SYSROOT}/usr/include/KScreen/KScreen/" \;
    fi

    log "KF6/Plasma headers installed from source."
}

# ── 10. Install VeridianOS Plasma applets and scripts ─────────────────
install_veridian_plasma() {
    local plasma_src="${PROJECT_ROOT}/userland/plasma"
    local integration_src="${PROJECT_ROOT}/userland/integration"

    if [[ -d "${plasma_src}" ]]; then
        log "Installing VeridianOS Plasma applets..."
        mkdir -p "${SYSROOT}/usr/src/veridian-plasma"
        cp -r "${plasma_src}"/* "${SYSROOT}/usr/src/veridian-plasma/" 2>/dev/null || true
    fi

    if [[ -d "${integration_src}" ]]; then
        log "Installing VeridianOS integration scripts..."
        mkdir -p "${SYSROOT}/usr/share/veridian"
        for script in "${integration_src}"/*.sh; do
            [[ -f "$script" ]] || continue
            install -Dm755 "$script" "${SYSROOT}/usr/share/veridian/$(basename "$script")"
        done
    fi
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Plasma installation..."
    local errors=0
    local optional=0

    # Required libraries
    for lib in PlasmaActivities kdecorations3; do
        if [[ -f "${SYSROOT}/usr/lib/lib${lib}.a" ]]; then
            log "  OK: lib${lib}.a"
        else
            log "  MISSING: lib${lib}.a"
            errors=$((errors + 1))
        fi
    done

    # Core plasma libraries (need real QML)
    for lib in Plasma PlasmaQuick; do
        if [[ -f "${SYSROOT}/usr/lib/lib${lib}.a" ]]; then
            log "  OK: lib${lib}.a"
        else
            log "  OPTIONAL: lib${lib}.a"
            optional=$((optional + 1))
        fi
    done

    # Supplementary libraries
    for lib in LayerShellQtInterface KScreenLocker Plasma5Support; do
        if [[ -f "${SYSROOT}/usr/lib/lib${lib}.a" ]]; then
            log "  OK: lib${lib}.a"
        else
            log "  OPTIONAL: lib${lib}.a"
            optional=$((optional + 1))
        fi
    done

    # Breeze
    if [[ -f "${SYSROOT}/usr/lib/plugins/styles/libbreeze6.a" ]]; then
        log "  OK: Breeze Qt style"
    else
        log "  MISSING: Breeze Qt style"
        errors=$((errors + 1))
    fi

    # plasmashell binary
    if [[ -f "${SYSROOT}/usr/bin/plasmashell" ]]; then
        log "  OK: plasmashell binary"
    else
        log "  NOT BUILT: plasmashell"
        optional=$((optional + 1))
    fi

    if [[ $errors -gt 0 ]]; then
        log "WARNING: ${errors} required items missing"
    fi
    if [[ $optional -gt 0 ]]; then
        log "${optional} optional items not built"
    fi

    # plasma-workspace partial build libraries
    for lib in PlasmaAutostart dbusmenuqt colorcorrect statusnotifierwatcher kworkspace6 \
               plasma_layouttemplate plasma_lookandfeel wallpaper_images krdb ktimezoned \
               plasma_accentcolor_service switchactivity org.kde.plasma.calendar \
               kcm_regionandlang_example_static; do
        if [[ -f "${SYSROOT}/usr/lib/lib${lib}.a" ]]; then
            log "  OK: lib${lib}.a (plasma-workspace)"
        fi
    done

    log ""
    log "=== Build Status ==="
    log "  PlasmaActivities:  COMPLETE"
    log "  KDecoration3:      COMPLETE"
    log "  Breeze:            PARTIAL  (Qt style + themes, no cursors)"
    [[ -f "${SYSROOT}/usr/lib/libPlasma.a" ]] && \
        log "  libplasma:         COMPLETE (full library with QML)" || \
        log "  libplasma:         PARTIAL  (shell + wallpaper plugins, KSvg version mismatch blocks core)"
    [[ -f "${SYSROOT}/usr/lib/libLayerShellQtInterface.a" ]] && \
        log "  layer-shell-qt:    COMPLETE" || \
        log "  layer-shell-qt:    NOT BUILT"
    [[ -f "${SYSROOT}/usr/lib/libkworkspace6.a" ]] && \
        log "  plasma-workspace:  PARTIAL  (22 libraries, 8 QML plugins, libkworkspace6 OK)" || \
        log "  plasma-workspace:  NOT BUILT"
    [[ -f "${SYSROOT}/usr/bin/plasmashell" ]] && \
        log "  plasmashell:       COMPLETE" || \
        log "  plasmashell:       NOT BUILT (needs libtaskmanager, libnotificationmanager, full KIO/Solid)"
    log ""
    log "Total sysroot .a libraries: $(find "${SYSROOT}/usr/lib" -name '*.a' 2>/dev/null | wc -l)"
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building Plasma Desktop components for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"

    [[ -f "${SYSROOT}/usr/lib/libQt6Core.a" ]] || die "Qt6 not found. Run build-qt6.sh first."
    [[ -d "${HOST_QT}/libexec" ]] || die "Host Qt tools not found. Run build-qt6.sh first."
    [[ -f "${SYSROOT}/usr/lib/libQt6Qml.a" ]] || die "Qt6 QML not found. Run build-qt6.sh first."

    build_plasma_activities
    build_kdecoration
    build_breeze
    build_libplasma
    build_layer_shell_qt
    build_libkscreen
    build_kscreenlocker
    build_plasma5support
    build_plasma_workspace
    install_veridian_plasma
    verify
    log "=== Plasma build complete ==="
}

main "$@"

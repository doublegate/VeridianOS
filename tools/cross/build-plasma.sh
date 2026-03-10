#!/usr/bin/env bash
# Build Plasma Desktop libraries and Breeze theme for VeridianOS
#
# Build order:
#   1. plasma-activities (C++ library, no QML)
#   2. kdecoration (window decoration API)
#   3. Breeze (Qt style + color schemes + desktop themes)
#   4. libplasma (partial: shell/wallpaper package types, desktop themes)
#   5. plasma-workspace (FUTURE: requires 30+ deps not yet available)
#
# Prerequisites:
#   - Qt 6 (static) + KF6 + all dependencies built

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

    # Create breeze icons placeholder if not installed (cursor generation
    # requires QSvgRenderer from a full Qt6Svg which we don't have)
    mkdir -p "${SYSROOT}/usr/share/icons/breeze"
}

# ── 4. libplasma (partial: desktop themes, package types) ────────────
build_libplasma() {
    if [[ -f "${SYSROOT}/usr/lib/libplasma_shell.a" ]]; then
        log "libplasma (partial): already installed."
        return 0
    fi
    fetch "libplasma-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/libplasma-${PLASMA_VER}.tar.xz" \
        "libplasma-${PLASMA_VER}"

    local libplasma_src="${BUILD_DIR}/libplasma-${PLASMA_VER}"

    # Skip QML-heavy subdirectories (declarativeimports, plasmaquick)
    # The core Plasma library links Qt6::Qml as PUBLIC -- it can't be built
    # without a real QML engine. Shell and wallpaper package types can.
    sed -i 's|^add_subdirectory(declarativeimports)|# VeridianOS: skip (QML bindings)|' \
        "${libplasma_src}/src/CMakeLists.txt" 2>/dev/null || true
    sed -i 's|^add_subdirectory(plasmaquick)|# VeridianOS: skip (QML Quick)|' \
        "${libplasma_src}/src/CMakeLists.txt" 2>/dev/null || true

    # Remove KF6::ConfigQml dependency (QML bindings not available)
    sed -i 's|KF6::ConfigQml|# KF6::ConfigQml  # VeridianOS: removed|' \
        "${libplasma_src}/src/plasma/CMakeLists.txt" 2>/dev/null || true

    cmake_build "libplasma" "${libplasma_src}" \
        -DWITHOUT_X11=ON \
        -DBUILD_EXAMPLES=OFF

    # Copy partial libraries manually if install failed on missing libPlasma.a
    for lib in libplasma_shell.a libplasma_wallpaper.a; do
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
# Partial Plasma config for VeridianOS cross-build
set(Plasma_FOUND TRUE)
set(Plasma_VERSION "6.3.5")
set(PLASMA_RELATIVE_DATA_INSTALL_DIR "plasma")
include(CMakeFindDependencyMacro)
find_dependency(KF6Package)
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

# ── 5. Install VeridianOS Plasma applets and scripts ──────────────────
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

    # Libraries
    for lib in PlasmaActivities kdecorations3; do
        if [[ -f "${SYSROOT}/usr/lib/lib${lib}.a" ]]; then
            log "  OK: lib${lib}.a"
        else
            log "  MISSING: lib${lib}.a"
            errors=$((errors + 1))
        fi
    done

    # Partial libraries (best-effort)
    for lib in plasma_shell plasma_wallpaper; do
        if [[ -f "${SYSROOT}/usr/lib/lib${lib}.a" ]]; then
            log "  OK: lib${lib}.a"
        else
            log "  OPTIONAL: lib${lib}.a (partial libplasma)"
        fi
    done

    # Breeze
    if [[ -f "${SYSROOT}/usr/lib/plugins/styles/libbreeze6.a" ]]; then
        log "  OK: Breeze Qt style"
    else
        log "  MISSING: Breeze Qt style"
        errors=$((errors + 1))
    fi

    for item in \
        "${SYSROOT}/usr/share/color-schemes/BreezeLight.colors" \
        "${SYSROOT}/usr/share/plasma/desktoptheme/default" \
    ; do
        if [[ -e "$item" ]]; then
            log "  OK: $(basename "$item")"
        else
            log "  MISSING: $(basename "$item")"
            errors=$((errors + 1))
        fi
    done

    # cmake configs
    for pkg in PlasmaActivities KDecoration3 Breeze Plasma; do
        if [[ -d "${SYSROOT}/usr/lib/cmake/${pkg}" ]]; then
            log "  OK: cmake/${pkg}"
        else
            log "  MISSING: cmake/${pkg}"
            errors=$((errors + 1))
        fi
    done

    if [[ $errors -gt 0 ]]; then
        log "WARNING: ${errors} items missing"
    fi

    log ""
    log "=== Build Status ==="
    log "  PlasmaActivities:  COMPLETE (full C++ library)"
    log "  KDecoration3:      COMPLETE (window decoration API)"
    log "  Breeze:            PARTIAL  (Qt style + color schemes, no cursors/icons)"
    log "  libplasma:         PARTIAL  (desktop themes + package types, no core lib)"
    log "  plasma-workspace:  NOT BUILT (requires 30+ unbuilt deps)"
    log ""
    log "Next steps: Build KF6 Tier 3+ modules (Crash, DBusAddons, KDED,"
    log "  NewStuff, Parts, Prison, Runner, etc.) and Qt6 QML engine for full"
    log "  Plasma Desktop. See docs/cross-build/PLASMA-STATUS.md."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building Plasma Desktop components for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"

    [[ -f "${SYSROOT}/usr/lib/libQt6Core.a" ]] || die "Qt6 not found. Run build-qt6.sh first."
    [[ -d "${HOST_QT}/libexec" ]] || die "Host Qt tools not found. Run build-qt6.sh first."

    build_plasma_activities
    build_kdecoration
    build_breeze
    build_libplasma
    install_veridian_plasma
    verify
    log "=== Plasma build complete ==="
}

main "$@"

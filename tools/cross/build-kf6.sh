#!/usr/bin/env bash
# Build KDE Frameworks 6 (minimal subset) for VeridianOS
#
# Build order follows KF6 dependency chain. Only the modules needed
# by KWin + Plasma Desktop are built.
#
# Prerequisites:
#   - Qt 6 (static) installed in sysroot

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/kf6"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
HOST_QT="${PROJECT_ROOT}/target/cross-build/qt6/host-qt"
JOBS="${JOBS:-$(nproc)}"

KF_VER="6.12.0"
KF_MAJOR="6.12"
KF_URL_BASE="https://download.kde.org/stable/frameworks/${KF_MAJOR}"

log() { echo "[build-kf6] $*"; }
die() { echo "[build-kf6] ERROR: $*" >&2; exit 1; }

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

# Common cmake flags for all KF6 modules
cmake_build() {
    local name="$1"
    local src="$2"
    shift 2
    local extra_args=("$@")
    local bld="${BUILD_DIR}/${name}-build"

    # KDE installs cmake configs as KF6<Name> where Name drops the K prefix
    # e.g. KConfig -> KF6Config, KCoreAddons -> KF6CoreAddons
    # Exception: KCMUtils -> KF6KCMUtils (K is part of the acronym KCM)
    local cmake_name="${name#K}"  # Strip leading K: KConfig -> Config
    if [[ -f "${SYSROOT}/usr/lib/cmake/KF6${cmake_name}/KF6${cmake_name}Config.cmake" ]] || \
       [[ -f "${SYSROOT}/usr/lib/cmake/KF6${name}/KF6${name}Config.cmake" ]]; then
        log "${name}: already installed."
        return 0
    fi

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    log "Building KF6 ${name}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DECM_DIR="${SYSROOT}/usr/share/ECM/cmake" \
            -DQT_HOST_PATH:PATH="${HOST_QT}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${HOST_QT}/lib/cmake" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DBUILD_QCH=OFF \
            -DBUILD_DESIGNERPLUGIN=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
            "${extra_args[@]}" && \
        cmake --build . --parallel "${JOBS}" -- -k || true && \
        cmake --install . 2>/dev/null || \
        cmake --install . --component Devel 2>/dev/null || true)

    # Copy any .a libraries that the install step missed (due to failed executables)
    for lib in "${bld}"/lib/libKF6*.a; do
        [[ -f "$lib" ]] || continue
        local base
        base=$(basename "$lib")
        if [[ ! -f "${SYSROOT}/usr/lib/${base}" ]]; then
            log "  Manually copying ${base}..."
            cp "$lib" "${SYSROOT}/usr/lib/"
        fi
    done

    # Generate missing Targets.cmake from built .a files
    local cmake_name="${name#K}"
    local cmake_dir=""
    for d in "${SYSROOT}/usr/lib/cmake/KF6${cmake_name}" "${SYSROOT}/usr/lib/cmake/KF6${name}"; do
        [[ -d "$d" ]] && cmake_dir="$d" && break
    done
    if [[ -n "${cmake_dir}" ]] && [[ ! -f "${cmake_dir}/KF6${cmake_name}Targets.cmake" ]] && [[ ! -f "${cmake_dir}/KF6${name}Targets.cmake" ]]; then
        # Auto-generate minimal targets file from built .a libraries
        local targets_file="${cmake_dir}/KF6${cmake_name}Targets.cmake"
        [[ -d "${SYSROOT}/usr/lib/cmake/KF6${name}" ]] && targets_file="${cmake_dir}/KF6${name}Targets.cmake"
        log "  Generating ${targets_file##*/}..."
        {
            echo "# Auto-generated targets for VeridianOS cross-build"
            for lib in "${SYSROOT}"/usr/lib/libKF6${cmake_name}*.a "${SYSROOT}"/usr/lib/libKF6${name}*.a; do
                [[ -f "$lib" ]] || continue
                local base tgt alias_name
                base=$(basename "$lib" .a)  # e.g. libKF6IconThemes -> KF6IconThemes
                tgt="${base#lib}"           # KF6IconThemes
                # KDE namespace: KF6IconThemes -> KF6::IconThemes
                alias_name="${tgt/KF6/KF6::}"  # KF6::IconThemes
                echo "if(NOT TARGET ${tgt})"
                echo "  add_library(${tgt} STATIC IMPORTED)"
                echo "  set_target_properties(${tgt} PROPERTIES"
                echo "    IMPORTED_LOCATION \"\${CMAKE_CURRENT_LIST_DIR}/../../../lib/${base}.a\""
                echo "    INTERFACE_INCLUDE_DIRECTORIES \"\${CMAKE_CURRENT_LIST_DIR}/../../../include/KF6/${cmake_name};\${CMAKE_CURRENT_LIST_DIR}/../../../include/KF6/${name}\""
                echo "  )"
                echo "endif()"
                if [[ "${alias_name}" != "${tgt}" ]]; then
                    echo "if(NOT TARGET ${alias_name})"
                    echo "  add_library(${alias_name} INTERFACE IMPORTED)"
                    echo "  set_target_properties(${alias_name} PROPERTIES"
                    echo "    INTERFACE_LINK_LIBRARIES ${tgt}"
                    echo "  )"
                    echo "endif()"
                fi
            done
        } > "${targets_file}"
    fi

    # Verify cmake config was installed (library or header-only module)
    if [[ -f "${SYSROOT}/usr/lib/cmake/KF6${cmake_name}/KF6${cmake_name}Config.cmake" ]] || \
       [[ -f "${SYSROOT}/usr/lib/cmake/KF6${name}/KF6${name}Config.cmake" ]]; then
        log "KF6 ${name}: done."
    else
        log "KF6 ${name}: cmake config not installed!"
        return 1
    fi
}

# ── Install VeridianOS KF6 backend files ──────────────────────────────
install_veridian_backends() {
    local kf6_src="${PROJECT_ROOT}/userland/kf6"
    if [[ ! -d "${kf6_src}" ]]; then
        log "No userland/kf6/ directory -- skipping backend integration."
        return 0
    fi
    log "Copying VeridianOS KF6 backends to sysroot..."
    mkdir -p "${SYSROOT}/usr/src/veridian-kf6"
    cp "${kf6_src}"/*.cpp "${SYSROOT}/usr/src/veridian-kf6/" 2>/dev/null || true
    cp "${kf6_src}"/*.h "${SYSROOT}/usr/src/veridian-kf6/" 2>/dev/null || true
    log "KF6 backends copied."
}

# ── 0. Extra CMake Modules (host only) ────────────────────────────────
build_ecm() {
    if [[ -d "${SYSROOT}/usr/share/ECM" ]]; then
        log "ECM: already installed."
        return 0
    fi
    fetch "extra-cmake-modules-${KF_VER}" \
        "${KF_URL_BASE}/extra-cmake-modules-${KF_VER}.tar.xz" \
        "extra-cmake-modules-${KF_VER}"

    local src="${BUILD_DIR}/extra-cmake-modules-${KF_VER}"
    local bld="${BUILD_DIR}/ecm-build"
    log "Building ECM..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DBUILD_TESTING=OFF \
            -DBUILD_HTML_DOCS=OFF \
            -DBUILD_MAN_DOCS=OFF && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "ECM: done."
}

# ── KF6 Module Builds (in dependency order) ───────────────────────────

# Common extra cmake args to avoid system package leakage
KF_COMMON_ARGS=(
    -DBUILD_PYTHON_BINDINGS=OFF
    -DCMAKE_DISABLE_FIND_PACKAGE_Shiboken6=ON
    -DCMAKE_DISABLE_FIND_PACKAGE_Qt6LinguistTools=ON
    -DKF_SKIP_PO_PROCESSING=ON
    -DWITH_X11=OFF
    -DWITH_WAYLAND=OFF
    -DWITH_BZIP2=OFF
    -DWITH_LIBLZMA=OFF
    -DWITH_LIBZSTD=OFF
    -DCMAKE_DISABLE_FIND_PACKAGE_BZip2=ON
    -DCMAKE_DISABLE_FIND_PACKAGE_LibLZMA=ON
    -DCMAKE_DISABLE_FIND_PACKAGE_LibZstd=ON
    -DCMAKE_PROJECT_INCLUDE="${SCRIPT_DIR}/wayland-scanner-target.cmake"
    # No Qt6Qml/Quick in static cross-build sysroot -- disable per-module QML options
    # Stub Qt6Qml/Quick/Svg cmake configs exist in sysroot for find_package()
    -DKCONFIG_USE_QML=OFF
    -DKCOREADDONS_USE_QML=OFF
    -DKICONTHEMES_USE_QML=OFF
    -DKICONTHEMES_USE_QTQUICK=OFF
    -DKCOLORSCHEME_USE_QML=OFF
    -DKSERVICE_USE_QML=OFF
    -DKCONFIGWIDGETS_USE_QML=OFF
    -DKNOTIFICATIONS_USE_QML=OFF
    -DKPACKAGE_USE_QML=OFF
    -DKIO_USE_QML=OFF
    -DBUILD_WITH_QML=OFF
    -DKWINDOWSYSTEM_QML=OFF
    -DBUILD_DESIGNERPLUGIN=OFF
    -DUSE_BreezeIcons=OFF
    -DCMAKE_DISABLE_FIND_PACKAGE_Canberra=ON
    -DCMAKE_DISABLE_FIND_PACKAGE_Phonon4Qt6=ON
    -DCMAKE_DISABLE_FIND_PACKAGE_KF6Sonnet=ON
    -DCMAKE_DISABLE_FIND_PACKAGE_LIBGIT2=ON
)

# Helper: fetch + cmake_build a KF6 module by name
build_kf_module() {
    local mod="$1"
    shift
    local extra=("$@")
    local lower
    lower=$(echo "${mod}" | tr '[:upper:]' '[:lower:]')
    local pkg="${lower}-${KF_VER}"
    fetch "${pkg}" "${KF_URL_BASE}/${pkg}.tar.xz" "${pkg}"
    cmake_build "${mod}" "${BUILD_DIR}/${pkg}" "${KF_COMMON_ARGS[@]}" "${extra[@]}"
}

# Tier 1: No KF dependencies
build_tier1() {
    local modules=(KConfig KCoreAddons KI18n KGuiAddons KWidgetsAddons
                   KColorScheme KArchive KCodecs KItemViews)
    for mod in "${modules[@]}"; do
        build_kf_module "${mod}"
    done
}

# ── Plasma Wayland Protocols (needed by KWindowSystem) ────────────────
build_plasma_wayland_protocols() {
    if [[ -d "${SYSROOT}/usr/lib/cmake/PlasmaWaylandProtocols" ]]; then
        log "PlasmaWaylandProtocols: already installed."
        return 0
    fi
    local PWP_VER="1.15.0"
    local pkg="plasma-wayland-protocols-${PWP_VER}"
    fetch "${pkg}" \
        "https://download.kde.org/stable/plasma-wayland-protocols/${pkg}.tar.xz" \
        "${pkg}"

    local bld="${BUILD_DIR}/plasma-wayland-protocols-build"
    log "Building PlasmaWaylandProtocols..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${BUILD_DIR}/${pkg}" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DECM_DIR="${SYSROOT}/usr/share/ECM/cmake" \
            -DBUILD_TESTING=OFF && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "PlasmaWaylandProtocols: done."
}

# Tier 2: Depends on Tier 1
build_tier2() {
    build_kf_module KIconThemes
    # KWindowSystem: disable X11 & Wayland platform plugins (MODULE .so incompatible with static)
    build_kf_module KWindowSystem -DKWINDOWSYSTEM_X11=OFF -DKWINDOWSYSTEM_WAYLAND=OFF
    build_kf_module KGlobalAccel
    build_kf_module KPackage
    build_kf_module KCompletion
    build_kf_module KNotifications
    build_kf_module KJobWidgets
    build_kf_module KAuth
    build_kf_module KConfigWidgets
    build_kf_module KService
    build_kf_module Solid
}

# Tier 3: Depends on Tier 1+2
build_tier3() {
    build_kf_module KDeclarative || log "KDeclarative: skipped (QML-heavy, optional for cross-build)"
    build_kf_module KXmlGui
    build_kf_module KBookmarks
    # KIO and KCMUtils are optional -- they have deep dependency chains
    # and require host-target library compatibility (MODULE plugins)
    build_kf_module KIO || log "KIO: skipped (optional for cross-build)"
    build_kf_module KCMUtils || log "KCMUtils: skipped (optional for cross-build)"
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KF6 installation..."
    local errors=0
    local optional_errors=0

    # Required modules (core KF6 for KWin + Plasma)
    for cmake_name in Config CoreAddons I18n GuiAddons WidgetsAddons ColorScheme \
                      Archive Codecs ItemViews IconThemes WindowSystem GlobalAccel \
                      Package Completion JobWidgets Auth ConfigWidgets Notifications \
                      Service Solid XmlGui Bookmarks; do
        if [[ -f "${SYSROOT}/usr/lib/cmake/KF6${cmake_name}/KF6${cmake_name}Config.cmake" ]]; then
            log "  OK: KF6${cmake_name}"
        else
            log "  MISSING: KF6${cmake_name}"
            errors=$((errors + 1))
        fi
    done

    # Optional modules (deep dependency chains, may fail in cross-build)
    for cmake_name in KIO KCMUtils Declarative; do
        if [[ -f "${SYSROOT}/usr/lib/cmake/KF6${cmake_name}/KF6${cmake_name}Config.cmake" ]]; then
            log "  OK: KF6${cmake_name}"
        else
            log "  OPTIONAL: KF6${cmake_name} (not installed)"
            optional_errors=$((optional_errors + 1))
        fi
    done

    if [[ $errors -gt 0 ]]; then
        die "${errors} required modules missing!"
    fi
    if [[ $optional_errors -gt 0 ]]; then
        log "${optional_errors} optional modules not installed"
    fi
    log "KDE Frameworks 6 ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building KDE Frameworks 6 ${KF_VER} for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"

    [[ -f "${SYSROOT}/usr/lib/libQt6Core.a" ]] || die "Qt6 not found. Run build-qt6.sh first."
    [[ -d "${HOST_QT}/libexec" ]] || die "Host Qt tools not found. Run build-qt6.sh first."

    build_ecm
    build_plasma_wayland_protocols
    build_tier1
    build_tier2
    build_tier3
    install_veridian_backends
    verify
    log "=== KF6 build complete ==="
}

main "$@"

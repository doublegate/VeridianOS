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
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
JOBS="${JOBS:-$(nproc)}"

KF_VER="6.0.0"
KF_URL_BASE="https://download.kde.org/stable/frameworks/6.0"

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
    local bld="${BUILD_DIR}/${name}-build"

    if [[ -f "${SYSROOT}/usr/lib/libKF6${name}.a" ]] || \
       [[ -f "${SYSROOT}/usr/lib/cmake/KF6${name}/KF6${name}Config.cmake" ]]; then
        log "${name}: already installed."
        return 0
    fi

    log "Building KF6 ${name}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DBUILD_QCH=OFF \
            -DCMAKE_BUILD_TYPE=Release && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "KF6 ${name}: done."
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

# Tier 1: No KF dependencies
build_tier1() {
    local modules=(KConfig KCoreAddons KI18n KGuiAddons KWidgetsAddons KColorScheme)
    for mod in "${modules[@]}"; do
        local lower
        lower=$(echo "${mod}" | tr '[:upper:]' '[:lower:]')
        local pkg="${lower}-${KF_VER}"
        fetch "${pkg}" "${KF_URL_BASE}/${pkg}.tar.xz" "${pkg}"
        cmake_build "${mod}" "${BUILD_DIR}/${pkg}"
    done
}

# Tier 2: Depends on Tier 1
build_tier2() {
    local modules=(KIconThemes KWindowSystem KGlobalAccel KPackage)
    for mod in "${modules[@]}"; do
        local lower
        lower=$(echo "${mod}" | tr '[:upper:]' '[:lower:]')
        local pkg="${lower}-${KF_VER}"
        fetch "${pkg}" "${KF_URL_BASE}/${pkg}.tar.xz" "${pkg}"
        cmake_build "${mod}" "${BUILD_DIR}/${pkg}"
    done
}

# Tier 3: Depends on Tier 1+2
build_tier3() {
    local modules=(KDeclarative KCMUtils)
    for mod in "${modules[@]}"; do
        local lower
        lower=$(echo "${mod}" | tr '[:upper:]' '[:lower:]')
        local pkg="${lower}-${KF_VER}"
        fetch "${pkg}" "${KF_URL_BASE}/${pkg}.tar.xz" "${pkg}"
        cmake_build "${mod}" "${BUILD_DIR}/${pkg}"
    done
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KF6 installation..."
    local errors=0
    for mod in KConfig KCoreAddons KI18n KWindowSystem KGlobalAccel KPackage; do
        local cmake_config="${SYSROOT}/usr/lib/cmake/KF6${mod}/KF6${mod}Config.cmake"
        if [[ -f "${cmake_config}" ]]; then
            log "  OK: KF6${mod}"
        else
            log "  MISSING: KF6${mod} (${cmake_config})"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} modules missing!"
    fi
    log "KDE Frameworks 6 ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building KDE Frameworks 6 for VeridianOS ==="
    build_ecm
    build_tier1
    build_tier2
    build_tier3
    install_veridian_backends
    verify
    log "=== KF6 build complete ==="
}

main "$@"

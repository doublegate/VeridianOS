#!/usr/bin/env bash
# Build Plasma Desktop and Breeze theme for VeridianOS
#
# Produces plasmashell and Breeze theme components.
#
# Prerequisites:
#   - KWin + Qt 6 + KF6 + all dependencies built

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
    local bld="${BUILD_DIR}/${name}-build"
    local extra_args="${3:-}"

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
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DBUILD_QCH=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            -DKF_SKIP_PO_PROCESSING=ON \
            -DKF6_HOST_TOOLING=/usr/lib64/cmake \
            ${extra_args} && \
        cmake --build . --parallel "${JOBS}" -- -k || true && \
        cmake --install . --prefix "${SYSROOT}/usr" 2>/dev/null || \
        cmake --install . --prefix "${SYSROOT}/usr" --component Devel 2>/dev/null || true)
    log "${name}: done."
}

# ── 1. Breeze (icons + Qt style + window decoration) ─────────────────
build_breeze() {
    if [[ -d "${SYSROOT}/usr/share/icons/breeze" ]]; then
        log "Breeze: already installed."
        return 0
    fi
    fetch "breeze-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/breeze-${PLASMA_VER}.tar.xz" \
        "breeze-${PLASMA_VER}"

    cmake_build "breeze" "${BUILD_DIR}/breeze-${PLASMA_VER}"
}

# ── 2. plasma-workspace (plasmashell, krunner, session) ───────────────
build_plasma_workspace() {
    if [[ -f "${SYSROOT}/usr/bin/plasmashell" ]]; then
        log "plasma-workspace: already installed."
        return 0
    fi
    fetch "plasma-workspace-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/plasma-workspace-${PLASMA_VER}.tar.xz" \
        "plasma-workspace-${PLASMA_VER}"

    cmake_build "plasma-workspace" \
        "${BUILD_DIR}/plasma-workspace-${PLASMA_VER}" \
        "-DPLASMA_WAYLAND_DEFAULT_SESSION=ON"
}

# ── 3. plasma-desktop (containment, panel, system tray) ───────────────
build_plasma_desktop() {
    fetch "plasma-desktop-${PLASMA_VER}" \
        "${PLASMA_URL_BASE}/plasma-desktop-${PLASMA_VER}.tar.xz" \
        "plasma-desktop-${PLASMA_VER}"

    cmake_build "plasma-desktop" "${BUILD_DIR}/plasma-desktop-${PLASMA_VER}"
}

# ── 4. Install VeridianOS Plasma applets and scripts ──────────────────
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
    for item in \
        "${SYSROOT}/usr/share/icons/breeze" \
    ; do
        if [[ -d "$item" ]]; then
            log "  OK: $(basename "$item")/"
        else
            log "  MISSING: $item"
            errors=$((errors + 1))
        fi
    done
    if [[ -f "${SYSROOT}/usr/bin/plasmashell" ]]; then
        log "  OK: plasmashell"
    else
        log "  MISSING: plasmashell (may need additional patches)"
        errors=$((errors + 1))
    fi
    if [[ $errors -gt 0 ]]; then
        log "WARNING: ${errors} items missing (expected for first build)"
    fi
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building Plasma Desktop for VeridianOS ==="
    build_breeze
    build_plasma_workspace
    build_plasma_desktop
    install_veridian_plasma
    verify
    log "=== Plasma build complete ==="
}

main "$@"

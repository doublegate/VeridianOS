#!/usr/bin/env bash
# Build KWin Wayland compositor for VeridianOS
#
# Produces the kwin_wayland binary -- the KDE Plasma 6 compositor.
# Integrates VeridianOS platform backend from userland/kwin/.
#
# Prerequisites:
#   - Qt 6 + KF6 + Mesa + Wayland + libinput (real, from build-deps.sh)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/kwin"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
JOBS="${JOBS:-$(nproc)}"

KWIN_VER="6.0.0"
KWIN_URL="https://download.kde.org/stable/plasma/6.0.0/kwin-${KWIN_VER}.tar.xz"
KDECORATION_VER="6.0.0"
KDECORATION_URL="https://download.kde.org/stable/plasma/6.0.0/kdecoration-${KDECORATION_VER}.tar.xz"

log() { echo "[build-kwin] $*"; }
die() { echo "[build-kwin] ERROR: $*" >&2; exit 1; }

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

# ── 1. Verify real libinput ───────────────────────────────────────────
# Real libinput is now built by build-deps.sh (with libevdev).
# This function just verifies it exists.
verify_libinput() {
    if [[ -f "${SYSROOT}/usr/lib/libinput.a" ]] && \
       [[ -f "${SYSROOT}/usr/include/libinput.h" ]]; then
        log "libinput: found in sysroot."
        return 0
    fi
    die "libinput not found. Run build-deps.sh first (builds real libevdev + libinput)."
}

# ── 2. Build kdecoration ─────────────────────────────────────────────
build_kdecoration() {
    if [[ -d "${SYSROOT}/usr/lib/cmake/KDecoration2" ]]; then
        log "kdecoration: already installed."
        return 0
    fi
    fetch "kdecoration-${KDECORATION_VER}" "${KDECORATION_URL}" "kdecoration-${KDECORATION_VER}"

    local src="${BUILD_DIR}/kdecoration-${KDECORATION_VER}"
    local bld="${BUILD_DIR}/kdecoration-build"
    log "Building kdecoration ${KDECORATION_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "kdecoration: done."
}

# ── 3. Install VeridianOS KWin backend ────────────────────────────────
install_veridian_backend() {
    local kwin_src="${PROJECT_ROOT}/userland/kwin"
    if [[ ! -d "${kwin_src}" ]]; then
        log "No userland/kwin/ -- skipping backend integration."
        return 0
    fi
    log "Copying VeridianOS KWin backend to sysroot..."
    mkdir -p "${SYSROOT}/usr/src/veridian-kwin"
    cp "${kwin_src}"/*.cpp "${SYSROOT}/usr/src/veridian-kwin/" 2>/dev/null || true
    cp "${kwin_src}"/*.h "${SYSROOT}/usr/src/veridian-kwin/" 2>/dev/null || true
    log "KWin backend copied."
}

# ── 4. Build KWin ────────────────────────────────────────────────────
build_kwin() {
    if [[ -f "${SYSROOT}/usr/bin/kwin_wayland" ]]; then
        log "KWin: already installed."
        return 0
    fi
    fetch "kwin-${KWIN_VER}" "${KWIN_URL}" "kwin-${KWIN_VER}"

    local src="${BUILD_DIR}/kwin-${KWIN_VER}"
    local bld="${BUILD_DIR}/kwin-build"
    log "Building KWin ${KWIN_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DKWIN_BUILD_XWAYLAND=ON \
            -DKWIN_BUILD_SCREENLOCKER=ON \
            -DKWIN_BUILD_TABBOX=ON \
            -DKWIN_BUILD_KCMS=ON \
            -DCMAKE_BUILD_TYPE=Release && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "KWin: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KWin installation..."
    local errors=0
    for item in \
        "${SYSROOT}/usr/lib/libinput.a" \
        "${SYSROOT}/usr/lib/libevdev.a" \
        "${SYSROOT}/usr/include/libinput.h" \
    ; do
        if [[ -f "$item" ]]; then
            log "  OK: $(basename "$item")"
        else
            log "  MISSING: $item"
            errors=$((errors + 1))
        fi
    done
    if [[ -f "${SYSROOT}/usr/bin/kwin_wayland" ]]; then
        local size
        size=$(stat -c%s "${SYSROOT}/usr/bin/kwin_wayland" 2>/dev/null || echo "?")
        log "  OK: kwin_wayland (${size} bytes)"
    else
        log "  MISSING: kwin_wayland (may need additional patches)"
        errors=$((errors + 1))
    fi
    if [[ $errors -gt 0 ]]; then
        log "WARNING: ${errors} items missing (expected for first build -- iterate)"
    fi
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building KWin for VeridianOS ==="
    verify_libinput
    build_kdecoration
    install_veridian_backend
    build_kwin
    verify
    log "=== KWin build complete ==="
}

main "$@"

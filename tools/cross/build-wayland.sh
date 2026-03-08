#!/usr/bin/env bash
# Build Wayland libraries for VeridianOS
#
# Two-stage build:
#   1. Build wayland-scanner for HOST (runs during cross-compilation to
#      generate protocol marshalling code)
#   2. Cross-compile libwayland-client, libwayland-server, libwayland-cursor
#      as static libraries for VeridianOS target
#   3. Install wayland-protocols (header-only XML protocol definitions)
#
# Prerequisites:
#   - musl libc + libffi + libexpat built
#   - meson + ninja

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/wayland"
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
MESON_CROSS="${SCRIPT_DIR}/meson-cross-veridian.txt"
JOBS="${JOBS:-$(nproc)}"

WAYLAND_VER="1.22.0"
WAYLAND_URL="https://gitlab.freedesktop.org/wayland/wayland/-/releases/${WAYLAND_VER}/downloads/wayland-${WAYLAND_VER}.tar.xz"
PROTOCOLS_VER="1.33"
PROTOCOLS_URL="https://gitlab.freedesktop.org/wayland/wayland-protocols/-/releases/${PROTOCOLS_VER}/downloads/wayland-protocols-${PROTOCOLS_VER}.tar.xz"

log() { echo "[build-wayland] $*"; }
die() { echo "[build-wayland] ERROR: $*" >&2; exit 1; }

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

# ── 1. Build wayland-scanner (HOST native) ───────────────────────────
build_scanner() {
    local scanner="${BUILD_DIR}/host-build/wayland-scanner"
    if [[ -x "${scanner}" ]]; then
        log "wayland-scanner: already built."
        return 0
    fi
    fetch "wayland-${WAYLAND_VER}" "${WAYLAND_URL}" "wayland-${WAYLAND_VER}"

    local src="${BUILD_DIR}/wayland-${WAYLAND_VER}"
    local bld="${BUILD_DIR}/host-build"
    log "Building wayland-scanner (host native)..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --prefix="${bld}/install" \
            -Dscanner=true \
            -Dlibraries=false \
            -Ddocumentation=false \
            -Dtests=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "wayland-scanner: done."
}

# ── 2. Cross-compile Wayland libraries ────────────────────────────────
build_wayland_libs() {
    if [[ -f "${SYSROOT}/usr/lib/libwayland-client.a" ]]; then
        log "Wayland libraries: already installed."
        return 0
    fi
    fetch "wayland-${WAYLAND_VER}" "${WAYLAND_URL}" "wayland-${WAYLAND_VER}"

    local src="${BUILD_DIR}/wayland-${WAYLAND_VER}"
    local bld="${BUILD_DIR}/cross-build"
    local scanner="${BUILD_DIR}/host-build/install/bin/wayland-scanner"

    if [[ ! -x "${scanner}" ]]; then
        # Fall back to system scanner
        scanner="$(command -v wayland-scanner 2>/dev/null || true)"
        if [[ -z "${scanner}" ]]; then
            die "No wayland-scanner found. Run build_scanner first."
        fi
    fi

    log "Cross-compiling Wayland libraries..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dscanner=false \
            -Dlibraries=true \
            -Ddocumentation=false \
            -Dtests=false \
            -Ddtd_validation=false && \
        ninja -j"${JOBS}" && \
        ninja install)

    # Install scanner to sysroot for downstream builds
    install -Dm755 "${scanner}" "${SYSROOT}/usr/bin/wayland-scanner"
    log "Wayland libraries: done."
}

# ── 3. Install wayland-protocols ──────────────────────────────────────
install_protocols() {
    if [[ -d "${SYSROOT}/usr/share/wayland-protocols/stable/xdg-shell" ]]; then
        log "wayland-protocols: already installed."
        return 0
    fi
    fetch "wayland-protocols-${PROTOCOLS_VER}" "${PROTOCOLS_URL}" "wayland-protocols-${PROTOCOLS_VER}"

    local src="${BUILD_DIR}/wayland-protocols-${PROTOCOLS_VER}"
    local bld="${BUILD_DIR}/protocols-build"
    log "Installing wayland-protocols ${PROTOCOLS_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --prefix="${SYSROOT}/usr" \
            -Dtests=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "wayland-protocols: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Wayland installation..."
    local errors=0
    for lib in libwayland-client.a libwayland-server.a libwayland-cursor.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            log "  OK: ${lib}"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    for item in \
        "${SYSROOT}/usr/bin/wayland-scanner" \
        "${SYSROOT}/usr/share/wayland-protocols/stable/xdg-shell" \
        "${SYSROOT}/usr/include/wayland-client.h" \
    ; do
        if [[ -e "$item" ]]; then
            log "  OK: $(basename "$item")"
        else
            log "  MISSING: $item"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} items missing!"
    fi
    log "Wayland stack ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building Wayland for VeridianOS ==="
    build_scanner
    build_wayland_libs
    install_protocols
    verify
    log "=== Wayland build complete ==="
}

main "$@"

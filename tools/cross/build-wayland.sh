#!/usr/bin/env bash
# Build Wayland libraries for VeridianOS
#
#   1. wayland-scanner for the HOST (it generates protocol marshalling code
#      during later builds), installed in ${VERIDIAN_HOST_TOOLS}
#   2. libwayland-client, -server, -cursor and -egl cross-compiled,
#      plus the core protocol (wayland.xml) and wayland-scanner.pc in the
#      sysroot for builds that read the protocol data
#   3. wayland-protocols (XML protocol definitions)
#
# Built and staged as lib/cross-env.sh describes.
# Prerequisites: build-deps.sh (libffi, expat); meson, ninja.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/wayland"
JOBS="${JOBS:-$(nproc)}"

# Checksums pinned when first downloaded.
WAYLAND_VER="1.26.0"
WAYLAND_SHA256="64176eaa46e4969903e286f8e5ef8331affc17fdf03ac9b58381d2b23162b7a3"
PROTOCOLS_VER="1.49"
PROTOCOLS_SHA256="ec4c8f74942d6dff7ace8b4ce4764f0ef9ff618a935d974ea77edee2ad240b14"

log() { echo "[build-wayland] $*"; }
die() { echo "[build-wayland] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

WAYLAND_URL="https://gitlab.freedesktop.org/wayland/wayland/-/releases/${WAYLAND_VER}/downloads/wayland-${WAYLAND_VER}.tar.xz"
PROTOCOLS_URL="https://gitlab.freedesktop.org/wayland/wayland-protocols/-/releases/${PROTOCOLS_VER}/downloads/wayland-protocols-${PROTOCOLS_VER}.tar.xz"

# A fresh Wayland tree, without the always-true meson version checks
# (it requires meson 0.64; deps-patches/meson_lint.py).
fetch_wayland() {
    fetch "wayland-${WAYLAND_VER}.tar.xz" "${WAYLAND_URL}" "wayland-${WAYLAND_VER}" "${WAYLAND_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/meson_lint.py" version-checks \
        "${BUILD_DIR}/wayland-${WAYLAND_VER}" 0.64.0 || die "failed to patch Wayland's meson files"
}

# ── 1. wayland-scanner (host) ─────────────────────────────────────────
build_scanner() {
    local scanner="${VERIDIAN_HOST_TOOLS}/bin/wayland-scanner"
    if [[ -x "${scanner}" && "$("${scanner}" --version 2>&1)" == "wayland-scanner ${WAYLAND_VER}" ]]; then
        log "wayland-scanner (host): already installed."
        return 0
    fi
    fetch_wayland
    log "Building wayland-scanner ${WAYLAND_VER} (host)..."
    local bld="${BUILD_DIR}/host-build"
    rm -rf "${bld}"
    # A native build: unset the target environment for it.
    (unset CC CXX AR RANLIB NM STRIP CFLAGS CXXFLAGS PKG_CONFIG_LIBDIR PKG_CONFIG_SYSROOT_DIR && \
        meson setup "${bld}" "${BUILD_DIR}/wayland-${WAYLAND_VER}" \
            --prefix="${VERIDIAN_HOST_TOOLS}" \
            --libdir=lib \
            -Dscanner=true \
            -Dlibraries=false \
            -Ddocumentation=false \
            -Ddtd_validation=false \
            -Dtests=false && \
        ninja -C "${bld}" -j"${JOBS}" && \
        ninja -C "${bld}" install)
}

# ── 2. Wayland libraries (target) ─────────────────────────────────────
build_wayland_libs() {
    installed libwayland-client.so "Wayland libraries" && return 0
    fetch_wayland
    log "Cross-compiling Wayland libraries..."
    # The scanner is the host one (a native dependency, found through
    # PKG_CONFIG_PATH_FOR_BUILD at exactly this version).
    meson_build "${BUILD_DIR}/wayland-${WAYLAND_VER}" "${BUILD_DIR}/cross-build" \
        -Dscanner=false \
        -Dlibraries=true \
        -Ddocumentation=false \
        -Dtests=false \
        -Ddtd_validation=false
}

# The core protocol and the scanner's pkg-config entry, which the target
# build (scanner=false) does not install: ECM's FindWayland and Qt's
# protocol code generation read wayland.xml through it. The scanner itself
# is the host program on PATH.
install_scanner_data() {
    local host_xml="${VERIDIAN_HOST_TOOLS}/share/wayland/wayland.xml"
    [[ -f "${host_xml}" ]] || die "host wayland-scanner install has no wayland.xml"
    install -Dm644 "${host_xml}" "${SYSROOT}/usr/share/wayland/wayland.xml"
    install -Dm644 /dev/stdin "${SYSROOT}/usr/lib/pkgconfig/wayland-scanner.pc" <<PCEOF
prefix=/usr
datarootdir=\${prefix}/share
pkgdatadir=\${datarootdir}/wayland
wayland_scanner=wayland-scanner

Name: Wayland Scanner
Description: Wayland scanner
Version: ${WAYLAND_VER}
PCEOF
}

# ── 3. wayland-protocols ──────────────────────────────────────────────
install_protocols() {
    if [[ -d "${SYSROOT}/usr/share/wayland-protocols/stable/xdg-shell" ]]; then
        log "wayland-protocols: already installed."
        return 0
    fi
    fetch "wayland-protocols-${PROTOCOLS_VER}.tar.xz" "${PROTOCOLS_URL}" \
        "wayland-protocols-${PROTOCOLS_VER}" "${PROTOCOLS_SHA256}"
    log "Installing wayland-protocols ${PROTOCOLS_VER}..."
    meson_build "${BUILD_DIR}/wayland-protocols-${PROTOCOLS_VER}" "${BUILD_DIR}/protocols-build" \
        -Dtests=false
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Wayland installation..."
    local errors=0 item
    for item in \
        "${SYSROOT}/usr/lib/libwayland-client.so" \
        "${SYSROOT}/usr/lib/libwayland-server.so" \
        "${SYSROOT}/usr/lib/libwayland-cursor.so" \
        "${SYSROOT}/usr/lib/libwayland-egl.so" \
        "${SYSROOT}/usr/include/wayland-client.h" \
        "${SYSROOT}/usr/share/wayland/wayland.xml" \
        "${SYSROOT}/usr/lib/pkgconfig/wayland-scanner.pc" \
        "${SYSROOT}/usr/share/wayland-protocols/stable/xdg-shell" \
        "${VERIDIAN_HOST_TOOLS}/bin/wayland-scanner" \
    ; do
        if [[ -e "${item}" ]]; then
            log "  OK: ${item#"${SYSROOT}"}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "Wayland stack ready."
}

main() {
    log "=== Building Wayland for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    [[ -f "${SYSROOT}/usr/lib/libffi.so" ]] || die "libffi not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/libexpat.so" ]] || die "expat not found. Run build-deps.sh first."

    build_scanner
    build_wayland_libs
    install_scanner_data
    install_protocols
    verify

    log "=== Wayland build complete ==="
}

main "$@"

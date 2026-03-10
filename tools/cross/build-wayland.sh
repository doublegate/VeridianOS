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
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
JOBS="${JOBS:-$(nproc)}"
CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"

WAYLAND_VER="1.23.1"
WAYLAND_URL="https://gitlab.freedesktop.org/wayland/wayland/-/releases/${WAYLAND_VER}/downloads/wayland-${WAYLAND_VER}.tar.xz"
# If system wayland-scanner version doesn't match, we build our own from this source
PROTOCOLS_VER="1.38"
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

# ── Generate meson cross file ─────────────────────────────────────────
generate_meson_cross() {
    local cross_file="${BUILD_DIR}/meson-cross.txt"
    cat > "${cross_file}" << CROSSEOF
[binaries]
c = '${CC}'
ar = 'ar'
strip = 'strip'
pkgconfig = 'pkg-config'

[built-in options]
c_args = ['-fPIC']
c_link_args = []

[properties]
sys_root = '${SYSROOT}'
pkg_config_libdir = '${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig'
needs_exe_wrapper = true

[host_machine]
system = 'linux'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
CROSSEOF
    echo "${cross_file}"
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
    local cross_file
    cross_file="$(generate_meson_cross)"

    if [[ ! -x "${scanner}" ]]; then
        # Fall back to system scanner
        scanner="$(command -v wayland-scanner 2>/dev/null || true)"
        if [[ -z "${scanner}" ]]; then
            die "No wayland-scanner found. Run build_scanner first."
        fi
    fi

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    # Do NOT set PKG_CONFIG_SYSROOT_DIR -- it double-prefixes paths like
    # wayland-scanner which are absolute paths in .pc files
    export PKG_CONFIG_SYSROOT_DIR=""

    # Point build-machine pkg-config to our host-built scanner so meson
    # finds the matching wayland-scanner version
    local host_pkgconfig="${BUILD_DIR}/host-build/install/lib/pkgconfig"
    if [[ -d "${host_pkgconfig}" ]]; then
        export PKG_CONFIG_PATH_FOR_BUILD="${host_pkgconfig}"
    fi

    # Apply patches if present (e.g., relax scanner version check)
    local patch_dir="${SCRIPT_DIR}/wayland-patches"
    if [[ -d "${patch_dir}" ]]; then
        local marker="${src}/.veridian_patched"
        if [[ ! -f "${marker}" ]]; then
            for patch in "${patch_dir}"/*.patch; do
                [[ -f "$patch" ]] || continue
                log "Applying $(basename "$patch")..."
                (cd "${src}" && patch -p1 < "$patch" 2>/dev/null || true)
            done
            touch "${marker}"
        fi
    fi

    log "Cross-compiling Wayland libraries..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${cross_file}" \
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
            local size
            size=$(stat -c%s "${SYSROOT}/usr/lib/${lib}" 2>/dev/null || echo "?")
            log "  OK: ${lib} (${size} bytes)"
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
    log "Sysroot: ${SYSROOT}"

    [[ -f "${SYSROOT}/usr/lib/libc.a" ]] || die "musl libc not found. Run build-musl.sh first."
    [[ -f "${SYSROOT}/usr/lib/libffi.a" ]] || die "libffi not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/libexpat.a" ]] || die "expat not found. Run build-deps.sh first."

    build_scanner
    build_wayland_libs
    install_protocols
    verify

    log "=== Wayland build complete ==="
}

main "$@"

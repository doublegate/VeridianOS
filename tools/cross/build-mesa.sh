#!/usr/bin/env bash
# Build Mesa with softpipe for VeridianOS
#
# Produces static EGL, GLES2, GBM, and DRM libraries using Mesa's
# softpipe gallium driver (pure software rendering, no GPU needed).
#
# Prerequisites:
#   - musl libc + zlib + libexpat built
#   - meson + ninja

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/mesa"
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
MESON_CROSS="${SCRIPT_DIR}/meson-cross-veridian.txt"
JOBS="${JOBS:-$(nproc)}"

MESA_VER="24.0.9"
MESA_URL="https://archive.mesa3d.org/mesa-${MESA_VER}.tar.xz"
LIBDRM_VER="2.4.120"
LIBDRM_URL="https://dri.freedesktop.org/libdrm/libdrm-${LIBDRM_VER}.tar.xz"

log() { echo "[build-mesa] $*"; }
die() { echo "[build-mesa] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

# ── Fetch helper ──────────────────────────────────────────────────────
fetch() {
    local name="$1" url="$2" dir="$3" ext="${4:-.tar.xz}"
    local tarball="${BUILD_DIR}/${name}${ext}"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        curl -fsSL -o "${tarball}" "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── 1. libdrm ────────────────────────────────────────────────────────
build_libdrm() {
    if [[ -f "${SYSROOT}/usr/lib/libdrm.a" ]]; then
        log "libdrm: already installed."
        return 0
    fi
    fetch "libdrm-${LIBDRM_VER}" "${LIBDRM_URL}" "libdrm-${LIBDRM_VER}"

    local src="${BUILD_DIR}/libdrm-${LIBDRM_VER}"
    local bld="${BUILD_DIR}/libdrm-build"
    log "Building libdrm ${LIBDRM_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dintel=disabled \
            -Dradeon=disabled \
            -Damdgpu=disabled \
            -Dnouveau=disabled \
            -Dvmwgfx=disabled \
            -Dfreedreno=disabled \
            -Dvc4=disabled \
            -Detnaviv=disabled \
            -Dtests=false \
            -Dman-pages=disabled \
            -Dvalgrind=disabled \
            -Dcairo-tests=disabled && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "libdrm: done."
}

# ── 2. Mesa ──────────────────────────────────────────────────────────
build_mesa() {
    if [[ -f "${SYSROOT}/usr/lib/libEGL.a" ]]; then
        log "Mesa: already installed."
        return 0
    fi
    fetch "mesa-${MESA_VER}" "${MESA_URL}" "mesa-${MESA_VER}"

    local src="${BUILD_DIR}/mesa-${MESA_VER}"
    local bld="${BUILD_DIR}/mesa-build"

    # Apply VeridianOS patches if present
    local patch_dir="${SCRIPT_DIR}/mesa-patches"
    if [[ -d "${patch_dir}" ]]; then
        local marker="${src}/.veridian_patched"
        if [[ ! -f "${marker}" ]]; then
            for patch in "${patch_dir}"/*.patch; do
                [[ -f "$patch" ]] || continue
                log "Applying $(basename "$patch")..."
                (cd "${src}" && patch -p1 < "$patch")
            done
            touch "${marker}"
        fi
    fi

    log "Building Mesa ${MESA_VER} (softpipe)..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dplatforms=wayland \
            -Dgallium-drivers=softpipe \
            -Dvulkan-drivers= \
            -Dglx=disabled \
            -Degl=enabled \
            -Dgles2=enabled \
            -Dopengl=false \
            -Dshared-glapi=disabled \
            -Dllvm=disabled \
            -Dgbm=enabled \
            -Ddri3=disabled \
            -Dglvnd=disabled \
            -Dlmsensors=disabled \
            -Dvalgrind=disabled \
            -Dbuild-tests=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "Mesa: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Mesa installation..."
    local errors=0
    for lib in libdrm.a libEGL.a libGLESv2.a libgbm.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            local size
            size=$(stat -c%s "${SYSROOT}/usr/lib/${lib}" 2>/dev/null || echo "?")
            log "  OK: ${lib} (${size} bytes)"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    for hdr in EGL/egl.h GLES2/gl2.h gbm.h xf86drm.h; do
        if [[ -f "${SYSROOT}/usr/include/${hdr}" ]]; then
            log "  OK: include/${hdr}"
        else
            log "  MISSING: include/${hdr}"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} items missing!"
    fi
    log "Mesa software rendering stack ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building Mesa softpipe for VeridianOS ==="
    build_libdrm
    build_mesa
    verify
    log "=== Mesa build complete ==="
}

main "$@"

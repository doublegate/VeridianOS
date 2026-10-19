#!/usr/bin/env bash
# Build Mesa with softpipe for VeridianOS
#
# EGL (Wayland and surfaceless platforms), OpenGL ES 2, GBM and libdrm from
# Mesa's softpipe gallium driver (software rendering, no GPU needed), and
# libepoxy on top of them. Shared libraries (ADR 0010), built and staged
# as lib/cross-env.sh describes.
#
# Prerequisites: build-deps.sh (zlib, expat), build-wayland.sh (libwayland,
# wayland-protocols, the host wayland-scanner); meson, ninja, python3-mako.
#
# Output: libdrm, libEGL, libGLESv2, libgbm (with gbm/dri_gbm.so), Mesa's
# gallium library and libepoxy in $SYSROOT/usr/lib.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/mesa"
JOBS="${JOBS:-$(nproc)}"

# Checksums pinned when first downloaded.
LIBDRM_VER="2.4.134"
LIBDRM_SHA256="ac5e74d157830eb8bee44c6a6bf3ad49774ef0dd2a72bdad74a8f20308b52a95"
MESA_VER="26.2.4"
MESA_SHA256="bce5f7fbebb934373b86c999a064d52fb5065878dc57f287f95346648ec832e9"
LIBEPOXY_VER="1.5.10"
LIBEPOXY_SHA256="072cda4b59dd098bba8c2363a6247299db1fa89411dc221c8b81b8ee8192e623"

log() { echo "[build-mesa] $*"; }
die() { echo "[build-mesa] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

# ── 1. libdrm ────────────────────────────────────────────────────────
build_libdrm() {
    installed libdrm.so libdrm && return 0
    fetch "libdrm-${LIBDRM_VER}.tar.xz" \
        "https://dri.freedesktop.org/libdrm/libdrm-${LIBDRM_VER}.tar.xz" \
        "libdrm-${LIBDRM_VER}" "${LIBDRM_SHA256}"
    log "Building libdrm ${LIBDRM_VER}..."
    meson_build "${BUILD_DIR}/libdrm-${LIBDRM_VER}" "${BUILD_DIR}/libdrm-build" \
        -Dintel=disabled \
        -Dradeon=disabled \
        -Damdgpu=disabled \
        -Dnouveau=disabled \
        -Dvmwgfx=disabled \
        -Dfreedreno=disabled \
        -Dvc4=disabled \
        -Detnaviv=disabled \
        -Dexynos=disabled \
        -Dtests=false \
        -Dman-pages=disabled \
        -Dvalgrind=disabled \
        -Dcairo-tests=disabled \
        -Dudev=false
}

# ── 2. Mesa (softpipe) ──────────────────────────────────────────────
build_mesa() {
    installed libEGL.so Mesa && return 0
    fetch "mesa-${MESA_VER}.tar.xz" \
        "https://archive.mesa3d.org/mesa-${MESA_VER}.tar.xz" \
        "mesa-${MESA_VER}" "${MESA_SHA256}"

    local src="${BUILD_DIR}/mesa-${MESA_VER}"
    local bld="${BUILD_DIR}/mesa-build"
    log "Building Mesa ${MESA_VER} (softpipe)..."
    # shared-glapi must be enabled (Mesa requires it for EGL + GLES2). The
    # Wayland platform: Qt and KDE programs render with EGL on Wayland;
    # with softpipe, EGL hands the compositor wl_shm buffers. The TLS
    # dialect is GCC's default, stated so Mesa need not probe it.
    meson_build "${src}" "${bld}" \
        -Dc_args="${CFLAGS} -mtls-dialect=gnu" \
        -Dcpp_args="${CXXFLAGS} -mtls-dialect=gnu" \
        -Dplatforms=wayland \
        -Dgallium-drivers=softpipe \
        -Dvulkan-drivers= \
        -Dglx=disabled \
        -Degl=enabled \
        -Dgles1=disabled \
        -Dgles2=enabled \
        -Dopengl=false \
        -Dshared-glapi=enabled \
        -Dllvm=disabled \
        -Dgbm=enabled \
        -Dglvnd=disabled \
        -Dvalgrind=disabled \
        -Dlibunwind=disabled \
        -Dlmsensors=disabled \
        -Dbuild-tests=false \
        -Dselinux=false \
        -Dxlib-lease=disabled \
        -Dgallium-va=disabled \
        -Dvideo-codecs= \
        -Dpower8=disabled \
        -Dzstd=disabled
}

# ── 3. libepoxy (GL function pointer manager, required by KWin) ──────
# EGL + GLES only: no GLX, no X11.
build_libepoxy() {
    installed libepoxy.so libepoxy && return 0
    fetch "libepoxy-${LIBEPOXY_VER}.tar.xz" \
        "https://download.gnome.org/sources/libepoxy/${LIBEPOXY_VER%.*}/libepoxy-${LIBEPOXY_VER}.tar.xz" \
        "libepoxy-${LIBEPOXY_VER}" "${LIBEPOXY_SHA256}"
    log "Building libepoxy ${LIBEPOXY_VER}..."
    meson_build "${BUILD_DIR}/libepoxy-${LIBEPOXY_VER}" "${BUILD_DIR}/libepoxy-build" \
        -Dglx=no \
        -Dx11=false \
        -Degl=yes \
        -Dtests=false \
        -Ddocs=false
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Mesa installation..."
    local errors=0 item
    for item in lib/libdrm.so lib/libEGL.so lib/libGLESv2.so lib/libgbm.so \
                lib/gbm/dri_gbm.so lib/libepoxy.so include/EGL/egl.h include/GLES2/gl2.h \
                include/gbm.h include/xf86drm.h; do
        if [[ -f "${SYSROOT}/usr/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "Mesa software rendering stack ready."
}

main() {
    log "=== Building Mesa softpipe for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    [[ -f "${SYSROOT}/usr/lib/libz.so" ]] || die "zlib not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/libexpat.so" ]] || die "expat not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/libwayland-client.so" ]] || die "Wayland not found. Run build-wayland.sh first."
    command -v meson &>/dev/null || die "meson not found."
    command -v ninja &>/dev/null || die "ninja not found."
    python3 -c "import mako" 2>/dev/null || die "python3-mako not found."

    build_libdrm
    build_mesa
    build_libepoxy
    verify

    log "=== Mesa build complete ==="
}

main "$@"

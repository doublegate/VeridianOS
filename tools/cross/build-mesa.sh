#!/usr/bin/env bash
# Build Mesa with softpipe for VeridianOS
#
# Produces EGL, GLES2, GBM, and DRM libraries using Mesa's
# softpipe gallium driver (pure software rendering, no GPU needed).
#
# Prerequisites:
#   - musl libc + zlib + libexpat built (with -fPIC)
#   - musl-g++ wrapper with glibc_shim (for Mesa's C++ code)
#   - meson + ninja + python3-mako
#
# Output:
#   $SYSROOT/usr/lib/libdrm.a
#   $SYSROOT/usr/lib/libEGL.so
#   $SYSROOT/usr/lib/libGLESv2.so
#   $SYSROOT/usr/lib/libgbm.so
#   $SYSROOT/usr/lib/libglapi.so
#   $SYSROOT/usr/lib/libgallium-24.2.8.so

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/mesa"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
JOBS="${JOBS:-$(nproc)}"
CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
CXX="${SYSROOT}/bin/x86_64-veridian-musl-g++"

LIBDRM_VER="2.4.123"
MESA_VER="24.2.8"

log() { echo "[build-mesa] $*"; }
die() { echo "[build-mesa] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

# ── Fetch helper ──────────────────────────────────────────────────────
fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${BUILD_DIR}/${name}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        curl -fsSL -o "${tarball}" -L "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── Generate meson cross file with resolved sysroot ──────────────────
# Uses -fPIC so static archives can be linked into Mesa's shared objects.
generate_meson_cross() {
    local cross_file="${BUILD_DIR}/meson-cross.txt"
    cat > "${cross_file}" << CROSSEOF
[binaries]
c = '${CC}'
cpp = '${CXX}'
ar = 'ar'
strip = 'strip'
pkgconfig = 'pkg-config'

[built-in options]
c_args = ['-fPIC']
c_link_args = []
cpp_args = ['-fPIC']
cpp_link_args = []

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

# ── 1. libdrm ────────────────────────────────────────────────────────
build_libdrm() {
    if [[ -f "${SYSROOT}/usr/lib/libdrm.a" ]]; then
        log "libdrm: already installed."
        return 0
    fi
    fetch "libdrm-${LIBDRM_VER}" \
        "https://dri.freedesktop.org/libdrm/libdrm-${LIBDRM_VER}.tar.xz" \
        "libdrm-${LIBDRM_VER}"

    local src="${BUILD_DIR}/libdrm-${LIBDRM_VER}"
    local bld="${BUILD_DIR}/libdrm-build"
    local cross_file
    cross_file="$(generate_meson_cross)"

    log "Building libdrm ${LIBDRM_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${cross_file}" \
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
            -Dexynos=disabled \
            -Dtests=false \
            -Dman-pages=disabled \
            -Dvalgrind=disabled \
            -Dcairo-tests=disabled \
            -Dudev=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "libdrm: done."
}

# ── 2. Mesa (softpipe) ──────────────────────────────────────────────
build_mesa() {
    if [[ -f "${SYSROOT}/usr/lib/libEGL.so" ]] && \
       [[ -f "${SYSROOT}/usr/lib/libGLESv2.so" ]]; then
        log "Mesa: already installed."
        return 0
    fi
    fetch "mesa-${MESA_VER}" \
        "https://archive.mesa3d.org/mesa-${MESA_VER}.tar.xz" \
        "mesa-${MESA_VER}"

    local src="${BUILD_DIR}/mesa-${MESA_VER}"
    local bld="${BUILD_DIR}/mesa-build"
    local cross_file
    cross_file="$(generate_meson_cross)"

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

    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

    # Mesa builds EGL/GBM/GLESv2 as shared objects (.so) even with
    # --default-library=static. shared-glapi=enabled is required for GLES2.
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${cross_file}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dplatforms= \
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
            -Ddri3=disabled \
            -Dglvnd=disabled \
            -Dvalgrind=disabled \
            -Dlibunwind=disabled \
            -Dlmsensors=disabled \
            -Dbuild-tests=false \
            -Dselinux=false \
            -Dosmesa=false \
            -Dxlib-lease=disabled \
            -Dgallium-vdpau=disabled \
            -Dgallium-va=disabled \
            -Dgallium-xa=disabled \
            -Dgallium-nine=false \
            -Dvideo-codecs= \
            -Dpower8=disabled \
            -Dzstd=disabled && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "Mesa: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Mesa installation..."
    local errors=0
    for lib in libdrm.a libEGL.so libGLESv2.so libgbm.so libglapi.so; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]] || \
           [[ -L "${SYSROOT}/usr/lib/${lib}" ]]; then
            local target="${SYSROOT}/usr/lib/${lib}"
            # Resolve symlinks to get actual file size
            while [[ -L "${target}" ]]; do
                target="${SYSROOT}/usr/lib/$(readlink "${target}")"
            done
            local size
            size=$(stat -c%s "${target}" 2>/dev/null || echo "?")
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
    log "Sysroot: ${SYSROOT}"

    # Verify prerequisites
    [[ -f "${SYSROOT}/usr/lib/libc.a" ]] || die "musl libc not found. Run build-musl.sh first."
    [[ -f "${SYSROOT}/usr/lib/libz.a" ]] || die "zlib not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/libexpat.a" ]] || die "expat not found. Run build-deps.sh first."
    [[ -f "${CXX}" ]] || die "C++ cross-compiler not found at ${CXX}"
    command -v meson &>/dev/null || die "meson not found."
    command -v ninja &>/dev/null || die "ninja not found."
    python3 -c "import mako" 2>/dev/null || die "python3-mako not found."

    build_libdrm
    build_mesa
    verify

    log "=== Mesa build complete ==="
}

main "$@"

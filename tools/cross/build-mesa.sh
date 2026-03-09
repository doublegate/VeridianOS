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
# Output (all static archives for static Qt6 linking):
#   $SYSROOT/usr/lib/libdrm.a
#   $SYSROOT/usr/lib/libEGL.a
#   $SYSROOT/usr/lib/libGLESv2.a
#   $SYSROOT/usr/lib/libgbm.a
#   $SYSROOT/usr/lib/libglapi.a

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
pkg-config = 'pkg-config'

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
    if [[ -f "${SYSROOT}/usr/lib/libEGL.a" ]] && \
       [[ -f "${SYSROOT}/usr/lib/libGLESv2.a" ]]; then
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

    # Build Mesa with --default-library=static for internal libraries.
    # shared-glapi must be enabled (Mesa requires it for EGL + GLES2).
    # NOTE: Mesa hardcodes EGL/GBM/GLES2/glapi as shared_library() in its
    # meson.build, so --default-library=static only affects internal libs.
    # We create static archives from the .so object files in a post-step.
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

    # Mesa hardcodes EGL/GBM/GLES2/glapi as shared_library() in meson,
    # ignoring --default-library=static. Create static archives from the
    # compiled object files, then remove the .so files from the sysroot.
    log "Creating static archives from Mesa shared library objects..."
    local mesa_bld="${bld}"

    # Step 1: Create base archives from .so object files.
    # Use find instead of glob: Mesa generates dot-prefixed .o files
    # (e.g. .._entry.c.o) that shell globs skip by default.
    find "${mesa_bld}/src/mapi/shared-glapi/libglapi.so.0.0.0.p" -name '*.o' -print0 \
        | xargs -0 ar rcs "${SYSROOT}/usr/lib/libglapi.a"

    local tmp_egl="${mesa_bld}/libEGL_base.a"
    find "${mesa_bld}/src/egl/libEGL.so.1.0.0.p" -name '*.o' -print0 \
        | xargs -0 ar rcs "${tmp_egl}"

    local tmp_gbm="${mesa_bld}/libgbm_base.a"
    find "${mesa_bld}/src/gbm/libgbm.so.1.0.0.p" -name '*.o' -print0 \
        | xargs -0 ar rcs "${tmp_gbm}"

    # Step 2: Create combined gallium archive from all Mesa internal static libs
    # Also collect the DRI target objects (contains dri_loader_get_extensions)
    local tmp_dri="${mesa_bld}/libdri_target.a"
    local dri_target_dir="${mesa_bld}/src/gallium/targets/dri"
    find "${dri_target_dir}" -name '*.o' -print0 2>/dev/null \
        | xargs -0 ar rcs "${tmp_dri}" 2>/dev/null || true

    local gallium_archive="${SYSROOT}/usr/lib/libmesa_gallium.a"
    rm -f "${gallium_archive}"
    local tmp_mri="${mesa_bld}/combine.mri"
    echo "create ${gallium_archive}" > "${tmp_mri}"
    for lib in \
        "${mesa_bld}/src/gallium/auxiliary/libgallium.a" \
        "${mesa_bld}/src/gallium/auxiliary/libgalliumvl.a" \
        "${mesa_bld}/src/gallium/drivers/softpipe/libsoftpipe.a" \
        "${mesa_bld}/src/gallium/frontends/dri/libdri.a" \
        "${mesa_bld}/src/gallium/winsys/sw/dri/libswdri.a" \
        "${mesa_bld}/src/gallium/winsys/sw/kms-dri/libswkmsdri.a" \
        "${mesa_bld}/src/gallium/winsys/sw/null/libws_null.a" \
        "${mesa_bld}/src/gallium/winsys/sw/wrapper/libwsw.a" \
        "${mesa_bld}/src/gallium/auxiliary/pipe-loader/libpipe_loader_static.a" \
        "${mesa_bld}/src/mesa/libmesa.a" \
        "${mesa_bld}/src/mesa/libmesa_sse41.a" \
        "${mesa_bld}/src/compiler/libcompiler.a" \
        "${mesa_bld}/src/compiler/nir/libnir.a" \
        "${mesa_bld}/src/compiler/glsl/libglsl.a" \
        "${mesa_bld}/src/compiler/glsl/glcpp/libglcpp.a" \
        "${mesa_bld}/src/compiler/spirv/libvtn.a" \
        "${mesa_bld}/src/compiler/isaspec/libisaspec.a" \
        "${mesa_bld}/src/loader/libloader.a" \
        "${mesa_bld}/src/util/libmesa_util.a" \
        "${mesa_bld}/src/util/libmesa_util_sse41.a" \
        "${mesa_bld}/src/util/blake3/libblake3.a" \
        "${mesa_bld}/src/util/libxmlconfig.a" \
        "${mesa_bld}/src/c11/impl/libmesa_util_c11.a" \
        "${tmp_dri}" \
    ; do
        [[ -f "$lib" ]] && echo "addlib $lib" >> "${tmp_mri}"
    done
    echo "save" >> "${tmp_mri}"
    echo "end" >> "${tmp_mri}"
    ar -M < "${tmp_mri}"
    ranlib "${gallium_archive}"
    rm -f "${tmp_mri}"

    # Step 3: Create "fat" archives -- each public library (EGL, GBM, GLES2)
    # includes all Mesa internals so consumers get a self-contained static
    # library without needing to know about Mesa's internal architecture.
    # This is critical because cmake Find modules (FindEGL.cmake, etc.) only
    # link -lEGL, not the full dependency chain.
    # Helper: create fat archive = base + gallium + glapi
    _create_fat_archive() {
        local output="$1" base="$2"
        local mri="${mesa_bld}/fat_$(basename "$output" .a).mri"
        echo "create ${output}" > "${mri}"
        echo "addlib ${base}" >> "${mri}"
        echo "addlib ${gallium_archive}" >> "${mri}"
        echo "addlib ${SYSROOT}/usr/lib/libglapi.a" >> "${mri}"
        echo "save" >> "${mri}"
        echo "end" >> "${mri}"
        ar -M < "${mri}"
        ranlib "${output}"
        rm -f "${mri}"
    }

    _create_fat_archive "${SYSROOT}/usr/lib/libEGL.a" "${tmp_egl}"
    _create_fat_archive "${SYSROOT}/usr/lib/libgbm.a" "${tmp_gbm}"
    # GLES2 is a glapi shim -- fat archive with gallium for link completeness
    _create_fat_archive "${SYSROOT}/usr/lib/libGLESv2.a" "${SYSROOT}/usr/lib/libglapi.a"

    rm -f "${tmp_egl}" "${tmp_gbm}" "${tmp_dri}"

    # Remove .so files -- we want ONLY static archives in the sysroot
    rm -f "${SYSROOT}/usr/lib/libEGL.so"* \
          "${SYSROOT}/usr/lib/libGLESv2.so"* \
          "${SYSROOT}/usr/lib/libgbm.so"* \
          "${SYSROOT}/usr/lib/libglapi.so"* \
          "${SYSROOT}/usr/lib/libgallium"*.so*

    # Rewrite pkg-config files with proper static link dependencies.
    # Mesa's generated .pc files assume shared linking; for static builds
    # consumers need the full internal dependency chain.
    cat > "${SYSROOT}/usr/lib/pkgconfig/egl.pc" << PCEOF
prefix=${SYSROOT}/usr
includedir=\${prefix}/include
libdir=\${prefix}/lib

Name: egl
Description: Mesa EGL Library (static)
Version: ${MESA_VER}
Requires.private: libdrm >= 2.4.75
Libs: -L\${libdir} -lEGL
Libs.private: -lmesa_gallium -lglapi -ldrm -lexpat -lz -lpthread -lm -ldl
Cflags: -I\${includedir}
PCEOF
    cat > "${SYSROOT}/usr/lib/pkgconfig/glesv2.pc" << PCEOF
prefix=${SYSROOT}/usr
includedir=\${prefix}/include
libdir=\${prefix}/lib

Name: glesv2
Description: Mesa OpenGL ES 2.0 library (static)
Version: ${MESA_VER}
Libs: -L\${libdir} -lGLESv2
Libs.private: -lmesa_gallium -lglapi -lpthread -lm -ldl
Cflags: -I\${includedir}
PCEOF
    cat > "${SYSROOT}/usr/lib/pkgconfig/gbm.pc" << PCEOF
prefix=${SYSROOT}/usr
includedir=\${prefix}/include
libdir=\${prefix}/lib

Name: gbm
Description: Mesa gbm library (static)
Version: ${MESA_VER}
Libs: -L\${libdir} -lgbm
Libs.private: -lmesa_gallium -lglapi -ldrm -lexpat -lz -lpthread -lm -ldl
Cflags: -I\${includedir}
PCEOF
    log "Mesa: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying Mesa installation..."
    local errors=0
    for lib in libdrm.a libEGL.a libGLESv2.a libgbm.a libglapi.a libmesa_gallium.a; do
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

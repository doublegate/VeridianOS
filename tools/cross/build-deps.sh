#!/usr/bin/env bash
# Build C library dependencies for KDE on VeridianOS
#
# Build order (dependency chain):
#   zlib -> libffi -> pcre2 -> libxml2 -> libjpeg-turbo -> libpng -> libxkbcommon
#
# All libraries are built as static archives (.a) into the sysroot.
#
# Prerequisites:
#   - musl libc built (run build-musl.sh first)
#   - cmake, meson, ninja for some libraries

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/deps"
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
MESON_CROSS="${SCRIPT_DIR}/meson-cross-veridian.txt"
JOBS="${JOBS:-$(nproc)}"
CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"

# Library versions
ZLIB_VER="1.3.1"
LIBFFI_VER="3.4.6"
PCRE2_VER="10.43"
LIBXML2_VER="2.12.6"
LIBJPEG_VER="3.0.3"
LIBPNG_VER="1.6.43"
XKBCOMMON_VER="1.7.0"
EXPAT_VER="2.6.2"

log() { echo "[build-deps] $*"; }
die() { echo "[build-deps] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

# ── Helper: download + extract ────────────────────────────────────────
fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${BUILD_DIR}/${name}.tar.gz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        curl -fsSL -o "${tarball}" "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xzf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── Common configure flags for autotools builds ───────────────────────
COMMON_CONFIGURE=(
    --host=x86_64-veridian
    --prefix="${SYSROOT}/usr"
    --enable-static
    --disable-shared
)

export CC CFLAGS="-O2 --sysroot=${SYSROOT}"
export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

# ── 1. zlib ───────────────────────────────────────────────────────────
build_zlib() {
    if [[ -f "${SYSROOT}/usr/lib/libz.a" ]]; then
        log "zlib: already installed."
        return 0
    fi
    fetch "zlib-${ZLIB_VER}" \
        "https://zlib.net/zlib-${ZLIB_VER}.tar.gz" \
        "zlib-${ZLIB_VER}"

    local src="${BUILD_DIR}/zlib-${ZLIB_VER}"
    log "Building zlib ${ZLIB_VER}..."
    (cd "${src}" && \
        ./configure --static --prefix="${SYSROOT}/usr" && \
        make -j"${JOBS}" && \
        make install)
    log "zlib: done."
}

# ── 2. libffi ─────────────────────────────────────────────────────────
build_libffi() {
    if [[ -f "${SYSROOT}/usr/lib/libffi.a" ]]; then
        log "libffi: already installed."
        return 0
    fi
    fetch "libffi-${LIBFFI_VER}" \
        "https://github.com/libffi/libffi/releases/download/v${LIBFFI_VER}/libffi-${LIBFFI_VER}.tar.gz" \
        "libffi-${LIBFFI_VER}"

    local src="${BUILD_DIR}/libffi-${LIBFFI_VER}"
    log "Building libffi ${LIBFFI_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" && \
        make -j"${JOBS}" && \
        make install)
    log "libffi: done."
}

# ── 3. pcre2 ──────────────────────────────────────────────────────────
build_pcre2() {
    if [[ -f "${SYSROOT}/usr/lib/libpcre2-8.a" ]]; then
        log "pcre2: already installed."
        return 0
    fi
    fetch "pcre2-${PCRE2_VER}" \
        "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-${PCRE2_VER}/pcre2-${PCRE2_VER}.tar.gz" \
        "pcre2-${PCRE2_VER}"

    local src="${BUILD_DIR}/pcre2-${PCRE2_VER}"
    log "Building pcre2 ${PCRE2_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --enable-unicode \
            --enable-pcre2-8 \
            --disable-pcre2-16 \
            --disable-pcre2-32 && \
        make -j"${JOBS}" && \
        make install)
    log "pcre2: done."
}

# ── 4. expat (needed by D-Bus and Fontconfig) ─────────────────────────
build_expat() {
    if [[ -f "${SYSROOT}/usr/lib/libexpat.a" ]]; then
        log "expat: already installed."
        return 0
    fi
    fetch "expat-${EXPAT_VER}" \
        "https://github.com/libexpat/libexpat/releases/download/R_${EXPAT_VER//./_}/expat-${EXPAT_VER}.tar.gz" \
        "expat-${EXPAT_VER}"

    local src="${BUILD_DIR}/expat-${EXPAT_VER}"
    log "Building expat ${EXPAT_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --without-docbook && \
        make -j"${JOBS}" && \
        make install)
    log "expat: done."
}

# ── 5. libxml2 (needs zlib) ──────────────────────────────────────────
build_libxml2() {
    if [[ -f "${SYSROOT}/usr/lib/libxml2.a" ]]; then
        log "libxml2: already installed."
        return 0
    fi
    fetch "libxml2-${LIBXML2_VER}" \
        "https://download.gnome.org/sources/libxml2/2.12/libxml2-${LIBXML2_VER}.tar.xz" \
        "libxml2-${LIBXML2_VER}"

    local src="${BUILD_DIR}/libxml2-${LIBXML2_VER}"
    log "Building libxml2 ${LIBXML2_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --without-python \
            --without-lzma \
            --without-icu \
            --with-zlib="${SYSROOT}/usr" && \
        make -j"${JOBS}" && \
        make install)
    log "libxml2: done."
}

# ── 6. libjpeg-turbo (cmake) ─────────────────────────────────────────
build_libjpeg() {
    if [[ -f "${SYSROOT}/usr/lib/libjpeg.a" ]]; then
        log "libjpeg-turbo: already installed."
        return 0
    fi
    fetch "libjpeg-turbo-${LIBJPEG_VER}" \
        "https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/${LIBJPEG_VER}/libjpeg-turbo-${LIBJPEG_VER}.tar.gz" \
        "libjpeg-turbo-${LIBJPEG_VER}"

    local src="${BUILD_DIR}/libjpeg-turbo-${LIBJPEG_VER}"
    local bld="${BUILD_DIR}/libjpeg-build"
    log "Building libjpeg-turbo ${LIBJPEG_VER}..."
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DENABLE_SHARED=OFF \
            -DENABLE_STATIC=ON \
            -DWITH_TURBOJPEG=OFF && \
        make -j"${JOBS}" && \
        make install)
    log "libjpeg-turbo: done."
}

# ── 7. libpng (needs zlib) ───────────────────────────────────────────
build_libpng() {
    if [[ -f "${SYSROOT}/usr/lib/libpng.a" ]] || [[ -f "${SYSROOT}/usr/lib/libpng16.a" ]]; then
        log "libpng: already installed."
        return 0
    fi
    fetch "libpng-${LIBPNG_VER}" \
        "https://download.sourceforge.net/libpng/libpng-${LIBPNG_VER}.tar.gz" \
        "libpng-${LIBPNG_VER}"

    local src="${BUILD_DIR}/libpng-${LIBPNG_VER}"
    log "Building libpng ${LIBPNG_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            LDFLAGS="-L${SYSROOT}/usr/lib" \
            CPPFLAGS="-I${SYSROOT}/usr/include" && \
        make -j"${JOBS}" && \
        make install)
    log "libpng: done."
}

# ── 8. libxkbcommon (meson, needs libxml2) ────────────────────────────
build_xkbcommon() {
    if [[ -f "${SYSROOT}/usr/lib/libxkbcommon.a" ]]; then
        log "libxkbcommon: already installed."
        return 0
    fi
    fetch "libxkbcommon-${XKBCOMMON_VER}" \
        "https://xkbcommon.org/download/libxkbcommon-${XKBCOMMON_VER}.tar.xz" \
        "libxkbcommon-${XKBCOMMON_VER}"

    local src="${BUILD_DIR}/libxkbcommon-${XKBCOMMON_VER}"
    local bld="${BUILD_DIR}/xkbcommon-build"
    log "Building libxkbcommon ${XKBCOMMON_VER}..."
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Denable-wayland=false \
            -Denable-x11=false \
            -Denable-docs=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "libxkbcommon: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying all dependencies..."
    local errors=0
    for lib in libz.a libffi.a libpcre2-8.a libexpat.a libxml2.a libjpeg.a libpng16.a libxkbcommon.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            local size
            size=$(stat -c%s "${SYSROOT}/usr/lib/${lib}" 2>/dev/null || echo "?")
            log "  OK: ${lib} (${size} bytes)"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} libraries missing!"
    fi
    log "All dependencies installed."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building C dependencies for VeridianOS KDE ==="
    log "Sysroot: ${SYSROOT}"

    build_zlib
    build_libffi
    build_pcre2
    build_expat
    build_libxml2
    build_libjpeg
    build_libpng
    build_xkbcommon

    verify

    log "=== All dependencies built ==="
}

main "$@"

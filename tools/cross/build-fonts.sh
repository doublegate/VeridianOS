#!/usr/bin/env bash
# Build font stack for VeridianOS KDE
#
# Handles the FreeType <-> HarfBuzz circular dependency:
#   1. Build FreeType WITHOUT HarfBuzz
#   2. Build HarfBuzz WITH FreeType
#   3. Rebuild FreeType WITH HarfBuzz
#   4. Build Fontconfig (needs FreeType + expat)
#
# Prerequisites:
#   - musl libc + zlib + libpng + expat built

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/fonts"
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
MESON_CROSS="${SCRIPT_DIR}/meson-cross-veridian.txt"
JOBS="${JOBS:-$(nproc)}"

FREETYPE_VER="2.13.2"
HARFBUZZ_VER="8.3.0"
FONTCONFIG_VER="2.15.0"

log() { echo "[build-fonts] $*"; }
die() { echo "[build-fonts] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

fetch() {
    local name="$1" url="$2" dir="$3"
    local ext="${4:-.tar.xz}"
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

export CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
export CFLAGS="-O2 --sysroot=${SYSROOT}"
export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

COMMON_CONFIGURE=(
    --host=x86_64-veridian
    --prefix="${SYSROOT}/usr"
    --enable-static
    --disable-shared
)

# ── 1. FreeType (pass 1 -- no HarfBuzz) ──────────────────────────────
build_freetype_pass1() {
    fetch "freetype-${FREETYPE_VER}" \
        "https://download.savannah.gnu.org/releases/freetype/freetype-${FREETYPE_VER}.tar.xz" \
        "freetype-${FREETYPE_VER}"

    local src="${BUILD_DIR}/freetype-${FREETYPE_VER}"
    local bld="${BUILD_DIR}/freetype-build-pass1"
    log "Building FreeType ${FREETYPE_VER} (pass 1, no HarfBuzz)..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${src}/configure" "${COMMON_CONFIGURE[@]}" \
            --with-zlib=yes \
            --with-png=yes \
            --with-harfbuzz=no \
            --with-bzip2=no \
            --with-brotli=no \
            ZLIB_CFLAGS="-I${SYSROOT}/usr/include" \
            ZLIB_LIBS="-L${SYSROOT}/usr/lib -lz" \
            LIBPNG_CFLAGS="-I${SYSROOT}/usr/include" \
            LIBPNG_LIBS="-L${SYSROOT}/usr/lib -lpng16 -lz" && \
        make -j"${JOBS}" && \
        make install)
    log "FreeType pass 1: done."
}

# ── 2. HarfBuzz (with FreeType) ──────────────────────────────────────
build_harfbuzz() {
    if [[ -f "${SYSROOT}/usr/lib/libharfbuzz.a" ]]; then
        log "HarfBuzz: already installed."
        return 0
    fi
    fetch "harfbuzz-${HARFBUZZ_VER}" \
        "https://github.com/harfbuzz/harfbuzz/releases/download/${HARFBUZZ_VER}/harfbuzz-${HARFBUZZ_VER}.tar.xz" \
        "harfbuzz-${HARFBUZZ_VER}"

    local src="${BUILD_DIR}/harfbuzz-${HARFBUZZ_VER}"
    local bld="${BUILD_DIR}/harfbuzz-build"
    log "Building HarfBuzz ${HARFBUZZ_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dfreetype=enabled \
            -Dglib=disabled \
            -Dgobject=disabled \
            -Dcairo=disabled \
            -Dicu=disabled \
            -Dgraphite2=disabled \
            -Dtests=disabled \
            -Ddocs=disabled && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "HarfBuzz: done."
}

# ── 3. FreeType (pass 2 -- with HarfBuzz) ────────────────────────────
build_freetype_pass2() {
    local src="${BUILD_DIR}/freetype-${FREETYPE_VER}"
    local bld="${BUILD_DIR}/freetype-build-pass2"
    log "Rebuilding FreeType ${FREETYPE_VER} (pass 2, with HarfBuzz)..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${src}/configure" "${COMMON_CONFIGURE[@]}" \
            --with-zlib=yes \
            --with-png=yes \
            --with-harfbuzz=yes \
            --with-bzip2=no \
            --with-brotli=no \
            ZLIB_CFLAGS="-I${SYSROOT}/usr/include" \
            ZLIB_LIBS="-L${SYSROOT}/usr/lib -lz" \
            LIBPNG_CFLAGS="-I${SYSROOT}/usr/include" \
            LIBPNG_LIBS="-L${SYSROOT}/usr/lib -lpng16 -lz" \
            HARFBUZZ_CFLAGS="-I${SYSROOT}/usr/include/harfbuzz" \
            HARFBUZZ_LIBS="-L${SYSROOT}/usr/lib -lharfbuzz" && \
        make -j"${JOBS}" && \
        make install)
    log "FreeType pass 2: done."
}

# ── 4. Fontconfig ─────────────────────────────────────────────────────
build_fontconfig() {
    if [[ -f "${SYSROOT}/usr/lib/libfontconfig.a" ]]; then
        log "Fontconfig: already installed."
        return 0
    fi
    fetch "fontconfig-${FONTCONFIG_VER}" \
        "https://www.freedesktop.org/software/fontconfig/release/fontconfig-${FONTCONFIG_VER}.tar.xz" \
        "fontconfig-${FONTCONFIG_VER}"

    local src="${BUILD_DIR}/fontconfig-${FONTCONFIG_VER}"
    local bld="${BUILD_DIR}/fontconfig-build"
    log "Building Fontconfig ${FONTCONFIG_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Ddoc=disabled \
            -Dtests=disabled \
            -Dtools=disabled && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "Fontconfig: done."
}

# ── Install base fonts ────────────────────────────────────────────────
install_fonts() {
    local fontdir="${SYSROOT}/usr/share/fonts/truetype"
    if [[ -f "${fontdir}/DejaVuSans.ttf" ]]; then
        log "Base fonts: already installed."
        return 0
    fi
    mkdir -p "${fontdir}"
    log "Installing DejaVu Sans fonts..."
    # Try to copy from system
    for d in /usr/share/fonts/truetype/dejavu /usr/share/fonts/dejavu /usr/share/fonts/TTF; do
        if [[ -f "${d}/DejaVuSans.ttf" ]]; then
            cp "${d}"/DejaVu*.ttf "${fontdir}/"
            log "Copied from ${d}"
            return 0
        fi
    done
    log "WARNING: DejaVu fonts not found on system. Download manually to ${fontdir}/"
}

# ── Create minimal fonts.conf ─────────────────────────────────────────
create_fonts_conf() {
    local conf="${SYSROOT}/etc/fonts/fonts.conf"
    if [[ -f "${conf}" ]]; then
        return 0
    fi
    mkdir -p "$(dirname "${conf}")"
    cat > "${conf}" << 'CONF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>/usr/share/fonts</dir>
  <cachedir>/tmp/fontconfig-cache</cachedir>
  <match target="pattern">
    <test qual="any" name="family">
      <string>sans-serif</string>
    </test>
    <edit name="family" mode="assign" binding="same">
      <string>DejaVu Sans</string>
    </edit>
  </match>
  <match target="pattern">
    <test qual="any" name="family">
      <string>monospace</string>
    </test>
    <edit name="family" mode="assign" binding="same">
      <string>DejaVu Sans Mono</string>
    </edit>
  </match>
</fontconfig>
CONF
    log "Created ${conf}"
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying font stack..."
    local errors=0
    for lib in libfreetype.a libharfbuzz.a libfontconfig.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            log "  OK: ${lib}"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} libraries missing!"
    fi
    log "Font stack ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building font stack for VeridianOS ==="
    build_freetype_pass1
    build_harfbuzz
    build_freetype_pass2
    build_fontconfig
    install_fonts
    create_fonts_conf
    verify
    log "=== Font stack build complete ==="
}

main "$@"

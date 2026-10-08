#!/usr/bin/env bash
# Build font stack for VeridianOS KDE
#
# Handles the FreeType <-> HarfBuzz circular dependency:
#   1. Build FreeType WITHOUT HarfBuzz
#   2. Build HarfBuzz WITH FreeType
#   3. Rebuild FreeType WITH HarfBuzz
#   4. Build Fontconfig (needs FreeType + expat), with its configuration
#      in /etc/fonts on the target
#   5. Install the DejaVu fonts and their fontconfig rules
#
# Built and staged as lib/cross-env.sh describes; gperf is built for the
# host tools first.
# Prerequisites: build-deps.sh (zlib, bzip2, libpng, expat).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/fonts"
JOBS="${JOBS:-$(nproc)}"

# Checksums pinned when first downloaded; DejaVu's as Alpine's (SHA-512
# bafa3932...0e00654).
FREETYPE_VER="2.14.3"
FREETYPE_SHA256="36bc4f1cc413335368ee656c42afca65c5a3987e8768cc28cf11ba775e785a5f"
HARFBUZZ_VER="14.6.0"
HARFBUZZ_SHA256="d07a007327277708a2a73ae437887cdbaf282937f6d03ca5467723e9099af586"
FONTCONFIG_VER="2.18.3"
FONTCONFIG_SHA256="4f7b554a38cdf78c033f666c8871f3749e14a094f65a07f630c91ed0b43d35e3"
DEJAVU_VER="2.37"
DEJAVU_SHA256="fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7"
# gperf generates fontconfig's object-name hash; without it on the host
# fontconfig builds gperf as a subproject. Signed by Bruno Haible
# (E0FFBD975397F77A32AB76ECB6301D9E1BBEAC08).
GPERF_VER="3.3"
GPERF_SHA256="fd87e0aba7e43ae054837afd6cd4db03a3f2693deb3619085e6ed9d8d9604ad8"

log() { echo "[build-fonts] $*"; }
die() { echo "[build-fonts] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

FREETYPE_SRC="${BUILD_DIR}/freetype-${FREETYPE_VER}"

# ── gperf (host) ──────────────────────────────────────────────────────
build_host_gperf() {
    if [[ "$("${VERIDIAN_HOST_TOOLS}/bin/gperf" --version 2>/dev/null | head -1)" == "GNU gperf ${GPERF_VER}" ]]; then
        log "gperf (host): already installed."
        return 0
    fi
    fetch "gperf-${GPERF_VER}.tar.gz" "https://ftp.gnu.org/gnu/gperf/gperf-${GPERF_VER}.tar.gz" \
        "gperf-${GPERF_VER}" "${GPERF_SHA256}"
    log "Building gperf ${GPERF_VER} (host)..."
    (cd "${BUILD_DIR}/gperf-${GPERF_VER}" && \
        env -u CC -u CXX -u AR -u RANLIB -u NM -u STRIP -u CFLAGS -u CXXFLAGS \
            ./configure --prefix="${VERIDIAN_HOST_TOOLS}" && \
        make -j"${JOBS}" && \
        make install)
}

fetch_freetype() {
    fetch "freetype-${FREETYPE_VER}.tar.xz" \
        "https://download.savannah.gnu.org/releases/freetype/freetype-${FREETYPE_VER}.tar.xz" \
        "freetype-${FREETYPE_VER}" "${FREETYPE_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/freetype_ftstroke_point.py" "${FREETYPE_SRC}/src/base/ftstroke.c" ||
        die "failed to patch FreeType ftstroke.c"
}

# build_freetype BUILD HARFBUZZ(enabled|disabled): meson build, staged.
build_freetype() {
    meson_build "${FREETYPE_SRC}" "$1" \
        -Dharfbuzz="$2" \
        -Dzlib=system \
        -Dpng=enabled \
        -Dbzip2=enabled \
        -Dbrotli=disabled \
        -Dtests=disabled
}

# ── 1. FreeType (pass 1 -- no HarfBuzz) ──────────────────────────────
build_freetype_pass1() {
    installed libharfbuzz.so "FreeType pass 1 (HarfBuzz present)" && return 0
    fetch_freetype
    log "Building FreeType ${FREETYPE_VER} (pass 1, no HarfBuzz)..."
    build_freetype "${BUILD_DIR}/freetype-build-pass1" disabled
}

# ── 2. HarfBuzz (with FreeType) ──────────────────────────────────────
build_harfbuzz() {
    installed libharfbuzz.so HarfBuzz && return 0
    fetch "harfbuzz-${HARFBUZZ_VER}.tar.xz" \
        "https://github.com/harfbuzz/harfbuzz/releases/download/${HARFBUZZ_VER}/harfbuzz-${HARFBUZZ_VER}.tar.xz" \
        "harfbuzz-${HARFBUZZ_VER}" "${HARFBUZZ_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/fonts_warnings.py" harfbuzz \
        "${BUILD_DIR}/harfbuzz-${HARFBUZZ_VER}" || die "failed to patch HarfBuzz"
    log "Building HarfBuzz ${HARFBUZZ_VER}..."
    meson_build "${BUILD_DIR}/harfbuzz-${HARFBUZZ_VER}" "${BUILD_DIR}/harfbuzz-build" \
        -Dfreetype=enabled \
        -Dglib=disabled \
        -Dgobject=disabled \
        -Dcairo=disabled \
        -Dicu=disabled \
        -Dgraphite2=disabled \
        -Dtests=disabled \
        -Ddocs=disabled
}

# ── 3. FreeType (pass 2 -- with HarfBuzz) ────────────────────────────
build_freetype_pass2() {
    if [[ -f "${SYSROOT}/usr/lib/libfontconfig.so" ]] && \
       grep -q harfbuzz "${SYSROOT}/usr/lib/pkgconfig/freetype2.pc"; then
        log "FreeType pass 2: already installed."
        return 0
    fi
    fetch_freetype
    log "Rebuilding FreeType ${FREETYPE_VER} (pass 2, with HarfBuzz)..."
    build_freetype "${BUILD_DIR}/freetype-build-pass2" enabled
}

# ── 4. Fontconfig ─────────────────────────────────────────────────────
build_fontconfig() {
    installed libfontconfig.so Fontconfig && return 0
    fetch "fontconfig-${FONTCONFIG_VER}.tar.xz" \
        "https://gitlab.freedesktop.org/api/v4/projects/890/packages/generic/fontconfig/${FONTCONFIG_VER}/fontconfig-${FONTCONFIG_VER}.tar.xz" \
        "fontconfig-${FONTCONFIG_VER}" "${FONTCONFIG_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/fonts_warnings.py" fontconfig \
        "${BUILD_DIR}/fontconfig-${FONTCONFIG_VER}" || die "failed to patch Fontconfig"
    log "Building Fontconfig ${FONTCONFIG_VER}..."
    # /etc/fonts and /var/cache/fontconfig on the target (meson's defaults
    # for the /usr prefix), with its default configuration.
    meson_build "${BUILD_DIR}/fontconfig-${FONTCONFIG_VER}" "${BUILD_DIR}/fontconfig-build" \
        -Ddoc=disabled \
        -Dtests=disabled \
        -Dtools=disabled \
        -Dcache-build=disabled
}

# ── 5. DejaVu fonts ───────────────────────────────────────────────────
install_fonts() {
    local fontdir="${SYSROOT}/usr/share/fonts/dejavu"
    if [[ -f "${fontdir}/DejaVuSans.ttf" ]]; then
        log "DejaVu fonts: already installed."
        return 0
    fi
    fetch "dejavu-fonts-ttf-${DEJAVU_VER}.tar.bz2" \
        "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_${DEJAVU_VER//./_}/dejavu-fonts-ttf-${DEJAVU_VER}.tar.bz2" \
        "dejavu-fonts-ttf-${DEJAVU_VER}" "${DEJAVU_SHA256}"
    log "Installing DejaVu ${DEJAVU_VER}..."
    local src="${BUILD_DIR}/dejavu-fonts-ttf-${DEJAVU_VER}"
    install -Dm644 -t "${fontdir}" "${src}"/ttf/*.ttf
    # Its fontconfig rules, enabled as distributions do.
    local conf
    for conf in "${src}"/fontconfig/*.conf; do
        install -Dm644 "${conf}" "${SYSROOT}/usr/share/fontconfig/conf.avail/$(basename "${conf}")"
        ln -sf "/usr/share/fontconfig/conf.avail/$(basename "${conf}")" \
            "${SYSROOT}/etc/fonts/conf.d/$(basename "${conf}")"
    done
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying font stack..."
    local errors=0 item
    for item in usr/lib/libfreetype.so usr/lib/libharfbuzz.so usr/lib/libfontconfig.so \
                etc/fonts/fonts.conf etc/fonts/conf.d/57-dejavu-sans.conf \
                usr/share/fonts/dejavu/DejaVuSans.ttf usr/share/fonts/dejavu/DejaVuSansMono.ttf; do
        if [[ -e "${SYSROOT}/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    grep -q harfbuzz "${SYSROOT}/usr/lib/pkgconfig/freetype2.pc" || {
        log "  FreeType was not built with HarfBuzz"
        errors=$((errors + 1))
    }
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "Font stack ready."
}

main() {
    log "=== Building font stack for VeridianOS ==="
    build_host_gperf
    build_freetype_pass1
    build_harfbuzz
    build_freetype_pass2
    build_fontconfig
    install_fonts
    verify
    log "=== Font stack build complete ==="
}

main "$@"

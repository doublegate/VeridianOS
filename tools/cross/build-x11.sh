#!/usr/bin/env bash
# Build the X11 client libraries for VeridianOS (shared, ADR 0010)
#
# There is no X server (XWayland is deferred: docs/KNOWN-LIMITATIONS.md),
# but KDE needs the client side, as every distribution builds it: Qt's X11
# platform plugin (xcb) and xkbcommon's X11 part, KWindowSystem's X11 API,
# which libplasma and libkscreen use without guards; KWin compiles against
# the XCB headers even with KWIN_BUILD_X11=OFF; the screen locker links
# libX11. This phase runs before build-deps.sh, which builds
# libxkbcommon-x11 against it.
#
#   xorgproto, xtrans             protocol and transport headers
#   xcb-proto                     the XCB protocol descriptions and the
#                                 Python xcbgen module libxcb's build runs
#                                 (its pkg-config file is sysroot-aware)
#   libXau, libXdmcp, libxcb      the XCB core
#   libX11, libXext, libXfixes, libXrender, libXcursor (cursor themes for
#   KDE's platform theme), libXi, libXtst
#   libICE, libSM                 X session management (ksmserver)
#   libxkbfile                    keymap files (plasma-desktop's keyboard
#                                 settings)
#   xcb-util, xcb-util-keysyms, xcb-util-image, xcb-util-renderutil,
#   xcb-util-wm, xcb-util-cursor (Qt's xcb plugin, plasma-desktop)
#
# Checksums: checksums/x11.sha256 (the xcb-util releases verified against
# Alan Coopersmith's signatures, 4A193C06D35E7C670FA4EF0BA2FB9E081F2D130E and
# 3AB285232C46AE43D8E192F4DAB0F78EA6E7E2D2, as libXrender; libXcursor against
# Thomas E. Dickey's, 19882D92DDA4C400C22C0D56CC2AF4472167BE03). Built and staged as
# lib/cross-env.sh describes. Prerequisites: the musl toolchain; a host
# Python 3.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/x11"
JOBS="${JOBS:-$(nproc)}"

log() { echo "[build-x11] $*"; }
die() { echo "[build-x11] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

XORG_URL="https://xorg.freedesktop.org/archive/individual"

# x11_fetch NAME-VERSION SUBDIR: a fresh, checked source tree (path echoed).
x11_fetch() {
    fetch "$1.tar.xz" "${XORG_URL}/$2/$1.tar.xz" "$1" "$(listed_sha256 x11.sha256 "$1.tar.xz")" >&2
    echo "${BUILD_DIR}/$1"
}

# Autoconf answers configure cannot find out by running a program on the
# build host: musl's malloc(0) returns a unique pointer, not NULL.
X11_CACHE=(xorg_cv_malloc0_returns_null=no)

# autotools_x11 NAME-VERSION SUBDIR [CONFIGURE OPTIONS...]
autotools_x11() {
    local name="$1" subdir="$2"
    shift 2
    local src
    src="$(x11_fetch "${name}" "${subdir}")"
    log "Building ${name}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" "${X11_CACHE[@]}" "$@" && \
        make_install)
}

# ── Headers ───────────────────────────────────────────────────────────
build_xorgproto() {
    if [[ -f "${SYSROOT}/usr/share/pkgconfig/xproto.pc" ]]; then
        log "xorgproto: already installed."
        return 0
    fi
    local src
    src="$(x11_fetch xorgproto-2026.1 proto)"
    log "Building xorgproto 2026.1..."
    meson_build "${src}" "${BUILD_DIR}/xorgproto-build" -Dlegacy=false
}

build_xtrans() {
    if [[ -f "${SYSROOT}/usr/share/pkgconfig/xtrans.pc" ]]; then
        log "xtrans: already installed."
        return 0
    fi
    autotools_x11 xtrans-1.6.0 lib --disable-docs
}

# The protocol descriptions (XML) and xcbgen, a pure Python module that
# libxcb's build imports with the host's Python; xcb-proto.pc names both
# through ${pc_sysrootdir}, so libxcb finds them in the sysroot.
build_xcb_proto() {
    if [[ -f "${SYSROOT}/usr/share/pkgconfig/xcb-proto.pc" ]]; then
        log "xcb-proto: already installed."
        return 0
    fi
    local src
    src="$(x11_fetch xcb-proto-1.17.0 proto)"
    log "Building xcb-proto 1.17.0..."
    (cd "${src}" && \
        PYTHON=python3 ./configure --prefix=/usr && \
        make install DESTDIR="${SYSROOT}")
}

# ── XCB ───────────────────────────────────────────────────────────────
build_libxau() {
    installed libXau.so libXau && return 0
    autotools_x11 libXau-1.0.12 lib
}

build_libxdmcp() {
    installed libXdmcp.so libXdmcp && return 0
    autotools_x11 libXdmcp-1.1.5 lib --disable-docs
}

build_libxcb() {
    installed libxcb.so libxcb && return 0
    # The C bindings are generated from the protocol XML by the host's
    # Python; the extensions KDE's clients use are the defaults.
    PYTHON=python3 autotools_x11 libxcb-1.17.0 lib \
        --disable-devel-docs \
        --without-doxygen \
        --enable-xinput \
        --enable-xkb
}

# ── Xlib and extensions ───────────────────────────────────────────────
build_libx11() {
    installed libX11.so libX11 && return 0
    # makekeys runs during the build: built for the host (CC_FOR_BUILD).
    CC_FOR_BUILD=cc CPP_FOR_BUILD="cc -E" autotools_x11 libX11-1.8.13 lib \
        --disable-specs \
        --without-launchd \
        --enable-xthreads
}

build_libxext() {
    installed libXext.so libXext && return 0
    autotools_x11 libXext-1.3.7 lib --disable-specs
}

build_libxfixes() {
    installed libXfixes.so libXfixes && return 0
    autotools_x11 libXfixes-6.0.2 lib
}

build_libxrender() {
    installed libXrender.so libXrender && return 0
    autotools_x11 libXrender-0.9.12 lib
}

build_libxcursor() {
    installed libXcursor.so libXcursor && return 0
    autotools_x11 libXcursor-1.2.3 lib
}

build_libxi() {
    installed libXi.so libXi && return 0
    autotools_x11 libXi-1.8.3 lib --disable-specs --disable-docs
}

build_libxtst() {
    installed libXtst.so libXtst && return 0
    autotools_x11 libXtst-1.2.5 lib --disable-specs
}

build_libxkbfile() {
    installed libxkbfile.so libxkbfile && return 0
    local src
    src="$(x11_fetch libxkbfile-1.2.0 lib)"
    log "Building libxkbfile 1.2.0..."
    meson_build "${src}" "${BUILD_DIR}/libxkbfile-build"
}

# libSM names clients in the XSMP specification's own format (address,
# time, process ID, sequence), not by UUID: libuuid comes later
# (build-deps.sh).
build_libice() {
    installed libICE.so libICE && return 0
    autotools_x11 libICE-1.1.2 lib --disable-docs --disable-specs
}

build_libsm() {
    installed libSM.so libSM && return 0
    autotools_x11 libSM-1.2.6 lib --disable-docs --without-libuuid
}

build_xcb_util() {
    installed libxcb-util.so xcb-util && return 0
    autotools_x11 xcb-util-0.4.1 lib
}

build_xcb_util_keysyms() {
    installed libxcb-keysyms.so xcb-util-keysyms && return 0
    autotools_x11 xcb-util-keysyms-0.4.1 lib
}

build_xcb_util_image() {
    installed libxcb-image.so xcb-util-image && return 0
    autotools_x11 xcb-util-image-0.4.1 lib
}

build_xcb_util_renderutil() {
    installed libxcb-render-util.so xcb-util-renderutil && return 0
    autotools_x11 xcb-util-renderutil-0.3.10 lib
}

build_xcb_util_wm() {
    installed libxcb-icccm.so xcb-util-wm && return 0
    autotools_x11 xcb-util-wm-0.4.2 lib
}

build_xcb_util_cursor() {
    installed libxcb-cursor.so xcb-util-cursor && return 0
    autotools_x11 xcb-util-cursor-0.1.6 lib
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying X11 client libraries..."
    local errors=0 item
    for item in lib/libxcb.so lib/libX11.so lib/libX11-xcb.so lib/libXau.so lib/libXdmcp.so \
                lib/libXext.so lib/libXfixes.so lib/libXrender.so lib/libXcursor.so lib/libXi.so lib/libXtst.so \
                lib/libICE.so lib/libSM.so lib/libxkbfile.so \
                lib/libxcb-util.so lib/libxcb-keysyms.so lib/libxcb-image.so lib/libxcb-render-util.so \
                lib/libxcb-icccm.so lib/libxcb-ewmh.so lib/libxcb-cursor.so lib/libxcb-xkb.so lib/libxcb-xinput.so \
                include/xcb/xcb.h include/X11/Xlib.h include/X11/Xlib-xcb.h \
                share/pkgconfig/xcb-proto.pc share/pkgconfig/xproto.pc; do
        if [[ -e "${SYSROOT}/usr/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "X11 client libraries ready."
}

main() {
    log "=== Building the X11 client libraries for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    build_xorgproto
    build_xtrans
    build_xcb_proto
    build_libxau
    build_libxdmcp
    build_libxcb
    build_libx11
    build_libxext
    build_libxfixes
    build_libxrender
    build_libxcursor
    build_libxi
    build_libxtst
    build_libice
    build_libsm
    build_libxkbfile
    build_xcb_util
    build_xcb_util_keysyms
    build_xcb_util_image
    build_xcb_util_renderutil
    build_xcb_util_wm
    build_xcb_util_cursor
    verify
    log "=== X11 client libraries complete ==="
}

main "$@"

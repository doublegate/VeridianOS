#!/usr/bin/env bash
# Build the X11 client libraries for VeridianOS (shared, ADR 0010)
#
# There is no X server (XWayland is deferred: docs/KNOWN-LIMITATIONS.md),
# but KDE needs the client side: KWin compiles against the XCB headers even
# with KWIN_BUILD_X11=OFF (its effect handler includes its X11 event-filter
# headers unconditionally, upstream too), and the screen locker
# (kscreenlocker) links libX11 and libxcb.
#
#   xorgproto, xtrans             protocol and transport headers
#   xcb-proto                     the XCB protocol descriptions and the
#                                 Python xcbgen module libxcb's build runs
#                                 (its pkg-config file is sysroot-aware)
#   libXau, libXdmcp, libxcb      the XCB core
#   libX11, libXext, libXfixes, libXi, libXtst
#   xcb-util, xcb-util-keysyms
#
# Checksums: checksums/x11.sha256. Built and staged as lib/cross-env.sh
# describes. Prerequisites: build-deps.sh; a host Python 3.

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

build_libxi() {
    installed libXi.so libXi && return 0
    autotools_x11 libXi-1.8.3 lib --disable-specs --disable-docs
}

build_libxtst() {
    installed libXtst.so libXtst && return 0
    autotools_x11 libXtst-1.2.5 lib --disable-specs
}

build_xcb_util() {
    installed libxcb-util.so xcb-util && return 0
    autotools_x11 xcb-util-0.4.1 lib
}

build_xcb_util_keysyms() {
    installed libxcb-keysyms.so xcb-util-keysyms && return 0
    autotools_x11 xcb-util-keysyms-0.4.1 lib
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying X11 client libraries..."
    local errors=0 item
    for item in lib/libxcb.so lib/libX11.so lib/libX11-xcb.so lib/libXau.so lib/libXdmcp.so \
                lib/libXext.so lib/libXfixes.so lib/libXi.so lib/libXtst.so \
                lib/libxcb-util.so lib/libxcb-keysyms.so lib/libxcb-xkb.so lib/libxcb-xinput.so \
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
    build_libxi
    build_libxtst
    build_xcb_util
    build_xcb_util_keysyms
    verify
    log "=== X11 client libraries complete ==="
}

main "$@"

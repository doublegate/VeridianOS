#!/usr/bin/env bash
# Build D-Bus, GLib and the GLib-based libraries for VeridianOS
#
# D-Bus is required by KDE Plasma 6 for inter-process communication
# between KWin, plasmashell, and KDE services; its configuration is
# upstream's (/usr/share/dbus-1, with /etc/dbus-1 for local additions).
# at-spi2-core (Qt's accessibility bridge) needs D-Bus and GLib, and
# libsecret (KWallet) needs GLib, so all are built here, as are libnm,
# polkit, and libgudev with libwacom (plasma-desktop's tablet settings).
#
# Built and staged as lib/cross-env.sh describes.
# Prerequisites: build-deps.sh (expat, libmount, libffi, pcre2, libgcrypt,
# libuuid, libudev, libndp).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/dbus"
JOBS="${JOBS:-$(nproc)}"

# libsecret's and GLib's checksums as GNOME publishes them; the others
# pinned when first downloaded.
DBUS_VER="1.16.2"  # 1.16 is meson-only (autotools was removed)
DBUS_SHA256="0ba2a1a4b16afe7bceb2c07e9ce99a8c2c3508e5dec290dbb643384bd6beb7e2"
GLIB_VER="2.90.1"
GLIB_SHA256="93c941aa17d5eb1d53fe838365f29a8b4e539c222a256d974ec8f30fc413e396"
ATSPI_VER="2.62.0"
ATSPI_SHA256="03a94f7bf35f300daf2843a37cdf36479a91bc53f59a8ea437c79e25d95d1de3"
LIBSECRET_VER="0.21.8.2"
LIBSECRET_SHA256="142948339c5b971d8f6a8c7099521f6fd319b6fe73d2694b4e6d3310ed28b6e6"
# NetworkManager: libnm, which KF6 NetworkManagerQt (Plasma) links. The
# project's published checksum.
NM_VER="1.58.1"
NM_SHA256="262864cfd198123d3e5dbe91937441b97ee9059d6d712e1a5bfbb1c54668d14d"
# polkit: authorization for privileged actions (plasma-workspace links
# polkit-qt-1). The tag archive as first downloaded (no published checksum).
POLKIT_VER="127"
POLKIT_SHA256="9b7bc16f086479dcc626c575976568ba4a85d34297a750d8ab3d2e57f6d8b988"
# libgudev: GNOME's published checksum. libwacom: the release archive,
# verified against Peter Hutterer's signature
# (3C2C43D9447D5938EF4551EBE23B7E70B467F0BF).
LIBGUDEV_VER="238"
LIBGUDEV_SHA256="61266ab1afc9d73dbc60a8b2af73e99d2fdff47d99544d085760e4fa667b5dd1"
LIBWACOM_VER="2.20.0"
LIBWACOM_SHA256="370b45b5e05a91960df0aeb9c9481ae05846aab92ab2d4ec66945a0da4216888"

log() { echo "[build-dbus] $*"; }
die() { echo "[build-dbus] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

build_dbus() {
    installed libdbus-1.so D-Bus && return 0
    fetch "dbus-${DBUS_VER}.tar.xz" \
        "https://dbus.freedesktop.org/releases/dbus/dbus-${DBUS_VER}.tar.xz" \
        "dbus-${DBUS_VER}" "${DBUS_SHA256}"
    log "Building D-Bus ${DBUS_VER}..."
    meson_build "${BUILD_DIR}/dbus-${DBUS_VER}" "${BUILD_DIR}/dbus-build" \
        -Dsystemd=disabled \
        -Dlaunchd=disabled \
        -Dselinux=disabled \
        -Dapparmor=disabled \
        -Dlibaudit=disabled \
        -Dkqueue=disabled \
        -Dxml_docs=disabled \
        -Ddoxygen_docs=disabled \
        -Dducktype_docs=disabled \
        -Dqt_help=disabled \
        -Dmodular_tests=disabled \
        -Dinstalled_tests=false \
        -Dx11_autolaunch=disabled \
        -Dsystem_socket=/run/dbus/system_bus_socket \
        -Druntime_dir=/run
}

# ── Fix dbus-1.pc for CMake consumers ─────────────────────────────────
# D-Bus's DBus1Config.cmake turns every non-include pkg-config C flag into
# an INTERFACE_COMPILE_DEFINITION, and the meson build (1.16) puts -pthread
# in Cflags, so Qt's moc got "-D-pthread" ("macro names must be
# identifiers"). -pthread stays in Libs. Idempotent, so it also repairs
# an existing install.
fix_dbus_pc() {
    local pc="${SYSROOT}/usr/lib/pkgconfig/dbus-1.pc"
    [[ -f "${pc}" ]] || die "${pc} not installed"
    local tmp="${pc}.tmp"
    sed -E '/^Cflags:/ s/[[:space:]]-pthread\b//g' "${pc}" > "${tmp}"
    [[ -s "${tmp}" ]] || die "rewriting ${pc} produced an empty file"
    mv "${tmp}" "${pc}"
    if grep -q '^Cflags:.*-pthread' "${pc}"; then
        die "could not remove -pthread from ${pc} Cflags"
    fi
}

# ── GLib ──────────────────────────────────────────────────────────────
fetch_glib() {
    fetch "glib-${GLIB_VER}.tar.xz" \
        "https://download.gnome.org/sources/glib/${GLIB_VER%.*}/glib-${GLIB_VER}.tar.xz" \
        "glib-${GLIB_VER}" "${GLIB_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/glib_vasnprintf_sign_compare.py" \
        "${BUILD_DIR}/glib-${GLIB_VER}/glib/gnulib/vasnprintf.c" || die "failed to patch GLib vasnprintf.c"
}

# The code generators (gdbus-codegen, glib-mkenums, glib-genmarshal,
# glib-compile-resources) the builds of libsecret and at-spi2-core run, at
# the version of the target GLib: a native GLib in the host tools, which
# native meson dependencies find first.
build_glib_host() {
    if [[ "$(env -u PKG_CONFIG_LIBDIR -u PKG_CONFIG_SYSROOT_DIR PKG_CONFIG_PATH="${PKG_CONFIG_PATH_FOR_BUILD}" \
            pkg-config --modversion gio-2.0 2>/dev/null)" == "${GLIB_VER}" && \
          -x "${VERIDIAN_HOST_TOOLS}/bin/gdbus-codegen" ]]; then
        log "GLib host tools: already installed."
        return 0
    fi
    fetch_glib
    log "Building GLib ${GLIB_VER} host tools..."
    local bld="${BUILD_DIR}/glib-host-build"
    rm -rf "${bld}"
    (unset CC CXX AR RANLIB NM STRIP CFLAGS CXXFLAGS PKG_CONFIG_LIBDIR PKG_CONFIG_SYSROOT_DIR && \
        meson setup "${bld}" "${BUILD_DIR}/glib-${GLIB_VER}" \
            --prefix="${VERIDIAN_HOST_TOOLS}" \
            --libdir=lib \
            --buildtype=release \
            -Dintrospection=disabled \
            -Dnls=disabled \
            -Dman-pages=disabled \
            -Ddocumentation=false \
            -Dtests=false \
            -Dinstalled_tests=false \
            -Dsysprof=disabled && \
        ninja -C "${bld}" -j"${JOBS}" && \
        ninja -C "${bld}" install)
}

# With libmount; no tests, docs or introspection.
build_glib() {
    installed libglib-2.0.so GLib && return 0
    fetch_glib
    log "Building GLib ${GLIB_VER}..."
    meson_build "${BUILD_DIR}/glib-${GLIB_VER}" "${BUILD_DIR}/glib-build" \
        -Dlibmount=enabled \
        -Dselinux=disabled \
        -Dlibelf=disabled \
        -Dsysprof=disabled \
        -Dintrospection=disabled \
        -Dman-pages=disabled \
        -Ddocumentation=false \
        -Dtests=false \
        -Dinstalled_tests=false \
        -Dbsymbolic_functions=false \
        -Dglib_debug=disabled
}

# ── libsecret (Secret Service client library) ──────────────────────
# Transport encryption through libgcrypt (build-deps.sh). Off: man pages,
# reference docs, Vala and GObject-introspection bindings (host tools,
# not run-time features).
build_libsecret() {
    installed libsecret-1.so libsecret && return 0
    local series="${LIBSECRET_VER%.*}"
    fetch "libsecret-${LIBSECRET_VER}.tar.xz" \
        "https://download.gnome.org/sources/libsecret/${series%.*}/libsecret-${LIBSECRET_VER}.tar.xz" \
        "libsecret-${LIBSECRET_VER}" "${LIBSECRET_SHA256}"
    local src="${BUILD_DIR}/libsecret-${LIBSECRET_VER}"
    python3 -I "${SCRIPT_DIR}/deps-patches/libsecret_fcntl_include.py" \
        "${src}/libsecret/secret-file-collection.c" || die "failed to patch libsecret"
    python3 -I "${SCRIPT_DIR}/deps-patches/libsecret_gcrypt_log_handler.py" \
        "${src}/egg/egg-libgcrypt.c" || die "failed to patch libsecret"
    log "Building libsecret ${LIBSECRET_VER}..."
    meson_build "${src}" "${BUILD_DIR}/libsecret-build" \
        -Dcrypto=libgcrypt \
        -Dmanpage=false \
        -Dgtk_doc=false \
        -Dvapi=false \
        -Dintrospection=false \
        -Dbash_completion=disabled \
        -Dtest_setup=disabled
}

# ── at-spi2-core (accessibility: libatspi, atk, atk-bridge) ───────────
build_atspi() {
    installed libatspi.so at-spi2-core && return 0
    fetch "at-spi2-core-${ATSPI_VER}.tar.xz" \
        "https://download.gnome.org/sources/at-spi2-core/${ATSPI_VER%.*}/at-spi2-core-${ATSPI_VER}.tar.xz" \
        "at-spi2-core-${ATSPI_VER}" "${ATSPI_SHA256}"
    local src="${BUILD_DIR}/at-spi2-core-${ATSPI_VER}"
    python3 -I "${SCRIPT_DIR}/deps-patches/atspi_device_legacy_x11_priv.py" \
        "${src}/atspi/atspi-device-legacy.c" || die "failed to patch at-spi2-core"
    log "Building at-spi2-core ${ATSPI_VER}..."
    meson_build "${src}" "${BUILD_DIR}/atspi-build" \
        -Dintrospection=disabled \
        -Dx11=disabled \
        -Duse_systemd=false \
        -Ddefault_bus=dbus-daemon \
        -Dgtk2_atk_adaptor=false \
        -Ddocs=false
}

# ── NetworkManager (libnm) ────────────────────────────────────────────
# Built whole (meson has no libnm-only build), with the daemon's
# optional integrations off: no Wi-Fi, PPP, ModemManager, polkit, systemd
# or tests. D-Bus policy and udev paths are given explicitly, as meson
# would otherwise derive them from the sysroot's pkg-config files.
build_networkmanager() {
    installed libnm.so NetworkManager && return 0
    fetch "NetworkManager-${NM_VER}.tar.xz" \
        "https://gitlab.freedesktop.org/api/v4/projects/411/packages/generic/NetworkManager/${NM_VER}/NetworkManager-${NM_VER}.tar.xz" \
        "NetworkManager-${NM_VER}" "${NM_SHA256}"
    local src="${BUILD_DIR}/NetworkManager-${NM_VER}"
    # Meson lint in its build files (it requires meson 0.56).
    if ! python3 -I "${SCRIPT_DIR}/deps-patches/meson_lint.py" version-checks "${src}" 0.56.0 ||
       ! python3 -I "${SCRIPT_DIR}/deps-patches/meson_lint.py" copy-config \
            "${src}/src/nmcli/meson.build" "${src}/src/libnmc-setting/meson.build" \
            "${src}/src/libnm-core-impl/meson.build"; then
        die "failed to patch NetworkManager's meson files"
    fi
    log "Building NetworkManager ${NM_VER} (libnm)..."
    meson_build "${src}" "${BUILD_DIR}/networkmanager-build" \
        -Dsystemdsystemunitdir=no \
        -Dsystemdsystemgeneratordir=no \
        -Dsystemd_journal=false \
        -Dsession_tracking=no \
        -Dsession_tracking_consolekit=false \
        -Dsuspend_resume=consolekit \
        -Dpolkit=false \
        -Dselinux=false \
        -Dlibaudit=no \
        -Dwifi=false \
        -Dppp=false \
        -Dmodem_manager=false \
        -Dovs=false \
        -Dnmcli=false \
        -Dnmtui=false \
        -Dnm_cloud_setup=false \
        -Dnbft=false \
        -Dclat=false \
        -Dconcheck=false \
        -Dintrospection=false \
        -Dvapi=false \
        -Ddocs=false \
        -Dman=false \
        -Dtests=no \
        -Dcrypto=null \
        -Dqt=false \
        -Dlibpsl=false \
        -Debpf=false \
        -Dfirewalld_zone=false \
        -Dreadline=none \
        -Dudev_dir=no \
        -Ddbus_conf_dir=/usr/share/dbus-1/system.d
}

# ── polkit ────────────────────────────────────────────────────────────
# The libraries, polkitd and pkexec, with rules in JavaScript run by duktape
# and authentication through Linux-PAM (build-deps.sh). Session tracking
# must name a tracker; ConsoleKit is the one that needs no library at build
# time (polkitd asks it over D-Bus). VeridianOS runs none yet, so rules
# that depend on an active local session do not match
# (userland/integration/KNOWN-LIMITATIONS.md). pkexec and
# polkit-agent-helper-1 become setuid root in the root filesystem.
build_polkit() {
    installed libpolkit-gobject-1.so polkit && return 0
    fetch "polkit-${POLKIT_VER}.tar.gz" \
        "https://github.com/polkit-org/polkit/archive/refs/tags/${POLKIT_VER}.tar.gz" \
        "polkit-${POLKIT_VER}" "${POLKIT_SHA256}"
    log "Building polkit ${POLKIT_VER}..."
    meson_build "${BUILD_DIR}/polkit-${POLKIT_VER}" "${BUILD_DIR}/polkit-build" \
        -Dsession_tracking=ConsoleKit \
        -Dauthfw=pam \
        -Dos_type=lfs \
        -Dpolkitd_user=polkitd \
        -Dsystemdsystemunitdir=/usr/lib/systemd/system \
        -Dgettext=true \
        -Dintrospection=false \
        -Dman=false \
        -Dgtk_doc=false \
        -Dexamples=false \
        -Dtests=false
}

# ── libgudev, libwacom ────────────────────────────────────────────────
# The tablet database plasma-desktop's tablet settings read. libgudev wraps
# libudev (libudev-zero, build-deps.sh) for GLib; libwacom's udev rules and
# hwdb go where udev would read them, generated by the host's Python.
build_libgudev() {
    installed libgudev-1.0.so libgudev && return 0
    fetch "libgudev-${LIBGUDEV_VER}.tar.xz" \
        "https://download.gnome.org/sources/libgudev/${LIBGUDEV_VER}/libgudev-${LIBGUDEV_VER}.tar.xz" \
        "libgudev-${LIBGUDEV_VER}" "${LIBGUDEV_SHA256}"
    log "Building libgudev ${LIBGUDEV_VER}..."
    meson_build "${BUILD_DIR}/libgudev-${LIBGUDEV_VER}" "${BUILD_DIR}/libgudev-build" \
        -Dtests=disabled \
        -Dintrospection=disabled \
        -Dvapi=disabled \
        -Dgtk_doc=false
}

build_libwacom() {
    installed libwacom.so libwacom && return 0
    fetch "libwacom-${LIBWACOM_VER}.tar.xz" \
        "https://github.com/linuxwacom/libwacom/releases/download/libwacom-${LIBWACOM_VER}/libwacom-${LIBWACOM_VER}.tar.xz" \
        "libwacom-${LIBWACOM_VER}" "${LIBWACOM_SHA256}"
    log "Building libwacom ${LIBWACOM_VER}..."
    meson_build "${BUILD_DIR}/libwacom-${LIBWACOM_VER}" "${BUILD_DIR}/libwacom-build" \
        -Dtests=disabled \
        -Ddocumentation=disabled \
        -Dudev-dir=/usr/lib/udev
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying D-Bus installation..."
    local errors=0 item
    # The KDE session cannot start without the bus daemon.
    for item in usr/lib/libdbus-1.so usr/include/dbus-1.0/dbus/dbus.h usr/bin/dbus-daemon \
                usr/share/dbus-1/session.conf usr/share/dbus-1/system.conf \
                usr/lib/libglib-2.0.so usr/lib/libgio-2.0.so usr/lib/libsecret-1.so usr/lib/libatspi.so \
                usr/lib/libnm.so usr/lib/pkgconfig/libnm.pc \
                usr/lib/libpolkit-gobject-1.so usr/lib/libpolkit-agent-1.so usr/lib/polkit-1/polkitd \
                usr/lib/libgudev-1.0.so usr/lib/libwacom.so usr/share/libwacom; do
        if [[ -e "${SYSROOT}/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "D-Bus ready."
}

main() {
    log "=== Building D-Bus for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    [[ -f "${SYSROOT}/usr/lib/libexpat.so" ]] || die "expat not found. Run build-deps.sh first."

    build_dbus
    fix_dbus_pc
    build_glib_host
    build_glib
    build_libsecret
    build_atspi
    build_networkmanager
    build_polkit
    build_libgudev
    build_libwacom
    verify
    log "=== D-Bus build complete ==="
}

main "$@"

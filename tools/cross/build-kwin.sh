#!/usr/bin/env bash
# Build KWin, the KDE Plasma 6 Wayland compositor, for VeridianOS
#
#   1. The Plasma libraries KWin requires: KWayland, PlasmaActivities,
#      KNightTime, and QAccessibilityClient (the zoom effect follows the
#      focus with it)
#   2. kdecoration (window decoration API)
#   3. kglobalacceld (global shortcuts; KWin hosts the service)
#   4. qtwaylandscanner_kde for the host (KWin's protocol code generator)
#   5. KWin: kwin_wayland, its libraries and its plugins
#
# Shared (ADR 0010), for /usr, staged into the sysroot (lib/cross-env.sh); sources
# checked against KDE's checksums (checksums/plasma.sha256; for
# QAccessibilityClient, from its release signature by Carl Schwan,
# 39FFA93CAE9C6AFC212AD00202325448204E452A).
# X11/Xwayland support is not built (deferred; docs/KNOWN-LIMITATIONS.md).
# The screen locker is built once Linux-PAM and pam_veridian are in the
# pipeline (KWIN_BUILD_SCREENLOCKER, below).
#
# Prerequisites: build-kf6.sh (KF6, Qt, Mesa, Wayland, libinput, ...).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/kwin"
JOBS="${JOBS:-$(nproc)}"

# KWin and its libraries ship with Plasma: the release is build-plasma.sh's.
PLASMA_VER="$(sed -n 's/^PLASMA_VER="\(.*\)"$/\1/p' "${SCRIPT_DIR}/build-plasma.sh")"
[[ -n "${PLASMA_VER}" ]] || { echo "[build-kwin] cannot read PLASMA_VER from build-plasma.sh" >&2; exit 1; }
PLASMA_URL_BASE="https://download.kde.org/stable/plasma/${PLASMA_VER}"

log() { echo "[build-kwin] $*"; }
die() { echo "[build-kwin] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"
# shellcheck source=lib/kde-build.sh
source "${SCRIPT_DIR}/lib/kde-build.sh"

QACCESSIBILITYCLIENT_VER="0.6.0"

plasma_src() {
    kde_fetch "$1-${PLASMA_VER}" "${PLASMA_URL_BASE}" plasma.sha256
}

# config_installed CONFIG: the package installing CMake config CONFIG is
# in the sysroot.
config_installed() {
    [[ -f "${SYSROOT}/usr/lib/cmake/$1/$1Config.cmake" ]] && log "$1: already installed."
}

# check_config CONFIG: the package just built installed its CMake config.
check_config() {
    [[ -f "${SYSROOT}/usr/lib/cmake/$1/$1Config.cmake" ]] || die "$1: CMake config not installed"
}

# ── 1. Plasma libraries ──────────────────────────────────────────────
build_kwayland() {
    config_installed KWayland && return 0
    local src
    src="$(plasma_src kwayland)"
    kde_cmake kwayland "${src}"
    check_config KWayland
}

build_plasma_activities() {
    config_installed PlasmaActivities && return 0
    local src
    src="$(plasma_src plasma-activities)"
    kde_cmake plasma-activities "${src}"
    check_config PlasmaActivities
}

build_knighttime() {
    config_installed KNightTime && return 0
    local src
    src="$(plasma_src knighttime)"
    kde_cmake knighttime "${src}"
    check_config KNightTime
}

build_qaccessibilityclient() {
    config_installed QAccessibilityClient6 && return 0
    local src
    src="$(kde_fetch "libqaccessibilityclient-${QACCESSIBILITYCLIENT_VER}" \
        "https://download.kde.org/stable/libqaccessibilityclient" plasma.sha256)"
    kde_cmake libqaccessibilityclient "${src}" -DQT_MAJOR_VERSION=6
    check_config QAccessibilityClient6
}

# ── 2. kdecoration ───────────────────────────────────────────────────
build_kdecoration() {
    config_installed KDecoration3 && return 0
    local src
    src="$(plasma_src kdecoration)"
    kde_cmake kdecoration "${src}"
    check_config KDecoration3
}

# ── 3. kglobalacceld ─────────────────────────────────────────────────
build_kglobalacceld() {
    config_installed KGlobalAccelD && return 0
    local src
    src="$(plasma_src kglobalacceld)"
    kde_cmake kglobalacceld "${src}"
    check_config KGlobalAccelD
}

# ── 4. qtwaylandscanner_kde (host) ───────────────────────────────────
# KWin's cross build takes it as QTWAYLANDSCANNER_KDE_EXECUTABLE; its
# source is a standalone CMake project, built against the host Qt.
build_host_scanner() {
    local tool="${VERIDIAN_HOST_TOOLS}/bin/qtwaylandscanner_kde"
    if [[ -x "${tool}" && -f "${VERIDIAN_HOST_TOOLS}/share/qtwaylandscanner_kde.version" &&
          "$(cat "${VERIDIAN_HOST_TOOLS}/share/qtwaylandscanner_kde.version")" == "${PLASMA_VER}" ]]; then
        log "qtwaylandscanner_kde (host): already built."
        return 0
    fi
    local src
    src="$(plasma_src kwin)"
    local bld="${BUILD_DIR}/qtwaylandscanner_kde-host"
    log "Building qtwaylandscanner_kde (host)..."
    rm -rf "${bld}"
    env -u CC -u CXX -u AR -u RANLIB -u NM -u STRIP -u CFLAGS -u CXXFLAGS \
        -u PKG_CONFIG -u PKG_CONFIG_LIBDIR -u PKG_CONFIG_SYSROOT_DIR \
        cmake -S "${src}/src/wayland/tools" -B "${bld}" \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_PREFIX_PATH="${VERIDIAN_HOST_TOOLS}/qt6" \
            -DECM_DIR="${SYSROOT}/usr/share/ECM/cmake"
    env -u CC -u CXX cmake --build "${bld}" --parallel "${JOBS}"
    install -Dm755 "${bld}/qtwaylandscanner_kde" "${tool}"
    echo "${PLASMA_VER}" > "${VERIDIAN_HOST_TOOLS}/share/qtwaylandscanner_kde.version"
}

# ── 5. KWin ──────────────────────────────────────────────────────────
build_kwin() {
    if [[ -f "${SYSROOT}/usr/bin/kwin_wayland" ]]; then
        log "KWin: already installed."
        return 0
    fi
    local src
    src="$(plasma_src kwin)"
    # Upstream fixes not yet in a 6.7 release (kwin-patches/, each the
    # upstream commit as `git format-patch` wrote it, one rebased): the
    # dialog and process-killer helpers and two headers (screenedge.h,
    # shadow.h) needed X11 even with KWIN_BUILD_X11=OFF (b0d9e808,
    # edc72540, 712c069d, e6e5f947; KDE bug 526689).
    apply_patches "${SCRIPT_DIR}/kwin-patches" "${src}"
    # Without logind, kwin opens /dev/dri/card0 through
    # NoopSession::openRestricted(). Older releases returned -1 there; since
    # 6.4 upstream opens the device itself. Check that it still does.
    grep -q '::open(fileName' "${src}/src/core/session_noop.cpp" ||
        die "session_noop.cpp: NoopSession::openRestricted() no longer opens the device"
    kde_cmake kwin "${src}" \
        -DKWIN_BUILD_X11=OFF \
        -DKWIN_BUILD_SCREENLOCKER=OFF \
        -DKWIN_BUILD_TABBOX=ON \
        -DKWIN_BUILD_KCMS=ON \
        -DKWIN_BUILD_GLOBALSHORTCUTS=ON \
        -DQTWAYLANDSCANNER_KDE_EXECUTABLE="${VERIDIAN_HOST_TOOLS}/bin/qtwaylandscanner_kde"
}

verify() {
    log "Verifying KWin installation..."
    local errors=0 item
    for item in usr/bin/kwin_wayland usr/lib/cmake/KDecoration3/KDecoration3Config.cmake \
                usr/lib/cmake/KGlobalAccelD/KGlobalAccelDConfig.cmake \
                usr/lib/cmake/KWayland/KWaylandConfig.cmake \
                usr/lib/cmake/PlasmaActivities/PlasmaActivitiesConfig.cmake \
                usr/lib/cmake/KNightTime/KNightTimeConfig.cmake \
                usr/lib/cmake/QAccessibilityClient6/QAccessibilityClient6Config.cmake; do
        if [[ -e "${SYSROOT}/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing"
}

main() {
    log "=== Building KWin ${PLASMA_VER} for VeridianOS ==="
    [[ -f "${SYSROOT}/usr/lib/libinput.so" ]] || die "libinput not found. Run build-deps.sh first."
    [[ -f "${SYSROOT}/usr/lib/cmake/KF6Config/KF6ConfigConfig.cmake" ]] || die "KF6 not found. Run build-kf6.sh first."
    build_kwayland
    build_plasma_activities
    build_knighttime
    build_qaccessibilityclient
    build_kdecoration
    build_kglobalacceld
    build_host_scanner
    build_kwin
    verify
    log "=== KWin build complete ==="
}

main "$@"

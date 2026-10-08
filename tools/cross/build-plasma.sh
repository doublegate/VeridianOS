#!/usr/bin/env bash
# Build the Plasma desktop for VeridianOS
#
# The Plasma 6 components beyond KWin (build-kwin.sh builds KWin and the
# libraries it needs), in dependency order:
#
#   breeze                   the Qt style, KWin decoration, cursors, colours
#   plasma-activities-stats, kactivitymanagerd
#   libplasma, plasma5support, layer-shell-qt, libkscreen, libksysguard
#   polkit-qt-1              Qt bindings of polkit (build-dbus.sh)
#   kscreenlocker            the screen locker (Linux-PAM, build-deps.sh)
#   milou                    KRunner's result list
#   plasma-workspace         plasmashell, the session, libkworkspace
#   powerdevil, plasma-integration, qqc2-breeze-style
#   plasma-desktop           the desktop containment, panels, KCMs
#   ocean-sound-theme, plasma-workspace-wallpapers
#
# Shared (ADR 0010), for /usr, staged into the sysroot (lib/cross-env.sh);
# sources checked against KDE's checksums (checksums/plasma.sha256).
# X11 client support is built, as KDE expects it, but no X11 session or X
# server parts (XWayland is deferred: docs/KNOWN-LIMITATIONS.md); glibc's
# locale machinery does not apply to musl. kpipewire (screen
# casting) waits for PipeWire and FFmpeg (to-dos/MASTER_TODO.md).
#
# Prerequisites: build-kwin.sh (and so KF6, Qt, Mesa, ...).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/plasma"
JOBS="${JOBS:-$(nproc)}"

PLASMA_VER="6.7.5"
PLASMA_URL_BASE="https://download.kde.org/stable/plasma/${PLASMA_VER}"
POLKIT_QT_VER="0.201.1"

log() { echo "[build-plasma] $*"; }
die() { echo "[build-plasma] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"
# shellcheck source=lib/kde-build.sh
source "${SCRIPT_DIR}/lib/kde-build.sh"

# component_installed NAME STAMP: NAME was built as STAMP (version and
# options) and every file
# its install wrote (CMake's install manifest, which lists them without the
# DESTDIR) is still in the sysroot.
component_installed() {
    local bld="${BUILD_DIR}/$1-build" file
    [[ "$(cat "${bld}/.veridian-version" 2>/dev/null)" == "$2" && -s "${bld}/install_manifest.txt" ]] || return 1
    while IFS= read -r file; do
        [[ -e "${SYSROOT}${file}" || -L "${SYSROOT}${file}" ]] || return 1
    done < "${bld}/install_manifest.txt"
    log "$1: already installed."
}

# plasma NAME [CMAKE OPTIONS...]: build and stage one Plasma component,
# with its patches (plasma-patches/NAME/, each explaining itself) applied to
# the fresh source tree.
plasma() {
    local name="$1"
    shift
    local stamp="${PLASMA_VER} $*"
    component_installed "${name}" "${stamp}" && return 0
    local src
    src="$(kde_fetch "${name}-${PLASMA_VER}" "${PLASMA_URL_BASE}" plasma.sha256)"
    apply_patches "${SCRIPT_DIR}/plasma-patches/${name}" "${src}"
    kde_cmake "${name}" "${src}" "$@"
    echo "${stamp}" > "${BUILD_DIR}/${name}-build/.veridian-version"
}

build_polkit_qt() {
    component_installed polkit-qt-1 "${POLKIT_QT_VER}" && return 0
    local src
    src="$(kde_fetch "polkit-qt-1-${POLKIT_QT_VER}" "https://download.kde.org/stable/polkit-qt-1" plasma.sha256)"
    kde_cmake polkit-qt-1 "${src}" -DQT_MAJOR_VERSION=6 -DBUILD_EXAMPLES=OFF
    echo "${POLKIT_QT_VER}" > "${BUILD_DIR}/polkit-qt-1-build/.veridian-version"
}

# The VeridianOS session scripts the kernel starts the desktop with
# (kernel/src/desktop/kde_session.rs: /usr/share/veridian/veridian-kde-init.sh).
install_veridian_session() {
    local script
    for script in "${PROJECT_ROOT}"/userland/integration/*.sh; do
        install -Dm755 "${script}" "${SYSROOT}/usr/share/veridian/$(basename "${script}")"
    done
}

verify() {
    log "Verifying Plasma installation..."
    local errors=0 item
    for item in usr/bin/plasmashell usr/bin/startplasma-wayland usr/bin/ksmserver \
                usr/lib/libPlasma.so usr/lib/libPlasmaQuick.so usr/lib/libkworkspace6.so \
                usr/lib/libKScreenLocker.so usr/lib/libLayerShellQtInterface.so \
                usr/lib/libKF6Screen.so usr/lib/libKSysGuardSystemStats.so usr/lib/libprocesscore.so usr/lib/libpolkit-qt6-core-1.so \
                usr/lib/libPlasma5Support.so usr/lib/libPlasmaActivities.so \
                usr/lib/qt6/plugins/styles/breeze6.so \
                usr/share/plasma/shells/org.kde.plasma.desktop \
                usr/share/plasma/look-and-feel/org.kde.breeze.desktop \
                usr/share/veridian/veridian-kde-init.sh; do
        if [[ -e "${SYSROOT}/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "Plasma ready."
}

main() {
    log "=== Building Plasma ${PLASMA_VER} for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    [[ -f "${SYSROOT}/usr/bin/kwin_wayland" ]] || die "KWin not found. Run build-kwin.sh first."

    plasma breeze -DBUILD_QT5=OFF -DBUILD_QT6=ON
    plasma plasma-activities-stats
    plasma kactivitymanagerd
    plasma libplasma
    plasma plasma5support
    plasma layer-shell-qt
    plasma libkscreen
    plasma libksysguard
    build_polkit_qt
    plasma kscreenlocker
    plasma milou
    # No X11 session: it needs an X server. X11 support itself is on
    # (lib/kde-build.sh turns it off by default): ksmserver, which the
    # Wayland session runs for logout and shutdown and whose D-Bus
    # interface plasma-desktop requires, is built only with it.
    plasma plasma-workspace \
        -DWITH_X11=ON \
        -DWITH_X11_SESSION=OFF \
        -DGLIBC_LOCALE=OFF \
        -DGLIBC_LOCALE_GEN=OFF
    plasma powerdevil
    plasma plasma-integration -DBUILD_QT5=OFF -DBUILD_QT6=ON
    plasma qqc2-breeze-style
    # The mouse and touchpad KCMs' X11 backends configure the X server's
    # input drivers (xorg-libinput, xorg-server): there is no X server.
    plasma plasma-desktop \
        -DBUILD_KCM_MOUSE_X11=OFF \
        -DBUILD_KCM_TOUCHPAD_X11=OFF
    plasma ocean-sound-theme
    plasma plasma-workspace-wallpapers
    install_veridian_session
    verify
    log "=== Plasma build complete ==="
}

main "$@"

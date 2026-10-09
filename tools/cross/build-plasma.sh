#!/usr/bin/env bash
# Build the Plasma desktop for VeridianOS
#
# The Plasma 6 components beyond KWin (build-kwin.sh builds KWin and the
# libraries it needs), in dependency order:
#
#   breeze                   the Qt style, KWin decoration, cursors, colours
#   plasma-activities-stats, kactivitymanagerd
#   libplasma, plasma5support, layer-shell-qt, libkscreen, libksysguard
#   kscreenlocker            the screen locker (Linux-PAM, build-deps.sh)
#   milou                    KRunner's result list
#   plasma-workspace         plasmashell, the session, libkworkspace
#   powerdevil, plasma-integration, qqc2-breeze-style
#   plasma-desktop           the desktop containment, panels, KCMs
#   polkit-kde-agent-1       asks for the password polkit needs
#   systemsettings, kde-cli-tools, kscreen, kmenuedit, kinfocenter
#   kwrited, ksystemstats, plasma-systemmonitor, kdeplasma-addons
#   kwallet-pam, ksshaskpass, aurorae
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
# Corrosion (CMake's Rust integration) for the build host: kdeplasma-addons
# builds a Rust helper. A tag archive; its checksum pinned when first
# downloaded (tag v0.6.1 is commit 1499b14e).
CORROSION_VER="0.6.1"
# The Rust toolchain for it, pinned (the repository's rust-toolchain.toml
# pins the kernel's nightly, which would otherwise apply).
RUST_VER="1.99.0"

log() { echo "[build-plasma] $*"; }
die() { echo "[build-plasma] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"
# shellcheck source=lib/kde-build.sh
source "${SCRIPT_DIR}/lib/kde-build.sh"

build_host_corrosion() {
    local prefix="${VERIDIAN_HOST_TOOLS}/corrosion"
    if [[ -f "${prefix}/lib/cmake/Corrosion/CorrosionConfig.cmake" &&
          "$(cat "${prefix}/.veridian-version" 2>/dev/null)" == "${CORROSION_VER}" ]]; then
        log "Corrosion: already installed."
        return 0
    fi
    fetch "corrosion-${CORROSION_VER}.tar.gz" \
        "https://github.com/corrosion-rs/corrosion/archive/refs/tags/v${CORROSION_VER}.tar.gz" \
        "corrosion-${CORROSION_VER}" "$(listed_sha256 plasma.sha256 "corrosion-${CORROSION_VER}.tar.gz")"
    log "Building Corrosion ${CORROSION_VER} (host)..."
    local bld="${BUILD_DIR}/corrosion-host-build"
    if ! host_env cmake -S "${BUILD_DIR}/corrosion-${CORROSION_VER}" -B "${bld}" \
            -DCMAKE_INSTALL_PREFIX="${prefix}" -DCORROSION_BUILD_TESTS=OFF ||
       ! host_env cmake --build "${bld}" --parallel "${JOBS}" ||
       ! host_env cmake --install "${bld}"; then
        die "Corrosion did not build"
    fi
    echo "${CORROSION_VER}" > "${prefix}/.veridian-version"
}

# kdeplasma-addons, whose QMK keyboard helper (kameleon-qmk-helper) is Rust:
# built for x86_64-unknown-linux-musl (VeridianOS runs Linux programs) and
# linked by the cross compiler against the sysroot's shared C library, like
# every other program (ADR 0010), not statically with Rust's own musl. Its
# crates come from crates.io as its Cargo.lock pins them (checksummed);
# rustup adds the musl standard library to the pinned toolchain if missing.
build_kdeplasma_addons() {
    build_host_corrosion
    (
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${VERIDIAN_CC}"
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=-crt-static"
        export RUSTUP_TOOLCHAIN="${RUST_VER}"
        plasma kdeplasma-addons \
            -DCorrosion_DIR="${VERIDIAN_HOST_TOOLS}/corrosion/lib/cmake/Corrosion" \
            -DRust_CARGO_TARGET=x86_64-unknown-linux-musl \
            -DRust_RUSTUP_INSTALL_MISSING_TARGET=ON
    )
}

# The VeridianOS scripts that run on the system: the session script the
# kernel starts the desktop with (kernel/src/desktop/kde_session.rs:
# /usr/share/veridian/veridian-kde-init.sh) and the session profiler. The
# other scripts in userland/integration run on the build host (rootfs and
# XWayland builds, QEMU launcher, the QMP-driven test suite).
install_veridian_session() {
    local script
    for script in veridian-kde-init.sh kde-perf-profile.sh; do
        install -Dm755 "${PROJECT_ROOT}/userland/integration/${script}" \
            "${SYSROOT}/usr/share/veridian/${script}"
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
                usr/bin/systemsettings usr/lib/libexec/polkit-kde-authentication-agent-1 \
                usr/bin/kioclient usr/bin/kmenuedit usr/bin/kinfocenter usr/bin/kwrited \
                usr/bin/ksystemstats usr/bin/plasma-systemmonitor usr/bin/ksshaskpass \
                usr/lib/qt6/plugins/kf6/kded/kscreen.so usr/lib/security/pam_kwallet5.so \
                usr/lib/libexec/kf6/kameleon-qmk-helper usr/lib/libexec/plasma-apply-aurorae \
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
    # polkit-qt-1 comes from build-kf6.sh, before KAuth.
    plasma kscreenlocker
    plasma milou
    # No X11 session: it needs an X server. X11 support itself is on
    # (lib/kde-build.sh turns it off by default): ksmserver, which the
    # Wayland session runs for logout and shutdown and whose D-Bus
    # interface plasma-desktop requires, is built only with it.
    # Optional parts declined: usage telemetry (UserFeedback), file search
    # (Baloo, not built), installing software from KRunner and the
    # launcher (PackageKit, AppStream: there is no package manager behind
    # them) and wallpaper orientation from EXIF data (KExiv2).
    plasma plasma-workspace \
        -DWITH_X11=ON \
        -DWITH_X11_SESSION=OFF \
        -DGLIBC_LOCALE=OFF \
        -DGLIBC_LOCALE_GEN=OFF \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6UserFeedback=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6Baloo=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_PackageKitQt6=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_AppStreamQt=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KExiv2Qt6=ON
    plasma powerdevil
    # The fonts and the desktop portal are run-time requirements (no
    # package provides a CMake configuration for them on any system); the
    # rootfs carries its fonts (build-fonts.sh) and has no portal.
    plasma plasma-integration -DBUILD_QT5=OFF -DBUILD_QT6=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_FontNotoSans=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_FontNotoColorEmoji=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_FontHack=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_XDGDesktopPortalKDE=ON
    plasma qqc2-breeze-style
    # The mouse and touchpad KCMs' X11 backends configure the X server's
    # input drivers (xorg-libinput, xorg-server): there is no X server.
    # Declined: online accounts (KAccounts), the game controller KCM
    # (SDL2), file search (Baloo), software installation (PackageKit), the
    # emoji font check (run time) and screen casting's QML module (kpipewire
    # waits for PipeWire, above).
    plasma plasma-desktop \
        -DBUILD_KCM_MOUSE_X11=OFF \
        -DBUILD_KCM_TOUCHPAD_X11=OFF \
        -DCMAKE_DISABLE_FIND_PACKAGE_KAccounts6=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_SDL2=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_KF6Baloo=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_PackageKitQt6=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_FontNotoColorEmoji=ON \
        -DCMAKE_DISABLE_FIND_PACKAGE_org.kde.pipewire-QMLModule=ON
    plasma polkit-kde-agent-1
    plasma systemsettings
    plasma kde-cli-tools
    plasma kscreen
    plasma kmenuedit
    # Its PipeWire page needs PipeWire (not built yet).
    plasma kinfocenter -DCMAKE_DISABLE_FIND_PACKAGE_pipewire=ON
    plasma kwrited
    plasma ksystemstats
    plasma plasma-systemmonitor
    build_kdeplasma_addons
    plasma kwallet-pam
    plasma ksshaskpass
    plasma aurorae
    plasma ocean-sound-theme
    plasma plasma-workspace-wallpapers
    install_veridian_session
    verify
    log "=== Plasma build complete ==="
}

main "$@"

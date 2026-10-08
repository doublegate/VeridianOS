#!/usr/bin/env bash
# Build KDE Frameworks 6 (the subset KWin and Plasma need) for VeridianOS
#
# Build order follows the KF6 dependency chain. Shared (ADR 0010), for /usr, staged
# into the sysroot (lib/cross-env.sh); Qt's directory layout for plugins
# and QML (lib/kde-build.sh). Tarball checksums: KDE's
# published ones (checksums/kf6.sha256).
#
# Prerequisites: build-qt6.sh (Qt 6 in the sysroot, host Qt).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/kf6"
JOBS="${JOBS:-$(nproc)}"

KF_VER="6.30.0"
KF_URL_BASE="https://download.kde.org/stable/frameworks/${KF_VER%.*}"
# QCA (Qt Cryptographic Architecture): KWallet's ksecretd needs it.
QCA_VER="2.3.12"
PWP_VER="1.23.0"
# QCoro (C++ coroutines for Qt; plasma-workspace), Alpine's checksum.
QCORO_VER="0.13.0"
QCORO_SHA256="4bff7513c5c8e301b66308df05795043b1792ed16381a484e5c990171b8ff19e"

log() { echo "[build-kf6] $*"; }
die() { echo "[build-kf6] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"
# shellcheck source=lib/kde-build.sh
source "${SCRIPT_DIR}/lib/kde-build.sh"


patch_kf_source() {
    local mod="$1" src="$2"
    case "${mod}" in
        KWallet)
            local f="${src}/src/runtime/ksecretd/CMakeLists.txt"
            # Its autotests are added whatever BUILD_TESTING says.
            replace_once "${f}" $'\nadd_subdirectory(autotests)\n' \
                $'\nif(BUILD_TESTING)\n    add_subdirectory(autotests)\nendif()\n'
            # musl declares explicit_bzero under _GNU_SOURCE (glibc by
            # default); g++ defines it for the real C++ source, but the C
            # test of check_symbol_exists does not, so the check missed it
            # and secrets were wiped by the fallback loop.
            local check='check_symbol_exists(explicit_bzero "string.h" KSECRETD_HAVE_EXPLICIT_BZERO)'
            replace_once "${f}" "${check}" $'set(CMAKE_REQUIRED_DEFINITIONS -D_GNU_SOURCE)\n'"${check}"
            # libsecret's SecretSchema ends in eight reserved fields; the
            # initializer leaves them out (-Wmissing-field-initializers).
            replace_once "${src}/src/runtime/kwalletd/secretserviceclient.cpp" \
                '{"type", SECRET_SCHEMA_ATTRIBUTE_STRING}}};' \
                '{"type", SECRET_SCHEMA_ATTRIBUTE_STRING}}, 0, nullptr, nullptr, nullptr, nullptr, nullptr, nullptr, nullptr};'
            ;;
        BreezeIcons)
            # Its icon tools run during the build: through the runner.
            python3 -I "${SCRIPT_DIR}/deps-patches/cmake_run_built_tools.py" \
                "${src}/icons/CMakeLists.txt" generate-symbolic-dark qrcAlias \
                || die "failed to patch breeze-icons' tool commands"
            ;;
    esac
}

# build_kf_module NAME [TARBALL_BASE] [-- CMAKE OPTIONS...]
build_kf_module() {
    local mod="$1"
    shift
    local base
    base="$(echo "${mod}" | tr '[:upper:]' '[:lower:]')"
    if [[ $# -gt 0 && "$1" != "--" ]]; then
        base="$1"
        shift
    fi
    [[ "${1:-}" == "--" ]] && shift
    if kf_config "${mod}" >/dev/null; then
        log "${mod}: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "${base}-${KF_VER}" "${KF_URL_BASE}" kf6.sha256)"
    patch_kf_source "${mod}" "${src}"
    kde_cmake "${mod}" "${src}" "$@"
    kf_config "${mod}" >/dev/null || die "KF6 ${mod}: CMake config not installed"
}

# ── Extra CMake Modules ───────────────────────────────────────────────
build_ecm() {
    if [[ -f "${SYSROOT}/usr/share/ECM/cmake/ECMConfig.cmake" ]]; then
        log "ECM: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "extra-cmake-modules-${KF_VER}" "${KF_URL_BASE}" kf6.sha256)"
    # The Qt directories are given, not queried (lib/kde-build.sh); the
    # metatypes one could not be.
    python3 -I "${SCRIPT_DIR}/deps-patches/ecm_metatypes_dir.py" \
        "${src}/kde-modules/KDEInstallDirs6.cmake" || die "failed to patch ECM KDEInstallDirs6.cmake"
    log "Building ECM..."
    cmake_build "${src}" "${BUILD_DIR}/ecm-build" \
        -DBUILD_TESTING=OFF \
        -DBUILD_HTML_DOCS=OFF \
        -DBUILD_MAN_DOCS=OFF
}

# ── Plasma Wayland Protocols (KWindowSystem, KWin, Plasma) ────────────
build_plasma_wayland_protocols() {
    if [[ -d "${SYSROOT}/usr/lib/cmake/PlasmaWaylandProtocols" ]]; then
        log "PlasmaWaylandProtocols: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "plasma-wayland-protocols-${PWP_VER}" \
        "https://download.kde.org/stable/plasma-wayland-protocols" kf6.sha256)"
    kde_cmake PlasmaWaylandProtocols "${src}"
}

# ── QCA ───────────────────────────────────────────────────────────────
# With its OpenSSL provider (a plugin QCA loads); KWallet's ksecretd needs it.
build_qca() {
    local targets="${SYSROOT}/usr/lib/cmake/Qca-qt6/Qca-qt6Targets.cmake"
    # An install that exports absolute paths names the build host's /usr.
    if [[ -f "${targets}" ]] && ! grep -q 'INTERFACE_INCLUDE_DIRECTORIES "/usr/' "${targets}"; then
        log "QCA: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "qca-${QCA_VER}" "https://download.kde.org/stable/qca/${QCA_VER}" kf6.sha256)"
    # QcaConfig.cmake.in tests @BUILD_SHARED_LIBRARIES@, which no variable
    # is called, so the installed config always takes its static branch.
    replace_once "${src}/QcaConfig.cmake.in" '@BUILD_SHARED_LIBRARIES@' '@BUILD_SHARED_LIBS@'
    kde_cmake QCA "${src}" \
        -DBUILD_WITH_QT6=ON \
        -DBUILD_TESTS=OFF \
        -DBUILD_TOOLS=OFF \
        -DWITH_ossl_PLUGIN=yes \
        -DUSE_RELATIVE_PATHS=ON
    # Relocatable: its targets name the headers relative to the package
    # (it exported /usr/include/..., the build host's, otherwise).
    [[ -f "${targets}" ]] && ! grep -q 'INTERFACE_INCLUDE_DIRECTORIES "/usr/' "${targets}" ||
        die "QCA did not install a relocatable Qca-qt6Targets.cmake"
}

# ── QCoro ─────────────────────────────────────────────────────────────
build_qcoro() {
    if [[ -f "${SYSROOT}/usr/lib/cmake/QCoro6/QCoro6Config.cmake" ]]; then
        log "QCoro: already installed."
        return 0
    fi
    fetch "qcoro-${QCORO_VER}.tar.gz" "https://github.com/qcoro/qcoro/archive/refs/tags/v${QCORO_VER}.tar.gz" \
        "qcoro-${QCORO_VER}" "${QCORO_SHA256}"
    kde_cmake QCoro "${BUILD_DIR}/qcoro-${QCORO_VER}" \
        -DUSE_QT_VERSION=6 \
        -DQCORO_BUILD_EXAMPLES=OFF
    [[ -f "${SYSROOT}/usr/lib/cmake/QCoro6/QCoro6Config.cmake" ]] || die "QCoro did not install QCoro6Config.cmake"
}

# Tier 1: No KF dependencies
build_tier1() {
    local mod
    for mod in KConfig KCoreAddons KI18n KGuiAddons KWidgetsAddons KColorScheme \
               KArchive KCodecs KItemViews; do
        build_kf_module "${mod}"
    done
    # Breeze icons, compiled into a library the icon loader uses
    # (KIconThemes, USE_BreezeIcons).
    # It generates icons with Python and lxml (host-python-requirements.txt).
    host_python
    build_kf_module BreezeIcons breeze-icons -- -DPython_EXECUTABLE="${HOST_PYTHON}"
    # Required by plasma-workspace: syntax highlighting (KTextEditor),
    # holidays (calendar), charts (system monitor applets), and
    # NetworkManagerQt (libnm from build-dbus.sh).
    build_ksyntaxhighlighting
    build_kf_module KHolidays
    build_kf_module KQuickCharts
    build_kf_module NetworkManagerQt networkmanager-qt
    build_qcoro
}

# KSyntaxHighlighting compiles its highlighting definitions during the build
# with katehighlightingindexer, a Qt program. Cross-compiling, upstream builds
# that as a native sub-project, which inherits the target compilers from this
# environment (CC/CXX) and so builds a musl program against the host Qt. The
# indexer is built here for the host instead -- host compilers, the host Qt,
# ECM from the sysroot -- and passed as KATEHIGHLIGHTINGINDEXER_EXECUTABLE,
# upstream's option for this, as wayland-scanner is a host build.
KATE_INDEXER="${VERIDIAN_HOST_TOOLS}/bin/katehighlightingindexer"
build_ksyntaxhighlighting() {
    if kf_config KSyntaxHighlighting >/dev/null; then
        log "KSyntaxHighlighting: already installed."
        return 0
    fi
    local src bld="${BUILD_DIR}/katehighlightingindexer-host"
    src="$(kde_fetch "syntax-highlighting-${KF_VER}" "${KF_URL_BASE}" kf6.sha256)"
    if [[ ! -x "${KATE_INDEXER}" || "$(cat "${KATE_INDEXER}.version" 2>/dev/null)" != "${KF_VER}" ]]; then
        log "Building katehighlightingindexer ${KF_VER} (host)..."
        rm -rf "${bld}"
        (unset CC CXX AR RANLIB NM STRIP CFLAGS CXXFLAGS LDFLAGS \
               PKG_CONFIG_LIBDIR PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR && \
            cmake -S "${src}" -B "${bld}" \
                -DCMAKE_BUILD_TYPE=Release \
                -DKSYNTAXHIGHLIGHTING_USE_GUI=OFF \
                -DBUILD_TESTING=OFF \
                -DBUILD_QCH=OFF \
                -DQT_MAJOR_VERSION=6 \
                -DECM_DIR="${SYSROOT}/usr/share/ECM/cmake" \
                -DCMAKE_PREFIX_PATH="${VERIDIAN_HOST_TOOLS}/qt6" && \
            cmake --build "${bld}" --target katehighlightingindexer -j"${JOBS}") \
            || die "host katehighlightingindexer build failed"
        install -Dm755 "${bld}/bin/katehighlightingindexer" "${KATE_INDEXER}"
        echo "${KF_VER}" > "${KATE_INDEXER}.version"
    fi
    patch_kf_source KSyntaxHighlighting "${src}"
    kde_cmake KSyntaxHighlighting "${src}" -DKATEHIGHLIGHTINGINDEXER_EXECUTABLE="${KATE_INDEXER}"
    kf_config KSyntaxHighlighting >/dev/null || die "KF6 KSyntaxHighlighting: CMake config not installed"
}

# Tier 2: Depends on Tier 1
build_tier2() {
    build_kf_module KIconThemes
    # KWindowSystem: Wayland (its platform plugin); no X11 server here.
    build_kf_module KWindowSystem -- -DKWINDOWSYSTEM_X11=OFF
    # KIdleTime: required by KWin. Wayland (its poller plugin).
    build_kf_module KIdleTime
    build_kf_module KGlobalAccel
    build_kf_module KPackage
    build_kf_module KCompletion
    build_kf_module KNotifications
    build_kf_module KJobWidgets
    build_kf_module KAuth
    build_kf_module KConfigWidgets
    build_kf_module KService
    build_kf_module Solid
}

# Tier 3: Depends on Tier 1+2
build_tier3() {
    build_kf_module KDeclarative
    build_kf_module KXmlGui
    build_kf_module KBookmarks
    # KIO requires KCrash and KDBusAddons.
    build_kf_module KCrash
    build_kf_module KDBusAddons
    build_kf_module KIO
    # KCMUtils requires the org.kde.kirigami QML module.
    build_kf_module Kirigami
    build_kf_module KCMUtils
}

# Tier 4: Additional modules needed by plasma-workspace
build_tier4() {
    # KItemModels: proxy model classes (KRunner links them)
    build_kf_module KItemModels
    # Sonnet: spell checking, required by KTextWidgets. No spell-check
    # backend (Hunspell, Aspell) is in the sysroot.
    build_kf_module Sonnet -- -DSONNET_NO_BACKENDS=ON
    # KTextWidgets: text editing widgets. No text-to-speech engine.
    build_kf_module KTextWidgets -- -DWITH_TEXT_TO_SPEECH=OFF
    build_qca
    # KWallet: secret storage -- the library, the ksecretd and kwalletd
    # daemons (libgcrypt from build-deps.sh) and kwallet-query. GpgME
    # wallets are optional and not in the sysroot.
    build_kf_module KWallet
    # Attica: Open Collaboration Services client, required by KNewStuff
    build_kf_module Attica
    build_kf_module KNewStuff
    build_kf_module KRunner
    build_kf_module KStatusNotifierItem
    # KNotifyConfig: notification settings, with sound preview (Canberra)
    build_kf_module KNotifyConfig
    build_kf_module KParts
    # KTextEditor: the editor component (plasma-workspace requires it).
    build_kf_module KTextEditor
    build_kf_module KDED
    # Prison: barcode/QR code library (used by some Plasma applets)
    build_kf_module Prison
    # KSvg: SVG rendering for Plasma themes
    build_kf_module KSvg
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KF6 installation..."
    local errors=0 mod
    # Every module this script builds: a failed build has already stopped
    # it, so a missing config here means an install went wrong.
    for mod in KConfig KCoreAddons KI18n KGuiAddons KWidgetsAddons KColorScheme KArchive \
               KCodecs KItemViews BreezeIcons KSyntaxHighlighting KHolidays KQuickCharts \
               NetworkManagerQt KIconThemes KWindowSystem KIdleTime \
               KGlobalAccel KPackage KCompletion KNotifications KJobWidgets KAuth \
               KConfigWidgets KService Solid KDeclarative KXmlGui KBookmarks KCrash \
               KDBusAddons KIO Kirigami KCMUtils KItemModels Sonnet KTextWidgets KWallet \
               Attica KNewStuff KRunner KStatusNotifierItem KNotifyConfig KParts KTextEditor KDED \
               Prison KSvg; do
        if kf_config "${mod}" >/dev/null; then
            log "  OK: ${mod}"
        else
            log "  MISSING: ${mod}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} modules missing!"
    log "KDE Frameworks 6 ready."
}

main() {
    log "=== Building KDE Frameworks 6 ${KF_VER} for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    [[ -f "${SYSROOT}/usr/lib/libQt6Core.so" ]] || die "Qt6 not found. Run build-qt6.sh first."
    [[ -x "${VERIDIAN_HOST_TOOLS}/qt6/libexec/moc" ]] || die "Host Qt not found. Run build-qt6.sh first."

    build_ecm
    build_plasma_wayland_protocols
    build_tier1
    build_tier2
    build_tier3
    build_tier4
    verify
    log "=== KF6 build complete ==="
}

main "$@"

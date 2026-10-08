#!/usr/bin/env bash
# Build KDE Frameworks 6 (the subset KWin and Plasma need) for VeridianOS
#
# Build order follows the KF6 dependency chain. Shared (ADR 0010), for /usr, staged
# into the sysroot (lib/cross-env.sh); Qt's directory layout for plugins
# and QML (lib/kde-build.sh). Tarball checksums: KDE's
# published ones (checksums/kf6.sha256; qqc2-desktop-style and Kirigami
# Addons also verified against their signatures, Nicolas Fella's
# 90A968ACA84537CC27B99EAF2C8DF587A6D4AAC1 and Carl Schwan's
# 39FFA93CAE9C6AFC212AD00202325448204E452A, both in KDE's release keyring).
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
# Kirigami Addons (released on its own; Plasma's settings pages use it).
KIRIGAMI_ADDONS_VER="1.15.0"
# QtKeychain (ksshaskpass): a tag archive, checksum pinned when first
# downloaded (tag 0.17.0 is commit 85835a63).
QTKEYCHAIN_VER="0.17.0"
POLKIT_QT_VER="0.201.1"

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
        KDED)
            # kded6 runs kconf_update at its installed path; the expression
            # names the build's wrapper for it (cmake/VeridianTargetTools.cmake),
            # a build-host path that would be compiled into the program.
            # shellcheck disable=SC2016 # CMake syntax, not shell expansions
            replace_once "${src}/src/CMakeLists.txt" \
                'KCONF_UPDATE_EXE="$<TARGET_FILE:KF6::kconf_update>"' \
                'KCONF_UPDATE_EXE="${KDE_INSTALL_FULL_LIBEXECDIR_KF}/kconf_update"'
            ;;
        KDocTools)
            # docbookl10nhelper runs while KDocTools builds: through the
            # runner (meinproc6 already runs by target name).
            python3 -I "${SCRIPT_DIR}/deps-patches/cmake_run_built_tools.py" \
                "${src}/src/CMakeLists.txt" KF6::docbookl10nhelper \
                || die "failed to patch KDocTools' tool commands"
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
    if [[ ! -f "${targets}" ]] || grep -q 'INTERFACE_INCLUDE_DIRECTORIES "/usr/' "${targets}"; then
        die "QCA did not install a relocatable Qca-qt6Targets.cmake"
    fi
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
    # KDocTools: the KDE handbooks (DocBook, build-deps.sh), installed with
    # paths relative to the DocBook files so they hold on VeridianOS too.
    build_kf_module KDocTools -- -DRELOCATABLE_DOCBOOK_FILES=ON
    # Required by plasma-workspace: syntax highlighting (KTextEditor),
    # holidays (calendar), charts (system monitor applets), and
    # NetworkManagerQt (libnm from build-dbus.sh).
    build_ksyntaxhighlighting
    build_kf_module KHolidays
    # Unit conversion (plasma5support's weather data engines).
    build_kf_module KUnitConversion
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

# KWindowSystem with its X11 API (KX11Extras, KWindowInfo), which libplasma
# and libkscreen use without guards; its Wayland platform plugin serves the
# session. An install without the X11 API is rebuilt.
build_kwindowsystem() {
    if kf_config KWindowSystem >/dev/null && \
       [[ -f "${SYSROOT}/usr/include/KF6/KWindowSystem/KX11Extras" ]]; then
        log "KWindowSystem: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "kwindowsystem-${KF_VER}" "${KF_URL_BASE}" kf6.sha256)"
    patch_kf_source KWindowSystem "${src}"
    kde_cmake KWindowSystem "${src}" -DKWINDOWSYSTEM_X11=ON -DKWINDOWSYSTEM_WAYLAND=ON
    [[ -f "${SYSROOT}/usr/include/KF6/KWindowSystem/KX11Extras" ]] ||
        die "KWindowSystem: X11 API not installed"
}

# Tier 2: Depends on Tier 1
build_tier2() {
    build_kf_module KIconThemes
    build_kwindowsystem
    # KIdleTime: required by KWin. Wayland (its poller plugin).
    build_kf_module KIdleTime
    build_kf_module KGlobalAccel
    build_kf_module KPackage
    build_kf_module KCompletion
    build_kf_module KNotifications
    build_kf_module KJobWidgets
    build_polkit_qt
    build_kauth
    build_kf_module KConfigWidgets
    build_kf_module KService
    build_kf_module Solid
}

# Tier 3: Depends on Tier 1+2
build_tier3() {
    build_kf_module KDeclarative
    build_kf_module KXmlGui
    build_kf_module KBookmarks
    # KIO requires KCrash and KDBusAddons, and uses KDED (proxies,
    # cookies) and KWallet when they are there at configure time: they
    # come first (they were built after it, so KIO was configured without).
    build_kf_module KCrash
    build_kf_module KDBusAddons
    build_kf_module KDED
    build_qca
    # KWallet: secret storage -- the library, the ksecretd and kwalletd
    # daemons (libgcrypt from build-deps.sh) and kwallet-query. GpgME
    # wallets are optional and not in the sysroot.
    build_kf_module KWallet
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
    build_qtkeychain
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
    # Prison: barcode/QR code library (used by some Plasma applets)
    build_kf_module Prison
    # KSvg: SVG rendering for Plasma themes
    build_kf_module KSvg
    # The Qt Quick Controls style that draws QML applications and KCMs
    # with the desktop's widget style (Breeze).
    build_kf_module QQC2DesktopStyle qqc2-desktop-style
    build_kirigami_addons
    # Pseudo-terminals (kwrited, Konsole) and running programs as another
    # user (kdesu, through kde-cli-tools).
    # No utempter: VeridianOS keeps no utmp login records (nothing reads
    # them yet), so terminals are not recorded there.
    build_kf_module KPty -- -DCMAKE_DISABLE_FIND_PACKAGE_UTEMPTER=ON
    build_ksu
    # Image format plugins for Qt (PSD, XCF, TGA, QOI, ...); no external
    # codec libraries yet (AVIF, HEIF, JPEG XL, OpenEXR, RAW).
    build_kf_module KImageFormats
}

# ── polkit-qt-1 and KAuth ─────────────────────────────────────────────
# KAuth runs privileged helpers (Plasma's KCMs: fonts, date and time,
# backlight) through a backend; its only backend here is polkit, through
# polkit-qt-1 (polkit itself from build-dbus.sh). polkit-qt-1 was built in
# the Plasma phase, after KAuth, which then built no backend at all ("No
# valid KAuth backends will be built"). An install without the backend
# plugin is rebuilt.
build_polkit_qt() {
    if [[ -f "${SYSROOT}/usr/lib/cmake/PolkitQt6-1/PolkitQt6-1Config.cmake" ]]; then
        log "polkit-qt-1: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "polkit-qt-1-${POLKIT_QT_VER}" "https://download.kde.org/stable/polkit-qt-1" plasma.sha256)"
    kde_cmake polkit-qt-1 "${src}" -DQT_MAJOR_VERSION=6 -DBUILD_EXAMPLES=OFF
    [[ -f "${SYSROOT}/usr/lib/cmake/PolkitQt6-1/PolkitQt6-1Config.cmake" ]] ||
        die "polkit-qt-1: CMake config not installed"
}

build_kauth() {
    local backend="${SYSROOT}/usr/lib/qt6/plugins/kf6/kauth/backend/kauth_backend_plugin.so"
    if kf_config KAuth >/dev/null && [[ -f "${backend}" ]]; then
        log "KAuth: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "kauth-${KF_VER}" "${KF_URL_BASE}" kf6.sha256)"
    patch_kf_source KAuth "${src}"
    kde_cmake KAuth "${src}" -DKAUTH_BACKEND_NAME=POLKITQT6-1
    [[ -f "${backend}" ]] || die "KAuth: no polkit backend plugin installed"
    kf_config KAuth >/dev/null || die "KF6 KAuth: CMake config not installed"
}

# ── KSu ───────────────────────────────────────────────────────────────
# kdesu checks that the daemon's socket belongs to the user through
# SO_PEERCRED, which needs `struct ucred`: musl declares it only under
# _GNU_SOURCE, which the C configure check does not define, so the check
# failed and kdesu fell back to its "sloppy" owner check (an lstat of the
# socket path). The marker records a build made with the check passing; an
# older install is rebuilt.
build_ksu() {
    local marker="${SYSROOT}/usr/lib/cmake/KF6Su/.veridian-peercred"
    if kf_config KSu >/dev/null && [[ -f "${marker}" ]]; then
        log "KSu: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "kdesu-${KF_VER}" "${KF_URL_BASE}" kf6.sha256)"
    patch_kf_source KSu "${src}"
    kde_cmake KSu "${src}" -DCMAKE_REQUIRED_DEFINITIONS=-D_GNU_SOURCE
    grep -Eq '^HAVE_STRUCT_UCRED:INTERNAL=(1|TRUE)$' "${BUILD_DIR}/KSu-build/CMakeCache.txt" ||
        die "KSu: struct ucred not found; kdesu would use its sloppy socket check"
    kf_config KSu >/dev/null || die "KF6 KSu: CMake config not installed"
    : > "${marker}"
}

# ── QtKeychain ────────────────────────────────────────────────────────
# Passwords in the desktop's secret service (KWallet's ksecretd, through
# libsecret).
build_qtkeychain() {
    if [[ -f "${SYSROOT}/usr/lib/cmake/Qt6Keychain/Qt6KeychainConfig.cmake" ]]; then
        log "QtKeychain: already installed."
        return 0
    fi
    fetch "qtkeychain-${QTKEYCHAIN_VER}.tar.gz" \
        "https://github.com/frankosterfeld/qtkeychain/archive/refs/tags/${QTKEYCHAIN_VER}.tar.gz" \
        "qtkeychain-${QTKEYCHAIN_VER}" "$(listed_sha256 kf6.sha256 "qtkeychain-${QTKEYCHAIN_VER}.tar.gz")"
    kde_cmake QtKeychain "${BUILD_DIR}/qtkeychain-${QTKEYCHAIN_VER}" \
        -DBUILD_WITH_QT5=OFF \
        -DBUILD_TEST_APPLICATION=OFF
    [[ -f "${SYSROOT}/usr/lib/cmake/Qt6Keychain/Qt6KeychainConfig.cmake" ]] ||
        die "QtKeychain: CMake config not installed"
}

# ── Kirigami Addons ───────────────────────────────────────────────────
# Its own release series: the QML components (form cards, dialogs, date
# pickers) Plasma's KCMs and plasma-desktop are written with.
build_kirigami_addons() {
    if kf_config KirigamiAddons >/dev/null; then
        log "KirigamiAddons: already installed."
        return 0
    fi
    local src
    src="$(kde_fetch "kirigami-addons-${KIRIGAMI_ADDONS_VER}" \
        "https://download.kde.org/stable/kirigami-addons" kf6.sha256)"
    kde_cmake KirigamiAddons "${src}"
    kf_config KirigamiAddons >/dev/null || die "Kirigami Addons: CMake config not installed"
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KF6 installation..."
    local errors=0 mod
    # Every module this script builds: a failed build has already stopped
    # it, so a missing config here means an install went wrong.
    for mod in KConfig KCoreAddons KI18n KGuiAddons KWidgetsAddons KColorScheme KArchive \
               KCodecs KItemViews BreezeIcons KSyntaxHighlighting KHolidays KUnitConversion KQuickCharts \
               NetworkManagerQt KIconThemes KWindowSystem KIdleTime \
               KGlobalAccel KPackage KCompletion KNotifications KJobWidgets KAuth \
               KConfigWidgets KService Solid KDeclarative KXmlGui KBookmarks KCrash \
               KDBusAddons KIO Kirigami KCMUtils KItemModels Sonnet KTextWidgets KWallet \
               Attica KNewStuff KRunner KStatusNotifierItem KNotifyConfig KParts KTextEditor KDED \
               Prison KSvg QQC2DesktopStyle KirigamiAddons KPty KSu KImageFormats; do
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

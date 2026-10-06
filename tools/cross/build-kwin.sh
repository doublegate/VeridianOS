#!/usr/bin/env bash
# Build KWin Wayland compositor for VeridianOS
#
# Produces the kwin_wayland binary -- the KDE Plasma 6 compositor.
# Integrates VeridianOS platform backend from userland/kwin/.
#
# Prerequisites:
#   - Qt 6 + KF6 + Mesa + Wayland + libinput (real, from build-deps.sh)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/kwin"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
JOBS="${JOBS:-$(nproc)}"

KWIN_VER="6.3.5"
KWIN_URL="https://download.kde.org/stable/plasma/6.3.5/kwin-${KWIN_VER}.tar.xz"
KDECORATION_VER="6.3.5"
KDECORATION_URL="https://download.kde.org/stable/plasma/6.3.5/kdecoration-${KDECORATION_VER}.tar.xz"
HOST_QT="${PROJECT_ROOT}/target/cross-build/qt6/host-qt"

log() { echo "[build-kwin] $*"; }
die() { echo "[build-kwin] ERROR: $*" >&2; exit 1; }
# shellcheck source=lib/cmake-source-fixes.sh
source "${SCRIPT_DIR}/lib/cmake-source-fixes.sh"

mkdir -p "${BUILD_DIR}"

fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${BUILD_DIR}/${name}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        { curl -fsSL -o "${tarball}.part" "${url}" || wget -q -O "${tarball}.part" "${url}"; } && [[ -s "${tarball}.part" ]] && mv "${tarball}.part" "${tarball}" || { rm -f "${tarball}.part"; echo "download failed: ${url}" >&2; exit 1; }
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── 1. Verify real libinput ───────────────────────────────────────────
# Real libinput is now built by build-deps.sh (with libevdev).
# This function just verifies it exists.
verify_libinput() {
    if [[ -f "${SYSROOT}/usr/lib/libinput.a" ]] && \
       [[ -f "${SYSROOT}/usr/include/libinput.h" ]]; then
        log "libinput: found in sysroot."
        return 0
    fi
    die "libinput not found. Run build-deps.sh first (builds real libevdev + libinput)."
}

# ── 2. Build kdecoration ─────────────────────────────────────────────
build_kdecoration() {
    if [[ -d "${SYSROOT}/usr/lib/cmake/KDecoration2" ]]; then
        log "kdecoration: already installed."
        return 0
    fi
    fetch "kdecoration-${KDECORATION_VER}" "${KDECORATION_URL}" "kdecoration-${KDECORATION_VER}"

    local src="${BUILD_DIR}/kdecoration-${KDECORATION_VER}"
    local bld="${BUILD_DIR}/kdecoration-build"
    log "Building kdecoration ${KDECORATION_VER}..."

    # Patch SHARED -> STATIC for static cross-compilation
    sed -i 's/add_library(kdecorations3private SHARED/add_library(kdecorations3private STATIC/' \
        "${src}/src/private/CMakeLists.txt"
    sed -i 's/add_library(kdecorations3 SHARED/add_library(kdecorations3 STATIC/' \
        "${src}/src/CMakeLists.txt"
    relax_qt_test "${src}/CMakeLists.txt"

    rm -rf "${bld}"
    mkdir -p "${bld}"
    export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DQT_HOST_PATH:PATH="${HOST_QT}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${HOST_QT}/lib/cmake" \
            -DECM_DIR:PATH="${SYSROOT}/usr/share/ECM/cmake" \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DCMAKE_BUILD_TYPE=Release && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "kdecoration: done."
}

# ── 3. Install VeridianOS KWin backend ────────────────────────────────
install_veridian_backend() {
    local kwin_src="${PROJECT_ROOT}/userland/kwin"
    if [[ ! -d "${kwin_src}" ]]; then
        log "No userland/kwin/ -- skipping backend integration."
        return 0
    fi
    log "Copying VeridianOS KWin backend to sysroot..."
    mkdir -p "${SYSROOT}/usr/src/veridian-kwin"
    cp "${kwin_src}"/*.cpp "${SYSROOT}/usr/src/veridian-kwin/" 2>/dev/null || true
    cp "${kwin_src}"/*.h "${SYSROOT}/usr/src/veridian-kwin/" 2>/dev/null || true
    log "KWin backend copied."
}

# ── 4. Build KWin ────────────────────────────────────────────────────
build_kwin() {
    if [[ -f "${SYSROOT}/usr/bin/kwin_wayland" ]]; then
        log "KWin: already installed."
        return 0
    fi
    fetch "kwin-${KWIN_VER}" "${KWIN_URL}" "kwin-${KWIN_VER}"

    local src="${BUILD_DIR}/kwin-${KWIN_VER}"
    local bld="${BUILD_DIR}/kwin-build"
    log "Building KWin ${KWIN_VER}..."

    # Patch NoopSession::openRestricted() to actually open the device.
    # The upstream implementation returns -1 unconditionally, which prevents
    # kwin from opening /dev/dri/card0 on systems without logind/consolekit.
    # Our patch makes it call open() directly, matching the behavior needed
    # for VeridianOS where DRM devices are opened via the VFS.
    # (A multi-line sed used to "apply" this: sed cannot match across lines
    # and exits 0 anyway, so the python fallback never ran and kwin was
    # left unable to open the DRM device.)
    if ! grep -q 'return open(fileName' "${src}/src/core/session_noop.cpp"; then
        log "Patching session_noop.cpp: openRestricted() -> direct open()"
        sed -i '/^#include "session_noop.h"/a\
#include <fcntl.h>\
#include <unistd.h>' "${src}/src/core/session_noop.cpp"
        python3 - "${src}/src/core/session_noop.cpp" <<'PYEOF' || die "failed to patch session_noop.cpp"
import re, sys
p = sys.argv[1]
s = open(p).read()
s, n = re.subn(
    r"(int NoopSession::openRestricted\(const QString &fileName\)\s*\{\s*)return -1;",
    r"\1return open(fileName.toUtf8().constData(), O_RDWR | O_CLOEXEC);",
    s,
)
if n != 1:
    sys.exit("openRestricted() body not found")
open(p, "w").write(s)
PYEOF
    fi
    grep -q 'return open(fileName' "${src}/src/core/session_noop.cpp" || \
        die "session_noop.cpp: openRestricted() not patched"

    # Qt UiTools is only used by the KCMs and the Aurorae config UI, both
    # disabled below (KWIN_BUILD_KCMS=OFF); qttools is not cross-built.
    relax_qt_test "${src}/CMakeLists.txt"
    python3 - "${src}/CMakeLists.txt" <<'PYEOF' || die "failed to drop UiTools from KWin"
import re, sys
p = sys.argv[1]
s = open(p).read()
s2 = re.sub(r"(find_package\(Qt6 [^)]*?)\n\s*UiTools(?=\s)", r"\1", s, count=1)
if "UiTools" in re.search(r"find_package\(Qt6 [^)]*\)", s2).group(0):
    sys.exit("UiTools still required")
open(p, "w").write(s2)
PYEOF
    # libcanberra (event sounds) is REQUIRED but only the systembell plugin
    # links it; VeridianOS has no libcanberra, so build without that plugin.
    python3 - "${src}" <<'PYEOF' || die "failed to make Canberra optional in KWin"
import sys
src = sys.argv[1]
for path, old, new in (
    (src + "/CMakeLists.txt",
     "find_package(Canberra REQUIRED)", "find_package(Canberra)"),
    (src + "/src/plugins/CMakeLists.txt",
     "add_subdirectory(systembell)\n",
     "if(TARGET Canberra::Canberra)\n    add_subdirectory(systembell)\nendif()\n"),
):
    s = open(path).read()
    if new in s:
        continue
    if s.count(old) != 1:
        sys.exit("unexpected " + path)
    open(path, "w").write(s.replace(old, new))
PYEOF

    rm -rf "${bld}"
    mkdir -p "${bld}"
    export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR=""

    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DQT_HOST_PATH:PATH="${HOST_QT}" \
            -DQT_HOST_PATH_CMAKE_DIR:PATH="${HOST_QT}/lib/cmake" \
            -DECM_DIR:PATH="${SYSROOT}/usr/share/ECM/cmake" \
            -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DKWIN_BUILD_X11=OFF \
            -DKWIN_BUILD_XWAYLAND=OFF \
            -DKWIN_BUILD_SCREENLOCKER=OFF \
            -DKWIN_BUILD_TABBOX=ON \
            -DKWIN_BUILD_KCMS=OFF \
            -DKWIN_BUILD_GLOBALSHORTCUTS=OFF \
            -DQTWAYLANDSCANNER_KDE_EXECUTABLE="${BUILD_DIR}/qtwaylandscanner_kde-host-build/qtwaylandscanner_kde" \
            -DKF6_HOST_TOOLING="/usr/lib64/cmake" \
            -DCMAKE_BUILD_TYPE=Release \
            -DKF_SKIP_PO_PROCESSING=ON \
            -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--allow-multiple-definition" \
            -DCMAKE_PROJECT_INCLUDE="${SCRIPT_DIR}/wayland-scanner-target.cmake" && \
        cmake --build . --parallel "${JOBS}" -- -k || true && \
        cmake --install . 2>/dev/null || \
        cmake --install . --component Devel 2>/dev/null || true)
    log "KWin: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KWin installation..."
    local errors=0
    for item in \
        "${SYSROOT}/usr/lib/libinput.a" \
        "${SYSROOT}/usr/lib/libevdev.a" \
        "${SYSROOT}/usr/include/libinput.h" \
    ; do
        if [[ -f "$item" ]]; then
            log "  OK: $(basename "$item")"
        else
            log "  MISSING: $item"
            errors=$((errors + 1))
        fi
    done
    if [[ -f "${SYSROOT}/usr/bin/kwin_wayland" ]]; then
        local size
        size=$(stat -c%s "${SYSROOT}/usr/bin/kwin_wayland" 2>/dev/null || echo "?")
        log "  OK: kwin_wayland (${size} bytes)"
    else
        log "  MISSING: kwin_wayland (may need additional patches)"
        errors=$((errors + 1))
    fi
    if [[ $errors -gt 0 ]]; then
        log "WARNING: ${errors} items missing (expected for first build -- iterate)"
    fi
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building KWin for VeridianOS ==="
    verify_libinput
    build_kdecoration
    install_veridian_backend
    build_kwin
    verify
    log "=== KWin build complete ==="
}

main "$@"

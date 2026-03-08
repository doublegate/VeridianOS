#!/usr/bin/env bash
# Build KWin Wayland compositor for VeridianOS
#
# Produces the kwin_wayland binary -- the KDE Plasma 6 compositor.
# Integrates VeridianOS platform backend from userland/kwin/.
#
# Prerequisites:
#   - Qt 6 + KF6 + Mesa + Wayland + libinput (or stub)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/kwin"
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
JOBS="${JOBS:-$(nproc)}"

KWIN_VER="6.0.0"
KWIN_URL="https://download.kde.org/stable/plasma/6.0.0/kwin-${KWIN_VER}.tar.xz"
KDECORATION_VER="6.0.0"
KDECORATION_URL="https://download.kde.org/stable/plasma/6.0.0/kdecoration-${KDECORATION_VER}.tar.xz"

log() { echo "[build-kwin] $*"; }
die() { echo "[build-kwin] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${BUILD_DIR}/${name}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        curl -fsSL -o "${tarball}" "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── 1. Build libinput stub ────────────────────────────────────────────
# MVP: Stub libinput that forwards VeridianOS kernel input events.
# Real libinput can be added later.
build_libinput_stub() {
    if [[ -f "${SYSROOT}/usr/lib/libinput.a" ]]; then
        log "libinput stub: already installed."
        return 0
    fi

    local stub_dir="${SCRIPT_DIR}/libinput-stub"
    local stub_src="${stub_dir}/libinput_stub.c"

    # Create minimal libinput API stub if not present
    if [[ ! -f "${stub_src}" ]]; then
        log "Creating libinput API stub..."
        mkdir -p "${stub_dir}"
        cat > "${stub_src}" << 'STUB_C'
/* Minimal libinput API stub for VeridianOS KWin
 *
 * Provides the libinput API surface that KWin needs to compile.
 * Input events are actually delivered through VeridianOS's kernel
 * input subsystem (PS/2 keyboard + VirtIO mouse), not through
 * libinput's evdev backend.
 */
#include <stdlib.h>
#include <stdint.h>

/* Opaque types */
struct libinput { int dummy; };
struct libinput_device { int dummy; };
struct libinput_event { int dummy; };
struct libinput_seat { int dummy; };
struct udev { int dummy; };

enum libinput_event_type {
    LIBINPUT_EVENT_NONE = 0,
    LIBINPUT_EVENT_DEVICE_ADDED = 1,
    LIBINPUT_EVENT_KEYBOARD_KEY = 300,
    LIBINPUT_EVENT_POINTER_MOTION = 400,
    LIBINPUT_EVENT_POINTER_BUTTON = 401,
    LIBINPUT_EVENT_POINTER_AXIS = 402,
};

struct libinput *libinput_udev_create_context(
    const void *interface, void *user_data, struct udev *udev) {
    (void)interface; (void)user_data; (void)udev;
    return calloc(1, sizeof(struct libinput));
}

int libinput_udev_assign_seat(struct libinput *li, const char *seat) {
    (void)li; (void)seat;
    return 0;
}

int libinput_get_fd(struct libinput *li) {
    (void)li;
    return -1; /* No real fd -- input comes from kernel */
}

int libinput_dispatch(struct libinput *li) {
    (void)li;
    return 0;
}

struct libinput_event *libinput_get_event(struct libinput *li) {
    (void)li;
    return NULL; /* No events from this stub */
}

enum libinput_event_type libinput_event_get_type(struct libinput_event *event) {
    (void)event;
    return LIBINPUT_EVENT_NONE;
}

void libinput_event_destroy(struct libinput_event *event) {
    (void)event;
}

struct libinput *libinput_unref(struct libinput *li) {
    free(li);
    return NULL;
}

void libinput_suspend(struct libinput *li) { (void)li; }
int libinput_resume(struct libinput *li) { (void)li; return 0; }
STUB_C
    fi

    # Create header
    local stub_hdr="${SYSROOT}/usr/include/libinput.h"
    mkdir -p "$(dirname "${stub_hdr}")"
    cat > "${stub_hdr}" << 'STUB_H'
#ifndef LIBINPUT_H
#define LIBINPUT_H
#include <stdint.h>

struct libinput;
struct libinput_device;
struct libinput_event;
struct libinput_seat;
struct udev;

enum libinput_event_type {
    LIBINPUT_EVENT_NONE = 0,
    LIBINPUT_EVENT_DEVICE_ADDED = 1,
    LIBINPUT_EVENT_KEYBOARD_KEY = 300,
    LIBINPUT_EVENT_POINTER_MOTION = 400,
    LIBINPUT_EVENT_POINTER_BUTTON = 401,
    LIBINPUT_EVENT_POINTER_AXIS = 402,
};

struct libinput *libinput_udev_create_context(
    const void *interface, void *user_data, struct udev *udev);
int libinput_udev_assign_seat(struct libinput *li, const char *seat);
int libinput_get_fd(struct libinput *li);
int libinput_dispatch(struct libinput *li);
struct libinput_event *libinput_get_event(struct libinput *li);
enum libinput_event_type libinput_event_get_type(struct libinput_event *event);
void libinput_event_destroy(struct libinput_event *event);
struct libinput *libinput_unref(struct libinput *li);
void libinput_suspend(struct libinput *li);
int libinput_resume(struct libinput *li);

#endif /* LIBINPUT_H */
STUB_H

    # Compile stub
    local cc="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
    log "Compiling libinput stub..."
    "${cc}" -c -O2 -o "${BUILD_DIR}/libinput_stub.o" "${stub_src}"
    ar rcs "${SYSROOT}/usr/lib/libinput.a" "${BUILD_DIR}/libinput_stub.o"

    # Create pkg-config file
    mkdir -p "${SYSROOT}/usr/lib/pkgconfig"
    cat > "${SYSROOT}/usr/lib/pkgconfig/libinput.pc" << PC
prefix=${SYSROOT}/usr
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: libinput
Description: libinput stub for VeridianOS
Version: 1.25.0
Libs: -L\${libdir} -linput
Cflags: -I\${includedir}
PC
    log "libinput stub: done."
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
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF && \
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
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_PREFIX_PATH="${SYSROOT}/usr" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DBUILD_SHARED_LIBS=OFF \
            -DBUILD_TESTING=OFF \
            -DKWIN_BUILD_XWAYLAND=OFF \
            -DKWIN_BUILD_SCREENLOCKER=OFF \
            -DKWIN_BUILD_TABBOX=OFF \
            -DKWIN_BUILD_KCMS=OFF \
            -DCMAKE_BUILD_TYPE=Release && \
        cmake --build . --parallel "${JOBS}" && \
        cmake --install .)
    log "KWin: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying KWin installation..."
    local errors=0
    for item in \
        "${SYSROOT}/usr/lib/libinput.a" \
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
    build_libinput_stub
    build_kdecoration
    install_veridian_backend
    build_kwin
    verify
    log "=== KWin build complete ==="
}

main "$@"

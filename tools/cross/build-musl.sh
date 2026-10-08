#!/usr/bin/env bash
# Build musl libc for VeridianOS cross-compilation
#
# This script downloads and cross-compiles musl libc 1.2.6 with the patches
# in musl-patches/ (no syscall patch since ADR 0009: the kernel speaks the
# Linux ABI): the C library, static and shared, its dynamic loader, and the
# C headers, in the sysroot.
#
# Prerequisites:
#   - GCC cross-compiler (x86_64-linux-musl or host gcc for static target)
#   - curl (or wget) for downloading source
#
# Output (installed for /usr, staged into the sysroot):
#   $SYSROOT/usr/lib/libc.a, libc.so and the crt objects
#   $SYSROOT/lib/ld-musl-x86_64.so.1 (the dynamic loader: a link to
#     /usr/lib/libc.so, which is both; ADR 0010)
#   $SYSROOT/usr/include/ (C library and Linux UAPI headers)
#   $SYSROOT/.musl-stamp (what this musl was built from)
#
# The host GCC builds musl itself (musl needs no C library to build);
# build-musl-toolchain.sh then builds the cross compiler against it.

set -euo pipefail

MUSL_VERSION="1.2.6"
MUSL_URL="https://musl.libc.org/releases/musl-${MUSL_VERSION}.tar.gz"
MUSL_SHA256="d585fd3b613c66151fc3249e8ed44f77020cb5e6c1e635a616d3f9f82460512a"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/musl"
SYSROOT="${VERIDIAN_SYSROOT}"
PATCH_DIR="${SCRIPT_DIR}/musl-patches"
JOBS="${JOBS:-$(nproc)}"

# musl is freestanding code: the host GCC builds it for x86_64.
CROSS_CC="${CROSS_CC:-gcc}"

log() { echo "[build-musl] $*"; }
die() { echo "[build-musl] ERROR: $*" >&2; exit 1; }

# ── Download ──────────────────────────────────────────────────────────
download_musl() {
    local tarball="${VERIDIAN_SOURCES}/musl-${MUSL_VERSION}.tar.gz"
    if [[ -f "${tarball}" ]]; then
        log "Source tarball already downloaded."
        verify_musl "${tarball}"
        return 0
    fi
    mkdir -p "${BUILD_DIR}"
    log "Downloading musl ${MUSL_VERSION}..."
    # curl first: wget may be a sandbox wrapper (firejail) that cannot
    # write outside the home directory, e.g. to /opt/veridian/sources.
    if command -v curl &>/dev/null; then
        curl -fsSL -o "${tarball}" "${MUSL_URL}"
    elif command -v wget &>/dev/null; then
        wget -q -O "${tarball}" "${MUSL_URL}"
    else
        die "Need wget or curl to download musl."
    fi
    verify_musl "${tarball}"
}

# The tarball must be the release the patches were made for.
verify_musl() {
    local got
    got="$(sha256sum "$1" | cut -d' ' -f1)"
    [[ "${got}" == "${MUSL_SHA256}" ]] \
        || die "musl tarball checksum mismatch (got ${got}); remove $1 and retry."
}

# ── Extract ───────────────────────────────────────────────────────────
# Identity of the patch set: the .patch files plus this script. A tree
# patched with anything else is re-extracted, so a changed patch can never
# be silently skipped.
patch_stamp() {
    # nullglob: an empty patch set is valid (cat of a literal "*.patch"
    # would fail, and with pipefail stop the build).
    local patches
    patches=$(shopt -s nullglob; echo "${PATCH_DIR}"/*.patch)
    # shellcheck disable=SC2086 # word splitting of the list is intended
    cat $patches "${BASH_SOURCE[0]}" | sha256sum | cut -d' ' -f1
}

extract_musl() {
    local src="${BUILD_DIR}/musl-${MUSL_VERSION}"
    local marker="${src}/.veridian_patched"
    if [[ -d "${src}" ]]; then
        if [[ -f "${marker}" && "$(cat "${marker}")" == "$(patch_stamp)" ]]; then
            log "Source already extracted."
            return 0
        fi
        log "Patch set changed (or tree not fully patched): re-extracting."
        rm -rf "${src}" "${BUILD_DIR}/build"
    fi
    log "Extracting..."
    mkdir -p "${BUILD_DIR}"
    tar -xzf "${VERIDIAN_SOURCES}/musl-${MUSL_VERSION}.tar.gz" -C "${BUILD_DIR}"
}

# ── Patch ─────────────────────────────────────────────────────────────
# Apply musl-patches/*.patch (an empty set is valid).
patch_musl() {
    local src="${BUILD_DIR}/musl-${MUSL_VERSION}"
    local marker="${src}/.veridian_patched"
    if [[ -f "${marker}" && "$(cat "${marker}")" == "$(patch_stamp)" ]]; then
        log "Already patched."
        return 0
    fi
    log "Applying VeridianOS patches (if any)..."
    if [[ -d "${PATCH_DIR}" ]]; then
        for patch in "${PATCH_DIR}"/*.patch; do
            [[ -f "$patch" ]] || continue
            log "  Applying $(basename "$patch")..."
            (cd "${src}" && patch -p1 < "$patch")
        done
    fi

    patch_stamp > "${marker}"
    log "Patches applied."
}

# ── Configure ─────────────────────────────────────────────────────────
configure_musl() {
    local src="${BUILD_DIR}/musl-${MUSL_VERSION}"
    local build="${BUILD_DIR}/build"
    if [[ -f "${build}/config.mak" ]]; then
        log "Already configured."
        return 0
    fi
    mkdir -p "${build}"
    log "Configuring musl for VeridianOS..."
    # For /usr on the target, staged into the sysroot by install_musl, with
    # the dynamic loader in /lib (where every dynamically linked program
    # asks for it). No musl-gcc wrapper: the cross toolchain is built
    # against this sysroot.
    (cd "${build}" && \
        "${src}/configure" \
            --prefix=/usr \
            --syslibdir=/lib \
            --enable-shared \
            --enable-static \
            --disable-wrapper \
            CC="${CROSS_CC}" \
            CFLAGS="-O2 -fPIC" \
    )
}

# ── Build ─────────────────────────────────────────────────────────────
build_musl() {
    local build="${BUILD_DIR}/build"
    log "Building musl (${JOBS} jobs)..."
    make -C "${build}" -j"${JOBS}"
}

# ── Install ───────────────────────────────────────────────────────────
install_musl() {
    local build="${BUILD_DIR}/build"
    log "Installing to ${SYSROOT}..."
    mkdir -p "${SYSROOT}/usr"
    make -C "${build}" install DESTDIR="${SYSROOT}"
    # What this musl is, next to it (the build tree is disposable): the
    # cross toolchain rebuilds when it changes.
    patch_stamp > "${SYSROOT}/.musl-stamp"
}

# ── Linux UAPI headers ────────────────────────────────────────────────
# musl does not ship the Linux kernel UAPI headers (linux/*, asm/*), but
# several dependencies include them (libffi: linux/limits.h; libinput,
# evdev, DRM users: linux/input.h, drm/*). VeridianOS implements the Linux
# x86_64 ABI, so the host's x86_64 linux-api-headers are the right set.
# Override with LINUX_HEADERS_DIR to use a pinned headers_install tree.
install_linux_headers() {
    local src="${LINUX_HEADERS_DIR:-/usr/include}"
    local dst="${SYSROOT}/usr/include"
    [[ -f "${src}/linux/limits.h" ]] || die "Linux UAPI headers not found in ${src} (install linux-api-headers or set LINUX_HEADERS_DIR)"
    log "Installing Linux UAPI headers from ${src}..."
    local dir
    for dir in linux asm asm-generic drm mtd rdma scsi sound video misc xen; do
        if [[ -d "${src}/${dir}" ]]; then
            mkdir -p "${dst}/${dir}"
            cp -a "${src}/${dir}/." "${dst}/${dir}/"
        fi
    done
}

# ── Verify ────────────────────────────────────────────────────────────
verify_install() {
    log "Verifying installation..."
    local errors=0
    for f in \
        "${SYSROOT}/usr/lib/libc.a" \
        "${SYSROOT}/usr/lib/libc.so" \
        "${SYSROOT}/usr/include/stdio.h" \
        "${SYSROOT}/usr/include/stdlib.h" \
        "${SYSROOT}/usr/include/unistd.h" \
        "${SYSROOT}/usr/include/pthread.h" \
        "${SYSROOT}/usr/include/sys/socket.h" \
        "${SYSROOT}/usr/include/sys/epoll.h" \
        "${SYSROOT}/usr/include/linux/limits.h" \
        "${SYSROOT}/usr/lib/crt1.o" \
    ; do
        if [[ ! -f "$f" ]]; then
            log "  MISSING: $f"
            errors=$((errors + 1))
        fi
    done

    # The loader is a link to libc.so, absolute as on the target.
    if [[ "$(readlink "${SYSROOT}/lib/ld-musl-x86_64.so.1")" != "/usr/lib/libc.so" ]]; then
        log "  MISSING: ${SYSROOT}/lib/ld-musl-x86_64.so.1 -> /usr/lib/libc.so"
        errors=$((errors + 1))
    fi

    if [[ $errors -eq 0 ]]; then
        log "All files present. musl libc ready."
        local size
        size=$(stat -c%s "${SYSROOT}/usr/lib/libc.a" 2>/dev/null || echo "?")
        log "  libc.a size: ${size} bytes"
    else
        die "${errors} files missing!"
    fi
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building musl libc ${MUSL_VERSION} for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"
    log "Build dir: ${BUILD_DIR}"

    download_musl
    extract_musl
    patch_musl
    configure_musl
    build_musl
    install_musl
    install_linux_headers
    verify_install

    log "=== musl libc build complete ==="
}

main "$@"

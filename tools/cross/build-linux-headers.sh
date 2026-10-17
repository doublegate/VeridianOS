#!/usr/bin/env bash
# Linux UAPI headers (linux/*, asm/*, asm-generic/*, ...) for the target
#
# musl does not ship them, but programs and libraries include them
# (linux/futex.h, linux/input.h, drm/*). VeridianOS implements the Linux
# system call ABI (ADR 0009), so Linux's own headers for the architecture are
# the right set. The host's /usr/include has them for x86_64 only, which is
# what build-musl.sh uses there; for aarch64 and riscv64 this installs them
# from the Linux source with `make headers_install` (the release whose
# system call tables abi/linux/ vendors, v7.2).
#
# Output: ${VERIDIAN_CROSS_BUILD}/linux-headers/include (build-musl.sh copies
# it into the sysroot when LINUX_HEADERS_DIR points there).

set -euo pipefail

LINUX_VERSION="7.2.11"
LINUX_URL="https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-${LINUX_VERSION}.tar.xz"
# From kernel.org's sha256sums.asc for v7.x.
LINUX_SHA256="fd0fc479f983a8c05313f13e9c36330fe6651cf725396748c40d5cd93fa1e6a4"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/linux-headers"
OUT="${BUILD_DIR}/include"

log() { echo "[build-linux-headers] $*"; }
die() { echo "[build-linux-headers] ERROR: $*" >&2; exit 1; }

case "${VERIDIAN_ARCH}" in
    x86_64) KARCH=x86 ;;
    aarch64) KARCH=arm64 ;;
    riscv64) KARCH=riscv ;;
esac

tarball="${VERIDIAN_SOURCES}/linux-${LINUX_VERSION}.tar.xz"
if [[ ! -f "${tarball}" ]]; then
    log "Downloading Linux ${LINUX_VERSION}..."
    curl -fsSL -o "${tarball}.part" "${LINUX_URL}" || { rm -f "${tarball}.part"; die "download failed: ${LINUX_URL}"; }
    mv "${tarball}.part" "${tarball}"
fi
echo "${LINUX_SHA256}  ${tarball}" | sha256sum -c --quiet - || die "linux-${LINUX_VERSION}: checksum mismatch"

stamp="${LINUX_VERSION} ${KARCH}"
if [[ -f "${OUT}/linux/futex.h" && "$(cat "${BUILD_DIR}/.stamp" 2>/dev/null)" == "${stamp}" ]]; then
    log "Already installed: ${OUT}"
    exit 0
fi
log "Installing the Linux ${LINUX_VERSION} UAPI headers for ${KARCH}..."
rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"
tar -xf "${tarball}" -C "${BUILD_DIR}"
make -C "${BUILD_DIR}/linux-${LINUX_VERSION}" ARCH="${KARCH}" \
    INSTALL_HDR_PATH="${BUILD_DIR}" headers_install >/dev/null
# The source tree is only needed for that.
rm -rf "${BUILD_DIR}/linux-${LINUX_VERSION}"
[[ -f "${OUT}/linux/futex.h" && -f "${OUT}/asm/types.h" ]] || die "headers_install left no headers in ${OUT}"
echo "${stamp}" > "${BUILD_DIR}/.stamp"
log "Done: ${OUT}"

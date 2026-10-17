#!/usr/bin/env bash
# Build a BusyBox root filesystem for AArch64 or RISC-V with the musl cross
# toolchain (tools/cross/build-musl-arch.sh), as a BlockFS image.
#
# The x86_64 root (scripts/build-busybox-rootfs.sh) is built with the native
# libc; these architectures have only musl, so everything here is built
# with it:
#   - BusyBox 1.36.1 (defconfig, static), with its applet links;
#   - /sbin/init (userland/init/init.c);
#   - the runtime tests: audit_runtime_test, musl_runtime_test with
#     wx_needed_test, and the dynamically linked musl_dynamic_test with its
#     library, musl's loader and the shared C++ runtime;
#   - the BusyBox self-test (userland/tests/busybox_test.sh) and its data.
#
# Usage: scripts/build-musl-rootfs.sh aarch64|riscv64
# Output: target/rootfs-musl-<arch>/ and target/rootfs-musl-<arch>.img
# Env:    BLOCKFS_SIZE (MiB, default 256), JOBS

set -euo pipefail

arch="${1:-}"
case "${arch}" in
    aarch64 | riscv64) ;;
    *) echo "usage: $0 aarch64|riscv64" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
unset VERIDIAN_SYSROOT VERIDIAN_TOOLCHAIN VERIDIAN_TARGET VERIDIAN_CROSS_BUILD VERIDIAN_CC VERIDIAN_CXX
export VERIDIAN_ARCH="${arch}"
# shellcheck source=../tools/cross/veridian-paths.sh
source "${ROOT}/tools/cross/veridian-paths.sh"

BUSYBOX_VERSION="1.36.1"
BUSYBOX_URL="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"
# busybox.net's busybox-1.36.1.tar.bz2.sha256.
BUSYBOX_SHA256="b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314"

BUILD_DIR="${ROOT}/target/rootfs-musl-${arch}"
IMG="${ROOT}/target/rootfs-musl-${arch}.img"
BB_SRC="${VERIDIAN_CROSS_BUILD}/busybox/busybox-${BUSYBOX_VERSION}"
TESTS="${ROOT}/userland/tests"
CC="${VERIDIAN_CC}"
CXX="${VERIDIAN_CXX}"
STRIP="${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-strip"
JOBS="${JOBS:-$(nproc)}"

log() { echo "[build-musl-rootfs:${arch}] $*"; }
die() { echo "[build-musl-rootfs:${arch}] ERROR: $*" >&2; exit 1; }

[[ -x "${CC}" ]] || die "no toolchain at ${CC} (run tools/cross/build-musl-arch.sh ${arch})"

build_busybox() {
    local tarball="${VERIDIAN_SOURCES}/busybox-${BUSYBOX_VERSION}.tar.bz2"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading BusyBox ${BUSYBOX_VERSION}..."
        curl -fsSL -o "${tarball}.part" "${BUSYBOX_URL}" || { rm -f "${tarball}.part"; die "download failed"; }
        mv "${tarball}.part" "${tarball}"
    fi
    echo "${BUSYBOX_SHA256}  ${tarball}" | sha256sum -c --quiet - || die "BusyBox checksum mismatch"
    log "Building BusyBox ${BUSYBOX_VERSION}..."
    rm -rf "${BB_SRC}"
    mkdir -p "$(dirname "${BB_SRC}")"
    tar -xjf "${tarball}" -C "$(dirname "${BB_SRC}")"
    local patch
    for patch in "${ROOT}"/tools/busybox/patches/*.patch; do
        [[ -f "${patch}" ]] || continue
        patch -d "${BB_SRC}" -p1 --forward --quiet < "${patch}" || die "failed to apply ${patch}"
    done
    local make_args=(ARCH="${arch/aarch64/arm64}" CROSS_COMPILE="${VERIDIAN_TARGET}-")
    make -C "${BB_SRC}" "${make_args[@]}" defconfig >/dev/null
    # Static; no `tc` (it does not build against current kernel headers,
    # which dropped the CBQ definitions it uses, as distributions found).
    python3 -I - "${BB_SRC}/.config" <<'EOF'
import sys
path = sys.argv[1]
lines = open(path).read().splitlines()
set_to = {"CONFIG_STATIC": "y", "CONFIG_TC": None}
out = []
for line in lines:
    key = line.split("=")[0].replace("# ", "").split(" ")[0]
    if key in set_to:
        v = set_to.pop(key)
        out.append(f"{key}={v}" if v else f"# {key} is not set")
    else:
        out.append(line)
for key, v in set_to.items():
    out.append(f"{key}={v}" if v else f"# {key} is not set")
open(path, "w").write("\n".join(out) + "\n")
EOF
    # New options take their defaults (`yes` ends with SIGPIPE: not an error).
    { yes "" || true; } | make -C "${BB_SRC}" "${make_args[@]}" oldconfig >/dev/null
    make -C "${BB_SRC}" "${make_args[@]}" -j"${JOBS}" busybox >/dev/null
    make -C "${BB_SRC}" "${make_args[@]}" CONFIG_PREFIX="${BUILD_DIR}" install >/dev/null
}

build_programs() {
    log "Building init and the runtime tests..."
    mkdir -p "${BUILD_DIR}/sbin" "${BUILD_DIR}/lib" "${BUILD_DIR}/usr/lib" "${BUILD_DIR}/usr/src"
    rm -f "${BUILD_DIR}/sbin/init"
    "${CC}" -static-pie -O2 -Wall -Wextra -o "${BUILD_DIR}/sbin/init" "${ROOT}/userland/init/init.c"
    # A static PIE: the kernel picks its address (ADR 0010).
    "${CC}" -static-pie -O2 -Wall -Wextra -o "${BUILD_DIR}/bin/musl_runtime_test" \
        "${TESTS}/musl_runtime_test.c"
    # The audit suite uses <veridian/sysno.h> and <veridian/syscall.h>
    # (the native libc's, for all three architectures) and GNU extensions.
    "${CC}" -D_GNU_SOURCE -static-pie -O2 -Wall -Wextra -idirafter "${ROOT}/userland/libc/include" \
        -o "${BUILD_DIR}/bin/audit_runtime_test" "${TESTS}/audit_runtime_test.c"
    # The W^X opt-in (ADR 0017): a program carrying the VeridianOS note.
    "${CC}" -static-pie -O2 -Wall -Wextra -idirafter "${ROOT}/userland/libc/include" \
        -o "${BUILD_DIR}/bin/wx_needed_test" "${TESTS}/wx_needed_test.c"
    # Dynamic linking: musl's loader (a copy of libc.so, which is both; a
    # link would be resolved on the build host), the shared C++ runtime and
    # a C++ program that dlopens a C++ library.
    "${STRIP}" --strip-unneeded -o "${BUILD_DIR}${VERIDIAN_LDSO}" "${VERIDIAN_SYSROOT}/usr/lib/libc.so"
    "${STRIP}" --strip-unneeded -o "${BUILD_DIR}/usr/lib/libstdc++.so.6" \
        "$(readlink -f "${VERIDIAN_SYSROOT}/usr/lib/libstdc++.so.6")"
    "${STRIP}" --strip-unneeded -o "${BUILD_DIR}/usr/lib/libgcc_s.so.1" \
        "${VERIDIAN_SYSROOT}/usr/lib/libgcc_s.so.1"
    "${CXX}" -O2 -Wall -Wextra -fPIC -shared -o "${BUILD_DIR}/usr/lib/libveridian_dltest.so" \
        "${TESTS}/musl_dltest_lib.cpp"
    "${CXX}" -O2 -Wall -Wextra -pthread -o "${BUILD_DIR}/bin/musl_dynamic_test" \
        "${TESTS}/musl_dynamic_test.cpp"
    # The BusyBox self-test and its data (as the x86_64 root has them).
    cp "${TESTS}/busybox_test.sh" "${BUILD_DIR}/usr/src/busybox_test.sh"
    chmod 755 "${BUILD_DIR}/usr/src/busybox_test.sh"
    printf 'Hello from VeridianOS\nCAT_PASS\n' > "${BUILD_DIR}/usr/src/cat_test.txt"
    printf 'one two\nthree four\nfive\n' > "${BUILD_DIR}/usr/src/wc_test.txt"
    printf 'cherry\napple\nbanana\n' > "${BUILD_DIR}/usr/src/sort_test.txt"
    mkdir -p "${BUILD_DIR}"/{dev,proc,tmp,etc,var}
}

build_image() {
    local mkfs_dir="${ROOT}/tools/mkfs-blockfs"
    local mkfs="${mkfs_dir}/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs"
    (cd "${mkfs_dir}" && cargo build --release --quiet)
    [[ -x "${mkfs}" ]] || die "mkfs-blockfs not built at ${mkfs}"
    log "Creating the ${BLOCKFS_SIZE:-256} MiB BlockFS image..."
    rm -f "${IMG}"
    "${mkfs}" --output "${IMG}" --size "${BLOCKFS_SIZE:-256}" --populate "${BUILD_DIR}" >/dev/null
}

rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"
build_busybox
build_programs
build_image
log "Done: ${IMG}"

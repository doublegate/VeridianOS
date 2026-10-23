#!/usr/bin/env bash
# Build the compat root filesystem: the architecture's BusyBox rootfs plus
# three conformance suites, built with the musl cross toolchain
# (tools/cross/build-musl-arch.sh, or build-musl-toolchain.sh for x86_64):
#
#   /opt/ltp         LTP: the binaries runtest/syscalls runs, the helpers
#                    built beside them, its data files, and the Open POSIX
#                    Test Suite (all static)
#   /opt/libc-test   musl libc-test (static and dynamic tests, the
#                    libraries the dynamic ones load)
#   /usr/lib/compat  the runner (userland/tests/compat/run.sh), compat_exec,
#                    one test list per suite (tools/compat/compat.py), and a
#                    smoke list per suite: the tests
#                    tests/compat/expected-<arch>.tsv has as passing
#
# scripts/run-compat-tests.sh boots it and runs the suites.
#
# Usage: scripts/build-compat-rootfs.sh x86_64|aarch64|riscv64
# Needs: the base root, target/rootfs-busybox-build (x86_64:
#        scripts/build-busybox-rootfs.sh rootfs) or target/rootfs-musl-<arch>
#        (scripts/build-musl-rootfs.sh <arch>), and the musl toolchain.
# Output: target/rootfs-compat-<arch>/, target/rootfs-compat-<arch>.img and
#         target/compat/<arch>/build.tsv (tests judged at build time).
# Env:   JOBS; COMPAT_REBUILD=1 rebuilds the suites even when their stamp
#        matches. The suites are built in ${VERIDIAN_PREFIX}/compat-build/<arch>
#        (not /tmp, which systemd-tmpfiles ages out, nor target/, which
#        `cargo clean` deletes).

set -euo pipefail

arch="${1:-}"
case "${arch}" in
    x86_64 | aarch64 | riscv64) ;;
    *) echo "usage: $0 x86_64|aarch64|riscv64" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
unset VERIDIAN_SYSROOT VERIDIAN_TOOLCHAIN VERIDIAN_TARGET VERIDIAN_CROSS_BUILD VERIDIAN_CC VERIDIAN_CXX
export VERIDIAN_ARCH="${arch}"
# shellcheck source=../tools/cross/veridian-paths.sh
source "${ROOT}/tools/cross/veridian-paths.sh"

# LTP's release tarball and the checksum published beside it.
LTP_VERSION="20260930"
LTP_URL="https://github.com/linux-test-project/ltp/releases/download/${LTP_VERSION}/ltp-full-${LTP_VERSION}.tar.xz"
LTP_SHA256="b9837481134fa6eabce91c2fba57d96695f81360468f83e11e6982584ad4d29d"
# libc-test has no releases; a commit of the upstream repository.
LIBC_TEST_URL="https://repo.or.cz/libc-test.git"
LIBC_TEST_COMMIT="7b95dfa5f5d5ca4d949221e0228ccc290bacc14e"

# LTP sources that do not compile against musl, left out of the build (the
# tests they are are reported NOTBUILT with this reason). LTP's directory
# loops stop at the first failure even with make -k, so each is listed.
LTP_EXCLUDE=(
    "testcases/kernel/syscalls/fmtmsg/fmtmsg01.c	addseverity() is a glibc extension musl does not have"
    "testcases/kernel/syscalls/timer_create/timer_create01.c	uses glibc's private struct sigevent member _sigev_un"
    "testcases/kernel/syscalls/timer_create/timer_create03.c	uses glibc's private struct sigevent member _sigev_un"
)

WORK="${VERIDIAN_PREFIX}/compat-build/${arch}"
LTP_SRC="${WORK}/ltp-full-${LTP_VERSION}"
LTP_STAGE="${WORK}/stage"
LIBC_TEST="${WORK}/libc-test"
OUT="${ROOT}/target/rootfs-compat-${arch}"
IMG="${ROOT}/target/rootfs-compat-${arch}.img"
RESULTS="${ROOT}/target/compat/${arch}"
CC="${VERIDIAN_CC}"
STRIP="${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-strip"
JOBS="${JOBS:-$(nproc)}"
if [[ ${arch} == x86_64 ]]; then
    BASE="${ROOT}/target/rootfs-busybox-build"
    base_hint="scripts/build-busybox-rootfs.sh rootfs"
else
    BASE="${ROOT}/target/rootfs-musl-${arch}"
    base_hint="scripts/build-musl-rootfs.sh ${arch}"
fi

log() { echo "[build-compat-rootfs:${arch}] $*"; }
die() { echo "[build-compat-rootfs:${arch}] ERROR: $*" >&2; exit 1; }

[[ -x "${CC}" ]] || die "no musl toolchain at ${CC}"
[[ -d "${BASE}/bin" ]] || die "no base root at ${BASE} (run ${base_hint})"
mkdir -p "${WORK}" "${RESULTS}"
export PATH="${VERIDIAN_TOOLCHAIN}/bin:${PATH}"

fetch_ltp() {
    local tarball="${VERIDIAN_SOURCES}/ltp-full-${LTP_VERSION}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading LTP ${LTP_VERSION}..."
        local part
        part="$(mktemp "${tarball}.XXXXXX")"
        curl -fsSL -o "${part}" "${LTP_URL}" || { rm -f "${part}"; die "LTP download failed"; }
        echo "${LTP_SHA256}  ${part}" | sha256sum -c --quiet - || { rm -f "${part}"; die "LTP checksum mismatch in the download"; }
        chmod a+r "${part}"  # mktemp creates it 0600
        mv "${part}" "${tarball}"
    fi
    echo "${LTP_SHA256}  ${tarball}" | sha256sum -c --quiet - || { rm -f "${tarball}"; die "LTP checksum mismatch (removed ${tarball})"; }
    LTP_TARBALL="${tarball}"
}

fetch_libc_test() {
    local mirror="${VERIDIAN_SOURCES}/libc-test.git"
    if ! git --git-dir="${mirror}" cat-file -e "${LIBC_TEST_COMMIT}^{commit}" 2>/dev/null; then
        log "Fetching libc-test..."
        if [[ -d "${mirror}" ]]; then
            git --git-dir="${mirror}" fetch -q origin
        else
            local tmp
            tmp="$(mktemp -d "${mirror}.XXXXXX")"
            git clone -q --bare "${LIBC_TEST_URL}" "${tmp}" || { rm -rf "${tmp}"; die "libc-test clone failed"; }
            mv "${tmp}" "${mirror}" 2>/dev/null || rm -rf "${tmp}"  # another run may have won
        fi
        git --git-dir="${mirror}" cat-file -e "${LIBC_TEST_COMMIT}^{commit}" ||
            die "libc-test commit ${LIBC_TEST_COMMIT} is not upstream"
    fi
    LIBC_TEST_MIRROR="${mirror}"
}

# The suites are rebuilt only when what goes into them changes.
stamp_of() {
    {
        echo "ltp=${LTP_VERSION} libc-test=${LIBC_TEST_COMMIT} target=${VERIDIAN_TARGET}"
        "${CC}" -dumpfullversion
        sha256sum "${VERIDIAN_SYSROOT}/usr/lib/libc.a" | cut -d' ' -f1
        sha256sum "$0" | cut -d' ' -f1
    } | sha256sum | cut -d' ' -f1
}

build_ltp() {
    log "Building LTP ${LTP_VERSION} (about five minutes)..."
    rm -rf "${LTP_SRC}" "${LTP_STAGE}"
    tar -xJf "${LTP_TARBALL}" -C "${WORK}"
    local entry
    : > "${WORK}/ltp-excluded.tsv"
    for entry in "${LTP_EXCLUDE[@]}"; do
        rm -f "${LTP_SRC}/${entry%%	*}"
        printf '%s\n' "${entry}" >> "${WORK}/ltp-excluded.tsv"
    done
    mkdir -p "${WORK}/empty-pkgconfig"
    # Static binaries, so a test checks the kernel and not the loader. No
    # pkg-config and no libcrypto: the x86_64 sysroot also holds the KDE
    # stack's libraries, and every architecture should build the same tests.
    (
        cd "${LTP_SRC}"
        PKG_CONFIG_LIBDIR="${WORK}/empty-pkgconfig" ./configure \
            --host="${VERIDIAN_TARGET}" --prefix=/opt/ltp \
            --with-open-posix-testsuite --without-modules --without-tirpc \
            CFLAGS="-O2 -g0" LDFLAGS=-static ac_cv_lib_crypto_SHA1_Init=no
    ) > "${WORK}/ltp-configure.log" 2>&1 || die "LTP configure failed (${WORK}/ltp-configure.log)"
    make -C "${LTP_SRC}" -j"${JOBS}" > "${WORK}/ltp-make.log" 2>&1 ||
        { grep -E ' error: ' "${WORK}/ltp-make.log" | head -5 >&2; die "LTP build failed (${WORK}/ltp-make.log)"; }
    make -C "${LTP_SRC}" install DESTDIR="${LTP_STAGE}" > "${WORK}/ltp-install.log" 2>&1 ||
        die "LTP install failed (${WORK}/ltp-install.log)"
}

build_libc_test() {
    log "Building libc-test..."
    rm -rf "${LIBC_TEST}"
    mkdir -p "${LIBC_TEST}"
    git --git-dir="${LIBC_TEST_MIRROR}" archive "${LIBC_TEST_COMMIT}" | tar -x -C "${LIBC_TEST}"
    # RUN_WRAP=true builds every test and runs none (they run in the guest).
    printf 'include config.mak.def\nCC=%s\nRUN_WRAP=true\n' "${CC}" > "${LIBC_TEST}/config.mak"
    # -k: a test that does not build is NOTBUILT, not a failed build.
    make -C "${LIBC_TEST}" -k -j"${JOBS}" > "${WORK}/libc-test-make.log" 2>&1 || true
    [[ -x "${LIBC_TEST}/src/functional/string.exe" ]] || die "libc-test did not build (${WORK}/libc-test-make.log)"
}

install_stripped() { # src dst
    mkdir -p "$(dirname "$2")"
    if "${STRIP}" --strip-all -o "$2" "$1" 2>/dev/null; then
        chmod 755 "$2"
    else
        cp -p "$1" "$2"  # a script
    fi
}

compose_root() {
    log "Composing ${OUT}..."
    rm -rf "${OUT}"
    cp -a "${BASE}" "${OUT}"
    # The x86_64 kernel runs the BusyBox self-test at boot when it is there;
    # this image is for the suites.
    rm -f "${OUT}/usr/src/busybox_test.sh"
    mkdir -p "${OUT}/lib" "${OUT}/tmp" "${OUT}/dev/shm" "${OUT}/usr/lib/compat" "${OUT}/etc"
    # The users and groups LTP requires (its IDcheck.sh), with Linux's
    # usual IDs, when the base root has no account files of its own.
    [[ -f "${OUT}/etc/passwd" ]] || cat > "${OUT}/etc/passwd" <<'EOF'
root:x:0:0:root:/root:/bin/sh
bin:x:1:1:bin:/bin:/bin/false
daemon:x:2:2:daemon:/sbin:/bin/false
nobody:x:65534:65534:nobody:/:/bin/false
EOF
    [[ -f "${OUT}/etc/group" ]] || cat > "${OUT}/etc/group" <<'EOF'
root:x:0:
bin:x:1:
daemon:x:2:
sys:x:3:
users:x:100:
nobody:x:65534:
EOF
    # musl's loader (libc.so is both) for libc-test's dynamic tests.
    [[ -f "${OUT}${VERIDIAN_LDSO}" ]] ||
        "${STRIP}" --strip-unneeded -o "${OUT}${VERIDIAN_LDSO}" "${VERIDIAN_SYSROOT}/usr/lib/libc.so"

    # LTP: what runtest/syscalls runs, every program built in the syscalls
    # and library source directories (the tests' helpers and children), the
    # shell API commands its shell tests call (tools/apicmds: tst_resm,
    # tst_exit, ...), and the data files.
    local bin="${LTP_STAGE}/opt/ltp/testcases/bin" dst="${OUT}/opt/ltp/testcases/bin" name
    {
        grep -v '^#' "${LTP_STAGE}/opt/ltp/runtest/syscalls" | awk 'NF >= 2 { print $2 }'
        find "${LTP_SRC}/testcases/kernel/syscalls" "${LTP_SRC}/testcases/lib" "${LTP_SRC}/tools/apicmds" -type f -perm -u+x \
            ! -name '*.[cho]' ! -name 'Makefile*' -printf '%f\n'
    } | sort -u | while read -r name; do
        # `if`, not `&&`: under set -e and pipefail a last name that is not
        # an installed binary would otherwise end the build.
        if [[ -f "${bin}/${name}" ]]; then
            install_stripped "${bin}/${name}" "${dst}/${name}"
        fi
    done
    cp -a "${LTP_STAGE}/opt/ltp/testcases/data" "${OUT}/opt/ltp/testcases/"
    mkdir -p "${OUT}/opt/ltp/runtest"
    cp "${LTP_STAGE}/opt/ltp/runtest/syscalls" "${OUT}/opt/ltp/runtest/"
    # The Open POSIX Test Suite, without its stress tests.
    local posix="${LTP_STAGE}/opt/ltp/testcases/open_posix_testsuite" f
    while IFS= read -r -d '' f; do
        install_stripped "${f}" "${OUT}/opt/ltp/testcases/open_posix_testsuite/${f#"${posix}"/}"
    done < <(find "${posix}/conformance" "${posix}/functional" -type f \( -name '*.run-test' -o -name '*.sh' \) -print0)

    # libc-test: the test programs and the libraries they load, in the
    # layout they expect (they name their libraries src/<dir>/<x>.so).
    while IFS= read -r -d '' f; do
        install_stripped "${f}" "${OUT}/opt/libc-test/${f#"${LIBC_TEST}"/}"
    done < <(find "${LIBC_TEST}/src" -path "${LIBC_TEST}/src/common" -prune -o \
        -type f \( -name '*.exe' -o -name '*.so' \) -print0)

    # The runner, its helper and the lists.
    "${CC}" -static -O2 -Wall -Wextra -Werror -o "${OUT}/usr/lib/compat/compat_exec" \
        "${ROOT}/userland/tests/compat/compat_exec.c"
    install -m 755 "${ROOT}/userland/tests/compat/run.sh" "${OUT}/usr/lib/compat/run.sh"
    python3 -I "${ROOT}/tools/compat/compat.py" lists --arch "${arch}" \
        --ltp-src "${LTP_SRC}" --ltp-stage "${LTP_STAGE}/opt/ltp" \
        --make-log "${WORK}/ltp-make.log" --removed "${WORK}/ltp-excluded.tsv" \
        --libc-test "${LIBC_TEST}" --libc-test-log "${WORK}/libc-test-make.log" \
        --root "${OUT}" --out "${OUT}/usr/lib/compat" --build-tsv "${RESULTS}/build.tsv" \
        --expected "${ROOT}/tests/compat/expected-${arch}.tsv"
    # A copy in the image, for a run whose output directory has none.
    cp "${RESULTS}/build.tsv" "${OUT}/usr/lib/compat/build.tsv"
}

build_image() {
    local mkfs_dir="${ROOT}/tools/mkfs-blockfs"
    local mkfs="${mkfs_dir}/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs"
    (cd "${mkfs_dir}" && cargo build --release --quiet)
    [[ -x "${mkfs}" ]] || die "mkfs-blockfs not built at ${mkfs}"
    # Room for what the tests write, and inodes for every file plus slack.
    local used files size inodes
    used="$(du -sm "${OUT}" | cut -f1)"
    files="$(find "${OUT}" | wc -l)"
    size="${BLOCKFS_SIZE:-$((used + used / 4 + 128))}"
    inodes="$((files + files / 4 + 1024))"
    log "Creating the ${size} MiB BlockFS image (${files} files, ${inodes} inodes)..."
    rm -f "${IMG}"
    # /tmp as on Linux (mode 1777): mkfs-blockfs keeps host modes within 0755,
    # and tests that drop to nobody create their directories there.
    printf '/tmp/ 0 0 1777\n' > "${WORK}/attrs"
    "${mkfs}" --output "${IMG}" --size "${size}" --inodes "${inodes}" --populate "${OUT}" \
        --attrs "${WORK}/attrs" >/dev/null
}

fetch_ltp
fetch_libc_test
stamp="$(stamp_of)"
if [[ ${COMPAT_REBUILD:-0} == 1 || ! -f "${WORK}/stamp" || "$(cat "${WORK}/stamp")" != "${stamp}" ||
    ! -d "${LTP_SRC}" || ! -d "${LTP_STAGE}/opt/ltp" || ! -d "${LIBC_TEST}/src" ]]; then
    rm -f "${WORK}/stamp"
    build_ltp
    build_libc_test
    echo "${stamp}" > "${WORK}/stamp"
else
    log "LTP and libc-test are up to date (${WORK})"
fi
compose_root
build_image
log "Done: ${IMG}"

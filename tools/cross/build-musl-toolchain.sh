#!/usr/bin/env bash
# Build the musl cross toolchain the KDE pipeline compiles with
#
# binutils and GCC (C and C++) for x86_64-veridian-linux-musl, using the
# musl in ${VERIDIAN_SYSROOT} (build-musl.sh) as the target C library, so
# libstdc++, libgcc and libgcc_eh are built for musl. Every later phase
# compiles with this toolchain.
#
# It replaces wrappers around the host GCC that compiled against the
# host's glibc libstdc++ headers, linked the host's glibc-built
# libstdc++.a (and searched the host's /usr/lib), and emulated glibc
# internals (locale structures, ctype tables, fortify functions) in a shim
# so that library would link.
#
# Programs link dynamically by default, against musl's loader
# (/lib/ld-musl-x86_64.so.1) and the shared C and C++ runtimes, as on
# Linux (ADR 0010); -static and -static-pie still work. The runtime
# libraries (libgcc_s, libstdc++) are installed into the sysroot too, so
# the root filesystem can ship them.
#
# Versions and checksums are the native toolchain's
# (scripts/build-cross-toolchain.sh), so both use the same GCC.
#
# Output: ${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-{gcc,g++,ar,...}

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/toolchain"
SYSROOT="${VERIDIAN_SYSROOT}"
PREFIX="${VERIDIAN_TOOLCHAIN}"
TARGET="${VERIDIAN_TARGET}"
JOBS="${JOBS:-$(nproc)}"
NATIVE_SCRIPT="${PROJECT_ROOT}/scripts/build-cross-toolchain.sh"

log() { echo "[build-musl-toolchain] $*"; }
die() { echo "[build-musl-toolchain] ERROR: $*" >&2; exit 1; }

# A variable assigned on its own line in the native toolchain script.
native_var() {
    local value
    value="$(sed -n "s/^$1=\"\(.*\)\"$/\1/p" "${NATIVE_SCRIPT}")"
    [[ -n "${value}" ]] || die "cannot read $1 from ${NATIVE_SCRIPT}"
    echo "${value}"
}

BINUTILS_VERSION="$(native_var BINUTILS_VERSION)"
GCC_VERSION="$(native_var GCC_VERSION)"
GMP_VERSION="$(native_var GMP_VERSION)"
MPFR_VERSION="$(native_var MPFR_VERSION)"
MPC_VERSION="$(native_var MPC_VERSION)"

# name|url|sha256
sources() {
    cat <<EOF
binutils-${BINUTILS_VERSION}|https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz|$(native_var BINUTILS_SHA256)
gcc-${GCC_VERSION}|https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz|$(native_var GCC_SHA256)
gmp-${GMP_VERSION}|https://ftp.gnu.org/gnu/gmp/gmp-${GMP_VERSION}.tar.xz|$(native_var GMP_SHA256)
mpfr-${MPFR_VERSION}|https://ftp.gnu.org/gnu/mpfr/mpfr-${MPFR_VERSION}.tar.xz|$(native_var MPFR_SHA256)
mpc-${MPC_VERSION}|https://ftp.gnu.org/gnu/mpc/mpc-${MPC_VERSION}.tar.xz|$(native_var MPC_SHA256)
EOF
}

# What the installed toolchain was built from: the functions that build
# it, the source versions and the musl it was configured against
# (libstdc++'s configure reads the C library's headers). A different stamp
# rebuilds it; changes elsewhere in this script (the checks in verify) do
# not.
stamp() {
    {
        declare -f fetch_sources build_binutils build_gcc
        sources
        cat "${SYSROOT}/.musl-stamp" 2>/dev/null || die "musl is not built (run build-musl.sh)"
    } | sha256sum | cut -d' ' -f1
}

fetch_sources() {
    local name url sha tarball
    while IFS='|' read -r name url sha; do
        tarball="${VERIDIAN_SOURCES}/${name}.tar.xz"
        if [[ ! -f "${tarball}" ]]; then
            log "Downloading ${name}..."
            curl -fsSL -o "${tarball}.part" "${url}" && [[ -s "${tarball}.part" ]] \
                || { rm -f "${tarball}.part"; die "download failed: ${url}"; }
            mv "${tarball}.part" "${tarball}"
        fi
        echo "${sha}  ${tarball}" | sha256sum -c --quiet - || die "${name}: checksum mismatch"
        rm -rf "${BUILD_DIR:?}/${name}"
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    done < <(sources)
    # GMP, MPFR and MPC are built in-tree with GCC.
    local gcc_src="${BUILD_DIR}/gcc-${GCC_VERSION}"
    ln -sfn "../gmp-${GMP_VERSION}" "${gcc_src}/gmp"
    ln -sfn "../mpfr-${MPFR_VERSION}" "${gcc_src}/mpfr"
    ln -sfn "../mpc-${MPC_VERSION}" "${gcc_src}/mpc"
}

build_binutils() {
    local bld="${BUILD_DIR}/build-binutils"
    log "Building binutils ${BINUTILS_VERSION} for ${TARGET}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        "${BUILD_DIR}/binutils-${BINUTILS_VERSION}/configure" \
            --target="${TARGET}" \
            --prefix="${PREFIX}" \
            --with-sysroot="${SYSROOT}" \
            --disable-nls \
            --disable-werror \
            --disable-multilib \
            --disable-gprofng \
            --enable-deterministic-archives && \
        make -j"${JOBS}" && \
        make install)
}

build_gcc() {
    local bld="${BUILD_DIR}/build-gcc"
    log "Building GCC ${GCC_VERSION} (C, C++) for ${TARGET}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    # Shared and static target libraries (libstdc++, libgcc_s).
    # --enable-default-pie: programs are position-independent; a -static
    # program is still an ordinary static executable. musl has no IFUNC and
    # provides its own stack protector; libsanitizer does not support musl.
    # The target libraries are compiled -fPIC: default-PIE code uses
    # local-exec TLS, which a shared object cannot contain, so without it
    # libstdc++.a could not go into a shared object (-static-libstdc++); in
    # a static program the linker relaxes the PIC TLS sequences again.
    (cd "${bld}" && \
        PATH="${PREFIX}/bin:${PATH}" \
        "${BUILD_DIR}/gcc-${GCC_VERSION}/configure" \
            --target="${TARGET}" \
            --prefix="${PREFIX}" \
            --with-sysroot="${SYSROOT}" \
            --with-native-system-header-dir=/usr/include \
            --with-pkgversion="VeridianOS musl" \
            --enable-languages=c,c++ \
            --disable-bootstrap \
            --disable-multilib \
            --disable-nls \
            --disable-werror \
            --enable-shared \
            --enable-static \
            --enable-default-pie \
            --enable-tls \
            --enable-initfini-array \
            --enable-libstdcxx-time=rt \
            --disable-gnu-indirect-function \
            --disable-libsanitizer \
            --disable-libssp && \
        PATH="${PREFIX}/bin:${PATH}" make -j"${JOBS}" \
            CFLAGS_FOR_TARGET="-g -O2 -fPIC" CXXFLAGS_FOR_TARGET="-g -O2 -fPIC" && \
        PATH="${PREFIX}/bin:${PATH}" make install)
}

# The target's runtime libraries (libgcc_s, libstdc++ and the other GCC
# runtimes built for musl) into the sysroot, beside libc.so, for the root
# filesystem; the toolchain itself links them from its own directory.
install_runtime() {
    # Where GCC put them (lib64 for x86_64), as the compiler reports it.
    local lib
    lib="$(dirname "$("${PREFIX}/bin/${TARGET}-gcc" -print-file-name=libgcc_s.so.1)")"
    [[ -e "${lib}/libgcc_s.so.1" && -e "${lib}/libstdc++.so" ]] || die "no shared runtime in ${lib}"
    log "Installing the shared runtime libraries into the sysroot..."
    cp -a "${lib}"/*.so* "${SYSROOT}/usr/lib/"
    # libstdc++'s gdb pretty-printer script is not a library.
    rm -f "${SYSROOT}"/usr/lib/*.so*-gdb.py
}

# C and C++ programs (exceptions, iostream, threads, thread_local with a
# destructor) build against musl and nothing from the host: static ones run
# directly on the build host, dynamic ones through musl's loader there
# (the host has no /lib/ld-musl-x86_64.so.1), including a dlopen of a C++
# shared object.
verify() {
    log "Verifying the toolchain..."
    local dir="${BUILD_DIR}/verify"
    rm -rf "${dir}"
    mkdir -p "${dir}"
    cat > "${dir}/t.cpp" <<'CPP'
#include <dlfcn.h>
#include <iostream>
#include <stdexcept>
#include <string>
#include <thread>
struct D { ~D() { std::cout << "tls dtor\n"; } };
thread_local D d;
int main(int argc, char **argv) {
    try { throw std::runtime_error("ok"); }
    catch (const std::exception &e) { std::cout << e.what() << '\n'; }
    std::thread t([] { (void)d; });
    t.join();
    if (argc > 1) {
        void *h = dlopen(argv[1], RTLD_NOW);
        auto f = h ? reinterpret_cast<int (*)(int)>(dlsym(h, "f")) : nullptr;
        std::cout << (f ? f(12345) : -1) << '\n';
    }
    return std::to_string(42) == "42" ? 0 : 1;
}
CPP
    printf '#include <stdio.h>\nint main(void){puts("ok");return 0;}\n' > "${dir}/t.c"
    # A C++ shared object that throws and catches inside.
    printf '#include <stdexcept>\n#include <string>\nextern "C" int f(int x) { try { throw std::runtime_error(std::to_string(x)); } catch (const std::exception &e) { return (int)std::string(e.what()).size(); } }\n' > "${dir}/s.cpp"
    local gcc="${PREFIX}/bin/${TARGET}-gcc" gxx="${PREFIX}/bin/${TARGET}-g++"
    local flags=(-Wall -Wextra -Werror)
    "${gxx}" "${flags[@]}" -fPIC -shared "${dir}/s.cpp" -o "${dir}/libs.so" \
        || die "C++ shared object did not build"
    # The C++ runtime linked statically into a shared object (no libstdc++.so
    # needed where it is loaded).
    "${gxx}" "${flags[@]}" -fPIC -shared -static-libstdc++ -static-libgcc "${dir}/s.cpp" \
        -o "${dir}/libs-static-rt.so" || die "C++ shared object with a static runtime did not build"
    "${gcc}" "${flags[@]}" -static "${dir}/t.c" -o "${dir}/t-c-static" || die "static C program did not build"
    "${gxx}" "${flags[@]}" -static -pthread "${dir}/t.cpp" -o "${dir}/t-cpp-static" \
        || die "static C++ program did not build"
    "${gcc}" "${flags[@]}" "${dir}/t.c" -o "${dir}/t-c" || die "dynamic C program did not build"
    "${gxx}" "${flags[@]}" -pthread "${dir}/t.cpp" -o "${dir}/t-cpp" || die "dynamic C++ program did not build"

    local exe
    for exe in t-c-static t-cpp-static; do
        [[ "$(file "${dir}/${exe}")" == *"statically linked"* ]] || die "${exe} is not static"
    done
    for exe in t-c t-cpp; do
        [[ "$(file "${dir}/${exe}")" == *"interpreter /lib/ld-musl-x86_64.so.1"* ]] \
            || die "${exe} does not ask for musl's loader"
    done
    [[ "$("${dir}/t-c-static")" == "ok" ]] || die "static C program failed"
    [[ "$("${dir}/t-cpp-static")" == $'ok\ntls dtor' ]] || die "static C++ program failed"
    # musl's loader as a command: <loader> [--library-path P] <program> <args>.
    local ld=("${SYSROOT}/usr/lib/libc.so" --library-path "${SYSROOT}/usr/lib")
    [[ "$("${ld[@]}" "${dir}/t-c")" == "ok" ]] || die "dynamic C program failed"
    [[ "$("${ld[@]}" "${dir}/t-cpp" "${dir}/libs.so")" == $'ok\ntls dtor\n5' ]] \
        || die "dynamic C++ program (with dlopen) failed"
    [[ "$("${ld[@]}" "${dir}/t-cpp" "${dir}/libs-static-rt.so")" == $'ok\ntls dtor\n5' ]] \
        || die "dynamic C++ program (dlopen, static runtime) failed"
    log "Toolchain OK: ${PREFIX}/bin/${TARGET}-{gcc,g++}"
}

main() {
    log "=== musl cross toolchain (${TARGET}) ==="
    [[ -f "${SYSROOT}/usr/lib/libc.a" ]] || die "musl not found. Run build-musl.sh first."
    mkdir -p "${BUILD_DIR}"
    local want
    want="$(stamp)"
    if [[ -x "${PREFIX}/bin/${TARGET}-g++" && "$(cat "${PREFIX}/.veridian-stamp" 2>/dev/null)" == "${want}" ]]; then
        log "Already built."
    else
        rm -rf "${PREFIX}"
        fetch_sources
        build_binutils
        build_gcc
        echo "${want}" > "${PREFIX}/.veridian-stamp"
    fi
    install_runtime
    verify
    log "=== Toolchain ready ==="
}

main "$@"

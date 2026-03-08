#!/usr/bin/env bash
# Build musl libc for VeridianOS cross-compilation
#
# This script downloads, patches, and cross-compiles musl libc 1.2.5
# to produce a static libc.a and C headers in the sysroot.
#
# Prerequisites:
#   - GCC cross-compiler (x86_64-linux-musl or host gcc for static target)
#   - wget/curl for downloading source
#
# Output:
#   $SYSROOT/usr/lib/libc.a
#   $SYSROOT/usr/include/ (POSIX headers)
#   $SYSROOT/bin/x86_64-veridian-musl-gcc (wrapper script)

set -euo pipefail

MUSL_VERSION="1.2.5"
MUSL_URL="https://musl.libc.org/releases/musl-${MUSL_VERSION}.tar.gz"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/musl"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
PATCH_DIR="${SCRIPT_DIR}/musl-patches"
JOBS="${JOBS:-$(nproc)}"

# Host GCC for cross-compilation. If a musl cross-compiler is available,
# prefer it; otherwise fall back to the system gcc targeting x86_64.
CROSS_CC="${CROSS_CC:-gcc}"

log() { echo "[build-musl] $*"; }
die() { echo "[build-musl] ERROR: $*" >&2; exit 1; }

# ── Download ──────────────────────────────────────────────────────────
download_musl() {
    local tarball="${BUILD_DIR}/musl-${MUSL_VERSION}.tar.gz"
    if [[ -f "${tarball}" ]]; then
        log "Source tarball already downloaded."
        return 0
    fi
    mkdir -p "${BUILD_DIR}"
    log "Downloading musl ${MUSL_VERSION}..."
    if command -v wget &>/dev/null; then
        wget -q -O "${tarball}" "${MUSL_URL}"
    elif command -v curl &>/dev/null; then
        curl -fsSL -o "${tarball}" "${MUSL_URL}"
    else
        die "Need wget or curl to download musl."
    fi
}

# ── Extract ───────────────────────────────────────────────────────────
extract_musl() {
    local src="${BUILD_DIR}/musl-${MUSL_VERSION}"
    if [[ -d "${src}" ]]; then
        log "Source already extracted."
        return 0
    fi
    log "Extracting..."
    tar -xzf "${BUILD_DIR}/musl-${MUSL_VERSION}.tar.gz" -C "${BUILD_DIR}"
}

# ── Patch ─────────────────────────────────────────────────────────────
# Patch musl's x86_64 syscall_arch.h to remap Linux syscall numbers
# to VeridianOS equivalents.
patch_musl() {
    local src="${BUILD_DIR}/musl-${MUSL_VERSION}"
    local marker="${src}/.veridian_patched"
    if [[ -f "${marker}" ]]; then
        log "Already patched."
        return 0
    fi
    log "Applying VeridianOS syscall patches..."
    if [[ -d "${PATCH_DIR}" ]]; then
        for patch in "${PATCH_DIR}"/*.patch; do
            [[ -f "$patch" ]] || continue
            log "  Applying $(basename "$patch")..."
            (cd "${src}" && patch -p1 < "$patch")
        done
    fi

    # Generate syscall number remapping header.
    # musl uses Linux syscall numbers from arch/x86_64/bits/syscall.h.in.
    # We create an overlay that redefines the critical ones to VeridianOS numbers.
    cat > "${src}/arch/x86_64/bits/veridian_syscall_map.h" << 'HEADER'
/* VeridianOS syscall number remapping for musl libc.
 *
 * musl's internal __syscall() calls use Linux x86_64 numbers.
 * This header is included from syscall_arch.h to remap them
 * to VeridianOS equivalents at compile time.
 *
 * Only the ~60 syscalls actually used by musl are remapped.
 * Unmapped syscalls will return -ENOSYS at runtime.
 */
#ifndef _VERIDIAN_SYSCALL_MAP_H
#define _VERIDIAN_SYSCALL_MAP_H

/* Filesystem */
#define __VER_SYS_read       52
#define __VER_SYS_write      53
#define __VER_SYS_open       50
#define __VER_SYS_close      51
#define __VER_SYS_stat       150
#define __VER_SYS_fstat      55
#define __VER_SYS_lstat      151
#define __VER_SYS_lseek      54
#define __VER_SYS_dup        57
#define __VER_SYS_dup2       58
#define __VER_SYS_dup3       66
#define __VER_SYS_pipe2      65
#define __VER_SYS_fcntl      158
#define __VER_SYS_truncate   188
#define __VER_SYS_ftruncate  56
#define __VER_SYS_getcwd     110
#define __VER_SYS_chdir      111
#define __VER_SYS_mkdir      60
#define __VER_SYS_rmdir      61
#define __VER_SYS_unlink     157
#define __VER_SYS_rename     154
#define __VER_SYS_link       155
#define __VER_SYS_symlink    156
#define __VER_SYS_readlink   152
#define __VER_SYS_chmod      185
#define __VER_SYS_fchmod     186
#define __VER_SYS_chown      197
#define __VER_SYS_fchown     198
#define __VER_SYS_umask      187
#define __VER_SYS_access     153
#define __VER_SYS_openat     190
#define __VER_SYS_mkdirat    193
#define __VER_SYS_unlinkat   192
#define __VER_SYS_renameat   194
#define __VER_SYS_fstatat    191
#define __VER_SYS_readv      183
#define __VER_SYS_writev     184
#define __VER_SYS_pread64    195
#define __VER_SYS_pwrite64   196
#define __VER_SYS_fsync      73
#define __VER_SYS_ioctl      112

/* Memory */
#define __VER_SYS_mmap       20
#define __VER_SYS_munmap     21
#define __VER_SYS_mprotect   22
#define __VER_SYS_brk        23

/* Process */
#define __VER_SYS_exit       11
#define __VER_SYS_exit_group 11
#define __VER_SYS_fork       12
#define __VER_SYS_execve     13
#define __VER_SYS_wait4      14
#define __VER_SYS_getpid     15
#define __VER_SYS_getppid    16
#define __VER_SYS_kill       113
#define __VER_SYS_getuid     170
#define __VER_SYS_geteuid    171
#define __VER_SYS_getgid     172
#define __VER_SYS_getegid    173
#define __VER_SYS_setuid     174
#define __VER_SYS_setgid     175
#define __VER_SYS_setpgid    176
#define __VER_SYS_getpgid    177
#define __VER_SYS_getpgrp    178
#define __VER_SYS_setsid     179
#define __VER_SYS_getsid     180
#define __VER_SYS_uname      204

/* Signals */
#define __VER_SYS_rt_sigaction   120
#define __VER_SYS_rt_sigprocmask 121
#define __VER_SYS_rt_sigsuspend  122
#define __VER_SYS_rt_sigreturn   123

/* Threading */
#define __VER_SYS_clone      310
#define __VER_SYS_futex      311
#define __VER_SYS_gettid     43

/* Time */
#define __VER_SYS_clock_gettime  160
#define __VER_SYS_clock_getres   161
#define __VER_SYS_nanosleep      162
#define __VER_SYS_gettimeofday   163

/* Socket */
#define __VER_SYS_socket     220
#define __VER_SYS_bind       221
#define __VER_SYS_listen     222
#define __VER_SYS_connect    223
#define __VER_SYS_accept     224
#define __VER_SYS_accept4    224
#define __VER_SYS_sendto     250
#define __VER_SYS_recvfrom   251
#define __VER_SYS_sendmsg    338
#define __VER_SYS_recvmsg    339
#define __VER_SYS_socketpair 228
#define __VER_SYS_setsockopt 254
#define __VER_SYS_getsockopt 255
#define __VER_SYS_getsockname 252
#define __VER_SYS_getpeername 253

/* I/O multiplexing */
#define __VER_SYS_poll       300
#define __VER_SYS_select     200
#define __VER_SYS_epoll_create1 262
#define __VER_SYS_epoll_ctl  263
#define __VER_SYS_epoll_wait 264

/* Event notification */
#define __VER_SYS_eventfd2   331
#define __VER_SYS_timerfd_create  334
#define __VER_SYS_timerfd_settime 335
#define __VER_SYS_timerfd_gettime 336
#define __VER_SYS_signalfd4  337
#define __VER_SYS_getrandom  330

/* PTY */
#define __VER_SYS_ioctl      112

/* Shared memory */
#define __VER_SYS_shmget     210
#define __VER_SYS_shmat      210

/* Misc */
#define __VER_SYS_arch_prctl     203
#define __VER_SYS_getrlimit      260
#define __VER_SYS_setrlimit      261
#define __VER_SYS_mknod          199
#define __VER_SYS_ptrace         140
#define __VER_SYS_set_tid_address 352
#define __VER_SYS_set_robust_list 353
#define __VER_SYS_madvise        345
#define __VER_SYS_getdents64     340
#define __VER_SYS_prlimit64      341
#define __VER_SYS_inotify_init1  342
#define __VER_SYS_memfd_create   351
#define __VER_SYS_fchmodat       346
#define __VER_SYS_fchownat       347
#define __VER_SYS_linkat         348
#define __VER_SYS_symlinkat      349
#define __VER_SYS_readlinkat     350
#define __VER_SYS_shutdown       227

#endif /* _VERIDIAN_SYSCALL_MAP_H */
HEADER

    touch "${marker}"
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
    (cd "${build}" && \
        "${src}/configure" \
            --prefix="${SYSROOT}/usr" \
            --syslibdir="${SYSROOT}/usr/lib" \
            --disable-shared \
            --enable-static \
            CC="${CROSS_CC}" \
            CFLAGS="-O2 -fPIC -DVERIDIAN_OS=1" \
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
    make -C "${build}" install

    # Create the musl-gcc wrapper script
    create_wrapper
}

# ── Wrapper Scripts ───────────────────────────────────────────────────
create_wrapper() {
    local gcc_wrapper="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
    local gxx_wrapper="${SYSROOT}/bin/x86_64-veridian-musl-g++"
    local compat_dir="${SCRIPT_DIR}/musl-compat"
    mkdir -p "${SYSROOT}/bin"

    # Detect GCC version and internal include path
    local gcc_ver
    gcc_ver=$(${CROSS_CC} -dumpversion 2>/dev/null || echo "15.2.1")
    local gcc_dir="/usr/lib/gcc/x86_64-pc-linux-gnu/${gcc_ver}"
    if [[ ! -d "${gcc_dir}" ]]; then
        # Try common alternative paths
        gcc_dir=$(${CROSS_CC} -print-search-dirs 2>/dev/null | grep install | awk '{print $2}' || echo "/usr/lib/gcc/x86_64-pc-linux-gnu/${gcc_ver}")
    fi

    # ── C wrapper ────────────────────────────────────────────────
    cat > "${gcc_wrapper}" << 'WRAPPER'
#!/usr/bin/env bash
# musl-gcc wrapper for VeridianOS cross-compilation
SYSROOT="$(cd "$(dirname "$0")/.." && pwd)"
GCC_DIR="PLACEHOLDER_GCC_DIR"

COMPILE_ONLY=0
SHARED=0
for arg in "$@"; do
    case "$arg" in
        -c|-S|-E) COMPILE_ONLY=1 ;;
        -shared) SHARED=1 ;;
    esac
done

if [[ $COMPILE_ONLY -eq 1 ]]; then
    exec gcc --sysroot="${SYSROOT}" -nostdinc \
        -isystem "${GCC_DIR}/include" \
        -isystem "${SYSROOT}/usr/include" \
        -static "$@"
elif [[ $SHARED -eq 1 ]]; then
    exec gcc --sysroot="${SYSROOT}" -nostdinc \
        -isystem "${GCC_DIR}/include" \
        -isystem "${SYSROOT}/usr/include" \
        -L"${SYSROOT}/usr/lib" -nostdlib "$@" -lc
else
    exec gcc --sysroot="${SYSROOT}" -nostdinc \
        -isystem "${GCC_DIR}/include" \
        -isystem "${SYSROOT}/usr/include" \
        -L"${SYSROOT}/usr/lib" -static -nostdlib \
        "${SYSROOT}/usr/lib/crt1.o" "${SYSROOT}/usr/lib/crti.o" \
        "$@" -lc "${SYSROOT}/usr/lib/crtn.o"
fi
WRAPPER
    sed -i "s|PLACEHOLDER_GCC_DIR|${gcc_dir}|" "${gcc_wrapper}"
    chmod +x "${gcc_wrapper}"

    # ── C++ wrapper ──────────────────────────────────────────────
    cat > "${gxx_wrapper}" << 'WRAPPER'
#!/usr/bin/env bash
# musl-g++ wrapper for VeridianOS cross-compilation
# Uses system GCC's libstdc++ with musl libc + glibc shim for missing symbols
GCC_DIR="PLACEHOLDER_GCC_DIR"
GCC_VER="PLACEHOLDER_GCC_VER"
SYSROOT="$(cd "$(dirname "$0")/.." && pwd)"

COMPILE_ONLY=0
SHARED=0
for arg in "$@"; do
    case "$arg" in
        -c|-S|-E) COMPILE_ONLY=1 ;;
        -shared) SHARED=1 ;;
    esac
done

COMMON_FLAGS=(
    -nostdinc
    -include "${SYSROOT}/usr/include/compat/glibc_compat.h"
    -isystem /usr/include/c++/${GCC_VER}
    -isystem /usr/include/c++/${GCC_VER}/x86_64-pc-linux-gnu
    -isystem ${GCC_DIR}/include
    -isystem ${GCC_DIR}/include-fixed
    -isystem "${SYSROOT}/usr/include"
)

if [[ $COMPILE_ONLY -eq 1 ]]; then
    exec g++ "${COMMON_FLAGS[@]}" "$@"
elif [[ $SHARED -eq 1 ]]; then
    exec g++ "${COMMON_FLAGS[@]}" -nostdlib \
        -L"${SYSROOT}/usr/lib" -L/usr/lib "$@" \
        -lstdc++ -lglibc_shim -lc -lpthread -lgcc -lgcc_eh \
        -lpthread -lc -lglibc_shim
else
    exec g++ "${COMMON_FLAGS[@]}" -static -nostdlib \
        "${SYSROOT}/usr/lib/crt1.o" "${SYSROOT}/usr/lib/crti.o" \
        -L"${SYSROOT}/usr/lib" -L/usr/lib "$@" \
        -lstdc++ -lglibc_shim -lc -lpthread -lgcc -lgcc_eh \
        -lpthread -lc -lglibc_shim "${SYSROOT}/usr/lib/crtn.o"
fi
WRAPPER
    sed -i "s|PLACEHOLDER_GCC_DIR|${gcc_dir}|" "${gxx_wrapper}"
    sed -i "s|PLACEHOLDER_GCC_VER|${gcc_ver}|" "${gxx_wrapper}"
    chmod +x "${gxx_wrapper}"

    # ── Install glibc compat files ───────────────────────────────
    if [[ -d "${compat_dir}" ]]; then
        mkdir -p "${SYSROOT}/usr/include/compat"
        cp "${compat_dir}/glibc_compat.h" "${SYSROOT}/usr/include/compat/"
        cp "${compat_dir}/glibc_shim.c" "${SYSROOT}/usr/lib/"

        # Compile the glibc shim
        "${gcc_wrapper}" -c -O2 -fPIC \
            -o "${SYSROOT}/usr/lib/glibc_shim.o" \
            "${SYSROOT}/usr/lib/glibc_shim.c"
        ar rcs "${SYSROOT}/usr/lib/libglibc_shim.a" \
            "${SYSROOT}/usr/lib/glibc_shim.o"
        log "Installed glibc compat shim."
    fi

    # Also create convenience symlinks for common tools
    for tool in ar ranlib strip objdump; do
        if command -v "x86_64-linux-musl-${tool}" &>/dev/null; then
            ln -sf "$(command -v "x86_64-linux-musl-${tool}")" \
                "${SYSROOT}/bin/x86_64-veridian-${tool}"
        elif command -v "${tool}" &>/dev/null; then
            ln -sf "$(command -v "${tool}")" \
                "${SYSROOT}/bin/x86_64-veridian-${tool}"
        fi
    done

    log "Wrapper scripts: ${gcc_wrapper}, ${gxx_wrapper}"
}

# ── Verify ────────────────────────────────────────────────────────────
verify_install() {
    log "Verifying installation..."
    local errors=0
    for f in \
        "${SYSROOT}/usr/lib/libc.a" \
        "${SYSROOT}/usr/include/stdio.h" \
        "${SYSROOT}/usr/include/stdlib.h" \
        "${SYSROOT}/usr/include/unistd.h" \
        "${SYSROOT}/usr/include/pthread.h" \
        "${SYSROOT}/usr/include/sys/socket.h" \
        "${SYSROOT}/usr/include/sys/epoll.h" \
        "${SYSROOT}/bin/x86_64-veridian-musl-gcc" \
    ; do
        if [[ ! -f "$f" ]]; then
            log "  MISSING: $f"
            errors=$((errors + 1))
        fi
    done

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
    verify_install

    log "=== musl libc build complete ==="
}

main "$@"

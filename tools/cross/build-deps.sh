#!/usr/bin/env bash
# Build C library dependencies for KDE on VeridianOS
#
# Build order (dependency chain):
#   zlib -> libffi -> pcre2 -> libxml2 -> libjpeg-turbo -> libpng -> libxkbcommon
#
# All libraries are built as static archives (.a) into the sysroot.
#
# Prerequisites:
#   - musl libc built (run build-musl.sh first)
#   - cmake, meson, ninja for some libraries

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/deps"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
TOOLCHAIN="${SCRIPT_DIR}/cmake-toolchain-veridian.cmake"
MESON_CROSS="${SCRIPT_DIR}/meson-cross-veridian.txt"
JOBS="${JOBS:-$(nproc)}"
CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
# For autotools --host, we need a "cross compiler" that autotools can find.
# Create temporary symlinks so configure can find x86_64-veridian-gcc etc.
CROSS_BIN="${BUILD_DIR}/.cross-bin"

# Library versions
ZLIB_VER="1.3.1"
LIBFFI_VER="3.4.6"
PCRE2_VER="10.43"
LIBXML2_VER="2.12.6"
LIBJPEG_VER="3.0.3"
LIBPNG_VER="1.6.43"
XKBCOMMON_VER="1.7.0"
EXPAT_VER="2.6.2"
SQLITE_VER="3490100"  # 3.49.1
OPENSSL_VER="3.3.2"
LIBEVDEV_VER="1.13.3"
LIBINPUT_VER="1.26.2"
ATSPI_VER="2.52.0"

log() { echo "[build-deps] $*"; }
die() { echo "[build-deps] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

# ── Helper: download + extract ────────────────────────────────────────
fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${BUILD_DIR}/${name}.tar.gz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        curl -fsSL -o "${tarball}" -L "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

# ── Cross-compilation setup ───────────────────────────────────────────
# Autotools --host requires a triplet-prefixed CC.  Create symlinks so
# configure can discover x86_64-veridian-{gcc,ar,ranlib,strip}.
setup_cross_symlinks() {
    mkdir -p "${CROSS_BIN}"
    ln -sf "${CC}" "${CROSS_BIN}/x86_64-unknown-linux-musl-gcc"
    for tool in ar ranlib strip objcopy objdump; do
        ln -sf "$(command -v ${tool})" "${CROSS_BIN}/x86_64-unknown-linux-musl-${tool}"
    done
    export PATH="${CROSS_BIN}:${PATH}"

    # Generate meson cross file with resolved sysroot path
    MESON_CROSS="${BUILD_DIR}/meson-cross-veridian.txt"
    sed "s|@SYSROOT@|${SYSROOT}|g" "${SCRIPT_DIR}/meson-cross-veridian.txt" > "${MESON_CROSS}"
}

# Use linux-musl triplet for autotools' config.sub validation.
# VeridianOS uses the Linux x86_64 ABI with musl libc, so this is accurate.
COMMON_CONFIGURE=(
    --host=x86_64-unknown-linux-musl
    --prefix="${SYSROOT}/usr"
    --enable-static
    --disable-shared
)

export CC
export CFLAGS="-O2 -fPIC"
export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

# ── 1. zlib ───────────────────────────────────────────────────────────
build_zlib() {
    if [[ -f "${SYSROOT}/usr/lib/libz.a" ]]; then
        log "zlib: already installed."
        return 0
    fi
    fetch "zlib-${ZLIB_VER}" \
        "https://github.com/madler/zlib/releases/download/v${ZLIB_VER}/zlib-${ZLIB_VER}.tar.gz" \
        "zlib-${ZLIB_VER}"

    local src="${BUILD_DIR}/zlib-${ZLIB_VER}"
    log "Building zlib ${ZLIB_VER}..."
    # zlib's configure is not autotools; it uses CC directly.
    (cd "${src}" && \
        CC="${CC}" \
        AR="ar" \
        RANLIB="ranlib" \
        ./configure --static --prefix="${SYSROOT}/usr" && \
        make -j"${JOBS}" && \
        make install)
    log "zlib: done."
}

# ── 2. libffi ─────────────────────────────────────────────────────────
build_libffi() {
    if [[ -f "${SYSROOT}/usr/lib/libffi.a" ]]; then
        log "libffi: already installed."
        return 0
    fi
    fetch "libffi-${LIBFFI_VER}" \
        "https://github.com/libffi/libffi/releases/download/v${LIBFFI_VER}/libffi-${LIBFFI_VER}.tar.gz" \
        "libffi-${LIBFFI_VER}"

    local src="${BUILD_DIR}/libffi-${LIBFFI_VER}"
    log "Building libffi ${LIBFFI_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" && \
        make -j"${JOBS}" && \
        make install)
    log "libffi: done."
}

# ── 3. pcre2 ──────────────────────────────────────────────────────────
build_pcre2() {
    if [[ -f "${SYSROOT}/usr/lib/libpcre2-8.a" ]]; then
        log "pcre2: already installed."
        return 0
    fi
    fetch "pcre2-${PCRE2_VER}" \
        "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-${PCRE2_VER}/pcre2-${PCRE2_VER}.tar.gz" \
        "pcre2-${PCRE2_VER}"

    local src="${BUILD_DIR}/pcre2-${PCRE2_VER}"
    log "Building pcre2 ${PCRE2_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --enable-unicode \
            --enable-pcre2-8 \
            --disable-pcre2-16 \
            --disable-pcre2-32 && \
        make -j"${JOBS}" && \
        make install)
    log "pcre2: done."
}

# ── 4. expat (needed by D-Bus and Fontconfig) ─────────────────────────
build_expat() {
    if [[ -f "${SYSROOT}/usr/lib/libexpat.a" ]]; then
        log "expat: already installed."
        return 0
    fi
    fetch "expat-${EXPAT_VER}" \
        "https://github.com/libexpat/libexpat/releases/download/R_${EXPAT_VER//./_}/expat-${EXPAT_VER}.tar.gz" \
        "expat-${EXPAT_VER}"

    local src="${BUILD_DIR}/expat-${EXPAT_VER}"
    log "Building expat ${EXPAT_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --without-docbook && \
        make -j"${JOBS}" && \
        make install)
    log "expat: done."
}

# ── 5. libxml2 (needs zlib) ──────────────────────────────────────────
build_libxml2() {
    if [[ -f "${SYSROOT}/usr/lib/libxml2.a" ]]; then
        log "libxml2: already installed."
        return 0
    fi
    fetch "libxml2-${LIBXML2_VER}" \
        "https://download.gnome.org/sources/libxml2/2.12/libxml2-${LIBXML2_VER}.tar.xz" \
        "libxml2-${LIBXML2_VER}"

    local src="${BUILD_DIR}/libxml2-${LIBXML2_VER}"
    log "Building libxml2 ${LIBXML2_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --without-python \
            --without-lzma \
            --without-icu \
            --with-zlib="${SYSROOT}/usr" && \
        make -j"${JOBS}" && \
        make install)
    log "libxml2: done."
}

# ── 6. libjpeg-turbo (cmake) ─────────────────────────────────────────
build_libjpeg() {
    if [[ -f "${SYSROOT}/usr/lib/libjpeg.a" ]]; then
        log "libjpeg-turbo: already installed."
        return 0
    fi
    fetch "libjpeg-turbo-${LIBJPEG_VER}" \
        "https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/${LIBJPEG_VER}/libjpeg-turbo-${LIBJPEG_VER}.tar.gz" \
        "libjpeg-turbo-${LIBJPEG_VER}"

    local src="${BUILD_DIR}/libjpeg-turbo-${LIBJPEG_VER}"
    local bld="${BUILD_DIR}/libjpeg-build"
    log "Building libjpeg-turbo ${LIBJPEG_VER}..."
    mkdir -p "${bld}"
    (cd "${bld}" && \
        cmake "${src}" \
            -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
            -DCMAKE_INSTALL_PREFIX="${SYSROOT}/usr" \
            -DENABLE_SHARED=OFF \
            -DENABLE_STATIC=ON \
            -DWITH_TURBOJPEG=OFF && \
        make -j"${JOBS}" && \
        make install)
    log "libjpeg-turbo: done."
}

# ── 7. libpng (needs zlib) ───────────────────────────────────────────
build_libpng() {
    if [[ -f "${SYSROOT}/usr/lib/libpng.a" ]] || [[ -f "${SYSROOT}/usr/lib/libpng16.a" ]]; then
        log "libpng: already installed."
        return 0
    fi
    fetch "libpng-${LIBPNG_VER}" \
        "https://download.sourceforge.net/libpng/libpng-${LIBPNG_VER}.tar.gz" \
        "libpng-${LIBPNG_VER}"

    local src="${BUILD_DIR}/libpng-${LIBPNG_VER}"
    log "Building libpng ${LIBPNG_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            LDFLAGS="-L${SYSROOT}/usr/lib" \
            CPPFLAGS="-I${SYSROOT}/usr/include" && \
        make -j"${JOBS}" && \
        make install)
    log "libpng: done."
}

# ── 8. libxkbcommon (meson, needs libxml2) ────────────────────────────
build_xkbcommon() {
    if [[ -f "${SYSROOT}/usr/lib/libxkbcommon.a" ]]; then
        log "libxkbcommon: already installed."
        return 0
    fi
    fetch "libxkbcommon-${XKBCOMMON_VER}" \
        "https://xkbcommon.org/download/libxkbcommon-${XKBCOMMON_VER}.tar.xz" \
        "libxkbcommon-${XKBCOMMON_VER}"

    local src="${BUILD_DIR}/libxkbcommon-${XKBCOMMON_VER}"
    local bld="${BUILD_DIR}/xkbcommon-build"
    log "Building libxkbcommon ${XKBCOMMON_VER}..."
    mkdir -p "${bld}"
    # Remove old build dir if present (meson can't reconfigure in-place)
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Denable-wayland=false \
            -Denable-x11=false \
            -Denable-docs=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "libxkbcommon: done."
}

# ── 8. SQLite ─────────────────────────────────────────────────────────
build_sqlite() {
    if [[ -f "${SYSROOT}/usr/lib/libsqlite3.a" ]]; then
        log "SQLite: already installed."
        return 0
    fi
    local url="https://www.sqlite.org/2025/sqlite-autoconf-${SQLITE_VER}.tar.gz"
    local tarball="${BUILD_DIR}/sqlite-${SQLITE_VER}.tar.gz"
    local dir="sqlite-autoconf-${SQLITE_VER}"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading SQLite..."
        curl -fsSL -o "${tarball}" "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting SQLite..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
    local src="${BUILD_DIR}/${dir}"
    log "Building SQLite..."
    (cd "${src}" && \
        CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc" \
        CFLAGS="-O2 -fPIC -DSQLITE_THREADSAFE=1" \
        ./configure \
            --host=x86_64-unknown-linux-musl \
            --prefix="${SYSROOT}/usr" \
            --enable-static \
            --disable-shared \
            --disable-readline && \
        make -j"${JOBS}" && \
        make install)
    log "SQLite: done."
}

# ── 9. OpenSSL (static, for Qt6 TLS) ─────────────────────────────────
build_openssl() {
    if [[ -f "${SYSROOT}/usr/lib/libssl.a" ]]; then
        log "OpenSSL: already installed."
        return 0
    fi
    local url="https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VER}/openssl-${OPENSSL_VER}.tar.gz"
    local tarball="${BUILD_DIR}/openssl-${OPENSSL_VER}.tar.gz"
    local dir="openssl-${OPENSSL_VER}"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading OpenSSL ${OPENSSL_VER}..."
        curl -fsSL -o "${tarball}" -L "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting OpenSSL..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
    local src="${BUILD_DIR}/${dir}"
    log "Building OpenSSL ${OPENSSL_VER}..."
    (cd "${src}" && \
        CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc" \
        AR="ar" \
        RANLIB="ranlib" \
        ./Configure linux-x86_64 \
            --prefix="${SYSROOT}/usr" \
            --openssldir="${SYSROOT}/etc/ssl" \
            --cross-compile-prefix= \
            no-shared \
            no-async \
            no-engine \
            no-tests \
            -fPIC && \
        make -j"${JOBS}" && \
        make install_sw)
    log "OpenSSL: done."
}

# ── 10. libevdev (meson, needed by libinput) ─────────────────────────
build_libevdev() {
    if [[ -f "${SYSROOT}/usr/lib/libevdev.a" ]]; then
        log "libevdev: already installed."
        return 0
    fi
    local url="https://freedesktop.org/software/libevdev/libevdev-${LIBEVDEV_VER}.tar.xz"
    local tarball="${BUILD_DIR}/libevdev-${LIBEVDEV_VER}.tar.xz"
    local dir="libevdev-${LIBEVDEV_VER}"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading libevdev ${LIBEVDEV_VER}..."
        curl -fsSL -o "${tarball}" -L "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting libevdev..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
    local src="${BUILD_DIR}/${dir}"
    local bld="${BUILD_DIR}/libevdev-build"
    log "Building libevdev ${LIBEVDEV_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dtests=disabled \
            -Ddocumentation=disabled && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "libevdev: done."
}

# ── 11. libinput (meson, needs libevdev) ─────────────────────────────
build_libinput() {
    if [[ -f "${SYSROOT}/usr/lib/libinput.a" ]]; then
        log "libinput: already installed."
        return 0
    fi
    local url="https://gitlab.freedesktop.org/libinput/libinput/-/archive/${LIBINPUT_VER}/libinput-${LIBINPUT_VER}.tar.gz"
    local tarball="${BUILD_DIR}/libinput-${LIBINPUT_VER}.tar.gz"
    local dir="libinput-${LIBINPUT_VER}"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading libinput ${LIBINPUT_VER}..."
        curl -fsSL -o "${tarball}" -L "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting libinput..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
    local src="${BUILD_DIR}/${dir}"
    local bld="${BUILD_DIR}/libinput-build"
    log "Building libinput ${LIBINPUT_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dlibwacom=false \
            -Ddebug-gui=false \
            -Dtests=false \
            -Ddocumentation=false \
            -Dzshcompletiondir=no && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "libinput: done."
}

# ── 12. at-spi2-core (meson, for Qt6 accessibility) ─────────────────
build_atspi() {
    if [[ -f "${SYSROOT}/usr/lib/libatspi.a" ]]; then
        log "at-spi2-core: already installed."
        return 0
    fi
    local url="https://download.gnome.org/sources/at-spi2-core/${ATSPI_VER%.*}/at-spi2-core-${ATSPI_VER}.tar.xz"
    local tarball="${BUILD_DIR}/at-spi2-core-${ATSPI_VER}.tar.xz"
    local dir="at-spi2-core-${ATSPI_VER}"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading at-spi2-core ${ATSPI_VER}..."
        curl -fsSL -o "${tarball}" -L "${url}" || wget -q -O "${tarball}" "${url}"
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting at-spi2-core..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
    local src="${BUILD_DIR}/${dir}"
    local bld="${BUILD_DIR}/atspi-build"
    log "Building at-spi2-core ${ATSPI_VER}..."
    rm -rf "${bld}"
    mkdir -p "${bld}"
    (cd "${bld}" && \
        meson setup "${src}" \
            --cross-file="${MESON_CROSS}" \
            --prefix="${SYSROOT}/usr" \
            --default-library=static \
            -Dintrospection=disabled \
            -Dx11=disabled \
            -Dsystemd=disabled \
            -Ddocs=false && \
        ninja -j"${JOBS}" && \
        ninja install)
    log "at-spi2-core: done."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying all dependencies..."
    local errors=0
    for lib in libz.a libffi.a libpcre2-8.a libexpat.a libxml2.a libjpeg.a libpng16.a libxkbcommon.a libsqlite3.a libssl.a libcrypto.a libevdev.a libinput.a libatspi.a; do
        if [[ -f "${SYSROOT}/usr/lib/${lib}" ]]; then
            local size
            size=$(stat -c%s "${SYSROOT}/usr/lib/${lib}" 2>/dev/null || echo "?")
            log "  OK: ${lib} (${size} bytes)"
        else
            log "  MISSING: ${lib}"
            errors=$((errors + 1))
        fi
    done
    if [[ $errors -gt 0 ]]; then
        die "${errors} libraries missing!"
    fi
    log "All dependencies installed."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building C dependencies for VeridianOS KDE ==="
    log "Sysroot: ${SYSROOT}"

    setup_cross_symlinks

    build_zlib
    build_libffi
    build_pcre2
    build_expat
    build_libxml2
    build_libjpeg
    build_libpng
    build_xkbcommon
    build_sqlite
    build_openssl
    build_libevdev
    build_libinput
    build_atspi

    verify

    log "=== All dependencies built ==="
}

main "$@"

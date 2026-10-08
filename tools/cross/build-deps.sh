#!/usr/bin/env bash
# Build C library dependencies for KDE on VeridianOS
#
# Built and staged as lib/cross-env.sh describes: for /usr on the target,
# so the paths a library reads at run time (xkbcommon's keymaps, libinput's
# quirks, OpenSSL's certificates, ALSA's configuration) are target paths.
#
# All libraries are shared (ADR 0010). Every source tarball is checked
# against the SHA-256 below: from the project's signature (GnuPG, ALSA,
# xkeyboard-config), the project's published checksum, a distribution's
# package checksum, or pinned when first downloaded.
#
# Prerequisites: build-musl.sh and build-musl-toolchain.sh; cmake, meson,
# ninja, nasm, python3, xsltproc and msgfmt on the host.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/deps"
JOBS="${JOBS:-$(nproc)}"

ZLIB_VER="1.3.2"
ZLIB_SHA256="bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16"
# Compression (KArchive's .bz2/.xz/.zst, Qt's zstd): bzip2 signed by Mark
# Wielaard (12768A96795990107A0D2FDFFC57E3CCACD99A78), xz by Lasse Collin
# (3690C240CE51B4670D30AD1C38EE757D69184620), zstd's published checksum.
BZIP2_VER="1.0.8"
BZIP2_SHA256="ab5a03176ee106d3f0fa90e381da478ddae405918153cca248e682cd0c4a2269"
XZ_VER="5.8.4"
XZ_SHA256="4ce24038fd4221e0d13bc1a2de7a4db56e90b92b3bf75321f6c14be73f65de4b"
ZSTD_VER="1.5.7"
ZSTD_SHA256="eb33e51f49a15e023950cd7825ca74a4a2b43db8354825ac24fc1b7ee09e6fa3"
LIBFFI_VER="3.8.0"
LIBFFI_SHA256="7da3e2d9a171eb0a038f592ecad3ff2bb2550f3496d87b3b29ad0cf4430c0db4"
PCRE2_VER="10.49"
PCRE2_SHA256="929f0b20e62879252a15886b06c89f1edef61a363cbd5826fb041080a5e557ae"
LIBXML2_VER="2.15.4"
LIBXML2_SHA256="98087fd181d9070724f3fbc65c7377db03038eb92bd882374daff44940138821"
LIBJPEG_VER="3.2.0"
LIBJPEG_SHA256="6f30092cef9fb839779646608f4ee14ae3cbac989c47fa05e841b0841f09878e"
LIBPNG_VER="1.6.59"
LIBPNG_SHA256="86a3e4b501f7f50e392c4e456ea158893e9a595b1c63eb88c7bb9f6cf8772dad"
XKBCOMMON_VER="1.13.2"
XKBCOMMON_SHA256="acc4d5f7c3cbba5f9f8d08d8bdbeede84ecede46792f47929aa9321873385528"
# Keyboard layouts xkbcommon compiles keymaps from (signed by Sergey
# Udaltsov, FFB4CCD275AAA422F5F9808E0661D98FC933A145, as Arch pins).
XKEYBOARD_CONFIG_VER="2.48"
XKEYBOARD_CONFIG_SHA256="b77041324f0109f77161ee43743fe04baa485866af8460d31e476ad3f7648fd5"
EXPAT_VER="2.9.0"
EXPAT_SHA256="16afbb9cefead2aa278105cf27d9f597bde7fbf3dbb85015857ca7ca6a4e89ba"
SQLITE_VER="3530400"  # 3.53.4
SQLITE_YEAR="2026"    # the sqlite.org release directory
SQLITE_SHA256="0e9483900e92cd5de8fd48d16bf9200145a61f7fd5be542a5ac81d8a9516eb9c"
OPENSSL_VER="4.0.3"   # checksum as published with the release
OPENSSL_SHA256="325b5c806167c13b40b1ffeadfe0248197c00eccc4cf123ec1e28d2d2fd216d9"
LIBEVDEV_VER="1.14.0"
LIBEVDEV_SHA256="5a0966c7110648665983848bad696c7acba614a2160d2865d535397101007332"
MTDEV_VER="1.1.7"
MTDEV_SHA256="a107adad2101fecac54ac7f9f0e0a0dd155d954193da55c2340c97f2ff1d814e"
# libudev without the udev daemon: enumerates devices from /sys and /dev
# and watches for hotplug, which is what KWin, libinput and Qt need.
LIBUDEV_ZERO_VER="1.0.5"
LIBUDEV_ZERO_SHA256="bf4372f79ddbe6b0e266a3d2994ffac7018a7edf4f87632aecb5176565d96138"
LIBINPUT_VER="1.32.0"
LIBINPUT_SHA256="7dd6c1ca964c86eb6810ccd6639cda634e68bdca43bf3afec39e94e942fac8d4"
# KWin's display and input libraries: libdisplay-info (EDID parsing; signed
# by Simon Ser, 34FF9526CFEF0E97A340E2E40FDE7BE0E88F5E48) with hwdata's PNP
# vendor IDs, libxcvt (CVT modelines; signed by Alan Coopersmith,
# 4A193C06D35E7C670FA4EF0BA2FB9E081F2D130E), Little CMS (colour management;
# as Arch pins it), and libei's libeis (input emulation for remote desktop
# and accessibility; hwdata and libei as Alpine pins them).
HWDATA_VER="0.412"
HWDATA_SHA256="f0c64cd7e31d70a5fb3a52e53ab50a61e74c0421a6381eaa45114eec3bde5fe7"
LIBDISPLAY_INFO_VER="0.4.0"
LIBDISPLAY_INFO_SHA256="43b180baa143e2035654759d84e2b2f5ee77d5fe817c423838c7fe59c0d68459"
LIBXCVT_VER="0.1.3"
LIBXCVT_SHA256="a929998a8767de7dfa36d6da4751cdbeef34ed630714f2f4a767b351f2442e01"
LCMS2_VER="2.19.1"
LCMS2_SHA256="bfc54f7bab59fbc921012014a8032e4cba4abd46db47d46b76416a8c0b2815c8"
LIBEI_VER="1.6.0"
LIBEI_SHA256="5ed6078fa63afd554cc04b1001675615da0ed8fe23b80492ab63403140b5a830"
UTIL_LINUX_VER="2.42.4"   # libblkid + libmount (KF6 KCoreAddons needs libmount)
UTIL_LINUX_SHA256="fbd62a100ab7bb8746ba0661255c3c48185b1e9021507c624da01fbc696330ec"
# libgpg-error + libgcrypt: KWallet's backend (ksecretd, kwalletd) needs
# libgcrypt. Checksums of the tarballs whose GnuPG release signatures were
# verified.
LIBGPG_ERROR_VER="1.61"
LIBGPG_ERROR_SHA256="7a85413f2bc354f4f8aa832b718af122e48965e9e0eb9012ee659c13c6385c93"
LIBGCRYPT_VER="1.12.4"
LIBGCRYPT_SHA256="d77f68f48879510e79a2f65977ccc68981781ea0923e5bdffac2a193ea3d660e"
VULKAN_VER="1.4.363.0"    # Vulkan-Headers + Vulkan-Loader (KWin 6.7 requires Vulkan)
VULKAN_HEADERS_SHA256="4a078be12bef21cfebc09d878b77a63cff9d68f899254a0b00d0e37ef73e7f7e"
VULKAN_LOADER_SHA256="941eff558fc74f248745fb7df367640cf89a8457f216dae498a74ae14b6ba6f7"
# Event sounds (KNotifications, KNotifyConfig, KWin's system bell):
# libcanberra playing Ogg Vorbis through ALSA. Xiph's published checksums;
# alsa-lib signed by the ALSA release key
# (F04DF50737AC1A884C4B3D718380596DA6E59C91); libcanberra as Debian's.
LIBOGG_VER="1.3.6"
LIBOGG_SHA256="5c8253428e181840cd20d41f3ca16557a9cc04bad4a3d04cce84808677fa1061"
LIBVORBIS_VER="1.3.7"
LIBVORBIS_SHA256="b33cc4934322bcbf6efcbacf49e3ca01aadbea4114ec9589d1b1e9d20f72954b"
ALSA_LIB_VER="1.2.16.1"
ALSA_LIB_SHA256="f740db7f488255944ffd4428416ee3390a96742856916433df468c281436480e"
LIBCANBERRA_VER="0.30"
LIBCANBERRA_SHA256="c2b671e67e0c288a69fc33dc1b6f1b534d07882c2aceed37004bf48c601afa72"
# Plasma's calculator (libqalculate with GMP, MPFR, ICU and libcurl),
# Unicode support (ICU) and NetworkManager's libndp. GMP and MPFR as the
# native toolchain pins them; ICU signed by the ICU release robot
# (E52F07877A5805F9AF4AB0ACD46C5610D06E7001), curl by Daniel Stenberg
# (27EDEAF22F3ABCEB50DB9A125CC908FDB71E12C2), libndp as Alpine's; the
# libqalculate release pinned when first downloaded.
GMP_VER="6.3.0"
GMP_SHA256="a3c2b80201b89e68616f4ad30bc66aee4927c3ce50e33929ca819d5c43538898"
MPFR_VER="4.2.2"
MPFR_SHA256="b67ba0383ef7e8a8563734e2e889ef5ec3c3b898a01d00fa0a6869ad81c6ce01"
ICU_VER="78.3"
ICU_SHA256="3a2e7a47604ba702f345878308e6fefeca612ee895cf4a5f222e7955fabfe0c0"
CURL_VER="8.22.0"
CURL_SHA256="f7ef3ae8a22e521f289803fe93543eb64c329b58aa73a9e224dfd915a2a5f4f7"
LIBQALCULATE_VER="5.13.1"
LIBQALCULATE_SHA256="cf8d3eaf3d85030115701e997b6585e69573e71add88e7758486c5d1b01d3cc3"
LIBNDP_VER="1.9"
LIBNDP_SHA256="e564f5914a6b1b799c3afa64c258824a801c1b79a29e2fe6525b682249c65261"
# Plasma's prerequisites. Boost (headers only: kactivitymanagerd), libnl and
# lm-sensors' libsensors (libksysguard's network and sensor plugins),
# Linux-PAM (the screen locker, polkit), duktape (polkit's JavaScript rules
# engine), libxslt and the DocBook DTD and stylesheets (KDocTools).
# Checksums: Boost, libnl and libxslt as published with the release;
# Linux-PAM's tarball verified against its maintainer's signature (Dmitry V.
# Levin, 7BECFE3AF7B280BB52FF77F104BA4521C996DDE1); lm-sensors, duktape and
# the DocBook files as first downloaded.
BOOST_VER="1.92.0"
BOOST_SHA256="ea7b982002cc9dfbe59b0b217b206f470dc75f3de0bb2973d844118934d82411"
LIBNL_VER="3.12.0"
LIBNL_SHA256="fc51ca7196f1a3f5fdf6ffd3864b50f4f9c02333be28be4eeca057e103c0dd18"
LM_SENSORS_VER="3.6.2"
LM_SENSORS_SHA256="c6a0587e565778a40d88891928bf8943f27d353f382d5b745a997d635978a8f0"
LINUX_PAM_VER="1.7.3"
LINUX_PAM_SHA256="2ce4765fd49df6693771ef2941f81e33d8ee14b94a81a5c7b369aa3b137b85a5"
DUKTAPE_VER="2.7.0"
DUKTAPE_SHA256="90f8d2fa8b5567c6899830ddef2c03f3c27960b11aca222fa17aa7ac613c2890"
LIBXSLT_VER="1.1.45"
LIBXSLT_SHA256="9acfe68419c4d06a45c550321b3212762d92f41465062ca4ea19e632ee5d216e"
DOCBOOK_XML_VER="4.5"
DOCBOOK_XML_SHA256="4e4e037a2b83c98c6c94818390d4bdd3f6e10f6ec62dd79188594e26190dc7b4"
DOCBOOK_XSL_VER="1.79.2"
DOCBOOK_XSL_SHA256="ee8b9eca0b7a8f89075832a2da7534bce8c5478fc8fc2676f512d5d87d832102"
# Barcodes for KF6 Prison (QR codes in Plasma's clipboard, Data Matrix,
# PDF417 and scanning): libqrencode and libdmtx as Git tag archives (their
# own site is gone, and both carry CMake builds), zxing-cpp as its release
# tarball (the SHA-256 GitHub publishes for the asset).
QRENCODE_VER="4.1.1"
QRENCODE_SHA256="5385bc1b8c2f20f3b91d258bf8ccc8cf62023935df2d2676b5b67049f31a049c"
LIBDMTX_VER="0.7.8"
LIBDMTX_SHA256="2394bf1d1d693a5a4ca3cfcc1bb28a4d878bdb831ea9ca8f3d5c995d274bdc39"
ZXING_VER="3.1.1"
ZXING_SHA256="c3c02c29c0b519de7bd4e25b376e606e87f0761befd1282815642a2246613d14"
# Root certificates (Mozilla's, as curl publishes them), for OpenSSL, Qt
# and curl: /etc/ssl/certs/ca-certificates.crt.
CACERT_DATE="2026-09-25"
CACERT_SHA256="a41b5d356aea97a529fe27e0f7316d2f9d946d75927476cf9cf1b90637d00505"

log() { echo "[build-deps] $*"; }
die() { echo "[build-deps] ERROR: $*" >&2; exit 1; }

# shellcheck source=lib/cross-env.sh
source "${SCRIPT_DIR}/lib/cross-env.sh"

# ── zlib ──────────────────────────────────────────────────────────────
build_zlib() {
    installed libz.so zlib && return 0
    fetch "zlib-${ZLIB_VER}.tar.gz" \
        "https://github.com/madler/zlib/releases/download/v${ZLIB_VER}/zlib-${ZLIB_VER}.tar.gz" \
        "zlib-${ZLIB_VER}" "${ZLIB_SHA256}"
    log "Building zlib ${ZLIB_VER}..."
    # zlib's configure is not autotools; it reads CC, AR and RANLIB.
    (cd "${BUILD_DIR}/zlib-${ZLIB_VER}" && ./configure --prefix=/usr && make_install)
}

# ── bzip2, xz (liblzma), zstd ──────────────────────────────────────────
build_bzip2() {
    installed libbz2.so bzip2 && return 0
    fetch "bzip2-${BZIP2_VER}.tar.gz" "https://sourceware.org/pub/bzip2/bzip2-${BZIP2_VER}.tar.gz" \
        "bzip2-${BZIP2_VER}" "${BZIP2_SHA256}"
    log "Building bzip2 ${BZIP2_VER} (library)..."
    # Plain Makefiles: the shared library (its own Makefile, soname
    # libbz2.so.1.0 as distributions use) and the header, installed by hand.
    local src="${BUILD_DIR}/bzip2-${BZIP2_VER}"
    make -C "${src}" -f Makefile-libbz2_so -j"${JOBS}" CC="${CC}" \
        CFLAGS="${CFLAGS} -D_FILE_OFFSET_BITS=64"
    install -Dm644 "${src}/bzlib.h" "${SYSROOT}/usr/include/bzlib.h"
    install -Dm755 "${src}/libbz2.so.${BZIP2_VER}" "${SYSROOT}/usr/lib/libbz2.so.${BZIP2_VER}"
    ln -sfn "libbz2.so.${BZIP2_VER}" "${SYSROOT}/usr/lib/libbz2.so.1.0"
    ln -sfn "libbz2.so.${BZIP2_VER}" "${SYSROOT}/usr/lib/libbz2.so"
}

build_xz() {
    installed liblzma.so xz && return 0
    fetch "xz-${XZ_VER}.tar.xz" "https://github.com/tukaani-project/xz/releases/download/v${XZ_VER}/xz-${XZ_VER}.tar.xz" \
        "xz-${XZ_VER}" "${XZ_SHA256}"
    log "Building xz ${XZ_VER} (liblzma)..."
    (cd "${BUILD_DIR}/xz-${XZ_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --disable-xz \
            --disable-xzdec \
            --disable-lzmadec \
            --disable-lzmainfo \
            --disable-lzma-links \
            --disable-scripts \
            --disable-doc \
            --disable-nls && \
        make_install)
}

build_zstd() {
    installed libzstd.so zstd && return 0
    fetch "zstd-${ZSTD_VER}.tar.gz" "https://github.com/facebook/zstd/releases/download/v${ZSTD_VER}/zstd-${ZSTD_VER}.tar.gz" \
        "zstd-${ZSTD_VER}" "${ZSTD_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/zstd_legacy_v01_init.py" \
        "${BUILD_DIR}/zstd-${ZSTD_VER}/lib/legacy/zstd_v01.c" || die "failed to patch zstd_v01.c"
    log "Building zstd ${ZSTD_VER} (library)..."
    cmake_build "${BUILD_DIR}/zstd-${ZSTD_VER}/build/cmake" "${BUILD_DIR}/zstd-build" \
        -DZSTD_BUILD_SHARED=ON \
        -DZSTD_BUILD_STATIC=OFF \
        -DZSTD_BUILD_PROGRAMS=OFF \
        -DZSTD_BUILD_TESTS=OFF
}

# ── libffi ────────────────────────────────────────────────────────────
build_libffi() {
    installed libffi.so libffi && return 0
    fetch "libffi-${LIBFFI_VER}.tar.gz" \
        "https://github.com/libffi/libffi/releases/download/v${LIBFFI_VER}/libffi-${LIBFFI_VER}.tar.gz" \
        "libffi-${LIBFFI_VER}" "${LIBFFI_SHA256}"
    local src="${BUILD_DIR}/libffi-${LIBFFI_VER}"
    python3 -I "${SCRIPT_DIR}/deps-patches/libffi_java_raw_api.py" "${src}/src/java_raw_api.c" ||
        die "failed to patch libffi java_raw_api.c"
    python3 -I "${SCRIPT_DIR}/deps-patches/libtool_grep_escape.py" "${src}/configure" ||
        die "failed to patch libffi configure"
    log "Building libffi ${LIBFFI_VER}..."
    # Into usr/lib like everything else, not GCC's multi-OS directory
    # (../lib64 on x86_64).
    (cd "${src}" && ./configure "${COMMON_CONFIGURE[@]}" --disable-multi-os-directory && make_install)
}

# ── pcre2 ─────────────────────────────────────────────────────────────
build_pcre2() {
    installed libpcre2-16.so pcre2 && return 0
    fetch "pcre2-${PCRE2_VER}.tar.gz" \
        "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-${PCRE2_VER}/pcre2-${PCRE2_VER}.tar.gz" \
        "pcre2-${PCRE2_VER}" "${PCRE2_SHA256}"
    log "Building pcre2 ${PCRE2_VER}..."
    (cd "${BUILD_DIR}/pcre2-${PCRE2_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --enable-unicode \
            --enable-pcre2-8 \
            --enable-pcre2-16 \
            --disable-pcre2-32 && \
        make_install)
}

# ── expat (D-Bus, Fontconfig, Wayland) ────────────────────────────────
build_expat() {
    installed libexpat.so expat && return 0
    fetch "expat-${EXPAT_VER}.tar.gz" \
        "https://github.com/libexpat/libexpat/releases/download/R_${EXPAT_VER//./_}/expat-${EXPAT_VER}.tar.gz" \
        "expat-${EXPAT_VER}" "${EXPAT_SHA256}"
    log "Building expat ${EXPAT_VER}..."
    (cd "${BUILD_DIR}/expat-${EXPAT_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" --without-docbook && make_install)
}

# ── libxml2 ───────────────────────────────────────────────────────────
build_libxml2() {
    installed libxml2.so libxml2 && return 0
    fetch "libxml2-${LIBXML2_VER}.tar.gz" \
        "https://download.gnome.org/sources/libxml2/${LIBXML2_VER%.*}/libxml2-${LIBXML2_VER}.tar.xz" \
        "libxml2-${LIBXML2_VER}" "${LIBXML2_SHA256}"
    log "Building libxml2 ${LIBXML2_VER}..."
    (cd "${BUILD_DIR}/libxml2-${LIBXML2_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --without-python \
            --without-icu \
            --with-zlib && \
        make_install)
}

# ── libjpeg-turbo ─────────────────────────────────────────────────────
# With its SIMD code (NASM), position-independent, as distributions build
# it.
build_libjpeg() {
    installed libjpeg.so libjpeg-turbo && return 0
    fetch "libjpeg-turbo-${LIBJPEG_VER}.tar.gz" \
        "https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/${LIBJPEG_VER}/libjpeg-turbo-${LIBJPEG_VER}.tar.gz" \
        "libjpeg-turbo-${LIBJPEG_VER}" "${LIBJPEG_SHA256}"
    log "Building libjpeg-turbo ${LIBJPEG_VER}..."
    cmake_build "${BUILD_DIR}/libjpeg-turbo-${LIBJPEG_VER}" "${BUILD_DIR}/libjpeg-build" \
        -DENABLE_SHARED=ON \
        -DENABLE_STATIC=OFF \
        -DWITH_TURBOJPEG=OFF \
        -DWITH_SIMD=ON \
        -DREQUIRE_SIMD=ON
}

# ── libpng ────────────────────────────────────────────────────────────
build_libpng() {
    installed libpng16.so libpng && return 0
    fetch "libpng-${LIBPNG_VER}.tar.gz" \
        "https://download.sourceforge.net/libpng/libpng-${LIBPNG_VER}.tar.gz" \
        "libpng-${LIBPNG_VER}" "${LIBPNG_SHA256}"
    log "Building libpng ${LIBPNG_VER}..."
    (cd "${BUILD_DIR}/libpng-${LIBPNG_VER}" && ./configure "${COMMON_CONFIGURE[@]}" && make_install)
}

# ── xkeyboard-config (data: keyboard layouts) ─────────────────────────
build_xkeyboard_config() {
    if [[ -f "${SYSROOT}/usr/share/X11/xkb/rules/evdev" ]]; then
        log "xkeyboard-config: already installed."
        return 0
    fi
    fetch "xkeyboard-config-${XKEYBOARD_CONFIG_VER}.tar.xz" \
        "https://www.x.org/releases/individual/data/xkeyboard-config/xkeyboard-config-${XKEYBOARD_CONFIG_VER}.tar.xz" \
        "xkeyboard-config-${XKEYBOARD_CONFIG_VER}" "${XKEYBOARD_CONFIG_SHA256}"
    log "Installing xkeyboard-config ${XKEYBOARD_CONFIG_VER}..."
    # Architecture-independent data: configured natively, for /usr.
    local bld="${BUILD_DIR}/xkeyboard-config-build"
    rm -rf "${bld}"
    meson setup "${bld}" "${BUILD_DIR}/xkeyboard-config-${XKEYBOARD_CONFIG_VER}" --prefix=/usr
    ninja -C "${bld}" -j"${JOBS}"
    DESTDIR="${SYSROOT}" ninja -C "${bld}" install
}

# ── libxkbcommon ──────────────────────────────────────────────────────
build_xkbcommon() {
    installed libxkbcommon-x11.so libxkbcommon && return 0
    # Releases are GitHub tags now (xkbcommon.org no longer hosts tarballs).
    fetch "libxkbcommon-${XKBCOMMON_VER}.tar.gz" \
        "https://github.com/xkbcommon/libxkbcommon/archive/refs/tags/xkbcommon-${XKBCOMMON_VER}.tar.gz" \
        "libxkbcommon-xkbcommon-${XKBCOMMON_VER}" "${XKBCOMMON_SHA256}"
    log "Building libxkbcommon ${XKBCOMMON_VER}..."
    meson_build "${BUILD_DIR}/libxkbcommon-xkbcommon-${XKBCOMMON_VER}" "${BUILD_DIR}/xkbcommon-build" \
        -Dxkb-config-root=/usr/share/X11/xkb \
        -Dx-locale-root=/usr/share/X11/locale \
        -Denable-wayland=false \
        -Denable-x11=true \
        -Denable-tools=false \
        -Denable-bash-completion=false \
        -Denable-docs=false
}

# ── SQLite ────────────────────────────────────────────────────────────
build_sqlite() {
    # An earlier build without a SONAME is replaced.
    if [[ -f "${SYSROOT}/usr/lib/libsqlite3.so" ]] &&
       "${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-readelf" -d "${SYSROOT}/usr/lib/libsqlite3.so" |
           grep -q 'Library soname: \[libsqlite3.so.0\]'; then
        log "SQLite: already installed."
        return 0
    fi
    fetch "sqlite-${SQLITE_VER}.tar.gz" \
        "https://www.sqlite.org/${SQLITE_YEAR}/sqlite-autoconf-${SQLITE_VER}.tar.gz" \
        "sqlite-autoconf-${SQLITE_VER}" "${SQLITE_SHA256}"
    log "Building SQLite..."
    # Column metadata: Qt's SQL driver uses sqlite3_column_table_name16;
    # unlock notification: it waits on locked tables with it. Distributions
    # build SQLite with both, and with the historical SONAME: without one
    # (SQLite's default) a program linked by path records the build host's
    # path as its dependency.
    (cd "${BUILD_DIR}/sqlite-autoconf-${SQLITE_VER}" && \
        CFLAGS="-O2 -fPIC -DSQLITE_THREADSAFE=1 -DSQLITE_ENABLE_COLUMN_METADATA=1 -DSQLITE_ENABLE_UNLOCK_NOTIFY=1" \
        ./configure \
            --host="${HOST}" \
            --prefix=/usr \
            --enable-shared \
            --disable-static \
            --disable-readline \
            --soname=legacy && \
        make_install)
}

# ── OpenSSL (Qt TLS, QCA) ─────────────────────────────────────────────
build_openssl() {
    installed libssl.so OpenSSL && return 0
    fetch "openssl-${OPENSSL_VER}.tar.gz" \
        "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VER}/openssl-${OPENSSL_VER}.tar.gz" \
        "openssl-${OPENSSL_VER}" "${OPENSSL_SHA256}"
    log "Building OpenSSL ${OPENSSL_VER}..."
    # Configuration and certificates in /etc/ssl on the target. No async
    # (musl has no makecontext).
    (cd "${BUILD_DIR}/openssl-${OPENSSL_VER}" && \
        ./Configure linux-x86_64 \
            --prefix=/usr \
            --libdir=lib \
            --openssldir=/etc/ssl \
            --cross-compile-prefix= \
            no-async \
            no-engine \
            no-tests \
            -fPIC && \
        make -j"${JOBS}" && \
        make install_sw install_ssldirs DESTDIR="${SYSROOT}")
}

# ── libevdev (libinput) ───────────────────────────────────────────────
build_libevdev() {
    installed libevdev.so libevdev && return 0
    fetch "libevdev-${LIBEVDEV_VER}.tar.xz" \
        "https://freedesktop.org/software/libevdev/libevdev-${LIBEVDEV_VER}.tar.xz" \
        "libevdev-${LIBEVDEV_VER}" "${LIBEVDEV_SHA256}"
    log "Building libevdev ${LIBEVDEV_VER}..."
    meson_build "${BUILD_DIR}/libevdev-${LIBEVDEV_VER}" "${BUILD_DIR}/libevdev-build" \
        -Dtests=disabled \
        -Ddocumentation=disabled
}

# ── mtdev (libinput) ──────────────────────────────────────────────────
build_mtdev() {
    installed libmtdev.so mtdev && return 0
    fetch "mtdev-${MTDEV_VER}.tar.gz" \
        "https://bitmath.org/code/mtdev/mtdev-${MTDEV_VER}.tar.bz2" \
        "mtdev-${MTDEV_VER}" "${MTDEV_SHA256}"
    log "Building mtdev ${MTDEV_VER}..."
    (cd "${BUILD_DIR}/mtdev-${MTDEV_VER}" && ./configure "${COMMON_CONFIGURE[@]}" && make_install)
}

# ── libudev (libinput, KWin, Qt) ──────────────────────────────────────
build_libudev() {
    installed libudev.so libudev-zero && return 0
    fetch "libudev-zero-${LIBUDEV_ZERO_VER}.tar.gz" \
        "https://github.com/illiliti/libudev-zero/archive/refs/tags/${LIBUDEV_ZERO_VER}.tar.gz" \
        "libudev-zero-${LIBUDEV_ZERO_VER}" "${LIBUDEV_ZERO_SHA256}"
    log "Building libudev-zero ${LIBUDEV_ZERO_VER}..."
    meson_build "${BUILD_DIR}/libudev-zero-${LIBUDEV_ZERO_VER}" "${BUILD_DIR}/libudev-zero-build"
}

# ── libinput ──────────────────────────────────────────────────────────
build_libinput() {
    installed libinput.so libinput && return 0
    fetch "libinput-${LIBINPUT_VER}.tar.gz" \
        "https://gitlab.freedesktop.org/libinput/libinput/-/archive/${LIBINPUT_VER}/libinput-${LIBINPUT_VER}.tar.gz" \
        "libinput-${LIBINPUT_VER}" "${LIBINPUT_SHA256}"
    local src="${BUILD_DIR}/libinput-${LIBINPUT_VER}"
    log "Building libinput ${LIBINPUT_VER}..."
    meson_build "${src}" "${BUILD_DIR}/libinput-build" \
        -Dlibwacom=false \
        -Ddebug-gui=false \
        -Dtests=false \
        -Ddocumentation=false \
        -Dzshcompletiondir=no
}

# ── hwdata (PNP, PCI and USB ID tables) ───────────────────────────────
# Data only: into the sysroot (/usr/share/hwdata, read by KWin at run
# time) and into the host tools, where libdisplay-info's build reads the
# PNP table (a native pkg-config lookup; otherwise it would take the build
# host's own hwdata, whatever version that is).
build_hwdata() {
    local host_pc="${VERIDIAN_HOST_TOOLS}/share/pkgconfig/hwdata.pc"
    if [[ -f "${SYSROOT}/usr/share/hwdata/pnp.ids" && -f "${host_pc}" ]] &&
       grep -qx "Version: ${HWDATA_VER}" "${host_pc}"; then
        log "hwdata: already installed."
        return 0
    fi
    fetch "hwdata-${HWDATA_VER}.tar.gz" \
        "https://github.com/vcrhonek/hwdata/archive/refs/tags/v${HWDATA_VER}.tar.gz" \
        "hwdata-${HWDATA_VER}" "${HWDATA_SHA256}"
    local src="${BUILD_DIR}/hwdata-${HWDATA_VER}"
    log "Installing hwdata ${HWDATA_VER}..."
    # No modprobe blacklist: there is no modprobe.
    (cd "${src}" && ./configure --prefix=/usr --disable-blacklist &&
        make install DESTDIR="${SYSROOT}")
    rm -f "${src}/Makefile.inc" "${src}/hwdata.pc"
    (cd "${src}" && ./configure --prefix="${VERIDIAN_HOST_TOOLS}" --disable-blacklist &&
        make install)
}

# ── libdisplay-info ───────────────────────────────────────────────────
build_libdisplay_info() {
    installed libdisplay-info.so libdisplay-info && return 0
    fetch "libdisplay-info-${LIBDISPLAY_INFO_VER}.tar.xz" \
        "https://gitlab.freedesktop.org/emersion/libdisplay-info/-/releases/${LIBDISPLAY_INFO_VER}/downloads/libdisplay-info-${LIBDISPLAY_INFO_VER}.tar.xz" \
        "libdisplay-info-${LIBDISPLAY_INFO_VER}" "${LIBDISPLAY_INFO_SHA256}"
    log "Building libdisplay-info ${LIBDISPLAY_INFO_VER}..."
    meson_build "${BUILD_DIR}/libdisplay-info-${LIBDISPLAY_INFO_VER}" "${BUILD_DIR}/libdisplay-info-build"
}

# ── libxcvt ───────────────────────────────────────────────────────────
build_libxcvt() {
    installed libxcvt.so libxcvt && return 0
    fetch "libxcvt-${LIBXCVT_VER}.tar.xz" \
        "https://www.x.org/releases/individual/lib/libxcvt-${LIBXCVT_VER}.tar.xz" \
        "libxcvt-${LIBXCVT_VER}" "${LIBXCVT_SHA256}"
    local src="${BUILD_DIR}/libxcvt-${LIBXCVT_VER}"
    # The library uses darwin_versions, new in meson 0.48, under a declared
    # minimum of 0.40 (meson warns); 0.48 is what it requires.
    replace_once "${src}/meson.build" \
        "meson_version: '>= 0.40.0'," \
        "meson_version: '>= 0.48.0',"
    log "Building libxcvt ${LIBXCVT_VER}..."
    meson_build "${src}" "${BUILD_DIR}/libxcvt-build"
}

# ── Little CMS ────────────────────────────────────────────────────────
build_lcms2() {
    installed liblcms2.so lcms2 && return 0
    fetch "lcms2-${LCMS2_VER}.tar.gz" \
        "https://github.com/mm2/Little-CMS/releases/download/lcms${LCMS2_VER}/lcms2-${LCMS2_VER}.tar.gz" \
        "lcms2-${LCMS2_VER}" "${LCMS2_SHA256}"
    log "Building Little CMS ${LCMS2_VER}..."
    # JPEG and TIFF are for the utilities only, which are not built.
    meson_build "${BUILD_DIR}/lcms2-${LCMS2_VER}" "${BUILD_DIR}/lcms2-build" \
        -Dtests=disabled \
        -Djpeg=disabled \
        -Dtiff=disabled \
        -Dutils=false
}

# ── libei (libeis) ────────────────────────────────────────────────────
# KWin serves input emulation (portals, remote desktop) through libeis.
# liboeffis talks to the portal over sd-bus, which VeridianOS does not
# have; it is a client-side convenience nothing here uses.
build_libei() {
    installed libeis.so libei && return 0
    fetch "libei-${LIBEI_VER}.tar.bz2" \
        "https://gitlab.freedesktop.org/libinput/libei/-/archive/${LIBEI_VER}/libei-${LIBEI_VER}.tar.bz2" \
        "libei-${LIBEI_VER}" "${LIBEI_SHA256}"
    log "Building libei ${LIBEI_VER}..."
    meson_build "${BUILD_DIR}/libei-${LIBEI_VER}" "${BUILD_DIR}/libei-build" \
        -Dliboeffis=disabled \
        -Dtests=disabled \
        -Ddocumentation=[]
}

# ── libmount, libblkid and libuuid from util-linux ────────────────────
# KF6 KCoreAddons requires libmount on Linux targets (mount point
# queries), and NetworkManager (libnm) libuuid; only the libraries are
# built, no util-linux programs.
build_libmount() {
    if [[ -f "${SYSROOT}/usr/lib/libmount.so" && -f "${SYSROOT}/usr/lib/libuuid.so" ]]; then
        log "libmount, libuuid: already installed."
        return 0
    fi
    fetch "util-linux-${UTIL_LINUX_VER}.tar.gz" \
        "https://cdn.kernel.org/pub/linux/utils/util-linux/v${UTIL_LINUX_VER%.*}/util-linux-${UTIL_LINUX_VER}.tar.xz" \
        "util-linux-${UTIL_LINUX_VER}" "${UTIL_LINUX_SHA256}"
    log "Building libblkid, libmount and libuuid ${UTIL_LINUX_VER}..."
    (cd "${BUILD_DIR}/util-linux-${UTIL_LINUX_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --disable-all-programs \
            --enable-libblkid \
            --enable-libmount \
            --enable-libuuid \
            --disable-liblastlog2 \
            --disable-nls \
            --disable-bash-completion \
            --disable-asciidoc \
            --disable-poman \
            --disable-makeinstall-chown \
            --disable-makeinstall-setuid \
            --without-python \
            --without-systemd \
            --without-udev \
            --without-cryptsetup \
            --without-econf \
            --without-ncursesw \
            --without-tinfo \
            --without-selinux \
            --without-audit && \
        make_install)
}

# ── libgpg-error and libgcrypt ────────────────────────────────────────
build_libgpg_error() {
    installed libgpg-error.so libgpg-error && return 0
    fetch "libgpg-error-${LIBGPG_ERROR_VER}.tar.gz" \
        "https://gnupg.org/ftp/gcrypt/libgpg-error/libgpg-error-${LIBGPG_ERROR_VER}.tar.bz2" \
        "libgpg-error-${LIBGPG_ERROR_VER}" "${LIBGPG_ERROR_SHA256}"
    log "Building libgpg-error ${LIBGPG_ERROR_VER}..."
    (cd "${BUILD_DIR}/libgpg-error-${LIBGPG_ERROR_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --disable-nls \
            --disable-doc \
            --disable-tests \
            --disable-languages && \
        make_install)
}

build_libgcrypt() {
    installed libgcrypt.so libgcrypt && return 0
    fetch "libgcrypt-${LIBGCRYPT_VER}.tar.gz" \
        "https://gnupg.org/ftp/gcrypt/libgcrypt/libgcrypt-${LIBGCRYPT_VER}.tar.bz2" \
        "libgcrypt-${LIBGCRYPT_VER}" "${LIBGCRYPT_SHA256}"
    log "Building libgcrypt ${LIBGCRYPT_VER}..."
    # The staged gpgrt-config (configure would otherwise find the build
    # host's in /usr/bin, which reads the host's gpg-error.pc). Given that,
    # configure points it at the sysroot's pkgconfig directory, and it
    # places the /usr paths it reads under PKG_CONFIG_SYSROOT_DIR.
    (cd "${BUILD_DIR}/libgcrypt-${LIBGCRYPT_VER}" && \
        GPGRT_CONFIG="${SYSROOT}/usr/bin/gpgrt-config" ./configure "${COMMON_CONFIGURE[@]}" \
            --with-libgpg-error-prefix="${SYSROOT}/usr" \
            --disable-doc && \
        make_install)
}

# ── Vulkan headers and loader ─────────────────────────────────────────
# KWin 6.7 requires Vulkan. The loader is Khronos's, as upstream builds
# it. With no Vulkan driver installed, vkCreateInstance reports no
# compatible driver and KWin renders with OpenGL.
build_vulkan() {
    installed libvulkan.so Vulkan && return 0
    local tag="vulkan-sdk-${VULKAN_VER}"
    fetch "Vulkan-Headers-${VULKAN_VER}.tar.gz" \
        "https://github.com/KhronosGroup/Vulkan-Headers/archive/refs/tags/${tag}.tar.gz" \
        "Vulkan-Headers-${tag}" "${VULKAN_HEADERS_SHA256}"
    fetch "Vulkan-Loader-${VULKAN_VER}.tar.gz" \
        "https://github.com/KhronosGroup/Vulkan-Loader/archive/refs/tags/${tag}.tar.gz" \
        "Vulkan-Loader-${tag}" "${VULKAN_LOADER_SHA256}"

    log "Building Vulkan-Headers ${VULKAN_VER}..."
    cmake_build "${BUILD_DIR}/Vulkan-Headers-${tag}" "${BUILD_DIR}/vulkan-headers-build"

    log "Building Vulkan-Loader ${VULKAN_VER}..."
    local lsrc="${BUILD_DIR}/Vulkan-Loader-${tag}"
    cmake_build "${lsrc}" "${BUILD_DIR}/vulkan-loader-build" \
        -DVULKAN_HEADERS_INSTALL_DIR="${SYSROOT}/usr" \
        -DBUILD_TESTS=OFF \
        -DBUILD_WSI_XCB_SUPPORT=OFF \
        -DBUILD_WSI_XLIB_SUPPORT=OFF \
        -DBUILD_WSI_XLIB_XRANDR_SUPPORT=OFF \
        -DBUILD_WSI_WAYLAND_SUPPORT=ON \
        -DBUILD_WSI_DIRECTFB_SUPPORT=OFF
}

# ── Audio: libogg, libvorbis, alsa-lib, libcanberra ───────────────────
build_libogg() {
    installed libogg.so libogg && return 0
    fetch "libogg-${LIBOGG_VER}.tar.xz" \
        "https://downloads.xiph.org/releases/ogg/libogg-${LIBOGG_VER}.tar.xz" \
        "libogg-${LIBOGG_VER}" "${LIBOGG_SHA256}"
    log "Building libogg ${LIBOGG_VER}..."
    (cd "${BUILD_DIR}/libogg-${LIBOGG_VER}" && ./configure "${COMMON_CONFIGURE[@]}" && make_install)
}

build_libvorbis() {
    installed libvorbisfile.so libvorbis && return 0
    fetch "libvorbis-${LIBVORBIS_VER}.tar.xz" \
        "https://downloads.xiph.org/releases/vorbis/libvorbis-${LIBVORBIS_VER}.tar.xz" \
        "libvorbis-${LIBVORBIS_VER}" "${LIBVORBIS_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/libvorbis_warnings.py" \
        "${BUILD_DIR}/libvorbis-${LIBVORBIS_VER}" || die "failed to patch libvorbis"
    log "Building libvorbis ${LIBVORBIS_VER}..."
    # --disable-oggtest: that check runs a target program.
    (cd "${BUILD_DIR}/libvorbis-${LIBVORBIS_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" --disable-oggtest --disable-docs --disable-examples && \
        make_install)
}

build_alsa_lib() {
    installed libasound.so alsa-lib && return 0
    fetch "alsa-lib-${ALSA_LIB_VER}.tar.bz2" \
        "https://www.alsa-project.org/files/pub/lib/alsa-lib-${ALSA_LIB_VER}.tar.bz2" \
        "alsa-lib-${ALSA_LIB_VER}" "${ALSA_LIB_SHA256}"
    log "Building alsa-lib ${ALSA_LIB_VER}..."
    # Configuration in /usr/share/alsa on the target.
    (cd "${BUILD_DIR}/alsa-lib-${ALSA_LIB_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" --disable-python && \
        make_install)
}

build_libcanberra() {
    installed libcanberra.so libcanberra && return 0
    fetch "libcanberra-${LIBCANBERRA_VER}.tar.xz" \
        "http://0pointer.de/lennart/projects/libcanberra/libcanberra-${LIBCANBERRA_VER}.tar.xz" \
        "libcanberra-${LIBCANBERRA_VER}" "${LIBCANBERRA_SHA256}"
    # Its 2012 config.sub does not know musl targets.
    refresh_config_scripts "${BUILD_DIR}/libcanberra-${LIBCANBERRA_VER}"
    log "Building libcanberra ${LIBCANBERRA_VER}..."
    # The ALSA driver is built in, so no driver is loaded at run time and
    # nothing links libltdl (only the "dso" driver does, src/Makefile.am);
    # configure checks for it whatever the driver, so its result is given.
    # udev serves only canberra-boot, a systemd boot-sound program.
    (cd "${BUILD_DIR}/libcanberra-${LIBCANBERRA_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --with-builtin=alsa \
            --disable-oss \
            --disable-pulse \
            --disable-gstreamer \
            --disable-null \
            --disable-gtk \
            --disable-gtk3 \
            --disable-udev \
            --disable-tdb \
            --disable-lynx \
            --with-systemdsystemunitdir=no \
            ac_cv_header_ltdl_h=yes \
            ac_cv_lib_ltdl_lt_dladvise_init=yes && \
        make_install)
}

# ── GMP, MPFR (libqalculate) ──────────────────────────────────────────
build_gmp() {
    installed libgmp.so GMP && return 0
    fetch "gmp-${GMP_VER}.tar.xz" "https://ftp.gnu.org/gnu/gmp/gmp-${GMP_VER}.tar.xz" \
        "gmp-${GMP_VER}" "${GMP_SHA256}"
    log "Building GMP ${GMP_VER}..."
    # Its build runs generator programs on the host (CC_FOR_BUILD). GMP
    # 6.3.0 is C17 code: its configure tests declare functions without
    # prototypes, which C23 (GCC 15's default) rejects, so every compiler
    # "fails" without -std=gnu17.
    (cd "${BUILD_DIR}/gmp-${GMP_VER}" && \
        CC_FOR_BUILD="gcc -std=gnu17" CFLAGS="${CFLAGS} -std=gnu17" \
            ./configure "${COMMON_CONFIGURE[@]}" --with-pic && \
        make_install)
}

build_mpfr() {
    installed libmpfr.so MPFR && return 0
    fetch "mpfr-${MPFR_VER}.tar.xz" "https://ftp.gnu.org/gnu/mpfr/mpfr-${MPFR_VER}.tar.xz" \
        "mpfr-${MPFR_VER}" "${MPFR_SHA256}"
    log "Building MPFR ${MPFR_VER}..."
    (cd "${BUILD_DIR}/mpfr-${MPFR_VER}" && ./configure "${COMMON_CONFIGURE[@]}" && make_install)
}

# ── ICU ───────────────────────────────────────────────────────────────
# A cross build of ICU needs a native build of the same version for its
# data tools (--with-cross-build); the native one is not installed.
build_icu() {
    installed libicuuc.so ICU && return 0
    fetch "icu4c-${ICU_VER}-sources.tgz" \
        "https://github.com/unicode-org/icu/releases/download/release-${ICU_VER}/icu4c-${ICU_VER}-sources.tgz" \
        icu "${ICU_SHA256}"
    local src="${BUILD_DIR}/icu/source" host_bld="${BUILD_DIR}/icu-host" bld="${BUILD_DIR}/icu-cross"
    log "Building ICU ${ICU_VER} (host tools)..."
    rm -rf "${host_bld}" "${bld}"
    mkdir -p "${host_bld}" "${bld}"
    (cd "${host_bld}" && \
        env -u CC -u CXX -u AR -u RANLIB -u NM -u STRIP -u CFLAGS -u CXXFLAGS \
            -u PKG_CONFIG -u PKG_CONFIG_LIBDIR -u PKG_CONFIG_SYSROOT_DIR \
            "${src}/configure" --disable-tests --disable-samples && \
        env -u CC -u CXX -u AR -u RANLIB -u NM -u STRIP -u CFLAGS -u CXXFLAGS make -j"${JOBS}")
    log "Building ICU ${ICU_VER}..."
    (cd "${bld}" && \
        "${src}/configure" "${COMMON_CONFIGURE[@]}" \
            --with-cross-build="${host_bld}" \
            --disable-tests \
            --disable-samples && \
        make_install)
}

# ── curl ──────────────────────────────────────────────────────────────
# With OpenSSL and the system root certificates. No public suffix list
# (libpsl): it serves only curl's cookie engine.
build_curl() {
    installed libcurl.so curl && return 0
    fetch "curl-${CURL_VER}.tar.xz" "https://curl.se/download/curl-${CURL_VER}.tar.xz" \
        "curl-${CURL_VER}" "${CURL_SHA256}"
    log "Building curl ${CURL_VER}..."
    (cd "${BUILD_DIR}/curl-${CURL_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --with-openssl \
            --with-zlib \
            --with-zstd \
            --without-libpsl \
            --with-ca-bundle=/etc/ssl/certs/ca-certificates.crt \
            --with-ca-path=/etc/ssl/certs \
            --disable-docs && \
        make_install)
}

# ── libqalculate (Plasma's calculator) ────────────────────────────────
# The library only: the qalc text program (readline) is not built.
build_libqalculate() {
    installed libqalculate.so libqalculate && return 0
    fetch "libqalculate-${LIBQALCULATE_VER}.tar.gz" \
        "https://github.com/Qalculate/libqalculate/releases/download/v${LIBQALCULATE_VER}/libqalculate-${LIBQALCULATE_VER}.tar.gz" \
        "libqalculate-${LIBQALCULATE_VER}" "${LIBQALCULATE_SHA256}"
    log "Building libqalculate ${LIBQALCULATE_VER}..."
    (cd "${BUILD_DIR}/libqalculate-${LIBQALCULATE_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" --disable-textport && \
        make_install)
}

# ── libndp (NetworkManager) ───────────────────────────────────────────
build_libndp() {
    installed libndp.so libndp && return 0
    fetch "libndp-${LIBNDP_VER}.tar.gz" "https://github.com/jpirko/libndp/archive/v${LIBNDP_VER}.tar.gz" \
        "libndp-${LIBNDP_VER}" "${LIBNDP_SHA256}"
    python3 -I "${SCRIPT_DIR}/deps-patches/libndp_sendto_cast.py" \
        "${BUILD_DIR}/libndp-${LIBNDP_VER}/libndp/libndp.c" || die "failed to patch libndp"
    log "Building libndp ${LIBNDP_VER}..."
    # A Git tag archive: the build system is generated (host autotools).
    (cd "${BUILD_DIR}/libndp-${LIBNDP_VER}" && \
        ./autogen.sh && \
        ./configure "${COMMON_CONFIGURE[@]}" && \
        make_install)
}

# ── Barcode libraries (KF6 Prison) ────────────────────────────────────
build_qrencode() {
    installed libqrencode.so libqrencode && return 0
    fetch "libqrencode-${QRENCODE_VER}.tar.gz" \
        "https://github.com/fukuchi/libqrencode/archive/refs/tags/v${QRENCODE_VER}.tar.gz" \
        "libqrencode-${QRENCODE_VER}" "${QRENCODE_SHA256}"
    # It declares CMake 3.1, which CMake 4 refuses; it builds unchanged
    # under current policies.
    replace_once "${BUILD_DIR}/libqrencode-${QRENCODE_VER}/CMakeLists.txt" \
        'cmake_minimum_required(VERSION 3.1.0)' 'cmake_minimum_required(VERSION 3.1.0...3.31)'
    log "Building libqrencode ${QRENCODE_VER}..."
    # The library; its qrencode tool (PNG output) is not used.
    cmake_build "${BUILD_DIR}/libqrencode-${QRENCODE_VER}" "${BUILD_DIR}/qrencode-build" \
        -DBUILD_SHARED_LIBS=ON \
        -DWITH_TOOLS=NO \
        -DWITH_TESTS=NO
}

build_libdmtx() {
    installed libdmtx.so libdmtx && return 0
    fetch "libdmtx-${LIBDMTX_VER}.tar.gz" \
        "https://github.com/dmtx/libdmtx/archive/refs/tags/v${LIBDMTX_VER}.tar.gz" \
        "libdmtx-${LIBDMTX_VER}" "${LIBDMTX_SHA256}"
    # CMake 3.5 policies draw CMake 4's deprecation warning; current ones
    # build it unchanged.
    replace_once "${BUILD_DIR}/libdmtx-${LIBDMTX_VER}/CMakeLists.txt" \
        'cmake_minimum_required(VERSION 3.5)' 'cmake_minimum_required(VERSION 3.5...3.31)'
    log "Building libdmtx ${LIBDMTX_VER}..."
    cmake_build "${BUILD_DIR}/libdmtx-${LIBDMTX_VER}" "${BUILD_DIR}/libdmtx-build" \
        -DDMTX_SHARED=ON \
        -DDMTX_STATIC=OFF \
        -DBUILD_TESTING=OFF
}

build_zxing() {
    installed libZXing.so zxing-cpp && return 0
    fetch "zxing-cpp-${ZXING_VER}.tar.gz" \
        "https://github.com/zxing-cpp/zxing-cpp/releases/download/v${ZXING_VER}/zxing-cpp-${ZXING_VER}.tar.gz" \
        "zxing-cpp-${ZXING_VER}" "${ZXING_SHA256}"
    log "Building zxing-cpp ${ZXING_VER}..."
    # Readers and both writer backends (Prison generates PDF417 and scans);
    # its bundled libzint; nothing fetched at build time (LOCAL). The C
    # wrapper (which loads images with stb) is for C programs; Prison and
    # everything else here use the C++ library.
    cmake_build "${BUILD_DIR}/zxing-cpp-${ZXING_VER}" "${BUILD_DIR}/zxing-build" \
        -DBUILD_SHARED_LIBS=ON \
        -DZXING_READERS=ON \
        -DZXING_WRITERS=BOTH \
        -DZXING_USE_BUNDLED_ZINT=ON \
        -DZXING_DEPENDENCIES=LOCAL \
        -DZXING_C_API=OFF \
        -DZXING_EXAMPLES=OFF \
        -DZXING_BLACKBOX_TESTS=OFF \
        -DZXING_UNIT_TESTS=OFF
}

# ── Plasma prerequisites ──────────────────────────────────────────────
# Boost: its headers (kactivitymanagerd uses header-only libraries).
build_boost() {
    if [[ -f "${SYSROOT}/usr/include/boost/version.hpp" ]]; then
        log "Boost headers: already installed."
        return 0
    fi
    fetch "boost-${BOOST_VER}-b2-nodocs.tar.xz" \
        "https://github.com/boostorg/boost/releases/download/boost-${BOOST_VER}/boost-${BOOST_VER}-b2-nodocs.tar.xz" \
        "boost-${BOOST_VER}" "${BOOST_SHA256}"
    log "Installing Boost ${BOOST_VER} headers..."
    mkdir -p "${SYSROOT}/usr/include"
    cp -a "${BUILD_DIR}/boost-${BOOST_VER}/boost" "${SYSROOT}/usr/include/"
}

build_libnl() {
    installed libnl-3.so libnl && return 0
    fetch "libnl-${LIBNL_VER}.tar.gz" \
        "https://github.com/thom311/libnl/releases/download/libnl${LIBNL_VER//./_}/libnl-${LIBNL_VER}.tar.gz" \
        "libnl-${LIBNL_VER}" "${LIBNL_SHA256}"
    log "Building libnl ${LIBNL_VER}..."
    (cd "${BUILD_DIR}/libnl-${LIBNL_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" --disable-cli && \
        make_install)
}

# libsensors (the library and the `sensors` program; the Perl detection
# scripts are installed as data). Its parser is generated with the host's
# flex and bison.
build_lm_sensors() {
    installed libsensors.so lm-sensors && return 0
    fetch "lm-sensors-${LM_SENSORS_VER}.tar.gz" \
        "https://github.com/lm-sensors/lm-sensors/archive/refs/tags/V${LM_SENSORS_VER//./-}.tar.gz" \
        "lm-sensors-${LM_SENSORS_VER//./-}" "${LM_SENSORS_SHA256}"
    log "Building lm-sensors ${LM_SENSORS_VER}..."
    local args=(PREFIX=/usr CC="${CC}" AR="${AR}" BUILD_STATIC_LIB=0 EXLDFLAGS=
                MACHINE=x86_64 PROG_EXTRA=)
    make -C "${BUILD_DIR}/lm-sensors-${LM_SENSORS_VER//./-}" -j"${JOBS}" "${args[@]}" user
    make -C "${BUILD_DIR}/lm-sensors-${LM_SENSORS_VER//./-}" "${args[@]}" \
        DESTDIR="${SYSROOT}" user_install
}

# Linux-PAM: the modules VeridianOS can use (pam_unix against /etc/shadow,
# pam_deny, pam_permit, ...). Its manual pages need DocBook tooling at build
# time and are not built; the example programs are not installed.
# pam_veridian: PAM authentication against the kernel's account store
# (userland/pam_veridian), with the system call numbers the kernel uses
# (<veridian/sysno.h>, generated from abi/syscalls.map). Rebuilt when its
# source or the numbers are newer than the module.
build_pam_veridian() {
    local src="${PROJECT_ROOT}/userland/pam_veridian/pam_veridian.c"
    local sysno="${PROJECT_ROOT}/userland/libc/include/veridian/sysno.h"
    local out="${SYSROOT}/usr/lib/security/pam_veridian.so"
    if [[ -f "${out}" && "${out}" -nt "${src}" && "${out}" -nt "${sysno}" ]]; then
        log "pam_veridian: already installed."
        return 0
    fi
    log "Building pam_veridian..."
    mkdir -p "$(dirname "${out}")"
    "${VERIDIAN_CC}" -shared -fPIC -O2 -Wall -Wextra -Werror \
        -o "${out}" "${src}" -lpam || die "pam_veridian did not build"
}

build_linux_pam() {
    installed libpam.so Linux-PAM && return 0
    fetch "Linux-PAM-${LINUX_PAM_VER}.tar.xz" \
        "https://github.com/linux-pam/linux-pam/releases/download/v${LINUX_PAM_VER}/Linux-PAM-${LINUX_PAM_VER}.tar.xz" \
        "Linux-PAM-${LINUX_PAM_VER}" "${LINUX_PAM_SHA256}"
    log "Building Linux-PAM ${LINUX_PAM_VER}..."
    meson_build "${BUILD_DIR}/Linux-PAM-${LINUX_PAM_VER}" "${BUILD_DIR}/linux-pam-build" \
        -Ddocs=disabled \
        -Dexamples=false \
        -Daudit=disabled \
        -Dselinux=disabled \
        -Dlogind=disabled \
        -Delogind=disabled \
        -Dnis=disabled \
        -Deconf=disabled \
        -Dpam_userdb=disabled
}

# duktape: polkit's JavaScript engine for authorization rules.
build_duktape() {
    installed libduktape.so duktape && return 0
    fetch "duktape-${DUKTAPE_VER}.tar.xz" \
        "https://github.com/svaarala/duktape/releases/download/v${DUKTAPE_VER}/duktape-${DUKTAPE_VER}.tar.xz" \
        "duktape-${DUKTAPE_VER}" "${DUKTAPE_SHA256}"
    log "Building duktape ${DUKTAPE_VER}..."
    local args=(-f Makefile.sharedlibrary CC="${CC}" INSTALL_PREFIX=/usr LIBDIR=/lib)
    make -C "${BUILD_DIR}/duktape-${DUKTAPE_VER}" -j"${JOBS}" "${args[@]}"
    make -C "${BUILD_DIR}/duktape-${DUKTAPE_VER}" "${args[@]}" DESTDIR="${SYSROOT}" install
}

build_libxslt() {
    installed libxslt.so libxslt && return 0
    fetch "libxslt-${LIBXSLT_VER}.tar.xz" \
        "https://download.gnome.org/sources/libxslt/${LIBXSLT_VER%.*}/libxslt-${LIBXSLT_VER}.tar.xz" \
        "libxslt-${LIBXSLT_VER}" "${LIBXSLT_SHA256}"
    log "Building libxslt ${LIBXSLT_VER}..."
    (cd "${BUILD_DIR}/libxslt-${LIBXSLT_VER}" && \
        ./configure "${COMMON_CONFIGURE[@]}" --without-python --without-crypto && \
        make_install)
}

# The DocBook XML 4.5 DTD and the DocBook XSL stylesheets (data; KDocTools
# turns the KDE handbooks into help pages with them), where KDocTools and
# libxml2 catalogs look for them.
install_docbook() {
    local dtd="${SYSROOT}/usr/share/xml/docbook/xml-dtd-${DOCBOOK_XML_VER}"
    local xsl="${SYSROOT}/usr/share/xml/docbook/xsl-stylesheets"
    if [[ -f "${dtd}/docbookx.dtd" && -f "${xsl}/VERSION" ]]; then
        log "DocBook: already installed."
        return 0
    fi
    local zip="${VERIDIAN_SOURCES}/docbook-xml-${DOCBOOK_XML_VER}.zip"
    [[ -f "${zip}" ]] || curl -fsSL -o "${zip}" \
        "https://archive.docbook.org/xml/${DOCBOOK_XML_VER}/docbook-xml-${DOCBOOK_XML_VER}.zip" ||
        die "download failed: docbook-xml-${DOCBOOK_XML_VER}.zip"
    echo "${DOCBOOK_XML_SHA256}  ${zip}" | sha256sum -c --quiet - || die "docbook-xml: checksum mismatch"
    rm -rf "${dtd}"
    mkdir -p "${dtd}"
    python3 -I -m zipfile -e "${zip}" "${dtd}"
    fetch "docbook-xsl-nons-${DOCBOOK_XSL_VER}.tar.bz2" \
        "https://github.com/docbook/xslt10-stylesheets/releases/download/release%2F${DOCBOOK_XSL_VER}/docbook-xsl-nons-${DOCBOOK_XSL_VER}.tar.bz2" \
        "docbook-xsl-nons-${DOCBOOK_XSL_VER}" "${DOCBOOK_XSL_SHA256}"
    rm -rf "${xsl}"
    mkdir -p "$(dirname "${xsl}")"
    cp -a "${BUILD_DIR}/docbook-xsl-nons-${DOCBOOK_XSL_VER}" "${xsl}"
}

# ── Root certificates ─────────────────────────────────────────────────
install_cacert() {
    local dest="${SYSROOT}/etc/ssl/certs/ca-certificates.crt"
    local cache="${VERIDIAN_SOURCES}/cacert-${CACERT_DATE}.pem"
    if [[ ! -f "${cache}" ]]; then
        log "Downloading the root certificates (${CACERT_DATE})..."
        curl -fsSL -o "${cache}.part" "https://curl.se/ca/cacert-${CACERT_DATE}.pem" ||
            { rm -f "${cache}.part"; die "download failed: cacert-${CACERT_DATE}.pem"; }
        mv "${cache}.part" "${cache}"
    fi
    echo "${CACERT_SHA256}  ${cache}" | sha256sum -c --quiet - || die "cacert-${CACERT_DATE}.pem: checksum mismatch"
    install -Dm644 "${cache}" "${dest}"
}

verify() {
    log "Verifying all dependencies..."
    local errors=0 item
    for item in libz.so libbz2.so liblzma.so libzstd.so libffi.so libpcre2-8.so libpcre2-16.so libexpat.so libxml2.so libjpeg.so libpng16.so \
                libxkbcommon.so libsqlite3.so libssl.so libcrypto.so libevdev.so libmtdev.so \
                libudev.so libinput.so libblkid.so libmount.so libuuid.so libgpg-error.so libgcrypt.so \
                libvulkan.so libogg.so libvorbis.so libvorbisfile.so libasound.so libcanberra.so \
                libgmp.so libmpfr.so libicuuc.so libicui18n.so libicudata.so libcurl.so \
                libqalculate.so libndp.so libqrencode.so libdmtx.so libZXing.so libnl-3.so libsensors.so libpam.so security/pam_veridian.so \
                libduktape.so libxslt.so ../include/boost/version.hpp \
                ../share/xml/docbook/xml-dtd-4.5/docbookx.dtd ../share/xml/docbook/xsl-stylesheets/VERSION libdisplay-info.so libxcvt.so liblcms2.so libei.so libeis.so \
                ../share/hwdata/pnp.ids ../../etc/ssl/certs/ca-certificates.crt \
                ../share/X11/xkb/rules/evdev ../share/alsa/alsa.conf ../share/libinput \
                ../../etc/ssl/openssl.cnf; do
        if [[ -e "${SYSROOT}/usr/lib/${item}" ]]; then
            log "  OK: ${item##*/}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    # One library directory: a package that chose usr/lib64 is found by
    # neither the checks above nor the consumers' expectations.
    if [[ -e "${SYSROOT}/usr/lib64" ]]; then
        log "  WRONG: ${SYSROOT}/usr/lib64 exists"
        errors=$((errors + 1))
    fi
    [[ $errors -eq 0 ]] || die "${errors} items missing!"
    log "All dependencies installed."
}

main() {
    log "=== Building C dependencies for VeridianOS KDE ==="
    log "Sysroot: ${SYSROOT}"

    build_zlib
    build_bzip2
    build_xz
    build_zstd
    build_libffi
    build_pcre2
    build_expat
    build_libxml2
    build_libjpeg
    build_libpng
    build_xkeyboard_config
    build_xkbcommon
    build_sqlite
    build_openssl
    build_libevdev
    build_mtdev
    build_libudev
    build_libinput
    build_hwdata
    build_libdisplay_info
    build_libxcvt
    build_lcms2
    build_libei
    build_libmount
    build_libgpg_error
    build_libgcrypt
    build_vulkan
    build_libogg
    build_libvorbis
    build_alsa_lib
    build_libcanberra
    build_gmp
    build_mpfr
    build_icu
    build_curl
    build_libqalculate
    build_libndp
    build_qrencode
    build_libdmtx
    build_zxing
    build_boost
    build_libnl
    build_lm_sensors
    build_linux_pam
    build_pam_veridian
    build_duktape
    build_libxslt
    install_docbook
    install_cacert

    verify
    log "=== All dependencies built ==="
}

main "$@"

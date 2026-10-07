#!/usr/bin/env bash
# Build D-Bus for VeridianOS
#
# Produces libdbus-1.a and dbus-daemon static binary.
# D-Bus is required by KDE Plasma 6 for inter-process communication
# between KWin, plasmashell, and KDE services.
#
# Prerequisites:
#   - musl libc + expat built

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
BUILD_DIR="${VERIDIAN_CROSS_BUILD}/dbus"
SYSROOT="${VERIDIAN_SYSROOT}"
JOBS="${JOBS:-$(nproc)}"

DBUS_VER="1.16.2"  # 1.16 is meson-only (autotools was removed)
DBUS_URL="https://dbus.freedesktop.org/releases/dbus/dbus-${DBUS_VER}.tar.xz"

log() { echo "[build-dbus] $*"; }
die() { echo "[build-dbus] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
export CC
export CFLAGS="-O2 -fPIC"
export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

# Meson cross file with the resolved sysroot (as in build-fonts.sh).
generate_meson_cross() {
    local cross_file="${BUILD_DIR}/meson-cross.txt"
    cat > "${cross_file}" << CROSSEOF
[binaries]
c = '${CC}'
ar = 'ar'
strip = 'strip'
pkgconfig = 'pkg-config'

[built-in options]
c_args = ['-fPIC']
c_link_args = []

[properties]
sys_root = '${SYSROOT}'
pkg_config_libdir = '${SYSROOT}/usr/lib/pkgconfig:${SYSROOT}/usr/share/pkgconfig'
needs_exe_wrapper = true

[host_machine]
system = 'linux'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
CROSSEOF
    echo "${cross_file}"
}

fetch() {
    local name="$1" url="$2" dir="$3"
    local tarball="${VERIDIAN_SOURCES}/${name}.tar.xz"
    if [[ ! -f "${tarball}" ]]; then
        log "Downloading ${name}..."
        { curl -fsSL -o "${tarball}.part" "${url}" || wget -q -O "${tarball}.part" "${url}"; } && [[ -s "${tarball}.part" ]] && mv "${tarball}.part" "${tarball}" || { rm -f "${tarball}.part"; echo "download failed: ${url}" >&2; exit 1; }
    fi
    if [[ ! -d "${BUILD_DIR}/${dir}" ]]; then
        log "Extracting ${name}..."
        tar -xf "${tarball}" -C "${BUILD_DIR}"
    fi
}

build_dbus() {
    if [[ -f "${SYSROOT}/usr/lib/libdbus-1.a" ]]; then
        log "D-Bus: already installed."
        return 0
    fi
    fetch "dbus-${DBUS_VER}" "${DBUS_URL}" "dbus-${DBUS_VER}"

    local src="${BUILD_DIR}/dbus-${DBUS_VER}"
    local bld="${BUILD_DIR}/dbus-build"
    log "Building D-Bus ${DBUS_VER}..."
    # The options of the old ./configure call, in meson form (XML parsing
    # is expat-only under meson).
    rm -rf "${bld}"
    meson setup "${bld}" "${src}" \
        --cross-file="$(generate_meson_cross)" \
        --prefix="${SYSROOT}/usr" \
        --default-library=static \
        -Dsystemd=disabled \
        -Dlaunchd=disabled \
        -Dselinux=disabled \
        -Dapparmor=disabled \
        -Dlibaudit=disabled \
        -Dkqueue=disabled \
        -Dxml_docs=disabled \
        -Ddoxygen_docs=disabled \
        -Dducktype_docs=disabled \
        -Dqt_help=disabled \
        -Dmodular_tests=disabled \
        -Dinstalled_tests=false \
        -Dx11_autolaunch=disabled \
        -Dsystem_socket=/run/dbus/system_bus_socket \
        -Dsession_socket_dir="${DBUS_SESSION_SOCKET_DIR:-/run/dbus/session}"
    ninja -C "${bld}" -j"${JOBS}"
    ninja -C "${bld}" install
    log "D-Bus: done."
}

# ── Create D-Bus config for VeridianOS ────────────────────────────────
create_dbus_config() {
    local conf_dir="${SYSROOT}/etc/dbus-1"
    mkdir -p "${conf_dir}"

    # Session bus configuration
    cat > "${conf_dir}/session.conf" << 'CONF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/run/dbus/session</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF

    # System bus configuration
    cat > "${conf_dir}/system.conf" << 'CONF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=/run/dbus/system_bus_socket</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
    log "D-Bus configs created."
}

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying D-Bus installation..."
    local errors=0
    for item in \
        "${SYSROOT}/usr/lib/libdbus-1.a" \
        "${SYSROOT}/usr/include/dbus-1.0/dbus/dbus.h" \
        "${SYSROOT}/etc/dbus-1/session.conf" \
        "${SYSROOT}/etc/dbus-1/system.conf" \
    ; do
        if [[ -e "$item" ]]; then
            log "  OK: $(basename "$item")"
        else
            log "  MISSING: $item"
            errors=$((errors + 1))
        fi
    done
    # dbus-daemon may or may not be statically built depending on config
    if [[ -f "${SYSROOT}/usr/bin/dbus-daemon" ]]; then
        log "  OK: dbus-daemon"
    else
        log "  NOTE: dbus-daemon binary not found (may need manual static link)"
    fi
    if [[ $errors -gt 0 ]]; then
        die "${errors} items missing!"
    fi
    log "D-Bus ready."
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Building D-Bus for VeridianOS ==="
    log "Sysroot: ${SYSROOT}"

    [[ -f "${SYSROOT}/usr/lib/libc.a" ]] || die "musl libc not found. Run build-musl.sh first."
    [[ -f "${SYSROOT}/usr/lib/libexpat.a" ]] || die "expat not found. Run build-deps.sh first."

    build_dbus
    create_dbus_config
    verify
    log "=== D-Bus build complete ==="
}

main "$@"

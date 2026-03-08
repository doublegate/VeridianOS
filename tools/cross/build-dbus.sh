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
BUILD_DIR="${PROJECT_ROOT}/target/cross-build/dbus"
SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
JOBS="${JOBS:-$(nproc)}"

DBUS_VER="1.14.10"
DBUS_URL="https://dbus.freedesktop.org/releases/dbus/dbus-${DBUS_VER}.tar.xz"

log() { echo "[build-dbus] $*"; }
die() { echo "[build-dbus] ERROR: $*" >&2; exit 1; }

mkdir -p "${BUILD_DIR}"

export CC="${SYSROOT}/bin/x86_64-veridian-musl-gcc"
export CFLAGS="-O2 --sysroot=${SYSROOT}"
export LDFLAGS="-L${SYSROOT}/usr/lib -static"
export CPPFLAGS="-I${SYSROOT}/usr/include"
export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

COMMON_CONFIGURE=(
    --host=x86_64-veridian
    --prefix="${SYSROOT}/usr"
    --enable-static
    --disable-shared
)

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

build_dbus() {
    if [[ -f "${SYSROOT}/usr/lib/libdbus-1.a" ]]; then
        log "D-Bus: already installed."
        return 0
    fi
    fetch "dbus-${DBUS_VER}" "${DBUS_URL}" "dbus-${DBUS_VER}"

    local src="${BUILD_DIR}/dbus-${DBUS_VER}"
    log "Building D-Bus ${DBUS_VER}..."
    (cd "${src}" && \
        ./configure "${COMMON_CONFIGURE[@]}" \
            --disable-systemd \
            --disable-launchd \
            --disable-selinux \
            --disable-apparmor \
            --disable-libaudit \
            --disable-inotify \
            --disable-kqueue \
            --disable-xml-docs \
            --disable-doxygen-docs \
            --disable-ducktype-docs \
            --disable-tests \
            --without-x \
            --with-xml=expat \
            --with-system-socket=/run/dbus/system_bus_socket \
            --with-session-socket-dir=/tmp/dbus-session && \
        make -j"${JOBS}" && \
        make install)
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
  <listen>unix:tmpdir=/tmp/dbus-session</listen>
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
    build_dbus
    create_dbus_config
    verify
    log "=== D-Bus build complete ==="
}

main "$@"

#!/usr/bin/env bash
# Assemble the KDE Plasma rootfs image for VeridianOS
#
# Stages everything the KDE session needs at run time from the sysroot
# (every package was configured for /usr and /etc on the target and staged
# there, see lib/cross-env.sh): the target programs, musl's dynamic loader
# and the shared libraries, plugins and QML modules (ADR 0010), the data under
# /usr/share (keymaps, ALSA configuration, fonts and fontconfig rules, D-Bus
# configuration and services, libinput quirks, icons, sounds, Plasma
# packages, translations) and the configuration under /etc. Build-only
# files stay behind: headers, pkg-config and CMake files, development links,
# documentation, and scripts that only serve builds. The staging tree
# becomes a BlockFS image (tools/mkfs-blockfs).
#
# Prerequisites: build-all-kde.sh up to the plasma phase.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
SYSROOT="${VERIDIAN_SYSROOT}"
STAGING="${PROJECT_ROOT}/target/rootfs-kde-staging"
OUTPUT="${PROJECT_ROOT}/target/rootfs-kde-blockfs.img"
MKFS_DIR="${PROJECT_ROOT}/tools/mkfs-blockfs"

# Programs the session cannot start without.
REQUIRED_PROGRAMS=(
    usr/bin/kwin_wayland
    usr/bin/plasmashell
    usr/bin/dbus-daemon
    usr/bin/dbus-run-session
)

# Build-only parts of /usr/share and /usr/lib.
EXCLUDED_DATA=(
    share/aclocal share/bash-completion share/cmake share/doc share/ECM
    share/gettext share/gir-1.0 share/gtk-doc share/info share/man
    share/pkgconfig share/vala share/zsh share/qt6/mkspecs
    share/glib-2.0/codegen share/glib-2.0/gdb share/gdb
    lib/cmake lib/pkgconfig lib/qt6/mkspecs lib/qt6/metatypes lib/qt6/modules
    lib/qt6/sbom
)

log() { echo "[rootfs-kde] $*"; }
die() { echo "[rootfs-kde] ERROR: $*" >&2; exit 1; }

prepare_staging() {
    log "Preparing staging directory..."
    rm -rf "${STAGING}"
    mkdir -p "${STAGING}"/{usr/bin,usr/lib,usr/share,etc,run,tmp,var/cache/fontconfig,var/lib/dbus}
}

STRIP="${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-strip"
READELF="${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-readelf"

# stage_elf FILE DEST: FILE, stripped, at DEST under the staging tree.
stage_elf() {
    mkdir -p "$(dirname "${STAGING}/$2")"
    "${STRIP}" --strip-unneeded -o "${STAGING}/$2" "$1" || die "cannot strip $1"
    chmod 755 "${STAGING}/$2"
}

is_elf() { [[ "$(head -c 4 "$1")" == $'\x7fELF' ]]; }

# Every ELF program the build installed (scripts in usr/bin are build
# helpers: *-config, code generators).
copy_programs() {
    log "Copying programs..."
    local dir file count=0
    for dir in usr/bin usr/libexec usr/lib/libexec; do
        [[ -d "${SYSROOT}/${dir}" ]] || continue
        while IFS= read -r -d '' file; do
            if is_elf "${file}"; then
                stage_elf "${file}" "${file#"${SYSROOT}/"}"
                count=$((count + 1))
            fi
        done < <(find "${SYSROOT}/${dir}" -type f -print0)
    done
    log "  ${count} programs."
}

# musl's dynamic loader, and every shared library, plugin and QML plugin
# (ADR 0010). A library directly in /usr/lib goes in once, under its
# SONAME, the name programs ask for: the image builder turns symlinks into
# copies, so the version links (libfoo.so.1 -> libfoo.so.1.2.3) would each
# be a full copy, and the development link (libfoo.so) serves only builds.
# Plugins and modules in subdirectories keep their paths (they are opened
# by path). The loader is musl's libc.so itself, which also answers a
# libc.so dependency, so /usr/lib gets no second copy.
copy_libraries() {
    log "Copying the dynamic loader and shared libraries..."
    stage_elf "${SYSROOT}/usr/lib/libc.so" lib/ld-musl-x86_64.so.1
    local file rel soname count=0
    while IFS= read -r -d '' file; do
        is_elf "${file}" || continue
        rel="${file#"${SYSROOT}/"}"
        [[ "${rel}" == usr/lib/libc.so ]] && continue
        if [[ "$(dirname "${rel}")" == usr/lib ]]; then
            soname="$("${READELF}" -d "${file}" | sed -n 's/.*Library soname: \[\(.*\)\]/\1/p')"
            [[ -n "${soname}" ]] && rel="usr/lib/${soname}"
        fi
        stage_elf "${file}" "${rel}"
        count=$((count + 1))
    done < <(find "${SYSROOT}/usr/lib" -type f \( -name '*.so' -o -name '*.so.*' \) \
                  -not -path '*/libexec/*' -print0)
    log "  ${count} libraries and plugins."
}

# /usr/share, and the run-time data under /usr/lib (QML modules' qmldir and
# QML files, ...), minus the build-only parts; the ELF files are staged
# above.
copy_data() {
    log "Copying data and configuration..."
    local excludes=() item
    for item in "${EXCLUDED_DATA[@]}"; do
        excludes+=(--exclude="/${item}")
    done
    rsync -a "${excludes[@]}" --exclude='*.a' --exclude='*.la' --exclude='*.h' \
        "${SYSROOT}/usr/share" "${STAGING}/usr/"
    rsync -a "${excludes[@]}" --exclude='*.a' --exclude='*.la' --exclude='*.h' \
        --exclude='*.so' --exclude='*.so.*' --exclude='/lib/libexec' --exclude='*.prl' \
        "${SYSROOT}/usr/lib" "${STAGING}/usr/"
    rsync -a "${SYSROOT}/etc/" "${STAGING}/etc/"
    # The session configuration (userland/integration).
    install -Dm644 "${PROJECT_ROOT}/userland/integration/default-session.conf" \
        "${STAGING}/etc/veridian/session.conf"
}

# GLib reads compiled schemas only; the host GLib tools (build-dbus.sh)
# compile the staged ones.
compile_schemas() {
    local dir="${STAGING}/usr/share/glib-2.0/schemas"
    [[ -d "${dir}" ]] || return 0
    "${VERIDIAN_HOST_TOOLS}/bin/glib-compile-schemas" --strict "${dir}" ||
        die "glib-compile-schemas failed"
}

verify() {
    log "Verifying rootfs..."
    local errors=0 item
    for item in "${REQUIRED_PROGRAMS[@]}" lib/ld-musl-x86_64.so.1 \
                etc/veridian/session.conf etc/fonts/fonts.conf etc/ssl/openssl.cnf \
                usr/share/X11/xkb/rules/evdev usr/share/alsa/alsa.conf \
                usr/share/dbus-1/session.conf usr/share/libinput \
                usr/share/fonts/dejavu/DejaVuSans.ttf; do
        if [[ -e "${STAGING}/${item}" ]]; then
            log "  OK: ${item}"
        else
            log "  MISSING: ${item}"
            errors=$((errors + 1))
        fi
    done
    [[ $errors -eq 0 ]] || die "${errors} items missing from the rootfs"
}

build_image() {
    log "Building BlockFS image..."
    local mkfs="${MKFS_DIR}/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs"
    (cd "${MKFS_DIR}" && cargo build --release --target x86_64-unknown-linux-gnu) ||
        die "mkfs-blockfs did not build"
    # Staging plus 50% headroom, at least 256 MB.
    local staging_mb img_mb
    staging_mb=$(( $(du -sb "${STAGING}" | cut -f1) / 1048576 ))
    img_mb=$(( staging_mb * 3 / 2 ))
    (( img_mb >= 256 )) || img_mb=256
    log "  Staging: ${staging_mb} MB, image: ${img_mb} MB"
    "${mkfs}" --populate "${STAGING}" --output "${OUTPUT}" --size "${img_mb}" ||
        die "mkfs-blockfs failed"
    log "Image: ${OUTPUT} ($(stat -c%s "${OUTPUT}") bytes)"
}

print_qemu_cmd() {
    cat << 'QEMU'

=== To boot VeridianOS with KDE Plasma 6 ===

qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive file=target/rootfs-kde-blockfs.img,if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -m 2G -serial stdio

Then at the shell prompt: startgui
QEMU
}

main() {
    log "=== Assembling KDE Plasma rootfs for VeridianOS ==="
    command -v rsync >/dev/null || die "rsync not found"
    prepare_staging
    copy_programs
    copy_libraries
    copy_data
    compile_schemas
    verify
    build_image
    print_qemu_cmd
    log "=== Rootfs assembly complete ==="
}

main "$@"

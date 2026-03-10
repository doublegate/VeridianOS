#!/usr/bin/env bash
# Assemble the KDE Plasma rootfs image for VeridianOS
#
# Creates a BlockFS disk image containing all KDE binaries, libraries,
# fonts, configs, and integration scripts for QEMU boot.
#
# Prerequisites:
#   - All KDE components built (run build-all-kde.sh first)
#   - tools/mkfs-blockfs/ (Rust tool) available

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SYSROOT="${VERIDIAN_SYSROOT:-${PROJECT_ROOT}/target/veridian-sysroot}"
STAGING="${PROJECT_ROOT}/target/rootfs-kde-staging"
OUTPUT="${PROJECT_ROOT}/target/rootfs-kde-blockfs.img"
MKFS_BLOCKFS="${PROJECT_ROOT}/tools/mkfs-blockfs"
JOBS="${JOBS:-$(nproc)}"

log() { echo "[rootfs-kde] $*"; }
die() { echo "[rootfs-kde] ERROR: $*" >&2; exit 1; }

# ── Clean staging area ────────────────────────────────────────────────
prepare_staging() {
    log "Preparing staging directory..."
    rm -rf "${STAGING}"
    mkdir -p "${STAGING}"/{usr/{bin,lib,share},etc,run,tmp}
    mkdir -p "${STAGING}/etc/dbus-1"
    mkdir -p "${STAGING}/etc/fonts"
    mkdir -p "${STAGING}/etc/xdg"
    mkdir -p "${STAGING}/etc/veridian"
    mkdir -p "${STAGING}/run/dbus"
    mkdir -p "${STAGING}/usr/share/fonts/truetype"
    mkdir -p "${STAGING}/usr/share/plasma"
    mkdir -p "${STAGING}/usr/share/kf6"
    mkdir -p "${STAGING}/usr/share/icons"
    mkdir -p "${STAGING}/usr/share/veridian"
}

# ── Copy binaries ────────────────────────────────────────────────────
copy_binaries() {
    log "Copying binaries..."
    local bins=(kwin_wayland kwin_wayland_wrapper kwin_killer_helper plasmashell dbus-daemon)
    for bin in "${bins[@]}"; do
        if [[ -f "${SYSROOT}/usr/bin/${bin}" ]]; then
            cp "${SYSROOT}/usr/bin/${bin}" "${STAGING}/usr/bin/"
            log "  ${bin}: $(stat -c%s "${SYSROOT}/usr/bin/${bin}" 2>/dev/null || echo "?") bytes"
        else
            log "  WARNING: ${bin} not found (optional)"
        fi
    done
}

# ── Copy fonts ───────────────────────────────────────────────────────
copy_fonts() {
    log "Copying fonts..."
    if [[ -d "${SYSROOT}/usr/share/fonts" ]]; then
        cp -r "${SYSROOT}/usr/share/fonts/"* "${STAGING}/usr/share/fonts/" 2>/dev/null || true
    fi
    local count
    count=$(find "${STAGING}/usr/share/fonts" -name '*.ttf' 2>/dev/null | wc -l)
    log "  ${count} font files copied."
}

# ── Copy configs ─────────────────────────────────────────────────────
copy_configs() {
    log "Copying configuration files..."

    # D-Bus configs
    if [[ -d "${SYSROOT}/etc/dbus-1" ]]; then
        cp -r "${SYSROOT}/etc/dbus-1/"* "${STAGING}/etc/dbus-1/"
    fi

    # Fontconfig
    if [[ -f "${SYSROOT}/etc/fonts/fonts.conf" ]]; then
        cp "${SYSROOT}/etc/fonts/fonts.conf" "${STAGING}/etc/fonts/"
    fi

    # VeridianOS session config
    cat > "${STAGING}/etc/veridian/session.conf" << 'CONF'
# VeridianOS session configuration
# Default session type: plasma (KDE Plasma 6)
session=plasma
CONF

    # XDG environment
    cat > "${STAGING}/etc/xdg/plasma-workspace.conf" << 'XDG'
[General]
DesktopSession=plasma
XDG_CURRENT_DESKTOP=KDE
XDG_SESSION_TYPE=wayland
XDG
}

# ── Copy icons and theme data ─────────────────────────────────────────
copy_theme_data() {
    log "Copying theme data..."
    if [[ -d "${SYSROOT}/usr/share/icons/breeze" ]]; then
        cp -r "${SYSROOT}/usr/share/icons/breeze" "${STAGING}/usr/share/icons/"
        log "  Breeze icons copied."
    fi
    if [[ -d "${SYSROOT}/usr/share/plasma" ]]; then
        cp -r "${SYSROOT}/usr/share/plasma/"* "${STAGING}/usr/share/plasma/" 2>/dev/null || true
    fi
}

# ── Copy integration scripts ─────────────────────────────────────────
copy_integration() {
    log "Copying integration scripts..."
    local scripts_dir="${PROJECT_ROOT}/userland/integration"
    if [[ -d "${scripts_dir}" ]]; then
        for script in "${scripts_dir}"/*.sh; do
            [[ -f "$script" ]] || continue
            install -Dm755 "$script" "${STAGING}/usr/share/veridian/$(basename "$script")"
        done
    fi

    # Also copy from sysroot if present
    if [[ -d "${SYSROOT}/usr/share/veridian" ]]; then
        cp "${SYSROOT}/usr/share/veridian/"*.sh "${STAGING}/usr/share/veridian/" 2>/dev/null || true
    fi
}

# ── Build BlockFS image ──────────────────────────────────────────────
build_image() {
    log "Building BlockFS image..."

    if [[ ! -d "${MKFS_BLOCKFS}" ]]; then
        die "mkfs-blockfs tool not found at ${MKFS_BLOCKFS}"
    fi

    # Build mkfs-blockfs if needed
    local mkfs_bin="${MKFS_BLOCKFS}/target/release/mkfs-blockfs"
    if [[ ! -f "${mkfs_bin}" ]]; then
        mkfs_bin="${MKFS_BLOCKFS}/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs"
    fi
    if [[ ! -f "${mkfs_bin}" ]]; then
        log "Building mkfs-blockfs tool..."
        (cd "${MKFS_BLOCKFS}" && cargo build --release)
        mkfs_bin="${MKFS_BLOCKFS}/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs"
        if [[ ! -f "${mkfs_bin}" ]]; then
            mkfs_bin="${MKFS_BLOCKFS}/target/release/mkfs-blockfs"
        fi
    fi

    # Calculate image size (staging + 50% headroom, minimum 256MB)
    local staging_size
    staging_size=$(du -sb "${STAGING}" | awk '{print $1}')
    local img_size=$(( (staging_size * 3 / 2) > 268435456 ? (staging_size * 3 / 2) : 268435456 ))
    local img_size_mb=$(( img_size / 1048576 ))
    log "  Staging: $(( staging_size / 1048576 )) MB, Image: ${img_size_mb} MB"

    "${mkfs_bin}" \
        --populate "${STAGING}" \
        --output "${OUTPUT}" \
        --size "${img_size_mb}" \
        2>&1 || {
        # Fallback: create raw image with tar
        log "  mkfs-blockfs failed, creating tar-based image..."
        tar -cf "${OUTPUT}" -C "${STAGING}" .
    }

    log "Image: ${OUTPUT}"
    log "  Size: $(stat -c%s "${OUTPUT}" 2>/dev/null || echo "?") bytes"
}

# ── Print QEMU launch command ─────────────────────────────────────────
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

# ── Verify ────────────────────────────────────────────────────────────
verify() {
    log "Verifying rootfs..."
    local errors=0
    for item in \
        "${STAGING}/etc/veridian/session.conf" \
    ; do
        if [[ -f "$item" ]]; then
            log "  OK: ${item#${STAGING}}"
        else
            log "  MISSING: ${item#${STAGING}}"
            errors=$((errors + 1))
        fi
    done
    local bin_count
    bin_count=$(find "${STAGING}/usr/bin" -type f 2>/dev/null | wc -l)
    log "  ${bin_count} binaries in rootfs."
    if [[ $errors -gt 0 ]]; then
        log "WARNING: ${errors} items missing."
    fi
}

# ── Main ──────────────────────────────────────────────────────────────
main() {
    log "=== Assembling KDE Plasma rootfs for VeridianOS ==="
    prepare_staging
    copy_binaries
    copy_fonts
    copy_configs
    copy_theme_data
    copy_integration
    verify
    build_image
    print_qemu_cmd
    log "=== Rootfs assembly complete ==="
}

main "$@"

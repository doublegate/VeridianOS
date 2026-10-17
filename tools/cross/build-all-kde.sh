#!/usr/bin/env bash
# Master build script: Cross-compile entire KDE Plasma 6 stack for VeridianOS
#
# Runs all build phases in dependency order:
#   Phase 1: musl libc
#   Phase 1b: musl cross toolchain (GCC, binutils, libstdc++ for musl)
#   Phase 2: C library dependencies (zlib, pcre2, etc.)
#   Phase 3: Wayland libraries
#   Phase 4: Mesa software rendering (softpipe, EGL on Wayland)
#   Phase 5: Font stack (FreeType, HarfBuzz, Fontconfig)
#   Phase 6: D-Bus
#   Phase 7: Qt 6
#   Phase 8: KDE Frameworks 6
#   Phase 9: KWin compositor
#   Phase 10: Plasma Desktop + rootfs assembly
#
# Usage:
#   ./tools/cross/build-all-kde.sh              # Build everything
#   ./tools/cross/build-all-kde.sh --from=mesa   # Resume from Mesa
#   ./tools/cross/build-all-kde.sh --only=qt6    # Build only Qt 6
#   ./tools/cross/build-all-kde.sh --no-snapshot # Skip the archives below
#
# After every completed phase the sysroot is archived to
# ${VERIDIAN_SNAPSHOTS}/sysroot-latest.tar.zst (replaced atomically; the phase
# it covers is in sysroot-latest.txt), and the cross toolchain once per GCC
# version. The default directory is in the workspace Backups/ folder, which
# Syncthing copies to the NAS. To recover after a loss:
#   mkdir -p /opt/veridian && tar --zstd -xf .../sysroot-latest.tar.zst -C /opt/veridian
#   then resume with --from=<the phase after the one in sysroot-latest.txt>.
#
# Environment (locations: tools/cross/veridian-paths.sh):
#   VERIDIAN_SYSROOT    Sysroot path (default: /opt/veridian/musl-sysroot)
#   VERIDIAN_SNAPSHOTS  Archive directory (default: ~/Code/Backups/veridian-builds)
#   JOBS                Parallelism (default: nproc)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=veridian-paths.sh
source "${SCRIPT_DIR}/veridian-paths.sh"
export JOBS="${JOBS:-$(nproc)}"

# Phase names in order
# Wayland before Mesa: Mesa's EGL has a Wayland platform.
PHASES=(musl toolchain x11 deps wayland mesa fonts dbus qt6 kf6 kwin plasma rootfs)

# Map phase names to scripts
declare -A PHASE_SCRIPTS=(
    [musl]="${SCRIPT_DIR}/build-musl.sh"
    [toolchain]="${SCRIPT_DIR}/build-musl-toolchain.sh"
    [deps]="${SCRIPT_DIR}/build-deps.sh"
    [mesa]="${SCRIPT_DIR}/build-mesa.sh"
    [wayland]="${SCRIPT_DIR}/build-wayland.sh"
    [x11]="${SCRIPT_DIR}/build-x11.sh"
    [fonts]="${SCRIPT_DIR}/build-fonts.sh"
    [dbus]="${SCRIPT_DIR}/build-dbus.sh"
    [qt6]="${SCRIPT_DIR}/build-qt6.sh"
    [kf6]="${SCRIPT_DIR}/build-kf6.sh"
    [kwin]="${SCRIPT_DIR}/build-kwin.sh"
    [plasma]="${SCRIPT_DIR}/build-plasma.sh"
    [rootfs]="${SCRIPT_DIR}/build-rootfs-kde.sh"
)

log() { echo ""; echo "========================================"; echo " $*"; echo "========================================"; }
die() { echo "ERROR: $*" >&2; exit 1; }

# ── Parse arguments ───────────────────────────────────────────────────
FROM_PHASE=""
SNAPSHOT=true
VERIDIAN_SNAPSHOTS="${VERIDIAN_SNAPSHOTS:-${HOME}/Code/Backups/veridian-builds}"

# Archive the sysroot after a completed phase (see the header). The new
# archive is written beside the old one (as .tmp, which Syncthing ignores)
# and renamed over it only when complete, so an interrupted snapshot never
# destroys the previous one.
snapshot_sysroot() {
    local phase="$1"
    [[ "${SNAPSHOT}" == "true" ]] || return 0
    command -v zstd >/dev/null || { echo "[snapshot] zstd not found; skipped"; return 0; }
    mkdir -p "${VERIDIAN_SNAPSHOTS}"
    local out="${VERIDIAN_SNAPSHOTS}/sysroot-latest.tar.zst"
    tar -C "$(dirname "${VERIDIAN_SYSROOT}")" --zstd -cf "${out}.tmp" \
        "$(basename "${VERIDIAN_SYSROOT}")"
    mv "${out}.tmp" "${out}"
    printf '%s\t%s\n' "${phase}" "$(date -Is)" > "${VERIDIAN_SNAPSHOTS}/sysroot-latest.txt"
    echo "[snapshot] sysroot after ${phase}: ${out} ($(du -h "${out}" | cut -f1))"

    # The toolchains, once per compiler version: the native one
    # (scripts/build-cross-toolchain.sh) and the musl one this pipeline
    # builds with.
    local dir cc tcout
    for dir in toolchain:x86_64-veridian-gcc "$(basename "${VERIDIAN_TOOLCHAIN}"):${VERIDIAN_TARGET}-gcc"; do
        cc="${VERIDIAN_PREFIX}/${dir%%:*}/bin/${dir#*:}"
        [[ -x "${cc}" ]] || continue
        tcout="${VERIDIAN_SNAPSHOTS}/${dir%%:*}-gcc$("${cc}" -dumpversion).tar.zst"
        if [[ ! -f "${tcout}" ]]; then
            tar -C "${VERIDIAN_PREFIX}" --zstd -cf "${tcout}.tmp" "${dir%%:*}"
            mv "${tcout}.tmp" "${tcout}"
            echo "[snapshot] toolchain: ${tcout}"
        fi
    done
}
ONLY_PHASE=""
for arg in "$@"; do
    case "$arg" in
        --from=*) FROM_PHASE="${arg#--from=}" ;;
        --only=*) ONLY_PHASE="${arg#--only=}" ;;
        --no-snapshot) SNAPSHOT=false ;;
        --help|-h)
            echo "Usage: $0 [--from=PHASE] [--only=PHASE] [--no-snapshot]"
            echo ""
            echo "Phases: ${PHASES[*]}"
            echo ""
            echo "Environment:"
            echo "  VERIDIAN_SYSROOT  Sysroot path (default: /opt/veridian/musl-sysroot; see veridian-paths.sh)"
            echo "  JOBS              Build parallelism (default: $(nproc))"
            exit 0
            ;;
        *) die "Unknown argument: $arg" ;;
    esac
done

# ── Execute phases ────────────────────────────────────────────────────
main() {
    echo "=== VeridianOS KDE Plasma 6 Cross-Compilation ==="
    echo "Sysroot: ${VERIDIAN_SYSROOT}"
    echo "Jobs: ${JOBS}"
    echo ""

    local started=true
    if [[ -n "${FROM_PHASE}" ]]; then
        started=false
    fi

    local start_time
    start_time=$(date +%s)

    for phase in "${PHASES[@]}"; do
        # Handle --from
        if [[ "${started}" == "false" ]]; then
            if [[ "${phase}" == "${FROM_PHASE}" ]]; then
                started=true
            else
                continue
            fi
        fi

        # Handle --only
        if [[ -n "${ONLY_PHASE}" ]] && [[ "${phase}" != "${ONLY_PHASE}" ]]; then
            continue
        fi

        local script="${PHASE_SCRIPTS[$phase]}"
        if [[ ! -f "${script}" ]]; then
            die "Script not found: ${script}"
        fi

        log "Phase: ${phase}"
        local phase_start
        phase_start=$(date +%s)
        bash "${script}"
        local phase_end
        phase_end=$(date +%s)
        local duration=$(( phase_end - phase_start ))
        echo "[${phase}] completed in ${duration}s"
        snapshot_sysroot "${phase}"
    done

    local end_time
    end_time=$(date +%s)
    local total=$(( end_time - start_time ))

    echo ""
    log "BUILD COMPLETE (${total}s total)"
    echo ""
    echo "Next steps:"
    echo "  1. Build kernel:  ./build-kernel.sh x86_64 dev"
    echo "  2. Boot in QEMU with rootfs (see build-rootfs-kde.sh output)"
    echo "  3. At shell prompt:  startgui"
    echo ""
}

main

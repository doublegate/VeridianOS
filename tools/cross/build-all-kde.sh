#!/usr/bin/env bash
# Master build script: Cross-compile entire KDE Plasma 6 stack for VeridianOS
#
# Runs all build phases in dependency order:
#   Phase 1: musl libc
#   Phase 2: C library dependencies (zlib, pcre2, etc.)
#   Phase 3: Mesa software rendering (softpipe)
#   Phase 4: Wayland libraries
#   Phase 5: Font stack (FreeType, HarfBuzz, Fontconfig)
#   Phase 6: D-Bus
#   Phase 7: Qt 6 (static)
#   Phase 8: KDE Frameworks 6
#   Phase 9: KWin compositor
#   Phase 10: Plasma Desktop + rootfs assembly
#
# Usage:
#   ./tools/cross/build-all-kde.sh              # Build everything
#   ./tools/cross/build-all-kde.sh --from=mesa   # Resume from Mesa
#   ./tools/cross/build-all-kde.sh --only=qt6    # Build only Qt 6
#
# Environment:
#   VERIDIAN_SYSROOT  Sysroot path (default: /opt/veridian-sysroot)
#   JOBS              Parallelism (default: nproc)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
export JOBS="${JOBS:-$(nproc)}"

# Phase names in order
PHASES=(musl deps mesa wayland fonts dbus qt6 kf6 kwin plasma rootfs)

# Map phase names to scripts
declare -A PHASE_SCRIPTS=(
    [musl]="${SCRIPT_DIR}/build-musl.sh"
    [deps]="${SCRIPT_DIR}/build-deps.sh"
    [mesa]="${SCRIPT_DIR}/build-mesa.sh"
    [wayland]="${SCRIPT_DIR}/build-wayland.sh"
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
ONLY_PHASE=""
for arg in "$@"; do
    case "$arg" in
        --from=*) FROM_PHASE="${arg#--from=}" ;;
        --only=*) ONLY_PHASE="${arg#--only=}" ;;
        --help|-h)
            echo "Usage: $0 [--from=PHASE] [--only=PHASE]"
            echo ""
            echo "Phases: ${PHASES[*]}"
            echo ""
            echo "Environment:"
            echo "  VERIDIAN_SYSROOT  Sysroot path (default: /opt/veridian-sysroot)"
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

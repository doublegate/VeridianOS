#!/bin/sh
# VeridianOS -- plasma-veridian-session.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# KDE Plasma 6 session startup script for VeridianOS.
#
# Performs the following startup sequence:
#   1. Set XDG environment variables
#   2. Start D-Bus session bus (if not already running)
#   3. Start supporting daemons (kded6, kglobalaccel6)
#   4. Start KWin Wayland compositor
#   5. Start Plasma shell (plasmashell)
#   6. Wait for session termination
#   7. Clean shutdown of all components
#
# Usage:
#   plasma-veridian-session.sh
#
# This script is invoked by the display manager or init system when the
# user selects the "KDE Plasma" session type at login.  The built-in
# VeridianOS compositor (kernel-space) is NOT started when this script
# runs; KWin takes over DRM/KMS directly.
#
# Environment:
#   VERIDIAN_SESSION_LOG  - Log directory (default: /tmp/plasma-session)
#   VERIDIAN_DRM_DEVICE   - DRM device path (default: /dev/dri/card0)
#   PLASMA_DEBUG          - Set to 1 for verbose logging

set -e

# =========================================================================
# Configuration
# =========================================================================

SESSION_NAME="KDE Plasma on VeridianOS"
LOG_DIR="${VERIDIAN_SESSION_LOG:-/tmp/plasma-session}"
DRM_DEVICE="${VERIDIAN_DRM_DEVICE:-/dev/dri/card0}"
PIDS=""

# =========================================================================
# XDG Environment
# =========================================================================

setup_xdg_environment() {
    echo "[plasma-session] Setting up XDG environment..."

    # Runtime directory -- must exist and be user-owned, mode 0700
    UID_NUM="$(id -u)"
    export XDG_RUNTIME_DIR="/run/user/${UID_NUM}"
    if [ ! -d "${XDG_RUNTIME_DIR}" ]; then
        mkdir -p "${XDG_RUNTIME_DIR}"
        chmod 0700 "${XDG_RUNTIME_DIR}"
        chown "${UID_NUM}:$(id -g)" "${XDG_RUNTIME_DIR}"
    fi

    # Standard XDG directories
    export XDG_CONFIG_HOME="${HOME}/.config"
    export XDG_DATA_HOME="${HOME}/.local/share"
    export XDG_CACHE_HOME="${HOME}/.cache"
    export XDG_STATE_HOME="${HOME}/.local/state"

    # XDG system paths -- VeridianOS sysroot layout
    export XDG_CONFIG_DIRS="/etc/xdg"
    export XDG_DATA_DIRS="/usr/share:/usr/local/share"

    # Ensure user directories exist
    mkdir -p "${XDG_CONFIG_HOME}" "${XDG_DATA_HOME}" \
             "${XDG_CACHE_HOME}" "${XDG_STATE_HOME}"

    # Plasma-specific environment
    export XDG_CURRENT_DESKTOP="KDE"
    export XDG_SESSION_TYPE="wayland"
    export XDG_SESSION_DESKTOP="KDE"
    export QT_QPA_PLATFORM="wayland"
    export QT_WAYLAND_DISABLE_WINDOWDECORATION=1
    export PLASMA_USE_QT_SCALING=1

    # KDE paths
    export KDEHOME="${XDG_CONFIG_HOME}/kde"
    export KDEDIR="/usr"

    echo "[plasma-session] XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR}"
    echo "[plasma-session] XDG_CONFIG_HOME=${XDG_CONFIG_HOME}"
    echo "[plasma-session] XDG_DATA_HOME=${XDG_DATA_HOME}"
}

# =========================================================================
# Logging
# =========================================================================

setup_logging() {
    mkdir -p "${LOG_DIR}"
    TIMESTAMP="$(date +%Y%m%d-%H%M%S 2>/dev/null || echo "session")"
    SESSION_LOG="${LOG_DIR}/plasma-session-${TIMESTAMP}.log"
    echo "[plasma-session] Logging to ${SESSION_LOG}"
}

log() {
    echo "[plasma-session] $*"
    if [ -n "${SESSION_LOG}" ]; then
        echo "$(date +%T 2>/dev/null) $*" >> "${SESSION_LOG}" 2>/dev/null || true
    fi
}

log_startup_time() {
    LABEL="$1"
    NOW="$(date +%s%N 2>/dev/null || date +%s)"
    TIMING_LOG="/tmp/veridian-boot-timing.log"
    echo "${LABEL}: ${NOW}" >> "${TIMING_LOG}" 2>/dev/null || true
    log "TIMING ${LABEL}: ${NOW}"
}

# =========================================================================
# D-Bus Session Bus
# =========================================================================

start_dbus_session() {
    log "Starting D-Bus session bus..."

    if [ -n "${DBUS_SESSION_BUS_ADDRESS}" ]; then
        log "D-Bus session bus already running: ${DBUS_SESSION_BUS_ADDRESS}"
        return 0
    fi

    # Launch dbus-daemon and capture its output
    DBUS_LAUNCH_OUTPUT="$(dbus-launch --sh-syntax)"
    if [ $? -ne 0 ]; then
        log "ERROR: dbus-launch failed"
        return 1
    fi

    # Export D-Bus environment variables
    eval "${DBUS_LAUNCH_OUTPUT}"
    export DBUS_SESSION_BUS_ADDRESS
    export DBUS_SESSION_BUS_PID

    log "D-Bus session bus started: PID=${DBUS_SESSION_BUS_PID}"
    log "  Address: ${DBUS_SESSION_BUS_ADDRESS}"
}

# =========================================================================
# KDE Daemons
# =========================================================================

start_kde_daemons() {
    log "Starting KDE background daemons..."

    # kded6 -- KDE Daemon: manages plugins, file indexing, hardware events
    if command -v kded6 >/dev/null 2>&1; then
        kded6 >> "${LOG_DIR}/kded6.log" 2>&1 &
        KDED_PID=$!
        PIDS="${PIDS} ${KDED_PID}"
        log "  kded6 started: PID=${KDED_PID}"
    else
        log "  WARNING: kded6 not found, some KDE features will be unavailable"
    fi

    # kglobalaccel6 -- global keyboard shortcut daemon
    if command -v kglobalaccel6 >/dev/null 2>&1; then
        kglobalaccel6 >> "${LOG_DIR}/kglobalaccel6.log" 2>&1 &
        KGLOBALACCEL_PID=$!
        PIDS="${PIDS} ${KGLOBALACCEL_PID}"
        log "  kglobalaccel6 started: PID=${KGLOBALACCEL_PID}"
    else
        log "  WARNING: kglobalaccel6 not found, global shortcuts unavailable"
    fi

    # kactivitymanagerd -- activity management (optional)
    if command -v kactivitymanagerd >/dev/null 2>&1; then
        kactivitymanagerd >> "${LOG_DIR}/kactivitymanagerd.log" 2>&1 &
        KACTIVITY_PID=$!
        PIDS="${PIDS} ${KACTIVITY_PID}"
        log "  kactivitymanagerd started: PID=${KACTIVITY_PID}"
    fi

    # Short delay for daemons to register on D-Bus
    sleep 1
}

# =========================================================================
# KWin Compositor
# =========================================================================

start_kwin() {
    log "Starting KWin Wayland compositor..."

    KWIN_ARGS="--drm-device ${DRM_DEVICE}"

    # Debug mode: enable verbose logging
    if [ "${PLASMA_DEBUG}" = "1" ]; then
        KWIN_ARGS="${KWIN_ARGS} --log-level debug"
        export QT_LOGGING_RULES="kwin_*=true"
    fi

    kwin_wayland ${KWIN_ARGS} >> "${LOG_DIR}/kwin.log" 2>&1 &
    KWIN_PID=$!
    PIDS="${PIDS} ${KWIN_PID}"
    log "  KWin started: PID=${KWIN_PID}"

    # Wait for KWin to create the Wayland socket
    WAYLAND_SOCKET="${XDG_RUNTIME_DIR}/wayland-0"
    WAIT_COUNT=0
    MAX_WAIT=50  # 5 seconds max
    while [ ! -e "${WAYLAND_SOCKET}" ] && [ ${WAIT_COUNT} -lt ${MAX_WAIT} ]; do
        sleep 0.1
        WAIT_COUNT=$((WAIT_COUNT + 1))
    done

    if [ ! -e "${WAYLAND_SOCKET}" ]; then
        log "ERROR: KWin failed to create Wayland socket after 5s"
        log "  Check ${LOG_DIR}/kwin.log for details"
        return 1
    fi

    export WAYLAND_DISPLAY="wayland-0"
    log "  Wayland socket ready: ${WAYLAND_SOCKET}"
}

# =========================================================================
# Plasma Shell
# =========================================================================

start_plasma_shell() {
    log "Starting Plasma shell..."

    plasmashell >> "${LOG_DIR}/plasmashell.log" 2>&1 &
    PLASMASHELL_PID=$!
    PIDS="${PIDS} ${PLASMASHELL_PID}"
    log "  Plasma shell started: PID=${PLASMASHELL_PID}"

    # Wait briefly for shell to initialize
    sleep 2

    # Verify plasmashell is still running
    if ! kill -0 "${PLASMASHELL_PID}" 2>/dev/null; then
        log "ERROR: Plasma shell exited prematurely"
        log "  Check ${LOG_DIR}/plasmashell.log for details"
        return 1
    fi

    log "  Plasma shell initialized"
}

# =========================================================================
# Session Save / Restore
# =========================================================================

SESSION_RESTORE_BINARY="/usr/lib/veridian/session-save-restore"

save_session_state() {
    log "Saving session state..."

    if [ -x "${SESSION_RESTORE_BINARY}" ]; then
        "${SESSION_RESTORE_BINARY}" --save "${XDG_CONFIG_HOME}" \
            >> "${LOG_DIR}/session-save.log" 2>&1 || true
        log "  Session state saved to ${XDG_CONFIG_HOME}/plasma-session/"
    else
        log "  WARNING: session-save-restore not found, skipping save"
        # Fallback: save running app list manually
        SESSION_SAVE_DIR="${XDG_CONFIG_HOME}/plasma-session"
        mkdir -p "${SESSION_SAVE_DIR}"
        SAVE_FILE="${SESSION_SAVE_DIR}/session-apps.conf"
        echo "# VeridianOS Plasma session state" > "${SAVE_FILE}"
        echo "# Saved at $(date 2>/dev/null || echo 'unknown')" >> "${SAVE_FILE}"
        echo "session_type=plasma" >> "${SAVE_FILE}"
        log "  Fallback: wrote minimal session state"
    fi
}

restore_session_state() {
    log "Checking for saved session..."

    SESSION_SAVE_DIR="${XDG_CONFIG_HOME}/plasma-session"
    SAVE_FILE="${SESSION_SAVE_DIR}/session-apps.conf"

    if [ ! -f "${SAVE_FILE}" ]; then
        log "  No saved session found"
        return 0
    fi

    log "  Found saved session at ${SAVE_FILE}"

    # Check if user wants to restore
    if [ "${VERIDIAN_SESSION_RESTORE:-ask}" = "never" ]; then
        log "  Session restore disabled (VERIDIAN_SESSION_RESTORE=never)"
        return 0
    fi

    if [ -x "${SESSION_RESTORE_BINARY}" ]; then
        "${SESSION_RESTORE_BINARY}" --restore "${XDG_CONFIG_HOME}" \
            >> "${LOG_DIR}/session-restore.log" 2>&1 || true
        log "  Session restored"
    else
        log "  WARNING: session-save-restore not found, skipping restore"
    fi
}

# =========================================================================
# Shutdown Handler
# =========================================================================

cleanup() {
    log "Session shutdown requested..."

    # Save session state before stopping components
    save_session_state

    # Stop components in reverse order
    for COMPONENT in plasmashell kwin_wayland kactivitymanagerd \
                     kglobalaccel6 kded6; do
        if pgrep -x "${COMPONENT}" >/dev/null 2>&1; then
            log "  Stopping ${COMPONENT}..."
            pkill -TERM -x "${COMPONENT}" 2>/dev/null || true
        fi
    done

    # Give processes time to exit gracefully
    sleep 2

    # Force-kill anything still running
    for PID in ${PIDS}; do
        if kill -0 "${PID}" 2>/dev/null; then
            log "  Force-killing PID ${PID}"
            kill -9 "${PID}" 2>/dev/null || true
        fi
    done

    # Stop D-Bus session bus
    if [ -n "${DBUS_SESSION_BUS_PID}" ]; then
        log "  Stopping D-Bus session bus (PID ${DBUS_SESSION_BUS_PID})"
        kill "${DBUS_SESSION_BUS_PID}" 2>/dev/null || true
    fi

    log "Session shutdown complete"
}

# =========================================================================
# Main
# =========================================================================

main() {
    echo "========================================"
    echo "  ${SESSION_NAME}"
    echo "========================================"

    # Register signal handlers for clean shutdown
    trap cleanup EXIT
    trap cleanup INT
    trap cleanup TERM
    trap cleanup HUP

    # Phase 1: Environment
    setup_logging
    log_startup_time "session_env_start"
    setup_xdg_environment
    log_startup_time "session_env_done"

    # Phase 1.5: Start background services in parallel (before KWin)
    # PipeWire, NetworkManager, and BlueZ can initialize while we set
    # up D-Bus and KDE daemons, saving ~3-4s of serial startup time.
    log "Starting background services in parallel..."
    log_startup_time "bg_services_start"

    if [ -x /usr/bin/pipewire ]; then
        /usr/bin/pipewire >> "${LOG_DIR}/pipewire.log" 2>&1 &
        PIPEWIRE_PID=$!
        PIDS="${PIDS} ${PIPEWIRE_PID}"
        log "  PipeWire launched: PID=${PIPEWIRE_PID}"
    fi

    if [ -x /usr/bin/nm-daemon ]; then
        /usr/bin/nm-daemon >> "${LOG_DIR}/nm.log" 2>&1 &
        NM_BG_PID=$!
        PIDS="${PIDS} ${NM_BG_PID}"
        log "  NetworkManager launched: PID=${NM_BG_PID}"
    fi

    if [ -x /usr/bin/bluez-daemon ]; then
        /usr/bin/bluez-daemon >> "${LOG_DIR}/bluez.log" 2>&1 &
        BLUEZ_BG_PID=$!
        PIDS="${PIDS} ${BLUEZ_BG_PID}"
        log "  BlueZ launched: PID=${BLUEZ_BG_PID}"
    fi

    log_startup_time "bg_services_launched"

    # Phase 2: D-Bus
    start_dbus_session
    log_startup_time "dbus_session_ready"

    # Phase 3: KDE daemons
    start_kde_daemons
    log_startup_time "kde_daemons_ready"

    # Phase 4: KWin compositor
    start_kwin
    log_startup_time "kwin_ready"

    # Phase 5: Plasma shell
    start_plasma_shell
    log_startup_time "plasma_shell_ready"

    # Phase 5.5: Deferred Baloo startup (30s delay to avoid I/O storm)
    if command -v baloo_file >/dev/null 2>&1; then
        (
            sleep 30
            log "Starting Baloo file indexer (deferred)..."
            baloo_file >> "${LOG_DIR}/baloo.log" 2>&1
        ) &
        log "  Baloo scheduled: 30s deferred start"
    fi

    # Phase 6: Restore previous session (if any)
    restore_session_state

    log_startup_time "session_fully_started"
    log "Plasma session fully started"
    echo "========================================"
    echo "  Session running -- PID $$"
    echo "  Log: ${SESSION_LOG}"
    echo "========================================"

    # Wait for the session to end.
    # The session ends when KWin exits (compositor is the anchor process).
    wait "${KWIN_PID}" 2>/dev/null || true

    log "KWin exited -- session ending"
}

main "$@"

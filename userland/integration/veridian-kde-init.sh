#!/bin/sh
# VeridianOS -- veridian-kde-init.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Init system integration for KDE Plasma 6 boot sequence on VeridianOS.
#
# This script is executed by PID 1 (init) early in the boot process.  It
# handles:
#   1. Starting the D-Bus system bus daemon
#   2. Starting the logind shim daemon
#   3. Setting up XDG_RUNTIME_DIR for the login user
#   4. Reading session type from /etc/veridian/session.conf
#   5. Dispatching to KDE Plasma or built-in DE accordingly
#   6. Clean shutdown in reverse order
#   7. Boot-to-desktop timing measurement
#
# Usage:
#   veridian-kde-init.sh
#
# This script is sourced or exec'd from init after the kernel has
# finished bootstrap.  It expects a minimal POSIX environment with
# /bin/sh, mkdir, cat, kill, date, and id.
#
# Configuration:
#   /etc/veridian/session.conf -- Session type ("builtin" or "plasma")

set -e

# =========================================================================
# Constants
# =========================================================================

SESSION_CONF="/etc/veridian/session.conf"
DBUS_SYSTEM_SOCKET="/run/dbus/system_bus_socket"
DBUS_SYSTEM_PID_FILE="/run/dbus/pid"
LOGIND_PID_FILE="/run/veridian-logind.pid"
BOOT_TIMING_LOG="/tmp/veridian-boot-timing.log"

DBUS_SYSTEM_PID=""
LOGIND_PID=""
SESSION_PID=""

# =========================================================================
# Logging
# =========================================================================

log() {
    echo "[kde-init] $*"
}

log_time() {
    LABEL="$1"
    NOW="$(date +%s%N 2>/dev/null || date +%s)"
    echo "${LABEL}: ${NOW}" >> "${BOOT_TIMING_LOG}"
    log "TIMING ${LABEL}: ${NOW}"
}

# =========================================================================
# D-Bus System Bus
# =========================================================================

start_dbus_system() {
    log "Starting D-Bus system bus..."

    # Create required directories
    mkdir -p /run/dbus
    mkdir -p /etc/dbus-1/system.d

    # Write minimal system bus config if missing
    if [ ! -f /etc/dbus-1/system.conf ]; then
        log "  Creating default D-Bus system bus configuration"
        mkdir -p /etc/dbus-1
        cat > /etc/dbus-1/system.conf << 'DBUSCONF'
<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=/run/dbus/system_bus_socket</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
  <includedir>/etc/dbus-1/system.d</includedir>
</busconfig>
DBUSCONF
    fi

    # Start the system bus daemon
    if [ -x /usr/bin/dbus-daemon ]; then
        /usr/bin/dbus-daemon --system --fork \
            --address="unix:path=${DBUS_SYSTEM_SOCKET}" \
            --print-pid > "${DBUS_SYSTEM_PID_FILE}" 2>/dev/null

        if [ -f "${DBUS_SYSTEM_PID_FILE}" ]; then
            DBUS_SYSTEM_PID="$(cat "${DBUS_SYSTEM_PID_FILE}")"
            log "  D-Bus system bus started: PID=${DBUS_SYSTEM_PID}"
        else
            log "  WARNING: D-Bus system bus PID file not created"
        fi

        export DBUS_SYSTEM_BUS_ADDRESS="unix:path=${DBUS_SYSTEM_SOCKET}"
    else
        log "  WARNING: dbus-daemon not found at /usr/bin/dbus-daemon"
        log "  KDE Plasma requires D-Bus -- session may fail"
    fi
}

stop_dbus_system() {
    if [ -n "${DBUS_SYSTEM_PID}" ]; then
        log "  Stopping D-Bus system bus (PID ${DBUS_SYSTEM_PID})..."
        kill "${DBUS_SYSTEM_PID}" 2>/dev/null || true
        rm -f "${DBUS_SYSTEM_PID_FILE}" "${DBUS_SYSTEM_SOCKET}"
    fi
}

# =========================================================================
# logind Shim
# =========================================================================

start_logind() {
    log "Starting logind shim..."

    if [ -x /usr/libexec/veridian-logind ]; then
        /usr/libexec/veridian-logind --daemon \
            --pid-file="${LOGIND_PID_FILE}" 2>/dev/null

        if [ -f "${LOGIND_PID_FILE}" ]; then
            LOGIND_PID="$(cat "${LOGIND_PID_FILE}")"
            log "  logind shim started: PID=${LOGIND_PID}"
        else
            log "  WARNING: logind PID file not created"
        fi
    else
        log "  WARNING: veridian-logind not found"
        log "  Session management will be limited"
    fi
}

stop_logind() {
    if [ -n "${LOGIND_PID}" ]; then
        log "  Stopping logind shim (PID ${LOGIND_PID})..."
        kill "${LOGIND_PID}" 2>/dev/null || true
        rm -f "${LOGIND_PID_FILE}"
    fi
}

# =========================================================================
# XDG Runtime Directory
# =========================================================================

setup_xdg_runtime() {
    UID_NUM="$(id -u 2>/dev/null || echo 0)"
    XDG_RUNTIME_DIR="/run/user/${UID_NUM}"

    log "Setting up XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR}"

    mkdir -p "${XDG_RUNTIME_DIR}"
    chmod 0700 "${XDG_RUNTIME_DIR}"
    if [ "${UID_NUM}" != "0" ]; then
        chown "${UID_NUM}:$(id -g)" "${XDG_RUNTIME_DIR}" 2>/dev/null || true
    fi

    export XDG_RUNTIME_DIR
}

# =========================================================================
# Session Type Detection
# =========================================================================

detect_session_type() {
    SESSION_TYPE="builtin"

    if [ -f "${SESSION_CONF}" ]; then
        CONF_TYPE="$(cat "${SESSION_CONF}" 2>/dev/null | \
            sed -n 's/^session_type=//p' | head -1)"
        case "${CONF_TYPE}" in
            plasma|kde)
                SESSION_TYPE="plasma"
                ;;
            builtin|default|"")
                SESSION_TYPE="builtin"
                ;;
            *)
                log "  WARNING: Unknown session type '${CONF_TYPE}', defaulting to builtin"
                SESSION_TYPE="builtin"
                ;;
        esac
    else
        log "  No session config found, defaulting to built-in DE"
    fi

    log "Session type: ${SESSION_TYPE}"
}

# =========================================================================
# Session Launch
# =========================================================================

launch_session() {
    case "${SESSION_TYPE}" in
        plasma)
            launch_plasma_session
            ;;
        builtin)
            launch_builtin_session
            ;;
    esac
}

launch_plasma_session() {
    log "Launching KDE Plasma 6 session..."
    log_time "plasma_session_start"

    # Verify prerequisites
    if [ ! -e "${DBUS_SYSTEM_SOCKET}" ]; then
        log "ERROR: D-Bus system bus not available -- cannot start Plasma"
        log "Falling back to built-in DE"
        launch_builtin_session
        return
    fi

    # The plasma-veridian-session.sh script handles:
    #   D-Bus session bus -> KDE daemons -> KWin -> Plasma shell
    if [ -x /usr/bin/plasma-veridian-session ]; then
        /usr/bin/plasma-veridian-session &
        SESSION_PID=$!
        log "  Plasma session launched: PID=${SESSION_PID}"

        # Wait for KWin to signal readiness (Wayland socket creation)
        WAYLAND_SOCKET="${XDG_RUNTIME_DIR}/wayland-0"
        WAIT=0
        while [ ! -e "${WAYLAND_SOCKET}" ] && [ ${WAIT} -lt 100 ]; do
            sleep 0.1
            WAIT=$((WAIT + 1))
        done

        if [ -e "${WAYLAND_SOCKET}" ]; then
            log_time "plasma_desktop_ready"
            BOOT_START="$(sed -n 's/^kwin_start: //p' "${BOOT_TIMING_LOG}" 2>/dev/null)"
            BOOT_END="$(sed -n 's/^plasma_desktop_ready: //p' "${BOOT_TIMING_LOG}" 2>/dev/null)"
            if [ -n "${BOOT_START}" ] && [ -n "${BOOT_END}" ]; then
                log "  Boot-to-desktop estimate logged"
            fi
        else
            log "  WARNING: Wayland socket not created after 10s"
        fi
    else
        log "ERROR: plasma-veridian-session not found at /usr/bin/"
        log "Falling back to built-in DE"
        launch_builtin_session
    fi
}

launch_builtin_session() {
    log "Launching built-in VeridianOS desktop..."
    log_time "builtin_session_start"

    # The built-in DE runs inside the kernel compositor.
    # Nothing to launch here -- the kernel's desktop module handles
    # everything.  We just need to spawn a shell for the user.
    if [ -x /bin/sh ]; then
        /bin/sh &
        SESSION_PID=$!
        log "  Shell spawned: PID=${SESSION_PID}"
    fi

    log_time "builtin_session_ready"
}

# =========================================================================
# Shutdown
# =========================================================================

shutdown_handler() {
    log "Shutdown requested -- cleaning up..."
    log_time "shutdown_start"

    # 1. Stop the session (Plasma or built-in shell)
    if [ -n "${SESSION_PID}" ]; then
        log "  Stopping session (PID ${SESSION_PID})..."
        kill -TERM "${SESSION_PID}" 2>/dev/null || true
        # Give Plasma time to save state and shut down KWin
        WAIT=0
        while kill -0 "${SESSION_PID}" 2>/dev/null && [ ${WAIT} -lt 50 ]; do
            sleep 0.1
            WAIT=$((WAIT + 1))
        done
        # Force kill if still running
        kill -9 "${SESSION_PID}" 2>/dev/null || true
    fi

    # 2. Stop logind
    stop_logind

    # 3. Stop D-Bus system bus (last -- others depend on it)
    stop_dbus_system

    log_time "shutdown_complete"
    log "Clean shutdown complete"
}

# =========================================================================
# Main
# =========================================================================

main() {
    log "========================================="
    log "  VeridianOS KDE Init System"
    log "========================================="

    # Register shutdown handler
    trap shutdown_handler EXIT
    trap shutdown_handler INT
    trap shutdown_handler TERM

    # Clear timing log
    : > "${BOOT_TIMING_LOG}"
    log_time "init_start"

    # Phase 1: System services
    start_dbus_system
    log_time "dbus_system_ready"

    start_logind
    log_time "logind_ready"

    # Phase 2: Runtime environment
    setup_xdg_runtime

    # Phase 3: Detect and launch session
    detect_session_type
    log_time "kwin_start"
    launch_session

    # Phase 4: Wait for session to end
    if [ -n "${SESSION_PID}" ]; then
        log "Waiting for session (PID ${SESSION_PID}) to exit..."
        wait "${SESSION_PID}" 2>/dev/null || true
        log "Session exited"
    fi
}

main "$@"

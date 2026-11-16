# shellcheck shell=bash
# fetch_verified DEST SHA256 URL... : download into DEST from the first URL
# that serves a file with the given SHA-256, trying each in turn.
#
# Each URL gets a bounded connect time, a stall limit once connected, and a
# few retries, and a file is accepted only if its checksum matches, so a
# mirror can only stand in for the primary with the identical file.
# busybox.net was unreachable from a CI runner for over two minutes on
# 2026-11-16, which failed the rootfs job on main with nothing wrong in the
# tree.
#
# DEST is kept across attempts (curl overwrites it), so a file the caller
# reserved with mktemp keeps its name and mode; it is removed only when
# every source has failed.
#
# Sourced by scripts/build-busybox-rootfs.sh and scripts/build-musl-rootfs.sh.
# Returns 0 with DEST in place, or 1 with DEST removed.
fetch_verified() {
    local dest="$1" sha="$2" url left
    shift 2
    left=$#
    for url in "$@"; do
        left=$((left - 1))
        echo "  Fetching ${url}" >&2
        # --speed-limit/--speed-time: a server that accepts the connection
        # and then stalls (under 1 KB/s for 30 s) fails over too.
        if curl -fsSL --connect-timeout 30 --speed-limit 1024 --speed-time 30 \
            --retry 3 --retry-delay 5 --retry-all-errors -o "${dest}" "${url}"; then
            if echo "${sha}  ${dest}" | sha256sum -c --quiet - >/dev/null 2>&1; then
                return 0
            fi
            echo "  WARNING: checksum mismatch from ${url}" >&2
        else
            echo "  WARNING: download from ${url} failed" >&2
        fi
        [ "${left}" -gt 0 ] && echo "  Trying the next source" >&2
    done
    rm -f -- "${dest}"
    return 1
}

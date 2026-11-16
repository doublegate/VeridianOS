#!/usr/bin/env bash
# fetch_verified DEST SHA256 URL... : download into DEST from the first URL
# that serves a file with the given SHA-256, trying each in turn.
#
# Each URL gets a bounded connect timeout and a few retries, and a file is
# accepted only if its checksum matches, so a mirror can only stand in for
# the primary with the identical file. busybox.net was unreachable from a CI
# runner for over two minutes on 2026-11-16, which failed the rootfs job on
# main with nothing wrong in the tree.
#
# Sourced by scripts/build-busybox-rootfs.sh and scripts/build-musl-rootfs.sh.
# Returns 0 with DEST in place, or 1 with DEST removed.
fetch_verified() {
    local dest="$1" sha="$2" url
    shift 2
    for url in "$@"; do
        echo "  Fetching ${url}" >&2
        if curl -fsSL --connect-timeout 30 --retry 3 --retry-delay 5 \
            --retry-all-errors -o "${dest}" "${url}"; then
            if echo "${sha}  ${dest}" | sha256sum -c --quiet - >/dev/null 2>&1; then
                return 0
            fi
            echo "  WARNING: checksum mismatch from ${url}; trying the next source" >&2
        else
            echo "  WARNING: download from ${url} failed; trying the next source" >&2
        fi
        rm -f -- "${dest}"
    done
    return 1
}

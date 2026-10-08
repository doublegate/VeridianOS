# shellcheck shell=bash
# Where the cross-built artifacts live. Sourced by every tools/cross script.
#
# Deliberately NOT under target/: Cargo owns that directory and `cargo clean`
# deletes it whole, which on 2026-10-07 took the musl/KDE sysroot and the
# 36 GB of KDE build trees with it. /opt/veridian already holds the cross
# toolchain (scripts/build-cross-toolchain.sh), which survived.
#
# Each location can be overridden from the environment.

VERIDIAN_PREFIX="${VERIDIAN_PREFIX:-/opt/veridian}"
# This directory (the build scripts, toolchain files and patches).
VERIDIAN_CROSS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# musl libc plus every library the KDE stack installs. (Not
# /opt/veridian/sysroot/<arch>: that is the generic toolchain/ files' layout.)
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-${VERIDIAN_PREFIX}/musl-sysroot}"
# Per-phase build trees (build-<phase>.sh uses ${VERIDIAN_CROSS_BUILD}/<phase>).
VERIDIAN_CROSS_BUILD="${VERIDIAN_CROSS_BUILD:-${VERIDIAN_PREFIX}/cross-build}"
# Downloaded source tarballs, kept apart from the build trees so a clean
# rebuild never downloads again.
VERIDIAN_SOURCES="${VERIDIAN_SOURCES:-${VERIDIAN_PREFIX}/sources}"

# The cross toolchain every phase after musl compiles with
# (build-musl-toolchain.sh): GCC and binutils for this triple, with
# libstdc++ built for the musl in the sysroot.
VERIDIAN_TARGET="${VERIDIAN_TARGET:-x86_64-veridian-linux-musl}"
VERIDIAN_TOOLCHAIN="${VERIDIAN_TOOLCHAIN:-${VERIDIAN_PREFIX}/musl-toolchain}"
VERIDIAN_CC="${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-gcc"
VERIDIAN_CXX="${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-g++"

# Programs the build runs on the host (code generators such as
# wayland-scanner), apart from the target sysroot. Native meson
# dependencies (dependency(..., native: true)) see its .pc files.
VERIDIAN_HOST_TOOLS="${VERIDIAN_HOST_TOOLS:-${VERIDIAN_PREFIX}/host-tools}"
# Runs a VeridianOS program on the build host -- a code generator a build
# compiled, or a configure check -- through musl's dynamic loader with the
# sysroot's libraries (written by lib/cross-env.sh; ADR 0010).
VERIDIAN_RUN_TARGET="${VERIDIAN_HOST_TOOLS}/bin/veridian-run-target"

# Exported, so the CMake toolchain files (which fall back to /opt/veridian)
# and every child build see the same locations: with VERIDIAN_PREFIX set
# elsewhere, an unexported toolchain path silently meant the default one.
export VERIDIAN_PREFIX VERIDIAN_CROSS_DIR VERIDIAN_SYSROOT VERIDIAN_HOST_TOOLS VERIDIAN_CROSS_BUILD VERIDIAN_SOURCES
export VERIDIAN_TARGET VERIDIAN_TOOLCHAIN VERIDIAN_CC VERIDIAN_CXX VERIDIAN_RUN_TARGET
export VERIDIAN_TARGET VERIDIAN_TOOLCHAIN VERIDIAN_CC VERIDIAN_CXX VERIDIAN_RUN_TARGET
# Autotools finds ${VERIDIAN_TARGET}-ar, -ranlib, -strip and the rest by
# name on PATH.
for _veridian_bin in "${VERIDIAN_TOOLCHAIN}/bin" "${VERIDIAN_HOST_TOOLS}/bin"; do
    case ":${PATH}:" in
        *":${_veridian_bin}:"*) ;;
        *) export PATH="${_veridian_bin}:${PATH}" ;;
    esac
done
unset _veridian_bin
export PKG_CONFIG_PATH_FOR_BUILD="${VERIDIAN_HOST_TOOLS}/lib/pkgconfig:${VERIDIAN_HOST_TOOLS}/share/pkgconfig"
mkdir -p "${VERIDIAN_SOURCES}" "${VERIDIAN_CROSS_BUILD}"

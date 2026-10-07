# Where the cross-built artifacts live. Sourced by every tools/cross script.
#
# Deliberately NOT under target/: Cargo owns that directory and `cargo clean`
# deletes it whole, which on 2026-10-07 took the musl/KDE sysroot and the
# 36 GB of KDE build trees with it. /opt/veridian already holds the cross
# toolchain (scripts/build-cross-toolchain.sh), which survived.
#
# Each location can be overridden from the environment.

VERIDIAN_PREFIX="${VERIDIAN_PREFIX:-/opt/veridian}"
# musl libc plus every library the KDE stack installs. (Not
# /opt/veridian/sysroot/<arch>: that is the generic toolchain/ files' layout.)
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-${VERIDIAN_PREFIX}/musl-sysroot}"
# Per-phase build trees (build-<phase>.sh uses ${VERIDIAN_CROSS_BUILD}/<phase>).
VERIDIAN_CROSS_BUILD="${VERIDIAN_CROSS_BUILD:-${VERIDIAN_PREFIX}/cross-build}"
# Downloaded source tarballs, kept apart from the build trees so a clean
# rebuild never downloads again.
VERIDIAN_SOURCES="${VERIDIAN_SOURCES:-${VERIDIAN_PREFIX}/sources}"

export VERIDIAN_PREFIX VERIDIAN_SYSROOT VERIDIAN_CROSS_BUILD VERIDIAN_SOURCES
mkdir -p "${VERIDIAN_SOURCES}" "${VERIDIAN_CROSS_BUILD}"

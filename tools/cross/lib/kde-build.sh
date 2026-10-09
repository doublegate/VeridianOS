# shellcheck shell=bash
# Helpers shared by the KDE phases (build-kf6.sh, build-kwin.sh,
# build-plasma.sh); sourced after lib/cross-env.sh.

# kde_fetch TARBALL_BASE URL_DIR LIST: a fresh tree checked against
# checksums/LIST (KDE's published checksums); its path is echoed.
kde_fetch() {
    local base="$1" url_dir="$2" list="$3"
    fetch "${base}.tar.xz" "${url_dir}/${base}.tar.xz" "${base}" \
        "$(listed_sha256 "${list}" "${base}.tar.xz")" >&2
    echo "${BUILD_DIR}/${base}"
}

# kde_cmake NAME SRC [OPTIONS...]: configure, build and stage one KDE
# CMake project, shared (ADR 0010), with the Qt/KDE toolchain and the
# stack's defaults (cmake/kde-defaults.cmake: no tests or docs, the target
# Qt's plugin, QML and metatypes directories, no X11, CMake 4 policies);
# OPTIONS override them.
kde_cmake() {
    local name="$1" src="$2"
    shift 2
    log "Building ${name}..."
    CMAKE_TOOLCHAIN="${CMAKE_TOOLCHAIN_KDE}" cmake_build "${src}" "${BUILD_DIR}/${name}-build" \
        -C "${VERIDIAN_CROSS_DIR}/cmake/kde-defaults.cmake" \
        -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
        "$@"
}

# component_installed NAME STAMP: NAME was built as STAMP (version and
# options) and every file its install wrote (CMake's install manifest,
# which lists them without the DESTDIR) is still in the sysroot.
component_installed() {
    local bld="${BUILD_DIR}/$1-build" file
    [[ "$(cat "${bld}/.veridian-version" 2>/dev/null)" == "$2" && -s "${bld}/install_manifest.txt" ]] || return 1
    while IFS= read -r file; do
        [[ -e "${SYSROOT}${file}" || -L "${SYSROOT}${file}" ]] || return 1
    done < "${bld}/install_manifest.txt"
    log "$1: already installed."
}

# plasma_url: where the Plasma release PLASMA_VER is published.
plasma_url() {
    echo "https://download.kde.org/stable/plasma/${PLASMA_VER}"
}

# plasma NAME [CMAKE OPTIONS...]: build and stage one Plasma component
# (release PLASMA_VER, from plasma_url), with its patches (plasma-patches/NAME/,
# each explaining itself) applied to the fresh source tree. Always in the
# Plasma phase's build directory, so a component built early by another
# phase (libplasma and milou, which KWin's QML needs) is found there and
# not built twice.
plasma() {
    local name="$1"
    shift
    local BUILD_DIR="${VERIDIAN_CROSS_BUILD}/plasma"
    mkdir -p "${BUILD_DIR}"
    local stamp="${PLASMA_VER} $*"
    component_installed "${name}" "${stamp}" && return 0
    local src
    src="$(kde_fetch "${name}-${PLASMA_VER}" "$(plasma_url)" plasma.sha256)"
    apply_patches "${VERIDIAN_CROSS_DIR}/plasma-patches/${name}" "${src}"
    kde_cmake "${name}" "${src}" "$@"
    echo "${stamp}" > "${BUILD_DIR}/${name}-build/.veridian-version"
}

# host_python: a Python for the build host with the modules the KDE builds
# need (host-python-requirements.txt, hash-pinned), in the host tools; its
# interpreter is ${HOST_PYTHON}. Rebuilt when the requirements or the host
# Python change.
HOST_PYTHON="${VERIDIAN_HOST_TOOLS}/python/bin/python3"
host_python() {
    local venv="${VERIDIAN_HOST_TOOLS}/python"
    local req="${VERIDIAN_CROSS_DIR}/host-python-requirements.txt"
    local stamp
    stamp="$(python3 --version) $(sha256sum < "${req}")"
    if [[ -x "${HOST_PYTHON}" && -f "${venv}/veridian-stamp" &&
          "$(cat "${venv}/veridian-stamp")" == "${stamp}" ]]; then
        return 0
    fi
    log "Setting up the host Python (${venv})..."
    rm -rf "${venv}"
    python3 -m venv "${venv}"
    "${HOST_PYTHON}" -m pip install --quiet --disable-pip-version-check \
        --require-hashes --no-deps -r "${req}"
    echo "${stamp}" > "${venv}/veridian-stamp"
}

# The CMake config a framework installs: KF6<Name> where Name drops a
# leading K (KConfig -> KF6Config), except where the K belongs to an
# acronym (KCMUtils -> KF6KCMUtils, KIO -> KF6KIO).
kf_config() {
    local name="$1" dir
    for dir in "KF6${name#K}" "KF6${name}"; do
        if [[ -f "${SYSROOT}/usr/lib/cmake/${dir}/${dir}Config.cmake" ]]; then
            echo "${dir}"
            return 0
        fi
    done
    return 1
}

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
# CMake project, shared (ADR 0010), with the Qt/KDE toolchain. Plugins, QML modules
# and metatypes go into the target Qt's directories (build-qt6.sh lays Qt
# out under /usr/lib/qt6). They are given rather than queried: ECM's query
# (KDE_INSTALL_USE_QT_SYS_PATHS) runs the host Qt's qtpaths, which answers
# with the host Qt's directories.
kde_cmake() {
    local name="$1" src="$2"
    shift 2
    log "Building ${name}..."
    CMAKE_TOOLCHAIN="${CMAKE_TOOLCHAIN_KDE}" cmake_build "${src}" "${BUILD_DIR}/${name}-build" \
        -DBUILD_TESTING=OFF \
        -DBUILD_QCH=OFF \
        -DBUILD_PYTHON_BINDINGS=OFF \
        -DBUILD_DESIGNERPLUGIN=OFF \
        -DKDE_INSTALL_USE_QT_SYS_PATHS=OFF \
        -DKDE_INSTALL_QTPLUGINDIR=lib/qt6/plugins \
        -DKDE_INSTALL_QMLDIR=lib/qt6/qml \
        -DKDE_INSTALL_QTMETATYPESDIR=lib/qt6/metatypes \
        -DWITH_X11=OFF \
        -DCMAKE_IGNORE_PREFIX_PATH="${CMAKE_IGNORE_PREFIX_PATH:-/home/linuxbrew/.linuxbrew}" \
        "$@"
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

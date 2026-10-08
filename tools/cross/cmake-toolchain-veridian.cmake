# CMake toolchain file for the Qt and KDE phases (VeridianOS x86_64, musl)
#
# The base toolchain (compiler, sysroot, search modes, pkg-config) plus
# the host tools these builds run: the host Qt (moc, rcc, uic, qmlcachegen,
# qtwaylandscanner, ...) and KDE's generators, from ${VERIDIAN_HOST_TOOLS}.
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=tools/cross/cmake-toolchain-veridian.cmake ..

include("${CMAKE_CURRENT_LIST_DIR}/cmake-toolchain-veridian-deps.cmake")

set(VERIDIAN_HOST_TOOLS "$ENV{VERIDIAN_HOST_TOOLS}")
if(NOT VERIDIAN_HOST_TOOLS)
    set(VERIDIAN_HOST_TOOLS "/opt/veridian/host-tools")
endif()

# The host Qt (build-qt6.sh) that runs Qt's code generators.
set(QT_HOST_PATH "${VERIDIAN_HOST_TOOLS}/qt6" CACHE PATH "Host Qt for cross-compilation")
set(QT_HOST_PATH_CMAKE_DIR "${VERIDIAN_HOST_TOOLS}/qt6/lib/cmake" CACHE PATH "Host Qt CMake directory")

# Tools other packages installed into the sysroot (kconfig_compiler, ...)
# run on this host through the target launcher (cmake/VeridianTargetTools.cmake).
set(CMAKE_PROJECT_INCLUDE "${CMAKE_CURRENT_LIST_DIR}/cmake/VeridianTargetTools.cmake")

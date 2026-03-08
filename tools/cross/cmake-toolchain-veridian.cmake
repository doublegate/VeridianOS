# CMake toolchain file for cross-compiling to VeridianOS (x86_64)
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=tools/cross/cmake-toolchain-veridian.cmake ..

set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

# Sysroot -- populated by build-musl.sh and subsequent dependency builds
set(VERIDIAN_SYSROOT "$ENV{VERIDIAN_SYSROOT}")
if(NOT VERIDIAN_SYSROOT)
    # Default to in-tree sysroot
    get_filename_component(_toolchain_dir "${CMAKE_CURRENT_LIST_DIR}" DIRECTORY)
    get_filename_component(_project_root "${_toolchain_dir}" DIRECTORY)
    set(VERIDIAN_SYSROOT "${_project_root}/target/veridian-sysroot")
endif()

set(CMAKE_SYSROOT "${VERIDIAN_SYSROOT}")

# Cross-compiler (musl-gcc wrapper)
set(CMAKE_C_COMPILER "${VERIDIAN_SYSROOT}/bin/x86_64-veridian-musl-gcc")
# No C++ cross-compiler yet; will be added when needed for Qt6
# set(CMAKE_CXX_COMPILER "${VERIDIAN_SYSROOT}/bin/x86_64-veridian-musl-g++")
set(CMAKE_AR "ar" CACHE FILEPATH "Archiver")
set(CMAKE_RANLIB "ranlib" CACHE FILEPATH "Ranlib")

# Search paths
set(CMAKE_FIND_ROOT_PATH "${VERIDIAN_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# Static linking by default
set(BUILD_SHARED_LIBS OFF CACHE BOOL "Build static libraries")

# Pkg-config
set(ENV{PKG_CONFIG_PATH} "${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${VERIDIAN_SYSROOT}")

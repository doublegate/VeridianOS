# Minimal CMake toolchain file for cross-compiling simple C deps to VeridianOS
# Unlike cmake-toolchain-veridian.cmake, this does NOT add KDE/Mesa/Wayland
# link-time dependencies, making it suitable for building foundational libs
# like libjpeg-turbo, zlib, etc.

set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_CROSSCOMPILING ON CACHE BOOL "Cross-compiling to VeridianOS" FORCE)

set(VERIDIAN_SYSROOT "$ENV{VERIDIAN_SYSROOT}")
if(NOT VERIDIAN_SYSROOT)
    get_filename_component(_tools_dir "${CMAKE_CURRENT_LIST_DIR}" DIRECTORY)
    get_filename_component(_project_root "${_tools_dir}" DIRECTORY)
    set(VERIDIAN_SYSROOT "${_project_root}/target/veridian-sysroot")
endif()

set(CMAKE_SYSROOT "${VERIDIAN_SYSROOT}")
set(CMAKE_C_COMPILER "${VERIDIAN_SYSROOT}/bin/x86_64-veridian-musl-gcc")
set(CMAKE_CXX_COMPILER "${VERIDIAN_SYSROOT}/bin/x86_64-veridian-musl-g++")
set(CMAKE_AR "ar" CACHE FILEPATH "Archiver")
set(CMAKE_RANLIB "ranlib" CACHE FILEPATH "Ranlib")

set(CMAKE_FIND_ROOT_PATH "${VERIDIAN_SYSROOT}" "${VERIDIAN_SYSROOT}/usr")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

set(BUILD_SHARED_LIBS OFF CACHE BOOL "Build static libraries")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

set(ENV{PKG_CONFIG_LIBDIR} "${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_PATH} "")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "")

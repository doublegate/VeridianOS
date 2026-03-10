# CMake toolchain file for cross-compiling to VeridianOS (x86_64)
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=tools/cross/cmake-toolchain-veridian.cmake ..

set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

# Force cross-compilation. Both host and target are Linux x86_64, so cmake
# does not auto-detect this as a cross build. We override it because we use
# a different libc (musl), sysroot, and toolchain.
set(CMAKE_CROSSCOMPILING ON CACHE BOOL "Cross-compiling to VeridianOS" FORCE)

# Sysroot -- populated by build-musl.sh and subsequent dependency builds
set(VERIDIAN_SYSROOT "$ENV{VERIDIAN_SYSROOT}")
if(NOT VERIDIAN_SYSROOT)
    # Default to in-tree sysroot (tools/cross/ -> tools/ -> project root)
    get_filename_component(_tools_dir "${CMAKE_CURRENT_LIST_DIR}" DIRECTORY)
    get_filename_component(_project_root "${_tools_dir}" DIRECTORY)
    set(VERIDIAN_SYSROOT "${_project_root}/target/veridian-sysroot")
endif()

# NOTE: Do NOT set CMAKE_SYSROOT here. The musl-g++ wrapper handles include
# paths via -nostdinc and explicit -isystem flags. Setting CMAKE_SYSROOT causes
# GCC to add --sysroot= which conflicts with the wrapper's include ordering,
# resulting in musl's stdlib.h vs GCC's cstdlib incompatibilities.
# set(CMAKE_SYSROOT "${VERIDIAN_SYSROOT}")  # DISABLED

# Cross-compiler (musl-gcc wrapper)
set(CMAKE_C_COMPILER "${VERIDIAN_SYSROOT}/bin/x86_64-veridian-musl-gcc")
set(CMAKE_CXX_COMPILER "${VERIDIAN_SYSROOT}/bin/x86_64-veridian-musl-g++")
set(CMAKE_AR "ar" CACHE FILEPATH "Archiver")
set(CMAKE_RANLIB "ranlib" CACHE FILEPATH "Ranlib")

# Host Qt for cross-compilation (tools like moc, rcc, uic run on host)
get_filename_component(_build_dir "${VERIDIAN_SYSROOT}/../cross-build/qt6/host-qt" ABSOLUTE)
if(EXISTS "${_build_dir}")
    set(QT_HOST_PATH "${_build_dir}" CACHE PATH "Host Qt for cross-compilation" FORCE)
    set(QT_HOST_PATH_CMAKE_DIR "${_build_dir}/lib/cmake" CACHE PATH "Host Qt cmake dir" FORCE)
endif()

# Host tools for KDE cross-compilation (qtwaylandscanner_kde runs on host)
get_filename_component(_kwin_build_dir "${VERIDIAN_SYSROOT}/../cross-build/kwin" ABSOLUTE)
if(EXISTS "${_kwin_build_dir}/qtwaylandscanner_kde-host-build/qtwaylandscanner_kde")
    set(QTWAYLANDSCANNER_KDE_EXECUTABLE "${_kwin_build_dir}/qtwaylandscanner_kde-host-build/qtwaylandscanner_kde"
        CACHE FILEPATH "Host qtwaylandscanner_kde" FORCE)
endif()

# Search paths -- include both sysroot root and /usr for cmake find_*()
set(CMAKE_FIND_ROOT_PATH "${VERIDIAN_SYSROOT}" "${VERIDIAN_SYSROOT}/usr")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# Static linking by default
set(BUILD_SHARED_LIBS OFF CACHE BOOL "Build static libraries")

# Cross-compilation: compile feature checks as static libraries (not executables)
# to avoid linker issues with .so files and musl's -static wrapper
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# Pkg-config: use LIBDIR (not PATH) to replace default search dirs entirely,
# preventing host system .pc files from leaking into cross-compilation.
# Do NOT set PKG_CONFIG_SYSROOT_DIR (it would double-prefix the paths).
set(ENV{PKG_CONFIG_LIBDIR} "${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_PATH} "")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "")

# Static linking: resolve circular dependencies between static archives.
# CMAKE_EXE_LINKER_FLAGS goes before cmake-generated libs (too early).
# CMAKE_CXX_STANDARD_LIBRARIES goes after ALL cmake libs (correct for resolution).
set(CMAKE_EXE_LINKER_FLAGS "-Wl,--allow-multiple-definition" CACHE STRING "Linker flags" FORCE)
set(CMAKE_CXX_STANDARD_LIBRARIES "-Wl,--start-group -lepoxy -lGLESv2 -lEGL -lgbm -ldrm -lglapi -lwayland-client -lwayland-server -lwayland-egl -lwayland-cursor -lffi -ludev -levdev -lexpat -lfreetype -lfontconfig -lsystemd -lcanberra -llcms2 -lxcvt -ldisplay-info -lz -lm -ldl -lpthread ${VERIDIAN_SYSROOT}/usr/lib/libwl_fixes_stub.a ${VERIDIAN_SYSROOT}/usr/lib/libkwin_stubs.a ${VERIDIAN_SYSROOT}/usr/lib/libgl_stubs.a ${VERIDIAN_SYSROOT}/usr/lib/libkf6_link_stubs.a ${VERIDIAN_SYSROOT}/usr/lib/glibc_shim.a -Wl,--end-group" CACHE STRING "Extra link libs for static" FORCE)
set(CMAKE_C_STANDARD_LIBRARIES "${CMAKE_CXX_STANDARD_LIBRARIES}" CACHE STRING "Extra C link libs for static" FORCE)

# Cross-compilation helpers: create Wayland::Scanner and KF6 stub targets.
# Set from toolchain to avoid CMAKE_FIND_ROOT_PATH_MODE_PACKAGE rewriting the path.
if(EXISTS "${VERIDIAN_SYSROOT}/usr/lib/cmake/wayland-scanner-target.cmake")
    set(CMAKE_PROJECT_INCLUDE "${VERIDIAN_SYSROOT}/usr/lib/cmake/wayland-scanner-target.cmake" CACHE FILEPATH "" FORCE)
endif()

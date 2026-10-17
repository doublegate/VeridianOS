# CMake toolchain file for cross-compiling to VeridianOS (x86_64, musl)
#
# The compiler is the musl cross toolchain (build-musl-toolchain.sh) and
# the sysroot the one every phase stages into. Packages are configured
# for /usr (CMAKE_INSTALL_PREFIX) and installed with DESTDIR=<sysroot>.
# cmake-toolchain-veridian.cmake adds what the Qt/KDE phases need.
#
# Locations come from the environment tools/cross/veridian-paths.sh sets,
# with its defaults.

set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(VERIDIAN_SYSROOT "$ENV{VERIDIAN_SYSROOT}")
if(NOT VERIDIAN_SYSROOT)
    set(VERIDIAN_SYSROOT "/opt/veridian/musl-sysroot")
endif()
set(VERIDIAN_TOOLCHAIN "$ENV{VERIDIAN_TOOLCHAIN}")
if(NOT VERIDIAN_TOOLCHAIN)
    set(VERIDIAN_TOOLCHAIN "/opt/veridian/musl-toolchain")
endif()
set(VERIDIAN_TARGET "$ENV{VERIDIAN_TARGET}")
if(NOT VERIDIAN_TARGET)
    set(VERIDIAN_TARGET "x86_64-veridian-linux-musl")
endif()

set(CMAKE_SYSROOT "${VERIDIAN_SYSROOT}")
set(CMAKE_C_COMPILER "${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-gcc")
set(CMAKE_CXX_COMPILER "${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-g++")
set(CMAKE_AR "${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-ar" CACHE FILEPATH "Archiver")
set(CMAKE_RANLIB "${VERIDIAN_TOOLCHAIN}/bin/${VERIDIAN_TARGET}-ranlib" CACHE FILEPATH "Ranlib")

# Libraries, headers and packages from the sysroot only; programs (code
# generators) from the host.
set(CMAKE_FIND_ROOT_PATH "${VERIDIAN_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# Shared libraries, loaded by musl's dynamic loader (ADR 0010).
set(BUILD_SHARED_LIBS ON CACHE BOOL "Build shared libraries")
# Feature checks link real executables, so a missing function is reported
# missing.
set(CMAKE_TRY_COMPILE_TARGET_TYPE EXECUTABLE)

# Programs a build compiles for VeridianOS and runs on this host (code
# generators, try_run checks) go through musl's loader with the sysroot's
# libraries (lib/cross-env.sh writes the launcher). Tools other packages
# installed are wrapped the same way (cmake/VeridianTargetTools.cmake).
set(VERIDIAN_RUN_TARGET "$ENV{VERIDIAN_RUN_TARGET}")
if(NOT VERIDIAN_RUN_TARGET)
    set(VERIDIAN_RUN_TARGET "/opt/veridian/host-tools/bin/veridian-run-target")
endif()
set(CMAKE_CROSSCOMPILING_EMULATOR "${VERIDIAN_RUN_TARGET}")

# Only the sysroot's .pc files, with their /usr paths placed in the sysroot.
set(ENV{PKG_CONFIG_LIBDIR} "${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_PATH} "")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${VERIDIAN_SYSROOT}")

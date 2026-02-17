# CMake toolchain file for cross-compiling to VeridianOS (AArch64)
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=path/to/veridian-aarch64.cmake ..
#
# Prerequisites:
#   - aarch64-veridian cross-compiler toolchain installed
#   - VeridianOS sysroot populated at ${VERIDIAN_SYSROOT}
#
# Environment variables:
#   VERIDIAN_SYSROOT  - Path to the VeridianOS sysroot (default: /opt/veridian/sysroot/aarch64)
#   VERIDIAN_TOOLCHAIN_PREFIX - Toolchain install prefix (default: /opt/veridian/toolchain)

cmake_minimum_required(VERSION 3.16)

# Target system identification
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_VERSION 0.4)
set(CMAKE_SYSTEM_PROCESSOR aarch64)

# Toolchain paths
if(DEFINED ENV{VERIDIAN_TOOLCHAIN_PREFIX})
    set(TOOLCHAIN_PREFIX "$ENV{VERIDIAN_TOOLCHAIN_PREFIX}")
else()
    set(TOOLCHAIN_PREFIX "/opt/veridian/toolchain")
endif()

if(DEFINED ENV{VERIDIAN_SYSROOT})
    set(VERIDIAN_SYSROOT "$ENV{VERIDIAN_SYSROOT}")
else()
    set(VERIDIAN_SYSROOT "/opt/veridian/sysroot/aarch64")
endif()

# Cross-compiler configuration
set(CROSS_COMPILE "aarch64-veridian-")

set(CMAKE_C_COMPILER   "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}gcc")
set(CMAKE_CXX_COMPILER "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}g++")
set(CMAKE_ASM_COMPILER "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}gcc")
set(CMAKE_AR           "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}ar" CACHE FILEPATH "Archiver")
set(CMAKE_RANLIB       "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}ranlib" CACHE FILEPATH "Ranlib")
set(CMAKE_STRIP        "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}strip" CACHE FILEPATH "Strip")
set(CMAKE_LINKER       "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}ld" CACHE FILEPATH "Linker")
set(CMAKE_NM           "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}nm" CACHE FILEPATH "NM")
set(CMAKE_OBJCOPY      "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}objcopy" CACHE FILEPATH "Objcopy")
set(CMAKE_OBJDUMP      "${TOOLCHAIN_PREFIX}/bin/${CROSS_COMPILE}objdump" CACHE FILEPATH "Objdump")

# Sysroot configuration
set(CMAKE_SYSROOT "${VERIDIAN_SYSROOT}")

# Compiler flags
set(CMAKE_C_FLAGS_INIT   "--sysroot=${VERIDIAN_SYSROOT}")
set(CMAKE_CXX_FLAGS_INIT "--sysroot=${VERIDIAN_SYSROOT}")
set(CMAKE_EXE_LINKER_FLAGS_INIT "--sysroot=${VERIDIAN_SYSROOT}")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "--sysroot=${VERIDIAN_SYSROOT}")
set(CMAKE_MODULE_LINKER_FLAGS_INIT "--sysroot=${VERIDIAN_SYSROOT}")

# Architecture-specific flags for AArch64
# ARMv8-A baseline with NEON (SIMD) enabled by default
set(CMAKE_C_FLAGS_INIT "${CMAKE_C_FLAGS_INIT} -march=armv8-a -mtune=generic")
set(CMAKE_CXX_FLAGS_INIT "${CMAKE_CXX_FLAGS_INIT} -march=armv8-a -mtune=generic")

# Search path configuration
# Search for programs only on the build host
set(CMAKE_FIND_ROOT_PATH "${VERIDIAN_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
# Search for libraries and headers only in the target sysroot
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# pkg-config configuration for cross-compilation
set(ENV{PKG_CONFIG_DIR} "")
set(ENV{PKG_CONFIG_LIBDIR} "${VERIDIAN_SYSROOT}/usr/lib/pkgconfig:${VERIDIAN_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${VERIDIAN_SYSROOT}")

# Install paths default to sysroot
if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX "${VERIDIAN_SYSROOT}/usr" CACHE PATH "Install prefix" FORCE)
endif()

# Disable compiler tests that would fail during cross-compilation
# if the platform module is not yet available in CMake
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

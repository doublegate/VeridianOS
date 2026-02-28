# CMake Toolchain File for cross-compiling to VeridianOS (x86_64)
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=<path>/veridian-x86_64-toolchain.cmake ..
#
# Requires:
#   - x86_64-veridian-gcc cross-compiler at VERIDIAN_TOOLCHAIN_PREFIX
#   - VeridianOS sysroot with headers and libc.a
#
# Environment variables (optional overrides):
#   VERIDIAN_TOOLCHAIN_PREFIX  -- path to toolchain (default: /opt/veridian/toolchain)
#   VERIDIAN_SYSROOT           -- path to sysroot   (default: ${PREFIX}/x86_64-veridian/sysroot)

# Target system identification
set(CMAKE_SYSTEM_NAME      VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

# Toolchain prefix (override via -DVERIDIAN_TOOLCHAIN_PREFIX=...)
if(NOT DEFINED VERIDIAN_TOOLCHAIN_PREFIX)
    if(DEFINED ENV{VERIDIAN_TOOLCHAIN_PREFIX})
        set(VERIDIAN_TOOLCHAIN_PREFIX "$ENV{VERIDIAN_TOOLCHAIN_PREFIX}")
    else()
        set(VERIDIAN_TOOLCHAIN_PREFIX "/opt/veridian/toolchain")
    endif()
endif()

# Sysroot
if(NOT DEFINED VERIDIAN_SYSROOT)
    if(DEFINED ENV{VERIDIAN_SYSROOT})
        set(VERIDIAN_SYSROOT "$ENV{VERIDIAN_SYSROOT}")
    else()
        set(VERIDIAN_SYSROOT "${VERIDIAN_TOOLCHAIN_PREFIX}/x86_64-veridian/sysroot")
    endif()
endif()

set(CMAKE_SYSROOT "${VERIDIAN_SYSROOT}")

# Cross-compiler programs
set(CROSS_PREFIX "${VERIDIAN_TOOLCHAIN_PREFIX}/bin/x86_64-veridian-")

set(CMAKE_C_COMPILER   "${CROSS_PREFIX}gcc")
set(CMAKE_CXX_COMPILER "${CROSS_PREFIX}g++")
set(CMAKE_AR           "${CROSS_PREFIX}ar")
set(CMAKE_RANLIB       "${CROSS_PREFIX}ranlib")
set(CMAKE_STRIP        "${CROSS_PREFIX}strip")
set(CMAKE_NM           "${CROSS_PREFIX}nm")
set(CMAKE_OBJCOPY      "${CROSS_PREFIX}objcopy")
set(CMAKE_OBJDUMP      "${CROSS_PREFIX}objdump")
set(CMAKE_LINKER       "${CROSS_PREFIX}ld")

# Static linking by default (VeridianOS dynamic linker is minimal)
set(CMAKE_C_FLAGS_INIT   "-static")
set(CMAKE_CXX_FLAGS_INIT "-static")
set(CMAKE_EXE_LINKER_FLAGS_INIT "-static")

# Compiler flags for VeridianOS compatibility
set(CMAKE_C_FLAGS   "${CMAKE_C_FLAGS_INIT} -fno-stack-protector -Wno-error=implicit-function-declaration" CACHE STRING "" FORCE)
set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS_INIT} -fno-stack-protector -fno-exceptions -fno-rtti" CACHE STRING "" FORCE)

# Where to search for target programs, libraries, and headers
set(CMAKE_FIND_ROOT_PATH "${VERIDIAN_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)   # Use host tools
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)     # Target libs only
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)     # Target headers only
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)     # Target packages only

# Disable features unavailable on VeridianOS
set(HAVE_LIBPTHREAD    OFF CACHE BOOL "" FORCE)
set(HAVE_PTHREAD_H     OFF CACHE BOOL "" FORCE)
set(THREADS_FOUND      OFF CACHE BOOL "" FORCE)
set(CMAKE_THREAD_LIBS_INIT "" CACHE STRING "" FORCE)

# Disable shared library support (static-only toolchain)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)

# VeridianOS does not have /dev/urandom (uses RDRAND or timer jitter)
set(HAVE_DEV_URANDOM   OFF CACHE BOOL "" FORCE)

# Platform detection for autoconf-based subprojects
set(CMAKE_CROSSCOMPILING TRUE)

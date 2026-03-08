# VeridianOS -- qt6-toolchain.cmake
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# CMake cross-compilation toolchain file for building Qt 6 targeting
# VeridianOS.  Pass this to cmake via -DCMAKE_TOOLCHAIN_FILE=<path>.
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=<srcdir>/userland/qt6/qt6-toolchain.cmake \
#         -DQT_HOST_PATH=/path/to/host-qt6 \
#         <other options> \
#         <qt6-source-dir>

# ---------------------------------------------------------------------------
# Target system identification
# ---------------------------------------------------------------------------
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_SYSTEM_VERSION 1)

# ---------------------------------------------------------------------------
# Cross-compiler toolchain
# ---------------------------------------------------------------------------
set(CMAKE_C_COMPILER   x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)
set(CMAKE_AR           x86_64-veridian-ar)
set(CMAKE_RANLIB       x86_64-veridian-ranlib)
set(CMAKE_STRIP        x86_64-veridian-strip)
set(CMAKE_LINKER       x86_64-veridian-ld)
set(CMAKE_NM           x86_64-veridian-nm)
set(CMAKE_OBJCOPY      x86_64-veridian-objcopy)
set(CMAKE_OBJDUMP      x86_64-veridian-objdump)

# ---------------------------------------------------------------------------
# Sysroot
# ---------------------------------------------------------------------------
set(CMAKE_SYSROOT /opt/veridian-sysroot)
set(CMAKE_FIND_ROOT_PATH /opt/veridian-sysroot)

# Search headers and libraries in sysroot only; programs on host only.
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# ---------------------------------------------------------------------------
# Default compiler / linker flags
# ---------------------------------------------------------------------------
set(CMAKE_C_FLAGS_INIT   "--sysroot=${CMAKE_SYSROOT} -I${CMAKE_SYSROOT}/usr/include")
set(CMAKE_CXX_FLAGS_INIT "--sysroot=${CMAKE_SYSROOT} -I${CMAKE_SYSROOT}/usr/include -std=c++17")
set(CMAKE_EXE_LINKER_FLAGS_INIT    "--sysroot=${CMAKE_SYSROOT} -L${CMAKE_SYSROOT}/usr/lib")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "--sysroot=${CMAKE_SYSROOT} -L${CMAKE_SYSROOT}/usr/lib")

# ---------------------------------------------------------------------------
# pkg-config
# ---------------------------------------------------------------------------
set(ENV{PKG_CONFIG_PATH}    "")
set(ENV{PKG_CONFIG_LIBDIR}  "${CMAKE_SYSROOT}/usr/lib/pkgconfig:${CMAKE_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${CMAKE_SYSROOT}")

# ---------------------------------------------------------------------------
# Qt 6 host tools path
#
# When cross-compiling Qt 6, the host must have a native Qt 6 build
# (moc, rcc, uic, qsb, qlalr) installed.  Set QT_HOST_PATH on the
# cmake command line or in the environment.
# ---------------------------------------------------------------------------
if(NOT DEFINED QT_HOST_PATH)
    if(DEFINED ENV{QT_HOST_PATH})
        set(QT_HOST_PATH "$ENV{QT_HOST_PATH}")
    else()
        message(WARNING "QT_HOST_PATH not set -- Qt 6 host tools (moc, rcc, uic) "
                        "will not be found.  Set -DQT_HOST_PATH=/path/to/host-qt6.")
    endif()
endif()

# ---------------------------------------------------------------------------
# Platform-specific feature hints for Qt 6 configure
# ---------------------------------------------------------------------------
set(CMAKE_CROSSCOMPILING ON)
set(CMAKE_CROSSCOMPILING_EMULATOR "" CACHE STRING "")

# VeridianOS is UNIX-like
set(UNIX  TRUE)
set(LINUX FALSE)

# EGL + OpenGL ES 2.0 available via Mesa
set(OpenGL_GL_PREFERENCE GLVND)

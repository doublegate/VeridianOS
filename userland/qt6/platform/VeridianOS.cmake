# VeridianOS -- VeridianOS.cmake
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# CMake platform module for VeridianOS.  This file is loaded by CMake when
# CMAKE_SYSTEM_NAME is "VeridianOS".  It lives in the cmake/platforms/
# directory of the Qt 6 source tree (or is installed into the sysroot at
# <sysroot>/usr/share/cmake/Modules/Platform/VeridianOS.cmake).
#
# VeridianOS is a UNIX-like microkernel OS with ELF binaries, POSIX threads,
# EGL/OpenGL ES 2.0, Wayland, and D-Bus.

# ---------------------------------------------------------------------------
# Platform identity
# ---------------------------------------------------------------------------
set(VERIDIAN    TRUE)
set(UNIX        TRUE)
set(CMAKE_DL_LIBS "dl")

# VeridianOS uses ELF executables and shared libraries
set(CMAKE_EXECUTABLE_SUFFIX "")
set(CMAKE_SHARED_LIBRARY_PREFIX "lib")
set(CMAKE_SHARED_LIBRARY_SUFFIX ".so")
set(CMAKE_SHARED_MODULE_PREFIX  "lib")
set(CMAKE_SHARED_MODULE_SUFFIX  ".so")
set(CMAKE_STATIC_LIBRARY_PREFIX "lib")
set(CMAKE_STATIC_LIBRARY_SUFFIX ".a")

# Shared library versioning: libfoo.so -> libfoo.so.1 -> libfoo.so.1.0.0
set(CMAKE_SHARED_LIBRARY_SONAME_C_FLAG   "-Wl,-soname,")
set(CMAKE_SHARED_LIBRARY_SONAME_CXX_FLAG "-Wl,-soname,")
set(CMAKE_SHARED_LIBRARY_RUNTIME_C_FLAG  "-Wl,-rpath,")
set(CMAKE_SHARED_LIBRARY_RUNTIME_CXX_FLAG "-Wl,-rpath,")
set(CMAKE_SHARED_LIBRARY_RPATH_ORIGIN_TOKEN "\$ORIGIN")

# PIC is required for shared libraries
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_C_COMPILE_OPTIONS_PIC   "-fPIC")
set(CMAKE_CXX_COMPILE_OPTIONS_PIC "-fPIC")
set(CMAKE_C_COMPILE_OPTIONS_PIE   "-fPIE")
set(CMAKE_CXX_COMPILE_OPTIONS_PIE "-fPIE")

# ---------------------------------------------------------------------------
# Default compiler flags
# ---------------------------------------------------------------------------
set(CMAKE_C_FLAGS_INIT   "")
set(CMAKE_CXX_FLAGS_INIT "")

# VeridianOS defaults to C11 and C++17
if(NOT CMAKE_C_STANDARD)
    set(CMAKE_C_STANDARD 11)
endif()
if(NOT CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()

# ---------------------------------------------------------------------------
# Threading support (POSIX threads)
# ---------------------------------------------------------------------------
set(CMAKE_THREAD_LIBS_INIT  "-lpthread")
set(CMAKE_HAVE_THREADS_LIBRARY 1)
set(CMAKE_USE_PTHREADS_INIT    1)
set(Threads_FOUND              TRUE)

# ---------------------------------------------------------------------------
# EGL / OpenGL ES 2.0
# ---------------------------------------------------------------------------
# Mesa provides libEGL and libGLESv2 in the sysroot.
set(EGL_FOUND    TRUE)
set(EGL_INCLUDE_DIR "${CMAKE_SYSROOT}/usr/include")
set(EGL_LIBRARY     "${CMAKE_SYSROOT}/usr/lib/libEGL.so")

set(GLESv2_FOUND    TRUE)
set(GLESv2_INCLUDE_DIR "${CMAKE_SYSROOT}/usr/include")
set(GLESv2_LIBRARY     "${CMAKE_SYSROOT}/usr/lib/libGLESv2.so")

# ---------------------------------------------------------------------------
# Wayland
# ---------------------------------------------------------------------------
set(Wayland_FOUND TRUE)

# ---------------------------------------------------------------------------
# D-Bus
# ---------------------------------------------------------------------------
set(DBus1_FOUND TRUE)

# ---------------------------------------------------------------------------
# pkg-config paths
# ---------------------------------------------------------------------------
set(ENV{PKG_CONFIG_PATH}   "")
set(ENV{PKG_CONFIG_LIBDIR} "${CMAKE_SYSROOT}/usr/lib/pkgconfig:${CMAKE_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${CMAKE_SYSROOT}")

# ---------------------------------------------------------------------------
# Dynamic linker
# ---------------------------------------------------------------------------
# VeridianOS uses ld-veridian.so.1 as the program interpreter.
set(CMAKE_INSTALL_RPATH_USE_LINK_PATH TRUE)

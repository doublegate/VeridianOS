# VeridianOS -- kwin-veridian-toolchain.cmake
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# CMake cross-compilation toolchain file for building KWin 6.x targeting
# VeridianOS.  Extends the Qt 6 toolchain with KWin-specific dependency
# paths and feature toggles.
#
# Usage:
#   cmake -DCMAKE_TOOLCHAIN_FILE=<srcdir>/userland/kwin/kwin-veridian-toolchain.cmake \
#         -DQT_HOST_PATH=/path/to/host-qt6 \
#         <other options> \
#         <kwin-source-dir>
#
# Prerequisites:
#   1. Qt 6 installed in sysroot (Sprint 9.6)
#   2. KDE Frameworks 6 installed in sysroot (Sprint 9.7)
#   3. libdrm, libgbm, libinput, xkbcommon, dbus-1 in sysroot
#   4. EGL + GLES 2.0 in sysroot (Sprint 9.3)
#   5. Cross-compiler toolchain (x86_64-veridian-gcc/g++)

# ===========================================================================
# Target system identification
# ===========================================================================
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_SYSTEM_VERSION 1)

# ===========================================================================
# Cross-compiler toolchain
# ===========================================================================
set(CMAKE_C_COMPILER   x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)
set(CMAKE_AR           x86_64-veridian-ar)
set(CMAKE_RANLIB       x86_64-veridian-ranlib)
set(CMAKE_STRIP        x86_64-veridian-strip)
set(CMAKE_LINKER       x86_64-veridian-ld)
set(CMAKE_NM           x86_64-veridian-nm)
set(CMAKE_OBJCOPY      x86_64-veridian-objcopy)
set(CMAKE_OBJDUMP      x86_64-veridian-objdump)

# ===========================================================================
# Sysroot
# ===========================================================================
set(CMAKE_SYSROOT /opt/veridian-sysroot)
set(CMAKE_FIND_ROOT_PATH /opt/veridian-sysroot)

# Search headers and libraries in sysroot only; programs on host only.
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# ===========================================================================
# Default compiler / linker flags
# ===========================================================================
set(CMAKE_C_FLAGS_INIT   "--sysroot=${CMAKE_SYSROOT} -I${CMAKE_SYSROOT}/usr/include")
set(CMAKE_CXX_FLAGS_INIT "--sysroot=${CMAKE_SYSROOT} -I${CMAKE_SYSROOT}/usr/include -std=c++20")
set(CMAKE_EXE_LINKER_FLAGS_INIT    "--sysroot=${CMAKE_SYSROOT} -L${CMAKE_SYSROOT}/usr/lib")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "--sysroot=${CMAKE_SYSROOT} -L${CMAKE_SYSROOT}/usr/lib")

# ===========================================================================
# pkg-config
# ===========================================================================
set(ENV{PKG_CONFIG_PATH}    "")
set(ENV{PKG_CONFIG_LIBDIR}  "${CMAKE_SYSROOT}/usr/lib/pkgconfig:${CMAKE_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${CMAKE_SYSROOT}")

# ===========================================================================
# Qt 6 host tools path
# ===========================================================================
if(NOT DEFINED QT_HOST_PATH)
    if(DEFINED ENV{QT_HOST_PATH})
        set(QT_HOST_PATH "$ENV{QT_HOST_PATH}")
    else()
        message(WARNING "QT_HOST_PATH not set -- Qt 6 host tools (moc, rcc, uic) "
                        "will not be found.  Set -DQT_HOST_PATH=/path/to/host-qt6.")
    endif()
endif()

# ===========================================================================
# ECM / KF6 module paths
# ===========================================================================
set(ECM_DIR "${CMAKE_SYSROOT}/usr/share/ECM/cmake"
    CACHE PATH "Path to Extra CMake Modules")
set(CMAKE_PREFIX_PATH
    "${CMAKE_SYSROOT}/usr"
    "${CMAKE_SYSROOT}/usr/lib/cmake"
    "${CMAKE_SYSROOT}/usr/lib64/cmake"
)

# Ensure CMake can locate Qt6, KF6, and Wayland packages in sysroot
list(APPEND CMAKE_MODULE_PATH
    "${ECM_DIR}"
    "${CMAKE_SYSROOT}/usr/lib/cmake"
    "${CMAKE_SYSROOT}/usr/lib64/cmake"
    "${CMAKE_SYSROOT}/usr/share/cmake/Modules"
)

# ===========================================================================
# Platform identification
# ===========================================================================
set(CMAKE_CROSSCOMPILING ON)
set(CMAKE_CROSSCOMPILING_EMULATOR "" CACHE STRING "")

# VeridianOS is UNIX-like but not Linux
set(UNIX  TRUE)
set(LINUX FALSE)

# EGL + OpenGL ES 2.0 available via Mesa
set(OpenGL_GL_PREFERENCE GLVND)

# ===========================================================================
# KWin feature toggles
#
# These disable X11 support entirely and configure KWin for pure
# Wayland + DRM/KMS compositing on VeridianOS.
# ===========================================================================

# Disable X11 / XCB (VeridianOS has no X server)
set(KWIN_BUILD_X11       OFF CACHE BOOL "Disable X11 backend")
set(KWIN_BUILD_XWAYLAND  OFF CACHE BOOL "Disable XWayland (initially)")

# Enable Wayland + DRM/KMS compositing
set(KWIN_BUILD_WAYLAND   ON  CACHE BOOL "Enable Wayland compositor")
set(KWIN_BUILD_DRM       ON  CACHE BOOL "Enable DRM/KMS backend")

# Enable libinput for input handling
set(KWIN_BUILD_LIBINPUT  ON  CACHE BOOL "Enable libinput backend")

# Compositing via EGL + OpenGL ES 2.0
set(KWIN_BUILD_EGL       ON  CACHE BOOL "Enable EGL support")
set(KWIN_BUILD_GLES      ON  CACHE BOOL "Use OpenGL ES 2.0")

# Session management via logind D-Bus API
set(KWIN_BUILD_LOGIND    ON  CACHE BOOL "Enable logind session support")

# Effects -- disable heavy GPU effects for initial bring-up
set(KWIN_BUILD_EFFECTS   ON  CACHE BOOL "Enable KWin effects system")

# Disable optional features not yet ported
set(KWIN_BUILD_SCREENLOCKER   OFF CACHE BOOL "Disable screen locker (initial)")
set(KWIN_BUILD_TABBOX          ON CACHE BOOL "Enable Alt+Tab task switcher")
set(KWIN_BUILD_ACTIVITIES     OFF CACHE BOOL "Disable activities (initial)")
set(KWIN_BUILD_RUNNERS        OFF CACHE BOOL "Disable KRunner integration (initial)")

# Wayland protocols directory
set(WAYLAND_PROTOCOLS_DIR "${CMAKE_SYSROOT}/usr/share/wayland-protocols"
    CACHE PATH "Path to wayland-protocols")
set(PLASMA_WAYLAND_PROTOCOLS_DIR "${CMAKE_SYSROOT}/usr/share/plasma-wayland-protocols"
    CACHE PATH "Path to KDE Wayland protocol XMLs")

# ===========================================================================
# DRM / GBM / libinput hints
# ===========================================================================
set(LIBDRM_INCLUDE_DIRS  "${CMAKE_SYSROOT}/usr/include"       CACHE PATH "")
set(LIBDRM_LIBRARIES     "${CMAKE_SYSROOT}/usr/lib/libdrm.so" CACHE FILEPATH "")
set(GBM_INCLUDE_DIRS     "${CMAKE_SYSROOT}/usr/include"       CACHE PATH "")
set(GBM_LIBRARIES        "${CMAKE_SYSROOT}/usr/lib/libgbm.so" CACHE FILEPATH "")
set(LIBINPUT_INCLUDE_DIRS "${CMAKE_SYSROOT}/usr/include"          CACHE PATH "")
set(LIBINPUT_LIBRARIES    "${CMAKE_SYSROOT}/usr/lib/libinput.so"  CACHE FILEPATH "")
set(UDEV_INCLUDE_DIRS    "${CMAKE_SYSROOT}/usr/include"        CACHE PATH "")
set(UDEV_LIBRARIES       "${CMAKE_SYSROOT}/usr/lib/libudev.so" CACHE FILEPATH "")

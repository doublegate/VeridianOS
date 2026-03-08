# VeridianOS -- VeridianOSPlatform.cmake
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# ECM platform module for VeridianOS.  Loaded when CMAKE_SYSTEM_NAME is
# "VeridianOS".  Provides KDE install directories, compiler settings,
# and platform feature detection for KDE Frameworks 6.
#
# Install into: <sysroot>/usr/share/ECM/cmake/
# Or pass via: -DECM_DIR=<path>/userland/kf6/ecm

# ---------------------------------------------------------------------------
# Platform detection
# ---------------------------------------------------------------------------
if(NOT CMAKE_SYSTEM_NAME STREQUAL "VeridianOS")
    message(FATAL_ERROR "VeridianOSPlatform.cmake loaded but CMAKE_SYSTEM_NAME "
                        "is '${CMAKE_SYSTEM_NAME}', not 'VeridianOS'.")
endif()

set(VERIDIAN    TRUE)
set(UNIX        TRUE)
set(LINUX       FALSE)

message(STATUS "ECM: Configuring for VeridianOS (${CMAKE_SYSTEM_PROCESSOR})")

# ---------------------------------------------------------------------------
# KDE install directories (GNUInstallDirs-compatible)
# ---------------------------------------------------------------------------
set(KDE_INSTALL_BINDIR        "bin"                          CACHE PATH "Executables")
set(KDE_INSTALL_SBINDIR       "sbin"                         CACHE PATH "System executables")
set(KDE_INSTALL_LIBDIR        "lib"                          CACHE PATH "Libraries")
set(KDE_INSTALL_LIBEXECDIR    "libexec"                      CACHE PATH "Helper programs")
set(KDE_INSTALL_INCLUDEDIR    "include"                      CACHE PATH "C/C++ headers")
set(KDE_INSTALL_CMAKEPACKAGEDIR "lib/cmake"                  CACHE PATH "CMake package files")
set(KDE_INSTALL_PLUGINDIR     "lib/plugins"                  CACHE PATH "Qt/KDE plugins")
set(KDE_INSTALL_QMLDIR        "lib/qml"                      CACHE PATH "QML modules")
set(KDE_INSTALL_QTPLUGINDIR   "lib/plugins"                  CACHE PATH "Qt plugins")
set(KDE_INSTALL_QTQUICKIMPORTSDIR "lib/qml"                  CACHE PATH "Qt Quick imports")

set(KDE_INSTALL_DATADIR       "share"                        CACHE PATH "Shared data")
set(KDE_INSTALL_DATAROOTDIR   "share"                        CACHE PATH "Read-only data root")
set(KDE_INSTALL_DOCBUNDLEDIR  "share/doc"                    CACHE PATH "Documentation bundles")
set(KDE_INSTALL_LOCALEDIR     "share/locale"                 CACHE PATH "Translation catalogs")
set(KDE_INSTALL_ICONDIR       "share/icons"                  CACHE PATH "Icon themes")
set(KDE_INSTALL_SOUNDDIR      "share/sounds"                 CACHE PATH "Sound files")
set(KDE_INSTALL_TEMPLATEDIR   "share/templates"              CACHE PATH "File templates")
set(KDE_INSTALL_WALLPAPERDIR  "share/wallpapers"             CACHE PATH "Desktop wallpapers")
set(KDE_INSTALL_APPDIR        "share/applications"           CACHE PATH ".desktop files")
set(KDE_INSTALL_DESKTOPDIR    "share/applications"           CACHE PATH "Desktop files (alias)")
set(KDE_INSTALL_MIMEDIR       "share/mime/packages"          CACHE PATH "MIME type definitions")
set(KDE_INSTALL_METAINFODIR   "share/metainfo"               CACHE PATH "AppStream metadata")
set(KDE_INSTALL_DBUSDIR       "share/dbus-1"                 CACHE PATH "D-Bus service files")
set(KDE_INSTALL_DBUSINTERFACEDIR "share/dbus-1/interfaces"   CACHE PATH "D-Bus interface files")
set(KDE_INSTALL_DBUSSERVICEDIR   "share/dbus-1/services"     CACHE PATH "D-Bus session services")
set(KDE_INSTALL_DBUSSYSTEMSERVICEDIR "share/dbus-1/system-services" CACHE PATH "D-Bus system services")
set(KDE_INSTALL_SYSCONFDIR    "etc"                          CACHE PATH "System configuration")
set(KDE_INSTALL_CONFDIR       "etc/xdg"                      CACHE PATH "XDG configuration")

# Full install prefix paths (for find_package)
set(KDE_INSTALL_FULL_BINDIR        "${CMAKE_INSTALL_PREFIX}/${KDE_INSTALL_BINDIR}")
set(KDE_INSTALL_FULL_LIBDIR        "${CMAKE_INSTALL_PREFIX}/${KDE_INSTALL_LIBDIR}")
set(KDE_INSTALL_FULL_INCLUDEDIR    "${CMAKE_INSTALL_PREFIX}/${KDE_INSTALL_INCLUDEDIR}")
set(KDE_INSTALL_FULL_PLUGINDIR     "${CMAKE_INSTALL_PREFIX}/${KDE_INSTALL_PLUGINDIR}")
set(KDE_INSTALL_FULL_DATADIR       "${CMAKE_INSTALL_PREFIX}/${KDE_INSTALL_DATADIR}")
set(KDE_INSTALL_FULL_CMAKEPACKAGEDIR "${CMAKE_INSTALL_PREFIX}/${KDE_INSTALL_CMAKEPACKAGEDIR}")

# ---------------------------------------------------------------------------
# Display server: Wayland only (no X11)
# ---------------------------------------------------------------------------
set(HAVE_X11      FALSE)
set(HAVE_WAYLAND  TRUE)

# Force X11 off for any KDE module that checks
set(X11_FOUND     FALSE)
set(XCB_FOUND     FALSE)
set(XCB_XCB_FOUND FALSE)

# Wayland available
set(Wayland_FOUND        TRUE)
set(WaylandClient_FOUND  TRUE)
set(WaylandServer_FOUND  TRUE)
set(WaylandScanner_FOUND TRUE)

# ---------------------------------------------------------------------------
# D-Bus available
# ---------------------------------------------------------------------------
set(DBus1_FOUND   TRUE)
set(DBUS_FOUND    TRUE)

# ---------------------------------------------------------------------------
# Compiler flags (match qt6-toolchain.cmake)
# ---------------------------------------------------------------------------
set(CMAKE_C_COMPILER   x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)

# Sysroot (overridable)
if(NOT CMAKE_SYSROOT)
    set(CMAKE_SYSROOT /opt/veridian-sysroot)
endif()

set(CMAKE_C_FLAGS_INIT
    "--sysroot=${CMAKE_SYSROOT} -I${CMAKE_SYSROOT}/usr/include")
set(CMAKE_CXX_FLAGS_INIT
    "--sysroot=${CMAKE_SYSROOT} -I${CMAKE_SYSROOT}/usr/include -std=c++17")
set(CMAKE_EXE_LINKER_FLAGS_INIT
    "--sysroot=${CMAKE_SYSROOT} -L${CMAKE_SYSROOT}/usr/lib")
set(CMAKE_SHARED_LINKER_FLAGS_INIT
    "--sysroot=${CMAKE_SYSROOT} -L${CMAKE_SYSROOT}/usr/lib")

# PIC required
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# C++17 minimum for KDE Frameworks 6
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ---------------------------------------------------------------------------
# Threading (POSIX pthreads)
# ---------------------------------------------------------------------------
set(CMAKE_THREAD_LIBS_INIT  "-lpthread")
set(CMAKE_HAVE_THREADS_LIBRARY 1)
set(CMAKE_USE_PTHREADS_INIT    1)
set(Threads_FOUND              TRUE)

# ---------------------------------------------------------------------------
# EGL / OpenGL ES 2.0 (Mesa in sysroot)
# ---------------------------------------------------------------------------
set(EGL_FOUND       TRUE)
set(EGL_INCLUDE_DIR "${CMAKE_SYSROOT}/usr/include")
set(EGL_LIBRARY     "${CMAKE_SYSROOT}/usr/lib/libEGL.so")

set(OpenGL_FOUND    TRUE)
set(OpenGL_GL_PREFERENCE GLVND)
set(GLESv2_FOUND    TRUE)
set(GLESv2_LIBRARY  "${CMAKE_SYSROOT}/usr/lib/libGLESv2.so")

set(epoxy_FOUND     TRUE)

# ---------------------------------------------------------------------------
# pkg-config paths
# ---------------------------------------------------------------------------
set(ENV{PKG_CONFIG_PATH}   "")
set(ENV{PKG_CONFIG_LIBDIR}
    "${CMAKE_SYSROOT}/usr/lib/pkgconfig:${CMAKE_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${CMAKE_SYSROOT}")

# ---------------------------------------------------------------------------
# Cross-compilation hints
# ---------------------------------------------------------------------------
set(CMAKE_CROSSCOMPILING ON)
set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# ---------------------------------------------------------------------------
# VeridianOS-specific defines passed to all KDE sources
# ---------------------------------------------------------------------------
add_definitions(-D__VERIDIAN__ -DVERIDIAN_OS)

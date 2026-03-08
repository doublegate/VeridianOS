# VeridianOS -- ECMVeridianOSConfig.cmake
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# ECM configuration for VeridianOS.  Provides KDE compiler settings
# overrides, feature detection results, and install directory variables.
#
# This file is loaded by ECM's find_package(ECM) when building KDE
# Frameworks 6 for VeridianOS.  Include it via:
#   -DECM_DIR=<path>/userland/kf6/ecm
#
# Or install to: <sysroot>/usr/share/ECM/cmake/ECMVeridianOSConfig.cmake

cmake_minimum_required(VERSION 3.22)

message(STATUS "ECMVeridianOSConfig: Loading VeridianOS KDE build configuration")

# =========================================================================
# KDE Compiler Settings Overrides
# =========================================================================
# KDE Frameworks 6 normally includes KDECompilerSettings.cmake which sets
# strict warnings.  These overrides adjust for VeridianOS cross-compilation.

# C++17 required for KDE Frameworks 6
set(CMAKE_CXX_STANDARD 17 CACHE STRING "C++ standard")
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# C11 for C code
set(CMAKE_C_STANDARD 11 CACHE STRING "C standard")

# Warnings matching KDE defaults (with VeridianOS adjustments)
set(KDE_COMPILERSETTINGS_LEVEL "DEFAULT" CACHE STRING "KDE compiler strictness level")

# Disable warnings that fire in cross-compilation context
set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} -Wno-unused-parameter")
set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} -Wno-missing-field-initializers")

# =========================================================================
# Feature Detection Results
# =========================================================================
# Pre-cached results for configure-time feature checks that cannot run
# during cross-compilation.

# D-Bus: available via libdbus-1 in sysroot (Sprint 9.5)
set(HAVE_DBUS       TRUE  CACHE BOOL "D-Bus support available")
set(DBUS_FOUND      TRUE  CACHE BOOL "libdbus-1 found")
set(DBus1_FOUND     TRUE  CACHE BOOL "libdbus-1 found (alt name)")

# Wayland: available via libwayland in sysroot (Sprint 9.1)
set(HAVE_WAYLAND    TRUE  CACHE BOOL "Wayland support available")
set(Wayland_FOUND   TRUE  CACHE BOOL "Wayland libraries found")
set(WaylandClient_FOUND TRUE CACHE BOOL "wayland-client found")
set(WaylandServer_FOUND TRUE CACHE BOOL "wayland-server found")
set(WaylandScanner_FOUND TRUE CACHE BOOL "wayland-scanner found")

# X11: NOT available on VeridianOS (Wayland only)
set(HAVE_X11        FALSE CACHE BOOL "X11 support NOT available")
set(X11_FOUND       FALSE CACHE BOOL "X11 NOT found")
set(XCB_FOUND       FALSE CACHE BOOL "XCB NOT found")
set(XCB_XCB_FOUND   FALSE CACHE BOOL "XCB NOT found")

# Polkit: available via libpolkit in sysroot (Sprint 9.5)
set(HAVE_POLKIT     TRUE  CACHE BOOL "PolicyKit support available")
set(PolkitQt6-1_FOUND TRUE CACHE BOOL "Polkit Qt6 bindings available")

# OpenGL: GLES2 via Mesa (Sprint 9.3)
set(HAVE_OPENGL     TRUE  CACHE BOOL "OpenGL available (ES 2.0)")
set(HAVE_EGL        TRUE  CACHE BOOL "EGL available")
set(OpenGL_FOUND    TRUE  CACHE BOOL "OpenGL found")

# libepoxy: available in sysroot (Sprint 9.3)
set(epoxy_FOUND     TRUE  CACHE BOOL "libepoxy found")

# Systemd (logind shim): available (Sprint 9.5)
set(HAVE_SYSTEMD    TRUE  CACHE BOOL "systemd-logind shim available")

# libinput: available in sysroot (Sprint 9.1)
set(Libinput_FOUND  TRUE  CACHE BOOL "libinput found")

# Pipewire: NOT available (audio/video not yet ported)
set(PipeWire_FOUND  FALSE CACHE BOOL "PipeWire NOT available")

# Gettext: available for KI18n (gettext stubs)
set(Gettext_FOUND   TRUE  CACHE BOOL "Gettext found")

# libintl: embedded or available
set(Intl_FOUND      TRUE  CACHE BOOL "libintl found")

# PCRE2: available in sysroot (Sprint 9.2)
set(PCRE2_FOUND     TRUE  CACHE BOOL "PCRE2 found")

# zlib: available in sysroot (Sprint 9.2)
set(ZLIB_FOUND      TRUE  CACHE BOOL "zlib found")

# OpenSSL: available in sysroot (Sprint 9.2)
set(OpenSSL_FOUND   TRUE  CACHE BOOL "OpenSSL found")

# Hunspell: NOT available yet (Sonnet will skip spell checking)
set(HUNSPELL_FOUND  FALSE CACHE BOOL "Hunspell NOT available")

# LibArchive: NOT available (KArchive uses built-in implementations)
set(LibArchive_FOUND FALSE CACHE BOOL "LibArchive NOT available")

# =========================================================================
# Install Directory Variables
# =========================================================================
# These are the ECM-standard install directory variables for VeridianOS.
# They mirror KDEInstallDirs but are set explicitly so they work even
# without the full ECM module path.

if(NOT DEFINED KDE_INSTALL_PREFIX)
    set(KDE_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}" CACHE PATH
        "KDE install prefix")
endif()

set(KDE_INSTALL_BINDIR      "bin"                    CACHE PATH "")
set(KDE_INSTALL_LIBDIR      "lib"                    CACHE PATH "")
set(KDE_INSTALL_INCLUDEDIR  "include"                CACHE PATH "")
set(KDE_INSTALL_PLUGINDIR   "lib/plugins"            CACHE PATH "")
set(KDE_INSTALL_QMLDIR      "lib/qml"                CACHE PATH "")
set(KDE_INSTALL_DATADIR     "share"                  CACHE PATH "")
set(KDE_INSTALL_ICONDIR     "share/icons"            CACHE PATH "")
set(KDE_INSTALL_LOCALEDIR   "share/locale"           CACHE PATH "")
set(KDE_INSTALL_CONFDIR     "etc/xdg"               CACHE PATH "")
set(KDE_INSTALL_DBUSDIR     "share/dbus-1"           CACHE PATH "")
set(KDE_INSTALL_CMAKEPACKAGEDIR "lib/cmake"          CACHE PATH "")
set(KDE_INSTALL_APPDIR      "share/applications"     CACHE PATH "")
set(KDE_INSTALL_MIMEDIR     "share/mime/packages"    CACHE PATH "")
set(KDE_INSTALL_METAINFODIR "share/metainfo"         CACHE PATH "")

# =========================================================================
# ECM Version
# =========================================================================
set(ECM_VERSION "6.3.0" CACHE STRING "ECM version for VeridianOS")
set(ECM_VERSION_MAJOR 6)
set(ECM_VERSION_MINOR 3)
set(ECM_VERSION_PATCH 0)

message(STATUS "ECMVeridianOSConfig: ECM ${ECM_VERSION} configured for VeridianOS")
message(STATUS "  D-Bus: ${HAVE_DBUS}  Wayland: ${HAVE_WAYLAND}  X11: ${HAVE_X11}")
message(STATUS "  EGL: ${HAVE_EGL}  Polkit: ${HAVE_POLKIT}  Systemd: ${HAVE_SYSTEMD}")

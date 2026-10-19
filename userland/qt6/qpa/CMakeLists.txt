# VeridianOS -- QPA plugin CMakeLists.txt
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds the "qveridian" Qt Platform Abstraction plugin. Applications select
# it with -platform veridian or QT_QPA_PLATFORM=veridian.
#
# This is a qtbase-internal build fragment, not a standalone project:
# tools/cross/build-qt6.sh copies this directory to
# qtbase/src/plugins/platforms/veridian and adds it to the platforms list,
# so the plugin is built with qtbase (static, as libqveridian.a) and can use
# qtbase's imported Wayland, EGL and xkbcommon targets.

# xdg-shell client bindings, from the protocol file qtbase bundles (Wayland
# moved into qtbase in Qt 6.10), generated with the host wayland-scanner.
find_program(VERIDIAN_WAYLAND_SCANNER wayland-scanner REQUIRED)
set(_veridian_xdg_xml
    "${QtBase_SOURCE_DIR}/src/3rdparty/wayland/protocols/xdg-shell/xdg-shell.xml")
set(_veridian_gen "${CMAKE_CURRENT_BINARY_DIR}/protocols")
file(MAKE_DIRECTORY "${_veridian_gen}")
add_custom_command(
    OUTPUT "${_veridian_gen}/xdg-shell-client-protocol.h"
           "${_veridian_gen}/xdg-shell-protocol.c"
    COMMAND "${VERIDIAN_WAYLAND_SCANNER}" client-header
            "${_veridian_xdg_xml}" "${_veridian_gen}/xdg-shell-client-protocol.h"
    COMMAND "${VERIDIAN_WAYLAND_SCANNER}" private-code
            "${_veridian_xdg_xml}" "${_veridian_gen}/xdg-shell-protocol.c"
    DEPENDS "${_veridian_xdg_xml}"
    COMMENT "Generating xdg-shell client bindings for the veridian platform"
    VERBATIM)

qt_internal_add_plugin(QVeridianIntegrationPlugin
    OUTPUT_NAME qveridian
    PLUGIN_TYPE platforms
    DEFAULT_IF "veridian" IN_LIST QT_QPA_PLATFORMS
    SOURCES
        qveridianmain.cpp
        qveridianintegration.h      qveridianintegration.cpp
        qveridianscreen.h           qveridianscreen.cpp
        qveridianwindow.h           qveridianwindow.cpp
        qveridianbackingstore.h     qveridianbackingstore.cpp
        qveridianglcontext.h        qveridianglcontext.cpp
        qveridiancursor.h           qveridiancursor.cpp
        qveridianclipboard.h        qveridianclipboard.cpp
        qveridiantheme.h            qveridiantheme.cpp
        qveridianeventdispatcher.h  qveridianeventdispatcher.cpp
        "${_veridian_gen}/xdg-shell-client-protocol.h"
        "${_veridian_gen}/xdg-shell-protocol.c"
    INCLUDE_DIRECTORIES
        "${_veridian_gen}"
    LIBRARIES
        Qt::Core
        Qt::CorePrivate
        Qt::Gui
        Qt::GuiPrivate
        EGL::EGL
        Wayland::Client
        Wayland::Cursor
        XKB::XKB
)

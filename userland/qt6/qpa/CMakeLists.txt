# VeridianOS -- QPA plugin CMakeLists.txt
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds the "qveridian" Qt Platform Abstraction plugin.
# This plugin is loaded by Qt applications via -platform veridian or
# QT_QPA_PLATFORM=veridian.
#
# Build requirements:
#   - Qt 6 (Core, Gui) built for VeridianOS
#   - EGL + GLESv2 (Mesa)
#   - wayland-client, wayland-cursor
#   - xkbcommon
#   - xdg-shell protocol headers

cmake_minimum_required(VERSION 3.16)
project(QVeridianQPA LANGUAGES CXX)

find_package(Qt6 REQUIRED COMPONENTS Core Gui)

qt_internal_add_plugin(QVeridianIntegrationPlugin
    OUTPUT_NAME qveridian
    PLUGIN_TYPE platforms
    DEFAULT_IF "veridian" IN_LIST QT_QPA_PLATFORMS
    SOURCES
        qveridianintegration.h      qveridianintegration.cpp
        qveridianscreen.h           qveridianscreen.cpp
        qveridianwindow.h           qveridianwindow.cpp
        qveridianbackingstore.h     qveridianbackingstore.cpp
        qveridianglcontext.h        qveridianglcontext.cpp
        qveridiancursor.h           qveridiancursor.cpp
        qveridianclipboard.h        qveridianclipboard.cpp
        qveridiantheme.h            qveridiantheme.cpp
        qveridianeventdispatcher.h  qveridianeventdispatcher.cpp
    LIBRARIES
        Qt::Core
        Qt::CorePrivate
        Qt::Gui
        Qt::GuiPrivate
        EGL
        GLESv2
        wayland-client
        wayland-cursor
        xkbcommon
)

# Install the plugin into Qt's platform plugins directory
# (e.g., /usr/lib/qt6/plugins/platforms/libqveridian.so)

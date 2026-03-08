/*
 * VeridianOS -- qveridianscreen.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformScreen implementation.  Provides geometry, color depth, pixel
 * format, and refresh rate for a single DRM/KMS display output.
 */

#include "qveridianscreen.h"

QT_BEGIN_NAMESPACE

QVeridianScreen::QVeridianScreen(const QRect &geometry, qreal refreshRate)
    : m_geometry(geometry)
    , m_refreshRate(refreshRate)
{
}

QVeridianScreen::~QVeridianScreen()
{
}

QRect QVeridianScreen::geometry() const
{
    return m_geometry;
}

QRect QVeridianScreen::availableGeometry() const
{
    /* Reserve no space for panels -- Plasma manages its own struts */
    return m_geometry;
}

int QVeridianScreen::depth() const
{
    return 32;
}

QImage::Format QVeridianScreen::format() const
{
    return QImage::Format_ARGB32_Premultiplied;
}

qreal QVeridianScreen::refreshRate() const
{
    return m_refreshRate;
}

QSizeF QVeridianScreen::physicalSize() const
{
    /* Default to ~96 DPI: for a 1920x1080 display that gives ~508x286 mm.
     * Real DRM connectors provide EDID physical size data. */
    const qreal dpi = 96.0;
    return QSizeF(m_geometry.width() * 25.4 / dpi,
                  m_geometry.height() * 25.4 / dpi);
}

QString QVeridianScreen::name() const
{
    return QStringLiteral("VeridianOS-DRM-%1").arg(m_connectorId);
}

QT_END_NAMESPACE

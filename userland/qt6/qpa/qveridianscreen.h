/*
 * VeridianOS -- qveridianscreen.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformScreen implementation for VeridianOS.  Represents a physical
 * display output backed by DRM/KMS connector and CRTC.
 */

#ifndef QVERIDIANSCREEN_H
#define QVERIDIANSCREEN_H

#include <QtGui/qpa/qplatformscreen.h>
#include <QtCore/QRect>

QT_BEGIN_NAMESPACE

class QVeridianScreen : public QPlatformScreen
{
public:
    QVeridianScreen(const QRect &geometry, qreal refreshRate);
    ~QVeridianScreen() override;

    QRect geometry() const override;
    QRect availableGeometry() const override;
    int depth() const override;
    QImage::Format format() const override;
    qreal refreshRate() const override;
    QSizeF physicalSize() const override;
    QString name() const override;

    /* DRM/KMS identification */
    uint32_t connectorId() const { return m_connectorId; }
    uint32_t crtcId() const { return m_crtcId; }
    void setConnectorId(uint32_t id) { m_connectorId = id; }
    void setCrtcId(uint32_t id) { m_crtcId = id; }

private:
    QRect    m_geometry;
    qreal    m_refreshRate;
    uint32_t m_connectorId = 0;
    uint32_t m_crtcId      = 0;
};

QT_END_NAMESPACE

#endif /* QVERIDIANSCREEN_H */

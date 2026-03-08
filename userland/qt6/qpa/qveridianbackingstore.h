/*
 * VeridianOS -- qveridianbackingstore.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformBackingStore implementation for VeridianOS.  Provides the
 * software rendering path using Wayland SHM buffers.
 */

#ifndef QVERIDIANBACKINGSTORE_H
#define QVERIDIANBACKINGSTORE_H

#include <QtGui/qpa/qplatformbackingstore.h>
#include <QtGui/QImage>

struct wl_buffer;
struct wl_shm_pool;

QT_BEGIN_NAMESPACE

class QVeridianIntegration;

class QVeridianBackingStore : public QPlatformBackingStore
{
public:
    QVeridianBackingStore(QWindow *window, QVeridianIntegration *integration);
    ~QVeridianBackingStore() override;

    QPaintDevice *paintDevice() override;
    void beginPaint(const QRegion &region) override;
    void endPaint() override;
    void flush(QWindow *window, const QRegion &region, const QPoint &offset) override;
    void resize(const QSize &size, const QRegion &staticContents) override;

private:
    void createShmBuffer(const QSize &size);
    void destroyShmBuffer();

    QVeridianIntegration *m_integration;
    QImage                m_image;
    struct wl_buffer     *m_wlBuffer  = nullptr;
    struct wl_shm_pool   *m_shmPool   = nullptr;
    int                   m_shmFd     = -1;
    int                   m_shmSize   = 0;
    void                 *m_shmData   = nullptr;
};

QT_END_NAMESPACE

#endif /* QVERIDIANBACKINGSTORE_H */

/*
 * VeridianOS -- qveridianbackingstore.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Software rendering backing store.  Allocates a POSIX shared-memory
 * buffer, wraps it as a wl_buffer via wl_shm, and attaches it to the
 * window's Wayland surface on flush.
 */

#include "qveridianbackingstore.h"
#include "qveridianwindow.h"
#include "qveridianintegration.h"

#include <wayland-client.h>

#include <sys/mman.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>

QT_BEGIN_NAMESPACE

/* ========================================================================= */
/* SHM file descriptor helper                                                */
/* ========================================================================= */

static int create_shm_fd(int size)
{
    /* Use POSIX shm or memfd_create if available.  Fallback to /dev/shm. */
    const char *path = "/dev/shm/qt-veridian-XXXXXX";
    char name[64];
    strncpy(name, path, sizeof(name) - 1);
    name[sizeof(name) - 1] = '\0';

    int fd = -1;

#ifdef __NR_memfd_create
    fd = memfd_create("qt-veridian", MFD_CLOEXEC);
#endif

    if (fd < 0) {
        /* Fallback: create a temp file in /dev/shm */
        fd = open("/dev/shm/qt-veridian-buf", O_RDWR | O_CREAT | O_TRUNC, 0600);
        if (fd >= 0)
            unlink("/dev/shm/qt-veridian-buf");
    }

    if (fd >= 0) {
        if (ftruncate(fd, size) < 0) {
            close(fd);
            return -1;
        }
    }

    return fd;
}

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

QVeridianBackingStore::QVeridianBackingStore(QWindow *window,
                                             QVeridianIntegration *integration)
    : QPlatformBackingStore(window)
    , m_integration(integration)
{
}

QVeridianBackingStore::~QVeridianBackingStore()
{
    destroyShmBuffer();
}

/* ========================================================================= */
/* SHM buffer management                                                     */
/* ========================================================================= */

void QVeridianBackingStore::createShmBuffer(const QSize &size)
{
    destroyShmBuffer();

    const int stride = size.width() * 4; /* ARGB32 = 4 bytes per pixel */
    m_shmSize = stride * size.height();

    m_shmFd = create_shm_fd(m_shmSize);
    if (m_shmFd < 0)
        return;

    m_shmData = mmap(nullptr, m_shmSize, PROT_READ | PROT_WRITE,
                     MAP_SHARED, m_shmFd, 0);
    if (m_shmData == MAP_FAILED) {
        m_shmData = nullptr;
        close(m_shmFd);
        m_shmFd = -1;
        return;
    }

    /* Wrap the QImage around our SHM data */
    m_image = QImage(static_cast<uchar *>(m_shmData),
                     size.width(), size.height(), stride,
                     QImage::Format_ARGB32_Premultiplied);
}

void QVeridianBackingStore::destroyShmBuffer()
{
    m_image = QImage();

    if (m_wlBuffer) {
        wl_buffer_destroy(m_wlBuffer);
        m_wlBuffer = nullptr;
    }
    if (m_shmPool) {
        wl_shm_pool_destroy(m_shmPool);
        m_shmPool = nullptr;
    }
    if (m_shmData) {
        munmap(m_shmData, m_shmSize);
        m_shmData = nullptr;
    }
    if (m_shmFd >= 0) {
        close(m_shmFd);
        m_shmFd = -1;
    }
    m_shmSize = 0;
}

/* ========================================================================= */
/* QPlatformBackingStore interface                                           */
/* ========================================================================= */

QPaintDevice *QVeridianBackingStore::paintDevice()
{
    return &m_image;
}

void QVeridianBackingStore::beginPaint(const QRegion &region)
{
    Q_UNUSED(region);

    /* Clear the dirty region to transparent */
    if (!m_image.isNull()) {
        for (const QRect &r : region)
            m_image.fill(Qt::transparent);
    }
}

void QVeridianBackingStore::endPaint()
{
    /* Nothing special needed -- data is in the SHM buffer */
}

void QVeridianBackingStore::flush(QWindow *window, const QRegion &region,
                                  const QPoint &offset)
{
    Q_UNUSED(offset);

    if (m_image.isNull() || !m_shmData)
        return;

    auto *platformWindow = static_cast<QVeridianWindow *>(window->handle());
    if (!platformWindow)
        return;

    struct wl_surface *surface = platformWindow->waylandSurface();
    if (!surface)
        return;

    /* Attach buffer and mark damaged regions */
    if (m_wlBuffer)
        wl_surface_attach(surface, m_wlBuffer, 0, 0);

    for (const QRect &r : region)
        wl_surface_damage_buffer(surface, r.x(), r.y(), r.width(), r.height());

    wl_surface_commit(surface);
}

void QVeridianBackingStore::resize(const QSize &size, const QRegion &staticContents)
{
    Q_UNUSED(staticContents);

    if (size == m_image.size())
        return;

    createShmBuffer(size);
}

QT_END_NAMESPACE

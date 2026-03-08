/*
 * VeridianOS -- qveridianwindow.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformWindow implementation.  Manages Wayland surface lifecycle:
 * wl_surface + xdg_surface + xdg_toplevel for top-level windows.
 */

#include "qveridianwindow.h"
#include "qveridianintegration.h"

#include <wayland-client.h>
#include <xdg-shell-client-protocol.h>

QT_BEGIN_NAMESPACE

/* ========================================================================= */
/* xdg_surface listener                                                      */
/* ========================================================================= */

static void xdg_surface_configure(void *data, struct xdg_surface *surface,
                                  uint32_t serial)
{
    xdg_surface_ack_configure(surface, serial);
    Q_UNUSED(data);
}

static const struct xdg_surface_listener xdg_surface_listener = {
    xdg_surface_configure,
};

/* ========================================================================= */
/* xdg_toplevel listener                                                     */
/* ========================================================================= */

static void xdg_toplevel_configure(void *data, struct xdg_toplevel *toplevel,
                                   int32_t width, int32_t height,
                                   struct wl_array *states)
{
    auto *win = static_cast<QVeridianWindow *>(data);
    if (width > 0 && height > 0) {
        QRect geo = win->geometry();
        geo.setSize(QSize(width, height));
        win->setGeometry(geo);
    }
    Q_UNUSED(toplevel);
    Q_UNUSED(states);
}

static void xdg_toplevel_close(void *data, struct xdg_toplevel *toplevel)
{
    auto *win = static_cast<QVeridianWindow *>(data);
    QWindowSystemInterface::handleCloseEvent(win->window());
    Q_UNUSED(toplevel);
}

static const struct xdg_toplevel_listener xdg_toplevel_listener = {
    xdg_toplevel_configure,
    xdg_toplevel_close,
};

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

QVeridianWindow::QVeridianWindow(QWindow *window, QVeridianIntegration *integration)
    : QPlatformWindow(window)
    , m_integration(integration)
    , m_geometry(window->geometry())
{
    createWaylandSurface();
}

QVeridianWindow::~QVeridianWindow()
{
    destroyWaylandSurface();
}

/* ========================================================================= */
/* Wayland surface lifecycle                                                 */
/* ========================================================================= */

void QVeridianWindow::createWaylandSurface()
{
    struct wl_compositor *compositor = m_integration->waylandCompositor();
    struct xdg_wm_base *wmBase = m_integration->xdgWmBase();

    if (!compositor || !wmBase)
        return;

    m_wlSurface = wl_compositor_create_surface(compositor);
    if (!m_wlSurface)
        return;

    m_xdgSurface = xdg_wm_base_get_xdg_surface(wmBase, m_wlSurface);
    if (!m_xdgSurface)
        return;

    xdg_surface_add_listener(m_xdgSurface, &xdg_surface_listener, this);

    m_xdgToplevel = xdg_surface_get_toplevel(m_xdgSurface);
    if (m_xdgToplevel)
        xdg_toplevel_add_listener(m_xdgToplevel, &xdg_toplevel_listener, this);

    wl_surface_commit(m_wlSurface);
}

void QVeridianWindow::destroyWaylandSurface()
{
    if (m_xdgToplevel) {
        xdg_toplevel_destroy(m_xdgToplevel);
        m_xdgToplevel = nullptr;
    }
    if (m_xdgSurface) {
        xdg_surface_destroy(m_xdgSurface);
        m_xdgSurface = nullptr;
    }
    if (m_wlSurface) {
        wl_surface_destroy(m_wlSurface);
        m_wlSurface = nullptr;
    }
}

/* ========================================================================= */
/* Window operations                                                         */
/* ========================================================================= */

void QVeridianWindow::setGeometry(const QRect &rect)
{
    m_geometry = rect;
    QPlatformWindow::setGeometry(rect);
    QWindowSystemInterface::handleGeometryChange(window(), rect);
}

QRect QVeridianWindow::geometry() const
{
    return m_geometry;
}

void QVeridianWindow::setVisible(bool visible)
{
    m_visible = visible;

    if (visible) {
        if (!m_wlSurface)
            createWaylandSurface();
    } else {
        destroyWaylandSurface();
    }

    QPlatformWindow::setVisible(visible);
    QWindowSystemInterface::handleExposeEvent(
        window(),
        visible ? QRect(QPoint(0, 0), m_geometry.size()) : QRegion());
}

void QVeridianWindow::setWindowTitle(const QString &title)
{
    if (m_xdgToplevel)
        xdg_toplevel_set_title(m_xdgToplevel, title.toUtf8().constData());
}

void QVeridianWindow::raise()
{
    /* Wayland does not support client-initiated raise.  The compositor
     * controls stacking order.  This is a no-op. */
}

void QVeridianWindow::lower()
{
    /* Same as raise() -- client cannot control z-order in Wayland. */
}

WId QVeridianWindow::winId() const
{
    return WId(m_wlSurface);
}

QT_END_NAMESPACE

/*
 * VeridianOS -- qveridianwindow.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformWindow implementation for VeridianOS.  Each window maps to a
 * Wayland wl_surface with an xdg_toplevel (or xdg_popup for popups).
 */

#ifndef QVERIDIANWINDOW_H
#define QVERIDIANWINDOW_H

#include <QtGui/qpa/qplatformwindow.h>

struct wl_surface;
struct xdg_surface;
struct xdg_toplevel;

QT_BEGIN_NAMESPACE

class QVeridianIntegration;

class QVeridianWindow : public QPlatformWindow
{
public:
    QVeridianWindow(QWindow *window, QVeridianIntegration *integration);
    ~QVeridianWindow() override;

    void setGeometry(const QRect &rect) override;
    QRect geometry() const override;
    void setVisible(bool visible) override;
    void setWindowTitle(const QString &title) override;
    void raise() override;
    void lower() override;
    WId winId() const override;

    struct wl_surface *waylandSurface() const { return m_wlSurface; }

private:
    void createWaylandSurface();
    void destroyWaylandSurface();

    QVeridianIntegration *m_integration;
    struct wl_surface    *m_wlSurface   = nullptr;
    struct xdg_surface   *m_xdgSurface  = nullptr;
    struct xdg_toplevel  *m_xdgToplevel = nullptr;
    QRect                 m_geometry;
    bool                  m_visible = false;
};

QT_END_NAMESPACE

#endif /* QVERIDIANWINDOW_H */

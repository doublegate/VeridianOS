/*
 * VeridianOS -- kwindowsystem-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWindowSystem Wayland backend for VeridianOS.  Provides window
 * management functions using KWin's Wayland protocols:
 *   - org_kde_plasma_window_management (window list, active window)
 *   - org_kde_plasma_virtual_desktop_management (virtual desktops)
 *
 * This backend replaces the X11/XCB-based window system plugin that
 * KWindowSystem uses on traditional Linux desktops.
 */

#ifndef KWINDOWSYSTEM_VERIDIAN_H
#define KWINDOWSYSTEM_VERIDIAN_H

#include <KWindowSystem/KWindowSystem>
#include <KWindowSystem/KWindowInfo>
#include <KWindowSystem/Platforms/Wayland/KWaylandIntegration>

#include <QObject>
#include <QString>
#include <QStringList>
#include <QRect>
#include <QPixmap>
#include <QList>
#include <QHash>
#include <QPoint>

struct wl_display;
struct wl_registry;
struct org_kde_plasma_window_management;
struct org_kde_plasma_window;
struct org_kde_plasma_virtual_desktop_management;
struct org_kde_plasma_virtual_desktop;

namespace KWindowSystemPrivate {

/* ========================================================================= */
/* WindowInfo -- per-window state from plasma_window_management              */
/* ========================================================================= */

struct WindowInfo {
    quint32 internalId;
    QString uuid;
    QString title;
    QString appId;
    QRect geometry;
    quint32 pid;
    quint32 state;          /* ORG_KDE_PLASMA_WINDOW_MANAGEMENT_STATE_* */
    quint32 virtualDesktop;
    bool active;
    bool minimized;
    bool maximized;
    bool fullscreen;
    bool keepAbove;
    bool keepBelow;
    bool demandsAttention;
    bool closeable;
    bool movable;
    bool resizable;
    bool maximizable;
    bool minimizable;
    bool fullscreenable;
    QStringList virtualDesktopIds;
};

/* ========================================================================= */
/* VirtualDesktopInfo -- per-desktop state                                   */
/* ========================================================================= */

struct VirtualDesktopInfo {
    QString id;
    QString name;
    quint32 number;
    bool active;
};

/* ========================================================================= */
/* VeridianWaylandIntegration                                                */
/* ========================================================================= */

/**
 * KWindowSystem Wayland backend for VeridianOS.
 *
 * Connects to KWin's Wayland protocols to provide window management
 * information to KDE applications (taskbar, window list, etc.).
 *
 * Binds:
 *   - org_kde_plasma_window_management v16
 *   - org_kde_plasma_virtual_desktop_management v2
 *
 * via the Wayland registry on the compositor's display socket.
 */
class VeridianWaylandIntegration : public QObject
{
    Q_OBJECT

public:
    explicit VeridianWaylandIntegration(QObject *parent = nullptr);
    ~VeridianWaylandIntegration() override;

    /* ----- Window list ----- */
    QList<quint32> windows() const;
    WindowInfo windowInfo(quint32 windowId) const;
    quint32 activeWindow() const;

    /* ----- Window operations ----- */
    void activateWindow(quint32 windowId);
    void closeWindow(quint32 windowId);
    void minimizeWindow(quint32 windowId);
    void unminimizeWindow(quint32 windowId);
    void maximizeWindow(quint32 windowId);
    void restoreWindow(quint32 windowId);
    void requestMoveWindow(quint32 windowId);
    void requestResizeWindow(quint32 windowId);
    void setWindowOnDesktop(quint32 windowId, const QString &desktopId);
    void setKeepAbove(quint32 windowId, bool above);
    void setKeepBelow(quint32 windowId, bool below);

    /* ----- Virtual desktops ----- */
    QList<VirtualDesktopInfo> virtualDesktops() const;
    QString currentDesktop() const;
    void setCurrentDesktop(const QString &desktopId);
    int numberOfDesktops() const;

    /* ----- Platform info ----- */
    bool isWayland() const { return true; }
    bool isX11() const { return false; }

Q_SIGNALS:
    void windowAdded(quint32 windowId);
    void windowRemoved(quint32 windowId);
    void windowChanged(quint32 windowId);
    void activeWindowChanged(quint32 windowId);
    void currentDesktopChanged(const QString &desktopId);
    void desktopListChanged();

private:
    /* ----- Wayland protocol callbacks ----- */
    static void registryGlobal(void *data, struct wl_registry *registry,
                               uint32_t name, const char *interface,
                               uint32_t version);
    static void registryGlobalRemove(void *data, struct wl_registry *registry,
                                     uint32_t name);

    /* plasma_window_management callbacks */
    static void windowManagementWindow(void *data,
                                       struct org_kde_plasma_window_management *mgr,
                                       uint32_t id);
    static void windowManagementShowDesktop(void *data,
                                            struct org_kde_plasma_window_management *mgr,
                                            uint32_t state);
    static void windowManagementStackingOrder(void *data,
                                              struct org_kde_plasma_window_management *mgr,
                                              struct wl_array *ids);

    /* plasma_window callbacks */
    static void windowTitle(void *data, struct org_kde_plasma_window *window,
                            const char *title);
    static void windowAppId(void *data, struct org_kde_plasma_window *window,
                            const char *appId);
    static void windowState(void *data, struct org_kde_plasma_window *window,
                            uint32_t flags);
    static void windowGeometry(void *data, struct org_kde_plasma_window *window,
                               int32_t x, int32_t y, uint32_t w, uint32_t h);
    static void windowVirtualDesktopEntered(void *data,
                                            struct org_kde_plasma_window *window,
                                            const char *id);
    static void windowClosed(void *data, struct org_kde_plasma_window *window);
    static void windowPid(void *data, struct org_kde_plasma_window *window,
                          uint32_t pid);

    /* virtual_desktop_management callbacks */
    static void desktopCreated(void *data,
                               struct org_kde_plasma_virtual_desktop_management *mgr,
                               const char *id, uint32_t position);
    static void desktopRemoved(void *data,
                               struct org_kde_plasma_virtual_desktop_management *mgr,
                               const char *id);

    void initWayland();
    void bindWindowManagement(struct wl_registry *registry,
                              uint32_t name, uint32_t version);
    void bindVirtualDesktopManagement(struct wl_registry *registry,
                                     uint32_t name, uint32_t version);

    struct wl_display *m_display;
    struct wl_registry *m_registry;
    struct org_kde_plasma_window_management *m_windowManagement;
    struct org_kde_plasma_virtual_desktop_management *m_desktopManagement;

    QHash<quint32, WindowInfo> m_windows;
    QHash<quint32, struct org_kde_plasma_window *> m_windowProxies;
    QList<VirtualDesktopInfo> m_desktops;
    quint32 m_activeWindow;
    QString m_currentDesktop;
};

} /* namespace KWindowSystemPrivate */

#endif /* KWINDOWSYSTEM_VERIDIAN_H */

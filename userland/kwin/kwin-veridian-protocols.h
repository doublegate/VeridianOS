/*
 * VeridianOS -- kwin-veridian-protocols.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KDE-specific Wayland protocol implementations for KWin on VeridianOS.
 *
 * Provides server-side handlers for KDE Plasma's custom Wayland protocols
 * that enable the Plasma Desktop shell, taskbar, and system settings to
 * communicate with the KWin compositor.
 *
 * Protocols implemented:
 *   - org_kde_plasma_shell (desktop/panel surface roles)
 *   - org_kde_plasma_window_management (window list for taskbar)
 *   - org_kde_kwin_server_decoration_manager (SSD/CSD negotiation)
 *   - org_kde_kwin_blur_manager (background blur regions)
 *   - org_kde_kwin_dpms (display power management signaling)
 *   - org_kde_kwin_outputdevice / outputmanagement (display configuration)
 *
 * These compile against KWin's Wayland server infrastructure and the
 * wayland-server C library.
 */

#ifndef KWIN_VERIDIAN_PROTOCOLS_H
#define KWIN_VERIDIAN_PROTOCOLS_H

#include <QObject>
#include <QString>
#include <QVector>
#include <QRect>
#include <QHash>
#include <QPoint>
#include <QSize>

/* Wayland server-side headers */
struct wl_client;
struct wl_resource;
struct wl_display;
struct wl_global;

namespace KWin {

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

class VeridianDrmBackend;
class VeridianDrmOutput;

/* ========================================================================= */
/* VeridianPlasmaShellSurface -- per-surface Plasma shell role               */
/* ========================================================================= */

/**
 * State for a surface that has been assigned a Plasma shell role
 * (desktop, panel, notification, overlay, etc.).
 */
struct PlasmaShellSurfaceData {
    struct wl_resource *resource;
    struct wl_resource *surface;
    uint32_t role;          /* panel, desktop, notification, overlay, etc. */
    int32_t x;
    int32_t y;
    uint32_t panelBehavior;  /* always-visible, auto-hide, etc. */
    bool positionSet;
    bool skipTaskbar;
    bool skipSwitcher;
};

/* ========================================================================= */
/* VeridianPlasmaShellInterface -- org_kde_plasma_shell                      */
/* ========================================================================= */

/**
 * Server-side implementation of the org_kde_plasma_shell protocol.
 *
 * Allows the Plasma Desktop shell to create surfaces with special roles:
 *   - Desktop: wallpaper/icon background (below all windows)
 *   - Panel: taskbar/system tray (above windows, screen-edge anchored)
 *   - Notification: popup notifications (overlay, auto-dismiss)
 *   - Override: lock screen, splash screen
 *
 * The Plasma shell process binds this global at startup and creates
 * shell surfaces for each desktop/panel/notification component.
 */
class VeridianPlasmaShellInterface : public QObject
{
    Q_OBJECT

public:
    explicit VeridianPlasmaShellInterface(struct wl_display *display,
                                          QObject *parent = nullptr);
    ~VeridianPlasmaShellInterface() override;

    QVector<PlasmaShellSurfaceData *> surfaces() const;
    PlasmaShellSurfaceData *surfaceForResource(struct wl_resource *resource) const;

Q_SIGNALS:
    void surfaceCreated(PlasmaShellSurfaceData *surface);
    void surfaceDestroyed(PlasmaShellSurfaceData *surface);

private:
    /* Protocol callbacks */
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);
    static void getShellSurface(struct wl_client *client,
                                struct wl_resource *resource,
                                uint32_t id,
                                struct wl_resource *surface);

    /* Shell surface callbacks */
    static void shellSurfaceSetRole(struct wl_client *client,
                                    struct wl_resource *resource,
                                    uint32_t role);
    static void shellSurfaceSetPosition(struct wl_client *client,
                                        struct wl_resource *resource,
                                        int32_t x, int32_t y);
    static void shellSurfaceSetPanelBehavior(struct wl_client *client,
                                             struct wl_resource *resource,
                                             uint32_t behavior);
    static void shellSurfaceSetSkipTaskbar(struct wl_client *client,
                                           struct wl_resource *resource,
                                           uint32_t skip);
    static void shellSurfaceSetSkipSwitcher(struct wl_client *client,
                                            struct wl_resource *resource,
                                            uint32_t skip);
    static void shellSurfaceDestroy(struct wl_resource *resource);

    struct wl_display *m_display;
    struct wl_global *m_global;
    QVector<PlasmaShellSurfaceData *> m_surfaces;
};

/* ========================================================================= */
/* VeridianWindowManagementInterface -- org_kde_plasma_window_management     */
/* ========================================================================= */

/**
 * Server-side state for a managed window exported to the taskbar.
 */
struct ManagedWindowData {
    uint32_t internalId;
    struct wl_resource *windowResource;
    QString title;
    QString appId;
    uint32_t pid;
    uint32_t state;
    QRect geometry;
    QStringList virtualDesktopIds;
    bool closeable;
    bool movable;
    bool resizable;
    bool maximizable;
    bool minimizable;
    bool fullscreenable;
};

/**
 * Server-side implementation of org_kde_plasma_window_management.
 *
 * Exports the window list to Plasma's taskbar.  For each managed window,
 * sends title, appId, state flags, geometry, and virtual desktop
 * membership.  Also receives window operation requests from the taskbar
 * (activate, close, minimize, maximize, etc.).
 */
class VeridianWindowManagementInterface : public QObject
{
    Q_OBJECT

public:
    explicit VeridianWindowManagementInterface(struct wl_display *display,
                                               QObject *parent = nullptr);
    ~VeridianWindowManagementInterface() override;

    /* ----- Window registration ----- */
    void addWindow(uint32_t internalId, const QString &title,
                   const QString &appId, uint32_t pid);
    void removeWindow(uint32_t internalId);
    void updateWindowTitle(uint32_t internalId, const QString &title);
    void updateWindowState(uint32_t internalId, uint32_t state);
    void updateWindowGeometry(uint32_t internalId, const QRect &geometry);
    void updateWindowVirtualDesktop(uint32_t internalId,
                                    const QStringList &desktopIds);

    /* ----- Show desktop ----- */
    void setShowDesktop(bool show);

    /* ----- Stacking order ----- */
    void updateStackingOrder(const QVector<uint32_t> &order);

Q_SIGNALS:
    void windowActivateRequested(uint32_t internalId);
    void windowCloseRequested(uint32_t internalId);
    void windowMinimizeRequested(uint32_t internalId, bool minimize);
    void windowMaximizeRequested(uint32_t internalId, bool maximize);
    void windowMoveRequested(uint32_t internalId);
    void windowResizeRequested(uint32_t internalId);
    void windowVirtualDesktopRequested(uint32_t internalId,
                                       const QString &desktopId);
    void showDesktopRequested(bool show);

private:
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);

    /* Per-window protocol callbacks */
    static void windowSetState(struct wl_client *client,
                               struct wl_resource *resource,
                               uint32_t flags, uint32_t state);
    static void windowClose(struct wl_client *client,
                            struct wl_resource *resource);
    static void windowRequestMove(struct wl_client *client,
                                  struct wl_resource *resource);
    static void windowRequestResize(struct wl_client *client,
                                    struct wl_resource *resource);
    static void windowRequestVirtualDesktop(struct wl_client *client,
                                            struct wl_resource *resource,
                                            const char *id);
    static void windowDestroy(struct wl_resource *resource);

    void sendWindowToClient(struct wl_resource *managerResource,
                            const ManagedWindowData &window);

    struct wl_display *m_display;
    struct wl_global *m_global;
    QHash<uint32_t, ManagedWindowData> m_windows;
    QVector<struct wl_resource *> m_boundResources;
};

/* ========================================================================= */
/* VeridianServerDecorationManager -- org_kde_kwin_server_decoration_manager */
/* ========================================================================= */

/**
 * Decoration mode negotiation between compositor and clients.
 *
 * KWin prefers server-side decorations (SSD) by default.  Clients can
 * request client-side decorations (CSD) and the compositor responds
 * with the actual mode.  On VeridianOS we default to SSD so that the
 * Breeze decoration plugin draws title bars.
 */
class VeridianServerDecorationManager : public QObject
{
    Q_OBJECT

public:
    enum DecorationMode {
        None   = 0,     /* No decorations */
        Client = 1,     /* Client-side decorations (CSD) */
        Server = 2      /* Server-side decorations (SSD) */
    };

    explicit VeridianServerDecorationManager(struct wl_display *display,
                                              QObject *parent = nullptr);
    ~VeridianServerDecorationManager() override;

    DecorationMode defaultMode() const;
    void setDefaultMode(DecorationMode mode);

Q_SIGNALS:
    void decorationModeRequested(struct wl_resource *surface,
                                 DecorationMode mode);

private:
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);
    static void create(struct wl_client *client,
                       struct wl_resource *resource,
                       uint32_t id, struct wl_resource *surface);
    static void requestMode(struct wl_client *client,
                            struct wl_resource *resource,
                            uint32_t mode);
    static void decorationDestroy(struct wl_resource *resource);

    struct wl_display *m_display;
    struct wl_global *m_global;
    DecorationMode m_defaultMode;
};

/* ========================================================================= */
/* VeridianBlurManager -- org_kde_kwin_blur_manager                          */
/* ========================================================================= */

/**
 * Per-surface blur region data.
 */
struct BlurRegionData {
    struct wl_resource *resource;
    struct wl_resource *surface;
    QVector<QRect> regions;
};

/**
 * Server-side implementation of org_kde_kwin_blur_manager.
 *
 * Clients (e.g., Plasma panel, Konsole with transparency) request
 * background blur on specific regions of their surface.  The compositor
 * applies a Gaussian blur to the area behind the surface during
 * compositing.
 *
 * On llvmpipe, blur is silently disabled (too expensive for software
 * rendering).
 */
class VeridianBlurManager : public QObject
{
    Q_OBJECT

public:
    explicit VeridianBlurManager(struct wl_display *display,
                                 bool gpuAccelerated,
                                 QObject *parent = nullptr);
    ~VeridianBlurManager() override;

    bool isEnabled() const;
    void setEnabled(bool enabled);
    QVector<BlurRegionData *> activeBlurs() const;
    BlurRegionData *blurForSurface(struct wl_resource *surface) const;

Q_SIGNALS:
    void blurCreated(BlurRegionData *blur);
    void blurDestroyed(BlurRegionData *blur);
    void blurRegionChanged(BlurRegionData *blur);

private:
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);
    static void createBlur(struct wl_client *client,
                           struct wl_resource *resource,
                           uint32_t id, struct wl_resource *surface);
    static void unsetBlur(struct wl_client *client,
                          struct wl_resource *resource,
                          struct wl_resource *surface);

    /* Per-blur callbacks */
    static void blurCommit(struct wl_client *client,
                           struct wl_resource *resource);
    static void blurSetRegion(struct wl_client *client,
                              struct wl_resource *resource,
                              struct wl_resource *region);
    static void blurDestroy(struct wl_resource *resource);

    struct wl_display *m_display;
    struct wl_global *m_global;
    QVector<BlurRegionData *> m_blurs;
    bool m_enabled;
    bool m_gpuAccelerated;
};

/* ========================================================================= */
/* VeridianDpmsManager -- org_kde_kwin_dpms                                  */
/* ========================================================================= */

/**
 * Display Power Management Signaling (DPMS) via Wayland.
 *
 * Allows clients (e.g., Plasma power settings) to query and change the
 * DPMS state of each output.  Maps to DRM connector DPMS property.
 *
 * States:
 *   0 = On, 1 = Standby, 2 = Suspend, 3 = Off
 */
class VeridianDpmsManager : public QObject
{
    Q_OBJECT

public:
    explicit VeridianDpmsManager(struct wl_display *display,
                                 VeridianDrmBackend *backend,
                                 QObject *parent = nullptr);
    ~VeridianDpmsManager() override;

    /* ----- State queries ----- */
    int dpmsState(uint32_t connectorId) const;
    bool isSupported(uint32_t connectorId) const;

Q_SIGNALS:
    void dpmsStateChanged(uint32_t connectorId, int state);

private:
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);
    static void getDpms(struct wl_client *client,
                        struct wl_resource *resource,
                        uint32_t id, struct wl_resource *output);
    static void setDpms(struct wl_client *client,
                        struct wl_resource *resource,
                        uint32_t mode);
    static void dpmsDestroy(struct wl_resource *resource);

    void sendDpmsState(struct wl_resource *resource, uint32_t connectorId);

    struct wl_display *m_display;
    struct wl_global *m_global;
    VeridianDrmBackend *m_backend;
    QHash<uint32_t, int> m_dpmsStates;     /* connectorId -> DPMS level */
};

/* ========================================================================= */
/* VeridianOutputDevice -- org_kde_kwin_outputdevice                         */
/* ========================================================================= */

/**
 * Information about a physical display output, exported via the
 * org_kde_kwin_outputdevice protocol.
 *
 * This is a read-only view of output properties: resolution, refresh
 * rate, physical size, model name, manufacturer, EDID data.
 */
struct OutputDeviceData {
    uint32_t id;
    QString manufacturer;
    QString model;
    QString serialNumber;
    QSize physicalSize;         /* mm */
    QPoint globalPosition;
    QVector<drmModeModeInfo> modes;
    int currentModeIndex;
    int preferredModeIndex;
    int transform;              /* 0=normal, 1=90, 2=180, 3=270 */
    int scale;                  /* fixed-point: 1000 = 1.0x */
    bool enabled;
    uint32_t connectorId;
};

/**
 * Server-side implementation of org_kde_kwin_outputdevice (v2).
 *
 * Broadcasts output device information to clients.  Used by KDE
 * System Settings (Display Configuration KCM) to show available
 * displays and their properties.
 */
class VeridianOutputDeviceInterface : public QObject
{
    Q_OBJECT

public:
    explicit VeridianOutputDeviceInterface(struct wl_display *display,
                                           QObject *parent = nullptr);
    ~VeridianOutputDeviceInterface() override;

    void addOutput(const OutputDeviceData &data);
    void removeOutput(uint32_t id);
    void updateOutput(const OutputDeviceData &data);

    QVector<OutputDeviceData> outputs() const;

private:
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);
    void sendOutputData(struct wl_resource *resource,
                        const OutputDeviceData &data);
    void sendMode(struct wl_resource *resource,
                  const drmModeModeInfo &mode, int index,
                  bool current, bool preferred);

    struct wl_display *m_display;
    struct wl_global *m_global;
    QVector<OutputDeviceData> m_outputs;
    QVector<struct wl_resource *> m_boundResources;
};

/* ========================================================================= */
/* VeridianOutputManagement -- org_kde_kwin_outputmanagement                 */
/* ========================================================================= */

/**
 * Configuration change request for a display output.
 */
struct OutputConfigChange {
    uint32_t outputId;
    int modeIndex;              /* -1 = no change */
    int transform;              /* -1 = no change */
    int scale;                  /* -1 = no change (fixed-point) */
    QPoint position;            /* INT_MIN = no change */
    int enabled;                /* -1 = no change, 0 = disable, 1 = enable */
};

/**
 * Server-side implementation of org_kde_kwin_outputmanagement.
 *
 * Receives display configuration changes from clients (System Settings)
 * and applies them via DRM/KMS mode setting.  Supports mode changes,
 * rotation, scaling, and multi-monitor layout configuration.
 */
class VeridianOutputManagement : public QObject
{
    Q_OBJECT

public:
    explicit VeridianOutputManagement(struct wl_display *display,
                                      VeridianDrmBackend *backend,
                                      QObject *parent = nullptr);
    ~VeridianOutputManagement() override;

Q_SIGNALS:
    void configurationChangeRequested(const QVector<OutputConfigChange> &changes);

private:
    static void bind(struct wl_client *client, void *data,
                     uint32_t version, uint32_t id);
    static void createConfiguration(struct wl_client *client,
                                    struct wl_resource *resource,
                                    uint32_t id);

    /* Configuration object callbacks */
    static void configEnable(struct wl_client *client,
                             struct wl_resource *resource,
                             struct wl_resource *outputDevice,
                             int32_t enable);
    static void configMode(struct wl_client *client,
                           struct wl_resource *resource,
                           struct wl_resource *outputDevice,
                           int32_t modeIndex);
    static void configTransform(struct wl_client *client,
                                struct wl_resource *resource,
                                struct wl_resource *outputDevice,
                                int32_t transform);
    static void configScale(struct wl_client *client,
                            struct wl_resource *resource,
                            struct wl_resource *outputDevice,
                            int32_t scale);
    static void configPosition(struct wl_client *client,
                               struct wl_resource *resource,
                               struct wl_resource *outputDevice,
                               int32_t x, int32_t y);
    static void configApply(struct wl_client *client,
                            struct wl_resource *resource);
    static void configDestroy(struct wl_resource *resource);

    bool applyConfiguration(const QVector<OutputConfigChange> &changes);

    struct wl_display *m_display;
    struct wl_global *m_global;
    VeridianDrmBackend *m_backend;
};

/* ========================================================================= */
/* VeridianProtocolRegistry -- registers all KDE protocol globals            */
/* ========================================================================= */

/**
 * Convenience class that creates and registers all KDE Wayland protocol
 * globals on the compositor's wl_display.
 *
 * Usage:
 *   VeridianProtocolRegistry registry(wlDisplay, drmBackend);
 *   registry.initialize();
 *
 * After initialization, Plasma shell, taskbar, and settings clients
 * can bind to the protocol globals via the Wayland registry.
 */
class VeridianProtocolRegistry : public QObject
{
    Q_OBJECT

public:
    explicit VeridianProtocolRegistry(struct wl_display *display,
                                      VeridianDrmBackend *backend,
                                      QObject *parent = nullptr);
    ~VeridianProtocolRegistry() override;

    bool initialize();

    VeridianPlasmaShellInterface *plasmaShell() const;
    VeridianWindowManagementInterface *windowManagement() const;
    VeridianServerDecorationManager *serverDecorationManager() const;
    VeridianBlurManager *blurManager() const;
    VeridianDpmsManager *dpmsManager() const;
    VeridianOutputDeviceInterface *outputDevice() const;
    VeridianOutputManagement *outputManagement() const;

private:
    struct wl_display *m_display;
    VeridianDrmBackend *m_backend;

    VeridianPlasmaShellInterface *m_plasmaShell;
    VeridianWindowManagementInterface *m_windowManagement;
    VeridianServerDecorationManager *m_serverDecoration;
    VeridianBlurManager *m_blurManager;
    VeridianDpmsManager *m_dpmsManager;
    VeridianOutputDeviceInterface *m_outputDevice;
    VeridianOutputManagement *m_outputManagement;
};

} /* namespace KWin */

#endif /* KWIN_VERIDIAN_PROTOCOLS_H */

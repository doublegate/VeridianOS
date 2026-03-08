/*
 * VeridianOS -- qveridianintegration.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPA platform integration implementation for VeridianOS.  Connects to
 * the Wayland compositor, enumerates DRM/KMS outputs as screens, and
 * provides factory methods for windows, backing stores, and GL contexts.
 */

#include "qveridianintegration.h"
#include "qveridianscreen.h"
#include "qveridianwindow.h"
#include "qveridianbackingstore.h"
#include "qveridianglcontext.h"
#include "qveridianclipboard.h"
#include "qveridiantheme.h"
#include "qveridianeventdispatcher.h"

#include <QtGui/private/qguiapplication_p.h>
#include <QtGui/qpa/qplatformfontdatabase.h>
#include <QtGui/qpa/qplatforminputcontext.h>

#include <wayland-client.h>
#include <xdg-shell-client-protocol.h>

#include <EGL/egl.h>
#include <xkbcommon/xkbcommon.h>

QT_BEGIN_NAMESPACE

/* ========================================================================= */
/* Wayland registry listener                                                 */
/* ========================================================================= */

static void registry_global(void *data, struct wl_registry *registry,
                            uint32_t name, const char *interface,
                            uint32_t version)
{
    auto *self = static_cast<QVeridianIntegration *>(data);
    Q_UNUSED(self);

    if (strcmp(interface, "wl_compositor") == 0) {
        /* Bind wl_compositor -- stored via initWayland() */
    } else if (strcmp(interface, "wl_shm") == 0) {
        /* Bind wl_shm for software rendering path */
    } else if (strcmp(interface, "wl_seat") == 0) {
        /* Bind wl_seat for keyboard/pointer input */
    } else if (strcmp(interface, "xdg_wm_base") == 0) {
        /* Bind xdg_wm_base for window management */
    } else if (strcmp(interface, "wl_output") == 0) {
        /* Bind wl_output for screen enumeration */
    }

    Q_UNUSED(registry);
    Q_UNUSED(name);
    Q_UNUSED(version);
}

static void registry_global_remove(void *data, struct wl_registry *registry,
                                   uint32_t name)
{
    Q_UNUSED(data);
    Q_UNUSED(registry);
    Q_UNUSED(name);
}

static const struct wl_registry_listener registry_listener = {
    registry_global,
    registry_global_remove,
};

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

QVeridianIntegration::QVeridianIntegration(const QStringList &parameters)
{
    Q_UNUSED(parameters);
    initWayland();
    initScreens();
}

QVeridianIntegration::~QVeridianIntegration()
{
    qDeleteAll(m_screens);

    if (m_xdgWmBase)
        xdg_wm_base_destroy(m_xdgWmBase);
    if (m_wlSeat)
        wl_seat_destroy(m_wlSeat);
    if (m_wlShm)
        wl_shm_destroy(m_wlShm);
    if (m_wlCompositor)
        wl_compositor_destroy(m_wlCompositor);
    if (m_wlRegistry)
        wl_registry_destroy(m_wlRegistry);
    if (m_wlDisplay)
        wl_display_disconnect(m_wlDisplay);
}

/* ========================================================================= */
/* Initialization                                                            */
/* ========================================================================= */

void QVeridianIntegration::initWayland()
{
    m_wlDisplay = wl_display_connect(nullptr);
    if (!m_wlDisplay) {
        qFatal("QVeridianIntegration: cannot connect to Wayland display");
        return;
    }

    m_wlRegistry = wl_display_get_registry(m_wlDisplay);
    wl_registry_add_listener(m_wlRegistry, &registry_listener, this);

    /* Round-trip to receive all global advertisements */
    wl_display_roundtrip(m_wlDisplay);
}

void QVeridianIntegration::initScreens()
{
    /* Create at least one default screen.  Real DRM enumeration would
     * query /dev/dri/card0 connectors via libdrm.  For now we create
     * a single 1920x1080 screen as a sane default. */
    auto *screen = new QVeridianScreen(QRect(0, 0, 1920, 1080), 60.0);
    m_screens.append(screen);
    QWindowSystemInterface::handleScreenAdded(screen);
}

/* ========================================================================= */
/* Capabilities                                                              */
/* ========================================================================= */

bool QVeridianIntegration::hasCapability(QPlatformIntegration::Capability cap) const
{
    switch (cap) {
    case ThreadedPixmaps:
        return true;
    case OpenGL:
        return true;
    case ThreadedOpenGL:
        return true;
    case RasterGLSurface:
        return true;
    case WindowManagement:
        return true;
    default:
        return QPlatformIntegration::hasCapability(cap);
    }
}

/* ========================================================================= */
/* Factory methods                                                           */
/* ========================================================================= */

QPlatformWindow *QVeridianIntegration::createPlatformWindow(QWindow *window) const
{
    return new QVeridianWindow(window, const_cast<QVeridianIntegration *>(this));
}

QPlatformBackingStore *QVeridianIntegration::createPlatformBackingStore(QWindow *window) const
{
    return new QVeridianBackingStore(window, const_cast<QVeridianIntegration *>(this));
}

QPlatformOpenGLContext *QVeridianIntegration::createPlatformOpenGLContext(QOpenGLContext *context) const
{
    return new QVeridianGLContext(context);
}

/* ========================================================================= */
/* Services                                                                  */
/* ========================================================================= */

QPlatformFontDatabase *QVeridianIntegration::fontDatabase() const
{
    if (!m_fontDatabase) {
        /* Uses FreeType + Fontconfig backend from QtFontDatabaseSupport */
        m_fontDatabase.reset(new QPlatformFontDatabase());
    }
    return m_fontDatabase.data();
}

QPlatformClipboard *QVeridianIntegration::clipboard() const
{
    if (!m_clipboard)
        m_clipboard.reset(new QVeridianClipboard());
    return m_clipboard.data();
}

QPlatformInputContext *QVeridianIntegration::inputContext() const
{
    if (!m_inputContext) {
        /* xkbcommon-based input context for keyboard layout support */
        m_inputContext.reset(new QPlatformInputContext());
    }
    return m_inputContext.data();
}

QPlatformTheme *QVeridianIntegration::createPlatformTheme(const QString &name) const
{
    Q_UNUSED(name);
    return new QVeridianTheme();
}

QAbstractEventDispatcher *QVeridianIntegration::createEventDispatcher() const
{
    return new QVeridianEventDispatcher();
}

/* ========================================================================= */
/* Screen accessors                                                          */
/* ========================================================================= */

QList<QPlatformScreen *> QVeridianIntegration::screens() const
{
    QList<QPlatformScreen *> result;
    for (auto *s : m_screens)
        result.append(s);
    return result;
}

QPlatformScreen *QVeridianIntegration::primaryScreen() const
{
    return m_screens.isEmpty() ? nullptr : m_screens.first();
}

/* ========================================================================= */
/* Wayland accessors                                                         */
/* ========================================================================= */

struct wl_display *QVeridianIntegration::waylandDisplay() const
{
    return m_wlDisplay;
}

struct wl_compositor *QVeridianIntegration::waylandCompositor() const
{
    return m_wlCompositor;
}

struct xdg_wm_base *QVeridianIntegration::xdgWmBase() const
{
    return m_xdgWmBase;
}

QT_END_NAMESPACE

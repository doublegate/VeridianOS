/*
 * VeridianOS -- qveridianintegration.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformIntegration subclass for VeridianOS.  Entry point for the
 * "qveridian" QPA plugin -- manages screens, windows, OpenGL contexts,
 * clipboard, input, cursor, and theming through the VeridianOS Wayland
 * compositor and DRM/KMS display stack.
 */

#ifndef QVERIDIANINTEGRATION_H
#define QVERIDIANINTEGRATION_H

#include <QtGui/qpa/qplatformintegration.h>
#include <QtCore/QScopedPointer>

QT_BEGIN_NAMESPACE

class QVeridianScreen;
class QVeridianClipboard;
class QVeridianTheme;

class QVeridianIntegration : public QPlatformIntegration
{
public:
    explicit QVeridianIntegration(const QStringList &parameters);
    ~QVeridianIntegration() override;

    /* -- Core factory methods ------------------------------------------- */

    bool hasCapability(QPlatformIntegration::Capability cap) const override;
    QPlatformWindow *createPlatformWindow(QWindow *window) const override;
    QPlatformBackingStore *createPlatformBackingStore(QWindow *window) const override;
    QPlatformOpenGLContext *createPlatformOpenGLContext(QOpenGLContext *context) const override;

    /* -- Services ------------------------------------------------------- */

    QPlatformFontDatabase *fontDatabase() const override;
    QPlatformClipboard *clipboard() const override;
    QPlatformInputContext *inputContext() const override;
    QPlatformTheme *createPlatformTheme(const QString &name) const override;
    QAbstractEventDispatcher *createEventDispatcher() const override;

    /* -- Screen management ---------------------------------------------- */

    QList<QPlatformScreen *> screens() const;
    QPlatformScreen *primaryScreen() const;

    /* -- Wayland helpers ------------------------------------------------ */

    struct wl_display *waylandDisplay() const;
    struct wl_compositor *waylandCompositor() const;
    struct xdg_wm_base *xdgWmBase() const;

private:
    void initWayland();
    void initScreens();

    struct wl_display    *m_wlDisplay    = nullptr;
    struct wl_registry   *m_wlRegistry   = nullptr;
    struct wl_compositor *m_wlCompositor = nullptr;
    struct wl_shm        *m_wlShm        = nullptr;
    struct wl_seat       *m_wlSeat       = nullptr;
    struct xdg_wm_base   *m_xdgWmBase   = nullptr;

    QList<QVeridianScreen *>        m_screens;
    mutable QScopedPointer<QPlatformFontDatabase> m_fontDatabase;
    mutable QScopedPointer<QVeridianClipboard>    m_clipboard;
    mutable QScopedPointer<QPlatformInputContext> m_inputContext;
};

QT_END_NAMESPACE

#endif /* QVERIDIANINTEGRATION_H */

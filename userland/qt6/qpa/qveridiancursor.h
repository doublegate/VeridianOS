/*
 * VeridianOS -- qveridiancursor.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformCursor implementation for VeridianOS.  Uses the Wayland
 * cursor protocol (wl_cursor_theme) for hardware cursor support.
 */

#ifndef QVERIDIANCURSOR_H
#define QVERIDIANCURSOR_H

#include <QtGui/qpa/qplatformcursor.h>

struct wl_cursor_theme;
struct wl_surface;

QT_BEGIN_NAMESPACE

class QVeridianScreen;

class QVeridianCursor : public QPlatformCursor
{
public:
    explicit QVeridianCursor(QVeridianScreen *screen);
    ~QVeridianCursor() override;

    void changeCursor(QCursor *windowCursor, QWindow *window) override;
    QPoint pos() const override;
    void setPos(const QPoint &pos) override;

private:
    QVeridianScreen      *m_screen;
    struct wl_cursor_theme *m_cursorTheme = nullptr;
    struct wl_surface      *m_cursorSurface = nullptr;
    QPoint                  m_pos;
};

QT_END_NAMESPACE

#endif /* QVERIDIANCURSOR_H */

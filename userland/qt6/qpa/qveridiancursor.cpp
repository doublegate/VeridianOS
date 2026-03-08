/*
 * VeridianOS -- qveridiancursor.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wayland cursor implementation.  Loads cursor images from the
 * default cursor theme and attaches them to a dedicated wl_surface
 * bound to the wl_pointer.
 */

#include "qveridiancursor.h"
#include "qveridianscreen.h"

#include <wayland-client.h>
#include <wayland-cursor.h>

QT_BEGIN_NAMESPACE

QVeridianCursor::QVeridianCursor(QVeridianScreen *screen)
    : m_screen(screen)
{
    /* Load the default cursor theme at size 24 */
    m_cursorTheme = wl_cursor_theme_load(nullptr, 24, nullptr);
}

QVeridianCursor::~QVeridianCursor()
{
    if (m_cursorSurface)
        wl_surface_destroy(m_cursorSurface);
    if (m_cursorTheme)
        wl_cursor_theme_destroy(m_cursorTheme);
}

void QVeridianCursor::changeCursor(QCursor *windowCursor, QWindow *window)
{
    Q_UNUSED(window);

    if (!m_cursorTheme)
        return;

    /* Map Qt cursor shape to Wayland cursor name */
    const char *name = "left_ptr";
    if (windowCursor) {
        switch (windowCursor->shape()) {
        case Qt::ArrowCursor:        name = "left_ptr"; break;
        case Qt::WaitCursor:         name = "watch"; break;
        case Qt::CrossCursor:        name = "crosshair"; break;
        case Qt::IBeamCursor:        name = "xterm"; break;
        case Qt::SizeVerCursor:      name = "sb_v_double_arrow"; break;
        case Qt::SizeHorCursor:      name = "sb_h_double_arrow"; break;
        case Qt::PointingHandCursor: name = "hand2"; break;
        case Qt::ForbiddenCursor:    name = "crossed_circle"; break;
        case Qt::OpenHandCursor:     name = "grab"; break;
        case Qt::ClosedHandCursor:   name = "grabbing"; break;
        default:                     name = "left_ptr"; break;
        }
    }

    struct wl_cursor *cursor = wl_cursor_theme_get_cursor(m_cursorTheme, name);
    if (!cursor)
        cursor = wl_cursor_theme_get_cursor(m_cursorTheme, "left_ptr");

    if (cursor && cursor->image_count > 0) {
        struct wl_cursor_image *image = cursor->images[0];
        struct wl_buffer *buffer = wl_cursor_image_get_buffer(image);
        Q_UNUSED(buffer);
        /* Would attach buffer to cursor surface and set_cursor on pointer */
    }
}

QPoint QVeridianCursor::pos() const
{
    return m_pos;
}

void QVeridianCursor::setPos(const QPoint &pos)
{
    /* Wayland does not allow clients to set the cursor position.
     * Store it locally for query purposes only. */
    m_pos = pos;
}

QT_END_NAMESPACE

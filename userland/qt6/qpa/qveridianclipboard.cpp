/*
 * VeridianOS -- qveridianclipboard.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wayland clipboard integration.  Uses wl_data_device_manager to
 * create data sources (for copy) and receive data offers (for paste).
 * Supports both Clipboard and Selection (primary selection) modes.
 */

#include "qveridianclipboard.h"

#include <QtCore/QMimeData>
#include <wayland-client.h>

QT_BEGIN_NAMESPACE

QVeridianClipboard::QVeridianClipboard()
    : m_clipboardData(new QMimeData())
    , m_selectionData(new QMimeData())
{
}

QVeridianClipboard::~QVeridianClipboard()
{
    if (m_dataSource)
        wl_data_source_destroy(m_dataSource);
    if (m_dataDevice)
        wl_data_device_destroy(m_dataDevice);

    delete m_clipboardData;
    delete m_selectionData;
}

QMimeData *QVeridianClipboard::mimeData(QClipboard::Mode mode)
{
    if (mode == QClipboard::Selection)
        return m_selectionData;
    return m_clipboardData;
}

void QVeridianClipboard::setMimeData(QMimeData *data, QClipboard::Mode mode)
{
    if (!data)
        return;

    if (mode == QClipboard::Selection) {
        delete m_selectionData;
        m_selectionData = data;
        m_ownsSelection = true;
    } else {
        delete m_clipboardData;
        m_clipboardData = data;
        m_ownsClipboard = true;
    }

    /* Create a wl_data_source and set the offered MIME types */
    if (m_dataDeviceManager && m_dataDevice) {
        if (m_dataSource)
            wl_data_source_destroy(m_dataSource);

        m_dataSource = wl_data_device_manager_create_data_source(
            m_dataDeviceManager);

        if (m_dataSource) {
            for (const QString &format : data->formats()) {
                wl_data_source_offer(m_dataSource,
                                     format.toUtf8().constData());
            }
            wl_data_device_set_selection(m_dataDevice, m_dataSource, 0);
        }
    }

    emitChanged(mode);
}

bool QVeridianClipboard::supportsMode(QClipboard::Mode mode) const
{
    /* Wayland supports both clipboard and primary selection */
    return mode == QClipboard::Clipboard || mode == QClipboard::Selection;
}

bool QVeridianClipboard::ownsMode(QClipboard::Mode mode) const
{
    if (mode == QClipboard::Selection)
        return m_ownsSelection;
    return m_ownsClipboard;
}

QT_END_NAMESPACE

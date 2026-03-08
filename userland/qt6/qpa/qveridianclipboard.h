/*
 * VeridianOS -- qveridianclipboard.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformClipboard implementation for VeridianOS.  Integrates with
 * the Wayland data_device_manager protocol for copy/paste operations.
 */

#ifndef QVERIDIANCLIPBOARD_H
#define QVERIDIANCLIPBOARD_H

#include <QtGui/qpa/qplatformclipboard.h>

struct wl_data_device_manager;
struct wl_data_device;
struct wl_data_source;
struct wl_data_offer;

QT_BEGIN_NAMESPACE

class QVeridianClipboard : public QPlatformClipboard
{
public:
    QVeridianClipboard();
    ~QVeridianClipboard() override;

    QMimeData *mimeData(QClipboard::Mode mode = QClipboard::Clipboard) override;
    void setMimeData(QMimeData *data,
                     QClipboard::Mode mode = QClipboard::Clipboard) override;
    bool supportsMode(QClipboard::Mode mode) const override;
    bool ownsMode(QClipboard::Mode mode) const override;

private:
    struct wl_data_device_manager *m_dataDeviceManager = nullptr;
    struct wl_data_device         *m_dataDevice        = nullptr;
    struct wl_data_source         *m_dataSource        = nullptr;
    struct wl_data_offer          *m_dataOffer         = nullptr;

    QMimeData *m_clipboardData  = nullptr;
    QMimeData *m_selectionData  = nullptr;
    bool       m_ownsClipboard  = false;
    bool       m_ownsSelection  = false;
};

QT_END_NAMESPACE

#endif /* QVERIDIANCLIPBOARD_H */

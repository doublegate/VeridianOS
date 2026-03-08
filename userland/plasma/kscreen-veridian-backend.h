/*
 * VeridianOS -- kscreen-veridian-backend.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KScreen display configuration backend for VeridianOS.  Provides the
 * bridge between KScreen's output management API and the VeridianOS
 * DRM/KMS subsystem for the System Settings display KCM.
 *
 * Responsibilities:
 *   - Output enumeration from DRM/KMS (/dev/dri/card0)
 *   - Resolution and refresh rate listing per output
 *   - Output enable/disable
 *   - Primary output selection
 *   - Multi-monitor layout (position, rotation, scale)
 *   - Mode setting via DRM ioctls
 *   - Hot-plug detection via udev/DRM events
 *
 * This backend is loaded by KScreen when running on VeridianOS and
 * replaces the standard DRM backend with VeridianOS-specific adaptations
 * (logind shim integration, custom udev paths).
 */

#ifndef KSCREEN_VERIDIAN_BACKEND_H
#define KSCREEN_VERIDIAN_BACKEND_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QVector>
#include <QSize>
#include <QPoint>
#include <QSocketNotifier>
#include <QHash>

/* KScreen API headers (from KDE Frameworks build tree) */
#include <KScreen/BackendInterface>
#include <KScreen/Config>
#include <KScreen/Output>
#include <KScreen/Screen>
#include <KScreen/Mode>

/* DRM headers */
#include <xf86drm.h>
#include <xf86drmMode.h>

namespace KScreen {

/* ========================================================================= */
/* VeridianOutput -- per-output DRM state                                    */
/* ========================================================================= */

/**
 * Internal representation of a DRM output for KScreen mapping.
 * Holds the DRM connector/CRTC/encoder state and maps it to KScreen's
 * Output/Mode abstractions.
 */
struct VeridianOutputInfo
{
    /* DRM identifiers */
    uint32_t connectorId;
    uint32_t crtcId;
    uint32_t encoderId;

    /* Display properties */
    QString name;               /* e.g. "HDMI-A-1" */
    QString vendor;             /* EDID manufacturer */
    QString model;              /* EDID model name */
    QString serial;             /* EDID serial number */
    int physicalWidth;          /* mm */
    int physicalHeight;         /* mm */
    bool connected;
    bool enabled;

    /* Current mode */
    QSize currentResolution;
    int currentRefreshRate;     /* mHz */

    /* Position in multi-monitor layout */
    QPoint position;
    int rotation;               /* 0, 90, 180, 270 degrees */
    qreal scale;                /* HiDPI scale factor (1.0, 1.25, 1.5, 2.0) */

    /* Available modes */
    struct ModeInfo {
        QString id;             /* unique mode identifier */
        QSize size;
        int refreshRate;        /* mHz */
        bool preferred;
        drmModeModeInfo drmMode;
    };
    QVector<ModeInfo> modes;

    /* DRM properties */
    uint32_t dpmsPropertyId;
    bool isPrimary;
};

/* ========================================================================= */
/* VeridianKScreenBackend -- KScreen backend implementation                  */
/* ========================================================================= */

/**
 * KScreen backend that reads display configuration from VeridianOS DRM/KMS.
 *
 * Provides:
 *   - Output enumeration via DRM connector scan
 *   - Mode listing from DRM mode arrays
 *   - Configuration application via drmModeSetCrtc / atomic modeset
 *   - Hot-plug monitoring via DRM fd polling
 *   - EDID parsing for vendor/model/serial
 *   - DPMS control for power management
 */
class VeridianKScreenBackend : public QObject, public KScreen::BackendInterface
{
    Q_OBJECT
    Q_INTERFACES(KScreen::BackendInterface)
    Q_PLUGIN_METADATA(IID "org.kde.kscreen.backends.veridian")

public:
    explicit VeridianKScreenBackend(QObject *parent = nullptr);
    ~VeridianKScreenBackend() override;

    /* ----- BackendInterface ----- */
    QString name() const override;
    QString serviceName() const override;
    KScreen::ConfigPtr config() const override;
    void setConfig(const KScreen::ConfigPtr &config) override;
    bool isValid() const override;

    /* ----- Output queries ----- */
    QVector<VeridianOutputInfo> outputs() const;
    VeridianOutputInfo *findOutput(const QString &name);
    VeridianOutputInfo *findOutputById(uint32_t connectorId);
    VeridianOutputInfo *primaryOutput();

    /* ----- Configuration ----- */
    bool applyConfiguration(const QVector<VeridianOutputInfo> &outputs);
    bool setOutputMode(uint32_t connectorId, const QString &modeId);
    bool setOutputEnabled(uint32_t connectorId, bool enabled);
    bool setOutputPosition(uint32_t connectorId, const QPoint &pos);
    bool setOutputRotation(uint32_t connectorId, int rotation);
    bool setOutputScale(uint32_t connectorId, qreal scale);
    bool setPrimaryOutput(uint32_t connectorId);

    /* ----- DPMS ----- */
    bool setDpms(uint32_t connectorId, int level);
    int dpmsState(uint32_t connectorId) const;

    /* ----- Hot-plug ----- */
    void startHotplugDetection();
    void stopHotplugDetection();

Q_SIGNALS:
    void configChanged(const KScreen::ConfigPtr &config);
    void outputConnected(const QString &name);
    void outputDisconnected(const QString &name);

private Q_SLOTS:
    void onDrmEvent();

private:
    /* ----- DRM operations ----- */
    bool openDrmDevice();
    void closeDrmDevice();
    bool scanOutputs();
    bool parseEdid(uint32_t connectorId, VeridianOutputInfo &output);
    QVector<VeridianOutputInfo::ModeInfo> parseModes(
        drmModeConnector *connector) const;
    uint32_t findProperty(uint32_t objectId, uint32_t objectType,
                          const char *name) const;

    /* ----- KScreen mapping ----- */
    KScreen::OutputPtr createKScreenOutput(
        const VeridianOutputInfo &output) const;
    KScreen::ModePtr createKScreenMode(
        const VeridianOutputInfo::ModeInfo &mode) const;
    KScreen::ScreenPtr createKScreenScreen() const;
    void updateKScreenConfig();

    /* ----- State ----- */
    int m_drmFd;
    QString m_drmPath;
    QString m_driverName;
    bool m_supportsAtomic;
    QVector<VeridianOutputInfo> m_outputs;
    KScreen::ConfigPtr m_config;
    QSocketNotifier *m_drmNotifier;
};

} /* namespace KScreen */

#endif /* KSCREEN_VERIDIAN_BACKEND_H */

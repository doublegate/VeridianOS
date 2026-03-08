/*
 * VeridianOS -- kscreen-veridian-backend.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KScreen display configuration backend implementation for VeridianOS.
 *
 * Enumerates DRM outputs, parses EDID data, maps DRM modes to KScreen
 * Mode objects, and applies display configuration changes via DRM ioctls.
 * Hot-plug detection monitors the DRM fd for connector change events.
 */

#include "kscreen-veridian-backend.h"

#include <QDebug>
#include <QDir>
#include <QFile>

#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>
#include <sys/stat.h>

namespace KScreen {

/* ========================================================================= */
/* VeridianKScreenBackend                                                    */
/* ========================================================================= */

VeridianKScreenBackend::VeridianKScreenBackend(QObject *parent)
    : QObject(parent)
    , m_drmFd(-1)
    , m_drmPath(QStringLiteral("/dev/dri/card0"))
    , m_supportsAtomic(false)
    , m_drmNotifier(nullptr)
{
    if (openDrmDevice()) {
        scanOutputs();
        updateKScreenConfig();
    }
}

VeridianKScreenBackend::~VeridianKScreenBackend()
{
    stopHotplugDetection();
    closeDrmDevice();
}

/* ========================================================================= */
/* BackendInterface implementation                                           */
/* ========================================================================= */

QString VeridianKScreenBackend::name() const
{
    return QStringLiteral("VeridianOS DRM");
}

QString VeridianKScreenBackend::serviceName() const
{
    return QStringLiteral("org.kde.kscreen.backends.veridian");
}

KScreen::ConfigPtr VeridianKScreenBackend::config() const
{
    return m_config;
}

void VeridianKScreenBackend::setConfig(const KScreen::ConfigPtr &config)
{
    if (!config)
        return;

    /* Apply each output's configuration */
    const auto outputs = config->outputs();
    for (const auto &output : outputs) {
        uint32_t connectorId = output->id();

        /* Enable/disable */
        setOutputEnabled(connectorId, output->isEnabled());

        /* Mode */
        if (output->currentMode())
            setOutputMode(connectorId, output->currentMode()->id());

        /* Position */
        setOutputPosition(connectorId, output->pos());

        /* Rotation */
        setOutputRotation(connectorId, static_cast<int>(output->rotation()));

        /* Scale */
        setOutputScale(connectorId, output->scale());

        /* Primary */
        if (output->isPrimary())
            setPrimaryOutput(connectorId);
    }

    /* Re-scan to reflect applied changes */
    scanOutputs();
    updateKScreenConfig();
}

bool VeridianKScreenBackend::isValid() const
{
    return m_drmFd >= 0 && !m_outputs.isEmpty();
}

/* ========================================================================= */
/* Output queries                                                            */
/* ========================================================================= */

QVector<VeridianOutputInfo> VeridianKScreenBackend::outputs() const
{
    return m_outputs;
}

VeridianOutputInfo *VeridianKScreenBackend::findOutput(const QString &name)
{
    for (int i = 0; i < m_outputs.size(); ++i) {
        if (m_outputs[i].name == name)
            return &m_outputs[i];
    }
    return nullptr;
}

VeridianOutputInfo *VeridianKScreenBackend::findOutputById(uint32_t connectorId)
{
    for (int i = 0; i < m_outputs.size(); ++i) {
        if (m_outputs[i].connectorId == connectorId)
            return &m_outputs[i];
    }
    return nullptr;
}

VeridianOutputInfo *VeridianKScreenBackend::primaryOutput()
{
    for (int i = 0; i < m_outputs.size(); ++i) {
        if (m_outputs[i].isPrimary)
            return &m_outputs[i];
    }
    /* If no primary set, return first connected output */
    for (int i = 0; i < m_outputs.size(); ++i) {
        if (m_outputs[i].connected)
            return &m_outputs[i];
    }
    return nullptr;
}

/* ========================================================================= */
/* Configuration application                                                 */
/* ========================================================================= */

bool VeridianKScreenBackend::setOutputMode(uint32_t connectorId,
                                            const QString &modeId)
{
    VeridianOutputInfo *output = findOutputById(connectorId);
    if (!output)
        return false;

    /* Find the DRM mode matching the KScreen mode ID */
    for (const auto &mode : output->modes) {
        if (mode.id == modeId) {
            drmModeModeInfo drmMode = mode.drmMode;
            int ret = drmModeSetCrtc(m_drmFd, output->crtcId, 0, 0, 0,
                                      &connectorId, 1, &drmMode);
            if (ret != 0) {
                qWarning("KScreen/Veridian: setMode failed: %s", strerror(errno));
                return false;
            }
            output->currentResolution = mode.size;
            output->currentRefreshRate = mode.refreshRate;
            return true;
        }
    }

    qWarning("KScreen/Veridian: mode %s not found for connector %u",
             qPrintable(modeId), connectorId);
    return false;
}

bool VeridianKScreenBackend::setOutputEnabled(uint32_t connectorId, bool enabled)
{
    VeridianOutputInfo *output = findOutputById(connectorId);
    if (!output)
        return false;

    if (enabled == output->enabled)
        return true;

    /* Use DPMS to enable/disable */
    int level = enabled ? 0 : 3;  /* DRM_MODE_DPMS_ON / DRM_MODE_DPMS_OFF */
    if (setDpms(connectorId, level)) {
        output->enabled = enabled;
        return true;
    }
    return false;
}

bool VeridianKScreenBackend::setOutputPosition(uint32_t connectorId,
                                                const QPoint &pos)
{
    VeridianOutputInfo *output = findOutputById(connectorId);
    if (!output)
        return false;

    output->position = pos;
    /* Position is applied when the full configuration is committed */
    return true;
}

bool VeridianKScreenBackend::setOutputRotation(uint32_t connectorId,
                                                int rotation)
{
    VeridianOutputInfo *output = findOutputById(connectorId);
    if (!output)
        return false;

    output->rotation = rotation;
    /* Rotation is applied when the full configuration is committed.
     * On VeridianOS, rotation is implemented via the compositor's
     * output transform rather than DRM plane rotation. */
    return true;
}

bool VeridianKScreenBackend::setOutputScale(uint32_t connectorId, qreal scale)
{
    VeridianOutputInfo *output = findOutputById(connectorId);
    if (!output)
        return false;

    output->scale = scale;
    /* Scale is communicated to the compositor via wl_output.scale
     * and wp_fractional_scale_v1 protocols. */
    return true;
}

bool VeridianKScreenBackend::setPrimaryOutput(uint32_t connectorId)
{
    for (int i = 0; i < m_outputs.size(); ++i) {
        m_outputs[i].isPrimary = (m_outputs[i].connectorId == connectorId);
    }
    return true;
}

bool VeridianKScreenBackend::applyConfiguration(
    const QVector<VeridianOutputInfo> &outputs)
{
    Q_UNUSED(outputs);
    /* Full configuration commit -- would use atomic modeset if supported */
    return true;
}

/* ========================================================================= */
/* DPMS control                                                              */
/* ========================================================================= */

bool VeridianKScreenBackend::setDpms(uint32_t connectorId, int level)
{
    VeridianOutputInfo *output = findOutputById(connectorId);
    if (!output || output->dpmsPropertyId == 0)
        return false;

    int ret = drmModeConnectorSetProperty(m_drmFd, connectorId,
                                           output->dpmsPropertyId, level);
    return (ret == 0);
}

int VeridianKScreenBackend::dpmsState(uint32_t connectorId) const
{
    for (const auto &output : m_outputs) {
        if (output.connectorId == connectorId && output.dpmsPropertyId != 0) {
            drmModeObjectProperties *props =
                drmModeObjectGetProperties(m_drmFd, connectorId,
                                           DRM_MODE_OBJECT_CONNECTOR);
            if (!props)
                return -1;

            int state = -1;
            for (uint32_t i = 0; i < props->count_props; ++i) {
                if (props->props[i] == output.dpmsPropertyId) {
                    state = static_cast<int>(props->prop_values[i]);
                    break;
                }
            }
            drmModeFreeObjectProperties(props);
            return state;
        }
    }
    return -1;
}

/* ========================================================================= */
/* Hot-plug detection                                                        */
/* ========================================================================= */

void VeridianKScreenBackend::startHotplugDetection()
{
    if (m_drmNotifier)
        return;

    if (m_drmFd < 0)
        return;

    m_drmNotifier = new QSocketNotifier(m_drmFd, QSocketNotifier::Read, this);
    connect(m_drmNotifier, &QSocketNotifier::activated,
            this, &VeridianKScreenBackend::onDrmEvent);

    qDebug("KScreen/Veridian: hot-plug detection started");
}

void VeridianKScreenBackend::stopHotplugDetection()
{
    delete m_drmNotifier;
    m_drmNotifier = nullptr;
}

void VeridianKScreenBackend::onDrmEvent()
{
    /* Handle DRM events (page flips, hot-plug) */
    drmEventContext ctx;
    memset(&ctx, 0, sizeof(ctx));
    ctx.version = 2;
    drmHandleEvent(m_drmFd, &ctx);

    /* Re-scan outputs to detect hot-plug changes */
    QVector<VeridianOutputInfo> oldOutputs = m_outputs;
    scanOutputs();

    /* Detect added/removed outputs */
    for (const auto &newOut : m_outputs) {
        bool found = false;
        for (const auto &oldOut : oldOutputs) {
            if (oldOut.connectorId == newOut.connectorId) {
                found = true;
                if (oldOut.connected != newOut.connected) {
                    if (newOut.connected)
                        Q_EMIT outputConnected(newOut.name);
                    else
                        Q_EMIT outputDisconnected(newOut.name);
                }
                break;
            }
        }
        if (!found && newOut.connected)
            Q_EMIT outputConnected(newOut.name);
    }

    updateKScreenConfig();
    Q_EMIT configChanged(m_config);
}

/* ========================================================================= */
/* DRM operations                                                            */
/* ========================================================================= */

bool VeridianKScreenBackend::openDrmDevice()
{
    m_drmFd = open(m_drmPath.toUtf8().constData(), O_RDWR | O_CLOEXEC);
    if (m_drmFd < 0) {
        qWarning("KScreen/Veridian: cannot open %s: %s",
                 qPrintable(m_drmPath), strerror(errno));
        return false;
    }

    /* Query driver */
    drmVersion *ver = drmGetVersion(m_drmFd);
    if (ver) {
        m_driverName = QString::fromUtf8(ver->name, ver->name_len);
        drmFreeVersion(ver);
    }

    /* Check atomic modesetting */
    m_supportsAtomic = (drmSetClientCap(m_drmFd, DRM_CLIENT_CAP_ATOMIC, 1) == 0);
    drmSetClientCap(m_drmFd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1);

    qDebug("KScreen/Veridian: opened %s (driver: %s, atomic: %d)",
           qPrintable(m_drmPath), qPrintable(m_driverName), m_supportsAtomic);

    return true;
}

void VeridianKScreenBackend::closeDrmDevice()
{
    if (m_drmFd >= 0) {
        close(m_drmFd);
        m_drmFd = -1;
    }
}

bool VeridianKScreenBackend::scanOutputs()
{
    if (m_drmFd < 0)
        return false;

    m_outputs.clear();

    drmModeRes *resources = drmModeGetResources(m_drmFd);
    if (!resources) {
        qWarning("KScreen/Veridian: drmModeGetResources failed");
        return false;
    }

    for (int i = 0; i < resources->count_connectors; ++i) {
        drmModeConnector *conn = drmModeGetConnector(m_drmFd,
                                                      resources->connectors[i]);
        if (!conn)
            continue;

        VeridianOutputInfo output;
        output.connectorId = conn->connector_id;
        output.encoderId = conn->encoder_id;
        output.crtcId = 0;
        output.physicalWidth = conn->mmWidth;
        output.physicalHeight = conn->mmHeight;
        output.connected = (conn->connection == DRM_MODE_CONNECTED);
        output.enabled = output.connected;
        output.position = QPoint(0, 0);
        output.rotation = 0;
        output.scale = 1.0;
        output.isPrimary = false;

        /* Connector name */
        static const char *typeNames[] = {
            "Unknown", "VGA", "DVI-I", "DVI-D", "DVI-A", "Composite",
            "SVIDEO", "LVDS", "Component", "9PinDIN", "DisplayPort",
            "HDMI-A", "HDMI-B", "TV", "eDP", "Virtual", "DSI", "DPI"
        };
        int typeIdx = conn->connector_type;
        if (typeIdx < 0 || typeIdx > 17)
            typeIdx = 0;
        output.name = QStringLiteral("%1-%2")
                          .arg(typeNames[typeIdx])
                          .arg(conn->connector_type_id);

        /* Find assigned CRTC via encoder */
        if (conn->encoder_id) {
            drmModeEncoder *enc = drmModeGetEncoder(m_drmFd, conn->encoder_id);
            if (enc) {
                output.crtcId = enc->crtc_id;
                drmModeFreeEncoder(enc);
            }
        }

        /* Parse modes */
        output.modes = parseModes(conn);

        /* Current mode from CRTC */
        if (output.crtcId) {
            drmModeCrtc *crtc = drmModeGetCrtc(m_drmFd, output.crtcId);
            if (crtc && crtc->mode_valid) {
                output.currentResolution = QSize(crtc->mode.hdisplay,
                                                  crtc->mode.vdisplay);
                output.currentRefreshRate = crtc->mode.vrefresh * 1000;
            }
            if (crtc)
                drmModeFreeCrtc(crtc);
        }

        /* EDID */
        parseEdid(conn->connector_id, output);

        /* DPMS property */
        output.dpmsPropertyId = findProperty(conn->connector_id,
                                              DRM_MODE_OBJECT_CONNECTOR,
                                              "DPMS");

        m_outputs.append(output);
        drmModeFreeConnector(conn);
    }

    drmModeFreeResources(resources);

    /* Mark first connected output as primary if none set */
    bool hasPrimary = false;
    for (const auto &o : m_outputs) {
        if (o.isPrimary) { hasPrimary = true; break; }
    }
    if (!hasPrimary) {
        for (int i = 0; i < m_outputs.size(); ++i) {
            if (m_outputs[i].connected) {
                m_outputs[i].isPrimary = true;
                break;
            }
        }
    }

    qDebug("KScreen/Veridian: found %d output(s)", m_outputs.size());
    return true;
}

QVector<VeridianOutputInfo::ModeInfo> VeridianKScreenBackend::parseModes(
    drmModeConnector *connector) const
{
    QVector<VeridianOutputInfo::ModeInfo> modes;

    for (int i = 0; i < connector->count_modes; ++i) {
        VeridianOutputInfo::ModeInfo mode;
        mode.drmMode = connector->modes[i];
        mode.size = QSize(connector->modes[i].hdisplay,
                          connector->modes[i].vdisplay);
        mode.refreshRate = connector->modes[i].vrefresh * 1000; /* mHz */
        mode.preferred = (connector->modes[i].type & DRM_MODE_TYPE_PREFERRED);
        mode.id = QStringLiteral("%1x%2@%3")
                      .arg(mode.size.width())
                      .arg(mode.size.height())
                      .arg(connector->modes[i].vrefresh);

        modes.append(mode);
    }

    return modes;
}

bool VeridianKScreenBackend::parseEdid(uint32_t connectorId,
                                        VeridianOutputInfo &output)
{
    /* Find EDID property blob */
    drmModeObjectProperties *props =
        drmModeObjectGetProperties(m_drmFd, connectorId,
                                   DRM_MODE_OBJECT_CONNECTOR);
    if (!props)
        return false;

    for (uint32_t i = 0; i < props->count_props; ++i) {
        drmModePropertyRes *prop = drmModeGetProperty(m_drmFd, props->props[i]);
        if (!prop)
            continue;

        if (strcmp(prop->name, "EDID") == 0 &&
            (prop->flags & DRM_MODE_PROP_BLOB)) {
            drmModePropertyBlobRes *blob =
                drmModeGetPropertyBlob(m_drmFd, props->prop_values[i]);
            if (blob && blob->length >= 128) {
                const uint8_t *edid = static_cast<const uint8_t *>(blob->data);

                /* Manufacturer ID (bytes 8-9, PNP compressed ASCII) */
                uint16_t mfg = (edid[8] << 8) | edid[9];
                char m1 = static_cast<char>(((mfg >> 10) & 0x1F) + 'A' - 1);
                char m2 = static_cast<char>(((mfg >> 5) & 0x1F) + 'A' - 1);
                char m3 = static_cast<char>((mfg & 0x1F) + 'A' - 1);
                output.vendor = QStringLiteral("%1%2%3").arg(m1).arg(m2).arg(m3);

                /* Look for monitor name descriptor (tag 0xFC) */
                for (int d = 0; d < 4; ++d) {
                    int offset = 54 + d * 18;
                    if (offset + 18 > static_cast<int>(blob->length))
                        break;
                    if (edid[offset] == 0 && edid[offset + 1] == 0 &&
                        edid[offset + 3] == 0xFC) {
                        char name[14];
                        memcpy(name, edid + offset + 5, 13);
                        name[13] = '\0';
                        /* Trim trailing whitespace/newline */
                        for (int j = 12; j >= 0; --j) {
                            if (name[j] == '\n' || name[j] == ' ')
                                name[j] = '\0';
                            else
                                break;
                        }
                        output.model = QString::fromUtf8(name);
                    }
                    /* Serial descriptor (tag 0xFF) */
                    if (edid[offset] == 0 && edid[offset + 1] == 0 &&
                        edid[offset + 3] == 0xFF) {
                        char ser[14];
                        memcpy(ser, edid + offset + 5, 13);
                        ser[13] = '\0';
                        for (int j = 12; j >= 0; --j) {
                            if (ser[j] == '\n' || ser[j] == ' ')
                                ser[j] = '\0';
                            else
                                break;
                        }
                        output.serial = QString::fromUtf8(ser);
                    }
                }

                drmModeFreePropertyBlob(blob);
            }
        }
        drmModeFreeProperty(prop);
    }

    drmModeFreeObjectProperties(props);
    return true;
}

uint32_t VeridianKScreenBackend::findProperty(uint32_t objectId,
                                               uint32_t objectType,
                                               const char *name) const
{
    drmModeObjectProperties *props =
        drmModeObjectGetProperties(m_drmFd, objectId, objectType);
    if (!props)
        return 0;

    uint32_t result = 0;
    for (uint32_t i = 0; i < props->count_props; ++i) {
        drmModePropertyRes *prop = drmModeGetProperty(m_drmFd, props->props[i]);
        if (prop) {
            if (strcmp(prop->name, name) == 0)
                result = prop->prop_id;
            drmModeFreeProperty(prop);
            if (result)
                break;
        }
    }

    drmModeFreeObjectProperties(props);
    return result;
}

/* ========================================================================= */
/* KScreen mapping                                                           */
/* ========================================================================= */

void VeridianKScreenBackend::updateKScreenConfig()
{
    m_config = KScreen::ConfigPtr::create();

    /* Screen */
    m_config->setScreen(createKScreenScreen());

    /* Outputs */
    KScreen::OutputList outputList;
    for (const auto &output : m_outputs) {
        KScreen::OutputPtr kOutput = createKScreenOutput(output);
        outputList.insert(kOutput->id(), kOutput);
    }
    m_config->setOutputs(outputList);
}

KScreen::OutputPtr VeridianKScreenBackend::createKScreenOutput(
    const VeridianOutputInfo &output) const
{
    KScreen::OutputPtr kOutput = KScreen::OutputPtr::create();

    kOutput->setId(output.connectorId);
    kOutput->setName(output.name);
    kOutput->setConnected(output.connected);
    kOutput->setEnabled(output.enabled);
    kOutput->setPos(output.position);
    kOutput->setPrimary(output.isPrimary);
    kOutput->setScale(output.scale);
    kOutput->setSizeMm(QSize(output.physicalWidth, output.physicalHeight));

    /* Rotation */
    switch (output.rotation) {
    case 90:  kOutput->setRotation(KScreen::Output::Right); break;
    case 180: kOutput->setRotation(KScreen::Output::Inverted); break;
    case 270: kOutput->setRotation(KScreen::Output::Left); break;
    default:  kOutput->setRotation(KScreen::Output::None); break;
    }

    /* Modes */
    KScreen::ModeList modeList;
    QString preferredId;
    QString currentId;

    for (const auto &mode : output.modes) {
        KScreen::ModePtr kMode = createKScreenMode(mode);
        modeList.insert(kMode->id(), kMode);

        if (mode.preferred)
            preferredId = mode.id;

        if (mode.size == output.currentResolution &&
            mode.refreshRate == output.currentRefreshRate)
            currentId = mode.id;
    }

    kOutput->setModes(modeList);
    if (!preferredId.isEmpty())
        kOutput->setPreferredModes(QStringList{preferredId});
    if (!currentId.isEmpty())
        kOutput->setCurrentModeId(currentId);

    return kOutput;
}

KScreen::ModePtr VeridianKScreenBackend::createKScreenMode(
    const VeridianOutputInfo::ModeInfo &mode) const
{
    KScreen::ModePtr kMode = KScreen::ModePtr::create();

    kMode->setId(mode.id);
    kMode->setName(QStringLiteral("%1x%2@%3Hz")
                       .arg(mode.size.width())
                       .arg(mode.size.height())
                       .arg(mode.refreshRate / 1000));
    kMode->setSize(mode.size);
    kMode->setRefreshRate(mode.refreshRate / 1000.0);

    return kMode;
}

KScreen::ScreenPtr VeridianKScreenBackend::createKScreenScreen() const
{
    KScreen::ScreenPtr screen = KScreen::ScreenPtr::create();

    /* Calculate total virtual screen size from all enabled outputs */
    int maxX = 0;
    int maxY = 0;
    for (const auto &output : m_outputs) {
        if (!output.connected || !output.enabled)
            continue;
        int right = output.position.x() + output.currentResolution.width();
        int bottom = output.position.y() + output.currentResolution.height();
        maxX = qMax(maxX, right);
        maxY = qMax(maxY, bottom);
    }

    screen->setCurrentSize(QSize(maxX, maxY));
    screen->setMaxSize(QSize(16384, 16384));
    screen->setMinSize(QSize(320, 200));

    return screen;
}

} /* namespace KScreen */

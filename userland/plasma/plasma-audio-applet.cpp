/*
 * VeridianOS -- plasma-audio-applet.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Implementation of the Plasma audio volume applet.  Manages volume,
 * mute, and output device selection by communicating with the PipeWire
 * daemon through D-Bus and/or the PulseAudio compatibility API.
 *
 * Volume is stored as an integer 0-100 internally and mapped to the
 * PulseAudio 0-65536 linear scale for the wire protocol.
 */

#include "plasma-audio-applet.h"

#include <QDebug>
#include <QDBusReply>
#include <QDBusMessage>
#include <QDBusArgument>

namespace PlasmaAudio {

/* ========================================================================= */
/* Volume conversion helpers                                                 */
/* ========================================================================= */

/* PA_VOLUME_NORM = 0x10000 = 65536 */
static constexpr uint32_t PA_VOL_NORM = 0x10000U;

uint32_t PlasmaAudioApplet::volumeToPA(int vol) {
    if (vol <= 0) return 0;
    if (vol >= 100) return PA_VOL_NORM;
    /* Integer linear mapping: vol * 65536 / 100 */
    return (uint32_t)(((uint64_t)vol * PA_VOL_NORM + 50) / 100);
}

int PlasmaAudioApplet::volumeFromPA(uint32_t paVol) {
    if (paVol == 0) return 0;
    if (paVol >= PA_VOL_NORM) return 100;
    /* Integer linear mapping: paVol * 100 / 65536 */
    return (int)(((uint64_t)paVol * 100 + PA_VOL_NORM / 2) / PA_VOL_NORM);
}

/* ========================================================================= */
/* Constructor / Destructor                                                  */
/* ========================================================================= */

PlasmaAudioApplet::PlasmaAudioApplet(QObject *parent)
    : QObject(parent)
    , m_volume(100)
    , m_muted(false)
    , m_currentSink(0)
    , m_dbusIface(nullptr)
    , m_pollTimer(nullptr)
{
    qDebug() << "[plasma-audio] Initialising audio applet";

    /* Connect to PipeWire's D-Bus portal interface */
    m_dbusIface = new QDBusInterface(
        QStringLiteral("org.freedesktop.impl.portal.PipeWire"),
        QStringLiteral("/org/freedesktop/impl/portal/PipeWire"),
        QStringLiteral("org.freedesktop.impl.portal.PipeWire"),
        QDBusConnection::sessionBus(),
        this
    );

    if (!m_dbusIface->isValid()) {
        qDebug() << "[plasma-audio] D-Bus interface not available, "
                    "using direct PipeWire API fallback";
    }

    /* Create a default sink entry (built-in audio) */
    SinkInfo defaultSink;
    defaultSink.id          = 1;
    defaultSink.name        = QStringLiteral("alsa_output.pci-0000_00_1b.0.analog-stereo");
    defaultSink.description = QStringLiteral("Built-in Audio Analog Stereo");
    defaultSink.volume      = m_volume;
    defaultSink.muted       = m_muted;
    defaultSink.channels    = 2;
    m_sinks.append(defaultSink);

    /* Set up polling timer for sink state changes (every 2 seconds) */
    m_pollTimer = new QTimer(this);
    m_pollTimer->setInterval(2000);
    connect(m_pollTimer, &QTimer::timeout, this, &PlasmaAudioApplet::onPollTimer);
    m_pollTimer->start();

    qDebug() << "[plasma-audio] Audio applet ready, default sink:"
             << defaultSink.description;
}

PlasmaAudioApplet::~PlasmaAudioApplet() {
    if (m_pollTimer) {
        m_pollTimer->stop();
    }
    qDebug() << "[plasma-audio] Audio applet destroyed";
}

/* ========================================================================= */
/* Property accessors                                                        */
/* ========================================================================= */

int PlasmaAudioApplet::volume() const {
    return m_volume;
}

bool PlasmaAudioApplet::isMuted() const {
    return m_muted;
}

QString PlasmaAudioApplet::currentSinkName() const {
    if (m_currentSink >= 0 && m_currentSink < m_sinks.size()) {
        return m_sinks[m_currentSink].description;
    }
    return QStringLiteral("No output");
}

QStringList PlasmaAudioApplet::availableSinks() const {
    QStringList names;
    for (const auto &sink : m_sinks) {
        names.append(sink.description);
    }
    return names;
}

/* ========================================================================= */
/* Volume / Mute control                                                     */
/* ========================================================================= */

void PlasmaAudioApplet::setVolume(int vol) {
    /* Clamp to 0-100 */
    if (vol < 0) vol = 0;
    if (vol > 100) vol = 100;

    if (vol == m_volume) return;
    m_volume = vol;

    /* Update current sink info */
    if (m_currentSink >= 0 && m_currentSink < m_sinks.size()) {
        m_sinks[m_currentSink].volume = vol;
    }

    applyVolume();
    Q_EMIT volumeChanged(m_volume);
    Q_EMIT volumeOsd(m_volume, m_muted);

    qDebug() << "[plasma-audio] Volume set to" << m_volume;
}

void PlasmaAudioApplet::setMuted(bool mute) {
    if (mute == m_muted) return;
    m_muted = mute;

    /* Update current sink info */
    if (m_currentSink >= 0 && m_currentSink < m_sinks.size()) {
        m_sinks[m_currentSink].muted = mute;
    }

    applyVolume();
    Q_EMIT mutedChanged(m_muted);
    Q_EMIT volumeOsd(m_volume, m_muted);

    qDebug() << "[plasma-audio] Mute" << (mute ? "on" : "off");
}

void PlasmaAudioApplet::selectSink(const QString &sinkName) {
    for (int i = 0; i < m_sinks.size(); i++) {
        if (m_sinks[i].description == sinkName ||
            m_sinks[i].name == sinkName) {
            m_currentSink = i;

            /* Sync volume/mute from the selected sink */
            m_volume = m_sinks[i].volume;
            m_muted  = m_sinks[i].muted;

            Q_EMIT currentSinkChanged(m_sinks[i].description);
            Q_EMIT volumeChanged(m_volume);
            Q_EMIT mutedChanged(m_muted);

            qDebug() << "[plasma-audio] Switched to sink:" << sinkName;
            return;
        }
    }
    qDebug() << "[plasma-audio] Sink not found:" << sinkName;
}

void PlasmaAudioApplet::volumeUp(int step) {
    setVolume(m_volume + step);
}

void PlasmaAudioApplet::volumeDown(int step) {
    setVolume(m_volume - step);
}

void PlasmaAudioApplet::toggleMute() {
    setMuted(!m_muted);
}

/* ========================================================================= */
/* Sink enumeration                                                          */
/* ========================================================================= */

void PlasmaAudioApplet::refreshSinks() {
    /*
     * Query PipeWire for the current list of audio sinks.
     *
     * In a full implementation this would use either:
     *   1. D-Bus introspection of org.freedesktop.impl.portal.PipeWire
     *   2. PipeWire C API (pw_context_connect + registry listener)
     *   3. PulseAudio compat API (pa_context_get_sink_info_list)
     *
     * For now, we maintain the static sink list established at init time
     * and attempt a D-Bus query if the interface is available.
     */
    if (m_dbusIface && m_dbusIface->isValid()) {
        QDBusMessage reply = m_dbusIface->call(
            QStringLiteral("ListSinks"));

        if (reply.type() == QDBusMessage::ReplyMessage &&
            reply.arguments().size() > 0) {
            /* Parse sink list from D-Bus reply */
            qDebug() << "[plasma-audio] Got sink list from D-Bus";
            /* TODO: parse QDBusArgument for real sink data */
        }
    }

    Q_EMIT sinksChanged();
}

/* ========================================================================= */
/* Internal                                                                  */
/* ========================================================================= */

void PlasmaAudioApplet::applyVolume() {
    /*
     * Send the current volume to PipeWire/ALSA.
     *
     * Path 1: D-Bus SetSinkVolume(sink_id, pa_volume, muted)
     * Path 2: Direct AudioSetVolume syscall via the PulseAudio compat layer
     *
     * We try D-Bus first; if unavailable, fall back to a hypothetical
     * direct call.  In a fully wired system the PA compat layer would
     * handle this transparently.
     */
    if (m_currentSink < 0 || m_currentSink >= m_sinks.size()) return;

    const SinkInfo &sink = m_sinks[m_currentSink];
    uint32_t paVol = m_muted ? 0 : volumeToPA(m_volume);

    if (m_dbusIface && m_dbusIface->isValid()) {
        m_dbusIface->asyncCall(
            QStringLiteral("SetSinkVolume"),
            sink.id,
            paVol,
            m_muted
        );
    } else {
        /*
         * Fallback: use the kernel audio syscall directly.
         *
         * AudioSetVolume(stream_id=0, volume=0-100)
         * This is a shim -- the real connection goes through PipeWire's
         * ALSA bridge when the full stack is running.
         */
        qDebug() << "[plasma-audio] Direct volume apply:"
                 << "sink=" << sink.id
                 << "volume=" << m_volume
                 << "muted=" << m_muted;
    }
}

void PlasmaAudioApplet::onPollTimer() {
    /*
     * Periodic poll for external volume changes (e.g. another client
     * changed the volume, hardware knob, etc.).
     *
     * In a full PipeWire integration this would be replaced by a
     * D-Bus signal subscription (PropertiesChanged on the sink node).
     */

    if (!m_dbusIface || !m_dbusIface->isValid()) return;

    QDBusMessage reply = m_dbusIface->call(
        QStringLiteral("GetSinkVolume"),
        m_sinks[m_currentSink].id
    );

    if (reply.type() == QDBusMessage::ReplyMessage &&
        reply.arguments().size() >= 2) {
        uint32_t paVol = reply.arguments().at(0).toUInt();
        bool muted     = reply.arguments().at(1).toBool();

        int newVol = volumeFromPA(paVol);
        if (newVol != m_volume) {
            m_volume = newVol;
            if (m_currentSink >= 0 && m_currentSink < m_sinks.size()) {
                m_sinks[m_currentSink].volume = newVol;
            }
            Q_EMIT volumeChanged(m_volume);
        }
        if (muted != m_muted) {
            m_muted = muted;
            if (m_currentSink >= 0 && m_currentSink < m_sinks.size()) {
                m_sinks[m_currentSink].muted = muted;
            }
            Q_EMIT mutedChanged(m_muted);
        }
    }
}

}  /* namespace PlasmaAudio */

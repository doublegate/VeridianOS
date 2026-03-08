/*
 * VeridianOS -- plasma-audio-applet.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Plasma audio volume applet for VeridianOS.  Provides volume control,
 * mute toggle, and output device selection in the KDE Plasma system tray.
 * Connects to PipeWire via D-Bus (or direct C API) to manage audio
 * streams and sinks.
 *
 * Responsibilities:
 *   - Volume slider (0-100 mapped to PA_VOLUME_MUTED..PA_VOLUME_NORM)
 *   - Mute toggle per-sink
 *   - Output device enumeration and selection
 *   - Volume change OSD notifications
 *   - Keyboard media key handling (XF86AudioRaiseVolume, etc.)
 */

#ifndef PLASMA_AUDIO_APPLET_H
#define PLASMA_AUDIO_APPLET_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QDBusInterface>
#include <QDBusConnection>
#include <QTimer>

/* Forward declarations */
class QQuickItem;

namespace PlasmaAudio {

/* ========================================================================= */
/* SinkInfo -- represents a single audio output device                       */
/* ========================================================================= */

struct SinkInfo {
    uint32_t id;           /**< PipeWire node ID */
    QString  name;         /**< Internal name */
    QString  description;  /**< Human-readable name */
    int      volume;       /**< Volume 0-100 (integer) */
    bool     muted;        /**< Mute state */
    int      channels;     /**< Number of channels */
};

/* ========================================================================= */
/* PlasmaAudioApplet                                                         */
/* ========================================================================= */

class PlasmaAudioApplet : public QObject {
    Q_OBJECT

    Q_PROPERTY(int volume READ volume WRITE setVolume NOTIFY volumeChanged)
    Q_PROPERTY(bool muted READ isMuted WRITE setMuted NOTIFY mutedChanged)
    Q_PROPERTY(QString currentSinkName READ currentSinkName
               NOTIFY currentSinkChanged)
    Q_PROPERTY(QStringList availableSinks READ availableSinks
               NOTIFY sinksChanged)

public:
    explicit PlasmaAudioApplet(QObject *parent = nullptr);
    ~PlasmaAudioApplet() override;

    /* Property accessors */
    int volume() const;
    bool isMuted() const;
    QString currentSinkName() const;
    QStringList availableSinks() const;

public Q_SLOTS:
    /** Set volume (0-100). Emits volumeChanged(). */
    void setVolume(int vol);

    /** Toggle or set mute state. Emits mutedChanged(). */
    void setMuted(bool mute);

    /** Switch to a different output sink by name. */
    void selectSink(const QString &sinkName);

    /** Increase volume by step (default 5). */
    void volumeUp(int step = 5);

    /** Decrease volume by step (default 5). */
    void volumeDown(int step = 5);

    /** Toggle mute on/off. */
    void toggleMute();

    /** Refresh the list of available sinks from PipeWire. */
    void refreshSinks();

Q_SIGNALS:
    void volumeChanged(int newVolume);
    void mutedChanged(bool muted);
    void currentSinkChanged(const QString &sinkName);
    void sinksChanged();
    void volumeOsd(int volume, bool muted);

private Q_SLOTS:
    void onPollTimer();

private:
    /** Map integer volume 0-100 to PA volume 0-65536. */
    static uint32_t volumeToPA(int vol);

    /** Map PA volume 0-65536 to integer 0-100. */
    static int volumeFromPA(uint32_t paVol);

    /** Send volume change to PipeWire via the ALSA bridge. */
    void applyVolume();

    /* State */
    int         m_volume;       /**< Current volume 0-100 */
    bool        m_muted;        /**< Current mute state */
    int         m_currentSink;  /**< Index into m_sinks */
    QList<SinkInfo> m_sinks;    /**< Known audio sinks */

    /* D-Bus interface to PipeWire portal */
    QDBusInterface *m_dbusIface;

    /* Polling timer for sink state refresh */
    QTimer *m_pollTimer;
};

}  /* namespace PlasmaAudio */

#endif /* PLASMA_AUDIO_APPLET_H */

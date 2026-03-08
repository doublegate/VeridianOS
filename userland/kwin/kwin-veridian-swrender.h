/*
 * VeridianOS -- kwin-veridian-swrender.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Software rendering backend for KWin on VeridianOS.
 *
 * Provides optimized compositing for llvmpipe / software OpenGL
 * renderers where hardware VSync and GPU-accelerated compositing
 * are unavailable.  Key features:
 *
 *   - Per-surface damage tracking via QRegion
 *   - Double-buffered blit with software VSync pacing (~60Hz)
 *   - Frame time measurement and drop detection
 *   - Automatic fallback to full composite on excessive damage
 *
 * This plugin works alongside the DRM/KMS platform backend
 * (kwin-veridian-platform.h) and is activated when the EGL
 * renderer string contains "llvmpipe" or "softpipe".
 */

#ifndef KWIN_VERIDIAN_SWRENDER_H
#define KWIN_VERIDIAN_SWRENDER_H

#include <QObject>
#include <QRegion>
#include <QRect>
#include <QSize>
#include <QTimer>
#include <QElapsedTimer>
#include <QVector>

/* Forward declarations -- KWin / VeridianOS */
namespace KWin {
class VeridianEglBackend;
class VeridianDrmOutput;
}

namespace KWin {

/* ========================================================================= */
/* Frame timing statistics                                                   */
/* ========================================================================= */

struct SwRenderFrameStats {
    quint64 frameCount;        /* Total presented frames      */
    quint64 droppedFrames;     /* Frames exceeding 2x VSync   */
    qint64  avgFrameTimeMs;    /* Rolling average (ms)        */
    qint64  minFrameTimeMs;    /* Minimum observed (ms)       */
    qint64  maxFrameTimeMs;    /* Maximum observed (ms)       */
    int     estimatedFps;      /* Integer FPS estimate        */
};

/* ========================================================================= */
/* VeridianSwRenderer                                                        */
/* ========================================================================= */

/**
 * Software rendering optimizer for llvmpipe / software GL.
 *
 * Activated by VeridianDrmBackend when VeridianEglBackend::isLlvmpipe()
 * returns true.  Wraps the existing EGL rendering path with:
 *   1. Damage region tracking (only re-composite dirty areas)
 *   2. Software VSync timer (~60Hz busy-wait on QElapsedTimer)
 *   3. Double-buffer management (back-buffer render, front-buffer blit)
 *   4. Frame statistics for performance monitoring
 */
class VeridianSwRenderer : public QObject
{
    Q_OBJECT

public:
    /**
     * Construct the software renderer.
     *
     * @param eglBackend  The EGL backend used for GL calls.
     * @param parent      Parent QObject (typically VeridianDrmBackend).
     */
    explicit VeridianSwRenderer(VeridianEglBackend *eglBackend,
                                 QObject *parent = nullptr);
    ~VeridianSwRenderer() override;

    /* ----- Initialization ----- */

    /**
     * Initialize back-buffer, damage tracker, and VSync timer.
     * @return true on success.
     */
    bool initialize();

    /* ----- Per-frame operations ----- */

    /**
     * Set the damage region for the current frame.
     *
     * Called by the compositor before compositeFrame().  If the region
     * is empty the frame is skipped; if it equals the full output the
     * damage tracker falls back to a full blit.
     */
    void setDamageRegion(const QRegion &region);

    /**
     * Composite the current frame.
     *
     * Performs a partial or full blit of the back-buffer into the
     * scanout buffer based on the current damage region.
     *
     * @return true if the frame was presented.
     */
    bool compositeFrame();

    /**
     * Wait for the next software VSync tick.
     *
     * Busy-waits using QElapsedTimer until the 16ms (60Hz) deadline.
     */
    void waitVSync();

    /* ----- Queries ----- */

    /** Whether the GL renderer is llvmpipe/softpipe. */
    bool isLlvmpipe() const;

    /** Current output size. */
    QSize outputSize() const;

    /** Frame statistics. */
    SwRenderFrameStats frameStats() const;

    /** Whether the renderer has been initialized. */
    bool isInitialized() const;

    /** Number of damage rects in the current frame. */
    int currentDamageRectCount() const;

    /** Maximum damage rects before full-surface fallback. */
    static int maxDamageRects();

    /** Update output resolution (triggers full damage). */
    void setOutputSize(const QSize &size);

    /** Set target refresh rate in millihertz (e.g. 60000). */
    void setRefreshRate(int mhz);

    /** Enable or disable software VSync. */
    void setVSyncEnabled(bool enabled);

    /** Reset frame statistics counters. */
    void resetStats();

    /** Log a performance summary via qDebug. */
    void logPerformanceSummary() const;

Q_SIGNALS:
    /** Emitted when a frame is dropped (took > 2x VSync interval). */
    void frameDropped(quint64 frameNumber);

    /** Emitted after each presented frame with timing info. */
    void framePresented(qint64 frameTimeMs);

private:
    /* ----- Internal helpers ----- */
    void recordFrameTime(qint64 ms);
    void checkForDroppedFrame(qint64 ms);
    void updateStats();
    bool shouldDoFullComposite() const;
    QRegion mergedDamageRegion() const;

    /* ----- Members ----- */
    VeridianEglBackend *m_eglBackend;
    bool m_initialized;

    /* Damage tracking */
    QRegion m_currentDamage;
    QRegion m_previousDamage;
    int m_maxDamageRects;

    /* Output geometry */
    QSize m_outputSize;

    /* VSync timing */
    QElapsedTimer m_vsyncTimer;
    qint64 m_lastVsyncMs;
    qint64 m_vsyncIntervalMs;        /* 16 ms for 60 Hz */
    bool m_vsyncEnabled;

    /* Frame statistics */
    quint64 m_frameCount;
    quint64 m_droppedFrames;
    QVector<qint64> m_frameTimeHistory;
    int m_historyWriteIdx;
    qint64 m_minFrameTimeMs;
    qint64 m_maxFrameTimeMs;

    /* Double-buffer state */
    bool m_backBufferDirty;

    /* Constants */
    static const int FRAME_HISTORY_SIZE = 60;
    static const int MAX_DAMAGE_RECTS_DEFAULT = 32;
    static const qint64 VSYNC_INTERVAL_MS = 16;
    static const qint64 DROPPED_THRESHOLD_MS = 33;  /* 2x 16ms */
};

} /* namespace KWin */

#endif /* KWIN_VERIDIAN_SWRENDER_H */

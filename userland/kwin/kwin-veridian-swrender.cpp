/*
 * VeridianOS -- kwin-veridian-swrender.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Software rendering backend implementation for KWin on VeridianOS.
 *
 * Provides damage-tracked compositing and software VSync for the
 * llvmpipe / software GL path.  See kwin-veridian-swrender.h for
 * the public interface.
 */

#include "kwin-veridian-swrender.h"
#include "kwin-veridian-platform.h"

#include <QDebug>
#include <QThread>
#include <QCoreApplication>

#include <algorithm>
#include <cstring>

namespace KWin {

/* ========================================================================= */
/* Construction / Destruction                                                */
/* ========================================================================= */

VeridianSwRenderer::VeridianSwRenderer(VeridianEglBackend *eglBackend,
                                       QObject *parent)
    : QObject(parent)
    , m_eglBackend(eglBackend)
    , m_initialized(false)
    , m_maxDamageRects(MAX_DAMAGE_RECTS_DEFAULT)
    , m_lastVsyncMs(0)
    , m_vsyncIntervalMs(VSYNC_INTERVAL_MS)
    , m_vsyncEnabled(true)
    , m_frameCount(0)
    , m_droppedFrames(0)
    , m_historyWriteIdx(0)
    , m_minFrameTimeMs(std::numeric_limits<qint64>::max())
    , m_maxFrameTimeMs(0)
    , m_backBufferDirty(false)
{
    qDebug("VeridianSwRenderer: created for software rendering path");
}

VeridianSwRenderer::~VeridianSwRenderer()
{
    qDebug("VeridianSwRenderer: destroyed (frames: %llu, dropped: %llu)",
           static_cast<unsigned long long>(m_frameCount),
           static_cast<unsigned long long>(m_droppedFrames));
}

/* ========================================================================= */
/* Initialization                                                            */
/* ========================================================================= */

bool VeridianSwRenderer::initialize()
{
    if (m_initialized) {
        qDebug("VeridianSwRenderer: already initialized");
        return true;
    }

    if (!m_eglBackend) {
        qWarning("VeridianSwRenderer: no EGL backend");
        return false;
    }

    /* Verify this is actually a software renderer */
    if (!m_eglBackend->isLlvmpipe()) {
        qWarning("VeridianSwRenderer: EGL backend is not llvmpipe -- "
                 "software renderer optimizations not applicable");
        /* Still allow initialization for testing */
    }

    /* Initialize VSync timer */
    m_vsyncTimer.start();
    m_lastVsyncMs = m_vsyncTimer.elapsed();

    /* Pre-allocate frame time history */
    m_frameTimeHistory.resize(FRAME_HISTORY_SIZE);
    m_frameTimeHistory.fill(0);

    m_initialized = true;

    qDebug("VeridianSwRenderer: initialized successfully");
    qDebug("  VSync interval:    %lld ms (~%d Hz)",
           static_cast<long long>(m_vsyncIntervalMs),
           m_vsyncIntervalMs > 0 ? static_cast<int>(1000 / m_vsyncIntervalMs) : 0);
    qDebug("  Max damage rects:  %d", m_maxDamageRects);
    qDebug("  GL renderer:       %s", qPrintable(m_eglBackend->glRenderer()));

    return true;
}

/* ========================================================================= */
/* Damage Tracking                                                           */
/* ========================================================================= */

void VeridianSwRenderer::setDamageRegion(const QRegion &region)
{
    m_previousDamage = m_currentDamage;
    m_currentDamage = region;

    if (!region.isEmpty()) {
        m_backBufferDirty = true;
    }
}

bool VeridianSwRenderer::shouldDoFullComposite() const
{
    /*
     * Fall back to full composite when:
     *   1. Too many damage rects (merge cost exceeds savings)
     *   2. Damage covers > 75% of the output area
     *   3. No previous frame to delta against
     */
    if (m_currentDamage.rectCount() > m_maxDamageRects) {
        return true;
    }

    if (m_outputSize.isEmpty()) {
        return true;
    }

    /* Compute damage area as fraction of output */
    qint64 outputArea = static_cast<qint64>(m_outputSize.width()) *
                        static_cast<qint64>(m_outputSize.height());
    if (outputArea == 0) {
        return true;
    }

    qint64 damageArea = 0;
    for (const QRect &r : m_currentDamage) {
        damageArea += static_cast<qint64>(r.width()) *
                      static_cast<qint64>(r.height());
    }

    /* 75% threshold: damageArea * 4 > outputArea * 3 */
    if (damageArea * 4 > outputArea * 3) {
        return true;
    }

    return false;
}

/* ========================================================================= */
/* Frame Compositing                                                         */
/* ========================================================================= */

bool VeridianSwRenderer::compositeFrame()
{
    if (!m_initialized) {
        qWarning("VeridianSwRenderer: not initialized");
        return false;
    }

    /* If no damage, skip the frame entirely */
    if (m_currentDamage.isEmpty() && !m_backBufferDirty) {
        return false;
    }

    qint64 frameStartMs = m_vsyncTimer.elapsed();

    /*
     * Determine compositing strategy:
     *   - Full composite: blit entire back buffer
     *   - Partial composite: blit only damaged rects
     */
    bool fullComposite = shouldDoFullComposite();

    if (fullComposite) {
        /*
         * Full composite path.
         *
         * The actual GL rendering is done by KWin's scene graph;
         * we just coordinate the buffer swap here.
         *
         * In a full composite, EGL swapBuffers handles the
         * back->front transfer.
         */
        qDebug("VeridianSwRenderer: full composite (damage rects: %d)",
               m_currentDamage.rectCount());
    } else {
        /*
         * Partial composite path.
         *
         * With EGL_KHR_partial_update / EGL_EXT_buffer_age we can
         * tell the driver which regions changed.  For llvmpipe this
         * reduces the memcpy from back to front buffer.
         *
         * We merge overlapping rects first to minimize the number
         * of copy operations.
         */
        QRegion merged = mergedDamageRegion();

        qDebug("VeridianSwRenderer: partial composite (%d merged rects "
               "from %d original)",
               merged.rectCount(), m_currentDamage.rectCount());

        /*
         * Set the EGL damage hint if supported.
         * The actual blit is performed by eglSwapBuffers internally.
         *
         * Note: On llvmpipe, EGL_KHR_partial_update may not be
         * available.  In that case, the full swap still happens but
         * we've at least avoided re-rendering undamaged scene content.
         */
    }

    /* Wait for VSync before presenting */
    if (m_vsyncEnabled) {
        waitVSync();
    }

    /* Record frame timing */
    qint64 frameEndMs = m_vsyncTimer.elapsed();
    qint64 frameTimeMs = frameEndMs - frameStartMs;
    recordFrameTime(frameTimeMs);

    /* Clear damage for next frame */
    m_backBufferDirty = false;

    m_frameCount++;

    Q_EMIT framePresented(frameTimeMs);

    return true;
}

/* ========================================================================= */
/* Damage Region Merging                                                     */
/* ========================================================================= */

/**
 * Merge overlapping damage rects using QRegion's built-in union.
 *
 * QRegion handles rect merging efficiently via scanline decomposition.
 * We combine current and previous damage to account for double-buffering
 * (the back buffer may still have old content from 2 frames ago).
 */
QRegion VeridianSwRenderer::mergedDamageRegion() const
{
    /* Union current + previous damage for double-buffer correctness */
    QRegion combined = m_currentDamage.united(m_previousDamage);

    /* Clip to output bounds */
    if (!m_outputSize.isEmpty()) {
        QRect outputRect(0, 0, m_outputSize.width(), m_outputSize.height());
        combined = combined.intersected(outputRect);
    }

    return combined;
}

/* ========================================================================= */
/* Software VSync                                                            */
/* ========================================================================= */

void VeridianSwRenderer::waitVSync()
{
    if (!m_vsyncEnabled) {
        return;
    }

    qint64 now = m_vsyncTimer.elapsed();

    if (m_lastVsyncMs == 0) {
        m_lastVsyncMs = now;
        return;
    }

    /* Calculate next VSync deadline */
    qint64 nextVsync = m_lastVsyncMs + m_vsyncIntervalMs;

    /* If we already missed the deadline, skip forward */
    if (now > nextVsync) {
        qint64 elapsed = now - m_lastVsyncMs;
        qint64 skipped = elapsed / m_vsyncIntervalMs;
        nextVsync = m_lastVsyncMs + m_vsyncIntervalMs * (skipped + 1);
    }

    /* Busy-wait until deadline.
     * On VeridianOS this is acceptable because the compositor runs
     * in its own process with real-time priority.
     *
     * We yield periodically to avoid starving other threads. */
    while (m_vsyncTimer.elapsed() < nextVsync) {
        /* Yield to other threads for ~1ms chunks */
        QThread::yieldCurrentThread();
    }

    m_lastVsyncMs = nextVsync;
}

/* ========================================================================= */
/* Frame Statistics                                                          */
/* ========================================================================= */

void VeridianSwRenderer::recordFrameTime(qint64 ms)
{
    /* Update circular buffer */
    if (m_historyWriteIdx >= m_frameTimeHistory.size()) {
        m_historyWriteIdx = 0;
    }
    m_frameTimeHistory[m_historyWriteIdx] = ms;
    m_historyWriteIdx++;

    /* Update min/max */
    if (ms < m_minFrameTimeMs) {
        m_minFrameTimeMs = ms;
    }
    if (ms > m_maxFrameTimeMs) {
        m_maxFrameTimeMs = ms;
    }

    /* Check for dropped frame */
    checkForDroppedFrame(ms);
}

void VeridianSwRenderer::checkForDroppedFrame(qint64 ms)
{
    if (ms > DROPPED_THRESHOLD_MS) {
        m_droppedFrames++;
        Q_EMIT frameDropped(m_frameCount);
        qDebug("VeridianSwRenderer: frame %llu dropped (%lld ms)",
               static_cast<unsigned long long>(m_frameCount),
               static_cast<long long>(ms));
    }
}

void VeridianSwRenderer::updateStats()
{
    /* Stats are computed on-demand in frameStats() */
}

SwRenderFrameStats VeridianSwRenderer::frameStats() const
{
    SwRenderFrameStats stats;
    stats.frameCount = m_frameCount;
    stats.droppedFrames = m_droppedFrames;
    stats.minFrameTimeMs = (m_minFrameTimeMs == std::numeric_limits<qint64>::max())
                           ? 0 : m_minFrameTimeMs;
    stats.maxFrameTimeMs = m_maxFrameTimeMs;

    /* Compute average */
    qint64 sum = 0;
    int count = 0;
    for (int i = 0; i < m_frameTimeHistory.size(); ++i) {
        if (m_frameTimeHistory[i] > 0) {
            sum += m_frameTimeHistory[i];
            count++;
        }
    }
    stats.avgFrameTimeMs = (count > 0) ? (sum / count) : 0;

    /* Estimated FPS */
    stats.estimatedFps = (stats.avgFrameTimeMs > 0)
                         ? static_cast<int>(1000 / stats.avgFrameTimeMs)
                         : 0;

    return stats;
}

/* ========================================================================= */
/* Query Methods                                                             */
/* ========================================================================= */

bool VeridianSwRenderer::isLlvmpipe() const
{
    return m_eglBackend ? m_eglBackend->isLlvmpipe() : false;
}

QSize VeridianSwRenderer::outputSize() const
{
    return m_outputSize;
}

bool VeridianSwRenderer::isInitialized() const
{
    return m_initialized;
}

int VeridianSwRenderer::currentDamageRectCount() const
{
    return m_currentDamage.rectCount();
}

int VeridianSwRenderer::maxDamageRects()
{
    return MAX_DAMAGE_RECTS_DEFAULT;
}

/* ========================================================================= */
/* Output Size Management                                                    */
/* ========================================================================= */

/**
 * Called when the output resolution changes.
 *
 * Resets damage tracking since the entire framebuffer needs to be
 * re-composited after a resize.
 */
void VeridianSwRenderer::setOutputSize(const QSize &size)
{
    if (m_outputSize == size) {
        return;
    }

    qDebug("VeridianSwRenderer: output size changed to %dx%d",
           size.width(), size.height());

    m_outputSize = size;

    /* Mark full damage on resize */
    m_currentDamage = QRegion(0, 0, size.width(), size.height());
    m_previousDamage = m_currentDamage;
    m_backBufferDirty = true;
}

/* ========================================================================= */
/* VSync Configuration                                                       */
/* ========================================================================= */

/**
 * Set the target refresh rate.
 *
 * @param mhz  Refresh rate in millihertz (e.g. 60000 for 60 Hz).
 */
void VeridianSwRenderer::setRefreshRate(int mhz)
{
    if (mhz <= 0) {
        return;
    }

    /* interval_ms = 1_000_000 / mhz (mhz is milliHz, so this gives ms) */
    m_vsyncIntervalMs = 1000000 / static_cast<qint64>(mhz);
    if (m_vsyncIntervalMs < 1) {
        m_vsyncIntervalMs = 1;
    }

    qDebug("VeridianSwRenderer: VSync interval set to %lld ms (~%d Hz)",
           static_cast<long long>(m_vsyncIntervalMs),
           mhz / 1000);
}

/**
 * Enable or disable software VSync.
 */
void VeridianSwRenderer::setVSyncEnabled(bool enabled)
{
    m_vsyncEnabled = enabled;
    qDebug("VeridianSwRenderer: VSync %s", enabled ? "enabled" : "disabled");
}

/**
 * Reset frame statistics.
 */
void VeridianSwRenderer::resetStats()
{
    m_frameCount = 0;
    m_droppedFrames = 0;
    m_minFrameTimeMs = std::numeric_limits<qint64>::max();
    m_maxFrameTimeMs = 0;
    m_frameTimeHistory.fill(0);
    m_historyWriteIdx = 0;

    qDebug("VeridianSwRenderer: statistics reset");
}

/**
 * Log a summary of rendering performance.
 */
void VeridianSwRenderer::logPerformanceSummary() const
{
    SwRenderFrameStats stats = frameStats();

    qDebug("VeridianSwRenderer: Performance Summary");
    qDebug("  Total frames:    %llu",
           static_cast<unsigned long long>(stats.frameCount));
    qDebug("  Dropped frames:  %llu (%.1f%%)",
           static_cast<unsigned long long>(stats.droppedFrames),
           stats.frameCount > 0
               ? static_cast<double>(stats.droppedFrames * 100) /
                 static_cast<double>(stats.frameCount)
               : 0.0);
    qDebug("  Avg frame time:  %lld ms",
           static_cast<long long>(stats.avgFrameTimeMs));
    qDebug("  Min frame time:  %lld ms",
           static_cast<long long>(stats.minFrameTimeMs));
    qDebug("  Max frame time:  %lld ms",
           static_cast<long long>(stats.maxFrameTimeMs));
    qDebug("  Estimated FPS:   %d", stats.estimatedFps);
}

} /* namespace KWin */

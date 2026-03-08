/*
 * VeridianOS -- kwin-veridian-effects.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWin effects configuration and capability-based fallback for VeridianOS.
 *
 * Probes the GL renderer to determine whether GPU acceleration is
 * available (virgl, virtio-gpu, real hardware) or whether we're running
 * on llvmpipe (software rendering).  Configures KWin's effect system
 * accordingly:
 *
 *   GPU-accelerated (virgl / real HW):
 *     - All lightweight effects enabled
 *     - Blur effect enabled (Gaussian, radius=12)
 *     - Overview / Desktop Grid enabled
 *     - Wobbly windows available but off by default
 *
 *   Software rendering (llvmpipe):
 *     - Blur DISABLED (too expensive)
 *     - Wobbly windows DISABLED
 *     - Minimize / fade / slide animations reduced
 *     - Overview enabled (CPU-based fallback)
 *     - Desktop Grid enabled (basic rendering)
 *
 * This file is compiled into KWin's effect loader and called during
 * the compositing initialization phase.
 */

#include "kwin-veridian-platform.h"

#include <QDebug>
#include <QString>
#include <QStringList>
#include <QSettings>
#include <QStandardPaths>
#include <QDir>

#include <GLES2/gl2.h>

namespace KWin {

/* ========================================================================= */
/* GL capability detection                                                   */
/* ========================================================================= */

/**
 * Renderer capability level for effect configuration decisions.
 */
enum class GpuCapability {
    Software,       /* llvmpipe -- minimal effects */
    BasicGpu,       /* virgl / basic virtio-gpu -- moderate effects */
    FullGpu         /* real GPU passthrough -- all effects */
};

/**
 * Detect GPU capability level from the GL renderer string.
 *
 * On VeridianOS under QEMU:
 *   - "llvmpipe" -> Software
 *   - "virgl" / "virtio" -> BasicGpu
 *   - Anything else (real GPU via VFIO) -> FullGpu
 */
static GpuCapability detectGpuCapability(const QString &renderer,
                                          const QString &vendor)
{
    QString rendererLower = renderer.toLower();
    QString vendorLower = vendor.toLower();

    /* Software renderers */
    if (rendererLower.contains(QStringLiteral("llvmpipe")) ||
        rendererLower.contains(QStringLiteral("softpipe")) ||
        rendererLower.contains(QStringLiteral("swrast")) ||
        rendererLower.contains(QStringLiteral("software"))) {
        return GpuCapability::Software;
    }

    /* VirtIO GPU / virgl (paravirtualized) */
    if (rendererLower.contains(QStringLiteral("virgl")) ||
        rendererLower.contains(QStringLiteral("virtio"))) {
        return GpuCapability::BasicGpu;
    }

    /* Real GPU (VFIO passthrough, Intel, AMD, NVIDIA) */
    return GpuCapability::FullGpu;
}

/**
 * Query additional GL capabilities relevant for effects.
 */
struct GlCapabilities {
    int maxTextureSize;
    int maxRenderbufferSize;
    bool supportsFramebufferBlit;
    bool supportsNonPowerOfTwo;
    bool supportsFloatTextures;
    int maxDrawBuffers;
    int maxSamples;         /* MSAA */
};

static GlCapabilities queryGlCapabilities()
{
    GlCapabilities caps;
    memset(&caps, 0, sizeof(caps));

    glGetIntegerv(GL_MAX_TEXTURE_SIZE, &caps.maxTextureSize);
    glGetIntegerv(GL_MAX_RENDERBUFFER_SIZE, &caps.maxRenderbufferSize);

    /* Check extensions */
    const char *extensions = reinterpret_cast<const char *>(
        glGetString(GL_EXTENSIONS));
    QString extStr = extensions ? QString::fromUtf8(extensions) : QString();

    caps.supportsFramebufferBlit =
        extStr.contains(QStringLiteral("GL_NV_framebuffer_blit")) ||
        extStr.contains(QStringLiteral("GL_ANGLE_framebuffer_blit"));
    caps.supportsNonPowerOfTwo =
        extStr.contains(QStringLiteral("GL_OES_texture_npot"));
    caps.supportsFloatTextures =
        extStr.contains(QStringLiteral("GL_OES_texture_float")) ||
        extStr.contains(QStringLiteral("GL_EXT_color_buffer_float"));

    caps.maxDrawBuffers = 1;
    caps.maxSamples = 0;

    return caps;
}

/* ========================================================================= */
/* Effect configuration                                                      */
/* ========================================================================= */

/**
 * Per-effect configuration entry.
 */
struct EffectConfig {
    QString name;           /* KWin effect plugin name */
    bool enabled;           /* Whether to enable this effect */
    QStringList options;    /* Key=Value options for the effect */
};

/**
 * Build the effect configuration list based on GPU capability.
 *
 * Returns a list of effects with their enabled state and options.
 * This list is written to KWin's effect configuration file so that
 * the effect loader picks up the correct settings on startup.
 */
static QVector<EffectConfig> buildEffectList(GpuCapability capability,
                                              const GlCapabilities &glCaps)
{
    QVector<EffectConfig> effects;

    /* === Always-enabled lightweight effects === */

    /* Fade: window open/close fade animation */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_fade");
        e.enabled = true;
        if (capability == GpuCapability::Software) {
            e.options << QStringLiteral("FadeDuration=100");
        } else {
            e.options << QStringLiteral("FadeDuration=200");
        }
        effects.append(e);
    }

    /* Slide: virtual desktop switching animation */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_slide");
        e.enabled = true;
        if (capability == GpuCapability::Software) {
            e.options << QStringLiteral("Duration=150");
        } else {
            e.options << QStringLiteral("Duration=300");
        }
        effects.append(e);
    }

    /* Minimize animation (magic lamp or scale) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_minimize");
        e.enabled = true;
        if (capability == GpuCapability::Software) {
            /* Use simple scale animation on software renderer */
            e.options << QStringLiteral("AnimationType=Scale");
            e.options << QStringLiteral("Duration=100");
        } else {
            e.options << QStringLiteral("AnimationType=MagicLamp");
            e.options << QStringLiteral("Duration=200");
        }
        effects.append(e);
    }

    /* Window snap (quarter-tiling visual feedback) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_windowsnap");
        e.enabled = true;
        effects.append(e);
    }

    /* Translucency: inactive window dimming */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_translucency");
        e.enabled = true;
        e.options << QStringLiteral("InactiveOpacity=90");
        effects.append(e);
    }

    /* Highlight window (taskbar hover) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_highlightwindow");
        e.enabled = true;
        effects.append(e);
    }

    /* === Conditional effects (GPU capability dependent) === */

    /* Blur: background blur for transparent surfaces */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_blur");
        e.enabled = (capability != GpuCapability::Software);
        if (e.enabled) {
            e.options << QStringLiteral("BlurStrength=12");
            e.options << QStringLiteral("NoiseStrength=0");
            if (capability == GpuCapability::BasicGpu) {
                /* Reduced blur on paravirtualized GPU */
                e.options << QStringLiteral("BlurStrength=8");
            }
        }
        effects.append(e);
    }

    /* Overview: workspace overview (Meta key) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_overview");
        e.enabled = true; /* Works on CPU too, just slower */
        if (capability == GpuCapability::Software) {
            e.options << QStringLiteral("AnimationDuration=100");
        } else {
            e.options << QStringLiteral("AnimationDuration=300");
        }
        effects.append(e);
    }

    /* Desktop Grid: virtual desktop grid view */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_desktopgrid");
        e.enabled = true;
        if (capability == GpuCapability::Software) {
            e.options << QStringLiteral("AnimationDuration=100");
        }
        effects.append(e);
    }

    /* Wobbly windows */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_wobblywindows");
        /* Only on real GPU -- too expensive otherwise */
        e.enabled = (capability == GpuCapability::FullGpu);
        e.options << QStringLiteral("Stiffness=10");
        e.options << QStringLiteral("Drag=85");
        effects.append(e);
    }

    /* Screen edge glow (for hot corners) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_screenedge");
        e.enabled = (capability != GpuCapability::Software);
        effects.append(e);
    }

    /* Dim screen for logout/lock dialog */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_dimscreen");
        e.enabled = true;
        effects.append(e);
    }

    /* Present windows (Alt+Tab 3D) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_presentwindows");
        e.enabled = true;
        effects.append(e);
    }

    /* Screenshot effect (for Spectacle integration) */
    {
        EffectConfig e;
        e.name = QStringLiteral("kwin4_effect_screenshot");
        e.enabled = true;
        effects.append(e);
    }

    Q_UNUSED(glCaps); /* Reserved for future capability-based decisions */

    return effects;
}

/* ========================================================================= */
/* Configuration writer                                                      */
/* ========================================================================= */

/**
 * Write effect configuration to KWin's config file.
 *
 * Creates/updates ~/.config/kwinrc [Plugins] and [Effect-*] sections
 * with the appropriate enabled states and options.
 */
static bool writeEffectConfig(const QVector<EffectConfig> &effects)
{
    QString configPath = QStandardPaths::writableLocation(
        QStandardPaths::ConfigLocation);
    if (configPath.isEmpty())
        configPath = QDir::homePath() + QStringLiteral("/.config");

    QString kwinrcPath = configPath + QStringLiteral("/kwinrc");
    QSettings kwinrc(kwinrcPath, QSettings::IniFormat);

    /* Write [Plugins] section: EffectName=true/false */
    kwinrc.beginGroup(QStringLiteral("Plugins"));
    for (const EffectConfig &e : effects) {
        QString key = e.name + QStringLiteral("Enabled");
        kwinrc.setValue(key, e.enabled);
    }
    kwinrc.endGroup();

    /* Write [Effect-*] sections for per-effect options */
    for (const EffectConfig &e : effects) {
        if (e.options.isEmpty())
            continue;

        kwinrc.beginGroup(QStringLiteral("Effect-") + e.name);
        for (const QString &opt : e.options) {
            int eq = opt.indexOf('=');
            if (eq > 0) {
                QString key = opt.left(eq);
                QString val = opt.mid(eq + 1);
                kwinrc.setValue(key, val);
            }
        }
        kwinrc.endGroup();
    }

    kwinrc.sync();

    qDebug("VeridianEffects: wrote configuration to %s", qPrintable(kwinrcPath));
    return true;
}

/* ========================================================================= */
/* Public API                                                                */
/* ========================================================================= */

/**
 * Configure KWin effects for VeridianOS based on detected GPU capabilities.
 *
 * Called during KWin compositor initialization after EGL context creation.
 * Must be called with a valid GL context current.
 *
 * @param eglBackend  The EGL backend (for renderer info)
 * @return true if configuration was written successfully
 */
bool configureVeridianEffects(VeridianEglBackend *eglBackend)
{
    if (!eglBackend) {
        qWarning("VeridianEffects: no EGL backend -- using software defaults");
        QVector<EffectConfig> effects = buildEffectList(
            GpuCapability::Software, GlCapabilities());
        return writeEffectConfig(effects);
    }

    /* Detect GPU capability */
    GpuCapability capability = detectGpuCapability(
        eglBackend->glRenderer(), eglBackend->glVendor());

    const char *capNames[] = { "Software", "BasicGpu", "FullGpu" };
    qDebug("VeridianEffects: GPU capability = %s (renderer: %s)",
           capNames[static_cast<int>(capability)],
           qPrintable(eglBackend->glRenderer()));

    /* Query GL capabilities */
    GlCapabilities glCaps = queryGlCapabilities();

    qDebug("VeridianEffects: maxTexture=%d maxRB=%d NPOT=%d float=%d",
           glCaps.maxTextureSize, glCaps.maxRenderbufferSize,
           glCaps.supportsNonPowerOfTwo, glCaps.supportsFloatTextures);

    /* Build and write effect configuration */
    QVector<EffectConfig> effects = buildEffectList(capability, glCaps);

    /* Log effect summary */
    int enabled = 0, disabled = 0;
    for (const EffectConfig &e : effects) {
        if (e.enabled) {
            ++enabled;
        } else {
            ++disabled;
            qDebug("VeridianEffects: DISABLED %s", qPrintable(e.name));
        }
    }

    qDebug("VeridianEffects: %d effects enabled, %d disabled", enabled, disabled);

    return writeEffectConfig(effects);
}

/**
 * Check if a specific effect should be enabled on VeridianOS.
 *
 * Can be called at runtime to check individual effect availability
 * without regenerating the full configuration.
 *
 * @param effectName  KWin effect plugin name
 * @param isLlvmpipe  Whether we're on software renderer
 * @return true if the effect should be enabled
 */
bool isEffectSupportedOnVeridian(const QString &effectName, bool isLlvmpipe)
{
    /* Effects always disabled on software renderer */
    static const QStringList gpuOnlyEffects = {
        QStringLiteral("kwin4_effect_blur"),
        QStringLiteral("kwin4_effect_wobblywindows"),
        QStringLiteral("kwin4_effect_screenedge"),
    };

    if (isLlvmpipe && gpuOnlyEffects.contains(effectName))
        return false;

    return true;
}

/**
 * Get recommended compositing settings for VeridianOS.
 *
 * Returns key-value pairs for the [Compositing] section of kwinrc.
 */
QStringList getVeridianCompositingDefaults(bool isLlvmpipe)
{
    QStringList defaults;

    /* OpenGL ES 2.0 compositing backend */
    defaults << QStringLiteral("Backend=OpenGL");
    defaults << QStringLiteral("GLCore=false");
    defaults << QStringLiteral("GLPreferBufferSwap=a"); /* auto */

    if (isLlvmpipe) {
        /* Software rendering: reduce quality for performance */
        defaults << QStringLiteral("GLTextureFilter=0");     /* Bilinear */
        defaults << QStringLiteral("XRenderSmoothScale=false");
        defaults << QStringLiteral("AnimationSpeed=1");      /* Faster */
        defaults << QStringLiteral("MaxFPS=30");
        defaults << QStringLiteral("RefreshRate=30");
    } else {
        /* GPU: full quality */
        defaults << QStringLiteral("GLTextureFilter=2");     /* Trilinear */
        defaults << QStringLiteral("XRenderSmoothScale=true");
        defaults << QStringLiteral("AnimationSpeed=3");      /* Normal */
        defaults << QStringLiteral("MaxFPS=60");
        defaults << QStringLiteral("RefreshRate=60");
    }

    /* VSync via page flip (DRM backend handles this) */
    defaults << QStringLiteral("GLVSync=true");

    return defaults;
}

} /* namespace KWin */

/*
 * VeridianOS -- kwin-veridian-platform.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWin DRM/KMS platform backend for VeridianOS.  Provides the core
 * platform integration between KWin's compositing engine and the
 * VeridianOS DRM/KMS + GBM + EGL graphics stack.
 *
 * Responsibilities:
 *   - DRM device discovery (/dev/dri/card0)
 *   - KMS mode setting (CRTC, connector, encoder enumeration)
 *   - GBM buffer allocation for scanout surfaces
 *   - EGL context creation on GBM device/surface
 *   - VSync via DRM page flip events
 *   - Output configuration (resolution, refresh rate, DPMS)
 *   - Session management (logind integration via D-Bus)
 *
 * This plugin compiles against the KWin build tree and implements
 * KWin's Platform / OutputBackend interfaces.
 */

#ifndef KWIN_VERIDIAN_PLATFORM_H
#define KWIN_VERIDIAN_PLATFORM_H

#include <QObject>
#include <QString>
#include <QSize>
#include <QVector>
#include <QSocketNotifier>
#include <QDBusInterface>

/* KWin platform API headers (from KWin build tree) */
#include <kwin/core/outputbackend.h>
#include <kwin/core/output.h>
#include <kwin/core/session.h>
#include <kwin/core/renderbackend.h>

/* System headers */
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <gbm.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>

namespace KWin {

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

class VeridianDrmOutput;
class VeridianDrmGpu;
class VeridianEglBackend;
class VeridianSession;

/* ========================================================================= */
/* VeridianDrmConnector -- represents a physical display output              */
/* ========================================================================= */

struct VeridianDrmConnector {
    uint32_t connectorId;
    uint32_t encoderId;
    uint32_t crtcId;
    drmModeModeInfo preferredMode;
    drmModeModeInfo currentMode;
    QVector<drmModeModeInfo> modes;
    int physicalWidth;      /* mm */
    int physicalHeight;     /* mm */
    QString name;           /* e.g. "HDMI-A-1", "Virtual-1" */
    bool connected;
    uint32_t dpmsPropertyId;
    uint32_t crtcIdPropertyId;
};

/* ========================================================================= */
/* VeridianDrmCrtc -- represents a display pipeline                          */
/* ========================================================================= */

struct VeridianDrmCrtc {
    uint32_t crtcId;
    uint32_t bufferId;
    int x;
    int y;
    drmModeModeInfo mode;
    bool modeValid;
    uint32_t possibleCrtcs;     /* bitmask from encoder */
};

/* ========================================================================= */
/* VeridianGbmSurface -- GBM surface + EGL for one output                    */
/* ========================================================================= */

struct VeridianGbmSurface {
    struct gbm_surface *surface;
    struct gbm_bo *currentBo;
    struct gbm_bo *previousBo;
    uint32_t currentFbId;
    uint32_t previousFbId;
    EGLSurface eglSurface;
    int width;
    int height;
    uint32_t format;            /* GBM_FORMAT_XRGB8888 */
};

/* ========================================================================= */
/* VeridianDrmOutput -- per-output state (one per connected display)         */
/* ========================================================================= */

/**
 * Represents a single display output driven by DRM/KMS.
 *
 * Manages the CRTC <-> encoder <-> connector pipeline, GBM scanout
 * buffers, and page flip synchronization for one physical display.
 */
class VeridianDrmOutput : public QObject
{
    Q_OBJECT

public:
    explicit VeridianDrmOutput(int drmFd, QObject *parent = nullptr);
    ~VeridianDrmOutput() override;

    /* ----- Configuration ----- */
    bool initFromConnector(const VeridianDrmConnector &connector,
                           const VeridianDrmCrtc &crtc);
    bool setMode(const drmModeModeInfo &mode);
    bool setDpms(int level);            /* DRM_MODE_DPMS_ON / OFF / STANDBY */

    /* ----- GBM surface management ----- */
    bool createGbmSurface(struct gbm_device *gbmDevice);
    void destroyGbmSurface();
    struct gbm_surface *gbmSurface() const;

    /* ----- Page flip ----- */
    bool schedulePageFlip();
    void pageFlipComplete();
    bool isPageFlipPending() const;

    /* ----- Framebuffer management ----- */
    uint32_t addFbFromBo(struct gbm_bo *bo);
    void releasePreviousBuffer();

    /* ----- Queries ----- */
    QString name() const;
    QSize sizePixels() const;
    QSize sizeMillimeters() const;
    int refreshRate() const;            /* mHz */
    uint32_t connectorId() const;
    uint32_t crtcId() const;
    QVector<drmModeModeInfo> availableModes() const;
    drmModeModeInfo currentMode() const;

Q_SIGNALS:
    void pageFlipped();
    void modeChanged(const QSize &size, int refreshRate);

private:
    int m_drmFd;
    VeridianDrmConnector m_connector;
    VeridianDrmCrtc m_crtc;
    VeridianGbmSurface m_gbm;
    bool m_pageFlipPending;
    bool m_dpmsOn;
};

/* ========================================================================= */
/* VeridianSession -- logind session management via D-Bus                    */
/* ========================================================================= */

/**
 * Manages the graphical session via systemd-logind D-Bus interface.
 *
 * On VeridianOS this talks to the logind shim (Sprint 9.5) which
 * provides TakeDevice / ReleaseDevice / TakeControl for DRM device
 * access and VT switching.
 */
class VeridianSession : public QObject
{
    Q_OBJECT

public:
    explicit VeridianSession(QObject *parent = nullptr);
    ~VeridianSession() override;

    bool open();
    void close();

    /* ----- Device management ----- */
    int takeDevice(const QString &path);
    void releaseDevice(int fd);
    bool takeControl();
    void releaseControl();

    /* ----- Session info ----- */
    QString sessionId() const;
    QString seatId() const;
    unsigned int vt() const;
    bool isActive() const;

    /* ----- VT switching ----- */
    void switchTo(unsigned int vt);

Q_SIGNALS:
    void activeChanged(bool active);
    void devicePaused(int major, int minor);
    void deviceResumed(int fd);

private Q_SLOTS:
    void onPropertiesChanged(const QString &interface,
                             const QVariantMap &changed,
                             const QStringList &invalidated);
    void onPauseDevice(uint32_t major, uint32_t minor, const QString &type);
    void onResumeDevice(uint32_t major, uint32_t minor, int fd);

private:
    bool findSession();
    bool connectToLogind();

    QDBusInterface *m_logindSession;
    QDBusInterface *m_logindSeat;
    QString m_sessionId;
    QString m_sessionPath;
    QString m_seatId;
    unsigned int m_vt;
    bool m_active;
    bool m_hasControl;
};

/* ========================================================================= */
/* VeridianEglBackend -- EGL/GLES rendering on GBM                           */
/* ========================================================================= */

/**
 * Manages the EGL display, contexts, and surfaces for OpenGL ES 2.0
 * compositing on GBM-backed scanout buffers.
 */
class VeridianEglBackend : public QObject
{
    Q_OBJECT

public:
    explicit VeridianEglBackend(struct gbm_device *gbmDevice,
                                QObject *parent = nullptr);
    ~VeridianEglBackend() override;

    /* ----- Initialization ----- */
    bool initEgl();
    bool createContext();

    /* ----- Surface management ----- */
    EGLSurface createSurface(struct gbm_surface *gbmSurface);
    void destroySurface(EGLSurface surface);

    /* ----- Context operations ----- */
    bool makeCurrent(EGLSurface surface);
    bool swapBuffers(EGLSurface surface);
    void doneCurrent();

    /* ----- Queries ----- */
    EGLDisplay eglDisplay() const;
    EGLContext eglContext() const;
    EGLConfig eglConfig() const;
    bool supportsBufferAge() const;
    bool supportsSurfacelessContext() const;
    int glMajorVersion() const;
    int glMinorVersion() const;
    QString glRenderer() const;
    QString glVendor() const;

    /* ----- Extension checks ----- */
    bool hasExtension(const char *extension) const;
    bool isLlvmpipe() const;

private:
    bool chooseConfig();
    bool checkExtensions();
    void queryGlInfo();

    struct gbm_device *m_gbmDevice;
    EGLDisplay m_display;
    EGLContext m_context;
    EGLConfig m_config;
    int m_glMajor;
    int m_glMinor;
    QString m_glRenderer;
    QString m_glVendor;
    bool m_supportsBufferAge;
    bool m_supportsSurfaceless;
    bool m_isLlvmpipe;
    QStringList m_eglExtensions;
};

/* ========================================================================= */
/* VeridianDrmBackend -- top-level DRM/KMS output backend                    */
/* ========================================================================= */

/**
 * Top-level KWin output backend for VeridianOS DRM/KMS.
 *
 * Discovers DRM devices, enumerates connectors/CRTCs/encoders, creates
 * VeridianDrmOutput instances for each connected display, manages the
 * GBM device, and coordinates page flips and VSync.
 *
 * Lifecycle:
 *   1. VeridianSession::open() acquires the session
 *   2. openDrmDevice() opens /dev/dri/card0 via logind TakeDevice
 *   3. initGbm() creates GBM device on the DRM fd
 *   4. scanConnectors() enumerates outputs
 *   5. For each output: createGbmSurface() + EGL surface
 *   6. Rendering loop: makeCurrent -> draw -> swapBuffers -> pageFlip
 *   7. VSync via DRM_EVENT_FLIP_COMPLETE on drmFd
 */
class VeridianDrmBackend : public QObject
{
    Q_OBJECT

public:
    explicit VeridianDrmBackend(QObject *parent = nullptr);
    ~VeridianDrmBackend() override;

    /* ----- Initialization ----- */
    bool initialize();
    bool openDrmDevice(const QString &path = QStringLiteral("/dev/dri/card0"));
    bool initGbm();
    bool scanConnectors();
    bool initEglBackend();

    /* ----- Output management ----- */
    QVector<VeridianDrmOutput *> outputs() const;
    VeridianDrmOutput *primaryOutput() const;
    bool updateOutputs();

    /* ----- Rendering ----- */
    VeridianEglBackend *eglBackend() const;
    bool beginFrame(VeridianDrmOutput *output);
    bool endFrame(VeridianDrmOutput *output);

    /* ----- Session ----- */
    VeridianSession *session() const;

    /* ----- Device info ----- */
    int drmFd() const;
    struct gbm_device *gbmDevice() const;
    QString driverName() const;
    bool supportsAtomicModesetting() const;

Q_SIGNALS:
    void outputAdded(VeridianDrmOutput *output);
    void outputRemoved(VeridianDrmOutput *output);
    void outputsQueried();

private Q_SLOTS:
    void onDrmEvent();
    void onSessionActiveChanged(bool active);

private:
    /* ----- DRM helpers ----- */
    QVector<VeridianDrmConnector> enumerateConnectors();
    QVector<VeridianDrmCrtc> enumerateCrtcs();
    bool assignCrtcs(const QVector<VeridianDrmConnector> &connectors,
                     const QVector<VeridianDrmCrtc> &crtcs);
    uint32_t findDpmsProperty(uint32_t connectorId);
    uint32_t findCrtcIdProperty(uint32_t connectorId);

    /* ----- Page flip handler ----- */
    static void pageFlipHandler(int fd, unsigned int sequence,
                                unsigned int tvSec, unsigned int tvUsec,
                                void *userData);

    VeridianSession *m_session;
    VeridianEglBackend *m_eglBackend;
    QVector<VeridianDrmOutput *> m_outputs;
    QSocketNotifier *m_drmNotifier;

    int m_drmFd;
    struct gbm_device *m_gbmDevice;
    QString m_driverName;
    bool m_supportsAtomic;
};

} /* namespace KWin */

#endif /* KWIN_VERIDIAN_PLATFORM_H */

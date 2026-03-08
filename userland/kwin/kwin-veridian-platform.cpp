/*
 * VeridianOS -- kwin-veridian-platform.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWin DRM/KMS platform backend implementation for VeridianOS.
 *
 * Implements the complete DRM/KMS output pipeline:
 *   1. Session acquisition via logind D-Bus (VeridianSession)
 *   2. DRM device open via TakeDevice
 *   3. GBM device creation for buffer allocation
 *   4. Connector/CRTC/encoder enumeration and assignment
 *   5. GBM surface creation per output
 *   6. EGL context on GBM for OpenGL ES 2.0 compositing
 *   7. Page flip with VSync via DRM_EVENT_FLIP_COMPLETE
 */

#include "kwin-veridian-platform.h"

#include <QDebug>
#include <QDir>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusReply>
#include <QDBusPendingReply>
#include <QDBusArgument>
#include <QDBusMetaType>
#include <QFile>
#include <QSocketNotifier>
#include <QCoreApplication>

#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>

namespace KWin {

/* ========================================================================= */
/* VeridianDrmOutput                                                         */
/* ========================================================================= */

VeridianDrmOutput::VeridianDrmOutput(int drmFd, QObject *parent)
    : QObject(parent)
    , m_drmFd(drmFd)
    , m_pageFlipPending(false)
    , m_dpmsOn(true)
{
    memset(&m_connector, 0, sizeof(m_connector));
    memset(&m_crtc, 0, sizeof(m_crtc));
    memset(&m_gbm, 0, sizeof(m_gbm));
}

VeridianDrmOutput::~VeridianDrmOutput()
{
    destroyGbmSurface();
}

bool VeridianDrmOutput::initFromConnector(const VeridianDrmConnector &connector,
                                          const VeridianDrmCrtc &crtc)
{
    m_connector = connector;
    m_crtc = crtc;

    /* Use preferred mode, or first available if no preferred */
    if (connector.preferredMode.clock != 0) {
        m_connector.currentMode = connector.preferredMode;
    } else if (!connector.modes.isEmpty()) {
        m_connector.currentMode = connector.modes.first();
    } else {
        qWarning("VeridianDrmOutput: no modes available for connector %s",
                 qPrintable(connector.name));
        return false;
    }

    qDebug("VeridianDrmOutput: initialized %s -- %dx%d@%dHz",
           qPrintable(connector.name),
           m_connector.currentMode.hdisplay,
           m_connector.currentMode.vdisplay,
           m_connector.currentMode.vrefresh);

    return true;
}

bool VeridianDrmOutput::setMode(const drmModeModeInfo &mode)
{
    int ret = drmModeSetCrtc(m_drmFd, m_crtc.crtcId,
                             m_gbm.currentFbId, 0, 0,
                             &m_connector.connectorId, 1, const_cast<drmModeModeInfo*>(&mode));
    if (ret != 0) {
        qWarning("VeridianDrmOutput: drmModeSetCrtc failed: %s", strerror(errno));
        return false;
    }

    m_connector.currentMode = mode;
    Q_EMIT modeChanged(QSize(mode.hdisplay, mode.vdisplay), mode.vrefresh * 1000);
    return true;
}

bool VeridianDrmOutput::setDpms(int level)
{
    if (m_connector.dpmsPropertyId == 0)
        return false;

    int ret = drmModeConnectorSetProperty(m_drmFd, m_connector.connectorId,
                                          m_connector.dpmsPropertyId, level);
    if (ret != 0) {
        qWarning("VeridianDrmOutput: DPMS set failed: %s", strerror(errno));
        return false;
    }

    m_dpmsOn = (level == 0); /* DRM_MODE_DPMS_ON = 0 */
    return true;
}

bool VeridianDrmOutput::createGbmSurface(struct gbm_device *gbmDevice)
{
    destroyGbmSurface();

    m_gbm.width = m_connector.currentMode.hdisplay;
    m_gbm.height = m_connector.currentMode.vdisplay;
    m_gbm.format = GBM_FORMAT_XRGB8888;

    m_gbm.surface = gbm_surface_create(gbmDevice,
                                        m_gbm.width, m_gbm.height,
                                        m_gbm.format,
                                        GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING);
    if (!m_gbm.surface) {
        qWarning("VeridianDrmOutput: gbm_surface_create failed");
        return false;
    }

    qDebug("VeridianDrmOutput: created GBM surface %dx%d for %s",
           m_gbm.width, m_gbm.height, qPrintable(m_connector.name));
    return true;
}

void VeridianDrmOutput::destroyGbmSurface()
{
    if (m_gbm.previousFbId)
        drmModeRmFB(m_drmFd, m_gbm.previousFbId);
    if (m_gbm.currentFbId)
        drmModeRmFB(m_drmFd, m_gbm.currentFbId);
    if (m_gbm.previousBo)
        gbm_surface_release_buffer(m_gbm.surface, m_gbm.previousBo);
    if (m_gbm.currentBo)
        gbm_surface_release_buffer(m_gbm.surface, m_gbm.currentBo);
    if (m_gbm.surface)
        gbm_surface_destroy(m_gbm.surface);

    memset(&m_gbm, 0, sizeof(m_gbm));
}

struct gbm_surface *VeridianDrmOutput::gbmSurface() const
{
    return m_gbm.surface;
}

bool VeridianDrmOutput::schedulePageFlip()
{
    if (m_pageFlipPending)
        return false;

    int ret = drmModePageFlip(m_drmFd, m_crtc.crtcId, m_gbm.currentFbId,
                              DRM_MODE_PAGE_FLIP_EVENT, this);
    if (ret != 0) {
        qWarning("VeridianDrmOutput: page flip failed: %s", strerror(errno));
        return false;
    }

    m_pageFlipPending = true;
    return true;
}

void VeridianDrmOutput::pageFlipComplete()
{
    m_pageFlipPending = false;
    releasePreviousBuffer();
    Q_EMIT pageFlipped();
}

bool VeridianDrmOutput::isPageFlipPending() const
{
    return m_pageFlipPending;
}

uint32_t VeridianDrmOutput::addFbFromBo(struct gbm_bo *bo)
{
    uint32_t width = gbm_bo_get_width(bo);
    uint32_t height = gbm_bo_get_height(bo);
    uint32_t stride = gbm_bo_get_stride(bo);
    uint32_t handle = gbm_bo_get_handle(bo).u32;
    uint32_t fbId = 0;

    int ret = drmModeAddFB(m_drmFd, width, height, 24, 32, stride, handle, &fbId);
    if (ret != 0) {
        qWarning("VeridianDrmOutput: drmModeAddFB failed: %s", strerror(errno));
        return 0;
    }

    /* Rotate buffers */
    m_gbm.previousBo = m_gbm.currentBo;
    m_gbm.previousFbId = m_gbm.currentFbId;
    m_gbm.currentBo = bo;
    m_gbm.currentFbId = fbId;

    return fbId;
}

void VeridianDrmOutput::releasePreviousBuffer()
{
    if (m_gbm.previousBo) {
        if (m_gbm.previousFbId)
            drmModeRmFB(m_drmFd, m_gbm.previousFbId);
        gbm_surface_release_buffer(m_gbm.surface, m_gbm.previousBo);
        m_gbm.previousBo = nullptr;
        m_gbm.previousFbId = 0;
    }
}

QString VeridianDrmOutput::name() const { return m_connector.name; }

QSize VeridianDrmOutput::sizePixels() const
{
    return QSize(m_connector.currentMode.hdisplay,
                 m_connector.currentMode.vdisplay);
}

QSize VeridianDrmOutput::sizeMillimeters() const
{
    return QSize(m_connector.physicalWidth, m_connector.physicalHeight);
}

int VeridianDrmOutput::refreshRate() const
{
    return m_connector.currentMode.vrefresh * 1000; /* mHz */
}

uint32_t VeridianDrmOutput::connectorId() const { return m_connector.connectorId; }
uint32_t VeridianDrmOutput::crtcId() const { return m_crtc.crtcId; }

QVector<drmModeModeInfo> VeridianDrmOutput::availableModes() const
{
    return m_connector.modes;
}

drmModeModeInfo VeridianDrmOutput::currentMode() const
{
    return m_connector.currentMode;
}

/* ========================================================================= */
/* VeridianSession                                                           */
/* ========================================================================= */

VeridianSession::VeridianSession(QObject *parent)
    : QObject(parent)
    , m_logindSession(nullptr)
    , m_logindSeat(nullptr)
    , m_vt(0)
    , m_active(false)
    , m_hasControl(false)
{
}

VeridianSession::~VeridianSession()
{
    close();
}

bool VeridianSession::open()
{
    if (!findSession())
        return false;
    if (!connectToLogind())
        return false;
    return takeControl();
}

void VeridianSession::close()
{
    if (m_hasControl)
        releaseControl();

    delete m_logindSession;
    m_logindSession = nullptr;
    delete m_logindSeat;
    m_logindSeat = nullptr;
}

bool VeridianSession::findSession()
{
    /* Query logind for our session ID.
     * On VeridianOS the logind shim provides this via D-Bus. */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.login1"),
        QStringLiteral("/org/freedesktop/login1"),
        QStringLiteral("org.freedesktop.login1.Manager"),
        QStringLiteral("GetSessionByPID"));
    msg << static_cast<uint32_t>(getpid());

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage) {
        qWarning("VeridianSession: GetSessionByPID failed: %s",
                 qPrintable(reply.errorMessage()));
        return false;
    }

    m_sessionPath = reply.arguments().first().value<QDBusObjectPath>().path();

    /* Extract session ID from path (last component) */
    m_sessionId = m_sessionPath.mid(m_sessionPath.lastIndexOf('/') + 1);

    qDebug("VeridianSession: found session %s at %s",
           qPrintable(m_sessionId), qPrintable(m_sessionPath));

    return true;
}

bool VeridianSession::connectToLogind()
{
    m_logindSession = new QDBusInterface(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QDBusConnection::systemBus(), this);

    if (!m_logindSession->isValid()) {
        qWarning("VeridianSession: invalid session D-Bus interface");
        return false;
    }

    /* Read session properties */
    m_seatId = m_logindSession->property("Seat").toString();
    m_vt = m_logindSession->property("VTNr").toUInt();
    m_active = m_logindSession->property("Active").toBool();

    /* Monitor property changes for session activation/deactivation */
    QDBusConnection::systemBus().connect(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.DBus.Properties"),
        QStringLiteral("PropertiesChanged"),
        this, SLOT(onPropertiesChanged(QString,QVariantMap,QStringList)));

    /* Monitor device pause/resume */
    QDBusConnection::systemBus().connect(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("PauseDevice"),
        this, SLOT(onPauseDevice(uint32_t,uint32_t,QString)));

    QDBusConnection::systemBus().connect(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("ResumeDevice"),
        this, SLOT(onResumeDevice(uint32_t,uint32_t,int)));

    qDebug("VeridianSession: connected to logind -- seat=%s vt=%u active=%d",
           qPrintable(m_seatId), m_vt, m_active);

    return true;
}

bool VeridianSession::takeControl()
{
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("TakeControl"));
    msg << false; /* force = false */

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage) {
        qWarning("VeridianSession: TakeControl failed: %s",
                 qPrintable(reply.errorMessage()));
        return false;
    }

    m_hasControl = true;
    qDebug("VeridianSession: TakeControl succeeded");
    return true;
}

void VeridianSession::releaseControl()
{
    if (!m_hasControl)
        return;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("ReleaseControl"));

    QDBusConnection::systemBus().call(msg);
    m_hasControl = false;
}

int VeridianSession::takeDevice(const QString &path)
{
    /* Stat the device to get major/minor numbers */
    struct stat st;
    if (stat(path.toUtf8().constData(), &st) != 0) {
        qWarning("VeridianSession: stat(%s) failed: %s",
                 qPrintable(path), strerror(errno));
        return -1;
    }

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("TakeDevice"));
    msg << static_cast<uint32_t>(major(st.st_rdev))
        << static_cast<uint32_t>(minor(st.st_rdev));

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage) {
        qWarning("VeridianSession: TakeDevice(%s) failed: %s",
                 qPrintable(path), qPrintable(reply.errorMessage()));
        return -1;
    }

    /* TakeDevice returns (fd, inactive) */
    int fd = reply.arguments().first().toInt();
    qDebug("VeridianSession: TakeDevice(%s) -> fd %d", qPrintable(path), fd);
    return fd;
}

void VeridianSession::releaseDevice(int fd)
{
    struct stat st;
    if (fstat(fd, &st) != 0)
        return;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.login1"),
        m_sessionPath,
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("ReleaseDevice"));
    msg << static_cast<uint32_t>(major(st.st_rdev))
        << static_cast<uint32_t>(minor(st.st_rdev));

    QDBusConnection::systemBus().call(msg);
}

QString VeridianSession::sessionId() const { return m_sessionId; }
QString VeridianSession::seatId() const { return m_seatId; }
unsigned int VeridianSession::vt() const { return m_vt; }
bool VeridianSession::isActive() const { return m_active; }

void VeridianSession::switchTo(unsigned int vt)
{
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.login1"),
        QStringLiteral("/org/freedesktop/login1/seat/") + m_seatId,
        QStringLiteral("org.freedesktop.login1.Seat"),
        QStringLiteral("SwitchTo"));
    msg << vt;
    QDBusConnection::systemBus().call(msg);
}

void VeridianSession::onPropertiesChanged(const QString &interface,
                                          const QVariantMap &changed,
                                          const QStringList &invalidated)
{
    Q_UNUSED(interface);
    Q_UNUSED(invalidated);

    if (changed.contains(QStringLiteral("Active"))) {
        bool active = changed.value(QStringLiteral("Active")).toBool();
        if (active != m_active) {
            m_active = active;
            Q_EMIT activeChanged(m_active);
        }
    }
}

void VeridianSession::onPauseDevice(uint32_t major, uint32_t minor,
                                    const QString &type)
{
    Q_UNUSED(type);
    Q_EMIT devicePaused(major, minor);

    /* Auto-complete pause for "pause" type (required by logind protocol) */
    if (type == QStringLiteral("pause")) {
        QDBusMessage msg = QDBusMessage::createMethodCall(
            QStringLiteral("org.freedesktop.login1"),
            m_sessionPath,
            QStringLiteral("org.freedesktop.login1.Session"),
            QStringLiteral("PauseDeviceComplete"));
        msg << major << minor;
        QDBusConnection::systemBus().call(msg);
    }
}

void VeridianSession::onResumeDevice(uint32_t major, uint32_t minor, int fd)
{
    Q_UNUSED(major);
    Q_UNUSED(minor);
    Q_EMIT deviceResumed(fd);
}

/* ========================================================================= */
/* VeridianEglBackend                                                        */
/* ========================================================================= */

VeridianEglBackend::VeridianEglBackend(struct gbm_device *gbmDevice,
                                       QObject *parent)
    : QObject(parent)
    , m_gbmDevice(gbmDevice)
    , m_display(EGL_NO_DISPLAY)
    , m_context(EGL_NO_CONTEXT)
    , m_config(nullptr)
    , m_glMajor(0)
    , m_glMinor(0)
    , m_supportsBufferAge(false)
    , m_supportsSurfaceless(false)
    , m_isLlvmpipe(false)
{
}

VeridianEglBackend::~VeridianEglBackend()
{
    if (m_display != EGL_NO_DISPLAY) {
        eglMakeCurrent(m_display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        if (m_context != EGL_NO_CONTEXT)
            eglDestroyContext(m_display, m_context);
        eglTerminate(m_display);
    }
}

bool VeridianEglBackend::initEgl()
{
    /* Get EGL display from GBM device.
     * Use eglGetPlatformDisplay if available (EGL 1.5), otherwise
     * fall back to eglGetDisplay. */
    PFNEGLGETPLATFORMDISPLAYEXTPROC getPlatformDisplay =
        reinterpret_cast<PFNEGLGETPLATFORMDISPLAYEXTPROC>(
            eglGetProcAddress("eglGetPlatformDisplayEXT"));

    if (getPlatformDisplay) {
        m_display = getPlatformDisplay(EGL_PLATFORM_GBM_KHR,
                                       m_gbmDevice, nullptr);
    } else {
        m_display = eglGetDisplay(reinterpret_cast<EGLNativeDisplayType>(m_gbmDevice));
    }

    if (m_display == EGL_NO_DISPLAY) {
        qWarning("VeridianEglBackend: eglGetDisplay failed");
        return false;
    }

    EGLint major, minor;
    if (!eglInitialize(m_display, &major, &minor)) {
        qWarning("VeridianEglBackend: eglInitialize failed: 0x%x", eglGetError());
        return false;
    }

    qDebug("VeridianEglBackend: EGL %d.%d initialized", major, minor);

    if (!eglBindAPI(EGL_OPENGL_ES_API)) {
        qWarning("VeridianEglBackend: eglBindAPI(GLES) failed");
        return false;
    }

    if (!checkExtensions())
        return false;

    if (!chooseConfig())
        return false;

    return true;
}

bool VeridianEglBackend::chooseConfig()
{
    const EGLint configAttribs[] = {
        EGL_SURFACE_TYPE,    EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,        8,
        EGL_GREEN_SIZE,      8,
        EGL_BLUE_SIZE,       8,
        EGL_ALPHA_SIZE,      0,
        EGL_NONE
    };

    EGLint numConfigs;
    if (!eglChooseConfig(m_display, configAttribs, &m_config, 1, &numConfigs)
        || numConfigs == 0) {
        qWarning("VeridianEglBackend: eglChooseConfig failed");
        return false;
    }

    return true;
}

bool VeridianEglBackend::checkExtensions()
{
    const char *extensions = eglQueryString(m_display, EGL_EXTENSIONS);
    if (extensions) {
        m_eglExtensions = QString::fromUtf8(extensions).split(' ');
    }

    m_supportsBufferAge = hasExtension("EGL_EXT_buffer_age");
    m_supportsSurfaceless = hasExtension("EGL_KHR_surfaceless_context");

    qDebug("VeridianEglBackend: buffer_age=%d surfaceless=%d",
           m_supportsBufferAge, m_supportsSurfaceless);

    return true;
}

bool VeridianEglBackend::createContext()
{
    const EGLint contextAttribs[] = {
        EGL_CONTEXT_CLIENT_VERSION, 2,
        EGL_NONE
    };

    m_context = eglCreateContext(m_display, m_config,
                                EGL_NO_CONTEXT, contextAttribs);
    if (m_context == EGL_NO_CONTEXT) {
        qWarning("VeridianEglBackend: eglCreateContext failed: 0x%x",
                 eglGetError());
        return false;
    }

    /* Make current with no surface to query GL info */
    if (m_supportsSurfaceless) {
        eglMakeCurrent(m_display, EGL_NO_SURFACE, EGL_NO_SURFACE, m_context);
        queryGlInfo();
        eglMakeCurrent(m_display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    }

    return true;
}

void VeridianEglBackend::queryGlInfo()
{
    const char *version = reinterpret_cast<const char *>(glGetString(GL_VERSION));
    const char *renderer = reinterpret_cast<const char *>(glGetString(GL_RENDERER));
    const char *vendor = reinterpret_cast<const char *>(glGetString(GL_VENDOR));

    if (version) {
        /* Parse "OpenGL ES X.Y ..." */
        QString v = QString::fromUtf8(version);
        QStringList parts = v.split(' ');
        for (const QString &p : parts) {
            if (p.contains('.')) {
                QStringList ver = p.split('.');
                if (ver.size() >= 2) {
                    bool ok1, ok2;
                    int maj = ver[0].toInt(&ok1);
                    int min = ver[1].toInt(&ok2);
                    if (ok1 && ok2) {
                        m_glMajor = maj;
                        m_glMinor = min;
                        break;
                    }
                }
            }
        }
    }

    m_glRenderer = renderer ? QString::fromUtf8(renderer) : QStringLiteral("Unknown");
    m_glVendor = vendor ? QString::fromUtf8(vendor) : QStringLiteral("Unknown");

    /* Detect llvmpipe (Mesa software renderer) */
    m_isLlvmpipe = m_glRenderer.contains(QStringLiteral("llvmpipe"),
                                          Qt::CaseInsensitive);

    qDebug("VeridianEglBackend: GL ES %d.%d -- %s (%s)%s",
           m_glMajor, m_glMinor,
           qPrintable(m_glRenderer), qPrintable(m_glVendor),
           m_isLlvmpipe ? " [SOFTWARE]" : "");
}

EGLSurface VeridianEglBackend::createSurface(struct gbm_surface *gbmSurface)
{
    EGLSurface surface = eglCreateWindowSurface(
        m_display, m_config,
        reinterpret_cast<EGLNativeWindowType>(gbmSurface), nullptr);

    if (surface == EGL_NO_SURFACE) {
        qWarning("VeridianEglBackend: eglCreateWindowSurface failed: 0x%x",
                 eglGetError());
    }

    return surface;
}

void VeridianEglBackend::destroySurface(EGLSurface surface)
{
    if (surface != EGL_NO_SURFACE)
        eglDestroySurface(m_display, surface);
}

bool VeridianEglBackend::makeCurrent(EGLSurface surface)
{
    if (!eglMakeCurrent(m_display, surface, surface, m_context)) {
        qWarning("VeridianEglBackend: eglMakeCurrent failed: 0x%x", eglGetError());
        return false;
    }
    return true;
}

bool VeridianEglBackend::swapBuffers(EGLSurface surface)
{
    if (!eglSwapBuffers(m_display, surface)) {
        qWarning("VeridianEglBackend: eglSwapBuffers failed: 0x%x", eglGetError());
        return false;
    }
    return true;
}

void VeridianEglBackend::doneCurrent()
{
    eglMakeCurrent(m_display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
}

EGLDisplay VeridianEglBackend::eglDisplay() const { return m_display; }
EGLContext VeridianEglBackend::eglContext() const { return m_context; }
EGLConfig VeridianEglBackend::eglConfig() const { return m_config; }
bool VeridianEglBackend::supportsBufferAge() const { return m_supportsBufferAge; }
bool VeridianEglBackend::supportsSurfacelessContext() const { return m_supportsSurfaceless; }
int VeridianEglBackend::glMajorVersion() const { return m_glMajor; }
int VeridianEglBackend::glMinorVersion() const { return m_glMinor; }
QString VeridianEglBackend::glRenderer() const { return m_glRenderer; }
QString VeridianEglBackend::glVendor() const { return m_glVendor; }

bool VeridianEglBackend::hasExtension(const char *extension) const
{
    return m_eglExtensions.contains(QString::fromUtf8(extension));
}

bool VeridianEglBackend::isLlvmpipe() const { return m_isLlvmpipe; }

/* ========================================================================= */
/* VeridianDrmBackend                                                        */
/* ========================================================================= */

VeridianDrmBackend::VeridianDrmBackend(QObject *parent)
    : QObject(parent)
    , m_session(nullptr)
    , m_eglBackend(nullptr)
    , m_drmNotifier(nullptr)
    , m_drmFd(-1)
    , m_gbmDevice(nullptr)
    , m_supportsAtomic(false)
{
}

VeridianDrmBackend::~VeridianDrmBackend()
{
    qDeleteAll(m_outputs);
    m_outputs.clear();

    delete m_eglBackend;

    if (m_gbmDevice)
        gbm_device_destroy(m_gbmDevice);

    if (m_drmFd >= 0) {
        if (m_session)
            m_session->releaseDevice(m_drmFd);
        ::close(m_drmFd);
    }

    if (m_session)
        m_session->close();
    delete m_session;
}

bool VeridianDrmBackend::initialize()
{
    /* Step 1: Acquire session */
    m_session = new VeridianSession(this);
    if (!m_session->open()) {
        qWarning("VeridianDrmBackend: session open failed");
        return false;
    }

    connect(m_session, &VeridianSession::activeChanged,
            this, &VeridianDrmBackend::onSessionActiveChanged);

    /* Step 2: Open DRM device */
    if (!openDrmDevice())
        return false;

    /* Step 3: Create GBM device */
    if (!initGbm())
        return false;

    /* Step 4: Initialize EGL */
    if (!initEglBackend())
        return false;

    /* Step 5: Enumerate outputs */
    if (!scanConnectors())
        return false;

    /* Step 6: Set up DRM event monitoring for page flips */
    m_drmNotifier = new QSocketNotifier(m_drmFd, QSocketNotifier::Read, this);
    connect(m_drmNotifier, &QSocketNotifier::activated,
            this, &VeridianDrmBackend::onDrmEvent);

    qDebug("VeridianDrmBackend: initialized with %d output(s)", m_outputs.size());
    return true;
}

bool VeridianDrmBackend::openDrmDevice(const QString &path)
{
    /* Open DRM device via logind TakeDevice for proper permissions */
    m_drmFd = m_session->takeDevice(path);
    if (m_drmFd < 0) {
        /* Fallback: direct open (for testing without logind) */
        m_drmFd = ::open(path.toUtf8().constData(), O_RDWR | O_CLOEXEC);
        if (m_drmFd < 0) {
            qWarning("VeridianDrmBackend: cannot open %s: %s",
                     qPrintable(path), strerror(errno));
            return false;
        }
    }

    /* Query driver name */
    drmVersion *version = drmGetVersion(m_drmFd);
    if (version) {
        m_driverName = QString::fromUtf8(version->name, version->name_len);
        drmFreeVersion(version);
    }

    /* Check for atomic modesetting support */
    m_supportsAtomic = (drmSetClientCap(m_drmFd, DRM_CLIENT_CAP_ATOMIC, 1) == 0);

    /* Enable universal planes */
    drmSetClientCap(m_drmFd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1);

    qDebug("VeridianDrmBackend: opened %s (driver: %s, atomic: %d)",
           qPrintable(path), qPrintable(m_driverName), m_supportsAtomic);

    return true;
}

bool VeridianDrmBackend::initGbm()
{
    m_gbmDevice = gbm_create_device(m_drmFd);
    if (!m_gbmDevice) {
        qWarning("VeridianDrmBackend: gbm_create_device failed");
        return false;
    }

    qDebug("VeridianDrmBackend: GBM device created (backend: %s)",
           gbm_device_get_backend_name(m_gbmDevice));
    return true;
}

bool VeridianDrmBackend::initEglBackend()
{
    m_eglBackend = new VeridianEglBackend(m_gbmDevice, this);

    if (!m_eglBackend->initEgl()) {
        qWarning("VeridianDrmBackend: EGL init failed");
        return false;
    }

    if (!m_eglBackend->createContext()) {
        qWarning("VeridianDrmBackend: EGL context creation failed");
        return false;
    }

    return true;
}

bool VeridianDrmBackend::scanConnectors()
{
    QVector<VeridianDrmConnector> connectors = enumerateConnectors();
    QVector<VeridianDrmCrtc> crtcs = enumerateCrtcs();

    if (connectors.isEmpty()) {
        qWarning("VeridianDrmBackend: no connected outputs found");
        return false;
    }

    if (!assignCrtcs(connectors, crtcs))
        return false;

    /* Create output objects for connected displays */
    for (const VeridianDrmConnector &conn : connectors) {
        if (!conn.connected || conn.crtcId == 0)
            continue;

        /* Find the CRTC data */
        VeridianDrmCrtc crtc;
        bool found = false;
        for (const VeridianDrmCrtc &c : crtcs) {
            if (c.crtcId == conn.crtcId) {
                crtc = c;
                found = true;
                break;
            }
        }
        if (!found)
            continue;

        auto *output = new VeridianDrmOutput(m_drmFd, this);
        if (!output->initFromConnector(conn, crtc)) {
            delete output;
            continue;
        }

        /* Create GBM surface and EGL surface for this output */
        if (!output->createGbmSurface(m_gbmDevice)) {
            delete output;
            continue;
        }

        EGLSurface eglSurface = m_eglBackend->createSurface(output->gbmSurface());
        if (eglSurface == EGL_NO_SURFACE) {
            delete output;
            continue;
        }

        /* Set initial mode */
        m_eglBackend->makeCurrent(eglSurface);

        /* Clear to dark background for initial frame */
        glClearColor(0.1f, 0.1f, 0.1f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);

        m_eglBackend->swapBuffers(eglSurface);

        /* Lock the front buffer and create a DRM framebuffer */
        struct gbm_bo *bo = gbm_surface_lock_front_buffer(output->gbmSurface());
        if (bo) {
            uint32_t fbId = output->addFbFromBo(bo);
            if (fbId > 0) {
                /* Set initial CRTC mode */
                drmModeModeInfo mode = output->currentMode();
                drmModeSetCrtc(m_drmFd, crtc.crtcId, fbId, 0, 0,
                               &conn.connectorId, 1, &mode);
            }
        }

        m_eglBackend->doneCurrent();

        m_outputs.append(output);
        Q_EMIT outputAdded(output);
    }

    Q_EMIT outputsQueried();
    return !m_outputs.isEmpty();
}

QVector<VeridianDrmConnector> VeridianDrmBackend::enumerateConnectors()
{
    QVector<VeridianDrmConnector> result;

    drmModeRes *resources = drmModeGetResources(m_drmFd);
    if (!resources)
        return result;

    for (int i = 0; i < resources->count_connectors; ++i) {
        drmModeConnector *conn = drmModeGetConnector(m_drmFd,
                                                      resources->connectors[i]);
        if (!conn)
            continue;

        VeridianDrmConnector info;
        info.connectorId = conn->connector_id;
        info.encoderId = conn->encoder_id;
        info.crtcId = 0;
        info.physicalWidth = conn->mmWidth;
        info.physicalHeight = conn->mmHeight;
        info.connected = (conn->connection == DRM_MODE_CONNECTED);
        info.dpmsPropertyId = findDpmsProperty(conn->connector_id);
        info.crtcIdPropertyId = findCrtcIdProperty(conn->connector_id);

        /* Build connector name from type + type_id */
        static const char *typeNames[] = {
            "Unknown", "VGA", "DVI-I", "DVI-D", "DVI-A", "Composite",
            "SVIDEO", "LVDS", "Component", "9PinDIN", "DisplayPort",
            "HDMI-A", "HDMI-B", "TV", "eDP", "Virtual", "DSI", "DPI"
        };
        int typeIdx = conn->connector_type;
        if (typeIdx < 0 || typeIdx > 17)
            typeIdx = 0;
        info.name = QStringLiteral("%1-%2")
                        .arg(typeNames[typeIdx])
                        .arg(conn->connector_type_id);

        /* Enumerate modes */
        memset(&info.preferredMode, 0, sizeof(info.preferredMode));
        for (int m = 0; m < conn->count_modes; ++m) {
            info.modes.append(conn->modes[m]);
            if (conn->modes[m].type & DRM_MODE_TYPE_PREFERRED)
                info.preferredMode = conn->modes[m];
        }

        result.append(info);
        drmModeFreeConnector(conn);
    }

    drmModeFreeResources(resources);
    return result;
}

QVector<VeridianDrmCrtc> VeridianDrmBackend::enumerateCrtcs()
{
    QVector<VeridianDrmCrtc> result;

    drmModeRes *resources = drmModeGetResources(m_drmFd);
    if (!resources)
        return result;

    for (int i = 0; i < resources->count_crtcs; ++i) {
        drmModeCrtc *crtc = drmModeGetCrtc(m_drmFd, resources->crtcs[i]);
        if (!crtc)
            continue;

        VeridianDrmCrtc info;
        info.crtcId = crtc->crtc_id;
        info.bufferId = crtc->buffer_id;
        info.x = crtc->x;
        info.y = crtc->y;
        info.mode = crtc->mode;
        info.modeValid = crtc->mode_valid;
        info.possibleCrtcs = (1u << i);  /* bit position in possible_crtcs */

        result.append(info);
        drmModeFreeCrtc(crtc);
    }

    drmModeFreeResources(resources);
    return result;
}

bool VeridianDrmBackend::assignCrtcs(const QVector<VeridianDrmConnector> &connectors,
                                     const QVector<VeridianDrmCrtc> &crtcs)
{
    /* Simple greedy CRTC assignment: for each connected connector,
     * find its current encoder's CRTC or the first available one. */
    QVector<bool> crtcUsed(crtcs.size(), false);

    for (int i = 0; i < connectors.size(); ++i) {
        if (!connectors[i].connected)
            continue;

        /* Try the encoder's current CRTC first */
        if (connectors[i].encoderId) {
            drmModeEncoder *enc = drmModeGetEncoder(m_drmFd, connectors[i].encoderId);
            if (enc) {
                for (int c = 0; c < crtcs.size(); ++c) {
                    if (crtcs[c].crtcId == enc->crtc_id && !crtcUsed[c]) {
                        const_cast<VeridianDrmConnector &>(connectors[i]).crtcId =
                            crtcs[c].crtcId;
                        crtcUsed[c] = true;
                        break;
                    }
                }
                drmModeFreeEncoder(enc);
            }
        }

        /* If no CRTC assigned yet, find first available compatible one */
        if (connectors[i].crtcId == 0) {
            for (int c = 0; c < crtcs.size(); ++c) {
                if (!crtcUsed[c]) {
                    const_cast<VeridianDrmConnector &>(connectors[i]).crtcId =
                        crtcs[c].crtcId;
                    crtcUsed[c] = true;
                    break;
                }
            }
        }
    }

    return true;
}

uint32_t VeridianDrmBackend::findDpmsProperty(uint32_t connectorId)
{
    drmModeObjectProperties *props =
        drmModeObjectGetProperties(m_drmFd, connectorId,
                                   DRM_MODE_OBJECT_CONNECTOR);
    if (!props)
        return 0;

    uint32_t result = 0;
    for (uint32_t i = 0; i < props->count_props; ++i) {
        drmModePropertyRes *prop = drmModeGetProperty(m_drmFd, props->props[i]);
        if (prop) {
            if (strcmp(prop->name, "DPMS") == 0)
                result = prop->prop_id;
            drmModeFreeProperty(prop);
            if (result)
                break;
        }
    }

    drmModeFreeObjectProperties(props);
    return result;
}

uint32_t VeridianDrmBackend::findCrtcIdProperty(uint32_t connectorId)
{
    drmModeObjectProperties *props =
        drmModeObjectGetProperties(m_drmFd, connectorId,
                                   DRM_MODE_OBJECT_CONNECTOR);
    if (!props)
        return 0;

    uint32_t result = 0;
    for (uint32_t i = 0; i < props->count_props; ++i) {
        drmModePropertyRes *prop = drmModeGetProperty(m_drmFd, props->props[i]);
        if (prop) {
            if (strcmp(prop->name, "CRTC_ID") == 0)
                result = prop->prop_id;
            drmModeFreeProperty(prop);
            if (result)
                break;
        }
    }

    drmModeFreeObjectProperties(props);
    return result;
}

void VeridianDrmBackend::pageFlipHandler(int fd, unsigned int sequence,
                                         unsigned int tvSec, unsigned int tvUsec,
                                         void *userData)
{
    Q_UNUSED(fd);
    Q_UNUSED(sequence);
    Q_UNUSED(tvSec);
    Q_UNUSED(tvUsec);

    auto *output = static_cast<VeridianDrmOutput *>(userData);
    output->pageFlipComplete();
}

void VeridianDrmBackend::onDrmEvent()
{
    drmEventContext ctx;
    memset(&ctx, 0, sizeof(ctx));
    ctx.version = 2;
    ctx.page_flip_handler = pageFlipHandler;
    drmHandleEvent(m_drmFd, &ctx);
}

void VeridianDrmBackend::onSessionActiveChanged(bool active)
{
    if (active) {
        /* Session resumed: re-scan outputs, re-set modes */
        qDebug("VeridianDrmBackend: session activated");
        updateOutputs();
    } else {
        /* Session suspended: stop rendering */
        qDebug("VeridianDrmBackend: session deactivated");
    }
}

QVector<VeridianDrmOutput *> VeridianDrmBackend::outputs() const { return m_outputs; }

VeridianDrmOutput *VeridianDrmBackend::primaryOutput() const
{
    return m_outputs.isEmpty() ? nullptr : m_outputs.first();
}

bool VeridianDrmBackend::updateOutputs()
{
    /* Re-enumerate connectors and update output list */
    return scanConnectors();
}

VeridianEglBackend *VeridianDrmBackend::eglBackend() const { return m_eglBackend; }
VeridianSession *VeridianDrmBackend::session() const { return m_session; }
int VeridianDrmBackend::drmFd() const { return m_drmFd; }
struct gbm_device *VeridianDrmBackend::gbmDevice() const { return m_gbmDevice; }
QString VeridianDrmBackend::driverName() const { return m_driverName; }
bool VeridianDrmBackend::supportsAtomicModesetting() const { return m_supportsAtomic; }

bool VeridianDrmBackend::beginFrame(VeridianDrmOutput *output)
{
    if (!output || !output->gbmSurface())
        return false;

    /* Wait for previous page flip to complete */
    while (output->isPageFlipPending()) {
        drmEventContext ctx;
        memset(&ctx, 0, sizeof(ctx));
        ctx.version = 2;
        ctx.page_flip_handler = pageFlipHandler;
        drmHandleEvent(m_drmFd, &ctx);
    }

    return true;
}

bool VeridianDrmBackend::endFrame(VeridianDrmOutput *output)
{
    if (!output || !output->gbmSurface())
        return false;

    /* Lock the front buffer from GBM */
    struct gbm_bo *bo = gbm_surface_lock_front_buffer(output->gbmSurface());
    if (!bo) {
        qWarning("VeridianDrmBackend: gbm_surface_lock_front_buffer failed");
        return false;
    }

    /* Create DRM framebuffer from GBM buffer object */
    uint32_t fbId = output->addFbFromBo(bo);
    if (fbId == 0)
        return false;

    /* Schedule page flip */
    return output->schedulePageFlip();
}

} /* namespace KWin */

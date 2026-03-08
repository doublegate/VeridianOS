/*
 * VeridianOS -- qveridianglcontext.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * EGL-based OpenGL ES 2.0 context for VeridianOS.  Uses the Mesa EGL
 * implementation provided by the VeridianOS sysroot.
 */

#include "qveridianglcontext.h"

#include <EGL/egl.h>
#include <EGL/eglext.h>

QT_BEGIN_NAMESPACE

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

QVeridianGLContext::QVeridianGLContext(QOpenGLContext *context)
{
    Q_UNUSED(context);

    /* Set up the surface format for OpenGL ES 2.0 */
    m_format.setRenderableType(QSurfaceFormat::OpenGLES);
    m_format.setMajorVersion(2);
    m_format.setMinorVersion(0);
    m_format.setRedBufferSize(8);
    m_format.setGreenBufferSize(8);
    m_format.setBlueBufferSize(8);
    m_format.setAlphaBufferSize(8);
    m_format.setDepthBufferSize(24);
    m_format.setStencilBufferSize(8);
    m_format.setSwapBehavior(QSurfaceFormat::DoubleBuffer);

    initEGL();
}

QVeridianGLContext::~QVeridianGLContext()
{
    destroyEGL();
}

/* ========================================================================= */
/* EGL lifecycle                                                             */
/* ========================================================================= */

void QVeridianGLContext::initEGL()
{
    m_eglDisplay = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (m_eglDisplay == EGL_NO_DISPLAY)
        return;

    EGLint major, minor;
    if (!eglInitialize(m_eglDisplay, &major, &minor))
        return;

    eglBindAPI(EGL_OPENGL_ES_API);

    /* Choose an EGL config matching our surface format */
    const EGLint configAttribs[] = {
        EGL_SURFACE_TYPE,    EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,        8,
        EGL_GREEN_SIZE,      8,
        EGL_BLUE_SIZE,       8,
        EGL_ALPHA_SIZE,      8,
        EGL_DEPTH_SIZE,      24,
        EGL_STENCIL_SIZE,    8,
        EGL_NONE,
    };

    EGLint numConfigs;
    eglChooseConfig(m_eglDisplay, configAttribs, &m_eglConfig, 1, &numConfigs);
    if (numConfigs == 0)
        return;

    /* Create the EGL context */
    const EGLint contextAttribs[] = {
        EGL_CONTEXT_CLIENT_VERSION, 2,
        EGL_NONE,
    };

    m_eglContext = eglCreateContext(m_eglDisplay, m_eglConfig,
                                    EGL_NO_CONTEXT, contextAttribs);
}

void QVeridianGLContext::destroyEGL()
{
    if (m_eglDisplay != EGL_NO_DISPLAY) {
        eglMakeCurrent(m_eglDisplay, EGL_NO_SURFACE, EGL_NO_SURFACE,
                       EGL_NO_CONTEXT);
        if (m_eglContext != EGL_NO_CONTEXT) {
            eglDestroyContext(m_eglDisplay, m_eglContext);
            m_eglContext = EGL_NO_CONTEXT;
        }
        eglTerminate(m_eglDisplay);
        m_eglDisplay = EGL_NO_DISPLAY;
    }
}

/* ========================================================================= */
/* QPlatformOpenGLContext interface                                           */
/* ========================================================================= */

bool QVeridianGLContext::makeCurrent(QPlatformSurface *surface)
{
    Q_UNUSED(surface);

    if (m_eglDisplay == EGL_NO_DISPLAY || m_eglContext == EGL_NO_CONTEXT)
        return false;

    /* For a real implementation, we would obtain the EGLSurface from the
     * platform window.  For now, make current with no surface. */
    return eglMakeCurrent(m_eglDisplay, EGL_NO_SURFACE, EGL_NO_SURFACE,
                          m_eglContext) == EGL_TRUE;
}

void QVeridianGLContext::doneCurrent()
{
    if (m_eglDisplay != EGL_NO_DISPLAY) {
        eglMakeCurrent(m_eglDisplay, EGL_NO_SURFACE, EGL_NO_SURFACE,
                       EGL_NO_CONTEXT);
    }
}

void QVeridianGLContext::swapBuffers(QPlatformSurface *surface)
{
    Q_UNUSED(surface);
    /* Would call eglSwapBuffers with the window's EGLSurface */
}

QSurfaceFormat QVeridianGLContext::format() const
{
    return m_format;
}

bool QVeridianGLContext::isSharing() const
{
    return false;
}

bool QVeridianGLContext::isValid() const
{
    return m_eglContext != EGL_NO_CONTEXT;
}

QFunctionPointer QVeridianGLContext::getProcAddress(const char *procName)
{
    return reinterpret_cast<QFunctionPointer>(eglGetProcAddress(procName));
}

QT_END_NAMESPACE

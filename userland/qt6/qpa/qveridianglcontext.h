/*
 * VeridianOS -- qveridianglcontext.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformOpenGLContext implementation for VeridianOS.  Wraps EGL
 * context lifecycle (eglCreateContext, eglMakeCurrent, eglSwapBuffers)
 * for OpenGL ES 2.0 rendering via Mesa.
 */

#ifndef QVERIDIANGLCONTEXT_H
#define QVERIDIANGLCONTEXT_H

#include <QtGui/qpa/qplatformopenglcontext.h>
#include <EGL/egl.h>

QT_BEGIN_NAMESPACE

class QVeridianGLContext : public QPlatformOpenGLContext
{
public:
    explicit QVeridianGLContext(QOpenGLContext *context);
    ~QVeridianGLContext() override;

    bool makeCurrent(QPlatformSurface *surface) override;
    void doneCurrent() override;
    void swapBuffers(QPlatformSurface *surface) override;
    QSurfaceFormat format() const override;
    bool isSharing() const override;
    bool isValid() const override;
    QFunctionPointer getProcAddress(const char *procName) override;

private:
    void initEGL();
    void destroyEGL();

    EGLDisplay m_eglDisplay = EGL_NO_DISPLAY;
    EGLContext m_eglContext = EGL_NO_CONTEXT;
    EGLConfig  m_eglConfig  = nullptr;
    QSurfaceFormat m_format;
};

QT_END_NAMESPACE

#endif /* QVERIDIANGLCONTEXT_H */

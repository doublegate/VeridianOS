/*
 * VeridianOS libc -- <EGL/eglplatform.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * EGL platform type definitions for VeridianOS.
 * Defines native display, window, and pixmap types.
 */

#ifndef _EGL_EGLPLATFORM_H
#define _EGL_EGLPLATFORM_H

#ifdef __cplusplus
extern "C" {
#endif

#include <KHR/khrplatform.h>

/* ========================================================================= */
/* Native type definitions for VeridianOS                                    */
/* ========================================================================= */

/*
 * On VeridianOS with Wayland, native types map to:
 *   Display  -> struct wl_display *
 *   Window   -> struct wl_egl_window *  (or struct gbm_surface *)
 *   Pixmap   -> void * (unused)
 */

typedef void *EGLNativeDisplayType;
typedef void *EGLNativeWindowType;
typedef void *EGLNativePixmapType;

/* ========================================================================= */
/* Integer type for EGL                                                      */
/* ========================================================================= */

typedef khronos_int32_t EGLint;

/* ========================================================================= */
/* EGL 1.5 attrib type                                                       */
/* ========================================================================= */

typedef intptr_t EGLAttrib;

#ifdef __cplusplus
}
#endif

#endif /* _EGL_EGLPLATFORM_H */

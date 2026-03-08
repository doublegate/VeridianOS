/*
 * VeridianOS libc -- <epoxy/egl.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libepoxy EGL dispatch header.
 * Includes EGL headers and provides epoxy EGL query functions.
 */

#ifndef _EPOXY_EGL_H
#define _EPOXY_EGL_H

#ifdef __cplusplus
extern "C" {
#endif

#include <epoxy/common.h>
#include <stdbool.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>

/* ========================================================================= */
/* Epoxy EGL dispatch queries                                                */
/* ========================================================================= */

EPOXY_PUBLIC int  epoxy_egl_version(EGLDisplay dpy);
EPOXY_PUBLIC bool epoxy_has_egl_extension(EGLDisplay dpy, const char *extension);

/* ========================================================================= */
/* Epoxy EGL generated dispatch (minimal stub)                               */
/* ========================================================================= */

#include <epoxy/egl_generated.h>

#ifdef __cplusplus
}
#endif

#endif /* _EPOXY_EGL_H */

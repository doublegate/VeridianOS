/*
 * VeridianOS libc -- <epoxy/gl.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libepoxy OpenGL dispatch header.
 * Includes GLES2 and provides GL 4.x type compatibility for desktop GL code.
 */

#ifndef _EPOXY_GL_H
#define _EPOXY_GL_H

#ifdef __cplusplus
extern "C" {
#endif

#include <epoxy/common.h>
#include <stdbool.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <GLES3/gl3.h>

/* ========================================================================= */
/* Desktop GL type compatibility                                             */
/* ========================================================================= */

/* These types are needed by code that targets desktop GL but may also      */
/* compile against GLES. They are the same underlying types.                */
typedef double   GLdouble;
typedef int64_t  GLint64EXT;
typedef uint64_t GLuint64EXT;

/* ========================================================================= */
/* Epoxy dispatch queries                                                    */
/* ========================================================================= */

EPOXY_PUBLIC int  epoxy_gl_version(void);
EPOXY_PUBLIC bool epoxy_is_desktop_gl(void);
EPOXY_PUBLIC bool epoxy_has_gl_extension(const char *extension);

/* ========================================================================= */
/* Epoxy GL generated dispatch (minimal stub)                                */
/* ========================================================================= */

#include <epoxy/gl_generated.h>

#ifdef __cplusplus
}
#endif

#endif /* _EPOXY_GL_H */

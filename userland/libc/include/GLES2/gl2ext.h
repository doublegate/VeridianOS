/*
 * VeridianOS libc -- <GLES2/gl2ext.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenGL ES 2.0 extension declarations.
 * Common extensions used by Qt 6 and KWin.
 */

#ifndef _GLES2_GL2EXT_H
#define _GLES2_GL2EXT_H

#ifdef __cplusplus
extern "C" {
#endif

#include <GLES2/gl2.h>

/* ========================================================================= */
/* GL_OES_vertex_array_object                                                */
/* ========================================================================= */

#ifndef GL_OES_vertex_array_object
#define GL_OES_vertex_array_object 1

#define GL_VERTEX_ARRAY_BINDING_OES     0x85B5

GL_APICALL void     GL_APIENTRY glBindVertexArrayOES(GLuint array);
GL_APICALL void     GL_APIENTRY glDeleteVertexArraysOES(GLsizei n, const GLuint *arrays);
GL_APICALL void     GL_APIENTRY glGenVertexArraysOES(GLsizei n, GLuint *arrays);
GL_APICALL GLboolean GL_APIENTRY glIsVertexArrayOES(GLuint array);

#endif

/* ========================================================================= */
/* GL_OES_mapbuffer                                                          */
/* ========================================================================= */

#ifndef GL_OES_mapbuffer
#define GL_OES_mapbuffer 1

#define GL_WRITE_ONLY_OES               0x88B9
#define GL_BUFFER_ACCESS_OES            0x88BB
#define GL_BUFFER_MAPPED_OES            0x88BC
#define GL_BUFFER_MAP_POINTER_OES       0x88BD

GL_APICALL void *   GL_APIENTRY glMapBufferOES(GLenum target, GLenum access);
GL_APICALL GLboolean GL_APIENTRY glUnmapBufferOES(GLenum target);
GL_APICALL void     GL_APIENTRY glGetBufferPointervOES(GLenum target, GLenum pname, void **params);

#endif

/* ========================================================================= */
/* GL_OES_depth24                                                            */
/* ========================================================================= */

#ifndef GL_OES_depth24
#define GL_OES_depth24 1

#define GL_DEPTH_COMPONENT24_OES        0x81A6

#endif

/* ========================================================================= */
/* GL_OES_depth32                                                            */
/* ========================================================================= */

#ifndef GL_OES_depth32
#define GL_OES_depth32 1

#define GL_DEPTH_COMPONENT32_OES        0x81A7

#endif

/* ========================================================================= */
/* GL_OES_packed_depth_stencil                                               */
/* ========================================================================= */

#ifndef GL_OES_packed_depth_stencil
#define GL_OES_packed_depth_stencil 1

#define GL_DEPTH_STENCIL_OES            0x84F9
#define GL_UNSIGNED_INT_24_8_OES        0x84FA
#define GL_DEPTH24_STENCIL8_OES         0x88F0

#endif

/* ========================================================================= */
/* GL_OES_EGL_image                                                          */
/* ========================================================================= */

#ifndef GL_OES_EGL_image
#define GL_OES_EGL_image 1

typedef void *GLeglImageOES;

GL_APICALL void GL_APIENTRY glEGLImageTargetTexture2DOES(GLenum target, GLeglImageOES image);
GL_APICALL void GL_APIENTRY glEGLImageTargetRenderbufferStorageOES(GLenum target, GLeglImageOES image);

#endif

/* ========================================================================= */
/* GL_EXT_texture_format_BGRA8888                                            */
/* ========================================================================= */

#ifndef GL_EXT_texture_format_BGRA8888
#define GL_EXT_texture_format_BGRA8888 1

#define GL_BGRA_EXT                     0x80E1

#endif

/* ========================================================================= */
/* GL_EXT_discard_framebuffer                                                */
/* ========================================================================= */

#ifndef GL_EXT_discard_framebuffer
#define GL_EXT_discard_framebuffer 1

#define GL_COLOR_EXT                    0x1800
#define GL_DEPTH_EXT                    0x1801
#define GL_STENCIL_EXT                  0x1802

GL_APICALL void GL_APIENTRY glDiscardFramebufferEXT(GLenum target, GLsizei numAttachments, const GLenum *attachments);

#endif

/* ========================================================================= */
/* GL_EXT_blend_minmax                                                       */
/* ========================================================================= */

#ifndef GL_EXT_blend_minmax
#define GL_EXT_blend_minmax 1

#define GL_MIN_EXT                      0x8007
#define GL_MAX_EXT                      0x8008

#endif

/* ========================================================================= */
/* GL_OES_standard_derivatives                                               */
/* ========================================================================= */

#ifndef GL_OES_standard_derivatives
#define GL_OES_standard_derivatives 1

#define GL_FRAGMENT_SHADER_DERIVATIVE_HINT_OES 0x8B8B

#endif

/* ========================================================================= */
/* GL_OES_rgb8_rgba8                                                         */
/* ========================================================================= */

#ifndef GL_OES_rgb8_rgba8
#define GL_OES_rgb8_rgba8 1

#define GL_RGB8_OES                     0x8051
#define GL_RGBA8_OES                    0x8058

#endif

#ifdef __cplusplus
}
#endif

#endif /* _GLES2_GL2EXT_H */

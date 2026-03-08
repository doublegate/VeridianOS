/*
 * VeridianOS libc -- <GLES3/gl3.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenGL ES 3.0 API declarations.
 * Superset of GLES2 -- includes all GLES2 plus ES 3.0 additions.
 * Provided for completeness; VeridianOS currently targets GLES 2.0.
 */

#ifndef _GLES3_GL3_H
#define _GLES3_GL3_H

#ifdef __cplusplus
extern "C" {
#endif

/* Include all of GLES2 as the base */
#include <GLES2/gl2.h>

/* ========================================================================= */
/* GLES 3.0 additional types                                                 */
/* ========================================================================= */

typedef unsigned short GLhalf;
typedef khronos_int64_t GLint64;
typedef khronos_uint64_t GLuint64;
typedef struct __GLsync *GLsync;

/* ========================================================================= */
/* GLES 3.0 additional enums                                                 */
/* ========================================================================= */

/* Internal formats */
#define GL_R8                           0x8229
#define GL_RG8                          0x822B
#define GL_RGB8                         0x8051
#define GL_RGBA8                        0x8058
#define GL_R16F                         0x822D
#define GL_RG16F                        0x822F
#define GL_RGB16F                       0x881B
#define GL_RGBA16F                      0x881A
#define GL_R32F                         0x822E
#define GL_RG32F                        0x8230
#define GL_RGB32F                       0x8815
#define GL_RGBA32F                      0x8814
#define GL_R8I                          0x8231
#define GL_R8UI                         0x8232
#define GL_R16I                         0x8233
#define GL_R16UI                        0x8234
#define GL_R32I                         0x8235
#define GL_R32UI                        0x8236
#define GL_RG8I                         0x8237
#define GL_RG8UI                        0x8238
#define GL_RG16I                        0x8239
#define GL_RG16UI                       0x823A
#define GL_RG32I                        0x823B
#define GL_RG32UI                       0x823C
#define GL_RGBA32I                      0x8D82
#define GL_RGBA32UI                     0x8D70
#define GL_RGBA16I                      0x8D88
#define GL_RGBA16UI                     0x8D76
#define GL_RGBA8I                       0x8D8E
#define GL_RGBA8UI                      0x8D7C
#define GL_RGB10_A2                     0x8059
#define GL_RGB10_A2UI                   0x906F
#define GL_SRGB8                        0x8C41
#define GL_SRGB8_ALPHA8                 0x8C43
#define GL_DEPTH_COMPONENT24            0x81A6
#define GL_DEPTH_COMPONENT32F           0x8CAC
#define GL_DEPTH32F_STENCIL8            0x8CAD

/* Pixel formats */
#define GL_RED                          0x1903
#define GL_RG                           0x8227
#define GL_RED_INTEGER                  0x8D94
#define GL_RG_INTEGER                   0x8228
#define GL_RGB_INTEGER                  0x8D98
#define GL_RGBA_INTEGER                 0x8D99

/* Pixel types */
#define GL_HALF_FLOAT                   0x140B
#define GL_UNSIGNED_INT_2_10_10_10_REV  0x8368
#define GL_UNSIGNED_INT_10F_11F_11F_REV 0x8C3B
#define GL_UNSIGNED_INT_5_9_9_9_REV     0x8C3E
#define GL_FLOAT_32_UNSIGNED_INT_24_8_REV 0x8DAD
#define GL_UNSIGNED_INT_24_8            0x84FA

/* Buffer targets */
#define GL_COPY_READ_BUFFER             0x8F36
#define GL_COPY_WRITE_BUFFER            0x8F37
#define GL_PIXEL_PACK_BUFFER            0x88EB
#define GL_PIXEL_UNPACK_BUFFER          0x88EC
#define GL_TRANSFORM_FEEDBACK_BUFFER    0x8C8E
#define GL_UNIFORM_BUFFER               0x8A11

/* Vertex array objects */
#define GL_VERTEX_ARRAY_BINDING         0x85B5

/* Read buffer */
#define GL_READ_FRAMEBUFFER             0x8CA8
#define GL_DRAW_FRAMEBUFFER             0x8CA9

/* Sync */
#define GL_SYNC_GPU_COMMANDS_COMPLETE   0x9117
#define GL_ALREADY_SIGNALED             0x911A
#define GL_TIMEOUT_EXPIRED              0x911B
#define GL_CONDITION_SATISFIED          0x911C
#define GL_WAIT_FAILED                  0x911D
#define GL_SYNC_FLUSH_COMMANDS_BIT      0x00000001
#define GL_TIMEOUT_IGNORED              0xFFFFFFFFFFFFFFFFull

/* Sampler objects */
#define GL_SAMPLER_BINDING              0x8919

/* Queries */
#define GL_ANY_SAMPLES_PASSED           0x8C2F
#define GL_ANY_SAMPLES_PASSED_CONSERVATIVE 0x8D6A
#define GL_TRANSFORM_FEEDBACK_PAUSED    0x8E23
#define GL_TRANSFORM_FEEDBACK_ACTIVE    0x8E24

/* Misc */
#define GL_NUM_EXTENSIONS               0x821D
#define GL_MAJOR_VERSION                0x821B
#define GL_MINOR_VERSION                0x821C
#define GL_MAX_ELEMENTS_VERTICES        0x80E8
#define GL_MAX_ELEMENTS_INDICES         0x80E9
#define GL_MAX_3D_TEXTURE_SIZE          0x8073
#define GL_MAX_ARRAY_TEXTURE_LAYERS     0x88FF
#define GL_MAX_COLOR_ATTACHMENTS        0x8CDF
#define GL_MAX_DRAW_BUFFERS             0x8824

/* Map buffer */
#define GL_MAP_READ_BIT                 0x0001
#define GL_MAP_WRITE_BIT                0x0002
#define GL_MAP_INVALIDATE_RANGE_BIT     0x0004
#define GL_MAP_INVALIDATE_BUFFER_BIT    0x0008
#define GL_MAP_FLUSH_EXPLICIT_BIT       0x0010
#define GL_MAP_UNSYNCHRONIZED_BIT       0x0020

/* Texture 3D / 2D array */
#define GL_TEXTURE_3D                   0x806F
#define GL_TEXTURE_2D_ARRAY             0x8C1A
#define GL_TEXTURE_WRAP_R               0x8072

/* ========================================================================= */
/* GLES 3.0 function declarations (commonly used subset)                     */
/* ========================================================================= */

/* Vertex array objects */
GL_APICALL void     GL_APIENTRY glBindVertexArray(GLuint array);
GL_APICALL void     GL_APIENTRY glDeleteVertexArrays(GLsizei n, const GLuint *arrays);
GL_APICALL void     GL_APIENTRY glGenVertexArrays(GLsizei n, GLuint *arrays);
GL_APICALL GLboolean GL_APIENTRY glIsVertexArray(GLuint array);

/* Buffer mapping */
GL_APICALL void *   GL_APIENTRY glMapBufferRange(GLenum target, GLintptr offset, GLsizeiptr length, GLbitfield access);
GL_APICALL GLboolean GL_APIENTRY glUnmapBuffer(GLenum target);
GL_APICALL void     GL_APIENTRY glFlushMappedBufferRange(GLenum target, GLintptr offset, GLsizeiptr length);
GL_APICALL void     GL_APIENTRY glCopyBufferSubData(GLenum readTarget, GLenum writeTarget, GLintptr readOffset, GLintptr writeOffset, GLsizeiptr size);

/* Framebuffer blit */
GL_APICALL void     GL_APIENTRY glBlitFramebuffer(GLint srcX0, GLint srcY0, GLint srcX1, GLint srcY1, GLint dstX0, GLint dstY0, GLint dstX1, GLint dstY1, GLbitfield mask, GLenum filter);
GL_APICALL void     GL_APIENTRY glReadBuffer(GLenum src);
GL_APICALL void     GL_APIENTRY glDrawBuffers(GLsizei n, const GLenum *bufs);
GL_APICALL void     GL_APIENTRY glInvalidateFramebuffer(GLenum target, GLsizei numAttachments, const GLenum *attachments);

/* Renderbuffer multisample */
GL_APICALL void     GL_APIENTRY glRenderbufferStorageMultisample(GLenum target, GLsizei samples, GLenum internalformat, GLsizei width, GLsizei height);

/* Texture 3D / 2D array */
GL_APICALL void     GL_APIENTRY glTexImage3D(GLenum target, GLint level, GLint internalformat, GLsizei width, GLsizei height, GLsizei depth, GLint border, GLenum format, GLenum type, const void *pixels);
GL_APICALL void     GL_APIENTRY glTexSubImage3D(GLenum target, GLint level, GLint xoffset, GLint yoffset, GLint zoffset, GLsizei width, GLsizei height, GLsizei depth, GLenum format, GLenum type, const void *pixels);
GL_APICALL void     GL_APIENTRY glTexStorage2D(GLenum target, GLsizei levels, GLenum internalformat, GLsizei width, GLsizei height);
GL_APICALL void     GL_APIENTRY glTexStorage3D(GLenum target, GLsizei levels, GLenum internalformat, GLsizei width, GLsizei height, GLsizei depth);

/* Sampler objects */
GL_APICALL void     GL_APIENTRY glGenSamplers(GLsizei count, GLuint *samplers);
GL_APICALL void     GL_APIENTRY glDeleteSamplers(GLsizei count, const GLuint *samplers);
GL_APICALL void     GL_APIENTRY glBindSampler(GLuint unit, GLuint sampler);
GL_APICALL void     GL_APIENTRY glSamplerParameteri(GLuint sampler, GLenum pname, GLint param);
GL_APICALL void     GL_APIENTRY glSamplerParameterf(GLuint sampler, GLenum pname, GLfloat param);

/* Sync objects */
GL_APICALL GLsync   GL_APIENTRY glFenceSync(GLenum condition, GLbitfield flags);
GL_APICALL void     GL_APIENTRY glDeleteSync(GLsync sync);
GL_APICALL GLenum   GL_APIENTRY glClientWaitSync(GLsync sync, GLbitfield flags, GLuint64 timeout);
GL_APICALL void     GL_APIENTRY glWaitSync(GLsync sync, GLbitfield flags, GLuint64 timeout);

/* Uniform buffers */
GL_APICALL GLuint   GL_APIENTRY glGetUniformBlockIndex(GLuint program, const GLchar *uniformBlockName);
GL_APICALL void     GL_APIENTRY glUniformBlockBinding(GLuint program, GLuint uniformBlockIndex, GLuint uniformBlockBinding);
GL_APICALL void     GL_APIENTRY glBindBufferBase(GLenum target, GLuint index, GLuint buffer);
GL_APICALL void     GL_APIENTRY glBindBufferRange(GLenum target, GLuint index, GLuint buffer, GLintptr offset, GLsizeiptr size);

/* Draw instanced */
GL_APICALL void     GL_APIENTRY glDrawArraysInstanced(GLenum mode, GLint first, GLsizei count, GLsizei instancecount);
GL_APICALL void     GL_APIENTRY glDrawElementsInstanced(GLenum mode, GLsizei count, GLenum type, const void *indices, GLsizei instancecount);
GL_APICALL void     GL_APIENTRY glVertexAttribDivisor(GLuint index, GLuint divisor);

/* Draw range elements */
GL_APICALL void     GL_APIENTRY glDrawRangeElements(GLenum mode, GLuint start, GLuint end, GLsizei count, GLenum type, const void *indices);

/* Transform feedback */
GL_APICALL void     GL_APIENTRY glBeginTransformFeedback(GLenum primitiveMode);
GL_APICALL void     GL_APIENTRY glEndTransformFeedback(void);
GL_APICALL void     GL_APIENTRY glTransformFeedbackVaryings(GLuint program, GLsizei count, const GLchar *const *varyings, GLenum bufferMode);

/* String query by index (GLES 3.0) */
GL_APICALL const GLubyte * GL_APIENTRY glGetStringi(GLenum name, GLuint index);

/* Integer64 query */
GL_APICALL void     GL_APIENTRY glGetInteger64v(GLenum pname, GLint64 *data);

/* Clear buffer */
GL_APICALL void     GL_APIENTRY glClearBufferfv(GLenum buffer, GLint drawbuffer, const GLfloat *value);
GL_APICALL void     GL_APIENTRY glClearBufferiv(GLenum buffer, GLint drawbuffer, const GLint *value);
GL_APICALL void     GL_APIENTRY glClearBufferuiv(GLenum buffer, GLint drawbuffer, const GLuint *value);
GL_APICALL void     GL_APIENTRY glClearBufferfi(GLenum buffer, GLint drawbuffer, GLfloat depth, GLint stencil);

/* Uniform matrix (non-square) */
GL_APICALL void     GL_APIENTRY glUniformMatrix2x3fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value);
GL_APICALL void     GL_APIENTRY glUniformMatrix3x2fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value);
GL_APICALL void     GL_APIENTRY glUniformMatrix2x4fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value);
GL_APICALL void     GL_APIENTRY glUniformMatrix4x2fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value);
GL_APICALL void     GL_APIENTRY glUniformMatrix3x4fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value);
GL_APICALL void     GL_APIENTRY glUniformMatrix4x3fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value);

/* Unsigned integer uniforms */
GL_APICALL void     GL_APIENTRY glUniform1ui(GLint location, GLuint v0);
GL_APICALL void     GL_APIENTRY glUniform2ui(GLint location, GLuint v0, GLuint v1);
GL_APICALL void     GL_APIENTRY glUniform3ui(GLint location, GLuint v0, GLuint v1, GLuint v2);
GL_APICALL void     GL_APIENTRY glUniform4ui(GLint location, GLuint v0, GLuint v1, GLuint v2, GLuint v3);

/* Program binary */
GL_APICALL void     GL_APIENTRY glGetProgramBinary(GLuint program, GLsizei bufSize, GLsizei *length, GLenum *binaryFormat, void *binary);
GL_APICALL void     GL_APIENTRY glProgramBinary(GLuint program, GLenum binaryFormat, const void *binary, GLsizei length);
GL_APICALL void     GL_APIENTRY glProgramParameteri(GLuint program, GLenum pname, GLint value);

#ifdef __cplusplus
}
#endif

#endif /* _GLES3_GL3_H */

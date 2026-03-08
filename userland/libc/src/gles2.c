/*
 * VeridianOS libc -- gles2.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenGL ES 2.0 stub implementation.
 * Provides state tracking and stub operations backed by the kernel's
 * software GLES2 rasterizer. Draw calls are no-ops; shader compilation
 * always succeeds; queries return realistic values.
 */

#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <GLES3/gl3.h>
#include <string.h>
#include <stdlib.h>

/* ========================================================================= */
/* Internal state                                                            */
/* ========================================================================= */

#define MAX_GL_TEXTURES      256
#define MAX_GL_BUFFERS       256
#define MAX_GL_SHADERS       128
#define MAX_GL_PROGRAMS      64
#define MAX_GL_FRAMEBUFFERS  32
#define MAX_GL_RENDERBUFFERS 32
#define MAX_GL_VERTEX_ARRAYS 32

static GLenum  g_error = GL_NO_ERROR;

/* Clear state */
static GLfloat g_clear_color[4] = { 0.0f, 0.0f, 0.0f, 0.0f };
static GLfloat g_clear_depth    = 1.0f;
static GLint   g_clear_stencil  = 0;

/* Viewport and scissor */
static GLint   g_viewport[4]    = { 0, 0, 0, 0 };
static GLint   g_scissor[4]     = { 0, 0, 0, 0 };

/* Enable/disable caps */
static GLboolean g_cap_blend         = GL_FALSE;
static GLboolean g_cap_depth_test    = GL_FALSE;
static GLboolean g_cap_scissor_test  = GL_FALSE;
static GLboolean g_cap_stencil_test  = GL_FALSE;
static GLboolean g_cap_cull_face     = GL_FALSE;
static GLboolean g_cap_dither        = GL_TRUE;
static GLboolean g_cap_polygon_offset = GL_FALSE;
static GLboolean g_cap_sample_alpha  = GL_FALSE;
static GLboolean g_cap_sample_cov    = GL_FALSE;

/* Blend state */
static GLenum  g_blend_src_rgb   = GL_ONE;
static GLenum  g_blend_dst_rgb   = GL_ZERO;
static GLenum  g_blend_src_alpha = GL_ONE;
static GLenum  g_blend_dst_alpha = GL_ZERO;
static GLenum  g_blend_eq_rgb    = GL_FUNC_ADD;
static GLenum  g_blend_eq_alpha  = GL_FUNC_ADD;
static GLfloat g_blend_color[4]  = { 0.0f, 0.0f, 0.0f, 0.0f };

/* Depth/stencil state */
static GLenum  g_depth_func     = GL_LESS;
static GLboolean g_depth_mask   = GL_TRUE;
static GLenum  g_stencil_func   = GL_ALWAYS;
static GLint   g_stencil_ref    = 0;
static GLuint  g_stencil_mask   = 0xFFFFFFFF;
static GLuint  g_stencil_writemask = 0xFFFFFFFF;
static GLenum  g_stencil_fail   = GL_KEEP;
static GLenum  g_stencil_zfail  = GL_KEEP;
static GLenum  g_stencil_zpass  = GL_KEEP;

/* Face culling */
static GLenum  g_cull_face_mode = GL_BACK;
static GLenum  g_front_face     = GL_CCW;

/* Color mask */
static GLboolean g_color_mask[4] = { GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE };

/* Pixel store */
static GLint   g_pack_alignment   = 4;
static GLint   g_unpack_alignment = 4;

/* Active texture unit */
static GLenum  g_active_texture = GL_TEXTURE0;

/* Current program */
static GLuint  g_current_program = 0;

/* Current bindings */
static GLuint  g_bound_array_buffer   = 0;
static GLuint  g_bound_element_buffer = 0;
static GLuint  g_bound_framebuffer    = 0;
static GLuint  g_bound_renderbuffer   = 0;
static GLuint  g_bound_texture_2d     = 0;
static GLuint  g_bound_texture_cube   = 0;
static GLuint  g_bound_vertex_array   = 0;

/* Line width */
static GLfloat g_line_width = 1.0f;

/* Depth range */
static GLfloat g_depth_range[2] = { 0.0f, 1.0f };

/* Object ID counters */
static GLuint  g_next_texture      = 1;
static GLuint  g_next_buffer       = 1;
static GLuint  g_next_shader       = 1;
static GLuint  g_next_program      = 1;
static GLuint  g_next_framebuffer  = 1;
static GLuint  g_next_renderbuffer = 1;
static GLuint  g_next_vertex_array = 1;

/* Shader/program status tracking */
struct shader_info {
    GLuint id;
    GLenum type;
    int    compiled;
    int    deleted;
};

struct program_info {
    GLuint id;
    int    linked;
    int    deleted;
};

static struct shader_info  g_shaders[MAX_GL_SHADERS];
static struct program_info g_programs[MAX_GL_PROGRAMS];
static int g_num_shaders  = 0;
static int g_num_programs = 0;

/* ========================================================================= */
/* String constants                                                          */
/* ========================================================================= */

static const char *GL_VENDOR_STR    = "VeridianOS";
static const char *GL_RENDERER_STR  = "llvmpipe (LLVM 19.1, 256 bits)";
static const char *GL_VERSION_STR   = "OpenGL ES 2.0 VeridianOS";
static const char *GL_GLSL_VERSION_STR = "OpenGL ES GLSL ES 1.00";
static const char *GL_EXTENSIONS_STR =
    "GL_OES_vertex_array_object "
    "GL_OES_mapbuffer "
    "GL_OES_depth24 "
    "GL_OES_depth32 "
    "GL_OES_packed_depth_stencil "
    "GL_OES_EGL_image "
    "GL_OES_standard_derivatives "
    "GL_OES_rgb8_rgba8 "
    "GL_EXT_texture_format_BGRA8888 "
    "GL_EXT_discard_framebuffer "
    "GL_EXT_blend_minmax";

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static void set_gl_error(GLenum err)
{
    /* Per spec, only the first error is retained */
    if (g_error == GL_NO_ERROR)
        g_error = err;
}

static GLboolean *cap_ptr(GLenum cap)
{
    switch (cap) {
    case GL_BLEND:                   return &g_cap_blend;
    case GL_DEPTH_TEST:              return &g_cap_depth_test;
    case GL_SCISSOR_TEST:            return &g_cap_scissor_test;
    case GL_STENCIL_TEST:            return &g_cap_stencil_test;
    case GL_CULL_FACE:               return &g_cap_cull_face;
    case GL_DITHER:                  return &g_cap_dither;
    case GL_POLYGON_OFFSET_FILL:     return &g_cap_polygon_offset;
    case GL_SAMPLE_ALPHA_TO_COVERAGE: return &g_cap_sample_alpha;
    case GL_SAMPLE_COVERAGE:         return &g_cap_sample_cov;
    default: return NULL;
    }
}

static struct shader_info *find_shader(GLuint id)
{
    int i;
    for (i = 0; i < g_num_shaders; i++) {
        if (g_shaders[i].id == id && !g_shaders[i].deleted)
            return &g_shaders[i];
    }
    return NULL;
}

static struct program_info *find_program(GLuint id)
{
    int i;
    for (i = 0; i < g_num_programs; i++) {
        if (g_programs[i].id == id && !g_programs[i].deleted)
            return &g_programs[i];
    }
    return NULL;
}

/* ========================================================================= */
/* Error query                                                               */
/* ========================================================================= */

GLenum glGetError(void)
{
    GLenum err = g_error;
    g_error = GL_NO_ERROR;
    return err;
}

/* ========================================================================= */
/* Enable / Disable                                                          */
/* ========================================================================= */

void glEnable(GLenum cap)
{
    GLboolean *p = cap_ptr(cap);
    if (p) *p = GL_TRUE;
    else set_gl_error(GL_INVALID_ENUM);
}

void glDisable(GLenum cap)
{
    GLboolean *p = cap_ptr(cap);
    if (p) *p = GL_FALSE;
    else set_gl_error(GL_INVALID_ENUM);
}

GLboolean glIsEnabled(GLenum cap)
{
    GLboolean *p = cap_ptr(cap);
    if (p) return *p;
    set_gl_error(GL_INVALID_ENUM);
    return GL_FALSE;
}

/* ========================================================================= */
/* Viewport and scissor                                                      */
/* ========================================================================= */

void glViewport(GLint x, GLint y, GLsizei width, GLsizei height)
{
    if (width < 0 || height < 0) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }
    g_viewport[0] = x;
    g_viewport[1] = y;
    g_viewport[2] = (GLint)width;
    g_viewport[3] = (GLint)height;
}

void glScissor(GLint x, GLint y, GLsizei width, GLsizei height)
{
    if (width < 0 || height < 0) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }
    g_scissor[0] = x;
    g_scissor[1] = y;
    g_scissor[2] = (GLint)width;
    g_scissor[3] = (GLint)height;
}

void glDepthRangef(GLfloat n, GLfloat f)
{
    g_depth_range[0] = n;
    g_depth_range[1] = f;
}

void glLineWidth(GLfloat width)
{
    g_line_width = width;
}

void glPolygonOffset(GLfloat factor, GLfloat units)
{
    (void)factor;
    (void)units;
}

/* ========================================================================= */
/* Clear                                                                     */
/* ========================================================================= */

void glClear(GLbitfield mask)
{
    (void)mask;
    /* No-op in stub; kernel rasterizer would handle this */
}

void glClearColor(GLfloat red, GLfloat green, GLfloat blue, GLfloat alpha)
{
    g_clear_color[0] = red;
    g_clear_color[1] = green;
    g_clear_color[2] = blue;
    g_clear_color[3] = alpha;
}

void glClearDepthf(GLfloat d)
{
    g_clear_depth = d;
}

void glClearStencil(GLint s)
{
    g_clear_stencil = s;
}

void glColorMask(GLboolean red, GLboolean green, GLboolean blue, GLboolean alpha)
{
    g_color_mask[0] = red;
    g_color_mask[1] = green;
    g_color_mask[2] = blue;
    g_color_mask[3] = alpha;
}

void glDepthMask(GLboolean flag)
{
    g_depth_mask = flag;
}

void glStencilMask(GLuint mask)
{
    g_stencil_writemask = mask;
}

void glStencilMaskSeparate(GLenum face, GLuint mask)
{
    (void)face;
    g_stencil_writemask = mask;
}

/* ========================================================================= */
/* Blending                                                                  */
/* ========================================================================= */

void glBlendFunc(GLenum sfactor, GLenum dfactor)
{
    g_blend_src_rgb   = sfactor;
    g_blend_dst_rgb   = dfactor;
    g_blend_src_alpha = sfactor;
    g_blend_dst_alpha = dfactor;
}

void glBlendFuncSeparate(GLenum sfactorRGB, GLenum dfactorRGB,
                          GLenum sfactorAlpha, GLenum dfactorAlpha)
{
    g_blend_src_rgb   = sfactorRGB;
    g_blend_dst_rgb   = dfactorRGB;
    g_blend_src_alpha = sfactorAlpha;
    g_blend_dst_alpha = dfactorAlpha;
}

void glBlendEquation(GLenum mode)
{
    g_blend_eq_rgb   = mode;
    g_blend_eq_alpha = mode;
}

void glBlendEquationSeparate(GLenum modeRGB, GLenum modeAlpha)
{
    g_blend_eq_rgb   = modeRGB;
    g_blend_eq_alpha = modeAlpha;
}

void glBlendColor(GLfloat red, GLfloat green, GLfloat blue, GLfloat alpha)
{
    g_blend_color[0] = red;
    g_blend_color[1] = green;
    g_blend_color[2] = blue;
    g_blend_color[3] = alpha;
}

/* ========================================================================= */
/* Depth and stencil                                                         */
/* ========================================================================= */

void glDepthFunc(GLenum func)
{
    g_depth_func = func;
}

void glStencilFunc(GLenum func, GLint ref, GLuint mask)
{
    g_stencil_func = func;
    g_stencil_ref  = ref;
    g_stencil_mask = mask;
}

void glStencilFuncSeparate(GLenum face, GLenum func, GLint ref, GLuint mask)
{
    (void)face;
    g_stencil_func = func;
    g_stencil_ref  = ref;
    g_stencil_mask = mask;
}

void glStencilOp(GLenum fail, GLenum zfail, GLenum zpass)
{
    g_stencil_fail  = fail;
    g_stencil_zfail = zfail;
    g_stencil_zpass = zpass;
}

void glStencilOpSeparate(GLenum face, GLenum sfail, GLenum dpfail, GLenum dppass)
{
    (void)face;
    g_stencil_fail  = sfail;
    g_stencil_zfail = dpfail;
    g_stencil_zpass = dppass;
}

/* ========================================================================= */
/* Face culling                                                              */
/* ========================================================================= */

void glCullFace(GLenum mode)
{
    g_cull_face_mode = mode;
}

void glFrontFace(GLenum mode)
{
    g_front_face = mode;
}

/* ========================================================================= */
/* Textures                                                                  */
/* ========================================================================= */

void glGenTextures(GLsizei n, GLuint *textures)
{
    GLsizei i;
    if (n < 0 || !textures) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }
    for (i = 0; i < n; i++)
        textures[i] = g_next_texture++;
}

void glDeleteTextures(GLsizei n, const GLuint *textures)
{
    (void)n;
    (void)textures;
}

void glBindTexture(GLenum target, GLuint texture)
{
    if (target == GL_TEXTURE_2D)
        g_bound_texture_2d = texture;
    else if (target == GL_TEXTURE_CUBE_MAP)
        g_bound_texture_cube = texture;
    else
        set_gl_error(GL_INVALID_ENUM);
}

void glActiveTexture(GLenum texture)
{
    if (texture < GL_TEXTURE0 || texture > GL_TEXTURE7) {
        set_gl_error(GL_INVALID_ENUM);
        return;
    }
    g_active_texture = texture;
}

void glTexImage2D(GLenum target, GLint level, GLint internalformat,
                  GLsizei width, GLsizei height, GLint border,
                  GLenum format, GLenum type, const void *pixels)
{
    (void)target; (void)level; (void)internalformat;
    (void)width; (void)height; (void)border;
    (void)format; (void)type; (void)pixels;
}

void glTexSubImage2D(GLenum target, GLint level, GLint xoffset, GLint yoffset,
                     GLsizei width, GLsizei height, GLenum format,
                     GLenum type, const void *pixels)
{
    (void)target; (void)level; (void)xoffset; (void)yoffset;
    (void)width; (void)height; (void)format; (void)type; (void)pixels;
}

void glTexParameteri(GLenum target, GLenum pname, GLint param)
{
    (void)target; (void)pname; (void)param;
}

void glTexParameterf(GLenum target, GLenum pname, GLfloat param)
{
    (void)target; (void)pname; (void)param;
}

void glTexParameteriv(GLenum target, GLenum pname, const GLint *params)
{
    (void)target; (void)pname; (void)params;
}

void glTexParameterfv(GLenum target, GLenum pname, const GLfloat *params)
{
    (void)target; (void)pname; (void)params;
}

void glGetTexParameteriv(GLenum target, GLenum pname, GLint *params)
{
    (void)target;
    if (!params) return;
    switch (pname) {
    case GL_TEXTURE_MIN_FILTER: *params = GL_NEAREST_MIPMAP_LINEAR; break;
    case GL_TEXTURE_MAG_FILTER: *params = GL_LINEAR;                break;
    case GL_TEXTURE_WRAP_S:     *params = GL_REPEAT;                break;
    case GL_TEXTURE_WRAP_T:     *params = GL_REPEAT;                break;
    default: *params = 0; break;
    }
}

void glGetTexParameterfv(GLenum target, GLenum pname, GLfloat *params)
{
    GLint ival;
    (void)target;
    if (!params) return;
    glGetTexParameteriv(target, pname, &ival);
    *params = (GLfloat)ival;
}

void glGenerateMipmap(GLenum target)
{
    (void)target;
}

GLboolean glIsTexture(GLuint texture)
{
    return (texture > 0 && texture < g_next_texture) ? GL_TRUE : GL_FALSE;
}

void glCopyTexImage2D(GLenum target, GLint level, GLenum internalformat,
                      GLint x, GLint y, GLsizei width, GLsizei height,
                      GLint border)
{
    (void)target; (void)level; (void)internalformat;
    (void)x; (void)y; (void)width; (void)height; (void)border;
}

void glCopyTexSubImage2D(GLenum target, GLint level, GLint xoffset,
                         GLint yoffset, GLint x, GLint y,
                         GLsizei width, GLsizei height)
{
    (void)target; (void)level; (void)xoffset; (void)yoffset;
    (void)x; (void)y; (void)width; (void)height;
}

void glCompressedTexImage2D(GLenum target, GLint level, GLenum internalformat,
                            GLsizei width, GLsizei height, GLint border,
                            GLsizei imageSize, const void *data)
{
    (void)target; (void)level; (void)internalformat;
    (void)width; (void)height; (void)border;
    (void)imageSize; (void)data;
}

void glCompressedTexSubImage2D(GLenum target, GLint level, GLint xoffset,
                               GLint yoffset, GLsizei width, GLsizei height,
                               GLenum format, GLsizei imageSize,
                               const void *data)
{
    (void)target; (void)level; (void)xoffset; (void)yoffset;
    (void)width; (void)height; (void)format;
    (void)imageSize; (void)data;
}

/* ========================================================================= */
/* Buffer objects                                                            */
/* ========================================================================= */

void glGenBuffers(GLsizei n, GLuint *buffers)
{
    GLsizei i;
    if (n < 0 || !buffers) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }
    for (i = 0; i < n; i++)
        buffers[i] = g_next_buffer++;
}

void glDeleteBuffers(GLsizei n, const GLuint *buffers)
{
    (void)n;
    (void)buffers;
}

void glBindBuffer(GLenum target, GLuint buffer)
{
    if (target == GL_ARRAY_BUFFER)
        g_bound_array_buffer = buffer;
    else if (target == GL_ELEMENT_ARRAY_BUFFER)
        g_bound_element_buffer = buffer;
}

void glBufferData(GLenum target, GLsizeiptr size, const void *data,
                  GLenum usage)
{
    (void)target; (void)size; (void)data; (void)usage;
}

void glBufferSubData(GLenum target, GLintptr offset, GLsizeiptr size,
                     const void *data)
{
    (void)target; (void)offset; (void)size; (void)data;
}

GLboolean glIsBuffer(GLuint buffer)
{
    return (buffer > 0 && buffer < g_next_buffer) ? GL_TRUE : GL_FALSE;
}

void glGetBufferParameteriv(GLenum target, GLenum pname, GLint *params)
{
    (void)target;
    if (!params) return;
    switch (pname) {
    case GL_BUFFER_SIZE:  *params = 0;              break;
    case GL_BUFFER_USAGE: *params = GL_STATIC_DRAW; break;
    default: *params = 0; break;
    }
}

/* ========================================================================= */
/* Shaders                                                                   */
/* ========================================================================= */

GLuint glCreateShader(GLenum type)
{
    if (type != GL_VERTEX_SHADER && type != GL_FRAGMENT_SHADER) {
        set_gl_error(GL_INVALID_ENUM);
        return 0;
    }

    if (g_num_shaders >= MAX_GL_SHADERS) {
        set_gl_error(GL_OUT_OF_MEMORY);
        return 0;
    }

    g_shaders[g_num_shaders].id       = g_next_shader++;
    g_shaders[g_num_shaders].type     = type;
    g_shaders[g_num_shaders].compiled = 0;
    g_shaders[g_num_shaders].deleted  = 0;
    g_num_shaders++;

    return g_shaders[g_num_shaders - 1].id;
}

void glDeleteShader(GLuint shader)
{
    struct shader_info *s = find_shader(shader);
    if (s) s->deleted = 1;
}

void glShaderSource(GLuint shader, GLsizei count, const GLchar *const *string,
                    const GLint *length)
{
    (void)shader; (void)count; (void)string; (void)length;
    /* Source is accepted but not stored */
}

void glCompileShader(GLuint shader)
{
    struct shader_info *s = find_shader(shader);
    if (s) s->compiled = 1;
}

void glGetShaderiv(GLuint shader, GLenum pname, GLint *params)
{
    struct shader_info *s = find_shader(shader);
    if (!params) return;

    if (!s) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }

    switch (pname) {
    case GL_SHADER_TYPE:      *params = (GLint)s->type;     break;
    case GL_COMPILE_STATUS:   *params = s->compiled;         break;
    case GL_DELETE_STATUS:    *params = s->deleted;          break;
    case GL_INFO_LOG_LENGTH:  *params = 1;                   break;
    case GL_SHADER_SOURCE_LENGTH: *params = 1;               break;
    default:
        set_gl_error(GL_INVALID_ENUM);
        break;
    }
}

void glGetShaderInfoLog(GLuint shader, GLsizei bufSize, GLsizei *length,
                        GLchar *infoLog)
{
    (void)shader;
    if (length) *length = 0;
    if (infoLog && bufSize > 0) infoLog[0] = '\0';
}

void glGetShaderSource(GLuint shader, GLsizei bufSize, GLsizei *length,
                       GLchar *source)
{
    (void)shader;
    if (length) *length = 0;
    if (source && bufSize > 0) source[0] = '\0';
}

GLboolean glIsShader(GLuint shader)
{
    return find_shader(shader) ? GL_TRUE : GL_FALSE;
}

/* ========================================================================= */
/* Programs                                                                  */
/* ========================================================================= */

GLuint glCreateProgram(void)
{
    if (g_num_programs >= MAX_GL_PROGRAMS) {
        set_gl_error(GL_OUT_OF_MEMORY);
        return 0;
    }

    g_programs[g_num_programs].id      = g_next_program++;
    g_programs[g_num_programs].linked  = 0;
    g_programs[g_num_programs].deleted = 0;
    g_num_programs++;

    return g_programs[g_num_programs - 1].id;
}

void glDeleteProgram(GLuint program)
{
    struct program_info *p = find_program(program);
    if (p) p->deleted = 1;
}

void glAttachShader(GLuint program, GLuint shader)
{
    (void)program; (void)shader;
}

void glDetachShader(GLuint program, GLuint shader)
{
    (void)program; (void)shader;
}

void glLinkProgram(GLuint program)
{
    struct program_info *p = find_program(program);
    if (p) p->linked = 1;
}

void glUseProgram(GLuint program)
{
    g_current_program = program;
}

void glValidateProgram(GLuint program)
{
    (void)program;
}

void glGetProgramiv(GLuint program, GLenum pname, GLint *params)
{
    struct program_info *p = find_program(program);
    if (!params) return;

    if (!p) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }

    switch (pname) {
    case GL_LINK_STATUS:      *params = p->linked;  break;
    case GL_VALIDATE_STATUS:  *params = p->linked;  break;
    case GL_DELETE_STATUS:    *params = p->deleted;  break;
    case GL_INFO_LOG_LENGTH:  *params = 1;           break;
    case GL_ATTACHED_SHADERS: *params = 2;           break;
    case GL_ACTIVE_UNIFORMS:  *params = 0;           break;
    case GL_ACTIVE_UNIFORM_MAX_LENGTH: *params = 1;  break;
    case GL_ACTIVE_ATTRIBUTES: *params = 0;          break;
    case GL_ACTIVE_ATTRIBUTE_MAX_LENGTH: *params = 1; break;
    default:
        set_gl_error(GL_INVALID_ENUM);
        break;
    }
}

void glGetProgramInfoLog(GLuint program, GLsizei bufSize, GLsizei *length,
                         GLchar *infoLog)
{
    (void)program;
    if (length) *length = 0;
    if (infoLog && bufSize > 0) infoLog[0] = '\0';
}

GLboolean glIsProgram(GLuint program)
{
    return find_program(program) ? GL_TRUE : GL_FALSE;
}

/* ========================================================================= */
/* Uniforms                                                                  */
/* ========================================================================= */

GLint glGetUniformLocation(GLuint program, const GLchar *name)
{
    (void)program;
    (void)name;
    /* Return a valid-looking location to avoid error paths */
    return 0;
}

void glGetActiveUniform(GLuint program, GLuint index, GLsizei bufSize,
                        GLsizei *length, GLint *size, GLenum *type,
                        GLchar *name)
{
    (void)program; (void)index;
    if (length) *length = 0;
    if (size)   *size   = 0;
    if (type)   *type   = GL_FLOAT;
    if (name && bufSize > 0) name[0] = '\0';
}

void glUniform1i(GLint location, GLint v0)
{ (void)location; (void)v0; }

void glUniform2i(GLint location, GLint v0, GLint v1)
{ (void)location; (void)v0; (void)v1; }

void glUniform3i(GLint location, GLint v0, GLint v1, GLint v2)
{ (void)location; (void)v0; (void)v1; (void)v2; }

void glUniform4i(GLint location, GLint v0, GLint v1, GLint v2, GLint v3)
{ (void)location; (void)v0; (void)v1; (void)v2; (void)v3; }

void glUniform1f(GLint location, GLfloat v0)
{ (void)location; (void)v0; }

void glUniform2f(GLint location, GLfloat v0, GLfloat v1)
{ (void)location; (void)v0; (void)v1; }

void glUniform3f(GLint location, GLfloat v0, GLfloat v1, GLfloat v2)
{ (void)location; (void)v0; (void)v1; (void)v2; }

void glUniform4f(GLint location, GLfloat v0, GLfloat v1, GLfloat v2, GLfloat v3)
{ (void)location; (void)v0; (void)v1; (void)v2; (void)v3; }

void glUniform1iv(GLint location, GLsizei count, const GLint *value)
{ (void)location; (void)count; (void)value; }

void glUniform2iv(GLint location, GLsizei count, const GLint *value)
{ (void)location; (void)count; (void)value; }

void glUniform3iv(GLint location, GLsizei count, const GLint *value)
{ (void)location; (void)count; (void)value; }

void glUniform4iv(GLint location, GLsizei count, const GLint *value)
{ (void)location; (void)count; (void)value; }

void glUniform1fv(GLint location, GLsizei count, const GLfloat *value)
{ (void)location; (void)count; (void)value; }

void glUniform2fv(GLint location, GLsizei count, const GLfloat *value)
{ (void)location; (void)count; (void)value; }

void glUniform3fv(GLint location, GLsizei count, const GLfloat *value)
{ (void)location; (void)count; (void)value; }

void glUniform4fv(GLint location, GLsizei count, const GLfloat *value)
{ (void)location; (void)count; (void)value; }

void glUniformMatrix2fv(GLint location, GLsizei count, GLboolean transpose,
                        const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix3fv(GLint location, GLsizei count, GLboolean transpose,
                        const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix4fv(GLint location, GLsizei count, GLboolean transpose,
                        const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glGetUniformfv(GLuint program, GLint location, GLfloat *params)
{ (void)program; (void)location; if (params) *params = 0.0f; }

void glGetUniformiv(GLuint program, GLint location, GLint *params)
{ (void)program; (void)location; if (params) *params = 0; }

/* ========================================================================= */
/* Vertex attributes                                                         */
/* ========================================================================= */

void glBindAttribLocation(GLuint program, GLuint index, const GLchar *name)
{ (void)program; (void)index; (void)name; }

GLint glGetAttribLocation(GLuint program, const GLchar *name)
{
    (void)program; (void)name;
    return 0;
}

void glGetActiveAttrib(GLuint program, GLuint index, GLsizei bufSize,
                       GLsizei *length, GLint *size, GLenum *type,
                       GLchar *name)
{
    (void)program; (void)index;
    if (length) *length = 0;
    if (size)   *size   = 0;
    if (type)   *type   = GL_FLOAT;
    if (name && bufSize > 0) name[0] = '\0';
}

void glVertexAttrib1f(GLuint index, GLfloat x)
{ (void)index; (void)x; }

void glVertexAttrib2f(GLuint index, GLfloat x, GLfloat y)
{ (void)index; (void)x; (void)y; }

void glVertexAttrib3f(GLuint index, GLfloat x, GLfloat y, GLfloat z)
{ (void)index; (void)x; (void)y; (void)z; }

void glVertexAttrib4f(GLuint index, GLfloat x, GLfloat y, GLfloat z, GLfloat w)
{ (void)index; (void)x; (void)y; (void)z; (void)w; }

void glVertexAttrib1fv(GLuint index, const GLfloat *v)
{ (void)index; (void)v; }

void glVertexAttrib2fv(GLuint index, const GLfloat *v)
{ (void)index; (void)v; }

void glVertexAttrib3fv(GLuint index, const GLfloat *v)
{ (void)index; (void)v; }

void glVertexAttrib4fv(GLuint index, const GLfloat *v)
{ (void)index; (void)v; }

void glVertexAttribPointer(GLuint index, GLint size, GLenum type,
                           GLboolean normalized, GLsizei stride,
                           const void *pointer)
{
    (void)index; (void)size; (void)type;
    (void)normalized; (void)stride; (void)pointer;
}

void glEnableVertexAttribArray(GLuint index)
{ (void)index; }

void glDisableVertexAttribArray(GLuint index)
{ (void)index; }

void glGetVertexAttribfv(GLuint index, GLenum pname, GLfloat *params)
{ (void)index; (void)pname; if (params) *params = 0.0f; }

void glGetVertexAttribiv(GLuint index, GLenum pname, GLint *params)
{ (void)index; (void)pname; if (params) *params = 0; }

void glGetVertexAttribPointerv(GLuint index, GLenum pname, void **pointer)
{ (void)index; (void)pname; if (pointer) *pointer = NULL; }

/* ========================================================================= */
/* Drawing                                                                   */
/* ========================================================================= */

void glDrawArrays(GLenum mode, GLint first, GLsizei count)
{
    (void)mode; (void)first; (void)count;
}

void glDrawElements(GLenum mode, GLsizei count, GLenum type,
                    const void *indices)
{
    (void)mode; (void)count; (void)type; (void)indices;
}

/* ========================================================================= */
/* Framebuffer objects                                                       */
/* ========================================================================= */

void glGenFramebuffers(GLsizei n, GLuint *framebuffers)
{
    GLsizei i;
    if (n < 0 || !framebuffers) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }
    for (i = 0; i < n; i++)
        framebuffers[i] = g_next_framebuffer++;
}

void glDeleteFramebuffers(GLsizei n, const GLuint *framebuffers)
{
    (void)n; (void)framebuffers;
}

void glBindFramebuffer(GLenum target, GLuint framebuffer)
{
    (void)target;
    g_bound_framebuffer = framebuffer;
}

GLenum glCheckFramebufferStatus(GLenum target)
{
    (void)target;
    return GL_FRAMEBUFFER_COMPLETE;
}

void glFramebufferTexture2D(GLenum target, GLenum attachment,
                            GLenum textarget, GLuint texture, GLint level)
{
    (void)target; (void)attachment; (void)textarget;
    (void)texture; (void)level;
}

void glFramebufferRenderbuffer(GLenum target, GLenum attachment,
                               GLenum renderbuffertarget, GLuint renderbuffer)
{
    (void)target; (void)attachment;
    (void)renderbuffertarget; (void)renderbuffer;
}

GLboolean glIsFramebuffer(GLuint framebuffer)
{
    return (framebuffer > 0 && framebuffer < g_next_framebuffer) ? GL_TRUE : GL_FALSE;
}

void glGetFramebufferAttachmentParameteriv(GLenum target, GLenum attachment,
                                            GLenum pname, GLint *params)
{
    (void)target; (void)attachment; (void)pname;
    if (params) *params = 0;
}

/* ========================================================================= */
/* Renderbuffer objects                                                      */
/* ========================================================================= */

void glGenRenderbuffers(GLsizei n, GLuint *renderbuffers)
{
    GLsizei i;
    if (n < 0 || !renderbuffers) {
        set_gl_error(GL_INVALID_VALUE);
        return;
    }
    for (i = 0; i < n; i++)
        renderbuffers[i] = g_next_renderbuffer++;
}

void glDeleteRenderbuffers(GLsizei n, const GLuint *renderbuffers)
{
    (void)n; (void)renderbuffers;
}

void glBindRenderbuffer(GLenum target, GLuint renderbuffer)
{
    (void)target;
    g_bound_renderbuffer = renderbuffer;
}

void glRenderbufferStorage(GLenum target, GLenum internalformat,
                           GLsizei width, GLsizei height)
{
    (void)target; (void)internalformat; (void)width; (void)height;
}

GLboolean glIsRenderbuffer(GLuint renderbuffer)
{
    return (renderbuffer > 0 && renderbuffer < g_next_renderbuffer) ? GL_TRUE : GL_FALSE;
}

void glGetRenderbufferParameteriv(GLenum target, GLenum pname, GLint *params)
{
    (void)target;
    if (!params) return;
    switch (pname) {
    case GL_RENDERBUFFER_WIDTH:           *params = 0; break;
    case GL_RENDERBUFFER_HEIGHT:          *params = 0; break;
    case GL_RENDERBUFFER_INTERNAL_FORMAT: *params = GL_RGBA; break;
    case GL_RENDERBUFFER_RED_SIZE:        *params = 8; break;
    case GL_RENDERBUFFER_GREEN_SIZE:      *params = 8; break;
    case GL_RENDERBUFFER_BLUE_SIZE:       *params = 8; break;
    case GL_RENDERBUFFER_ALPHA_SIZE:      *params = 8; break;
    case GL_RENDERBUFFER_DEPTH_SIZE:      *params = 0; break;
    case GL_RENDERBUFFER_STENCIL_SIZE:    *params = 0; break;
    default: *params = 0; break;
    }
}

/* ========================================================================= */
/* Pixel operations                                                          */
/* ========================================================================= */

void glPixelStorei(GLenum pname, GLint param)
{
    if (pname == GL_PACK_ALIGNMENT)
        g_pack_alignment = param;
    else if (pname == GL_UNPACK_ALIGNMENT)
        g_unpack_alignment = param;
}

void glReadPixels(GLint x, GLint y, GLsizei width, GLsizei height,
                  GLenum format, GLenum type, void *pixels)
{
    (void)x; (void)y; (void)width; (void)height;
    (void)format; (void)type;
    /* Zero-fill the output buffer */
    if (pixels && width > 0 && height > 0) {
        int bpp = 4; /* RGBA */
        if (format == GL_RGB) bpp = 3;
        memset(pixels, 0, (size_t)(width * height * bpp));
    }
}

/* ========================================================================= */
/* String and parameter queries                                              */
/* ========================================================================= */

const GLubyte *glGetString(GLenum name)
{
    switch (name) {
    case GL_VENDOR:                  return (const GLubyte *)GL_VENDOR_STR;
    case GL_RENDERER:                return (const GLubyte *)GL_RENDERER_STR;
    case GL_VERSION:                 return (const GLubyte *)GL_VERSION_STR;
    case GL_SHADING_LANGUAGE_VERSION: return (const GLubyte *)GL_GLSL_VERSION_STR;
    case GL_EXTENSIONS:              return (const GLubyte *)GL_EXTENSIONS_STR;
    default:
        set_gl_error(GL_INVALID_ENUM);
        return NULL;
    }
}

void glGetIntegerv(GLenum pname, GLint *data)
{
    if (!data) return;

    switch (pname) {
    case GL_MAX_TEXTURE_SIZE:             *data = 4096;   break;
    case GL_MAX_RENDERBUFFER_SIZE:        *data = 4096;   break;
    case GL_MAX_VIEWPORT_DIMS:            data[0] = 4096; data[1] = 4096; break;
    case GL_MAX_VERTEX_ATTRIBS:           *data = 16;     break;
    case GL_MAX_VERTEX_UNIFORM_VECTORS:   *data = 256;    break;
    case GL_MAX_VARYING_VECTORS:          *data = 15;     break;
    case GL_MAX_FRAGMENT_UNIFORM_VECTORS: *data = 256;    break;
    case GL_MAX_TEXTURE_IMAGE_UNITS:      *data = 8;      break;
    case GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS: *data = 8;    break;
    case GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS: *data = 16; break;
    case GL_SUBPIXEL_BITS:                *data = 4;      break;
    case GL_NUM_COMPRESSED_TEXTURE_FORMATS: *data = 0;    break;
    case GL_SAMPLE_BUFFERS:               *data = 0;      break;
    case GL_SAMPLES:                      *data = 0;      break;
    case GL_PACK_ALIGNMENT:               *data = g_pack_alignment; break;
    case GL_UNPACK_ALIGNMENT:             *data = g_unpack_alignment; break;
    case GL_VIEWPORT:
        data[0] = g_viewport[0]; data[1] = g_viewport[1];
        data[2] = g_viewport[2]; data[3] = g_viewport[3];
        break;
    case GL_SCISSOR_BOX:
        data[0] = g_scissor[0]; data[1] = g_scissor[1];
        data[2] = g_scissor[2]; data[3] = g_scissor[3];
        break;
    case GL_COLOR_WRITEMASK:
        data[0] = g_color_mask[0]; data[1] = g_color_mask[1];
        data[2] = g_color_mask[2]; data[3] = g_color_mask[3];
        break;
    case GL_DEPTH_WRITEMASK:              *data = g_depth_mask;        break;
    case GL_STENCIL_WRITEMASK:            *data = (GLint)g_stencil_writemask; break;
    case GL_STENCIL_REF:                  *data = g_stencil_ref;       break;
    case GL_STENCIL_VALUE_MASK:           *data = (GLint)g_stencil_mask; break;
    case GL_STENCIL_FUNC:                 *data = (GLint)g_stencil_func; break;
    case GL_STENCIL_FAIL:                 *data = (GLint)g_stencil_fail; break;
    case GL_STENCIL_PASS_DEPTH_FAIL:      *data = (GLint)g_stencil_zfail; break;
    case GL_STENCIL_PASS_DEPTH_PASS:      *data = (GLint)g_stencil_zpass; break;
    case GL_BLEND_SRC_RGB:                *data = (GLint)g_blend_src_rgb; break;
    case GL_BLEND_DST_RGB:                *data = (GLint)g_blend_dst_rgb; break;
    case GL_BLEND_SRC_ALPHA:              *data = (GLint)g_blend_src_alpha; break;
    case GL_BLEND_DST_ALPHA:              *data = (GLint)g_blend_dst_alpha; break;
    case GL_BLEND_EQUATION_RGB:           *data = (GLint)g_blend_eq_rgb; break;
    case GL_BLEND_EQUATION_ALPHA:         *data = (GLint)g_blend_eq_alpha; break;
    case GL_FRONT_FACE:                   *data = (GLint)g_front_face;   break;
    case GL_CULL_FACE_MODE:               *data = (GLint)g_cull_face_mode; break;
    case GL_DEPTH_FUNC:                   *data = (GLint)g_depth_func;   break;
    case GL_ACTIVE_TEXTURE:               *data = (GLint)g_active_texture; break;
    case GL_CURRENT_PROGRAM:              *data = (GLint)g_current_program; break;
    case GL_IMPLEMENTATION_COLOR_READ_TYPE:   *data = GL_UNSIGNED_BYTE;  break;
    case GL_IMPLEMENTATION_COLOR_READ_FORMAT: *data = GL_RGBA;           break;
    case GL_GENERATE_MIPMAP_HINT:         *data = GL_DONT_CARE;          break;
    case GL_ALIASED_POINT_SIZE_RANGE:     data[0] = 1; data[1] = 256;   break;
    case GL_ALIASED_LINE_WIDTH_RANGE:     data[0] = 1; data[1] = 16;    break;
    /* GLES 3.0 queries */
    case GL_MAJOR_VERSION:                *data = 2;      break;
    case GL_MINOR_VERSION:                *data = 0;      break;
    case GL_NUM_EXTENSIONS:               *data = 11;     break;
    case GL_MAX_ELEMENTS_VERTICES:        *data = 65536;  break;
    case GL_MAX_ELEMENTS_INDICES:         *data = 65536;  break;
    case GL_MAX_3D_TEXTURE_SIZE:          *data = 256;    break;
    case GL_MAX_ARRAY_TEXTURE_LAYERS:     *data = 256;    break;
    case GL_MAX_COLOR_ATTACHMENTS:        *data = 4;      break;
    case GL_MAX_DRAW_BUFFERS:             *data = 4;      break;
    default:
        *data = 0;
        break;
    }
}

void glGetFloatv(GLenum pname, GLfloat *data)
{
    if (!data) return;

    switch (pname) {
    case GL_COLOR_CLEAR_VALUE:
        data[0] = g_clear_color[0]; data[1] = g_clear_color[1];
        data[2] = g_clear_color[2]; data[3] = g_clear_color[3];
        break;
    case GL_DEPTH_CLEAR_VALUE:
        *data = g_clear_depth;
        break;
    case GL_DEPTH_RANGE:
        data[0] = g_depth_range[0]; data[1] = g_depth_range[1];
        break;
    case GL_LINE_WIDTH:
        *data = g_line_width;
        break;
    case GL_BLEND_COLOR:
        data[0] = g_blend_color[0]; data[1] = g_blend_color[1];
        data[2] = g_blend_color[2]; data[3] = g_blend_color[3];
        break;
    default: {
        GLint ival;
        glGetIntegerv(pname, &ival);
        *data = (GLfloat)ival;
        break;
    }
    }
}

void glGetBooleanv(GLenum pname, GLboolean *data)
{
    GLint ival;
    if (!data) return;
    glGetIntegerv(pname, &ival);
    *data = (ival != 0) ? GL_TRUE : GL_FALSE;
}

/* ========================================================================= */
/* Hints, flush, finish                                                      */
/* ========================================================================= */

void glHint(GLenum target, GLenum mode)
{
    (void)target; (void)mode;
}

void glFlush(void)
{
    /* No-op */
}

void glFinish(void)
{
    /* No-op */
}

/* ========================================================================= */
/* Sample coverage                                                           */
/* ========================================================================= */

void glSampleCoverage(GLfloat value, GLboolean invert)
{
    (void)value; (void)invert;
}

/* ========================================================================= */
/* Shader precision                                                          */
/* ========================================================================= */

void glGetShaderPrecisionFormat(GLenum shadertype, GLenum precisiontype,
                                GLint *range, GLint *precision)
{
    (void)shadertype; (void)precisiontype;
    if (range) { range[0] = 127; range[1] = 127; }
    if (precision) *precision = 23;
}

void glReleaseShaderCompiler(void)
{
    /* No-op */
}

void glShaderBinary(GLsizei count, const GLuint *shaders,
                    GLenum binaryformat, const void *binary, GLsizei length)
{
    (void)count; (void)shaders; (void)binaryformat;
    (void)binary; (void)length;
    set_gl_error(GL_INVALID_ENUM); /* Binary shaders not supported */
}

/* ========================================================================= */
/* GLES 3.0 stubs                                                           */
/* ========================================================================= */

void glBindVertexArray(GLuint array)
{
    g_bound_vertex_array = array;
}

void glDeleteVertexArrays(GLsizei n, const GLuint *arrays)
{
    (void)n; (void)arrays;
}

void glGenVertexArrays(GLsizei n, GLuint *arrays)
{
    GLsizei i;
    if (n < 0 || !arrays) { set_gl_error(GL_INVALID_VALUE); return; }
    for (i = 0; i < n; i++)
        arrays[i] = g_next_vertex_array++;
}

GLboolean glIsVertexArray(GLuint array)
{
    return (array > 0 && array < g_next_vertex_array) ? GL_TRUE : GL_FALSE;
}

void *glMapBufferRange(GLenum target, GLintptr offset, GLsizeiptr length,
                       GLbitfield access)
{
    (void)target; (void)offset; (void)length; (void)access;
    return NULL;
}

GLboolean glUnmapBuffer(GLenum target)
{
    (void)target;
    return GL_TRUE;
}

void glFlushMappedBufferRange(GLenum target, GLintptr offset, GLsizeiptr length)
{
    (void)target; (void)offset; (void)length;
}

void glCopyBufferSubData(GLenum readTarget, GLenum writeTarget,
                         GLintptr readOffset, GLintptr writeOffset,
                         GLsizeiptr size)
{
    (void)readTarget; (void)writeTarget;
    (void)readOffset; (void)writeOffset; (void)size;
}

void glBlitFramebuffer(GLint srcX0, GLint srcY0, GLint srcX1, GLint srcY1,
                       GLint dstX0, GLint dstY0, GLint dstX1, GLint dstY1,
                       GLbitfield mask, GLenum filter)
{
    (void)srcX0; (void)srcY0; (void)srcX1; (void)srcY1;
    (void)dstX0; (void)dstY0; (void)dstX1; (void)dstY1;
    (void)mask; (void)filter;
}

void glReadBuffer(GLenum src)
{ (void)src; }

void glDrawBuffers(GLsizei n, const GLenum *bufs)
{ (void)n; (void)bufs; }

void glInvalidateFramebuffer(GLenum target, GLsizei numAttachments,
                             const GLenum *attachments)
{ (void)target; (void)numAttachments; (void)attachments; }

void glRenderbufferStorageMultisample(GLenum target, GLsizei samples,
                                      GLenum internalformat,
                                      GLsizei width, GLsizei height)
{
    (void)target; (void)samples; (void)internalformat;
    (void)width; (void)height;
}

void glTexImage3D(GLenum target, GLint level, GLint internalformat,
                  GLsizei width, GLsizei height, GLsizei depth, GLint border,
                  GLenum format, GLenum type, const void *pixels)
{
    (void)target; (void)level; (void)internalformat;
    (void)width; (void)height; (void)depth; (void)border;
    (void)format; (void)type; (void)pixels;
}

void glTexSubImage3D(GLenum target, GLint level, GLint xoffset, GLint yoffset,
                     GLint zoffset, GLsizei width, GLsizei height,
                     GLsizei depth, GLenum format, GLenum type,
                     const void *pixels)
{
    (void)target; (void)level; (void)xoffset; (void)yoffset; (void)zoffset;
    (void)width; (void)height; (void)depth;
    (void)format; (void)type; (void)pixels;
}

void glTexStorage2D(GLenum target, GLsizei levels, GLenum internalformat,
                    GLsizei width, GLsizei height)
{
    (void)target; (void)levels; (void)internalformat;
    (void)width; (void)height;
}

void glTexStorage3D(GLenum target, GLsizei levels, GLenum internalformat,
                    GLsizei width, GLsizei height, GLsizei depth)
{
    (void)target; (void)levels; (void)internalformat;
    (void)width; (void)height; (void)depth;
}

void glGenSamplers(GLsizei count, GLuint *samplers)
{
    GLsizei i;
    if (count < 0 || !samplers) { set_gl_error(GL_INVALID_VALUE); return; }
    for (i = 0; i < count; i++)
        samplers[i] = (GLuint)(i + 1);
}

void glDeleteSamplers(GLsizei count, const GLuint *samplers)
{ (void)count; (void)samplers; }

void glBindSampler(GLuint unit, GLuint sampler)
{ (void)unit; (void)sampler; }

void glSamplerParameteri(GLuint sampler, GLenum pname, GLint param)
{ (void)sampler; (void)pname; (void)param; }

void glSamplerParameterf(GLuint sampler, GLenum pname, GLfloat param)
{ (void)sampler; (void)pname; (void)param; }

GLsync glFenceSync(GLenum condition, GLbitfield flags)
{
    (void)condition; (void)flags;
    return (GLsync)(intptr_t)1; /* Sentinel */
}

void glDeleteSync(GLsync sync)
{ (void)sync; }

GLenum glClientWaitSync(GLsync sync, GLbitfield flags, GLuint64 timeout)
{
    (void)sync; (void)flags; (void)timeout;
    return GL_ALREADY_SIGNALED;
}

void glWaitSync(GLsync sync, GLbitfield flags, GLuint64 timeout)
{ (void)sync; (void)flags; (void)timeout; }

GLuint glGetUniformBlockIndex(GLuint program, const GLchar *uniformBlockName)
{ (void)program; (void)uniformBlockName; return 0; }

void glUniformBlockBinding(GLuint program, GLuint uniformBlockIndex,
                           GLuint uniformBlockBinding)
{ (void)program; (void)uniformBlockIndex; (void)uniformBlockBinding; }

void glBindBufferBase(GLenum target, GLuint index, GLuint buffer)
{ (void)target; (void)index; (void)buffer; }

void glBindBufferRange(GLenum target, GLuint index, GLuint buffer,
                       GLintptr offset, GLsizeiptr size)
{ (void)target; (void)index; (void)buffer; (void)offset; (void)size; }

void glDrawArraysInstanced(GLenum mode, GLint first, GLsizei count,
                           GLsizei instancecount)
{ (void)mode; (void)first; (void)count; (void)instancecount; }

void glDrawElementsInstanced(GLenum mode, GLsizei count, GLenum type,
                             const void *indices, GLsizei instancecount)
{ (void)mode; (void)count; (void)type; (void)indices; (void)instancecount; }

void glVertexAttribDivisor(GLuint index, GLuint divisor)
{ (void)index; (void)divisor; }

void glDrawRangeElements(GLenum mode, GLuint start, GLuint end, GLsizei count,
                         GLenum type, const void *indices)
{ (void)mode; (void)start; (void)end; (void)count; (void)type; (void)indices; }

void glBeginTransformFeedback(GLenum primitiveMode)
{ (void)primitiveMode; }

void glEndTransformFeedback(void)
{ }

void glTransformFeedbackVaryings(GLuint program, GLsizei count,
                                  const GLchar *const *varyings,
                                  GLenum bufferMode)
{ (void)program; (void)count; (void)varyings; (void)bufferMode; }

const GLubyte *glGetStringi(GLenum name, GLuint index)
{
    (void)name; (void)index;
    return (const GLubyte *)"";
}

void glGetInteger64v(GLenum pname, GLint64 *data)
{
    GLint ival;
    if (!data) return;
    glGetIntegerv(pname, &ival);
    *data = (GLint64)ival;
}

void glClearBufferfv(GLenum buffer, GLint drawbuffer, const GLfloat *value)
{ (void)buffer; (void)drawbuffer; (void)value; }

void glClearBufferiv(GLenum buffer, GLint drawbuffer, const GLint *value)
{ (void)buffer; (void)drawbuffer; (void)value; }

void glClearBufferuiv(GLenum buffer, GLint drawbuffer, const GLuint *value)
{ (void)buffer; (void)drawbuffer; (void)value; }

void glClearBufferfi(GLenum buffer, GLint drawbuffer, GLfloat depth, GLint stencil)
{ (void)buffer; (void)drawbuffer; (void)depth; (void)stencil; }

void glUniformMatrix2x3fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix3x2fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix2x4fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix4x2fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix3x4fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniformMatrix4x3fv(GLint location, GLsizei count, GLboolean transpose, const GLfloat *value)
{ (void)location; (void)count; (void)transpose; (void)value; }

void glUniform1ui(GLint location, GLuint v0)
{ (void)location; (void)v0; }

void glUniform2ui(GLint location, GLuint v0, GLuint v1)
{ (void)location; (void)v0; (void)v1; }

void glUniform3ui(GLint location, GLuint v0, GLuint v1, GLuint v2)
{ (void)location; (void)v0; (void)v1; (void)v2; }

void glUniform4ui(GLint location, GLuint v0, GLuint v1, GLuint v2, GLuint v3)
{ (void)location; (void)v0; (void)v1; (void)v2; (void)v3; }

void glGetProgramBinary(GLuint program, GLsizei bufSize, GLsizei *length,
                        GLenum *binaryFormat, void *binary)
{ (void)program; (void)bufSize; (void)binary;
  if (length) *length = 0; if (binaryFormat) *binaryFormat = 0; }

void glProgramBinary(GLuint program, GLenum binaryFormat,
                     const void *binary, GLsizei length)
{ (void)program; (void)binaryFormat; (void)binary; (void)length; }

void glProgramParameteri(GLuint program, GLenum pname, GLint value)
{ (void)program; (void)pname; (void)value; }

/* ========================================================================= */
/* OES extension stubs                                                       */
/* ========================================================================= */

void glBindVertexArrayOES(GLuint array)
{
    glBindVertexArray(array);
}

void glDeleteVertexArraysOES(GLsizei n, const GLuint *arrays)
{
    glDeleteVertexArrays(n, arrays);
}

void glGenVertexArraysOES(GLsizei n, GLuint *arrays)
{
    glGenVertexArrays(n, arrays);
}

GLboolean glIsVertexArrayOES(GLuint array)
{
    return glIsVertexArray(array);
}

void *glMapBufferOES(GLenum target, GLenum access)
{
    (void)target; (void)access;
    return NULL;
}

GLboolean glUnmapBufferOES(GLenum target)
{
    (void)target;
    return GL_TRUE;
}

void glGetBufferPointervOES(GLenum target, GLenum pname, void **params)
{
    (void)target; (void)pname;
    if (params) *params = NULL;
}

void glEGLImageTargetTexture2DOES(GLenum target, GLeglImageOES image)
{
    (void)target; (void)image;
}

void glEGLImageTargetRenderbufferStorageOES(GLenum target, GLeglImageOES image)
{
    (void)target; (void)image;
}

void glDiscardFramebufferEXT(GLenum target, GLsizei numAttachments,
                             const GLenum *attachments)
{
    (void)target; (void)numAttachments; (void)attachments;
}

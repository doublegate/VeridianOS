/*
 * VeridianOS libc -- <EGL/egl.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * EGL 1.5 API declarations.
 * Backed by VeridianOS DRM/GBM and kernel GLES2 rasterizer.
 */

#ifndef _EGL_EGL_H
#define _EGL_EGL_H

#ifdef __cplusplus
extern "C" {
#endif

#include <EGL/eglplatform.h>

/* ========================================================================= */
/* EGL version                                                               */
/* ========================================================================= */

#define EGL_VERSION_1_0 1
#define EGL_VERSION_1_1 1
#define EGL_VERSION_1_2 1
#define EGL_VERSION_1_3 1
#define EGL_VERSION_1_4 1
#define EGL_VERSION_1_5 1

/* ========================================================================= */
/* Opaque handle types                                                       */
/* ========================================================================= */

typedef void *EGLDisplay;
typedef void *EGLConfig;
typedef void *EGLSurface;
typedef void *EGLContext;
typedef void *EGLImage;
typedef void *EGLSync;

typedef void (*__eglMustCastToProperFunctionPointerType)(void);

/* ========================================================================= */
/* Special values                                                            */
/* ========================================================================= */

#define EGL_DEFAULT_DISPLAY     ((EGLNativeDisplayType)0)
#define EGL_NO_DISPLAY          ((EGLDisplay)0)
#define EGL_NO_CONTEXT          ((EGLContext)0)
#define EGL_NO_SURFACE          ((EGLSurface)0)
#define EGL_NO_IMAGE            ((EGLImage)0)
#define EGL_NO_SYNC             ((EGLSync)0)

#define EGL_DONT_CARE           ((EGLint)-1)
#define EGL_UNKNOWN             ((EGLint)-1)

/* ========================================================================= */
/* Boolean                                                                   */
/* ========================================================================= */

typedef unsigned int EGLBoolean;

#define EGL_TRUE                1
#define EGL_FALSE               0

/* ========================================================================= */
/* Errors                                                                    */
/* ========================================================================= */

#define EGL_SUCCESS                     0x3000
#define EGL_NOT_INITIALIZED             0x3001
#define EGL_BAD_ACCESS                  0x3002
#define EGL_BAD_ALLOC                   0x3003
#define EGL_BAD_ATTRIBUTE               0x3004
#define EGL_BAD_CONFIG                  0x3005
#define EGL_BAD_CONTEXT                 0x3006
#define EGL_BAD_CURRENT_SURFACE         0x3007
#define EGL_BAD_DISPLAY                 0x3008
#define EGL_BAD_MATCH                   0x3009
#define EGL_BAD_NATIVE_PIXMAP           0x300A
#define EGL_BAD_NATIVE_WINDOW           0x300B
#define EGL_BAD_PARAMETER               0x300C
#define EGL_BAD_SURFACE                 0x300D
#define EGL_CONTEXT_LOST                0x300E

/* ========================================================================= */
/* Config attributes                                                         */
/* ========================================================================= */

#define EGL_BUFFER_SIZE                 0x3020
#define EGL_ALPHA_SIZE                  0x3021
#define EGL_BLUE_SIZE                   0x3022
#define EGL_GREEN_SIZE                  0x3023
#define EGL_RED_SIZE                    0x3024
#define EGL_DEPTH_SIZE                  0x3025
#define EGL_STENCIL_SIZE                0x3026
#define EGL_CONFIG_CAVEAT               0x3027
#define EGL_CONFIG_ID                   0x3028
#define EGL_LEVEL                       0x3029
#define EGL_MAX_PBUFFER_HEIGHT          0x302A
#define EGL_MAX_PBUFFER_PIXELS          0x302B
#define EGL_MAX_PBUFFER_WIDTH           0x302C
#define EGL_NATIVE_RENDERABLE           0x302D
#define EGL_NATIVE_VISUAL_ID            0x302E
#define EGL_NATIVE_VISUAL_TYPE          0x302F
#define EGL_SAMPLES                     0x3031
#define EGL_SAMPLE_BUFFERS              0x3032
#define EGL_SURFACE_TYPE                0x3033
#define EGL_TRANSPARENT_TYPE            0x3034
#define EGL_TRANSPARENT_BLUE_VALUE      0x3035
#define EGL_TRANSPARENT_GREEN_VALUE     0x3036
#define EGL_TRANSPARENT_RED_VALUE       0x3037
#define EGL_NONE                        0x3038
#define EGL_BIND_TO_TEXTURE_RGB         0x3039
#define EGL_BIND_TO_TEXTURE_RGBA        0x303A
#define EGL_MIN_SWAP_INTERVAL           0x303B
#define EGL_MAX_SWAP_INTERVAL           0x303C
#define EGL_LUMINANCE_SIZE              0x303D
#define EGL_ALPHA_MASK_SIZE             0x303E
#define EGL_COLOR_BUFFER_TYPE           0x303F
#define EGL_RENDERABLE_TYPE             0x3040
#define EGL_CONFORMANT                  0x3042

/* ========================================================================= */
/* Config caveat values                                                      */
/* ========================================================================= */

#define EGL_SLOW_CONFIG                 0x3050
#define EGL_NON_CONFORMANT_CONFIG       0x3051

/* ========================================================================= */
/* Surface attributes                                                        */
/* ========================================================================= */

#define EGL_PBUFFER_BIT                 0x0001
#define EGL_PIXMAP_BIT                  0x0002
#define EGL_WINDOW_BIT                  0x0004

#define EGL_HEIGHT                      0x3056
#define EGL_WIDTH                       0x3057
#define EGL_LARGEST_PBUFFER             0x3058
#define EGL_RENDER_BUFFER               0x3086

#define EGL_BACK_BUFFER                 0x3084
#define EGL_SINGLE_BUFFER               0x3085

/* ========================================================================= */
/* Context attributes                                                        */
/* ========================================================================= */

#define EGL_CONTEXT_CLIENT_VERSION      0x3098
#define EGL_CONTEXT_MAJOR_VERSION       0x3098
#define EGL_CONTEXT_MINOR_VERSION       0x30FB
#define EGL_CONTEXT_OPENGL_PROFILE_MASK 0x30FD

/* ========================================================================= */
/* API binding                                                               */
/* ========================================================================= */

#define EGL_OPENGL_ES_API               0x30A0
#define EGL_OPENVG_API                  0x30A1
#define EGL_OPENGL_API                  0x30A2

/* ========================================================================= */
/* Renderable type bits                                                      */
/* ========================================================================= */

#define EGL_OPENGL_ES_BIT               0x0001
#define EGL_OPENVG_BIT                  0x0002
#define EGL_OPENGL_ES2_BIT              0x0004
#define EGL_OPENGL_ES3_BIT              0x0040
#define EGL_OPENGL_BIT                  0x0008

/* ========================================================================= */
/* Color buffer type                                                         */
/* ========================================================================= */

#define EGL_RGB_BUFFER                  0x308E
#define EGL_LUMINANCE_BUFFER            0x308F

/* ========================================================================= */
/* Transparent type                                                          */
/* ========================================================================= */

#define EGL_TRANSPARENT_RGB             0x3052

/* ========================================================================= */
/* Query strings                                                             */
/* ========================================================================= */

#define EGL_VENDOR                      0x3053
#define EGL_VERSION                     0x3054
#define EGL_EXTENSIONS                  0x3055
#define EGL_CLIENT_APIS                 0x308D

/* ========================================================================= */
/* Swap behavior                                                             */
/* ========================================================================= */

#define EGL_SWAP_BEHAVIOR               0x3093
#define EGL_BUFFER_PRESERVED            0x3094
#define EGL_BUFFER_DESTROYED            0x3095

/* ========================================================================= */
/* EGL 1.5 platform                                                          */
/* ========================================================================= */

#define EGL_PLATFORM_WAYLAND_KHR        0x31D8
#define EGL_PLATFORM_GBM_MESA           0x31D7

/* ========================================================================= */
/* Core EGL functions                                                        */
/* ========================================================================= */

/* Display management */
EGLDisplay eglGetDisplay(EGLNativeDisplayType display_id);
EGLBoolean eglInitialize(EGLDisplay dpy, EGLint *major, EGLint *minor);
EGLBoolean eglTerminate(EGLDisplay dpy);

/* Config management */
EGLBoolean eglChooseConfig(EGLDisplay dpy, const EGLint *attrib_list,
                           EGLConfig *configs, EGLint config_size,
                           EGLint *num_config);
EGLBoolean eglGetConfigAttrib(EGLDisplay dpy, EGLConfig config,
                              EGLint attribute, EGLint *value);
EGLBoolean eglGetConfigs(EGLDisplay dpy, EGLConfig *configs,
                         EGLint config_size, EGLint *num_config);

/* Context management */
EGLContext eglCreateContext(EGLDisplay dpy, EGLConfig config,
                            EGLContext share_context,
                            const EGLint *attrib_list);
EGLBoolean eglDestroyContext(EGLDisplay dpy, EGLContext ctx);

/* Surface management */
EGLSurface eglCreateWindowSurface(EGLDisplay dpy, EGLConfig config,
                                   EGLNativeWindowType win,
                                   const EGLint *attrib_list);
EGLSurface eglCreatePbufferSurface(EGLDisplay dpy, EGLConfig config,
                                    const EGLint *attrib_list);
EGLBoolean eglDestroySurface(EGLDisplay dpy, EGLSurface surface);

/* Rendering */
EGLBoolean eglMakeCurrent(EGLDisplay dpy, EGLSurface draw,
                           EGLSurface read, EGLContext ctx);
EGLBoolean eglSwapBuffers(EGLDisplay dpy, EGLSurface surface);
EGLBoolean eglSwapInterval(EGLDisplay dpy, EGLint interval);

/* Query */
EGLint     eglGetError(void);
const char *eglQueryString(EGLDisplay dpy, EGLint name);
EGLBoolean eglQuerySurface(EGLDisplay dpy, EGLSurface surface,
                            EGLint attribute, EGLint *value);
EGLBoolean eglQueryContext(EGLDisplay dpy, EGLContext ctx,
                            EGLint attribute, EGLint *value);

/* API binding */
EGLBoolean eglBindAPI(unsigned int api);
unsigned int eglQueryAPI(void);
EGLBoolean eglWaitClient(void);
EGLBoolean eglWaitGL(void);
EGLBoolean eglWaitNative(EGLint engine);
EGLBoolean eglReleaseThread(void);

/* Current context queries */
EGLContext eglGetCurrentContext(void);
EGLSurface eglGetCurrentSurface(EGLint readdraw);
EGLDisplay eglGetCurrentDisplay(void);

/* Proc address */
__eglMustCastToProperFunctionPointerType eglGetProcAddress(const char *procname);

/* EGL 1.5 platform display */
EGLDisplay eglGetPlatformDisplay(unsigned int platform, void *native_display,
                                  const EGLAttrib *attrib_list);

#ifdef __cplusplus
}
#endif

#endif /* _EGL_EGL_H */
